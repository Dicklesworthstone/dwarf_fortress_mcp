#![forbid(unsafe_code)]

//! Closed, authenticated transport for the experimental jobs-only DFHack profile.
//! No method name, plugin name, protobuf type, command, or path comes from a client.

#[path = "live_operations_rpc.rs"]
pub mod operations;

use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use crate::live_jobs::{LiveJobObservation, MAX_JOBS, MAX_JOB_FRAME_BYTES};

const MAX_RPC: usize = MAX_JOB_FRAME_BYTES + 4096;
const PLUGIN: &str = "dfmcp_jobs_v1_2";
const REQUEST_TYPE: &str = "dfmcp.jobs.v1_2.Request";
const REPLY_TYPE: &str = "dfmcp.jobs.v1_2.Reply";

fn failure(code: ErrorCode, text: &str) -> DfmcpError { DfmcpError::new(code, text) }
fn malformed() -> DfmcpError { failure(ErrorCode::AdapterRejected, "invalid jobs/1.2 protobuf reply") }
fn io_failure(_: io::Error) -> DfmcpError {
    // Never reflect remote text, token material, or unbounded OS error details.
    failure(ErrorCode::AdapterUnavailable, "jobs RPC I/O failed or exceeded its deadline")
}
fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 { out.push((value as u8 & 127) | 128); value >>= 7; }
    out.push(value as u8);
}
fn number(out: &mut Vec<u8>, field: u32, value: u64) {
    varint(out, u64::from(field) << 3); varint(out, value);
}
fn bytes(out: &mut Vec<u8>, field: u32, value: &[u8]) {
    varint(out, (u64::from(field) << 3) | 2);
    varint(out, value.len() as u64); out.extend_from_slice(value);
}
fn read_varint(input: &[u8], offset: &mut usize) -> Result<u64> {
    let mut result = 0u64;
    for index in 0..10 {
        let byte = *input.get(*offset).ok_or_else(malformed)?;
        *offset += 1;
        if index == 9 && byte > 1 { return Err(malformed()); }
        result |= u64::from(byte & 127) << (index * 7);
        if byte < 128 {
            if index > 0 && byte == 0 { return Err(malformed()); }
            return Ok(result);
        }
    }
    Err(malformed())
}
#[derive(Clone, Copy)]
enum Field<'a> { Number(u64), Bytes(&'a [u8]) }
struct Message<'a>(BTreeMap<u32, Field<'a>>);
impl<'a> Message<'a> {
    fn parse(input: &'a [u8], maximum_field: u32) -> Result<Self> {
        if input.len() > MAX_RPC { return Err(failure(ErrorCode::BudgetExceeded, "jobs RPC payload too large")); }
        let mut offset = 0;
        let mut fields = BTreeMap::new();
        while offset < input.len() {
            let key = read_varint(input, &mut offset)?;
            let field = u32::try_from(key >> 3).map_err(|_| malformed())?;
            if field == 0 || field > maximum_field || fields.contains_key(&field) { return Err(malformed()); }
            let value = match key & 7 {
                0 => Field::Number(read_varint(input, &mut offset)?),
                2 => {
                    let length = usize::try_from(read_varint(input, &mut offset)?).map_err(|_| malformed())?;
                    let end = offset.checked_add(length).ok_or_else(malformed)?;
                    let value = input.get(offset..end).ok_or_else(malformed)?;
                    offset = end;
                    Field::Bytes(value)
                }
                _ => return Err(malformed()),
            };
            fields.insert(field, value);
        }
        Ok(Self(fields))
    }
    fn number(&self, field: u32) -> Result<u64> {
        match self.0.get(&field) { Some(Field::Number(value)) => Ok(*value), _ => Err(malformed()) }
    }
    fn bytes(&self, field: u32, maximum: usize) -> Result<&'a [u8]> {
        match self.0.get(&field) {
            Some(Field::Bytes(value)) if value.len() <= maximum => Ok(value),
            _ => Err(malformed()),
        }
    }
    fn text(&self, field: u32) -> Result<String> {
        let value = std::str::from_utf8(self.bytes(field, 128)?).map_err(|_| malformed())?;
        if value.is_empty() || value.contains('\0') { return Err(malformed()); }
        Ok(value.to_owned())
    }
}

fn header(id: i16, length: i32) -> [u8; 8] {
    let mut result = [0u8; 8];
    result[..2].copy_from_slice(&id.to_le_bytes());
    result[4..].copy_from_slice(&length.to_le_bytes());
    result
}
fn call<S: Read + Write>(stream: &mut S, method: i16, request: &[u8], limit: usize) -> Result<Vec<u8>> {
    let length = i32::try_from(request.len()).map_err(|_| malformed())?;
    stream.write_all(&header(method, length)).map_err(io_failure)?;
    stream.write_all(request).map_err(io_failure)?;
    stream.flush().map_err(io_failure)?;
    let mut text_bytes = 0usize;
    for _ in 0..9 {
        let mut head = [0; 8]; stream.read_exact(&mut head).map_err(io_failure)?;
        let id = i16::from_le_bytes([head[0], head[1]]);
        let signed = i32::from_le_bytes([head[4], head[5], head[6], head[7]]);
        if id == -2 { return Err(failure(ErrorCode::AdapterFailure, "DFHack rejected the jobs RPC")); }
        if id != -1 && id != -3 { return Err(malformed()); }
        let length = usize::try_from(signed).map_err(|_| malformed())?;
        let ceiling = if id == -3 { 65_536 } else { limit.min(MAX_RPC) };
        if length > ceiling { return Err(failure(ErrorCode::BudgetExceeded, "jobs RPC reply exceeds byte budget")); }
        if id == -3 {
            text_bytes = text_bytes.checked_add(length).ok_or_else(malformed)?;
            if text_bytes > 262_144 { return Err(failure(ErrorCode::BudgetExceeded, "jobs RPC text budget exceeded")); }
        }
        let mut reply = vec![0; length]; stream.read_exact(&mut reply).map_err(io_failure)?;
        if id == -1 { return Ok(reply); }
        // Ignore DFHack text notifications. They are neither observations nor errors to echo.
    }
    Err(failure(ErrorCode::BudgetExceeded, "too many jobs RPC text notifications"))
}
fn bind<S: Read + Write>(stream: &mut S, method: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (field, value) in [(1, method), (2, REQUEST_TYPE), (3, REPLY_TYPE), (4, PLUGIN)] {
        bytes(&mut request, field, value.as_bytes());
    }
    let reply = call(stream, 0, &request, 1024)?;
    let id = i16::try_from(Message::parse(&reply, 1)?.number(1)?).map_err(|_| malformed())?;
    if id < 2 { return Err(malformed()); }
    Ok(id)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobsManifest {
    pub generation: u64,
    pub df_version: String,
    pub dfhack_version: String,
}
fn decode_reply<'a>(data: &'a [u8], nonce: &[u8], observation: bool) -> Result<(JobsManifest, Option<&'a [u8]>)> {
    let message = Message::parse(data, 9)?;
    if message.number(4)? != 1 || message.number(5)? != 2 {
        return Err(failure(ErrorCode::VersionMismatch, "jobs bridge must implement exactly profile 1.2"));
    }
    if message.bytes(3, 64)? != nonce { return Err(malformed()); }
    let accepted = message.number(1)?;
    let code = message.number(2)?;
    if accepted == 0 {
        if code == 0 || message.0.contains_key(&9) { return Err(malformed()); }
        return Err(failure(match code {
            1 => ErrorCode::CapabilityDenied, 2 => ErrorCode::VersionMismatch,
            3 => ErrorCode::BudgetExceeded, 4 => ErrorCode::FortressNotLoaded,
            _ => ErrorCode::AdapterFailure,
        }, "jobs bridge refused the request; no job roster was published"));
    }
    if accepted != 1 || code != 0 { return Err(malformed()); }
    let generation = message.number(6)?;
    if generation == 0 { return Err(malformed()); }
    let manifest = JobsManifest { generation, df_version: message.text(7)?, dfhack_version: message.text(8)? };
    let payload = if observation { Some(message.bytes(9, MAX_JOB_FRAME_BYTES)?) }
        else if message.0.contains_key(&9) { return Err(malformed()); } else { None };
    Ok((manifest, payload))
}

/// Owns credentials but intentionally has no Debug implementation.
pub struct JobsRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    manifest: JobsManifest,
    read_method: i16,
    max_jobs: u32,
    max_bytes: usize,
    poisoned: bool,
}
impl<S: Read + Write> JobsRpcClient<S> {
    pub fn negotiate(mut stream: S, token: Vec<u8>, nonce: Vec<u8>, max_jobs: u32, max_bytes: usize) -> Result<Self> {
        if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) || max_jobs == 0
            || max_jobs as usize > MAX_JOBS || !(1024..=MAX_RPC).contains(&max_bytes) {
            return Err(failure(ErrorCode::InvalidRequest, "invalid jobs bootstrap credentials or budgets"));
        }
        let mut hello = b"DFHack?\n".to_vec(); hello.extend_from_slice(&1i32.to_le_bytes());
        stream.write_all(&hello).map_err(io_failure)?;
        stream.flush().map_err(io_failure)?;
        let mut response = [0; 12]; stream.read_exact(&mut response).map_err(io_failure)?;
        if &response[..8] != b"DFHack!\n" || response[8..] != 1i32.to_le_bytes() { return Err(malformed()); }
        let handshake = bind(&mut stream, "Handshake")?;
        let read_method = bind(&mut stream, "ReadObservation")?;
        if handshake == read_method { return Err(malformed()); }
        let request = request(&token, &nonce, max_jobs);
        let response = call(&mut stream, handshake, &request, 4096.min(max_bytes))?;
        let (manifest, _) = decode_reply(&response, &nonce, false)?;
        Ok(Self { stream, token, nonce, manifest, read_method, max_jobs, max_bytes, poisoned: false })
    }
    #[must_use]
    pub fn manifest(&self) -> &JobsManifest { &self.manifest }
    #[must_use]
    pub fn poisoned(&self) -> bool { self.poisoned }
    pub fn fence(&mut self) { self.poisoned = true; }
    /// The injected transport owns timeout/cancellation. The TCP implementation
    /// below uses one absolute deadline across all fragments and notifications.
    pub fn read_observation(&mut self) -> Result<LiveJobObservation> {
        if self.poisoned { return Err(failure(ErrorCode::AdapterUnavailable, "jobs connection is fenced; reopen session")); }
        let result = (|| {
            let request = request(&self.token, &self.nonce, self.max_jobs);
            let response = call(&mut self.stream, self.read_method, &request, self.max_bytes)?;
            let (manifest, payload) = decode_reply(&response, &self.nonce, true)?;
            if manifest.df_version != self.manifest.df_version || manifest.dfhack_version != self.manifest.dfhack_version
                || manifest.generation < self.manifest.generation {
                return Err(failure(ErrorCode::StaleAnchor, "jobs bridge software or generation regressed"));
            }
            let observation = LiveJobObservation::decode_payload(payload.ok_or_else(malformed)?,
                manifest.generation, manifest.df_version.clone(), manifest.dfhack_version.clone())?;
            if observation.jobs.len() > self.max_jobs as usize {
                return Err(failure(ErrorCode::BudgetExceeded, "jobs bridge exceeded requested roster bound"));
            }
            self.manifest = manifest;
            Ok(observation)
        })();
        if result.is_err() { self.poisoned = true; }
        result
    }
}
fn request(token: &[u8], nonce: &[u8], max_jobs: u32) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(&mut out, 1, token); bytes(&mut out, 2, nonce);
    number(&mut out, 3, 1); number(&mut out, 4, 2); number(&mut out, 5, u64::from(max_jobs));
    out
}
pub struct DeadlineStream { stream: TcpStream, deadline: Instant }
fn checked_timeout(timeout: Duration) -> Result<Duration> {
    if timeout < Duration::from_millis(1) || timeout > Duration::from_secs(60) {
        return Err(failure(ErrorCode::BudgetExceeded, "jobs call deadline must be 1..60000 milliseconds"));
    }
    Ok(timeout)
}
impl DeadlineStream {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline.checked_duration_since(Instant::now()).filter(|value| !value.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "jobs call deadline"))
    }
}
impl Read for DeadlineStream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        self.stream.read(out)
    }
}
impl Write for DeadlineStream {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        self.stream.write(data)
    }
    fn flush(&mut self) -> io::Result<()> { self.remaining()?; self.stream.flush() }
}
impl JobsRpcClient<DeadlineStream> {
    pub fn connect(endpoint: SocketAddr, token: Vec<u8>, nonce: Vec<u8>, timeout: Duration,
        max_jobs: u32, max_bytes: usize) -> Result<Self> {
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(failure(ErrorCode::CapabilityDenied, "jobs endpoint must be numeric loopback with a nonzero port"));
        }
        let deadline = Instant::now().checked_add(checked_timeout(timeout)?)
            .ok_or_else(|| failure(ErrorCode::BudgetExceeded, "jobs deadline overflow"))?;
        let stream = TcpStream::connect_timeout(&endpoint, timeout).map_err(io_failure)?;
        stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream { stream, deadline }, token, nonce, max_jobs, max_bytes)
    }
    pub fn refresh(&mut self, timeout: Duration) -> Result<LiveJobObservation> {
        self.stream.deadline = Instant::now().checked_add(checked_timeout(timeout)?)
            .ok_or_else(|| failure(ErrorCode::BudgetExceeded, "jobs deadline overflow"))?;
        self.read_observation()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    struct Script { input: Cursor<Vec<u8>>, output: Vec<u8> }
    impl Read for Script { fn read(&mut self, out: &mut [u8]) -> io::Result<usize> { self.input.read(out) } }
    impl Write for Script {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> { self.output.extend_from_slice(input); Ok(input.len()) }
        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }
    fn reply(payload: Option<&[u8]>) -> Vec<u8> {
        let mut out = Vec::new(); number(&mut out, 1, 1); number(&mut out, 2, 0);
        bytes(&mut out, 3, &[b'n'; 16]); number(&mut out, 4, 1); number(&mut out, 5, 2);
        number(&mut out, 6, 7); bytes(&mut out, 7, b"df"); bytes(&mut out, 8, b"dfhack");
        if let Some(payload) = payload { bytes(&mut out, 9, payload); } out
    }
    fn frame(payload: &[u8]) -> Vec<u8> {
        let mut out = header(-1, payload.len() as i32).to_vec(); out.extend_from_slice(payload); out
    }
    #[test]
    fn reply_rejects_duplicates_overlong_varints_and_wrong_wire_types() {
        for data in [vec![8, 128, 0], vec![8, 1, 8, 1], vec![9, 0, 0, 0, 0, 0, 0, 0, 0],
            vec![0], vec![80, 1], vec![8, 255, 255, 255, 255, 255, 255, 255, 255, 255, 2]] {
            assert!(Message::parse(&data, 9).is_err());
        }
    }
    #[test]
    fn manifest_nonce_and_method_payload_are_not_interchangeable() -> Result<()> {
        assert_eq!(decode_reply(&reply(None), &[b'n'; 16], false)?.0.generation, 7);
        assert!(decode_reply(&reply(None), &[b'x'; 16], false).is_err());
        assert!(decode_reply(&reply(None), &[b'n'; 16], true).is_err());
        assert!(decode_reply(&reply(Some(b"payload")), &[b'n'; 16], false).is_err());
        let mut duplicated = reply(None); number(&mut duplicated, 6, 8);
        assert!(decode_reply(&duplicated, &[b'n'; 16], false).is_err());
        Ok(())
    }
    #[test]
    fn authenticated_bootstrap_and_read_execute_real_wire_decoder() -> Result<()> {
        let observation = LiveJobObservation { bridge_generation: 7, df_version: "df".to_owned(),
            dfhack_version: "dfhack".to_owned(), year: 105, year_tick: 3, paused: true,
            site_id: 1, world_folder: "region1".to_owned(), next_job_id: 0, jobs: Vec::new() };
        let payload = observation.encode_payload()?;
        let mut input = b"DFHack!\n".to_vec(); input.extend_from_slice(&1i32.to_le_bytes());
        for id in [2, 3] { let mut bind = Vec::new(); number(&mut bind, 1, id); input.extend(frame(&bind)); }
        input.extend(frame(&reply(None))); input.extend(frame(&reply(Some(&payload))));
        let mut client = JobsRpcClient::negotiate(Script { input: Cursor::new(input), output: Vec::new() },
            vec![b't'; 32], vec![b'n'; 16], 5, 8192)?;
        assert_eq!(client.read_observation()?, observation);
        assert!(!client.poisoned());
        assert!(client.read_observation().is_err());
        assert!(client.poisoned());
        let bytes_sent = client.stream.output.len();
        assert!(client.read_observation().is_err());
        assert_eq!(client.stream.output.len(), bytes_sent);
        Ok(())
    }
    #[test]
    fn oversized_and_negative_reply_lengths_fail_before_allocating_payload() {
        for length in [-1, (MAX_RPC + 1) as i32] {
            let mut script = Script { input: Cursor::new(header(-1, length).to_vec()), output: Vec::new() };
            assert!(call(&mut script, 2, b"", MAX_RPC).is_err());
        }
    }
    #[test]
    fn remote_address_and_invalid_deadlines_fail_before_io() {
        let endpoint = SocketAddr::from(([192, 0, 2, 1], 5000));
        assert!(JobsRpcClient::connect(endpoint, vec![b't'; 32], vec![b'n'; 16], Duration::from_millis(1), 1, 4096).is_err());
        assert!(checked_timeout(Duration::ZERO).is_err());
        assert!(checked_timeout(Duration::from_secs(61)).is_err());
    }
}