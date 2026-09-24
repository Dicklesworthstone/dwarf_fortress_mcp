//! Fixed-method, bounded transport for the isolated work-orders/1.10 plugin.
//!
//! This is a low-level native boundary, not mutation authority. A production-
//! authorized coordinator must sync durable dispatch intent before commit.
//! Every error after entering commit I/O is conservatively indeterminate.
//! Query absence never permits a new insertion; there are no automatic retries.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use super::{WorkOrderEffect, WorkOrderObservation, WorkOrderPlan, WorkOrderState};
use dfmcp_core::{DfmcpError, ErrorCode, Result};

pub const MAX_RPC_BYTES: usize = 32 * 1024;
const MAX_NOTIFICATIONS: usize = 8;
const MAX_NOTIFICATION_BYTES: usize = 64 * 1024;
const MAX_NOTIFICATION_TOTAL: usize = 256 * 1024;
const METHODS: [&str; 5] = [
    "Handshake",
    "ReadOrders",
    "PrepareOrder",
    "CommitOrder",
    "QueryOrder",
];
const PLUGIN: &str = "dfmcp_work_orders_v1_10";
const REQUEST: &str = "dfmcp.work_orders.v1_10.Request";
const REPLY: &str = "dfmcp.work_orders.v1_10.Reply";

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn malformed() -> DfmcpError {
    error(ErrorCode::AdapterRejected, "malformed work-orders/1.10 RPC")
}
fn io_error(_: io::Error) -> DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        "work-order I/O failed; a dispatched insertion may have happened",
    )
}
fn deadline(timeout: Duration) -> Result<Instant> {
    if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&timeout) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "work-order timeout must be 1..60000 milliseconds",
        ));
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "work-order deadline overflow"))
}
fn remaining(until: Instant) -> Result<Duration> {
    until
        .checked_duration_since(Instant::now())
        .filter(|t| !t.is_zero())
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "work-order wall-time budget exhausted",
            )
        })
}
fn credentials(token: &[u8], nonce: &[u8]) -> Result<()> {
    if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid work-order token or nonce length",
        ));
    }
    Ok(())
}

/// Injected streams must apply the absolute deadline to every blocking operation.
pub trait WorkOrderStream: Read + Write {
    fn set_deadline(&mut self, until: Instant) -> io::Result<()>;
}
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push((value as u8 & 127) | 128);
        value >>= 7;
    }
    out.push(value as u8);
}
fn number(out: &mut Vec<u8>, field: u32, value: u64) {
    varint(out, u64::from(field) << 3);
    varint(out, value);
}
fn bytes(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    varint(out, (u64::from(field) << 3) | 2);
    varint(out, value.len() as u64);
    out.extend_from_slice(value);
}
fn read_varint(data: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0;
    for shift in 0..10 {
        let byte = *data.get(*offset).ok_or_else(malformed)?;
        *offset += 1;
        if shift == 9 && byte > 1 {
            return Err(malformed());
        }
        value |= u64::from(byte & 127) << (shift * 7);
        if byte < 128 {
            if shift > 0 && byte == 0 {
                return Err(malformed());
            }
            return Ok(value);
        }
    }
    Err(malformed())
}
#[derive(Clone, Copy)]
enum Field<'a> {
    Number(u64),
    Bytes(&'a [u8]),
}
struct Message<'a>(BTreeMap<u32, Field<'a>>);
impl<'a> Message<'a> {
    fn parse(data: &'a [u8], maximum_field: u32) -> Result<Self> {
        if data.len() > MAX_RPC_BYTES {
            return Err(malformed());
        }
        let mut offset = 0;
        let mut fields = BTreeMap::new();
        while offset < data.len() {
            let key = read_varint(data, &mut offset)?;
            let field = u32::try_from(key >> 3).map_err(|_| malformed())?;
            if field == 0 || field > maximum_field || fields.contains_key(&field) {
                return Err(malformed());
            }
            let value = match key & 7 {
                0 => Field::Number(read_varint(data, &mut offset)?),
                2 => {
                    let length = usize::try_from(read_varint(data, &mut offset)?)
                        .map_err(|_| malformed())?;
                    let end = offset.checked_add(length).ok_or_else(malformed)?;
                    let slice = data.get(offset..end).ok_or_else(malformed)?;
                    offset = end;
                    Field::Bytes(slice)
                }
                _ => return Err(malformed()),
            };
            fields.insert(field, value);
        }
        Ok(Self(fields))
    }
    fn has(&self, field: u32) -> bool {
        self.0.contains_key(&field)
    }
    fn number(&self, field: u32) -> Result<u64> {
        match self.0.get(&field) {
            Some(Field::Number(value)) => Ok(*value),
            _ => Err(malformed()),
        }
    }
    fn boolean(&self, field: u32) -> Result<bool> {
        match self.number(field)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(malformed()),
        }
    }
    fn bytes(&self, field: u32, maximum: usize) -> Result<&'a [u8]> {
        match self.0.get(&field) {
            Some(Field::Bytes(value)) if value.len() <= maximum => Ok(value),
            _ => Err(malformed()),
        }
    }
    fn text(&self, field: u32, empty_allowed: bool) -> Result<String> {
        let text = std::str::from_utf8(self.bytes(field, 128)?).map_err(|_| malformed())?;
        if text.contains('\0') || (!empty_allowed && text.is_empty()) {
            return Err(malformed());
        }
        Ok(text.to_owned())
    }
}
fn header(method: i16, length: i32) -> [u8; 8] {
    let mut out = [0; 8];
    out[..2].copy_from_slice(&method.to_le_bytes());
    out[4..].copy_from_slice(&length.to_le_bytes());
    out
}
fn call<S: WorkOrderStream>(stream: &mut S, method: i16, request: &[u8]) -> Result<Vec<u8>> {
    if request.len() > MAX_RPC_BYTES {
        return Err(malformed());
    }
    stream
        .write_all(&header(method, request.len() as i32))
        .map_err(io_error)?;
    stream.write_all(request).map_err(io_error)?;
    stream.flush().map_err(io_error)?;
    let mut notifications = 0;
    let mut total = 0;
    loop {
        let mut h = [0; 8];
        stream.read_exact(&mut h).map_err(io_error)?;
        let method = i16::from_le_bytes([h[0], h[1]]);
        // Bytes 2..4 are DFHack alignment padding, not semantic fields.
        let length = i32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        if method == -2 {
            return Err(error(
                ErrorCode::AdapterFailure,
                "DFHack rejected work-order call; insertion outcome not established",
            ));
        }
        if method != -1 && method != -3 {
            return Err(malformed());
        }
        let size = usize::try_from(length).map_err(|_| malformed())?;
        if method == -3 {
            notifications += 1;
            total += size.min(MAX_NOTIFICATION_TOTAL + 1);
            if notifications > MAX_NOTIFICATIONS
                || size > MAX_NOTIFICATION_BYTES
                || total > MAX_NOTIFICATION_TOTAL
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "work-order notification bound exceeded",
                ));
            }
        } else if size > MAX_RPC_BYTES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "work-order reply exceeds 32 KiB",
            ));
        }
        let mut payload = vec![0; size];
        stream.read_exact(&mut payload).map_err(io_error)?;
        if method == -1 {
            return Ok(payload);
        }
    }
}
fn bind<S: WorkOrderStream>(stream: &mut S, method: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (field, value) in [(1, method), (2, REQUEST), (3, REPLY), (4, PLUGIN)] {
        bytes(&mut request, field, value.as_bytes());
    }
    let reply = call(stream, 0, &request)?;
    let id = i16::try_from(Message::parse(&reply, 1)?.number(1)?).map_err(|_| malformed())?;
    if id < 2 {
        return Err(malformed());
    }
    Ok(id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkOrderManifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Method {
    Handshake = 0,
    Read = 1,
    Prepare = 2,
    Commit = 3,
    Query = 4,
}
struct Reply {
    manifest: WorkOrderManifest,
    observation: Option<Vec<u8>>,
    effect: Option<Vec<u8>>,
    replayed: Option<bool>,
}
fn decode(data: &[u8], nonce: &[u8], method: Method) -> Result<Reply> {
    let m = Message::parse(data, 11)?;
    if m.number(4)? != 1 || m.number(5)? != 10 {
        return Err(error(
            ErrorCode::VersionMismatch,
            "work-order bridge must be exactly protocol 1.10",
        ));
    }
    if m.bytes(3, 64)? != nonce {
        return Err(malformed());
    }
    let accepted = m.boolean(1)?;
    let code = m.number(2)?;
    let generation = m.number(6)?;
    let df_version = m.text(7, !accepted)?;
    let dfhack_version = m.text(8, !accepted)?;
    if !accepted {
        if !(1..=8).contains(&code)
            || generation != 0
            || !df_version.is_empty()
            || !dfhack_version.is_empty()
            || (9..=11).any(|field| m.has(field))
        {
            return Err(malformed());
        }
        let kind = match code {
            1 => ErrorCode::CapabilityDenied,
            2 => ErrorCode::VersionMismatch,
            3 => ErrorCode::InvalidRequest,
            4 => ErrorCode::PreconditionsFailed,
            5 => ErrorCode::AdapterFailure,
            6 => ErrorCode::StaleAnchor,
            7 => ErrorCode::Conflict,
            _ => ErrorCode::EffectIndeterminate,
        };
        return Err(error(
            kind,
            "native work-order request refused; no successful insertion evidence",
        ));
    }
    if code != 0 || generation == 0 || generation == u64::MAX {
        return Err(malformed());
    }
    let expected = match method {
        Method::Handshake => (false, false, false),
        Method::Read => (true, false, false),
        Method::Prepare => (false, true, true),
        Method::Commit => (false, true, false),
        Method::Query => (false, m.has(10), false),
    };
    if (m.has(9), m.has(10), m.has(11)) != expected {
        return Err(malformed());
    }
    Ok(Reply {
        manifest: WorkOrderManifest {
            generation,
            df_version,
            dfhack_version,
        },
        observation: if m.has(9) {
            Some(m.bytes(9, super::MAX_OBSERVATION_BYTES)?.to_vec())
        } else {
            None
        },
        effect: if m.has(10) {
            Some(m.bytes(10, super::MAX_EFFECT_BYTES)?.to_vec())
        } else {
            None
        },
        replayed: if m.has(11) {
            Some(m.boolean(11)?)
        } else {
            None
        },
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkOrderPreparation {
    effect: WorkOrderEffect,
    replayed: bool,
}
impl WorkOrderPreparation {
    pub fn effect(&self) -> &WorkOrderEffect {
        &self.effect
    }
    pub fn replayed(&self) -> bool {
        self.replayed
    }
}

pub struct WorkOrderRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    methods: [i16; 5],
    manifest: WorkOrderManifest,
    fenced: bool,
}
impl<S: WorkOrderStream> WorkOrderRpcClient<S> {
    pub fn negotiate(stream: S, token: Vec<u8>, nonce: Vec<u8>, timeout: Duration) -> Result<Self> {
        credentials(&token, &nonce)?;
        Self::negotiate_until(stream, token, nonce, deadline(timeout)?)
    }
    fn negotiate_until(
        mut stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        until: Instant,
    ) -> Result<Self> {
        remaining(until)?;
        stream.set_deadline(until).map_err(io_error)?;
        stream.write_all(b"DFHack?\n").map_err(io_error)?;
        stream.write_all(&1i32.to_le_bytes()).map_err(io_error)?;
        stream.flush().map_err(io_error)?;
        let mut hello = [0; 12];
        stream.read_exact(&mut hello).map_err(io_error)?;
        if &hello[..8] != b"DFHack!\n" || hello[8..] != 1i32.to_le_bytes() {
            return Err(malformed());
        }
        let mut methods = [0; 5];
        for (i, name) in METHODS.iter().enumerate() {
            methods[i] = bind(&mut stream, name)?;
            if methods[..i].contains(&methods[i]) {
                return Err(malformed());
            }
            remaining(until)?;
        }
        let reply = decode(
            &call(&mut stream, methods[0], &Self::base(&token, &nonce))?,
            &nonce,
            Method::Handshake,
        )?;
        remaining(until)?;
        Ok(Self {
            stream,
            token,
            nonce,
            methods,
            manifest: reply.manifest,
            fenced: false,
        })
    }
    fn base(token: &[u8], nonce: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        bytes(&mut out, 1, token);
        bytes(&mut out, 2, nonce);
        number(&mut out, 3, 1);
        number(&mut out, 4, 10);
        out
    }
    fn request(&self) -> Vec<u8> {
        Self::base(&self.token, &self.nonce)
    }
    pub fn manifest(&self) -> &WorkOrderManifest {
        &self.manifest
    }
    pub fn poisoned(&self) -> bool {
        self.fenced
    }
    pub fn fence(&mut self) {
        self.fenced = true;
    }
    fn ready(&self) -> Result<()> {
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "work-order connection fenced; reopen only for explicit recovery",
            ));
        }
        Ok(())
    }
    fn check_plan(&self, plan: &WorkOrderPlan) -> Result<()> {
        self.ready()?;
        if plan.observation().generation() != self.manifest.generation {
            return Err(error(
                ErrorCode::StaleAnchor,
                "work-order plan belongs to another native incarnation",
            ));
        }
        Ok(())
    }
    fn invoke<T>(
        &mut self,
        method: Method,
        request: Vec<u8>,
        until: Instant,
        project: impl FnOnce(Reply) -> Result<T>,
    ) -> Result<T> {
        let result = (|| {
            remaining(until)?;
            self.stream.set_deadline(until).map_err(io_error)?;
            let reply = decode(
                &call(&mut self.stream, self.methods[method as usize], &request)?,
                &self.nonce,
                method,
            )?;
            if reply.manifest != self.manifest {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "work-order source incarnation or software changed; never redispatch",
                ));
            }
            let result = project(reply)?;
            remaining(until)?;
            Ok(result)
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    pub fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation> {
        self.ready()?;
        let until = deadline(timeout)?;
        self.invoke(Method::Read, self.request(), until, |reply| {
            let observation =
                WorkOrderObservation::decode(&reply.observation.ok_or_else(malformed)?)?;
            if observation.generation() != reply.manifest.generation {
                return Err(malformed());
            }
            Ok(observation)
        })
    }
    pub fn prepare(
        &mut self,
        plan: &WorkOrderPlan,
        timeout: Duration,
    ) -> Result<WorkOrderPreparation> {
        self.check_plan(plan)?;
        let until = deadline(timeout)?;
        let mut request = self.request();
        bytes(&mut request, 5, plan.key().as_bytes());
        number(&mut request, 6, plan.spec().recipe() as u64);
        number(&mut request, 7, u64::from(plan.spec().amount()));
        bytes(&mut request, 8, plan.observation().witness().as_bytes());
        bytes(&mut request, 9, plan.digest().as_bytes());
        self.invoke(Method::Prepare, request, until, |reply| {
            let effect = WorkOrderEffect::decode(&reply.effect.ok_or_else(malformed)?, plan)?;
            let replayed = reply.replayed.ok_or_else(malformed)?;
            if !replayed && effect.state() != WorkOrderState::Prepared {
                return Err(malformed());
            }
            Ok(WorkOrderPreparation { effect, replayed })
        })
    }
    /// Caller MUST durably record dispatch before calling. This method does not
    /// grant capability or provide cross-process duplicate protection. A native
    /// error envelope, failed readback or lost reply is NOT proof of no insertion.
    pub fn commit_prepared(
        &mut self,
        plan: &WorkOrderPlan,
        prepared: &WorkOrderEffect,
        timeout: Duration,
    ) -> Result<WorkOrderEffect> {
        self.check_plan(plan)?;
        let until = deadline(timeout)?;
        let checked = WorkOrderEffect::decode(prepared.canonical_bytes(), plan)?;
        if checked.state() != WorkOrderState::Prepared {
            return Err(error(
                ErrorCode::Conflict,
                "only an exact preparation can enter work-order dispatch",
            ));
        }
        let mut request = self.request();
        bytes(&mut request, 5, plan.key().as_bytes());
        bytes(&mut request, 9, plan.digest().as_bytes());
        bytes(&mut request, 10, plan.prepare_token());
        self.invoke(Method::Commit, request, until, |reply| {
            let effect = WorkOrderEffect::decode(&reply.effect.ok_or_else(malformed)?, plan)?;
            if effect.state() == WorkOrderState::Prepared { return Err(malformed()); }
            Ok(effect)
        }).map_err(|_| error(ErrorCode::EffectIndeterminate,
            "work-order dispatch outcome is uncertain; retain durable intent and query, never retry insertion"))
    }
    /// None means no retained native record, not proof of non-creation.
    pub fn query(
        &mut self,
        plan: &WorkOrderPlan,
        timeout: Duration,
    ) -> Result<Option<WorkOrderEffect>> {
        self.check_plan(plan)?;
        let until = deadline(timeout)?;
        let mut request = self.request();
        bytes(&mut request, 5, plan.key().as_bytes());
        bytes(&mut request, 9, plan.digest().as_bytes());
        self.invoke(Method::Query, request, until, |reply| {
            reply
                .effect
                .map(|data| WorkOrderEffect::decode(&data, plan))
                .transpose()
        })
    }
}

pub struct WorkOrderTcpStream {
    stream: TcpStream,
    until: Instant,
}
impl WorkOrderTcpStream {
    fn timeout(&self) -> io::Result<Duration> {
        self.until
            .checked_duration_since(Instant::now())
            .filter(|time| !time.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "work-order deadline exhausted"))
    }
}
impl WorkOrderStream for WorkOrderTcpStream {
    fn set_deadline(&mut self, until: Instant) -> io::Result<()> {
        self.until = until;
        self.timeout()?;
        Ok(())
    }
}
impl Read for WorkOrderTcpStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.timeout()?))?;
        self.stream.read(out)
    }
}
impl Write for WorkOrderTcpStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.timeout()?))?;
        self.stream.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.timeout()?;
        self.stream.flush()
    }
}
impl WorkOrderRpcClient<WorkOrderTcpStream> {
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        timeout: Duration,
    ) -> Result<Self> {
        credentials(&token, &nonce)?;
        let until = deadline(timeout)?;
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "work-order endpoint must be numeric loopback with nonzero port",
            ));
        }
        let stream = TcpStream::connect_timeout(&endpoint, remaining(until)?).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        Self::negotiate_until(WorkOrderTcpStream { stream, until }, token, nonce, until)
    }
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
