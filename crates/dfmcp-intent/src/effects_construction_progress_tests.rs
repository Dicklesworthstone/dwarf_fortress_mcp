use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph};

const LEDGER: EntityId = EntityId::new(91);

fn world() -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for z in [10, 11] {
        let chunk = ChunkCoord { x: 0, y: 0, z };
        graph
            .chunks
            .insert(chunk, uniform_chunk(chunk, tile_codes::FLOOR));
    }
    WorldSnapshot::new(
        FortressId::new(95),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

fn area(min: (i32, i32, i32), max: (i32, i32, i32)) -> Result<MapCuboid> {
    MapCuboid::new(
        MapCoord::new(min.0, min.1, min.2),
        MapCoord::new(max.0, max.1, max.2),
    )
}

fn build(footprint: MapCuboid) -> Action {
    Action::Build {
        kind: BuildingKind::Workshop("Still".to_owned()),
        location: footprint.max,
        footprint,
        material: crate::MaterialSelector::default(),
    }
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    apply_effect(snapshot, action, key)?;
    snapshot.refresh_hash();
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    snapshot.tick = GameTick(snapshot.tick.0 + ticks);
    advance_effects(snapshot, ticks)?;
    snapshot.refresh_hash();
    Ok(())
}

fn assert_advance_refused(
    snapshot: &mut WorldSnapshot,
    ticks: u64,
    expected: ErrorCode,
) -> Result<()> {
    snapshot.tick = GameTick(snapshot.tick.0 + ticks);
    snapshot.refresh_hash();
    let before = snapshot.clone();
    // The direct physical-engine call must refuse during admission. A caller
    // shadow would conceal partial construction, production or custody writes.
    let error = advance_effects(snapshot, ticks)
        .err()
        .ok_or_else(|| precondition("invalid construction unexpectedly advanced"))?;
    assert_eq!(error.code, expected);
    assert_eq!(*snapshot, before);
    Ok(())
}

fn put(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, Value)>,
) -> Result<()> {
    create_entity(
        snapshot,
        id,
        kind,
        "construction progress fixture".to_owned(),
        fields
            .into_iter()
            .map(|(field, value)| (field.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn record_mut(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<&mut EntityRecord> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| precondition("construction progress fixture missing"))
}

fn number(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, field, snapshot.tick)
}

#[test]
fn restored_construction_requires_floor_across_its_whole_footprint_before_progress() -> Result<()> {
    let footprint = area((2, 2, 10), (4, 4, 10))?;
    for progress in [0, 1, BUILD_TICKS - 1] {
        let mut source = world();
        apply(&mut source, &build(footprint), "construction")?;
        advance(&mut source, progress)?;
        for code in [
            tile_codes::OPEN_SPACE,
            tile_codes::SOLID_WALL,
            tile_codes::STAIR,
            tile_codes::UNKNOWN,
        ] {
            let mut snapshot = source.clone();
            // The location is footprint.max. A different footprint tile still
            // has to support construction, including on its final work tick.
            snapshot.set_tile_code(footprint.min, code)?;
            assert_advance_refused(&mut snapshot, 1, ErrorCode::PreconditionsFailed)?;
        }
        for variant in 0..5 {
            let mut snapshot = source.clone();
            let chunk = snapshot
                .graph
                .chunks
                .get_mut(&ChunkCoord { x: 0, y: 0, z: 10 })
                .ok_or_else(|| precondition("construction terrain fixture missing"))?;
            match variant {
                0 => chunk.terrain_runs.push(dfmcp_world::TerrainRun {
                    tile_code: tile_codes::FLOOR,
                    length: 0,
                }),
                1 => chunk.terrain_runs.extend(std::iter::repeat_n(
                    dfmcp_world::TerrainRun {
                        tile_code: tile_codes::FLOOR,
                        length: 0,
                    },
                    256,
                )),
                2 => chunk.coord.z = 11,
                3 => chunk.width = 15,
                _ => chunk.terrain_runs[0].length = 255,
            }
            assert_advance_refused(&mut snapshot, 1, ErrorCode::PreconditionsFailed)?;
        }
        let mut snapshot = source;
        snapshot
            .graph
            .chunks
            .remove(&ChunkCoord { x: 0, y: 0, z: 10 });
        assert_advance_refused(&mut snapshot, 1, ErrorCode::PreconditionsFailed)?;
    }
    Ok(())
}

#[test]
fn recovered_partial_overlap_refuses_at_every_occupied_neighbor_lifecycle() -> Result<()> {
    let original = area((2, 2, 10), (4, 4, 10))?;
    for (stage, progress) in [
        (STAGE_PLANNED, 0),
        (STAGE_UNDER_CONSTRUCTION, 1),
        (STAGE_COMPLETE, BUILD_TICKS),
    ] {
        let mut snapshot = world();
        apply(&mut snapshot, &build(original), "construction")?;
        apply(
            &mut snapshot,
            &build(area((7, 7, 10), (9, 9, 10))?),
            "neighbor",
        )?;
        // Restore a conflicting extent whose position, like the original
        // construction's position, is outside their one-cell intersection.
        write_fields(
            &mut snapshot,
            created_entity_id("neighbor", 0),
            vec![
                ("position".to_owned(), Value::Coord(MapCoord::new(0, 0, 10))),
                (
                    "footprint_min".to_owned(),
                    Value::Coord(MapCoord::new(0, 0, 10)),
                ),
                ("footprint_max".to_owned(), Value::Coord(original.min)),
                (
                    CONSTRUCTION_STAGE_FIELD.to_owned(),
                    Value::Text(stage.to_owned()),
                ),
                ("progress_ticks".to_owned(), Value::U64(progress)),
            ],
        )?;
        assert_advance_refused(&mut snapshot, BUILD_TICKS, ErrorCode::PreconditionsFailed)?;
    }
    Ok(())
}

#[test]
fn construction_progress_requires_qualified_position_and_complete_current_extent() -> Result<()> {
    let mut source = world();
    apply(
        &mut source,
        &build(area((2, 2, 10), (4, 4, 10))?),
        "construction",
    )?;
    let id = created_entity_id("construction", 0);
    for field in ["position", "footprint_min", "footprint_max"] {
        for variant in 0..6 {
            let mut snapshot = source.clone();
            let tick = snapshot.tick;
            let record = record_mut(&mut snapshot, id)?;
            if variant == 0 {
                record.fields.remove(field);
            } else {
                let fact = record
                    .fields
                    .get_mut(field)
                    .ok_or_else(|| precondition("construction geometry fact missing"))?;
                match variant {
                    1 => fact.presence = Some(FactPresence::Unknown("unobserved".to_owned())),
                    2 => fact.presence = Some(FactPresence::Known(Value::Null)),
                    3 => fact.source = FactSource::Derived("unqualified".to_owned()),
                    4 => fact.observed_at = GameTick(tick.0 + 1),
                    _ => fact.source_digest = Digest32::of_bytes(b"wrong source manifest"),
                }
            }
            assert_advance_refused(&mut snapshot, 1, ErrorCode::PreconditionsFailed)?;
        }
    }
    for (field, coord) in [
        ("position", MapCoord::new(8, 8, 10)),
        ("footprint_min", MapCoord::new(5, 2, 10)),
    ] {
        let mut snapshot = source.clone();
        write_fields(
            &mut snapshot,
            id,
            vec![(field.to_owned(), Value::Coord(coord))],
        )?;
        assert_advance_refused(&mut snapshot, 1, ErrorCode::PreconditionsFailed)?;
    }
    Ok(())
}

#[test]
fn released_sites_and_disjoint_levels_keep_valid_construction_progress() -> Result<()> {
    let mut snapshot = world();
    let footprint = area((2, 2, 10), (4, 4, 10))?;
    let original = build(footprint);
    apply(&mut snapshot, &original, "original")?;
    advance(&mut snapshot, 1)?;
    cancel_effect(&mut snapshot, &original, "original")?;
    let cancelled = entity(&snapshot, created_entity_id("original", 0))?.clone();
    for (key, footprint) in [
        ("replacement", footprint),
        ("adjacent", area((5, 2, 10), (7, 4, 10))?),
        ("other-level", area((2, 2, 11), (4, 4, 11))?),
    ] {
        apply(&mut snapshot, &build(footprint), key)?;
    }
    advance(&mut snapshot, BUILD_TICKS - 1)?;
    advance(&mut snapshot, 1)?;
    for key in ["replacement", "adjacent", "other-level"] {
        let record = entity(&snapshot, created_entity_id(key, 0))?;
        assert_eq!(
            field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick),
            Some(STAGE_COMPLETE)
        );
        assert_eq!(
            field_u64(record, "progress_ticks", snapshot.tick)?,
            BUILD_TICKS
        );
    }
    assert_eq!(
        entity(&snapshot, created_entity_id("original", 0))?,
        &cancelled
    );
    Ok(())
}

fn material_world() -> Result<WorldSnapshot> {
    let mut snapshot = world();
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    put(
        &mut snapshot,
        settings,
        EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
        vec![
            (
                LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            ),
            (
                LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                Value::Text(LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
            ),
        ],
    )?;
    for (id, construction, carpentry) in [(11, true, false), (12, false, true)] {
        put(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.CONSTRUCTION", Value::Bool(construction)),
                ("labor.CARPENTRY", Value::Bool(carpentry)),
                (SQUAD_FIELD, Value::Null),
            ],
        )?;
    }
    let at = MapCoord::new(0, 0, 10);
    put(
        &mut snapshot,
        EntityId::new(51),
        EntityKind::Building,
        vec![
            (
                "building_kind",
                Value::Text("workshop:Carpenters".to_owned()),
            ),
            (
                CONSTRUCTION_STAGE_FIELD,
                Value::Text(STAGE_COMPLETE.to_owned()),
            ),
            ("position", Value::Coord(at)),
            ("footprint_min", Value::Coord(at)),
            ("footprint_max", Value::Coord(at)),
        ],
    )?;
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        vec![
            (STOCK_DRINK_FIELD, Value::U64(100)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
            (STOCK_WOOD_FIELD, Value::U64(2)),
            (STOCK_BED_FIELD, Value::U64(0)),
        ],
    )?;
    Ok(snapshot)
}

#[test]
fn invalid_construction_preserves_held_material_and_unrelated_production_before_events()
-> Result<()> {
    let footprint = area((2, 2, 10), (4, 4, 10))?;
    let action = build(footprint);
    let mut source = material_world()?;
    apply(&mut source, &action, "construction")?;
    apply(
        &mut source,
        &Action::CreateWorkOrder {
            name: "independent bed".to_owned(),
            job_token: "CONSTRUCT_BED".to_owned(),
            amount: 1,
            conditions: Vec::new(),
        },
        "production",
    )?;
    let construction = created_entity_id("construction", 0);
    let production = created_entity_id("production", 0);
    // Establish that both effects really can finish in this interval before
    // introducing either spatial fault into the otherwise identical source.
    let mut valid = source.clone();
    advance(&mut valid, BUILD_TICKS)?;
    assert_eq!(number(&valid, production, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&valid, LEDGER, STOCK_BED_FIELD)?, 1);
    assert_eq!(
        field_text(
            entity(&valid, construction)?,
            CONSTRUCTION_MATERIAL_STATE_FIELD,
            valid.tick
        ),
        Some("consumed")
    );

    for overlap in [false, true] {
        let mut snapshot = source.clone();
        if overlap {
            put(
                &mut snapshot,
                EntityId::new(52),
                EntityKind::Building,
                vec![
                    ("building_kind", Value::Text("workshop:Kitchen".to_owned())),
                    (
                        CONSTRUCTION_STAGE_FIELD,
                        Value::Text(STAGE_COMPLETE.to_owned()),
                    ),
                    ("position", Value::Coord(footprint.min)),
                    ("footprint_min", Value::Coord(footprint.min)),
                    ("footprint_max", Value::Coord(footprint.min)),
                ],
            )?;
        } else {
            snapshot.set_tile_code(footprint.min, tile_codes::OPEN_SPACE)?;
        }
        assert_advance_refused(&mut snapshot, BUILD_TICKS, ErrorCode::PreconditionsFailed)?;
        let report = construction_material_report(&snapshot)?
            .ok_or_else(|| precondition("construction material report missing"))?;
        assert_eq!(
            (
                report.available_wood,
                report.held_wood,
                report.consumed_wood
            ),
            (1, 1, 0)
        );
        assert_eq!(number(&snapshot, construction, "progress_ticks")?, 0);
        assert_eq!(number(&snapshot, production, AMOUNT_REMAINING_FIELD)?, 1);
        assert_eq!(number(&snapshot, production, "work_ticks")?, 0);
        assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 0);
        // Explicit cancellation still releases the held log despite a changed
        // site, and the independent production can subsequently use inventory.
        cancel_effect(&mut snapshot, &action, "construction")?;
        assert_eq!(number(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 2);
        advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
        assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
        assert_eq!(number(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 1);
    }
    Ok(())
}

#[test]
fn progress_charges_terrain_and_complete_overlap_scan_before_visiting_tiles() -> Result<()> {
    let mut snapshot = world();
    let footprint = area((2, 2, 10), (3, 3, 10))?;
    apply(&mut snapshot, &build(footprint), "construction")?;
    let id = created_entity_id("construction", 0);
    snapshot
        .graph
        .chunks
        .get_mut(&ChunkCoord { x: 0, y: 0, z: 10 })
        .ok_or_else(|| precondition("construction terrain fixture missing"))?
        .terrain_runs = vec![
        dfmcp_world::TerrainRun {
            tile_code: tile_codes::FLOOR,
            length: 1,
        };
        256
    ];
    snapshot.refresh_hash();
    let occupied = dfmcp_world::laboratory_building_footprints(&snapshot)?;
    let mut budget = EffectAdvanceBudget::new(
        EffectAdvanceLimits {
            max_work_units: 4 * 320,
            ..EffectAdvanceLimits::default()
        },
        0,
    )?;
    // Four bounded terrain reads plus the complete one-record overlap census.
    assert!(
        occupancy::validate_progress(&snapshot, entity(&snapshot, id)?, &occupied, &mut budget)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(budget.work_units, 0);
    let mut budget = EffectAdvanceBudget::new(
        EffectAdvanceLimits {
            max_work_units: 4 * 320 + 1,
            ..EffectAdvanceLimits::default()
        },
        0,
    )?;
    occupancy::validate_progress(&snapshot, entity(&snapshot, id)?, &occupied, &mut budget)?;
    assert_eq!(budget.work_units, 4 * 320 + 1);
    write_fields(
        &mut snapshot,
        id,
        vec![
            ("position".to_owned(), Value::Coord(MapCoord::new(0, 0, 10))),
            (
                "footprint_min".to_owned(),
                Value::Coord(MapCoord::new(0, 0, 10)),
            ),
            (
                "footprint_max".to_owned(),
                Value::Coord(MapCoord::new(65_536, 0, 10)),
            ),
        ],
    )?;
    assert_advance_refused(&mut snapshot, 1, ErrorCode::BudgetExceeded)
}
