#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_core::{
    Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, StateAnchor,
};
use dfmcp_world::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource,
    MapChunk, TerrainRun, Value, WorldChange, WorldGraph, WorldSnapshot, apply_delta, build_delta,
};

fn make_fact(val: i64, tick: u64) -> Fact {
    Fact::known(
        Value::I64(val),
        GameTick(tick),
        FactSource::Replay,
        Digest32::ZERO,
    )
}

fn make_unit(id: u64, generation: u32, rev: u64, label: &str) -> EntityRecord {
    let mut fields = BTreeMap::new();
    fields.insert("health".to_owned(), make_fact(100, rev));
    EntityRecord {
        id: EntityId::new(id),
        generation,
        revision: rev,
        kind: EntityKind::Unit,
        label: label.to_owned(),
        fields,
    }
}

fn make_edge(id: u128, from: u64, to: u64, rev: u64, kind: EdgeKind) -> EdgeRecord {
    EdgeRecord {
        id: EdgeId::new(id),
        revision: rev,
        kind,
        from: EntityId::new(from),
        to: EntityId::new(to),
        fields: BTreeMap::new(),
    }
}

fn make_chunk(x: i32, y: i32, z: i32, rev: u64, width: u16, height: u16) -> MapChunk {
    let total_tiles = u32::from(width) * u32::from(height);
    MapChunk {
        coord: ChunkCoord { x, y, z },
        revision: rev,
        width,
        height,
        terrain_runs: vec![TerrainRun {
            tile_code: 1, // wall
            length: total_tiles,
        }],
        sparse_overlays: BTreeMap::new(),
    }
}

fn make_base_snapshot() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph
        .entities
        .insert(EntityId::new(1), make_unit(1, 1, 1, "Urist"));
    graph
        .entities
        .insert(EntityId::new(2), make_unit(2, 1, 1, "Miner"));
    graph.edges.insert(
        EdgeId::new(100),
        make_edge(100, 1, 2, 1, EdgeKind::Supports),
    );
    graph.chunks.insert(
        ChunkCoord { x: 0, y: 0, z: 0 },
        make_chunk(0, 0, 0, 1, 16, 16),
    );

    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

/// TEST-005: Graph Integrity — Edge endpoints existence
#[test]
fn test_005_edge_endpoints_must_exist() -> Result<(), Box<dyn std::error::Error>> {
    let base = make_base_snapshot();

    // Adding an edge pointing to a non-existent 'to' entity (999)
    let dangling_edge = make_edge(101, 1, 999, 1, EdgeKind::AssignedTo);
    let delta_res = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertEdge(dangling_edge)],
    );

    assert!(delta_res.is_err());
    let Err(err) = delta_res else {
        return Err("expected build_delta to fail for missing endpoint".into());
    };
    assert_eq!(err.code, ErrorCode::InternalInvariantViolation);
    assert!(err.message.contains("missing endpoint"));
    Ok(())
}

/// TEST-005: Graph Integrity — Entity and Edge revision monotonicity
#[test]
fn test_005_revision_monotonicity() -> Result<(), Box<dyn std::error::Error>> {
    let base = make_base_snapshot();

    // 1. Regressing entity revision at the same generation
    let mut regressed_entity = make_unit(1, 1, 0, "Urist Regressed");
    regressed_entity.revision = 0; // base was 1
    let Err(err1) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertEntity(regressed_entity)],
    ) else {
        return Err("expected regression to fail".into());
    };
    assert_eq!(err1.code, ErrorCode::StaleAnchor);
    assert!(err1.message.contains("revision regressed"));

    // 2. Conflicting entity at same generation and revision
    let conflicting_entity = make_unit(1, 1, 1, "Urist Mutated Without Revision Bump");
    let Err(err2) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertEntity(conflicting_entity)],
    ) else {
        return Err("expected conflict to fail".into());
    };
    assert_eq!(err2.code, ErrorCode::Conflict);
    assert!(
        err2.message
            .contains("different content at the same generation and revision")
    );

    // 3. Regressing edge revision
    let regressed_edge = make_edge(100, 1, 2, 0, EdgeKind::Supports);
    let Err(err3) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertEdge(regressed_edge)],
    ) else {
        return Err("expected edge regression to fail".into());
    };
    assert_eq!(err3.code, ErrorCode::StaleAnchor);
    assert!(err3.message.contains("revision regressed"));

    // 4. Conflicting edge at same revision
    let conflicting_edge = make_edge(100, 1, 2, 1, EdgeKind::Threatens);
    let Err(err4) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertEdge(conflicting_edge)],
    ) else {
        return Err("expected edge conflict to fail".into());
    };
    assert_eq!(err4.code, ErrorCode::Conflict);
    Ok(())
}

/// TEST-005: MapChunk Tile Count and Sparse Overlay Validation
#[test]
fn test_005_map_chunk_integrity() -> Result<(), Box<dyn std::error::Error>> {
    let base = make_base_snapshot();

    // 1. Nonzero dimension check
    let zero_chunk = MapChunk {
        coord: ChunkCoord { x: 1, y: 1, z: 0 },
        revision: 1,
        width: 0,
        height: 16,
        terrain_runs: vec![],
        sparse_overlays: BTreeMap::new(),
    };
    let Err(err1) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertMapChunk(zero_chunk)],
    ) else {
        return Err("expected zero dimension to fail".into());
    };
    assert_eq!(err1.code, ErrorCode::InvalidRequest);
    assert!(err1.message.contains("must be nonzero"));

    // 2. Terrain run length sum mismatch (declares 16x16=256 tiles, but run only has 100)
    let incomplete_chunk = MapChunk {
        coord: ChunkCoord { x: 1, y: 1, z: 0 },
        revision: 1,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            tile_code: 1,
            length: 100,
        }],
        sparse_overlays: BTreeMap::new(),
    };
    let Err(err2) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertMapChunk(incomplete_chunk)],
    ) else {
        return Err("expected incomplete chunk to fail".into());
    };
    assert_eq!(err2.code, ErrorCode::InvalidRequest);
    assert!(err2.message.contains("do not cover exactly"));

    // 3. Sparse overlay offset out of bounds
    let mut overlays = BTreeMap::new();
    let mut overlay_props = BTreeMap::new();
    overlay_props.insert("liquid".to_owned(), Value::U64(7));
    overlays.insert(300, overlay_props); // 300 >= 256
    let oob_overlay_chunk = MapChunk {
        coord: ChunkCoord { x: 1, y: 1, z: 0 },
        revision: 1,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            tile_code: 1,
            length: 256,
        }],
        sparse_overlays: overlays,
    };
    let Err(err3) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::UpsertMapChunk(oob_overlay_chunk)],
    ) else {
        return Err("expected oob overlay to fail".into());
    };
    assert_eq!(err3.code, ErrorCode::InvalidRequest);
    assert!(err3.message.contains("out of bounds"));
    Ok(())
}

/// TEST-002: Identity / ABA semantics — Deletion, Generation Mismatch, and ID Reuse
#[test]
fn test_002_identity_and_aba_semantics() -> Result<(), Box<dyn std::error::Error>> {
    let base = make_base_snapshot();

    // 1. Remove entity with wrong revision -> StaleAnchor
    let Err(err_remove) = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![WorldChange::RemoveEntity {
            id: EntityId::new(1),
            expected_generation: 1,
            expected_revision: 999, // mismatch!
        }],
    ) else {
        return Err("expected remove mismatch to fail".into());
    };
    assert_eq!(err_remove.code, ErrorCode::StaleAnchor);

    // 2. Legitimate deletion of edge and entity
    let delta_del = build_delta(
        &base,
        base.cursor.next(),
        GameTick(101),
        vec![
            WorldChange::RemoveEdge {
                id: EdgeId::new(100),
                expected_revision: 1,
            },
            WorldChange::RemoveEntity {
                id: EntityId::new(2),
                expected_generation: 1,
                expected_revision: 1,
            },
        ],
    )?;
    let snapshot_after_del = apply_delta(&base, &delta_del)?;
    assert!(
        !snapshot_after_del
            .graph
            .entities
            .contains_key(&EntityId::new(2))
    );
    assert!(
        !snapshot_after_del
            .graph
            .edges
            .contains_key(&EdgeId::new(100))
    );

    // 3. Re-inserting Entity 2 with new generation 2 (legitimate anti-ABA lifecycle)
    let reinserted_gen2 = make_unit(2, 2, 1, "New Dwarf Reusing Slot");
    let delta_reuse = build_delta(
        &snapshot_after_del,
        snapshot_after_del.cursor.next(),
        GameTick(102),
        vec![WorldChange::UpsertEntity(reinserted_gen2)],
    )?;
    let snapshot_after_reuse = apply_delta(&snapshot_after_del, &delta_reuse)?;
    let reused = snapshot_after_reuse
        .graph
        .entities
        .get(&EntityId::new(2))
        .ok_or("reused entity missing")?;
    assert_eq!(reused.generation, 2);
    assert_eq!(reused.label, "New Dwarf Reusing Slot");
    Ok(())
}

/// TEST-003: Presence & Provenance Algebra
#[test]
fn test_003_presence_algebra_and_provenance() {
    let anchor = StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 42,
        },
        tick: GameTick(100),
        state_hash: Digest32::ZERO,
    };

    // Test each presence variant
    let presences = [
        FactPresence::Known(Value::Text("Active".to_owned())),
        FactPresence::Absent,
        FactPresence::Unknown("Unexplored tile".to_owned()),
        FactPresence::Unsupported("DFHack 53.16 field omitted".to_owned()),
        FactPresence::Omitted("Budget limit reached".to_owned()),
        FactPresence::Redacted("Policy security gate".to_owned()),
        FactPresence::Stale(anchor),
    ];

    for (i, p) in presences.iter().enumerate() {
        let fact = Fact::with_presence(
            p.clone(),
            GameTick(100),
            FactSource::Derived("UnitAnalyzer".to_owned()),
            Digest32::ZERO,
        );

        match p {
            FactPresence::Known(v) => {
                assert!(p.is_known());
                assert_eq!(p.as_known(), Some(v));
                assert_eq!(&fact.value, v);
            }
            FactPresence::Absent => {
                assert!(p.is_absent());
                assert_eq!(&fact.value, &Value::Null);
            }
            FactPresence::Stale(_) => {
                assert!(p.is_stale());
                assert_eq!(&fact.value, &Value::Null);
            }
            _ => {
                assert!(!p.is_known());
                assert_eq!(&fact.value, &Value::Null);
            }
        }

        // Deterministic encoding test
        let bytes1 = fact.canonical_bytes();
        let bytes2 = fact.canonical_bytes();
        assert_eq!(
            bytes1, bytes2,
            "Fact encoding must be deterministic for variant {i}"
        );
    }
}
