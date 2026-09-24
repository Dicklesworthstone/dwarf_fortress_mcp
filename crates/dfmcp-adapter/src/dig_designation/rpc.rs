//! Fixed dig/1.16 RPC, with explicit authority, bounded I/O and no reconnect.
//!
//! Calls are synchronous and must run in the caller's owned blocking region.
//! Each connection is pinned to one bounded region. This low-level adapter is
//! not a durable coordinator or production admission.
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, FortressId, GameTick, OperationContext, Result, RiskTier,
};

use super::{DigEffect, DigObservation, DigPhase, DigPlan, DigRegion, Reader, error, require};

pub const MAX_REPLY_BYTES: usize = 32 * 1024;
pub const RPC_BYTES: u64 = 300 * 1024;
pub const CONNECT_BYTES: u64 = 7 * RPC_BYTES + 24;
const METHODS: [&str; 6] = [
    "Handshake",
    "ReadDesignation",
    "PrepareDesignation",
    "CommitDesignation",
    "QueryDesignation",
    "CancelDesignation",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigManifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigPreparation {
    effect: DigEffect,
    replayed: bool,
}
impl DigPreparation {
    pub fn decode(raw: &[u8], replayed: bool, plan: &DigPlan) -> Result<Self> {
        let effect = DigEffect::decode(raw, plan)?;
        require(
            replayed || effect.phase() == DigPhase::Prepared,
            "fresh dig preparation has a historical outcome",
        )?;
        Ok(Self { effect, replayed })
    }
    pub fn effect(&self) -> &DigEffect {
        &self.effect
    }
    pub fn replayed(&self) -> bool {
        self.replayed
    }
}

pub trait DigSource {
    fn manifest(&self) -> &DigManifest;
    fn endpoint(&self) -> Option<SocketAddr>;
    fn observe(&mut self, region: DigRegion, context: &OperationContext) -> Result<DigObservation>;
    fn prepare(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<DigPreparation>;
    fn commit(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<DigEffect>;
    fn query(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<Option<DigEffect>>;
    fn cancel(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<DigEffect>;
}

/// Current grants only. Retained bytes never restore authority. Query/Observe
/// cover the full halo; mutations require the complete shared-block write area.
pub fn authorize(
    context: &OperationContext,
    fortress: FortressId,
    tick: u64,
    region: DigRegion,
    observe: bool,
    write: bool,
    prepare: bool,
) -> Result<()> {
    if context.anchor.fortress_id != fortress {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "dig authority belongs to another fortress",
        ));
    }
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(context.anchor.tick.get()));
    current.authorize(
        Capability::Query,
        RiskTier::ReadOnly,
        &[],
        Some(region.halo()),
    )?;
    if observe || write {
        current.authorize(
            Capability::Observe,
            RiskTier::ReadOnly,
            &[],
            Some(region.halo()),
        )?;
    }
    if write {
        current.authorize(
            Capability::Designate,
            RiskTier::Guarded,
            &[],
            Some(region.write_area()),
        )?;
    }
    if prepare {
        current.authorize(
            Capability::Plan,
            RiskTier::Guarded,
            &[],
            Some(region.write_area()),
        )?;
    }
    Ok(())
}

fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        "dig RPC failed; a failed commit does not prove nonapplication",
    )
    .retryable(false)
}
fn allowance(context: &OperationContext) -> Result<Duration> {
    context.budget.validate()?;
    if context.budget.max_wall_millis > 60_000 || context.budget.max_bytes < RPC_BYTES {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "dig RPC exceeds its bounded work allowance",
        ));
    }
    Ok(Duration::from_millis(context.budget.max_wall_millis))
}

pub trait DigStream: Read + Write {
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()>;
}
pub struct DigTcpStream {
    stream: TcpStream,
    deadline: Instant,
}
impl DigTcpStream {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "dig deadline"))
    }
}
impl DigStream for DigTcpStream {
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()> {
        let candidate = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "dig deadline overflow"))?;
        self.deadline = self.deadline.min(candidate);
        self.remaining().map_err(io_error)?;
        Ok(())
    }
}
impl Read for DigTcpStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(out)
    }
}
impl Write for DigTcpStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.stream.flush()
    }
}

fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn number(out: &mut Vec<u8>, tag: u64, n: u64) {
    varint(out, tag * 8);
    varint(out, n);
}
fn bytes(out: &mut Vec<u8>, tag: u64, data: &[u8]) {
    varint(out, tag * 8 + 2);
    varint(out, data.len() as u64);
    out.extend_from_slice(data);
}
#[derive(Clone, Copy)]
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}
struct Message<'a>([Option<Field<'a>>; 12]);
impl<'a> Message<'a> {
    fn parse(raw: &'a [u8], maximum: usize) -> Result<Self> {
        require(
            raw.len() <= MAX_REPLY_BYTES && maximum <= 11,
            "dig reply field/byte bound",
        )?;
        let mut r = Reader(raw);
        let mut out = Self([None; 12]);
        fn take(r: &mut Reader<'_>) -> Result<u64> {
            let mut value = 0;
            for i in 0..10 {
                let byte = r.byte()?;
                require(i < 9 || byte <= 1, "protobuf integer overflow")?;
                value |= u64::from(byte & 127) << (i * 7);
                if byte < 128 {
                    require(i == 0 || byte != 0, "nonminimal protobuf integer")?;
                    return Ok(value);
                }
            }
            Err(error(
                ErrorCode::AdapterRejected,
                "unterminated protobuf integer",
            ))
        }
        while !r.0.is_empty() {
            let tag = take(&mut r)?;
            require(
                tag >> 3 > 0 && tag >> 3 <= maximum as u64,
                "unknown dig reply field",
            )?;
            let n = (tag >> 3) as usize;
            require(out.0[n].is_none(), "duplicate dig reply field")?;
            out.0[n] = Some(match tag & 7 {
                0 => Field::Number(take(&mut r)?),
                2 => {
                    let width = take(&mut r)?;
                    require(width <= r.0.len() as u64, "truncated protobuf byte field")?;
                    Field::Bytes(r.take(width as usize)?)
                }
                _ => {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "unsupported dig protobuf wire type",
                    ));
                }
            });
        }
        Ok(out)
    }
    fn number(&self, n: usize) -> Result<u64> {
        match self.0[n] {
            Some(Field::Number(v)) => Ok(v),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "missing or mistyped dig number",
            )),
        }
    }
    fn bytes(&self, n: usize) -> Result<&'a [u8]> {
        match self.0[n] {
            Some(Field::Bytes(v)) => Ok(v),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "missing or mistyped dig bytes",
            )),
        }
    }
    fn has(&self, n: usize) -> bool {
        self.0[n].is_some()
    }
    fn count(&self) -> usize {
        self.0.iter().filter(|field| field.is_some()).count()
    }
}
fn version(raw: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(raw)
        .map_err(|_| error(ErrorCode::AdapterRejected, "invalid dig version UTF-8"))?;
    require(
        !value.is_empty() && value.len() <= 128 && !value.contains('\0'),
        "invalid dig version extent",
    )?;
    Ok(value.to_owned())
}
fn frame<S: Read + Write>(stream: &mut S, method: i16, request: &[u8]) -> Result<Vec<u8>> {
    require(request.len() <= 2048, "dig request exceeds 2 KiB")?;
    let mut header = [0; 8];
    header[..2].copy_from_slice(&method.to_le_bytes());
    header[4..].copy_from_slice(&(request.len() as i32).to_le_bytes());
    stream
        .write_all(&header)
        .and_then(|_| stream.write_all(request))
        .and_then(|_| stream.flush())
        .map_err(io_error)?;
    let mut count = 0;
    let mut total = 0;
    loop {
        stream.read_exact(&mut header).map_err(io_error)?;
        let kind = i16::from_le_bytes([header[0], header[1]]);
        let size = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        require(
            matches!(kind, -1 | -3)
                && size >= 0
                && size as usize <= if kind == -1 { MAX_REPLY_BYTES } else { 65_536 },
            "native dig frame refused",
        )?;
        if kind == -3 {
            count += 1;
            total += size;
            require(
                count <= 8 && total <= 262_144,
                "dig notification allowance exhausted",
            )?;
        }
        let mut payload = vec![0; size as usize];
        stream.read_exact(&mut payload).map_err(io_error)?;
        if kind == -1 {
            return Ok(payload);
        }
    }
}

pub struct DigRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    methods: [i16; 6],
    manifest: DigManifest,
    region: DigRegion,
    endpoint: Option<SocketAddr>,
    fenced: bool,
    fresh_preparation: Option<Digest32>,
    commit_attempted: bool,
    remaining_bytes: u64,
}
struct Reply {
    observation: Option<DigObservation>,
    effect: Option<DigEffect>,
    replayed: Option<bool>,
}
impl<S: DigStream> DigRpcClient<S> {
    pub fn negotiate(
        mut stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        region: DigRegion,
        context: &OperationContext,
    ) -> Result<Self> {
        context.authorize(
            Capability::Query,
            RiskTier::ReadOnly,
            &[],
            Some(region.halo()),
        )?;
        require(
            (32..=256).contains(&token.len())
                && (16..=64).contains(&nonce.len())
                && context.budget.max_bytes >= CONNECT_BYTES
                && region.halo_count() as u32 <= context.budget.max_entities,
            "dig credentials or bootstrap allowance refused",
        )?;
        stream.narrow_deadline(allowance(context)?)?;
        stream
            .write_all(b"DFHack?\n\x01\0\0\0")
            .and_then(|_| stream.flush())
            .map_err(io_error)?;
        let mut greeting = [0; 12];
        stream.read_exact(&mut greeting).map_err(io_error)?;
        require(
            &greeting == b"DFHack!\n\x01\0\0\0",
            "invalid DFHack greeting",
        )?;
        let mut methods = [0; 6];
        for (i, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            bytes(&mut request, 2, b"dfmcp.dig.v1_16.Request");
            bytes(&mut request, 3, b"dfmcp.dig.v1_16.Reply");
            bytes(&mut request, 4, b"dfmcp_dig_v1_16");
            let payload = frame(&mut stream, 0, &request)?;
            let m = Message::parse(&payload, 1)?;
            let id = m.number(1)?;
            require(
                m.count() == 1 && (2..=32767).contains(&id) && !methods.contains(&(id as i16)),
                "invalid or aliased dig method binding",
            )?;
            methods[i] = id as i16;
        }
        let mut client = Self {
            stream,
            token,
            nonce,
            methods,
            manifest: DigManifest {
                generation: 0,
                df_version: String::new(),
                dfhack_version: String::new(),
            },
            region,
            endpoint: None,
            fenced: false,
            fresh_preparation: None,
            commit_attempted: false,
            remaining_bytes: context.budget.max_bytes - CONNECT_BYTES,
        };
        client.invoke(0, None, None, context)?;
        Ok(client)
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    fn invoke(
        &mut self,
        op: usize,
        plan: Option<&DigPlan>,
        region: Option<DigRegion>,
        context: &OperationContext,
    ) -> Result<Reply> {
        require(
            !self.fenced,
            "dig stream is fenced; recovery requires an explicit new connection",
        )?;
        let region = match (plan, region) {
            (Some(plan), _) => plan.before().region(),
            (None, Some(region)) => region,
            (None, None) => self.region,
        };
        require(
            region == self.region,
            "dig connection is pinned to a different region",
        )?;
        require(
            region.halo_count() as u32 <= context.budget.max_entities,
            "dig halo exceeds entity allowance",
        )?;
        authorize(
            context,
            plan.map_or(context.anchor.fortress_id, |p| p.before().fortress_id()),
            plan.map_or(context.anchor.tick.get(), |p| p.before().tick()),
            region,
            op == 1,
            matches!(op, 2 | 3 | 5),
            op == 2,
        )?;
        if let Some(p) = plan {
            require(
                p.before().generation() == self.manifest.generation,
                "dig source incarnation changed",
            )?;
        }
        // The initial connection reservation already covers all bindings and
        // handshake. Every subsequent attempt spends its whole worst-case frame
        // allowance before dispatch, even when the reply is lost or malformed.
        if op != 0 {
            if self.remaining_bytes < RPC_BYTES || context.budget.max_bytes < RPC_BYTES {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "dig connection byte allowance exhausted",
                ));
            }
            self.remaining_bytes -= RPC_BYTES;
        }
        let mut request = Vec::new();
        bytes(&mut request, 1, &self.token);
        bytes(&mut request, 2, &self.nonce);
        number(&mut request, 3, 1);
        number(&mut request, 4, 16);
        if matches!(op, 1 | 2) {
            for (i, n) in region.coordinates().iter().enumerate() {
                number(&mut request, 5 + i as u64, u64::from(*n));
            }
        }
        if let Some(p) = plan {
            bytes(&mut request, 11, p.key().as_bytes());
            bytes(&mut request, 13, p.digest().as_bytes());
            if op == 2 {
                number(&mut request, 10, u64::from(p.allow_hidden_neighbors()));
                bytes(&mut request, 12, p.before().witness().as_bytes());
            } else if matches!(op, 3 | 5) {
                bytes(&mut request, 14, p.token());
            }
        }
        let result = (|| {
            self.stream.narrow_deadline(allowance(context)?)?;
            let raw = frame(&mut self.stream, self.methods[op], &request)?;
            let m = Message::parse(&raw, 11)?;
            require(
                (1..=8).all(|n| m.has(n))
                    && m.bytes(3)? == self.nonce
                    && m.number(4)? == 1
                    && m.number(5)? == 16,
                "dig nonce/profile/base fields mismatch",
            )?;
            let accepted = m.number(1)?;
            let code = m.number(2)?;
            require(accepted <= 1 && code <= 8, "invalid dig acceptance code")?;
            if accepted == 0 {
                require(
                    code != 0
                        && m.count() == 8
                        && m.number(6)? == 0
                        && m.bytes(7)?.is_empty()
                        && m.bytes(8)?.is_empty(),
                    "noncanonical dig refusal",
                )?;
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "native dig refusal; no effect outcome inferred",
                )
                .retryable(false));
            }
            let source = DigManifest {
                generation: m.number(6)?,
                df_version: version(m.bytes(7)?)?,
                dfhack_version: version(m.bytes(8)?)?,
            };
            require(
                code == 0
                    && source.generation > 0
                    && source.generation < u64::MAX
                    && (self.manifest.generation == 0 || self.manifest == source),
                "dig source/software drift",
            )?;
            require(
                m.has(9) == (op == 1)
                    && m.has(11) == (op == 2)
                    && (op == 4 || m.has(10) == matches!(op, 2 | 3 | 5)),
                "unexpected dig reply payload",
            )?;
            let observation = if op == 1 {
                let observed = DigObservation::decode(m.bytes(9)?)?;
                require(
                    observed.region() == region && observed.generation() == source.generation,
                    "dig capture selection/source mismatch",
                )?;
                authorize(
                    context,
                    observed.fortress_id(),
                    observed.tick(),
                    observed.region(),
                    true,
                    false,
                    false,
                )?;
                Some(observed)
            } else {
                None
            };
            let effect = if m.has(10) {
                let plan =
                    plan.ok_or_else(|| error(ErrorCode::AdapterRejected, "unexpected dig effect"))?;
                let effect = DigEffect::decode(m.bytes(10)?, plan)?;
                require(
                    !matches!(op, 3 | 5) || effect.phase() != DigPhase::Prepared,
                    "dig operation did not retire preparation",
                )?;
                Some(effect)
            } else {
                None
            };
            let replayed = if op == 2 {
                let value = m.number(11)?;
                require(
                    value <= 1
                        && (value == 1
                            || effect
                                .as_ref()
                                .is_some_and(|e| e.phase() == DigPhase::Prepared)),
                    "fresh dig preparation has historical outcome",
                )?;
                Some(value == 1)
            } else {
                None
            };
            self.stream.narrow_deadline(allowance(context)?)?;
            self.manifest = source;
            Ok(Reply {
                observation,
                effect,
                replayed,
            })
        })();
        if result.is_err() {
            self.fenced = true;
            self.fresh_preparation = None;
        }
        result
    }
}
impl DigRpcClient<DigTcpStream> {
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        region: DigRegion,
        context: &OperationContext,
    ) -> Result<Self> {
        context.authorize(
            Capability::Query,
            RiskTier::ReadOnly,
            &[],
            Some(region.halo()),
        )?;
        require(
            endpoint.ip().is_loopback()
                && endpoint.port() != 0
                && context.budget.max_bytes >= CONNECT_BYTES
                && (32..=256).contains(&token.len())
                && (16..=64).contains(&nonce.len())
                && region.halo_count() as u32 <= context.budget.max_entities,
            "invalid dig connection",
        )?;
        let duration = allowance(context)?;
        let deadline = Instant::now()
            .checked_add(duration)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "dig deadline overflow"))?;
        let stream = TcpStream::connect_timeout(&endpoint, duration).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        let mut client = Self::negotiate(
            DigTcpStream { stream, deadline },
            token,
            nonce,
            region,
            context,
        )?;
        client.endpoint = Some(endpoint);
        Ok(client)
    }
}
impl<S: DigStream> DigSource for DigRpcClient<S> {
    fn manifest(&self) -> &DigManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.endpoint
    }
    fn observe(&mut self, region: DigRegion, context: &OperationContext) -> Result<DigObservation> {
        self.invoke(1, None, Some(region), context)?
            .observation
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing dig capture"))
    }
    fn prepare(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<DigPreparation> {
        require(
            self.fresh_preparation.is_none() && !self.commit_attempted,
            "dig client already owns or dispatched a preparation",
        )?;
        let reply = self.invoke(2, Some(plan), None, context)?;
        let effect = reply
            .effect
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing dig preparation"))?;
        let replayed = reply
            .replayed
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing dig replay flag"))?;
        if !replayed {
            self.fresh_preparation = Some(Digest32::of_bytes(&plan.canonical_bytes()));
        }
        Ok(DigPreparation { effect, replayed })
    }
    fn commit(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<DigEffect> {
        require(
            !self.commit_attempted
                && self.fresh_preparation == Some(Digest32::of_bytes(&plan.canonical_bytes())),
            "dig commit requires the same connection's fresh preparation; never replay",
        )?;
        self.commit_attempted = true;
        self.fresh_preparation = None;
        self.invoke(3, Some(plan), None, context)?
            .effect
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing dig commit evidence"))
    }
    fn query(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<Option<DigEffect>> {
        Ok(self.invoke(4, Some(plan), None, context)?.effect)
    }
    fn cancel(&mut self, plan: &DigPlan, context: &OperationContext) -> Result<DigEffect> {
        if self.fresh_preparation == Some(Digest32::of_bytes(&plan.canonical_bytes())) {
            self.fresh_preparation = None;
        }
        self.invoke(5, Some(plan), None, context)?
            .effect
            .ok_or_else(|| {
                error(
                    ErrorCode::AdapterRejected,
                    "missing dig cancellation evidence",
                )
            })
    }
}

#[cfg(test)]
mod tests;
