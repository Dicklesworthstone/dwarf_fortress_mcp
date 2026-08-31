#![forbid(unsafe_code)]

//! Semantic Rebase Conflict Resolution and Deterministic Certificate Generation.
//!
//! WP-WOR-04: Resolves optimistic concurrency conflicts when rebasing prepared plans
//! across world state epochs, emitting cryptographic certificates of conflict.

use dfmcp_core::{Digest32, PlanId, StateAnchor};

/// Classification of semantic rebase conflicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    AnchorDivergence,
    PreconditionViolated { description: String },
    SpatialOverlap,
    EntityUnavailable,
    ResourceDepleted,
}

/// Cryptographic certificate proving why a rebase failed or was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictCertificate {
    pub plan_id: PlanId,
    pub base_anchor: StateAnchor,
    pub target_anchor: StateAnchor,
    pub conflict_kind: ConflictKind,
    pub diagnosis: String,
    pub certificate_digest: Digest32,
}

impl ConflictCertificate {
    #[must_use]
    pub fn new(
        plan_id: PlanId,
        base_anchor: StateAnchor,
        target_anchor: StateAnchor,
        conflict_kind: ConflictKind,
        diagnosis: String,
    ) -> Self {
        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(&plan_id.get().to_be_bytes());
        hasher_bytes.extend_from_slice(base_anchor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(target_anchor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(diagnosis.as_bytes());
        let certificate_digest = Digest32::of_bytes(&hasher_bytes);

        Self {
            plan_id,
            base_anchor,
            target_anchor,
            conflict_kind,
            diagnosis,
            certificate_digest,
        }
    }
}
