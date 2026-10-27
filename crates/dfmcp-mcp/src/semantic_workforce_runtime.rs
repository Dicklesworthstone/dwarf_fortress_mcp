//! One original SetLabor goal: exact canonical identities, archived semantics,
//! paired native review, one dispatch and durable foreground goal observation.
//! Operator-owned source policy is explicit development acceptance, not admission.
use dfmcp_adapter::LiveObservationCapsule;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::live_projection::{
    LiveWorldProjection, project_live_capsule, raw_unit_id_to_entity_id,
};
use dfmcp_adapter::live_routing::LiveRoutingEvidence;
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_adapter::semantic_workforce::store::AssociationStore;
use dfmcp_adapter::semantic_workforce::{
    DurableWorkforceGoalMonitor, GoalHistoryStore, SemanticWorkforceReview,
    SemanticWorkforceSession, SingleLaborResult, WorkforceEvidenceOwner, WorkforceGoalProgress,
};
use dfmcp_adapter::workforce_control::journal::{
    AssignmentRecord, AssignmentState, WorkforceBinding, WorkforceMode,
};
use dfmcp_adapter::workforce_control::rpc::WorkforceSource;
use dfmcp_adapter::workforce_control::{AssignmentPhase, WorkforceCapture};
use dfmcp_adapter::workforce_session::WorkforceSession;
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

pub mod identity;
pub mod native;
pub mod policy;
pub mod request;
use identity::CapsuleStore;
pub use native::{Config, NativeRuntime};
pub use request::Request;

pub const DEFAULT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
// Retained goal observations are independently reprojected and reauthorized.
// Their logical byte-work reservations accumulate on recovery, so the bounded
// full history may require an explicit allowance above the ordinary default.
pub const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
pub const MAX_WALL_MILLIS: u64 = 60_000;
pub const MAX_OUTPUT_TOKENS: u32 = 32_768;
const MIB: u64 = 1024 * 1024;
const VIEW_BYTES: u64 = 64 * MIB;
const ARCHIVE_BYTES: u64 = 4 * MIB;
const IDENTITY_BYTES: u64 = 32 * MIB;

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
        // Archive accounting includes collection members and repeated predicate
        // values. A complete 32-citizen goal exceeds 512 archive nodes while
        // remaining below the adapter's separate 512-predicate complexity cap.
        max_nodes: 1024,
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
    pub const fn journal(self) -> WorkforceMode {
        match self {
            Self::Offline => WorkforceMode::Offline,
            Self::Reconcile => WorkforceMode::Recover,
            Self::Control => WorkforceMode::Control,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Recovery {
    pub seal: PlanArchiveSeal,
    pub anchor: StateAnchor,
    pub identity_digest: Digest32,
    pub history_present: bool,
}
/// Trusted factories are constructor inputs. Client requests cannot choose a
/// source endpoint, path, native unit ID, work detail or policy issuer.
pub trait Shell {
    type Journal: EffectJournalStorage;
    type Association: EffectJournalStorage;
    type Native: WorkforceSource;
    type Archive: EffectJournalStorage;
    type Identity: EffectJournalStorage;
    type History: EffectJournalStorage;
    type Evidence: WorkforceEvidenceOwner;
    fn acceptance(&self) -> &policy::SourceAcceptance;
    fn recovery(&self) -> Option<Recovery>;
    fn check_environment(&self) -> Result<()>;
    fn labor_enabled(&self) -> bool;
    fn open_control(
        &self,
        mode: Mode,
        c: &OperationContext,
    ) -> Result<(
        WorkforceSession<Self::Journal>,
        AssociationStore<Self::Association>,
        Digest32,
    )>;
    fn native(&self, binding: &WorkforceBinding, c: &OperationContext) -> Result<Self::Native>;
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
    fn create_identity(
        &self,
        capsule: &LiveObservationCapsule,
        c: &OperationContext,
    ) -> Result<CapsuleStore<Self::Identity>>;
    fn recover_identity(
        &self,
        seal: Digest32,
        c: &OperationContext,
    ) -> Result<CapsuleStore<Self::Identity>>;
    fn history(
        &self,
        journal: Digest32,
        review: &SemanticWorkforceReview,
        initialize: bool,
        read_only: bool,
        c: &OperationContext,
    ) -> Result<GoalHistoryStore<Self::History>>;
}
type Semantic<S> = SemanticWorkforceSession<<S as Shell>::Journal, <S as Shell>::Association>;
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
    labor_at_open: bool,
    basis: Option<Basis>,
    evidence: Option<S::Evidence>,
    original: Option<PreparedPlan>,
    archive: Option<PlanArchiveStore<S::Archive>>,
    identity: Option<CapsuleStore<S::Identity>>,
    review: Option<SemanticWorkforceReview>,
    monitor: Option<DurableWorkforceGoalMonitor<S::History>>,
    history_initialized: bool,
}
#[derive(Clone, Debug)]
pub struct View {
    pub mode: Mode,
    pub anchor: StateAnchor,
    pub journal: Digest32,
    pub observation: Option<Value>,
    pub original: Option<PreparedPlan>,
    pub review: Option<SemanticWorkforceReview>,
    pub archive: Option<PlanArchiveSeal>,
    pub identity_digest: Option<Digest32>,
    pub native: Option<AssignmentRecord>,
    pub action: Option<SingleLaborResult>,
    pub goal: Option<WorkforceGoalProgress>,
    pub first_satisfied_anchor: Option<StateAnchor>,
    pub historical_obligation: Option<ObligationStatus>,
    pub history_events: usize,
    pub history_bytes: usize,
    pub history_present: bool,
    pub history_fenced: bool,
    pub labor_control_enabled: bool,
}
fn anchor_json(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,"sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
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
        let verified = matches!(self.action, Some(SingleLaborResult::Verified { .. }));
        let action=self.action.as_ref().map(|a|match a {
            SingleLaborResult::Unverified=>json!({"state":"unverified","verified":false,"receipt":null}),
            SingleLaborResult::Verified{receipt}=>json!({"state":"verified","verified":true,"receipt":receipt.to_hex()}),
            SingleLaborResult::AppliedOutsideSemantics{receipt}=>json!({"state":"applied_outside_semantics","verified":false,"receipt":receipt.to_hex()}),
        });
        let original=self.original.as_ref().map(|p|json!({"plan_digest":p.digest.to_hex(),"intent_id":p.intent_id.to_string(),"anchor":anchor_json(p.anchor),"summary":p.summary,"expires_at_tick":p.expires_at_tick.get(),"terminal":format!("{:?}",p.terminal_condition),
            "steps":p.steps.iter().map(|s|json!({"key":s.idempotency_key,"action":format!("{:?}",s.action),"postconditions":s.postconditions.iter().map(|p|format!("{p:?}")).collect::<Vec<_>>(),"obligation":s.obligation.as_ref().map(|o|json!({"deadline_tick":o.deadline_tick.get(),"poll_interval_ticks":o.poll_interval_ticks,"stable_samples":o.stable_for_observations}))})).collect::<Vec<_>>() }));
        let goal=self.goal.as_ref().map(|p|json!({"anchor":anchor_json(p.anchor),"source_digest":p.source_digest.to_hex(),"current_truth":truth(p.current_goal),"postconditions":truth(p.postconditions),"plan_terminal":truth(p.plan_terminal),"obligation_terminal":p.obligation_terminal.map(truth),"failure_predicate":p.failure_predicate.map(truth),"temporal_status":p.obligation.as_ref().map(|s|format!("{s:?}")),"original_goal_proven":p.current_goal_proven(),"semantic_completion_proven":p.current_goal_proven()&&verified,"continuous":p.continuous}));
        let recovery=self.archive.zip(self.original.as_ref()).zip(self.identity_digest).map(|((seal,p),identity)|json!({"plan_digest":seal.plan_digest.to_hex(),"object_digest":seal.object_digest.to_hex(),"anchor":anchor_json(p.anchor),"identity_digest":identity.to_hex(),"journal_id":self.journal.to_hex(),"history_present":self.history_present}));
        json!({"ok":true,"retained_state_currently_verified":true,"mode":self.mode.as_str(),"anchor":anchor_json(self.anchor),"journal_id":self.journal.to_hex(),"identity_digest":self.identity_digest.map(|d|d.to_hex()),"partial_goal_publication":self.identity_digest.is_some()&&self.original.is_none(),"runtime_admitted":false,"source_acceptance":policy::ACCEPTANCE_KIND,"source_acceptance_is_compatibility_admission":false,
            "clock_advanced":false,"development_labor_enabled":self.labor_control_enabled,"observation":self.observation,"original_plan":original,
            "review":self.review.as_ref().map(|r|json!({"paired_seal":r.seal().to_hex(),"native_plan_digest":r.native_plan().digest().to_hex(),"semantic_plan_digest":r.original_plan().digest.to_hex()})),"recovery":recovery,
            "native_state":self.native.as_ref().map(|n|n.state().as_str()),"native_phase":self.native.as_ref().and_then(|n|n.effect()).map(|e|e.phase().as_str()),
            "native_needs_query":self.native.as_ref().is_some_and(|n|n.needs_query()),"effect_uncertain":self.native.as_ref().is_some_and(|n|n.state().unresolved()),"action_result":action,
            "current_goal":goal,"current_goal_available":self.goal.is_some(),"first_satisfied_anchor":self.first_satisfied_anchor.map(anchor_json),"historical_obligation":self.historical_obligation.as_ref().map(|s|format!("{s:?}")),
            "history":{"events":self.history_events,"bytes":self.history_bytes,"fenced":self.history_fenced},"one_goal_limit":1,"automatic_redispatch":false,
            "coverage_omissions":["job assignment and execution","native skill eligibility","fortress-wide workforce completeness","automatic clock advancement","continuous goal truth"]})
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
        let millis = self
            .until
            .checked_duration_since(Instant::now())
            .ok_or_else(|| exhausted("semantic workforce deadline expired"))?
            .as_millis();
        if millis == 0 {
            return Err(exhausted("semantic workforce deadline expired"));
        }
        let mut c = self.context.clone();
        c.budget.max_wall_millis =
            u64::try_from(millis).map_err(|_| exhausted("deadline overflow"))?;
        c.budget.max_bytes = self.bytes;
        Ok(c)
    }
    fn reserve(&mut self, n: u64) -> Result<OperationContext> {
        let mut c = self.current()?;
        self.bytes = self
            .bytes
            .checked_sub(n)
            .ok_or_else(|| exhausted("complete workforce operation exceeds byte-work budget"))?;
        c.budget.max_bytes = n;
        Ok(c)
    }
    fn rest(&mut self) -> Result<OperationContext> {
        self.reserve(self.bytes)
    }
    fn narrow(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = c.clone();
        out.budget.max_wall_millis = self.current()?.budget.max_wall_millis;
        out.anchor = self.context.anchor;
        Ok(out)
    }
}
fn grants(mode: Mode, fortress: dfmcp_core::FortressId, enabled: bool) -> Vec<CapabilityGrant> {
    let mut capabilities = vec![Capability::Query, Capability::Observe];
    if mode == Mode::Control && enabled {
        capabilities.extend([Capability::Plan, Capability::ConfigureLabor]);
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
/// Verify immutable original custody inside every evidence refresh, including
/// the refresh after native DispatchStarted synchronization. That is later
/// than the public request's initial verification and prevents archive damage
/// during synchronization from authorizing the assignment setter.
struct GuardedEvidence<'a, S: Shell> {
    shell: &'a S,
    owner: &'a mut S::Evidence,
    archive: &'a mut PlanArchiveStore<S::Archive>,
    identity: &'a mut CapsuleStore<S::Identity>,
    original: &'a PreparedPlan,
    work: Work,
}
impl<S: Shell> GuardedEvidence<'_, S> {
    fn verify(&mut self, c: &OperationContext) -> Result<()> {
        self.shell.check_environment()?;
        if !self.shell.labor_enabled() {
            return Err(denied("operator labor authority was revoked"));
        }
        self.work.context.anchor = c.anchor;
        if self.archive.recover(&self.work.reserve(ARCHIVE_BYTES)?)? != *self.original {
            return Err(denied(
                "original plan archive changed at the effect boundary",
            ));
        }
        let capsule = self.identity.recover(&self.work.reserve(IDENTITY_BYTES)?)?;
        let projection = project_live_capsule(
            &capsule,
            self.original.anchor.fortress_id,
            self.original.anchor.cursor,
        )?;
        if projection.snapshot.anchor() != self.original.anchor {
            return Err(denied(
                "original identity archive changed at the effect boundary",
            ));
        }
        self.shell.acceptance().citizens(&capsule, &projection, c)?;
        self.shell.check_environment()
    }
}
impl<S: Shell> WorkforceEvidenceOwner for GuardedEvidence<'_, S> {
    fn refresh_after_capture(
        &mut self,
        capture: &WorkforceCapture,
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
        self.shell.check_environment()?;
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
        shell.check_environment()?;
        shell.acceptance().validate()?;
        budget.validate()?;
        if session == SessionId::NIL
            || budget.max_bytes > MAX_BYTES
            || budget.max_wall_millis > MAX_WALL_MILLIS
            || budget.max_entities < 8192
            || budget.max_actions != 1
            || budget.max_output_tokens > MAX_OUTPUT_TOKENS
            || budget.max_game_ticks != 0
        {
            return Err(exhausted(
                "runtime requires zero game ticks, 8192 entities and one action within fixed ceilings",
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
            return Err(denied("recovery belongs to another operator fortress"));
        }
        let c = OperationContext {
            session_id: session,
            request_id: RequestId::new(1),
            anchor,
            budget,
            grants: grants(mode, anchor.fortress_id, shell.labor_enabled()),
            cancellation_requested: false,
        };
        let mut work = Work::new(&c)?;
        let (native, associations, journal) =
            shell.open_control(mode, &work.reserve(320 * MIB)?)?;
        let labor_at_open = mode == Mode::Control && native.mode() == WorkforceMode::Control;
        let semantic =
            SemanticWorkforceSession::new(native, associations, &work.reserve(32 * MIB)?)?;
        let mut out = Self {
            shell,
            mode,
            session,
            request: 1,
            budget,
            anchor,
            semantic,
            journal,
            labor_at_open,
            basis: None,
            evidence: None,
            original: None,
            archive: None,
            identity: None,
            review: None,
            monitor: None,
            history_initialized: recovery.is_some_and(|r| r.history_present),
        };
        let inventory = out.semantic.inventory(&work.reserve(32 * MIB)?)?;
        out.shell
            .acceptance()
            .native_manifest(inventory.binding.manifest())?;
        if inventory.records.len() > 1 || (!inventory.records.is_empty() && recovery.is_none()) {
            return Err(denied(
                "one-goal runtime requires the external exact archive and identity handoff for native work",
            ));
        }
        if let Some(r) = recovery {
            let mut identity = out
                .shell
                .recover_identity(r.identity_digest, &work.reserve(IDENTITY_BYTES)?)?;
            let capsule = identity.recover(&work.reserve(IDENTITY_BYTES)?)?;
            let projection = project_live_capsule(&capsule, anchor.fortress_id, anchor.cursor)?;
            if projection.snapshot.anchor() != anchor {
                return Err(denied(
                    "external original anchor differs from identity archive",
                ));
            }
            out.shell.acceptance().historical_citizens(
                &capsule,
                &projection,
                &work.reserve(MIB)?,
            )?;
            let mut archive = out
                .shell
                .recover_archive(r, &work.reserve(ARCHIVE_BYTES)?)?;
            let plan = archive.recover(&work.reserve(ARCHIVE_BYTES)?)?;
            if plan.anchor != anchor {
                return Err(denied("external original anchor differs from plan archive"));
            }
            out.identity = Some(identity);
            out.original = Some(plan.clone());
            out.archive = Some(archive);
            if !inventory.records.is_empty() {
                out.review = Some(out.semantic.reattach(plan, &work.reserve(32 * MIB)?)?);
                if r.history_present {
                    out.start_monitor(false, &mut work)?;
                }
            } else if r.history_present {
                return Err(denied(
                    "goal history cannot exist without original native preparation",
                ));
            }
        }
        out.raise_monitor_floor();
        Ok(out)
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
    pub fn labor_control_enabled(&self) -> bool {
        self.labor_at_open && self.mode == Mode::Control && self.shell.labor_enabled()
    }
    pub fn opening_recovery_reference(&self) -> Value {
        let recovery=self.archive.as_ref().zip(self.original.as_ref()).zip(self.identity.as_ref()).map(|((a,p),i)|json!({"plan_digest":a.seal().plan_digest.to_hex(),"object_digest":a.seal().object_digest.to_hex(),"anchor":anchor_json(p.anchor),"identity_digest":i.seal().to_hex(),"journal_id":self.journal.to_hex(),"history_present":self.history_initialized}));
        json!({"ok":false,"retained_state_currently_verified":false,"mode":self.mode.as_str(),"anchor":anchor_json(self.anchor),"journal_id":self.journal.to_hex(),"identity_digest":self.identity.as_ref().map(|i|i.seal().to_hex()),"review":self.review.as_ref().map(|r|json!({"paired_seal":r.seal().to_hex(),"native_plan_digest":r.native_plan().digest().to_hex(),"semantic_plan_digest":r.original_plan().digest.to_hex()})),"original_plan":self.original.as_ref().map(|p|json!({"plan_digest":p.digest.to_hex(),"anchor":anchor_json(p.anchor)})),"recovery":recovery,"current_goal":null,"current_goal_available":false,"runtime_admitted":false,"automatic_redispatch":false})
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
                self.labor_control_enabled(),
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
            || c.budget.max_game_ticks != 0
        {
            return Err(denied(
                "runtime context differs from retained session or widens its allowance",
            ));
        }
        c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if control {
            if !self.labor_control_enabled() {
                return Err(denied(
                    "runtime lacks current operator labor control authority",
                ));
            }
            c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
            c.authorize(Capability::ConfigureLabor, RiskTier::Guarded, &[], None)?;
        }
        Ok(())
    }
    fn raise_monitor_floor(&mut self) {
        if let Some(a) = self
            .monitor
            .as_ref()
            .and_then(|m| m.last_observation_anchor())
        {
            if a.cursor.sequence > self.anchor.cursor.sequence && a.tick >= self.anchor.tick {
                self.anchor = a;
            }
        }
    }
    fn verify_original(&mut self, work: &mut Work) -> Result<()> {
        if let Some(original) = &self.original {
            let archive = self
                .archive
                .as_mut()
                .ok_or_else(|| denied("original plan archive is absent"))?;
            if archive.recover(&work.reserve(ARCHIVE_BYTES)?)? != *original {
                return Err(denied("original archive differs from retained plan"));
            }
            let capsule = self
                .identity
                .as_mut()
                .ok_or_else(|| denied("original citizen identity archive is absent"))?
                .recover(&work.reserve(IDENTITY_BYTES)?)?;
            if project_live_capsule(
                &capsule,
                original.anchor.fortress_id,
                original.anchor.cursor,
            )?
            .snapshot
            .anchor()
                != original.anchor
            {
                return Err(denied("identity archive differs from original plan anchor"));
            }
        }
        Ok(())
    }
    fn start_monitor(&mut self, initialize: bool, work: &mut Work) -> Result<()> {
        if self.monitor.is_some() {
            return Ok(());
        }
        let review = self
            .review
            .as_ref()
            .ok_or_else(|| invalid("original workforce review is absent"))?
            .clone();
        self.verify_original(work)?;
        let capsule = self
            .identity
            .as_mut()
            .ok_or_else(|| denied("original identity archive absent"))?
            .recover(&work.reserve(IDENTITY_BYTES)?)?;
        let projection = project_live_capsule(
            &capsule,
            review.original_plan().anchor.fortress_id,
            review.original_plan().anchor.cursor,
        )?;
        let policy = self.shell.acceptance().historical_citizens(
            &capsule,
            &projection,
            &work.reserve(MIB)?,
        )?;
        let scope = LiveRoutingEvidence::citizens_v1(
            PredicateEvidence::scoped(&projection.snapshot, policy)?,
            &projection,
            &capsule,
        )?;
        let history = self.shell.history(
            self.journal,
            &review,
            initialize,
            self.mode == Mode::Offline,
            &work.reserve(64 * MIB)?,
        )?;
        self.history_initialized = true;
        let shell = &self.shell;
        let monitor = self.semantic.begin_durable_goal_monitor(
            history,
            review.seal(),
            &scope,
            &work.rest()?,
            |p, c| {
                shell.check_environment()?;
                shell.acceptance().goal(p, c)
            },
        )?;
        self.monitor = Some(monitor);
        Ok(())
    }
    fn observation_page(&self, offset: usize, limit: usize) -> Result<Option<Value>> {
        let Some(b) = &self.basis else {
            return Ok(None);
        };
        if !(1..=32).contains(&limit) || offset > b.capsule.citizens.len() {
            return Err(invalid(
                "citizen page requires a valid offset and limit 1..32",
            ));
        }
        let end = offset.saturating_add(limit).min(b.capsule.citizens.len());
        let units=b.capsule.citizens[offset..end].iter().map(|citizen| {
            let id=raw_unit_id_to_entity_id(citizen.unit_id)?;
            let record=b.projection.snapshot.graph.entities.get(&id).ok_or_else(||invalid("canonical citizen missing"))?;
            Ok(json!({"entity_id":id.get().to_string(),"generation":record.generation,"native_unit_id":citizen.unit_id,"name":citizen.name}))
        }).collect::<Result<Vec<_>>>()?;
        Ok(Some(
            json!({"seal":b.seal.to_hex(),"source_digest":b.capsule.content_digest.to_hex(),"paused":true,"anchor":anchor_json(b.projection.snapshot.anchor()),"citizens":b.capsule.citizens.len(),"units":units,"offset":offset,"limit":limit,"returned":end-offset,"total":b.capsule.citizens.len(),"complete":end==b.capsule.citizens.len(),"next_query":if end<b.capsule.citizens.len(){json!({"mode":"citizens","expected_observation":b.seal.to_hex(),"offset":end,"limit":limit})}else{Value::Null}}),
        ))
    }
    fn view(&mut self, c: &OperationContext) -> Result<View> {
        self.shell.check_environment()?;
        let mut work = Work::new(c)?;
        self.verify_original(&mut work)?;
        let inventory = self.semantic.inventory(&work.reserve(8 * MIB)?)?;
        if inventory.records.len() > 1 {
            return Err(denied("one-goal custody contains multiple native records"));
        }
        let result = if inventory.records.is_empty() {
            None
        } else if let Some(review) = &self.review {
            Some(self.semantic.inspect(review.seal(), &work.rest()?)?)
        } else {
            return Err(denied("native record has no attached original review"));
        };
        self.shell.check_environment()?;
        Ok(View {
            mode: self.mode,
            anchor: self.anchor,
            journal: self.journal,
            observation: self.observation_page(0, 32)?,
            original: self.original.clone(),
            review: self.review.clone(),
            archive: self.archive.as_ref().map(|a| a.seal()),
            identity_digest: self.identity.as_ref().map(|i| i.seal()),
            native: result.as_ref().map(|r| r.native_record().clone()),
            action: result.as_ref().map(|r| r.action_result().clone()),
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
            labor_control_enabled: self.labor_control_enabled(),
        })
    }
    pub fn inspect(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, false)?;
        self.view(c)
    }
    pub fn citizens_page(
        &mut self,
        expected: Digest32,
        offset: usize,
        limit: usize,
        c: &OperationContext,
    ) -> Result<View> {
        self.check(c, false)?;
        if self.basis.as_ref().is_none_or(|b| b.seal != expected) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "citizen page requires the exact retained observation seal",
            ));
        }
        let page = self.observation_page(offset, limit)?;
        let mut view = self.view(c)?;
        view.observation = page;
        Ok(view)
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
        self.verify_original(&mut work)?;
        if self
            .original
            .as_ref()
            .is_some_and(|p| p.anchor != self.anchor)
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "original goal already has later observations; use wait without replacing its source or deadline",
            ));
        }
        let capsule = self.shell.acquire(&work.reserve(16 * MIB)?)?;
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
                    .ok_or_else(|| exhausted("canonical cursor exhausted"))?,
            });
        let projection = project_live_capsule(&capsule, self.anchor.fortress_id, cursor)?;
        if self
            .original
            .as_ref()
            .is_some_and(|p| p.anchor != projection.snapshot.anchor())
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "original semantic source changed; goal cannot be replaced or renewed",
            ));
        }
        work.context.anchor = projection.snapshot.anchor();
        self.shell
            .acceptance()
            .citizens(&capsule, &projection, &work.reserve(MIB)?)?;
        let evidence = self
            .shell
            .evidence(capsule.clone(), &work.reserve(16 * MIB)?)?;
        let mut bytes = b"dfmcp-native-workforce-observation/1\0".to_vec();
        bytes.extend_from_slice(capsule.content_digest.as_bytes());
        bytes.extend_from_slice(projection.snapshot.state_hash.as_bytes());
        bytes.extend_from_slice(&cursor.epoch.to_be_bytes());
        bytes.extend_from_slice(&cursor.sequence.to_be_bytes());
        self.anchor = projection.snapshot.anchor();
        self.basis = Some(Basis {
            capsule,
            projection,
            seal: Digest32::of_bytes(&bytes),
        });
        self.evidence = Some(evidence);
        self.view(&work.narrow(&output)?)
    }
    pub fn plan(&mut self, request: Request, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let b = self
            .basis
            .as_ref()
            .filter(|b| request.observation().is_ok_and(|s| s == b.seal))
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "exact current observation seal is required",
                )
            })?;
        let policy =
            self.shell
                .acceptance()
                .citizens(&b.capsule, &b.projection, &work.reserve(MIB)?)?;
        let plan = request.compile(
            &PredicateEvidence::scoped(&b.projection.snapshot, policy)?,
            &work.reserve(MIB)?,
        )?;
        if let Some(original) = &self.original {
            if original != &plan {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "runtime retains one exact original goal and cannot replace it",
                ));
            }
            if self.review.is_some()
                && !self
                    .semantic
                    .inventory(&work.reserve(8 * MIB)?)?
                    .records
                    .is_empty()
            {
                return self.view(&work.narrow(&output)?);
            }
        } else {
            if let Some(identity) = &mut self.identity {
                if identity.recover(&work.reserve(IDENTITY_BYTES)?)? != b.capsule {
                    return Err(denied(
                        "partial original publication belongs to another observation",
                    ));
                }
            } else {
                self.identity = Some(
                    self.shell
                        .create_identity(&b.capsule, &work.reserve(IDENTITY_BYTES)?)?,
                );
            }
            let archive = self
                .shell
                .create_archive(&plan, &work.reserve(ARCHIVE_BYTES)?)?;
            self.original = Some(plan.clone());
            self.archive = Some(archive);
        }
        let guard = work.reserve(256 * MIB)?;
        let observe = work.reserve(128 * MIB)?;
        let prepare = work.reserve(128 * MIB)?;
        let shell = &self.shell;
        let mut evidence = GuardedEvidence {
            shell,
            owner: self
                .evidence
                .as_mut()
                .ok_or_else(|| invalid("observe before semantic preparation"))?,
            archive: self
                .archive
                .as_mut()
                .ok_or_else(|| denied("original archive absent"))?,
            identity: self
                .identity
                .as_mut()
                .ok_or_else(|| denied("original identity absent"))?,
            original: self
                .original
                .as_ref()
                .ok_or_else(|| denied("original plan absent"))?,
            work: Work::new(&guard)?,
        };
        let review =
            self.semantic
                .observe(plan, &mut evidence, &work.narrow(&observe)?, |b, c| {
                    shell.native(b, c)
                })?;
        self.review = Some(review.clone());
        self.semantic.prepare(
            review.seal(),
            &mut evidence,
            &work.narrow(&prepare)?,
            |b, c| shell.native(b, c),
        )?;
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
            return Err(denied(
                "confirm the exact paired semantic/native review seal",
            ));
        }
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let guard = work.reserve(192 * MIB)?;
        let shell = &self.shell;
        let mut evidence = GuardedEvidence {
            shell,
            owner: self
                .evidence
                .as_mut()
                .ok_or_else(|| invalid("reacquire the exact original observation before commit"))?,
            archive: self
                .archive
                .as_mut()
                .ok_or_else(|| denied("original archive absent"))?,
            identity: self
                .identity
                .as_mut()
                .ok_or_else(|| denied("original identity absent"))?,
            original: self
                .original
                .as_ref()
                .ok_or_else(|| denied("original plan absent"))?,
            work: Work::new(&guard)?,
        };
        self.basis = None;
        let result = self
            .semantic
            .commit(seal, true, &mut evidence, &work.rest()?, |b, c| {
                shell.native(b, c)
            });
        self.evidence = None;
        result?;
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
        self.verify_original(&mut work)?;
        if self.mode == Mode::Offline {
            return self.view(&work.narrow(&output)?);
        }
        let shell = &self.shell;
        let current =
            self.semantic
                .reconcile(review.seal(), &work.reserve(128 * MIB)?, |b, c| {
                    shell.native(b, c)
                })?;
        if matches!(
            current.native_record().state(),
            AssignmentState::Intent
                | AssignmentState::Prepared
                | AssignmentState::CancelledBeforeDispatch
        ) || current.native_record().effect().is_some_and(|e| {
            matches!(
                e.phase(),
                AssignmentPhase::Refused | AssignmentPhase::Cancelled
            )
        }) {
            return self.view(&work.narrow(&output)?);
        }
        c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        // Attempted current sampling retires stale preparation affordances even
        // if acquisition or durable publication fails.
        self.basis = None;
        self.evidence = None;
        if self.monitor.is_none() {
            let reserved = work.reserve(512 * MIB)?;
            self.start_monitor(!self.history_initialized, &mut Work::new(&reserved)?)?;
        }
        let monitor = self
            .monitor
            .as_mut()
            .ok_or_else(|| invalid("original goal monitor absent"))?;
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
        let result = self.semantic.poll_original_goal_durable(
            monitor,
            cursor,
            &work.rest()?,
            |b, c| shell.native(b, c),
            |p, c| {
                shell.check_environment()?;
                shell.acceptance().goal(p, c)
            },
        );
        self.raise_monitor_floor();
        if let Ok(sample) = &result {
            self.anchor = sample.progress().anchor;
        }
        work.context.anchor = self.anchor;
        result?;
        self.basis = None;
        self.evidence = None;
        self.view(&work.narrow(&output)?)
    }
    pub fn cancel(&mut self, c: &OperationContext) -> Result<View> {
        self.check(c, true)?;
        let seal = self
            .review
            .as_ref()
            .ok_or_else(|| invalid("no retained original goal to cancel"))?
            .seal();
        let mut work = Work::new(c)?;
        let output = work.reserve(VIEW_BYTES)?;
        self.verify_original(&mut work)?;
        let shell = &self.shell;
        self.semantic
            .cancel(seal, &work.rest()?, |b, c| shell.native(b, c))?;
        self.view(&work.narrow(&output)?)
    }
}
#[cfg(test)]
mod tests;
