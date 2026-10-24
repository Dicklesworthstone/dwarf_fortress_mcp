//! Opt-in, source-bound service capacity shared by registered laboratory work.
//!
//! The model is part of canonical state. It never grants a legacy task a new
//! history, and a stamped task prevents a world from silently losing its model.
//! Assignments are interval decisions, not reservations of native game workers.

use std::cmp::Reverse;
use std::collections::VecDeque;

use super::*;

/// Canonical fortress-settings selector for the opt-in workforce semantics.
pub const LABORATORY_WORKFORCE_MODEL_FIELD: &str = "laboratory_workforce_model";
pub const LABORATORY_WORKFORCE_MODEL: &str = "shared-workforce/1";
/// Persisted on each registered activity created under the selected model.
pub const LABORATORY_WORKFORCE_TASK_MODEL_FIELD: &str = "laboratory_workforce_task_model";

const FAMILIES: [(Option<&str>, &str); 5] = [
    (None, "MINING"),
    (None, "CONSTRUCTION"),
    (Some("workshop:Still"), "BREW"),
    (Some("workshop:Kitchen"), "COOK"),
    (Some("workshop:Carpenters"), "CARPENTRY"),
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaboratoryWorkforceAssignment {
    pub work: EntityId,
    pub worker: EntityId,
    pub labor: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaboratoryWorkforceBlocker {
    pub work: EntityId,
    pub labor: &'static str,
    pub reason: String,
}

/// A pure selection at this snapshot's source clock, covering only the five
/// registered activity families. Unregistered work-order recipes remain abstract.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaboratoryWorkforceReport {
    pub model: &'static str,
    pub assignments: Vec<LaboratoryWorkforceAssignment>,
    pub blocked: Vec<LaboratoryWorkforceBlocker>,
    pub active_activities: usize,
    pub eligible_workers: usize,
    pub remaining_service_ticks: u64,
    pub work_units: u64,
}

fn order_family(job: &str) -> Option<usize> {
    match job {
        "BREW_DRINK" => Some(2),
        "PREPARE_MEAL" | "COOK_MEAL" => Some(3),
        token if carpentry::recipe(token).is_some() => Some(4),
        _ => None,
    }
}

pub(super) fn registered_action(action: &Action) -> bool {
    match action {
        Action::DesignateDig { .. } | Action::Build { .. } | Action::PlaceFurniture { .. } => true,
        Action::CreateWorkOrder { job_token, .. } => order_family(job_token).is_some(),
        _ => false,
    }
}

fn registered_record(record: &EntityRecord, tick: GameTick) -> bool {
    match &record.kind {
        EntityKind::Building => true,
        EntityKind::Other(kind) if kind == DIG_DESIGNATION_KIND => true,
        EntityKind::WorkOrder => field_text(record, "job_token", tick)
            .and_then(order_family)
            .is_some(),
        _ => false,
    }
}

/// Read the explicit model without guessing from labels, labor fields or old
/// progress. A malformed declaration, misplaced marker, or surviving task stamp
/// without its global model is a refusal, never an invitation to legacy fallback.
pub fn laboratory_workforce_model(snapshot: &WorldSnapshot) -> Result<Option<&'static str>> {
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    let mut declared = false;
    let mut stamped = false;
    if snapshot.graph.edges.values().any(|record| {
        record.fields.contains_key(LABORATORY_WORKFORCE_MODEL_FIELD)
            || record
                .fields
                .contains_key(LABORATORY_WORKFORCE_TASK_MODEL_FIELD)
    }) {
        return Err(precondition(
            "laboratory workforce model declarations and task stamps require canonical entity records",
        ));
    }
    for (id, record) in &snapshot.graph.entities {
        if record.fields.contains_key(LABORATORY_WORKFORCE_MODEL_FIELD) {
            if *id != settings
                || record.id != *id
                || record.generation != 1
                || record.kind != EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned())
                || field_text(record, LABORATORY_WORKFORCE_MODEL_FIELD, snapshot.tick)
                    != Some(LABORATORY_WORKFORCE_MODEL)
            {
                return Err(precondition(
                    "laboratory workforce model requires one qualified supported declaration on the canonical fortress settings identity",
                ));
            }
            declared = true;
        }
        if record
            .fields
            .contains_key(LABORATORY_WORKFORCE_TASK_MODEL_FIELD)
        {
            if record.id != *id
                || record.generation != 1
                || !registered_record(record, snapshot.tick)
                || field_text(record, LABORATORY_WORKFORCE_TASK_MODEL_FIELD, snapshot.tick)
                    != Some(LABORATORY_WORKFORCE_MODEL)
            {
                return Err(precondition(
                    "laboratory workforce task has an unsupported, unavailable or misplaced model stamp",
                ));
            }
            stamped = true;
        }
    }
    if !declared {
        return if stamped {
            Err(precondition(
                "laboratory workforce model is missing while stamped task history remains; downgrade is refused",
            ))
        } else {
            Ok(None)
        };
    }
    if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
        return Err(advance_budget_error(
            "shared workforce exceeds its explicit canonical entity bound",
        ));
    }
    // A model can be introduced beside completed fixture workshops. It cannot
    // reinterpret an already active legacy activity as previously staffed work.
    require_progress_selectors(snapshot)?;
    for record in snapshot.graph.entities.values() {
        let active = if record.kind == EntityKind::Building {
            matches!(
                field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick),
                Some(STAGE_PLANNED | STAGE_UNDER_CONSTRUCTION)
            )
        } else {
            field_text(record, STATUS_FIELD, snapshot.tick) == Some(STATUS_ACTIVE)
        };
        if active
            && registered_record(record, snapshot.tick)
            && !record
                .fields
                .contains_key(LABORATORY_WORKFORCE_TASK_MODEL_FIELD)
        {
            return Err(precondition(format!(
                "active registered activity {} lacks its laboratory workforce task model stamp",
                record.id.get()
            )));
        }
    }
    Ok(Some(LABORATORY_WORKFORCE_MODEL))
}

pub(super) fn stamp(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<bool> {
    write_fields(
        snapshot,
        id,
        vec![(
            LABORATORY_WORKFORCE_TASK_MODEL_FIELD.to_owned(),
            Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
        )],
    )
}

pub(super) fn clear_stamped_blocker(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<bool> {
    if snapshot.graph.entities.get(&id).is_some_and(|record| {
        record
            .fields
            .contains_key(LABORATORY_WORKFORCE_TASK_MODEL_FIELD)
    }) {
        write_production_blocker(snapshot, id, None)
    } else {
        Ok(false)
    }
}

#[derive(Clone, Copy)]
enum Activity {
    Dig,
    Building,
    Order(usize),
}

struct Candidate {
    id: EntityId,
    activity: Activity,
    family: usize,
    partial: u64,
}

struct Capacity {
    workers: Vec<EntityId>,
    eligible: [Vec<usize>; FAMILIES.len()],
    workshops: [usize; FAMILIES.len()],
}

impl Capacity {
    fn capture(snapshot: &WorldSnapshot, budget: &mut EffectAdvanceBudget) -> Result<Self> {
        if snapshot.graph.entities.len() > MAX_EFFECT_ADVANCE_PRODUCTION_ENTITIES {
            return Err(advance_budget_error(
                "shared workforce exceeds its explicit canonical entity bound",
            ));
        }
        budget
            .charge(snapshot.graph.entities.len() as u64 * (2 * FAMILIES.len() as u64 + 4) + 1)?;
        let mut capacity = Self {
            workers: Vec::new(),
            eligible: std::array::from_fn(|_| Vec::new()),
            workshops: [0; FAMILIES.len()],
        };
        for (id, record) in &snapshot.graph.entities {
            if record.id != *id {
                return Err(precondition(
                    "shared workforce source has a mismatched entity identity",
                ));
            }
            if record.kind == EntityKind::Unit
                && field_value(record, "alive", snapshot.tick) == Some(&Value::Bool(true))
            {
                let eligible = FAMILIES.map(|(_, labor)| {
                    field_value(
                        record,
                        &format!("{LABOR_FIELD_PREFIX}{labor}"),
                        snapshot.tick,
                    ) == Some(&Value::Bool(true))
                });
                if eligible.iter().any(|value| *value) {
                    for (family, enabled) in eligible.into_iter().enumerate() {
                        if enabled {
                            capacity.eligible[family].push(capacity.workers.len());
                        }
                    }
                    capacity.workers.push(*id);
                }
            } else if record.kind == EntityKind::Building
                && field_text(record, CONSTRUCTION_STAGE_FIELD, snapshot.tick)
                    == Some(STAGE_COMPLETE)
            {
                for (family, (workshop, _)) in FAMILIES.iter().enumerate() {
                    if workshop.is_some()
                        && field_text(record, "building_kind", snapshot.tick) == *workshop
                    {
                        capacity.workshops[family] += 1;
                    }
                }
            }
        }
        Ok(capacity)
    }
}

pub(super) struct Selection {
    pub(super) ready: Vec<usize>,
    pub(super) buildings: BTreeSet<EntityId>,
    pub(super) digs: BTreeSet<EntityId>,
    pub(super) blockers: BTreeMap<EntityId, Option<String>>,
    assignments: Vec<LaboratoryWorkforceAssignment>,
    blocked: Vec<LaboratoryWorkforceBlocker>,
    active_activities: usize,
    eligible_workers: usize,
}

// Iterative augmentation retains earlier accepted activities while moving
// flexible workers out of specialist-only slots. The five families have uniform
// eligibility within each family and disjoint workshop kinds, so a family cap
// cannot sacrifice a larger feasible matching from a later activity.
fn augment(
    root: usize,
    candidates: &[Candidate],
    capacity: &Capacity,
    owner: &mut [Option<usize>],
    assigned: &mut [Option<usize>],
    budget: &mut EffectAdvanceBudget,
) -> Result<bool> {
    budget.charge(capacity.workers.len() as u64 * 2 + candidates.len() as u64 + 1)?;
    let mut seen = vec![false; capacity.workers.len()];
    let mut parent = vec![None; capacity.workers.len()];
    let mut queue = VecDeque::from([root]);
    while let Some(candidate) = queue.pop_front() {
        budget.charge(1)?;
        for worker in &capacity.eligible[candidates[candidate].family] {
            budget.charge(1)?;
            if seen[*worker] {
                continue;
            }
            seen[*worker] = true;
            parent[*worker] = Some(candidate);
            if let Some(previous) = owner[*worker] {
                queue.push_back(previous);
                continue;
            }
            let mut available = *worker;
            loop {
                budget.charge(1)?;
                let next = parent[available].ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "shared workforce lost an augmenting-path predecessor",
                    )
                })?;
                let previous = assigned[next].replace(available);
                owner[available] = Some(next);
                if let Some(previous) = previous {
                    available = previous;
                } else {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

pub(super) fn select(
    snapshot: &WorldSnapshot,
    timeline: &EffectTimeline,
    budget: &mut EffectAdvanceBudget,
) -> Result<Selection> {
    let capacity = Capacity::capture(snapshot, budget)?;
    budget.charge(
        (timeline.orders.len() + timeline.buildings.len() + timeline.digs.len()) as u64 * 5 + 1,
    )?;
    let mut candidates = Vec::new();
    let mut selection = Selection {
        ready: Vec::new(),
        buildings: BTreeSet::new(),
        digs: BTreeSet::new(),
        blockers: BTreeMap::new(),
        assignments: Vec::new(),
        blocked: Vec::new(),
        active_activities: 0,
        eligible_workers: capacity.workers.len(),
    };
    for (id, dig) in &timeline.digs {
        if !dig.pending.is_empty() {
            candidates.push(Candidate {
                id: *id,
                activity: Activity::Dig,
                family: 0,
                partial: field_u64(entity(snapshot, *id)?, "work_ticks", snapshot.tick)?,
            });
            selection.active_activities += 1;
        }
    }
    for id in &timeline.buildings {
        if active_building(snapshot, *id)? {
            candidates.push(Candidate {
                id: *id,
                activity: Activity::Building,
                family: 1,
                partial: field_u64(entity(snapshot, *id)?, "progress_ticks", snapshot.tick)?,
            });
            selection.active_activities += 1;
        }
    }
    for (index, order) in timeline.orders.iter().enumerate() {
        if !active_order(snapshot, order.id)? {
            continue;
        }
        let family = order_family(&order.job);
        if family.is_some() {
            selection.active_activities += 1;
        }
        match timeline.gate(snapshot, order, budget)? {
            ProductionGate::Ready { .. } => {
                if let Some(family) = family {
                    candidates.push(Candidate {
                        id: order.id,
                        activity: Activity::Order(index),
                        family,
                        partial: field_u64(
                            entity(snapshot, order.id)?,
                            "work_ticks",
                            snapshot.tick,
                        )?,
                    });
                } else {
                    selection.ready.push(index);
                    selection.blockers.insert(order.id, None);
                }
            }
            ProductionGate::Blocked(reason) => {
                selection.blockers.insert(order.id, Some(reason.clone()));
                if let Some(family) = family {
                    selection.blocked.push(LaboratoryWorkforceBlocker {
                        work: order.id,
                        labor: FAMILIES[family].1,
                        reason,
                    });
                }
            }
        }
    }
    let depth = usize::BITS - candidates.len().leading_zeros();
    budget
        .charge(candidates.len() as u64 * (u64::from(depth) + 2) + capacity.workers.len() as u64)?;
    candidates.sort_by_key(|candidate| (Reverse(candidate.partial), candidate.id));
    let mut owner = vec![None; capacity.workers.len()];
    let mut assigned = vec![None; candidates.len()];
    let mut used_workshops = [0; FAMILIES.len()];
    for (index, candidate) in candidates.iter().enumerate() {
        budget.charge(1)?;
        if (FAMILIES[candidate.family].0.is_none()
            || used_workshops[candidate.family] < capacity.workshops[candidate.family])
            && augment(
                index,
                &candidates,
                &capacity,
                &mut owner,
                &mut assigned,
                budget,
            )?
        {
            used_workshops[candidate.family] += 1;
        }
    }
    for (index, candidate) in candidates.iter().enumerate() {
        budget.charge(1)?;
        let (workshop, labor) = FAMILIES[candidate.family];
        if let Some(worker) = assigned[index] {
            match candidate.activity {
                Activity::Dig => {
                    selection.digs.insert(candidate.id);
                }
                Activity::Building => {
                    selection.buildings.insert(candidate.id);
                }
                Activity::Order(index) => selection.ready.push(index),
            }
            selection.blockers.insert(candidate.id, None);
            selection.assignments.push(LaboratoryWorkforceAssignment {
                work: candidate.id,
                worker: capacity.workers[worker],
                labor,
            });
        } else {
            let reason = if capacity.eligible[candidate.family].is_empty() {
                format!("living unit with qualified {labor} labor enabled is not established")
            } else if let Some(workshop) = workshop {
                if capacity.workshops[candidate.family] == 0 {
                    format!("completed {workshop} is not established")
                } else if used_workshops[candidate.family] == capacity.workshops[candidate.family] {
                    format!(
                        "workforce capacity busy: every established {workshop} is assigned to another activity"
                    )
                } else {
                    format!(
                        "workforce capacity busy: eligible {labor} workers are assigned to other activities"
                    )
                }
            } else {
                format!(
                    "workforce capacity busy: eligible {labor} workers are assigned to other activities"
                )
            };
            selection
                .blockers
                .insert(candidate.id, Some(reason.clone()));
            selection.blocked.push(LaboratoryWorkforceBlocker {
                work: candidate.id,
                labor,
                reason,
            });
        }
    }
    // Dispatch and public reports are in canonical work identity order, not
    // the incidental traversal order used to discover augmenting paths.
    let width = selection.ready.len() + selection.assignments.len() + selection.blocked.len();
    let depth = usize::BITS - width.leading_zeros();
    budget.charge(width as u64 * (u64::from(depth) + 1))?;
    selection.ready.sort_unstable();
    selection
        .assignments
        .sort_by_key(|assignment| assignment.work);
    selection.blocked.sort_by_key(|blocker| blocker.work);
    Ok(selection)
}

fn remaining_ticks(
    snapshot: &WorldSnapshot,
    timeline: &EffectTimeline,
    budget: &mut EffectAdvanceBudget,
) -> Result<u64> {
    budget.charge(
        (timeline.orders.len() + timeline.buildings.len() + timeline.digs.len()) as u64 * 5 + 1,
    )?;
    let mut total = 0u64;
    let mut add = |remaining: u64| -> Result<()> {
        total = total
            .checked_add(remaining)
            .ok_or_else(|| advance_budget_error("remaining shared workforce service overflowed"))?;
        Ok(())
    };
    for (id, dig) in &timeline.digs {
        let record = entity(snapshot, *id)?;
        let tiles = validate_region(dig.area)?;
        if field_u64(record, "tiles_total", snapshot.tick)? != tiles
            || field_u64(record, TILES_REMAINING_FIELD, snapshot.tick)? > tiles
        {
            return Err(precondition(
                "staffed excavation has noncanonical tile counters",
            ));
        }
        if !dig.pending.is_empty() {
            let partial = field_u64(record, "work_ticks", snapshot.tick)?;
            add((dig.pending.len() as u64)
                .checked_mul(DIG_TICKS_PER_TILE)
                .and_then(|ticks| ticks.checked_sub(partial))
                .ok_or_else(|| advance_budget_error("remaining excavation service overflowed"))?)?;
        }
    }
    for id in &timeline.buildings {
        if active_building(snapshot, *id)? {
            let record = entity(snapshot, *id)?;
            add(field_u64(record, "required_ticks", snapshot.tick)?
                .checked_sub(field_u64(record, "progress_ticks", snapshot.tick)?)
                .ok_or_else(|| {
                    precondition("staffed construction has noncanonical progress counters")
                })?)?;
        }
    }
    for order in &timeline.orders {
        if order_family(&order.job).is_some() && active_order(snapshot, order.id)? {
            let record = entity(snapshot, order.id)?;
            let remaining = field_u64(record, AMOUNT_REMAINING_FIELD, snapshot.tick)?;
            if remaining > field_u64(record, "amount_total", snapshot.tick)? {
                return Err(precondition(
                    "staffed production has noncanonical amount counters",
                ));
            }
            let partial = field_u64(record, "work_ticks", snapshot.tick)?;
            add(remaining
                .checked_mul(WORK_ORDER_TICKS_PER_UNIT)
                .and_then(|ticks| ticks.checked_sub(partial))
                .ok_or_else(|| advance_budget_error("remaining production service overflowed"))?)?;
        }
    }
    Ok(total)
}

/// Conservatively sum remaining registered service, including blocked work.
/// Overlapping designations may overestimate, but cannot create an optimistic
/// deadline. Checked arithmetic and qualified counters precede the result.
pub fn laboratory_workforce_remaining_ticks(snapshot: &WorldSnapshot) -> Result<Option<u64>> {
    if laboratory_workforce_model(snapshot)?.is_none() {
        return Ok(None);
    }
    let mut budget = EffectAdvanceBudget::new(EffectAdvanceLimits::default(), 0)?;
    budget.charge(
        snapshot.graph.entities.len() as u64 * 4 + snapshot.graph.edges.len() as u64 * 2 + 1,
    )?;
    let timeline = EffectTimeline::new(snapshot, 0, &mut budget)?;
    Ok(Some(remaining_ticks(snapshot, &timeline, &mut budget)?))
}

/// Explain the same source-clock matching used by physical progress, without
/// mutating facts or treating forecast allocations as durable reservations.
pub fn laboratory_workforce_report(
    snapshot: &WorldSnapshot,
) -> Result<Option<LaboratoryWorkforceReport>> {
    if laboratory_workforce_model(snapshot)?.is_none() {
        return Ok(None);
    }
    let mut budget = EffectAdvanceBudget::new(EffectAdvanceLimits::default(), 0)?;
    budget.charge(
        snapshot.graph.entities.len() as u64 * 4 + snapshot.graph.edges.len() as u64 * 2 + 1,
    )?;
    let timeline = EffectTimeline::new(snapshot, 0, &mut budget)?;
    let remaining_service_ticks = remaining_ticks(snapshot, &timeline, &mut budget)?;
    let selection = select(snapshot, &timeline, &mut budget)?;
    Ok(Some(LaboratoryWorkforceReport {
        model: LABORATORY_WORKFORCE_MODEL,
        assignments: selection.assignments,
        blocked: selection.blocked,
        active_activities: selection.active_activities,
        eligible_workers: selection.eligible_workers,
        remaining_service_ticks,
        work_units: budget.work_units,
    }))
}
