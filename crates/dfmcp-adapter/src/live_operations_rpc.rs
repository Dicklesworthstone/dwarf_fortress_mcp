//! Closed operations/1.3 client. Compiled as a child of live_jobs_rpc so the
//! existing bounded framing, canonical protobuf reader, and absolute-deadline
//! stream are reused without exposing arbitrary plugin or method selection.

#[path = "live_map_rpc.rs"]
pub mod map;
#[path = "live_operations_paged_rpc.rs"]
pub mod paged;

use super::{
    DeadlineStream, JobsManifest, Message, bytes, call, checked_timeout, io_failure, number,
};
use crate::live_jobs::MAX_JOBS;
use crate::live_operations::{
    LiveOperationsObservation, MAX_BUILDINGS, MAX_ITEMS, MAX_OPERATIONS_BYTES,
};
use dfmcp_core::{DfmcpError, ErrorCode, Result};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

fn invalid(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::AdapterRejected, text)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OperationsLimits {
    pub jobs: u32,
    pub buildings: u32,
    pub items: u32,
    pub payload_bytes: usize,
}
impl Default for OperationsLimits {
    fn default() -> Self {
        Self {
            jobs: 1024,
            buildings: 1024,
            items: 8192,
            payload_bytes: MAX_OPERATIONS_BYTES,
        }
    }
}
impl OperationsLimits {
    pub fn validate(self) -> Result<()> {
        if self.jobs == 0
            || self.jobs as usize > MAX_JOBS
            || self.buildings == 0
            || self.buildings as usize > MAX_BUILDINGS
            || self.items == 0
            || self.items as usize > MAX_ITEMS
            || !(1024..=MAX_OPERATIONS_BYTES).contains(&self.payload_bytes)
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "operations limits exceed supported bounds",
            ));
        }
        Ok(())
    }
    #[must_use]
    pub fn entity_limit(self) -> u32 {
        self.jobs
            .saturating_add(self.buildings)
            .saturating_add(self.items)
            .saturating_add(1)
    }
    fn check(self, observation: &LiveOperationsObservation) -> Result<()> {
        if observation.jobs.jobs.len() > self.jobs as usize
            || observation.buildings.len() > self.buildings as usize
            || observation.items.len() > self.items as usize
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "operations reply exceeded negotiated roster bounds",
            ));
        }
        Ok(())
    }
}
fn credentials(token: &[u8], nonce: &[u8], limits: OperationsLimits) -> Result<()> {
    limits.validate()?;
    if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "invalid operations credentials or nonce lengths",
        ));
    }
    Ok(())
}
fn request(token: &[u8], nonce: &[u8], limits: OperationsLimits) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(&mut out, 1, token);
    bytes(&mut out, 2, nonce);
    for (field, value) in [
        (3, 1),
        (4, 3),
        (5, u64::from(limits.jobs)),
        (6, u64::from(limits.buildings)),
        (7, u64::from(limits.items)),
        (8, limits.payload_bytes as u64),
    ] {
        number(&mut out, field, value);
    }
    out
}
fn bind<S: Read + Write>(stream: &mut S, method: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (field, value) in [
        (1, method),
        (2, "dfmcp.operations.v1_3.Request"),
        (3, "dfmcp.operations.v1_3.Reply"),
        (4, "dfmcp_operations_v1_3"),
    ] {
        bytes(&mut request, field, value.as_bytes());
    }
    let response = call(stream, 0, &request, 1024)?;
    let id = i16::try_from(Message::parse(&response, 1)?.number(1)?)
        .map_err(|_| invalid("operations method ID exceeds i16"))?;
    if id < 2 {
        return Err(invalid("operations method bound to a reserved core ID"));
    }
    Ok(id)
}
fn decode<'a>(
    data: &'a [u8],
    nonce: &[u8],
    observation: bool,
    limit: usize,
) -> Result<(JobsManifest, Option<&'a [u8]>)> {
    let value = Message::parse(data, 9)?;
    if value.number(4)? != 1 || value.number(5)? != 3 {
        return Err(DfmcpError::new(
            ErrorCode::VersionMismatch,
            "operations bridge must be exactly protocol 1.3",
        ));
    }
    if value.bytes(3, 64)? != nonce {
        return Err(invalid("operations reply nonce mismatch"));
    }
    let accepted = value.number(1)?;
    let code = value.number(2)?;
    if accepted == 0 {
        if code == 0 || value.0.contains_key(&9) {
            return Err(invalid("rejected operations reply carried a payload"));
        }
        return Err(DfmcpError::new(
            match code {
                1 => ErrorCode::CapabilityDenied,
                2 => ErrorCode::VersionMismatch,
                3 => ErrorCode::BudgetExceeded,
                4 => ErrorCode::FortressNotLoaded,
                _ => ErrorCode::AdapterFailure,
            },
            "operations bridge rejected the request; no partial roster was published",
        ));
    }
    if accepted != 1 || code != 0 {
        return Err(invalid("inconsistent operations acceptance fields"));
    }
    let manifest = JobsManifest {
        generation: value.number(6)?,
        df_version: value.text(7)?,
        dfhack_version: value.text(8)?,
    };
    if manifest.generation == 0 || manifest.generation == u64::MAX {
        return Err(invalid("invalid operations source generation"));
    }
    let payload = if observation {
        Some(value.bytes(9, limit)?)
    } else if value.0.contains_key(&9) {
        return Err(invalid("operations handshake carried an observation"));
    } else {
        None
    };
    Ok((manifest, payload))
}

/// Credentials intentionally have no Debug projection.
pub struct OperationsRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    limits: OperationsLimits,
    manifest: JobsManifest,
    read_method: i16,
    poisoned: bool,
}
impl<S: Read + Write> OperationsRpcClient<S> {
    pub fn negotiate(
        mut stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        limits: OperationsLimits,
    ) -> Result<Self> {
        credentials(&token, &nonce, limits)?;
        let mut hello = b"DFHack?\n".to_vec();
        hello.extend_from_slice(&1i32.to_le_bytes());
        stream.write_all(&hello).map_err(io_failure)?;
        stream.flush().map_err(io_failure)?;
        let mut reply = [0; 12];
        stream.read_exact(&mut reply).map_err(io_failure)?;
        if &reply[..8] != b"DFHack!\n" || reply[8..] != 1i32.to_le_bytes() {
            return Err(invalid("invalid native RPC handshake"));
        }
        let handshake = bind(&mut stream, "Handshake")?;
        let read_method = bind(&mut stream, "ReadObservation")?;
        if handshake == read_method {
            return Err(invalid("operations methods share an ID"));
        }
        let response = call(
            &mut stream,
            handshake,
            &request(&token, &nonce, limits),
            4096,
        )?;
        let (manifest, _) = decode(&response, &nonce, false, limits.payload_bytes)?;
        Ok(Self {
            stream,
            token,
            nonce,
            limits,
            manifest,
            read_method,
            poisoned: false,
        })
    }
    #[must_use]
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn fence(&mut self) {
        self.poisoned = true;
    }
    pub fn read_observation(&mut self) -> Result<LiveOperationsObservation> {
        if self.poisoned {
            return Err(DfmcpError::new(
                ErrorCode::AdapterUnavailable,
                "operations source is fenced; reopen session",
            ));
        }
        let result = (|| {
            let response = call(
                &mut self.stream,
                self.read_method,
                &request(&self.token, &self.nonce, self.limits),
                self.limits.payload_bytes.saturating_add(4096),
            )?;
            let (manifest, payload) =
                decode(&response, &self.nonce, true, self.limits.payload_bytes)?;
            if manifest.generation < self.manifest.generation
                || manifest.df_version != self.manifest.df_version
                || manifest.dfhack_version != self.manifest.dfhack_version
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "operations source version or generation changed incompatibly",
                ));
            }
            let result = LiveOperationsObservation::decode_payload(
                payload.ok_or_else(|| invalid("missing operations payload"))?,
                manifest.generation,
                manifest.df_version.clone(),
                manifest.dfhack_version.clone(),
            )?;
            self.limits.check(&result)?;
            self.manifest = manifest;
            Ok(result)
        })();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
}
impl OperationsRpcClient<DeadlineStream> {
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        timeout: Duration,
        limits: OperationsLimits,
    ) -> Result<Self> {
        credentials(&token, &nonce, limits)?;
        checked_timeout(timeout)?;
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(DfmcpError::new(
                ErrorCode::CapabilityDenied,
                "operations endpoint must be numeric loopback with a nonzero port",
            ));
        }
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| invalid("operations deadline overflow"))?;
        let stream = TcpStream::connect_timeout(&endpoint, timeout).map_err(io_failure)?;
        stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream { stream, deadline }, token, nonce, limits)
    }
    pub fn refresh(&mut self, timeout: Duration) -> Result<LiveOperationsObservation> {
        self.stream.deadline = Instant::now()
            .checked_add(checked_timeout(timeout)?)
            .ok_or_else(|| invalid("operations deadline overflow"))?;
        self.read_observation()
    }
}

#[cfg(test)]
mod tests {
    use super::super::header;
    use super::*;
    use std::io::{self, Cursor};
    struct Script {
        input: Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl Read for Script {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.input.read(out)
        }
    }
    impl Write for Script {
        fn write(&mut self, data: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(data);
            Ok(data.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn reply(minor: u64, payload: Option<&[u8]>) -> Vec<u8> {
        let mut value = Vec::new();
        for (f, n) in [(1, 1), (2, 0), (4, 1), (5, minor), (6, 7)] {
            number(&mut value, f, n);
        }
        bytes(&mut value, 3, &[b'n'; 16]);
        bytes(&mut value, 7, b"df");
        bytes(&mut value, 8, b"dfhack");
        if let Some(payload) = payload {
            bytes(&mut value, 9, payload);
        }
        value
    }
    fn frame(value: &[u8]) -> Vec<u8> {
        let mut result = header(-1, value.len() as i32).to_vec();
        result.extend_from_slice(value);
        result
    }
    fn input(payload: &[u8]) -> Vec<u8> {
        let mut input = b"DFHack!\n".to_vec();
        input.extend_from_slice(&1i32.to_le_bytes());
        for id in [2, 3] {
            let mut value = Vec::new();
            number(&mut value, 1, id);
            input.extend(frame(&value));
        }
        input.extend(frame(&reply(3, None)));
        input.extend(frame(&reply(3, Some(payload))));
        input
    }
    fn golden() -> Result<Vec<u8>> {
        let value = include_str!("../tests/fixtures/operations_v1_3.hex").trim();
        (0..value.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&value[i..i + 2], 16)
                    .map_err(|_| invalid("bad operations golden hex"))
            })
            .collect()
    }
    #[test]
    fn closed_transport_decodes_native_golden_and_fences_failures() -> Result<()> {
        let payload = golden()?;
        let mut client = OperationsRpcClient::negotiate(
            Script {
                input: Cursor::new(input(&payload)),
                output: Vec::new(),
            },
            vec![b't'; 32],
            vec![b'n'; 16],
            OperationsLimits::default(),
        )?;
        let observation = client.read_observation()?;
        assert_eq!(observation.items.len(), 3);
        assert_eq!(observation.attachments.len(), 1);
        assert!(!client.poisoned());
        assert!(client.read_observation().is_err());
        assert!(client.poisoned());
        let written = client.stream.output.len();
        assert!(client.read_observation().is_err());
        assert_eq!(written, client.stream.output.len());
        Ok(())
    }
    #[test]
    fn profile_nonce_and_handshake_payload_cannot_be_confused() {
        assert!(decode(&reply(2, None), &[b'n'; 16], false, 1024).is_err());
        assert!(decode(&reply(3, None), &[b'x'; 16], false, 1024).is_err());
        assert!(decode(&reply(3, Some(b"x")), &[b'n'; 16], false, 1024).is_err());
        assert!(decode(&reply(3, None), &[b'n'; 16], true, 1024).is_err());
    }
    #[test]
    fn negotiated_counts_are_enforced_after_decoding() -> Result<()> {
        let mut client = OperationsRpcClient::negotiate(
            Script {
                input: Cursor::new(input(&golden()?)),
                output: Vec::new(),
            },
            vec![b't'; 32],
            vec![b'n'; 16],
            OperationsLimits {
                items: 1,
                ..OperationsLimits::default()
            },
        )?;
        assert!(matches!(client.read_observation(),Err(e) if e.code==ErrorCode::BudgetExceeded));
        assert!(client.poisoned());
        Ok(())
    }
    #[test]
    fn endpoint_credentials_and_budget_are_checked_before_connecting() {
        let endpoint = SocketAddr::from(([192, 0, 2, 1], 5000));
        assert!(
            OperationsRpcClient::connect(
                endpoint,
                vec![b't'; 32],
                vec![b'n'; 16],
                Duration::from_millis(1),
                OperationsLimits::default()
            )
            .is_err()
        );
        assert!(credentials(b"bad", &[b'n'; 16], OperationsLimits::default()).is_err());
        assert!(
            OperationsLimits {
                items: 0,
                ..OperationsLimits::default()
            }
            .validate()
            .is_err()
        );
    }
}
