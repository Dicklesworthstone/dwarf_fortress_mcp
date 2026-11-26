use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph};

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for z in [10, 11] {
        let chunk = ChunkCoord { x: 0, y: 0, z };
        graph
            .chunks
            .insert(chunk, uniform_chunk(chunk, tile_codes::FLOOR));
    }
    WorldSnapshot::new(
        FortressId::new(94),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

fn region(min: MapCoord, max: MapCoord) -> Result<MapCuboid> {
    MapCuboid::new(min, max)
}

fn tile(at: MapCoord) -> MapCuboid {
    MapCuboid { min: at, max: at }
}

fn building(at: MapCoord) -> Action {
    Action::Build {
        kind: BuildingKind::Workshop("Still".to_owned()),
        location: at,
        footprint: tile(at),
        material: crate::MaterialSelector::default(),
    }
}

fn dig(area: MapCuboid, mode: DigMode) -> Action {
    Action::DesignateDig { area, mode }
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    let mut shadow = snapshot.clone();
    apply_effect(&mut shadow, action, key)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    let mut shadow = snapshot.clone();
    shadow.tick = GameTick(snapshot.tick.0 + ticks);
    advance_effects(&mut shadow, ticks)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, name: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(name.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn assert_refused(snapshot: &mut WorldSnapshot, action: &Action, key: &str) {
    let before = snapshot.clone();
    assert!(
        apply(snapshot, action, key)
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    assert_eq!(*snapshot, before);
}

#[test]
fn excavation_cannot_change_occupied_terrain_at_any_building_lifecycle() -> Result<()> {
    let at = MapCoord::new(2, 2, 10);
    for progress in [0, 1, BUILD_TICKS] {
        for mode in [
            DigMode::Channel,
            DigMode::UpStair,
            DigMode::DownStair,
            DigMode::UpDownStair,
            DigMode::Ramp,
        ] {
            let mut snapshot = world();
            apply(&mut snapshot, &building(at), "workshop")?;
            advance(&mut snapshot, progress)?;
            assert_refused(&mut snapshot, &dig(tile(at), mode), "unsafe-dig");
            assert_eq!(snapshot.tile_code_at(at), Some(tile_codes::FLOOR));
        }
    }
    Ok(())
}

#[test]
fn room_repair_and_already_complete_mining_preserve_installed_occupancy() -> Result<()> {
    let mut snapshot = world();
    let at = MapCoord::new(2, 2, 10);
    apply(&mut snapshot, &building(at), "workshop")?;
    advance(&mut snapshot, BUILD_TICKS)?;
    let id = created_entity_id("workshop", 0);
    let original = entity(&snapshot, id)?.clone();
    apply(
        &mut snapshot,
        &dig(tile(at), DigMode::Mine),
        "already-floor",
    )?;
    assert_eq!(
        field_text(
            entity(&snapshot, created_entity_id("already-floor", 0))?,
            STATUS_FIELD,
            snapshot.tick
        ),
        Some(STATUS_COMPLETE),
    );
    let area = region(MapCoord::new(1, 2, 10), MapCoord::new(3, 2, 10))?;
    snapshot.set_tile_code(area.min, tile_codes::SOLID_WALL)?;
    snapshot.set_tile_code(area.max, tile_codes::SOLID_WALL)?;
    snapshot.refresh_hash();
    apply(&mut snapshot, &dig(area, DigMode::Mine), "repair")?;
    advance(&mut snapshot, 2 * DIG_TICKS_PER_TILE)?;
    assert!(
        region_tiles(area).all(|coord| snapshot.tile_code_at(coord) == Some(tile_codes::FLOOR))
    );
    assert_eq!(entity(&snapshot, id)?, &original);
    Ok(())
}

#[test]
fn accepted_channel_blocks_build_and_furniture_until_explicit_cancellation() -> Result<()> {
    let mut snapshot = world();
    let at = MapCoord::new(2, 2, 10);
    let channel = dig(tile(at), DigMode::Channel);
    apply(&mut snapshot, &channel, "channel")?;
    assert_refused(&mut snapshot, &building(at), "blocked-workshop");
    assert!(
        validate_furniture_location(&snapshot, at)
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    assert_eq!(snapshot.tile_code_at(at), Some(tile_codes::FLOOR));
    cancel_effect(&mut snapshot, &channel, "channel")?;
    snapshot.refresh_hash();
    apply(&mut snapshot, &building(at), "after-cancel")?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(
        field_text(
            entity(&snapshot, created_entity_id("after-cancel", 0))?,
            CONSTRUCTION_STAGE_FIELD,
            snapshot.tick
        ),
        Some(STAGE_COMPLETE),
    );
    Ok(())
}

#[test]
fn construction_can_coexist_with_safe_mining_and_disjoint_excavation() -> Result<()> {
    let mut snapshot = world();
    let at = MapCoord::new(2, 2, 10);
    let west = MapCoord::new(1, 2, 10);
    snapshot.set_tile_code(west, tile_codes::SOLID_WALL)?;
    snapshot.refresh_hash();
    apply(
        &mut snapshot,
        &dig(region(west, at)?, DigMode::Mine),
        "room-mine",
    )?;
    apply(&mut snapshot, &building(at), "workshop")?;
    // A different level is independent even at identical horizontal coordinates.
    let upper = MapCoord::new(2, 2, 11);
    apply(
        &mut snapshot,
        &dig(tile(upper), DigMode::Channel),
        "upper-channel",
    )?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_eq!(snapshot.tile_code_at(west), Some(tile_codes::FLOOR));
    assert_eq!(snapshot.tile_code_at(at), Some(tile_codes::FLOOR));
    assert_eq!(snapshot.tile_code_at(upper), Some(tile_codes::OPEN_SPACE));
    Ok(())
}

#[test]
fn recovered_overlap_refuses_before_unrelated_timeline_effects() -> Result<()> {
    let mut snapshot = world();
    let at = MapCoord::new(2, 2, 10);
    let other = MapCoord::new(8, 8, 10);
    snapshot.set_tile_code(other, tile_codes::SOLID_WALL)?;
    snapshot.refresh_hash();
    apply(
        &mut snapshot,
        &dig(tile(at), DigMode::Channel),
        "old-channel",
    )?;
    apply(
        &mut snapshot,
        &dig(tile(other), DigMode::Mine),
        "other-mine",
    )?;
    // Represent an old or externally changed snapshot containing an overlap.
    // Current dispatch refuses this state, but recovery may encounter it.
    let id = EntityId::new(800);
    create_entity(
        &mut snapshot,
        id,
        EntityKind::Building,
        "existing workshop".to_owned(),
        vec![
            (
                "building_kind".to_owned(),
                Value::Text("workshop:Still".to_owned()),
            ),
            ("position".to_owned(), Value::Coord(at)),
            ("footprint_min".to_owned(), Value::Coord(at)),
            ("footprint_max".to_owned(), Value::Coord(at)),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_COMPLETE.to_owned()),
            ),
            ("required_ticks".to_owned(), Value::U64(BUILD_TICKS)),
            ("progress_ticks".to_owned(), Value::U64(BUILD_TICKS)),
        ],
    )?;
    snapshot.tick = GameTick(snapshot.tick.0 + DIG_TICKS_PER_TILE);
    snapshot.refresh_hash();
    let before = snapshot.clone();
    // Call the physical engine directly: a transactional test wrapper would
    // hide partial mutations and fail to verify admission before every event.
    assert!(
        advance_effects(&mut snapshot, DIG_TICKS_PER_TILE)
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    assert_eq!(snapshot, before);
    assert_eq!(snapshot.tile_code_at(other), Some(tile_codes::SOLID_WALL));
    Ok(())
}

#[test]
fn occupancy_uncertainty_and_canonical_aliases_cannot_authorize_excavation() -> Result<()> {
    let at = MapCoord::new(2, 2, 10);
    let mut source = world();
    apply(&mut source, &building(at), "workshop")?;
    advance(&mut source, BUILD_TICKS)?;
    let id = created_entity_id("workshop", 0);
    for variant in 0..5 {
        let mut snapshot = source.clone();
        let tick = snapshot.tick;
        let record = snapshot
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| precondition("excavation test building missing"))?;
        match variant {
            0 => record.kind = EntityKind::Other("building".to_owned()),
            1 => {
                record.fields.remove("footprint_min");
            }
            _ => {
                let fact = record
                    .fields
                    .get_mut("footprint_max")
                    .ok_or_else(|| precondition("excavation test extent missing"))?;
                match variant {
                    2 => fact.observed_at = GameTick(tick.0 + 1),
                    3 => fact.presence = Some(FactPresence::Unknown("unobserved".to_owned())),
                    _ => fact.source = FactSource::Derived("unqualified".to_owned()),
                }
            }
        }
        snapshot.refresh_hash();
        assert_refused(&mut snapshot, &dig(tile(at), DigMode::Channel), "unsafe");
    }
    Ok(())
}

#[test]
fn construction_requires_current_dig_lifecycle_extent_and_relevant_target() -> Result<()> {
    let at = MapCoord::new(2, 2, 10);
    let mut source = world();
    apply(&mut source, &dig(tile(at), DigMode::Channel), "channel")?;
    let id = created_entity_id("channel", 0);
    for field in [STATUS_FIELD, "area_min", "area_max", "target_tile_code"] {
        let mut snapshot = source.clone();
        snapshot
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| precondition("excavation test designation missing"))?
            .fields
            .remove(field);
        snapshot.refresh_hash();
        assert_refused(&mut snapshot, &building(at), "unsafe");
    }
    // A complete qualified extent proves that an unavailable target cannot
    // affect a distant new building. No terrain/effect claim uses that target.
    source
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| precondition("excavation test designation missing"))?
        .fields
        .remove("target_tile_code");
    source.refresh_hash();
    apply(&mut source, &building(MapCoord::new(8, 8, 10)), "distant")?;
    set(
        &mut source,
        id,
        STATUS_FIELD,
        Value::Text(STATUS_CANCELLED.to_owned()),
    )?;
    apply(&mut source, &building(at), "released")?;
    Ok(())
}

#[test]
fn excavation_mask_work_is_bounded_before_allocation_and_skips_disjoint_grids() -> Result<()> {
    let area = region(MapCoord::new(0, 0, 10), MapCoord::new(15, 15, 10))?;
    let buildings = BTreeMap::from([(EntityId::new(1), tile(area.min))]);
    let pending = BTreeSet::from([(10, 15, 15)]);
    let mut budget = EffectAdvanceBudget::new(
        EffectAdvanceLimits {
            max_work_units: 256 * 40 + 16,
            ..EffectAdvanceLimits::default()
        },
        0,
    )?;
    assert!(
        excavation_occupancy::validate_pending(area, &pending, &buildings, &mut budget)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    let disjoint = BTreeMap::from([(EntityId::new(1), tile(MapCoord::new(0, 0, 11)))]);
    let mut budget = EffectAdvanceBudget::new(
        EffectAdvanceLimits {
            max_work_units: 1,
            ..EffectAdvanceLimits::default()
        },
        0,
    )?;
    excavation_occupancy::validate_pending(area, &pending, &disjoint, &mut budget)?;
    assert_eq!(budget.work_units, 1);
    Ok(())
}
