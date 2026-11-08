//! Bounded original-plan forecast on a discarded laboratory fork.
//!
//! Predicted goal truth, action proof and physical quiescence remain distinct.
//! No predicted snapshot becomes canonical evidence or dispatch authority.

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{ActionId, CommitState, DfmcpError, ErrorCode, OperationContext, Result};
use dfmcp_intent::PreparedPlan;
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::PredicateTruth;
use serde_json::json;

#[path = "plan_forecast_budget.rs"]
mod comparison_budget;
pub(super) use comparison_budget::ForecastBudget;

/// Most simulated time slices a forecast may take.
const MAX_FORECAST_SLICES: u64 = 400;

/// Counterfactual: commit the sealed plan on a fork of the current world and
/// run deterministic laboratory time forward to every step's obligation
/// deadline, reporting when each step would verify or fail. The fork is
/// discarded; nothing here changes canonical state or grants authority. The
/// forecast assumes the fortress stays as it is now (paused stays paused) and
/// that no other agent acts.
pub(super) fn forecast_plan(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    template: &OperationContext,
) -> serde_json::Value {
    forecast_inner(adapter, plan, template, None, None).unwrap_or_else(|error| unavailable(&error))
}

/// Keep the production completion forecast separate from the explicitly
/// requested future stock frontier. Neither establishes continuous maintenance.
pub(super) fn forecast_plan_with_reserve(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    template: &OperationContext,
    reserve_until_tick: Option<u64>,
) -> serde_json::Value {
    match reserve_until_tick {
        Some(tick) => forecast_inner(adapter, plan, template, Some(tick), None)
            .unwrap_or_else(|error| unavailable(&error)),
        None => forecast_plan(adapter, plan, template),
    }
}

/// Compare candidates on the existing model under one owned source, deadline
/// and aggregate work meter. Only the common simulation horizon is shortened;
/// the original sealed obligations and their deadlines remain unchanged.
pub(super) fn forecast_plan_bounded(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    template: &OperationContext,
    reserve_until_tick: Option<u64>,
    budget: &mut ForecastBudget,
) -> Result<serde_json::Value> {
    budget.begin_candidate(adapter.snapshot(), plan, template)?;
    let result = forecast_inner(
        adapter,
        plan,
        template,
        reserve_until_tick,
        Some(&mut *budget),
    );
    // Model infeasibility can return early, but cannot outrun the request's
    // deadline or leave its current authority unchecked at the boundary.
    budget.check(template)?;
    result
}

fn unavailable(reason: &DfmcpError) -> serde_json::Value {
    json!({
        "epistemic_state": "predicted",
        "available": false,
        "reason": {"code": reason.code.as_str(), "message": reason.message},
    })
}

fn model_failure(error: DfmcpError, bounded: bool) -> Result<serde_json::Value> {
    if bounded
        && matches!(
            error.code,
            ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
        )
    {
        Err(error)
    } else {
        Ok(unavailable(&error))
    }
}

fn check_budget(budget: &Option<&mut ForecastBudget>, current: &OperationContext) -> Result<()> {
    if let Some(budget) = budget.as_deref() {
        budget.check(current)?;
    }
    Ok(())
}

fn forecast_inner(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    template: &OperationContext,
    reserve_until_tick: Option<u64>,
    mut budget: Option<&mut ForecastBudget>,
) -> Result<serde_json::Value> {
    if let Some(budget) = budget.as_deref_mut() {
        budget.before_fork(adapter.snapshot(), template)?;
    }
    let mut fork = adapter.clone();
    check_budget(&budget, template)?;
    let context = |fork: &MemoryAdapter| OperationContext {
        anchor: fork.snapshot().anchor(),
        ..template.clone()
    };
    let start = fork.snapshot().tick;
    if reserve_until_tick.is_some_and(|until| {
        until <= start.0 || until - start.0 > dfmcp_intent::effects::MAX_DEFAULT_OBLIGATION_TICKS
    }) {
        return model_failure(
            DfmcpError::new(
                dfmcp_core::ErrorCode::InvalidRequest,
                "reserve forecast requires a future tick within one bounded game year",
            ),
            budget.is_some(),
        );
    }
    let prepared = match fork.prepare(plan, &context(&fork)) {
        Ok(prepared) => prepared,
        Err(error) => return model_failure(error, budget.is_some()),
    };
    check_budget(&budget, template)?;
    let receipt = match fork.commit(plan, &prepared, &context(&fork)) {
        Ok(receipt) => receipt,
        Err(error) => return model_failure(error, budget.is_some()),
    };
    check_budget(&budget, template)?;
    let mut outcomes: Vec<(dfmcp_core::StepId, ActionId, CommitState, Option<u64>)> = receipt
        .actions
        .iter()
        .map(|action| {
            let at = action.state.is_terminal().then_some(start.0);
            (action.step_id, action.action_id, action.state, at)
        })
        .collect();
    let work_quiescent = |fork: &MemoryAdapter| -> dfmcp_core::Result<bool> {
        let mut quiet = true;
        for action in &receipt.actions {
            quiet &= fork.action_work_state(action.action_id)?.is_quiescent();
        }
        Ok(quiet)
    };
    let goal_truth = |fork: &MemoryAdapter| -> dfmcp_core::Result<PredicateTruth> {
        crate::lab_world::plan_goal_truth(fork.snapshot(), plan)
    };
    let mut quiescent = match work_quiescent(&fork) {
        Ok(quiet) => quiet,
        Err(error) => return model_failure(error, budget.is_some()),
    };
    let mut goal = match goal_truth(&fork) {
        Ok(truth) => truth,
        Err(error) => return model_failure(error, budget.is_some()),
    };
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|o| o.deadline_tick.0))
        .max()
        .unwrap_or(start.0);
    let blocked_by_pause = fork.snapshot().paused
        && (outcomes.iter().any(|(_, _, state, _)| !state.is_terminal())
            || !quiescent
            || goal != PredicateTruth::True);
    let simulation_horizon = budget.as_deref().map(ForecastBudget::horizon_tick);
    let work_horizon = simulation_horizon.map_or(horizon, |limit| horizon.min(limit));
    let span = work_horizon.saturating_sub(start.0);
    // Reserve one extra slice for an exact in-work frontier and one for the
    // later reserve fork, keeping the complete forecast within the same bound.
    let work_slices = if reserve_until_tick.is_some() {
        MAX_FORECAST_SLICES - 2
    } else {
        MAX_FORECAST_SLICES
    };
    let resolution_span = budget
        .as_deref()
        .map_or(span, ForecastBudget::horizon_ticks);
    let slice = resolution_span
        .div_ceil(work_slices)
        .max(dfmcp_intent::effects::DEFAULT_POLL_INTERVAL_TICKS);
    let mut reserve = None;
    if !blocked_by_pause && span > 0 {
        let mut elapsed = 0u64;
        while elapsed < span
            && (outcomes.iter().any(|(_, _, state, _)| !state.is_terminal())
                || !quiescent
                || goal != PredicateTruth::True)
        {
            check_budget(&budget, template)?;
            let mut advance = slice.min(span - elapsed);
            if let Some(until) = reserve_until_tick
                && until > fork.snapshot().tick.0
            {
                // Inspect the requested frontier exactly, including when work
                // is still outstanding. Never report a later stock as if it
                // had already existed at the requested tick.
                advance = advance.min(until - fork.snapshot().tick.0);
            }
            if let Some(budget) = budget.as_deref_mut() {
                budget.before_advance(advance, template)?;
            }
            if let Err(error) = fork.advance_ticks(advance) {
                return model_failure(error, budget.is_some());
            }
            if let Some(budget) = budget.as_deref_mut() {
                budget.advanced(advance);
            }
            check_budget(&budget, template)?;
            elapsed += advance;
            for outcome in &mut outcomes {
                if outcome.2.is_terminal() {
                    continue;
                }
                if let Some(budget) = budget.as_deref_mut() {
                    budget.before_poll(template)?;
                }
                let polled = match fork.poll_action(outcome.1, &context(&fork)) {
                    Ok(polled) => polled,
                    Err(error) => return model_failure(error, budget.is_some()),
                };
                outcome.2 = polled.state;
                if polled.state.is_terminal() {
                    outcome.3 = Some(fork.snapshot().tick.0);
                }
            }
            quiescent = match work_quiescent(&fork) {
                Ok(quiet) => quiet,
                Err(error) => return model_failure(error, budget.is_some()),
            };
            goal = match goal_truth(&fork) {
                Ok(truth) => truth,
                Err(error) => return model_failure(error, budget.is_some()),
            };
            check_budget(&budget, template)?;
            if reserve_until_tick == Some(fork.snapshot().tick.0) {
                reserve = Some(reserve_frontier(&fork, goal, quiescent));
            }
        }
    }
    let steps: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|(step, _, state, at)| {
            json!({
                "step": step.get(),
                "predicted_state": format!("{state:?}"),
                "predicted_terminal_tick": at,
            })
        })
        .collect();
    let actions_complete = outcomes
        .iter()
        .all(|(_, _, state, _)| *state == CommitState::Verified);
    let goal_complete = goal == PredicateTruth::True;
    let completes = actions_complete && goal_complete && quiescent;
    let mut forecast = json!({
        "epistemic_state": "predicted",
        "available": true,
        "method": "deterministic_laboratory_simulation_on_a_discarded_fork",
        "from_tick": start.0,
        "horizon_tick": horizon,
        "predicted_complete": completes,
        "predicted_completion_tick": completes.then_some(fork.snapshot().tick.0),
        "predicted_actions_complete": actions_complete,
        "predicted_actions_completion_tick": actions_complete.then(|| outcomes.iter().filter_map(|o| o.3).max()).flatten(),
        "predicted_goal_complete": goal_complete,
        "predicted_goal_truth": match goal {
            PredicateTruth::True => "true",
            PredicateTruth::False => "false",
            PredicateTruth::Unknown => "unknown",
        },
        "predicted_physical_quiescent": quiescent,
        "predicted_frontier_tick": fork.snapshot().tick.0,
        "completion_rule": "verified action proofs, true original terminal predicate and physically quiet original work at the same predicted frontier",
        "resolution_ticks": slice,
        "cadence_note": "deferred steps dispatch when a wait observes their prerequisites, so real completion also depends on how often the agent waits",
        "blocked_by_pause": blocked_by_pause,
        "steps": steps,
        "assumes": "the fortress stays as it is now and no other agent acts; a prediction is not evidence",
    });
    if let Some(limit) = simulation_horizon {
        forecast["simulation_horizon_tick"] = json!(limit);
        forecast["simulation_horizon_reached"] = json!(fork.snapshot().tick.0 >= limit);
        forecast["simulation_horizon_truncated"] =
            json!(!completes && work_horizon < horizon && fork.snapshot().tick.0 >= work_horizon);
    }
    if let Some(until) = reserve_until_tick {
        if reserve.is_none()
            && !fork.snapshot().paused
            && fork.snapshot().tick.0 < until
            && simulation_horizon.is_none_or(|limit| until <= limit)
        {
            // Preserve the work-forecast frontier above and all of its reports.
            // Later consumption runs on another discarded fork; it never
            // prolongs action obligations or changes their proof.
            if let Some(budget) = budget.as_deref_mut() {
                budget.before_fork(fork.snapshot(), template)?;
            }
            let mut future = fork.clone();
            let advance = until - future.snapshot().tick.0;
            if let Some(budget) = budget.as_deref_mut() {
                budget.before_advance(advance, template)?;
            }
            reserve = Some(match future.advance_ticks(advance) {
                Ok(()) => {
                    if let Some(budget) = budget.as_deref_mut() {
                        budget.advanced(advance);
                    }
                    match (goal_truth(&future), work_quiescent(&future)) {
                        (Ok(truth), Ok(quiet)) => reserve_frontier(&future, truth, quiet),
                        (Err(error), _) | (_, Err(error)) => {
                            model_failure(error, budget.is_some())?
                        }
                    }
                }
                Err(error) => model_failure(error, budget.is_some())?,
            });
            check_budget(&budget, template)?;
        }
        let mut report = reserve.unwrap_or_else(|| {
            let reason = if simulation_horizon.is_some_and(|limit| until > limit) {
                "the requested reserve frontier exceeds the common simulation horizon"
            } else {
                "the requested future frontier was not reached; a paused or unavailable simulation cannot establish future stock"
            };
            json!({"epistemic_state":"predicted", "available":false, "reason":reason})
        });
        report["reserve_until_tick"] = json!(until);
        report["continuous_maintenance_proven"] = json!(false);
        report["changes_action_deadlines"] = json!(false);
        report["scope"] = json!(
            "original stock quotas at one predicted future frontier; intervening stock minima and real future availability are not proved"
        );
        forecast["reserve_forecast"] = report;
    }
    check_budget(&budget, template)?;
    match crate::lab_world::workforce_json(fork.snapshot()) {
        Ok(Some(mut workforce)) => {
            // A fork's scheduler is a prediction, even though it follows the
            // same exact model as execution. Its anchor is never canonical.
            workforce["epistemic_state"] = json!("predicted");
            workforce["evidence_scope"] = json!("discarded_laboratory_forecast");
            if let Some(fields) = workforce.as_object_mut()
                && let Some(anchor) = fields.remove("source_anchor")
            {
                fields.insert("predicted_anchor".to_owned(), anchor);
            }
            forecast["predicted_workforce"] = workforce;
        }
        Ok(None) => {}
        Err(error) => return model_failure(error, budget.is_some()),
    }
    check_budget(&budget, template)?;
    match crate::lab_world::construction_material_json(fork.snapshot()) {
        Ok(Some(mut materials)) => {
            materials["epistemic_state"] = json!("predicted");
            materials["evidence_scope"] = json!("discarded_laboratory_forecast");
            if let Some(fields) = materials.as_object_mut()
                && let Some(anchor) = fields.remove("source_anchor")
            {
                fields.insert("predicted_anchor".to_owned(), anchor);
            }
            forecast["predicted_construction_materials"] = materials;
        }
        Ok(None) => {}
        Err(error) => return model_failure(error, budget.is_some()),
    }
    check_budget(&budget, template)?;
    match crate::lab_world::provisions_json(fork.snapshot()) {
        Ok(Some(mut provisions)) => {
            provisions["epistemic_state"] = json!("predicted");
            provisions["evidence_scope"] = json!("discarded_laboratory_forecast");
            if let Some(fields) = provisions.as_object_mut()
                && let Some(anchor) = fields.remove("source_anchor")
            {
                fields.insert("predicted_anchor".to_owned(), anchor);
            }
            forecast["predicted_provisions"] = provisions;
        }
        Ok(None) => {}
        Err(error) => return model_failure(error, budget.is_some()),
    }
    check_budget(&budget, template)?;
    Ok(forecast)
}

fn reserve_frontier(fork: &MemoryAdapter, truth: PredicateTruth, quiet: bool) -> serde_json::Value {
    json!({
        "epistemic_state": "predicted",
        "available": true,
        "method": "deterministic_laboratory_simulation_on_a_discarded_fork",
        "predicted_tick": fork.snapshot().tick.0,
        "predicted_state_hash": fork.snapshot().state_hash.to_hex(),
        "predicted_goal_truth": match truth {
            PredicateTruth::True => "true",
            PredicateTruth::False => "false",
            PredicateTruth::Unknown => "unknown",
        },
        "predicted_physical_quiescent": quiet,
    })
}

#[cfg(test)]
#[path = "plan_forecast_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "provisions_control_loop_tests.rs"]
mod provisions_tests;

#[cfg(test)]
#[path = "reserve_forecast_tests.rs"]
mod reserve_tests;

#[cfg(test)]
#[path = "plan_forecast_budget_tests.rs"]
mod budget_tests;
