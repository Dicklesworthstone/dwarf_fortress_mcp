//! Source-bound procurement for an original production goal. Unfunded recipe
//! demand stays private diagnostic data: the first executable stage contains
//! only actual finite harvest and its explicitly authorized labor setup. A
//! later foreground continuation observes yield and recompiles production.

use super::*;
use dfmcp_intent::{ResourceAcquisitionPlan, ResourceStockTarget};
use dfmcp_world::{PredicateEvidence, PredicateTruth};

const MAX_RESOURCE_ROUNDS: u32 = 64;

/// Qualified at the exact source of a retained plan, never inferred from a
/// post-dispatch world. This is transient custody: durable recovery reconstructs
/// it from the already archived source without changing any sealed format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProductionGoalLedger {
    anchor: dfmcp_core::StateAnchor,
    entity: EntityId,
    generation: u32,
}

impl ProductionGoalLedger {
    pub(crate) fn capture(snapshot: &WorldSnapshot) -> Result<Self> {
        inspect_bound(snapshot)?;
        PredicateEvidence::laboratory(snapshot)?;
        let ledger = unique_ledger(snapshot)?;
        Ok(Self {
            anchor: snapshot.anchor(),
            entity: ledger.id,
            generation: ledger.generation,
        })
    }

    pub(crate) fn validate_plan(&self, plan: &PreparedPlan) -> Result<()> {
        if self.anchor != plan.anchor {
            return Err(production_setup_refusal(
                "production ledger custody does not belong to the exact original sealed source",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_current(&self, snapshot: &WorldSnapshot) -> Result<()> {
        let current = Self::capture(snapshot)?;
        if current.anchor.fortress_id != self.anchor.fortress_id
            || current.entity != self.entity
            || current.generation != self.generation
        {
            return Err(production_setup_refusal(
                "the original production stock ledger identity or generation was replaced; reconcile custody before reporting its output goal or continuing it",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ResourceRequirement {
    resource: HarvestResource,
    required_stock: u64,
    observed_stock: u64,
    deficit: u64,
    production_units: u64,
    existing_demand_units: u64,
    workshop_units: u64,
}

fn bounded(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn inspect_bound(snapshot: &WorldSnapshot) -> Result<()> {
    if snapshot.graph.entities.len() > MAX_PRODUCTION_SETUP_ENTITIES
        || snapshot.graph.edges.len() > MAX_PRODUCTION_SETUP_ENTITIES
        || snapshot.graph.chunks.len() > MAX_PRODUCTION_SETUP_ENTITIES
        || snapshot.graph.events.len() > MAX_PRODUCTION_SETUP_ENTITIES
    {
        return Err(bounded(
            "resource-aware production exceeds its bounded complete-source inspection",
        ));
    }
    Ok(())
}

/// Require a unique original ledger without requiring ingredients that an
/// already achieved output goal no longer needs. Missing output facts remain
/// Unknown; a second ledger cannot select whichever count happens to be useful.
pub(super) fn semantic_goal_truth(
    request: &ProductionRequest,
    snapshot: &WorldSnapshot,
) -> Result<PredicateTruth> {
    inspect_bound(snapshot)?;
    let evidence = PredicateEvidence::laboratory(snapshot)?;
    let ledger = unique_ledger(snapshot)?;
    let terms = request
        .quotas
        .iter()
        .map(|(item, minimum)| {
            Ok(Predicate::FieldCompare {
                entity_id: ledger.id,
                field: production_stock_field(item)
                    .ok_or_else(|| invalid("production goal escaped its closed output model"))?
                    .to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(u64::from(*minimum)),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    evidence.evaluate(&Predicate::All(terms).normalized())
}

pub(super) fn semantic_goal_truth_in_ledger(
    request: &ProductionRequest,
    snapshot: &WorldSnapshot,
    original: ProductionGoalLedger,
) -> Result<PredicateTruth> {
    original.validate_current(snapshot)?;
    semantic_goal_truth(request, snapshot)
}

fn unique_ledger(snapshot: &WorldSnapshot) -> Result<&EntityRecord> {
    let mut ledgers = snapshot.graph.entities.iter().filter(|(_, record)| {
        record.kind == EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned())
    });
    let (id, ledger) = ledgers.next().ok_or_else(|| {
        production_setup_refusal("production goal requires its original qualified stock ledger")
    })?;
    if ledgers.next().is_some()
        || *id != ledger.id
        || *id == EntityId::NIL
        || ledger.generation == 0
    {
        return Err(production_setup_refusal(
            "production goal requires one unambiguous original stock ledger",
        ));
    }
    Ok(ledger)
}

/// Read typed model balances, actual deduplicated workshop cost and qualified
/// existing demand. No candidate stock is edited and no error text is parsed.
pub(super) fn requirements(
    snapshot: &WorldSnapshot,
    plan: &dfmcp_intent::ProductionPlan,
    workload: &ProductionWorkload,
    setup_wood: u64,
) -> Result<Vec<ResourceRequirement>> {
    if plan
        .shortages()
        .iter()
        .any(|row| !matches!(row.item_token.as_str(), "WOOD" | "PLANT"))
    {
        return Err(production_setup_refusal(
            "resource-aware production can repair only complete WOOD and PLANT input shortages",
        ));
    }
    let ledger = effects::stock_ledger(snapshot)
        .and_then(|id| snapshot.graph.entities.get(&id))
        .ok_or_else(|| production_setup_refusal("production resource ledger is not established"))?;
    let mut rows = Vec::new();
    for (resource, token, field, existing_demand_units, workshop_units) in [
        (
            HarvestResource::Wood,
            "WOOD",
            effects::STOCK_WOOD_FIELD,
            workload.reserved_wood_units(),
            setup_wood,
        ),
        (
            HarvestResource::Plant,
            "PLANT",
            effects::STOCK_PLANT_FIELD,
            workload.reserved_plant_units(),
            0,
        ),
    ] {
        let production_units = plan
            .requirements()
            .iter()
            .find(|row| row.item_token == token)
            .map_or(0, |row| u64::from(row.consumed_units));
        // An unrelated resource is not a new goal. Whenever this production
        // needs it, retain every existing consumer's demand before sizing more.
        if production_units == 0 && workshop_units == 0 {
            continue;
        }
        let Some(Value::U64(observed_stock)) = production_fact(ledger, field, snapshot.tick) else {
            return Err(production_setup_refusal(format!(
                "resource-aware production requires qualified current {field}; unknown stock is not zero",
            )));
        };
        let required_stock = production_units
            .checked_add(existing_demand_units)
            .and_then(|units| units.checked_add(workshop_units))
            .ok_or_else(|| bounded("production resource demand overflows its exact bound"))?;
        rows.push(ResourceRequirement {
            resource,
            required_stock,
            observed_stock: *observed_stock,
            deficit: required_stock.saturating_sub(*observed_stock),
            production_units,
            existing_demand_units,
            workshop_units,
        });
    }
    Ok(rows)
}

/// A scheduling-only projection of the exact candidate. Conditions and names
/// cannot contribute available stock or change registered service duration.
/// No caller can obtain this projection as an executable compilation.
fn production_horizon(candidate: &ProductionCandidate) -> Result<u64> {
    let mut steps = candidate.setup.actions.clone();
    let offset = u32::try_from(steps.len())
        .map_err(|_| bounded("production setup index exceeds its exact bound"))?;
    let mut indices = BTreeMap::<String, u32>::new();
    for step in candidate.plan.steps() {
        let mut dependencies = candidate
            .setup
            .dependencies
            .get(&step.job_token)
            .cloned()
            .unwrap_or_default();
        if let Some(previous_jobs) = candidate.setup.serial_after.get(&step.job_token) {
            for job in previous_jobs {
                dependencies.insert(*indices.get(job).ok_or_else(|| {
                    invalid("production schedule dependency does not precede its consumer")
                })?);
            }
        }
        for dependency in &step.depends_on {
            let index = u32::try_from(*dependency)
                .ok()
                .and_then(|index| index.checked_add(offset))
                .ok_or_else(|| bounded("production dependency exceeds its exact bound"))?;
            dependencies.insert(index);
        }
        indices.insert(
            step.job_token.clone(),
            u32::try_from(steps.len())
                .map_err(|_| bounded("production schedule exceeds its exact step bound"))?,
        );
        steps.push(json!({
            "action": {
                "kind": "create_work_order",
                "name": format!("resource-aware schedule {}", steps.len()),
                "job_token": step.job_token,
                "amount": step.batches,
                "conditions": [],
            },
            "depends_on": dependencies.into_iter().collect::<Vec<_>>(),
        }));
    }
    candidate.workload.horizon_ticks(&json!(steps).to_string())
}

/// The public grammar is only an exact, bounded program skeleton. Binding
/// installs the core acquisition's qualified guards and compensation intact.
fn acquisition_actions(plan: &ResourceAcquisitionPlan) -> Result<String> {
    let mut steps = Vec::with_capacity(plan.requested_actions.len());
    for requested in &plan.requested_actions {
        let action = match &requested.action {
            Action::HarvestResource {
                source,
                source_generation,
                resource,
                amount,
            } => json!({
                "kind": "harvest_resource",
                "source": source.get().to_string(),
                "source_generation": source_generation,
                "resource": resource.as_str(),
                "amount": amount,
            }),
            Action::SetLabor {
                units,
                labor,
                enabled: true,
            } => json!({
                "kind": "set_labor",
                "units": units.iter().map(|unit| unit.get().to_string()).collect::<Vec<_>>(),
                "labor": labor,
                "enabled": true,
            }),
            _ => {
                return Err(invalid(
                    "resource stage escaped its finite acquisition program",
                ));
            }
        };
        steps.push(json!({"action": action, "depends_on": requested.depends_on}));
    }
    let actions = json!(steps).to_string();
    let skeleton = parse_steps(&actions)?;
    if skeleton.len() != plan.requested_actions.len()
        || skeleton
            .iter()
            .zip(&plan.requested_actions)
            .any(|(current, expected)| {
                current.action != expected.action || current.depends_on != expected.depends_on
            })
    {
        return Err(invalid(
            "resource acquisition projection changed its exact program",
        ));
    }
    Ok(actions)
}

pub(super) fn compile(
    request: &ProductionRequest,
    snapshot: &WorldSnapshot,
) -> Result<ProductionCompilation> {
    let policy = request.acquire_resources.ok_or_else(|| {
        invalid("resource-aware production requires its original acquisition permission")
    })?;
    if request.semantic_goal_truth(snapshot)? == PredicateTruth::True {
        return Err(DfmcpError::new(
            ErrorCode::InvalidIntent,
            "observed stock already meets every original production quota; nothing to produce or acquire",
        ));
    }
    let model = production_consumption::ConsumptionModel::capture(snapshot)?;
    let mut targets = request.quotas.clone();
    for round in 1..=MAX_RESOURCE_ROUNDS {
        let candidate = request.compile_candidate(snapshot, &targets, true, true)?;
        let production_ticks = production_horizon(&candidate)?;
        let deficits = candidate
            .resource_requirements
            .iter()
            .any(|row| row.deficit != 0);
        let acquisition = if deficits {
            let resource_targets = candidate
                .resource_requirements
                .iter()
                .map(|row| ResourceStockTarget {
                    resource: row.resource,
                    minimum: row.required_stock,
                })
                .collect::<Vec<_>>();
            Some(ResourceAcquisitionPlan::compile(
                snapshot,
                &resource_targets,
                policy.assign_labor,
            )?)
        } else {
            None
        };
        let harvest_ticks = acquisition
            .as_ref()
            .map(|plan| {
                plan.deadline_tick
                    .and_then(|deadline| deadline.0.checked_sub(snapshot.tick.0))
                    .filter(|ticks| *ticks > 0)
                    .ok_or_else(|| {
                        invalid("deficient production acquisition has no bounded work horizon")
                    })
            })
            .transpose()?
            .unwrap_or(0);
        let horizon = production_ticks.checked_add(harvest_ticks)
            .filter(|ticks| *ticks <= effects::MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(|| bounded("combined acquisition and production exceed the bounded one-year planning horizon"))?;
        let mut next = targets.clone();
        for (item, minimum) in &request.quotas {
            let target = model
                .allowance(item, horizon)?
                .checked_add(u64::from(*minimum))
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    bounded("resource-aware consumption target exceeds its exact u32 bound")
                })?;
            let previous = next
                .get_mut(item)
                .ok_or_else(|| invalid("resource-aware planning lost an original quota"))?;
            *previous = (*previous).max(target);
        }
        if next != targets {
            targets = next;
            continue;
        }
        let workflow = workflow_report(
            request,
            snapshot,
            &candidate,
            acquisition.as_ref(),
            harvest_ticks,
            production_ticks,
            horizon,
            round,
        )?;
        let mut compiled = match acquisition {
            Some(acquisition) => {
                if acquisition.requested_actions.is_empty() {
                    return Err(invalid(
                        "an unfunded production stage requires actual acquisition work",
                    ));
                }
                let mut analysis = candidate.analysis;
                if let Some(capacity) = candidate.workload.analysis() {
                    analysis["capacity"] = capacity;
                }
                ProductionCompilation {
                    actions: acquisition_actions(&acquisition)?,
                    analysis,
                    terminal: candidate.terminal,
                    workload: candidate.workload,
                    consumption_horizon: None,
                    satisfied_goal_reserve_until: None,
                    setup_workforce: None,
                    acquisition: Some(acquisition),
                }
            }
            None => {
                let mut compiled = request.finish_candidate(snapshot, candidate)?;
                let actual_horizon = compiled.workload.horizon_ticks(&compiled.actions)?;
                if actual_horizon != production_ticks {
                    return Err(DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "resource-aware diagnostic schedule and funded production horizon disagree",
                    ));
                }
                compiled.consumption_horizon = Some(actual_horizon);
                compiled
            }
        };
        production_consumption::annotate(
            request,
            snapshot,
            &model,
            &targets,
            horizon,
            round,
            &mut compiled,
        )?;
        compiled.analysis["consumption"]["planner"] = json!("resource_aware_v1");
        compiled.analysis["consumption"]["method"] =
            json!("monotone_reserve_through_acquisition_and_production_horizons");
        compiled.analysis["resource_workflow"] = workflow;
        return Ok(compiled);
    }
    Err(bounded(
        "resource-aware production did not establish a stable harvest and consumption budget within 64 bounded rounds; the conservative model cannot establish this staged plan",
    ))
}

#[allow(clippy::too_many_arguments)]
fn workflow_report(
    request: &ProductionRequest,
    snapshot: &WorldSnapshot,
    candidate: &ProductionCandidate,
    acquisition: Option<&ResourceAcquisitionPlan>,
    harvest_ticks: u64,
    production_ticks: u64,
    horizon: u64,
    rounds: u32,
) -> Result<Json> {
    let original: Json = serde_json::from_str(&request.canonical_json()).map_err(|error| {
        invalid(format!(
            "canonical resource-aware request is invalid: {error}"
        ))
    })?;
    Ok(json!({
        "schema": "dfmcp.production-resource-workflow/1",
        "planner": "resource_aware_v1",
        "stage": if acquisition.is_some() { "acquire_resources" } else { "production" },
        "original_request": original,
        "source_state_hash": snapshot.state_hash.to_hex(),
        "source_tick": snapshot.tick.0,
        "requirements": candidate.resource_requirements.iter().map(|row| json!({
            "resource": row.resource.as_str(),
            "required_stock": row.required_stock,
            "observed_stock": row.observed_stock,
            "deficit": row.deficit,
            "production_units": row.production_units,
            "existing_demand_units": row.existing_demand_units,
            "workshop_units": row.workshop_units,
        })).collect::<Vec<_>>(),
        "original_production_funded": candidate.material_feasible,
        "production_requirements_validated": true,
        "harvest_horizon_ticks": harvest_ticks,
        "production_horizon_ticks": production_ticks,
        "planning_horizon_ticks": horizon,
        "fixed_point_rounds": rounds,
        "maximum_rounds": MAX_RESOURCE_ROUNDS,
        "allocations": acquisition.map(|plan| plan.allocations.iter().map(|row| json!({
            "resource": row.resource.as_str(),
            "source_entity_id": row.source.get().to_string(),
            "source_generation": row.source_generation,
            "amount": row.amount,
            "action_index": row.action_index,
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "staffing": acquisition.map(|plan| plan.staffing.iter().map(|row| json!({
            "resource": row.resource.as_str(),
            "worker_entity_id": row.worker.get().to_string(),
            "generation": row.generation,
            "previously_enabled": row.previously_enabled,
            "action_index": row.action_index,
        })).collect::<Vec<_>>()).unwrap_or_default(),
        "next_stage": if acquisition.is_some() {
            "After observed harvest completion, explicitly plan continue_goal for the original handle, review the fresh source-bound plan and commit it. Production is recomputed from observed stock."
        } else {
            "Observe and verify every original output quota after production."
        },
        "automatic_continuation": false,
        "future_output_counted_as_stock": false,
        "original_terminal_quotas_preserved": true,
        "completion_guaranteed": false,
        "forecast_assumptions": "Conservative reference service through sequential harvest and production horizons; current workers, workshops and population persist, foreground observations release dependencies by their deadlines, and no new competing work arrives. Additional delay or interference requires fresh continuation analysis.",
    }))
}

#[cfg(test)]
#[path = "production_resource_tests.rs"]
mod tests;
