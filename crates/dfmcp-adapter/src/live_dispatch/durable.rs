//! Durable ownership of one complete sealed live mutation plan.
//!
//! Family journals remain effect truth. This owner records preparation intent
//! before contacting a family and dispatch intent before a confirmed commit.
//! Replay restores history, never a native permit or local review. An opaque
//! family receipt is sequencing evidence, not proof of the original plan goal.

use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, CapabilityGrant, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext,
    Result, RiskTier, SessionId, StepId,
};
use dfmcp_intent::plan_archive::{self, ArchiveLimits, PlanArchiveSeal};
use dfmcp_intent::{PlanStep, PreparedPlan};

use super::{FamilyPreparation, FamilyReceipt, LiveDispatchCoordinator, LiveFamilyExecutor};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;
use crate::live_routing::{LiveFamily, RoutedStep};

mod authority;
mod wire;

pub const MAX_STEPS: usize = 16;
pub const MAX_PLAN_BYTES: usize = 256 * 1024;
pub const MAX_STORE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_FRAMES: usize = 4096;
const MAX_FRAME_BYTES: usize = 128;

fn limits() -> ArchiveLimits {
    ArchiveLimits {
        max_bytes: MAX_PLAN_BYTES,
        max_steps: MAX_STEPS,
        max_nodes: 4096,
        max_depth: 32,
    }
}
fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "live dispatch custody or family evidence changed; preserve original history and reopen without repair",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "durable live dispatch exhausted its original bounded operation or retention allowance",
    )
}
fn denied() -> DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "durable live dispatch lacks its retained and current scoped authority",
    )
}
fn conflict() -> DfmcpError {
    error(
        ErrorCode::Conflict,
        "original live dispatch frontier requires its exact current review or family recovery",
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

/// Keep this complete pin outside this journal, including after failures. A checksum found in an untrusted file
/// is not its own expected identity. Only complete validated descendants qualify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub journal_id: Digest32,
    pub head: Digest32,
    /// May exceed the last synchronized frame after a failed native read.
    /// Retain this field even when an operation returns an error.
    pub authority_tick_floor: u64,
}
impl Recovery {
    pub fn validate(self) -> Result<()> {
        if self.journal_id == Digest32::ZERO || self.head == Digest32::ZERO {
            return Err(error(
                ErrorCode::InvalidRequest,
                "live dispatch recovery requires its original nonzero journal identity and acknowledged head",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Pending,
    PrepareStarted,
    Prepared,
    DispatchStarted,
    CancelStarted,
    Indeterminate,
    Applied,
    Failed,
    Cancelled,
}
impl Phase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Applied | Self::Failed | Self::Cancelled)
    }
    pub fn uncertain(self) -> bool {
        matches!(
            self,
            Self::PrepareStarted
                | Self::DispatchStarted
                | Self::CancelStarted
                | Self::Indeterminate
        )
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::PrepareStarted => "prepare_started",
            Self::Prepared => "prepared",
            Self::DispatchStarted => "dispatch_started",
            Self::CancelStarted => "cancel_started",
            Self::Indeterminate => "indeterminate",
            Self::Applied => "applied",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub step: StepId,
    pub family: LiveFamily,
    pub idempotency_key: String,
    pub depends_on: Vec<StepId>,
    pub phase: Phase,
    pub preparation: Option<FamilyPreparation>,
    pub receipt: Option<FamilyReceipt>,
    /// A synchronized dispatch marker is never erased by later reconstruction.
    pub dispatch_attempted: bool,
}

/// Only a successful preparation in this live owner creates this review. The
/// head and session are included in its seal; replay deliberately restores none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub step: StepId,
    pub family: LiveFamily,
    pub preparation: FamilyPreparation,
    pub seal: Digest32,
    pub head: Digest32,
}

#[derive(Clone, Debug)]
pub struct View {
    pub recovery: Recovery,
    pub plan_digest: Digest32,
    pub original_object_digest: Digest32,
    pub mode: BuildMode,
    pub high_tick: u64,
    pub entries: Vec<Entry>,
    pub review: Option<Review>,
    pub next_ready: Option<StepId>,
}
impl View {
    pub fn all_applied(&self) -> bool {
        self.entries
            .iter()
            .all(|entry| entry.phase == Phase::Applied)
    }
    pub const fn original_goal_proven(&self) -> bool {
        false
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.entries.iter().any(|entry| entry.phase.uncertain())
    }
}

/// Exact original family custody, independently checked by the concrete owner.
/// Absence alone is never AbsentBeforeDispatch. Prepared means proven native
/// preparation with no reusable process-local commit permit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveredFamily {
    AbsentBeforeDispatch,
    Prepared(FamilyPreparation),
    Applied(FamilyReceipt),
    Failed(Option<FamilyReceipt>),
    Cancelled(Option<FamilyReceipt>),
    Indeterminate,
}

/// Recovery has no prepare/commit fallback. Implementations must verify exact
/// original semantic and native journals, keys and family semantics. Preparation
/// identities bind their native journal incarnation; receipt identities bind
/// that same semantic association and strictly validated native result.
/// Before every new effect, a concrete executor must recheck actual predecessor
/// family custody and the original selected-step postconditions/obligations.
pub trait DurableFamilyExecutor: LiveFamilyExecutor {
    fn recover_step(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        route: &RoutedStep,
        context: &OperationContext,
    ) -> Result<RecoveredFamily>;
    /// Every consumed native clock narrows authority even when a later read or
    /// effect fails. This getter grants no new evidence or mutation permission.
    fn observed_tick_floor(&self) -> u64 {
        0
    }
    /// Withdraw process-local effect permission after a higher-level failure.
    /// This must not erase durable family custody, receipts or consumed clocks.
    fn abandon_preparation(&mut self) {}
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Event {
    Prepare(StepId),
    Prepared(StepId, Digest32),
    Dispatch(StepId),
    Applied(StepId, Digest32),
    Failed(StepId, Option<Digest32>),
    Cancelled(StepId, Option<Digest32>),
    Absent(StepId),
    RecoveredPrepared(StepId, Digest32),
    Unknown(StepId),
    Cancel(StepId),
    Floor,
}

fn entry_index(entries: &[Entry], step: StepId) -> Result<usize> {
    entries
        .iter()
        .position(|entry| entry.step == step)
        .ok_or_else(conflict)
}
fn active(entries: &[Entry]) -> Option<usize> {
    entries
        .iter()
        .position(|entry| entry.phase != Phase::Pending && !entry.phase.terminal())
}
fn next(entries: &[Entry]) -> Result<Option<StepId>> {
    if active(entries).is_some() {
        return Ok(None);
    }
    for entry in entries {
        if entry.phase == Phase::Pending
            && entry.depends_on.iter().all(|id| {
                entries
                    .iter()
                    .any(|prior| prior.step == *id && prior.phase == Phase::Applied)
            })
        {
            return Ok(Some(entry.step));
        }
    }
    Ok(None)
}
fn transition(entries: &[Entry], event: &Event) -> Result<Vec<Entry>> {
    let mut out = entries.to_vec();
    let step = match event {
        Event::Prepare(id)
        | Event::Prepared(id, _)
        | Event::Dispatch(id)
        | Event::Applied(id, _)
        | Event::Failed(id, _)
        | Event::Cancelled(id, _)
        | Event::Absent(id)
        | Event::RecoveredPrepared(id, _)
        | Event::Unknown(id)
        | Event::Cancel(id) => *id,
        Event::Floor => return Ok(out),
    };
    let index = entry_index(entries, step)?;
    if matches!(event, Event::Prepare(_)) {
        require(next(entries)? == Some(step))?;
    } else {
        require(active(entries) == Some(index) || entries[index].phase.terminal())?;
    }
    let entry = &mut out[index];
    let preparation = |digest: Digest32| -> Result<FamilyPreparation> {
        FamilyPreparation::new(entry.family, digest)
    };
    let receipt =
        |digest: Digest32| -> Result<FamilyReceipt> { FamilyReceipt::new(entry.family, digest) };
    match *event {
        Event::Prepare(_) => {
            require(entry.phase == Phase::Pending)?;
            entry.phase = Phase::PrepareStarted;
        }
        Event::Prepared(_, digest) => {
            require(entry.phase == Phase::PrepareStarted && entry.preparation.is_none())?;
            entry.preparation = Some(preparation(digest)?);
            entry.phase = Phase::Prepared;
        }
        Event::Dispatch(_) => {
            require(
                entry.phase == Phase::Prepared
                    && entry.preparation.is_some()
                    && !entry.dispatch_attempted,
            )?;
            entry.dispatch_attempted = true;
            entry.phase = Phase::DispatchStarted;
        }
        Event::Applied(_, digest) => {
            let value = receipt(digest)?;
            require(
                !entry.phase.terminal()
                    || (entry.phase == Phase::Applied && entry.receipt == Some(value)),
            )?;
            entry.receipt = Some(value);
            entry.phase = Phase::Applied;
        }
        Event::Failed(_, digest) | Event::Cancelled(_, digest) => {
            let value = digest.map(receipt).transpose()?;
            let phase = if matches!(event, Event::Failed(_, _)) {
                Phase::Failed
            } else {
                Phase::Cancelled
            };
            require(!entry.phase.terminal() || (entry.phase == phase && entry.receipt == value))?;
            entry.receipt = value;
            entry.phase = phase;
        }
        Event::Absent(_) => {
            require(
                entry.phase.uncertain() && !entry.dispatch_attempted && entry.preparation.is_none(),
            )?;
            entry.phase = Phase::Pending;
        }
        Event::RecoveredPrepared(_, digest) => {
            let value = preparation(digest)?;
            require(!entry.phase.terminal() && entry.preparation.is_none_or(|old| old == value))?;
            entry.preparation = Some(value);
            entry.phase = Phase::Prepared;
        }
        Event::Unknown(_) => {
            require(!entry.phase.terminal())?;
            entry.phase = Phase::Indeterminate;
        }
        Event::Cancel(_) => {
            require(entry.phase == Phase::Prepared && entry.preparation.is_some())?;
            entry.phase = Phase::CancelStarted;
        }
        Event::Floor => {}
    }
    require(
        out.iter()
            .filter(|entry| entry.phase != Phase::Pending && !entry.phase.terminal())
            .count()
            <= 1,
    )?;
    Ok(out)
}

struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(context: OperationContext) -> Result<Self> {
        context.budget.validate()?;
        if context.budget.max_wall_millis > 60_000
            || context.budget.max_bytes > 2 * 1024 * 1024 * 1024
        {
            return Err(bounded());
        }
        let out = Self {
            bytes: context.budget.max_bytes,
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(bounded)?,
            context,
        };
        out.current()?;
        Ok(out)
    }
    fn current(&self) -> Result<OperationContext> {
        let mut c = self.context.clone();
        c.budget.max_wall_millis = self
            .deadline
            .checked_duration_since(Instant::now())
            .and_then(|left| u64::try_from(left.as_millis()).ok())
            .filter(|left| *left > 0)
            .ok_or_else(bounded)?;
        c.budget.max_bytes = self.bytes;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(c)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.current()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(bounded)?;
        Ok(())
    }
    fn family(&mut self, retained: u64, action: bool) -> Result<OperationContext> {
        let mut c = self.current()?;
        let bytes = self
            .bytes
            .checked_sub(retained)
            .filter(|left| *left > 0)
            .ok_or_else(bounded)?;
        if action && c.budget.max_actions == 0 {
            return Err(bounded());
        }
        self.bytes = retained;
        c.budget.max_bytes = bytes;
        if action {
            c.budget.max_actions = 1;
        }
        Ok(c)
    }
    fn narrow_family(&self, reserved: &OperationContext) -> Result<OperationContext> {
        let current = self.current()?;
        let mut delegated = reserved.clone();
        delegated.anchor.tick =
            GameTick(delegated.anchor.tick.get().max(current.anchor.tick.get()));
        delegated.budget.max_wall_millis = delegated
            .budget
            .max_wall_millis
            .min(current.budget.max_wall_millis);
        delegated.cancellation_requested |= current.cancellation_requested;
        Ok(delegated)
    }
    fn floor(&mut self, tick: u64) {
        self.context.anchor.tick = GameTick(self.context.anchor.tick.get().max(tick));
    }
}

fn read<S: EffectJournalStorage>(storage: &mut S, work: &mut Work) -> Result<Vec<u8>> {
    storage.validate_identity().map_err(|_| corrupt())?;
    work.current()?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    require(length <= MAX_STORE_BYTES as u64)?;
    work.charge(length as usize)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut out = vec![0; length as usize];
    let mut offset = 0;
    while offset < out.len() {
        work.current()?;
        let end = out.len().min(offset + 64 * 1024);
        let count = storage.read(&mut out[offset..end]).map_err(|_| corrupt())?;
        require(count > 0)?;
        offset += count;
    }
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.current()?;
    Ok(out)
}

/// Compute and externally retain this pin before attempting exclusive creation.
pub fn initial_recovery(plan: &PreparedPlan, c: &OperationContext) -> Result<Recovery> {
    let mut work = Work::new(c.clone())?;
    let header = wire::header(plan, c, &mut work)?;
    Ok(Recovery {
        journal_id: header.id,
        head: header.id,
        authority_tick_floor: work.context.anchor.tick.get(),
    })
}

/// Exclusive owner of the frontier and its complete original plan archive.
pub struct Coordinator<S> {
    storage: S,
    compiled: LiveDispatchCoordinator,
    seal: PlanArchiveSeal,
    raw: Vec<u8>,
    id: Digest32,
    head: Digest32,
    heads: Vec<Digest32>,
    frames: usize,
    entries: Vec<Entry>,
    mode: BuildMode,
    owner: SessionId,
    grants: Vec<CapabilityGrant>,
    high_tick: u64,
    stored_tick: u64,
    review: Option<Review>,
    fenced: bool,
}

impl<S: EffectJournalStorage> Coordinator<S> {
    /// New custody requires explicit Control mode and Plan authority. Existing
    /// history always requires its external original pin; writable recovery
    /// re-synchronizes a complete surviving descendant but performs no repair.
    pub fn open(
        mut storage: S,
        c: &OperationContext,
        mode: BuildMode,
        expected_plan: Option<PreparedPlan>,
        recovery: Recovery,
        create: bool,
    ) -> Result<Self> {
        recovery.validate()?;
        let mut current = c.clone();
        current.anchor.tick = GameTick(
            current
                .anchor
                .tick
                .get()
                .max(recovery.authority_tick_floor)
                .max(
                    expected_plan
                        .as_ref()
                        .map_or(0, |plan| plan.anchor.tick.get()),
                ),
        );
        let mut work = Work::new(current)?;
        if create {
            if mode != BuildMode::Control || expected_plan.is_none() {
                return Err(denied());
            }
            work.current()?
                .authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        }
        let mut raw = read(&mut storage, &mut work)?;
        if create {
            require(raw.is_empty())?;
            let header = wire::header(expected_plan.as_ref().ok_or_else(corrupt)?, c, &mut work)?;
            require(recovery.journal_id == header.id && recovery.head == header.id)?;
            raw = header.raw;
        }
        let replay = wire::replay(&raw, &mut work)?;
        require(replay.id == recovery.journal_id && replay.heads.contains(&recovery.head))?;
        if let Some(expected) = expected_plan {
            let (bytes, expected_seal) =
                plan_archive::encode_with_check(&expected, limits(), || {
                    work.current().map(|_| ())
                })?;
            work.charge(bytes.len().saturating_mul(2))?;
            require(expected_seal == replay.seal && bytes == replay.archive)?;
        }
        if replay.compiled.plan.anchor.fortress_id != c.anchor.fortress_id {
            return Err(denied());
        }
        work.floor(replay.high_tick.max(recovery.authority_tick_floor));
        work.current()?;
        if create {
            work.charge(raw.len().saturating_mul(2))?;
            storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
            storage.write_all(&raw).map_err(|_| corrupt())?;
        }
        if mode != BuildMode::Offline {
            work.current()?;
            storage
                .flush()
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
        }
        require(read(&mut storage, &mut work)? == raw)?;
        Ok(Self {
            storage,
            compiled: replay.compiled,
            seal: replay.seal,
            raw,
            id: replay.id,
            head: replay.head,
            heads: replay.heads,
            frames: replay.frames,
            entries: replay.entries,
            mode,
            owner: c.session_id,
            grants: c.grants.clone(),
            high_tick: replay
                .high_tick
                .max(c.anchor.tick.get())
                .max(recovery.authority_tick_floor),
            stored_tick: replay.high_tick,
            review: None,
            fenced: false,
        })
    }
    pub fn original_plan(&self) -> &PreparedPlan {
        &self.compiled.plan
    }
    pub fn recovery(&self) -> Recovery {
        Recovery {
            journal_id: self.id,
            head: self.head,
            authority_tick_floor: self.high_tick,
        }
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub fn mode(&self) -> BuildMode {
        self.mode
    }
    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub fn abandon_review(&mut self) {
        self.review = None;
    }
    pub fn abandon_preparation<E: DurableFamilyExecutor>(&mut self, executor: &mut E) {
        self.review = None;
        executor.abandon_preparation();
    }
    fn retain_executor_floor<E: DurableFamilyExecutor>(
        &mut self,
        executor: &E,
        c: &OperationContext,
    ) {
        if c.session_id == self.owner
            && c.anchor.fortress_id == self.compiled.plan.anchor.fortress_id
        {
            self.high_tick = self.high_tick.max(executor.observed_tick_floor());
        }
    }
    /// Caller-owned I/O is separate. This reserves this journal's checks,
    /// intent/result frames and final publication before a family call starts.
    pub fn operation_reserve(&self) -> u64 {
        (8 * (self.raw.len() + 3 * MAX_FRAME_BYTES) + 64 * 1024) as u64
    }
    fn retain_capacity(&self, frames: usize) -> Result<()> {
        if self.frames.saturating_add(frames) > MAX_FRAMES
            || self
                .raw
                .len()
                .saturating_add(frames.saturating_mul(MAX_FRAME_BYTES))
                > MAX_STORE_BYTES
        {
            return Err(bounded());
        }
        Ok(())
    }
    fn delegated(&self, mut c: OperationContext) -> Result<OperationContext> {
        c.grants = authority::intersect(&c, &self.grants)?;
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(c)
    }
    fn access(&mut self, c: &OperationContext) -> Result<OperationContext> {
        if c.session_id != self.owner
            || c.anchor.fortress_id != self.compiled.plan.anchor.fortress_id
        {
            return Err(denied());
        }
        self.high_tick = self.high_tick.max(c.anchor.tick.get());
        let mut current = c.clone();
        current.anchor.tick = GameTick(self.high_tick);
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let mut retained = current.clone();
        retained.grants = self.grants.clone();
        retained.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if self.fenced {
            return Err(corrupt());
        }
        Ok(current)
    }
    fn authorize_step(&self, step: StepId, c: &OperationContext, cancel: bool) -> Result<()> {
        let index = entry_index(&self.entries, step)?;
        let selected = &self.compiled.steps[index].semantic;
        if (!cancel && self.mode != BuildMode::Control) || self.mode == BuildMode::Offline {
            return Err(denied());
        }
        if !cancel
            && (c.anchor.tick >= self.compiled.plan.expires_at_tick
                || selected
                    .obligation
                    .as_ref()
                    .is_some_and(|o| c.anchor.tick >= o.deadline_tick))
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "original live plan or selected obligation expired; only historical recovery remains",
            ));
        }
        let scope = selected.action.scope();
        for opening in [false, true] {
            let mut checked = c.clone();
            if opening {
                checked.grants = self.grants.clone();
            }
            if !cancel {
                checked.authorize(
                    Capability::Plan,
                    RiskTier::Guarded,
                    &scope.entity_ids,
                    scope.map_area,
                )?;
            }
            checked.authorize(
                selected.required_capability,
                selected.risk,
                &scope.entity_ids,
                scope.map_area,
            )?;
        }
        Ok(())
    }
    fn verify(&mut self, work: &mut Work) -> Result<()> {
        self.access(&work.current()?)?;
        match read(&mut self.storage, work) {
            Ok(raw) if raw == self.raw => Ok(()),
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
                self.review = None;
                Err(corrupt())
            }
        }
    }
    fn append(&mut self, event: Event, work: &mut Work) -> Result<()> {
        if self.mode == BuildMode::Offline {
            return Err(denied());
        }
        self.access(&work.current()?)?;
        let entries = transition(&self.entries, &event)?;
        let tick = self.high_tick.max(work.context.anchor.tick.get());
        let frame = wire::frame(self.frames, self.head, tick, &event)?;
        if self.frames >= MAX_FRAMES || self.raw.len().saturating_add(frame.len()) > MAX_STORE_BYTES
        {
            return Err(bounded());
        }
        work.charge(frame.len())?;
        self.verify(work)?;
        let mut raw = self.raw.clone();
        raw.extend_from_slice(&frame);
        self.fenced = true;
        self.storage.validate_identity().map_err(|_| corrupt())?;
        require(
            self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == self.raw.len() as u64,
        )?;
        self.storage.write_all(&frame).map_err(|_| corrupt())?;
        work.current()?;
        self.storage
            .flush()
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        require(read(&mut self.storage, work)? == raw)?;
        let current = work.current()?;
        let mut retained = current.clone();
        retained.grants = self.grants.clone();
        retained.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.head = wire::tail(&frame)?;
        self.heads.push(self.head);
        self.raw = raw;
        self.entries = entries;
        self.frames += 1;
        self.high_tick = tick;
        self.stored_tick = tick;
        self.fenced = false;
        Ok(())
    }
    fn view(&self) -> Result<View> {
        Ok(View {
            recovery: self.recovery(),
            plan_digest: self.seal.plan_digest,
            original_object_digest: self.seal.object_digest,
            mode: self.mode,
            high_tick: self.high_tick,
            entries: self.entries.clone(),
            review: self.review.clone(),
            next_ready: if self.mode == BuildMode::Control {
                next(&self.entries)?
            } else {
                None
            },
        })
    }
    fn finish(&mut self, work: &mut Work, publication: bool) -> Result<View> {
        let result = self.verify(work).and_then(|()| self.view());
        if result.is_err() {
            self.review = None;
            if publication {
                self.fenced = true;
            }
        }
        result
    }
    fn completed<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.review = None;
        }
        result
    }
    pub fn inspect(&mut self, c: &OperationContext) -> Result<View> {
        let result = (|| {
            let current = self.access(c)?;
            let mut work = Work::new(current)?;
            self.finish(&mut work, false)
        })();
        self.completed(result)
    }
    pub fn verify_recovery(&mut self, c: &OperationContext, recovery: Recovery) -> Result<View> {
        let result = (|| {
            recovery.validate()?;
            let mut current = c.clone();
            current.anchor.tick =
                GameTick(current.anchor.tick.get().max(recovery.authority_tick_floor));
            let view = self.inspect(&current)?;
            require(recovery.journal_id == self.id && self.heads.contains(&recovery.head))?;
            Ok(view)
        })();
        self.completed(result)
    }
    pub fn prepare_next<E: DurableFamilyExecutor>(
        &mut self,
        executor: &mut E,
        c: &OperationContext,
    ) -> Result<Review> {
        self.review = None;
        let result = self.prepare_inner(executor, c);
        if result.is_err() {
            executor.abandon_preparation();
        }
        self.completed(result)
    }
    fn prepare_inner<E: DurableFamilyExecutor>(
        &mut self,
        executor: &mut E,
        c: &OperationContext,
    ) -> Result<Review> {
        self.retain_executor_floor(executor, c);
        let current = self.access(c)?;
        let mut work = Work::new(current)?;
        let step = next(&self.entries)?.ok_or_else(conflict)?;
        self.authorize_step(step, &work.current()?, false)?;
        // The prepared owner must still be able to commit or cancel after this
        // call; do not spend the final retention slots merely creating it.
        self.retain_capacity(4)?;
        let family_context = self.delegated(work.family(self.operation_reserve(), true)?)?;
        self.verify(&mut work)?;
        self.append(Event::Prepare(step), &mut work)?;
        let family_context = self.access(&work.narrow_family(&family_context)?)?;
        self.authorize_step(step, &family_context, false)?;
        let index = entry_index(&self.entries, step)?;
        let compiled = &self.compiled.steps[index];
        let outcome = executor.prepare(
            &self.compiled.plan,
            &compiled.semantic,
            &compiled.route,
            &family_context,
        );
        self.high_tick = self.high_tick.max(executor.observed_tick_floor());
        work.floor(self.high_tick);
        let prepared = outcome?;
        if prepared.family != self.entries[index].family || prepared.digest == Digest32::ZERO {
            return Err(corrupt());
        }
        if let Err(cause) = self.append(Event::Prepared(step, prepared.digest), &mut work) {
            self.fenced = true;
            return Err(cause);
        }
        if let Err(cause) = work
            .current()
            .and_then(|c| self.authorize_step(step, &c, false))
        {
            self.fenced = true;
            return Err(cause);
        }
        let mut seal = Vec::new();
        seal.extend_from_slice(self.id.as_bytes());
        seal.extend_from_slice(self.head.as_bytes());
        seal.extend_from_slice(self.seal.object_digest.as_bytes());
        seal.extend_from_slice(&step.get().to_be_bytes());
        seal.extend_from_slice(prepared.digest.as_bytes());
        seal.extend_from_slice(&self.owner.get().to_be_bytes());
        let review = Review {
            step,
            family: prepared.family,
            preparation: prepared,
            seal: hash(b"dfmcp-live-dispatch-review/1\0", &seal),
            head: self.head,
        };
        self.review = Some(review.clone());
        self.finish(&mut work, true)?;
        if let Err(cause) = work
            .current()
            .and_then(|c| self.authorize_step(step, &c, false))
        {
            self.fenced = true;
            return Err(cause);
        }
        Ok(review)
    }
    pub fn commit<E: DurableFamilyExecutor>(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        executor: &mut E,
        c: &OperationContext,
    ) -> Result<View> {
        let review = self.review.take();
        let result = (|| {
            self.retain_executor_floor(executor, c);
            let current = self.access(c)?;
            let mut work = Work::new(current)?;
            let review = review
                .filter(|r| confirmed && r.seal == seal && r.head == self.head)
                .ok_or_else(denied)?;
            let index = entry_index(&self.entries, review.step)?;
            require(
                self.entries[index].phase == Phase::Prepared
                    && self.entries[index].preparation == Some(review.preparation)
                    && !self.entries[index].dispatch_attempted,
            )?;
            self.authorize_step(review.step, &work.current()?, false)?;
            self.retain_capacity(2)?;
            let family_context = self.delegated(work.family(self.operation_reserve(), true)?)?;
            self.verify(&mut work)?;
            self.append(Event::Dispatch(review.step), &mut work)?;
            let family_context = self.access(&work.narrow_family(&family_context)?)?;
            self.authorize_step(review.step, &family_context, false)?;
            let compiled = &self.compiled.steps[index];
            let outcome = executor.commit(
                &self.compiled.plan,
                &compiled.semantic,
                &compiled.route,
                review.preparation,
                &family_context,
            );
            self.high_tick = self.high_tick.max(executor.observed_tick_floor());
            work.floor(self.high_tick);
            let receipt = outcome?;
            if receipt.family != self.entries[index].family || receipt.digest == Digest32::ZERO {
                return Err(corrupt());
            }
            if let Err(cause) = self.append(Event::Applied(review.step, receipt.digest), &mut work)
            {
                self.fenced = true;
                return Err(cause);
            }
            self.finish(&mut work, true)
        })();
        if result.is_err() {
            executor.abandon_preparation();
        }
        self.completed(result)
    }
    pub fn reconcile<E: DurableFamilyExecutor>(
        &mut self,
        step: StepId,
        executor: &mut E,
        c: &OperationContext,
    ) -> Result<View> {
        self.review = None;
        let result = (|| {
            self.retain_executor_floor(executor, c);
            let current = self.access(c)?;
            let mut work = Work::new(current)?;
            if self.mode == BuildMode::Offline {
                return Err(denied());
            }
            let index = entry_index(&self.entries, step)?;
            if self.entries[index].phase == Phase::Pending {
                return Err(conflict());
            }
            self.retain_capacity(1)?;
            let family_context = self.delegated(work.family(self.operation_reserve(), false)?)?;
            self.verify(&mut work)?;
            let family_context = self.access(&work.narrow_family(&family_context)?)?;
            let compiled = &self.compiled.steps[index];
            let outcome = executor.recover_step(
                &self.compiled.plan,
                &compiled.semantic,
                &compiled.route,
                &family_context,
            );
            self.high_tick = self.high_tick.max(executor.observed_tick_floor());
            work.floor(self.high_tick);
            let check_receipt = |receipt: FamilyReceipt| -> Result<Digest32> {
                if receipt.family != self.entries[index].family || receipt.digest == Digest32::ZERO
                {
                    return Err(corrupt());
                }
                Ok(receipt.digest)
            };
            let recovered = match outcome {
                Ok(recovered) => recovered,
                Err(cause) => {
                    if cause.code == ErrorCode::CorruptLedger {
                        self.fenced = true;
                    }
                    return Err(cause);
                }
            };
            let event = (|| match recovered {
                RecoveredFamily::AbsentBeforeDispatch => Ok(Event::Absent(step)),
                RecoveredFamily::Prepared(prepared) => {
                    if prepared.family != self.entries[index].family
                        || prepared.digest == Digest32::ZERO
                    {
                        return Err(corrupt());
                    }
                    Ok(Event::RecoveredPrepared(step, prepared.digest))
                }
                RecoveredFamily::Applied(receipt) => {
                    Ok(Event::Applied(step, check_receipt(receipt)?))
                }
                RecoveredFamily::Failed(receipt) => {
                    Ok(Event::Failed(step, receipt.map(check_receipt).transpose()?))
                }
                RecoveredFamily::Cancelled(receipt) => Ok(Event::Cancelled(
                    step,
                    receipt.map(check_receipt).transpose()?,
                )),
                RecoveredFamily::Indeterminate => Ok(Event::Unknown(step)),
            })();
            let event = match event {
                Ok(event) => event,
                Err(cause) => {
                    self.fenced = true;
                    return Err(cause);
                }
            };
            let proposed = match transition(&self.entries, &event) {
                Ok(proposed) => proposed,
                Err(cause) => {
                    self.fenced = true;
                    return Err(cause);
                }
            };
            if proposed != self.entries || self.high_tick > self.stored_tick {
                if let Err(cause) = self.append(event, &mut work) {
                    self.fenced = true;
                    return Err(cause);
                }
            }
            self.finish(&mut work, true)
        })();
        if result.is_err() {
            executor.abandon_preparation();
        }
        self.completed(result)
    }
    /// Retire only a proven preparation. This never compensates an Applied
    /// effect, grants future ticks or bypasses the family's own cancellation.
    pub fn cancel_prepared<E: DurableFamilyExecutor>(
        &mut self,
        step: StepId,
        executor: &mut E,
        c: &OperationContext,
    ) -> Result<View> {
        self.review = None;
        let result = (|| {
            self.retain_executor_floor(executor, c);
            let current = self.access(c)?;
            let mut work = Work::new(current)?;
            let index = entry_index(&self.entries, step)?;
            if self.entries[index].phase != Phase::Prepared {
                return Err(conflict());
            }
            let preparation = self.entries[index].preparation.ok_or_else(corrupt)?;
            self.authorize_step(step, &work.current()?, true)?;
            self.retain_capacity(2)?;
            let family_context = self.delegated(work.family(self.operation_reserve(), true)?)?;
            self.verify(&mut work)?;
            self.append(Event::Cancel(step), &mut work)?;
            let family_context = self.access(&work.narrow_family(&family_context)?)?;
            self.authorize_step(step, &family_context, true)?;
            let compiled = &self.compiled.steps[index];
            let outcome = executor.cancel_prepared(
                &self.compiled.plan,
                &compiled.semantic,
                &compiled.route,
                preparation,
                &family_context,
            );
            self.high_tick = self.high_tick.max(executor.observed_tick_floor());
            work.floor(self.high_tick);
            let receipt = outcome?;
            if receipt.is_some_and(|value| {
                value.family != self.entries[index].family || value.digest == Digest32::ZERO
            }) {
                return Err(corrupt());
            }
            if let Err(cause) = self.append(
                Event::Cancelled(step, receipt.map(|value| value.digest)),
                &mut work,
            ) {
                self.fenced = true;
                return Err(cause);
            }
            self.finish(&mut work, true)
        })();
        if result.is_err() {
            executor.abandon_preparation();
        }
        self.completed(result)
    }
}

/// Operator-selected private custody, using the existing no-follow, exact-mode
/// file/directory lock shell. No client-selected path, repair or truncation.
pub fn open_private_coordinator(
    path: &Path,
    c: &OperationContext,
    mode: BuildMode,
    expected_plan: Option<PreparedPlan>,
    recovery: Recovery,
    initialize: bool,
) -> Result<Coordinator<PrivateBuildFile>> {
    recovery.validate()?;
    let mut current = c.clone();
    current.anchor.tick = GameTick(
        current
            .anchor
            .tick
            .get()
            .max(recovery.authority_tick_floor)
            .max(
                expected_plan
                    .as_ref()
                    .map_or(0, |plan| plan.anchor.tick.get()),
            ),
    );
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mut work = Work::new(current)?;
    if initialize {
        if mode != BuildMode::Control || expected_plan.is_none() {
            return Err(denied());
        }
        work.current()?
            .authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        let header = wire::header(expected_plan.as_ref().ok_or_else(corrupt)?, c, &mut work)?;
        require(recovery.journal_id == header.id && recovery.head == header.id)?;
    }
    let (storage, created, mut current) =
        open_private_storage(path, &work.current()?, mode, initialize, MAX_STORE_BYTES)?;
    if initialize != created {
        return Err(conflict());
    }
    current.budget.max_wall_millis = current
        .budget
        .max_wall_millis
        .min(work.current()?.budget.max_wall_millis);
    Coordinator::open(storage, &current, mode, expected_plan, recovery, created)
}

#[cfg(test)]
mod tests;
