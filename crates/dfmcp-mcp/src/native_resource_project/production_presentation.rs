//! Production next steps preserve the actual child's original and current scope.
use crate::agent_turn::recommendation;
use crate::live_production_presentation::original_prepared_review_request;
use dfmcp_core::SessionId;
use serde_json::{Value, json};

/// Never promote a retained child or failed publication into current advice.
fn current_child<'a>(result: &'a Value, reference: &Value) -> Option<&'a Value> {
    let child = &result["child"];
    if result["ok"] != true
        || result["mode"] != "control"
        || result["stage"] != "production"
        || result["project_custody_currently_verified"] != true
        || result["project_cancelled"] == true
        || result["session_closed"] == true
        || result["original_goal_proven"] == true
        || result["current_stock_goal"]["already_satisfied"].is_object()
        || reference["controller_fenced"] == true
        || super::publication_pending(reference)
        || child["ok"] != true
        || child["mode"] != "control"
        || child["local_parent_only"] == true
        || !child["no_work_stock_goal"].is_null()
        || !result["current_stock_goal"]["no_work_stock_goal"].is_null()
        || !child["original_plan"]["plan_digest"].is_string()
        || reference["retained_stage"]["stage"] != "production"
        || reference["retained_stage"]["checkpoint"]["production"]["recovery"]["plan_digest"]
            != child["original_plan"]["plan_digest"]
    {
        return None;
    }
    Some(child)
}

fn evidence(pointers: &[String]) -> Vec<Value> {
    pointers.iter().map(|pointer| json!({
        "complete_evidence_pointer":format!("/result/child{pointer}"),
    })).collect()
}

fn plan_next(
    session: SessionId,
    request: &Value,
    reason: &str,
    pointers: &[String],
) -> Value {
    let mut next = recommendation(
        "continue_original_production",
        "fortress_plan",
        reason,
        "high",
        "high",
        "guarded",
        "preparation can be cancelled; dispatch requires a separate current confirmation",
        false,
        json!({"session_id":session.to_string(),"request":request.to_string()}),
    );
    next["grants_authority"] = json!(false);
    next["prepares_review_only"] = json!(true);
    next["separate_commit_confirmation_required"] = json!(true);
    next["confidence"]["evidence"] = json!(evidence(pointers));
    next["invalidating_conditions"] = json!([
        "original custody, selection or independent authority changes",
        "source, epoch, consumed floor or paused observation changes",
        "original deadline or request allowance expires",
        "another original effect or clock becomes pending",
    ]);
    next
}

pub(super) fn prepared_review_next(
    result: &Value,
    reference: &Value,
    session: SessionId,
) -> Option<Vec<Value>> {
    let child = current_child(result, reference)?;
    let (step, request) = original_prepared_review_request(child, true)?;
    let mut next = plan_next(
        session,
        &request,
        "Obtain a fresh review of the exact retained undispatched original step. Native preparation is not repeated, its lifetime is not renewed, and a new confirmation is required.",
        &["/frontier".into(), "/original_plan".into()],
    );
    next["original_step_id"] = json!(step);
    Some(vec![next])
}

pub(super) fn affordances(result: &Value, reference: &Value) -> Vec<Value> {
    let Some(child) = current_child(result, reference) else {
        return Vec::new();
    };
    original_prepared_review_request(child, true)
        .map(|(step, request)| json!({
            "intent":"review_prepared_original_step",
            "original_step_id":step,
            "request":request,
            "risk":"guarded",
            "prepares_review_only":true,
            "native_preparation_repeated":false,
            "separate_commit_confirmation_required":true,
            "grants_authority":false,
            "complete_evidence_pointer":"/result/child/frontier",
        }))
        .into_iter()
        .collect()
}
