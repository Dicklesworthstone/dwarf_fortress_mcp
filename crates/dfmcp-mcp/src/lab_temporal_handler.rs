//! Capture current work while the laboratory session is owned.
//! Temporal queries and candidate planning reuse this pure publication input.

use super::*;

pub(super) fn projection(
    session: &LabSession,
    current: &OperationContext,
) -> Result<temporal_response::TemporalResponseProjection> {
    let action_ids = session
        .open_actions
        .iter()
        .chain(&session.last_plan_actions)
        .copied()
        .chain(session.last_action)
        .collect::<std::collections::BTreeSet<_>>();
    let mut actions = Vec::new();
    let mut obligations = Vec::new();
    for action_id in action_ids {
        let receipt = session.adapter.action_receipt(action_id);
        let step = session.adapter.action_step(action_id);
        actions.push(json!({
            "action_id":action_id.to_string(),
            "step":receipt.map(|receipt| receipt.step_id.get()),
            "state":receipt.map(|receipt| format!("{:?}",receipt.state)),
            "work_state":action_work_json(session,action_id),
            "risk":step.map(|step| step.risk.as_str()),
            "capability":step.map(|step| step.required_capability.as_str()),
        }));
        if receipt.is_some_and(|receipt| !receipt.state.is_terminal())
            && let Some(obligation) = step.and_then(|step| step.obligation.as_ref())
        {
            obligations.push(json!({
                "action_id":action_id.to_string(),
                "terminal":crate::lab_world::predicate_json(&obligation.terminal),
                "deadline_tick":obligation.deadline_tick.0,
                "blocked_by_pause":session.adapter.snapshot().paused,
                "next_step":{"tool":"fortress.wait","arguments":{
                    "session_id":session.session_id.to_string(),"max_game_ticks":100}},
            }));
        }
    }
    let pending_plans = session
        .pending
        .iter()
        .map(|pending| {
            let fresh_plan = (pending.plan.anchor != current.anchor)
                .then(|| pending.source.resource_replan_step(session.session_id))
                .flatten();
            json!({
                "plan_digest":pending.digest,
                "state":if fresh_plan.is_some() {"review_required"} else {"prepared"},
                "anchor":anchor_json(&pending.plan.anchor),
                "expires_at_tick":pending.plan.expires_at_tick.0,
                "steps":crate::lab_world::plan_steps_json(&pending.plan),
                "minimum_safe_next_step":fresh_plan.unwrap_or_else(|| json!({
                    "tool":"fortress.commit","arguments":{
                    "session_id":session.session_id.to_string(),"plan_digest":pending.digest}})),
            })
        })
        .collect::<Vec<_>>();
    let mut active_work = crate::agent_turn::empty_active_work();
    active_work["pending_plans"] = json!(pending_plans);
    active_work["actions"] = json!(actions);
    active_work["obligations"] = json!(obligations);
    active_work["untracked_work"] = untracked_work_json(session);
    Ok(temporal_response::TemporalResponseProjection {
        session_id: session.session_id.to_string(),
        request_id: current.request_id.to_string(),
        anchor: anchor_json(&current.anchor),
        paused: session.adapter.snapshot().paused,
        budget: current.budget,
        active_work,
        objectives: objectives_json(session),
    })
}

pub(super) fn execute(
    session: &LabSession,
    current: &OperationContext,
    request: &temporal_query::Request,
) -> Result<String> {
    let started = std::time::Instant::now();
    let projection = projection(session, current)?;
    let result_bytes = projection.result_byte_budget()?;
    let spent_millis = started.elapsed().as_nanos().div_ceil(1_000_000);
    if spent_millis >= u128::from(current.budget.max_wall_millis) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "current laboratory work exhausted the temporal query wall-time allowance",
        ));
    }
    let mut remaining = current.clone();
    remaining.budget.max_wall_millis -= spent_millis as u64;
    temporal_query::execute(
        session.adapter.snapshot(),
        &remaining,
        request,
        result_bytes,
        |payload| projection.finish(payload),
    )
}
