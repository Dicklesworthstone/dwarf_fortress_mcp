#![forbid(unsafe_code)]

pub mod agent;
pub mod clock;
mod digest;
mod error;
mod ids;
pub mod lease;
mod model;
pub mod roles;

pub use agent::{
    AgentPhase, AgentTurnState, Affordance, Confidence, Continuity, ContinuityStatus, CostEstimate,
    CoverageDomain, CoverageReport, CoverageStatus, EpistemicClaim, EpistemicState, FortressTool,
    HandoffPacket, MemoryRecord, MemoryStatus, MemoryStratum, ObjectiveSpec, ObjectiveStatus,
    ObservationProfile, Recommendation, RecommendationKind, RecoveryClass, RejectedDecision,
    SemanticInvocation, SurpriseKind, SurpriseRecord, CONFIDENCE_PARTS_PER_MILLION,
    MAX_AGENT_COLLECTION_ITEMS, MAX_AGENT_DETAIL_BYTES, MAX_AGENT_EVIDENCE_REFS,
    MAX_AGENT_SUMMARY_BYTES, MAX_AGENT_TOKEN_BYTES, MAX_HANDOFF_REJECTIONS,
    MAX_OBJECTIVE_CHILDREN,
};
pub use clock::{ClockGovernor, ClockPolicy};
pub use digest::{Digest32, sha256};
pub use error::{DfmcpError, ErrorCode, Result};
pub use ids::{
    ActionId, AttentionId, CheckpointId, EdgeId, EntityId, EventId, EvidenceId, FortressId,
    HandoffId, IntentId, LeaseId, MemoryId, ObjectiveId, PlanId, RecommendationId, RequestId,
    SessionId, StepId, SurpriseId,
};
pub use lease::{LeaseKind, LeaseManager, LeaseRecord, cuboids_intersect};
pub use model::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, Evidence, EvidenceKind, GameTick,
    MapCoord, MapCuboid, ObservationCursor, OperationContext, OperationOutcome, RiskTier,
    StateAnchor, WorkBudget,
};
pub use roles::{DelegationToken, RoleManager, SwarmRole};
