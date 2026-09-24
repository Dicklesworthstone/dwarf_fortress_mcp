//! Fixed native order-run/1.14 transport. No client-selected RPC, reconnect or retry.
use super::{FortressIdentity, OrderCapture, OrderRunPlan, OrderRunRecord, RunPhase, text};
use crate::bounded_run::{error, require};
use dfmcp_core::{Capability, ErrorCode, GameTick, OperationContext, Result, RiskTier};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

pub const RPC_RESERVE_BYTES: u64 = 272 * 1024;
pub const CONNECT_RESERVE_BYTES: u64 = 8 * RPC_RESERVE_BYTES;
const METHODS: [&str; 6] = [
    "Handshake",
    "ObserveRun",
    "PrepareRun",
    "CommitRun",
    "QueryRun",
    "CancelRun",
];
const MAX_PAYLOAD: usize = 4096;

pub fn authorize(
    context: &OperationContext,
    fortress: &FortressIdentity,
    clock: bool,
) -> Result<()> {
    if context.anchor.fortress_id != fortress.fortress_id() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "order-run authority belongs to another fortress",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if clock {
        context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
    }
    Ok(())
}
pub fn authorize_plan(context: &OperationContext, plan: &OrderRunPlan) -> Result<()> {
    authorize(context, plan.before().fortress(), true)?;
    context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    if u64::from(plan.spec().game_ticks()) > context.budget.max_game_ticks {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete order-run horizon exceeds game-tick allowance",
        ));
    }
    let end = plan
        .before()
        .tick()
        .checked_add(u64::from(plan.spec().game_ticks()))
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "order-run horizon overflow"))?;
    let mut horizon = context.clone();
    horizon.anchor.tick = GameTick(end.max(context.anchor.tick.get()));
    authorize(&horizon, plan.before().fortress(), true)
}
fn deadline_bound(timeout: Duration) -> Result<()> {
    if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&timeout) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "native RPC deadline must be 1..60000ms",
        ));
    }
    Ok(())
}
fn io_error(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        "order-run transport failed or deadline expired; no effect outcome inferred",
    )
}
fn malformed() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::AdapterRejected,
        "invalid order-run native framing or protobuf",
    )
}
fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 {
        out.push((n as u8 & 127) | 128);
        n >>= 7;
    }
    out.push(n as u8);
}
fn number(out: &mut Vec<u8>, tag: u32, n: u64) {
    varint(out, u64::from(tag) << 3);
    varint(out, n);
}
fn bytes(out: &mut Vec<u8>, tag: u32, data: &[u8]) {
    varint(out, (u64::from(tag) << 3) | 2);
    varint(out, data.len() as u64);
    out.extend_from_slice(data);
}
#[derive(Clone, Copy)]
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}
struct Message<'a>(BTreeMap<u32, Field<'a>>);
impl<'a> Message<'a> {
    fn parse(data: &'a [u8], maximum: u32) -> Result<Self> {
        require(
            data.len() <= MAX_PAYLOAD,
            "order-run protobuf exceeds 4 KiB",
        )?;
        let mut r = crate::bounded_run::Reader(data);
        let mut result = BTreeMap::new();
        fn read(r: &mut crate::bounded_run::Reader<'_>) -> Result<u64> {
            let mut out = 0;
            for i in 0..10 {
                let byte = r.byte()?;
                if i == 9 && byte > 1 {
                    return Err(malformed());
                }
                out |= u64::from(byte & 127) << (i * 7);
                if byte < 128 {
                    if i > 0 && byte == 0 {
                        return Err(malformed());
                    }
                    return Ok(out);
                }
            }
            Err(malformed())
        }
        while !r.0.is_empty() {
            let key = read(&mut r)?;
            let tag = u32::try_from(key >> 3).map_err(|_| malformed())?;
            if tag == 0 || tag > maximum || result.contains_key(&tag) {
                return Err(malformed());
            }
            let value = match key & 7 {
                0 => Field::Number(read(&mut r)?),
                2 => {
                    let n = usize::try_from(read(&mut r)?).map_err(|_| malformed())?;
                    Field::Bytes(r.take(n)?)
                }
                _ => return Err(malformed()),
            };
            result.insert(tag, value);
        }
        Ok(Self(result))
    }
    fn has(&self, tag: u32) -> bool {
        self.0.contains_key(&tag)
    }
    fn number(&self, tag: u32) -> Result<u64> {
        match self.0.get(&tag) {
            Some(Field::Number(n)) => Ok(*n),
            _ => Err(malformed()),
        }
    }
    fn boolean(&self, tag: u32) -> Result<bool> {
        match self.number(tag)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(malformed()),
        }
    }
    fn bytes(&self, tag: u32) -> Result<&'a [u8]> {
        match self.0.get(&tag) {
            Some(Field::Bytes(data)) => Ok(*data),
            _ => Err(malformed()),
        }
    }
}
fn header(id: i16, length: i32) -> [u8; 8] {
    let mut out = [0; 8];
    out[..2].copy_from_slice(&id.to_le_bytes());
    out[4..].copy_from_slice(&length.to_le_bytes());
    out
}
fn call<S: Read + Write>(stream: &mut S, method: i16, data: &[u8]) -> Result<Vec<u8>> {
    require(data.len() <= 2048, "native order-run request exceeds 2 KiB")?;
    stream
        .write_all(&header(method, data.len() as i32))
        .map_err(io_error)?;
    stream.write_all(data).map_err(io_error)?;
    stream.flush().map_err(io_error)?;
    let mut notification_bytes = 0;
    for index in 0..=8 {
        let mut h = [0; 8];
        stream.read_exact(&mut h).map_err(io_error)?;
        let id = i16::from_le_bytes([h[0], h[1]]);
        let n = i32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        if !matches!(id, -1 | -3) || n < 0 {
            return Err(malformed());
        }
        let n = n as usize;
        require(
            n <= if id == -1 { MAX_PAYLOAD } else { 65536 },
            "native frame exceeds profile bound",
        )?;
        if id == -3 {
            notification_bytes += n;
            require(
                index < 8 && notification_bytes <= 262144,
                "native notification budget exceeded",
            )?;
        }
        let mut payload = vec![0; n];
        stream.read_exact(&mut payload).map_err(io_error)?;
        if id == -1 {
            return Ok(payload);
        }
    }
    Err(malformed())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRunManifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}
pub trait OrderRunSource {
    fn manifest(&self) -> &OrderRunManifest;
    fn endpoint(&self) -> Option<SocketAddr>;
    fn fortress(&self) -> &FortressIdentity;
    fn fence(&mut self);
    fn observe(
        &mut self,
        id: u32,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderCapture>;
    fn prepare(
        &mut self,
        plan: &OrderRunPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord>;
    fn commit(
        &mut self,
        plan: &OrderRunPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord>;
    fn query(
        &mut self,
        plan: &OrderRunPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<Option<OrderRunRecord>>;
    fn cancel(
        &mut self,
        plan: &OrderRunPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord>;
}
pub trait OrderRunStream: Read + Write {
    /// May narrow, never extend, the connection's original absolute deadline.
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()>;
}
pub struct OrderRunRpc<S> {
    stream: S,
    secret: Vec<u8>,
    nonce: Vec<u8>,
    methods: [i16; 6],
    manifest: OrderRunManifest,
    fortress: FortressIdentity,
    endpoint: Option<SocketAddr>,
    fenced: bool,
}
struct Reply {
    capture: Option<OrderCapture>,
    record: Option<OrderRunRecord>,
}
impl<S: OrderRunStream> OrderRunRpc<S> {
    pub fn negotiate(
        mut stream: S,
        secret: Vec<u8>,
        nonce: Vec<u8>,
        fortress: FortressIdentity,
        context: &OperationContext,
    ) -> Result<Self> {
        authorize(context, &fortress, false)?;
        if context.budget.max_bytes < CONNECT_RESERVE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "order-run handshake exceeds work allowance",
            ));
        }
        require(
            (32..=256).contains(&secret.len()) && (16..=64).contains(&nonce.len()),
            "invalid order-run credentials",
        )?;
        let timeout = Duration::from_millis(context.budget.max_wall_millis);
        deadline_bound(timeout)?;
        stream.narrow_deadline(timeout)?;
        stream
            .write_all(b"DFHack?\n\x01\x00\x00\x00")
            .map_err(io_error)?;
        stream.flush().map_err(io_error)?;
        let mut hello = [0; 12];
        stream.read_exact(&mut hello).map_err(io_error)?;
        require(
            &hello == b"DFHack!\n\x01\x00\x00\x00",
            "invalid DFHack handshake",
        )?;
        let mut methods = [0; 6];
        for (index, name) in METHODS.iter().enumerate() {
            let mut request = Vec::new();
            for (tag, value) in [
                (1, *name),
                (2, "dfmcp.order_run.v1_14.Request"),
                (3, "dfmcp.order_run.v1_14.Reply"),
                (4, "dfmcp_order_run_v1_14"),
            ] {
                bytes(&mut request, tag, value.as_bytes());
            }
            let response = call(&mut stream, 0, &request)?;
            let id =
                i16::try_from(Message::parse(&response, 1)?.number(1)?).map_err(|_| malformed())?;
            require(
                id >= 2 && !methods[..index].contains(&id),
                "native method ID aliases or is reserved",
            )?;
            methods[index] = id;
        }
        let mut client = Self {
            stream,
            secret,
            nonce,
            methods,
            fortress,
            endpoint: None,
            fenced: false,
            manifest: OrderRunManifest {
                generation: 0,
                df_version: String::new(),
                dfhack_version: String::new(),
            },
        };
        client.invoke(0, None, None, context, timeout)?;
        Ok(client)
    }
    pub fn poisoned(&self) -> bool {
        self.fenced
    }
    fn invoke(
        &mut self,
        operation: usize,
        plan: Option<&OrderRunPlan>,
        target: Option<u32>,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<Reply> {
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "order-run source fenced; use recovery, never replay commit",
            ));
        }
        authorize(context, &self.fortress, matches!(operation, 2 | 3 | 5))?;
        if let Some(plan) = plan {
            require(
                plan.before().fortress() == &self.fortress,
                "plan fortress differs from selected native source",
            )?;
            if matches!(operation, 2 | 3) {
                authorize_plan(context, plan)?;
            }
        }
        if context.budget.max_bytes < RPC_RESERVE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "native order-run call exceeds work allowance",
            ));
        }
        if target.is_some_and(|id| id > i32::MAX as u32) {
            return Err(malformed());
        }
        deadline_bound(timeout)?;
        let timeout = timeout.min(Duration::from_millis(context.budget.max_wall_millis));
        self.stream.narrow_deadline(timeout)?;
        let outcome = (|| {
            let mut request = Vec::new();
            bytes(&mut request, 1, &self.secret);
            bytes(&mut request, 2, &self.nonce);
            number(&mut request, 3, 1);
            number(&mut request, 4, 14);
            if let Some(id) = target {
                number(&mut request, 11, u64::from(id));
            }
            if let Some(plan) = plan {
                bytes(&mut request, 5, plan.key().as_bytes());
                bytes(&mut request, 9, plan.digest().as_bytes());
                if operation == 2 {
                    let s = plan.spec();
                    number(&mut request, 6, u64::from(s.game_ticks()));
                    number(&mut request, 7, u64::from(s.wall_ms()));
                    bytes(&mut request, 8, plan.before().canonical_bytes());
                    number(&mut request, 11, u64::from(plan.before().order_id()));
                    for (tag, value) in [
                        (12, u32::from(s.predicate().code())),
                        (13, s.predicate().threshold()),
                        (14, s.samples()),
                        (15, s.interval()),
                    ] {
                        number(&mut request, tag, u64::from(value));
                    }
                }
                if matches!(operation, 3 | 5) {
                    bytes(&mut request, 10, plan.token());
                }
            }
            let response = call(&mut self.stream, self.methods[operation], &request)?;
            let m = Message::parse(&response, 12)?;
            require(
                m.bytes(3)? == self.nonce && m.number(4)? == 1 && m.number(5)? == 14,
                "order-run reply nonce/profile mismatch",
            )?;
            let accepted = m.boolean(1)?;
            let code = m.number(2)?;
            if !accepted {
                require(
                    (1..=8).contains(&code)
                        && (6..=8).all(|tag| m.has(tag))
                        && (9..=12).all(|tag| !m.has(tag)),
                    "invalid native refusal",
                )?;
                return Err(error(
                    ErrorCode::AdapterRejected,
                    "native order-run refused; no effect outcome inferred",
                ));
            }
            require(
                code == 0 && m.number(12)? <= 256,
                "invalid accepted order-run metadata",
            )?;
            let active = m.boolean(11)?;
            let manifest = OrderRunManifest {
                generation: m.number(6)?,
                df_version: text(m.bytes(7)?, 128)?,
                dfhack_version: text(m.bytes(8)?, 128)?,
            };
            require(
                manifest.generation > 0 && manifest.generation < u64::MAX,
                "invalid native incarnation",
            )?;
            if self.manifest.generation != 0 {
                require(
                    manifest.generation >= self.manifest.generation
                        && manifest.df_version == self.manifest.df_version
                        && manifest.dfhack_version == self.manifest.dfhack_version,
                    "native software changed or generation regressed",
                )?;
            }
            require(
                m.has(9) == (operation == 1)
                    && (operation == 4 || m.has(10) == matches!(operation, 2 | 3 | 5)),
                "reply has wrong optional fields",
            )?;
            let capture = if m.has(9) {
                let capture = OrderCapture::decode(m.bytes(9)?)?;
                require(
                    capture.fortress() == &self.fortress
                        && Some(capture.order_id()) == target
                        && capture.generation() == manifest.generation,
                    "native capture differs from selected fortress/order",
                )?;
                let mut fresh = context.clone();
                fresh.anchor.tick = GameTick(capture.tick().max(context.anchor.tick.get()));
                authorize(&fresh, &self.fortress, false)?;
                Some(capture)
            } else {
                None
            };
            let record = if m.has(10) {
                let record = OrderRunRecord::decode(m.bytes(10)?)?;
                require(
                    Some(record.plan()) == plan
                        && record.plan().before().generation() <= manifest.generation,
                    "native receipt belongs to another complete intent",
                )?;
                if matches!(record.phase(), RunPhase::Running | RunPhase::Stopping) {
                    require(
                        active && record.plan().before().generation() == manifest.generation,
                        "unowned running receipt",
                    )?;
                }
                let mut fresh = context.clone();
                fresh.anchor.tick = GameTick(
                    record
                        .observed_tick()
                        .map_or(context.anchor.tick.get(), |tick| {
                            tick.max(context.anchor.tick.get())
                        }),
                );
                authorize(&fresh, &self.fortress, false)?;
                Some(record)
            } else {
                None
            };
            self.manifest = manifest;
            Ok(Reply { capture, record })
        })();
        if outcome.is_err() {
            self.fenced = true;
        }
        outcome
    }
    fn required_record(
        &mut self,
        op: usize,
        plan: &OrderRunPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<OrderRunRecord> {
        self.invoke(op, Some(plan), None, context, timeout)?
            .record
            .ok_or_else(malformed)
    }
}
impl<S: OrderRunStream> OrderRunSource for OrderRunRpc<S> {
    fn manifest(&self) -> &OrderRunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.endpoint
    }
    fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn observe(&mut self, id: u32, c: &OperationContext, t: Duration) -> Result<OrderCapture> {
        self.invoke(1, None, Some(id), c, t)?
            .capture
            .ok_or_else(malformed)
    }
    fn prepare(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        self.required_record(2, p, c, t)
    }
    fn commit(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        self.required_record(3, p, c, t)
    }
    fn query(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        Ok(self.invoke(4, Some(p), None, c, t)?.record)
    }
    fn cancel(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        t: Duration,
    ) -> Result<OrderRunRecord> {
        self.required_record(5, p, c, t)
    }
}

pub struct OrderRunTcp {
    stream: TcpStream,
    deadline: Instant,
}
impl OrderRunTcp {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|t| !t.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "order-run deadline exhausted"))
    }
}
impl Read for OrderRunTcp {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(out)
    }
}
impl Write for OrderRunTcp {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.remaining()?;
        self.stream.flush()
    }
}
impl OrderRunStream for OrderRunTcp {
    fn narrow_deadline(&mut self, timeout: Duration) -> Result<()> {
        deadline_bound(timeout)?;
        let requested = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "deadline overflow"))?;
        self.deadline = self.deadline.min(requested);
        self.remaining().map_err(io_error)?;
        Ok(())
    }
}
impl OrderRunRpc<OrderRunTcp> {
    pub fn connect(
        endpoint: SocketAddr,
        secret: Vec<u8>,
        nonce: Vec<u8>,
        fortress: FortressIdentity,
        context: &OperationContext,
    ) -> Result<Self> {
        authorize(context, &fortress, false)?;
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "native order-run endpoint must be numeric loopback",
            ));
        }
        if context.budget.max_bytes < CONNECT_RESERVE_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "handshake exceeds work allowance",
            ));
        }
        require(
            (32..=256).contains(&secret.len()) && (16..=64).contains(&nonce.len()),
            "invalid order-run credentials",
        )?;
        let timeout = Duration::from_millis(context.budget.max_wall_millis);
        deadline_bound(timeout)?;
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "deadline overflow"))?;
        let stream = TcpStream::connect_timeout(&endpoint, timeout).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        let mut result = Self::negotiate(
            OrderRunTcp { stream, deadline },
            secret,
            nonce,
            fortress,
            context,
        )?;
        result.endpoint = Some(endpoint);
        Ok(result)
    }
}

#[cfg(test)]
mod tests;
