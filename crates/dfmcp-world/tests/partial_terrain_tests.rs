//! Partial live terrain must not invent facts outside its exact observed cells.
use dfmcp_core::{FortressId, GameTick, MapCoord, MapCuboid, ObservationCursor, Result};
use dfmcp_world::terrain::{RegionTruth, tile_codes};
use dfmcp_world::{
    ChunkCoord, ChunkSpatialIndex, EvidencePolicy, MapChunk, Predicate, PredicateEvidence,
    PredicateTruth, TerrainRun, WorldGraph, WorldSnapshot,
};
use std::collections::BTreeMap;

fn partial() -> WorldSnapshot {
    let coord = ChunkCoord { x: 0, y: 0, z: 2 };
    let mut graph = WorldGraph::default();
    graph.chunks.insert(
        coord,
        MapChunk {
            coord,
            revision: 1,
            width: 16,
            height: 16,
            terrain_runs: vec![
                TerrainRun {
                    tile_code: tile_codes::FLOOR,
                    length: 1,
                },
                TerrainRun {
                    tile_code: tile_codes::UNKNOWN,
                    length: 1,
                },
                TerrainRun {
                    tile_code: tile_codes::SOLID_WALL,
                    length: 1,
                },
                TerrainRun {
                    tile_code: tile_codes::UNKNOWN,
                    length: 253,
                },
            ],
            sparse_overlays: BTreeMap::new(),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(20),
        ObservationCursor {
            epoch: 1,
            sequence: 2,
        },
        true,
        graph,
    )
}
fn region(start: i32, end: i32) -> Result<MapCuboid> {
    MapCuboid::new(MapCoord::new(start, 0, 2), MapCoord::new(end, 0, 2))
}

#[test]
fn partial_chunk_roundtrip_preserves_observed_and_unknown_cells() -> Result<()> {
    let original = partial();
    let bytes = original.canonical_bytes();
    let recovered = WorldSnapshot::from_canonical_bytes(&bytes)?;
    assert_eq!(recovered.canonical_bytes(), bytes);
    assert_eq!(recovered.anchor(), original.anchor());
    assert_eq!(
        recovered.tile_code_at(MapCoord::new(0, 0, 2)),
        Some(tile_codes::FLOOR)
    );
    assert_eq!(recovered.tile_code_at(MapCoord::new(1, 0, 2)), None);
    assert_eq!(
        recovered.tile_code_at(MapCoord::new(2, 0, 2)),
        Some(tile_codes::SOLID_WALL)
    );
    assert_eq!(recovered.tile_code_at(MapCoord::new(15, 15, 2)), None);
    assert_eq!(
        recovered.region_terrain_truth(region(0, 0)?, tile_codes::FLOOR),
        RegionTruth::Matches
    );
    assert_eq!(
        recovered.region_terrain_truth(region(0, 1)?, tile_codes::FLOOR),
        RegionTruth::Unknown
    );
    assert_eq!(
        recovered.region_terrain_truth(region(0, 2)?, tile_codes::FLOOR),
        RegionTruth::Differs
    );
    assert_eq!(
        recovered.region_terrain_truth(region(1, 1)?, tile_codes::UNKNOWN),
        RegionTruth::Unknown
    );
    Ok(())
}

#[test]
fn even_complete_coverage_cannot_certify_unknown_terrain_or_its_negation() -> Result<()> {
    let snapshot = partial();
    let mut policy = EvidencePolicy::at(snapshot.anchor());
    policy.terrain_regions.push(region(0, 15)?);
    let evidence = PredicateEvidence::scoped(&snapshot, policy)?;
    let known = Predicate::RegionTerrain {
        area: region(0, 0)?,
        tile_code: tile_codes::FLOOR,
    };
    assert!(evidence.establishes(&known)?);
    for predicate in [
        Predicate::RegionTerrain {
            area: region(0, 1)?,
            tile_code: tile_codes::FLOOR,
        },
        Predicate::RegionTerrain {
            area: region(1, 1)?,
            tile_code: tile_codes::UNKNOWN,
        },
        Predicate::RegionTerrain {
            area: region(3, 15)?,
            tile_code: tile_codes::OPEN_SPACE,
        },
    ] {
        assert_eq!(evidence.evaluate(&predicate)?, PredicateTruth::Unknown);
        assert_eq!(
            evidence.evaluate(&Predicate::Not(Box::new(predicate)))?,
            PredicateTruth::Unknown
        );
    }
    assert_eq!(
        evidence.evaluate(&Predicate::RegionTerrain {
            area: region(0, 2)?,
            tile_code: tile_codes::FLOOR,
        })?,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn reference_mutation_cannot_fill_unobserved_cells_or_change_their_history() -> Result<()> {
    let mut snapshot = partial();
    let before = snapshot.canonical_bytes();
    for replacement in [
        tile_codes::FLOOR,
        tile_codes::SOLID_WALL,
        tile_codes::UNKNOWN,
    ] {
        assert!(
            snapshot
                .set_tile_code(MapCoord::new(1, 0, 2), replacement)
                .is_err()
        );
        assert_eq!(snapshot.canonical_bytes(), before);
    }
    assert!(snapshot.set_tile_code(MapCoord::new(2, 0, 2), tile_codes::FLOOR)?);
    assert_eq!(
        snapshot.tile_code_at(MapCoord::new(2, 0, 2)),
        Some(tile_codes::FLOOR)
    );
    assert_eq!(snapshot.tile_code_at(MapCoord::new(1, 0, 2)), None);
    Ok(())
}

#[test]
fn spatial_index_refuses_unknown_cells_instead_of_inventing_safe_properties() -> Result<()> {
    let snapshot = partial();
    let mut index = ChunkSpatialIndex::new();
    for chunk in snapshot.graph.chunks.values() {
        assert!(index.insert_or_update_chunk(chunk).is_err());
    }
    assert!(index.is_empty());
    assert!(index.get_tile(MapCoord::new(1, 0, 2)).is_none());
    assert!(
        index
            .find_walkable_neighbors(MapCoord::new(1, 0, 2))
            .is_empty()
    );
    assert!(index.find_cuboid(&region(0, 2)?)?.is_empty());
    Ok(())
}
