use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph};

const DIG_KEY: &str = "retained-excavation";
const AT: MapCoord = MapCoord::new(2, 2, 10);

fn world(shared: bool) -> Result<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    for (x, code) in [(0, tile_codes::SOLID_WALL), (1, tile_codes::FLOOR)] {
        let coord = ChunkCoord { x, y: 0, z: 10 };
        graph.chunks.insert(coord, uniform_chunk(coord, code));
    }
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(94),
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
    Ok(snapshot)
}

fn dig(mode: DigMode) -> Action {
    Action::DesignateDig {
        area: MapCuboid { min: AT, max: AT },
        mode,
    }
}

fn record_mut(snapshot: &mut WorldSnapshot) -> Result<&mut EntityRecord> {
    snapshot
        .graph
        .entities
        .get_mut(&created_entity_id(DIG_KEY, 0))
        .ok_or_else(|| precondition("test designation is missing"))
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    snapshot.tick = GameTick(snapshot.tick.0 + ticks);
    advance_effects(snapshot, ticks)?;
    snapshot.refresh_hash();
    Ok(())
}

#[test]
fn every_registered_excavation_mode_preserves_its_target_and_partitioned_service() -> Result<()> {
    for shared in [false, true] {
        for mode in [
            DigMode::Mine,
            DigMode::Channel,
            DigMode::UpStair,
            DigMode::DownStair,
            DigMode::UpDownStair,
            DigMode::Ramp,
            DigMode::RemoveConstruction,
        ] {
            let mut source = world(shared)?;
            apply_effect(&mut source, &dig(mode), DIG_KEY)?;
            let mut whole = source.clone();
            let mut split = source;
            advance(&mut whole, DIG_TICKS_PER_TILE)?;
            advance(&mut split, 3)?;
            advance(&mut split, DIG_TICKS_PER_TILE - 3)?;
            for snapshot in [&whole, &split] {
                assert_eq!(snapshot.tile_code_at(AT), Some(dig_target_tile_code(mode)));
                let record = entity(snapshot, created_entity_id(DIG_KEY, 0))?;
                assert_eq!(
                    field_text(record, STATUS_FIELD, snapshot.tick),
                    Some(STATUS_COMPLETE)
                );
                assert_eq!(field_u64(record, TILES_REMAINING_FIELD, snapshot.tick)?, 0);
                assert_eq!(field_u64(record, "work_ticks", snapshot.tick)?, 0);
            }
        }
    }
    Ok(())
}

#[test]
fn recovered_excavation_refuses_substituted_semantics_before_any_physical_event() -> Result<()> {
    for shared in [false, true] {
        let mut source = world(shared)?;
        apply_effect(&mut source, &dig(DigMode::Mine), DIG_KEY)?;
        let location = MapCoord::new(20, 2, 10);
        apply_effect(
            &mut source,
            &Action::Build {
                kind: BuildingKind::Workshop("Still".to_owned()),
                location,
                footprint: MapCuboid {
                    min: location,
                    max: location,
                },
                material: crate::MaterialSelector::default(),
            },
            "unrelated-construction",
        )?;
        for variant in 0..13 {
            let mut snapshot = source.clone();
            let tick = snapshot.tick;
            let record = record_mut(&mut snapshot)?;
            match variant {
                0 => {
                    record.fields.remove("mode");
                }
                1 => {
                    record.fields.insert(
                        "mode".to_owned(),
                        known(Value::Text("unknown".to_owned()), tick),
                    );
                }
                2 => {
                    record.fields.insert(
                        "mode".to_owned(),
                        known(Value::Text("channel".to_owned()), tick),
                    );
                }
                3 => {
                    record.fields.insert(
                        "target_tile_code".to_owned(),
                        known(Value::U64(u64::from(tile_codes::MAGMA_WALL)), tick),
                    );
                }
                4 => {
                    record.fields.remove("tiles_total");
                }
                5 => {
                    record
                        .fields
                        .insert("tiles_total".to_owned(), known(Value::U64(2), tick));
                }
                6 => {
                    record
                        .fields
                        .insert(TILES_REMAINING_FIELD.to_owned(), known(Value::U64(0), tick));
                }
                7 => {
                    record
                        .fields
                        .insert(TILES_REMAINING_FIELD.to_owned(), known(Value::U64(2), tick));
                }
                8 => {
                    record.fields.insert(
                        "work_ticks".to_owned(),
                        known(Value::U64(DIG_TICKS_PER_TILE), tick),
                    );
                }
                9 => {
                    record.generation = 2;
                }
                _ => {
                    let fact = record
                        .fields
                        .get_mut("mode")
                        .ok_or_else(|| precondition("test mode missing"))?;
                    match variant {
                        10 => fact.source = FactSource::Derived("unqualified".to_owned()),
                        11 => fact.presence = Some(FactPresence::Unknown("lost mode".to_owned())),
                        _ => fact.observed_at = GameTick(tick.0 + 1),
                    }
                }
            }
            snapshot.tick = GameTick(tick.0 + DIG_TICKS_PER_TILE);
            snapshot.refresh_hash();
            let before = snapshot.clone();
            // Directly inspect the effect engine, without a shadow wrapper
            // that could hide mutation before the eventual refusal.
            assert!(
                advance_effects(&mut snapshot, DIG_TICKS_PER_TILE)
                    .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed),
                "shared={shared}, variant={variant}",
            );
            assert_eq!(snapshot, before, "shared={shared}, variant={variant}");
        }
    }
    Ok(())
}

#[test]
fn construction_cannot_treat_a_substituted_channel_target_as_safe_mining() -> Result<()> {
    let mut snapshot = world(false)?;
    let location = MapCoord::new(20, 2, 10);
    let channel = Action::DesignateDig {
        area: MapCuboid {
            min: location,
            max: location,
        },
        mode: DigMode::Channel,
    };
    apply_effect(&mut snapshot, &channel, DIG_KEY)?;
    let tick = snapshot.tick;
    record_mut(&mut snapshot)?.fields.insert(
        "target_tile_code".to_owned(),
        known(Value::U64(u64::from(tile_codes::FLOOR)), tick),
    );
    snapshot.refresh_hash();
    let before = snapshot.clone();
    assert!(
        apply_effect(
            &mut snapshot,
            &Action::Build {
                kind: BuildingKind::Workshop("Still".to_owned()),
                location,
                footprint: MapCuboid {
                    min: location,
                    max: location
                },
                material: crate::MaterialSelector::default(),
            },
            "unsafe-workshop",
        )
        .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn current_terrain_can_finish_valid_retained_work_without_rewriting_its_original_counter()
-> Result<()> {
    for shared in [false, true] {
        let mut snapshot = world(shared)?;
        apply_effect(&mut snapshot, &dig(DigMode::Mine), DIG_KEY)?;
        advance(&mut snapshot, 3)?;
        snapshot.set_tile_code(AT, tile_codes::FLOOR)?;
        // Another valid actor can finish the physical terrain. The retained
        // previous sample is not required to match this fresh remaining count.
        advance(&mut snapshot, 1)?;
        let record = entity(&snapshot, created_entity_id(DIG_KEY, 0))?;
        assert_eq!(
            field_text(record, STATUS_FIELD, snapshot.tick),
            Some(STATUS_COMPLETE)
        );
        assert_eq!(field_u64(record, TILES_REMAINING_FIELD, snapshot.tick)?, 0);
    }
    Ok(())
}
