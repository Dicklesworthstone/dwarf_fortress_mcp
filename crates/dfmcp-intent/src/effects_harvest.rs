//! Finite ecological acquisition in the explicit laboratory model.
//!
//! Completion atomically moves one source unit into the ordinary stock ledger.
//! The source and every original task retain conservation history; observations
//! and cancellation cannot synthesize supply or refund completed harvests.

use super::*;
use crate::HarvestResource;

pub const LABORATORY_HARVEST_MODEL_FIELD: &str = "laboratory_harvest_model";
pub const LABORATORY_HARVEST_MODEL: &str = "finite-harvest/1";
pub const HARVEST_SOURCE_MODEL_FIELD: &str = "harvest_source_model";
pub const HARVEST_TASK_MODEL_FIELD: &str = "harvest_task_model";
pub const HARVEST_RESOURCE_FIELD: &str = "harvest_resource";
pub const HARVEST_INITIAL_FIELD: &str = "harvest_initial";
pub const HARVEST_REMAINING_FIELD: &str = "harvest_remaining";
pub const HARVEST_YIELDED_FIELD: &str = "harvest_yielded";
pub const HARVEST_SOURCE_FIELD: &str = "harvest_source";
pub const HARVEST_SOURCE_GENERATION_FIELD: &str = "harvest_source_generation";
pub const HARVEST_LEDGER_FIELD: &str = "harvest_ledger";
pub const HARVEST_LEDGER_GENERATION_FIELD: &str = "harvest_ledger_generation";
pub const HARVEST_TASK_KIND: &str = "harvest_task";
pub const HARVEST_TICKS_PER_UNIT: u64 = 50;
pub const MAX_HARVEST_AMOUNT: u32 = 100;
pub const MAX_HARVEST_SOURCES: usize = 1_024;
pub const MAX_HARVEST_TASKS: usize = 4_096;

const FIELDS: [&str; 11] = [
    HARVEST_SOURCE_MODEL_FIELD,
    HARVEST_TASK_MODEL_FIELD,
    HARVEST_RESOURCE_FIELD,
    HARVEST_INITIAL_FIELD,
    HARVEST_REMAINING_FIELD,
    HARVEST_YIELDED_FIELD,
    HARVEST_SOURCE_FIELD,
    HARVEST_SOURCE_GENERATION_FIELD,
    HARVEST_LEDGER_FIELD,
    HARVEST_LEDGER_GENERATION_FIELD,
    LABORATORY_HARVEST_MODEL_FIELD,
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarvestSource {
    pub source: EntityId,
    pub generation: u64,
    pub resource: HarvestResource,
    pub initial: u64,
    pub remaining: u64,
    pub harvested: u64,
    pub active_demand: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarvestTask {
    pub task: EntityId,
    pub source: EntityId,
    pub source_generation: u64,
    pub resource: HarvestResource,
    pub ledger: EntityId,
    pub ledger_generation: u64,
    pub amount_total: u64,
    pub amount_remaining: u64,
    pub harvested: u64,
    pub work_ticks: u64,
    pub status: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarvestReport {
    pub model: &'static str,
    pub ledger: EntityId,
    pub ledger_generation: u64,
    pub sources: Vec<HarvestSource>,
    pub tasks: Vec<HarvestTask>,
}

fn has_fields(fields: &BTreeMap<String, Fact>) -> bool {
    FIELDS.iter().any(|field| fields.contains_key(*field))
}

fn required(record: &EntityRecord, field: &str, value: &Value, tick: GameTick) -> Result<()> {
    if field_value(record, field, tick) == Some(value) {
        Ok(())
    } else {
        Err(precondition(format!(
            "finite harvest requires qualified {field} on entity {}",
            record.id.get()
        )))
    }
}

fn resource(record: &EntityRecord, tick: GameTick) -> Result<HarvestResource> {
    match field_text(record, HARVEST_RESOURCE_FIELD, tick) {
        Some("WOOD") => Ok(HarvestResource::Wood),
        Some("PLANT") => Ok(HarvestResource::Plant),
        _ => Err(precondition(
            "finite harvest has an unknown or unsupported resource",
        )),
    }
}

fn reference(record: &EntityRecord, field: &str, tick: GameTick) -> Result<EntityId> {
    match field_value(record, field, tick) {
        Some(Value::Entity(id)) if *id != EntityId::NIL => Ok(*id),
        _ => Err(precondition(
            "finite harvest requires an eligible original entity reference",
        )),
    }
}

pub(super) fn source_record(record: &EntityRecord, tick: GameTick) -> Result<HarvestSource> {
    if record.kind != EntityKind::Plant || record.id == EntityId::NIL || record.generation == 0 {
        return Err(precondition(
            "finite harvest source requires a positive original Plant identity",
        ));
    }
    required(
        record,
        HARVEST_SOURCE_MODEL_FIELD,
        &Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
        tick,
    )?;
    let initial = field_u64(record, HARVEST_INITIAL_FIELD, tick)?;
    let remaining = field_u64(record, HARVEST_REMAINING_FIELD, tick)?;
    let harvested = field_u64(record, HARVEST_YIELDED_FIELD, tick)?;
    if remaining.checked_add(harvested) != Some(initial) {
        return Err(precondition(
            "finite harvest source quantities violate conservation",
        ));
    }
    Ok(HarvestSource {
        source: record.id,
        generation: u64::from(record.generation),
        resource: resource(record, tick)?,
        initial,
        remaining,
        harvested,
        active_demand: 0,
    })
}

fn task_record(record: &EntityRecord, tick: GameTick) -> Result<HarvestTask> {
    if record.kind != EntityKind::Other(HARVEST_TASK_KIND.to_owned()) || record.generation != 1 {
        return Err(precondition(
            "finite harvest task requires its original kind and generation",
        ));
    }
    required(
        record,
        HARVEST_TASK_MODEL_FIELD,
        &Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
        tick,
    )?;
    required(
        record,
        LABORATORY_WORKFORCE_TASK_MODEL_FIELD,
        &Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        tick,
    )?;
    let amount_total = field_u64(record, "amount_total", tick)?;
    let amount_remaining = field_u64(record, AMOUNT_REMAINING_FIELD, tick)?;
    let harvested = field_u64(record, HARVEST_YIELDED_FIELD, tick)?;
    let work_ticks = field_u64(record, "work_ticks", tick)?;
    let status = field_text(record, STATUS_FIELD, tick)
        .ok_or_else(|| precondition("finite harvest task has unknown lifecycle"))?;
    if !(1..=u64::from(MAX_HARVEST_AMOUNT)).contains(&amount_total)
        || amount_remaining.checked_add(harvested) != Some(amount_total)
        || work_ticks >= HARVEST_TICKS_PER_UNIT
        || !matches!(status, STATUS_ACTIVE | STATUS_COMPLETE | STATUS_CANCELLED)
        || (status == STATUS_COMPLETE && (amount_remaining != 0 || work_ticks != 0))
        || (status == STATUS_ACTIVE && amount_remaining == 0)
    {
        return Err(precondition(
            "finite harvest task has malformed progress or yield history",
        ));
    }
    Ok(HarvestTask {
        task: record.id,
        source: reference(record, HARVEST_SOURCE_FIELD, tick)?,
        source_generation: field_u64(record, HARVEST_SOURCE_GENERATION_FIELD, tick)?,
        resource: resource(record, tick)?,
        ledger: reference(record, HARVEST_LEDGER_FIELD, tick)?,
        ledger_generation: field_u64(record, HARVEST_LEDGER_GENERATION_FIELD, tick)?,
        amount_total,
        amount_remaining,
        harvested,
        work_ticks,
        status: status.to_owned(),
    })
}

fn bounded() -> DfmcpError {
    advance_budget_error("finite harvest exceeds its bounded source, task or arithmetic domain")
}

/// Complete source and task census. Unknown fields and lost model/custody never
/// become absent resources, free replenishment or completed work.
pub(super) fn capture(
    snapshot: &WorldSnapshot,
    budget: &mut EffectAdvanceBudget,
) -> Result<Option<HarvestReport>> {
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    let declared = snapshot
        .graph
        .entities
        .get(&settings)
        .is_some_and(|record| record.fields.contains_key(LABORATORY_HARVEST_MODEL_FIELD));
    // Absence detection uses fixed field lookups, not an unbounded field walk.
    budget.charge(
        snapshot.graph.entities.len() as u64 * 14 + snapshot.graph.edges.len() as u64 * 11 + 1,
    )?;
    let touched = snapshot.graph.entities.values().any(|record| {
        has_fields(&record.fields) || record.kind == EntityKind::Other(HARVEST_TASK_KIND.to_owned())
    });
    if snapshot
        .graph
        .edges
        .values()
        .any(|record| has_fields(&record.fields))
    {
        return Err(precondition(
            "finite harvest model and custody belong on canonical entity records",
        ));
    }
    if !declared {
        return if touched {
            Err(precondition(
                "finite harvest model is missing while source or task history remains",
            ))
        } else {
            Ok(None)
        };
    }
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(bounded());
    }
    let settings_record = entity(snapshot, settings)?;
    if settings_record.id != settings
        || settings_record.generation != 1
        || settings_record.kind != EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned())
    {
        return Err(precondition(
            "finite harvest requires the original canonical fortress settings",
        ));
    }
    required(
        settings_record,
        LABORATORY_HARVEST_MODEL_FIELD,
        &Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
        snapshot.tick,
    )?;
    if laboratory_workforce_model(snapshot)?.is_none() {
        return Err(precondition(
            "finite harvest requires the qualified shared-workforce/1 model",
        ));
    }
    budget.charge(snapshot.graph.entities.len() as u64 * 32 + 1)?;
    let mut sources = BTreeMap::new();
    let mut tasks = Vec::new();
    let mut ledger = None;
    for (id, record) in &snapshot.graph.entities {
        if record.fields.contains_key(LABORATORY_HARVEST_MODEL_FIELD) && *id != settings {
            return Err(precondition("finite harvest model selector is misplaced"));
        }
        if record.kind == EntityKind::Other(STOCK_LEDGER_KIND.to_owned()) {
            if record.id != *id
                || *id == EntityId::NIL
                || record.generation == 0
                || ledger.is_some()
            {
                return Err(precondition(
                    "finite harvest requires one unambiguous original stock ledger",
                ));
            }
            field_u64(record, STOCK_WOOD_FIELD, snapshot.tick)?;
            field_u64(record, STOCK_PLANT_FIELD, snapshot.tick)?;
            ledger = Some((*id, u64::from(record.generation)));
        }
        if *id == settings {
            if FIELDS[..FIELDS.len() - 1]
                .iter()
                .any(|field| record.fields.contains_key(*field))
            {
                return Err(precondition(
                    "harvest source or task fields are misplaced on settings",
                ));
            }
            continue;
        }
        if !has_fields(&record.fields)
            && record.kind != EntityKind::Other(HARVEST_TASK_KIND.to_owned())
        {
            continue;
        }
        if record.id != *id {
            return Err(precondition(
                "finite harvest record key and identity disagree",
            ));
        }
        if record.kind == EntityKind::Plant {
            if record.fields.contains_key(HARVEST_TASK_MODEL_FIELD)
                || record.fields.contains_key(HARVEST_SOURCE_FIELD)
                || record.fields.contains_key(HARVEST_LEDGER_FIELD)
                || record.fields.contains_key(HARVEST_SOURCE_GENERATION_FIELD)
                || record.fields.contains_key(HARVEST_LEDGER_GENERATION_FIELD)
                || sources.len() == MAX_HARVEST_SOURCES
            {
                return Err(precondition(
                    "finite harvest source has misplaced custody or exceeds its bound",
                ));
            }
            sources.insert(*id, source_record(record, snapshot.tick)?);
        } else if record.kind == EntityKind::Other(HARVEST_TASK_KIND.to_owned()) {
            if record.fields.contains_key(HARVEST_SOURCE_MODEL_FIELD)
                || record.fields.contains_key(HARVEST_INITIAL_FIELD)
                || record.fields.contains_key(HARVEST_REMAINING_FIELD)
                || tasks.len() == MAX_HARVEST_TASKS
            {
                return Err(precondition(
                    "finite harvest task has misplaced source fields or exceeds its bound",
                ));
            }
            tasks.push(task_record(record, snapshot.tick)?);
        } else {
            return Err(precondition(
                "finite harvest fields occur on an unsupported entity kind",
            ));
        }
    }
    let (ledger, ledger_generation) =
        ledger.ok_or_else(|| precondition("finite harvest stock ledger is missing"))?;
    let mut yields = BTreeMap::<EntityId, u64>::new();
    budget.charge(tasks.len() as u64 * 24 + sources.len() as u64 * 4 + 1)?;
    for task in &tasks {
        let source = sources
            .get_mut(&task.source)
            .ok_or_else(|| precondition("original harvest source is missing"))?;
        if source.generation != task.source_generation
            || source.resource != task.resource
            || task.ledger != ledger
            || task.ledger_generation != ledger_generation
        {
            return Err(precondition(
                "finite harvest source or ledger no longer matches original custody",
            ));
        }
        let yielded = yields.entry(task.source).or_default();
        *yielded = yielded.checked_add(task.harvested).ok_or_else(bounded)?;
        if task.status == STATUS_ACTIVE {
            source.active_demand = source
                .active_demand
                .checked_add(task.amount_remaining)
                .ok_or_else(bounded)?;
        }
    }
    for source in sources.values() {
        if source.harvested != yields.get(&source.source).copied().unwrap_or(0) {
            return Err(precondition(
                "source yield disagrees with complete original harvest task history",
            ));
        }
    }
    Ok(Some(HarvestReport {
        model: LABORATORY_HARVEST_MODEL,
        ledger,
        ledger_generation,
        sources: sources.into_values().collect(),
        tasks,
    }))
}

pub fn harvest_report(snapshot: &WorldSnapshot) -> Result<Option<HarvestReport>> {
    capture(
        snapshot,
        &mut EffectAdvanceBudget::new(EffectAdvanceLimits::default(), 0)?,
    )
}

pub fn harvest_model(snapshot: &WorldSnapshot) -> Result<Option<&'static str>> {
    Ok(harvest_report(snapshot)?.map(|_| LABORATORY_HARVEST_MODEL))
}

pub(super) fn validate_action(action: &Action) -> Result<()> {
    if let Action::HarvestResource {
        source,
        source_generation,
        amount,
        ..
    } = action
    {
        if *source == EntityId::NIL
            || *source_generation == 0
            || u32::try_from(*source_generation).is_err()
            || !(1..=MAX_HARVEST_AMOUNT).contains(amount)
        {
            return Err(precondition(
                "harvest requires a nonzero source, original generation and amount in 1..=100",
            ));
        }
        Ok(())
    } else {
        Err(precondition(
            "harvest custody requires its original harvest action",
        ))
    }
}

pub(super) fn dispatch(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<bool> {
    validate_action(action)?;
    let Action::HarvestResource {
        source,
        source_generation,
        resource,
        amount,
    } = action
    else {
        return Err(precondition("expected harvest action"));
    };
    let report = harvest_report(snapshot)?
        .ok_or_else(|| precondition("harvesting requires explicit finite-harvest/1 semantics"))?;
    let original = report
        .sources
        .iter()
        .find(|row| row.source == *source)
        .ok_or_else(|| precondition("selected harvest source is not observed"))?;
    if original.generation != *source_generation
        || original.resource != *resource
        || original
            .active_demand
            .checked_add(u64::from(*amount))
            .is_none_or(|needed| needed > original.remaining)
    {
        return Err(precondition(
            "harvest source identity or uncommitted finite supply does not satisfy the request",
        ));
    }
    create_entity(
        snapshot,
        created_entity_id(key, 0),
        EntityKind::Other(HARVEST_TASK_KIND.to_owned()),
        format!("finite {} harvest", resource.as_str()),
        vec![
            (
                HARVEST_TASK_MODEL_FIELD.to_owned(),
                Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            (
                HARVEST_RESOURCE_FIELD.to_owned(),
                Value::Text(resource.as_str().to_owned()),
            ),
            (HARVEST_SOURCE_FIELD.to_owned(), Value::Entity(*source)),
            (
                HARVEST_SOURCE_GENERATION_FIELD.to_owned(),
                Value::U64(*source_generation),
            ),
            (
                HARVEST_LEDGER_FIELD.to_owned(),
                Value::Entity(report.ledger),
            ),
            (
                HARVEST_LEDGER_GENERATION_FIELD.to_owned(),
                Value::U64(report.ledger_generation),
            ),
            ("amount_total".to_owned(), Value::U64(u64::from(*amount))),
            (
                AMOUNT_REMAINING_FIELD.to_owned(),
                Value::U64(u64::from(*amount)),
            ),
            (HARVEST_YIELDED_FIELD.to_owned(), Value::U64(0)),
            ("work_ticks".to_owned(), Value::U64(0)),
            (
                STATUS_FIELD.to_owned(),
                Value::Text(STATUS_ACTIVE.to_owned()),
            ),
        ],
    )?;
    Ok(true)
}

pub fn validate_harvest_identity(
    snapshot: &WorldSnapshot,
    task_id: EntityId,
    action: &Action,
) -> Result<()> {
    let Action::HarvestResource {
        source,
        source_generation,
        resource,
        amount,
    } = action
    else {
        return if snapshot.graph.entities.get(&task_id).is_some_and(|record| {
            record.kind == EntityKind::Other(HARVEST_TASK_KIND.to_owned())
                || record.fields.contains_key(HARVEST_TASK_MODEL_FIELD)
        }) {
            Err(precondition(
                "a non-harvest action cannot own retained harvest work",
            ))
        } else {
            Ok(())
        };
    };
    validate_action(action)?;
    let report = harvest_report(snapshot)?
        .ok_or_else(|| precondition("original harvest model is missing"))?;
    let task = report
        .tasks
        .iter()
        .find(|task| task.task == task_id)
        .ok_or_else(|| precondition("original harvest task is not observed"))?;
    if task.source != *source
        || task.source_generation != *source_generation
        || task.resource != *resource
        || task.amount_total != u64::from(*amount)
    {
        return Err(precondition(
            "harvest task does not match its immutable original action",
        ));
    }
    Ok(())
}

pub fn validate_harvest_completion(
    snapshot: &WorldSnapshot,
    task_id: EntityId,
    action: &Action,
) -> Result<()> {
    validate_harvest_identity(snapshot, task_id, action)?;
    if matches!(action, Action::HarvestResource { .. }) {
        let task = task_record(entity(snapshot, task_id)?, snapshot.tick)?;
        if task.status != STATUS_COMPLETE
            || task.amount_remaining != 0
            || task.harvested != task.amount_total
        {
            return Err(precondition(
                "original finite harvest has not completed its full conserved yield",
            ));
        }
    }
    Ok(())
}

pub fn harvest_completion_predicates(action: &Action, key: &str) -> Vec<Predicate> {
    let Action::HarvestResource {
        source,
        source_generation,
        resource,
        amount,
    } = action
    else {
        return Vec::new();
    };
    if validate_action(action).is_err() {
        return vec![Predicate::False];
    }
    let id = created_entity_id(key, 0);
    vec![
        Predicate::EntityKind {
            entity_id: id,
            kind: EntityKind::Other(HARVEST_TASK_KIND.to_owned()),
        },
        field_eq(
            id,
            HARVEST_TASK_MODEL_FIELD,
            Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
        ),
        field_eq(id, HARVEST_SOURCE_FIELD, Value::Entity(*source)),
        field_eq(
            id,
            HARVEST_SOURCE_GENERATION_FIELD,
            Value::U64(*source_generation),
        ),
        field_eq(
            id,
            HARVEST_RESOURCE_FIELD,
            Value::Text(resource.as_str().to_owned()),
        ),
        field_eq(id, "amount_total", Value::U64(u64::from(*amount))),
        field_eq(id, HARVEST_YIELDED_FIELD, Value::U64(u64::from(*amount))),
        field_eq(id, AMOUNT_REMAINING_FIELD, Value::U64(0)),
        field_eq(id, STATUS_FIELD, Value::Text(STATUS_COMPLETE.to_owned())),
    ]
}

pub(super) fn cancel(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    key: &str,
) -> Result<Option<bool>> {
    let id = created_entity_id(key, 0);
    if !matches!(action, Action::HarvestResource { .. }) {
        validate_harvest_identity(snapshot, id, action)?;
        return Ok(None);
    }
    if !snapshot.graph.entities.contains_key(&id) {
        return Ok(Some(false));
    }
    validate_harvest_identity(snapshot, id, action)?;
    if field_text(entity(snapshot, id)?, STATUS_FIELD, snapshot.tick) != Some(STATUS_ACTIVE) {
        return Ok(Some(false));
    }
    Ok(Some(write_fields(
        snapshot,
        id,
        vec![(
            STATUS_FIELD.to_owned(),
            Value::Text(STATUS_CANCELLED.to_owned()),
        )],
    )?))
}

/// Used only inside a timeline whose full census was already validated. The
/// timeline cannot replace model, source or ledger identities between events.
pub(super) fn blocker(snapshot: &WorldSnapshot, id: EntityId) -> Result<Option<String>> {
    let task = task_record(entity(snapshot, id)?, snapshot.tick)?;
    let source = source_record(entity(snapshot, task.source)?, snapshot.tick)?;
    let ledger = entity(snapshot, task.ledger)?;
    if source.generation != task.source_generation
        || source.resource != task.resource
        || u64::from(ledger.generation) != task.ledger_generation
        || ledger.kind != EntityKind::Other(STOCK_LEDGER_KIND.to_owned())
    {
        return Err(precondition(
            "harvest source or ledger changed during reference progress",
        ));
    }
    field_u64(ledger, stock_field(task.resource), snapshot.tick)?;
    Ok((source.remaining == 0)
        .then(|| format!("finite {} source is depleted", task.resource.as_str())))
}

pub(super) const fn stock_field(resource: HarvestResource) -> &'static str {
    match resource {
        HarvestResource::Wood => STOCK_WOOD_FIELD,
        HarvestResource::Plant => STOCK_PLANT_FIELD,
    }
}

pub(super) fn advance(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    elapsed: u64,
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    budget.charge(64)?;
    let task = task_record(entity(snapshot, id)?, snapshot.tick)?;
    if task.status != STATUS_ACTIVE {
        return Ok(false);
    }
    let progress = task.work_ticks.checked_add(elapsed).ok_or_else(bounded)?;
    if progress > HARVEST_TICKS_PER_UNIT {
        return Err(bounded());
    }
    if let Some(reason) = blocker(snapshot, id)? {
        // This task was eligible at the interval's start. An earlier task may
        // consume the last source unit at this same boundary. Retain earned
        // partial service, but never the completion tick that lacked supply.
        // Later depleted intervals do not select the task and earn no service.
        return write_fields(
            snapshot,
            id,
            vec![
                (
                    "work_ticks".to_owned(),
                    Value::U64(progress.min(HARVEST_TICKS_PER_UNIT - 1)),
                ),
                (BLOCKED_BY_FIELD.to_owned(), Value::Text(reason)),
            ],
        );
    }
    if progress < HARVEST_TICKS_PER_UNIT {
        return write_fields(
            snapshot,
            id,
            vec![("work_ticks".to_owned(), Value::U64(progress))],
        );
    }
    let source = source_record(entity(snapshot, task.source)?, snapshot.tick)?;
    let stock = field_u64(
        entity(snapshot, task.ledger)?,
        stock_field(task.resource),
        snapshot.tick,
    )?;
    let source_remaining = source.remaining.checked_sub(1).ok_or_else(bounded)?;
    let source_harvested = source.harvested.checked_add(1).ok_or_else(bounded)?;
    let remaining = task.amount_remaining.checked_sub(1).ok_or_else(bounded)?;
    let harvested = task.harvested.checked_add(1).ok_or_else(bounded)?;
    let stock = stock.checked_add(1).ok_or_else(bounded)?;
    // The caller publishes one transaction shadow, so source, stock and original
    // history become visible together or none are published on any refusal.
    let mut changed = write_fields(
        snapshot,
        task.source,
        vec![
            (
                HARVEST_REMAINING_FIELD.to_owned(),
                Value::U64(source_remaining),
            ),
            (
                HARVEST_YIELDED_FIELD.to_owned(),
                Value::U64(source_harvested),
            ),
        ],
    )?;
    changed |= write_fields(
        snapshot,
        task.ledger,
        vec![(stock_field(task.resource).to_owned(), Value::U64(stock))],
    )?;
    changed |= write_fields(
        snapshot,
        id,
        vec![
            (AMOUNT_REMAINING_FIELD.to_owned(), Value::U64(remaining)),
            (HARVEST_YIELDED_FIELD.to_owned(), Value::U64(harvested)),
            ("work_ticks".to_owned(), Value::U64(0)),
            (
                STATUS_FIELD.to_owned(),
                Value::Text(
                    if remaining == 0 {
                        STATUS_COMPLETE
                    } else {
                        STATUS_ACTIVE
                    }
                    .to_owned(),
                ),
            ),
            (BLOCKED_BY_FIELD.to_owned(), Value::Null),
        ],
    )?;
    Ok(changed)
}

#[cfg(test)]
#[path = "effects_harvest_tests.rs"]
mod tests;
