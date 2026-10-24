//! Explicit finite inputs for the laboratory's brewing and cooking recipes.
//!
//! One completed batch consumes one qualified PLANT and produces the existing
//! five units of drink or food. Inputs remain available until completion;
//! partial work and cancellation neither consume nor refund an unfinished
//! batch. Retained per-order consumption is history, not a reservation.

use super::*;

pub const LABORATORY_PROVISIONS_MODEL_FIELD: &str = "laboratory_provisions_model";
pub const LABORATORY_PROVISIONS_MODEL: &str = "plant-provisions/1";
pub const STOCK_PLANT_FIELD: &str = "stock.material.PLANT";
pub const PROVISIONS_LEDGER_FIELD: &str = "provisions_ledger";
pub const PROVISIONS_LEDGER_GENERATION_FIELD: &str = "provisions_ledger_generation";
pub const PROVISIONS_CONSUMED_FIELD: &str = "provisions_consumed_plants";

const RECEIPT_FIELDS: [&str; 3] = [
    PROVISIONS_LEDGER_FIELD,
    PROVISIONS_LEDGER_GENERATION_FIELD,
    PROVISIONS_CONSUMED_FIELD,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisionsOrder {
    pub order: EntityId,
    pub job: String,
    pub amount_total: u64,
    pub amount_remaining: u64,
    pub consumed_plants: u64,
    pub work_ticks: u64,
    pub status: &'static str,
}

/// Current qualified supply and the complete retained consumption census.
/// Remaining demand describes active physical work; it reserves no input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisionsReport {
    pub model: &'static str,
    pub ledger: EntityId,
    pub ledger_generation: u64,
    pub available_plants: u64,
    pub consumed_plants: u64,
    pub remaining_demand_plants: u64,
    pub orders: Vec<ProvisionsOrder>,
}

/// A timeline owns this validated immutable inventory identity. Its internal
/// transitions cannot add or replace ledgers, model declarations or recipes.
#[derive(Clone, Copy)]
pub(super) struct Model {
    ledger: EntityId,
    ledger_generation: u64,
}

pub(super) fn recipe(job: &str) -> Option<&'static str> {
    match job {
        "BREW_DRINK" => Some(STOCK_DRINK_FIELD),
        "PREPARE_MEAL" | "COOK_MEAL" => Some(STOCK_FOOD_FIELD),
        _ => None,
    }
}

fn food_action(action: &Action) -> bool {
    matches!(action, Action::CreateWorkOrder { job_token, .. } if recipe(job_token).is_some())
}

fn has_provisions_fields(fields: &BTreeMap<String, Fact>) -> bool {
    RECEIPT_FIELDS
        .iter()
        .any(|field| fields.contains_key(*field))
        || fields
            .get(WORK_ORDER_PRODUCTION_MODEL_FIELD)
            .is_some_and(|fact| {
                matches!(&fact.value, Value::Text(model) if model.starts_with("plant-provisions/"))
                    || fields.get("job_token").is_some_and(
                        |job| matches!(&job.value, Value::Text(job) if recipe(job).is_some()),
                    )
            })
}

fn require_value(
    record: &EntityRecord,
    field: &str,
    expected: &Value,
    tick: GameTick,
) -> Result<()> {
    if field_value(record, field, tick) != Some(expected) {
        return Err(precondition(format!(
            "provisions history requires qualified {field} on entity {}",
            record.id.get(),
        )));
    }
    Ok(())
}

fn validate_action(action: &Action) -> Result<()> {
    let Action::CreateWorkOrder {
        name,
        job_token,
        amount,
        conditions,
    } = action
    else {
        return Err(precondition(
            "provisions history requires a food work order",
        ));
    };
    if recipe(job_token).is_none() || *amount == 0 {
        return Err(precondition(
            "plant-provisions/1 requires a positive finite BREW_DRINK, PREPARE_MEAL or COOK_MEAL order",
        ));
    }
    validate_condition_text(name)?;
    work_order_conditions_value(conditions)?;
    Ok(())
}

fn ledger<'a>(snapshot: &'a WorldSnapshot, model: &Model) -> Result<&'a EntityRecord> {
    let record = entity(snapshot, model.ledger)?;
    if record.id != model.ledger
        || record.kind != EntityKind::Other(STOCK_LEDGER_KIND.to_owned())
        || record.generation != 1
        || u64::from(record.generation) != model.ledger_generation
    {
        return Err(precondition(
            "provisions input requires its original stock-ledger identity and generation",
        ));
    }
    Ok(record)
}

fn order_state(
    snapshot: &WorldSnapshot,
    record: &EntityRecord,
    model: &Model,
) -> Result<ProvisionsOrder> {
    if record.kind != EntityKind::WorkOrder || record.generation != 1 {
        return Err(precondition(
            "provisions history requires its original work-order generation",
        ));
    }
    let job = field_text(record, "job_token", snapshot.tick)
        .filter(|job| recipe(job).is_some())
        .ok_or_else(|| precondition("provisions history has no qualified closed food recipe"))?;
    for (field, value) in [
        (
            WORK_ORDER_PRODUCTION_MODEL_FIELD,
            Value::Text(LABORATORY_PROVISIONS_MODEL.to_owned()),
        ),
        (PROVISIONS_LEDGER_FIELD, Value::Entity(model.ledger)),
        (
            PROVISIONS_LEDGER_GENERATION_FIELD,
            Value::U64(model.ledger_generation),
        ),
        (
            LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        ),
    ] {
        require_value(record, field, &value, snapshot.tick)?;
    }
    let total = field_u64(record, "amount_total", snapshot.tick)?;
    let remaining = field_u64(record, AMOUNT_REMAINING_FIELD, snapshot.tick)?;
    let consumed = field_u64(record, PROVISIONS_CONSUMED_FIELD, snapshot.tick)?;
    let work = field_u64(record, "work_ticks", snapshot.tick)?;
    if total == 0
        || total > u64::from(u32::MAX)
        || remaining > total
        || consumed != total - remaining
        || work >= WORK_ORDER_TICKS_PER_UNIT
    {
        return Err(precondition(
            "provisions history has inconsistent original, remaining, consumed or partial-work counters",
        ));
    }
    let status = match field_text(record, STATUS_FIELD, snapshot.tick) {
        Some(STATUS_ACTIVE) if remaining > 0 => STATUS_ACTIVE,
        Some(STATUS_COMPLETE) if remaining == 0 && work == 0 => STATUS_COMPLETE,
        Some(STATUS_CANCELLED) if remaining > 0 => STATUS_CANCELLED,
        _ => {
            return Err(precondition(
                "provisions history has a noncanonical progress lifecycle",
            ));
        }
    };
    Ok(ProvisionsOrder {
        order: record.id,
        job: job.to_owned(),
        amount_total: total,
        amount_remaining: remaining,
        consumed_plants: consumed,
        work_ticks: work,
        status,
    })
}

fn capture(
    snapshot: &WorldSnapshot,
    budget: &mut EffectAdvanceBudget,
) -> Result<Option<ProvisionsReport>> {
    let count = snapshot.graph.entities.len() as u64;
    budget.charge(
        count
            .saturating_mul(8)
            .saturating_add((snapshot.graph.edges.len() as u64).saturating_mul(8))
            .saturating_add(1),
    )?;
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    let mut declared = false;
    let mut stamped = false;
    for record in snapshot.graph.edges.values() {
        if record
            .fields
            .contains_key(LABORATORY_PROVISIONS_MODEL_FIELD)
            || has_provisions_fields(&record.fields)
        {
            return Err(precondition(
                "provisions model declarations and consumption history require canonical entity records",
            ));
        }
    }
    for (id, record) in &snapshot.graph.entities {
        if record
            .fields
            .contains_key(LABORATORY_PROVISIONS_MODEL_FIELD)
        {
            if *id != settings
                || record.id != *id
                || record.generation != 1
                || record.kind != EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned())
                || field_text(record, LABORATORY_PROVISIONS_MODEL_FIELD, snapshot.tick)
                    != Some(LABORATORY_PROVISIONS_MODEL)
            {
                return Err(precondition(
                    "provisions model requires a qualified supported declaration on the canonical fortress settings identity",
                ));
            }
            declared = true;
        }
        stamped |= has_provisions_fields(&record.fields);
    }
    if !declared {
        return if stamped {
            Err(precondition(
                "provisions model is missing while consumption history remains; downgrade is refused",
            ))
        } else {
            Ok(None)
        };
    }
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "provisions census exceeds its canonical entity bound",
        ));
    }
    budget.charge(count.saturating_mul(16).saturating_add(1))?;
    if laboratory_workforce_model(snapshot)? != Some(LABORATORY_WORKFORCE_MODEL) {
        return Err(precondition(
            "plant-provisions/1 requires the qualified shared-workforce/1 model",
        ));
    }
    let mut ledgers = snapshot.graph.entities.iter().filter(
        |(_, record)| matches!(&record.kind, EntityKind::Other(kind) if kind == STOCK_LEDGER_KIND),
    );
    let (ledger_id, inventory) = ledgers
        .next()
        .ok_or_else(|| precondition("provisions inventory requires one qualified stock ledger"))?;
    if ledgers.next().is_some() || inventory.id != *ledger_id || inventory.generation != 1 {
        return Err(precondition(
            "provisions inventory requires one unambiguous original stock-ledger identity",
        ));
    }
    let model = Model {
        ledger: *ledger_id,
        ledger_generation: u64::from(inventory.generation),
    };
    let available_plants = field_u64(inventory, STOCK_PLANT_FIELD, snapshot.tick)?;
    field_u64(inventory, STOCK_DRINK_FIELD, snapshot.tick)?;
    field_u64(inventory, STOCK_FOOD_FIELD, snapshot.tick)?;
    let mut report = ProvisionsReport {
        model: LABORATORY_PROVISIONS_MODEL,
        ledger: model.ledger,
        ledger_generation: model.ledger_generation,
        available_plants,
        consumed_plants: 0,
        remaining_demand_plants: 0,
        orders: Vec::new(),
    };
    for (id, record) in &snapshot.graph.entities {
        if record.id != *id {
            return Err(precondition(
                "provisions source has a mismatched entity identity",
            ));
        }
        if has_provisions_fields(&record.fields) {
            // Bound full name/condition validation before decoding or copying
            // any retained list. The timeline subsequently checks fixed-size
            // custody and counter fields at each completion boundary.
            budget.charge(640)?;
            let name = field_text(record, WORK_ORDER_NAME_FIELD, snapshot.tick)
                .ok_or_else(|| precondition("provisions history has no qualified order name"))?;
            validate_condition_text(name)?;
            let conditions = field_value(record, WORK_ORDER_CONDITIONS_FIELD, snapshot.tick)
                .ok_or_else(|| {
                    precondition("provisions history has no qualified condition list")
                })?;
            let Value::List(values) = conditions else {
                return Err(precondition(
                    "provisions history conditions are not a canonical list",
                ));
            };
            if values.len() > MAX_WORK_ORDER_CONDITIONS {
                return Err(advance_budget_error(
                    "provisions conditions exceed their list bound",
                ));
            }
            budget.charge((values.len() as u64).saturating_mul(1024))?;
            decode_conditions(conditions)?;
            let row = order_state(snapshot, record, &model)?;
            report.consumed_plants = report
                .consumed_plants
                .checked_add(row.consumed_plants)
                .ok_or_else(|| advance_budget_error("provisions consumption total overflow"))?;
            if row.status == STATUS_ACTIVE {
                report.remaining_demand_plants = report
                    .remaining_demand_plants
                    .checked_add(row.amount_remaining)
                    .ok_or_else(|| advance_budget_error("provisions pending demand overflow"))?;
            }
            report.orders.push(row);
        } else if record.kind.as_str() == EntityKind::WorkOrder.as_str() {
            match field_text(record, STATUS_FIELD, snapshot.tick) {
                Some(STATUS_COMPLETE | STATUS_CANCELLED) => {}
                Some(STATUS_ACTIVE) => {
                    let job = field_text(record, "job_token", snapshot.tick).ok_or_else(|| {
                        precondition(
                            "provisions census requires qualified active work-order recipes",
                        )
                    })?;
                    if recipe(job).is_some() {
                        return Err(precondition(
                            "active food production lacks its explicit provisions history; legacy work cannot acquire consumed inputs",
                        ));
                    }
                }
                _ => {
                    return Err(precondition(
                        "provisions census requires qualified work-order lifecycle selectors",
                    ));
                }
            }
        }
    }
    Ok(Some(report))
}

pub(super) fn validate_model(
    snapshot: &WorldSnapshot,
    budget: &mut EffectAdvanceBudget,
) -> Result<Option<Model>> {
    Ok(capture(snapshot, budget)?.map(|report| Model {
        ledger: report.ledger,
        ledger_generation: report.ledger_generation,
    }))
}

pub fn provisions_model(snapshot: &WorldSnapshot) -> Result<Option<&'static str>> {
    Ok(provisions_report(snapshot)?.map(|report| report.model))
}

pub fn provisions_report(snapshot: &WorldSnapshot) -> Result<Option<ProvisionsReport>> {
    let mut budget = EffectAdvanceBudget::new(EffectAdvanceLimits::default(), 0)?;
    capture(snapshot, &mut budget)
}

/// Total PLANT demand for a new finite food order. This is a calculation, not a
/// reservation. Other actions and model-free worlds keep their old semantics.
pub fn provisions_input_cost(snapshot: &WorldSnapshot, action: &Action) -> Result<Option<u64>> {
    if !food_action(action) || provisions_model(snapshot)?.is_none() {
        return Ok(None);
    }
    validate_action(action)?;
    match action {
        Action::CreateWorkOrder { amount, .. } => Ok(Some(u64::from(*amount))),
        _ => Err(precondition("provisions cost requires a food work order")),
    }
}

fn validate_identity(
    snapshot: &WorldSnapshot,
    order: EntityId,
    action: &Action,
    report: &ProvisionsReport,
) -> Result<()> {
    validate_action(action)?;
    let record = entity(snapshot, order)?;
    order_state(
        snapshot,
        record,
        &Model {
            ledger: report.ledger,
            ledger_generation: report.ledger_generation,
        },
    )?;
    let Action::CreateWorkOrder {
        name,
        job_token,
        amount,
        conditions,
    } = action
    else {
        return Err(precondition(
            "provisions identity requires a food work order",
        ));
    };
    for (field, value) in [
        (WORK_ORDER_NAME_FIELD, Value::Text(name.clone())),
        ("job_token", Value::Text(job_token.clone())),
        ("amount_total", Value::U64(u64::from(*amount))),
        (
            WORK_ORDER_CONDITIONS_FIELD,
            work_order_conditions_value(conditions)?,
        ),
    ] {
        require_value(record, field, &value, snapshot.tick)?;
    }
    Ok(())
}

/// Validate original work identity and retained input history independently of
/// whether the action's goal has completed. This also governs cancellation.
pub fn validate_provisions_identity(
    snapshot: &WorldSnapshot,
    order: EntityId,
    action: &Action,
) -> Result<()> {
    if !food_action(action) {
        return Ok(());
    }
    let Some(report) = provisions_report(snapshot)? else {
        return Ok(());
    };
    validate_identity(snapshot, order, action, &report)
}

pub fn validate_provisions_completion(
    snapshot: &WorldSnapshot,
    order: EntityId,
    action: &Action,
) -> Result<()> {
    if !food_action(action) {
        return Ok(());
    }
    let Some(report) = provisions_report(snapshot)? else {
        return Ok(());
    };
    validate_identity(snapshot, order, action, &report)?;
    require_value(
        entity(snapshot, order)?,
        STATUS_FIELD,
        &Value::Text(STATUS_COMPLETE.to_owned()),
        snapshot.tick,
    )
}

/// Mandatory new-model proof; existing default postconditions and canonical
/// action bytes remain unchanged. Publication also validates the census and
/// original generations, which cannot be expressed by field predicates alone.
pub fn provisions_completion_predicates(
    snapshot: &WorldSnapshot,
    action: &Action,
    key: &str,
) -> Result<Vec<Predicate>> {
    if !food_action(action) {
        return Ok(Vec::new());
    }
    let Some(report) = provisions_report(snapshot)? else {
        return Ok(Vec::new());
    };
    validate_action(action)?;
    let Action::CreateWorkOrder {
        name,
        job_token,
        amount,
        conditions,
    } = action
    else {
        return Err(precondition("provisions proof requires a food work order"));
    };
    let order = created_entity_id(key, 0);
    let mut result = vec![Predicate::EntityKind {
        entity_id: order,
        kind: EntityKind::WorkOrder,
    }];
    for (field, value) in [
        (WORK_ORDER_NAME_FIELD, Value::Text(name.clone())),
        ("job_token", Value::Text(job_token.clone())),
        ("amount_total", Value::U64(u64::from(*amount))),
        (AMOUNT_REMAINING_FIELD, Value::U64(0)),
        ("work_ticks", Value::U64(0)),
        (STATUS_FIELD, Value::Text(STATUS_COMPLETE.to_owned())),
        (
            WORK_ORDER_CONDITIONS_FIELD,
            work_order_conditions_value(conditions)?,
        ),
        (
            WORK_ORDER_PRODUCTION_MODEL_FIELD,
            Value::Text(LABORATORY_PROVISIONS_MODEL.to_owned()),
        ),
        (PROVISIONS_LEDGER_FIELD, Value::Entity(report.ledger)),
        (
            PROVISIONS_LEDGER_GENERATION_FIELD,
            Value::U64(report.ledger_generation),
        ),
        (PROVISIONS_CONSUMED_FIELD, Value::U64(u64::from(*amount))),
        (
            LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        ),
    ] {
        result.push(field_eq(order, field, value));
    }
    Ok(result)
}

pub(super) fn dispatch_fields(snapshot: &WorldSnapshot, job: &str) -> Result<Vec<(String, Value)>> {
    if recipe(job).is_none() {
        return Ok(Vec::new());
    }
    let Some(report) = provisions_report(snapshot)? else {
        return Ok(Vec::new());
    };
    if snapshot.graph.entities.len() >= MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "provisions order would exceed the canonical entity bound",
        ));
    }
    Ok(vec![
        (
            WORK_ORDER_PRODUCTION_MODEL_FIELD.to_owned(),
            Value::Text(LABORATORY_PROVISIONS_MODEL.to_owned()),
        ),
        (
            PROVISIONS_LEDGER_FIELD.to_owned(),
            Value::Entity(report.ledger),
        ),
        (
            PROVISIONS_LEDGER_GENERATION_FIELD.to_owned(),
            Value::U64(report.ledger_generation),
        ),
        (PROVISIONS_CONSUMED_FIELD.to_owned(), Value::U64(0)),
    ])
}

pub(super) fn blocker(
    snapshot: &WorldSnapshot,
    order: &TimelineOrder,
    model: Option<&Model>,
) -> Result<Option<String>> {
    if recipe(&order.job).is_none() {
        return Ok(None);
    }
    let Some(model) = model else {
        return Ok(None);
    };
    let record = entity(snapshot, order.id)?;
    let row = order_state(snapshot, record, model)?;
    if row.job != order.job || row.status != STATUS_ACTIVE {
        return Err(precondition(
            "provisions timeline lost its original active recipe",
        ));
    }
    let held = field_u64(ledger(snapshot, model)?, STOCK_PLANT_FIELD, snapshot.tick)?;
    if held == 0 {
        return Ok(Some(
            "provisions require one available PLANT per completed brewing or cooking batch"
                .to_owned(),
        ));
    }
    Ok(None)
}

/// Debit input and advance retained consumption. The caller publishes output
/// and remaining-work changes in the same transaction shadow immediately after
/// this helper; no intermediate history is externally visible.
pub(super) fn complete(
    snapshot: &mut WorldSnapshot,
    order: &TimelineOrder,
    model: Option<&Model>,
    remaining_after: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    let Some(output) = recipe(&order.job) else {
        return Ok(false);
    };
    let Some(model) = model else {
        return Ok(false);
    };
    budget.charge(64)?;
    let row = order_state(snapshot, entity(snapshot, order.id)?, model)?;
    if row.job != order.job
        || row.status != STATUS_ACTIVE
        || row.amount_remaining.checked_sub(1) != Some(remaining_after)
    {
        return Err(precondition(
            "provisions completion lost its original finite batch",
        ));
    }
    let inventory = ledger(snapshot, model)?;
    let after = field_u64(inventory, STOCK_PLANT_FIELD, snapshot.tick)?
        .checked_sub(1)
        .ok_or_else(|| precondition("provisions completion has no available PLANT input"))?;
    field_u64(inventory, output, snapshot.tick)?
        .checked_add(5)
        .ok_or_else(|| advance_budget_error("provisions output would overflow its stock count"))?;
    let consumed = row
        .consumed_plants
        .checked_add(1)
        .ok_or_else(|| advance_budget_error("provisions consumption history overflow"))?;
    let mut changed = write_fields(
        snapshot,
        model.ledger,
        vec![(STOCK_PLANT_FIELD.to_owned(), Value::U64(after))],
    )?;
    changed |= write_fields(
        snapshot,
        order.id,
        vec![(PROVISIONS_CONSUMED_FIELD.to_owned(), Value::U64(consumed))],
    )?;
    Ok(changed)
}

pub(super) fn cancel(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    key: &str,
) -> Result<Option<bool>> {
    let order = created_entity_id(key, 0);
    let owns_history = snapshot
        .graph
        .entities
        .get(&order)
        .is_some_and(|record| has_provisions_fields(&record.fields));
    // A changed action must not bypass ownership by leaving the food recipe
    // family and falling through to generic work-order cancellation.
    if !food_action(action) && !owns_history {
        return Ok(None);
    }
    let Some(report) = provisions_report(snapshot)? else {
        return Ok(None);
    };
    if !snapshot.graph.entities.contains_key(&order) {
        return Ok(Some(false));
    }
    validate_identity(snapshot, order, action, &report)?;
    if field_text(entity(snapshot, order)?, STATUS_FIELD, snapshot.tick) != Some(STATUS_ACTIVE) {
        return Ok(Some(false));
    }
    Ok(Some(write_fields(
        snapshot,
        order,
        vec![
            (
                STATUS_FIELD.to_owned(),
                Value::Text(STATUS_CANCELLED.to_owned()),
            ),
            (BLOCKED_BY_FIELD.to_owned(), Value::Null),
        ],
    )?))
}
