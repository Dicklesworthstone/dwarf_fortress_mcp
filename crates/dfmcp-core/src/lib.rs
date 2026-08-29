#![forbid(unsafe_code)]

mod digest;
mod error;
mod ids;
mod model;

pub use digest::{sha256, Digest32};
pub use error::{DfmcpError, ErrorCode, Result};
pub use ids::{
    ActionId, CheckpointId, EdgeId, EntityId, EventId, EvidenceId, FortressId, IntentId, LeaseId, PlanId, RequestId,
    SessionId, StepId,
};
pub use model::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, Evidence, EvidenceKind, GameTick,
    MapCoord, MapCuboid, ObservationCursor, OperationContext, OperationOutcome, RiskTier,
    StateAnchor, WorkBudget,
};
