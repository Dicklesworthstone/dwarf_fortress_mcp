//! Fixed workforce/1.17 RPC. One foreground connection; failed streams cannot be reused.
use super::{
    AssignmentEffect, AssignmentPhase, AssignmentPlan, MAX_CAPTURE, MAX_EFFECT, WorkforceCapture,
    error, require, validate_ids,
};
use dfmcp_core::{Capability, ErrorCode, FortressId, GameTick, OperationContext, Result, RiskTier};
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub const MAX_REPLY: usize = 128 * 1024;
pub const RPC_BYTES: u64 = 400 * 1024;
pub const CONNECT_BYTES: u64 = 7 * RPC_BYTES + 24;
const METHODS: [&str; 6] = [
    "Handshake",
    "ObserveWorkforce",
    "PrepareAssignment",
    "CommitAssignment",
    "QueryAssignment",
    "CancelAssignment",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforceManifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}
pub trait WorkforceSource {
    fn manifest(&self) -> &WorkforceManifest;
    fn endpoint(&self) -> Option<SocketAddr>;
    fn observe(&mut self, ids: &[u32], context: &OperationContext) -> Result<WorkforceCapture>;
    fn prepare(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect>;
    fn commit(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect>;
    fn query(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<Option<AssignmentEffect>>;
    fn cancel(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect>;
}
/// Current grants, never restored journal bytes. Limited-use grants are rejected
/// by authorize() rather than copied into a replayable consumption counter.
pub fn authorize(
    context: &OperationContext,
    fortress: FortressId,
    tick: u64,
    write: bool,
) -> Result<()> {
    if context.anchor.fortress_id != fortress {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "workforce authority belongs to another fortress",
        ));
    }
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(context.anchor.tick.get()));
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if write {
        current.authorize(Capability::ConfigureLabor, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}
fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        "workforce RPC failed; a failed commit is not proof of nonapplication",
    )
}
fn timeout(context: &OperationContext) -> Result<Duration> {
    context.budget.validate()?;
    require(
        context.budget.max_wall_millis <= 60_000 && context.budget.max_bytes >= RPC_BYTES,
        "workforce RPC exceeds deadline or byte allowance",
    )?;
    Ok(Duration::from_millis(context.budget.max_wall_millis))
}
pub trait WorkforceStream: Read + Write {
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()>;
}
pub struct WorkforceTcpStream {
    stream: TcpStream,
    deadline: Instant,
}
impl WorkforceTcpStream {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "workforce deadline"))
    }
}
impl WorkforceStream for WorkforceTcpStream {
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()> {
        let candidate = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "workforce deadline overflow"))?;
        self.deadline = self.deadline.min(candidate);
        self.remaining().map_err(io_error)?;
        Ok(())
    }
}
impl Read for WorkforceTcpStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(out)
    }
}
impl Write for WorkforceTcpStream {
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
struct Message<'a>([Option<Field<'a>>; 13]);
impl<'a> Message<'a> {
    fn parse(raw: &'a [u8], max: usize) -> Result<Self> {
        require(raw.len() <= MAX_REPLY && max <= 12, "workforce reply bound")?;
        let mut r = crate::bounded_run::Reader(raw);
        let mut out = Self([None; 13]);
        fn take(r: &mut crate::bounded_run::Reader<'_>) -> Result<u64> {
            let mut value = 0u64;
            for i in 0..10 {
                let b = r.byte()?;
                require(i < 9 || b <= 1, "protobuf integer overflow")?;
                value |= u64::from(b & 127) << (i * 7);
                if b < 128 {
                    require(i == 0 || b != 0, "overlong protobuf integer")?;
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
            require(tag >> 3 <= max as u64, "protobuf field number overflow")?;
            let n = (tag >> 3) as usize;
            require(
                n > 0 && n <= max && out.0[n].is_none(),
                "unknown or duplicate reply field",
            )?;
            let field = match tag & 7 {
                0 => Field::Number(take(&mut r)?),
                2 => {
                    let width = take(&mut r)?;
                    require(width <= r.0.len() as u64, "truncated protobuf bytes")?;
                    Field::Bytes(r.take(width as usize)?)
                }
                _ => {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "unsupported protobuf wire type",
                    ));
                }
            };
            out.0[n] = Some(field);
        }
        Ok(out)
    }
    fn num(&self, n: usize) -> Result<u64> {
        match self.0[n] {
            Some(Field::Number(v)) => Ok(v),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "missing/wrong numeric field",
            )),
        }
    }
    fn data(&self, n: usize) -> Result<&'a [u8]> {
        match self.0[n] {
            Some(Field::Bytes(v)) => Ok(v),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "missing/wrong byte field",
            )),
        }
    }
    fn has(&self, n: usize) -> bool {
        self.0[n].is_some()
    }
    fn count(&self) -> usize {
        self.0.iter().filter(|f| f.is_some()).count()
    }
}
fn version(raw: &[u8]) -> Result<String> {
    let value = std::str::from_utf8(raw)
        .map_err(|_| error(ErrorCode::AdapterRejected, "invalid version UTF-8"))?;
    require(
        !value.is_empty() && value.len() <= 128 && !value.contains('\0'),
        "invalid version length",
    )?;
    Ok(value.to_owned())
}
fn frame<S: Read + Write>(stream: &mut S, method: i16, request: &[u8]) -> Result<Vec<u8>> {
    require(request.len() <= 2048, "workforce request exceeds 2 KiB")?;
    let mut header = [0u8; 8];
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
        let tag = i16::from_le_bytes([header[0], header[1]]);
        let n = i32::from_le_bytes([header[4], header[5], header[6], header[7]]);
        require(
            matches!(tag, -1 | -3)
                && n >= 0
                && n as usize <= if tag == -1 { MAX_REPLY } else { 65_536 },
            "native frame rejected",
        )?;
        if tag == -3 {
            count += 1;
            total += n;
            require(
                count <= 8 && total <= 262_144,
                "notification allowance exhausted",
            )?;
        }
        let mut payload = vec![0; n as usize];
        stream.read_exact(&mut payload).map_err(io_error)?;
        if tag == -1 {
            return Ok(payload);
        }
    }
}
pub struct WorkforceRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    methods: [i16; 6],
    manifest: WorkforceManifest,
    endpoint: Option<SocketAddr>,
    fenced: bool,
}
impl<S: WorkforceStream> WorkforceRpcClient<S> {
    pub fn negotiate(
        mut stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        context: &OperationContext,
    ) -> Result<Self> {
        authorize(
            context,
            context.anchor.fortress_id,
            context.anchor.tick.get(),
            false,
        )?;
        require(
            (32..=256).contains(&token.len())
                && (16..=64).contains(&nonce.len())
                && context.budget.max_bytes >= CONNECT_BYTES,
            "workforce credentials or bootstrap allowance refused",
        )?;
        stream.narrow_deadline(timeout(context)?)?;
        stream
            .write_all(b"DFHack?\n\x01\0\0\0")
            .and_then(|_| stream.flush())
            .map_err(io_error)?;
        let mut header = [0; 12];
        stream.read_exact(&mut header).map_err(io_error)?;
        require(
            &header == b"DFHack!\n\x01\0\0\0",
            "DFHack handshake mismatch",
        )?;
        let mut methods = [0i16; 6];
        for (i, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            bytes(&mut request, 1, name.as_bytes());
            bytes(&mut request, 2, b"dfmcp.workforce.v1_17.Request");
            bytes(&mut request, 3, b"dfmcp.workforce.v1_17.Reply");
            bytes(&mut request, 4, b"dfmcp_workforce_v1_17");
            let payload = frame(&mut stream, 0, &request)?;
            let binding = Message::parse(&payload, 1)?;
            let id = binding.num(1)?;
            require(
                binding.count() == 1
                    && (2..=32767).contains(&id)
                    && !methods.contains(&(id as i16)),
                "invalid/aliased binding",
            )?;
            methods[i] = id as i16;
        }
        let mut client = Self {
            stream,
            token,
            nonce,
            methods,
            manifest: WorkforceManifest {
                generation: 0,
                df_version: String::new(),
                dfhack_version: String::new(),
            },
            endpoint: None,
            fenced: false,
        };
        client.invoke(0, None, &[], context)?;
        Ok(client)
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    fn invoke(
        &mut self,
        op: usize,
        plan: Option<&AssignmentPlan>,
        ids: &[u32],
        context: &OperationContext,
    ) -> Result<(Option<WorkforceCapture>, Option<AssignmentEffect>)> {
        require(
            !self.fenced,
            "workforce stream is fenced; reconcile on an explicit new connection",
        )?;
        authorize(
            context,
            plan.map_or(context.anchor.fortress_id, |p| p.before().fortress_id()),
            plan.map_or(context.anchor.tick.get(), |p| p.before().tick()),
            matches!(op, 2 | 3 | 5),
        )?;
        if op == 2 {
            let mut current = context.clone();
            current.anchor.tick = GameTick(
                plan.map_or(context.anchor.tick.get(), |p| p.before().tick())
                    .max(context.anchor.tick.get()),
            );
            current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        }
        if op == 1 {
            validate_ids(ids)?;
        }
        if let Some(p) = plan {
            require(
                p.before().generation() == self.manifest.generation,
                "workforce source changed",
            )?;
        }
        let mut request = Vec::new();
        bytes(&mut request, 1, &self.token);
        bytes(&mut request, 2, &self.nonce);
        number(&mut request, 3, 1);
        number(&mut request, 4, 17);
        if let Some(p) = plan {
            bytes(&mut request, 5, p.key().as_bytes());
            bytes(&mut request, 9, p.digest().as_bytes());
            if op == 2 {
                number(&mut request, 6, u64::from(p.spec().detail()));
                number(&mut request, 7, u64::from(p.spec().assigned()));
                bytes(&mut request, 8, p.before().witness().as_bytes());
                for id in p.before().ids() {
                    number(&mut request, 11, u64::from(id));
                }
            } else if matches!(op, 3 | 5) {
                bytes(&mut request, 10, p.token());
            }
        } else if op == 1 {
            for id in ids {
                number(&mut request, 11, u64::from(*id));
            }
        }
        self.stream.narrow_deadline(timeout(context)?)?;
        let result = (|| {
            let raw = frame(&mut self.stream, self.methods[op], &request)?;
            let m = Message::parse(&raw, 12)?;
            require(
                m.data(3)? == self.nonce && m.num(4)? == 1 && m.num(5)? == 17,
                "workforce nonce/profile mismatch",
            )?;
            let accepted = m.num(1)?;
            let code = m.num(2)?;
            require(
                accepted <= 1 && code <= 8 && (1..=8).all(|n| m.has(n)),
                "malformed workforce reply",
            )?;
            if accepted == 0 {
                require(code != 0 && m.count() == 8, "malformed workforce refusal")?;
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "native workforce refusal; failed commit outcome remains unknown",
                ));
            }
            let manifest = WorkforceManifest {
                generation: m.num(6)?,
                df_version: version(m.data(7)?)?,
                dfhack_version: version(m.data(8)?)?,
            };
            require(
                code == 0
                    && manifest.generation > 0
                    && manifest.generation < u64::MAX
                    && (self.manifest.generation == 0 || manifest == self.manifest),
                "workforce source/software changed",
            )?;
            let unresolved = m.num(11)?;
            let retained = m.num(12)?;
            require(
                unresolved <= 1
                    && retained <= 64
                    && m.has(9) == (op == 1)
                    && (op == 4 || m.has(10) == matches!(op, 2 | 3 | 5)),
                "invalid workforce coordination fields",
            )?;
            let capture = if op == 1 {
                require(m.data(9)?.len() <= MAX_CAPTURE, "capture bound")?;
                let value = WorkforceCapture::decode(m.data(9)?)?;
                require(
                    value.generation() == manifest.generation && value.ids().as_slice() == ids,
                    "workforce selection changed",
                )?;
                authorize(context, value.fortress_id(), value.tick(), false)?;
                require(
                    value.citizens().len()
                        + value.details().len()
                        + value
                            .details()
                            .iter()
                            .map(|d| d.members().len())
                            .sum::<usize>()
                        <= context.budget.max_entities as usize,
                    "workforce entity allowance exhausted",
                )?;
                Some(value)
            } else {
                None
            };
            let effect = if m.has(10) {
                let p = plan.ok_or_else(|| {
                    error(ErrorCode::AdapterRejected, "unexpected workforce effect")
                })?;
                require(
                    m.data(10)?.len() <= MAX_EFFECT && retained > 0,
                    "invalid retained workforce effect",
                )?;
                let value = AssignmentEffect::decode(m.data(10)?, p)?;
                require(
                    value.phase() != AssignmentPhase::Unknown || unresolved == 1,
                    "unowned workforce uncertainty",
                )?;
                Some(value)
            } else {
                None
            };
            self.manifest = manifest;
            Ok((capture, effect))
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    fn effect(
        &mut self,
        op: usize,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        self.invoke(op, Some(plan), &[], context)?
            .1
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing workforce effect"))
    }
}
impl WorkforceRpcClient<WorkforceTcpStream> {
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        context: &OperationContext,
    ) -> Result<Self> {
        authorize(
            context,
            context.anchor.fortress_id,
            context.anchor.tick.get(),
            false,
        )?;
        require(
            endpoint.ip().is_loopback()
                && endpoint.port() != 0
                && context.budget.max_bytes >= CONNECT_BYTES
                && (32..=256).contains(&token.len())
                && (16..=64).contains(&nonce.len()),
            "invalid workforce connection",
        )?;
        let duration = timeout(context)?;
        let deadline = Instant::now()
            .checked_add(duration)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "workforce deadline overflow"))?;
        let stream = TcpStream::connect_timeout(&endpoint, duration).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        let mut client = Self::negotiate(
            WorkforceTcpStream { stream, deadline },
            token,
            nonce,
            context,
        )?;
        client.endpoint = Some(endpoint);
        Ok(client)
    }
}
impl<S: WorkforceStream> WorkforceSource for WorkforceRpcClient<S> {
    fn manifest(&self) -> &WorkforceManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.endpoint
    }
    fn observe(&mut self, ids: &[u32], context: &OperationContext) -> Result<WorkforceCapture> {
        self.invoke(1, None, ids, context)?
            .0
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing workforce capture"))
    }
    fn prepare(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        self.effect(2, plan, context)
    }
    fn commit(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        self.effect(3, plan, context)
    }
    fn query(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<Option<AssignmentEffect>> {
        Ok(self.invoke(4, Some(plan), &[], context)?.1)
    }
    fn cancel(
        &mut self,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentEffect> {
        self.effect(5, plan, context)
    }
}

#[cfg(test)]
mod tests;
