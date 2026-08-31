#![forbid(unsafe_code)]

//! Integration tests for WP-WLD-03 ATP Verifiable Proof Capsules & Merkle Trees.

use dfmcp_core::{FortressId, GameTick, ObservationCursor, Result};
use dfmcp_world::atp::{AtpProofCapsule, AtpProofVerifier};
use dfmcp_world::{StateDelta, WorldGraph, WorldSnapshot};

fn sample_snapshot(tick: u64, cursor: ObservationCursor) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(42),
        GameTick(tick),
        cursor,
        true,
        WorldGraph::default(),
    )
}

#[test]
fn test_atp_proof_capsule_tamper_detection() -> Result<()> {
    let snap_base = sample_snapshot(100, ObservationCursor::ORIGIN);
    let snap_target = sample_snapshot(
        101,
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
    );

    let delta = StateDelta {
        fortress_id: FortressId::new(42),
        base_cursor: snap_base.cursor,
        target_cursor: snap_target.cursor,
        base_hash: snap_base.state_hash,
        target_hash: snap_target.state_hash,
        target_tick: snap_target.tick,
        changes: Vec::new(),
        truncated: false,
        continuation: None,
    };

    let mut capsule = AtpProofCapsule::seal(&snap_base, &snap_target, delta, GameTick(101))?;
    let verifier = AtpProofVerifier;

    assert!(verifier.verify_capsule(&capsule).is_ok());

    // Tamper with Merkle root
    capsule.merkle_root = dfmcp_core::Digest32::of_bytes(b"tampered_root");
    assert!(verifier.verify_capsule(&capsule).is_err());

    Ok(())
}
