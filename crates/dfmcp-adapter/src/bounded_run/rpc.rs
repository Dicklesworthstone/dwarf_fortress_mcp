//! Fixed native run/1.13 transport. No implicit reconnect, retry or deadline renewal.
use super::{RunObservation, RunPhase, RunPlan, RunRecord, error, require};
use dfmcp_core::{Capability, ErrorCode, FortressId, GameTick, OperationContext, Result, RiskTier};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub const METHODS: [&str; 6] = [
    "Handshake",
    "ObserveRun",
    "PrepareRun",
    "CommitRun",
    "QueryRun",
    "CancelRun",
];
const MAX_PAYLOAD: usize = 2048;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunManifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}
impl RunManifest {
    pub fn validate(&self) -> Result<()> {
        require(
            self.generation > 0 && self.generation < u64::MAX,
            "invalid run manifest generation",
        )?;
        for value in [&self.df_version, &self.dfhack_version] {
            require(
                !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control),
                "invalid run software identity",
            )?;
        }
        Ok(())
    }
    pub fn same_software(&self, other: &Self) -> bool {
        self.df_version == other.df_version && self.dfhack_version == other.dfhack_version
    }
}
/// Injected native boundary. The coordinator, not this low-level interface, owns
/// durable dispatch ordering. No trait method may perform hidden retries.
pub trait RunSource {
    fn manifest(&self) -> &RunManifest;
    fn endpoint(&self) -> Option<SocketAddr>;
    fn observe(&mut self, context: &OperationContext) -> Result<RunObservation>;
    fn prepare(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<RunRecord>;
    fn commit(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<RunRecord>;
    fn query(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<Option<RunRecord>>;
    fn cancel(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<RunRecord>;
}

pub(crate) fn authorize(context: &OperationContext, control: bool) -> Result<()> {
    // Native run/1.13 has no fortress folder/site. Never invent that binding from
    // a user supplied ID or silently accept a grant scoped to a real fortress.
    if context.anchor.fortress_id != FortressId::NIL {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "run/1.13 authority is source-bound, not fortress-lineage authority",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if control {
        context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}
pub(crate) fn authorize_plan(context: &OperationContext, plan: &RunPlan) -> Result<()> {
    authorize(context, true)?;
    if u64::from(plan.spec().game_ticks()) > context.budget.max_game_ticks {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "run exceeds the caller's game-tick allowance",
        ));
    }
    let tick = plan
        .before()
        .tick()
        .ok_or_else(|| error(ErrorCode::StaleAnchor, "run has no observed game tick"))?;
    let end = tick
        .checked_add(u64::from(plan.spec().game_ticks()))
        .filter(|value| *value <= super::MAX_NATIVE_TICK)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "run horizon exceeds the native clock domain",
            )
        })?;
    let mut horizon = context.clone();
    horizon.anchor.tick = GameTick(end.max(context.anchor.tick.get()));
    horizon.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
}
fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        "bounded-run I/O failed or exceeded its deadline",
    )
}
fn deadline(context: &OperationContext) -> Result<Instant> {
    authorize(context, false)?;
    if !(1..=60_000).contains(&context.budget.max_wall_millis) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "run RPC deadline must be 1..60000 milliseconds",
        ));
    }
    Instant::now()
        .checked_add(Duration::from_millis(context.budget.max_wall_millis))
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "run RPC deadline overflow"))
}
fn remaining(end: Instant) -> Result<Duration> {
    end.checked_duration_since(Instant::now())
        .filter(|d| *d >= Duration::from_millis(1))
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "run RPC deadline exhausted"))
}
fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn number(out: &mut Vec<u8>, field: u32, n: u64) {
    varint(out, u64::from(field) << 3);
    varint(out, n);
}
fn bytes(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    varint(out, (u64::from(field) << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn read_varint(raw: &[u8], offset: &mut usize) -> Result<u64> {
    let mut n = 0;
    for index in 0..10 {
        let b = *raw
            .get(*offset)
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "truncated run protobuf"))?;
        *offset += 1;
        require(index != 9 || b <= 1, "run protobuf overflow")?;
        n |= u64::from(b & 127) << (index * 7);
        if b < 128 {
            require(index == 0 || b != 0, "noncanonical run protobuf")?;
            return Ok(n);
        }
    }
    Err(error(
        ErrorCode::AdapterRejected,
        "unterminated run protobuf",
    ))
}
#[derive(Clone, Copy)]
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}
struct Message<'a>(BTreeMap<u32, Field<'a>>);
impl<'a> Message<'a> {
    fn parse(raw: &'a [u8], maximum: u32) -> Result<Self> {
        require(raw.len() <= MAX_PAYLOAD, "run RPC exceeds 2 KiB")?;
        let mut fields = BTreeMap::new();
        let mut offset = 0;
        while offset < raw.len() {
            let tag = read_varint(raw, &mut offset)?;
            let field = u32::try_from(tag >> 3)
                .map_err(|_| error(ErrorCode::AdapterRejected, "run protobuf field overflow"))?;
            require(
                field > 0 && field <= maximum && !fields.contains_key(&field),
                "unknown or duplicate run field",
            )?;
            let value = match tag & 7 {
                0 => Field::Number(read_varint(raw, &mut offset)?),
                2 => {
                    let length = usize::try_from(read_varint(raw, &mut offset)?).map_err(|_| {
                        error(ErrorCode::AdapterRejected, "run byte length overflow")
                    })?;
                    require(length <= raw.len() - offset, "truncated run byte field")?;
                    let value = &raw[offset..offset + length];
                    offset += length;
                    Field::Bytes(value)
                }
                _ => {
                    return Err(error(
                        ErrorCode::AdapterRejected,
                        "unsupported run wire type",
                    ));
                }
            };
            fields.insert(field, value);
        }
        Ok(Self(fields))
    }
    fn has(&self, id: u32) -> bool {
        self.0.contains_key(&id)
    }
    fn number(&self, id: u32) -> Result<u64> {
        match self.0.get(&id) {
            Some(Field::Number(n)) => Ok(*n),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "missing or mistyped run number",
            )),
        }
    }
    fn boolean(&self, id: u32) -> Result<bool> {
        match self.number(id)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "invalid run protobuf boolean",
            )),
        }
    }
    fn bytes(&self, id: u32) -> Result<&'a [u8]> {
        match self.0.get(&id) {
            Some(Field::Bytes(b)) => Ok(b),
            _ => Err(error(
                ErrorCode::AdapterRejected,
                "missing or mistyped run bytes",
            )),
        }
    }
    fn text(&self, id: u32) -> Result<String> {
        let value = std::str::from_utf8(self.bytes(id)?)
            .map_err(|_| error(ErrorCode::AdapterRejected, "invalid run software UTF-8"))?;
        require(
            !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control),
            "invalid run software field",
        )?;
        Ok(value.to_owned())
    }
}

/// Actual TCP reads and writes share a single absolute deadline, including
/// connect/handshake/binding. Partial reads never renew it.
pub struct RunTcpStream {
    stream: TcpStream,
    deadline: Instant,
}
impl Read for RunTcpStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let timeout = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "run deadline"))?;
        self.stream.set_read_timeout(Some(timeout))?;
        self.stream.read(out)
    }
}
impl Write for RunTcpStream {
    fn write(&mut self, out: &[u8]) -> io::Result<usize> {
        let timeout = self
            .deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "run deadline"))?;
        self.stream.set_write_timeout(Some(timeout))?;
        self.stream.write(out)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

pub struct RunRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    methods: [i16; 6],
    manifest: RunManifest,
    deadline: Instant,
    bytes_left: u64,
    call_bytes_left: u64,
    fenced: bool,
    endpoint: Option<SocketAddr>,
}
struct Reply {
    observation: Option<RunObservation>,
    record: Option<RunRecord>,
}
impl<S: Read + Write> RunRpcClient<S> {
    pub fn negotiate(
        stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        context: &OperationContext,
    ) -> Result<Self> {
        Self::negotiate_until(stream, token, nonce, context, deadline(context)?, None)
    }
    fn negotiate_until(
        stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        context: &OperationContext,
        end: Instant,
        endpoint: Option<SocketAddr>,
    ) -> Result<Self> {
        require(
            (32..=256).contains(&token.len()) && (16..=64).contains(&nonce.len()),
            "invalid run credentials",
        )?;
        let mut client = Self {
            stream,
            token,
            nonce,
            methods: [0; 6],
            manifest: RunManifest {
                generation: 0,
                df_version: String::new(),
                dfhack_version: String::new(),
            },
            deadline: end,
            bytes_left: context.budget.max_bytes,
            call_bytes_left: context.budget.max_bytes,
            fenced: false,
            endpoint,
        };
        client.charge(24)?;
        client
            .stream
            .write_all(b"DFHack?\n\x01\0\0\0")
            .map_err(io_error)?;
        client.stream.flush().map_err(io_error)?;
        let mut hello = [0; 12];
        client.stream.read_exact(&mut hello).map_err(io_error)?;
        require(
            &hello == b"DFHack!\n\x01\0\0\0",
            "invalid native run handshake",
        )?;
        for (index, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            for (field, value) in [
                (1, *name),
                (2, "dfmcp.run.v1_13.Request"),
                (3, "dfmcp.run.v1_13.Reply"),
                (4, "dfmcp_run_v1_13"),
            ] {
                bytes(&mut request, field, value.as_bytes());
            }
            let raw = client.frame(0, &request)?;
            let reply = Message::parse(&raw, 1)?;
            let id = i16::try_from(reply.number(1)?)
                .map_err(|_| error(ErrorCode::AdapterRejected, "run binding overflow"))?;
            require(
                id >= 2 && !client.methods[..index].contains(&id),
                "run binding aliases a core or prior method",
            )?;
            client.methods[index] = id;
        }
        client.invoke(0, None, context)?;
        Ok(client)
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    fn charge(&mut self, n: usize) -> Result<()> {
        remaining(self.deadline)?;
        self.call_bytes_left = self.call_bytes_left.checked_sub(n as u64).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "run RPC call byte budget exhausted",
            )
        })?;
        self.bytes_left = self
            .bytes_left
            .checked_sub(n as u64)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "run RPC byte budget exhausted"))?;
        Ok(())
    }
    fn frame(&mut self, id: i16, request: &[u8]) -> Result<Vec<u8>> {
        require(request.len() <= MAX_PAYLOAD, "run request exceeds 2 KiB")?;
        self.charge(8 + request.len())?;
        let mut head = [0u8; 8];
        head[..2].copy_from_slice(&id.to_le_bytes());
        head[4..].copy_from_slice(&(request.len() as i32).to_le_bytes());
        self.stream.write_all(&head).map_err(io_error)?;
        self.stream.write_all(request).map_err(io_error)?;
        self.stream.flush().map_err(io_error)?;
        let mut notifications = 0;
        let mut notification_bytes = 0;
        loop {
            self.charge(8)?;
            self.stream.read_exact(&mut head).map_err(io_error)?;
            let kind = i16::from_le_bytes([head[0], head[1]]);
            let length = i32::from_le_bytes([head[4], head[5], head[6], head[7]]);
            require(
                matches!(kind, -1 | -3) && length >= 0,
                "native run frame refused",
            )?;
            let length = length as usize;
            require(
                length <= if kind == -1 { MAX_PAYLOAD } else { 65_536 },
                "native run frame exceeds bound",
            )?;
            if kind == -3 {
                notifications += 1;
                notification_bytes += length;
                require(
                    notifications <= 8 && notification_bytes <= 262_144,
                    "run notification budget exceeded",
                )?;
            }
            self.charge(length)?;
            let mut payload = vec![0; length];
            self.stream.read_exact(&mut payload).map_err(io_error)?;
            if kind == -1 {
                return Ok(payload);
            }
        }
    }
    fn invoke(
        &mut self,
        operation: usize,
        plan: Option<&RunPlan>,
        context: &OperationContext,
    ) -> Result<Reply> {
        authorize(context, matches!(operation, 2 | 3 | 5))?;
        if let Some(plan) = plan {
            if matches!(operation, 2 | 3) {
                authorize_plan(context, plan)?;
            }
        }
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "run connection fenced; reopen for query/cancel recovery",
            ));
        }
        self.call_bytes_left = self.bytes_left.min(context.budget.max_bytes);
        let outcome = (|| {
            let mut request = Vec::new();
            bytes(&mut request, 1, &self.token);
            bytes(&mut request, 2, &self.nonce);
            number(&mut request, 3, 1);
            number(&mut request, 4, 13);
            if let Some(plan) = plan {
                bytes(&mut request, 5, plan.key().as_bytes());
                bytes(&mut request, 9, plan.digest().as_bytes());
                if operation == 2 {
                    number(&mut request, 6, u64::from(plan.spec().game_ticks()));
                    number(&mut request, 7, u64::from(plan.spec().wall_ms()));
                    bytes(&mut request, 8, plan.before().canonical_bytes());
                }
                if matches!(operation, 3 | 5) {
                    bytes(&mut request, 10, plan.token());
                }
            }
            let raw = self.frame(self.methods[operation], &request)?;
            self.decode(&raw, operation, plan)
        })();
        if outcome.is_err() {
            self.fenced = true;
        }
        outcome
    }
    fn decode(&mut self, raw: &[u8], operation: usize, plan: Option<&RunPlan>) -> Result<Reply> {
        let m = Message::parse(raw, 12)?;
        require(
            (1..=8).all(|id| m.has(id)),
            "required run reply fields missing",
        )?;
        require(
            m.bytes(3)? == self.nonce && m.number(4)? == 1 && m.number(5)? == 13,
            "run nonce/profile mismatch",
        )?;
        let accepted = m.boolean(1)?;
        let code = m.number(2)?;
        if !accepted {
            require(
                (1..=8).contains(&code) && m.0.len() == 8,
                "malformed run refusal",
            )?;
            return Err(error(
                match code {
                    1 => ErrorCode::CapabilityDenied,
                    2 => ErrorCode::VersionMismatch,
                    3 => ErrorCode::InvalidRequest,
                    4 => ErrorCode::FortressNotLoaded,
                    6 => ErrorCode::StaleAnchor,
                    7 | 8 => ErrorCode::Conflict,
                    _ => ErrorCode::AdapterFailure,
                },
                "native run request refused; no effect outcome inferred",
            ));
        }
        require(code == 0, "run success carries failure code")?;
        let next = RunManifest {
            generation: m.number(6)?,
            df_version: m.text(7)?,
            dfhack_version: m.text(8)?,
        };
        next.validate()?;
        require(
            self.manifest.generation == 0
                || (next.generation >= self.manifest.generation
                    && next.same_software(&self.manifest)),
            "run software changed or generation regressed",
        )?;
        let active = m.boolean(11)?;
        require(m.number(12)? <= 256, "invalid retained run count")?;
        require(
            m.has(9) == (operation == 1),
            "unexpected or missing run observation",
        )?;
        require(
            operation == 4 || m.has(10) == matches!(operation, 2 | 3 | 5),
            "unexpected or missing run record",
        )?;
        let observation = if m.has(9) {
            let observation = RunObservation::decode(m.bytes(9)?)?;
            require(
                observation.generation() == next.generation,
                "run observation source mismatch",
            )?;
            Some(observation)
        } else {
            None
        };
        let record = if m.has(10) {
            let record = RunRecord::decode(m.bytes(10)?)?;
            require(
                plan.is_some_and(|p| record.plan() == p),
                "run reply belongs to another sealed intent",
            )?;
            require(
                record.plan().before().generation() <= next.generation,
                "run record comes from a future generation",
            )?;
            if matches!(record.phase(), RunPhase::Running | RunPhase::Stopping) {
                require(
                    active && record.plan().before().generation() == next.generation,
                    "active run lacks matching native owner",
                )?;
            }
            Some(record)
        } else {
            None
        };
        self.manifest = next;
        Ok(Reply {
            observation,
            record,
        })
    }
    fn effect(
        &mut self,
        operation: usize,
        plan: &RunPlan,
        context: &OperationContext,
    ) -> Result<RunRecord> {
        self.invoke(operation, Some(plan), context)?
            .record
            .ok_or_else(|| {
                error(
                    ErrorCode::AdapterRejected,
                    "missing native run effect record",
                )
            })
    }
}
impl RunRpcClient<RunTcpStream> {
    /// Narrow an existing connection and its actual socket to the remaining
    /// foreground context. This can never renew the original deadline.
    pub fn restrict_deadline(&mut self, context: &OperationContext) -> Result<()> {
        self.deadline = self.deadline.min(deadline(context)?);
        self.stream.deadline = self.stream.deadline.min(self.deadline);
        remaining(self.deadline).map(|_| ())
    }
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        context: &OperationContext,
    ) -> Result<Self> {
        let end = deadline(context)?;
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "unencrypted native run transport is numeric-loopback-only",
            ));
        }
        require(
            (32..=256).contains(&token.len()) && (16..=64).contains(&nonce.len()),
            "invalid run credentials",
        )?;
        let stream = TcpStream::connect_timeout(&endpoint, remaining(end)?).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        Self::negotiate_until(
            RunTcpStream {
                stream,
                deadline: end,
            },
            token,
            nonce,
            context,
            end,
            Some(endpoint),
        )
    }
}
impl<S: Read + Write> RunSource for RunRpcClient<S> {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.endpoint
    }
    fn observe(&mut self, context: &OperationContext) -> Result<RunObservation> {
        self.invoke(1, None, context)?
            .observation
            .ok_or_else(|| error(ErrorCode::AdapterRejected, "missing run observation"))
    }
    fn prepare(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<RunRecord> {
        self.effect(2, plan, context)
    }
    fn commit(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<RunRecord> {
        self.effect(3, plan, context)
    }
    fn query(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<Option<RunRecord>> {
        Ok(self.invoke(4, Some(plan), context)?.record)
    }
    fn cancel(&mut self, plan: &RunPlan, context: &OperationContext) -> Result<RunRecord> {
        self.effect(5, plan, context)
    }
}

#[cfg(test)]
mod tests;
