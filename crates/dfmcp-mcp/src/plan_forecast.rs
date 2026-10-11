//! Bounded original-plan forecast on a discarded laboratory fork.
//!
//! Predicted goal truth, action proof and physical quiescence remain distinct.
//! No predicted snapshot becomes canonical evidence or dispatch authority.

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{ActionId, CommitState, DfmcpError, OperationContext};
use dfmcp_intent::PreparedPlan;
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{PredicateEvidence, PredicateTruth};
use serde_json::json;

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
    let mut fork = adapter.clone();
    let context = |fork: &MemoryAdapter| OperationContext {
        anchor: fork.snapshot().anchor(),
        ..template.clone()
    };
    let start = fork.snapshot().tick;
    let unavailable = |reason: &DfmcpError| {
        json!({
            "epistemic_state": "predicted",
            "available": false,
            "reason": {"code": reason.code.as_str(), "message": reason.message},
        })
    };
    let prepared = match fork.prepare(plan, &context(&fork)) {
        Ok(prepared) => prepared,
        Err(error) => return unavailable(&error),
    };
    let receipt = match fork.commit(plan, &prepared, &context(&fork)) {
        Ok(receipt) => receipt,
        Err(error) => return unavailable(&error),
    };
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
        PredicateEvidence::laboratory(fork.snapshot())?.evaluate(&plan.terminal_condition)
    };
    let mut quiescent = match work_quiescent(&fork) {
        Ok(quiet) => quiet,
        Err(error) => return unavailable(&error),
    };
    let mut goal = match goal_truth(&fork) {
        Ok(truth) => truth,
        Err(error) => return unavailable(&error),
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
    let span = horizon.saturating_sub(start.0);
    let slice = span
        .div_ceil(MAX_FORECAST_SLICES)
        .max(dfmcp_intent::effects::DEFAULT_POLL_INTERVAL_TICKS);
    if !blocked_by_pause && span > 0 {
        let mut elapsed = 0u64;
        while elapsed < span
            && (outcomes.iter().any(|(_, _, state, _)| !state.is_terminal())
                || !quiescent
                || goal != PredicateTruth::True)
        {
            let advance = slice.min(span - elapsed);
            if let Err(error) = fork.advance_ticks(advance) {
                return unavailable(&error);
            }
            elapsed += advance;
            for outcome in &mut outcomes {
                if outcome.2.is_terminal() {
                    continue;
                }
                let polled = match fork.poll_action(outcome.1, &context(&fork)) {
                    Ok(polled) => polled,
                    Err(error) => return unavailable(&error),
                };
                outcome.2 = polled.state;
                if polled.state.is_terminal() {
                    outcome.3 = Some(fork.snapshot().tick.0);
                }
            }
            quiescent = match work_quiescent(&fork) {
                Ok(quiet) => quiet,
                Err(error) => return unavailable(&error),
            };
            goal = match goal_truth(&fork) {
                Ok(truth) => truth,
                Err(error) => return unavailable(&error),
            };
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
    json!({
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
    })
}

#[cfg(test)]
#[path = "plan_forecast_tests.rs"]
mod tests;
