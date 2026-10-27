//! Pure, bounded Agent Turn projection for one original semantic order goal.
//! Retained state is evidence at its anchor, never authority or live freshness.
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_core::{Digest32, RequestId, SessionId, StateAnchor, WorkBudget};
use serde_json::{Value, json};

pub(super) const RESPONSE_RESERVE: u64 = 128 * 1024;

fn anchor_value(a: StateAnchor) -> Value {
    json!({"fortress_id":a.fortress_id.get().to_string(),"epoch":a.cursor.epoch,
        "sequence":a.cursor.sequence,"tick":a.tick.get(),"state_hash":a.state_hash.to_hex()})
}
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
fn recovery(view: &Value) -> Option<Value> {
    let r = &view["recovery"];
    Some(json!({"plan_digest":digest(&r["plan_digest"])?,
        "object_digest":digest(&r["object_digest"])?,"anchor":anchor(&r["anchor"])?,
        "history_present":r["history_present"].as_bool()?}))
}
fn review(view: &Value) -> Option<Value> {
    let r = &view["review"];
    Some(json!({"paired_seal":digest(&r["paired_seal"])?,
        "native_plan_digest":digest(&r["native_plan_digest"])?,
        "semantic_plan_digest":digest(&r["semantic_plan_digest"])?}))
}
fn native_state(view: &Value) -> Option<&str> {
    view["native_state"].as_str().filter(|s| {
        matches!(
            *s,
            "prepared"
                | "dispatch_started"
                | "indeterminate"
                | "created"
                | "refused"
                | "cancelled_before_dispatch"
        )
    })
}
fn obligation(view: &Value) -> Option<Value> {
    let o = &view["original_plan"]["steps"].as_array()?.first()?["obligation"];
    Some(json!({"deadline_tick":o["deadline_tick"].as_u64()?,
        "poll_interval_ticks":o["poll_interval_ticks"].as_u64()?,
        "stable_samples":o["stable_samples"].as_u64()?}))
}
/// Only fixed-shape identities survive overflow. Unknown fields, free text and
/// current goal claims cannot inflate or acquire authority through the fallback.
fn compact(view: &Value) -> Value {
    json!({"review":review(view),"recovery":recovery(view),
        "native_state":native_state(view),"anchor":anchor(&view["anchor"]),
        "original_plan":{"plan_digest":digest(&view["original_plan"]["plan_digest"]),
            "anchor":anchor(&view["original_plan"]["anchor"]),
            "steps":[{"obligation":obligation(view)}]},
        "insertion":{"verified":view["insertion"]["verified"].as_bool(),
            "native_order_id":view["insertion"]["native_order_id"].as_u64(),
            "receipt":digest(&view["insertion"]["receipt"])},
        "first_satisfied_anchor":anchor(&view["first_satisfied_anchor"]),
        "current_goal":null,"current_goal_available":false,"details_omitted":true})
}

struct Context<'a> {
    operation: &'a str,
    session: Option<SessionId>,
    anchor: Option<StateAnchor>,
    mode: Option<&'a str>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
}
fn next(tool: &str, reason: &str, arguments: Value, confirm: bool) -> Value {
    recommendation(
        tool,
        tool,
        reason,
        "preserve the original goal and exact custody",
        "resolve the next protocol boundary",
        if confirm { "guarded" } else { "read_only" },
        if confirm {
            "native insertion is not automatically reversible"
        } else {
            "not_applicable"
        },
        confirm,
        arguments,
    )
}
fn finish(mut turn: Value, result: Value, retained: Option<&Value>, verified: bool) -> String {
    // The shared builder adds ambient production provenance. This isolated
    // development profile must also remove it from refusals and overflow paths.
    if let Some(briefing) = turn["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    json!({"agent_turn":turn,"result":result,"retained_state":retained,
        "retained_state_currently_verified":verified})
    .to_string()
}
fn render(
    c: &Context<'_>,
    result: Value,
    retained: Option<&Value>,
    may_have_started: bool,
) -> String {
    let empty = Value::Null;
    let view = retained.unwrap_or(&empty);
    let closed = result["closed"] == true;
    let verified = view.is_object()
        && view["ok"] == true
        && result["ok"] == true
        && !closed
        && result["retained_state_currently_verified"] != false;
    let state = native_state(view);
    let uncertain = may_have_started || matches!(state, Some("dispatch_started" | "indeterminate"));
    let production =
        verified && c.mode == Some("control") && result["development_production_enabled"] == true;
    let observed = digest(&view["observation"]["seal"]).is_some();
    let paired = review(view);
    let original = digest(&view["original_plan"]["plan_digest"]).is_some();
    let commit =
        production && observed && paired.is_some() && state == Some("prepared") && !uncertain;
    let goal = verified
        .then_some(&view["current_goal"])
        .filter(|v| v.is_object());
    let completion = goal.map(|g| {
        g["semantic_completion_proven"] == true
            && g["original_goal_proven"] == true
            && g["receipt_linked"] == true
            && g["physical_quiescence_proven"] == true
    });
    let identity = json!({"review":paired,"recovery":recovery(view),
        "last_known_native_state":state,"currently_verified":verified,
        "as_of_anchor":anchor(&view["anchor"])});
    let mut work = crate::agent_turn::empty_active_work();
    work["scope"] = json!("one_original_semantic_work_order_goal");
    work["state_known"] = json!(verified);
    work["retained_reference_available"] = json!(retained.is_some());
    work["retained_identity"] = identity.clone();
    if verified && state == Some("prepared") {
        work["pending_plans"] = json!([identity]);
        work["confirmations"] = json!([{"paired_seal":digest(&view["review"]["paired_seal"]),
            "requires_confirmation":true,"commit_available":commit}]);
    }
    if verified && state == Some("created") {
        work["actions"] = json!([{"state":"created","insertion":view["insertion"],
            "meaning":"order insertion receipt; goal and item production require separate evidence"}]);
    }
    if uncertain || (!verified && retained.is_some()) {
        work["indeterminate_effects"] = json!([{"identity":identity,
            "effect_may_have_started":if uncertain {json!(true)} else {Value::Null},"automatic_redispatch":false,
            "reason":if verified {"native outcome unresolved"} else {"retained custody not verified by this result"}}]);
    }
    if let Some(o) = obligation(view) {
        work["obligations"] = json!([{"original":o,"currently_verified":verified,
            "goal_as_of_anchor":goal,"first_satisfied_anchor":anchor(&view["first_satisfied_anchor"]),
            "historical_achievement_is_current_truth":false}]);
    }
    let mut recommendations = Vec::new();
    if let Some(session) = c.session.filter(|_| !closed) {
        let args = json!({"session_id":session.to_string()});
        if !verified {
            recommendations.push(next(
                "fortress.query",
                "verify retained custody before any further control",
                args.clone(),
                false,
            ));
            recommendations.push(next(
                "fortress.doctor",
                "diagnose unavailable custody or authority without dispatch",
                args,
                false,
            ));
        } else if uncertain || state == Some("created") && completion != Some(true) {
            if c.mode != Some("offline") {
                recommendations.push(next("fortress.wait", "perform one receipt reconciliation and original-goal sample; this does not advance the game clock", args, false));
            }
        } else if commit {
            recommendations.push(next("fortress.commit", "confirm the exact paired review; commit revalidates all authority and evidence",
                json!({"session_id":session.to_string(),"paired_seal":digest(&view["review"]["paired_seal"]),"confirmed":true}), true));
        } else if c.mode != Some("offline")
            && !observed
            && (state.is_none() || state == Some("prepared"))
        {
            recommendations.push(next(
                "fortress.observe",
                "acquire the exact canonical observation before planning or recovering preparation",
                args,
                false,
            ));
        }
    }
    let phase = match c.operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" if closed => AgentPhase::Handoff,
        _ => AgentPhase::Inspect,
    };
    let mut turn = AgentTurnBuilder::new(c.operation, phase)
        .continuity(if uncertain {ContinuityStatus::Indeterminate} else if verified {ContinuityStatus::Partial}
            else if retained.is_some() {ContinuityStatus::Stale} else {ContinuityStatus::Bootstrap}, None, None, None)
        .briefing(json!({"runtime":"unadmitted_semantic_work_order_development","runtime_admitted":false,
            "source_acceptance_is_compatibility_admission":false,"mode":c.mode,"session_closed":closed,
            "development_production_enabled":production,"mutation_admissible":false,
            "retained_state_currently_verified":verified,"current_freshness_proven":false,
            "evidence_scope":"as_of_anchor","as_of_anchor":anchor(&view["anchor"]),
            "insertion_verified_at_anchor":verified.then(||view["insertion"]["verified"].as_bool()).flatten(),
            "original_goal_proven_at_anchor":goal.and_then(|g|g["original_goal_proven"].as_bool()),
            "physical_quiescence_proven_at_anchor":goal.and_then(|g|g["physical_quiescence_proven"].as_bool()),
            "semantic_completion_proven_at_anchor":completion,
            "historical_first_satisfied_anchor":anchor(&view["first_satisfied_anchor"]),
            "clock_advanced":false,"automatic_redispatch":false}))
        .active_work(work).recommendations(recommendations)
        .attention(if uncertain || !verified && retained.is_some() {vec![json!({
            "attention_id":"retain_original_goal_custody","category":"recovery","severity":"high",
            "urgency":"before further control","epistemic_state":"indeterminate",
            "finding":"Resolve retained work before attempting another insertion.",
            "evidence":identity,"suggested_response":"fortress.query followed by exact reconciliation"})]} else {vec![]})
        .affordances(vec![
            json!({"action":"fortress.plan","enabled":production && observed && state.is_none() && !uncertain,
                "purpose":if original {"resume_only_the_unchanged_original_request"} else {"prepare_one_original_goal"},
                "recipes":["wooden_bed","wooden_door","wooden_table","wooden_chair"],"amount_min":1,"amount_max":100,
                "requires":["operator production authority","exact observation seal","unchanged original request and deadline when recovering"],"grants_authority":false}),
            json!({"action":"fortress.commit","enabled":commit,"requires_confirmation":true,
                "requires":["exact paired seal","current authority and evidence revalidation"],"grants_authority":false}),
            json!({"action":"fortress.wait","enabled":verified && paired.is_some() && c.mode != Some("offline") && !closed,
                "game_ticks_consumed":0,"insertion_retry":false,"grants_authority":false})])
        .uncertainty(vec![
            json!({"uncertainty_id":"development_only","epistemic_state":"unknown",
                "statement":"Operator source acceptance supplies no compatibility or production admission."}),
            json!({"uncertainty_id":"anchored_observations","epistemic_state":if verified {"observed"} else {"unknown"},
                "statement":"Evidence is valid only at its named anchor; query and recovery do not acquire a fresh fortress sample."}),
            json!({"uncertainty_id":"retained_custody","epistemic_state":if verified {"observed"} else {"unknown"},
                "statement":"Unavailable retained state does not prove absence of work. Never retry insertion from an uncertain result."})])
        .coverage(json!({"status":if verified {"partial"} else {"unavailable"},
            "complete_domains":if verified {vec!["this_runtime_retained_original_goal"]} else {vec![]},
            "partial_domains":["canonical_citizens","receipt_linked_native_order_progress","global_job_quiescence"],
            "omitted_domains":["item output quantities","material and workshop feasibility","automatic clock control","other controllers"],
            "continuation":null}))
        .references(vec![json!({"kind":"original_goal_custody","review":review(view),"recovery":recovery(view),
            "currently_verified":verified})]);
    if let Some(s) = c.session {
        turn = turn.session_id(s.to_string());
    }
    if let Some(r) = c.request {
        turn = turn.request_id(r.to_string());
    }
    if let Some(a) = c.anchor.filter(|a| a.state_hash != Digest32::ZERO) {
        turn = turn.anchor(anchor_value(a));
    }
    if let Some(b) = c.budget {
        turn = turn.budget(json!({"admitted":{"max_bytes":b.max_bytes,"max_wall_millis":b.max_wall_millis,
            "max_output_tokens":b.max_output_tokens},"response_reserved_bytes":RESPONSE_RESERVE,
            "consumed":{},"remaining":null,"token_accounting":"four_byte_proxy_not_tokenizer_count"}));
    }
    finish(turn.build(), result, retained, verified)
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
    let c = Context {
        operation: match operation {
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
        },
        session,
        anchor,
        mode: mode.filter(|m| matches!(*m, "control" | "reconcile" | "offline")),
        request,
        budget,
    };
    let out = render(
        &c,
        result.clone(),
        retained.as_ref(),
        effect_may_have_started,
    );
    if out.len() as u64 <= RESPONSE_RESERVE {
        return out;
    }
    let may_have_started = effect_may_have_started
        || matches!(
            c.operation,
            "fortress.plan" | "fortress.commit" | "fortress.wait" | "fortress.cancel"
        );
    let reduced = retained.as_ref().map(compact);
    render(
        &c,
        json!({"ok":false,"error":{"code":if may_have_started {"effect_indeterminate"} else {"budget_exceeded"},
        "message":"Detailed response exceeds its reservation; inspect retained evidence before further control."},
        "retained_operation_ok":result["ok"].as_bool(),"retained_state_currently_verified":false,
        "closed":result["closed"].as_bool().unwrap_or(false),
        "effects_cancelled":result["effects_cancelled"].as_bool(),
        "details_omitted":true,"runtime_admitted":false,"automatic_redispatch":false,
        "effect_may_have_started":may_have_started,"output_limit_bytes":RESPONSE_RESERVE}),
        reduced.as_ref(),
        may_have_started,
    )
}

#[cfg(test)]
#[path = "semantic_work_order_presentation_tests.rs"]
mod tests;
