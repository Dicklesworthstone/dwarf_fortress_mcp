//! Separately reviewed finite run/1.13 intervals for one original furnishing batch.
//!
//! The existing native coordinator owns durable intent and one-attempt unpause.
//! This layer associates every retained run with the entire immutable monitor,
//! its original placement receipt set and unchanged temporal policy. The clock
//! source has no named-fortress lineage; its independently issued NIL authority
//! is never manufactured by rewriting the named-fortress construction grants.
use std::cell::Cell as ClockFloor;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use super::origin::MonitorDefinition;
use super::{LinkedSample, Manifest, Progress};
use crate::bounded_run::journal::{DurableRun, RunBinding, RunJournal, RunMode, RunState};
use crate::bounded_run::rpc::{RunManifest, RunSource};
use crate::bounded_run::{RunObservation, RunPlan, RunRecord, RunSpec};
use crate::control_effect_journal::EffectJournalStorage;
use crate::live_operations::{LiveOperationsObservation, OperationsProfile};
use crate::room_condition::{self as room, clock::RoomClockSample};
use dfmcp_core::{
    Capability, CapabilityGrant, DfmcpError, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};

pub const MAX_RUNS: usize = 64;
pub const VIEW_BYTES: u64 = 8 * 1024 * 1024;
/// The caller prepays original-custody reads and receipt-bracket acquisition.
/// This owner prepays its own two complete assessment decodes and native journal.
pub const OPERATION_BYTES: u64 = 192 * 1024 * 1024;
pub const READINESS_BYTES: u64 = 96 * 1024 * 1024;
const ACTION_BYTES: u64 = 32 * 1024 * 1024;
const OBSERVE_BYTES: u64 = 1024 * 1024;

fn error(code: ErrorCode, text: &str) -> DfmcpError {
    DfmcpError::new(code, text)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "clock history differs from the entire original furniture obligation",
    )
}
fn denied() -> DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "furniture clock lacks its separate current original and native authority",
    )
}
fn stale() -> DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "fresh whole-plan construction and original clock source no longer agree",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete furniture clock work or original finite horizon is exhausted",
    )
}
fn uncertain() -> DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "reconcile or stop the retained original clock run; never repeat unpause",
    )
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(domain: &[u8], fields: &[&[u8]]) -> Digest32 {
    let mut raw = domain.to_vec();
    raw.push(0);
    for field in fields {
        raw.extend_from_slice(&(field.len() as u64).to_be_bytes());
        raw.extend_from_slice(field);
    }
    Digest32::of_bytes(&raw)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub journal_id: Digest32,
    pub head: Digest32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunRequest {
    pub monitor_id: Digest32,
    pub game_ticks: u32,
    pub wall_millis: u32,
}
impl RunRequest {
    pub fn spec(self) -> Result<RunSpec> {
        RunSpec::new(self.game_ticks, self.wall_millis)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Prepare,
    Commit,
}

/// A newly acquired complete receipt bracket together with the unchanged
/// monitor's retained progress. The host revalidates original custody before
/// acquiring these bytes and returns them before any later publication check.
#[derive(Clone, Debug)]
pub struct ClockSample {
    pub progress: Progress,
    pub sample: LinkedSample,
}

/// Complete current construction eligibility, never a published goal sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readiness {
    tick: u64,
    digest: Digest32,
    pending_count: u32,
    complete_count: u32,
    withdraws_stability: bool,
    terrain: Option<room::TerrainAssessment>,
}
fn start_tick(spec: &MonitorDefinition) -> u64 {
    spec.origin()
        .receipts()
        .iter()
        .flat_map(|record| {
            [
                record.plan().before().tick(),
                record.after().map_or(0, |after| after.tick()),
            ]
        })
        .max()
        .unwrap_or(0)
}
fn decode_capture(sample: &LinkedSample) -> Result<LiveOperationsObservation> {
    sample.operations.validate()?;
    if sample.capture.is_empty() || sample.capture.len() > super::MAX_CAPTURE {
        return Err(bounded());
    }
    LiveOperationsObservation::decode_profile(
        &sample.capture,
        sample.operations.generation,
        sample.operations.df_version.clone(),
        sample.operations.dfhack_version.clone(),
        OperationsProfile::PagedV1_4,
    )
}
impl Readiness {
    pub fn capture(
        spec: &MonitorDefinition,
        operations: &Manifest,
        progress: &Progress,
        sample: &LinkedSample,
        context: &OperationContext,
    ) -> Result<Self> {
        context.budget.validate()?;
        if context.budget.max_bytes < READINESS_BYTES || context.budget.max_wall_millis > 60_000 {
            return Err(bounded());
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(bounded)?;
        let observed = decode_capture(sample)?;
        let readiness = Self::from_capture(
            spec,
            operations,
            progress,
            sample,
            &narrow(deadline, context)?,
            observed,
        )?;
        narrow(deadline, context)?;
        Ok(readiness)
    }
    fn from_capture(
        spec: &MonitorDefinition,
        operations: &Manifest,
        progress: &Progress,
        sample: &LinkedSample,
        context: &OperationContext,
        observed: LiveOperationsObservation,
    ) -> Result<Self> {
        if context.budget.max_bytes < READINESS_BYTES {
            return Err(bounded());
        }
        let goal = spec.goal();
        let binding = spec.origin().definition().binding();
        let tick = observed.jobs.tick().get();
        if progress.goal_digest != goal.digest()
            || progress.terminal()
            || progress.reading
            || progress.observations == 0
            || progress.observations >= goal.timing().max_observations
            || !observed.jobs.paused
            || &sample.operations != operations
            || progress.source.as_ref().is_none_or(|source| {
                source.furniture_generation != binding.generation()
                    || &source.operations != operations
            })
            || tick < context.anchor.tick.get()
            || tick < start_tick(spec)
            || tick >= goal.timing().deadline
        {
            return Err(stale());
        }
        let mut current = context.clone();
        current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        // The host accounts for this completed acquisition separately. Recheck
        // every original predicate without charging another observation,
        // adding positive stability credit, or projecting goal satisfaction.
        let preview = super::readiness::probe(&progress.begin_read()?, goal, sample, &current)?;
        if preview.terminal()
            || preview.assessments.len() != goal.records().len()
            || preview
                .assessments
                .iter()
                .any(|row| !matches!(row.condition.status, "pending" | "condition_met"))
        {
            return Err(stale());
        }
        // The same probe accepts pre- or post-acknowledgement progress without
        // consuming the final remaining acquisition a second time.
        let complete_count = preview.condition_met_count() as u32;
        let pending_count = goal.records().len() as u32 - complete_count;
        let capture = Digest32::of_bytes(&sample.capture);
        let withdraws_stability = progress.streak > 0
            && (pending_count > 0
                || progress.last_tick.is_some_and(|last| {
                    tick.saturating_sub(last) > u64::from(goal.timing().max_gap)
                        || (tick == last && progress.last_capture != Some(capture))
                }));
        let raw = sample.canonical_bytes()?;
        Ok(Self {
            tick,
            digest: hash(
                b"dfmcp-furniture-clock-readiness/1",
                &[spec.id().as_bytes(), &raw],
            ),
            pending_count,
            complete_count,
            withdraws_stability,
            terrain: None,
        })
    }
    pub(crate) fn from_room_capture(
        definition: &room::Definition,
        source: &room::SourceIdentity,
        progress: &room::Progress,
        sample: &room::Sample,
        context: &OperationContext,
    ) -> Result<Self> {
        let assessment = room::clock::assess(definition, source, progress, sample, context)?;
        Ok(Self {
            tick: assessment.tick,
            digest: hash(
                b"dfmcp-room-clock-readiness/1",
                &[definition.id().as_bytes(), &sample.canonical_bytes()?],
            ),
            pending_count: assessment.pending_count,
            complete_count: assessment.complete_count,
            withdraws_stability: assessment.withdraws_stability,
            terrain: Some(assessment.terrain),
        })
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn pending_count(&self) -> u32 {
        self.pending_count
    }
    pub fn complete_count(&self) -> u32 {
        self.complete_count
    }
    pub fn withdraws_stability(&self) -> bool {
        self.withdraws_stability
    }
    /// Complete room deficits disclosed by a room-bound clock review. A known
    /// unmet requirement can accompany an explicitly reviewed global interval;
    /// this assessment never promises excavation work or positive goal credit.
    /// Ordinary furniture clock reviews retain their original representation.
    pub fn terrain(&self) -> Option<&room::TerrainAssessment> {
        self.terrain.as_ref()
    }
}

struct RoomObjective {
    definition: room::Definition,
    source: room::SourceIdentity,
    source_bytes: Vec<u8>,
}
/// Closed internal variants prevent an ordinary furnishing monitor or caller
/// supplied predicate from acquiring authority over a different room goal.
enum Objective {
    Furniture,
    Room(Box<RoomObjective>),
}
impl Objective {
    fn operation_bytes(&self) -> u64 {
        match self {
            Self::Furniture => OPERATION_BYTES,
            Self::Room(_) => room::clock::OPERATION_BYTES,
        }
    }
    fn readiness_bytes(&self) -> u64 {
        match self {
            Self::Furniture => READINESS_BYTES,
            Self::Room(_) => room::clock::READINESS_BYTES,
        }
    }
}
enum ReadinessCapture {
    Furniture(Box<ClockSample>),
    Room(Box<RoomClockSample>),
}

#[derive(Clone, Debug)]
pub struct ClockRunView {
    pub entry: DurableRun,
    /// Present only for the current process's separately prepared interval.
    pub review_seal: Option<Digest32>,
}
#[derive(Clone, Debug)]
pub struct ClockView {
    pub recovery: Recovery,
    pub mode: RunMode,
    pub monitor_id: Digest32,
    pub runs: Vec<ClockRunView>,
    pub high_tick: u64,
    pub high_sequence: u64,
    pub pending: bool,
    pub unresolved: bool,
    pub readiness: Option<Readiness>,
    pub verified: bool,
}
fn settled(run: &DurableRun) -> bool {
    run.terminal() && !run.unresolved()
}
impl ClockView {
    pub fn active(&self) -> Option<&ClockRunView> {
        self.runs.iter().find(|run| !settled(&run.entry))
    }
    pub fn may_be_running(&self) -> bool {
        self.unresolved
    }
    pub fn can_sample_goal(&self) -> bool {
        self.verified && !self.pending && !self.unresolved
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.active().is_some_and(|run| !run.entry.terminal())
    }
    pub fn cancellation_needs_source(&self) -> bool {
        self.active().is_some_and(|run| {
            !run.entry.terminal()
                && !matches!(run.entry.state(), RunState::Intent | RunState::Prepared)
        })
    }
}

#[derive(Clone)]
struct Review {
    key: String,
    plan: Digest32,
    seal: Digest32,
}
#[derive(Clone)]
struct Authority {
    owner: SessionId,
    canonical: Vec<CapabilityGrant>,
    native: Vec<CapabilityGrant>,
    game_ticks: u64,
}
#[derive(Clone, Copy)]
enum Access {
    Query,
    Plan,
    Cancel,
}
fn authorize(
    spec: &MonitorDefinition,
    authority: &Authority,
    high_tick: u64,
    c: &OperationContext,
    n: &OperationContext,
    access: Access,
) -> Result<(OperationContext, OperationContext)> {
    if c.session_id != authority.owner
        || n.session_id != authority.owner
        || c.request_id != n.request_id
        || n.anchor.fortress_id != FortressId::NIL
        || c.anchor.fortress_id == FortressId::NIL
    {
        return Err(denied());
    }
    let floor = high_tick.max(c.anchor.tick.get()).max(n.anchor.tick.get());
    let mut current = c.clone();
    let mut native = n.clone();
    current.anchor.tick = GameTick(floor);
    native.anchor.tick = GameTick(floor);
    for original in [false, true] {
        let mut canonical_check = current.clone();
        let mut native_check = native.clone();
        if original {
            canonical_check.grants = authority.canonical.clone();
            native_check.grants = authority.native.clone();
        }
        if canonical_check.anchor.fortress_id
            != spec
                .origin()
                .definition()
                .binding()
                .fortress()
                .fortress_id()
        {
            return Err(denied());
        }
        canonical_check.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if matches!(access, Access::Plan) {
            canonical_check.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        }
        native_check.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if matches!(access, Access::Plan | Access::Cancel) {
            canonical_check.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            native_check.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        if matches!(access, Access::Plan) {
            canonical_check.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            native_check.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        }
    }
    Ok((current, native))
}
fn common_budget(a: WorkBudget, b: WorkBudget) -> WorkBudget {
    WorkBudget {
        max_wall_millis: a.max_wall_millis.min(b.max_wall_millis),
        max_game_ticks: a.max_game_ticks.min(b.max_game_ticks),
        max_bytes: a.max_bytes.min(b.max_bytes),
        max_entities: a.max_entities.min(b.max_entities),
        max_output_tokens: a.max_output_tokens.min(b.max_output_tokens),
        max_actions: a.max_actions.min(b.max_actions),
    }
}
struct Work {
    deadline: Instant,
    bytes: u64,
    c: OperationContext,
    n: OperationContext,
}
impl Work {
    fn new(c: OperationContext, n: OperationContext, minimum: u64) -> Result<Self> {
        let budget = common_budget(c.budget, n.budget);
        budget.validate()?;
        if budget.max_wall_millis > 60_000 || budget.max_bytes < minimum {
            return Err(bounded());
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(budget.max_wall_millis))
            .ok_or_else(bounded)?;
        let mut c = c;
        let mut n = n;
        c.budget = budget;
        n.budget = budget;
        Ok(Self {
            deadline,
            bytes: budget.max_bytes,
            c,
            n,
        })
    }
    fn take(&mut self, bytes: u64, native: bool) -> Result<OperationContext> {
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(bounded)?;
        let mut c = if native {
            self.n.clone()
        } else {
            self.c.clone()
        };
        c.budget.max_bytes = bytes;
        self.narrow(&c)
    }
    fn narrow(&self, c: &OperationContext) -> Result<OperationContext> {
        let wall = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(bounded)?
            .as_millis();
        if wall == 0 {
            return Err(bounded());
        }
        let mut current = c.clone();
        current.budget.max_wall_millis = current
            .budget
            .max_wall_millis
            .min(u64::try_from(wall).map_err(|_| bounded())?);
        Ok(current)
    }
}

/// Original goal and finite clock custody. Only a live `Review` can enter
/// commit, and it is consumed before the journal's dispatch path is entered.
pub struct Clock<S: EffectJournalStorage> {
    journal: RunJournal<S>,
    objective: Objective,
    spec: MonitorDefinition,
    operations: Manifest,
    operations_bytes: Vec<u8>,
    authority: Authority,
    high_tick: u64,
    high_sequence: u64,
    review: Option<Review>,
    readiness: Option<Readiness>,
}
impl<S: EffectJournalStorage> Clock<S> {
    pub fn new(
        journal: RunJournal<S>,
        spec: MonitorDefinition,
        operations: Manifest,
        recovery: Option<Recovery>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Self> {
        Self::new_objective(
            journal,
            spec,
            operations,
            Objective::Furniture,
            recovery,
            c,
            n,
        )
    }
    pub(crate) fn new_room(
        journal: RunJournal<S>,
        definition: room::Definition,
        source: room::SourceIdentity,
        recovery: Option<Recovery>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Self> {
        let source_bytes = room::clock::source_bytes(&definition, &source)?;
        let spec = definition.original().clone();
        let operations = source.furniture.operations.clone();
        let objective = Objective::Room(Box::new(RoomObjective {
            definition,
            source,
            source_bytes,
        }));
        Self::new_objective(journal, spec, operations, objective, recovery, c, n)
    }
    fn new_objective(
        mut journal: RunJournal<S>,
        spec: MonitorDefinition,
        operations: Manifest,
        objective: Objective,
        recovery: Option<Recovery>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Self> {
        let operations_bytes = operations.canonical_bytes()?;
        if operations.df_version != spec.origin().definition().binding().df_version()
            || operations.dfhack_version != spec.origin().definition().binding().dfhack_version()
        {
            return Err(stale());
        }
        if journal.binding().endpoint() != spec.origin().definition().binding().endpoint()
            || journal.binding().manifest().df_version
                != spec.origin().definition().binding().df_version()
            || journal.binding().manifest().dfhack_version
                != spec.origin().definition().binding().dfhack_version()
        {
            return Err(stale());
        }
        let authority = Authority {
            owner: c.session_id,
            canonical: c.grants.clone(),
            native: n.grants.clone(),
            game_ticks: c.budget.max_game_ticks.min(n.budget.max_game_ticks),
        };
        let (current, native) =
            authorize(&spec, &authority, start_tick(&spec), c, n, Access::Query)?;
        let mut work = Work::new(current, native, 2 * VIEW_BYTES)?;
        if let Some(pin) = recovery {
            journal.verify_recovery(pin.journal_id, pin.head, &work.take(VIEW_BYTES, true)?)?;
        } else if !journal.initialized_here() || journal.cached_records().next().is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "existing furniture clock requires its exact original journal identity and head",
            ));
        }
        let mut clock = Self {
            journal,
            objective,
            high_tick: start_tick(&spec).max(work.c.anchor.tick.get()),
            spec,
            operations,
            operations_bytes,
            authority,
            high_sequence: 0,
            review: None,
            readiness: None,
        };
        let inspection = work.take(VIEW_BYTES, true)?;
        clock.inspect(&work.narrow(&work.c)?, &inspection)?;
        Ok(clock)
    }
    pub fn spec(&self) -> &MonitorDefinition {
        &self.spec
    }
    pub fn definition(&self) -> &MonitorDefinition {
        &self.spec
    }
    pub fn operations(&self) -> &Manifest {
        &self.operations
    }
    pub fn binding(&self) -> &RunBinding {
        self.journal.binding()
    }
    pub fn recovery(&self) -> Recovery {
        Recovery {
            journal_id: self.journal.id(),
            head: self.journal.head(),
        }
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn high_sequence(&self) -> u64 {
        self.high_sequence
    }
    pub fn mode(&self) -> RunMode {
        self.journal.mode()
    }
    pub fn is_control(&self) -> bool {
        self.mode() == RunMode::Control
    }
    pub fn prepared_seal(&self) -> Option<Digest32> {
        (!self.journal.fenced())
            .then(|| self.review.as_ref().map(|r| r.seal))
            .flatten()
    }
    pub fn abandon_preparation(&mut self) {
        self.review = None;
        self.readiness = None;
    }
    fn target_digest(&self) -> Digest32 {
        if let Objective::Room(room) = &self.objective {
            return hash(
                b"dfmcp-room-clock-target/1",
                &[
                    room.definition.canonical_bytes(),
                    &room.source_bytes,
                    self.journal.id().as_bytes(),
                ],
            );
        }
        hash(
            b"dfmcp-furniture-clock-target/1",
            &[
                self.spec.canonical_bytes(),
                &self.operations_bytes,
                self.journal.id().as_bytes(),
            ],
        )
    }
    fn key(&self, plan: Digest32) -> String {
        if matches!(self.objective, Objective::Room(_)) {
            return format!(
                "room-clock-{}",
                hash(
                    b"dfmcp-room-clock-key/1",
                    &[self.target_digest().as_bytes(), plan.as_bytes()]
                )
                .to_hex()
            );
        }
        format!(
            "furniture-clock-{}",
            hash(
                b"dfmcp-furniture-clock-key/1",
                &[self.target_digest().as_bytes(), plan.as_bytes()]
            )
            .to_hex()
        )
    }
    fn monitor_id(&self) -> Digest32 {
        match &self.objective {
            Objective::Furniture => self.spec.id(),
            Objective::Room(room) => room.definition.id(),
        }
    }
    fn validate_plan(&self, plan: &RunPlan) -> Result<()> {
        let tick = plan.before().tick().ok_or_else(corrupt)?;
        require(
            plan.before().eligible()
                && tick >= start_tick(&self.spec)
                && plan.before().generation() == self.binding().manifest().generation
                && plan.key() == self.key(plan.digest())
                && u64::from(plan.spec().game_ticks())
                    <= u64::from(self.spec.goal().timing().max_gap)
                && tick
                    .checked_add(u64::from(plan.spec().game_ticks()))
                    .is_some_and(|end| end <= self.spec.goal().timing().deadline),
        )
    }
    fn view(&self, verified: bool) -> ClockView {
        let runs = self
            .journal
            .cached_records()
            .cloned()
            .map(|entry| {
                let review_seal = self
                    .review
                    .as_ref()
                    .filter(|review| {
                        entry.state() == RunState::Prepared
                            && review.key == entry.plan().key()
                            && review.plan == entry.plan().digest()
                    })
                    .map(|review| review.seal);
                ClockRunView {
                    entry,
                    review_seal: verified.then_some(review_seal).flatten(),
                }
            })
            .collect::<Vec<_>>();
        let fenced = self.journal.fenced();
        ClockView {
            recovery: self.recovery(),
            mode: self.mode(),
            monitor_id: self.monitor_id(),
            pending: fenced || runs.iter().any(|run| !settled(&run.entry)),
            unresolved: fenced || runs.iter().any(|run| run.entry.unresolved()),
            runs,
            high_tick: self.high_tick,
            high_sequence: self.high_sequence,
            readiness: verified.then(|| self.readiness.clone()).flatten(),
            verified: verified && !fenced,
        }
    }
    pub fn cached_view(&self) -> ClockView {
        self.view(false)
    }
    /// Authorize a retained handoff without performing I/O. Both the opening
    /// and current native/canonical Query grants apply at the consumed floor.
    pub fn authorize_disclosure(&self, c: &OperationContext, n: &OperationContext) -> Result<()> {
        authorize(
            &self.spec,
            &self.authority,
            self.high_tick,
            c,
            n,
            Access::Query,
        )
        .map(|_| ())
    }
    fn retain_context_floor(&mut self, c: &OperationContext, n: &OperationContext) {
        // A later authoritative session floor must survive a rejected operation.
        // Foreign contexts cannot modify this owner's remembered history.
        if c.session_id == self.authority.owner
            && n.session_id == self.authority.owner
            && c.request_id == n.request_id
            && c.anchor.fortress_id
                == self
                    .spec
                    .origin()
                    .definition()
                    .binding()
                    .fortress()
                    .fortress_id()
            && n.anchor.fortress_id == FortressId::NIL
        {
            self.high_tick = self
                .high_tick
                .max(c.anchor.tick.get())
                .max(n.anchor.tick.get());
        }
    }
    pub fn inspect(&mut self, c: &OperationContext, n: &OperationContext) -> Result<ClockView> {
        self.retain_context_floor(c, n);
        let (current, native) = authorize(
            &self.spec,
            &self.authority,
            self.high_tick,
            c,
            n,
            Access::Query,
        )?;
        let records = self.journal.records(&native)?;
        if records.len() > MAX_RUNS.min(self.spec.goal().timing().max_observations as usize)
            || records.iter().filter(|run| !settled(run)).count() > 1
        {
            return Err(corrupt());
        }
        for run in &records {
            self.validate_plan(run.plan())?;
            let before = run.plan().before();
            self.high_tick = self.high_tick.max(before.tick().ok_or_else(corrupt)?);
            let mut sequence = before.sequence();
            if let Some(record) = run.native() {
                self.high_tick = self.high_tick.max(record.observed_tick().unwrap_or(0));
                if record.unpause_attempted() {
                    sequence = sequence.checked_add(1).ok_or_else(corrupt)?;
                }
            }
            self.high_sequence = self.high_sequence.max(sequence);
        }
        authorize(
            &self.spec,
            &self.authority,
            self.high_tick,
            &current,
            &native,
            Access::Query,
        )?;
        Ok(self.view(true))
    }
    fn control(
        &mut self,
        c: &OperationContext,
        n: &OperationContext,
        access: Access,
    ) -> Result<(OperationContext, OperationContext)> {
        self.retain_context_floor(c, n);
        if !self.is_control() {
            return Err(denied());
        }
        authorize(&self.spec, &self.authority, self.high_tick, c, n, access)
    }
    fn horizon(&self, plan: &RunPlan, c: &OperationContext, n: &OperationContext) -> Result<()> {
        self.validate_plan(plan)?;
        let ticks = u64::from(plan.spec().game_ticks());
        if ticks
            > c.budget
                .max_game_ticks
                .min(n.budget.max_game_ticks)
                .min(self.authority.game_ticks)
        {
            return Err(bounded());
        }
        let end = plan
            .before()
            .tick()
            .ok_or_else(stale)?
            .checked_add(ticks)
            .ok_or_else(bounded)?;
        let mut current = c.clone();
        let mut native = n.clone();
        current.anchor.tick = GameTick(
            end.max(self.high_tick)
                .max(c.anchor.tick.get())
                .max(n.anchor.tick.get()),
        );
        native.anchor.tick = current.anchor.tick;
        authorize(
            &self.spec,
            &self.authority,
            end,
            &current,
            &native,
            Access::Plan,
        )?;
        Ok(())
    }
    fn source(&self, source: &impl RunSource, exact: bool) -> Result<()> {
        source.manifest().validate()?;
        if source.endpoint() != Some(self.binding().endpoint())
            || !source.manifest().same_software(self.binding().manifest())
            || (exact && source.manifest().generation != self.binding().manifest().generation)
            || (!exact && source.manifest().generation < self.binding().manifest().generation)
        {
            return Err(stale());
        }
        Ok(())
    }
    fn native_plan_context(n: &OperationContext, plan: &RunPlan) -> Result<OperationContext> {
        let before = plan.before();
        let mut current = n.clone();
        current.anchor = StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor {
                epoch: before.generation(),
                sequence: before.sequence(),
            },
            tick: GameTick(before.tick().ok_or_else(stale)?),
            state_hash: before.witness(),
        };
        Ok(current)
    }
    pub fn prepare_guarded<N, F>(
        &mut self,
        source: &mut N,
        request: &RunRequest,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<ClockSample>,
    {
        if !matches!(self.objective, Objective::Furniture) {
            return Err(denied());
        }
        self.prepare_capture(source, request, c, n, |edge, plan, context| {
            guard(edge, plan, context).map(|sample| ReadinessCapture::Furniture(Box::new(sample)))
        })
    }
    pub(crate) fn prepare_room_guarded<N, F>(
        &mut self,
        source: &mut N,
        request: &RunRequest,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<RoomClockSample>,
    {
        if !matches!(self.objective, Objective::Room(_)) {
            return Err(denied());
        }
        self.prepare_capture(source, request, c, n, |edge, plan, context| {
            guard(edge, plan, context).map(|sample| ReadinessCapture::Room(Box::new(sample)))
        })
    }
    fn prepare_capture<N, F>(
        &mut self,
        source: &mut N,
        request: &RunRequest,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<ReadinessCapture>,
    {
        let (current, native) = self.control(c, n, Access::Plan)?;
        if request.monitor_id != self.monitor_id() {
            return Err(stale());
        }
        let requested = request.spec()?;
        let mut work = Work::new(current, native, self.objective.operation_bytes())?;
        let output = work.take(VIEW_BYTES, true)?;
        let inspection = work.take(VIEW_BYTES, true)?;
        let before = self.inspect(&work.c, &inspection)?;
        if before.pending {
            return Err(uncertain());
        }
        if before.runs.len() == MAX_RUNS.min(self.spec.goal().timing().max_observations as usize) {
            return Err(bounded());
        }
        self.abandon_preparation();
        self.source(source, true)?;
        let observation = source.observe(&work.take(OBSERVE_BYTES, true)?)?;
        let prior_tick = self.high_tick;
        let prior_sequence = self.high_sequence;
        self.high_tick = self.high_tick.max(observation.tick().unwrap_or(0));
        self.high_sequence = self.high_sequence.max(observation.sequence());
        self.source(source, true)?;
        if !observation.eligible()
            || observation.tick().is_none_or(|tick| tick < prior_tick)
            || observation.sequence() < prior_sequence
            || observation.generation() != self.binding().manifest().generation
        {
            return Err(stale());
        }
        let candidate = match self.objective {
            Objective::Furniture => "furniture-clock-candidate",
            Objective::Room(_) => "room-clock-candidate",
        };
        let provisional = RunPlan::new(candidate, requested, observation)?;
        let plan = RunPlan::new(
            &self.key(provisional.digest()),
            requested,
            provisional.before().clone(),
        )?;
        self.horizon(&plan, &work.c, &work.n)?;
        let reservation = work.take(ACTION_BYTES, true)?;
        let native = Self::native_plan_context(&reservation, &plan)?;
        let mut canonical = work.take(self.objective.readiness_bytes(), false)?;
        canonical.anchor.tick = native.anchor.tick;
        let tick = ClockFloor::new(self.high_tick);
        let sequence = ClockFloor::new(self.high_sequence);
        let prepared = {
            let mut boundary = Boundary {
                edge: Edge::Prepare,
                objective: &self.objective,
                spec: &self.spec,
                operations: &self.operations,
                authority: &self.authority,
                canonical,
                native: work.n.clone(),
                high_tick: &tick,
                readiness: &mut self.readiness,
                deadline: work.deadline,
                used: false,
            };
            let mut tracked = TrackedSource {
                inner: source,
                high_tick: &tick,
                high_sequence: &sequence,
                deadline: work.deadline,
            };
            self.journal.prepare_guarded(
                &mut tracked,
                plan,
                &work.narrow(&native)?,
                |plan, context| boundary.check(plan, context, &mut guard),
            )
        };
        self.high_tick = self.high_tick.max(tick.get());
        self.high_sequence = self.high_sequence.max(sequence.get());
        let entry = match prepared {
            Ok(entry) => entry,
            Err(cause) => {
                self.abandon_preparation();
                return Err(cause);
            }
        };
        if entry.state() != RunState::Prepared {
            return Err(uncertain());
        }
        let ready = self.readiness.as_ref().ok_or_else(stale)?;
        let review_domain: &[u8] = match self.objective {
            Objective::Furniture => b"dfmcp-furniture-clock-review/1",
            Objective::Room(_) => b"dfmcp-room-clock-review/1",
        };
        let seal = hash(
            review_domain,
            &[
                self.target_digest().as_bytes(),
                &entry.plan().canonical_bytes(),
                ready.digest().as_bytes(),
                &self.authority.owner.get().to_be_bytes(),
            ],
        );
        self.review = Some(Review {
            key: entry.plan().key().into(),
            plan: entry.plan().digest(),
            seal,
        });
        self.publish_prepared(&work, &output)
    }
    /// Every final publication failure withdraws the process-local review,
    /// including deadline expiry before the final journal inspection begins.
    fn publish_prepared(&mut self, work: &Work, output: &OperationContext) -> Result<ClockView> {
        let result = (|| {
            let native = work.narrow(output)?;
            let canonical = work.narrow(&work.c)?;
            self.inspect(&canonical, &native)
        })();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    pub fn commit_guarded<N, F>(
        &mut self,
        source: &mut N,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<ClockSample>,
    {
        if !matches!(self.objective, Objective::Furniture) {
            return Err(denied());
        }
        self.commit_capture(source, seal, confirmed, c, n, |edge, plan, context| {
            guard(edge, plan, context).map(|sample| ReadinessCapture::Furniture(Box::new(sample)))
        })
    }
    pub(crate) fn commit_room_guarded<N, F>(
        &mut self,
        source: &mut N,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<RoomClockSample>,
    {
        if !matches!(self.objective, Objective::Room(_)) {
            return Err(denied());
        }
        self.commit_capture(source, seal, confirmed, c, n, |edge, plan, context| {
            guard(edge, plan, context).map(|sample| ReadinessCapture::Room(Box::new(sample)))
        })
    }
    fn commit_capture<N, F>(
        &mut self,
        source: &mut N,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<ReadinessCapture>,
    {
        let (current, native) = self.control(c, n, Access::Plan)?;
        if !confirmed {
            return Err(denied());
        }
        let review = self
            .review
            .take()
            .filter(|review| review.seal == seal)
            .ok_or_else(denied)?;
        let mut work = Work::new(current, native, self.objective.operation_bytes())?;
        let output = work.take(VIEW_BYTES, true)?;
        let inspection = work.take(VIEW_BYTES, true)?;
        let before = self.inspect(&work.c, &inspection)?;
        let selected = before
            .runs
            .iter()
            .find(|run| {
                run.entry.plan().key() == review.key && run.entry.plan().digest() == review.plan
            })
            .ok_or_else(corrupt)?;
        if selected.entry.state() != RunState::Prepared || before.unresolved {
            return Err(uncertain());
        }
        let plan = selected.entry.plan().clone();
        self.horizon(&plan, &work.c, &work.n)?;
        if plan
            .before()
            .tick()
            .is_none_or(|tick| tick < self.high_tick)
            || plan.before().sequence() < self.high_sequence
        {
            return Err(stale());
        }
        self.source(source, true)?;
        let reservation = work.take(ACTION_BYTES, true)?;
        let native = Self::native_plan_context(&reservation, &plan)?;
        let mut canonical = work.take(self.objective.readiness_bytes(), false)?;
        canonical.anchor.tick = native.anchor.tick;
        self.readiness = None;
        let tick = ClockFloor::new(self.high_tick);
        let sequence = ClockFloor::new(self.high_sequence);
        let result = {
            let mut boundary = Boundary {
                edge: Edge::Commit,
                objective: &self.objective,
                spec: &self.spec,
                operations: &self.operations,
                authority: &self.authority,
                canonical,
                native: work.n.clone(),
                high_tick: &tick,
                readiness: &mut self.readiness,
                deadline: work.deadline,
                used: false,
            };
            let mut tracked = TrackedSource {
                inner: source,
                high_tick: &tick,
                high_sequence: &sequence,
                deadline: work.deadline,
            };
            self.journal.commit_guarded(
                &mut tracked,
                plan.key(),
                plan.digest(),
                &work.narrow(&native)?,
                |plan, context| boundary.check(plan, context, &mut guard),
            )
        };
        self.high_tick = self.high_tick.max(tick.get());
        self.high_sequence = self.high_sequence.max(sequence.get());
        if let Err(cause) = result {
            self.readiness = None;
            return Err(cause);
        }
        // Keep the pre-dispatch assessment only for negative continuity handling
        // by the caller. It is historical and grants no further review permit.
        self.inspect(&work.narrow(&work.c)?, &work.narrow(&output)?)
    }
    pub fn reconcile<N: RunSource>(
        &mut self,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        self.retain_context_floor(c, n);
        let (current, native) = authorize(
            &self.spec,
            &self.authority,
            self.high_tick,
            c,
            n,
            Access::Query,
        )?;
        let mut work = Work::new(current, native, OPERATION_BYTES)?;
        let output = work.take(VIEW_BYTES, true)?;
        let inspection = work.take(VIEW_BYTES, true)?;
        let before = self.inspect(&work.c, &inspection)?;
        if !before.needs_reconciliation() {
            return Ok(before);
        }
        if self.mode() == RunMode::Offline {
            return Err(denied());
        }
        let active = before.active().ok_or_else(uncertain)?;
        self.source(source, false)?;
        let mut recorded = RecoverySource {
            inner: source,
            high_tick: &mut self.high_tick,
            high_sequence: &mut self.high_sequence,
        };
        let result = self.journal.reconcile(
            &mut recorded,
            active.entry.plan().key(),
            active.entry.plan().digest(),
            &work.take(ACTION_BYTES, true)?,
        );
        self.abandon_preparation();
        result?;
        self.inspect(&work.narrow(&work.c)?, &work.narrow(&output)?)
    }
    pub fn cancel(
        &mut self,
        source: Option<&mut dyn RunSource>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        let (current, native) = self.control(c, n, Access::Cancel)?;
        let mut work = Work::new(current, native, OPERATION_BYTES)?;
        let output = work.take(VIEW_BYTES, true)?;
        let inspection = work.take(VIEW_BYTES, true)?;
        let before = self.inspect(&work.c, &inspection)?;
        self.abandon_preparation();
        if let Some(active) = before.active().filter(|run| !run.entry.terminal()) {
            let mut recorded = source.map(|inner| RecoverySource {
                inner,
                high_tick: &mut self.high_tick,
                high_sequence: &mut self.high_sequence,
            });
            self.journal.cancel(
                recorded.as_mut().map(|value| value as &mut dyn RunSource),
                active.entry.plan().key(),
                active.entry.plan().digest(),
                &work.take(ACTION_BYTES, true)?,
            )?;
        }
        self.inspect(&work.narrow(&work.c)?, &work.narrow(&output)?)
    }
}

fn narrow(deadline: Instant, c: &OperationContext) -> Result<OperationContext> {
    let left = deadline
        .checked_duration_since(Instant::now())
        .ok_or_else(bounded)?
        .as_millis();
    if left == 0 {
        return Err(bounded());
    }
    let mut current = c.clone();
    current.budget.max_wall_millis = current
        .budget
        .max_wall_millis
        .min(u64::try_from(left).map_err(|_| bounded())?);
    Ok(current)
}
struct Boundary<'a> {
    edge: Edge,
    objective: &'a Objective,
    spec: &'a MonitorDefinition,
    operations: &'a Manifest,
    authority: &'a Authority,
    canonical: OperationContext,
    native: OperationContext,
    high_tick: &'a ClockFloor<u64>,
    readiness: &'a mut Option<Readiness>,
    deadline: Instant,
    used: bool,
}
impl Boundary<'_> {
    fn check(
        &mut self,
        plan: &RunPlan,
        n: &OperationContext,
        guard: &mut impl FnMut(Edge, &RunPlan, &OperationContext) -> Result<ReadinessCapture>,
    ) -> Result<()> {
        if self.used {
            return Err(denied());
        }
        self.used = true;
        let (mut canonical, _) = authorize(
            self.spec,
            self.authority,
            self.high_tick.get(),
            &narrow(self.deadline, &self.canonical)?,
            &narrow(self.deadline, &self.native)?,
            Access::Plan,
        )?;
        canonical.budget.max_wall_millis = canonical
            .budget
            .max_wall_millis
            .min(n.budget.max_wall_millis);
        let capture = guard(self.edge, plan, &canonical)?;
        // Retain a decoded source clock before stricter identity, authority or
        // original-goal checks can reject this already consumed observation.
        let readiness = match (self.objective, capture) {
            (Objective::Furniture, ReadinessCapture::Furniture(capture)) => {
                let observed = decode_capture(&capture.sample)?;
                self.high_tick
                    .set(self.high_tick.get().max(observed.jobs.tick().get()));
                Readiness::from_capture(
                    self.spec,
                    self.operations,
                    &capture.progress,
                    &capture.sample,
                    &canonical,
                    observed,
                )?
            }
            (Objective::Room(room), ReadinessCapture::Room(capture)) => {
                let mut tick = self
                    .high_tick
                    .get()
                    .max(capture.progress.observed_tick_floor);
                let decoded = capture.sample.retain_observed_tick(&mut tick);
                self.high_tick.set(tick);
                decoded?;
                Readiness::from_room_capture(
                    &room.definition,
                    &room.source,
                    &capture.progress,
                    &capture.sample,
                    &canonical,
                )?
            }
            _ => return Err(denied()),
        };
        if Some(readiness.tick()) != plan.before().tick() || readiness.tick() < self.high_tick.get()
        {
            return Err(stale());
        }
        authorize(
            self.spec,
            self.authority,
            self.high_tick.get(),
            &canonical,
            &self.native,
            Access::Plan,
        )?;
        narrow(self.deadline, n)?;
        *self.readiness = Some(readiness);
        Ok(())
    }
}
struct TrackedSource<'a, N> {
    inner: &'a mut N,
    high_tick: &'a ClockFloor<u64>,
    high_sequence: &'a ClockFloor<u64>,
    deadline: Instant,
}
impl<N> TrackedSource<'_, N> {
    fn record(&self, record: &RunRecord) -> Result<()> {
        let mut tick = self.high_tick.get();
        let mut sequence = self.high_sequence.get();
        retain_record(record, &mut tick, &mut sequence)?;
        self.high_tick.set(tick);
        self.high_sequence.set(sequence);
        Ok(())
    }
}
impl<N: RunSource> RunSource for TrackedSource<'_, N> {
    fn manifest(&self) -> &RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        let before_tick = self.high_tick.get();
        let before_sequence = self.high_sequence.get();
        let raw = self.inner.observe(&narrow(self.deadline, c)?)?;
        self.high_tick
            .set(self.high_tick.get().max(raw.tick().unwrap_or(0)));
        self.high_sequence
            .set(self.high_sequence.get().max(raw.sequence()));
        if raw.tick().is_none_or(|tick| tick < before_tick) || raw.sequence() < before_sequence {
            return Err(stale());
        }
        Ok(raw)
    }
    fn prepare(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        let record = self.inner.prepare(plan, &narrow(self.deadline, c)?)?;
        self.record(&record)?;
        Ok(record)
    }
    fn commit(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        let record = self.inner.commit(plan, &narrow(self.deadline, c)?)?;
        self.record(&record)?;
        Ok(record)
    }
    fn query(&mut self, _: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        Err(denied())
    }
    fn cancel(&mut self, _: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        Err(denied())
    }
}

fn retain_record(record: &RunRecord, high_tick: &mut u64, high_sequence: &mut u64) -> Result<()> {
    RunRecord::decode(record.canonical_bytes())?;
    *high_tick = (*high_tick).max(record.observed_tick().unwrap_or(0));
    let sequence = record
        .plan()
        .before()
        .sequence()
        .checked_add(u64::from(record.unpause_attempted()))
        .ok_or_else(corrupt)?;
    *high_sequence = (*high_sequence).max(sequence);
    Ok(())
}
struct RecoverySource<'a> {
    inner: &'a mut dyn RunSource,
    high_tick: &'a mut u64,
    high_sequence: &'a mut u64,
}
impl RunSource for RecoverySource<'_> {
    fn manifest(&self) -> &RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, _: &OperationContext) -> Result<RunObservation> {
        Err(denied())
    }
    fn prepare(&mut self, _: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        Err(denied())
    }
    fn commit(&mut self, _: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        Err(denied())
    }
    fn query(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        let record = self.inner.query(plan, c)?;
        if let Some(record) = &record {
            retain_record(record, self.high_tick, self.high_sequence)?;
        }
        Ok(record)
    }
    fn cancel(&mut self, plan: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        let record = self.inner.cancel(plan, c)?;
        retain_record(&record, self.high_tick, self.high_sequence)?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests;
