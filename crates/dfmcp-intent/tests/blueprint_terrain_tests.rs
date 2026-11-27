#![forbid(unsafe_code)]

//! Blueprint terrain feasibility shares the executable excavation model.
//! Owners: df-lab-connected-room-access-7c2, df-predicate-evidence-authority-r7m.

use dfmcp_core::{FortressId, GameTick, IntentId, MapCoord, MapCuboid, ObservationCursor, Result};
use dfmcp_intent::blueprint::{BlueprintPlanner, BlueprintTemplate};
use dfmcp_intent::{Action, effects};
use dfmcp_world::spatial_index::ChunkSpatialIndex;
use dfmcp_world::terrain::{tile_codes, uniform_chunk};
use dfmcp_world::{ChunkCoord, PredicateEvidence, WorldGraph, WorldSnapshot};

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for z in 9..=11 {
        for y in -1..=1 {
            for x in -1..=1 {
                let coord = ChunkCoord { x, y, z };
                graph
                    .chunks
                    .insert(coord, uniform_chunk(coord, tile_codes::SOLID_WALL));
            }
        }
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

fn index(snapshot: &WorldSnapshot) -> Result<ChunkSpatialIndex> {
    let mut index = ChunkSpatialIndex::new();
    for chunk in snapshot.graph.chunks.values() {
        index.insert_or_update_chunk(chunk)?;
    }
    Ok(index)
}

fn room() -> BlueprintTemplate {
    BlueprintTemplate::DiningHall {
        width: 4,
        height: 4,
    }
}

#[test]
fn room_compilers_refuse_terrain_that_cannot_become_a_floor() -> Result<()> {
    let origin = MapCoord::new(3, 3, 10);
    for source in [
        tile_codes::OPEN_SPACE,
        tile_codes::CHASM,
        tile_codes::TREE,
        42,
    ] {
        let mut snapshot = world();
        snapshot.set_tile_code(MapCoord::new(4, 4, 10), source)?;
        snapshot.refresh_hash();
        let spatial = index(&snapshot)?;
        for furnished in [false, true] {
            let result = if furnished {
                BlueprintPlanner.compile_furnished_blueprint_intent(
                    IntentId::new(10),
                    snapshot.anchor(),
                    origin,
                    room(),
                    &spatial,
                )
            } else {
                BlueprintPlanner.compile_blueprint_intent(
                    IntentId::new(10),
                    snapshot.anchor(),
                    origin,
                    room(),
                    &spatial,
                )
            };
            assert!(
                result.is_err(),
                "source {source}, furnished {furnished} must not manufacture a room floor"
            );
        }
    }
    Ok(())
}

#[test]
fn equipped_workshops_refuse_a_void_inside_their_excavation() -> Result<()> {
    let mut snapshot = world();
    let origin = MapCoord::new(3, 3, 10);
    let template = BlueprintTemplate::WorkshopHub { bays_count: 1 };
    let layout = BlueprintPlanner.layout(origin, template)?;
    let at = layout.excavations()[0].area.min;
    snapshot.set_tile_code(at, tile_codes::OPEN_SPACE)?;
    snapshot.refresh_hash();
    assert!(
        BlueprintPlanner
            .compile_workshop_hub_intent(
                IntentId::new(11),
                snapshot.anchor(),
                origin,
                &["workshop:Carpenters".to_owned()],
                &index(&snapshot)?,
            )
            .is_err()
    );
    Ok(())
}

#[test]
fn a_valid_mixed_room_plan_reaches_its_actual_floor_goal() -> Result<()> {
    let mut snapshot = world();
    let origin = MapCoord::new(3, 3, 10);
    for (x, source) in [
        tile_codes::FLOOR,
        tile_codes::STAIR,
        tile_codes::RAMP,
        tile_codes::FORTIFICATION,
    ]
    .into_iter()
    .enumerate()
    {
        snapshot.set_tile_code(MapCoord::new(3 + x as i32, 4, 10), source)?;
    }
    snapshot.refresh_hash();
    let intent = BlueprintPlanner.compile_blueprint_intent(
        IntentId::new(12),
        snapshot.anchor(),
        origin,
        room(),
        &index(&snapshot)?,
    )?;
    assert!(!PredicateEvidence::laboratory(&snapshot)?.establishes(&intent.terminal_condition)?);
    for (i, requested) in intent.requested_actions.iter().enumerate() {
        assert!(matches!(requested.action, Action::DesignateDig { .. }));
        effects::apply_effect(&mut snapshot, &requested.action, &format!("valid-room-{i}"))?;
    }
    snapshot.tick = GameTick(201);
    effects::advance_effects(&mut snapshot, 200)?;
    snapshot.cursor.sequence += 1;
    snapshot.refresh_hash();
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(&intent.terminal_condition)?);
    Ok(())
}

#[test]
fn excavation_checks_only_selected_cells_and_preserves_moat_noops() -> Result<()> {
    let origin = MapCoord::new(3, 3, 10);
    for source in [tile_codes::OPEN_SPACE, tile_codes::CHASM, tile_codes::TREE] {
        let mut snapshot = world();
        // The halo is observed, but this cell is outside the requested room.
        snapshot.set_tile_code(MapCoord::new(2, 4, 10), source)?;
        snapshot.refresh_hash();
        BlueprintPlanner.compile_furnished_blueprint_intent(
            IntentId::new(13),
            snapshot.anchor(),
            origin,
            room(),
            &index(&snapshot)?,
        )?;
    }
    let mut snapshot = world();
    let perimeter = MapCuboid::new(origin, MapCoord::new(8, 8, 10))?;
    let template = BlueprintTemplate::DefensiveMoat {
        perimeter_cuboid: perimeter,
        drawbridge_span: 0,
    };
    let layout = BlueprintPlanner.layout(origin, template.clone())?;
    for part in layout.excavations() {
        for at in dfmcp_world::terrain::region_tiles(part.area) {
            snapshot.set_tile_code(at, tile_codes::OPEN_SPACE)?;
        }
    }
    // A chasm in the unselected interior is not a requested transformation.
    snapshot.set_tile_code(MapCoord::new(5, 5, 10), tile_codes::CHASM)?;
    snapshot.refresh_hash();
    let intent = BlueprintPlanner.compile_blueprint_intent(
        IntentId::new(14),
        snapshot.anchor(),
        origin,
        template,
        &index(&snapshot)?,
    )?;
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(&intent.terminal_condition)?);
    Ok(())
}

#[test]
fn moat_rejects_chasms_and_trees_without_rejecting_real_floor_channeling() -> Result<()> {
    let origin = MapCoord::new(3, 3, 10);
    let template = BlueprintTemplate::DefensiveMoat {
        perimeter_cuboid: MapCuboid::new(origin, MapCoord::new(8, 8, 10))?,
        drawbridge_span: 0,
    };
    let layout = BlueprintPlanner.layout(origin, template.clone())?;
    let selected = layout.excavations()[0].area.min;
    for (source, accepted) in [
        (tile_codes::FLOOR, true),
        (tile_codes::OPEN_SPACE, true),
        (tile_codes::CHASM, false),
        (tile_codes::TREE, false),
    ] {
        let mut snapshot = world();
        snapshot.set_tile_code(selected, source)?;
        snapshot.refresh_hash();
        let result = BlueprintPlanner.compile_blueprint_intent(
            IntentId::new(15),
            snapshot.anchor(),
            origin,
            template.clone(),
            &index(&snapshot)?,
        );
        assert_eq!(result.is_ok(), accepted, "source {source}");
    }
    Ok(())
}
