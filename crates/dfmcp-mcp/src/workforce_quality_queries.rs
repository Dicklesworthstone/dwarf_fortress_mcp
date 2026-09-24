//! Opt-in workforce objective. The default planner and its wp1 identity remain
//! unchanged; quality-aware pages use a separately versioned model and wq1 token.
use super::*;
use dfmcp_adapter::workforce_analysis::{WorkforcePlan, quality as quality_adapter};
use dfmcp_world::workforce_allocation as weighted;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Objective {
    #[default]
    MaxFilledSlots,
    PrioritySkillDistance,
}

pub(super) struct Details {
    priorities: Vec<u16>,
    allocation: weighted::Allocation,
}

pub(super) fn prepare(
    state: &LiveSpatialCitizenState,
    context: &OperationContext,
    demands: Vec<DemandInput>,
    objective: Option<Objective>,
    maximum: u64,
) -> Result<(WorkforcePlan, Option<Details>)> {
    let objective = objective.unwrap_or_default();
    if objective == Objective::MaxFilledSlots && demands.iter().any(|d| d.priority.is_some()) {
        return Err(invalid(
            "demand priority requires objective priority_skill_distance",
        ));
    }
    let priorities: BTreeMap<_, _> = demands
        .iter()
        .filter_map(|d| d.priority.map(|priority| (d.key.clone(), priority)))
        .collect();
    let requested: Vec<_> = demands.into_iter().map(DemandInput::normalized).collect();
    match objective {
        Objective::MaxFilledSlots => {
            Ok((workforce::plan(state, context, &requested, maximum)?, None))
        }
        Objective::PrioritySkillDistance => {
            let out = quality_adapter::plan(state, context, &requested, &priorities, maximum)?;
            Ok((
                out.plan,
                Some(Details {
                    priorities: out.priorities,
                    allocation: out.quality,
                }),
            ))
        }
    }
}

pub(super) fn model(a: &WorkforceAnalysis, details: Option<&Details>) -> Digest32 {
    let base = model_digest(a, "workforce_plan");
    let Some(details) = details else {
        return base;
    };
    Digest32::of_bytes(json!({"domain":"dfmcp-workforce-quality-model/1","base_model":base.to_string(),
        "objective":"priority_skill_distance","policy":weighted::POLICY,"priorities":details.priorities}).to_string().as_bytes())
}

pub(super) fn decorate(out: &mut Value, details: Option<&Details>, model: Digest32) -> Result<()> {
    let Some(details) = details else {
        return Ok(());
    };
    let allocation = &details.allocation;
    let total_priority = allocation.cost.0[0]
        .checked_neg()
        .ok_or_else(|| invariant("priority sum overflow"))?;
    let total_effective = allocation.cost.0[1]
        .checked_neg()
        .ok_or_else(|| invariant("effective skill sum overflow"))?;
    let total_nominal = allocation.cost.0[2]
        .checked_neg()
        .ok_or_else(|| invariant("nominal skill sum overflow"))?;
    let proof = json!({"domain":"dfmcp-workforce-quality-certificate/1","model":model.to_string(),
        "assignments":allocation.assignments.iter().map(|a| json!([a.demand_index,a.worker_id.to_string()])).collect::<Vec<_>>(),
        "cost":allocation.cost.0,"potentials":allocation.witness.potentials.iter().map(|c| c.0).collect::<Vec<_>>(),
        "source_side":allocation.witness.source_side});
    out["optimization"] = json!({"objective":"priority_skill_distance","policy":weighted::POLICY,
        "order":["maximum_filled_slots","maximum_total_priority","maximum_total_effective_skill","maximum_total_nominal_skill","minimum_total_candidate_steps"],
        "achieved":{"priority":total_priority,"effective_skill":total_effective,"nominal_skill":total_nominal,"candidate_steps":allocation.cost.0[3]},
        "maximum_cardinality_checked":true,"residual_optimality_checked":true,"model_only":true,
        "certificate_digest":Digest32::of_bytes(proof.to_string().as_bytes()).to_string(),
        "tie_policy":"canonical_demand_key_worker_id_arc_order_first_equal_parent",
        "solver_work_units":allocation.work_units,"augmentations":allocation.assigned_workers,
        "node_count":allocation.witness.potentials.len()});
    let demands = out["demands"]
        .as_array_mut()
        .ok_or_else(|| invariant("workforce demand summary missing"))?;
    if demands.len() != details.priorities.len() {
        return Err(invariant("workforce priority summary length differs"));
    }
    for (demand, priority) in demands.iter_mut().zip(&details.priorities) {
        demand["request"]["priority"] = json!(priority);
    }
    Ok(())
}
