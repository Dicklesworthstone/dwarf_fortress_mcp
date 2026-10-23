use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::{FactPresence, WorldGraph};

const WORKER: EntityId = EntityId::new(11);
const CARPENTERS: EntityId = EntityId::new(51);
const LEDGER: EntityId = EntityId::new(91);

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
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn world() -> Result<WorldSnapshot> {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    put(
        &mut snapshot,
        WORKER,
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.CARPENTRY", Value::Bool(true)),
            ("labor.BREW", Value::Bool(false)),
            ("labor.COOK", Value::Bool(false)),
        ],
    )?;
    workshop(&mut snapshot, CARPENTERS, "Carpenters")?;
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        vec![
            (STOCK_DRINK_FIELD, Value::U64(100)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
            (STOCK_WOOD_FIELD, Value::U64(10)),
            (STOCK_BED_FIELD, Value::U64(0)),
            (STOCK_DOOR_FIELD, Value::U64(0)),
            (STOCK_TABLE_FIELD, Value::U64(0)),
            (STOCK_CHAIR_FIELD, Value::U64(0)),
        ],
    )?;
    Ok(snapshot)
}

fn workshop(snapshot: &mut WorldSnapshot, id: EntityId, kind: &str) -> Result<()> {
    put(
        snapshot,
        id,
        EntityKind::Building,
        vec![
            ("building_kind", Value::Text(format!("workshop:{kind}"))),
            (
                CONSTRUCTION_STAGE_FIELD,
                Value::Text(STAGE_COMPLETE.to_owned()),
            ),
        ],
    )
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(field.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn order(
    snapshot: &mut WorldSnapshot,
    key: &str,
    job: &str,
    amount: u32,
    conditions: Vec<WorkOrderCondition>,
) -> Result<EntityId> {
    apply_effect(
        snapshot,
        &Action::CreateWorkOrder {
            name: key.to_owned(),
            job_token: job.to_owned(),
            amount,
            conditions,
        },
        key,
    )?;
    snapshot.refresh_hash();
    Ok(created_entity_id(key, 0))
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

fn count(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, field, snapshot.tick)
}

fn item_count(snapshot: &WorldSnapshot) -> usize {
    snapshot
        .graph
        .entities
        .values()
        .filter(|entity| entity.kind == EntityKind::Item)
        .count()
}

fn physical(snapshot: &WorldSnapshot) -> WorldGraph {
    let mut graph = snapshot.graph.clone();
    for entity in graph.entities.values_mut() {
        entity.revision = 0;
        for fact in entity.fields.values_mut() {
            fact.observed_at = GameTick(0);
        }
    }
    graph
}

#[test]
fn closed_recipes_consume_logs_and_create_exact_distinct_items() -> Result<()> {
    for (job, token, field) in [
        ("CONSTRUCT_BED", "BED", STOCK_BED_FIELD),
        ("wooden_bed", "BED", STOCK_BED_FIELD),
        ("CONSTRUCT_DOOR", "DOOR", STOCK_DOOR_FIELD),
        ("wooden_door", "DOOR", STOCK_DOOR_FIELD),
        ("CONSTRUCT_TABLE", "TABLE", STOCK_TABLE_FIELD),
        ("wooden_table", "TABLE", STOCK_TABLE_FIELD),
        ("CONSTRUCT_THRONE", "CHAIR", STOCK_CHAIR_FIELD),
        ("CONSTRUCT_CHAIR", "CHAIR", STOCK_CHAIR_FIELD),
        ("wooden_chair", "CHAIR", STOCK_CHAIR_FIELD),
    ] {
        let mut snapshot = world()?;
        let id = order(&mut snapshot, "furniture", job, 2, Vec::new())?;
        assert_eq!(work_order_output_item(job), Some(token));
        assert_eq!(work_order_product(job), Some((field, 1)));
        assert_eq!(work_order_material(job), Some((STOCK_WOOD_FIELD, 1)));
        assert_eq!(
            work_order_requirements(job),
            Some(("workshop:Carpenters", "CARPENTRY"))
        );
        advance(&mut snapshot, 100)?;
        assert_eq!(count(&snapshot, LEDGER, field)?, 2);
        assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 8);
        assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 0);
        assert_eq!(item_count(&snapshot), 2);
        for ordinal in 1..=2 {
            let item = entity(&snapshot, produced_item_id(id, ordinal))?;
            assert_eq!(item.kind, EntityKind::Item);
            assert_eq!(field_text(item, "item_token", snapshot.tick), Some(token));
            assert_eq!(
                field_text(item, "material_token", snapshot.tick),
                Some("WOOD")
            );
            assert_eq!(field_u64(item, "stack_size", snapshot.tick)?, 1);
            assert_eq!(
                field_value(item, "source_order", snapshot.tick),
                Some(&Value::Entity(id))
            );
            assert_eq!(
                field_u64(item, "production_ordinal", snapshot.tick)?,
                u64::from(ordinal)
            );
            assert!(!item.fields.contains_key("position"));
            assert!(!item.fields.contains_key("native_item_id"));
        }
        let bytes = snapshot.canonical_bytes();
        let mut reopened = WorldSnapshot::from_canonical_bytes(&bytes)?;
        advance(&mut reopened, 100)?;
        assert_eq!(count(&reopened, LEDGER, field)?, 2);
        assert_eq!(count(&reopened, LEDGER, STOCK_WOOD_FIELD)?, 8);
        assert_eq!(item_count(&reopened), 2);
    }
    assert_eq!(work_order_output_item("CONSTRUCT_BARREL"), None);
    assert_eq!(work_order_material("BREW_DRINK"), None);
    Ok(())
}

#[test]
fn partial_work_and_material_shortage_never_consume_or_bank_waiting_ticks() -> Result<()> {
    let mut snapshot = world()?;
    set(&mut snapshot, LEDGER, STOCK_WOOD_FIELD, Value::U64(1))?;
    let id = order(&mut snapshot, "beds", "CONSTRUCT_BED", 2, Vec::new())?;
    advance(&mut snapshot, 49)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 1);
    assert_eq!(item_count(&snapshot), 0);
    advance(&mut snapshot, 101)?;
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
    assert_eq!(count(&snapshot, id, "work_ticks")?, 0);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 0);
    assert_eq!(item_count(&snapshot), 1);
    assert!(
        field_text(entity(&snapshot, id)?, BLOCKED_BY_FIELD, snapshot.tick)
            .is_some_and(|reason| reason.contains("WOOD"))
    );
    set(&mut snapshot, LEDGER, STOCK_WOOD_FIELD, Value::U64(1))?;
    advance(&mut snapshot, 49)?;
    assert_eq!(item_count(&snapshot), 1);
    advance(&mut snapshot, 1)?;
    assert_eq!(item_count(&snapshot), 2);
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 0);
    Ok(())
}

#[test]
fn same_tick_material_competition_is_partition_invariant() -> Result<()> {
    let mut source = world()?;
    put(
        &mut source,
        EntityId::new(12),
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.CARPENTRY", Value::Bool(true)),
        ],
    )?;
    workshop(&mut source, EntityId::new(52), "Carpenters")?;
    set(&mut source, LEDGER, STOCK_WOOD_FIELD, Value::U64(1))?;
    let a = order(&mut source, "bed", "CONSTRUCT_BED", 1, Vec::new())?;
    let b = order(&mut source, "chair", "CONSTRUCT_THRONE", 1, Vec::new())?;
    let mut whole = source.clone();
    advance(&mut whole, 100)?;
    let mut partitioned = source;
    for _ in 0..100 {
        advance(&mut partitioned, 1)?;
    }
    assert_eq!(physical(&whole), physical(&partitioned));
    assert_eq!(item_count(&whole), 1);
    assert_eq!(count(&whole, a.min(b), AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(count(&whole, a.max(b), "work_ticks")?, 49);
    set(&mut whole, LEDGER, STOCK_WOOD_FIELD, Value::U64(1))?;
    advance(&mut whole, 1)?;
    assert_eq!(item_count(&whole), 2);
    assert_eq!(count(&whole, a.max(b), AMOUNT_REMAINING_FIELD)?, 0);
    Ok(())
}

#[test]
fn all_carpentry_recipes_share_one_workshop_and_worker_pool() -> Result<()> {
    let mut snapshot = world()?;
    for (key, job) in [
        ("bed", "CONSTRUCT_BED"),
        ("door", "CONSTRUCT_DOOR"),
        ("table", "CONSTRUCT_TABLE"),
        ("chair", "CONSTRUCT_THRONE"),
    ] {
        order(&mut snapshot, key, job, 1, Vec::new())?;
    }
    advance(&mut snapshot, 50)?;
    assert_eq!(item_count(&snapshot), 1);
    advance(&mut snapshot, 150)?;
    assert_eq!(item_count(&snapshot), 4);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 6);
    for field in [
        STOCK_BED_FIELD,
        STOCK_DOOR_FIELD,
        STOCK_TABLE_FIELD,
        STOCK_CHAIR_FIELD,
    ] {
        assert_eq!(count(&snapshot, LEDGER, field)?, 1);
    }
    Ok(())
}

fn maximum_matching(masks: [u8; 3], role: usize, used: u8) -> usize {
    if role == 3 {
        return 0;
    }
    let mut best = maximum_matching(masks, role + 1, used);
    for (worker, mask) in masks.iter().enumerate() {
        if used & (1 << worker) == 0 && mask & (1 << role) != 0 {
            best = best.max(1 + maximum_matching(masks, role + 1, used | (1 << worker)));
        }
    }
    best
}

#[test]
fn carpentry_brewing_and_cooking_use_maximum_distinct_worker_matching() -> Result<()> {
    for encoded in 0u16..512 {
        let masks = [
            (encoded & 7) as u8,
            ((encoded >> 3) & 7) as u8,
            ((encoded >> 6) & 7) as u8,
        ];
        let mut snapshot = world()?;
        for worker in [EntityId::new(12), EntityId::new(13)] {
            put(
                &mut snapshot,
                worker,
                EntityKind::Unit,
                vec![("alive", Value::Bool(true))],
            )?;
        }
        for (worker, mask) in [WORKER, EntityId::new(12), EntityId::new(13)]
            .into_iter()
            .zip(masks)
        {
            for (role, labor) in ["BREW", "COOK", "CARPENTRY"].into_iter().enumerate() {
                set(
                    &mut snapshot,
                    worker,
                    &format!("labor.{labor}"),
                    Value::Bool(mask & (1 << role) != 0),
                )?;
            }
        }
        workshop(&mut snapshot, EntityId::new(52), "Still")?;
        workshop(&mut snapshot, EntityId::new(53), "Kitchen")?;
        let mut orders = Vec::new();
        for (key, job) in [
            ("brew", "BREW_DRINK"),
            ("cook", "PREPARE_MEAL"),
            ("bed", "CONSTRUCT_BED"),
        ] {
            orders.push(order(&mut snapshot, key, job, 1, Vec::new())?);
        }
        advance(&mut snapshot, 50)?;
        let mut completed = 0;
        for id in orders {
            completed += usize::from(count(&snapshot, id, AMOUNT_REMAINING_FIELD)? == 0);
        }
        assert_eq!(
            completed,
            maximum_matching(masks, 0, 0),
            "skill masks {masks:?}"
        );
    }
    Ok(())
}

#[test]
fn output_threshold_and_material_conditions_remain_conjunctive() -> Result<()> {
    let mut snapshot = world()?;
    set(&mut snapshot, LEDGER, STOCK_WOOD_FIELD, Value::U64(3))?;
    let id = order(
        &mut snapshot,
        "chairs",
        "CONSTRUCT_THRONE",
        5,
        vec![
            WorkOrderCondition::ItemCountBelow {
                item_token: "CHAIR".to_owned(),
                threshold: 4,
            },
            WorkOrderCondition::MaterialAvailable {
                material_token: "WOOD".to_owned(),
                minimum: 2,
            },
        ],
    )?;
    advance(&mut snapshot, 250)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_CHAIR_FIELD)?, 2);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 1);
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 3);
    set(&mut snapshot, LEDGER, STOCK_WOOD_FIELD, Value::U64(10))?;
    advance(&mut snapshot, 150)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_CHAIR_FIELD)?, 4);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 8);
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
    assert_eq!(item_count(&snapshot), 4);
    Ok(())
}

#[test]
fn named_prerequisites_cannot_retroactively_produce_carpentry() -> Result<()> {
    let mut snapshot = world()?;
    order(&mut snapshot, "door", "CONSTRUCT_DOOR", 1, Vec::new())?;
    let table = order(
        &mut snapshot,
        "table",
        "CONSTRUCT_TABLE",
        1,
        vec![WorkOrderCondition::CompletedOrder {
            order_name: "door".to_owned(),
        }],
    )?;
    advance(&mut snapshot, 50)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_DOOR_FIELD)?, 1);
    assert_eq!(count(&snapshot, table, "work_ticks")?, 0);
    assert_eq!(count(&snapshot, LEDGER, STOCK_TABLE_FIELD)?, 0);
    advance(&mut snapshot, 50)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_TABLE_FIELD)?, 1);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 8);
    Ok(())
}

#[test]
fn absent_untrusted_and_unavailable_inventory_never_authorizes_production() -> Result<()> {
    for field in [STOCK_WOOD_FIELD, STOCK_BED_FIELD] {
        for variant in 0..7 {
            let mut snapshot = world()?;
            let id = order(&mut snapshot, "bed", "CONSTRUCT_BED", 1, Vec::new())?;
            let ledger = snapshot
                .graph
                .entities
                .get_mut(&LEDGER)
                .ok_or_else(|| precondition("fixture ledger missing"))?;
            if variant == 0 {
                ledger.fields.remove(field);
            } else {
                let fact = ledger
                    .fields
                    .get_mut(field)
                    .ok_or_else(|| precondition("fixture count missing"))?;
                match variant {
                    1 => fact.presence = Some(FactPresence::Unknown("missing".to_owned())),
                    2 => fact.presence = Some(FactPresence::Omitted("not captured".to_owned())),
                    3 => fact.source = FactSource::AgentAssertion("claimed stock".to_owned()),
                    4 => fact.source = FactSource::Replay,
                    5 => fact.source = FactSource::Derived("unregistered".to_owned()),
                    _ => fact.value = Value::I64(10),
                }
            }
            snapshot.refresh_hash();
            advance(&mut snapshot, 100)?;
            assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
            assert_eq!(count(&snapshot, id, "work_ticks")?, 0);
            assert_eq!(item_count(&snapshot), 0);
            assert!(
                field_text(entity(&snapshot, id)?, BLOCKED_BY_FIELD, snapshot.tick)
                    .is_some_and(|reason| reason.contains(field))
            );
        }
    }
    Ok(())
}

#[test]
fn legacy_abstract_orders_and_unknown_life_do_not_gain_carpentry_authority() -> Result<()> {
    for variant in 0..4 {
        let mut snapshot = world()?;
        let id = order(&mut snapshot, "bed", "CONSTRUCT_BED", 1, Vec::new())?;
        let (entity_id, field) = if variant < 2 {
            (id, WORK_ORDER_PRODUCTION_MODEL_FIELD)
        } else {
            (WORKER, "alive")
        };
        let record = snapshot
            .graph
            .entities
            .get_mut(&entity_id)
            .ok_or_else(|| precondition("fixture source missing"))?;
        if variant % 2 == 0 {
            record.fields.remove(field);
        } else if let Some(fact) = record.fields.get_mut(field) {
            fact.source = FactSource::AgentAssertion("untrusted".to_owned());
        }
        snapshot.refresh_hash();
        // Explicit unknown life also prevents the population model advancing;
        // missing life retains its old population default but not carpentry.
        if variant == 3 {
            let before = snapshot.clone();
            assert!(advance(&mut snapshot, 50).is_err());
            assert_eq!(snapshot, before);
        } else {
            advance(&mut snapshot, 50)?;
            assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
            assert_eq!(item_count(&snapshot), 0);
            assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 10);
        }
    }
    Ok(())
}

#[test]
fn collision_overflow_future_source_and_invalid_totals_preserve_atomic_world() -> Result<()> {
    for variant in 0..5 {
        let mut snapshot = world()?;
        let id = order(&mut snapshot, "bed", "CONSTRUCT_BED", 2, Vec::new())?;
        match variant {
            0 => put(
                &mut snapshot,
                produced_item_id(id, 2),
                EntityKind::Item,
                Vec::new(),
            )?,
            1 => set(
                &mut snapshot,
                LEDGER,
                STOCK_BED_FIELD,
                Value::U64(u64::MAX - 1),
            )?,
            2 => {
                let future = snapshot.tick.0 + 1;
                let ledger = snapshot
                    .graph
                    .entities
                    .get_mut(&LEDGER)
                    .ok_or_else(|| precondition("fixture ledger missing"))?;
                let fact = ledger
                    .fields
                    .get_mut(STOCK_WOOD_FIELD)
                    .ok_or_else(|| precondition("fixture wood missing"))?;
                fact.observed_at = GameTick(future);
                snapshot.refresh_hash();
            }
            3 => set(&mut snapshot, id, "amount_total", Value::U64(1))?,
            _ => set(
                &mut snapshot,
                id,
                "amount_total",
                Value::U64(u64::from(u32::MAX) + 1),
            )?,
        }
        let before = snapshot.clone();
        assert!(advance(&mut snapshot, 100).is_err());
        assert_eq!(snapshot, before);
    }
    Ok(())
}

#[test]
fn bounded_item_growth_refuses_whole_advance_after_first_possible_output() -> Result<()> {
    let mut snapshot = world()?;
    order(&mut snapshot, "beds", "CONSTRUCT_BED", 2, Vec::new())?;
    let mut next = 1_000;
    while snapshot.graph.entities.len() < MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES - 1 {
        create_entity(
            &mut snapshot,
            EntityId::new(next),
            EntityKind::Item,
            "bounded inventory".to_owned(),
            Vec::new(),
        )?;
        next += 1;
    }
    snapshot.refresh_hash();
    let before = snapshot.clone();
    let error = advance(&mut snapshot, 100)
        .err()
        .ok_or_else(|| precondition("oversized inventory unexpectedly succeeded"))?;
    assert_eq!(error.code, ErrorCode::BudgetExceeded);
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn cancellation_preserves_produced_items_and_spent_logs() -> Result<()> {
    let mut snapshot = world()?;
    let action = Action::CreateWorkOrder {
        name: "beds".to_owned(),
        job_token: "CONSTRUCT_BED".to_owned(),
        amount: 3,
        conditions: Vec::new(),
    };
    apply_effect(&mut snapshot, &action, "beds")?;
    snapshot.refresh_hash();
    advance(&mut snapshot, 75)?;
    cancel_effect(&mut snapshot, &action, "beds")?;
    snapshot.refresh_hash();
    advance(&mut snapshot, 500)?;
    assert_eq!(item_count(&snapshot), 1);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 9);
    assert_eq!(count(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    assert_eq!(
        count(
            &snapshot,
            created_entity_id("beds", 0),
            AMOUNT_REMAINING_FIELD
        )?,
        2
    );
    Ok(())
}
