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

fn sample_index() -> Result<ChunkSpatialIndex, Box<dyn Error>> {
    let mut index = ChunkSpatialIndex::new();
    for z in 99..=101 {
        for y in -1..=1 {
            for x in -1..=1 {
                index.insert_or_update_chunk(&MapChunk {
                    coord: ChunkCoord { x, y, z },
                    revision: 1,
                    width: 16,
                    height: 16,
                    terrain_runs: vec![TerrainRun {
                        tile_code: 2,
                        length: 256,
                    }],
                    sparse_overlays: BTreeMap::new(),
                })?;
            }
        }
    }
    Ok(index)
}

#[test]
fn test_workshop_hub_blueprint_compilation() -> Result<(), Box<dyn Error>> {
    let planner = BlueprintPlanner;
    let index = sample_index()?;
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
fn test_magma_hazard_preflight_rejection() -> Result<(), Box<dyn Error>> {
    let planner = BlueprintPlanner;
    let mut index = sample_index()?;

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
    index.insert_or_update_chunk(&magma_chunk)?;

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
    Ok(())
}

#[test]
fn test_stockpile_vault_and_defensive_moat_compilation() -> Result<(), Box<dyn Error>> {
    let planner = BlueprintPlanner;
    let index = sample_index()?;
    let anchor = StateAnchor {
        fortress_id: FortressId::new(1),
        cursor: ObservationCursor::ORIGIN,
        tick: GameTick(100),
        state_hash: Digest32::ZERO,
    };

    // 1. Stockpile vault
    let stockpile_intent = planner.compile_blueprint_intent(
        IntentId::new(20),
        anchor,
        MapCoord { x: 0, y: 0, z: 100 },
        BlueprintTemplate::StockpileVault {
            width: 5,
            height: 5,
            category: "Metal Bars & Ore".to_owned(),
        },
        &index,
    )?;
    assert_eq!(stockpile_intent.requested_actions.len(), 1);
    assert!(stockpile_intent.summary.contains("stockpile vault"));

    // 2. Defensive moat
    let moat_cuboid = dfmcp_core::MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 100 },
        MapCoord { x: 5, y: 5, z: 100 },
    )?;
    let moat_intent = planner.compile_blueprint_intent(
        IntentId::new(21),
        anchor,
        MapCoord { x: 0, y: 0, z: 100 },
        BlueprintTemplate::DefensiveMoat {
            perimeter_cuboid: moat_cuboid,
            drawbridge_span: 0,
        },
        &index,
    )?;
    assert_eq!(moat_intent.requested_actions.len(), 1);
    assert_eq!(moat_intent.summary, "excavate defensive moat");

    Ok(())
}
