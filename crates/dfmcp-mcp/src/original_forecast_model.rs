//! Joint prediction of already committed originals on one discarded world.
//! Retain action identities, obligation runtimes, deadlines and dispatch order.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    ActionId, Capability, CommitState, DfmcpError, Digest32, ErrorCode, OperationContext, Result,
    RiskTier,
};
use dfmcp_intent::EffectWorkState;
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{PredicateTruth, WorldSnapshot};
use serde_json::{Value, json};

const MAX_SLICES: u64 = 4_096;
const MAX_POLLS: u64 = 65_536;
const MAX_TRACKED: usize = 1_024;

#[path = "original_project_forecast.rs"]
mod projects;
pub(super) use projects::Project;

#[derive(Clone)]
pub(super) struct Original {
    pub(super) requested: Digest32,
    pub(super) actual: Digest32,
    pub(super) actions: Vec<ActionId>,
    pub(super) reserve_until: Option<u64>,
    pub(super) reserve_observation: Option<PredicateTruth>,
    pub(super) can_continue: bool,
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn truth_name(truth: PredicateTruth) -> &'static str {
    match truth {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

fn work_json(work: &EffectWorkState) -> Value {
    match work {
        EffectWorkState::NeverDispatched => json!({"kind":"never_dispatched"}),
        EffectWorkState::Active { entity_id } => {
            json!({"kind":"active","entity_id":entity_id.to_string()})
        }
        EffectWorkState::Quiescent { entity_id } => {
            json!({"kind":"quiescent","entity_id":entity_id.map(|id|id.to_string())})
        }
        EffectWorkState::Unknown { entity_id, reason } => {
            json!({"kind":"unknown","entity_id":entity_id.map(|id|id.to_string()),"reason":reason})
        }
    }
}

fn check(context: &OperationContext, started: Instant) -> Result<()> {
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        context.authorize(capability, RiskTier::ReadOnly, &[], None)?;
    }
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "joint forecast exhausted its shared wall-time budget",
        ));
    }
    Ok(())
}

struct Outcome {
    value: Value,
    complete: bool,
    attention: bool,
    continuation: bool,
}

fn observe(
    fork: &MemoryAdapter,
    original: &Original,
    reserve: Option<PredicateTruth>,
    goal: PredicateTruth,
    stage: PredicateTruth,
) -> Result<Outcome> {
    let mut verified = 0;
    let mut terminal = 0;
    let mut active = 0;
    let mut unknown = 0;
    let mut failed = false;
    let mut cancelled = false;
    let mut cancelling = false;
    let mut indeterminate = false;
    let mut steps = Vec::with_capacity(original.actions.len());
    for id in &original.actions {
        let receipt = fork.action_receipt(*id).ok_or_else(|| {
            error(
                ErrorCode::InvalidPlan,
                "predicted original action disappeared",
            )
        })?;
        verified += usize::from(receipt.state == CommitState::Verified);
        terminal += usize::from(receipt.state.is_terminal());
        failed |= receipt.state == CommitState::Failed;
        cancelled |= matches!(
            receipt.state,
            CommitState::Cancelled | CommitState::Compensated
        );
        cancelling |= matches!(
            receipt.state,
            CommitState::CancelRequested | CommitState::CompensationPending
        );
        indeterminate |= matches!(
            receipt.state,
            CommitState::Committing | CommitState::Indeterminate
        );
        let work = fork.action_work_state(*id)?;
        active += usize::from(matches!(work, EffectWorkState::Active { .. }));
        unknown += usize::from(matches!(work, EffectWorkState::Unknown { .. }));
        steps.push(
            json!({"action_id":id.to_string(), "step":receipt.step_id.get(),
            "predicted_state":format!("{:?}",receipt.state),
            "predicted_physical_work":work_json(&work)}),
        );
    }
    let quiet = unknown == 0 && active == 0;
    let all_verified = !original.actions.is_empty() && verified == original.actions.len();
    let all_terminal = !original.actions.is_empty() && terminal == original.actions.len();
    let now = fork.snapshot().tick.0;
    let reserve_complete =
        original.reserve_until.is_none() || reserve == Some(PredicateTruth::True);
    let complete = all_verified && quiet && goal == PredicateTruth::True && reserve_complete;
    let stop = if indeterminate {
        "indeterminate"
    } else if unknown > 0 {
        "work_unknown"
    } else if failed {
        "failed"
    } else if cancelled {
        "cancelled"
    } else if cancelling {
        "cancellation_pending"
    } else if original.reserve_until.is_some_and(|until| now > until) && reserve.is_none() {
        "reserve_unobserved"
    } else if reserve == Some(PredicateTruth::False) {
        "reserve_unmet"
    } else if reserve == Some(PredicateTruth::Unknown) {
        "reserve_unknown"
    } else if complete {
        "complete"
    } else if all_terminal && quiet && goal == PredicateTruth::Unknown {
        "goal_unknown"
    } else if all_terminal && quiet && goal == PredicateTruth::False {
        if original.can_continue && all_verified && stage == PredicateTruth::True {
            "needs_continuation"
        } else {
            "goal_unmet"
        }
    } else if all_terminal && quiet && !all_verified {
        "goal_unmet"
    } else if all_verified && quiet && original.reserve_until.is_some_and(|until| now < until) {
        "reserve_pending"
    } else {
        "pending"
    };
    Ok(Outcome {
        complete,
        continuation: false,
        attention: !matches!(stop, "complete" | "pending" | "reserve_pending"),
        value: json!({
            "requested_plan_digest":original.requested.to_hex(),
            "actual_plan_digest":original.actual.to_hex(),
            "predicted_outcome":stop, "predicted_complete":complete,
            "predicted_goal_truth":truth_name(goal), "predicted_stage_truth":truth_name(stage),
            "predicted_actions_complete":all_verified,
            "predicted_physical_quiescent":if unknown == 0 {Some(quiet)} else {None},
            "reserve_until_tick":original.reserve_until,
            "reserve_frontier_truth":reserve.map(truth_name),
            "reserve_observation_origin":if original.reserve_observation.is_some() {"retained_original"} else {"private_prediction"},
            "continuous_maintenance_proven":false, "steps":steps,
        }),
    })
}

/// The caller admits exact current custody and the complete source before this
/// single clone. `inspect` bounds every derived source; `evaluate` reads the
/// admitted immutable original goals without publishing any history.
pub(super) fn forecast(
    adapter: &MemoryAdapter,
    originals: &[Original],
    tracked: &[ActionId],
    context: &OperationContext,
    horizon_ticks: u64,
    started: Instant,
    mut projects: Option<Vec<Project>>,
    mut inspect: impl FnMut(&WorldSnapshot, u64, u64) -> Result<()>,
    mut evaluate: impl FnMut(Digest32, &WorldSnapshot) -> Result<(PredicateTruth, PredicateTruth)>,
) -> Result<Value> {
    check(context, started)?;
    if adapter.snapshot().anchor() != context.anchor {
        return Err(error(
            ErrorCode::StaleAnchor,
            "joint forecast requires its exact current source",
        ));
    }
    if originals.is_empty() || originals.len() > 8 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "joint forecast requires one to eight originals",
        ));
    }
    if horizon_ticks > context.budget.max_game_ticks
        || tracked.len() > MAX_TRACKED
        || tracked.len() > context.budget.max_entities as usize
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "joint forecast exceeds its horizon or complete action-domain budget",
        ));
    }
    let horizon = context
        .anchor
        .tick
        .0
        .checked_add(horizon_ticks)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "joint forecast exceeds the game-time identity space",
            )
        })?;
    let selected: BTreeSet<_> = tracked.iter().copied().collect();
    if selected.len() != tracked.len() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "joint forecast cannot duplicate tracked actions",
        ));
    }
    let mut preceding = BTreeSet::new();
    for id in tracked {
        let step = adapter.action_step(*id).ok_or_else(|| {
            error(
                ErrorCode::InvalidPlan,
                "tracked forecast action lacks its original step",
            )
        })?;
        let plan = adapter.action_plan_receipt(*id).ok_or_else(|| {
            error(
                ErrorCode::InvalidPlan,
                "tracked forecast action lacks its original commit",
            )
        })?;
        for dependency in &step.depends_on {
            if let Some(parent) = adapter.step_receipt(plan.plan_id, *dependency)
                && selected.contains(&parent.action_id)
                && !preceding.contains(&parent.action_id)
            {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "joint forecast must preserve original prerequisite-before-dependent dispatch order",
                ));
            }
        }
        preceding.insert(*id);
    }
    if projects.as_ref().is_some_and(|projects| {
        projects.len() != originals.len()
            || projects
                .iter()
                .zip(originals)
                .any(|(project, original)| project.original != original.actual)
    }) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "joint project sources differ from selected original identities",
        ));
    }
    let mut all_actions = selected.clone();
    for original in originals {
        all_actions.extend(&original.actions);
    }
    let mut modeled_actions = 0u64;
    let mut modeled_plan_bytes = 0u64;
    let mut modeled_stages = 0u64;
    let mut project_order: Vec<_> = (0..originals.len()).collect();
    project_order.sort_by_key(|index| originals[*index].actual);
    inspect(adapter.snapshot(), 0, 0)?;
    let mut fork = adapter.clone();
    let mut samples: Vec<_> = originals
        .iter()
        .map(|original| original.reserve_observation)
        .collect();
    let mut slices = 0u64;
    let mut polls = 0u64;
    let mut dispatches = 0u32;
    let mut first_poll = true;
    let mut pending = tracked.to_vec();
    let (outcomes, stop) = loop {
        check(context, started)?;
        let observation_context = OperationContext {
            anchor: fork.snapshot().anchor(),
            ..context.clone()
        };
        observation_context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if projects.is_some() {
            observation_context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        }
        let mut outcomes = Vec::with_capacity(originals.len());
        for (index, original) in originals.iter().enumerate() {
            let (mut goal, stage) = evaluate(original.actual, fork.snapshot())?;
            if let Some(projects) = &projects {
                goal = projects[index].goal_truth(fork.snapshot())?;
            }
            if samples[index].is_none() && original.reserve_until == Some(fork.snapshot().tick.0) {
                samples[index] = Some(goal);
            }
            outcomes.push(match &projects {
                Some(projects) => {
                    projects[index].outcome(&fork, original, samples[index], goal, stage)?
                }
                None => observe(&fork, original, samples[index], goal, stage)?,
            });
        }
        // Capture every exact reserve endpoint above before any future stage
        // can change the shared world at this same tick. One deterministic
        // successor is admitted at a time, then all goals are reevaluated.
        if fork.snapshot().tick.0 < horizon
            && !outcomes.iter().any(|outcome| outcome.attention)
            && let Some(projects) = projects.as_mut()
            && let Some(index) = project_order
                .iter()
                .copied()
                .find(|index| outcomes[*index].continuation)
        {
            check(context, started)?;
            inspect(fork.snapshot(), modeled_actions, modeled_plan_bytes)?;
            let modeled_context = OperationContext {
                anchor: fork.snapshot().anchor(),
                ..context.clone()
            };
            let plan = projects[index].prepare_stage(&fork, &modeled_context)?;
            check(context, started)?;
            let added_actions = plan.steps.len() as u64;
            let next_actions = modeled_actions.saturating_add(added_actions);
            if (all_actions.len() as u64).saturating_add(added_actions)
                > u64::from(context.budget.max_actions).min(MAX_TRACKED as u64)
                || (all_actions.len() as u64).saturating_add(added_actions)
                    > u64::from(context.budget.max_entities)
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "joint project exceeds its cumulative original and hypothetical action budget",
                ));
            }
            let (archive, _) = dfmcp_intent::plan_archive::encode_with_check(
                &plan,
                dfmcp_intent::plan_archive::ArchiveLimits {
                    max_bytes: usize::try_from(context.budget.max_bytes)
                        .map_or(dfmcp_intent::plan_archive::MAX_ARCHIVE_BYTES, |value| value)
                        .min(dfmcp_intent::plan_archive::MAX_ARCHIVE_BYTES),
                    max_steps: 64,
                    ..dfmcp_intent::plan_archive::ArchiveLimits::default()
                },
                || check(context, started),
            )?;
            let next_bytes = modeled_plan_bytes.saturating_add(archive.len() as u64);
            inspect(fork.snapshot(), next_actions, next_bytes)?;
            let prepared = fork.prepare(&plan, &modeled_context)?;
            let receipt = fork.commit(&plan, &prepared, &modeled_context)?;
            let actions: Vec<_> = receipt
                .actions
                .into_iter()
                .map(|receipt| receipt.action_id)
                .collect();
            if actions.len() != plan.steps.len()
                || actions.iter().any(|id| !all_actions.insert(*id))
            {
                return Err(error(
                    ErrorCode::InternalInvariantViolation,
                    "joint model stage reused an action identity",
                ));
            }
            pending.extend(&actions);
            projects[index].committed(plan, actions);
            modeled_actions = next_actions;
            modeled_plan_bytes = next_bytes;
            modeled_stages += 1;
            inspect(fork.snapshot(), modeled_actions, modeled_plan_bytes)?;
            first_poll = true;
            continue;
        }
        if outcomes.iter().all(|outcome| outcome.complete) {
            break (outcomes, "all_complete");
        }
        if outcomes.iter().any(|outcome| outcome.attention) {
            break (outcomes, "attention_required");
        }
        if fork.snapshot().tick.0 >= horizon {
            break (outcomes, "horizon_reached");
        }
        if !first_poll && fork.snapshot().paused {
            break (outcomes, "paused");
        }
        if slices >= MAX_SLICES || polls.saturating_add(pending.len() as u64) > MAX_POLLS {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "joint forecast exceeds its complete simulation-work bound",
            ));
        }
        let now = fork.snapshot().tick.0;
        let mut interval = 100.min(horizon - now);
        for original in originals {
            if let Some(until) = original.reserve_until
                && until > now
            {
                interval = interval.min(until - now);
            }
        }
        for id in &pending {
            let receipt = fork.action_receipt(*id).ok_or_else(|| {
                error(
                    ErrorCode::InvalidPlan,
                    "tracked forecast receipt disappeared",
                )
            })?;
            if matches!(
                receipt.state,
                CommitState::Prepared | CommitState::AppliedAwaitingVerification
            ) && let Some(obligation) = fork
                .action_step(*id)
                .and_then(|step| step.obligation.as_ref())
            {
                if obligation.poll_interval_ticks == 0 {
                    return Err(error(
                        ErrorCode::InvalidPlan,
                        "joint forecast original has zero observation cadence",
                    ));
                }
                interval = interval.min(obligation.poll_interval_ticks);
                if obligation.deadline_tick.0 > now {
                    interval = interval.min(obligation.deadline_tick.0 - now);
                }
            }
        }
        let advance = if first_poll || fork.snapshot().paused {
            0
        } else {
            interval
        };
        if advance > 0 {
            fork.advance_ticks(advance)?;
            inspect(fork.snapshot(), modeled_actions, modeled_plan_bytes)?;
        }
        for id in &pending {
            check(context, started)?;
            let was_prepared = fork
                .action_receipt(*id)
                .is_some_and(|receipt| receipt.state == CommitState::Prepared);
            let predicted_context = OperationContext {
                anchor: fork.snapshot().anchor(),
                ..context.clone()
            };
            // No new grant is minted. Deferred actions recheck their real scope
            // and expiry at the modeled frontier, including modeled pauses.
            let receipt = fork.poll_action(*id, &predicted_context)?;
            polls += 1;
            if was_prepared
                && receipt.state != CommitState::Prepared
                && !matches!(
                    fork.action_work_state(*id)?,
                    EffectWorkState::NeverDispatched
                )
            {
                dispatches = dispatches.checked_add(1).ok_or_else(|| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "joint forecast dispatch counter exhausted",
                    )
                })?;
                if dispatches > context.budget.max_actions {
                    return Err(error(
                        ErrorCode::BudgetExceeded,
                        "joint forecast exceeds its aggregate deferred-dispatch budget",
                    ));
                }
                inspect(fork.snapshot(), modeled_actions, modeled_plan_bytes)?;
            }
        }
        slices += 1;
        first_poll = false;
        inspect(fork.snapshot(), modeled_actions, modeled_plan_bytes)?;
        // Match the foreground owner's pruning of fully drained originals.
        // Their immutable receipts remain available to goal inspection, while
        // a late reserve horizon does not repeatedly poll already quiet work.
        let mut unfinished = Vec::with_capacity(pending.len());
        for id in pending {
            let receipt = fork.action_receipt(id).ok_or_else(|| {
                error(
                    ErrorCode::InvalidPlan,
                    "tracked forecast receipt disappeared",
                )
            })?;
            if !receipt.state.is_terminal() || !fork.action_work_state(id)?.is_quiescent() {
                unfinished.push(id);
            }
        }
        pending = unfinished;
    };
    check(context, started)?;
    let mut report = json!({
        "epistemic_state":"predicted", "available":true,
        "method":"joint_committed_originals_on_one_discarded_laboratory_fork",
        "all_complete":outcomes.iter().all(|outcome|outcome.complete),
        "stop_reason":stop, "originals":outcomes.into_iter().map(|outcome|outcome.value).collect::<Vec<_>>(),
        "from_tick":context.anchor.tick.0, "horizon_tick":horizon,
        "predicted_frontier_tick":fork.snapshot().tick.0,
        "predicted_frontier_hash":fork.snapshot().state_hash.to_hex(),
        "predicted_paused":fork.snapshot().paused,
        "completion_rule":"all selected original goals true, exact reserve endpoints satisfied, original action proofs Verified and original physical work quiet at the same predicted frontier",
        "forecast_work":{"source_forks":1,"tracked_actions":tracked.len(),
            "simulation_slices":slices,"action_polls":polls,"deferred_dispatches":dispatches,
            "simulated_game_ticks":fork.snapshot().tick.0-context.anchor.tick.0,
            "actual_game_ticks_advanced":0,"maximum_slices":MAX_SLICES,"maximum_polls":MAX_POLLS},
        "effect_evidence":false,"changes_action_deadlines":false,
        "assumptions":[
            "one finite shared worker, stock, material and consumption world; every current-session open original action is polled in commit order",
            "already-dispatched work from other owners evolves in the shared world, but their deferred actions are not polled",
            "the current paused state is preserved unless an already-committed authorized action changes it; no automatic unpause",
            "no new plan, project continuation or external intervention is invented",
            "canonical snapshot source accounting is not a measured heap-byte count of retained adapter history",
            "predicted anchors, receipts and reserve samples are discarded and confer no evidence or authority"
        ],
    });
    if projects.is_some() {
        report["method"] =
            json!("joint_complete_original_projects_on_one_discarded_laboratory_fork");
        report["forecast_scope"] = json!("complete_project");
        report["future_stage_plans_are_executable"] = json!(false);
        report["requires_review_between_stages"] = json!(true);
        report["completion_rule"] = json!(
            "all selected original semantic goals true, exact reserves true, original and hypothetical actions Verified, and all their physical work quiet at one shared frontier"
        );
        report["forecast_work"]["hypothetical_stages"] = json!(modeled_stages);
        report["forecast_work"]["hypothetical_action_steps"] = json!(modeled_actions);
        report["forecast_work"]["hypothetical_plan_bytes"] = json!(modeled_plan_bytes);
        report["forecast_work"]["maximum_future_stages_per_original"] =
            json!(projects::MAX_FUTURE_STAGES);
        report["future_stage_order"] = json!(
            "ready original actual digest ascending; existing action polls retain commit order"
        );
        report["assumptions"][3] = json!(
            "each eligible selected root receives a separately reviewed hypothetical continuation without delay; unselected roots receive no new stages"
        );
    }
    Ok(report)
}

#[cfg(test)]
#[path = "original_forecast_model_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "original_project_forecast_tests.rs"]
mod project_tests;
