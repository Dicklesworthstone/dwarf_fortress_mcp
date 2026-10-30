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
    let mut result = json!({"plan_digest":digest(&r["plan_digest"])?,
        "object_digest":digest(&r["object_digest"])?,"anchor":anchor(&r["anchor"])?,
        "history_present":r["history_present"].as_bool()?});
    if let Some(id) = digest(&r["clock_journal_id"]) {
        result["clock_journal_id"] = json!(id);
    }
    Some(result)
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
fn run_state(value: &Value) -> Option<&str> {
    value.as_str().filter(|s| {
        matches!(
            *s,
            "intent"
                | "prepared"
                | "dispatch_started"
                | "tracking"
                | "terminal"
                | "cancel_requested"
                | "cancelled_before_dispatch"
        )
    })
}
fn clock_run(value: &Value) -> Option<Value> {
    let key = value["key"].as_str().filter(|s| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    })?;
    let native = &value["native"];
    Some(
        json!({"review_seal":digest(&value["review_seal"])?,"key":key,
        "plan_digest":digest(&value["plan_digest"])?,"state":run_state(&value["state"]),
        "native_order_id":value["native_order_id"].as_u64(),"before_tick":value["before_tick"].as_u64(),
        "game_ticks":value["game_ticks"].as_u64().filter(|n|(1..=1200).contains(n)),
        "wall_millis":value["wall_millis"].as_u64().filter(|n|(1..=60000).contains(n)),
        "pending":value["pending"].as_bool(),"unresolved":value["unresolved"].as_bool(),
        "purpose":value["purpose"].as_str().filter(|s|matches!(*s,"advance_original_order"|"advance_for_original_stability_or_quiescence")),
        "stop_predicate":value["stop_predicate"].as_str().filter(|s|matches!(*s,"remaining_at_most"|"active")),
        "native":{"phase":native["phase"].as_str().filter(|s|matches!(*s,"prepared"|"running"|"stopping"|"stopped"|"refused"|"source_lost")),
            "receipt":digest(&native["receipt"]),"unpause_attempted":native["unpause_attempted"].as_bool(),
            "historical_pause_verified":native["historical_pause_verified"].as_bool(),
            "observed_tick":native["observed_tick"].as_u64(),"predicate_observed":native["predicate_observed"].as_bool()},
        "current_pause_proven":false,"original_goal_proven":false}),
    )
}
fn clock_identity(view: &Value) -> Option<Value> {
    let clock = &view["clock"];
    Some(
        json!({"journal_id":digest(&clock["journal_id"])?,"head":digest(&clock["head"])?,
        "mode":clock["mode"].as_str().filter(|s|matches!(*s,"control"|"reconcile"|"offline")),
        "target_digest":digest(&clock["target_digest"]),
        "original_paired_seal":digest(&clock["original_paired_seal"]),
        "run_count":clock["run_count"].as_u64().filter(|n|*n<=32),
        "pending":clock["pending"].as_bool(),"unresolved":clock["unresolved"].as_bool(),
        "may_be_running":clock["may_be_running"].as_bool(),
        "needs_reconciliation":clock["needs_reconciliation"].as_bool(),
        "high_tick":clock["high_tick"].as_u64(),"active_run":clock_run(&clock["active_run"]),
        "current_pause_proven":false,"original_goal_proven":false}),
    )
}
/// Only fixed-shape identities survive overflow. Unknown fields, free text and
/// current goal claims cannot inflate or acquire authority through the fallback.
fn compact(view: &Value) -> Value {
    let mut result = json!({"review":review(view),"recovery":recovery(view),
        "native_state":native_state(view),"anchor":anchor(&view["anchor"]),
        "original_plan":{"plan_digest":digest(&view["original_plan"]["plan_digest"]),
            "anchor":anchor(&view["original_plan"]["anchor"]),
            "steps":[{"obligation":obligation(view)}]},
        "insertion":{"verified":view["insertion"]["verified"].as_bool(),
            "native_order_id":view["insertion"]["native_order_id"].as_u64(),
            "receipt":digest(&view["insertion"]["receipt"])},
        "first_satisfied_anchor":anchor(&view["first_satisfied_anchor"]),
        "current_goal":null,"current_goal_available":false,"details_omitted":true});
    if let Some(clock) = clock_identity(view) {
        result["clock"] = clock;
    }
    result
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
fn next_clock(tool: &str, reason: &str, arguments: Value, confirm: bool) -> Value {
    recommendation(
        tool,
        tool,
        reason,
        "advance only the retained original order within its deadline",
        "review finite time exposure separately from insertion and original-goal proof",
        "guarded",
        "elapsed game time is not reversed by cancellation",
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
    let clock = clock_identity(view);
    let clock_active = clock
        .as_ref()
        .and_then(|v| v["active_run"].as_object().map(|_| &v["active_run"]));
    let clock_uncertain = clock
        .as_ref()
        .is_some_and(|v| v["unresolved"] == true || v["may_be_running"] == true)
        || clock_active.is_some_and(|v| {
            matches!(
                run_state(&v["state"]),
                Some("dispatch_started" | "tracking" | "cancel_requested")
            )
        });
    let uncertain = may_have_started
        || clock_uncertain
        || matches!(state, Some("dispatch_started" | "indeterminate"));
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
    let clock_authority = verified
        && c.mode == Some("control")
        && result["development_clock_enabled"] == true
        && clock.as_ref().is_some_and(|v| v["mode"] == "control");
    let deadline = obligation(view).and_then(|o| o["deadline_tick"].as_u64());
    let anchor_tick = view["anchor"]["tick"].as_u64();
    let known_tick = anchor_tick.map(|tick| {
        tick.max(
            clock
                .as_ref()
                .and_then(|v| v["high_tick"].as_u64())
                .unwrap_or(tick),
        )
    });
    let allowed_ticks = deadline
        .zip(known_tick)
        .and_then(|(end, tick)| end.checked_sub(tick))
        .zip(c.budget)
        .map(|(remaining, b)| remaining.min(b.max_game_ticks).min(1200))
        .filter(|n| *n > 0);
    // A completed order no longer has remaining work to stop a run early.
    // Preserve room for the original distinct stability samples instead of
    // recommending that its Active guard consume the whole remaining deadline.
    let proposed_ticks = if goal.is_some_and(|g| g["current_truth"] == "true") {
        allowed_ticks
            .zip(obligation(view).and_then(|o| o["poll_interval_ticks"].as_u64()))
            .map(|(ticks, cadence)| ticks.min(cadence))
            .filter(|n| *n > 0)
    } else {
        allowed_ticks
    };
    let clock_linked = clock.as_ref().is_some_and(|v| {
        digest(&v["target_digest"]).is_some()
            && digest(&v["original_paired_seal"]).is_some()
            && v["original_paired_seal"] == view["review"]["paired_seal"]
            && v["journal_id"] == view["recovery"]["clock_journal_id"]
    });
    let clock_commit = clock_authority
        && clock_linked
        && !uncertain
        && completion != Some(true)
        && state == Some("created")
        && view["insertion"]["verified"] == true
        && clock_active.is_some_and(|r| {
            r["state"] == "prepared"
                && r["before_tick"].as_u64() == anchor_tick
                && r["before_tick"].as_u64() == known_tick
                && r["game_ticks"]
                    .as_u64()
                    .zip(c.budget)
                    .is_some_and(|(ticks, b)| ticks <= b.max_game_ticks)
                && r["before_tick"]
                    .as_u64()
                    .zip(r["game_ticks"].as_u64())
                    .and_then(|(tick, ticks)| tick.checked_add(ticks))
                    .zip(deadline)
                    .is_some_and(|(end, deadline)| end <= deadline)
        });
    let run_proposal = clock_authority
        && clock_linked
        && !uncertain
        && completion != Some(true)
        && proposed_ticks.is_some()
        && state == Some("created")
        && view["insertion"]["verified"] == true
        && clock.as_ref().is_some_and(|v| {
            v["pending"] == false && v["run_count"].as_u64().is_some_and(|n| n < 32)
        })
        && goal.is_some_and(|g| {
            g["receipt_linked"] == true
                && matches!(g["current_truth"].as_str(), Some("true" | "false"))
        });
    let identity = json!({"review":paired,"recovery":recovery(view),
        "clock":clock,
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
    if verified && let Some(run) = clock_active {
        if run["state"] == "prepared" {
            if let Some(plans) = work["pending_plans"].as_array_mut() {
                plans.push(json!({"scope":"run","clock":clock,"original_review":review(view),"recovery":recovery(view)}));
            }
            if let Some(confirmations) = work["confirmations"].as_array_mut() {
                confirmations.push(json!({"scope":"run","paired_seal":run["review_seal"],
                        "requires_confirmation":true,"commit_available":clock_commit,
                        "purpose":"explicit bounded game-time exposure for the retained order"}));
            }
        }
        if let Some(actions) = work["actions"].as_array_mut() {
            actions.push(json!({"scope":"run","clock_run":run,
                    "meaning":"historical native clock coordination; original goal and current pause require separate observations"}));
        }
    }

    if uncertain || (!verified && retained.is_some()) {
        work["indeterminate_effects"] = json!([{"identity":identity,
            "effect_may_have_started":if uncertain {json!(true)} else {Value::Null},"automatic_redispatch":false,
            "clock_may_be_running":clock_uncertain,
            "reason":if verified {"insertion or clock outcome unresolved"} else {"retained custody not verified by this result"}}]);
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
        } else if uncertain {
            if c.mode != Some("offline") {
                recommendations.push(next("fortress.wait", "reconcile retained insertion and clock effects; fresh goal sampling waits until the clock owner is settled", args, false));
            }
        } else if clock_commit {
            recommendations.push(next_clock("fortress.commit","confirm the separate clock review; unpause is attempted once after original-custody revalidation",
                json!({"session_id":session.to_string(),"paired_seal":clock_active.map(|r|&r["review_seal"]),"confirmed":true,"scope":"run"}),true));
        } else if commit {
            recommendations.push(next("fortress.commit", "confirm the exact paired review; commit revalidates all authority and evidence",
                json!({"session_id":session.to_string(),"paired_seal":digest(&view["review"]["paired_seal"]),"confirmed":true}), true));
        } else if run_proposal {
            let request = json!({"kind":"run","original_paired_seal":digest(&view["review"]["paired_seal"]),
                "game_ticks":proposed_ticks,"wall_millis":c.budget.map(|b|b.max_wall_millis.min(5000)).unwrap_or(0)});
            recommendations.push(next_clock("fortress.plan","prepare a finite clock slice for the retained goal, then review its separate seal before unpausing",
                json!({"session_id":session.to_string(),"request":request.to_string()}),false));
        } else if state == Some("created") && completion != Some(true) && c.mode != Some("offline")
        {
            recommendations.push(next("fortress.wait","reconcile retained effects and obtain one fresh original-goal sample; wait does not start or extend a clock run",args,false));
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
    let mut affordances = vec![
        json!({"action":"fortress.plan","enabled":production && observed && state.is_none() && !uncertain,
            "purpose":if original {"resume_only_the_unchanged_original_request"} else {"prepare_one_original_goal"},
            "recipes":["wooden_bed","wooden_door","wooden_table","wooden_chair"],"amount_min":1,"amount_max":100,
            "requires":["operator production authority","exact observation seal","unchanged original request and deadline when recovering"],"grants_authority":false}),
        json!({"action":"fortress.commit","enabled":commit,"requires_confirmation":true,
            "requires":["exact paired seal","current authority and evidence revalidation"],"grants_authority":false}),
        json!({"action":"fortress.wait","enabled":verified && paired.is_some() && c.mode != Some("offline") && !closed,
            "game_ticks_consumed":0,"insertion_retry":false,"clock_run_started":false,"grants_authority":false}),
    ];
    if clock.is_some() {
        affordances.extend([
            json!({"action":"fortress.plan","scope":"run","enabled":run_proposal,"request_kind":"run",
                "game_ticks_max":allowed_ticks,"game_ticks_min":1,"wall_millis_max":60000,
                "requires":["current operator clock authority","negotiated nonzero game-tick budget","verified original insertion","original deadline remains open","fresh matching paused sources"],
                "native_preparation_is_unpause":false,"grants_authority":false}),
            json!({"action":"fortress.commit","scope":"run","enabled":clock_commit,"requires_confirmation":true,
                "paired_seal":clock_active.map(|r|&r["review_seal"]),
                "requires":["separate exact clock review","current clock authority","fresh original-custody revalidation"],"grants_authority":false}),
            json!({"action":"fortress.cancel","scope":"run","enabled":clock_authority && clock_linked && clock_active.is_some(),
                "purpose":"retire undispatched clock intent or request a native safety pause","original_goal_abandoned":false,
                "current_pause_proven":false,"grants_authority":false}),
        ]);
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
            "development_production_enabled":production,"development_clock_enabled":clock_authority,"mutation_admissible":false,
            "retained_state_currently_verified":verified,"current_freshness_proven":false,
            "evidence_scope":"as_of_anchor","as_of_anchor":anchor(&view["anchor"]),
            "insertion_verified_at_anchor":verified.then(||view["insertion"]["verified"].as_bool()).flatten(),
            "original_goal_proven_at_anchor":goal.and_then(|g|g["original_goal_proven"].as_bool()),
            "physical_quiescence_proven_at_anchor":goal.and_then(|g|g["physical_quiescence_proven"].as_bool()),
            "semantic_completion_proven_at_anchor":completion,
            "historical_first_satisfied_anchor":anchor(&view["first_satisfied_anchor"]),
            "clock_advanced":if clock.as_ref().is_some_and(|v|v["run_count"].as_u64().is_some_and(|n|n>0)) || clock_uncertain {Value::Null} else {json!(false)},
            "clock_may_be_running":clock_uncertain,"clock_receipt_is_goal_proof":false,"current_pause_proven":false,
            "automatic_redispatch":false}))
        .active_work(work).recommendations(recommendations)
        .attention(if uncertain || !verified && retained.is_some() {vec![json!({
            "attention_id":"retain_original_goal_custody","category":"recovery","severity":"high",
            "urgency":"before further control","epistemic_state":"indeterminate",
            "finding":"Resolve retained insertion and clock work before another dispatch.",
            "evidence":identity,"suggested_response":"fortress.query followed by exact reconciliation"})]} else {vec![]})
        .affordances(affordances)
        .uncertainty(vec![
            json!({"uncertainty_id":"development_only","epistemic_state":"unknown",
                "statement":"Operator source acceptance supplies no compatibility or production admission."}),
            json!({"uncertainty_id":"anchored_observations","epistemic_state":if verified {"observed"} else {"unknown"},
                "statement":"Evidence is valid only at its named anchor; query and recovery do not acquire a fresh fortress sample."}),
            json!({"uncertainty_id":"retained_custody","epistemic_state":if verified {"observed"} else {"unknown"},
                "statement":"Unavailable retained state does not prove absence of work. Never retry insertion or unpause from an uncertain result."}),
            json!({"uncertainty_id":"native_clock_receipts","epistemic_state":"unknown",
                "statement":"Native clock stop limits are callback triggers. Historical pause and sampled stop predicates prove neither current pause nor the original goal."})])
        .coverage(json!({"status":if verified {"partial"} else {"unavailable"},
            "complete_domains":if verified {vec!["this_runtime_retained_original_goal"]} else {vec![]},
            "partial_domains":["canonical_citizens","receipt_linked_native_order_progress","global_job_quiescence"],
            "omitted_domains":["item output quantities","material and workshop feasibility","automatic chaining of clock runs","other controllers","continuous goal truth"],
            "continuation":null}))
        .references(vec![json!({"kind":"original_goal_custody","review":review(view),"recovery":recovery(view),
            "clock":clock,"currently_verified":verified})]);
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
            "max_output_tokens":b.max_output_tokens,"max_game_ticks":b.max_game_ticks},"response_reserved_bytes":RESPONSE_RESERVE,
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
