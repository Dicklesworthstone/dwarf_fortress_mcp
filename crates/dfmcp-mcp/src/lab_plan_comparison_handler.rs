//! Own a laboratory candidate request through complete response publication.
//! Forecasts are discarded. Only successful selection installs a pending plan.

use super::*;
use std::time::{Duration, Instant};

fn check(current: &OperationContext, started: Instant) -> Result<()> {
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        authorize_entry(current, capability, RiskTier::ReadOnly)?;
    }
    if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "candidate planning exhausted its complete request wall-time budget",
        ));
    }
    Ok(())
}

fn remaining(current: &OperationContext, started: Instant) -> Result<OperationContext> {
    check(current, started)?;
    let spent = started.elapsed().as_nanos().div_ceil(1_000_000);
    if spent >= u128::from(current.budget.max_wall_millis) {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "candidate planning has no wall-time allowance remaining",
        ));
    }
    let mut remaining = current.clone();
    remaining.budget.max_wall_millis -= spent as u64;
    Ok(remaining)
}

fn complete_query_work(current: &OperationContext, started: Instant) -> Result<serde_json::Value> {
    let baselines = temporal_query::retained_baseline_work(&remaining(current, started)?)?;
    let watches = temporal_query::retained_watch_work(&remaining(current, started)?)?;
    Ok(json!({
        "query_baselines": baselines,
        "query_watches": watches["query_watches"],
        "condition_watches": watches["condition_watches"],
    }))
}

pub(super) fn execute(
    session_id: Option<String>,
    request: &plan_comparison::Request,
    started: Instant,
) -> Result<String> {
    let session = resolve_session(session_id)?;
    let _admission = LAB_ADMISSION.read().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "laboratory admission lock poisoned",
        )
    })?;
    ensure_durable_owner(&session)?;
    with_session_view(
        &session,
        || {
            Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "candidate planning session poisoned",
            ))
        },
        |guard| execute_owned(guard, request, started),
        false,
    )
}

fn execute_owned(
    session: &mut LabSession,
    request: &plan_comparison::Request,
    started: Instant,
) -> Result<String> {
    let (rid, current) = next_context(session)?;
    check(&current, started)?;
    let mut selected = None;
    let mut payload = match request {
        plan_comparison::Request::Compare(_) => plan_comparison::compare(
            &session.adapter,
            &current,
            IntentId::new(rid),
            request,
            started,
        )?,
        plan_comparison::Request::Select(_) => {
            let snapshot = session.adapter.snapshot();
            let source_work =
                query_budget::admit_sources(&[snapshot], &remaining(&current, started)?)?;
            let source = plan_comparison::selected_source(request, snapshot, &current)?
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "selected candidate did not yield a plan source",
                    )
                })?;
            check(&current, started)?;
            let (intent, production) = source.compile(IntentId::new(rid), snapshot)?;
            check(&current, started)?;
            let plan = StaticPlanner::default().prepare_laboratory(snapshot, &intent, &current)?;
            check(&current, started)?;
            let digest = plan.digest.to_string();
            let result = json!({
                "kind": "select_candidate",
                "schema": "dfmcp.plan-comparison/1",
                "version": 1,
                "plan_id": plan.id.to_string(),
                "plan_digest": digest,
                "terminal_condition": format!("{:?}", intent.terminal_condition),
                "max_risk": plan.max_risk.as_str(),
                "required_capabilities": plan.required_capabilities.iter()
                    .map(|capability| capability.as_str()).collect::<Vec<_>>(),
                "requires_checkpoint": plan.requires_checkpoint,
                "expires_at_tick": plan.expires_at_tick.0,
                "steps": crate::lab_world::plan_steps_json(&plan),
                "production": production,
                "live_routing": live_routing_json(&plan),
                "source_work": source_work,
                "forecast": null,
                "effect_evidence": false,
                "pending_plan_changed": true,
                "next_step": {"tool": "fortress.commit", "arguments": {
                    "session_id": session.session_id.to_string(), "plan_digest": digest,
                }},
                "note": "freshly prepared ordinary plan; commit still revalidates current authority, source and registered effect policy",
            });
            selected = Some(PendingPlan {
                plan,
                digest,
                source,
            });
            result
        }
    };
    payload["ok"] = json!(true);
    payload["session_id"] = json!(session.session_id.to_string());
    payload["anchor"] = anchor_json(&current.anchor);
    check(&current, started)?;
    let mut projection = temporal_handler::projection(session, &current)?;
    // Recovered work has no live action handle. Keep its retained descriptors
    // visible without sampling or rewriting historical proof during comparison.
    // A terminal goal can still own active or unknown physical work.
    projection.active_work["carried_obligations"] = json!(
        session
            .carried
            .iter()
            .filter(|carried| !carried.is_final())
            .map(CarriedStep::to_json)
            .collect::<Vec<_>>()
    );
    let query_work = complete_query_work(&current, started)?;
    let selected_pending = selected.as_ref().map(|pending| {
        json!({
            "plan_digest": pending.digest,
            "state": "prepared",
            "anchor": anchor_json(&pending.plan.anchor),
            "expires_at_tick": pending.plan.expires_at_tick.0,
            "steps": crate::lab_world::plan_steps_json(&pending.plan),
            "minimum_safe_next_step": {"tool": "fortress.commit", "arguments": {
                "session_id": session.session_id.to_string(), "plan_digest": pending.digest,
            }},
        })
    });
    let output =
        plan_comparison_response::finish(&projection, payload, query_work, selected_pending)?;
    // The session and shared-world owner remain held through this final check.
    // Nothing fallible may replace the accepted packet after pending changes.
    check(&current, started)?;
    if let Some(pending) = selected {
        session.pending = Some(pending);
    }
    Ok(output)
}
