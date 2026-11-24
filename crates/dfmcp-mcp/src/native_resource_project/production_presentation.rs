//! Production next steps preserve the actual child's original and current scope.
use crate::agent_turn::recommendation;
use crate::live_production_presentation::original_prepared_review_request;
use crate::live_production_runtime::clock_guidance::{self, Guidance, NextAction};
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

fn guidance(result: &Value, reference: &Value) -> Option<Guidance> {
    clock_guidance::from_view(current_child(result, reference)?)
}

fn evidence(pointers: &[String]) -> Vec<Value> {
    pointers.iter().map(|pointer| json!({
        "complete_evidence_pointer":format!("/result/child{pointer}"),
    })).collect()
}

fn next_action_name(action: &NextAction) -> &'static str {
    match action {
        NextAction::Wait => "wait",
        NextAction::Inspect => "inspect",
        NextAction::Operator => "operator",
        NextAction::None => "none",
    }
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
    if let Some(ticks) = request["game_ticks"].as_u64() {
        next["estimated_cost"]["game_ticks_after_separate_commit"] = json!(ticks);
        next["estimated_cost"]["native_time_advanced_by_planning"] = json!(0);
        next["estimated_cost"]["clock_wall_millis"] = request["wall_millis"].clone();
    }
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

pub(super) fn clock_next(
    result: &Value,
    reference: &Value,
    session: SessionId,
) -> Option<Vec<Value>> {
    let guide = guidance(result, reference)?;
    if let Some(offer) = guide.offers.first() {
        let mut next = plan_next(session, &offer.plan_request, &offer.reason, &offer.evidence);
        next["original_step_id"] = json!(offer.order_step_id);
        next["clock_scope"] = json!(offer.scope);
        next["clock_proposal_state"] = json!(offer.state);
        return Some(vec![next]);
    }
    let blocker = guide.blockers.first()?;
    let mut arguments = json!({"session_id":session.to_string()});
    let tool = match &blocker.next_action {
        NextAction::Wait => {
            arguments["scope"] = json!("goal");
            "fortress_wait"
        }
        NextAction::Inspect => {
            if result["child"]["portfolio"]["schema"] == "dfmcp.production-portfolio/1"
                && let Some(step) = blocker.order_step_id
            {
                arguments["order_step_id"] = json!(step);
                "fortress_query"
            } else {
                "fortress_explain"
            }
        }
        NextAction::Operator => "fortress_explain",
        NextAction::None => return Some(Vec::new()),
    };
    let mut next = recommendation(
        "resolve_original_production_clock",
        tool,
        &blocker.message,
        "high",
        "high",
        "read_only",
        "not_applicable",
        false,
        arguments,
    );
    next["blocker_code"] = json!(blocker.code);
    next["clock_scope"] = json!(blocker.scope);
    next["operator_action_required"] = json!(matches!(&blocker.next_action, NextAction::Operator));
    next["grants_authority"] = json!(false);
    next["confidence"]["evidence"] = json!(evidence(&blocker.evidence));
    Some(vec![next])
}

pub(super) fn summary(result: &Value, reference: &Value) -> Value {
    let Some(guide) = guidance(result, reference) else {
        return Value::Null;
    };
    json!({
        "original_plan_digest":guide.original_plan_digest,
        "selected_order_step_id":guide.selected_order_step_id,
        "source_anchor":guide.anchor,
        "plan_offers":guide.offers.len(),
        "blockers":guide.blockers.iter().map(|blocker| json!({
            "scope":blocker.scope,
            "code":blocker.code,
            "message":blocker.message,
            "next_action":next_action_name(&blocker.next_action),
            "order_step_id":blocker.order_step_id,
            "evidence":evidence(&blocker.evidence),
        })).collect::<Vec<_>>(),
        "grants_authority":false,
        "restored_from_recovery":false,
        "complete_evidence_pointer":"/result/child/clock_guidance",
    })
}

pub(super) fn affordances(result: &Value, reference: &Value) -> Vec<Value> {
    if super::review(result).is_some() {
        return Vec::new();
    }
    let Some(child) = current_child(result, reference) else {
        return Vec::new();
    };
    let mut offers = Vec::new();
    if let Some((step, request)) = original_prepared_review_request(child, true) {
        offers.push(json!({
            "intent":"review_prepared_original_step",
            "original_step_id":step,
            "request":request,
            "risk":"guarded",
            "prepares_review_only":true,
            "native_preparation_repeated":false,
            "separate_commit_confirmation_required":true,
            "grants_authority":false,
            "complete_evidence_pointer":"/result/child/frontier",
        }));
    }
    if let Some(guide) = clock_guidance::from_view(child) {
        offers.extend(guide.offers.iter().map(|offer| json!({
            "intent":"review_original_production_clock",
            "clock_scope":offer.scope,
            "original_step_id":offer.order_step_id,
            "state":offer.state,
            "request":offer.plan_request,
            "risk":"guarded",
            "prepares_review_only":true,
            "native_time_advanced_by_planning":0,
            "separate_commit_confirmation_required":true,
            "grants_authority":false,
            "evidence":evidence(&offer.evidence),
            "complete_evidence_pointer":"/result/child/clock_guidance",
        })));
    }
    offers
}
