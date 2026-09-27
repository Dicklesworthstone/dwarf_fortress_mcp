//! Durable custody of one complete furniture batch and one permanent local stop.
//!
//! The parent definition links the original placement journal; child effects
//! remain in that unchanged journal. Reopening never repairs bytes or restores
//! dispatch permission. A stop prevents future batch advancement, not game work.
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, SessionId,
};

use super::BatchDefinition;
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;

pub const MAX_STORE_BYTES: usize = 64 * 1024;
const MAGIC: &[u8; 8] = b"DFMFBJ01";
const HEADER_END: &[u8; 8] = b"DFMFBHE1";
const STOP: &[u8; 8] = b"DFMFBST1";
const STOP_END: &[u8; 8] = b"DFMFBSE1";
const HEADER_OVERHEAD: usize = 8 + 4 + 32 + 8;
const STOP_BYTES: usize = 8 + 32 + 32 + 32 + 8;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "furniture batch parent custody or bytes changed; reopen original evidence without repair",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "furniture batch storage allowance exhausted",
    )
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = Vec::with_capacity(domain.len() + bytes.len());
    input.extend_from_slice(domain);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}
fn authorize(context: &OperationContext, definition: Option<&BatchDefinition>) -> Result<()> {
    if definition
        .is_some_and(|d| d.binding().fortress().fortress_id() != context.anchor.fortress_id)
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "furniture batch belongs to another fortress",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
}

struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(context: &OperationContext) -> Result<Self> {
        authorize(context, None)?;
        if context.budget.max_wall_millis > 60_000 {
            return Err(bounded());
        }
        let work = Self {
            context: context.clone(),
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(bounded)?,
            bytes: context.budget.max_bytes,
        };
        work.check()?;
        Ok(work)
    }
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(bounded());
        }
        authorize(&self.context, None)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.check()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(bounded)?;
        Ok(())
    }
    fn reserve(&self, bytes: usize) -> Result<()> {
        self.check()?;
        if self.bytes < bytes as u64 {
            return Err(bounded());
        }
        Ok(())
    }
}

fn read<S: EffectJournalStorage>(storage: &mut S, work: &mut Work) -> Result<Vec<u8>> {
    work.check()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    require(length <= MAX_STORE_BYTES as u64)?;
    work.charge(length as usize)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut bytes = vec![0; length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        work.check()?;
        let read = storage.read(&mut bytes[offset..]).map_err(|_| corrupt())?;
        require(read > 0)?;
        offset += read;
    }
    work.check()?;
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.check()?;
    Ok(bytes)
}

fn header(definition: &BatchDefinition) -> Result<Vec<u8>> {
    let body = definition.canonical_bytes();
    require(!body.is_empty() && body.len() <= MAX_STORE_BYTES - HEADER_OVERHEAD - STOP_BYTES)?;
    let mut bytes = Vec::with_capacity(body.len() + HEADER_OVERHEAD);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(&(body.len() as u32).to_be_bytes());
    bytes.extend_from_slice(&body);
    let digest = hash(b"dfmcp-furniture-batch-store/1\0", &bytes);
    bytes.extend_from_slice(digest.as_bytes());
    bytes.extend_from_slice(HEADER_END);
    Ok(bytes)
}
fn stop_frame(definition: &BatchDefinition, header: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(STOP_BYTES);
    bytes.extend_from_slice(STOP);
    bytes.extend_from_slice(definition.id().as_bytes());
    bytes.extend_from_slice(hash(b"dfmcp-furniture-batch-parent/1\0", header).as_bytes());
    let digest = hash(b"dfmcp-furniture-batch-stop/1\0", &bytes);
    bytes.extend_from_slice(digest.as_bytes());
    bytes.extend_from_slice(STOP_END);
    bytes
}
fn decode(bytes: &[u8]) -> Result<(BatchDefinition, bool)> {
    require(bytes.len() >= HEADER_OVERHEAD && bytes.len() <= MAX_STORE_BYTES)?;
    require(bytes.get(..8) == Some(MAGIC.as_slice()))?;
    let length: [u8; 4] = bytes
        .get(8..12)
        .ok_or_else(corrupt)?
        .try_into()
        .map_err(|_| corrupt())?;
    let length = u32::from_be_bytes(length) as usize;
    require(length > 0 && length <= MAX_STORE_BYTES - HEADER_OVERHEAD - STOP_BYTES)?;
    let definition = BatchDefinition::decode(bytes.get(12..12 + length).ok_or_else(corrupt)?)
        .map_err(|_| corrupt())?;
    let expected = header(&definition)?;
    require(bytes.get(..expected.len()) == Some(expected.as_slice()))?;
    let stopped = bytes.len() != expected.len();
    if stopped {
        require(
            bytes.get(expected.len()..) == Some(stop_frame(&definition, &expected).as_slice()),
        )?;
    }
    Ok((definition, stopped))
}

/// One owner retains the exact expected file bytes. Call `verify` before using
/// cached definition/stop information as current custody evidence.
pub struct BatchStore<S> {
    storage: S,
    definition: BatchDefinition,
    raw: Vec<u8>,
    mode: BuildMode,
    owner: SessionId,
    stopped: bool,
    fenced: bool,
}
impl BatchStore<PrivateBuildFile> {
    /// Identity of the already held original parent. Full byte custody is
    /// established separately by verify, including any permanent stop marker.
    pub fn private_identity(
        &self,
        context: &OperationContext,
    ) -> Result<crate::build_placement::journal::private_file::PrivateFileIdentity> {
        self.access(context)?;
        self.storage.private_identity(context)
    }
}
impl<S: EffectJournalStorage> BatchStore<S> {
    /// `create` is valid only for an exclusively created empty Control store.
    /// The file shell, rather than retained bytes or client input, supplies it.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: BuildMode,
        expected: Option<BatchDefinition>,
        create: bool,
    ) -> Result<Self> {
        authorize(context, expected.as_ref())?;
        if create {
            if mode != BuildMode::Control || expected.is_none() {
                return Err(error(
                    ErrorCode::CapabilityDenied,
                    "batch creation requires an exact Control definition",
                ));
            }
            context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        }
        let mut work = Work::new(context)?;
        let mut raw = read(&mut storage, &mut work)?;
        if create {
            require(raw.is_empty())?;
            let definition = expected.as_ref().ok_or_else(corrupt)?;
            raw = header(definition)?;
            // Account for complete validation and reserve write/readback before
            // any publication. Exhaustion cannot leave an acknowledged header.
            work.reserve(raw.len().saturating_mul(8))?;
            work.charge(raw.len().saturating_mul(4))?;
            let (decoded, stopped) = decode(&raw)?;
            require(!stopped && decoded.canonical_bytes() == definition.canonical_bytes())?;
            work.check()?;
            context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
            storage.write_all(&raw).map_err(|_| corrupt())?;
            work.check()?;
            storage
                .flush()
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
            require(read(&mut storage, &mut work)? == raw)?;
        }
        work.charge(raw.len().saturating_mul(3))?;
        let (definition, stopped) = decode(&raw)?;
        if let Some(expected) = expected {
            require(expected.canonical_bytes() == definition.canonical_bytes())?;
        }
        authorize(context, Some(&definition))?;
        work.check()?;
        Ok(Self {
            storage,
            definition,
            raw,
            mode,
            owner: context.session_id,
            stopped,
            fenced: false,
        })
    }
    pub fn definition(&self) -> &BatchDefinition {
        &self.definition
    }
    /// A failed custody/publication check also closes local advancement. Only a
    /// successful reopened replay can distinguish that fence from a durable stop.
    pub fn stopped(&self) -> bool {
        self.stopped || self.fenced
    }
    /// A successfully published or replayed permanent stop, distinct from an
    /// unacknowledged local fence. Presentation must still verify current custody.
    pub fn durable_stopped(&self) -> bool {
        self.stopped
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    fn access(&self, context: &OperationContext) -> Result<()> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture batch has another session owner",
            ));
        }
        authorize(context, Some(&self.definition))?;
        if self.fenced {
            return Err(corrupt());
        }
        Ok(())
    }
    fn verify_work(&mut self, work: &mut Work) -> Result<()> {
        self.access(&work.context)?;
        match read(&mut self.storage, work) {
            Ok(bytes) if bytes == self.raw => Ok(()),
            Err(cause)
                if matches!(
                    cause.code,
                    ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
                ) =>
            {
                Err(cause)
            }
            _ => {
                self.fenced = true;
                Err(corrupt())
            }
        }
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context)?;
        self.verify_work(&mut Work::new(context)?)
    }
    /// Append one permanent stop using Query authority. Stopping never issues
    /// native cancellation and remains available after placement revocation.
    pub fn stop(&mut self, context: &OperationContext) -> Result<()> {
        self.access(context)?;
        if self.mode == BuildMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline batch custody cannot publish a stop",
            ));
        }
        let result = self.publish_stop(context);
        if result.is_err() {
            // An authorized stop request must not leave an old preparation
            // usable, even when the foreground allowance expired before write.
            self.fenced = true;
        }
        result
    }
    fn publish_stop(&mut self, context: &OperationContext) -> Result<()> {
        let mut work = Work::new(context)?;
        self.verify_work(&mut work)?;
        if self.stopped {
            return Ok(());
        }
        let frame = stop_frame(&self.definition, &self.raw);
        let mut proposed = self.raw.clone();
        proposed.extend_from_slice(&frame);
        require(proposed.len() <= MAX_STORE_BYTES)?;
        // Reserve complete validation, frame writing and final full readback
        // before the append. The readback itself charges the remaining bytes.
        work.reserve(proposed.len().saturating_mul(4) + frame.len())?;
        work.charge(proposed.len().saturating_mul(3) + frame.len())?;
        let (definition, stopped) = decode(&proposed)?;
        require(stopped && definition.canonical_bytes() == self.definition.canonical_bytes())?;
        work.check()?;
        self.access(&work.context)?;
        self.fenced = true;
        self.storage.validate_identity().map_err(|_| corrupt())?;
        require(
            self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == self.raw.len() as u64,
        )?;
        self.storage.write_all(&frame).map_err(|_| corrupt())?;
        work.check()?;
        self.storage
            .flush()
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        require(read(&mut self.storage, &mut work)? == proposed)?;
        authorize(&work.context, Some(&self.definition))?;
        work.check()?;
        self.raw = proposed;
        self.stopped = true;
        self.fenced = false;
        Ok(())
    }
}

/// Operator-selected private custody, sharing the unchanged placement file
/// implementation. Missing stores are created only for an explicit definition
/// in Control mode under current Query and Plan authority.
pub fn open_private_batch(
    path: &Path,
    context: &OperationContext,
    mode: BuildMode,
    expected: Option<BatchDefinition>,
) -> Result<BatchStore<PrivateBuildFile>> {
    authorize(context, expected.as_ref())?;
    let allow_create = mode == BuildMode::Control
        && expected.is_some()
        && context
            .authorize(Capability::Plan, RiskTier::Guarded, &[], None)
            .is_ok();
    let (storage, created, current) =
        open_private_storage(path, context, mode, allow_create, MAX_STORE_BYTES)?;
    BatchStore::open(storage, &current, mode, expected, created)
}

#[cfg(test)]
mod tests;
