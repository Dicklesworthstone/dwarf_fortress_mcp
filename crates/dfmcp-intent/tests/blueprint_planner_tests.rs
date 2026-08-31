#![forbid(unsafe_code)]

//! Integration tests for WP-PLN-01 Blueprint & Defensive Fortress Layout Planner.

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{
    Digest32, FortressId, GameTick, IntentId, MapCoord, ObservationCursor, StateAnchor,
};
use dfmcp_intent::blueprint::{BlueprintPlanner, BlueprintTemplate};
use dfmcp_world::spatial_index::ChunkSpatialIndex;
use dfmcp_world::{ChunkCoord, MapChunk, TerrainRun};

fn sample_index() -> ChunkSpatialIndex {
    let mut index = ChunkSpatialIndex::new();
    let chunk = MapChunk {
        coord: ChunkCoord { x: 0, y: 0, z: 100 },
        revision: 1,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            tile_code: 2, // SolidWall
            length: 256,
        }],
        sparse_overlays: BTreeMap::new(),
    };
    index.insert_or_update_chunk(&chunk);
    index
}

#[test]
fn test_workshop_hub_blueprint_compilation() -> Result<(), Box<dyn Error>> {
    let planner = BlueprintPlanner;
    let index = sample_index();
    let anchor = StateAnchor {
        fortress_id: FortressId::new(1),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(100),
        state_hash: Digest32::ZERO,
    };

    let intent = planner.compile_blueprint_intent(
        IntentId::new(10),
        anchor,
        MapCoord { x: 0, y: 0, z: 100 },
        BlueprintTemplate::WorkshopHub { bays_count: 3 },
        &index,
    )?;

    assert_eq!(intent.requested_actions.len(), 3);
    assert_eq!(intent.summary, "excavate 3 workshop bays");

    Ok(())
}

#[test]
fn test_magma_hazard_preflight_rejection() {
    let planner = BlueprintPlanner;
    let mut index = sample_index();

    // Place a magma tile adjacent to our planned excavation
    let magma_chunk = MapChunk {
        coord: ChunkCoord { x: 0, y: 0, z: 100 },
        revision: 2,
        width: 16,
        height: 16,
        terrain_runs: vec![TerrainRun {
            tile_code: 7, // MagmaWall
            length: 256,
        }],
        sparse_overlays: BTreeMap::new(),
    };
    index.insert_or_update_chunk(&magma_chunk);

    let anchor = StateAnchor {
        fortress_id: FortressId::new(1),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(100),
        state_hash: Digest32::ZERO,
    };

    let result = planner.compile_blueprint_intent(
        IntentId::new(11),
        anchor,
        MapCoord { x: 5, y: 5, z: 100 },
        BlueprintTemplate::DiningHall {
            width: 4,
            height: 4,
        },
        &index,
    );

    assert!(result.is_err());
}
