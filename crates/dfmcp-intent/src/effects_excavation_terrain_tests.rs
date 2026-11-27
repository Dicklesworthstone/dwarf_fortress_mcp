//! Physical terrain conservation through dispatch, retained work and an event
//! shared by competing designations. These tests use the actual effect engine.

use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, WorldGraph};

const AT: MapCoord = MapCoord::new(2, 2, 10);
const MODES: [DigMode; 7] = [
    DigMode::Mine,
    DigMode::Channel,
    DigMode::UpStair,
    DigMode::DownStair,
    DigMode::UpDownStair,
    DigMode::Ramp,
    DigMode::RemoveConstruction,
];

fn world(shared: bool) -> Result<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(chunk, uniform_chunk(chunk, tile_codes::SOLID_WALL));
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(942),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    if shared {
        create_entity(
            &mut snapshot,
            EntityId::new(1),
            EntityKind::Unit,
            "miner and builder".to_owned(),
            vec![
                ("alive".to_owned(), Value::Bool(true)),
                ("labor.MINING".to_owned(), Value::Bool(true)),
                ("labor.CONSTRUCTION".to_owned(), Value::Bool(true)),
            ],
        )?;
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        create_entity(
            &mut snapshot,
            settings,
            EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
            "settings".to_owned(),
            vec![(
                LABORATORY_WORKFORCE_MODEL_FIELD.to_owned(),
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            )],
        )?;
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn dig(mode: DigMode) -> Action {
    Action::DesignateDig {
        area: MapCuboid { min: AT, max: AT },
        mode,
    }
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    let mut shadow = snapshot.clone();
    shadow.tick = GameTick(snapshot.tick.0 + ticks);
    advance_effects(&mut shadow, ticks)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

#[test]
fn excavation_cannot_manufacture_ground_or_erase_hazardous_and_unsupported_terrain() -> Result<()> {
    for shared in [false, true] {
        for mode in MODES {
            for code in [
                tile_codes::OPEN_SPACE,
                tile_codes::CHASM,
                tile_codes::TREE,
                tile_codes::MAGMA_WALL,
                99,
                tile_codes::UNKNOWN,
            ] {
                if mode == DigMode::Channel && code == tile_codes::OPEN_SPACE {
                    continue; // Already-excavated empty space is a valid no-op.
                }
                let mut snapshot = world(shared)?;
                snapshot.set_tile_code(AT, code)?;
                snapshot.refresh_hash();
                let before = snapshot.clone();
                let result = apply_effect(&mut snapshot, &dig(mode), "invalid-terrain");
                assert!(
                    result.is_err_and(|error| error.code == ErrorCode::PreconditionsFailed),
                    "shared={shared}, mode={mode:?}, source={code}",
                );
                assert_eq!(snapshot, before, "rejected designation must be inert");
            }
        }
    }
    Ok(())
}

#[test]
fn a_changed_retained_excavation_source_refuses_before_unrelated_physical_work() -> Result<()> {
    for shared in [false, true] {
        for mode in MODES {
            let mut source = world(shared)?;
            apply_effect(&mut source, &dig(mode), "retained-dig")?;
            advance(&mut source, 3)?;
            let site = MapCoord::new(8, 8, 10);
            source.set_tile_code(site, tile_codes::FLOOR)?;
            apply_effect(
                &mut source,
                &Action::Build {
                    kind: BuildingKind::Workshop("Still".to_owned()),
                    location: site,
                    footprint: MapCuboid {
                        min: site,
                        max: site,
                    },
                    material: crate::MaterialSelector::default(),
                },
                "unrelated-building",
            )?;
            for code in [
                tile_codes::OPEN_SPACE,
                tile_codes::CHASM,
                tile_codes::TREE,
                tile_codes::MAGMA_WALL,
                99,
            ] {
                if mode == DigMode::Channel && code == tile_codes::OPEN_SPACE {
                    continue;
                }
                let mut changed = source.clone();
                changed.set_tile_code(AT, code)?;
                changed.tick = GameTick(changed.tick.0 + BUILD_TICKS);
                changed.refresh_hash();
                let before = changed.clone();
                // Inspect the engine directly: a transactional wrapper would
                // hide unrelated writes made before source admission failed.
                assert!(
                    advance_effects(&mut changed, BUILD_TICKS)
                        .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed),
                    "shared={shared}, mode={mode:?}, source={code}",
                );
                assert_eq!(changed, before);
            }
        }
    }
    Ok(())
}

#[test]
fn competing_channel_cannot_be_replaced_with_new_ground_inside_one_event() -> Result<()> {
    // Both actions are individually valid at their original ordinary wall.
    // Canonical order makes the channel finish first within the shared event.
    let keys = ["simultaneous-first", "simultaneous-second"];
    let (channel_key, later_key) = if created_entity_id(keys[0], 0) < created_entity_id(keys[1], 0)
    {
        (keys[0], keys[1])
    } else {
        (keys[1], keys[0])
    };
    for mode in [
        DigMode::Mine,
        DigMode::UpStair,
        DigMode::Ramp,
        DigMode::RemoveConstruction,
    ] {
        let mut snapshot = world(false)?;
        apply_effect(&mut snapshot, &dig(DigMode::Channel), channel_key)?;
        apply_effect(&mut snapshot, &dig(mode), later_key)?;
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            advance(&mut snapshot, DIG_TICKS_PER_TILE)
                .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed),
            "competing mode {mode:?} must recheck its current cell",
        );
        assert_eq!(
            snapshot, before,
            "the transaction must publish no partial event"
        );
        assert!(cancel_effect(
            &mut snapshot,
            &dig(DigMode::Channel),
            channel_key
        )?);
        advance(&mut snapshot, DIG_TICKS_PER_TILE)?;
        assert_eq!(snapshot.tile_code_at(AT), Some(dig_target_tile_code(mode)));
    }
    Ok(())
}

#[test]
fn supported_excavation_shapes_and_noop_targets_preserve_partitioned_service() -> Result<()> {
    // This is an independent table of the existing coarse laboratory terrain
    // model, not a claim about native material origin or stair direction.
    let allowed = [
        (
            DigMode::Mine,
            vec![
                tile_codes::SOLID_WALL,
                tile_codes::FORTIFICATION,
                tile_codes::STAIR,
                tile_codes::RAMP,
            ],
        ),
        (
            DigMode::RemoveConstruction,
            vec![
                tile_codes::SOLID_WALL,
                tile_codes::FORTIFICATION,
                tile_codes::STAIR,
                tile_codes::RAMP,
            ],
        ),
        (
            DigMode::Channel,
            vec![
                tile_codes::SOLID_WALL,
                tile_codes::FLOOR,
                tile_codes::FORTIFICATION,
                tile_codes::STAIR,
                tile_codes::RAMP,
            ],
        ),
        (
            DigMode::DownStair,
            vec![tile_codes::SOLID_WALL, tile_codes::FLOOR],
        ),
        (DigMode::UpStair, vec![tile_codes::SOLID_WALL]),
        (DigMode::UpDownStair, vec![tile_codes::SOLID_WALL]),
        (DigMode::Ramp, vec![tile_codes::SOLID_WALL]),
    ];
    for shared in [false, true] {
        for (mode, codes) in &allowed {
            for code in codes {
                let mut source = world(shared)?;
                source.set_tile_code(AT, *code)?;
                apply_effect(&mut source, &dig(*mode), "supported-dig")?;
                let mut whole = source.clone();
                let mut split = source;
                advance(&mut whole, DIG_TICKS_PER_TILE)?;
                advance(&mut split, 3)?;
                advance(&mut split, DIG_TICKS_PER_TILE - 3)?;
                for snapshot in [&whole, &split] {
                    assert_eq!(snapshot.tile_code_at(AT), Some(dig_target_tile_code(*mode)));
                    let record = entity(snapshot, created_entity_id("supported-dig", 0))?;
                    assert_eq!(
                        field_text(record, STATUS_FIELD, snapshot.tick),
                        Some(STATUS_COMPLETE)
                    );
                    assert_eq!(field_u64(record, TILES_REMAINING_FIELD, snapshot.tick)?, 0);
                    assert_eq!(field_u64(record, "work_ticks", snapshot.tick)?, 0);
                }
            }
        }
        for mode in MODES {
            let mut snapshot = world(shared)?;
            snapshot.set_tile_code(AT, dig_target_tile_code(mode))?;
            let terrain = snapshot.graph.chunks.clone();
            apply_effect(&mut snapshot, &dig(mode), "already-done")?;
            advance(&mut snapshot, DIG_TICKS_PER_TILE)?;
            assert_eq!(snapshot.graph.chunks, terrain);
            assert_eq!(
                field_text(
                    entity(&snapshot, created_entity_id("already-done", 0))?,
                    STATUS_FIELD,
                    snapshot.tick
                ),
                Some(STATUS_COMPLETE),
            );
        }
    }
    Ok(())
}

#[test]
fn excavation_does_not_add_upward_structure_to_a_floor() -> Result<()> {
    for mode in [DigMode::UpStair, DigMode::UpDownStair, DigMode::Ramp] {
        let mut snapshot = world(false)?;
        snapshot.set_tile_code(AT, tile_codes::FLOOR)?;
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            apply_effect(&mut snapshot, &dig(mode), "floor-is-not-wall")
                .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed),
        );
        assert_eq!(snapshot, before);
    }
    Ok(())
}
