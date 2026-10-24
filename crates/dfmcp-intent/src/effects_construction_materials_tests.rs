use super::*;
use crate::MaterialSelector;
use dfmcp_core::ObservationCursor;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{ChunkCoord, FactPresence, WorldGraph, evaluate};

const CONSTRUCTOR: EntityId = EntityId::new(11);
const CARPENTER: EntityId = EntityId::new(12);
const CARPENTERS: EntityId = EntityId::new(51);
const LEDGER: EntityId = EntityId::new(91);
const FIRST_SITE: MapCoord = MapCoord::new(4, 4, 10);
const SECOND_SITE: MapCoord = MapCoord::new(6, 4, 10);
const THIRD_SITE: MapCoord = MapCoord::new(8, 4, 10);

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

fn world(staffed: bool, materials: bool, wood: u64) -> Result<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(chunk, uniform_chunk(chunk, tile_codes::FLOOR));
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    for (id, construction, carpentry) in [(CONSTRUCTOR, true, false), (CARPENTER, false, true)] {
        put(
            &mut snapshot,
            id,
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.CONSTRUCTION", Value::Bool(construction)),
                ("labor.CARPENTRY", Value::Bool(carpentry)),
                (SQUAD_FIELD, Value::Null),
            ],
        )?;
    }
    let workshop = MapCoord::new(0, 0, 10);
    put(
        &mut snapshot,
        CARPENTERS,
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
            (STOCK_WOOD_FIELD, Value::U64(wood)),
            (STOCK_BED_FIELD, Value::U64(0)),
            (STOCK_DOOR_FIELD, Value::U64(0)),
            (STOCK_TABLE_FIELD, Value::U64(0)),
            (STOCK_CHAIR_FIELD, Value::U64(0)),
        ],
    )?;
    let mut fields = Vec::new();
    if staffed {
        fields.push((
            LABORATORY_WORKFORCE_MODEL_FIELD,
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        ));
    }
    if materials {
        fields.push((
            LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
            Value::Text(LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
        ));
    }
    if !fields.is_empty() {
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        put(
            &mut snapshot,
            settings,
            EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
            fields,
        )?;
    }
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
        .ok_or_else(|| precondition("construction material test record missing"))
}

fn fact_mut<'a>(
    snapshot: &'a mut WorldSnapshot,
    id: EntityId,
    field: &str,
) -> Result<&'a mut Fact> {
    record_mut(snapshot, id)?
        .fields
        .get_mut(field)
        .ok_or_else(|| precondition("construction material test fact missing"))
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
    shadow.tick = GameTick(
        snapshot
            .tick
            .0
            .checked_add(ticks)
            .ok_or_else(|| advance_budget_error("construction material test clock overflow"))?,
    );
    advance_effects_with_limits(&mut shadow, ticks, limits)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    advance_with(snapshot, ticks, EffectAdvanceLimits::default())
}

fn after(source: &WorldSnapshot, cuts: &[u64]) -> Result<WorldSnapshot> {
    let mut result = source.clone();
    for ticks in cuts {
        advance(&mut result, *ticks)?;
    }
    Ok(result)
}

fn physical(snapshot: &WorldSnapshot) -> WorldGraph {
    let mut graph = snapshot.graph.clone();
    for record in graph.entities.values_mut() {
        record.revision = 0;
        for fact in record.fields.values_mut() {
            fact.observed_at = GameTick(0);
        }
    }
    for chunk in graph.chunks.values_mut() {
        chunk.revision = 0;
    }
    graph
}

fn assert_physical(left: &WorldSnapshot, right: &WorldSnapshot) {
    assert_eq!(left.tick, right.tick);
    assert_eq!(left.paused, right.paused);
    assert_eq!(physical(left), physical(right));
}

fn number(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, field, snapshot.tick)
}

fn build_action(kind: &str, position: MapCoord) -> Result<Action> {
    Ok(Action::Build {
        kind: BuildingKind::Workshop(kind.to_owned()),
        location: position,
        footprint: MapCuboid::new(position, position)?,
        material: MaterialSelector::default(),
    })
}

fn build(
    snapshot: &mut WorldSnapshot,
    kind: &str,
    position: MapCoord,
    key: &str,
) -> Result<EntityId> {
    apply(snapshot, &build_action(kind, position)?, key)?;
    Ok(created_entity_id(key, 0))
}

fn order(snapshot: &mut WorldSnapshot, amount: u32, key: &str) -> Result<EntityId> {
    apply(
        snapshot,
        &Action::CreateWorkOrder {
            name: key.to_owned(),
            job_token: "CONSTRUCT_BED".to_owned(),
            amount,
            conditions: Vec::new(),
        },
        key,
    )?;
    Ok(created_entity_id(key, 0))
}

fn assert_custody(snapshot: &WorldSnapshot, building: EntityId, state: &str) -> Result<()> {
    let record = entity(snapshot, building)?;
    assert_eq!(
        field_text(record, CONSTRUCTION_MATERIAL_MODEL_FIELD, snapshot.tick),
        Some(LABORATORY_CONSTRUCTION_MATERIAL_MODEL)
    );
    assert_eq!(
        field_value(record, CONSTRUCTION_MATERIAL_LEDGER_FIELD, snapshot.tick),
        Some(&Value::Entity(LEDGER))
    );
    assert_eq!(
        number(
            snapshot,
            building,
            CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD
        )?,
        1
    );
    assert_eq!(
        number(snapshot, building, CONSTRUCTION_MATERIAL_WOOD_FIELD)?,
        1
    );
    assert_eq!(
        field_text(record, CONSTRUCTION_MATERIAL_STATE_FIELD, snapshot.tick),
        Some(state)
    );
    Ok(())
}

fn assert_inventory(
    snapshot: &WorldSnapshot,
    available: u64,
    held: u64,
    consumed: u64,
    released: u64,
) -> Result<()> {
    let before = snapshot.clone();
    let report = construction_material_report(snapshot)?
        .ok_or_else(|| precondition("construction material report missing"))?;
    assert_eq!(
        *snapshot, before,
        "inspection must not allocate or refund wood"
    );
    assert_eq!(report.ledger, LEDGER);
    assert_eq!(report.ledger_generation, 1);
    assert_eq!(report.available_wood, available);
    assert_eq!(report.held_wood, held);
    assert_eq!(report.consumed_wood, consumed);
    assert_eq!(report.released_wood, released);
    assert_eq!(number(snapshot, LEDGER, STOCK_WOOD_FIELD)?, available);
    Ok(())
}

#[test]
fn each_closed_workshop_holds_one_log_at_dispatch_and_consumes_it_only_once() -> Result<()> {
    for kind in ["Still", "Kitchen", "Carpenters"] {
        let mut snapshot = world(true, true, 2)?;
        let action = build_action(kind, FIRST_SITE)?;
        let proofs = construction_completion_predicates(&snapshot, &action, "one-workshop")?;
        let before_planning = snapshot.clone();
        assert_eq!(construction_wood_cost(&snapshot, &action)?, Some(1));
        assert_eq!(
            construction_material_model(&snapshot)?,
            Some(LABORATORY_CONSTRUCTION_MATERIAL_MODEL)
        );
        assert_eq!(snapshot, before_planning);
        assert!(!proofs.is_empty());
        let building = created_entity_id("one-workshop", 0);
        apply(&mut snapshot, &action, "one-workshop")?;
        assert_inventory(&snapshot, 1, 1, 0, 0)?;
        assert_custody(&snapshot, building, "held")?;
        assert!(!proofs.iter().all(|proof| evaluate(&snapshot, proof)));
        advance(&mut snapshot, BUILD_TICKS - 1)?;
        assert_inventory(&snapshot, 1, 1, 0, 0)?;
        assert_custody(&snapshot, building, "held")?;
        advance(&mut snapshot, 1)?;
        assert_inventory(&snapshot, 1, 0, 1, 0)?;
        assert_custody(&snapshot, building, "consumed")?;
        assert!(proofs.iter().all(|proof| evaluate(&snapshot, proof)));
        let completed = snapshot.clone();
        assert!(!cancel(&mut snapshot, &action, "one-workshop")?);
        assert_eq!(snapshot, completed);
        advance(&mut snapshot, BUILD_TICKS)?;
        assert_inventory(&snapshot, 1, 0, 1, 0)?;
    }
    Ok(())
}

#[test]
fn scarce_wood_cannot_back_two_buildings_and_released_stock_can_be_reused_once() -> Result<()> {
    let mut snapshot = world(true, true, 2)?;
    let first = build_action("Still", FIRST_SITE)?;
    let second = build_action("Kitchen", SECOND_SITE)?;
    let third = build_action("Carpenters", THIRD_SITE)?;
    apply(&mut snapshot, &first, "scarce-first")?;
    apply(&mut snapshot, &second, "scarce-second")?;
    assert_inventory(&snapshot, 0, 2, 0, 0)?;
    let before = snapshot.clone();
    assert!(apply(&mut snapshot, &third, "scarce-third").is_err());
    assert_eq!(snapshot, before);
    assert!(apply(&mut snapshot, &first, "scarce-first").is_err());
    assert_eq!(snapshot, before, "a repeated key cannot hold a second log");

    advance(&mut snapshot, BUILD_TICKS)?;
    assert_inventory(&snapshot, 0, 1, 1, 0)?;
    let first_cancelled = cancel(&mut snapshot, &first, "scarce-first")?;
    let second_cancelled = cancel(&mut snapshot, &second, "scarce-second")?;
    assert_ne!(first_cancelled, second_cancelled);
    assert_inventory(&snapshot, 1, 0, 1, 1)?;
    apply(&mut snapshot, &third, "scarce-third")?;
    assert_inventory(&snapshot, 0, 1, 1, 1)?;
    let reused = snapshot.clone();
    assert!(!cancel(&mut snapshot, &first, "scarce-first")?);
    assert!(!cancel(&mut snapshot, &second, "scarce-second")?);
    assert_eq!(
        snapshot, reused,
        "historical release cannot refund reused stock"
    );
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_inventory(&snapshot, 0, 0, 2, 1)?;
    Ok(())
}

#[test]
fn held_construction_wood_cannot_be_spent_by_carpentry_before_its_refund() -> Result<()> {
    let mut snapshot = world(true, true, 1)?;
    let production = order(&mut snapshot, 1, "competing-bed")?;
    let action = build_action("Still", FIRST_SITE)?;
    apply(&mut snapshot, &action, "competing-building")?;
    let building = created_entity_id("competing-building", 0);
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    assert_inventory(&snapshot, 0, 1, 0, 0)?;
    assert_eq!(
        number(&snapshot, building, "progress_ticks")?,
        WORK_ORDER_TICKS_PER_UNIT
    );
    assert_eq!(number(&snapshot, production, AMOUNT_REMAINING_FIELD)?, 1);
    assert_eq!(number(&snapshot, production, "work_ticks")?, 0);
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 0);
    assert!(
        field_text(
            entity(&snapshot, production)?,
            BLOCKED_BY_FIELD,
            snapshot.tick
        )
        .is_some_and(|reason| reason.contains("WOOD"))
    );
    cancel(&mut snapshot, &action, "competing-building")?;
    assert_inventory(&snapshot, 1, 0, 0, 1)?;
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT - 1)?;
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 0);
    advance(&mut snapshot, 1)?;
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    assert_inventory(&snapshot, 0, 0, 0, 1)?;
    assert_custody(&snapshot, building, "released")?;
    let spent = snapshot.clone();
    assert!(!cancel(&mut snapshot, &action, "competing-building")?);
    assert_eq!(snapshot, spent);
    Ok(())
}

#[test]
fn completion_does_not_debit_the_last_log_already_spent_by_parallel_carpentry() -> Result<()> {
    let mut snapshot = world(true, true, 2)?;
    let production = order(&mut snapshot, 1, "parallel-bed")?;
    let building = build(&mut snapshot, "Kitchen", FIRST_SITE, "parallel-building")?;
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    assert_eq!(number(&snapshot, production, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    assert_inventory(&snapshot, 0, 1, 0, 0)?;
    advance(&mut snapshot, BUILD_TICKS - WORK_ORDER_TICKS_PER_UNIT)?;
    assert_custody(&snapshot, building, "consumed")?;
    assert_inventory(&snapshot, 0, 0, 1, 0)?;
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    Ok(())
}

#[test]
fn unfinished_cancellation_refunds_once_across_reopen_without_erasing_earned_work() -> Result<()> {
    for earned in [0, 1, BUILD_TICKS - 1, BUILD_TICKS] {
        let mut snapshot = world(true, true, 1)?;
        let action = build_action("Carpenters", FIRST_SITE)?;
        apply(&mut snapshot, &action, "cancel-material")?;
        let building = created_entity_id("cancel-material", 0);
        advance(&mut snapshot, earned)?;
        let finished = earned == BUILD_TICKS;
        assert_eq!(
            cancel(&mut snapshot, &action, "cancel-material")?,
            !finished
        );
        assert_eq!(number(&snapshot, building, "progress_ticks")?, earned);
        if finished {
            assert_inventory(&snapshot, 0, 0, 1, 0)?;
            assert_custody(&snapshot, building, "consumed")?;
        } else {
            assert_inventory(&snapshot, 1, 0, 0, 1)?;
            assert_custody(&snapshot, building, "released")?;
        }
        let bytes = snapshot.canonical_bytes();
        let mut reopened = WorldSnapshot::from_canonical_bytes(&bytes)?;
        assert!(!cancel(&mut reopened, &action, "cancel-material")?);
        assert_eq!(reopened.canonical_bytes(), bytes);
        assert!(apply(&mut reopened, &action, "cancel-material").is_err());
        assert_eq!(reopened.canonical_bytes(), bytes);
        advance(&mut reopened, BUILD_TICKS)?;
        assert_eq!(number(&reopened, building, "progress_ticks")?, earned);
        assert_eq!(
            number(&reopened, LEDGER, STOCK_WOOD_FIELD)?,
            u64::from(!finished)
        );
    }
    Ok(())
}

#[test]
fn competing_material_work_is_partition_and_snapshot_recovery_invariant() -> Result<()> {
    let mut source = world(true, true, 4)?;
    build(&mut source, "Still", FIRST_SITE, "partition-still")?;
    build(&mut source, "Kitchen", SECOND_SITE, "partition-kitchen")?;
    order(&mut source, 2, "partition-beds")?;
    let whole = after(&source, &[1_000])?;
    assert_inventory(&whole, 0, 0, 2, 0)?;
    assert_eq!(number(&whole, LEDGER, STOCK_BED_FIELD)?, 2);
    assert_physical(&whole, &after(&source, &[1, 49, 50, 123, 277, 499, 1])?);
    let checkpoint = after(&source, &[125])?;
    let reopened = WorldSnapshot::from_canonical_bytes(&checkpoint.canonical_bytes())?;
    assert_physical(&whole, &after(&reopened, &[875])?);
    let mut each_tick = source.clone();
    for _ in 0..1_000 {
        advance(&mut each_tick, 1)?;
    }
    assert_physical(&whole, &each_tick);
    Ok(())
}

#[test]
fn exact_item_furniture_placement_never_charges_construction_wood_again() -> Result<()> {
    let mut snapshot = world(true, true, 1)?;
    let production = order(&mut snapshot, 1, "placement-bed")?;
    advance(&mut snapshot, WORK_ORDER_TICKS_PER_UNIT)?;
    let item = produced_item_id(production, 1);
    let action = Action::PlaceFurniture {
        kind: FurnitureKind::Bed,
        item,
        location: FIRST_SITE,
    };
    assert_eq!(construction_wood_cost(&snapshot, &action)?, None);
    assert!(construction_completion_predicates(&snapshot, &action, "placement")?.is_empty());
    apply(&mut snapshot, &action, "placement")?;
    let building = created_entity_id("placement", 0);
    assert!(
        !entity(&snapshot, building)?
            .fields
            .contains_key(CONSTRUCTION_MATERIAL_MODEL_FIELD)
    );
    advance(&mut snapshot, BUILD_TICKS)?;
    validate_furniture_completion(&snapshot, building, FurnitureKind::Bed, item, FIRST_SITE)?;
    assert_inventory(&snapshot, 0, 0, 0, 0)?;
    assert_eq!(number(&snapshot, LEDGER, STOCK_BED_FIELD)?, 0);
    Ok(())
}

#[test]
fn selector_contract_is_closed_and_unsupported_builds_never_hold_stock() -> Result<()> {
    for required in [BTreeSet::new(), BTreeSet::from(["WOOD".to_owned()])] {
        let mut snapshot = world(true, true, 1)?;
        let mut action = build_action("Still", FIRST_SITE)?;
        if let Action::Build { material, .. } = &mut action {
            material.required_tokens = required;
            material.forbidden_tokens = BTreeSet::from(["STONE".to_owned(), "IRON".to_owned()]);
        }
        assert_eq!(construction_wood_cost(&snapshot, &action)?, Some(1));
        apply(&mut snapshot, &action, "valid-selector")?;
        assert_inventory(&snapshot, 0, 1, 0, 0)?;
    }
    let source = world(true, true, 20)?;
    for variant in 0..7 {
        let mut action = build_action("Still", FIRST_SITE)?;
        if let Action::Build { material, .. } = &mut action {
            match variant {
                0 => material.required_tokens = BTreeSet::from(["STONE".to_owned()]),
                1 => {
                    material.required_tokens =
                        BTreeSet::from(["WOOD".to_owned(), "STONE".to_owned()])
                }
                2 => material.forbidden_tokens = BTreeSet::from(["WOOD".to_owned()]),
                3 => material.prefer_nearest = true,
                4 => material.reserve_count = 1,
                5 => material.forbidden_tokens = BTreeSet::from([String::new()]),
                _ => material.forbidden_tokens = BTreeSet::from(["A".repeat(257)]),
            }
        }
        let mut snapshot = source.clone();
        assert!(
            construction_wood_cost(&snapshot, &action).is_err(),
            "selector {variant}"
        );
        assert!(
            apply(&mut snapshot, &action, "bad-selector").is_err(),
            "selector {variant}"
        );
        assert_eq!(snapshot, source, "selector {variant}");
    }
    for kind in [
        BuildingKind::Workshop("Masons".to_owned()),
        BuildingKind::Workshop("still".to_owned()),
        BuildingKind::Furnace("WoodFurnace".to_owned()),
        BuildingKind::Furniture("Bed".to_owned()),
        BuildingKind::Construction("Floor".to_owned()),
        BuildingKind::Trap("Cage".to_owned()),
        BuildingKind::FarmPlot,
        BuildingKind::Bridge,
        BuildingKind::Well,
        BuildingKind::Custom("arbitrary".to_owned()),
    ] {
        let mut action = build_action("Still", FIRST_SITE)?;
        if let Action::Build { kind: actual, .. } = &mut action {
            *actual = kind;
        }
        let mut snapshot = source.clone();
        assert!(construction_wood_cost(&snapshot, &action).is_err());
        assert!(apply(&mut snapshot, &action, "unsupported-build").is_err());
        assert_eq!(snapshot, source);
    }
    Ok(())
}

#[test]
fn occupied_unknown_and_unbounded_sites_refuse_but_valid_cancelled_sites_are_reusable() -> Result<()>
{
    for variant in 0..7 {
        let mut snapshot = world(true, true, 2)?;
        let mut action = build_action("Still", FIRST_SITE)?;
        match variant {
            0 => {
                snapshot.set_tile_code(FIRST_SITE, tile_codes::SOLID_WALL)?;
            }
            1 => action = build_action("Still", MapCoord::new(16, 4, 10))?,
            2 => action = build_action("Still", MapCoord::new(0, 0, 10))?,
            3 => {
                if let Action::Build { footprint, .. } = &mut action {
                    *footprint = MapCuboid::new(MapCoord::new(0, 4, 10), MapCoord::new(15, 8, 10))?;
                }
            }
            4 => {
                if let Action::Build { footprint, .. } = &mut action {
                    *footprint = MapCuboid::new(FIRST_SITE, MapCoord::new(4, 4, 11))?;
                }
            }
            5 => {
                fact_mut(&mut snapshot, CARPENTERS, "footprint_min")?.presence = Some(
                    FactPresence::Unknown("workshop extent unavailable".to_owned()),
                );
            }
            _ => {
                record_mut(&mut snapshot, CARPENTERS)?.kind =
                    EntityKind::Other("building".to_owned());
                action = build_action("Still", MapCoord::new(0, 0, 10))?;
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            apply(&mut snapshot, &action, "bad-site").is_err(),
            "site variant {variant}"
        );
        assert_eq!(snapshot, before, "site variant {variant}");
    }
    let mut snapshot = world(true, true, 2)?;
    let first = build_action("Still", FIRST_SITE)?;
    let replacement = build_action("Kitchen", FIRST_SITE)?;
    apply(&mut snapshot, &first, "old-site")?;
    let occupied = snapshot.clone();
    assert!(apply(&mut snapshot, &replacement, "new-site").is_err());
    assert_eq!(snapshot, occupied);
    cancel(&mut snapshot, &first, "old-site")?;
    apply(&mut snapshot, &replacement, "new-site")?;
    assert_inventory(&snapshot, 1, 1, 0, 1)?;
    advance(&mut snapshot, BUILD_TICKS)?;
    assert_inventory(&snapshot, 1, 0, 1, 1)?;
    Ok(())
}

#[test]
fn missing_ambiguous_or_unqualified_wood_ledgers_refuse_before_allocation() -> Result<()> {
    let action = build_action("Still", FIRST_SITE)?;
    for variant in 0..12 {
        let mut snapshot = world(true, true, 1)?;
        match variant {
            0 => {
                record_mut(&mut snapshot, LEDGER)?
                    .fields
                    .remove(STOCK_WOOD_FIELD);
            }
            1 => {
                fact_mut(&mut snapshot, LEDGER, STOCK_WOOD_FIELD)?.presence =
                    Some(FactPresence::Omitted("wood unavailable".to_owned()));
            }
            2 => {
                fact_mut(&mut snapshot, LEDGER, STOCK_WOOD_FIELD)?.source =
                    FactSource::AgentAssertion("one log".to_owned());
            }
            3 => fact_mut(&mut snapshot, LEDGER, STOCK_WOOD_FIELD)?.source = FactSource::Replay,
            4 => {
                fact_mut(&mut snapshot, LEDGER, STOCK_WOOD_FIELD)?.source_digest =
                    Digest32::of_bytes(b"unqualified wood producer");
            }
            5 => set(
                &mut snapshot,
                LEDGER,
                STOCK_WOOD_FIELD,
                Value::Text("1".to_owned()),
            )?,
            6 => {
                fact_mut(&mut snapshot, LEDGER, STOCK_WOOD_FIELD)?.presence =
                    Some(FactPresence::Known(Value::U64(2)));
            }
            7 => {
                snapshot.graph.entities.remove(&LEDGER);
            }
            8 => {
                let mut duplicate = entity(&snapshot, LEDGER)?.clone();
                duplicate.id = EntityId::new(92);
                snapshot.graph.entities.insert(duplicate.id, duplicate);
            }
            9 => record_mut(&mut snapshot, LEDGER)?.id = EntityId::new(92),
            10 => record_mut(&mut snapshot, LEDGER)?.kind = EntityKind::Item,
            _ => {
                let future = GameTick(snapshot.tick.0 + 1);
                fact_mut(&mut snapshot, LEDGER, STOCK_WOOD_FIELD)?.observed_at = future;
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            construction_material_report(&snapshot).is_err(),
            "ledger variant {variant}"
        );
        assert!(
            apply(&mut snapshot, &action, "invalid-ledger").is_err(),
            "ledger variant {variant}"
        );
        assert_eq!(snapshot, before, "ledger variant {variant}");
    }
    Ok(())
}

#[test]
fn malformed_model_declarations_and_missing_workforce_never_fall_back_to_free_building()
-> Result<()> {
    let action = build_action("Still", FIRST_SITE)?;
    for variant in 0..10 {
        let mut snapshot = world(true, true, 1)?;
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        match variant {
            0 => set(
                &mut snapshot,
                settings,
                LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                Value::Bool(true),
            )?,
            1 => set(
                &mut snapshot,
                settings,
                LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                Value::Text("wooden-workshops/999".to_owned()),
            )?,
            2 => {
                fact_mut(
                    &mut snapshot,
                    settings,
                    LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                )?
                .presence = Some(FactPresence::Unknown("model unavailable".to_owned()));
            }
            3 => {
                fact_mut(
                    &mut snapshot,
                    settings,
                    LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                )?
                .source = FactSource::AgentAssertion("wood accounting".to_owned());
            }
            4 => {
                record_mut(&mut snapshot, settings)?
                    .fields
                    .remove(LABORATORY_WORKFORCE_MODEL_FIELD);
            }
            5 => {
                fact_mut(&mut snapshot, settings, LABORATORY_WORKFORCE_MODEL_FIELD)?.presence =
                    Some(FactPresence::Omitted("workforce unavailable".to_owned()));
            }
            6 => record_mut(&mut snapshot, settings)?.kind = EntityKind::Unit,
            7 | 8 => {
                let mut duplicate = entity(&snapshot, settings)?.clone();
                duplicate.id = EntityId::new(94);
                snapshot.graph.entities.insert(duplicate.id, duplicate);
                if variant == 7 {
                    snapshot.graph.entities.remove(&settings);
                }
            }
            _ => {
                let future = GameTick(snapshot.tick.0 + 1);
                fact_mut(
                    &mut snapshot,
                    settings,
                    LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                )?
                .observed_at = future;
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            construction_material_model(&snapshot).is_err(),
            "model variant {variant}"
        );
        assert!(
            construction_material_report(&snapshot).is_err(),
            "model variant {variant}"
        );
        assert!(
            apply(&mut snapshot, &action, "invalid-model").is_err(),
            "model variant {variant}"
        );
        assert!(
            advance(&mut snapshot, 1).is_err(),
            "model variant {variant}"
        );
        assert_eq!(snapshot, before, "model variant {variant}");
    }
    Ok(())
}

#[test]
fn model_history_cannot_downgrade_or_adopt_unfunded_active_workshops() -> Result<()> {
    let action = build_action("Still", FIRST_SITE)?;
    for elapsed in [0, BUILD_TICKS] {
        let mut snapshot = world(true, true, 1)?;
        apply(&mut snapshot, &action, "history")?;
        advance(&mut snapshot, elapsed)?;
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        record_mut(&mut snapshot, settings)?
            .fields
            .remove(LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD);
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(construction_material_model(&snapshot).is_err());
        assert!(construction_material_report(&snapshot).is_err());
        assert!(advance(&mut snapshot, 1).is_err());
        assert!(cancel(&mut snapshot, &action, "history").is_err());
        assert_eq!(snapshot, before);
    }
    let mut legacy = world(true, false, 1)?;
    apply(&mut legacy, &action, "unfunded-history")?;
    let settings = fortress_settings_entity_id(legacy.fortress_id);
    set(
        &mut legacy,
        settings,
        LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
        Value::Text(LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
    )?;
    let before = legacy.clone();
    assert!(construction_material_report(&legacy).is_err());
    assert!(advance(&mut legacy, 1).is_err());
    assert_eq!(legacy, before);
    Ok(())
}

#[test]
fn damaged_material_custody_never_authorizes_progress_or_a_refund() -> Result<()> {
    let mut source = world(true, true, 2)?;
    let action = build_action("Still", FIRST_SITE)?;
    apply(&mut source, &action, "custody")?;
    let building = created_entity_id("custody", 0);
    for variant in 0..21 {
        let mut snapshot = source.clone();
        match variant {
            0 => {
                record_mut(&mut snapshot, building)?
                    .fields
                    .remove(CONSTRUCTION_MATERIAL_MODEL_FIELD);
            }
            1 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_MODEL_FIELD,
                Value::Text("wooden-workshops/999".to_owned()),
            )?,
            2 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_LEDGER_FIELD,
                Value::Entity(CARPENTERS),
            )?,
            3 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_LEDGER_GENERATION_FIELD,
                Value::U64(2),
            )?,
            4 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_WOOD_FIELD,
                Value::U64(0),
            )?,
            5 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_WOOD_FIELD,
                Value::U64(2),
            )?,
            6 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_STATE_FIELD,
                Value::Text("consumed".to_owned()),
            )?,
            7 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_STATE_FIELD,
                Value::Text("released".to_owned()),
            )?,
            8 => {
                fact_mut(&mut snapshot, building, CONSTRUCTION_MATERIAL_LEDGER_FIELD)?.source =
                    FactSource::AgentAssertion("owned log".to_owned());
            }
            9 => {
                fact_mut(&mut snapshot, building, CONSTRUCTION_MATERIAL_STATE_FIELD)?.presence =
                    Some(FactPresence::Unknown("custody unavailable".to_owned()));
            }
            10 => {
                fact_mut(&mut snapshot, building, CONSTRUCTION_MATERIAL_WOOD_FIELD)?.presence =
                    Some(FactPresence::Known(Value::U64(2)));
            }
            11 => record_mut(&mut snapshot, LEDGER)?.generation = 2,
            12 => record_mut(&mut snapshot, building)?.kind = EntityKind::Item,
            13 => record_mut(&mut snapshot, building)?.generation = 2,
            14 => {
                record_mut(&mut snapshot, building)?
                    .fields
                    .remove(CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD);
            }
            15 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_NEAREST_FIELD,
                Value::Bool(true),
            )?,
            16 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_RESERVE_FIELD,
                Value::U64(1),
            )?,
            17 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
                Value::List(vec![Value::Text("WOOD".to_owned())]),
            )?,
            18 => set(
                &mut snapshot,
                building,
                CONSTRUCTION_MATERIAL_FORBIDDEN_FIELD,
                Value::List(vec![
                    Value::Text("STONE".to_owned()),
                    Value::Text("STONE".to_owned()),
                ]),
            )?,
            19 => set(
                &mut snapshot,
                building,
                "material_tokens",
                Value::List(vec![Value::Text("STONE".to_owned())]),
            )?,
            _ => {
                let future = GameTick(snapshot.tick.0 + 1);
                fact_mut(&mut snapshot, building, CONSTRUCTION_MATERIAL_STATE_FIELD)?.observed_at =
                    future;
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            construction_material_report(&snapshot).is_err(),
            "custody variant {variant}"
        );
        assert!(
            advance(&mut snapshot, 1).is_err(),
            "custody variant {variant}"
        );
        assert!(
            cancel(&mut snapshot, &action, "custody").is_err(),
            "custody variant {variant}"
        );
        assert_eq!(snapshot, before, "custody variant {variant}");
    }
    Ok(())
}

#[test]
fn a_different_build_action_cannot_release_the_original_allocation() -> Result<()> {
    let mut source = world(true, true, 1)?;
    let original = build_action("Still", FIRST_SITE)?;
    apply(&mut source, &original, "bound-action")?;
    for variant in 0..4 {
        let mut different = original.clone();
        if let Action::Build {
            kind,
            location,
            footprint,
            material,
        } = &mut different
        {
            match variant {
                0 => *kind = BuildingKind::Workshop("Kitchen".to_owned()),
                1 => *location = SECOND_SITE,
                2 => *footprint = MapCuboid::new(FIRST_SITE, SECOND_SITE)?,
                _ => material.required_tokens = BTreeSet::from(["WOOD".to_owned()]),
            }
        }
        let mut snapshot = source.clone();
        assert!(
            cancel(&mut snapshot, &different, "bound-action").is_err(),
            "action variant {variant}"
        );
        assert_eq!(snapshot, source, "action variant {variant}");
    }
    Ok(())
}

#[test]
fn identity_collisions_refund_overflow_and_event_budgets_publish_no_partial_custody() -> Result<()>
{
    let action = build_action("Still", FIRST_SITE)?;
    let mut collision = world(true, true, 2)?;
    put(
        &mut collision,
        created_entity_id("collision", 0),
        EntityKind::Item,
        Vec::new(),
    )?;
    let before = collision.clone();
    assert!(apply(&mut collision, &action, "collision").is_err());
    assert_eq!(collision, before);

    let mut overflow = world(true, true, 1)?;
    apply(&mut overflow, &action, "overflow")?;
    set(
        &mut overflow,
        LEDGER,
        STOCK_WOOD_FIELD,
        Value::U64(u64::MAX),
    )?;
    let before = overflow.clone();
    assert!(cancel(&mut overflow, &action, "overflow").is_err());
    assert_eq!(overflow, before);

    let mut revision = world(true, true, 1)?;
    record_mut(&mut revision, LEDGER)?.revision = u64::MAX;
    revision.refresh_hash();
    let before = revision.clone();
    assert!(apply(&mut revision, &action, "revision-exhausted").is_err());
    assert_eq!(revision, before);

    let mut source = world(true, true, 2)?;
    build(&mut source, "Still", FIRST_SITE, "budget-first")?;
    build(&mut source, "Kitchen", SECOND_SITE, "budget-second")?;
    for limits in [
        EffectAdvanceLimits {
            max_events: 1,
            ..EffectAdvanceLimits::default()
        },
        EffectAdvanceLimits {
            max_work_units: 1,
            ..EffectAdvanceLimits::default()
        },
    ] {
        let mut snapshot = source.clone();
        assert!(advance_with(&mut snapshot, 1_000, limits).is_err());
        assert_eq!(snapshot, source);
        assert_inventory(&snapshot, 0, 2, 0, 0)?;
    }
    assert_inventory(&after(&source, &[1_000])?, 0, 0, 2, 0)?;
    Ok(())
}

// The pre-material Build record and completion writes are reconstructed
// independently. Comparing full canonical bytes catches extra stamps, changed
// revisions or stock writes even when the visible construction result matches.
fn legacy_reference(source: &WorldSnapshot, action: &Action, key: &str) -> Result<WorldSnapshot> {
    let Action::Build {
        kind,
        location,
        footprint,
        material,
    } = action
    else {
        return Err(precondition("legacy reference requires a Build action"));
    };
    let staffed = laboratory_workforce_model(source)?.is_some();
    let mut expected = source.clone();
    let label = building_kind_label(kind);
    let building = created_entity_id(key, 0);
    create_entity(
        &mut expected,
        building,
        EntityKind::Building,
        label.clone(),
        vec![
            ("building_kind".to_owned(), Value::Text(label)),
            ("position".to_owned(), Value::Coord(*location)),
            ("footprint_min".to_owned(), Value::Coord(footprint.min)),
            ("footprint_max".to_owned(), Value::Coord(footprint.max)),
            (
                "material_tokens".to_owned(),
                Value::List(
                    material
                        .required_tokens
                        .iter()
                        .cloned()
                        .map(Value::Text)
                        .collect(),
                ),
            ),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_PLANNED.to_owned()),
            ),
            ("progress_ticks".to_owned(), Value::U64(0)),
            ("required_ticks".to_owned(), Value::U64(BUILD_TICKS)),
        ],
    )?;
    if staffed {
        write_fields(
            &mut expected,
            building,
            vec![(
                LABORATORY_WORKFORCE_TASK_MODEL_FIELD.to_owned(),
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            )],
        )?;
    }
    expected.tick = GameTick(source.tick.0 + BUILD_TICKS);
    write_fields(
        &mut expected,
        building,
        vec![
            ("progress_ticks".to_owned(), Value::U64(BUILD_TICKS)),
            (
                CONSTRUCTION_STAGE_FIELD.to_owned(),
                Value::Text(STAGE_COMPLETE.to_owned()),
            ),
        ],
    )?;
    if staffed {
        write_fields(
            &mut expected,
            building,
            vec![(BLOCKED_BY_FIELD.to_owned(), Value::Null)],
        )?;
    }
    advance_metabolism(&mut expected, BUILD_TICKS)?;
    expected.refresh_hash();
    Ok(expected)
}

#[test]
fn absent_material_model_preserves_exact_legacy_world_bytes_and_selector_interpretation()
-> Result<()> {
    for staffed in [false, true] {
        for kind in ["Still", "Kitchen", "Carpenters", "Masons"] {
            let mut source = world(staffed, false, 0)?;
            fact_mut(&mut source, LEDGER, STOCK_WOOD_FIELD)?.presence = Some(
                FactPresence::Omitted("legacy construction needs no wood observation".to_owned()),
            );
            source.refresh_hash();
            let mut action = build_action(kind, FIRST_SITE)?;
            if let Action::Build { material, .. } = &mut action {
                material.required_tokens = BTreeSet::from(["STONE".to_owned()]);
                material.forbidden_tokens = BTreeSet::from(["STONE".to_owned()]);
                material.prefer_nearest = true;
                material.reserve_count = 7;
            }
            assert_eq!(construction_material_model(&source)?, None);
            assert_eq!(construction_wood_cost(&source, &action)?, None);
            assert!(construction_material_report(&source)?.is_none());
            assert!(construction_completion_predicates(&source, &action, "legacy")?.is_empty());
            let expected = legacy_reference(&source, &action, "legacy")?;
            let mut actual = source;
            apply(&mut actual, &action, "legacy")?;
            advance(&mut actual, BUILD_TICKS)?;
            assert_eq!(
                actual.canonical_bytes(),
                expected.canonical_bytes(),
                "legacy {staffed}/{kind}"
            );
            assert!(
                !entity(&actual, created_entity_id("legacy", 0))?
                    .fields
                    .contains_key(CONSTRUCTION_MATERIAL_MODEL_FIELD)
            );
        }
    }
    Ok(())
}
