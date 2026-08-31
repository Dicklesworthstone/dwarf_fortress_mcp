#![forbid(unsafe_code)]

//! Autonomous Trust Protocol (ATP) Proof Capsule Distribution and Verification.
//!
//! WP-WLD-03: Packages state transitions, diffs, and cryptographic Merkle state proofs
//! into content-addressed capsules for multi-agent swarm verification (INV-005).

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, GameTick, Result, StateAnchor};

use crate::delta::StateDelta;
use crate::merkle::MerkleStateTree;
use crate::model::WorldSnapshot;

/// Sealed, content-addressed ATP proof capsule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtpProofCapsule {
    pub basis_anchor: StateAnchor,
    pub successor_anchor: StateAnchor,
    pub merkle_root: Digest32,
    pub delta: StateDelta,
    pub capsule_digest: Digest32,
    pub published_at_tick: GameTick,
}

impl AtpProofCapsule {
    /// Seal a new ATP proof capsule from basis and successor snapshots and their delta.
    pub fn seal(
        basis_snapshot: &WorldSnapshot,
        successor_snapshot: &WorldSnapshot,
        delta: StateDelta,
        published_at_tick: GameTick,
    ) -> Result<Self> {
        let tree = MerkleStateTree::from_snapshot(successor_snapshot);
        let merkle_root = tree.overall_root;

        let basis_anchor = basis_snapshot.anchor();
        let successor_anchor = successor_snapshot.anchor();

        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(basis_anchor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(successor_anchor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(merkle_root.as_bytes());
        hasher_bytes.extend_from_slice(delta.target_hash.as_bytes());

        let capsule_digest = Digest32::of_bytes(&hasher_bytes);

        Ok(Self {
            basis_anchor,
            successor_anchor,
            merkle_root,
            delta,
            capsule_digest,
            published_at_tick,
        })
    }

    /// Verify transition proof integrity.
    #[must_use]
    pub fn verify_transition(&self) -> bool {
        let mut hasher_bytes = Vec::new();
        hasher_bytes.extend_from_slice(self.basis_anchor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(self.successor_anchor.state_hash.as_bytes());
        hasher_bytes.extend_from_slice(self.merkle_root.as_bytes());
        hasher_bytes.extend_from_slice(self.delta.target_hash.as_bytes());

        let expected = Digest32::of_bytes(&hasher_bytes);
        expected == self.capsule_digest
    }
}

/// Standalone verifier for ATP state transition proofs.
#[derive(Clone, Debug, Default)]
pub struct AtpProofVerifier;

impl AtpProofVerifier {
    /// Verify an incoming transition proof capsule.
    pub fn verify_capsule(&self, capsule: &AtpProofCapsule) -> Result<()> {
        if !capsule.verify_transition() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "ATP proof capsule digest mismatch: potential bit-rot or tampering detected",
            ));
        }

        if capsule.delta.target_hash != capsule.successor_anchor.state_hash {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "delta target_hash does not match successor anchor state_hash",
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorldGraph;
    use dfmcp_core::{FortressId, ObservationCursor};

    fn sample_snapshot(tick: u64, cursor: ObservationCursor) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            cursor,
            true,
            WorldGraph::default(),
        )
    }

    #[test]
    fn test_atp_proof_capsule_seal_and_verify() -> Result<()> {
        let snap_base = sample_snapshot(100, ObservationCursor::ORIGIN);
        let snap_target = sample_snapshot(
            101,
            ObservationCursor {
                epoch: 0,
                sequence: 1,
            },
        );

        let delta = StateDelta {
            fortress_id: FortressId::new(1),
            base_cursor: snap_base.cursor,
            target_cursor: snap_target.cursor,
            base_hash: snap_base.state_hash,
            target_hash: snap_target.state_hash,
            target_tick: snap_target.tick,
            changes: Vec::new(),
            truncated: false,
            continuation: None,
        };

        let capsule = AtpProofCapsule::seal(&snap_base, &snap_target, delta, GameTick(101))?;
        assert!(capsule.verify_transition());

        let verifier = AtpProofVerifier;
        assert!(verifier.verify_capsule(&capsule).is_ok());

        Ok(())
    }
}
