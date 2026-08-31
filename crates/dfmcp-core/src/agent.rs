//! Typed semantic vocabulary for agent orientation, objectives, affordances,
//! recommendations, surprise, memory, and handoff.
//!
//! These types deliberately live below MCP and JSON. They define meaning that
//! adapters, ledgers, planners, replay, and alternative transports can share.
//! None of the cognition artifacts in this module confer authority: an
//! affordance or recommendation may propose an invocation, but only the normal
//! capability, witness, lease/fence, plan-sealing, and commit protocol can
//! authorize an effect.

use std::collections::{BTreeMap, BTreeSet};

use crate::{
    ActionId, AttentionId, Capability, CheckpointId, DfmcpError, Digest32, EntityId, ErrorCode,
    EvidenceId, FortressId, GameTick, HandoffId, MapCuboid, MemoryId, ObjectiveId, PlanId,
    RecommendationId, Result, RiskTier, SessionId, StateAnchor, SurpriseId,
};

pub const MAX_AGENT_TOKEN_BYTES: usize = 256;
pub const MAX_AGENT_SUMMARY_BYTES: usize = 4_096;
pub const MAX_AGENT_DETAIL_BYTES: usize = 16_384;
pub const MAX_AGENT_COLLECTION_ITEMS: usize = 1_024;
pub const MAX_AGENT_EVIDENCE_REFS: usize = 1_024;
pub const MAX_OBJECTIVE_CHILDREN: usize = 256;
pub const MAX_HANDOFF_REJECTIONS: usize = 256;
pub const CONFIDENCE_PARTS_PER_MILLION: u32 = 1_000_000;

fn validate_bounded_text(value: &str, field: &str, max_bytes: usize) -> Result<()> {
    if value.is_empty() {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            format!("{field} must not be empty"),
        ));
    }
    if value.len() > max_bytes {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            format!("{field} exceeds its {max_bytes}-byte bound"),
        ));
    }
    Ok(())
}

fn validate_collection_len(len: usize, field: &str, max: usize) -> Result<()> {
    if len > max {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            format!("{field} contains {len} items, exceeding its bound of {max}"),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AgentPhase {
    Bootstrap,
    Orient,
    Inspect,
    Formulate,
    Propose,
    Compare,
    Commit,
    Verify,
    Learn,
    Handoff,
    Reconcile,
}

impl AgentPhase {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Orient => "orient",
            Self::Inspect => "inspect",
            Self::Formulate => "formulate",
            Self::Propose => "propose",
            Self::Compare => "compare",
            Self::Commit => "commit",
            Self::Verify => "verify",
            Self::Learn => "learn",
            Self::Handoff => "handoff",
            Self::Reconcile => "reconcile",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ContinuityStatus {
    Bootstrap,
    Continuous,
    Heartbeat,
    Partial,
    Gap,
    Reset,
    Stale,
    Indeterminate,
}

impl ContinuityStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bootstrap => "bootstrap",
            Self::Continuous => "continuous",
            Self::Heartbeat => "heartbeat",
            Self::Partial => "partial",
            Self::Gap => "gap",
            Self::Reset => "reset",
            Self::Stale => "stale",
            Self::Indeterminate => "indeterminate",
        }
    }

    #[must_use]
    pub const fn permits_delta_application(self) -> bool {
        matches!(self, Self::Continuous | Self::Heartbeat | Self::Partial)
    }

    #[must_use]
    pub const fn requires_full_refresh(self) -> bool {
        matches!(self, Self::Gap | Self::Reset | Self::Indeterminate)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObservationProfile {
    Pulse,
    Briefing,
    Tactical,
    Forensic,
    Custom,
}

impl ObservationProfile {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pulse => "pulse",
            Self::Briefing => "briefing",
            Self::Tactical => "tactical",
            Self::Forensic => "forensic",
            Self::Custom => "custom",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EpistemicState {
    Observed,
    CertifiedDerived,
    Inferred,
    Predicted,
    Assumed,
    Stale,
    Unknown,
    Contradicted,
    Indeterminate,
}

impl EpistemicState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::CertifiedDerived => "certified_derived",
            Self::Inferred => "inferred",
            Self::Predicted => "predicted",
            Self::Assumed => "assumed",
            Self::Stale => "stale",
            Self::Unknown => "unknown",
            Self::Contradicted => "contradicted",
            Self::Indeterminate => "indeterminate",
        }
    }

    #[must_use]
    pub const fn may_satisfy_mutation_precondition(self) -> bool {
        matches!(self, Self::Observed | Self::CertifiedDerived)
    }

    #[must_use]
    pub const fn is_authoritative_fact(self) -> bool {
        matches!(self, Self::Observed | Self::CertifiedDerived)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecoveryClass {
    NeverUnchanged,
    SafeReadRetry,
    RefreshAndRetry,
    RebaseRequired,
    Backoff,
    ReconciliationRequired,
    ConfirmationRequired,
    OperatorActionRequired,
}

impl RecoveryClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NeverUnchanged => "never_unchanged",
            Self::SafeReadRetry => "safe_read_retry",
            Self::RefreshAndRetry => "refresh_and_retry",
            Self::RebaseRequired => "rebase_required",
            Self::Backoff => "backoff",
            Self::ReconciliationRequired => "reconciliation_required",
            Self::ConfirmationRequired => "confirmation_required",
            Self::OperatorActionRequired => "operator_action_required",
        }
    }

    #[must_use]
    pub const fn permits_unchanged_retry(self) -> bool {
        matches!(self, Self::SafeReadRetry | Self::Backoff)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FortressTool {
    OpenSession,
    Observe,
    Query,
    Plan,
    Commit,
    Wait,
    Cancel,
    Checkpoint,
    Restore,
    Explain,
    Doctor,
}

impl FortressTool {
    pub const ALL: [Self; 11] = [
        Self::OpenSession,
        Self::Observe,
        Self::Query,
        Self::Plan,
        Self::Commit,
        Self::Wait,
        Self::Cancel,
        Self::Checkpoint,
        Self::Restore,
        Self::Explain,
        Self::Doctor,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OpenSession => "fortress.open_session",
            Self::Observe => "fortress.observe",
            Self::Query => "fortress.query",
            Self::Plan => "fortress.plan",
            Self::Commit => "fortress.commit",
            Self::Wait => "fortress.wait",
            Self::Cancel => "fortress.cancel",
            Self::Checkpoint => "fortress.checkpoint",
            Self::Restore => "fortress.restore",
            Self::Explain => "fortress.explain",
            Self::Doctor => "fortress.doctor",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Confidence {
    pub parts_per_million: Option<u32>,
    pub epistemic_state: EpistemicState,
    pub evidence: BTreeSet<EvidenceId>,
}

impl Confidence {
    pub fn validate(&self) -> Result<()> {
        if self
            .parts_per_million
            .is_some_and(|value| value > CONFIDENCE_PARTS_PER_MILLION)
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "confidence exceeds one million parts per million",
            ));
        }
        validate_collection_len(
            self.evidence.len(),
            "confidence evidence",
            MAX_AGENT_EVIDENCE_REFS,
        )
    }

    pub fn validate_for_mutation_precondition(&self) -> Result<()> {
        self.validate()?;
        if !self.epistemic_state.may_satisfy_mutation_precondition() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                format!(
                    "epistemic state {} cannot satisfy a mutation precondition",
                    self.epistemic_state.as_str()
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpistemicClaim<T> {
    pub value: Option<T>,
    pub confidence: Confidence,
    pub anchor: Option<StateAnchor>,
    pub statement: String,
}

impl<T> EpistemicClaim<T> {
    pub fn validate(&self) -> Result<()> {
        validate_bounded_text(
            &self.statement,
            "epistemic claim statement",
            MAX_AGENT_DETAIL_BYTES,
        )?;
        self.confidence.validate()
    }

    #[must_use]
    pub fn can_satisfy_mutation_precondition(&self) -> bool {
        self.value.is_some()
            && self.anchor.is_some()
            && self.confidence.epistemic_state.may_satisfy_mutation_precondition()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Continuity {
    pub status: ContinuityStatus,
    pub basis: Option<StateAnchor>,
    pub target: Option<StateAnchor>,
    pub first_missing_sequence: Option<u64>,
    pub reset_reason: Option<String>,
}

impl Continuity {
    pub fn validate(&self) -> Result<()> {
        if let Some(reason) = self.reset_reason.as_ref() {
            validate_bounded_text(reason, "continuity reset reason", MAX_AGENT_DETAIL_BYTES)?;
        }
        match self.status {
            ContinuityStatus::Bootstrap => {
                if self.target.is_none() {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "bootstrap continuity requires a target anchor",
                    ));
                }
            }
            ContinuityStatus::Continuous
            | ContinuityStatus::Heartbeat
            | ContinuityStatus::Partial => {
                let Some(basis) = self.basis else {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "continuous continuity requires a basis anchor",
                    ));
                };
                let Some(target) = self.target else {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "continuous continuity requires a target anchor",
                    ));
                };
                if basis.fortress_id != target.fortress_id
                    || basis.cursor.epoch != target.cursor.epoch
                    || basis.cursor.sequence > target.cursor.sequence
                {
                    return Err(DfmcpError::new(
                        ErrorCode::CursorGap,
                        "continuity basis and target do not form a monotonic same-epoch chain",
                    ));
                }
                if self.status == ContinuityStatus::Heartbeat && basis != target {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "heartbeat continuity requires identical basis and target anchors",
                    ));
                }
            }
            ContinuityStatus::Gap => {
                if self.basis.is_none()
                    || self.target.is_none()
                    || self.first_missing_sequence.is_none()
                {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "gap continuity requires basis, target, and first missing sequence",
                    ));
                }
            }
            ContinuityStatus::Reset => {
                let Some(basis) = self.basis else {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "reset continuity requires a prior basis anchor",
                    ));
                };
                let Some(target) = self.target else {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "reset continuity requires a target anchor",
                    ));
                };
                if basis.fortress_id != target.fortress_id
                    || basis.cursor.epoch >= target.cursor.epoch
                    || self.reset_reason.is_none()
                {
                    return Err(DfmcpError::new(
                        ErrorCode::InvalidRequest,
                        "reset continuity requires a later epoch and an explicit reason",
                    ));
                }
            }
            ContinuityStatus::Stale | ContinuityStatus::Indeterminate => {}
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CoverageStatus {
    Complete,
    Partial,
    Omitted,
    Unknown,
}

impl CoverageStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Partial => "partial",
            Self::Omitted => "omitted",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub const fn can_prove_absence(self) -> bool {
        matches!(self, Self::Complete)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageDomain {
    pub domain: String,
    pub status: CoverageStatus,
    pub reason: Option<String>,
    pub evidence: BTreeSet<EvidenceId>,
}

impl CoverageDomain {
    pub fn validate(&self) -> Result<()> {
        validate_bounded_text(&self.domain, "coverage domain", MAX_AGENT_TOKEN_BYTES)?;
        if let Some(reason) = self.reason.as_ref() {
            validate_bounded_text(reason, "coverage reason", MAX_AGENT_DETAIL_BYTES)?;
        }
        if matches!(self.status, CoverageStatus::Partial | CoverageStatus::Omitted)
            && self.reason.is_none()
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "partial or omitted coverage requires an explicit reason",
            ));
        }
        validate_collection_len(
            self.evidence.len(),
            "coverage evidence",
            MAX_AGENT_EVIDENCE_REFS,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CoverageReport {
    pub anchor: Option<StateAnchor>,
    pub domains: BTreeMap<String, CoverageDomain>,
    pub continuation: Option<String>,
}

impl CoverageReport {
    pub fn validate(&self) -> Result<()> {
        validate_collection_len(
            self.domains.len(),
            "coverage domains",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        if let Some(continuation) = self.continuation.as_ref() {
            validate_bounded_text(
                continuation,
                "coverage continuation",
                MAX_AGENT_DETAIL_BYTES,
            )?;
            if self.anchor.is_none() {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "a continuation must be bound to an anchor",
                ));
            }
        }
        for (key, domain) in &self.domains {
            if key != &domain.domain {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "coverage map key does not match domain identity",
                ));
            }
            domain.validate()?;
        }
        Ok(())
    }

    #[must_use]
    pub fn proves_absence_in(&self, domain: &str) -> bool {
        self.domains
            .get(domain)
            .is_some_and(|entry| entry.status.can_prove_absence())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CostEstimate {
    pub output_tokens: Option<u64>,
    pub output_bytes: Option<u64>,
    pub bridge_bytes: Option<u64>,
    pub wall_millis: Option<u64>,
    pub game_ticks: Option<u64>,
    pub actions: Option<u32>,
    pub confidence: Option<Confidence>,
}

impl CostEstimate {
    pub fn validate(&self) -> Result<()> {
        if let Some(confidence) = self.confidence.as_ref() {
            confidence.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticInvocation {
    pub tool: FortressTool,
    pub family: String,
    pub arguments_digest: Digest32,
    pub argument_summary: String,
}

impl SemanticInvocation {
    pub fn validate(&self) -> Result<()> {
        validate_bounded_text(&self.family, "invocation family", MAX_AGENT_TOKEN_BYTES)?;
        validate_bounded_text(
            &self.argument_summary,
            "invocation argument summary",
            MAX_AGENT_SUMMARY_BYTES,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Affordance {
    pub id: RecommendationId,
    pub invocation: SemanticInvocation,
    pub capability: Capability,
    pub risk: RiskTier,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
    pub entity_scope: BTreeSet<EntityId>,
    pub map_scope: Option<MapCuboid>,
    pub known_preconditions: BTreeSet<Digest32>,
    pub unverified_preconditions: BTreeSet<Digest32>,
    pub checkpoint_required: bool,
    pub confirmation_required: bool,
    pub reversible: bool,
    pub estimated_cost: CostEstimate,
    pub confidence: Confidence,
}

impl Affordance {
    pub fn validate(&self) -> Result<()> {
        if self.id == RecommendationId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "affordance identity must not be zero",
            ));
        }
        self.invocation.validate()?;
        self.estimated_cost.validate()?;
        self.confidence.validate()?;
        validate_collection_len(
            self.entity_scope.len(),
            "affordance entity scope",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.known_preconditions.len(),
            "affordance known preconditions",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.unverified_preconditions.len(),
            "affordance unverified preconditions",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        if self.enabled && self.disabled_reason.is_some() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "an enabled affordance cannot carry a disabled reason",
            ));
        }
        if !self.enabled && self.disabled_reason.is_none() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "a disabled affordance requires an explicit reason",
            ));
        }
        if let Some(reason) = self.disabled_reason.as_ref() {
            validate_bounded_text(reason, "affordance disabled reason", MAX_AGENT_DETAIL_BYTES)?;
        }
        Ok(())
    }

    #[must_use]
    pub const fn can_dispatch_effect(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecommendationKind {
    Informational,
    Deliberative,
    Mutating,
    Wait,
    Reconcile,
    Confirm,
    OperatorAction,
}

impl RecommendationKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Informational => "informational",
            Self::Deliberative => "deliberative",
            Self::Mutating => "mutating",
            Self::Wait => "wait",
            Self::Reconcile => "reconcile",
            Self::Confirm => "confirm",
            Self::OperatorAction => "operator_action",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recommendation {
    pub id: RecommendationId,
    pub kind: RecommendationKind,
    pub invocation: Option<SemanticInvocation>,
    pub reason: String,
    pub expected_utility_micros: Option<i64>,
    pub expected_information_value_micros: Option<i64>,
    pub risk: RiskTier,
    pub reversible: bool,
    pub estimated_cost: CostEstimate,
    pub prerequisites: BTreeSet<Digest32>,
    pub invalidating_conditions: BTreeSet<Digest32>,
    pub confidence: Confidence,
    pub confirmation_required: bool,
}

impl Recommendation {
    pub fn validate(&self) -> Result<()> {
        if self.id == RecommendationId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "recommendation identity must not be zero",
            ));
        }
        validate_bounded_text(&self.reason, "recommendation reason", MAX_AGENT_DETAIL_BYTES)?;
        if let Some(invocation) = self.invocation.as_ref() {
            invocation.validate()?;
        }
        self.estimated_cost.validate()?;
        self.confidence.validate()?;
        validate_collection_len(
            self.prerequisites.len(),
            "recommendation prerequisites",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.invalidating_conditions.len(),
            "recommendation invalidating conditions",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        if matches!(self.kind, RecommendationKind::Mutating) && self.invocation.is_none() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "a mutating recommendation requires a typed semantic invocation",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn can_dispatch_effect(&self) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ObjectiveStatus {
    Proposed,
    Active,
    Suspended,
    Satisfied,
    Failed,
    Abandoned,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjectiveSpec<P> {
    pub id: ObjectiveId,
    pub parent: Option<ObjectiveId>,
    pub summary: String,
    pub terminal_predicates: Vec<P>,
    pub forbidden_predicates: Vec<P>,
    pub priority: u32,
    pub urgency: u32,
    pub horizon_end_tick: Option<GameTick>,
    pub review_interval_ticks: Option<u64>,
    pub max_risk: RiskTier,
    pub max_cost: CostEstimate,
    pub owner_session: SessionId,
    pub child_objectives: BTreeSet<ObjectiveId>,
    pub status: ObjectiveStatus,
    pub evidence_required: bool,
}

impl<P> ObjectiveSpec<P> {
    pub fn validate(&self) -> Result<()> {
        if self.id == ObjectiveId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "objective identity must not be zero",
            ));
        }
        if self.owner_session == SessionId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "objective owner session must not be zero",
            ));
        }
        validate_bounded_text(&self.summary, "objective summary", MAX_AGENT_SUMMARY_BYTES)?;
        validate_collection_len(
            self.terminal_predicates.len(),
            "objective terminal predicates",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.forbidden_predicates.len(),
            "objective forbidden predicates",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.child_objectives.len(),
            "objective children",
            MAX_OBJECTIVE_CHILDREN,
        )?;
        if self.parent == Some(self.id) || self.child_objectives.contains(&self.id) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "an objective cannot be its own parent or child",
            ));
        }
        if self.review_interval_ticks == Some(0) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "objective review interval must be positive when present",
            ));
        }
        self.max_cost.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SurpriseKind {
    ExpectedEffectMissing,
    UnexpectedEffectObserved,
    DurationExceeded,
    ResourceCostExceeded,
    PreconditionChanged,
    CompensationDiverged,
    RecommendationWouldChange,
}

impl SurpriseKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ExpectedEffectMissing => "expected_effect_missing",
            Self::UnexpectedEffectObserved => "unexpected_effect_observed",
            Self::DurationExceeded => "duration_exceeded",
            Self::ResourceCostExceeded => "resource_cost_exceeded",
            Self::PreconditionChanged => "precondition_changed",
            Self::CompensationDiverged => "compensation_diverged",
            Self::RecommendationWouldChange => "recommendation_would_change",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurpriseRecord {
    pub id: SurpriseId,
    pub kind: SurpriseKind,
    pub objective_id: Option<ObjectiveId>,
    pub plan_id: Option<PlanId>,
    pub action_id: Option<ActionId>,
    pub predicted_anchor: Option<StateAnchor>,
    pub observed_anchor: StateAnchor,
    pub predicted_digest: Digest32,
    pub observed_digest: Digest32,
    pub materiality_micros: u64,
    pub summary: String,
    pub evidence: BTreeSet<EvidenceId>,
}

impl SurpriseRecord {
    pub fn validate(&self) -> Result<()> {
        if self.id == SurpriseId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "surprise identity must not be zero",
            ));
        }
        validate_bounded_text(&self.summary, "surprise summary", MAX_AGENT_DETAIL_BYTES)?;
        validate_collection_len(
            self.evidence.len(),
            "surprise evidence",
            MAX_AGENT_EVIDENCE_REFS,
        )?;
        if self.predicted_digest == self.observed_digest && self.materiality_micros > 0 {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "a materially divergent surprise requires different predicted and observed digests",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryStratum {
    Episodic,
    Semantic,
    Procedural,
    Policy,
    Negative,
}

impl MemoryStratum {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Episodic => "episodic",
            Self::Semantic => "semantic",
            Self::Procedural => "procedural",
            Self::Policy => "policy",
            Self::Negative => "negative",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MemoryStatus {
    RawEpisode,
    LessonCandidate,
    ShadowEvaluated,
    Admitted,
    Suspended,
    Refuted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryRecord {
    pub id: MemoryId,
    pub stratum: MemoryStratum,
    pub status: MemoryStatus,
    pub summary: String,
    pub applicability: String,
    pub source_anchors: BTreeSet<StateAnchor>,
    pub evidence: BTreeSet<EvidenceId>,
    pub supporting_surprises: BTreeSet<SurpriseId>,
    pub contradictory_evidence: BTreeSet<EvidenceId>,
    pub policy_epoch: Option<u64>,
    pub rollback_digest: Option<Digest32>,
}

impl MemoryRecord {
    pub fn validate(&self) -> Result<()> {
        if self.id == MemoryId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "memory identity must not be zero",
            ));
        }
        validate_bounded_text(&self.summary, "memory summary", MAX_AGENT_SUMMARY_BYTES)?;
        validate_bounded_text(
            &self.applicability,
            "memory applicability",
            MAX_AGENT_DETAIL_BYTES,
        )?;
        validate_collection_len(
            self.source_anchors.len(),
            "memory source anchors",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.evidence.len(),
            "memory evidence",
            MAX_AGENT_EVIDENCE_REFS,
        )?;
        validate_collection_len(
            self.supporting_surprises.len(),
            "memory supporting surprises",
            MAX_AGENT_COLLECTION_ITEMS,
        )?;
        validate_collection_len(
            self.contradictory_evidence.len(),
            "memory contradictory evidence",
            MAX_AGENT_EVIDENCE_REFS,
        )?;
        if matches!(self.status, MemoryStatus::Admitted)
            && (self.source_anchors.is_empty() || self.evidence.is_empty())
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "admitted memory requires source anchors and evidence",
            ));
        }
        if matches!(self.stratum, MemoryStratum::Policy)
            && matches!(self.status, MemoryStatus::Admitted)
            && (self.policy_epoch.is_none() || self.rollback_digest.is_none())
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "admitted policy memory requires a policy epoch and rollback digest",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub const fn can_grant_authority(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn can_satisfy_live_precondition(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedDecision {
    pub decision_digest: Digest32,
    pub reason: String,
    pub anchor: StateAnchor,
    pub evidence: BTreeSet<EvidenceId>,
}

impl RejectedDecision {
    pub fn validate(&self) -> Result<()> {
        validate_bounded_text(
            &self.reason,
            "rejected decision reason",
            MAX_AGENT_DETAIL_BYTES,
        )?;
        validate_collection_len(
            self.evidence.len(),
            "rejected decision evidence",
            MAX_AGENT_EVIDENCE_REFS,
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HandoffPacket {
    pub id: HandoffId,
    pub fortress_id: FortressId,
    pub session_id: SessionId,
    pub anchor: StateAnchor,
    pub objective_ids: BTreeSet<ObjectiveId>,
    pub pending_plan_ids: BTreeSet<PlanId>,
    pub active_action_ids: BTreeSet<ActionId>,
    pub checkpoint_ids: BTreeSet<CheckpointId>,
    pub unresolved_attention: BTreeSet<AttentionId>,
    pub unresolved_surprises: BTreeSet<SurpriseId>,
    pub memory_refs: BTreeSet<MemoryId>,
    pub rejected_decisions: Vec<RejectedDecision>,
    pub capability_digest: Digest32,
    pub budget_digest: Digest32,
    pub minimum_safe_next_tool: Option<FortressTool>,
    pub summary: String,
}

impl HandoffPacket {
    pub fn validate(&self) -> Result<()> {
        if self.id == HandoffId::NIL
            || self.fortress_id == FortressId::NIL
            || self.session_id == SessionId::NIL
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "handoff, fortress, and session identities must not be zero",
            ));
        }
        if self.anchor.fortress_id != self.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "handoff anchor belongs to a different fortress",
            ));
        }
        validate_bounded_text(&self.summary, "handoff summary", MAX_AGENT_SUMMARY_BYTES)?;
        for (name, len) in [
            ("handoff objectives", self.objective_ids.len()),
            ("handoff pending plans", self.pending_plan_ids.len()),
            ("handoff active actions", self.active_action_ids.len()),
            ("handoff checkpoints", self.checkpoint_ids.len()),
            ("handoff attention", self.unresolved_attention.len()),
            ("handoff surprises", self.unresolved_surprises.len()),
            ("handoff memory references", self.memory_refs.len()),
        ] {
            validate_collection_len(len, name, MAX_AGENT_COLLECTION_ITEMS)?;
        }
        validate_collection_len(
            self.rejected_decisions.len(),
            "handoff rejected decisions",
            MAX_HANDOFF_REJECTIONS,
        )?;
        for rejected in &self.rejected_decisions {
            rejected.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTurnState {
    pub phase: AgentPhase,
    pub session_id: SessionId,
    pub anchor: StateAnchor,
    pub continuity: Continuity,
    pub profile: ObservationProfile,
    pub active_plan_ids: BTreeSet<PlanId>,
    pub active_action_ids: BTreeSet<ActionId>,
    pub attention_ids: BTreeSet<AttentionId>,
    pub affordances: Vec<Affordance>,
    pub recommendations: Vec<Recommendation>,
    pub uncertainty_claims: Vec<EpistemicClaim<String>>,
    pub coverage: CoverageReport,
    pub handoff: Option<HandoffId>,
}

impl AgentTurnState {
    pub fn validate(&self) -> Result<()> {
        if self.session_id == SessionId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "agent turn session identity must not be zero",
            ));
        }
        self.continuity.validate()?;
        if self.continuity.target != Some(self.anchor) {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "agent turn anchor must equal the continuity target",
            ));
        }
        if self.coverage.anchor != Some(self.anchor) {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "agent turn coverage must bind the same anchor",
            ));
        }
        for (name, len) in [
            ("agent turn plans", self.active_plan_ids.len()),
            ("agent turn actions", self.active_action_ids.len()),
            ("agent turn attention", self.attention_ids.len()),
            ("agent turn affordances", self.affordances.len()),
            ("agent turn recommendations", self.recommendations.len()),
            ("agent turn uncertainty", self.uncertainty_claims.len()),
        ] {
            validate_collection_len(len, name, MAX_AGENT_COLLECTION_ITEMS)?;
        }
        for affordance in &self.affordances {
            affordance.validate()?;
        }
        for recommendation in &self.recommendations {
            recommendation.validate()?;
        }
        for claim in &self.uncertainty_claims {
            claim.validate()?;
        }
        self.coverage.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ObservationCursor, sha256};

    fn anchor(epoch: u64, sequence: u64) -> StateAnchor {
        StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor { epoch, sequence },
            tick: GameTick::new(sequence),
            state_hash: sha256(&[epoch as u8, sequence as u8]),
        }
    }

    #[test]
    fn only_authoritative_epistemic_states_can_satisfy_mutation_preconditions() {
        assert!(EpistemicState::Observed.may_satisfy_mutation_precondition());
        assert!(EpistemicState::CertifiedDerived.may_satisfy_mutation_precondition());
        for state in [
            EpistemicState::Inferred,
            EpistemicState::Predicted,
            EpistemicState::Assumed,
            EpistemicState::Stale,
            EpistemicState::Unknown,
            EpistemicState::Contradicted,
            EpistemicState::Indeterminate,
        ] {
            assert!(!state.may_satisfy_mutation_precondition());
        }
    }

    #[test]
    fn heartbeat_requires_identical_basis_and_target() {
        let invalid = Continuity {
            status: ContinuityStatus::Heartbeat,
            basis: Some(anchor(1, 4)),
            target: Some(anchor(1, 5)),
            first_missing_sequence: None,
            reset_reason: None,
        };
        assert!(invalid.validate().is_err());

        let stable = anchor(1, 4);
        let valid = Continuity {
            status: ContinuityStatus::Heartbeat,
            basis: Some(stable),
            target: Some(stable),
            first_missing_sequence: None,
            reset_reason: None,
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn reset_requires_a_later_epoch_and_explicit_reason() {
        let valid = Continuity {
            status: ContinuityStatus::Reset,
            basis: Some(anchor(1, 99)),
            target: Some(anchor(2, 0)),
            first_missing_sequence: None,
            reset_reason: Some("checkpoint restore".to_owned()),
        };
        assert!(valid.validate().is_ok());
    }

    #[test]
    fn absence_requires_complete_domain_coverage() {
        let mut domains = BTreeMap::new();
        domains.insert(
            "units".to_owned(),
            CoverageDomain {
                domain: "units".to_owned(),
                status: CoverageStatus::Partial,
                reason: Some("token budget".to_owned()),
                evidence: BTreeSet::new(),
            },
        );
        let report = CoverageReport {
            anchor: Some(anchor(1, 1)),
            domains,
            continuation: Some("opaque".to_owned()),
        };
        assert!(report.validate().is_ok());
        assert!(!report.proves_absence_in("units"));
    }

    #[test]
    fn cognition_artifacts_never_dispatch_or_grant_authority() {
        let confidence = Confidence {
            parts_per_million: Some(CONFIDENCE_PARTS_PER_MILLION),
            epistemic_state: EpistemicState::CertifiedDerived,
            evidence: BTreeSet::new(),
        };
        let affordance = Affordance {
            id: RecommendationId::new(1),
            invocation: SemanticInvocation {
                tool: FortressTool::Plan,
                family: "control_clock".to_owned(),
                arguments_digest: sha256(b"pause=false"),
                argument_summary: "resume the simulation".to_owned(),
            },
            capability: Capability::ControlClock,
            risk: RiskTier::Reversible,
            enabled: true,
            disabled_reason: None,
            entity_scope: BTreeSet::new(),
            map_scope: None,
            known_preconditions: BTreeSet::new(),
            unverified_preconditions: BTreeSet::new(),
            checkpoint_required: false,
            confirmation_required: false,
            reversible: true,
            estimated_cost: CostEstimate::default(),
            confidence,
        };
        assert!(affordance.validate().is_ok());
        assert!(!affordance.can_dispatch_effect());
    }

    #[test]
    fn admitted_policy_memory_requires_rollback() {
        let memory = MemoryRecord {
            id: MemoryId::new(1),
            stratum: MemoryStratum::Policy,
            status: MemoryStatus::Admitted,
            summary: "prefer pulse observations during stable work".to_owned(),
            applicability: "only for compatible pause-state laboratory sessions".to_owned(),
            source_anchors: BTreeSet::from([anchor(1, 1)]),
            evidence: BTreeSet::from([EvidenceId::new(1)]),
            supporting_surprises: BTreeSet::new(),
            contradictory_evidence: BTreeSet::new(),
            policy_epoch: Some(3),
            rollback_digest: None,
        };
        assert!(memory.validate().is_err());
        assert!(!memory.can_grant_authority());
        assert!(!memory.can_satisfy_live_precondition());
    }
}
