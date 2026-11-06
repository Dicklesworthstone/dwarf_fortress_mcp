//! Reusable bounded Run/1.13 supervision for an immutable semantic goal target.
//!
//! Run/1.13 identifies a source incarnation, not a named fortress. Its authority
//! is supplied independently from the canonical goal's fortress authority. This
//! owner never rewrites a fortress grant into a source-bound grant. The caller's
//! final guard checks original custody and fresh goal evidence after the durable
//! clock marker; the native coordinator still owns single-attempt unpause.
use super::{Work, denied, digest, exhausted, invalid};
use dfmcp_adapter::bounded_run::journal::{DurableRun, RunJournal, RunMode, RunState};
use dfmcp_adapter::bounded_run::rpc::{RunManifest, RunSource};
use dfmcp_adapter::bounded_run::{RunObservation, RunPlan, RunRecord, RunSpec};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor,
    OperationContext, Result, RiskTier, StateAnchor, WorkBudget,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::marker::PhantomData;
use std::net::SocketAddr;
use std::time::Instant;

/// One initial sample plus explicitly reviewed advances can establish the
/// existing 64-sample original goal. The native journal bound is unchanged.
pub const MAX_RUNS: usize = 64;
const VIEW_BYTES: u64 = 4 * 1024 * 1024;
const ACTION_BYTES: u64 = 32 * 1024 * 1024;

fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
}
fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}
fn uncertain() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::EffectIndeterminate,
        "settle or cancel the retained clock run; never repeat unpause",
    )
}
/// Trusted target binding supplied by the semantic owner. Implementations bind
/// one immutable original goal and its verified effect custody; the supervisor
/// owns bounded native Run/1.13 dispatch, recovery, deadlines and source floors.
pub trait GoalClockTarget {
    const REQUEST_KIND: &'static str;
    const CANDIDATE_KEY: &'static str;
    const PURPOSE: &'static str;
    fn digest(&self) -> Digest32;
    fn original_paired_seal(&self) -> Digest32;
    fn original_tick(&self) -> u64;
    fn context(&self, c: &OperationContext) -> Result<()>;
    fn capture(&self, capture: &RunObservation) -> Result<u64>;
    fn plan(&self, plan: &RunPlan) -> Result<()>;
    fn key(&self, digest: Digest32) -> String;
    fn seal(&self, journal: Digest32, plan: &RunPlan) -> Digest32;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    pub kind: String,
    pub original_paired_seal: String,
    pub game_ticks: u32,
    pub wall_millis: u32,
}
impl RunRequest {
    pub fn parse(raw: &str, kind: &str) -> Result<Self> {
        if raw.len() > 2048 {
            return Err(exhausted("clock request exceeds 2048 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed original-goal clock request"))?;
        request.validate(kind)?;
        Ok(request)
    }
    fn validate(&self, kind: &str) -> Result<()> {
        if self.kind != kind {
            return Err(invalid(
                "clock request kind differs from its semantic owner",
            ));
        }
        digest(&self.original_paired_seal)?;
        self.spec()?;
        Ok(())
    }
    fn spec(&self) -> Result<RunSpec> {
        RunSpec::new(self.game_ticks, self.wall_millis)
    }
}

fn settled(entry: &DurableRun) -> bool {
    entry.terminal() && !entry.unresolved()
}
#[derive(Clone, Debug)]
pub struct ClockRunView {
    pub review_seal: Digest32,
    pub entry: DurableRun,
    purpose: &'static str,
}
impl ClockRunView {
    pub fn to_json(&self) -> Value {
        let p = self.entry.plan();
        let native = self.entry.native().map(|n| json!({
            "phase":n.phase().as_str(), "reason":n.reason().as_str(),
            "unpause_attempted":n.unpause_attempted(), "historical_pause_verified":n.pause_verified(),
            "observed_tick":n.observed_tick(), "observed_tick_overshoot":n.observed_tick_overshoot(),
            "receipt":n.receipt().to_hex(), "current_pause_proven":false, "original_goal_proven":false,
        }));
        json!({"review_seal":self.review_seal.to_hex(), "key":p.key(), "plan_digest":p.digest().to_hex(),
            "state":self.entry.state().as_str(), "pending":!settled(&self.entry), "unresolved":self.entry.unresolved(),
            "before_tick":p.before().tick(), "source_generation":p.before().generation(),
            "source_sequence":p.before().sequence(), "game_ticks":p.spec().game_ticks(),
            "wall_millis":p.spec().wall_ms(), "purpose":self.purpose, "native":native})
    }
}
#[derive(Clone, Debug)]
pub struct ClockView {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub mode: RunMode,
    pub target_digest: Option<Digest32>,
    pub original_paired_seal: Option<Digest32>,
    pub runs: Vec<ClockRunView>,
    pub pending: bool,
    pub unresolved: bool,
    pub high_tick: u64,
    pub high_sequence: u64,
}
impl ClockView {
    pub fn has_runs(&self) -> bool {
        !self.runs.is_empty()
    }
    pub fn may_be_running(&self) -> bool {
        self.unresolved
    }
    /// Allows attempting a fresh independently authorized observation. No clock
    /// receipt is treated as proof of present pause or the current goal.
    pub fn can_sample_goal(&self) -> bool {
        !self.unresolved
    }
    pub fn active(&self) -> Option<&ClockRunView> {
        self.runs.iter().find(|r| !settled(&r.entry))
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.active().is_some_and(|r| !r.entry.terminal())
    }
    pub fn cancellation_needs_source(&self) -> bool {
        self.active().is_some_and(|r| {
            matches!(
                r.entry.state(),
                RunState::DispatchStarted | RunState::Tracking | RunState::CancelRequested
            )
        })
    }
    pub fn to_json(&self) -> Value {
        json!({"journal_id":self.journal_id.to_hex(), "head":self.head.to_hex(),
            "mode":match self.mode { RunMode::Control=>"control", RunMode::Recover=>"reconcile", RunMode::Offline=>"offline" },
            "target_digest":self.target_digest.map(|d|d.to_hex()),
            "original_paired_seal":self.original_paired_seal.map(|d|d.to_hex()),
            "run_count":self.runs.len(), "pending":self.pending, "unresolved":self.unresolved,
            "needs_reconciliation":self.needs_reconciliation(), "may_be_running":self.may_be_running(),
            "current_pause_proven":false, "original_goal_proven":false,
            "high_tick":self.high_tick, "high_source_sequence":self.high_sequence,
            "clock_floor_semantics":"native source floors only; rejected read floors are process-local",
            "source_identity_semantics":"source_incarnation_only_not_named_fortress_lineage",
            "active_run":self.active().map(ClockRunView::to_json),
            "runs":self.runs.iter().map(ClockRunView::to_json).collect::<Vec<_>>()})
    }
}

/// Check the two authorities without converting either grant set. The operation
/// shells must independently issue the native NIL scope and the real-fortress
/// scope; sharing a request identity does not manufacture either authority.
fn contexts(c: &OperationContext, n: &OperationContext) -> Result<()> {
    if c.anchor.fortress_id == FortressId::NIL
        || n.anchor.fortress_id != FortressId::NIL
        || c.session_id != n.session_id
        || c.request_id != n.request_id
        || c.cancellation_requested
        || n.cancellation_requested
    {
        return Err(denied(
            "clock requires separate canonical and source-bound authority for the same active request",
        ));
    }
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    n.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)
}
fn budget(a: WorkBudget, b: WorkBudget) -> WorkBudget {
    WorkBudget {
        max_bytes: a.max_bytes.min(b.max_bytes),
        max_wall_millis: a.max_wall_millis.min(b.max_wall_millis),
        max_game_ticks: a.max_game_ticks.min(b.max_game_ticks),
        max_entities: a.max_entities.min(b.max_entities),
        max_actions: a.max_actions.min(b.max_actions),
        max_output_tokens: a.max_output_tokens.min(b.max_output_tokens),
    }
}
fn work(c: &OperationContext, n: &OperationContext) -> Result<Work> {
    contexts(c, n)?;
    let mut narrowed = c.clone();
    narrowed.budget = budget(c.budget, n.budget);
    Work::new(&narrowed)
}
fn native_context(
    native: &OperationContext,
    reservation: &OperationContext,
    observation: Option<&RunObservation>,
) -> Result<OperationContext> {
    contexts(reservation, native)?;
    let mut result = native.clone();
    result.budget = budget(native.budget, reservation.budget);
    if let Some(observation) = observation {
        // This is the native source's own observation anchor. Canonical fortress
        // identity and grants never enter it; the two clocks are bracketed by
        // the exact tick checks and independently supplied effect guard.
        result.anchor = StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor {
                epoch: observation.generation(),
                sequence: observation.sequence(),
            },
            tick: GameTick(
                observation
                    .tick()
                    .ok_or_else(|| stale("native clock observation has no tick"))?,
            ),
            state_hash: observation.witness(),
        };
    } else {
        result.anchor.tick = result.anchor.tick.max(reservation.anchor.tick);
    }
    Ok(result)
}

pub struct Clock<J: EffectJournalStorage, T: GoalClockTarget> {
    journal: RunJournal<J>,
    observed_tick_floor: u64,
    observed_sequence_floor: u64,
    target: PhantomData<T>,
}
impl<J: EffectJournalStorage, T: GoalClockTarget> Clock<J, T> {
    pub fn new(journal: RunJournal<J>) -> Self {
        Self {
            journal,
            observed_tick_floor: 0,
            observed_sequence_floor: 0,
            target: PhantomData,
        }
    }
    pub fn high_tick(&self) -> u64 {
        self.observed_tick_floor
    }
    pub fn mode(&self) -> RunMode {
        self.journal.mode()
    }
    pub fn is_control(&self) -> bool {
        self.mode() == RunMode::Control
    }
    fn view(
        &mut self,
        target: Option<&T>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        contexts(c, n)?;
        if let Some(t) = target {
            t.context(c)?;
        }
        let entries = self.journal.records(&native_context(n, c, None)?)?;
        if entries.len() > MAX_RUNS || (target.is_none() && !entries.is_empty()) {
            return Err(corrupt(
                "clock custody has excess runs or no exact original goal association",
            ));
        }
        let mut runs = Vec::with_capacity(entries.len());
        let mut high_tick = target.map_or(0, |t| t.original_tick());
        let mut high_sequence = 0;
        for entry in entries {
            let t = target.ok_or_else(|| corrupt("clock history has no original goal target"))?;
            t.plan(entry.plan())?;
            high_tick = high_tick.max(t.capture(entry.plan().before())?);
            let mut sequence = entry.plan().before().sequence();
            if let Some(record) = entry.native() {
                if let Some(tick) = record.observed_tick() {
                    high_tick = high_tick.max(tick);
                }
                if record.unpause_attempted() {
                    sequence = sequence
                        .checked_add(1)
                        .ok_or_else(|| corrupt("clock sequence overflow"))?;
                }
            }
            high_sequence = high_sequence.max(sequence);
            runs.push(ClockRunView {
                review_seal: t.seal(self.journal.id(), entry.plan()),
                entry,
                purpose: T::PURPOSE,
            });
        }
        self.observed_tick_floor = self.observed_tick_floor.max(high_tick);
        self.observed_sequence_floor = self.observed_sequence_floor.max(high_sequence);
        let pending = runs.iter().any(|r| !settled(&r.entry));
        let unresolved = runs.iter().any(|r| r.entry.unresolved());
        if runs.iter().filter(|r| !settled(&r.entry)).count() > 1 {
            return Err(corrupt("more than one unsettled original-goal clock run"));
        }
        let view = ClockView {
            journal_id: self.journal.id(),
            head: self.journal.head(),
            mode: self.mode(),
            target_digest: target.map(|t| t.digest()),
            original_paired_seal: target.map(|t| t.original_paired_seal()),
            runs,
            pending,
            unresolved,
            high_tick: self.observed_tick_floor,
            high_sequence: self.observed_sequence_floor,
        };
        Self::authorize_floor(&view, c, n, false)?;
        Ok(view)
    }
    fn authorize_floor(
        view: &ClockView,
        c: &OperationContext,
        n: &OperationContext,
        clock: bool,
    ) -> Result<()> {
        for context in [c, n] {
            // Authority checks only; no native tick replaces the published
            // canonical state anchor or establishes current state.
            let mut current = context.clone();
            current.anchor.tick = GameTick(current.anchor.tick.get().max(view.high_tick));
            current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            if clock {
                current.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            }
        }
        Ok(())
    }
    pub fn inspect(
        &mut self,
        target: Option<&T>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        let mut work = work(c, n)?;
        self.view(target, &work.reserve(VIEW_BYTES)?, n)
    }
    fn control(&self, target: &T, c: &OperationContext, n: &OperationContext) -> Result<()> {
        contexts(c, n)?;
        target.context(c)?;
        if !self.is_control() {
            return Err(denied("clock recovery mode cannot become control"));
        }
        for context in [c, n] {
            context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
    }
    fn authorize_plan(
        target: &T,
        plan: &RunPlan,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<()> {
        target.plan(plan)?;
        let end = target
            .capture(plan.before())?
            .checked_add(u64::from(plan.spec().game_ticks()))
            .ok_or_else(|| exhausted("clock horizon overflow"))?;
        for context in [c, n] {
            if u64::from(plan.spec().game_ticks()) > context.budget.max_game_ticks {
                return Err(exhausted(
                    "clock run exceeds an independently supplied tick allowance",
                ));
            }
            let mut horizon = context.clone();
            horizon.anchor.tick = GameTick(end.max(context.anchor.tick.get()));
            horizon.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    // Both independently issued authority scopes are explicit at this boundary.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_guarded<N, F>(
        &mut self,
        target: &T,
        request: &RunRequest,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(&OperationContext) -> Result<()>,
    {
        self.control(target, c, n)?;
        request.validate(T::REQUEST_KIND)?;
        if digest(&request.original_paired_seal)? != target.original_paired_seal() {
            return Err(denied(
                "run request must name the exact original paired review",
            ));
        }
        let mut work = work(c, n)?;
        let output = work.reserve(VIEW_BYTES)?;
        let before = self.view(Some(target), &work.reserve(VIEW_BYTES)?, n)?;
        if let Some(active) = before.active() {
            let p = active.entry.plan();
            if p.spec().game_ticks() == request.game_ticks
                && p.spec().wall_ms() == request.wall_millis
                && p.before().tick() == Some(c.anchor.tick.get())
            {
                return self.inspect(Some(target), &work.narrow(&output)?, n);
            }
            return Err(uncertain());
        }
        let capture = self.journal.observe(
            &mut ObservedSource {
                inner: source,
                tick_floor: &mut self.observed_tick_floor,
                sequence_floor: &mut self.observed_sequence_floor,
            },
            &native_context(n, &work.reserve(ACTION_BYTES)?, None)?,
        )?;
        let tick = target.capture(&capture)?;
        if tick != c.anchor.tick.get()
            || tick < n.anchor.tick.get()
            || tick < before.high_tick
            || capture.sequence() < before.high_sequence
        {
            return Err(stale(
                "clock capture must match the fresh goal tick and retained source floors",
            ));
        }
        let provisional = RunPlan::new(T::CANDIDATE_KEY, request.spec()?, capture)?;
        let plan = RunPlan::new(
            &target.key(provisional.digest()),
            provisional.spec(),
            provisional.before().clone(),
        )?;
        Self::authorize_plan(target, &plan, c, n)?;
        if before.runs.iter().any(|r| r.entry.plan() == &plan) {
            return self.inspect(Some(target), &work.narrow(&output)?, n);
        }
        if before.runs.len() == MAX_RUNS {
            return Err(exhausted("original goal retains at most 64 clock runs"));
        }
        let canonical = work.reserve(ACTION_BYTES)?;
        let native = native_context(n, &canonical, Some(plan.before()))?;
        let mut observed = ObservedSource {
            inner: source,
            tick_floor: &mut self.observed_tick_floor,
            sequence_floor: &mut self.observed_sequence_floor,
        };
        let mut checked = GuardedSource {
            inner: &mut observed,
            guard: &mut guard,
            canonical,
            started: Instant::now(),
            edge: GuardEdge::Prepare,
        };
        self.journal.prepare(&mut checked, plan, &native)?;
        self.inspect(Some(target), &work.narrow(&output)?, n)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn commit_guarded<N, F>(
        &mut self,
        target: &T,
        seal: Digest32,
        confirmed: bool,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
        mut guard: F,
    ) -> Result<ClockView>
    where
        N: RunSource,
        F: FnMut(&OperationContext) -> Result<()>,
    {
        self.control(target, c, n)?;
        if !confirmed {
            return Err(denied(
                "confirm the separately reviewed original-goal clock run",
            ));
        }
        let mut work = work(c, n)?;
        let output = work.reserve(VIEW_BYTES)?;
        let view = self.view(Some(target), &work.reserve(VIEW_BYTES)?, n)?;
        let run = view
            .runs
            .iter()
            .find(|r| r.review_seal == seal)
            .ok_or_else(|| denied("clock commit requires its exact target-bound review seal"))?;
        if settled(&run.entry) {
            return self.inspect(Some(target), &work.narrow(&output)?, n);
        }
        if run.entry.state() != RunState::Prepared {
            return Err(uncertain());
        }
        let p = run.entry.plan();
        if p.before().tick() != Some(c.anchor.tick.get())
            || c.anchor.tick.get() < n.anchor.tick.get()
            || c.anchor.tick.get() < view.high_tick
            || p.before().sequence() < view.high_sequence
        {
            return Err(stale(
                "clock commit requires the fresh unchanged original goal tick",
            ));
        }
        Self::authorize_plan(target, p, c, n)?;
        let canonical = work.reserve(ACTION_BYTES)?;
        let native = native_context(n, &canonical, Some(p.before()))?;
        let mut observed = ObservedSource {
            inner: source,
            tick_floor: &mut self.observed_tick_floor,
            sequence_floor: &mut self.observed_sequence_floor,
        };
        let mut checked = GuardedSource {
            inner: &mut observed,
            guard: &mut guard,
            canonical,
            started: Instant::now(),
            edge: GuardEdge::Commit,
        };
        self.journal
            .commit(&mut checked, p.key(), p.digest(), &native)?;
        self.inspect(Some(target), &work.narrow(&output)?, n)
    }
    pub fn reconcile<N: RunSource>(
        &mut self,
        target: &T,
        source: &mut N,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        target.context(c)?;
        let mut work = work(c, n)?;
        let output = work.reserve(VIEW_BYTES)?;
        let view = self.view(Some(target), &work.reserve(VIEW_BYTES)?, n)?;
        if view.needs_reconciliation() {
            let run = view.active().ok_or_else(uncertain)?;
            self.journal.reconcile(
                &mut ObservedSource {
                    inner: source,
                    tick_floor: &mut self.observed_tick_floor,
                    sequence_floor: &mut self.observed_sequence_floor,
                },
                run.entry.plan().key(),
                run.entry.plan().digest(),
                &native_context(n, &work.reserve(ACTION_BYTES)?, None)?,
            )?;
        }
        self.inspect(Some(target), &work.narrow(&output)?, n)
    }
    pub fn cancel(
        &mut self,
        target: &T,
        source: Option<&mut dyn RunSource>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<ClockView> {
        // A safety pause remains available after the goal deadline and does not
        // require current goal truth, Plan, Observe or assignment authority.
        target.context(c)?;
        contexts(c, n)?;
        if !self.is_control() {
            return Err(denied("clock recovery mode cannot request cancellation"));
        }
        let mut work = work(c, n)?;
        let output = work.reserve(VIEW_BYTES)?;
        let view = self.view(Some(target), &work.reserve(VIEW_BYTES)?, n)?;
        Self::authorize_floor(&view, c, n, true)?;
        if let Some(run) = view.active() {
            let mut observed = source.map(|inner| ObservedSource {
                inner,
                tick_floor: &mut self.observed_tick_floor,
                sequence_floor: &mut self.observed_sequence_floor,
            });
            self.journal.cancel(
                observed.as_mut().map(|source| source as &mut dyn RunSource),
                run.entry.plan().key(),
                run.entry.plan().digest(),
                &native_context(n, &work.reserve(ACTION_BYTES)?, None)?,
            )?;
        }
        self.inspect(Some(target), &work.narrow(&output)?, n)
    }
}

/// Consume complete native evidence before the journal's subsequent witness,
/// identity, authority or storage checks can reject it. A refused preflight may
/// observe a later tick without creating any durable run record; it must still
/// prevent a later rollback from reviving old goal evidence in this owner.
/// These floors grant no eligibility, current pause proof or dispatch permit.
struct ObservedSource<'a, N: ?Sized> {
    inner: &'a mut N,
    tick_floor: &'a mut u64,
    sequence_floor: &'a mut u64,
}
impl<N: ?Sized> ObservedSource<'_, N> {
    fn observation(&mut self, observation: &RunObservation) {
        if let Some(tick) = observation.tick() {
            *self.tick_floor = (*self.tick_floor).max(tick);
        }
        *self.sequence_floor = (*self.sequence_floor).max(observation.sequence());
    }
    fn record(&mut self, record: &RunRecord) {
        self.observation(record.plan().before());
        if let Some(tick) = record.observed_tick() {
            *self.tick_floor = (*self.tick_floor).max(tick);
        }
        if record.unpause_attempted() {
            *self.sequence_floor =
                (*self.sequence_floor).max(record.plan().before().sequence().saturating_add(1));
        }
    }
}
impl<N: RunSource + ?Sized> RunSource for ObservedSource<'_, N> {
    fn manifest(&self) -> &RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        let observation = self.inner.observe(c)?;
        self.observation(&observation);
        Ok(observation)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        let record = self.inner.prepare(p, c)?;
        self.record(&record);
        Ok(record)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        let record = self.inner.commit(p, c)?;
        self.record(&record);
        Ok(record)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        let record = self.inner.query(p, c)?;
        if let Some(record) = &record {
            self.record(record);
        }
        Ok(record)
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        let record = self.inner.cancel(p, c)?;
        self.record(&record);
        Ok(record)
    }
}

enum GuardEdge {
    Prepare,
    Commit,
}
struct GuardedSource<'a, N, F> {
    inner: &'a mut N,
    guard: &'a mut F,
    canonical: OperationContext,
    started: Instant,
    edge: GuardEdge,
}
impl<N, F: FnMut(&OperationContext) -> Result<()>> GuardedSource<'_, N, F> {
    fn guarded_context(&mut self, native: &OperationContext) -> Result<OperationContext> {
        let started = Instant::now();
        let mut canonical = self.canonical.clone();
        let elapsed = u64::try_from(self.started.elapsed().as_millis())
            .map_err(|_| exhausted("clock custody guard deadline exhausted"))?;
        canonical.budget.max_wall_millis = canonical
            .budget
            .max_wall_millis
            .checked_sub(elapsed)
            .filter(|n| *n > 0)
            .ok_or_else(|| exhausted("clock custody guard deadline exhausted"))?;
        // Canonical custody has its own prepaid reservation. The native RPC
        // allowance is already separately bounded by the journal coordinator.
        canonical.budget.max_wall_millis = canonical
            .budget
            .max_wall_millis
            .min(native.budget.max_wall_millis);
        (self.guard)(&canonical)?;
        let elapsed = u64::try_from(started.elapsed().as_millis())
            .map_err(|_| exhausted("clock custody guard deadline exhausted"))?;
        let mut narrowed = native.clone();
        narrowed.budget.max_wall_millis = native
            .budget
            .max_wall_millis
            .checked_sub(elapsed)
            .filter(|n| *n > 0)
            .ok_or_else(|| exhausted("clock custody guard deadline exhausted"))?;
        Ok(narrowed)
    }
}
impl<N: RunSource, F: FnMut(&OperationContext) -> Result<()>> RunSource
    for GuardedSource<'_, N, F>
{
    fn manifest(&self) -> &RunManifest {
        self.inner.manifest()
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        self.inner.endpoint()
    }
    fn observe(&mut self, c: &OperationContext) -> Result<RunObservation> {
        self.inner.observe(c)
    }
    fn prepare(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        if matches!(self.edge, GuardEdge::Prepare) {
            let narrowed = self.guarded_context(c)?;
            return self.inner.prepare(p, &narrowed);
        }
        self.inner.prepare(p, c)
    }
    fn commit(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        if matches!(self.edge, GuardEdge::Commit) {
            let narrowed = self.guarded_context(c)?;
            return self.inner.commit(p, &narrowed);
        }
        self.inner.commit(p, c)
    }
    fn query(&mut self, p: &RunPlan, c: &OperationContext) -> Result<Option<RunRecord>> {
        self.inner.query(p, c)
    }
    fn cancel(&mut self, p: &RunPlan, c: &OperationContext) -> Result<RunRecord> {
        self.inner.cancel(p, c)
    }
}
