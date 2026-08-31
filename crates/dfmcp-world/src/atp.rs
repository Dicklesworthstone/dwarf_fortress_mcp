#![forbid(unsafe_code)]

//! Autonomous Trust Protocol (ATP) Proof Capsule Distribution and Verification.
//!
//! WP-WLD-03: Packages state transitions, diffs, and cryptographic Merkle state proofs
//! into content-addressed capsules for multi-agent swarm verification (INV-005).

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, GameTick, Result, StateAnchor};

use crate::delta::{StateDelta, apply_delta};
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
        validate_transition_inputs(basis_snapshot, successor_snapshot, &delta)?;
        if published_at_tick < successor_snapshot.tick {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "ATP capsule publication tick precedes its successor state",
            ));
        }
        let tree = MerkleStateTree::from_snapshot(successor_snapshot);
        let merkle_root = tree.overall_root;

        let basis_anchor = basis_snapshot.anchor();
        let successor_anchor = successor_snapshot.anchor();

        let capsule_digest = capsule_digest(
            basis_anchor,
            successor_anchor,
            merkle_root,
            &delta,
            published_at_tick,
        );

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
        let expected = capsule_digest(
            self.basis_anchor,
            self.successor_anchor,
            self.merkle_root,
            &self.delta,
            self.published_at_tick,
        );
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

        if capsule.delta.base_hash != capsule.basis_anchor.state_hash
            || capsule.delta.fortress_id != capsule.basis_anchor.fortress_id
            || capsule.successor_anchor.fortress_id != capsule.basis_anchor.fortress_id
            || capsule.delta.base_cursor != capsule.basis_anchor.cursor
            || capsule.delta.target_cursor != capsule.successor_anchor.cursor
            || capsule.delta.target_tick != capsule.successor_anchor.tick
        {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "ATP capsule anchor and delta continuity mismatch",
            ));
        }
        if capsule.published_at_tick < capsule.successor_anchor.tick {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "ATP capsule publication tick precedes its successor state",
            ));
        }

        Ok(())
    }

    /// Verify a chain of contiguous transition capsules.
    pub fn verify_chain(&self, chain: &[AtpProofCapsule]) -> Result<()> {
        if chain.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "cannot verify empty ATP proof capsule chain",
            ));
        }

        for (i, capsule) in chain.iter().enumerate() {
            self.verify_capsule(capsule)?;
            if i > 0 {
                let prev = &chain[i - 1];
                if prev.successor_anchor != capsule.basis_anchor {
                    return Err(DfmcpError::new(
                        ErrorCode::CursorGap,
                        format!(
                            "chain gap between capsule {} and {}: prev successor != current basis",
                            i - 1,
                            i
                        ),
                    ));
                }
            }
        }
        Ok(())
    }
}

fn validate_transition_inputs(
    basis: &WorldSnapshot,
    successor: &WorldSnapshot,
    delta: &StateDelta,
) -> Result<()> {
    if !basis.hash_is_valid() || !successor.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "cannot seal ATP proof from a snapshot with an invalid state hash",
        ));
    }
    if delta.fortress_id != basis.fortress_id
        || successor.fortress_id != basis.fortress_id
        || delta.base_cursor != basis.cursor
        || delta.target_cursor != successor.cursor
        || delta.base_hash != basis.state_hash
        || delta.target_hash != successor.state_hash
        || delta.target_tick != successor.tick
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "ATP proof inputs do not describe one continuous state transition",
        ));
    }
    let reconstructed = apply_delta(basis, delta).map_err(|error| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            format!("ATP delta cannot reconstruct its successor: {error}"),
        )
    })?;
    if reconstructed != *successor {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "ATP delta reconstructs a state other than the declared successor",
        ));
    }
    Ok(())
}

fn capsule_digest(
    basis: StateAnchor,
    successor: StateAnchor,
    merkle_root: Digest32,
    delta: &StateDelta,
    published_at_tick: GameTick,
) -> Digest32 {
    let mut bytes = Vec::new();
    crate::canonical::put_str(&mut bytes, "dfmcp-atp-proof-capsule-v1");
    crate::canonical::put_anchor(&mut bytes, basis);
    crate::canonical::put_anchor(&mut bytes, successor);
    crate::canonical::put_bytes(&mut bytes, merkle_root.as_bytes());
    crate::canonical::put_bytes(&mut bytes, &delta.canonical_bytes());
    crate::canonical::put_u64(&mut bytes, published_at_tick.0);
    Digest32::of_bytes(&bytes)
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

    #[test]
    fn seal_rejects_partial_or_premature_transition() {
        let basis = sample_snapshot(100, ObservationCursor::ORIGIN);
        let successor = sample_snapshot(
            101,
            ObservationCursor {
                epoch: 0,
                sequence: 1,
            },
        );
        let mut delta = StateDelta {
            fortress_id: basis.fortress_id,
            base_cursor: basis.cursor,
            target_cursor: successor.cursor,
            base_hash: basis.state_hash,
            target_hash: successor.state_hash,
            target_tick: successor.tick,
            changes: Vec::new(),
            truncated: true,
            continuation: Some("more".to_owned()),
        };
        assert!(AtpProofCapsule::seal(&basis, &successor, delta.clone(), GameTick(101)).is_err());
        delta.truncated = false;
        delta.continuation = None;
        assert!(AtpProofCapsule::seal(&basis, &successor, delta, GameTick(100)).is_err());
    }
}
