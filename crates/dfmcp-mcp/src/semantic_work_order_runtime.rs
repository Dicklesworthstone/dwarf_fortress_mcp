//! One original native furniture-order goal, from observation through durable
//! semantic review, single insertion, foreground proof and read-only recovery.
//! The resource shell is operator-owned; no MCP value can issue source authority.
use dfmcp_adapter::LiveObservationCapsule;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_projection::{LiveWorldProjection, project_live_capsule};
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_adapter::semantic_work_orders::{
    AssociationStore, DurableWorkOrderGoalMonitor, GoalHistoryStore, NativeOrderCreation,
    OperationsGoalSource, SemanticWorkOrderReview, SemanticWorkOrderSession,
    WorkOrderEvidenceOwner, WorkOrderGoalProgress,
};
use dfmcp_adapter::work_order_control::session::WorkOrderSession;
use dfmcp_adapter::work_order_control::{
    CreationRecord, CreationState, JournalMode, WorkOrderSource,
};
use dfmcp_adapter::work_order_progress::ProgressSource;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use dfmcp_intent::plan_archive::{ArchiveLimits, PlanArchiveSeal};
use dfmcp_intent::{ObligationStatus, PreparedPlan};
use dfmcp_world::{PredicateEvidence, PredicateTruth};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

pub mod clock;
pub mod native;
pub mod policy;
pub mod request;
use clock::{Clock, ClockTarget, ClockView, RunRequest};
use dfmcp_adapter::order_run::journal::OrderRunJournal;
use dfmcp_adapter::order_run::rpc::OrderRunSource;
pub use native::{Config, NativeRuntime};
pub use request::Request;

pub const MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_WALL_MILLIS: u64 = 60_000;
pub const MAX_OUTPUT_TOKENS: u32 = 32_768;
const MIB: u64 = 1024 * 1024;
const VIEW_BYTES: u64 = 32 * MIB;
pub const MAX_GAME_TICKS: u64 = 1200;
const ARCHIVE_BYTES: u64 = 4 * MIB;

pub(crate) fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}
pub(crate) fn denied(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CapabilityDenied, message)
}
pub(crate) fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}
pub(crate) fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid(
            "digest must be 64 lowercase hexadecimal characters",
        ));
    }
    Digest32::from_hex(raw)
        .filter(|d| *d != Digest32::ZERO)
        .ok_or_else(|| invalid("invalid or reserved digest"))
}
pub fn archive_limits() -> ArchiveLimits {
    ArchiveLimits {
        max_bytes: 64 * 1024,
        max_steps: 1,
        max_nodes: 512,
        max_depth: 16,
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Offline,
    Reconcile,
    Control,
}
impl Mode {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw {
            "offline" => Ok(Self::Offline),
            "reconcile" => Ok(Self::Reconcile),
            "control" => Ok(Self::Control),
            _ => Err(invalid("mode must be offline, reconcile or control")),
        }
    }
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offline => "offline",
            Self::Reconcile => "reconcile",
            Self::Control => "control",
        }
    }
    pub const fn journal(self) -> JournalMode {
        match self {
            Self::Offline => JournalMode::Offline,
            Self::Reconcile => JournalMode::Reconcile,
            Self::Control => JournalMode::Control,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Recovery {
    pub seal: PlanArchiveSeal,
    pub anchor: StateAnchor,
    pub history_present: bool,
}

/// Resource factories are trusted embedding inputs, never client JSON. Tests
/// inject bounded streams/storage while executing the same real coordinators.
pub trait Shell {
    type Journal: EffectJournalStorage;
    type Association: EffectJournalStorage;
    type Native: WorkOrderSource;
    type Archive: EffectJournalStorage;
    type History: EffectJournalStorage;
    type Evidence: WorkOrderEvidenceOwner;
    type Progress: ProgressSource;
    type Operations: OperationsGoalSource;
    type ClockJournal: EffectJournalStorage;
    type ClockSource: OrderRunSource;
    fn clock_enabled(&self) -> bool;
    fn open_clock(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<Option<OrderRunJournal<Self::ClockJournal>>>;
    fn clock_source(&self, c: &OperationContext) -> Result<Self::ClockSource>;
    fn acceptance(&self) -> &policy::SourceAcceptance;
    fn recovery(&self) -> Option<Recovery>;
    fn check_environment(&self) -> Result<()>;
    fn production_enabled(&self) -> bool;
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkOrderSession<Self::Journal, Self::Native>,
        AssociationStore<Self::Association>,
        Digest32,
    )>;
    fn acquire(&self, c: &OperationContext) -> Result<LiveObservationCapsule>;
    fn evidence(
        &self,
        capsule: LiveObservationCapsule,
        c: &OperationContext,
    ) -> Result<Self::Evidence>;
    fn create_archive(
        &self,
        plan: &PreparedPlan,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Self::Archive>>;
    fn recover_archive(
        &self,
        recovery: Recovery,
        c: &OperationContext,
    ) -> Result<PlanArchiveStore<Self::Archive>>;
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticWorkOrderReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Self::History>>;
    fn progress(&self, c: &OperationContext) -> Result<Self::Progress>;
    fn operations(&self, c: &OperationContext) -> Result<Self::Operations>;
}
type Semantic<S> = SemanticWorkOrderSession<
    <S as Shell>::Journal,
    <S as Shell>::Association,
    <S as Shell>::Native,
>;
struct Basis {
    capsule: LiveObservationCapsule,
    projection: LiveWorldProjection,
    seal: Digest32,
}

pub struct Runtime<S: Shell> {
    shell: S,
    mode: Mode,
    session: SessionId,
    request: u128,
    budget: WorkBudget,
    anchor: StateAnchor,
    semantic: Semantic<S>,
    journal: Digest32,
    basis: Option<Basis>,
    evidence: Option<S::Evidence>,
    original: Option<PreparedPlan>,
    archive: Option<PlanArchiveStore<S::Archive>>,
    review: Option<SemanticWorkOrderReview>,
    monitor: Option<DurableWorkOrderGoalMonitor<S::History>>,
    history_initialized: bool,
    clock: Option<Clock<S::ClockJournal>>,
    production_at_open: bool,
    opening_clock: Option<ClockView>,
}

/// Compact complete view of this one retained goal. Historical proof and current
/// truth are separate; absence of a current sample is explicitly unavailable.
#[derive(Clone, Debug)]
pub struct View {
    pub mode: Mode,
    pub anchor: StateAnchor,
    pub observation: Option<(Digest32, Digest32, usize)>,
    pub original: Option<PreparedPlan>,
    pub review: Option<SemanticWorkOrderReview>,
    pub archive: Option<PlanArchiveSeal>,
    pub native: Option<CreationRecord>,
    pub creation: Option<NativeOrderCreation>,
    pub goal: Option<WorkOrderGoalProgress>,
    pub first_satisfied_anchor: Option<StateAnchor>,
    pub historical_obligation: Option<ObligationStatus>,
    pub history_events: usize,
    pub history_bytes: usize,
    pub history_present: bool,
    pub history_fenced: bool,
    pub clock: Option<ClockView>,
    pub clock_control_enabled: bool,
}
fn anchor_json(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,
        "sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
}
fn truth(t: PredicateTruth) -> &'static str {
    match t {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}
impl View {
    pub fn to_json(&self) -> Value {
        let insertion = self.creation.as_ref().map(|creation| match creation {
            NativeOrderCreation::Unverified => json!({"verified":false,"native_order_id":null,"receipt":null}),
            NativeOrderCreation::Inserted { native_order_id, receipt } =>
                json!({"verified":true,"native_order_id":native_order_id,"receipt":receipt.to_hex()}),
        });
        let original = self.original.as_ref().map(|plan| json!({
            "plan_digest":plan.digest.to_hex(),"intent_id":plan.intent_id.to_string(),
            "anchor":anchor_json(plan.anchor),"summary":plan.summary,"expires_at_tick":plan.expires_at_tick.get(),
            "terminal":format!("{:?}",plan.terminal_condition),
            "steps":plan.steps.iter().map(|s|json!({"key":s.idempotency_key,"action":format!("{:?}",s.action),
                "postconditions":s.postconditions.iter().map(|p|format!("{p:?}")).collect::<Vec<_>>(),
                "obligation":s.obligation.as_ref().map(|o|json!({"deadline_tick":o.deadline_tick.get(),
                    "poll_interval_ticks":o.poll_interval_ticks,"stable_samples":o.stable_for_observations}))})).collect::<Vec<_>>()
        }));
        let goal = self.goal.as_ref().map(|p| json!({"anchor":anchor_json(p.anchor),
            "source_digest":p.source_digest.to_hex(),"current_truth":truth(p.current_goal),
            "postconditions":truth(p.postconditions),"plan_terminal":truth(p.plan_terminal),
            "obligation_terminal":p.obligation_terminal.map(truth),"failure_predicate":p.failure_predicate.map(truth),
            "temporal_status":p.obligation.as_ref().map(|s|format!("{s:?}")),
            "original_goal_proven":p.current_goal_proven(),"physical_work":format!("{:?}",p.physical_work),
            "physical_quiescence_proven":p.physical_work.is_quiescent(),"receipt_linked":p.receipt_linked,
            "continuous":p.continuous,
            "semantic_completion_proven":p.current_goal_proven() && p.receipt_linked && p.physical_work.is_quiescent()
        }));
        let recovery = self.archive.zip(self.original.as_ref()).map(|(seal, p)| {
            let mut value = json!({"plan_digest":seal.plan_digest.to_hex(),
                "object_digest":seal.object_digest.to_hex(),"anchor":anchor_json(p.anchor),
                "history_present":self.history_present});
            if let Some(clock) = &self.clock {
                value["clock_journal_id"] = json!(clock.journal_id.to_hex());
            }
            value
        });
        json!({"ok":true,"mode":self.mode.as_str(),"anchor":anchor_json(self.anchor),
            "runtime_admitted":false,"source_acceptance":policy::ACCEPTANCE_KIND,
            "source_acceptance_is_compatibility_admission":false,
            "clock_advanced":if self.clock.as_ref().is_some_and(|v|v.has_runs()) { Value::Null } else {json!(false)},
            "clock":self.clock.as_ref().map(ClockView::to_json),
            "development_clock_enabled":self.clock_control_enabled,
            "observation":self.observation.map(|(seal,source,count)|json!({"seal":seal.to_hex(),
                "source_digest":source.to_hex(),"citizens":count,"paused":true})),
            "original_plan":original,"review":self.review.as_ref().map(|r|json!({
                "paired_seal":r.seal().to_hex(),"native_plan_digest":r.native_plan().digest().to_hex(),
                "semantic_plan_digest":r.original_plan().digest.to_hex()})),
            "recovery":recovery,
            "native_state":self.native.as_ref().map(|n|n.state().as_str()),"insertion":insertion,
            "current_goal":goal,"current_goal_available":self.goal.is_some(),
            "first_satisfied_anchor":self.first_satisfied_anchor.map(anchor_json),
            "historical_obligation":self.historical_obligation.as_ref().map(|s|format!("{s:?}")),
            "history":{"events":self.history_events,"bytes":self.history_bytes,"fenced":self.history_fenced},
            "one_goal_limit":1,"automatic_redispatch":false,
            "coverage_omissions":["item output quantities","material and workshop feasibility","automatic chaining of clock runs","continuous goal truth"]})
    }
}

struct Work {
    context: OperationContext,
    until: Instant,
    bytes: u64,
}
impl Work {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        let until = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(|| exhausted("operation deadline overflow"))?;
        Ok(Self {
            context: c.clone(),
            until,
            bytes: c.budget.max_bytes,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let left = self
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(|| exhausted("semantic runtime deadline expired"))?;
        let millis = u64::try_from(left.as_millis()).map_err(|_| exhausted("deadline overflow"))?;
        if millis == 0 {
            return Err(exhausted("semantic runtime deadline expired"));
        }
        let mut c = self.context.clone();
        c.budget.max_wall_millis = millis;
        c.budget.max_bytes = self.bytes;
        Ok(c)
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut c = self.current()?;
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .ok_or_else(|| exhausted("complete runtime operation exceeds byte-work budget"))?;
        c.budget.max_bytes = bytes;
        Ok(c)
    }
    fn rest(&mut self) -> Result<OperationContext> {
        self.reserve(self.bytes)
    }
    fn narrow(&self, reserved: &OperationContext) -> Result<OperationContext> {
        let mut c = reserved.clone();
        c.budget.max_wall_millis = self.current()?.budget.max_wall_millis;
        c.anchor = self.context.anchor;
        Ok(c)
    }
}
fn grants(
    mode: Mode,
    fortress: dfmcp_core::FortressId,
    enabled: bool,
    clock: bool,
) -> Vec<CapabilityGrant> {
    let mut capabilities = vec![Capability::Query, Capability::Observe];
    if mode == Mode::Control && enabled {
        capabilities.extend([Capability::Plan, Capability::ConfigureProduction]);
    }
    if mode == Mode::Control && clock {
        if !capabilities.contains(&Capability::Plan) {
            capabilities.push(Capability::Plan);
        }
        capabilities.push(Capability::ControlClock);
    }
    capabilities
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::Guarded,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
}
impl<S: Shell> Runtime<S> {
    pub fn from_shell(
        shell: S,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
    ) -> Result<Self> {
        shell.check_environment()?;
        shell.acceptance().validate()?;
        budget.validate()?;
        if session == SessionId::NIL
            || budget.max_bytes > MAX_BYTES
            || budget.max_wall_millis > MAX_WALL_MILLIS
            || budget.max_entities < 4096
            || budget.max_actions != 1
            || budget.max_output_tokens > MAX_OUTPUT_TOKENS
            || budget.max_game_ticks > MAX_GAME_TICKS
        {
            return Err(exhausted(
                "runtime requires at most 1200 game ticks, 4096 entities and one action",
            ));
        }
        if mode == Mode::Control && !shell.production_enabled() && !shell.clock_enabled() {
            return Err(denied(
                "operator production and clock authority are disabled",
            ));
        }
        let recovery = shell.recovery();
        let anchor = recovery.map(|r| r.anchor).unwrap_or(StateAnchor {
            fortress_id: shell.acceptance().fortress(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        });
        if anchor.fortress_id != shell.acceptance().fortress() {
            return Err(denied(
                "recovery anchor belongs to another operator fortress",
            ));
        }
        let context = OperationContext {
            session_id: session,
            request_id: RequestId::new(1),
            anchor,
            budget,
            grants: grants(
                mode,
                anchor.fortress_id,
                shell.production_enabled(),
                shell.clock_enabled(),
            ),
            cancellation_requested: false,
        };
        let mut work = Work::new(&context)?;
        let clock_validation = work.reserve(32 * MIB)?;
        let (native, associations, journal) = shell.open_control(mode, &work.reserve(80 * MIB)?)?;
        let production_at_open = mode == Mode::Control
            && native.summary(&work.reserve(MIB)?)?.mode == JournalMode::Control;
        let semantic =
            SemanticWorkOrderSession::new(native, associations, &work.reserve(VIEW_BYTES)?)?;
        let clock = shell
            .open_clock(mode, &work.reserve(32 * MIB)?)?
            .map(Clock::new);
        let mut result = Self {
            shell,
            mode,
            session,
            request: 1,
            budget,
            anchor,
            semantic,
            journal,
            basis: None,
            evidence: None,
            original: None,
            archive: None,
            review: None,
            monitor: None,
            history_initialized: recovery.is_some_and(|r| r.history_present),
            clock,
            production_at_open,
            opening_clock: None,
        };
        let records = result.semantic.inventory(&work.reserve(VIEW_BYTES)?)?;
        for record in &records {
            result
                .shell
                .acceptance()
                .native_manifest(record.manifest())?;
        }
        if records.len() > 1 || (!records.is_empty() && recovery.is_none()) {
            return Err(denied(
                "one-goal runtime requires the external exact archive handoff for existing native work",
            ));
        }
        if let Some(recovery) = recovery {
            let mut archive = result
                .shell
                .recover_archive(recovery, &work.reserve(ARCHIVE_BYTES)?)?;
            let plan = archive.recover(&work.reserve(ARCHIVE_BYTES)?)?;
            if plan.anchor != recovery.anchor {
                return Err(denied(
                    "external original anchor differs from archived plan",
                ));
            }
            result.original = Some(plan.clone());
            result.archive = Some(archive);
            if !records.is_empty() {
                let review = result.semantic.reattach(plan, &work.reserve(VIEW_BYTES)?)?;
                result.review = Some(review);
                if recovery.history_present {
                    result.start_monitor(false, &mut work)?;
                }
            } else if recovery.history_present {
                return Err(denied(
                    "goal history cannot exist without its original native preparation",
                ));
            }
        }
        result.raise_monitor_floor();
        // Verify the optional clock history against the externally retained
        // original goal before publishing this recovered owner.
        let mut validation = work.narrow(&clock_validation)?;
        validation.anchor = result.anchor;
        result.opening_clock = result.inspect_clock(&validation)?;
        Ok(result)
    }
    /// Recovery identities validated during open, retained without further I/O.
    /// This reference cannot certify current custody, source truth or authority.
    pub fn opening_recovery_reference(&self) -> Value {
        let recovery = self
            .archive
            .as_ref()
            .zip(self.original.as_ref())
            .map(|(archive, plan)| {
                let seal = archive.seal();
                let mut value = json!({"plan_digest":seal.plan_digest.to_hex(),
                "object_digest":seal.object_digest.to_hex(),"anchor":anchor_json(plan.anchor),
                "history_present":self.history_initialized});
                if let Some(clock) = &self.opening_clock {
                    value["clock_journal_id"] = json!(clock.journal_id.to_hex());
                }
                value
            });
        json!({"ok":false,"retained_state_currently_verified":false,
            "mode":self.mode.as_str(),"anchor":anchor_json(self.anchor),
            "clock":self.opening_clock.as_ref().map(ClockView::to_json),
            "review":self.review.as_ref().map(|review|json!({
                "paired_seal":review.seal().to_hex(),
                "native_plan_digest":review.native_plan().digest().to_hex(),
                "semantic_plan_digest":review.original_plan().digest.to_hex()})),
            "original_plan":self.original.as_ref().map(|plan|json!({
                "plan_digest":plan.digest.to_hex(),"anchor":anchor_json(plan.anchor)})),
            "recovery":recovery,"current_goal":null,"current_goal_available":false,
            "runtime_admitted":false,"automatic_redispatch":false})
    }
    pub fn anchor(&self) -> StateAnchor {
        self.anchor
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn session_id(&self) -> SessionId {
        self.session
    }
    pub fn budget(&self) -> WorkBudget {
        self.budget
    }
    pub fn context(&mut self, cancelled: bool) -> Result<OperationContext> {
        self.shell.check_environment()?;
        self.raise_monitor_floor();
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| exhausted("request identities exhausted"))?;
        Ok(OperationContext {
            session_id: self.session,
            request_id: RequestId::new(self.request),
            anchor: self.anchor,
            budget: self.budget,
            grants: grants(
                self.mode,
                self.anchor.fortress_id,
                self.production_control_enabled(),
                self.clock_control_enabled(),
            ),
            cancellation_requested: cancelled,
        })
    }
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        self.shell.check_environment()?;
        if c.session_id != self.session
            || c.anchor != self.anchor
            || c.budget.max_bytes > self.budget.max_bytes
            || c.budget.max_wall_millis > self.budget.max_wall_millis
            || c.budget.max_actions > self.budget.max_actions
            || c.budget.max_entities > self.budget.max_entities
            || c.budget.max_output_tokens > self.budget.max_output_tokens
            || c.budget.max_game_ticks > self.budget.max_game_ticks
        {
            return Err(denied(
                "runtime context differs from retained session or widens its allowance",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if !self.production_control_enabled() {
                return Err(denied(
                    "runtime is not currently operator-authorized for production",
                ));
            }
            c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            c.authorize(
                Capability::ConfigureProduction,
                RiskTier::Guarded,
                &[],
                None,
            )?;
        }
        Ok(())
    }
    fn raise_monitor_floor(&mut self) {
        // Only exact published historical anchors may replace this anchor.
        // Failed acquisitions retain their separate authority/cursor floors in
        // the coordinator and monitor; a new tick with an old hash is no state.
        if let Some(anchor) = self
            .monitor
            .as_ref()
            .and_then(|m| m.last_observation_anchor())
        {
            if anchor.cursor.sequence > self.anchor.cursor.sequence
                && anchor.tick >= self.anchor.tick
            {
                self.anchor = anchor;
            }
        }
    }
    fn start_monitor(&mut self, initialize: bool, work: &mut Work) -> Result<()> {
        if self.monitor.is_some() {
            return Ok(());
        }
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| invalid("original review is absent"))?
            .clone();
        let history = self.shell.history(
            self.journal,
            &review,
            initialize,
            self.mode == Mode::Offline,
            &work.reserve(64 * MIB)?,
        )?;
        self.history_initialized = true;
        let shell = &self.shell;
        let acceptance = shell.acceptance().clone();
        let monitor = self.semantic.begin_durable_goal_monitor(
            history,
            review.seal(),
            &work.rest()?,
            |p, c| {
                shell.check_environment()?;
                acceptance.goal(p, c)
            },
        )?;
        self.monitor = Some(monitor);
        Ok(())
    }
    fn verify_archive(&mut self, work: &mut Work) -> Result<()> {
        match (&mut self.archive, &self.original) {
            (Some(archive), Some(original)) => {
                if archive.recover(&work.reserve(ARCHIVE_BYTES)?)? != *original {
                    return Err(denied("original archive differs from retained plan"));
                }
            }
            (None, None) => {}
            _ => return Err(denied("original goal archive custody is incomplete")),
        }
        Ok(())
    }
    fn view(&mut self, c: &OperationContext) -> Result<View> {
        self.shell.check_environment()?;
        let mut work = Work::new(c)?;
        if let Some(archive) = &mut self.archive {
            let original = archive.recover(&work.reserve(ARCHIVE_BYTES)?)?;
            if self.original.as_ref() != Some(&original) {
                return Err(denied("original archive differs from retained plan"));
            }
        }
        let clock_budget = self
            .clock
            .as_ref()
            .map(|_| work.reserve(8 * MIB))
            .transpose()?;
        let inventory = self.semantic.inventory(&work.reserve(4 * MIB)?)?;
        if inventory.len() > 1 {
            return Err(denied(
                "one-goal custody contains more than one native record",
            ));
        }
        let result = if inventory.is_empty() {
            None
        } else if let Some(review) = &self.review {
            Some(self.semantic.inspect(review.seal(), &work.rest()?)?)
        } else {
            return Err(denied(
                "retained native record has no exact original review",
            ));
        };
        let clock = if self.clock.is_some() {
            let target = self.clock_target(result.as_ref().map(|r| r.creation_result()))?;
            self.clock
                .as_mut()
                .ok_or_else(|| invalid("clock owner absent"))?
                .inspect(
                    target.as_ref(),
                    &work.narrow(
                        &clock_budget.ok_or_else(|| exhausted("clock view reservation absent"))?,
                    )?,
                )?
                .into()
        } else {
            None
        };
        self.shell.check_environment()?;
        Ok(View {
            mode: self.mode,
            anchor: self.anchor,
            observation: self
                .basis
                .as_ref()
                .map(|b| (b.seal, b.capsule.content_digest, b.capsule.citizens.len())),
            original: self.original.clone(),
            review: self.review.clone(),
            archive: self.archive.as_ref().map(|a| a.seal()),
            native: result.as_ref().map(|r| r.native_record().clone()),
            creation: result.as_ref().map(|r| r.creation_result().clone()),
            goal: self.monitor.as_ref().and_then(|m| m.latest()).cloned(),
            first_satisfied_anchor: self
                .monitor
                .as_ref()
                .and_then(|m| m.first_satisfied_anchor()),
            historical_obligation: self
                .monitor
                .as_ref()
                .and_then(|m| m.historical_obligation_status())
                .cloned(),
            history_events: self.monitor.as_ref().map_or(0, |m| m.history_event_count()),
            history_bytes: self.monitor.as_ref().map_or(0, |m| m.history_byte_len()),
            history_present: self.history_initialized,
            history_fenced: self.monitor.as_ref().is_some_and(|m| m.is_fenced()),
            clock,
            clock_control_enabled: self.clock_control_enabled(),
        })
    }
    pub fn inspect(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        self.view(c)
    }
    pub fn observe(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return Err(denied(
                "offline runtime cannot acquire current observations",
            ));
        }
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.basis = None;
        self.evidence = None;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let capsule = self.shell.acquire(&work.reserve(16 * MIB)?)?;
        // A retained original plan may only reacquire its exact sealing anchor.
        // New observations never silently recompile or renew its deadlines.
        let cursor = self
            .original
            .as_ref()
            .map(|p| p.anchor.cursor)
            .unwrap_or(ObservationCursor {
                epoch: self.anchor.cursor.epoch,
                sequence: self
                    .anchor
                    .cursor
                    .sequence
                    .checked_add(1)
                    .ok_or_else(|| exhausted("canonical observation cursor exhausted"))?,
            });
        let projection = project_live_capsule(&capsule, self.anchor.fortress_id, cursor)?;
        if let Some(original) = &self.original {
            if projection.snapshot.anchor() != original.anchor {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "original semantic source changed; no rebase or new insertion is permitted",
                ));
            }
        }
        work.context.anchor = projection.snapshot.anchor();
        self.shell
            .acceptance()
            .citizens(&capsule, &projection, &work.reserve(MIB)?)?;
        let evidence = self
            .shell
            .evidence(capsule.clone(), &work.reserve(16 * MIB)?)?;
        let mut identity = b"dfmcp-native-semantic-observation/1\0".to_vec();
        identity.extend_from_slice(capsule.content_digest.as_bytes());
        identity.extend_from_slice(projection.snapshot.state_hash.as_bytes());
        identity.extend_from_slice(&cursor.epoch.to_be_bytes());
        identity.extend_from_slice(&cursor.sequence.to_be_bytes());
        let seal = Digest32::of_bytes(&identity);
        self.anchor = projection.snapshot.anchor();
        self.basis = Some(Basis {
            capsule,
            projection,
            seal,
        });
        self.evidence = Some(evidence);
        if let Some(review) = &self.review {
            let record = self
                .semantic
                .inspect(review.seal(), &work.reserve(VIEW_BYTES)?)?;
            if record.native_record().state() == CreationState::Prepared
                && self.mode == Mode::Control
            {
                let evidence = self
                    .evidence
                    .as_mut()
                    .ok_or_else(|| invalid("canonical evidence absent"))?;
                self.semantic.refresh_prepared(
                    review.seal(),
                    evidence,
                    &work.reserve(64 * MIB)?,
                )?;
            }
        }
        self.view(&work.narrow(&output)?)
    }
    pub fn plan(&mut self, request: Request, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let basis = self
            .basis
            .as_ref()
            .filter(|b| request.observation().is_ok_and(|s| s == b.seal))
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "exact current observation seal is required",
                )
            })?;
        let policy = self.shell.acceptance().citizens(
            &basis.capsule,
            &basis.projection,
            &work.reserve(MIB)?,
        )?;
        let plan = request.compile(
            &PredicateEvidence::scoped(&basis.projection.snapshot, policy)?,
            &work.reserve(MIB)?,
        )?;
        if let Some(original) = &self.original {
            if *original != plan {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "this runtime retains one exact original goal; it cannot replace or extend it",
                ));
            }
            if self.review.is_some()
                && !self.semantic.inventory(&work.reserve(4 * MIB)?)?.is_empty()
            {
                return self.view(&work.narrow(&output)?);
            }
        } else {
            let archive = self
                .shell
                .create_archive(&plan, &work.reserve(ARCHIVE_BYTES)?)?;
            self.original = Some(plan.clone());
            self.archive = Some(archive);
        }
        let evidence = self
            .evidence
            .as_mut()
            .ok_or_else(|| invalid("observe before native semantic preparation"))?;
        let review = self
            .semantic
            .observe(plan, evidence, &work.reserve(64 * MIB)?)?;
        self.review = Some(review.clone());
        self.semantic
            .prepare(review.seal(), evidence, &work.reserve(64 * MIB)?)?;
        // Beginning goal history intentionally retires the native insertion
        // selection. Start it only from an explicit wait after dispatch, so the
        // prepared review remains eligible for its one confirmed insertion.
        self.view(&work.narrow(&output)?)
    }
    pub fn commit(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        self.check(c, true)?;
        if !confirmed || self.review.as_ref().is_none_or(|r| r.seal() != seal) {
            return Err(denied("confirm the exact paired semantic/native seal"));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let evidence = self
            .evidence
            .as_mut()
            .ok_or_else(|| invalid("reacquire the original observation before native commit"))?;
        self.semantic
            .commit(seal, true, evidence, &work.reserve(64 * MIB)?)?;
        self.view(&work.narrow(&output)?)
    }
    pub fn wait(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| invalid("no retained original goal"))?
            .clone();
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        if self.mode == Mode::Offline {
            return self.view(&work.narrow(&output)?);
        }
        let current = self
            .semantic
            .reconcile(review.seal(), &work.reserve(VIEW_BYTES)?)?;
        if matches!(
            current.native_record().state(),
            CreationState::Prepared
                | CreationState::Refused
                | CreationState::CancelledBeforeDispatch
        ) {
            return self.view(&work.narrow(&output)?);
        }
        if self.clock.is_some() {
            let target = self.clock_target(Some(current.creation_result()))?;
            let clock = self
                .clock
                .as_mut()
                .ok_or_else(|| invalid("clock owner absent"))?;
            let view = clock.inspect(target.as_ref(), &work.reserve(8 * MIB)?)?;
            if view.needs_reconciliation() {
                let target = target
                    .as_ref()
                    .ok_or_else(|| denied("clock history lacks verified original insertion"))?;
                let mut source = self.shell.clock_source(&work.reserve(4 * MIB)?)?;
                let view = clock.reconcile(target, &mut source, &work.reserve(96 * MIB)?)?;
                if !view.can_sample_goal() {
                    return self.view(&work.narrow(&output)?);
                }
            } else if !view.can_sample_goal() {
                return self.view(&work.narrow(&output)?);
            }
        }
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if self.monitor.is_none() {
            // Recovery without a history may initialize a new monitor, but never
            // import a proof streak or modify any original deadline.
            let monitor_budget = work.reserve(256 * MIB)?;
            let mut monitor_work = Work::new(&monitor_budget)?;
            self.start_monitor(!self.history_initialized, &mut monitor_work)?;
        }
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| invalid("original-goal monitor absent"))?;
        let cursor = ObservationCursor {
            epoch: self.anchor.cursor.epoch,
            sequence: self
                .anchor
                .cursor
                .sequence
                .max(monitor.high_cursor().sequence)
                .checked_add(1)
                .ok_or_else(|| exhausted("canonical goal cursor exhausted"))?,
        };
        let shell = &self.shell;
        let acceptance = shell.acceptance().clone();
        let result = self.semantic.poll_original_goal_durable(
            monitor,
            cursor,
            &work.rest()?,
            |c| shell.progress(c),
            |c| shell.operations(c),
            |p, c| {
                shell.check_environment()?;
                acceptance.goal(p, c)
            },
        );
        self.raise_monitor_floor();
        if let Ok(sample) = &result {
            self.anchor = sample.progress().anchor;
        }
        work.context.anchor = self.anchor;
        result?;
        self.view(&work.narrow(&output)?)
    }
    pub fn production_control_enabled(&self) -> bool {
        self.production_at_open && self.mode == Mode::Control && self.shell.production_enabled()
    }
    pub fn clock_control_enabled(&self) -> bool {
        self.mode == Mode::Control
            && self.shell.clock_enabled()
            && self.clock.as_ref().is_some_and(Clock::is_control)
    }
    fn clock_target(&self, creation: Option<&NativeOrderCreation>) -> Result<Option<ClockTarget>> {
        let Some(creation @ NativeOrderCreation::Inserted { .. }) = creation else {
            return Ok(None);
        };
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original goal is absent"))?;
        let archive = self
            .archive
            .as_ref()
            .ok_or_else(|| denied("original archive is absent"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review is absent"))?;
        ClockTarget::new(original, archive.seal(), self.journal, review, creation).map(Some)
    }
    fn inspect_clock(&mut self, c: &OperationContext) -> Result<Option<ClockView>> {
        if self.clock.is_none() {
            return Ok(None);
        }
        let mut work = Work::new(c)?;
        let created = if let Some(review) = &self.review {
            Some(
                self.semantic
                    .inspect(review.seal(), &work.reserve(16 * MIB)?)?,
            )
        } else {
            None
        };
        let target = self.clock_target(created.as_ref().map(|v| v.creation_result()))?;
        self.clock
            .as_mut()
            .ok_or_else(|| invalid("clock owner absent"))?
            .inspect(target.as_ref(), &work.reserve(8 * MIB)?)
            .map(Some)
    }
    fn check_clock(&self, c: &OperationContext) -> Result<()> {
        self.check(c, false)?;
        if !self.clock_control_enabled() {
            return Err(denied("this owner has no current operator clock authority"));
        }
        c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
    }
    fn sample_for_clock(&mut self, work: &mut Work) -> Result<ClockTarget> {
        // This reserves a distinct share of the caller's operation. It does not
        // renew the deadline or spend the same native/read budget twice.
        let sample = self.wait(&work.reserve(1024 * MIB)?)?;
        work.context.anchor = self.anchor;
        if sample.clock.as_ref().is_some_and(|v| !v.can_sample_goal()) {
            return Err(denied("reconcile the retained clock before another run"));
        }
        let goal = sample
            .goal
            .as_ref()
            .filter(|g| g.anchor == self.anchor && g.receipt_linked)
            .ok_or_else(|| {
                denied("clock control requires a fresh receipt-linked original-goal sample")
            })?;
        if goal.current_goal_proven() && goal.physical_work.is_quiescent() {
            return Err(denied(
                "the original goal is already proved and physically quiet",
            ));
        }
        self.clock_target(sample.creation.as_ref())?
            .ok_or_else(|| denied("clock control requires verified original insertion"))
    }
    fn verify_clock_custody(
        shell: &S,
        archive: &mut PlanArchiveStore<S::Archive>,
        semantic: &mut Semantic<S>,
        original: &PreparedPlan,
        review: &SemanticWorkOrderReview,
        target: &ClockTarget,
        journal: Digest32,
        c: &OperationContext,
    ) -> Result<()> {
        shell.check_environment()?;
        if !shell.clock_enabled() {
            return Err(denied("operator clock authority was revoked"));
        }
        let mut work = Work::new(c)?;
        if archive.recover(&work.reserve(ARCHIVE_BYTES)?)? != *original {
            return Err(denied(
                "original archive changed before native clock effect",
            ));
        }
        let retained = semantic.inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let current = ClockTarget::new(
            original,
            archive.seal(),
            journal,
            review,
            retained.creation_result(),
        )?;
        if current.digest() != target.digest() {
            return Err(denied(
                "original insertion identity changed before native clock effect",
            ));
        }
        shell.check_environment()?;
        work.current()?;
        Ok(())
    }
    pub fn plan_run(&mut self, request: RunRequest, c: &OperationContext) -> Result<View> {
        self.check_clock(c)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let target = self.sample_for_clock(&mut work)?;
        let mut source = self.shell.clock_source(&work.reserve(4 * MIB)?)?;
        let mut guard_budget = Some(work.reserve(64 * MIB)?);
        let run_budget = work.reserve(96 * MIB)?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original archive absent"))?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original plan absent"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let shell = &self.shell;
        let semantic = &mut self.semantic;
        let journal = self.journal;
        self.clock
            .as_mut()
            .ok_or_else(|| denied("operator clock source is not configured"))?
            .prepare_guarded(&target, &request, &mut source, &run_budget, |edge| {
                let mut reserved = guard_budget
                    .take()
                    .ok_or_else(|| denied("clock guard cannot repeat"))?;
                reserved.anchor = edge.anchor;
                reserved.budget.max_wall_millis = reserved
                    .budget
                    .max_wall_millis
                    .min(edge.budget.max_wall_millis);
                Self::verify_clock_custody(
                    shell, archive, semantic, original, review, &target, journal, &reserved,
                )
            })?;
        self.view(&work.narrow(&output)?)
    }
    pub fn commit_run(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        self.check_clock(c)?;
        if !confirmed {
            return Err(denied("confirm the exact original-bound clock review"));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let target = self.sample_for_clock(&mut work)?;
        let mut source = self.shell.clock_source(&work.reserve(4 * MIB)?)?;
        let mut guard_budget = Some(work.reserve(64 * MIB)?);
        let run_budget = work.reserve(96 * MIB)?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original archive absent"))?;
        let original = self
            .original
            .as_ref()
            .ok_or_else(|| denied("original plan absent"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let shell = &self.shell;
        let semantic = &mut self.semantic;
        let journal = self.journal;
        // The original paused planning observation cannot describe the world
        // after an attempted unpause. Retained goal samples keep their anchors.
        self.basis = None;
        self.evidence = None;
        self.clock
            .as_mut()
            .ok_or_else(|| denied("operator clock source is not configured"))?
            .commit_guarded(&target, seal, true, &mut source, &run_budget, |edge| {
                let mut reserved = guard_budget
                    .take()
                    .ok_or_else(|| denied("clock guard cannot repeat"))?;
                reserved.anchor = edge.anchor;
                reserved.budget.max_wall_millis = reserved
                    .budget
                    .max_wall_millis
                    .min(edge.budget.max_wall_millis);
                Self::verify_clock_custody(
                    shell, archive, semantic, original, review, &target, journal, &reserved,
                )
            })?;
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel_run(&mut self, c: &OperationContext) -> Result<View> {
        self.check_clock(c)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        let current = self
            .inspect_clock(&work.reserve(32 * MIB)?)?
            .ok_or_else(|| denied("operator clock source is not configured"))?;
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| denied("original review absent"))?;
        let created = self
            .semantic
            .inspect(review.seal(), &work.reserve(16 * MIB)?)?;
        let target = self
            .clock_target(Some(created.creation_result()))?
            .ok_or_else(|| denied("clock cancellation requires its original insertion identity"))?;
        let mut source = if current.cancellation_needs_source() {
            Some(self.shell.clock_source(&work.reserve(4 * MIB)?)?)
        } else {
            None
        };
        self.clock
            .as_mut()
            .ok_or_else(|| denied("clock owner absent"))?
            .cancel(&target, source.as_mut(), &work.reserve(96 * MIB)?)?;
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        let seal = self
            .review
            .as_ref()
            .ok_or_else(|| invalid("no retained original goal"))?
            .seal();
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_archive(&mut work)?;
        self.semantic.cancel(seal, &work.reserve(VIEW_BYTES)?)?;
        self.view(&work.narrow(&output)?)
    }
}

#[cfg(test)]
mod tests;
