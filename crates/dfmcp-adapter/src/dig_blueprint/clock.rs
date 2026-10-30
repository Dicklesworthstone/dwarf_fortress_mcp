//! Separately reviewed finite run/1.13 intervals for one original blueprint.
//!
//! The existing native coordinator owns durable intent and one-attempt unpause.
//! This layer associates every retained run with the entire immutable monitor,
//! its original designation receipt set and unchanged temporal policy. The clock
//! source has no named-fortress lineage; its independently issued NIL authority
//! is never manufactured by rewriting the named-fortress terrain grants.
use std::cell::Cell as ClockFloor;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, CapabilityGrant, DfmcpError, Digest32, ErrorCode, FortressId, GameTick,
    ObservationCursor, OperationContext, Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_world::map_region::{Cell, Shape};

use super::monitor::{MonitorSpec, capture_digest};
use crate::bounded_run::journal::{DurableRun, RunBinding, RunJournal, RunMode, RunState};
use crate::bounded_run::rpc::{RunManifest, RunSource};
use crate::bounded_run::{RunObservation, RunPlan, RunRecord, RunSpec};
use crate::control_effect_journal::EffectJournalStorage;
use crate::live_map::LiveMapObservation;

pub const MAX_RUNS: usize = 64;
pub const VIEW_BYTES: u64 = 8 * 1024 * 1024;
pub const OPERATION_BYTES: u64 = 96 * 1024 * 1024;
const ACTION_BYTES: u64 = 32 * 1024 * 1024;
const OBSERVE_BYTES: u64 = 1024 * 1024;

fn error(code: ErrorCode, text: &str) -> DfmcpError {
    DfmcpError::new(code, text)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "clock history differs from the entire original blueprint obligation",
    )
}
fn denied() -> DfmcpError {
    error(
        ErrorCode::CapabilityDenied,
        "blueprint clock lacks its separate current original and native authority",
    )
}
fn stale() -> DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "fresh blueprint terrain and original clock source no longer agree",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "complete blueprint clock work or original finite horizon is exhausted",
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

/// A complete independent map acquisition qualified for the exact sparse mask.
/// Eligibility is neither goal completion, reachability nor structural safety.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readiness {
    tick: u64,
    digest: Digest32,
    pending_tiles: u32,
    cleared_tiles: u32,
}
impl Readiness {
    pub fn capture(spec: &MonitorSpec, capture: LiveMapObservation) -> Result<Self> {
        spec.validate_capture(&capture)?;
        let mut pending_tiles = 0;
        let mut cleared_tiles = 0;
        for position in spec.parent().layout().targets() {
            let tile = capture
                .map
                .region
                .index(*position)
                .and_then(|index| capture.map.cells.get(index));
            match tile {
                Some(Cell::Visible(tile))
                    if tile.liquid_depth == 0
                        && !tile.magma
                        && tile.building_occupancy == 0
                        && tile.shape == Shape::Wall
                        && tile.dig_designation == 1 =>
                {
                    pending_tiles += 1;
                }
                Some(Cell::Visible(tile))
                    if tile.liquid_depth == 0
                        && !tile.magma
                        && tile.building_occupancy == 0
                        && tile.shape == Shape::Floor
                        && tile.dig_designation == 0 =>
                {
                    cleared_tiles += 1;
                }
                _ => return Err(stale()),
            }
        }
        Ok(Self {
            tick: capture.tick().get(),
            digest: capture_digest(&capture)?,
            pending_tiles,
            cleared_tiles,
        })
    }
    pub fn tick(&self) -> u64 {
        self.tick
    }
    pub fn digest(&self) -> Digest32 {
        self.digest
    }
    pub fn pending_tiles(&self) -> u32 {
        self.pending_tiles
    }
    pub fn cleared_tiles(&self) -> u32 {
        self.cleared_tiles
    }
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
        self.verified && !self.unresolved
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
    spec: &MonitorSpec,
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
        spec.authorize(&canonical_check, matches!(access, Access::Plan))?;
        native_check.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if matches!(access, Access::Plan | Access::Cancel) {
            canonical_check.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
            native_check.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?;
        }
        if matches!(access, Access::Plan) {
            canonical_check.authorize(
                Capability::Plan,
                RiskTier::Guarded,
                &[],
                Some(spec.parent().binding().scope()),
            )?;
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
    spec: MonitorSpec,
    authority: Authority,
    high_tick: u64,
    high_sequence: u64,
    review: Option<Review>,
    readiness: Option<Readiness>,
}
impl<S: EffectJournalStorage> Clock<S> {
    pub fn new(
        mut journal: RunJournal<S>,
        spec: MonitorSpec,
        recovery: Option<Recovery>,
        c: &OperationContext,
        n: &OperationContext,
    ) -> Result<Self> {
        if journal.binding().endpoint() != spec.source().endpoint()
            || journal.binding().manifest().df_version != spec.source().df_version()
            || journal.binding().manifest().dfhack_version != spec.source().dfhack_version()
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
            authorize(&spec, &authority, spec.start_tick(), c, n, Access::Query)?;
        let mut work = Work::new(current, native, 2 * VIEW_BYTES)?;
        if let Some(pin) = recovery {
            journal.verify_recovery(pin.journal_id, pin.head, &work.take(VIEW_BYTES, true)?)?;
        } else if !journal.initialized_here() || journal.cached_records().next().is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "existing blueprint clock requires its exact original journal identity and head",
            ));
        }
        let mut clock = Self {
            journal,
            high_tick: spec.start_tick().max(work.c.anchor.tick.get()),
            spec,
            authority,
            high_sequence: 0,
            review: None,
            readiness: None,
        };
        let inspection = work.take(VIEW_BYTES, true)?;
        clock.inspect(&work.narrow(&work.c)?, &inspection)?;
        Ok(clock)
    }
    pub fn spec(&self) -> &MonitorSpec {
        &self.spec
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
        hash(
            b"dfmcp-blueprint-clock-target/1",
            &[&self.spec.canonical_bytes(), self.journal.id().as_bytes()],
        )
    }
    fn key(&self, plan: Digest32) -> String {
        format!(
            "blueprint-clock-{}",
            hash(
                b"dfmcp-blueprint-clock-key/1",
                &[self.target_digest().as_bytes(), plan.as_bytes()]
            )
            .to_hex()
        )
    }
    fn validate_plan(&self, plan: &RunPlan) -> Result<()> {
        let tick = plan.before().tick().ok_or_else(corrupt)?;
        require(
            plan.before().eligible()
                && tick >= self.spec.start_tick()
                && plan.before().generation() == self.binding().manifest().generation
                && plan.key() == self.key(plan.digest())
                && u64::from(plan.spec().game_ticks()) <= self.spec.policy().max_gap_ticks()
                && tick
                    .checked_add(u64::from(plan.spec().game_ticks()))
                    .is_some_and(|end| end <= self.spec.policy().deadline_tick()),
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
            monitor_id: self.spec.id(),
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
    fn retain_context_floor(&mut self, c: &OperationContext, n: &OperationContext) {
        // A later authoritative session floor must survive a rejected operation.
        // Foreign contexts cannot modify this owner's remembered history.
        if c.session_id == self.authority.owner
            && n.session_id == self.authority.owner
            && c.request_id == n.request_id
            && c.anchor.fortress_id == self.spec.parent().binding().fortress_id()
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
        if records.len() > MAX_RUNS.min(self.spec.policy().max_samples() as usize)
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
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<LiveMapObservation>,
    {
        let (current, native) = self.control(c, n, Access::Plan)?;
        if request.monitor_id != self.spec.id() {
            return Err(stale());
        }
        let requested = request.spec()?;
        let mut work = Work::new(current, native, OPERATION_BYTES)?;
        let output = work.take(VIEW_BYTES, true)?;
        let inspection = work.take(VIEW_BYTES, true)?;
        let before = self.inspect(&work.c, &inspection)?;
        if before.pending {
            return Err(uncertain());
        }
        if before.runs.len() == MAX_RUNS.min(self.spec.policy().max_samples() as usize) {
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
        let provisional = RunPlan::new("blueprint-clock-candidate", requested, observation)?;
        let plan = RunPlan::new(
            &self.key(provisional.digest()),
            requested,
            provisional.before().clone(),
        )?;
        self.horizon(&plan, &work.c, &work.n)?;
        let reservation = work.take(ACTION_BYTES, true)?;
        let native = Self::native_plan_context(&reservation, &plan)?;
        let mut canonical = work.narrow(&work.c)?;
        canonical.anchor.tick = native.anchor.tick;
        let tick = ClockFloor::new(self.high_tick);
        let sequence = ClockFloor::new(self.high_sequence);
        let prepared = {
            let mut boundary = Boundary {
                edge: Edge::Prepare,
                spec: &self.spec,
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
        let seal = hash(
            b"dfmcp-blueprint-clock-review/1",
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
        let final_native = work.narrow(&output)?;
        match self.inspect(&work.narrow(&work.c)?, &final_native) {
            Ok(view) => Ok(view),
            Err(cause) => {
                self.abandon_preparation();
                Err(cause)
            }
        }
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
        F: FnMut(Edge, &RunPlan, &OperationContext) -> Result<LiveMapObservation>,
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
        let mut work = Work::new(current, native, OPERATION_BYTES)?;
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
        let mut canonical = work.narrow(&work.c)?;
        canonical.anchor.tick = native.anchor.tick;
        self.readiness = None;
        let tick = ClockFloor::new(self.high_tick);
        let sequence = ClockFloor::new(self.high_sequence);
        let result = {
            let mut boundary = Boundary {
                edge: Edge::Commit,
                spec: &self.spec,
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
        self.readiness = None;
        result?;
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
    spec: &'a MonitorSpec,
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
        guard: &mut impl FnMut(Edge, &RunPlan, &OperationContext) -> Result<LiveMapObservation>,
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
        capture.validate()?;
        self.high_tick
            .set(self.high_tick.get().max(capture.tick().get()));
        let readiness = Readiness::capture(self.spec, capture)?;
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
