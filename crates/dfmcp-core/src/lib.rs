#![forbid(unsafe_code)]

pub mod clock;
mod digest;
mod error;
mod ids;
pub mod lease;
mod model;
pub mod roles;

pub use clock::{ClockGovernor, ClockPolicy};
pub use digest::{Digest32, sha256};
pub use error::{DfmcpError, ErrorCode, Result};
pub use ids::{
    ActionId, CheckpointId, EdgeId, EntityId, EventId, EvidenceId, FortressId, IntentId, LeaseId,
    PlanId, RequestId, SessionId, StepId,
};
pub use lease::{LeaseKind, LeaseManager, LeaseRecord, cuboids_intersect};
pub use model::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, Evidence, EvidenceKind, GameTick,
    MapCoord, MapCuboid, ObservationCursor, OperationContext, OperationOutcome, RiskTier,
    StateAnchor, WorkBudget,
};
pub use roles::{DelegationToken, RoleManager, SwarmRole};
