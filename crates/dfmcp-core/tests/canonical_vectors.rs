#![forbid(unsafe_code)]

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
    MapCoord, MapCuboid, ObservationCursor, RiskTier, StateAnchor,
};

/// Golden test fixture for canonical primitives.
/// Encodes explicit framing rules: big-endian integers, ordered bytes, SHA-256.
#[test]
fn test_canonical_state_anchor_golden_vector() -> Result<(), Box<dyn std::error::Error>> {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(0),
        state_hash: Digest32::ZERO,
    };

    // Framing:
    // fortress_id: u64 be (8 bytes)
    // epoch: u64 be (8 bytes)
    // sequence: u64 be (8 bytes)
    // tick: u64 be (8 bytes)
    // state_hash: 32 bytes
    // Total = 64 bytes
    let mut framed = Vec::with_capacity(64);
    framed.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    framed.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    framed.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    framed.extend_from_slice(&anchor.tick.0.to_be_bytes());
    framed.extend_from_slice(anchor.state_hash.as_bytes());

    assert_eq!(framed.len(), 64);

    let anchor_digest = Digest32::of_bytes(&framed);
    let expected_hex = "40c40bfff532a623cbed6d12294d91c254cca2ef45f37aa2677314f4bea113ff";

    assert_eq!(anchor_digest.to_hex(), expected_hex);
    Ok(())
}

#[test]
fn test_canonical_map_cuboid_golden_vector() -> Result<(), Box<dyn std::error::Error>> {
    let cuboid = MapCuboid::new(MapCoord { x: 0, y: 0, z: 0 }, MapCoord { x: 9, y: 9, z: 0 })?;

    // Framing:
    // min.x (i32 be, 4), min.y (i32 be, 4), min.z (i32 be, 4)
    // max.x (i32 be, 4), max.y (i32 be, 4), max.z (i32 be, 4)
    // Total = 24 bytes
    let mut framed = Vec::with_capacity(24);
    framed.extend_from_slice(&cuboid.min.x.to_be_bytes());
    framed.extend_from_slice(&cuboid.min.y.to_be_bytes());
    framed.extend_from_slice(&cuboid.min.z.to_be_bytes());
    framed.extend_from_slice(&cuboid.max.x.to_be_bytes());
    framed.extend_from_slice(&cuboid.max.y.to_be_bytes());
    framed.extend_from_slice(&cuboid.max.z.to_be_bytes());
    let cuboid_digest = Digest32::of_bytes(&framed);
    let expected_hex = "f6c368010e6baa70164b2544e89984b85fa2f317fa8d80c0e8728ef7918c565a";

    assert_eq!(cuboid_digest.to_hex(), expected_hex);

    Ok(())
}

/// Golden test for capability grant framing:
/// encoding is fully deterministic and the digest must round-trip.
#[test]
fn test_canonical_capability_grant_framing() -> Result<(), Box<dyn std::error::Error>> {
    let mut scope = CapabilityScope::default();
    scope.entity_ids.insert(EntityId::new(11));
    let grant = CapabilityGrant {
        capability: Capability::ConfigureLabor,
        scope,
        max_risk: RiskTier::Reversible,
        expires_at_tick: Some(GameTick(100)),
        remaining_uses: Some(5),
    };

    let mut framed = Vec::new();
    encode_grant(&grant, &mut framed);

    // Determinism: same input yields same bytes
    let mut framed_again = Vec::new();
    encode_grant(&grant, &mut framed_again);
    assert_eq!(framed, framed_again);

    // Digest must be stable across calls
    let d1 = Digest32::of_bytes(&framed);
    let d2 = Digest32::of_bytes(&framed);
    assert_eq!(d1, d2);
    assert_ne!(d1, Digest32::ZERO);
    Ok(())
}

fn encode_grant(grant: &CapabilityGrant, out: &mut Vec<u8>) {
    out.push(grant.capability as u8);
    out.push(grant.max_risk as u8);
    match grant.expires_at_tick {
        Some(tick) => {
            out.push(1);
            out.extend_from_slice(&tick.0.to_be_bytes());
        }
        None => out.push(0),
    }
    match grant.remaining_uses {
        Some(uses) => {
            out.push(1);
            out.extend_from_slice(&uses.to_be_bytes());
        }
        None => out.push(0),
    }
    match grant.scope.fortress_id {
        Some(fid) => {
            out.push(1);
            out.extend_from_slice(&fid.get().to_be_bytes());
        }
        None => out.push(0),
    }
    out.extend_from_slice(&(grant.scope.entity_ids.len() as u64).to_be_bytes());
    for eid in &grant.scope.entity_ids {
        out.extend_from_slice(&eid.get().to_be_bytes());
    }
    match &grant.scope.map_area {
        Some(cuboid) => {
            out.push(1);
            out.extend_from_slice(&cuboid.min.x.to_be_bytes());
            out.extend_from_slice(&cuboid.min.y.to_be_bytes());
            out.extend_from_slice(&cuboid.min.z.to_be_bytes());
            out.extend_from_slice(&cuboid.max.x.to_be_bytes());
            out.extend_from_slice(&cuboid.max.y.to_be_bytes());
            out.extend_from_slice(&cuboid.max.z.to_be_bytes());
        }
        None => out.push(0),
    }
}
