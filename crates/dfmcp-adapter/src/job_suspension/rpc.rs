//! Fixed-method, bounded DFHack transport for isolated job-control/1.9.
//!
//! This low-level boundary receives explicit operator credentials and per-call
//! wall-time bounds. It does not create capability grants or durable custody.
//! A caller must durably record dispatch intent before `commit_prepared`; EVERY
//! error after that point is potentially ambiguous, including native refusals
//! without a verified effect record. Query recovery never repeats a setter.

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use super::{JobObservation, SuspensionEffect, SuspensionPlan, SuspensionState};
use dfmcp_core::{DfmcpError, ErrorCode, Result};

const MAX_RPC: usize = 8192;
const MAX_NOTIFICATIONS: usize = 8;
const MAX_NOTIFICATION: usize = 65_536;
const MAX_NOTIFICATION_TOTAL: usize = 262_144;
const PLUGIN: &str = "dfmcp_job_control_v1_9";
const REQUEST_TYPE: &str = "dfmcp.job_control.v1_9.Request";
const REPLY_TYPE: &str = "dfmcp.job_control.v1_9.Reply";
const METHODS: [&str; 5] = [
    "Handshake",
    "ReadJob",
    "PrepareSuspension",
    "CommitSuspension",
    "QuerySuspension",
];

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn malformed() -> DfmcpError {
    error(ErrorCode::AdapterRejected, "malformed job-control/1.9 RPC")
}
fn io_error(_: io::Error) -> DfmcpError {
    error(
        ErrorCode::AdapterUnavailable,
        "job-control I/O failed; a dispatched effect may be indeterminate",
    )
}
fn deadline(timeout: Duration) -> Result<Instant> {
    if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&timeout) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "job-control timeout must be 1..60000 milliseconds",
        ));
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "job-control deadline overflow"))
}
fn remaining(until: Instant) -> Result<Duration> {
    until
        .checked_duration_since(Instant::now())
        .filter(|t| !t.is_zero())
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "job-control wall-time budget exhausted",
            )
        })
}
fn credentials(token: &[u8], nonce: &[u8]) -> Result<()> {
    if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid job-control credential or nonce length",
        ));
    }
    Ok(())
}

/// An injected stream must apply the absolute deadline to every blocking read,
/// write and flush. The concrete TCP implementation below does so without tasks.
pub trait JobControlStream: Read + Write {
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
    let mut value = 0u64;
    for shift in 0..10 {
        let byte = *data.get(*offset).ok_or_else(malformed)?;
        *offset += 1;
        if shift == 9 && byte > 1 {
            return Err(malformed());
        }
        value |= u64::from(byte & 127) << (7 * shift);
        if byte < 128 {
            if shift != 0 && byte == 0 {
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
        if data.len() > MAX_RPC {
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
                    let value = data.get(offset..end).ok_or_else(malformed)?;
                    offset = end;
                    Field::Bytes(value)
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
            Some(Field::Number(n)) => Ok(*n),
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
            Some(Field::Bytes(b)) if b.len() <= maximum => Ok(b),
            _ => Err(malformed()),
        }
    }
    fn text(&self, field: u32, empty: bool) -> Result<String> {
        let value = std::str::from_utf8(self.bytes(field, 128)?).map_err(|_| malformed())?;
        if value.contains('\0') || (!empty && value.is_empty()) {
            return Err(malformed());
        }
        Ok(value.to_owned())
    }
}
fn header(method: i16, length: i32) -> [u8; 8] {
    let mut bytes = [0; 8];
    bytes[..2].copy_from_slice(&method.to_le_bytes());
    bytes[4..].copy_from_slice(&length.to_le_bytes());
    bytes
}
fn call<S: JobControlStream>(stream: &mut S, method: i16, request: &[u8]) -> Result<Vec<u8>> {
    if request.len() > MAX_RPC {
        return Err(malformed());
    }
    stream
        .write_all(&header(method, request.len() as i32))
        .map_err(io_error)?;
    stream.write_all(request).map_err(io_error)?;
    stream.flush().map_err(io_error)?;
    let mut notifications = 0;
    let mut notification_bytes = 0;
    loop {
        let mut h = [0; 8];
        stream.read_exact(&mut h).map_err(io_error)?;
        let id = i16::from_le_bytes([h[0], h[1]]);
        // DFHack's two alignment bytes are not semantic protocol fields.
        let signed = i32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        if id == -2 {
            return Err(error(
                ErrorCode::AdapterFailure,
                "DFHack rejected job-control RPC; outcome not established",
            ));
        }
        if id != -1 && id != -3 {
            return Err(malformed());
        }
        let size = usize::try_from(signed).map_err(|_| malformed())?;
        if id == -3 {
            notifications += 1;
            notification_bytes += size.min(MAX_NOTIFICATION_TOTAL + 1);
            if notifications > MAX_NOTIFICATIONS
                || size > MAX_NOTIFICATION
                || notification_bytes > MAX_NOTIFICATION_TOTAL
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "job-control text-notification budget exhausted",
                ));
            }
        } else if size > MAX_RPC {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "job-control reply exceeds 8 KiB",
            ));
        }
        let mut payload = vec![0; size];
        stream.read_exact(&mut payload).map_err(io_error)?;
        if id == -1 {
            return Ok(payload);
        }
    }
}
fn bind<S: JobControlStream>(stream: &mut S, name: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (field, value) in [(1, name), (2, REQUEST_TYPE), (3, REPLY_TYPE), (4, PLUGIN)] {
        bytes(&mut request, field, value.as_bytes());
    }
    let response = call(stream, 0, &request)?;
    let id = i16::try_from(Message::parse(&response, 1)?.number(1)?).map_err(|_| malformed())?;
    if id < 2 {
        return Err(malformed());
    }
    Ok(id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobControlManifest {
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
    manifest: JobControlManifest,
    observation: Option<Vec<u8>>,
    effect: Option<Vec<u8>>,
    replayed: Option<bool>,
}
fn decode(data: &[u8], nonce: &[u8], method: Method) -> Result<Reply> {
    let m = Message::parse(data, 11)?;
    if m.number(4)? != 1 || m.number(5)? != 9 {
        return Err(error(
            ErrorCode::VersionMismatch,
            "job-control bridge must be exactly protocol 1.9",
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
        if code == 0
            || code > 7
            || generation != 0
            || !df_version.is_empty()
            || !dfhack_version.is_empty()
            || (9..=11).any(|field| m.has(field))
        {
            return Err(malformed());
        }
        let code = match code {
            1 => ErrorCode::CapabilityDenied,
            2 => ErrorCode::VersionMismatch,
            3 => ErrorCode::InvalidRequest,
            4 => ErrorCode::AdapterRejected,
            6 => ErrorCode::StaleAnchor,
            7 => ErrorCode::Conflict,
            _ => ErrorCode::AdapterFailure,
        };
        return Err(error(
            code,
            "job-control request rejected without verified effect evidence",
        ));
    }
    if code != 0 || generation == 0 || generation == u64::MAX {
        return Err(malformed());
    }
    let shape = (m.has(9), m.has(10), m.has(11));
    let valid = match method {
        Method::Handshake => shape == (false, false, false),
        Method::Read => shape == (true, false, false),
        Method::Prepare => shape == (false, true, true),
        Method::Commit => shape == (false, true, false),
        Method::Query => !shape.0 && !shape.2,
    };
    if !valid {
        return Err(malformed());
    }
    Ok(Reply {
        manifest: JobControlManifest {
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

pub struct PreparationReply {
    effect: SuspensionEffect,
    replayed: bool,
}
impl PreparationReply {
    pub fn effect(&self) -> &SuspensionEffect {
        &self.effect
    }
    pub fn replayed(&self) -> bool {
        self.replayed
    }
}

pub struct JobControlRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    methods: [i16; 5],
    manifest: JobControlManifest,
    fenced: bool,
}
impl<S: JobControlStream> JobControlRpcClient<S> {
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
        for (index, name) in METHODS.iter().enumerate() {
            methods[index] = bind(&mut stream, name)?;
            if methods[..index].contains(&methods[index]) {
                return Err(malformed());
            }
            remaining(until)?;
        }
        let request = Self::base_request(&token, &nonce);
        let reply = decode(
            &call(&mut stream, methods[0], &request)?,
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
    fn base_request(token: &[u8], nonce: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        bytes(&mut out, 1, token);
        bytes(&mut out, 2, nonce);
        number(&mut out, 3, 1);
        number(&mut out, 4, 9);
        out
    }
    fn request(&self) -> Vec<u8> {
        Self::base_request(&self.token, &self.nonce)
    }
    pub fn manifest(&self) -> &JobControlManifest {
        &self.manifest
    }
    pub fn poisoned(&self) -> bool {
        self.fenced
    }
    pub fn fence(&mut self) {
        self.fenced = true;
    }
    fn check_plan(&self, plan: &SuspensionPlan) -> Result<()> {
        if plan.observation().generation() != self.manifest.generation {
            return Err(error(
                ErrorCode::StaleAnchor,
                "job plan belongs to another native incarnation",
            ));
        }
        Ok(())
    }
    fn invoke<T>(
        &mut self,
        method: Method,
        request: Vec<u8>,
        timeout: Duration,
        project: impl FnOnce(Reply) -> Result<T>,
    ) -> Result<T> {
        if self.fenced {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "job-control connection is fenced; query recovery requires a new connection",
            ));
        }
        let until = deadline(timeout)?;
        let result = (|| {
            self.stream.set_deadline(until).map_err(io_error)?;
            let reply = decode(
                &call(&mut self.stream, self.methods[method as usize], &request)?,
                &self.nonce,
                method,
            )?;
            if reply.manifest != self.manifest {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "job-control incarnation or software changed; do not redispatch",
                ));
            }
            let value = project(reply)?;
            remaining(until)?;
            Ok(value)
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    pub fn read_job(&mut self, job: u32, timeout: Duration) -> Result<JobObservation> {
        if job > i32::MAX as u32 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "native job ID exceeds signed 32-bit range",
            ));
        }
        let mut request = self.request();
        number(&mut request, 6, u64::from(job));
        self.invoke(Method::Read, request, timeout, |reply| {
            let raw = reply.observation.ok_or_else(malformed)?;
            let observation = JobObservation::decode(&raw)?;
            if observation.job_id() != job || observation.generation() != reply.manifest.generation
            {
                return Err(malformed());
            }
            Ok(observation)
        })
    }
    pub fn prepare(
        &mut self,
        plan: &SuspensionPlan,
        timeout: Duration,
    ) -> Result<PreparationReply> {
        self.check_plan(plan)?;
        let mut request = self.request();
        bytes(&mut request, 5, plan.key().as_bytes());
        number(&mut request, 6, u64::from(plan.observation().job_id()));
        number(&mut request, 7, u64::from(plan.desired()));
        bytes(&mut request, 8, plan.observation().witness().as_bytes());
        bytes(&mut request, 9, plan.digest().as_bytes());
        self.invoke(Method::Prepare, request, timeout, |reply| {
            let effect = SuspensionEffect::decode(&reply.effect.ok_or_else(malformed)?, plan)?;
            let replayed = reply.replayed.ok_or_else(malformed)?;
            if !replayed && effect.state() != SuspensionState::Prepared {
                return Err(malformed());
            }
            Ok(PreparationReply { effect, replayed })
        })
    }
    /// The caller must sync durable dispatch intent BEFORE this call. Transport
    /// failures, native error envelopes and timeouts are never proof of no effect.
    pub fn commit_prepared(
        &mut self,
        plan: &SuspensionPlan,
        prepared: &SuspensionEffect,
        timeout: Duration,
    ) -> Result<SuspensionEffect> {
        self.check_plan(plan)?;
        let checked = SuspensionEffect::decode(prepared.canonical_bytes(), plan)?;
        if checked.state() != SuspensionState::Prepared {
            return Err(error(
                ErrorCode::Conflict,
                "only an exact native preparation may be dispatched; reconcile other states",
            ));
        }
        let mut request = self.request();
        bytes(&mut request, 5, plan.key().as_bytes());
        bytes(&mut request, 9, plan.digest().as_bytes());
        bytes(&mut request, 10, plan.prepare_token());
        self.invoke(Method::Commit, request, timeout, |reply| {
            let effect = SuspensionEffect::decode(&reply.effect.ok_or_else(malformed)?, plan)?;
            if effect.state() == SuspensionState::Prepared {
                return Err(malformed());
            }
            Ok(effect)
        })
    }
    /// `None` means no retained native record, NOT proof that dispatch never ran.
    pub fn query(
        &mut self,
        plan: &SuspensionPlan,
        timeout: Duration,
    ) -> Result<Option<SuspensionEffect>> {
        self.check_plan(plan)?;
        let mut request = self.request();
        bytes(&mut request, 5, plan.key().as_bytes());
        bytes(&mut request, 9, plan.digest().as_bytes());
        self.invoke(Method::Query, request, timeout, |reply| {
            reply
                .effect
                .map(|bytes| SuspensionEffect::decode(&bytes, plan))
                .transpose()
        })
    }
}

pub struct DeadlineTcpStream {
    stream: TcpStream,
    until: Instant,
}
impl DeadlineTcpStream {
    fn timeout(&self) -> io::Result<Duration> {
        self.until
            .checked_duration_since(Instant::now())
            .filter(|t| !t.is_zero())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::TimedOut, "job-control deadline exhausted")
            })
    }
}
impl JobControlStream for DeadlineTcpStream {
    fn set_deadline(&mut self, until: Instant) -> io::Result<()> {
        self.until = until;
        self.timeout()?;
        Ok(())
    }
}
impl Read for DeadlineTcpStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.timeout()?))?;
        self.stream.read(out)
    }
}
impl Write for DeadlineTcpStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.timeout()?))?;
        self.stream.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.timeout()?;
        self.stream.flush()
    }
}
impl JobControlRpcClient<DeadlineTcpStream> {
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
                "job-control endpoint must be numeric loopback with a nonzero port",
            ));
        }
        let stream = TcpStream::connect_timeout(&endpoint, remaining(until)?).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        Self::negotiate_until(DeadlineTcpStream { stream, until }, token, nonce, until)
    }
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
