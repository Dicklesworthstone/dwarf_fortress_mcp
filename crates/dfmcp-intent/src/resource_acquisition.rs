//! Bounded stock objectives over the explicit finite laboratory resource model.
//!
//! This compiler proposes ordinary harvest and optional labor actions from one
//! complete reference snapshot. It never reserves sources, workers or inventory,
//! and never treats unfinished harvest as stock. The observing shell retains the
//! original request, seals it through `StaticPlanner::prepare_laboratory`, and
//! requires reviewed replay after any source-anchor change.

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, GameTick, Result, StateAnchor};
use dfmcp_world::{
    CompareOp, EntityKind, EntityRecord, Predicate, PredicateEvidence, Value, WorldSnapshot,
    laboratory_fact_value,
};

use crate::{
    Action, HarvestResource, RequestedAction, effects,
    workforce_planning::LaboratoryWorkforcePlanning,
};

pub const MAX_RESOURCE_ACQUISITION_TARGETS: usize = 2;
pub const MAX_RESOURCE_ACQUISITION_ACTIONS: usize = 16;
pub const MAX_RESOURCE_ACQUISITION_ENTITIES: usize = 65_536;

/// The original minimum is a stock goal, not an amount of newly produced work.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ResourceStockTarget {
    pub resource: HarvestResource,
    pub minimum: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAcquisitionRequirement {
    pub resource: HarvestResource,
    pub minimum: u64,
    pub observed: u64,
    pub deficit: u64,
    /// Complete finite source supply after existing demand, not inventory.
    pub available_source_units: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAcquisitionAllocation {
    pub resource: HarvestResource,
    pub source: EntityId,
    pub source_generation: u64,
    pub amount: u32,
    /// Index into `ResourceAcquisitionPlan::requested_actions`.
    pub action_index: u32,
}

/// A source-qualified capacity choice, not a worker reservation. The generation
/// is retained for inspection; exact source replay and normal action witnesses
/// supply the dispatch fence. The worker matcher may choose another eligible
/// worker while executing the physical harvest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAcquisitionStaffing {
    pub resource: HarvestResource,
    pub worker: EntityId,
    pub generation: u32,
    pub previously_enabled: bool,
    /// The explicit enable, if any, in the complete proposed action list.
    pub action_index: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceAcquisitionPlan {
    pub source_anchor: StateAnchor,
    pub ledger: EntityId,
    pub ledger_generation: u64,
    pub requested_actions: Vec<RequestedAction>,
    pub terminal_condition: Predicate,
    pub requirements: Vec<ResourceAcquisitionRequirement>,
    pub allocations: Vec<ResourceAcquisitionAllocation>,
    pub staffing: Vec<ResourceAcquisitionStaffing>,
    pub queued_activities: usize,
    pub queued_service_ticks: u64,
    pub requested_service_ticks: u64,
    /// The same common floor used by the ordinary shared-workforce planner.
    /// `None` means the complete original stock goal is already satisfied.
    pub deadline_tick: Option<GameTick>,
}

impl ResourceAcquisitionPlan {
    /// Compile only from a complete laboratory-adapter-owned snapshot. Matching
    /// producer labels in imported or native data do not grant that ownership.
    ///
    /// One or two distinct positive minima are accepted. Targets are ordered
    /// WOOD then PLANT. Sources are selected by largest useful tranche (at most
    /// 100 units), then canonical source ID. Existing active harvest of a
    /// deficient resource refuses; its output must be observed before deciding
    /// how much more to acquire. A no-op produces no actions or staffing edits.
    pub fn compile(
        snapshot: &WorldSnapshot,
        targets: &[ResourceStockTarget],
        assign_labor: bool,
    ) -> Result<Self> {
        if targets.is_empty()
            || targets.len() > MAX_RESOURCE_ACQUISITION_TARGETS
            || targets.iter().any(|target| target.minimum == 0)
        {
            return Err(invalid(
                "resource acquisition requires one or two distinct positive stock minima",
            ));
        }
        let mut targets = targets.to_vec();
        targets.sort_by_key(|target| target.resource);
        if targets
            .windows(2)
            .any(|pair| pair[0].resource == pair[1].resource)
        {
            return Err(invalid(
                "resource acquisition cannot repeat a WOOD or PLANT target",
            ));
        }
        if snapshot.graph.entities.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
            || snapshot.graph.edges.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
            || snapshot.graph.chunks.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
            || snapshot.graph.events.len() > MAX_RESOURCE_ACQUISITION_ENTITIES
        {
            return Err(bounded());
        }
        let evidence = PredicateEvidence::laboratory(snapshot)?;
        let snapshot = evidence.snapshot();
        let report = effects::harvest_report(snapshot)?.ok_or_else(|| {
            refused("resource acquisition requires explicit finite-harvest/1 laboratory sources")
        })?;
        let ledger = snapshot.graph.entities.get(&report.ledger).ok_or_else(|| {
            refused("resource acquisition requires its complete original stock ledger")
        })?;
        let mut goal = vec![
            Predicate::EntityKind {
                entity_id: report.ledger,
                kind: EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            },
            model_guard(snapshot),
        ];
        let mut requirements = Vec::with_capacity(targets.len());
        for target in &targets {
            let field = stock_field(target.resource);
            let observed = match value(ledger, field, snapshot.tick) {
                Some(Value::U64(quantity)) => *quantity,
                _ => {
                    return Err(refused(format!(
                        "resource acquisition requires qualified current {field}",
                    )));
                }
            };
            let deficit = target.minimum.saturating_sub(observed);
            let mut available_source_units = 0u64;
            let mut active_harvest = false;
            for source in &report.sources {
                if source.resource != target.resource {
                    continue;
                }
                active_harvest |= source.active_demand != 0;
                available_source_units = available_source_units
                    .checked_add(source.remaining.saturating_sub(source.active_demand))
                    .ok_or_else(bounded)?;
            }
            if deficit != 0 && active_harvest {
                return Err(refused(format!(
                    "original {} harvest remains active; observe its completed yield or explicitly stop its original work before sizing another stock acquisition",
                    target.resource.as_str(),
                )));
            }
            if deficit > available_source_units {
                return Err(refused(format!(
                    "{} stock target requires {deficit} new units but complete qualified sources provide only {available_source_units} after existing demand",
                    target.resource.as_str(),
                )));
            }
            goal.push(field_guard(
                report.ledger,
                field,
                CompareOp::Ge,
                Value::U64(target.minimum),
            ));
            requirements.push(ResourceAcquisitionRequirement {
                resource: target.resource,
                minimum: target.minimum,
                observed,
                deficit,
                available_source_units,
            });
        }
        let terminal_condition = Predicate::All(goal).normalized();
        terminal_condition.validate_shape()?;
        let mut plan = Self {
            source_anchor: snapshot.anchor(),
            ledger: report.ledger,
            ledger_generation: report.ledger_generation,
            requested_actions: Vec::new(),
            terminal_condition,
            requirements,
            allocations: Vec::new(),
            staffing: Vec::new(),
            queued_activities: 0,
            queued_service_ticks: 0,
            requested_service_ticks: 0,
            deadline_tick: None,
        };
        if plan.requirements.iter().all(|row| row.deficit == 0) {
            // No scheduling or worker requirement can create work for an
            // already observed stock goal. The complete harvest census above
            // still prevents unknown custody from becoming a no-op success.
            return Ok(plan);
        }

        // Capture the entire registered queue before presenting a horizon.
        // Unknown lifecycle/job/service facts refuse in this shared model.
        let workforce = LaboratoryWorkforcePlanning::capture(snapshot)?.ok_or_else(|| {
            refused("resource acquisition requires qualified shared-workforce/1 service history")
        })?;
        plan.queued_activities = workforce.queued_activities;
        plan.queued_service_ticks = workforce.queued_service_ticks;
        refuse_competing_consumers(snapshot, &plan.requirements)?;
        for requirement in &plan.requirements {
            if requirement.deficit == 0 {
                continue;
            }
            let (worker, previously_enabled) =
                select_worker(snapshot, requirement.resource, assign_labor)?;
            let action_index = if previously_enabled {
                None
            } else {
                let action_index = next_action_index(&plan.requested_actions)?;
                plan.requested_actions.push(RequestedAction {
                    action: Action::SetLabor {
                        units: vec![worker.id],
                        labor: requirement.resource.labor().to_owned(),
                        enabled: true,
                    },
                    preconditions: worker_guards(worker.id, requirement.resource, Some(false)),
                    postconditions: worker_guards(worker.id, requirement.resource, Some(true)),
                    compensation: Some(Action::SetLabor {
                        units: vec![worker.id],
                        labor: requirement.resource.labor().to_owned(),
                        enabled: false,
                    }),
                    obligation: None,
                    depends_on: Vec::new(),
                });
                Some(action_index)
            };
            plan.staffing.push(ResourceAcquisitionStaffing {
                resource: requirement.resource,
                worker: worker.id,
                generation: worker.generation,
                previously_enabled,
                action_index,
            });
        }

        for requirement in &plan.requirements {
            if requirement.deficit == 0 {
                continue;
            }
            let staffing = plan
                .staffing
                .iter()
                .find(|row| row.resource == requirement.resource)
                .ok_or_else(|| refused("resource acquisition has no qualified staffing choice"))?;
            let mut supplies: Vec<_> = report
                .sources
                .iter()
                .filter(|source| source.resource == requirement.resource)
                .map(|source| {
                    (
                        source,
                        source.remaining.saturating_sub(source.active_demand),
                    )
                })
                .collect();
            let mut remaining = requirement.deficit;
            while remaining != 0 {
                let action_index = next_action_index(&plan.requested_actions)?;
                let mut selected: Option<(usize, u64, EntityId)> = None;
                for (index, (source, available)) in supplies.iter().enumerate() {
                    let amount = (*available)
                        .min(remaining)
                        .min(u64::from(effects::MAX_HARVEST_AMOUNT));
                    if amount != 0
                        && selected.as_ref().is_none_or(|(_, best, id)| {
                            amount > *best || (amount == *best && source.source < *id)
                        })
                    {
                        selected = Some((index, amount, source.source));
                    }
                }
                let (index, amount, _) = selected.ok_or_else(|| {
                    refused("qualified finite resource supply cannot satisfy the complete target")
                })?;
                let (source, available) = &mut supplies[index];
                let amount = u32::try_from(amount).map_err(|_| bounded())?;
                *available -= u64::from(amount);
                remaining -= u64::from(amount);
                plan.requested_service_ticks = plan
                    .requested_service_ticks
                    .checked_add(
                        u64::from(amount)
                            .checked_mul(effects::HARVEST_TICKS_PER_UNIT)
                            .ok_or_else(bounded)?,
                    )
                    .ok_or_else(bounded)?;
                let mut preconditions = worker_guards(
                    staffing.worker,
                    staffing.resource,
                    staffing.previously_enabled.then_some(true),
                );
                preconditions.push(model_guard(snapshot));
                plan.requested_actions.push(RequestedAction {
                    action: Action::HarvestResource {
                        source: source.source,
                        source_generation: source.generation,
                        resource: source.resource,
                        amount,
                    },
                    preconditions,
                    // StaticPlanner adds original task, source, ledger and
                    // conservation proof. A stock goal cannot replace it.
                    postconditions: Vec::new(),
                    compensation: None,
                    obligation: None,
                    depends_on: staffing.action_index.into_iter().collect(),
                });
                plan.allocations.push(ResourceAcquisitionAllocation {
                    resource: source.resource,
                    source: source.source,
                    source_generation: source.generation,
                    amount,
                    action_index,
                });
            }
        }
        plan.deadline_tick = Some(workforce.deadline_floor(&plan.requested_actions)?);
        Ok(plan)
    }
}

fn refuse_competing_consumers(
    snapshot: &WorldSnapshot,
    requirements: &[ResourceAcquisitionRequirement],
) -> Result<()> {
    // A companion quota that is currently met remains part of the original
    // goal while other resources are harvested. Preserve it too. A complete
    // no-op returns before this check because it schedules no future interval.
    if requirements
        .iter()
        .any(|requirement| requirement.resource == HarvestResource::Wood)
    {
        for order in snapshot.graph.entities.values() {
            if order.kind != EntityKind::WorkOrder {
                continue;
            }
            match value(order, effects::STATUS_FIELD, snapshot.tick) {
                Some(Value::Text(status))
                    if status == effects::STATUS_COMPLETE
                        || status == effects::STATUS_CANCELLED =>
                {
                    continue;
                }
                Some(Value::Text(status)) if status == effects::STATUS_ACTIVE => {}
                _ => {
                    return Err(refused(
                        "resource acquisition cannot establish existing work-order lifecycle",
                    ));
                }
            }
            let Some(Value::Text(job)) = value(order, "job_token", snapshot.tick) else {
                return Err(refused(
                    "resource acquisition cannot establish existing work-order material demand",
                ));
            };
            if effects::work_order_material(job)
                .is_some_and(|(field, amount)| field == effects::STOCK_WOOD_FIELD && amount != 0)
            {
                return Err(refused(
                    "original WOOD-consuming production remains active; observe its completion or explicitly stop its original work before acquiring a stock minimum",
                ));
            }
        }
    }
    if requirements
        .iter()
        .any(|requirement| requirement.resource == HarvestResource::Plant)
        && effects::provisions_report(snapshot)?
            .is_some_and(|report| report.remaining_demand_plants != 0)
    {
        return Err(refused(
            "original PLANT-consuming production remains active; observe its completion or explicitly stop its original work before acquiring a stock minimum",
        ));
    }
    Ok(())
}

fn stock_field(resource: HarvestResource) -> &'static str {
    match resource {
        HarvestResource::Wood => effects::STOCK_WOOD_FIELD,
        HarvestResource::Plant => effects::STOCK_PLANT_FIELD,
    }
}

fn value<'a>(record: &'a EntityRecord, field: &str, tick: GameTick) -> Option<&'a Value> {
    record
        .fields
        .get(field)
        .and_then(|fact| laboratory_fact_value(fact, tick))
}

fn select_worker(
    snapshot: &WorldSnapshot,
    resource: HarvestResource,
    assign_labor: bool,
) -> Result<(&EntityRecord, bool)> {
    let field = format!("{}{}", effects::LABOR_FIELD_PREFIX, resource.labor());
    let mut selected: Option<(&EntityRecord, bool, u8)> = None;
    for worker in snapshot.graph.entities.values() {
        if worker.kind != EntityKind::Unit
            || worker.id == EntityId::NIL
            || worker.generation == 0
            || value(worker, "alive", snapshot.tick) != Some(&Value::Bool(true))
        {
            continue;
        }
        let enabled = match value(worker, &field, snapshot.tick) {
            Some(Value::Bool(true)) => true,
            Some(Value::Bool(false)) if assign_labor => false,
            _ => continue,
        };
        let military = match value(worker, effects::SQUAD_FIELD, snapshot.tick) {
            Some(Value::Null) => 0,
            Some(Value::Entity(_) | Value::U64(_)) => 2,
            _ => 1,
        };
        if selected
            .as_ref()
            .is_none_or(|(previous, was_enabled, rank)| {
                (!enabled, military, worker.id) < (!*was_enabled, *rank, previous.id)
            })
        {
            selected = Some((worker, enabled, military));
        }
    }
    selected
        .map(|(worker, enabled, _)| (worker, enabled))
        .ok_or_else(|| {
            refused(format!(
                "resource acquisition needs a qualified living {} worker; assign_labor=true may enable an observed false labor, never an unknown one",
                resource.labor(),
            ))
        })
}

fn worker_guards(
    worker: EntityId,
    resource: HarvestResource,
    enabled: Option<bool>,
) -> Vec<Predicate> {
    let mut guards = vec![
        Predicate::EntityKind {
            entity_id: worker,
            kind: EntityKind::Unit,
        },
        field_guard(worker, "alive", CompareOp::Eq, Value::Bool(true)),
    ];
    if let Some(enabled) = enabled {
        guards.push(field_guard(
            worker,
            &format!("{}{}", effects::LABOR_FIELD_PREFIX, resource.labor()),
            CompareOp::Eq,
            Value::Bool(enabled),
        ));
    }
    guards
}

fn field_guard(entity_id: EntityId, field: &str, op: CompareOp, value: Value) -> Predicate {
    Predicate::FieldCompare {
        entity_id,
        field: field.to_owned(),
        op,
        value,
    }
}

fn model_guard(snapshot: &WorldSnapshot) -> Predicate {
    field_guard(
        effects::fortress_settings_entity_id(snapshot.fortress_id),
        effects::LABORATORY_HARVEST_MODEL_FIELD,
        CompareOp::Eq,
        Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
    )
}

fn next_action_index(actions: &[RequestedAction]) -> Result<u32> {
    if actions.len() >= MAX_RESOURCE_ACQUISITION_ACTIONS {
        return Err(bounded());
    }
    u32::try_from(actions.len()).map_err(|_| bounded())
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn refused(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::PreconditionsFailed, message)
}

fn bounded() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "resource acquisition exceeds its complete source, arithmetic or sixteen-action bound",
    )
}

#[cfg(test)]
#[path = "resource_acquisition_tests.rs"]
mod tests;
