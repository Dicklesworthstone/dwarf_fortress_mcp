//! Operator-owned original furniture goal from canonical planning to durable proof.
//! This composition adds no admission, source grant or automatic clock control.
use std::time::{Duration, Instant};

use dfmcp_adapter::bounded_run::journal::RunJournal;
use dfmcp_adapter::bounded_run::rpc::RunSource;
use dfmcp_adapter::build_placement::journal::{
    BuildEntry, BuildGuard, BuildMode, BuildSource, BuildState,
};
use dfmcp_adapter::build_placement::session::BuildSession;
use dfmcp_adapter::build_placement::{BuildBinding, BuildCapture, BuildPhase, BuildSelection};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_operations::item_entity_id;
use dfmcp_adapter::live_routing::LiveRoutingEvidence;
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_adapter::live_spatial::citizens::{
    LiveSpatialCitizenObservation, LiveSpatialCitizenState,
};
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_adapter::semantic_furniture::goal_history::{
    DurableFurnitureGoalMonitor, GoalHistoryStore,
};
use dfmcp_adapter::semantic_furniture::goal_monitor::{FurnitureGoalProgress, FurnitureGoalSource};
use dfmcp_adapter::semantic_furniture::goal_projection::FurnitureGoalProjection;
use dfmcp_adapter::semantic_furniture::{
    AssociationStore, FurnitureEvidenceOwner, NativeFurniturePlacement, SemanticFurnitureReview,
    SemanticFurnitureSession,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, FortressId,
    GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId,
    StateAnchor, WorkBudget,
};
use dfmcp_intent::plan_archive::{ArchiveLimits, PlanArchiveSeal};
use dfmcp_intent::{ObligationStatus, PreparedPlan};
use dfmcp_world::{EvidencePolicy, PredicateEvidence, PredicateTruth};
use serde_json::{Value, json};

mod advancement;
pub mod clock;
pub mod native;
use clock::{Clock, ClockTarget, ClockView, RunRequest};
pub mod policy;
pub mod request;
pub use native::{Config, NativeRuntime};
pub use request::Request;

pub const DEFAULT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub const MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MAX_WALL_MILLIS: u64 = 60_000;
pub const MAX_GAME_TICKS: u64 = 1200;
pub const MAX_OUTPUT_TOKENS: u32 = 32_768;
const MIB: u64 = 1024 * 1024;
const VIEW_BYTES: u64 = 128 * MIB;
const ARCHIVE_BYTES: u64 = 4 * MIB;
const HISTORY_OPEN_BYTES: u64 = 256 * MIB;
const LOCAL_BYTES: u64 = 64 * MIB;

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
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
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
        max_nodes: 1024,
        max_depth: 32,
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
    pub const fn journal(self) -> BuildMode {
        match self {
            Self::Offline => BuildMode::Offline,
            Self::Reconcile => BuildMode::Recover,
            Self::Control => BuildMode::Control,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Recovery {
    pub journal_id: Digest32,
    pub seal: PlanArchiveSeal,
    pub anchor: StateAnchor,
    pub history_present: bool,
}
/// All paths, endpoints, credentials, source policy and leases are supplied by
/// the host. None is reconstructed from an MCP request or a retained fingerprint.
pub trait Shell {
    type Journal: EffectJournalStorage;
    type Association: EffectJournalStorage;
    type Native: BuildSource;
    type Archive: EffectJournalStorage;
    type History: EffectJournalStorage;
    type Evidence: FurnitureEvidenceOwner;
    type Guard: BuildGuard;
    type GoalSource: FurnitureGoalSource;
    type ClockJournal: EffectJournalStorage;
    type ClockSource: RunSource;
    fn clock_enabled(&self) -> bool {
        false
    }
    fn open_clock(
        &self,
        _mode: Mode,
        _c: &OperationContext,
    ) -> Result<Option<RunJournal<Self::ClockJournal>>> {
        Ok(None)
    }
    /// The host independently issues run/1.13 source authority; it cannot rewrite fortress grants.
    fn clock_context(&self, _c: &OperationContext, _control: bool) -> Result<OperationContext> {
        Err(denied("operator clock authority is not configured"))
    }
    fn clock_source(&self, _c: &OperationContext) -> Result<Self::ClockSource> {
        Err(denied("operator clock source is not configured"))
    }
    fn fortress(&self) -> FortressId;
    fn recovery(&self) -> Option<Recovery>;
    fn check_environment(&self) -> Result<()>;
    fn placement_enabled(&self) -> bool;
    fn checkpoint_exception(&self) -> bool;
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        BuildSession<Self::Journal, Self::Native>,
        AssociationStore<Self::Association>,
        Digest32,
    )>;
    fn native(
        &self,
        binding: &BuildBinding,
        selection: BuildSelection,
        c: &OperationContext,
        timeout: Duration,
    ) -> Result<Self::Native>;
    fn guard(&self, journal: Digest32, c: &OperationContext) -> Result<Self::Guard>;
    fn acquire(&self, c: &OperationContext) -> Result<LiveSpatialCitizenObservation>;
    fn spatial_policy(
        &self,
        state: &LiveSpatialCitizenState,
        c: &OperationContext,
    ) -> Result<EvidencePolicy>;
    fn evidence(
        &self,
        state: LiveSpatialCitizenState,
        c: &OperationContext,
    ) -> Result<Self::Evidence>;
    fn evidence_reserve_bytes(&self, _state: &LiveSpatialCitizenState) -> Result<u64> {
        Ok(LOCAL_BYTES)
    }
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
        review: &SemanticFurnitureReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Self::History>>;
    fn goal_source(&self, binding: &BuildBinding, c: &OperationContext)
    -> Result<Self::GoalSource>;
    fn goal_policy(
        &self,
        projection: &FurnitureGoalProjection,
        c: &OperationContext,
    ) -> Result<EvidencePolicy>;
}
type Semantic<S> = SemanticFurnitureSession<
    <S as Shell>::Journal,
    <S as Shell>::Association,
    <S as Shell>::Native,
>;
struct Basis {
    state: LiveSpatialCitizenState,
    seal: Digest32,
}
pub struct Runtime<S: Shell> {
    shell: S,
    mode: Mode,
    session: SessionId,
    request: u128,
    budget: WorkBudget,
    authority: StateAnchor,
    anchor: Option<StateAnchor>,
    high_tick: GameTick,
    journal: Digest32,
    semantic: Semantic<S>,
    placement_at_open: bool,
    spatial_state: LiveSpatialCitizenState,
    source_fenced: bool,
    basis: Option<Basis>,
    original: Option<PreparedPlan>,
    archive: Option<PlanArchiveStore<S::Archive>>,
    archive_seal: Option<PlanArchiveSeal>,
    evidence: Option<S::Evidence>,
    review: Option<SemanticFurnitureReview>,
    monitor: Option<DurableFurnitureGoalMonitor<S::History>>,
    history_initialized: bool,
    prepared_in_opening: bool,
    clock: Option<Clock<S::ClockJournal>>,
    opening_clock: Option<ClockView>,
    clock_invalidates_goal: bool,
}
#[derive(Clone, Debug)]
pub struct View {
    pub mode: Mode,
    pub anchor: Option<StateAnchor>,
    pub journal: Digest32,
    pub observation: Option<Value>,
    pub original: Option<PreparedPlan>,
    pub review: Option<SemanticFurnitureReview>,
    pub archive: Option<PlanArchiveSeal>,
    pub native: Option<BuildEntry>,
    pub placement: Option<NativeFurniturePlacement>,
    pub goal: Option<FurnitureGoalProgress>,
    pub first_satisfied_anchor: Option<StateAnchor>,
    pub historical_obligation: Option<ObligationStatus>,
    pub history_events: usize,
    pub history_bytes: usize,
    pub history_present: bool,
    pub history_fenced: bool,
    pub placement_control_enabled: bool,
    pub prepared_in_opening: bool,
    pub clock: Option<ClockView>,
    pub clock_control_enabled: bool,
}
pub(crate) fn anchor_json(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,
        "sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
}
fn truth(value: PredicateTruth) -> &'static str {
    match value {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}
impl View {
    pub fn to_json(&self) -> Value {
        let placed = matches!(
            self.placement,
            Some(NativeFurniturePlacement::Placed { .. })
        );
        let placement = self.placement.as_ref().map(|value| match value {
            NativeFurniturePlacement::Unverified => json!({"state":"unverified","verified":false}),
            NativeFurniturePlacement::Placed {
                native_building_id,
                native_job_id,
                native_item_id,
                receipt,
            } => json!({"state":"placed","verified":true,"native_building_id":native_building_id,
                    "native_job_id":native_job_id,"native_item_id":native_item_id,
                    "receipt":receipt.to_hex(),"construction_complete":false}),
        });
        let original=self.original.as_ref().map(|p|json!({
            "plan_digest":p.digest.to_hex(),"intent_id":p.intent_id.to_string(),"anchor":anchor_json(p.anchor),
            "summary":p.summary,"expires_at_tick":p.expires_at_tick.get(),"requires_checkpoint":p.requires_checkpoint,
            "terminal":format!("{:?}",p.terminal_condition),
            "steps":p.steps.iter().map(|step|json!({"key":step.idempotency_key,
                "action":format!("{:?}",step.action),"postconditions":step.postconditions.iter().map(|p|format!("{p:?}")).collect::<Vec<_>>(),
                "obligation":step.obligation.as_ref().map(|o|json!({"deadline_tick":o.deadline_tick.get(),
                    "poll_interval_ticks":o.poll_interval_ticks,"stable_samples":o.stable_for_observations}))})).collect::<Vec<_>>() }));
        let goal=self.goal.as_ref().map(|p|json!({"anchor":anchor_json(p.anchor),
            "source_digest":p.source_digest.to_hex(),"current_truth":truth(p.current_goal),
            "postconditions":truth(p.postconditions),"plan_terminal":truth(p.plan_terminal),
            "obligation_terminal":p.obligation_terminal.map(truth),"failure_predicate":p.failure_predicate.map(truth),
            "temporal_status":p.obligation.as_ref().map(|v|format!("{v:?}")),
            "original_goal_proven":p.current_goal_proven(),"construction_condition":p.construction_condition,
            "construction_proven":p.construction_proven,"receipt_linked":p.receipt_linked,
            "semantic_completion_proven":p.current_goal_proven()&&p.construction_proven&&placed,
            "continuous":p.continuous}));
        let recovery = self.archive.zip(self.original.as_ref()).map(|(seal, p)| {
            let mut value = json!({
            "journal_id":self.journal.to_hex(),"plan_digest":seal.plan_digest.to_hex(),
            "object_digest":seal.object_digest.to_hex(),"anchor":anchor_json(p.anchor),
            "history_present":self.history_present});
            if let Some(clock) = &self.clock {
                value["clock_journal_id"] = json!(clock.journal_id.to_hex());
            }
            value
        });
        json!({"ok":true,"retained_state_currently_verified":true,"mode":self.mode.as_str(),
            "anchor":self.anchor.map(anchor_json),"journal_id":self.journal.to_hex(),
            "runtime_admitted":false,"source_acceptance":"operator-development-source-acceptance/1",
            "source_acceptance_is_compatibility_admission":false,
            "clock_advanced":if self.clock.as_ref().is_some_and(|v|v.has_runs()){Value::Null}else{json!(false)},
            "clock":self.clock.as_ref().map(ClockView::to_json),
            "development_clock_enabled":self.clock_control_enabled,
            "development_placement_enabled":self.placement_control_enabled,
            "observation":self.observation,"original_plan":original,
            "review":self.review.as_ref().map(|r|json!({"paired_seal":r.seal().to_hex(),
                "native_plan_digest":r.native_plan().digest().to_hex(),
                "semantic_plan_digest":r.original_plan().digest.to_hex(),
                "original_object_digest":r.original_object_digest().to_hex()})),
            "recovery":recovery,"prepared_in_this_opening":self.prepared_in_opening,
            "native_state":self.native.as_ref().map(|n|n.state().as_str()),
            "native_phase":self.native.as_ref().and_then(|n|n.native()).map(|n|n.phase().as_str()),
            "native_needs_query":self.native.as_ref().is_some_and(|n|n.needs_reconciliation()),
            "effect_uncertain":self.native.as_ref().is_some_and(|n| (n.dispatch_started() && n.unresolved()) || n.native().is_some_and(|r| r.phase()==BuildPhase::Indeterminate)),
            "placement":placement,"current_goal":goal,"current_goal_available":self.goal.is_some(),
            "first_satisfied_anchor":self.first_satisfied_anchor.map(anchor_json),
            "historical_obligation":self.historical_obligation.as_ref().map(|v|format!("{v:?}")),
            "history":{"events":self.history_events,"bytes":self.history_bytes,"fenced":self.history_fenced},
            "one_goal_limit":1,"automatic_redispatch":false,
            "coverage_omissions":["automatic clock chaining","native clock fortress-lineage identity","other controllers","cross-incarnation recovery",
                "checkpoint and restore","multi-step furnishing","continuous goal truth"]})
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
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        Ok(Self {
            context: c.clone(),
            until: Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis))
                .ok_or_else(|| exhausted("operation deadline overflow"))?,
            bytes: c.budget.max_bytes,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let millis = self
            .until
            .checked_duration_since(Instant::now())
            .filter(|v| *v >= Duration::from_millis(1))
            .ok_or_else(|| exhausted("furniture operation deadline expired"))?
            .as_millis();
        let mut c = self.context.clone();
        c.budget.max_wall_millis =
            u64::try_from(millis).map_err(|_| exhausted("deadline overflow"))?;
        c.budget.max_bytes = self.bytes;
        Ok(c)
    }
    fn reserve(&mut self, n: u64) -> Result<OperationContext> {
        let mut c = self.current()?;
        if n == 0 {
            return Err(exhausted("empty furniture reservation"));
        }
        self.bytes = self
            .bytes
            .checked_sub(n)
            .ok_or_else(|| exhausted("complete furniture work exceeds byte allowance"))?;
        c.budget.max_bytes = n;
        Ok(c)
    }
    fn rest(&mut self) -> Result<OperationContext> {
        self.reserve(self.bytes)
    }
    fn narrow(&self, reserved: &OperationContext) -> Result<OperationContext> {
        let mut c = reserved.clone();
        c.anchor = self.context.anchor;
        c.budget.max_wall_millis = c
            .budget
            .max_wall_millis
            .min(self.current()?.budget.max_wall_millis);
        Ok(c)
    }
    fn boundary(&mut self, c: &OperationContext) -> Result<()> {
        if c.session_id != self.context.session_id
            || c.request_id != self.context.request_id
            || c.grants != self.context.grants
            || c.anchor.fortress_id != self.context.anchor.fortress_id
            || c.anchor.cursor.epoch != self.context.anchor.cursor.epoch
            || c.anchor.tick < self.context.anchor.tick
        {
            return Err(denied(
                "original furniture callback changed its authority context",
            ));
        }
        self.context.anchor = c.anchor;
        self.context.cancellation_requested |= c.cancellation_requested;
        self.until = self.until.min(
            Instant::now()
                .checked_add(Duration::from_millis(c.budget.max_wall_millis))
                .ok_or_else(|| exhausted("callback deadline overflow"))?,
        );
        Ok(())
    }
}
fn grants(mode: Mode, fortress: FortressId, enabled: bool, clock: bool) -> Vec<CapabilityGrant> {
    let mut capabilities = vec![Capability::Query, Capability::Observe];
    if mode == Mode::Control && enabled {
        capabilities.extend([Capability::Plan, Capability::Construct]);
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
struct GuardedEvidence<'a, S: Shell> {
    shell: &'a S,
    owner: &'a mut S::Evidence,
    archive: &'a mut PlanArchiveStore<S::Archive>,
    original: &'a PreparedPlan,
    work: Work,
}
impl<S: Shell> GuardedEvidence<'_, S> {
    fn verify(&mut self, c: &OperationContext) -> Result<()> {
        self.work.boundary(c)?;
        self.shell.check_environment()?;
        if !self.shell.placement_enabled() || !self.shell.checkpoint_exception() {
            return Err(denied(
                "operator construction or checkpoint exception was revoked",
            ));
        }
        if self
            .archive
            .recover_at_authority_floor(&self.work.reserve(ARCHIVE_BYTES)?)?
            != *self.original
        {
            return Err(denied(
                "exact original furniture archive changed at the native boundary",
            ));
        }
        self.shell.check_environment()
    }
}
impl<S: Shell> FurnitureEvidenceOwner for GuardedEvidence<'_, S> {
    fn refresh_reserve_bytes(&self) -> Result<u64> {
        self.owner.refresh_reserve_bytes()
    }
    fn refresh_after_capture(
        &mut self,
        capture: &BuildCapture,
        c: &OperationContext,
    ) -> Result<()> {
        self.verify(c)?;
        let mut current = c.clone();
        current.budget.max_wall_millis = current
            .budget
            .max_wall_millis
            .min(self.work.current()?.budget.max_wall_millis);
        self.owner.refresh_after_capture(capture, &current)?;
        self.verify(c)
    }
    fn routing_evidence(&self) -> Result<LiveRoutingEvidence<'_>> {
        self.owner.routing_evidence()
    }
}

impl<S: Shell> Runtime<S> {
    pub fn from_shell(
        shell: S,
        mode: Mode,
        session: SessionId,
        budget: WorkBudget,
    ) -> Result<Self> {
        budget.validate()?;
        if session == SessionId::NIL
            || shell.fortress() == FortressId::NIL
            || budget.max_bytes > MAX_BYTES
            || budget.max_wall_millis > MAX_WALL_MILLIS
            || budget.max_output_tokens > MAX_OUTPUT_TOKENS
            || budget.max_game_ticks > MAX_GAME_TICKS
        {
            return Err(invalid(
                "semantic furniture session identity or bounded allowance is invalid",
            ));
        }
        shell.check_environment()?;
        let recovery = shell.recovery();
        let initial = recovery.map_or(
            StateAnchor {
                fortress_id: shell.fortress(),
                cursor: ObservationCursor::ORIGIN,
                tick: GameTick(0),
                state_hash: Digest32::ZERO,
            },
            |r| r.anchor,
        );
        if initial.fortress_id != shell.fortress() {
            return Err(denied("original recovery belongs to another fortress"));
        }
        let placement_at_open =
            mode == Mode::Control && shell.placement_enabled() && shell.checkpoint_exception();
        let c = OperationContext {
            session_id: session,
            request_id: RequestId::new(1),
            anchor: initial,
            budget,
            grants: grants(
                mode,
                shell.fortress(),
                placement_at_open,
                shell.clock_enabled(),
            ),
            cancellation_requested: false,
        };
        let mut work = Work::new(&c)?;
        // Initial custody can be published by open_control. Prepay every known
        // opening check before that boundary, so an insufficient byte allowance
        // cannot leave new files without returning the session and journal ID.
        let native_open = work.reserve(256 * MIB)?;
        let semantic_open = work.reserve(VIEW_BYTES)?;
        let inventory_open = work.reserve(VIEW_BYTES)?;
        let clock_open = work.reserve(32 * MIB)?;
        let clock_validation = work.reserve(32 * MIB)?;
        let (native, associations, journal) =
            shell.open_control(mode, &work.narrow(&native_open)?)?;
        let high_tick = GameTick(native.high_tick()).max(initial.tick);
        let mut current = work.current()?;
        current.anchor.tick = high_tick;
        work.context.anchor = current.anchor;
        let mut semantic =
            SemanticFurnitureSession::new(native, associations, &work.narrow(&semantic_open)?)?;
        let inventory = semantic.inventory(&work.narrow(&inventory_open)?)?;
        if inventory.journal_id != journal
            || inventory.total_records() > 1
            || (recovery.is_none() && inventory.total_records() != 0)
            || recovery.is_some_and(|r| r.journal_id != journal)
        {
            return Err(denied(
                "one-goal journal requires exact external original recovery identity",
            ));
        }
        let clock = shell
            .open_clock(mode, &work.narrow(&clock_open)?)?
            .map(Clock::new);
        let mut out = Self {
            shell,
            mode,
            session,
            request: 1,
            budget,
            authority: current.anchor,
            anchor: recovery.map(|r| r.anchor),
            high_tick,
            journal,
            semantic,
            placement_at_open,
            spatial_state: LiveSpatialCitizenState::default(),
            source_fenced: false,
            basis: None,
            original: None,
            archive: None,
            archive_seal: None,
            evidence: None,
            review: None,
            monitor: None,
            history_initialized: recovery.is_some_and(|r| r.history_present),
            prepared_in_opening: false,
            clock,
            opening_clock: None,
            clock_invalidates_goal: false,
        };
        if let Some(recovery) = recovery {
            let mut archive = out
                .shell
                .recover_archive(recovery, &work.reserve(ARCHIVE_BYTES)?)?;
            let plan = archive.recover_at_authority_floor(&work.reserve(ARCHIVE_BYTES)?)?;
            if archive.seal() != recovery.seal || plan.anchor != recovery.anchor {
                return Err(denied(
                    "recovered original furniture archive identity differs",
                ));
            }
            let review = if inventory.total_records() == 0 {
                None
            } else {
                let review = out
                    .semantic
                    .reattach(plan.clone(), &work.reserve(VIEW_BYTES)?)?;
                if review.original_object_digest() != recovery.seal.object_digest {
                    return Err(denied(
                        "original furniture object differs from immutable association",
                    ));
                }
                Some(review)
            };
            out.original = Some(plan);
            out.archive_seal = Some(archive.seal());
            out.archive = Some(archive);
            out.review = review;
        }
        // Retained clock time gates historical policy before proof replay.
        let mut validation = work.narrow(&clock_validation)?;
        validation.anchor = out.authority;
        out.opening_clock = out.inspect_clock(&validation)?;
        out.clock_invalidates_goal = out
            .opening_clock
            .as_ref()
            .is_some_and(|clock| clock.has_runs());
        out.raise_floor();
        work.context.anchor = out.authority;
        if recovery.is_some_and(|r| r.history_present) {
            out.start_monitor(false, &mut work)?;
        }
        out.shell.check_environment()?;
        Ok(out)
    }
    pub fn session_id(&self) -> SessionId {
        self.session
    }
    pub fn budget(&self) -> WorkBudget {
        self.budget
    }
    pub fn anchor(&self) -> Option<StateAnchor> {
        self.anchor
    }
    pub fn mode(&self) -> Mode {
        self.mode
    }
    pub fn placement_control_enabled(&self) -> bool {
        self.mode == Mode::Control
            && self.placement_at_open
            && self.shell.placement_enabled()
            && self.shell.checkpoint_exception()
    }
    pub fn recovery_hint(&self) -> Option<Value> {
        self.archive_seal
            .zip(self.original.as_ref())
            .map(|(seal, plan)| {
                let mut value = json!({
            "journal_id":self.journal.to_hex(),"plan_digest":seal.plan_digest.to_hex(),
            "object_digest":seal.object_digest.to_hex(),"anchor":anchor_json(plan.anchor),
            "history_present":self.history_initialized,
            "archive_publication_unknown":self.archive.is_none(),
            "paired_preparation_present":self.review.is_some()});
                if let Some(clock) = &self.opening_clock {
                    value["clock_journal_id"] = json!(clock.journal_id.to_hex());
                }
                value
            })
    }
    pub fn opening_recovery_reference(&self) -> Value {
        json!({"ok":false,"retained_state_currently_verified":false,"mode":self.mode.as_str(),
            "anchor":self.anchor.map(anchor_json),"journal_id":self.journal.to_hex(),
            "review":self.review.as_ref().map(|r|json!({"paired_seal":r.seal().to_hex(),
                "native_plan_digest":r.native_plan().digest().to_hex(),
                "semantic_plan_digest":r.original_plan().digest.to_hex(),
                "original_object_digest":r.original_object_digest().to_hex()})),
            "recovery":self.recovery_hint(),
            "clock":self.opening_clock.as_ref().map(ClockView::to_json),
            "original_plan":self.original.as_ref().map(|p|json!({"plan_digest":p.digest.to_hex(),"anchor":anchor_json(p.anchor)})),
            "partial_original_publication":self.original.is_some()&&self.review.is_none(),
            "current_goal":null,"current_goal_available":false,
            "runtime_admitted":false,"automatic_redispatch":false})
    }
    /// A checked raw projection may consume archive authority before its
    /// independent policy denies publication. Retain that exact private floor;
    /// only the monitor may publish a new current world anchor or goal proof.
    fn retain_authority_floor(authority: &mut StateAnchor, current: StateAnchor) -> Result<()> {
        if current.fortress_id != authority.fortress_id
            || current.cursor.epoch != authority.cursor.epoch
            || current.cursor.sequence < authority.cursor.sequence
            || current.tick < authority.tick
            || (current.cursor == authority.cursor && current.state_hash != authority.state_hash)
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "construction authority projection regressed or changed lineage",
            ));
        }
        *authority = current;
        Ok(())
    }
    fn raise_floor(&mut self) {
        if let Some(monitor) = &self.monitor {
            self.high_tick = self.high_tick.max(monitor.high_tick());
            if let Some(anchor) = monitor.last_observation_anchor() {
                if self
                    .anchor
                    .is_none_or(|old| anchor.cursor.sequence > old.cursor.sequence)
                {
                    self.anchor = Some(anchor);
                }
                if anchor.cursor.epoch == self.authority.cursor.epoch
                    && anchor.cursor.sequence > self.authority.cursor.sequence
                    && anchor.tick >= self.authority.tick
                {
                    self.authority = anchor;
                }
            }
        }
        if let Some(clock) = &self.clock {
            self.high_tick = self.high_tick.max(GameTick(clock.high_tick()));
        }
        self.authority.tick = self.authority.tick.max(self.high_tick);
    }
    pub fn context(&mut self, cancelled: bool) -> Result<OperationContext> {
        self.shell.check_environment()?;
        self.raise_floor();
        self.request = self
            .request
            .checked_add(1)
            .ok_or_else(|| exhausted("request identities exhausted"))?;
        Ok(OperationContext {
            session_id: self.session,
            request_id: RequestId::new(self.request),
            anchor: self.authority,
            budget: self.budget,
            grants: grants(
                self.mode,
                self.shell.fortress(),
                self.placement_control_enabled(),
                self.clock_control_enabled(),
            ),
            cancellation_requested: cancelled,
        })
    }
    fn check(&self, c: &OperationContext, control: bool) -> Result<()> {
        self.shell.check_environment()?;
        if c.session_id != self.session
            || c.request_id.get() != self.request
            || c.anchor != self.authority
            || c.budget.max_bytes > self.budget.max_bytes
            || c.budget.max_wall_millis > self.budget.max_wall_millis
            || c.budget.max_actions > self.budget.max_actions
            || c.budget.max_entities > self.budget.max_entities
            || c.budget.max_output_tokens > self.budget.max_output_tokens
            || c.budget.max_game_ticks > self.budget.max_game_ticks
        {
            return Err(denied(
                "furniture context differs from its session or widens the allowance",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if !self.placement_control_enabled() {
                return Err(denied(
                    "current operator construction authority is unavailable",
                ));
            }
            c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            c.authorize(Capability::Construct, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn verify_original(&mut self, work: &mut Work) -> Result<()> {
        match (&self.original, &mut self.archive) {
            (Some(plan), Some(archive)) => {
                if archive.recover_at_authority_floor(&work.reserve(ARCHIVE_BYTES)?)? == *plan {
                    Ok(())
                } else {
                    Err(denied("exact original furniture archive changed"))
                }
            }
            (None, None) => Ok(()),
            _ => Err(denied(
                "exact original furniture archive is absent or changed",
            )),
        }
    }
    fn original(&self) -> Result<PreparedPlan> {
        self.original
            .clone()
            .ok_or_else(|| invalid("original construction goal has not been prepared"))
    }
    fn reviewed(&self) -> Result<SemanticFurnitureReview> {
        self.review
            .clone()
            .ok_or_else(|| invalid("original paired construction review is absent"))
    }
    fn observation_page(&self, offset: usize, limit: usize) -> Result<Option<Value>> {
        let Some(basis) = &self.basis else {
            return Ok(None);
        };
        let observation = basis
            .state
            .observation_full()
            .ok_or_else(|| invalid("typed spatial source is absent"))?;
        let snapshot = basis
            .state
            .snapshot()
            .ok_or_else(|| invalid("canonical spatial projection is absent"))?;
        let all = &observation.spatial().operations().items;
        if !(1..=32).contains(&limit) || offset > all.len() {
            return Err(invalid("item page needs a valid offset and limit 1..32"));
        }
        let end = offset.saturating_add(limit).min(all.len());
        let items=all[offset..end].iter().map(|item|json!({
            "entity_id":item_entity_id(item.native_id).get().to_string(),
            "type":item.type_key,"position":[item.raw_position.x,item.raw_position.y,item.raw_position.z],
            "material_type":item.material_type,"material_index":item.material_index,
            "on_ground":item.flags&(1<<6)!=0,"in_job":item.flags&(1<<1)!=0,
            "native_placement_eligibility":"requires_fresh_native_capture"})).collect::<Vec<_>>();
        Ok(Some(
            json!({"seal":basis.seal.to_hex(),"source_digest":observation.source_digest()?.to_hex(),
            "anchor":anchor_json(snapshot.anchor()),"paused":snapshot.paused,"items":items,
            "offset":offset,"limit":limit,"returned":end-offset,"total":all.len(),"complete":end==all.len(),
            "next_query":if end<all.len(){json!({"mode":"items","expected_observation":basis.seal.to_hex(),"offset":end,"limit":limit})}else{Value::Null}}),
        ))
    }
    fn view(&mut self, c: &OperationContext) -> Result<View> {
        self.shell.check_environment()?;
        let mut work = Work::new(c)?;
        self.verify_original(&mut work)?;
        if let Some(monitor) = &mut self.monitor {
            let cost = (monitor.history_byte_len() as u64)
                .checked_add(MIB)
                .ok_or_else(|| exhausted("history verification allowance overflow"))?;
            monitor.verify_history(&work.reserve(cost)?)?;
        }
        let inventory = self.semantic.inventory(&work.reserve(32 * MIB)?)?;
        if inventory.journal_id != self.journal || inventory.total_records() > 1 {
            return Err(denied(
                "one-goal furniture journal identity or count changed",
            ));
        }
        let clock_budget = if self.clock.is_some() {
            Some(work.reserve(8 * MIB)?)
        } else {
            None
        };
        let result = if inventory.total_records() == 0 {
            None
        } else {
            let review = self
                .review
                .as_ref()
                .ok_or_else(|| denied("native furniture has no original paired review"))?;
            Some(self.semantic.inspect(review.seal(), &work.rest()?)?)
        };
        let clock = if let Some(reserved) = clock_budget {
            let target = self.clock_target(result.as_ref().map(|r| r.native_record()))?;
            let canonical = work.narrow(&reserved)?;
            let native = self.shell.clock_context(&canonical, false)?;
            Some(
                self.clock
                    .as_mut()
                    .ok_or_else(|| invalid("clock owner absent"))?
                    .inspect(target.as_ref(), &canonical, &native)?,
            )
        } else {
            None
        };
        self.opening_clock = clock.clone();
        self.raise_floor();
        self.shell.check_environment()?;
        Ok(View {
            mode: self.mode,
            anchor: self.anchor,
            journal: self.journal,
            observation: self.observation_page(0, 32)?,
            original: self.original.clone(),
            review: self.review.clone(),
            archive: self.archive.as_ref().map(|a| a.seal()),
            native: result.as_ref().map(|r| r.native_record().clone()),
            placement: result.as_ref().map(|r| r.placement().clone()),
            goal: self
                .monitor
                .as_ref()
                .filter(|m| !m.is_fenced())
                .and_then(|m| m.latest())
                .filter(|goal| {
                    !self.clock_invalidates_goal
                        && clock.as_ref().is_none_or(|clock| {
                            clock.can_sample_goal() && goal.anchor.tick.get() >= clock.high_tick
                        })
                })
                .cloned(),
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
            placement_control_enabled: self.placement_control_enabled(),
            prepared_in_opening: self.prepared_in_opening,
            clock,
            clock_control_enabled: self.clock_control_enabled(),
        })
    }
    pub fn inspect(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        self.view(c)
    }
    pub fn items_page(
        &mut self,
        seal: Digest32,
        offset: usize,
        limit: usize,
        c: &OperationContext,
    ) -> Result<View> {
        self.check(c, false)?;
        if self.basis.as_ref().is_none_or(|b| b.seal != seal) {
            return Err(invalid(
                "item page requires the current original observation seal",
            ));
        }
        let page = self.observation_page(offset, limit)?;
        let mut view = self.view(c)?;
        view.observation = page;
        Ok(view)
    }
    pub fn observe(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline || self.source_fenced || self.original.is_some() {
            return Err(denied(
                "fresh spatial observation is unavailable; retain the original goal and use wait or recovery",
            ));
        }
        self.basis = None;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let publish = work.reserve(LOCAL_BYTES)?;
        let read = work.rest()?;
        let observation = self.shell.acquire(&read)?;
        let tick = observation.spatial().terrain().tick();
        if tick < self.high_tick {
            self.source_fenced = true;
            return Err(denied("spatial source tick regressed"));
        }
        self.high_tick = self.high_tick.max(tick);
        self.authority.tick = self.authority.tick.max(tick);
        let prior = self.spatial_state.snapshot().map(|s| s.cursor);
        self.spatial_state.publish(observation)?;
        let anchor = self
            .spatial_state
            .snapshot()
            .ok_or_else(|| invalid("spatial publication is absent"))?
            .anchor();
        if prior.is_some_and(|p| p.epoch != anchor.cursor.epoch) {
            self.source_fenced = true;
            return Err(denied(
                "spatial source changed incarnation or identity history",
            ));
        }
        let mut current = work.narrow(&publish)?;
        current.anchor = anchor;
        let policy = self.shell.spatial_policy(&self.spatial_state, &current)?;
        PredicateEvidence::scoped(
            self.spatial_state
                .snapshot()
                .ok_or_else(|| invalid("spatial snapshot absent"))?,
            policy,
        )?;
        self.shell.check_environment()?;
        let observation = self
            .spatial_state
            .observation_full()
            .ok_or_else(|| invalid("typed spatial source absent"))?;
        let mut bytes = b"dfmcp-original-furniture-observation/1\0".to_vec();
        bytes.extend_from_slice(anchor.state_hash.as_bytes());
        bytes.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
        bytes.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
        bytes.extend_from_slice(observation.source_digest()?.as_bytes());
        self.basis = Some(Basis {
            state: self.spatial_state.clone(),
            seal: Digest32::of_bytes(&bytes),
        });
        self.anchor = Some(anchor);
        self.authority = anchor;
        work.context.anchor = anchor;
        self.view(&work.narrow(&output)?)
    }
    pub fn plan(&mut self, request: Request, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        if self.source_fenced || self.original.is_some() || self.archive.is_some() {
            return Err(denied(
                "one original furniture goal is already retained; inspect or recover its paired review",
            ));
        }
        let basis = self
            .basis
            .as_ref()
            .ok_or_else(|| invalid("observe the canonical spatial source before planning"))?;
        if request.observation()? != basis.seal {
            return Err(invalid(
                "request does not bind the current spatial observation",
            ));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        let local = work.reserve(LOCAL_BYTES)?;
        let policy = self.shell.spatial_policy(&basis.state, &local)?;
        let evidence = PredicateEvidence::scoped(
            basis
                .state
                .snapshot()
                .ok_or_else(|| invalid("spatial snapshot absent"))?,
            policy,
        )?;
        let plan = request.compile(&evidence, &local, self.shell.checkpoint_exception())?;
        let selection = request.selection(&basis.state)?;
        let owner_cost = self.shell.evidence_reserve_bytes(&basis.state)?;
        let owner = self
            .shell
            .evidence(basis.state.clone(), &work.reserve(owner_cost)?)?;
        // Preserve the exact intended identity before any archive persistence:
        // a lost fsync acknowledgement cannot permit a replacement goal.
        let archive_check = work.reserve(ARCHIVE_BYTES)?;
        let (_, archive_seal) =
            dfmcp_intent::plan_archive::encode_with_check(&plan, archive_limits(), || {
                archive_check.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
                work.current().map(|_| ())
            })?;
        self.original = Some(plan.clone());
        self.archive_seal = Some(archive_seal);
        self.evidence = Some(owner);
        let archive = self
            .shell
            .create_archive(&plan, &work.reserve(ARCHIVE_BYTES)?)?;
        if archive.seal() != archive_seal {
            return Err(denied(
                "created original archive differs from the intended exact identity",
            ));
        }
        self.archive = Some(archive);
        let evidence_guard = work.reserve(4 * ARCHIVE_BYTES + 1)?;
        let mut guard = self.shell.guard(self.journal, &work.reserve(MIB)?)?;
        let shell = &self.shell;
        let mut guarded = GuardedEvidence {
            shell,
            owner: self
                .evidence
                .as_mut()
                .ok_or_else(|| invalid("evidence owner absent"))?,
            archive: self
                .archive
                .as_mut()
                .ok_or_else(|| invalid("archive absent"))?,
            original: &plan,
            work: Work::new(&evidence_guard)?,
        };
        // Observation and preparation share the remaining allowance, while the
        // two effect-boundary archive checks and final view were prepaid above.
        let observe_bytes = guarded
            .refresh_reserve_bytes()?
            .checked_add(64 * MIB)
            .ok_or_else(|| exhausted("furniture evidence reservation overflow"))?;
        let observed = self.semantic.observe(
            plan.clone(),
            selection,
            &mut guarded,
            &work.reserve(observe_bytes)?,
            |binding, current, timeout| shell.native(binding, selection, current, timeout),
            &mut guard,
        )?;
        self.review = Some(observed.clone());
        let result =
            self.semantic
                .prepare(observed.seal(), &mut guarded, &work.rest()?, &mut guard)?;
        self.prepared_in_opening = result.native_record().state() == BuildState::Prepared;
        self.view(&work.narrow(&output)?)
    }
    pub fn commit(
        &mut self,
        seal: Digest32,
        confirmed: bool,
        c: &OperationContext,
    ) -> Result<View> {
        self.check(c, true)?;
        let review = self.reviewed()?;
        if seal != review.seal() || !confirmed {
            return Err(denied("confirm the exact original paired furniture seal"));
        }
        if !self.prepared_in_opening {
            return Err(denied(
                "recovered or retired preparation cannot dispatch; use original-key recovery",
            ));
        }
        self.prepared_in_opening = false;
        let original = self.original()?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let evidence_guard = work.reserve(2 * ARCHIVE_BYTES + 1)?;
        let mut guard = self.shell.guard(self.journal, &work.reserve(MIB)?)?;
        let mut guarded = GuardedEvidence {
            shell: &self.shell,
            owner: self
                .evidence
                .as_mut()
                .ok_or_else(|| denied("original evidence owner is unavailable"))?,
            archive: self
                .archive
                .as_mut()
                .ok_or_else(|| denied("original archive is unavailable"))?,
            original: &original,
            work: Work::new(&evidence_guard)?,
        };
        self.semantic
            .commit(seal, true, &mut guarded, &work.rest()?, &mut guard)?;
        self.view(&work.narrow(&output)?)
    }
    fn start_monitor(&mut self, initialize: bool, work: &mut Work) -> Result<()> {
        if self.monitor.is_some() {
            return Ok(());
        }
        let review = self.reviewed()?;
        self.verify_original(work)?;
        // Once initialization is attempted, recovery must account for a complete
        // surviving file even if acknowledgement is lost.
        self.history_initialized = true;
        let history = self.shell.history(
            self.journal,
            &review,
            initialize,
            self.mode == Mode::Offline,
            &work.reserve(HISTORY_OPEN_BYTES)?,
        )?;
        let checks = (history.event_count() as u64)
            .checked_mul(4)
            .and_then(|n| n.checked_add(4))
            .and_then(|n| n.checked_mul(ARCHIVE_BYTES))
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| exhausted("historical archive verification allowance overflow"))?;
        let mut archive_work = Work::new(&work.reserve(checks)?)?;
        let original = self.original()?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original plan archive absent"))?;
        let shell = &self.shell;
        let monitor = self.semantic.begin_goal_monitor_durable(
            review.seal(),
            history,
            &work.rest()?,
            |projection, current| {
                archive_work.boundary(current)?;
                shell.check_environment()?;
                if archive.recover_at_authority_floor(&archive_work.reserve(ARCHIVE_BYTES)?)?
                    != original
                {
                    return Err(denied(
                        "original archive changed before historical goal authorization",
                    ));
                }
                let policy = shell.goal_policy(projection, current)?;
                if archive.recover_at_authority_floor(&archive_work.reserve(ARCHIVE_BYTES)?)?
                    != original
                {
                    return Err(denied(
                        "original archive changed during historical goal authorization",
                    ));
                }
                shell.check_environment()?;
                Ok(policy)
            },
        )?;
        self.monitor = Some(monitor);
        self.raise_floor();
        Ok(())
    }
    pub fn wait(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return Err(denied(
                "offline furniture mode cannot acquire a native outcome or goal",
            ));
        }
        self.prepared_in_opening = false;
        let review = self.reviewed()?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let current = self
            .semantic
            .inspect(review.seal(), &work.reserve(32 * MIB)?)?;
        if current.native_record().needs_reconciliation()
            || current
                .native_record()
                .native()
                .is_some_and(|n| n.phase() == BuildPhase::Indeterminate)
        {
            let selection = review.native_plan().before().selection();
            let mut guard = self.shell.guard(self.journal, &work.reserve(MIB)?)?;
            let shell = &self.shell;
            let result = self.semantic.reconcile(
                review.seal(),
                &work.rest()?,
                |binding, current, timeout| shell.native(binding, selection, current, timeout),
                &mut guard,
            )?;
            // Reconciliation and fresh construction acquisition are distinct
            // bounded calls; an ambiguous effect never triggers another placement.
            let _ = result;
            return self.view(&work.narrow(&output)?);
        }
        if !matches!(current.placement(), NativeFurniturePlacement::Placed { .. }) {
            return self.view(&work.narrow(&output)?);
        }
        let mut clock_tick_floor = 0;
        if self.clock.is_some() {
            let target = self
                .clock_target(Some(current.native_record()))?
                .ok_or_else(|| denied("clock history requires the original placement"))?;
            let canonical = work.reserve(8 * MIB)?;
            let native = self.shell.clock_context(&canonical, false)?;
            let clock = self
                .clock
                .as_mut()
                .ok_or_else(|| invalid("clock owner absent"))?;
            let mut view = clock.inspect(Some(&target), &canonical, &native)?;
            if view.needs_reconciliation() {
                let connection = self.shell.clock_context(&work.reserve(4 * MIB)?, false)?;
                let mut source = self.shell.clock_source(&connection)?;
                let canonical = work.reserve(64 * MIB)?;
                let native = self.shell.clock_context(&canonical, false)?;
                view = clock.reconcile(&target, &mut source, &canonical, &native)?;
            }
            clock_tick_floor = view.high_tick;
            let can_sample = view.can_sample_goal();
            self.opening_clock = Some(view);
            self.raise_floor();
            work.context.anchor = self.authority;
            if !can_sample {
                return self.view(&work.narrow(&output)?);
            }
        }
        if self.monitor.is_none() {
            let reserve = work.rest()?;
            self.start_monitor(!self.history_initialized, &mut Work::new(&reserve)?)?;
            return self.view(&work.narrow(&output)?);
        }
        let cursor = self
            .monitor
            .as_ref()
            .ok_or_else(|| invalid("goal monitor absent"))?
            .high_cursor();
        let cursor = ObservationCursor {
            epoch: cursor.epoch,
            sequence: cursor
                .sequence
                .max(self.authority.cursor.sequence)
                .checked_add(1)
                .ok_or_else(|| exhausted("construction observation cursor exhausted"))?,
        };
        let mut archive_work = Work::new(&work.reserve(4 * ARCHIVE_BYTES + 1)?)?;
        let original = self.original()?;
        let archive = self
            .archive
            .as_mut()
            .ok_or_else(|| denied("original archive absent"))?;
        let shell = &self.shell;
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| invalid("goal monitor absent"))?;
        let authority = &mut self.authority;
        let result = self.semantic.poll_original_goal_durable(
            monitor,
            cursor,
            &work.rest()?,
            |binding, current| shell.goal_source(binding, current),
            |projection, current| {
                archive_work.boundary(current)?;
                Self::retain_authority_floor(authority, current.anchor)?;
                shell.check_environment()?;
                if projection.anchor().tick.get() < clock_tick_floor {
                    return Err(DfmcpError::new(
                        ErrorCode::StaleAnchor,
                        "construction observation predates retained native clock evidence",
                    ));
                }
                if archive.recover_at_authority_floor(&archive_work.reserve(ARCHIVE_BYTES)?)?
                    != original
                {
                    return Err(denied(
                        "original archive changed before current goal authorization",
                    ));
                }
                let policy = shell.goal_policy(projection, current)?;
                if archive.recover_at_authority_floor(&archive_work.reserve(ARCHIVE_BYTES)?)?
                    != original
                {
                    return Err(denied(
                        "original archive changed during current goal authorization",
                    ));
                }
                shell.check_environment()?;
                Ok(policy)
            },
        );
        self.raise_floor();
        let result = result?;
        self.anchor = Some(result.progress().anchor);
        self.clock_invalidates_goal = false;
        self.authority = result.progress().anchor;
        self.authority.tick = self.authority.tick.max(self.high_tick);
        work.context.anchor = self.authority;
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        if self.mode == Mode::Offline {
            return Err(denied(
                "offline mode cannot retire native furniture preparation",
            ));
        }
        self.prepared_in_opening = false;
        if self.review.is_none() {
            // There is no owned native entry to retire. Preserve the partial
            // original archive and return evidence for explicit session release.
            return self.view(c);
        }
        let review = self.reviewed()?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let selection = review.native_plan().before().selection();
        let mut guard = self.shell.guard(self.journal, &work.reserve(MIB)?)?;
        let shell = &self.shell;
        self.semantic.cancel(
            review.seal(),
            &work.rest()?,
            |binding, current, timeout| shell.native(binding, selection, current, timeout),
            &mut guard,
        )?;
        self.view(&work.narrow(&output)?)
    }
}

#[cfg(test)]
mod tests;
