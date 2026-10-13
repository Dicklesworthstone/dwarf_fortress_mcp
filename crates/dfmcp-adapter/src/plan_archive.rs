//! Immutable original-plan custody, separate from every native effect journal.
//!
//! Only an operator-chosen storage handle/path can open an archive. Current
//! Query authority is required for reads and Plan authority for publication.
//! Successful recovery supplies original data, never current dispatch authority.
//! Existing complete files are read-only; no overwrite, repair or truncation.

use std::io::{self, SeekFrom};
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier,
    SessionId, StateAnchor};
use dfmcp_intent::PreparedPlan;
use dfmcp_intent::plan_archive::{self as codec, ArchiveLimits, PlanArchiveSeal};

use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;
use crate::semantic_work_orders::{SemanticWorkOrderReview, SemanticWorkOrderSession};
use crate::work_order_control::WorkOrderSource;

fn denied(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CapabilityDenied, message)
}
fn corrupt(_: io::Error) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger,
        "original-plan archive storage failed or changed; retain bytes and reopen without repair")
}
fn exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "original-plan archive operation budget exhausted")
}
fn mismatch() -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, "original-plan archive bytes or extent changed")
}

struct Call {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}
impl Call {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        if context.session_id == SessionId::NIL { return Err(denied("archive owner session is missing")); }
        let until = Instant::now().checked_add(Duration::from_millis(
            context.budget.max_wall_millis.min(60_000))).ok_or_else(exhausted)?;
        let call = Self { context: context.clone(), until, bytes: context.budget.max_bytes };
        call.check()?;
        Ok(call)
    }
    fn check(&self) -> Result<()> {
        self.context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if Instant::now() >= self.until { return Err(exhausted()); }
        Ok(())
    }
    fn original(&self, anchor: StateAnchor, write: bool) -> Result<()> {
        self.check()?;
        if anchor.fortress_id == dfmcp_core::FortressId::NIL
            || anchor.fortress_id != self.context.anchor.fortress_id
        { return Err(denied("archive belongs to another fortress")); }
        // Do not substitute the archived tick for the current authority tick.
        if self.context.anchor.cursor.epoch < anchor.cursor.epoch
            || (self.context.anchor.cursor.epoch == anchor.cursor.epoch
                && (self.context.anchor.tick < anchor.tick
                    || self.context.anchor.cursor.sequence < anchor.cursor.sequence
                    || (self.context.anchor.cursor == anchor.cursor
                        && self.context.anchor != anchor)))
        {
            return Err(DfmcpError::new(ErrorCode::StaleAnchor,
                "archive cannot be read under a context older than its original epoch/tick"));
        }
        if write { self.context.authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)?; }
        Ok(())
    }
    fn ensure(&self, bytes: u64) -> Result<()> {
        self.check()?;
        if self.bytes <= bytes { return Err(exhausted()); }
        Ok(())
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.ensure(bytes)?;
        self.bytes -= bytes;
        Ok(())
    }
    fn codec(&mut self, limits: ArchiveLimits) -> Result<()> {
        limits.validate()?;
        // Reserve four profile-sized byte units before codec work. This policy
        // reservation is not an exact count of internal copies, CPU operations,
        // or allocator peak memory; the codec's hard node/depth caps also apply.
        self.charge((limits.max_bytes as u64).checked_mul(4).ok_or_else(exhausted)?)
    }
    fn current(&self) -> Result<OperationContext> {
        self.check()?;
        let millis = self.until.checked_duration_since(Instant::now()).ok_or_else(exhausted)?.as_millis();
        if millis == 0 { return Err(exhausted()); }
        let mut context = self.context.clone();
        context.budget.max_bytes = self.bytes;
        context.budget.max_wall_millis = u64::try_from(millis).map_err(|_| exhausted())?;
        Ok(context)
    }
    fn after_open(&mut self, context: &OperationContext) -> Result<()> {
        let until = Instant::now().checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        self.until = self.until.min(until);
        self.bytes = self.bytes.min(context.budget.max_bytes);
        self.check()
    }
}

/// Non-cloneable owner. A verified cache cannot be read after storage corruption;
/// recovery verifies every byte again and returns no mutation/observation receipt.
pub struct PlanArchiveStore<S> {
    storage: S,
    raw: Vec<u8>,
    seal: PlanArchiveSeal,
    anchor: StateAnchor,
    session: SessionId,
    limits: ArchiveLimits,
    authority_anchor: StateAnchor,
    fenced: bool,
}
pub type PrivatePlanArchive = PlanArchiveStore<PrivateBuildFile>;

fn prepare(plan: &PreparedPlan, limits: ArchiveLimits, call: &mut Call)
    -> Result<(Vec<u8>, PlanArchiveSeal)>
{
    call.original(plan.anchor, true)?;
    if plan.steps.len() > call.context.budget.max_entities as usize { return Err(exhausted()); }
    call.codec(limits)?;
    let result = codec::encode_with_check(plan, limits, || call.check())?;
    call.ensure(2 * result.0.len() as u64)?;
    call.original(plan.anchor, true)?;
    Ok(result)
}

impl<S: EffectJournalStorage> PlanArchiveStore<S> {
    /// Caller must have exclusively created this empty storage. Consumes the
    /// handle on failure; complete/torn surviving bytes are never overwritten.
    pub fn create(storage: S, plan: &PreparedPlan, limits: ArchiveLimits,
        context: &OperationContext) -> Result<Self>
    {
        let mut call = Call::new(context)?;
        let (raw, seal) = prepare(plan, limits, &mut call)?;
        Self::publish(storage, raw, seal, plan.anchor, limits, &mut call)
    }
    fn publish(mut storage: S, raw: Vec<u8>, seal: PlanArchiveSeal,
        anchor: StateAnchor, limits: ArchiveLimits, call: &mut Call) -> Result<Self>
    {
        call.original(anchor, true)?;
        call.charge(2 * raw.len() as u64)?;
        storage.validate_identity().map_err(corrupt)?;
        if storage.seek(SeekFrom::End(0)).map_err(corrupt)? != 0 {
            return Err(DfmcpError::new(ErrorCode::Conflict, "archive publication requires empty exclusive storage"));
        }
        call.original(anchor, true)?;
        storage.seek(SeekFrom::Start(0)).map_err(corrupt)?;
        for chunk in raw.chunks(4096) {
            call.original(anchor, true)?;
            storage.write_all(chunk).map_err(corrupt)?;
        }
        call.original(anchor, true)?;
        storage.flush().and_then(|_| storage.sync()).map_err(corrupt)?;
        Self::verify_bytes(&mut storage, &raw, call)?;
        call.original(anchor, true)?;
        Ok(Self { storage, raw, seal, anchor, session: call.context.session_id, limits, authority_anchor: call.context.anchor, fenced: false })
    }
    /// Existing bytes are read-only, even after a lost publication acknowledgment.
    /// Both identities come from the selected original handoff, not the file.
    pub fn open(storage: S, expected: PlanArchiveSeal, limits: ArchiveLimits,
        context: &OperationContext) -> Result<Self>
    {
        Self::read(storage, expected, limits, &mut Call::new(context)?)
    }
    fn read(mut storage: S, expected: PlanArchiveSeal, limits: ArchiveLimits,
        call: &mut Call) -> Result<Self>
    {
        let limits = ArchiveLimits {
            max_steps: limits.max_steps.min(call.context.budget.max_entities as usize), ..limits
        };
        call.codec(limits)?;
        storage.validate_identity().map_err(corrupt)?;
        let length = storage.seek(SeekFrom::End(0)).map_err(corrupt)?;
        if length > codec::MAX_ARCHIVE_BYTES as u64 || length > limits.max_bytes as u64 {
            return Err(exhausted());
        }
        if length == 0 { return Err(mismatch()); }
        call.charge(2 * length)?;
        let mut raw = Vec::new();
        raw.try_reserve_exact(length as usize).map_err(|_| exhausted())?;
        raw.resize(length as usize, 0);
        storage.seek(SeekFrom::Start(0)).map_err(corrupt)?;
        for chunk in raw.chunks_mut(4096) { call.check()?; storage.read_exact(chunk).map_err(corrupt)?; }
        let plan = codec::decode_with_check(&raw, expected, limits, || call.check())?;
        if plan.steps.len() > call.context.budget.max_entities as usize { return Err(exhausted()); }
        call.original(plan.anchor, false)?;
        Self::verify_bytes(&mut storage, &raw, call)?;
        call.original(plan.anchor, false)?;
        Ok(Self { storage, raw, seal: expected, anchor: plan.anchor,
            session: call.context.session_id, limits, authority_anchor: call.context.anchor, fenced: false })
    }
    fn verify_bytes(storage: &mut S, expected: &[u8], call: &Call) -> Result<()> {
        call.check()?;
        storage.validate_identity().map_err(corrupt)?;
        if storage.seek(SeekFrom::End(0)).map_err(corrupt)? != expected.len() as u64 { return Err(mismatch()); }
        storage.seek(SeekFrom::Start(0)).map_err(corrupt)?;
        let mut buffer = [0; 4096];
        for chunk in expected.chunks(buffer.len()) {
            call.check()?;
            storage.read_exact(&mut buffer[..chunk.len()]).map_err(corrupt)?;
            if &buffer[..chunk.len()] != chunk { return Err(mismatch()); }
        }
        storage.validate_identity().map_err(corrupt)?;
        if storage.seek(SeekFrom::End(0)).map_err(corrupt)? != expected.len() as u64 { return Err(mismatch()); }
        call.check()
    }
    fn recover_in(&mut self, call: &mut Call) -> Result<PreparedPlan> {
        if self.fenced { return Err(mismatch()); }
        if call.context.session_id != self.session { return Err(denied("archive belongs to another session opening")); }
        call.original(self.anchor, false)?;
        call.original(self.authority_anchor, false)?;
        self.authority_anchor = call.context.anchor;
        call.codec(self.limits)?;
        call.charge(2 * self.raw.len() as u64)?;
        let outcome: Result<PreparedPlan> = (|| {
            Self::verify_bytes(&mut self.storage, &self.raw, call)?;
            let limits = ArchiveLimits {
                max_steps: self.limits.max_steps.min(call.context.budget.max_entities as usize), ..self.limits
            };
            let plan = codec::decode_with_check(&self.raw, self.seal, limits, || call.check())?;
            if plan.steps.len() > call.context.budget.max_entities as usize { return Err(exhausted()); }
            Self::verify_bytes(&mut self.storage, &self.raw, call)?;
            call.original(plan.anchor, false)?;
            Ok(plan)
        })();
        if outcome.as_ref().is_err_and(|e| e.code == ErrorCode::CorruptLedger) { self.fenced = true; }
        outcome
    }
    pub fn recover(&mut self, context: &OperationContext) -> Result<PreparedPlan> {
        self.recover_in(&mut Call::new(context)?)
    }
    pub fn seal(&self) -> PlanArchiveSeal { self.seal }
    pub fn is_fenced(&self) -> bool { self.fenced }

    /// Use recovered original content in the existing native association check.
    /// The shared deadline and remaining byte allowance survive the handoff.
    /// Does NOT prepare, insert, reconcile, renew expiry or start goal monitoring.
    pub fn reattach_work_order<A: EffectJournalStorage, B: EffectJournalStorage, N: WorkOrderSource>(
        &mut self, coordinator: &mut SemanticWorkOrderSession<A, B, N>,
        context: &OperationContext,
    ) -> Result<SemanticWorkOrderReview> {
        let mut call = Call::new(context)?;
        let plan = self.recover_in(&mut call)?;
        coordinator.reattach(plan, &call.current()?)
    }
}

/// Persist before native preparation. The full archive and output seal are
/// validated/reserved before exclusive path creation. Never replace an old file.
pub fn create_private_plan_archive(path: &Path, plan: &PreparedPlan, limits: ArchiveLimits,
    context: &OperationContext) -> Result<PrivatePlanArchive>
{
    let mut call = Call::new(context)?;
    let (raw, seal) = prepare(plan, limits, &mut call)?;
    let (storage, created, remaining) = open_private_storage(path, &call.current()?,
        BuildMode::Control, true, limits.max_bytes)?;
    if !created { return Err(DfmcpError::new(ErrorCode::Conflict, "original-plan archive already exists; recover it without overwriting")); }
    call.after_open(&remaining)?;
    PlanArchiveStore::publish(storage, raw, seal, plan.anchor, limits, &mut call)
}

/// Operator path only: exact 0700 parent, single-link 0600 regular file, no
/// symlinks, exclusive descriptor-pinned custody. Unsupported platforms refuse.
/// No write, flush, sync, truncation, repair, creation or native contact on read.
pub fn open_private_plan_archive(path: &Path, expected: PlanArchiveSeal, limits: ArchiveLimits,
    context: &OperationContext) -> Result<PrivatePlanArchive>
{
    limits.validate()?;
    let mut call = Call::new(context)?;
    call.ensure((limits.max_bytes as u64).checked_mul(6).ok_or_else(exhausted)?)?;
    let (storage, created, remaining) = open_private_storage(path, &call.current()?,
        BuildMode::Offline, false, limits.max_bytes)?;
    if created { return Err(mismatch()); }
    call.after_open(&remaining)?;
    PlanArchiveStore::read(storage, expected, limits, &mut call)
}
