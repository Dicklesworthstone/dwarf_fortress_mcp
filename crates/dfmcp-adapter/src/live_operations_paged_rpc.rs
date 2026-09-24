//! Closed operations/1.4 acquisition: one capture, immutable pages, verified
//! whole payload, explicit release. No partial semantic world is ever returned.

#[path = "retained_snapshot.rs"]
pub mod snapshot;

use self::snapshot::{
    MAX_PAGE_BYTES, MIN_PAGE_BYTES, SnapshotAssembler, SnapshotManifest, SnapshotPage,
};
use crate::live_jobs_rpc::{
    DeadlineStream, Message, bytes, call, checked_timeout, failure, io_failure, malformed, number,
};
use crate::live_operations::{LiveOperationsObservation, OperationsProfile};
use dfmcp_core::{Digest32, ErrorCode, Result};
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PagedOperationsLimits {
    pub jobs: u32,
    pub buildings: u32,
    pub items: u32,
    pub payload_bytes: usize,
    pub page_bytes: usize,
}
impl Default for PagedOperationsLimits {
    fn default() -> Self {
        Self {
            jobs: 4096,
            buildings: 4096,
            items: 65536,
            payload_bytes: OperationsProfile::PagedV1_4.maximum_bytes(),
            page_bytes: 64 * 1024,
        }
    }
}
impl PagedOperationsLimits {
    pub fn validate(self) -> Result<()> {
        if !(1..=4096).contains(&self.jobs)
            || !(1..=4096).contains(&self.buildings)
            || !(1..=65536).contains(&self.items)
            || !(1024..=OperationsProfile::PagedV1_4.maximum_bytes()).contains(&self.payload_bytes)
            || !(MIN_PAGE_BYTES..=MAX_PAGE_BYTES).contains(&self.page_bytes)
        {
            return Err(failure(
                ErrorCode::BudgetExceeded,
                "invalid operations/1.4 acquisition bounds",
            ));
        }
        Ok(())
    }
    pub fn entity_limit(self) -> u32 {
        1u32.saturating_add(self.jobs)
            .saturating_add(self.buildings)
            .saturating_add(self.items)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SourceManifest {
    generation: u64,
    df: String,
    dfhack: String,
}
fn envelope<'a>(data: &'a [u8], nonce: &[u8]) -> Result<(Message<'a>, SourceManifest)> {
    let message = Message::parse(data, 14)?;
    if message.number(4)? != 1 || message.number(5)? != 4 {
        return Err(failure(
            ErrorCode::VersionMismatch,
            "native paging requires exactly operations/1.4",
        ));
    }
    if message.bytes(3, 64)? != nonce {
        return Err(malformed());
    }
    let accepted = message.number(1)?;
    let code = message.number(2)?;
    if accepted == 0 && code != 0 {
        if (9..=14).any(|field| message.0.contains_key(&field)) {
            return Err(malformed());
        }
        return Err(failure(
            match code {
                1 => ErrorCode::CapabilityDenied,
                2 => ErrorCode::VersionMismatch,
                3 => ErrorCode::BudgetExceeded,
                4 => ErrorCode::FortressNotLoaded,
                6 => ErrorCode::StaleAnchor,
                _ => ErrorCode::AdapterFailure,
            },
            "operations/1.4 refused the snapshot request; no partial observation was published",
        ));
    }
    if accepted != 1 || code != 0 {
        return Err(malformed());
    }
    let source = SourceManifest {
        generation: message.number(6)?,
        df: message.text(7)?,
        dfhack: message.text(8)?,
    };
    if source.generation == 0 || source.generation == u64::MAX {
        return Err(malformed());
    }
    Ok((message, source))
}
fn bind<S: Read + Write>(stream: &mut S, name: &str) -> Result<i16> {
    let mut request = Vec::new();
    for (field, text) in [
        (1, name),
        (2, "dfmcp.operations.v1_4.Request"),
        (3, "dfmcp.operations.v1_4.Reply"),
        (4, "dfmcp_operations_v1_4"),
    ] {
        bytes(&mut request, field, text.as_bytes());
    }
    let reply = call(stream, 0, &request, 1024)?;
    let id = i16::try_from(Message::parse(&reply, 1)?.number(1)?).map_err(|_| malformed())?;
    if id < 2 {
        return Err(malformed());
    }
    Ok(id)
}
fn request(
    token: &[u8],
    nonce: &[u8],
    limits: PagedOperationsLimits,
    snapshot: &[u8],
    offset: usize,
    release: bool,
) -> Vec<u8> {
    let mut out = Vec::new();
    bytes(&mut out, 1, token);
    bytes(&mut out, 2, nonce);
    for (field, value) in [
        (3, 1),
        (4, 4),
        (5, u64::from(limits.jobs)),
        (6, u64::from(limits.buildings)),
        (7, u64::from(limits.items)),
        (8, limits.payload_bytes as u64),
    ] {
        number(&mut out, field, value);
    }
    if !snapshot.is_empty() {
        bytes(&mut out, 9, snapshot);
    }
    number(&mut out, 10, offset as u64);
    number(&mut out, 11, limits.page_bytes as u64);
    number(&mut out, 12, u64::from(release));
    out
}
fn page(data: &[u8], nonce: &[u8], maximum: usize) -> Result<SnapshotPage> {
    let (message, source) = envelope(data, nonce)?;
    let complete = match message.number(14)? {
        0 => false,
        1 => true,
        _ => return Err(malformed()),
    };
    Ok(SnapshotPage {
        manifest: SnapshotManifest {
            token: message.bytes(10, 16)?.try_into().map_err(|_| malformed())?,
            generation: source.generation,
            df_version: source.df,
            dfhack_version: source.dfhack,
            total_bytes: usize::try_from(message.number(12)?).map_err(|_| malformed())?,
            payload_digest: Digest32::from_bytes(
                message.bytes(13, 32)?.try_into().map_err(|_| malformed())?,
            ),
        },
        offset: usize::try_from(message.number(11)?).map_err(|_| malformed())?,
        bytes: message.bytes(9, maximum)?.to_vec(),
        complete,
    })
}

/// Credentials are owned, never formatted with Debug or reflected in failures.
pub struct PagedOperationsRpcClient<S> {
    stream: S,
    token: Vec<u8>,
    nonce: Vec<u8>,
    manifest: SourceManifest,
    read_method: i16,
    limits: PagedOperationsLimits,
    poisoned: bool,
    last_pages: u32,
}
impl<S: Read + Write> PagedOperationsRpcClient<S> {
    pub fn negotiate(
        mut stream: S,
        token: Vec<u8>,
        nonce: Vec<u8>,
        limits: PagedOperationsLimits,
    ) -> Result<Self> {
        limits.validate()?;
        if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
            return Err(failure(
                ErrorCode::InvalidRequest,
                "invalid operations paging credentials",
            ));
        }
        let mut hello = b"DFHack?\n".to_vec();
        hello.extend_from_slice(&1i32.to_le_bytes());
        stream.write_all(&hello).map_err(io_failure)?;
        stream.flush().map_err(io_failure)?;
        let mut response = [0; 12];
        stream.read_exact(&mut response).map_err(io_failure)?;
        if &response[..8] != b"DFHack!\n" || response[8..] != 1i32.to_le_bytes() {
            return Err(malformed());
        }
        let handshake = bind(&mut stream, "Handshake")?;
        let read_method = bind(&mut stream, "ReadObservation")?;
        if handshake == read_method {
            return Err(malformed());
        }
        let reply = call(
            &mut stream,
            handshake,
            &request(&token, &nonce, limits, &[], 0, false),
            4096,
        )?;
        let (message, manifest) = envelope(&reply, &nonce)?;
        if (9..=14).any(|field| message.0.contains_key(&field)) {
            return Err(malformed());
        }
        Ok(Self {
            stream,
            token,
            nonce,
            manifest,
            read_method,
            limits,
            poisoned: false,
            last_pages: 0,
        })
    }
    pub fn poisoned(&self) -> bool {
        self.poisoned
    }
    pub fn fence(&mut self) {
        self.poisoned = true;
    }
    pub fn last_page_count(&self) -> u32 {
        self.last_pages
    }

    pub fn read_observation(&mut self) -> Result<LiveOperationsObservation> {
        if self.poisoned {
            return Err(failure(
                ErrorCode::AdapterUnavailable,
                "paged source is fenced; reopen the session",
            ));
        }
        let result = self.acquire();
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    fn acquire(&mut self) -> Result<LiveOperationsObservation> {
        let mut assembly =
            SnapshotAssembler::new(self.limits.payload_bytes, self.limits.page_bytes)?;
        let mut pages = 0;
        // All pages, including release, share the transport's single deadline.
        while !assembly.complete() {
            if pages >= 1024 {
                return Err(failure(
                    ErrorCode::BudgetExceeded,
                    "native snapshot page count exceeded",
                ));
            }
            let snapshot = assembly.manifest().map_or(&[][..], |v| v.token.as_slice());
            let request = request(
                &self.token,
                &self.nonce,
                self.limits,
                snapshot,
                assembly.offset(),
                false,
            );
            let reply = call(
                &mut self.stream,
                self.read_method,
                &request,
                self.limits.page_bytes + 4096,
            )?;
            let page = page(&reply, &self.nonce, self.limits.page_bytes)?;
            if page.manifest.df_version != self.manifest.df
                || page.manifest.dfhack_version != self.manifest.dfhack
                || page.manifest.generation < self.manifest.generation
            {
                return Err(failure(
                    ErrorCode::StaleAnchor,
                    "native paging software or generation regressed",
                ));
            }
            assembly.push(page)?;
            pages += 1;
        }
        let (manifest, payload) = assembly.finish()?;
        let observation = LiveOperationsObservation::decode_profile(
            &payload,
            manifest.generation,
            manifest.df_version.clone(),
            manifest.dfhack_version.clone(),
            OperationsProfile::PagedV1_4,
        )?;
        if observation.jobs.jobs.len() > self.limits.jobs as usize
            || observation.buildings.len() > self.limits.buildings as usize
            || observation.items.len() > self.limits.items as usize
        {
            return Err(failure(
                ErrorCode::BudgetExceeded,
                "native snapshot exceeds requested roster counts",
            ));
        }
        let release = request(
            &self.token,
            &self.nonce,
            self.limits,
            &manifest.token,
            0,
            true,
        );
        let reply = call(&mut self.stream, self.read_method, &release, 4096)?;
        let (ack, source) = envelope(&reply, &self.nonce)?;
        if ack.bytes(10, 16)? != manifest.token.as_slice()
            || ack.0.contains_key(&9)
            || (11..=14).any(|field| ack.0.contains_key(&field))
            || source.generation != manifest.generation
            || source.df != manifest.df_version
            || source.dfhack != manifest.dfhack_version
        {
            return Err(malformed());
        }
        self.manifest = source;
        self.last_pages = pages;
        Ok(observation)
    }
}
impl PagedOperationsRpcClient<DeadlineStream> {
    pub fn connect(
        endpoint: SocketAddr,
        token: Vec<u8>,
        nonce: Vec<u8>,
        timeout: Duration,
        limits: PagedOperationsLimits,
    ) -> Result<Self> {
        limits.validate()?;
        if !(32..=256).contains(&token.len()) || !(16..=64).contains(&nonce.len()) {
            return Err(failure(
                ErrorCode::InvalidRequest,
                "invalid operations paging credentials",
            ));
        }
        if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
            return Err(failure(
                ErrorCode::CapabilityDenied,
                "native paging endpoint must be numeric loopback",
            ));
        }
        let deadline = Instant::now()
            .checked_add(checked_timeout(timeout)?)
            .ok_or_else(|| failure(ErrorCode::BudgetExceeded, "native paging deadline overflow"))?;
        let stream = TcpStream::connect_timeout(&endpoint, timeout).map_err(io_failure)?;
        stream.set_nodelay(true).map_err(io_failure)?;
        Self::negotiate(DeadlineStream { stream, deadline }, token, nonce, limits)
    }
    pub fn refresh(&mut self, timeout: Duration) -> Result<LiveOperationsObservation> {
        self.stream.deadline = Instant::now()
            .checked_add(checked_timeout(timeout)?)
            .ok_or_else(|| failure(ErrorCode::BudgetExceeded, "native paging deadline overflow"))?;
        self.read_observation()
    }
}

#[cfg(test)]
#[path = "live_operations_paged_rpc_tests.rs"]
mod tests;
