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

#[test]
fn test_atp_chain_verification_and_gap_detection() -> Result<()> {
    let snap0 = sample_snapshot(100, ObservationCursor::ORIGIN);
    let snap1 = sample_snapshot(
        101,
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
    );
    let snap2 = sample_snapshot(
        102,
        ObservationCursor {
            epoch: 0,
            sequence: 2,
        },
    );

    let delta1 = StateDelta {
        fortress_id: FortressId::new(42),
        base_cursor: snap0.cursor,
        target_cursor: snap1.cursor,
        base_hash: snap0.state_hash,
        target_hash: snap1.state_hash,
        target_tick: snap1.tick,
        changes: Vec::new(),
        truncated: false,
        continuation: None,
    };
    let cap1 = AtpProofCapsule::seal(&snap0, &snap1, delta1, GameTick(101))?;

    let delta2 = StateDelta {
        fortress_id: FortressId::new(42),
        base_cursor: snap1.cursor,
        target_cursor: snap2.cursor,
        base_hash: snap1.state_hash,
        target_hash: snap2.state_hash,
        target_tick: snap2.tick,
        changes: Vec::new(),
        truncated: false,
        continuation: None,
    };
    let cap2 = AtpProofCapsule::seal(&snap1, &snap2, delta2, GameTick(102))?;

    let verifier = AtpProofVerifier;
    assert!(verifier.verify_chain(&[cap1.clone(), cap2.clone()]).is_ok());

    // Introduce chain gap: cap2 following cap2 instead of cap1
    assert!(verifier.verify_chain(&[cap2.clone(), cap1]).is_err());

    Ok(())
}
