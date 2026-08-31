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
        let mut bytes = Vec::new();
        crate::canonical::put_str(&mut bytes, "dfmcp-conflict-certificate-v1");
        bytes.extend_from_slice(&plan_id.get().to_be_bytes());
        crate::canonical::put_anchor(&mut bytes, base_anchor);
        crate::canonical::put_anchor(&mut bytes, target_anchor);
        encode_conflict_kind(&mut bytes, &conflict_kind);
        crate::canonical::put_str(&mut bytes, &diagnosis);
        let certificate_digest = Digest32::of_bytes(&bytes);

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

fn encode_conflict_kind(output: &mut Vec<u8>, kind: &ConflictKind) {
    match kind {
        ConflictKind::AnchorDivergence => output.push(0),
        ConflictKind::PreconditionViolated { description } => {
            output.push(1);
            crate::canonical::put_str(output, description);
        }
        ConflictKind::SpatialOverlap => output.push(2),
        ConflictKind::EntityUnavailable => output.push(3),
        ConflictKind::ResourceDepleted => output.push(4),
    }
}
