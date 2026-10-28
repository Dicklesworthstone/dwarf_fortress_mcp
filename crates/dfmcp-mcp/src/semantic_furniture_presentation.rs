//! Bounded Agent Turn orientation for one exact original furniture goal.
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_core::{Digest32, RequestId, SessionId, StateAnchor, WorkBudget};
use serde_json::{Value, json};

pub(super) const RESPONSE_RESERVE: u64 = 128 * 1024;
fn digest(value: &Value) -> Option<&str> {
    value
        .as_str()
        .filter(|s| {
            s.len() == 64
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
        .filter(|s| s.bytes().any(|b| b != b'0'))
}
fn anchor(value: &Value) -> Option<Value> {
    let fortress = value["fortress_id"].as_str()?;
    let id = fortress.parse::<u64>().ok()?;
    if id == 0 || id.to_string() != fortress {
        return None;
    }
    Some(
        json!({"fortress_id":fortress,"epoch":value["epoch"].as_u64()?,
        "sequence":value["sequence"].as_u64()?,"tick":value["tick"].as_u64()?,
        "state_hash":digest(&value["state_hash"])?}),
    )
}
fn review(value: &Value) -> Option<Value> {
    let r = &value["review"];
    Some(json!({"paired_seal":digest(&r["paired_seal"])?,
        "native_plan_digest":digest(&r["native_plan_digest"])?,
        "semantic_plan_digest":digest(&r["semantic_plan_digest"])?,
        "original_object_digest":digest(&r["original_object_digest"])?}))
}
fn recovery(value: &Value) -> Option<Value> {
    let r = &value["recovery"];
    let mut out = json!({"journal_id":digest(&r["journal_id"])?,"plan_digest":digest(&r["plan_digest"])?,
        "object_digest":digest(&r["object_digest"])?,"anchor":anchor(&r["anchor"])?,
        "history_present":r["history_present"].as_bool()?});
    // Optional diagnostic flags must remain absent when unavailable. JSON null
    // is not a valid defaultable boolean in the exact operator recovery input.
    for field in ["archive_publication_unknown", "paired_preparation_present"] {
        if let Some(flag) = r[field].as_bool() {
            out[field] = json!(flag);
        }
    }
    Some(out)
}
fn native_state(value: &Value) -> Option<&str> {
    value["native_state"].as_str().filter(|s| {
        matches!(
            *s,
            "intent"
                | "prepared"
                | "dispatch_started"
                | "tracking"
                | "cancel_requested"
                | "terminal"
        )
    })
}
fn native_phase(value: &Value) -> Option<&str> {
    value["native_phase"].as_str().filter(|s| {
        matches!(
            *s,
            "prepared" | "indeterminate" | "placed" | "refused" | "cancelled"
        )
    })
}
fn obligation(value: &Value) -> Option<Value> {
    let o = &value["original_plan"]["steps"].as_array()?.first()?["obligation"];
    Some(json!({"deadline_tick":o["deadline_tick"].as_u64()?,
        "poll_interval_ticks":o["poll_interval_ticks"].as_u64()?,
        "stable_samples":o["stable_samples"].as_u64()?}))
}
fn compact(value: &Value) -> Value {
    json!({"journal_id":digest(&value["journal_id"]),"anchor":anchor(&value["anchor"]),
        "review":review(value),"recovery":recovery(value),
        "native_state":native_state(value),"native_phase":native_phase(value),
        "native_needs_query":value["native_needs_query"].as_bool(),
        "effect_uncertain":value["effect_uncertain"].as_bool(),
        "prepared_in_this_opening":value["prepared_in_this_opening"].as_bool(),
        "original_plan":{"plan_digest":digest(&value["original_plan"]["plan_digest"]),
            "anchor":anchor(&value["original_plan"]["anchor"]),
            "steps":[{"obligation":obligation(value)}]},
        "placement":{"verified":value["placement"]["verified"].as_bool(),
            "receipt":digest(&value["placement"]["receipt"]),
            "native_building_id":value["placement"]["native_building_id"].as_u64(),
            "native_job_id":value["placement"]["native_job_id"].as_u64(),
            "native_item_id":value["placement"]["native_item_id"].as_u64()},
        "current_goal":null,"current_goal_available":false,
        "first_satisfied_anchor":anchor(&value["first_satisfied_anchor"]),
        "history":{"fenced":value["history"]["fenced"].as_bool()},
        "retained_state_currently_verified":false,"runtime_admitted":false,"automatic_redispatch":false})
}
fn next(id: &str, tool: &str, reason: &str, confirmation: bool, arguments: Value) -> Value {
    recommendation(
        id,
        tool,
        reason,
        "high",
        "high",
        if confirmation { "guarded" } else { "read_only" },
        if confirmation {
            "native placement cannot be undone by this runtime"
        } else {
            "not_applicable"
        },
        confirmation,
        arguments,
    )
}
#[allow(clippy::too_many_arguments)]
fn render(
    operation: &str,
    session: Option<SessionId>,
    at: Option<StateAnchor>,
    mode: Option<&str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    result: Value,
    retained: Option<&Value>,
    effect_may_have_started: bool,
) -> String {
    let verified = result["ok"].as_bool() == Some(true)
        && result["retained_state_currently_verified"].as_bool() == Some(true);
    let view = if verified {
        &result
    } else {
        retained.map_or(&result, |v| v)
    };
    let closed = result["session_closed"].as_bool() == Some(true);
    let uncertain = effect_may_have_started
        || result["placement_may_have_started"].as_bool() == Some(true)
        || view["effect_uncertain"].as_bool() == Some(true);
    let paired = review(view);
    let prepared = verified
        && !uncertain
        && !closed
        && mode == Some("control")
        && view["development_placement_enabled"].as_bool() == Some(true)
        && view["prepared_in_this_opening"].as_bool() == Some(true)
        && native_state(view) == Some("prepared")
        && native_phase(view) == Some("prepared")
        && paired.is_some();
    let placed = verified
        && view["placement"]["verified"].as_bool() == Some(true)
        && native_phase(view) == Some("placed");
    let goal = if verified
        && view["history"]["fenced"].as_bool() != Some(true)
        && view["current_goal_available"].as_bool() == Some(true)
    {
        view.get("current_goal").filter(|g| g.is_object())
    } else {
        None
    };
    let complete =
        placed && goal.is_some_and(|g| g["semantic_completion_proven"].as_bool() == Some(true));
    let original = digest(&view["original_plan"]["plan_digest"]);
    let mut recommendations = Vec::new();
    if !closed {
        if !verified {
            if session.is_some() {
                recommendations.push(next(
                    "verify_original_custody",
                    "fortress.query",
                    "Verify the retained original identities before any further native operation.",
                    false,
                    json!({"session_id":session.map(|s|s.to_string())}),
                ));
            }
        } else if view["history"]["fenced"].as_bool() == Some(true) {
            recommendations.push(next(
                "release_fenced_goal_owner",
                "fortress.cancel",
                "Construction history is fenced. Preserve the exact original recovery handoff and release this local session; another wait cannot restore this proof owner.",
                false,
                json!({"session_id":session.map(|s|s.to_string()),"scope":"session"}),
            ));
        } else if !uncertain
            && !prepared
            && mode != Some("offline")
            && native_state(view) == Some("tracking")
            && native_phase(view) == Some("prepared")
        {
            recommendations.push(next(
                "retire_recovered_preparation",
                "fortress.cancel",
                "The original native query confirmed a remaining preparation. Retire it explicitly; recovery cannot restore its placement permission.",
                false,
                json!({"session_id":session.map(|s|s.to_string()),"scope":"goal"}),
            ));
        } else if uncertain || (view["native_needs_query"].as_bool() == Some(true) && !prepared) {
            if mode != Some("offline") {
                recommendations.push(next("reconcile_original_placement","fortress.wait",
                    "Query the original native key; retained uncertainty never permits another placement.",false,
                    json!({"session_id":session.map(|s|s.to_string())})));
            }
        } else if prepared {
            recommendations.push(next("confirm_original_furniture","fortress.commit",
                "Review the exact original goal and paired native selection before the single placement attempt.",true,
                json!({"session_id":session.map(|s|s.to_string()),
                    "paired_seal":paired.as_ref().and_then(|v|v["paired_seal"].as_str()),"confirmed":true})));
        } else if placed && !complete && mode != Some("offline") {
            recommendations.push(next("observe_original_construction","fortress.wait",
                "Acquire one bounded receipt-linked construction sample at the original cadence; this call does not advance game time.",false,
                json!({"session_id":session.map(|s|s.to_string())})));
        } else if original.is_none() && view["observation"].is_null() && mode != Some("offline") {
            recommendations.push(next(
                "observe_furniture_basis",
                "fortress.observe",
                "Acquire the canonical spatial source before choosing a furniture goal.",
                false,
                json!({"session_id":session.map(|s|s.to_string())}),
            ));
        }
    }
    let phase = if uncertain {
        AgentPhase::Reconcile
    } else {
        match operation {
            "fortress.open_session" => AgentPhase::Bootstrap,
            "fortress.observe" => AgentPhase::Orient,
            "fortress.plan" => AgentPhase::Propose,
            "fortress.commit" => AgentPhase::Commit,
            "fortress.wait" => AgentPhase::Verify,
            "fortress.cancel" => AgentPhase::Reconcile,
            _ => AgentPhase::Inspect,
        }
    };
    let continuity = if uncertain {
        ContinuityStatus::Indeterminate
    } else if !verified && retained.is_some() {
        ContinuityStatus::Stale
    } else if operation == "fortress.open_session" {
        ContinuityStatus::Bootstrap
    } else {
        ContinuityStatus::Partial
    };
    let identity = json!({"journal_id":digest(&view["journal_id"]),"review":paired,
        "recovery":recovery(view),"currently_verified":verified});
    let action = if native_state(view).is_some() {
        vec![json!({
        "kind":"original_furniture_placement","state":native_state(view),"native_phase":native_phase(view),
        "evidence":identity,"placement_verified_at_anchor":if verified {view["placement"]["verified"].clone()}else{Value::Null}})]
    } else {
        vec![]
    };
    let obligations = if original.is_some() {
        vec![json!({"kind":"original_construction",
        "original_plan_digest":original,"original_obligation":obligation(view),
        "current_goal":goal,"historical_first_satisfied_anchor":anchor(&view["first_satisfied_anchor"]),
        "complete_at_anchor":goal.map(|_|complete),"currently_verified":verified&&goal.is_some(),
        "original_custody_currently_verified":verified})]
    } else {
        vec![]
    };
    let confirmations = if prepared {
        vec![json!({"kind":"paired_original_native_furniture",
        "paired_seal":paired.as_ref().and_then(|v|v["paired_seal"].as_str()),
        "required":true,"authority":"separate_current_native_checks"})]
    } else {
        vec![]
    };
    let affordances = vec![json!({"kind":"original_single_furniture_build",
        "enabled":verified&&!closed&&mode==Some("control")&&original.is_none()
            &&view["development_placement_enabled"].as_bool()==Some(true)&&!view["observation"].is_null(),
        "tool":"fortress.plan","required_capabilities":["plan","construct"],"risk":"guarded",
        "parameters":{"expected_observation":"exact observation seal","kind":["bed","chair","table"],
            "item":"canonical item entity ID","target":"one [x,y,z] tile",
            "deadline_tick":"absolute original deadline","poll_interval_ticks":"positive cadence",
            "stable_samples":"1..64"},
        "material_selector":"default only","confirmation":"exact paired review seal",
        "checkpoint_policy":"operator-owned; no tool-supplied exception",
        "commit_requires":"fresh independent canonical source, exact native capture, durable marker, lease and current policy"})];
    let mut turn=AgentTurnBuilder::new(operation,phase)
        .continuity(continuity,anchor(&view["anchor"]),None,None)
        .briefing(json!({"runtime_admitted":false,"source_acceptance_is_compatibility_admission":false,
            "mode":mode,"session_closed":closed,"mutation_admissible":false,
            "retained_state_currently_verified":verified,"current_freshness_proven":false,
            "evidence_scope":"as_of_anchor","as_of_anchor":anchor(&view["anchor"]),
            "placement_verified_at_anchor":if verified {view["placement"]["verified"].clone()}else{Value::Null},
            "original_goal_proven_at_anchor":goal.and_then(|g|g["original_goal_proven"].as_bool()),
            "semantic_completion_proven_at_anchor":if goal.is_some(){json!(complete)}else{Value::Null},
            "placement_receipt_is_goal_proof":false,"clock_advanced":false,"automatic_redispatch":false}))
        .active_work(json!({"pending_plans":if prepared {vec![identity.clone()]}else{vec![]},
            "actions":action,"obligations":obligations,
            "cancellation_drains":if native_state(view)==Some("cancel_requested"){vec![identity.clone()]}else{vec![]},
            "indeterminate_effects":if uncertain{vec![identity.clone()]}else{vec![]},
            "publications":[],"confirmations":confirmations}))
        .recommendations(recommendations).affordances(affordances)
        .attention(if uncertain{vec![json!({"attention_id":"recover_original_placement","category":"recovery",
            "severity":"high","urgency":"before further control","epistemic_state":"indeterminate",
            "finding":"The original placement outcome needs reconciliation.","evidence":identity,
            "suggested_response":"fortress.wait on the retained original goal"})]}else{vec![]})
        .uncertainty(vec![
            json!({"uncertainty_id":"development_source","epistemic_state":"unknown",
                "statement":"Operator source acceptance supplies no compatibility or production admission."}),
            json!({"uncertainty_id":"placement_and_completion","epistemic_state":"unknown",
                "statement":"A Placed receipt proves historical construction-job registration. Original goal proof requires fresh receipt-linked evidence and the original stability requirement."}),
            json!({"uncertainty_id":"anchored_truth","epistemic_state":"unknown",
                "statement":"Inspection and historical recovery do not prove present fortress state or continuous construction truth."}),
            json!({"uncertainty_id":"clock","epistemic_state":"unknown",
                "statement":"This runtime does not advance game time. Repeated samples at one tick cannot manufacture temporal stability."})])
        .coverage(json!({"status":if verified{"partial"}else{"unavailable"},
            "complete_domains":if verified{vec!["this_runtime_original_goal_custody"]}else{vec![]},
            "partial_domains":["canonical spatial item selection","receipt-linked selected construction"],
            "omitted_domains":["other controllers","multi-step furnishing","clock control","checkpoint and restore","continuous goal truth"],
            "continuation":if verified{view["observation"]["next_query"].clone()}else{Value::Null}}))
        .references(vec![identity]);
    if let Some(id) = session {
        turn = turn.session_id(id.to_string());
    }
    if let Some(id) = request {
        turn = turn.request_id(id.to_string());
    }
    if let Some(at) = at.filter(|a| a.state_hash != Digest32::ZERO) {
        turn=turn.anchor(json!({"fortress_id":at.fortress_id.get().to_string(),"epoch":at.cursor.epoch,
            "sequence":at.cursor.sequence,"tick":at.tick.get(),"state_hash":at.state_hash.to_hex()}));
    }
    if let Some(b) = budget {
        turn = turn.budget(
            json!({"admitted":{"max_bytes":b.max_bytes,"max_wall_millis":b.max_wall_millis,
            "max_output_tokens":b.max_output_tokens,"max_game_ticks":b.max_game_ticks},
            "response_reserved_bytes":RESPONSE_RESERVE,"consumed":{},"remaining":null,
            "token_accounting":"four_byte_proxy_not_tokenizer_count"}),
        );
    }
    turn.attach(
        json!({"result":result,"retained_reference":retained,"retained_reference_current":false}),
    )
}
#[allow(clippy::too_many_arguments)]
pub(super) fn packet(
    operation: &str,
    session: Option<SessionId>,
    anchor: Option<StateAnchor>,
    mode: Option<&str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    result: Value,
    retained: Option<Value>,
    effect_may_have_started: bool,
) -> String {
    let operation = match operation {
        "fortress.open_session"
        | "fortress.observe"
        | "fortress.query"
        | "fortress.plan"
        | "fortress.commit"
        | "fortress.wait"
        | "fortress.explain"
        | "fortress.cancel"
        | "fortress.checkpoint"
        | "fortress.restore"
        | "fortress.doctor" => operation,
        _ => "fortress.unknown",
    };
    let mode = mode.filter(|m| matches!(*m, "control" | "reconcile" | "offline"));
    let rendered = render(
        operation,
        session,
        anchor,
        mode,
        request,
        budget,
        result.clone(),
        retained.as_ref(),
        effect_may_have_started,
    );
    if rendered.len() as u64 <= RESPONSE_RESERVE {
        return rendered;
    }
    let candidate = if result["retained_state_currently_verified"].as_bool() == Some(true) {
        Some(compact(&result))
    } else {
        retained.as_ref().map(compact)
    };
    let uncertain =
        effect_may_have_started || matches!(operation, "fortress.commit" | "fortress.cancel");
    let closed = result["session_closed"].as_bool() == Some(true);
    render(
        operation,
        session,
        anchor,
        mode,
        request,
        budget,
        json!({"ok":false,"error":{"code":"budget_exceeded","retryable":false,
            "message":"Complete response exceeds its reserved output; exact recovery identities are retained."},
            "placement_may_have_started":uncertain,"retained_state_currently_verified":false,
            "session_closed":closed,
            "current_goal_available":false,"original_goal_proven":false,"automatic_redispatch":false}),
        candidate.as_ref(),
        uncertain,
    )
}

#[cfg(test)]
#[path = "semantic_furniture_presentation_tests.rs"]
mod tests;
