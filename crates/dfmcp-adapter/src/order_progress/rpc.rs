//! Fixed read-only DFHack protocol 1.11. Credentials/deadlines are explicit;
//! semantic Query authority belongs to the session, never to this transport.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use super::{OrderProgress, error};

const MAX_FRAME: usize = 4096;
pub const RPC_RESERVE_BYTES: u64 = 2 * MAX_FRAME as u64 + 262_144;
const PLUGIN: &str = "dfmcp_order_progress_v1_11";
const REQUEST: &str = "dfmcp.order_progress.v1_11.Request";
const REPLY: &str = "dfmcp.order_progress.v1_11.Reply";
fn malformed() -> DfmcpError { error(ErrorCode::AdapterRejected,"malformed order-progress/1.11 RPC") }
fn io_error(_: io::Error) -> DfmcpError { error(ErrorCode::AdapterUnavailable,"order-progress read failed; connection fenced") }
fn until(timeout: Duration) -> Result<Instant> {
    if !(Duration::from_millis(1)..=Duration::from_secs(60)).contains(&timeout) {
        return Err(error(ErrorCode::BudgetExceeded,"progress wall allowance must be 1..60000ms"));
    }
    Instant::now().checked_add(timeout).ok_or_else(malformed)
}
fn left(deadline: Instant) -> Result<Duration> {
    deadline.checked_duration_since(Instant::now()).filter(|v|!v.is_zero())
        .ok_or_else(||error(ErrorCode::BudgetExceeded,"progress read deadline exhausted"))
}
fn credentials(token: &[u8], nonce: &[u8]) -> Result<()> {
    if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
        return Err(error(ErrorCode::InvalidRequest,"invalid progress credential or nonce size"));
    }
    Ok(())
}
pub trait ProgressStream: Read + Write {
    /// All blocking operations must honor this absolute deadline.
    fn set_deadline(&mut self, deadline: Instant) -> io::Result<()>;
}
fn varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 128 { out.push((n as u8 & 127) | 128); n >>= 7; } out.push(n as u8);
}
fn number(out: &mut Vec<u8>, field: u32, n: u64) { varint(out,u64::from(field)<<3); varint(out,n); }
fn bytes(out: &mut Vec<u8>, field: u32, data: &[u8]) {
    varint(out,(u64::from(field)<<3)|2); varint(out,data.len() as u64); out.extend_from_slice(data);
}
fn read_varint(data: &mut &[u8]) -> Result<u64> {
    let mut n = 0;
    for shift in 0..10 {
        let (&b,rest) = data.split_first().ok_or_else(malformed)?; *data = rest;
        if shift == 9 && b > 1 { return Err(malformed()); }
        n |= u64::from(b & 127) << (shift*7);
        if b < 128 { if shift > 0 && b == 0 { return Err(malformed()); } return Ok(n); }
    }
    Err(malformed())
}
enum Field<'a> { Number(u64), Bytes(&'a [u8]) }
struct Message<'a>(BTreeMap<u32,Field<'a>>);
impl<'a> Message<'a> {
    fn parse(mut data: &'a [u8], maximum: u32) -> Result<Self> {
        if data.len() > MAX_FRAME { return Err(malformed()); }
        let mut fields = BTreeMap::new();
        while !data.is_empty() {
            let tag = read_varint(&mut data)?;
            let id = u32::try_from(tag >> 3).map_err(|_|malformed())?;
            if id == 0 || id > maximum || fields.contains_key(&id) { return Err(malformed()); }
            let value = match tag & 7 {
                0 => Field::Number(read_varint(&mut data)?),
                2 => {
                    let len = usize::try_from(read_varint(&mut data)?).map_err(|_|malformed())?;
                    let value = data.get(..len).ok_or_else(malformed)?; data = &data[len..]; Field::Bytes(value)
                }
                _ => return Err(malformed()),
            };
            fields.insert(id,value);
        }
        Ok(Self(fields))
    }
    fn number(&self, id: u32) -> Result<u64> { match self.0.get(&id) { Some(Field::Number(n))=>Ok(*n),_=>Err(malformed()) } }
    fn bytes(&self, id: u32, maximum: usize) -> Result<&'a [u8]> {
        match self.0.get(&id) { Some(Field::Bytes(b)) if b.len()<=maximum=>Ok(b),_=>Err(malformed()) }
    }
    fn text(&self, id: u32) -> Result<String> {
        let text = std::str::from_utf8(self.bytes(id,128)?).map_err(|_|malformed())?;
        if text.contains('\0') { return Err(malformed()); } Ok(text.to_owned())
    }
}
fn header(method: i16, size: i32) -> [u8;8] {
    let mut out = [0;8]; out[..2].copy_from_slice(&method.to_le_bytes()); out[4..].copy_from_slice(&size.to_le_bytes()); out
}
fn call<S: ProgressStream>(stream: &mut S, method: i16, request: &[u8]) -> Result<Vec<u8>> {
    if request.len() > MAX_FRAME { return Err(malformed()); }
    stream.write_all(&header(method,request.len() as i32)).map_err(io_error)?;
    stream.write_all(request).map_err(io_error)?; stream.flush().map_err(io_error)?;
    let mut notifications = 0; let mut text_bytes = 0;
    loop {
        let mut h = [0;8]; stream.read_exact(&mut h).map_err(io_error)?;
        let kind = i16::from_le_bytes([h[0],h[1]]);
        if kind == -2 { return Err(error(ErrorCode::AdapterFailure,"DFHack refused progress read")); }
        if !matches!(kind,-1|-3) { return Err(malformed()); }
        let size = usize::try_from(i32::from_le_bytes([h[4],h[5],h[6],h[7]])).map_err(|_|malformed())?;
        if kind == -3 {
            notifications += 1;
            if notifications > 8 || size > 65_536 || size > 262_144 - text_bytes {
                return Err(error(ErrorCode::BudgetExceeded,"progress notification bound exceeded"));
            }
            text_bytes += size;
        } else if size > MAX_FRAME { return Err(error(ErrorCode::BudgetExceeded,"progress reply exceeds 4KiB")); }
        let mut payload = vec![0;size]; stream.read_exact(&mut payload).map_err(io_error)?;
        if kind == -1 { return Ok(payload); }
    }
}
fn bind<S: ProgressStream>(stream: &mut S, name: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (id,text) in [(1,name),(2,REQUEST),(3,REPLY),(4,PLUGIN)] { bytes(&mut request,id,text.as_bytes()); }
    let data = call(stream,0,&request)?;
    let id = i16::try_from(Message::parse(&data,1)?.number(1)?).map_err(|_|malformed())?;
    if id < 2 { return Err(malformed()); } Ok(id)
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgressManifest { pub generation: u64, pub df_version: String, pub dfhack_version: String }
fn decode(data: &[u8], nonce: &[u8], observation: bool) -> Result<(ProgressManifest,Option<Vec<u8>>)> {
    let m = Message::parse(data,9)?;
    if m.number(4)? != 1 || m.number(5)? != 11 { return Err(error(ErrorCode::VersionMismatch,"progress protocol must be exactly 1.11")); }
    if m.bytes(3,64)? != nonce { return Err(malformed()); }
    let accepted = m.number(1)?; let code = m.number(2)?; let generation = m.number(6)?;
    let df_version = m.text(7)?; let dfhack_version = m.text(8)?;
    if accepted == 0 {
        if !(1..=5).contains(&code) || generation != 0 || !df_version.is_empty() || !dfhack_version.is_empty() || m.0.contains_key(&9) {
            return Err(malformed());
        }
        return Err(error(match code { 1=>ErrorCode::CapabilityDenied,2=>ErrorCode::VersionMismatch,
            3=>ErrorCode::InvalidRequest,4=>ErrorCode::FortressNotLoaded,_=>ErrorCode::AdapterFailure },"native progress read refused"));
    }
    if accepted != 1 || code != 0 || generation == 0 || generation == u64::MAX || df_version.is_empty()
        || dfhack_version.is_empty() || m.0.contains_key(&9) != observation { return Err(malformed()); }
    let payload = if observation { Some(m.bytes(9,super::MAX_PROGRESS_BYTES)?.to_vec()) } else { None };
    Ok((ProgressManifest { generation,df_version,dfhack_version },payload))
}
pub struct ProgressClient<S> { stream:S, token:Vec<u8>, nonce:Vec<u8>, read_method:i16,
    manifest:ProgressManifest, sequence:u64, fenced:bool }
impl<S: ProgressStream> ProgressClient<S> {
    pub fn negotiate(stream:S, token:Vec<u8>, nonce:Vec<u8>, timeout:Duration) -> Result<Self> {
        credentials(&token,&nonce)?; Self::negotiate_until(stream,token,nonce,until(timeout)?)
    }
    fn negotiate_until(mut stream:S, token:Vec<u8>, nonce:Vec<u8>, deadline:Instant) -> Result<Self> {
        left(deadline)?; stream.set_deadline(deadline).map_err(io_error)?;
        stream.write_all(b"DFHack?\n").map_err(io_error)?; stream.write_all(&1i32.to_le_bytes()).map_err(io_error)?;
        stream.flush().map_err(io_error)?;
        let mut hello = [0;12]; stream.read_exact(&mut hello).map_err(io_error)?;
        if &hello[..8] != b"DFHack!\n" || hello[8..] != 1i32.to_le_bytes() { return Err(malformed()); }
        let handshake = bind(&mut stream,"Handshake")?; left(deadline)?;
        let read_method = bind(&mut stream,"ReadOrderProgress")?; left(deadline)?;
        if handshake == read_method { return Err(malformed()); }
        let (manifest,_) = decode(&call(&mut stream,handshake,&Self::base(&token,&nonce))?,&nonce,false)?;
        left(deadline)?;
        Ok(Self { stream,token,nonce,read_method,manifest,sequence:0,fenced:false })
    }
    fn base(token:&[u8],nonce:&[u8]) -> Vec<u8> {
        let mut out = Vec::new(); bytes(&mut out,1,token); bytes(&mut out,2,nonce);
        number(&mut out,3,1); number(&mut out,4,11); out
    }
    pub fn manifest(&self) -> &ProgressManifest { &self.manifest }
    pub fn poisoned(&self) -> bool { self.fenced }
    pub fn fence(&mut self) { self.fenced = true; }
    pub fn read_order(&mut self, order:u32, timeout:Duration) -> Result<OrderProgress> {
        if self.fenced { return Err(error(ErrorCode::AdapterUnavailable,"progress connection fenced; close and reopen explicitly")); }
        if order > i32::MAX as u32 { return Err(error(ErrorCode::InvalidRequest,"native order ID out of range")); }
        let deadline = until(timeout)?;
        let mut request = Self::base(&self.token,&self.nonce); number(&mut request,5,u64::from(order));
        let result = (|| {
            self.stream.set_deadline(deadline).map_err(io_error)?;
            let (manifest,payload) = decode(&call(&mut self.stream,self.read_method,&request)?,&self.nonce,true)?;
            if manifest != self.manifest { return Err(error(ErrorCode::StaleAnchor,"progress incarnation or software changed")); }
            let observation = OrderProgress::decode(&payload.ok_or_else(malformed)?)?;
            if observation.generation() != manifest.generation || observation.order_id() != order || observation.sequence() <= self.sequence {
                return Err(malformed());
            }
            left(deadline)?; Ok(observation)
        })();
        match &result { Ok(o)=>self.sequence = o.sequence(),Err(_)=>self.fence() }
        result
    }
}
pub struct ProgressTcpStream { stream:TcpStream, deadline:Instant }
impl ProgressTcpStream {
    fn timeout(&self) -> io::Result<Duration> { self.deadline.checked_duration_since(Instant::now()).filter(|v|!v.is_zero())
        .ok_or_else(||io::Error::new(io::ErrorKind::TimedOut,"progress deadline exhausted")) }
}
impl ProgressStream for ProgressTcpStream {
    fn set_deadline(&mut self, deadline:Instant) -> io::Result<()> { self.deadline = deadline; self.timeout()?; Ok(()) }
}
impl Read for ProgressTcpStream {
    fn read(&mut self, out:&mut [u8]) -> io::Result<usize> { self.stream.set_read_timeout(Some(self.timeout()?))?; self.stream.read(out) }
}
impl Write for ProgressTcpStream {
    fn write(&mut self, data:&[u8]) -> io::Result<usize> { self.stream.set_write_timeout(Some(self.timeout()?))?; self.stream.write(data) }
    fn flush(&mut self) -> io::Result<()> { self.timeout()?; self.stream.flush() }
}
impl ProgressClient<ProgressTcpStream> {
    pub fn connect(endpoint:SocketAddr,token:Vec<u8>,nonce:Vec<u8>,timeout:Duration) -> Result<Self> {
        credentials(&token,&nonce)?; let deadline = until(timeout)?;
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 { return Err(error(ErrorCode::CapabilityDenied,"progress endpoint must be numeric loopback with nonzero port")); }
        let stream = TcpStream::connect_timeout(&endpoint,left(deadline)?).map_err(io_error)?;
        stream.set_nodelay(true).map_err(io_error)?;
        Self::negotiate_until(ProgressTcpStream { stream,deadline },token,nonce,deadline)
    }
}
#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
