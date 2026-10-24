//! Independent conservation, chronology and source-custody checks for the
//! separately selected finite food/drink reference model.

use super::*;
use dfmcp_core::ObservationCursor;
use dfmcp_world::{FactPresence, WorldGraph};

const LEDGER: EntityId = EntityId::new(91);
const BREWER: EntityId = EntityId::new(11);
const COOK: EntityId = EntityId::new(12);
const MODEL_FIELD: &str = "laboratory_provisions_model";
const MODEL: &str = "plant-provisions/1";
const PLANTS: &str = "stock.material.PLANT";
const RECEIPT_LEDGER: &str = "provisions_ledger";
const RECEIPT_GENERATION: &str = "provisions_ledger_generation";
const RECEIPT_CONSUMED: &str = "provisions_consumed_plants";

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
            .map(|(field, value)| (field.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn source(plants: u64, provisions: bool, shared: bool) -> Result<WorldSnapshot> {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(719),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    for (id, labor) in [(BREWER, "BREW"), (COOK, "COOK")] {
        put(
            &mut snapshot,
            id,
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.BREW", Value::Bool(labor == "BREW")),
                ("labor.COOK", Value::Bool(labor == "COOK")),
                ("labor.CARPENTRY", Value::Bool(false)),
                ("labor.MINING", Value::Bool(false)),
                ("labor.CONSTRUCTION", Value::Bool(false)),
            ],
        )?;
    }
    for (id, workshop) in [(51, "Still"), (52, "Kitchen"), (53, "Carpenters")] {
        put(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Building,
            vec![
                ("building_kind", Value::Text(format!("workshop:{workshop}"))),
                (
                    CONSTRUCTION_STAGE_FIELD,
                    Value::Text(STAGE_COMPLETE.to_owned()),
                ),
            ],
        )?;
    }
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        vec![
            (PLANTS, Value::U64(plants)),
            (STOCK_DRINK_FIELD, Value::U64(100)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
            (STOCK_WOOD_FIELD, Value::U64(10)),
            (STOCK_BED_FIELD, Value::U64(0)),
        ],
    )?;
    let mut models = Vec::new();
    if shared {
        models.push((
            LABORATORY_WORKFORCE_MODEL_FIELD,
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        ));
    }
    if provisions {
        models.push((MODEL_FIELD, Value::Text(MODEL.to_owned())));
    }
    if !models.is_empty() {
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        put(
            &mut snapshot,
            settings,
            EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
            models,
        )?;
    }
    Ok(snapshot)
}

fn world(plants: u64) -> Result<WorldSnapshot> {
    source(plants, true, true)
}

fn record_mut(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<&mut EntityRecord> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| precondition("provisions test record missing"))
}

fn fact_mut<'a>(
    snapshot: &'a mut WorldSnapshot,
    id: EntityId,
    field: &str,
) -> Result<&'a mut Fact> {
    record_mut(snapshot, id)?
        .fields
        .get_mut(field)
        .ok_or_else(|| precondition("provisions test fact missing"))
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(field.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn action(name: &str, job: &str, amount: u32, conditions: Vec<WorkOrderCondition>) -> Action {
    Action::CreateWorkOrder {
        name: name.to_owned(),
        job_token: job.to_owned(),
        amount,
        conditions,
    }
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<bool> {
    let mut shadow = snapshot.clone();
    let changed = apply_effect(&mut shadow, action, key)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(changed)
}

fn order(snapshot: &mut WorldSnapshot, key: &str, job: &str, amount: u32) -> Result<EntityId> {
    apply(snapshot, &action(key, job, amount, Vec::new()), key)?;
    Ok(created_entity_id(key, 0))
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
    elapsed: u64,
    limits: EffectAdvanceLimits,
) -> Result<()> {
    let mut shadow = snapshot.clone();
    shadow.tick = GameTick(
        snapshot
            .tick
            .0
            .checked_add(elapsed)
            .ok_or_else(|| advance_budget_error("provisions test clock overflow"))?,
    );
    advance_effects_with_limits(&mut shadow, elapsed, limits)?;
    shadow.refresh_hash();
    *snapshot = shadow;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<()> {
    advance_with(snapshot, elapsed, EffectAdvanceLimits::default())
}

fn count(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, field, snapshot.tick)
}

fn report(snapshot: &WorldSnapshot) -> Result<ProvisionsReport> {
    provisions_report(snapshot)?.ok_or_else(|| precondition("provisions test model missing"))
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

fn assert_conserved(snapshot: &WorldSnapshot, original_plants: u64) -> Result<()> {
    let report = report(snapshot)?;
    assert_eq!(report.model, MODEL);
    assert_eq!(report.ledger, LEDGER);
    assert_eq!(report.ledger_generation, 1);
    assert_eq!(
        report.available_plants + report.consumed_plants,
        original_plants
    );
    let produced = count(snapshot, LEDGER, STOCK_DRINK_FIELD)?
        + count(snapshot, LEDGER, STOCK_FOOD_FIELD)?
        - 200;
    assert_eq!(produced, 5 * report.consumed_plants);
    Ok(())
}

#[test]
fn all_three_recipes_debit_one_plant_only_when_each_batch_completes() -> Result<()> {
    for (job, output) in [
        ("BREW_DRINK", STOCK_DRINK_FIELD),
        ("PREPARE_MEAL", STOCK_FOOD_FIELD),
        ("COOK_MEAL", STOCK_FOOD_FIELD),
    ] {
        let mut snapshot = world(2)?;
        let requested = action("two batches", job, 2, Vec::new());
        assert_eq!(provisions_input_cost(&snapshot, &requested)?, Some(2));
        apply(&mut snapshot, &requested, job)?;
        let id = created_entity_id(job, 0);
        assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 2);
        assert_eq!(count(&snapshot, id, RECEIPT_CONSUMED)?, 0);
        let initial = report(&snapshot)?;
        assert_eq!(initial.remaining_demand_plants, 2);
        assert_eq!(initial.consumed_plants, 0);
        assert_eq!(initial.orders.len(), 1);
        assert_eq!(initial.orders[0].job, job);
        assert_eq!(initial.orders[0].amount_total, 2);
        assert_eq!(initial.orders[0].status, "active");
        assert_eq!(
            field_value(entity(&snapshot, id)?, RECEIPT_LEDGER, snapshot.tick),
            Some(&Value::Entity(LEDGER))
        );
        assert_eq!(count(&snapshot, id, RECEIPT_GENERATION)?, 1);
        advance(&mut snapshot, 49)?;
        assert_eq!(count(&snapshot, LEDGER, output)?, 100);
        assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 2);
        assert_eq!(count(&snapshot, id, "work_ticks")?, 49);
        advance(&mut snapshot, 1)?;
        assert_eq!(count(&snapshot, LEDGER, output)?, 105);
        assert_eq!(count(&snapshot, id, RECEIPT_CONSUMED)?, 1);
        assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
        advance(&mut snapshot, 50)?;
        assert_eq!(count(&snapshot, LEDGER, output)?, 110);
        assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 0);
        assert_eq!(report(&snapshot)?.remaining_demand_plants, 0);
        validate_provisions_completion(&snapshot, id, &requested)?;
        assert_conserved(&snapshot, 2)?;
        let mut reopened = WorldSnapshot::from_canonical_bytes(&snapshot.canonical_bytes())?;
        advance(&mut reopened, 100)?;
        assert_conserved(&reopened, 2)?;
        assert_eq!(report(&reopened)?.orders[0].status, "complete");
    }
    Ok(())
}

#[test]
fn empty_inventory_stalls_without_reservation_or_banked_service_then_restock_resumes() -> Result<()>
{
    let mut snapshot = world(0)?;
    let id = order(&mut snapshot, "starved", "BREW_DRINK", 2)?;
    advance(&mut snapshot, 200)?;
    assert_eq!(count(&snapshot, id, "work_ticks")?, 0);
    assert_eq!(count(&snapshot, id, RECEIPT_CONSUMED)?, 0);
    assert_eq!(report(&snapshot)?.remaining_demand_plants, 2);
    assert!(
        field_text(entity(&snapshot, id)?, BLOCKED_BY_FIELD, snapshot.tick)
            .is_some_and(|reason| reason.contains("PLANT"))
    );
    set(&mut snapshot, LEDGER, PLANTS, Value::U64(1))?;
    advance(&mut snapshot, 25)?;
    assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 1);
    assert_eq!(count(&snapshot, id, "work_ticks")?, 25);
    set(&mut snapshot, LEDGER, PLANTS, Value::U64(0))?;
    advance(&mut snapshot, 150)?;
    assert_eq!(count(&snapshot, id, "work_ticks")?, 25);
    set(&mut snapshot, LEDGER, PLANTS, Value::U64(1))?;
    advance(&mut snapshot, 24)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_DRINK_FIELD)?, 100);
    advance(&mut snapshot, 1)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_DRINK_FIELD)?, 105);
    assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 0);
    advance(&mut snapshot, 100)?;
    assert_eq!(count(&snapshot, id, "work_ticks")?, 0);
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
    Ok(())
}

#[test]
fn simultaneous_brewing_and_cooking_share_the_last_plant_independently_of_wait_cuts() -> Result<()>
{
    let mut source = world(1)?;
    let brew = order(&mut source, "competing brew", "BREW_DRINK", 1)?;
    let cook = order(&mut source, "competing cook", "PREPARE_MEAL", 1)?;
    let mut whole = source.clone();
    advance(&mut whole, 200)?;
    let mut single_ticks = source;
    for _ in 0..200 {
        advance(&mut single_ticks, 1)?;
    }
    assert_eq!(physical(&whole), physical(&single_ticks));
    assert_conserved(&whole, 1)?;
    assert_eq!(count(&whole, brew.min(cook), AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(count(&whole, brew.max(cook), AMOUNT_REMAINING_FIELD)?, 1);
    assert_eq!(count(&whole, brew.max(cook), "work_ticks")?, 49);
    assert_eq!(report(&whole)?.remaining_demand_plants, 1);
    advance(&mut whole, 100)?;
    assert_eq!(count(&whole, brew.max(cook), "work_ticks")?, 49);
    set(&mut whole, LEDGER, PLANTS, Value::U64(1))?;
    advance(&mut whole, 1)?;
    assert_eq!(count(&whole, brew.max(cook), AMOUNT_REMAINING_FIELD)?, 0);
    assert_conserved(&whole, 2)?;
    Ok(())
}

#[test]
fn conditions_gate_each_batch_without_consuming_the_threshold_or_counting_future_outputs()
-> Result<()> {
    let mut snapshot = world(3)?;
    let requested = action(
        "conditional meals",
        "PREPARE_MEAL",
        5,
        vec![
            WorkOrderCondition::ItemCountBelow {
                item_token: "FOOD".to_owned(),
                threshold: 117,
            },
            WorkOrderCondition::MaterialAvailable {
                material_token: "PLANT".to_owned(),
                minimum: 2,
            },
        ],
    );
    apply(&mut snapshot, &requested, "conditional")?;
    let id = created_entity_id("conditional", 0);
    advance(&mut snapshot, 250)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_FOOD_FIELD)?, 110);
    assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 1);
    assert_eq!(count(&snapshot, id, RECEIPT_CONSUMED)?, 2);
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 3);
    assert_eq!(count(&snapshot, id, "work_ticks")?, 0);
    set(&mut snapshot, LEDGER, PLANTS, Value::U64(5))?;
    advance(&mut snapshot, 250)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_FOOD_FIELD)?, 120);
    assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 3);
    assert_eq!(count(&snapshot, id, RECEIPT_CONSUMED)?, 4);
    assert_eq!(count(&snapshot, id, AMOUNT_REMAINING_FIELD)?, 1);
    Ok(())
}

#[test]
fn named_prerequisite_uses_the_input_only_in_a_later_eligible_interval() -> Result<()> {
    let mut snapshot = world(2)?;
    order(&mut snapshot, "first brew", "BREW_DRINK", 1)?;
    let requested = action(
        "then cook",
        "COOK_MEAL",
        1,
        vec![WorkOrderCondition::CompletedOrder {
            order_name: "first brew".to_owned(),
        }],
    );
    apply(&mut snapshot, &requested, "dependent cook")?;
    let cook = created_entity_id("dependent cook", 0);
    advance(&mut snapshot, 50)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_DRINK_FIELD)?, 105);
    assert_eq!(count(&snapshot, LEDGER, STOCK_FOOD_FIELD)?, 100);
    assert_eq!(count(&snapshot, cook, "work_ticks")?, 0);
    assert_conserved(&snapshot, 2)?;
    advance(&mut snapshot, 50)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_FOOD_FIELD)?, 105);
    assert_conserved(&snapshot, 2)?;
    Ok(())
}

#[test]
fn cancellation_keeps_consumed_history_and_never_refunds_unreserved_or_spent_plants() -> Result<()>
{
    for elapsed in [25, 75, 150] {
        let mut snapshot = world(3)?;
        let requested = action("cancel meals", "COOK_MEAL", 3, Vec::new());
        apply(&mut snapshot, &requested, "cancel meals")?;
        let id = created_entity_id("cancel meals", 0);
        advance(&mut snapshot, elapsed)?;
        let completed = elapsed / 50;
        assert_eq!(
            cancel(&mut snapshot, &requested, "cancel meals")?,
            completed < 3
        );
        assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 3 - completed);
        assert_eq!(count(&snapshot, id, RECEIPT_CONSUMED)?, completed);
        assert_eq!(report(&snapshot)?.remaining_demand_plants, 0);
        let once = snapshot.clone();
        assert!(!cancel(&mut snapshot, &requested, "cancel meals")?);
        assert_eq!(snapshot, once);
        let mut recovered = WorldSnapshot::from_canonical_bytes(&snapshot.canonical_bytes())?;
        assert!(!cancel(&mut recovered, &requested, "cancel meals")?);
        advance(&mut recovered, 150)?;
        assert_eq!(count(&recovered, LEDGER, PLANTS)?, 3 - completed);
        assert_eq!(count(&recovered, id, RECEIPT_CONSUMED)?, completed);
        assert_conserved(&recovered, 3)?;
        assert_eq!(
            report(&recovered)?.orders[0].status,
            if completed == 3 {
                "complete"
            } else {
                "cancelled"
            }
        );
    }
    Ok(())
}

#[test]
fn original_action_identity_is_required_for_cancellation_and_completion() -> Result<()> {
    let requested = action("original", "BREW_DRINK", 2, Vec::new());
    let mut snapshot = world(2)?;
    apply(&mut snapshot, &requested, "identity")?;
    advance(&mut snapshot, 75)?;
    let id = created_entity_id("identity", 0);
    validate_provisions_identity(&snapshot, id, &requested)?;
    assert!(validate_provisions_completion(&snapshot, id, &requested).is_err());
    for changed in [
        action("other name", "BREW_DRINK", 2, Vec::new()),
        action("original", "PREPARE_MEAL", 2, Vec::new()),
        action("original", "BREW_DRINK", 1, Vec::new()),
        action(
            "original",
            "BREW_DRINK",
            2,
            vec![WorkOrderCondition::MaterialAvailable {
                material_token: "PLANT".to_owned(),
                minimum: 1,
            }],
        ),
    ] {
        let before = snapshot.clone();
        assert!(validate_provisions_identity(&snapshot, id, &changed).is_err());
        assert!(cancel(&mut snapshot, &changed, "identity").is_err());
        assert_eq!(snapshot, before);
    }
    // Non-provisions validators are intentionally no-ops for unrelated
    // actions. Cancellation must still inspect the original key-owned record
    // before falling back to any generic work-order path.
    for job in ["CONSTRUCT_BED", "MAKE_BED", "UNREGISTERED"] {
        let before = snapshot.clone();
        let unrelated = action("original", job, 2, Vec::new());
        assert!(cancel(&mut snapshot, &unrelated, "identity").is_err());
        assert_eq!(snapshot, before, "wrong cancellation job {job}");
    }
    advance(&mut snapshot, 25)?;
    validate_provisions_completion(&snapshot, id, &requested)?;
    Ok(())
}

#[test]
fn unavailable_input_or_output_facts_refuse_without_publishing_a_production_prefix() -> Result<()> {
    for field in [PLANTS, STOCK_DRINK_FIELD, STOCK_FOOD_FIELD] {
        for variant in 0..10 {
            let mut snapshot = world(2)?;
            order(&mut snapshot, "qualified", "BREW_DRINK", 2)?;
            let future = GameTick(snapshot.tick.0 + 1);
            if variant == 0 {
                record_mut(&mut snapshot, LEDGER)?.fields.remove(field);
            } else {
                let fact = fact_mut(&mut snapshot, LEDGER, field)?;
                match variant {
                    1 => fact.presence = Some(FactPresence::Unknown("unobserved".to_owned())),
                    2 => fact.presence = Some(FactPresence::Omitted("not selected".to_owned())),
                    3 => fact.source = FactSource::AgentAssertion("claimed plants".to_owned()),
                    4 => fact.source = FactSource::Replay,
                    5 => fact.source = FactSource::Derived("unregistered authority".to_owned()),
                    6 => fact.observed_at = future,
                    7 => fact.value = Value::I64(2),
                    8 => fact.presence = Some(FactPresence::Known(Value::U64(999))),
                    _ => fact.presence = Some(FactPresence::Absent),
                }
            }
            snapshot.refresh_hash();
            let before = snapshot.clone();
            assert!(provisions_report(&snapshot).is_err(), "{field}/{variant}");
            assert!(advance(&mut snapshot, 100).is_err(), "{field}/{variant}");
            assert_eq!(snapshot, before, "{field}/{variant}");
        }
    }
    Ok(())
}

#[test]
fn model_identity_and_required_workforce_cannot_be_guessed_from_compatible_values() -> Result<()> {
    let requested = action("model check", "BREW_DRINK", 1, Vec::new());
    for variant in 0..9 {
        let mut snapshot = world(1)?;
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        match variant {
            0 => {
                fact_mut(&mut snapshot, settings, MODEL_FIELD)?.value =
                    Value::Text("plant-provisions/2".to_owned())
            }
            1 => {
                fact_mut(&mut snapshot, settings, MODEL_FIELD)?.source =
                    FactSource::AgentAssertion("opt in".to_owned())
            }
            2 => {
                fact_mut(&mut snapshot, settings, MODEL_FIELD)?.presence =
                    Some(FactPresence::Unknown("model missing".to_owned()))
            }
            3 => record_mut(&mut snapshot, settings)?.generation = 2,
            4 => {
                record_mut(&mut snapshot, settings)?.kind =
                    EntityKind::Other("wrong_settings".to_owned())
            }
            5 => {
                record_mut(&mut snapshot, settings)?
                    .fields
                    .remove(LABORATORY_WORKFORCE_MODEL_FIELD);
            }
            6 => {
                let fact = record_mut(&mut snapshot, settings)?
                    .fields
                    .remove(MODEL_FIELD)
                    .ok_or_else(|| precondition("model fixture missing"))?;
                record_mut(&mut snapshot, LEDGER)?
                    .fields
                    .insert(MODEL_FIELD.to_owned(), fact);
            }
            7 => {
                let duplicate = fact_mut(&mut snapshot, settings, MODEL_FIELD)?.clone();
                record_mut(&mut snapshot, LEDGER)?
                    .fields
                    .insert(MODEL_FIELD.to_owned(), duplicate);
            }
            _ => record_mut(&mut snapshot, settings)?.id = EntityId::new(123),
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            provisions_model(&snapshot).is_err(),
            "model variant {variant}"
        );
        assert!(
            apply(&mut snapshot, &requested, "model check").is_err(),
            "model variant {variant}"
        );
        assert_eq!(snapshot, before, "model variant {variant}");
    }
    Ok(())
}

#[test]
fn ledger_identity_domain_and_original_generation_remain_exact_after_dispatch() -> Result<()> {
    for variant in 0..6 {
        let mut snapshot = world(2)?;
        let id = order(&mut snapshot, "ledger check", "BREW_DRINK", 2)?;
        match variant {
            0 => record_mut(&mut snapshot, LEDGER)?.generation = 2,
            1 => record_mut(&mut snapshot, id)?.generation = 2,
            2 => {
                let mut duplicate = entity(&snapshot, LEDGER)?.clone();
                duplicate.id = EntityId::new(92);
                snapshot.graph.entities.insert(duplicate.id, duplicate);
            }
            3 => record_mut(&mut snapshot, LEDGER)?.id = EntityId::new(92),
            4 => record_mut(&mut snapshot, id)?.kind = EntityKind::Other("work_order".to_owned()),
            _ => {
                let mut replacement = snapshot
                    .graph
                    .entities
                    .remove(&LEDGER)
                    .ok_or_else(|| precondition("original ledger missing"))?;
                replacement.id = EntityId::new(92);
                snapshot.graph.entities.insert(replacement.id, replacement);
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            provisions_report(&snapshot).is_err(),
            "ledger variant {variant}"
        );
        assert!(
            advance(&mut snapshot, 100).is_err(),
            "ledger variant {variant}"
        );
        assert_eq!(snapshot, before, "ledger variant {variant}");
    }
    Ok(())
}

#[test]
fn consumed_input_receipts_and_lifecycle_cannot_be_rewritten_into_apparent_completion() -> Result<()>
{
    for variant in 0..15 {
        let mut snapshot = world(3)?;
        let requested = action("custody", "PREPARE_MEAL", 3, Vec::new());
        apply(&mut snapshot, &requested, "custody")?;
        let id = created_entity_id("custody", 0);
        advance(&mut snapshot, 75)?;
        match variant {
            0 => {
                record_mut(&mut snapshot, id)?
                    .fields
                    .remove(WORK_ORDER_PRODUCTION_MODEL_FIELD);
            }
            1 => set(
                &mut snapshot,
                id,
                WORK_ORDER_PRODUCTION_MODEL_FIELD,
                Value::Text("plant-provisions/2".to_owned()),
            )?,
            2 => set(
                &mut snapshot,
                id,
                RECEIPT_LEDGER,
                Value::Entity(EntityId::new(92)),
            )?,
            3 => set(&mut snapshot, id, RECEIPT_GENERATION, Value::U64(2))?,
            4 => set(&mut snapshot, id, RECEIPT_CONSUMED, Value::U64(0))?,
            5 => set(&mut snapshot, id, RECEIPT_CONSUMED, Value::U64(2))?,
            6 => set(
                &mut snapshot,
                id,
                STATUS_FIELD,
                Value::Text(STATUS_COMPLETE.to_owned()),
            )?,
            7 => set(&mut snapshot, id, "work_ticks", Value::U64(50))?,
            8 => set(&mut snapshot, id, "amount_total", Value::U64(1))?,
            9 => set(
                &mut snapshot,
                id,
                "amount_total",
                Value::U64(u64::from(u32::MAX) + 1),
            )?,
            10 => set(
                &mut snapshot,
                id,
                "job_token",
                Value::Text("CUSTOM_MEAL".to_owned()),
            )?,
            11 => {
                fact_mut(&mut snapshot, id, RECEIPT_CONSUMED)?.source =
                    FactSource::AgentAssertion("consumed".to_owned())
            }
            12 => {
                record_mut(&mut snapshot, id)?
                    .fields
                    .remove(RECEIPT_GENERATION);
            }
            13 => {
                fact_mut(&mut snapshot, id, WORK_ORDER_CONDITIONS_FIELD)?.presence =
                    Some(FactPresence::Unknown("lost conditions".to_owned()))
            }
            _ => fact_mut(&mut snapshot, id, WORK_ORDER_NAME_FIELD)?.source = FactSource::Replay,
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            provisions_report(&snapshot).is_err(),
            "custody variant {variant}"
        );
        assert!(
            advance(&mut snapshot, 100).is_err(),
            "custody variant {variant}"
        );
        assert!(
            cancel(&mut snapshot, &requested, "custody").is_err(),
            "custody variant {variant}"
        );
        assert_eq!(snapshot, before, "custody variant {variant}");
    }
    Ok(())
}

#[test]
fn active_legacy_orders_and_retained_input_history_cannot_cross_model_generations() -> Result<()> {
    let mut legacy = source(2, false, true)?;
    order(&mut legacy, "old brew", "BREW_DRINK", 1)?;
    let settings = fortress_settings_entity_id(legacy.fortress_id);
    set(
        &mut legacy,
        settings,
        MODEL_FIELD,
        Value::Text(MODEL.to_owned()),
    )?;
    let before = legacy.clone();
    assert!(provisions_report(&legacy).is_err());
    assert!(advance(&mut legacy, 50).is_err());
    assert_eq!(legacy, before);

    for finished in [false, true] {
        let mut snapshot = world(1)?;
        order(&mut snapshot, "new brew", "BREW_DRINK", 1)?;
        if finished {
            advance(&mut snapshot, 50)?;
        }
        let settings = fortress_settings_entity_id(snapshot.fortress_id);
        record_mut(&mut snapshot, settings)?
            .fields
            .remove(MODEL_FIELD);
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(provisions_model(&snapshot).is_err());
        assert!(advance(&mut snapshot, 50).is_err());
        assert_eq!(snapshot, before);
    }
    let mut historical = source(1, false, true)?;
    order(&mut historical, "completed legacy", "BREW_DRINK", 1)?;
    advance(&mut historical, 50)?;
    let settings = fortress_settings_entity_id(historical.fortress_id);
    set(
        &mut historical,
        settings,
        MODEL_FIELD,
        Value::Text(MODEL.to_owned()),
    )?;
    assert!(report(&historical)?.orders.is_empty());
    assert_eq!(report(&historical)?.consumed_plants, 0);
    Ok(())
}

#[test]
fn partitioned_time_and_canonical_recovery_conserve_the_same_inputs_outputs_and_history()
-> Result<()> {
    let mut source = world(7)?;
    order(&mut source, "partition brew", "BREW_DRINK", 3)?;
    order(&mut source, "partition prepare", "PREPARE_MEAL", 2)?;
    order(&mut source, "partition cook", "COOK_MEAL", 1)?;
    let mut whole = source.clone();
    advance(&mut whole, 300)?;
    for partitions in [
        vec![1; 300],
        vec![49, 1, 23, 102, 125],
        vec![50, 50, 50, 150],
    ] {
        let mut split = source.clone();
        for elapsed in partitions {
            advance(&mut split, elapsed)?;
            assert_conserved(&split, 7)?;
        }
        assert_eq!(physical(&split), physical(&whole));
        assert_eq!(report(&split)?, report(&whole)?);
    }
    let mut partial = source;
    advance(&mut partial, 75)?;
    let bytes = partial.canonical_bytes();
    let mut reopened = WorldSnapshot::from_canonical_bytes(&bytes)?;
    assert_eq!(reopened.canonical_bytes(), bytes);
    advance(&mut reopened, 225)?;
    assert_eq!(physical(&reopened), physical(&whole));
    assert_eq!(report(&reopened)?.available_plants, 1);
    assert_eq!(report(&reopened)?.consumed_plants, 6);
    assert_eq!(count(&reopened, LEDGER, STOCK_DRINK_FIELD)?, 115);
    assert_eq!(count(&reopened, LEDGER, STOCK_FOOD_FIELD)?, 115);
    Ok(())
}

#[test]
fn budgets_counter_overflow_and_revision_exhaustion_publish_no_partial_batch() -> Result<()> {
    let mut source = world(3)?;
    let id = order(&mut source, "bounded", "BREW_DRINK", 3)?;
    for limits in [
        EffectAdvanceLimits {
            max_events: 1,
            ..EffectAdvanceLimits::default()
        },
        EffectAdvanceLimits {
            max_work_units: 1,
            ..EffectAdvanceLimits::default()
        },
        EffectAdvanceLimits {
            max_game_ticks: 49,
            ..EffectAdvanceLimits::default()
        },
    ] {
        let mut snapshot = source.clone();
        assert!(advance_with(&mut snapshot, 150, limits).is_err());
        assert_eq!(snapshot, source);
        assert_conserved(&snapshot, 3)?;
    }
    for variant in 0..4 {
        let mut snapshot = source.clone();
        match variant {
            0 => set(
                &mut snapshot,
                LEDGER,
                STOCK_DRINK_FIELD,
                Value::U64(u64::MAX - 4),
            )?,
            1 => record_mut(&mut snapshot, LEDGER)?.revision = u64::MAX,
            2 => record_mut(&mut snapshot, id)?.revision = u64::MAX,
            _ => set(
                &mut snapshot,
                LEDGER,
                METABOLISM_TICKS_FIELD,
                Value::U64(u64::MAX - 1),
            )?,
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            advance(&mut snapshot, 150).is_err(),
            "overflow variant {variant}"
        );
        assert_eq!(snapshot, before, "overflow variant {variant}");
    }
    let mut collision = source.clone();
    assert!(
        apply(
            &mut collision,
            &action("bounded", "BREW_DRINK", 3, Vec::new()),
            "bounded"
        )
        .is_err()
    );
    assert_eq!(collision, source);
    advance(&mut source, 150)?;
    assert_conserved(&source, 3)?;
    Ok(())
}

#[test]
fn carpentry_keeps_its_separate_wood_recipe_without_provisions_history() -> Result<()> {
    let mut snapshot = world(0)?;
    set(&mut snapshot, BREWER, "labor.CARPENTRY", Value::Bool(true))?;
    let requested = action("unrelated bed", "CONSTRUCT_BED", 1, Vec::new());
    assert_eq!(provisions_input_cost(&snapshot, &requested)?, None);
    apply(&mut snapshot, &requested, "unrelated bed")?;
    let id = created_entity_id("unrelated bed", 0);
    advance(&mut snapshot, 50)?;
    assert_eq!(count(&snapshot, LEDGER, PLANTS)?, 0);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 9);
    assert_eq!(count(&snapshot, LEDGER, STOCK_BED_FIELD)?, 1);
    assert!(report(&snapshot)?.orders.is_empty());
    assert!(!entity(&snapshot, id)?.fields.contains_key(RECEIPT_CONSUMED));
    Ok(())
}

// Reconstruct the pre-provisions dispatch and one-unit completion writes.
// This independently checks full canonical legacy bytes, including revisions,
// rather than comparing only a few visible quantities.
fn legacy_reference(source: &WorldSnapshot, job: &str, key: &str) -> Result<WorldSnapshot> {
    let mut expected = source.clone();
    let id = created_entity_id(key, 0);
    create_entity(
        &mut expected,
        id,
        EntityKind::WorkOrder,
        key.to_owned(),
        vec![
            (
                WORK_ORDER_NAME_FIELD.to_owned(),
                Value::Text(key.to_owned()),
            ),
            (
                WORK_ORDER_CONDITIONS_FIELD.to_owned(),
                Value::List(Vec::new()),
            ),
            ("job_token".to_owned(), Value::Text(job.to_owned())),
            ("amount_total".to_owned(), Value::U64(1)),
            (AMOUNT_REMAINING_FIELD.to_owned(), Value::U64(1)),
            ("work_ticks".to_owned(), Value::U64(0)),
            (
                STATUS_FIELD.to_owned(),
                Value::Text(STATUS_ACTIVE.to_owned()),
            ),
        ],
    )?;
    if laboratory_workforce_model(source)?.is_some() {
        write_fields(
            &mut expected,
            id,
            vec![(
                LABORATORY_WORKFORCE_TASK_MODEL_FIELD.to_owned(),
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            )],
        )?;
    }
    expected.tick = GameTick(source.tick.0 + 50);
    let output = if job == "BREW_DRINK" {
        STOCK_DRINK_FIELD
    } else {
        STOCK_FOOD_FIELD
    };
    write_fields(
        &mut expected,
        LEDGER,
        vec![(output.to_owned(), Value::U64(105))],
    )?;
    write_fields(
        &mut expected,
        id,
        vec![
            (AMOUNT_REMAINING_FIELD.to_owned(), Value::U64(0)),
            ("work_ticks".to_owned(), Value::U64(0)),
            (
                STATUS_FIELD.to_owned(),
                Value::Text(STATUS_COMPLETE.to_owned()),
            ),
            (BLOCKED_BY_FIELD.to_owned(), Value::Null),
        ],
    )?;
    advance_metabolism(&mut expected, 50)?;
    expected.refresh_hash();
    Ok(expected)
}

#[test]
fn absent_model_preserves_exact_legacy_bytes_even_when_plant_inventory_is_unavailable() -> Result<()>
{
    for shared in [false, true] {
        for job in ["BREW_DRINK", "PREPARE_MEAL", "COOK_MEAL"] {
            let mut source = source(0, false, shared)?;
            fact_mut(&mut source, LEDGER, PLANTS)?.presence = Some(FactPresence::Omitted(
                "legacy food production did not require plant evidence".to_owned(),
            ));
            source.refresh_hash();
            let requested = action("legacy", job, 1, Vec::new());
            assert_eq!(provisions_model(&source)?, None);
            assert_eq!(provisions_input_cost(&source, &requested)?, None);
            assert!(provisions_report(&source)?.is_none());
            assert!(provisions_completion_predicates(&source, &requested, "legacy")?.is_empty());
            let expected = legacy_reference(&source, job, "legacy")?;
            let mut actual = source;
            apply(&mut actual, &requested, "legacy")?;
            advance(&mut actual, 50)?;
            assert_eq!(
                actual.canonical_bytes(),
                expected.canonical_bytes(),
                "legacy {shared}/{job}"
            );
            let record = entity(&actual, created_entity_id("legacy", 0))?;
            assert!(
                !record
                    .fields
                    .contains_key(WORK_ORDER_PRODUCTION_MODEL_FIELD)
            );
            assert!(!record.fields.contains_key(RECEIPT_CONSUMED));
        }
    }
    Ok(())
}
