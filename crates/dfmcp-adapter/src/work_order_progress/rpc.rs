//! Fixed two-method read-only transport. Does not confer authority or reconnect.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use super::{MAX_OBSERVATION_BYTES, ProgressManifest, ProgressObservation, ProgressSource, validate_targets};

const MAX_FRAME: usize = 32 * 1024;
const PLUGIN: &str = "dfmcp_work_order_progress_v1_12";
const REQUEST: &str = "dfmcp.work_order_progress.v1_12.Request";
const REPLY: &str = "dfmcp.work_order_progress.v1_12.Reply";
fn bad() -> DfmcpError { DfmcpError::new(ErrorCode::AdapterRejected, "malformed progress/1.12 RPC") }
fn io_error(_: io::Error) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterUnavailable, "progress I/O failed; connection fenced, no game mutation dispatched")
}
fn deadline(timeout: Duration) -> Result<Instant> {
    if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&timeout) {
        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "progress timeout must be 1..60000 milliseconds"));
    }
    Instant::now().checked_add(timeout).ok_or_else(bad)
}
fn remaining(until: Instant) -> Result<Duration> {
    until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
        .ok_or_else(|| DfmcpError::new(ErrorCode::BudgetExceeded, "progress deadline expired"))
}
fn credentials(token: &[u8], nonce: &[u8]) -> Result<()> {
    if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
        return Err(DfmcpError::new(ErrorCode::InvalidRequest, "progress credentials or nonce exceed bounds"));
    }
    Ok(())
}
pub trait ProgressStream: Read + Write {
    /// Every blocking read, write and flush must honor this absolute deadline.
    fn set_deadline(&mut self, until: Instant) -> io::Result<()>;
}
fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 { out.push((n as u8 & 127) | 128); n >>= 7; } out.push(n as u8);
}
fn number(out: &mut Vec<u8>, field: u32, n: u64) { varint(out, u64::from(field) << 3); varint(out, n); }
fn bytes(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    varint(out, (u64::from(field) << 3) | 2); varint(out, data.len() as u64); out.extend_from_slice(data);
}
fn read_varint(data: &[u8], offset: &mut usize) -> Result<u64> {
    let mut value = 0;
    for shift in 0..10 {
        let b = *data.get(*offset).ok_or_else(bad)?; *offset += 1;
        if shift == 9 && b > 1 { return Err(bad()); }
        value |= u64::from(b & 127) << (shift * 7);
        if b < 128 { if shift > 0 && b == 0 { return Err(bad()); } return Ok(value); }
    }
    Err(bad())
}
enum Field<'a> { Number(u64), Bytes(&'a [u8]) }
struct Message<'a>(BTreeMap<u32, Field<'a>>);
impl<'a> Message<'a> {
    fn decode(data: &'a [u8], maximum: u32) -> Result<Self> {
        if data.len() > MAX_FRAME { return Err(bad()); }
        let mut fields = BTreeMap::new(); let mut offset = 0;
        while offset < data.len() {
            let key = read_varint(data, &mut offset)?;
            let field = u32::try_from(key >> 3).map_err(|_| bad())?;
            if field == 0 || field > maximum || fields.contains_key(&field) { return Err(bad()); }
            let value = match key & 7 {
                0 => Field::Number(read_varint(data, &mut offset)?),
                2 => {
                    let length = usize::try_from(read_varint(data, &mut offset)?).map_err(|_| bad())?;
                    let end = offset.checked_add(length).ok_or_else(bad)?;
                    let value = data.get(offset..end).ok_or_else(bad)?; offset = end; Field::Bytes(value)
                }
                _ => return Err(bad()),
            };
            fields.insert(field, value);
        }
        Ok(Self(fields))
    }
    fn number(&self, key: u32) -> Result<u64> {
        match self.0.get(&key) { Some(Field::Number(v)) => Ok(*v), _ => Err(bad()) }
    }
    fn boolean(&self, key: u32) -> Result<bool> {
        match self.number(key)? { 0 => Ok(false), 1 => Ok(true), _ => Err(bad()) }
    }
    fn bytes(&self, key: u32, maximum: usize) -> Result<&'a [u8]> {
        match self.0.get(&key) { Some(Field::Bytes(v)) if v.len() <= maximum => Ok(v), _ => Err(bad()) }
    }
    fn text(&self, key: u32, empty: bool) -> Result<String> {
        let s = std::str::from_utf8(self.bytes(key, 128)?).map_err(|_| bad())?;
        if s.contains('\0') || (!empty && s.is_empty()) { return Err(bad()); } Ok(s.to_owned())
    }
}
fn header(id: i16, length: i32) -> [u8; 8] {
    let mut h = [0; 8]; h[..2].copy_from_slice(&id.to_le_bytes()); h[4..].copy_from_slice(&length.to_le_bytes()); h
}
fn call<S: ProgressStream>(stream: &mut S, method: i16, request: &[u8]) -> Result<Vec<u8>> {
    if request.len() > MAX_FRAME { return Err(bad()); }
    stream.write_all(&header(method, request.len() as i32)).map_err(io_error)?;
    stream.write_all(request).map_err(io_error)?; stream.flush().map_err(io_error)?;
    let mut notifications = 0; let mut notification_bytes = 0usize;
    loop {
        let mut h = [0; 8]; stream.read_exact(&mut h).map_err(io_error)?;
        let method = i16::from_le_bytes([h[0], h[1]]);
        if method == -2 { return Err(DfmcpError::new(ErrorCode::AdapterFailure, "DFHack refused progress RPC")); }
        if method != -1 && method != -3 { return Err(bad()); }
        let size = usize::try_from(i32::from_le_bytes([h[4],h[5],h[6],h[7]])).map_err(|_| bad())?;
        if method == -3 {
            notifications += 1;
            notification_bytes = notification_bytes.checked_add(size).ok_or_else(bad)?;
            if notifications > 8 || size > 65_536 || notification_bytes > 262_144 { return Err(bad()); }
        } else if size > MAX_FRAME { return Err(bad()); }
        let mut data = vec![0; size]; stream.read_exact(&mut data).map_err(io_error)?;
        if method == -1 { return Ok(data); }
    }
}
fn bind<S: ProgressStream>(stream: &mut S, name: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (field, s) in [(1,name),(2,REQUEST),(3,REPLY),(4,PLUGIN)] { bytes(&mut request, field, s.as_bytes()); }
    let raw = call(stream, 0, &request)?; let message = Message::decode(&raw, 1)?;
    let id = i16::try_from(message.number(1)?).map_err(|_| bad())?;
    if id < 2 { return Err(bad()); } Ok(id)
}
fn decode_reply(raw: &[u8], nonce: &[u8], ids: Option<&[u32]>) -> Result<(ProgressManifest, Option<ProgressObservation>)> {
    let m = Message::decode(raw, 9)?;
    if m.number(4)? != 1 || m.number(5)? != 12 {
        return Err(DfmcpError::new(ErrorCode::VersionMismatch, "progress requires exact protocol 1.12"));
    }
    if m.bytes(3, 64)? != nonce { return Err(bad()); }
    let accepted = m.boolean(1)?; let code = m.number(2)?; let generation = m.number(6)?;
    let df_version = m.text(7, !accepted)?; let dfhack_version = m.text(8, !accepted)?;
    if !accepted {
        if !(1..=5).contains(&code) || generation != 0 || !df_version.is_empty() || !dfhack_version.is_empty() || m.0.contains_key(&9) {
            return Err(bad());
        }
        let kind = match code { 1 => ErrorCode::CapabilityDenied, 2 => ErrorCode::VersionMismatch,
            3 => ErrorCode::InvalidRequest, 4 => ErrorCode::AdapterRejected, _ => ErrorCode::AdapterFailure };
        return Err(DfmcpError::new(kind, "native progress request refused; no capture published"));
    }
    if code != 0 || generation == 0 || generation == u64::MAX || m.0.contains_key(&9) != ids.is_some() { return Err(bad()); }
    let observation = match ids {
        Some(ids) => {
            let o = ProgressObservation::decode(m.bytes(9, MAX_OBSERVATION_BYTES)?, ids)?;
            if o.generation() != generation { return Err(bad()); } Some(o)
        }
        None => None,
    };
    Ok((ProgressManifest { generation, df_version, dfhack_version }, observation))
}
pub struct ProgressRpcClient<S> {
    stream: S, token: Vec<u8>, nonce: Vec<u8>, read_method: i16,
    manifest: ProgressManifest, fenced: bool,
}
impl<S: ProgressStream> ProgressRpcClient<S> {
    pub fn negotiate(stream: S, token: Vec<u8>, nonce: Vec<u8>, timeout: Duration) -> Result<Self> {
        credentials(&token, &nonce)?; Self::negotiate_until(stream, token, nonce, deadline(timeout)?)
    }
    fn negotiate_until(mut stream: S, token: Vec<u8>, nonce: Vec<u8>, until: Instant) -> Result<Self> {
        stream.set_deadline(until).map_err(io_error)?; remaining(until)?;
        stream.write_all(b"DFHack?\n").map_err(io_error)?; stream.write_all(&1i32.to_le_bytes()).map_err(io_error)?;
        stream.flush().map_err(io_error)?;
        let mut hello = [0;12]; stream.read_exact(&mut hello).map_err(io_error)?;
        if &hello[..8] != b"DFHack!\n" || hello[8..] != 1i32.to_le_bytes() { return Err(bad()); }
        let handshake = bind(&mut stream, "Handshake")?; remaining(until)?;
        let read_method = bind(&mut stream, "ReadObservation")?;
        if handshake == read_method { return Err(bad()); } remaining(until)?;
        let raw = call(&mut stream, handshake, &Self::base(&token, &nonce))?;
        let (manifest, _) = decode_reply(&raw, &nonce, None)?; remaining(until)?;
        Ok(Self { stream, token, nonce, read_method, manifest, fenced: false })
    }
    fn base(token: &[u8], nonce: &[u8]) -> Vec<u8> {
        let mut request = Vec::new(); bytes(&mut request, 1, token); bytes(&mut request, 2, nonce);
        number(&mut request, 3, 1); number(&mut request, 4, 12); request
    }
    pub fn poisoned(&self) -> bool { self.fenced }
}
impl<S: ProgressStream> ProgressSource for ProgressRpcClient<S> {
    fn manifest(&self) -> &ProgressManifest { &self.manifest }
    fn fence(&mut self) { self.fenced = true; }
    fn read(&mut self, ids: &[u32], timeout: Duration) -> Result<ProgressObservation> {
        validate_targets(ids)?; let until = deadline(timeout)?;
        if self.fenced { return Err(DfmcpError::new(ErrorCode::AdapterUnavailable, "progress connection fenced; close and reopen explicitly")); }
        let mut request = Self::base(&self.token, &self.nonce);
        for id in ids { number(&mut request, 5, u64::from(*id)); }
        let result = (|| {
            self.stream.set_deadline(until).map_err(io_error)?;
            let raw = call(&mut self.stream, self.read_method, &request)?;
            let (manifest, observation) = decode_reply(&raw, &self.nonce, Some(ids))?;
            if manifest != self.manifest { return Err(DfmcpError::new(ErrorCode::StaleAnchor, "progress source incarnation or software changed")); }
            remaining(until)?; observation.ok_or_else(bad)
        })();
        if result.is_err() { self.fenced = true; } result
    }
}
pub struct ProgressTcpStream { stream: TcpStream, until: Instant }
impl ProgressTcpStream {
    fn timeout(&self) -> io::Result<Duration> {
        self.until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "progress deadline expired"))
    }
}
impl ProgressStream for ProgressTcpStream {
    fn set_deadline(&mut self, until: Instant) -> io::Result<()> { self.until = until; self.timeout()?; Ok(()) }
}
impl Read for ProgressTcpStream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.timeout()?))?; self.stream.read(bytes)
    }
}
impl Write for ProgressTcpStream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.timeout()?))?; self.stream.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> { self.timeout()?; self.stream.flush() }
}
impl ProgressRpcClient<ProgressTcpStream> {
    pub fn connect(endpoint: SocketAddr, token: Vec<u8>, nonce: Vec<u8>, timeout: Duration) -> Result<Self> {
        credentials(&token, &nonce)?; let until = deadline(timeout)?;
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(DfmcpError::new(ErrorCode::CapabilityDenied, "progress endpoint must be numeric loopback with a nonzero port"));
        }
        let stream = TcpStream::connect_timeout(&endpoint, remaining(until)?).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        Self::negotiate_until(ProgressTcpStream { stream, until }, token, nonce, until)
    }
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
