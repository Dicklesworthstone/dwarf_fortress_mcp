use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph, evaluate};

const WORKER: EntityId = EntityId::new(11);
const WORKSHOP: EntityId = EntityId::new(51);
const LEDGER: EntityId = EntityId::new(91);
const TARGET: MapCoord = MapCoord::new(4, 4, 10);

fn put(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, Value)>,
) -> Result<()> {
    create_entity(
        snapshot,
        id,
        kind.clone(),
        kind.as_str().to_owned(),
        fields
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn world() -> Result<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    let at = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(at, uniform_chunk(at, tile_codes::FLOOR));
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    put(
        &mut snapshot,
        WORKER,
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.CARPENTRY", Value::Bool(true)),
        ],
    )?;
    let workshop = MapCoord::new(0, 0, 10);
    put(
        &mut snapshot,
        WORKSHOP,
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
            ("position", Value::Coord(workshop)),
            ("footprint_min", Value::Coord(workshop)),
            ("footprint_max", Value::Coord(workshop)),
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
            (STOCK_WOOD_FIELD, Value::U64(20)),
            (STOCK_BED_FIELD, Value::U64(0)),
            (STOCK_DOOR_FIELD, Value::U64(0)),
            (STOCK_TABLE_FIELD, Value::U64(0)),
            (STOCK_CHAIR_FIELD, Value::U64(0)),
        ],
    )?;
    Ok(snapshot)
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(field.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn record_mut(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<&mut EntityRecord> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| precondition("test record missing"))
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<bool> {
    let mut shadow = snapshot.clone();
    let changed = apply_effect(&mut shadow, action, key)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(changed)
}

fn cancel(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<bool> {
    let mut shadow = snapshot.clone();
    let changed = cancel_effect(&mut shadow, action, key)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(changed)
}

fn advance_with(
    snapshot: &mut WorldSnapshot,
    ticks: u64,
    limits: EffectAdvanceLimits,
) -> Result<()> {
    let mut shadow = snapshot.clone();
    shadow.tick = GameTick(snapshot.tick.0 + ticks);
    advance_effects_with_limits(&mut shadow, ticks, limits)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    advance_with(snapshot, ticks, EffectAdvanceLimits::default())
}

fn produce(
    snapshot: &mut WorldSnapshot,
    kind: FurnitureKind,
    amount: u32,
    key: &str,
) -> Result<Vec<EntityId>> {
    let job = match kind {
        FurnitureKind::Bed => "CONSTRUCT_BED",
        FurnitureKind::Door => "CONSTRUCT_DOOR",
        FurnitureKind::Table => "CONSTRUCT_TABLE",
        FurnitureKind::Chair => "CONSTRUCT_THRONE",
    };
    apply(
        snapshot,
        &Action::CreateWorkOrder {
            name: key.to_owned(),
            job_token: job.to_owned(),
            amount,
            conditions: Vec::new(),
        },
        key,
    )?;
    advance(snapshot, u64::from(amount) * WORK_ORDER_TICKS_PER_UNIT)?;
    Ok((1..=amount)
        .map(|ordinal| produced_item_id(created_entity_id(key, 0), ordinal))
        .collect())
}

fn place(kind: FurnitureKind, item: EntityId, location: MapCoord) -> Action {
    Action::PlaceFurniture {
        kind,
        item,
        location,
    }
}

fn count(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, LEDGER)?, field, snapshot.tick)
}

fn physical(snapshot: &WorldSnapshot) -> WorldGraph {
    let mut graph = snapshot.graph.clone();
    for record in graph.entities.values_mut() {
        record.revision = 0;
        for fact in record.fields.values_mut() {
            fact.observed_at = GameTick(0);
        }
    }
    graph
}

#[test]
fn every_closed_furniture_kind_consumes_one_exact_item_and_proves_installation() -> Result<()> {
    for kind in [
        FurnitureKind::Bed,
        FurnitureKind::Door,
        FurnitureKind::Table,
        FurnitureKind::Chair,
    ] {
        let mut snapshot = world()?;
        let items = produce(&mut snapshot, kind, 2, "supply")?;
        let action = place(kind, items[0], TARGET);
        let building = created_entity_id("place", 0);
        let proofs = default_postconditions(&action, "place", snapshot.fortress_id);
        apply(&mut snapshot, &action, "place")?;
        assert_eq!(count(&snapshot, furniture_stock_field(kind))?, 1);
        assert_eq!(count(&snapshot, STOCK_WOOD_FIELD)?, 18);
        assert_eq!(loose_furniture_items(&snapshot, kind)?, vec![items[1]]);
        assert_eq!(
            field_text(
                entity(&snapshot, items[0])?,
                FURNITURE_INVENTORY_STATE_FIELD,
                snapshot.tick
            ),
            Some("allocated")
        );
        validate_furniture_identity(&snapshot, building, kind, items[0], TARGET)?;
        validate_furniture_binding(&snapshot, building)?;
        assert!(!proofs.iter().all(|proof| evaluate(&snapshot, proof)));
        advance(&mut snapshot, BUILD_TICKS - 1)?;
        assert_eq!(
            field_text(
                entity(&snapshot, building)?,
                CONSTRUCTION_STAGE_FIELD,
                snapshot.tick
            ),
            Some(STAGE_UNDER_CONSTRUCTION)
        );
        advance(&mut snapshot, 1)?;
        assert!(proofs.iter().all(|proof| evaluate(&snapshot, proof)));
        validate_furniture_binding(&snapshot, building)?;
        assert_eq!(
            field_value(entity(&snapshot, items[0])?, "position", snapshot.tick),
            Some(&Value::Coord(TARGET))
        );
        assert_eq!(count(&snapshot, furniture_stock_field(kind))?, 1);
        let completed = snapshot.clone();
        assert!(!cancel(&mut snapshot, &action, "place")?);
        assert_eq!(snapshot, completed);
        assert!(apply(&mut snapshot, &action, "place").is_err());
        assert_eq!(snapshot, completed);
        let mut reopened = WorldSnapshot::from_canonical_bytes(&snapshot.canonical_bytes())?;
        advance(&mut reopened, BUILD_TICKS)?;
        assert_eq!(count(&reopened, furniture_stock_field(kind))?, 1);
        assert_eq!(loose_furniture_items(&reopened, kind)?, vec![items[1]]);
        assert!(!cancel(&mut reopened, &action, "place")?);
    }
    Ok(())
}

#[test]
fn exact_items_and_occupied_targets_are_exclusive_and_cancelled_items_are_reusable() -> Result<()> {
    let mut snapshot = world()?;
    let items = produce(&mut snapshot, FurnitureKind::Bed, 2, "supply")?;
    let first = place(FurnitureKind::Bed, items[0], TARGET);
    apply(&mut snapshot, &first, "first")?;
    let before = snapshot.clone();
    assert!(
        apply(
            &mut snapshot,
            &place(FurnitureKind::Bed, items[0], MapCoord::new(5, 4, 10)),
            "duplicate-item"
        )
        .is_err()
    );
    assert!(
        apply(
            &mut snapshot,
            &place(FurnitureKind::Bed, items[1], TARGET),
            "duplicate-target"
        )
        .is_err()
    );
    assert_eq!(snapshot, before);
    advance(&mut snapshot, 125)?;
    assert!(cancel(&mut snapshot, &first, "first")?);
    assert_eq!(count(&snapshot, STOCK_BED_FIELD)?, 2);
    assert_eq!(count(&snapshot, STOCK_WOOD_FIELD)?, 18);
    assert_eq!(
        occupied_building_footprint(&snapshot, created_entity_id("first", 0))?,
        None
    );
    let cancelled = snapshot.clone();
    assert!(!cancel(&mut snapshot, &first, "first")?);
    assert_eq!(snapshot, cancelled);
    // A released historical record cannot prevent later reuse or refund again.
    let second = place(FurnitureKind::Bed, items[0], TARGET);
    apply(&mut snapshot, &second, "second")?;
    advance(&mut snapshot, BUILD_TICKS)?;
    validate_furniture_binding(&snapshot, created_entity_id("first", 0))?;
    validate_furniture_binding(&snapshot, created_entity_id("second", 0))?;
    assert!(!cancel(&mut snapshot, &first, "first")?);
    assert_eq!(count(&snapshot, STOCK_BED_FIELD)?, 1);
    Ok(())
}

#[test]
fn legacy_items_stay_ineligible_while_partial_or_unqualified_custody_refuses() -> Result<()> {
    let mut source = world()?;
    let items = produce(&mut source, FurnitureKind::Bed, 2, "supply")?;
    let record = record_mut(&mut source, items[0])?;
    record.fields.remove(FURNITURE_INVENTORY_STATE_FIELD);
    record.fields.remove(FURNITURE_ALLOCATED_TO_FIELD);
    source.refresh_hash();
    let legacy = source.clone();
    assert_eq!(
        loose_furniture_items(&source, FurnitureKind::Bed)?,
        vec![items[1]]
    );
    assert_eq!(source, legacy);
    assert!(
        apply(
            &mut source,
            &place(FurnitureKind::Bed, items[0], TARGET),
            "legacy"
        )
        .is_err()
    );
    assert_eq!(source, legacy);
    apply(
        &mut source,
        &place(FurnitureKind::Bed, items[1], TARGET),
        "new",
    )?;
    assert_eq!(count(&source, STOCK_BED_FIELD)?, 1);
    assert!(loose_furniture_items(&source, FurnitureKind::Bed)?.is_empty());

    for field in [
        FURNITURE_INVENTORY_STATE_FIELD,
        FURNITURE_ALLOCATED_TO_FIELD,
    ] {
        for variant in 0..6 {
            let mut snapshot = legacy.clone();
            let stale_anchor = snapshot.anchor();
            let record = record_mut(&mut snapshot, items[1])?;
            if variant == 0 {
                record.fields.remove(field);
            } else {
                let fact = record
                    .fields
                    .get_mut(field)
                    .ok_or_else(|| precondition("test custody missing"))?;
                match variant {
                    1 => fact.presence = Some(FactPresence::Unknown("lost custody".to_owned())),
                    2 => fact.source = FactSource::AgentAssertion("untrusted test".to_owned()),
                    3 => fact.observed_at = GameTick(100_000),
                    4 => fact.value = Value::Bool(false),
                    _ => fact.presence = Some(FactPresence::Stale(stale_anchor)),
                }
            }
            snapshot.refresh_hash();
            let before = snapshot.clone();
            assert!(
                loose_furniture_items(&snapshot, FurnitureKind::Bed).is_err(),
                "{field} variant {variant}"
            );
            assert!(
                apply(
                    &mut snapshot,
                    &place(FurnitureKind::Bed, items[1], TARGET),
                    "unknown"
                )
                .is_err()
            );
            assert_eq!(snapshot, before);
        }
    }
    Ok(())
}

#[test]
fn source_kind_generation_material_and_original_production_identity_are_required() -> Result<()> {
    let mut source = world()?;
    let item = produce(&mut source, FurnitureKind::Bed, 1, "supply")?[0];
    let order = created_entity_id("supply", 0);
    for variant in 0..9 {
        let mut snapshot = source.clone();
        match variant {
            0 => record_mut(&mut snapshot, item)?.generation = 2,
            1 => record_mut(&mut snapshot, order)?.generation = 2,
            2 => set(&mut snapshot, item, "production_ordinal", Value::U64(0))?,
            3 => set(
                &mut snapshot,
                item,
                "material_token",
                Value::Text("STONE".to_owned()),
            )?,
            4 => set(&mut snapshot, item, "stack_size", Value::U64(2))?,
            5 => set(
                &mut snapshot,
                item,
                "source_order",
                Value::Entity(EntityId::new(999)),
            )?,
            6 => set(&mut snapshot, order, AMOUNT_REMAINING_FIELD, Value::U64(1))?,
            7 => set(&mut snapshot, LEDGER, STOCK_BED_FIELD, Value::U64(0))?,
            _ => set(
                &mut snapshot,
                item,
                WORK_ORDER_PRODUCTION_MODEL_FIELD,
                Value::Text("unknown".to_owned()),
            )?,
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            apply(
                &mut snapshot,
                &place(FurnitureKind::Bed, item, TARGET),
                "bad"
            )
            .is_err(),
            "variant {variant}"
        );
        assert_eq!(snapshot, before);
    }
    let before = source.clone();
    assert!(
        apply(
            &mut source,
            &place(FurnitureKind::Chair, item, TARGET),
            "wrong-kind"
        )
        .is_err()
    );
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn unavailable_geometry_and_damaged_released_history_do_not_prove_a_free_target() -> Result<()> {
    let mut snapshot = world()?;
    let item = produce(&mut snapshot, FurnitureKind::Bed, 1, "supply")?[0];
    let action = place(FurnitureKind::Bed, item, TARGET);
    let before = snapshot.clone();
    assert!(
        apply(
            &mut snapshot,
            &place(FurnitureKind::Bed, item, MapCoord::new(30, 30, 10)),
            "unobserved"
        )
        .is_err()
    );
    assert!(
        apply(
            &mut snapshot,
            &place(FurnitureKind::Bed, item, MapCoord::new(0, 0, 10)),
            "occupied"
        )
        .is_err()
    );
    assert_eq!(snapshot, before);
    record_mut(&mut snapshot, WORKSHOP)?
        .fields
        .remove("footprint_max");
    snapshot.refresh_hash();
    let before = snapshot.clone();
    assert!(apply(&mut snapshot, &action, "unknown-footprint").is_err());
    assert_eq!(snapshot, before);
    set(
        &mut snapshot,
        WORKSHOP,
        "footprint_max",
        Value::Coord(MapCoord::new(0, 0, 10)),
    )?;
    apply(&mut snapshot, &action, "released")?;
    cancel(&mut snapshot, &action, "released")?;
    let building = created_entity_id("released", 0);
    set(
        &mut snapshot,
        building,
        FURNITURE_ATTACHMENT_STATE_FIELD,
        Value::Text("attached".to_owned()),
    )?;
    assert!(occupied_building_footprint(&snapshot, building).is_err());
    assert!(validate_furniture_location(&snapshot, TARGET).is_err());
    Ok(())
}

#[test]
fn broken_reciprocal_custody_and_progress_budget_refuse_atomic_advancement() -> Result<()> {
    let mut source = world()?;
    let item = produce(&mut source, FurnitureKind::Bed, 1, "supply")?[0];
    apply(
        &mut source,
        &place(FurnitureKind::Bed, item, TARGET),
        "place",
    )?;
    let building = created_entity_id("place", 0);
    for variant in 0..5 {
        let mut snapshot = source.clone();
        match variant {
            0 => set(
                &mut snapshot,
                item,
                FURNITURE_ALLOCATED_TO_FIELD,
                Value::Null,
            )?,
            1 => set(
                &mut snapshot,
                item,
                FURNITURE_INVENTORY_STATE_FIELD,
                Value::Text("loose".to_owned()),
            )?,
            2 => record_mut(&mut snapshot, building)?.generation = 2,
            3 => set(
                &mut snapshot,
                building,
                FURNITURE_SOURCE_ITEM_GENERATION_FIELD,
                Value::U64(2),
            )?,
            _ => set(&mut snapshot, building, "required_ticks", Value::U64(1))?,
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            advance(&mut snapshot, BUILD_TICKS).is_err(),
            "variant {variant}"
        );
        assert_eq!(snapshot, before);
        assert!(validate_furniture_binding(&snapshot, building).is_err());
    }
    let before = source.clone();
    assert_eq!(
        advance_with(
            &mut source,
            BUILD_TICKS,
            EffectAdvanceLimits {
                max_work_units: 1,
                ..EffectAdvanceLimits::default()
            }
        )
        .err()
        .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn rejected_publication_and_refund_overflow_cannot_leave_partial_item_custody() -> Result<()> {
    let mut source = world()?;
    let item = produce(&mut source, FurnitureKind::Bed, 1, "supply")?[0];
    let action = place(FurnitureKind::Bed, item, TARGET);
    let mut failed = source.clone();
    record_mut(&mut failed, item)?.revision = u64::MAX;
    failed.refresh_hash();
    let before = failed.clone();
    assert!(apply(&mut failed, &action, "place").is_err());
    assert_eq!(failed, before);
    assert!(
        !failed
            .graph
            .entities
            .contains_key(&created_entity_id("place", 0))
    );
    apply(&mut source, &action, "place")?;
    set(&mut source, LEDGER, STOCK_BED_FIELD, Value::U64(u64::MAX))?;
    let before = source.clone();
    assert_eq!(
        cancel(&mut source, &action, "place")
            .err()
            .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(source, before);
    assert!(
        cancel(
            &mut source,
            &place(FurnitureKind::Bed, item, MapCoord::new(5, 5, 10)),
            "place"
        )
        .is_err()
    );
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn placement_is_partition_and_snapshot_recovery_invariant_and_legacy_build_stays_abstract()
-> Result<()> {
    let mut source = world()?;
    let items = produce(&mut source, FurnitureKind::Table, 2, "supply")?;
    for (index, item) in items.iter().enumerate() {
        apply(
            &mut source,
            &place(
                FurnitureKind::Table,
                *item,
                MapCoord::new(4 + index as i32, 4, 10),
            ),
            &format!("place-{index}"),
        )?;
    }
    let mut whole = source.clone();
    advance(&mut whole, BUILD_TICKS)?;
    let mut pieces = source.clone();
    for _ in 0..50 {
        advance(&mut pieces, 10)?;
    }
    assert_eq!(physical(&whole), physical(&pieces));
    let mut recovered = source.clone();
    advance(&mut recovered, 250)?;
    recovered = WorldSnapshot::from_canonical_bytes(&recovered.canonical_bytes())?;
    advance(&mut recovered, 250)?;
    assert_eq!(physical(&whole), physical(&recovered));

    let mut legacy = world()?;
    let action = Action::Build {
        kind: BuildingKind::Furniture("Bed".to_owned()),
        location: TARGET,
        footprint: MapCuboid::new(TARGET, TARGET)?,
        material: crate::MaterialSelector::default(),
    };
    apply(&mut legacy, &action, "legacy-build")?;
    advance(&mut legacy, BUILD_TICKS)?;
    assert_eq!(count(&legacy, STOCK_BED_FIELD)?, 0);
    assert!(
        legacy
            .graph
            .entities
            .values()
            .all(|record| record.kind != EntityKind::Item)
    );
    assert!(
        default_postconditions(&action, "legacy-build", legacy.fortress_id)
            .iter()
            .all(|proof| evaluate(&legacy, proof))
    );
    Ok(())
}
