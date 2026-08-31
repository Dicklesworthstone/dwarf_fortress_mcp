#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EdgeId, EntityId, FortressId, GameTick, ObservationCursor};
use dfmcp_world::{
    ChunkCoord, EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, MapChunk,
    TerrainRun, Value, WorldGraph, WorldSnapshot,
};

/// Golden test for MapChunk canonical hashing
#[test]
fn test_canonical_map_chunk_golden_vector() {
    let mut overlays = BTreeMap::new();
    let mut tile_props = BTreeMap::new();
    tile_props.insert("liquid_depth".to_owned(), Value::U64(7));
    tile_props.insert("magma".to_owned(), Value::Bool(true));
    overlays.insert(5, tile_props);

    let chunk = MapChunk {
        coord: ChunkCoord {
            x: 2,
            y: -4,
            z: 120,
        },
        revision: 3,
        width: 4,
        height: 4,
        terrain_runs: vec![
            TerrainRun {
                tile_code: 1, // wall
                length: 8,
            },
            TerrainRun {
                tile_code: 2, // floor
                length: 8,
            },
        ],
        sparse_overlays: overlays,
    };

    assert_eq!(chunk.tile_count(), 16);
    assert_eq!(chunk.encoded_tile_count(), Some(16));

    let hash = chunk.compute_hash();
    let expected_hex = "49657aec75f40151984b878392e7030d828a4e8cc5f7b5bfe02eb69fe87674c3";
    assert_eq!(
        hash.to_hex(),
        expected_hex,
        "MapChunk golden vector hash mismatch"
    );
}

/// Golden test for EntityRecord and EdgeRecord canonical encoding
#[test]
fn test_canonical_entity_and_edge_golden_vector() {
    let mut fields = BTreeMap::new();
    fields.insert(
        "profession".to_owned(),
        Fact::known(
            Value::Text("Miner".to_owned()),
            GameTick(1000),
            FactSource::DfhackField("unit.profession".to_owned()),
            Digest32::ZERO,
        ),
    );

    let entity = EntityRecord {
        id: EntityId::new(42),
        generation: 1,
        revision: 5,
        kind: EntityKind::Unit,
        label: "Urist McMiner".to_owned(),
        fields,
    };

    let edge = EdgeRecord {
        id: EdgeId::new(99),
        revision: 2,
        kind: EdgeKind::LocatedAt,
        from: EntityId::new(42),
        to: EntityId::new(1),
        fields: BTreeMap::new(),
    };

    let mut graph = WorldGraph::default();
    graph.entities.insert(entity.id, entity);
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 1,
            kind: EntityKind::Building,
            label: "Mine Entrance".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    graph.edges.insert(edge.id, edge);

    let snapshot = WorldSnapshot::new(
        FortressId::new(7),
        GameTick(1000),
        ObservationCursor {
            epoch: 1,
            sequence: 10,
        },
        false,
        graph,
    );

    let expected_hex = "1fff5bbb6091c942e73eb970d9a11d1940902a1b375469975a325b2c5f8cc7de";
    assert_eq!(
        snapshot.state_hash.to_hex(),
        expected_hex,
        "WorldSnapshot golden vector mismatch"
    );
}

/// Sparse-overlay vs Dense Representation Equivalence Test
#[test]
fn test_sparse_overlay_vs_dense_representation_equivalence() {
    // A 4x4 chunk with 16 tiles
    // Tile 0..15: all terrain_code 1 (wall), except tile 5 which has overlay liquid_depth=7
    let mut overlays = BTreeMap::new();
    let mut tile_props = BTreeMap::new();
    tile_props.insert("liquid_depth".to_owned(), Value::U64(7));
    overlays.insert(5, tile_props.clone());

    let chunk_sparse = MapChunk {
        coord: ChunkCoord { x: 0, y: 0, z: 0 },
        revision: 1,
        width: 4,
        height: 4,
        terrain_runs: vec![TerrainRun {
            tile_code: 1,
            length: 16,
        }],
        sparse_overlays: overlays,
    };

    // Reconstruct dense grid of tile values from chunk_sparse
    let mut dense_grid: Vec<(u32, BTreeMap<String, Value>)> = Vec::with_capacity(16);
    let mut tile_idx = 0u32;
    for run in &chunk_sparse.terrain_runs {
        for _ in 0..run.length {
            let props = chunk_sparse
                .sparse_overlays
                .get(&tile_idx)
                .cloned()
                .unwrap_or_default();
            dense_grid.push((run.tile_code, props));
            tile_idx += 1;
        }
    }

    assert_eq!(dense_grid.len(), 16);
    assert_eq!(dense_grid[5].0, 1);
    assert_eq!(dense_grid[5].1.get("liquid_depth"), Some(&Value::U64(7)));
    assert!(dense_grid[0].1.is_empty());
    assert!(dense_grid[15].1.is_empty());
}
