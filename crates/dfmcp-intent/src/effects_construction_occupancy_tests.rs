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
        FortressId::new(93),
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

fn build(footprint: MapCuboid, workshop: &str) -> Action {
    Action::Build {
        kind: BuildingKind::Workshop(workshop.to_owned()),
        location: footprint.max,
        footprint,
        material: crate::MaterialSelector::default(),
    }
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

fn put(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: &[(&str, Value)],
) -> Result<()> {
    create_entity(
        snapshot,
        id,
        kind,
        "construction occupancy fixture".to_owned(),
        fields
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
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
        .ok_or_else(|| precondition("construction occupancy test record missing"))
}

fn assert_refused(snapshot: &mut WorldSnapshot, action: &Action) {
    let before = snapshot.clone();
    assert!(
        apply(snapshot, action, "refused-construction")
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    assert_eq!(*snapshot, before);
}

#[test]
fn construction_checks_whole_footprints_at_every_occupied_lifecycle() -> Result<()> {
    let original = build(area((1, 1, 10), (3, 3, 10))?, "Still");
    let overlap = build(area((3, 3, 10), (5, 5, 10))?, "Kitchen");
    for progress in [0, 1, BUILD_TICKS] {
        let mut snapshot = world();
        apply(&mut snapshot, &original, "original")?;
        advance(&mut snapshot, progress)?;
        // The requested location (5,5,10) is free. Its footprint still
        // intersects the original at the inclusive corner (3,3,10).
        assert_refused(&mut snapshot, &overlap);
        assert_refused(&mut snapshot, &original);
        assert_refused(
            &mut snapshot,
            &build(area((2, 2, 10), (2, 2, 10))?, "Kitchen"),
        );
    }
    Ok(())
}

#[test]
fn adjacent_and_other_level_sites_work_and_only_unfinished_cancellation_releases_ground()
-> Result<()> {
    let original = build(area((1, 1, 10), (3, 3, 10))?, "Still");
    for complete in [false, true] {
        let mut snapshot = world();
        apply(&mut snapshot, &original, "original")?;
        advance(&mut snapshot, if complete { BUILD_TICKS } else { 1 })?;
        cancel_effect(&mut snapshot, &original, "original")?;
        snapshot.refresh_hash();
        if complete {
            assert_refused(&mut snapshot, &original);
        } else {
            apply(&mut snapshot, &original, "replacement")?;
        }
        apply(
            &mut snapshot,
            &build(area((4, 1, 10), (6, 3, 10))?, "Kitchen"),
            "adjacent",
        )?;
        apply(
            &mut snapshot,
            &build(area((1, 1, 11), (3, 3, 11))?, "Kitchen"),
            "other-level",
        )?;
    }
    Ok(())
}

#[test]
fn construction_requires_complete_current_source_qualified_occupancy() -> Result<()> {
    let original = build(area((1, 1, 10), (3, 3, 10))?, "Still");
    let requested = build(area((8, 8, 10), (9, 9, 10))?, "Kitchen");
    let id = created_entity_id("original", 0);
    let mut source = world();
    apply(&mut source, &original, "original")?;
    for variant in 0..12 {
        let mut snapshot = source.clone();
        let tick = snapshot.tick;
        let record = record_mut(&mut snapshot, id)?;
        match variant {
            0 => {
                record.fields.remove("footprint_min");
            }
            1 => {
                record.fields.remove("footprint_max");
            }
            2 => {
                record.fields.remove(CONSTRUCTION_STAGE_FIELD);
            }
            3 => {
                record.fields.insert(
                    CONSTRUCTION_STAGE_FIELD.to_owned(),
                    known(Value::Text("unknown-phase".to_owned()), tick),
                );
            }
            4 => {
                record.fields.insert(
                    "footprint_min".to_owned(),
                    known(Value::Coord(MapCoord::new(4, 4, 10)), tick),
                );
            }
            5 => record.id = EntityId::new(7),
            _ => {
                let field = record
                    .fields
                    .get_mut("footprint_max")
                    .ok_or_else(|| precondition("occupancy test extent missing"))?;
                match variant {
                    6 => field.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                    7 => field.observed_at = GameTick(tick.0 + 1),
                    8 => field.source = FactSource::Derived("untrusted-model".to_owned()),
                    9 => field.source_digest = Digest32::of_bytes(b"different producer"),
                    10 => field.presence = Some(FactPresence::Redacted("withheld".to_owned())),
                    _ => {
                        field.presence =
                            Some(FactPresence::Known(Value::Coord(MapCoord::new(9, 9, 10))));
                    }
                }
            }
        }
        snapshot.refresh_hash();
        assert_refused(&mut snapshot, &requested);
    }
    Ok(())
}

#[test]
fn canonical_building_alias_cannot_hide_from_construction_or_exact_furniture() -> Result<()> {
    let footprint = area((1, 1, 10), (3, 3, 10))?;
    let mut snapshot = world();
    apply(&mut snapshot, &build(footprint, "Still"), "original")?;
    let hash = snapshot.state_hash;
    record_mut(&mut snapshot, created_entity_id("original", 0))?.kind =
        EntityKind::Other("building".to_owned());
    snapshot.refresh_hash();
    assert_eq!(snapshot.state_hash, hash);
    assert_refused(&mut snapshot, &build(footprint, "Kitchen"));
    assert!(
        validate_furniture_location(&snapshot, MapCoord::new(2, 2, 10))
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    validate_furniture_location(&snapshot, MapCoord::new(8, 8, 10))?;
    Ok(())
}

#[test]
fn refusing_stacked_workshops_cannot_manufacture_parallel_production_capacity() -> Result<()> {
    let mut snapshot = world();
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    put(
        &mut snapshot,
        settings,
        EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
        &[(
            LABORATORY_WORKFORCE_MODEL_FIELD,
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        )],
    )?;
    for id in [11, 12] {
        put(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Unit,
            &[
                ("alive", Value::Bool(true)),
                ("labor.BREW", Value::Bool(true)),
                ("labor.CONSTRUCTION", Value::Bool(true)),
            ],
        )?;
    }
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        &[
            (STOCK_DRINK_FIELD, Value::U64(0)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    )?;
    let workshop = build(area((1, 1, 10), (3, 3, 10))?, "Still");
    apply(&mut snapshot, &workshop, "first-still")?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_refused(&mut snapshot, &workshop);
    for key in ["first-order", "second-order"] {
        apply(
            &mut snapshot,
            &Action::CreateWorkOrder {
                name: key.to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            key,
        )?;
    }
    let report = laboratory_workforce_report(&snapshot)?
        .ok_or_else(|| precondition("test workforce report missing"))?;
    assert_eq!(report.assignments.len(), 1);
    assert_eq!(report.blocked.len(), 1);
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    assert_eq!(
        field_u64(entity(&snapshot, LEDGER)?, STOCK_DRINK_FIELD, snapshot.tick)?,
        5,
    );
    let mut remaining = 0;
    for key in ["first-order", "second-order"] {
        remaining += field_u64(
            entity(&snapshot, created_entity_id(key, 0))?,
            AMOUNT_REMAINING_FIELD,
            snapshot.tick,
        )?;
    }
    assert_eq!(remaining, 1);
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    assert_eq!(
        field_u64(entity(&snapshot, LEDGER)?, STOCK_DRINK_FIELD, snapshot.tick)?,
        10,
    );
    Ok(())
}

#[test]
fn occupancy_admits_only_its_bounded_complete_domain_before_new_building_creation() -> Result<()> {
    let mut snapshot = world();
    for id in 1..=MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES as u64 {
        let id = EntityId::new(id);
        snapshot.graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Other("non-building".to_owned()),
                label: String::new(),
                fields: BTreeMap::new(),
            },
        );
    }
    let footprint = area((1, 1, 10), (1, 1, 10))?;
    assert!(
        apply_effect(&mut snapshot, &build(footprint, "Still"), "at-capacity")
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    let id = EntityId::new(MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES as u64 + 1);
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind: EntityKind::Other("non-building".to_owned()),
            label: String::new(),
            fields: BTreeMap::new(),
        },
    );
    assert!(
        validate_furniture_location(&snapshot, footprint.min)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}
