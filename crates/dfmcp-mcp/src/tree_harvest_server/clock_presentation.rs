//! Bounded Agent Turn guidance for the original Tree's separately reviewed clock.
use super::*;

fn summary(clock: &Value, verified: bool, commit_available: bool) -> Value {
    if !clock.is_object() {
        return Value::Null;
    }
    let mut out = json!({});
    for key in [
        "enabled",
        "mode",
        "target_id",
        "original_plan_digest",
        "recovery",
        "deadline_tick",
        "pending",
        "unresolved",
        "run_count",
        "omitted_run_count",
        "advancement_fenced",
        "remaining_tree_samples",
        "remaining_horizon_ticks",
        "high_tick",
        "consumed_tick_floor",
    ] {
        if let Some(value) = clock.get(key) {
            out[key] = value.clone();
        }
    }
    if let Some(run) = clock.get("active_run").filter(|run| run.is_object()) {
        let mut active = json!({});
        for key in [
            "key",
            "plan_digest",
            "state",
            "native_phase",
            "game_ticks",
            "wall_millis",
            "start_tick",
            "observed_tick",
            "native_unpause_attempted",
            "stopped_with_pause",
            "unresolved",
        ] {
            if let Some(value) = run.get(key) {
                active[key] = value.clone();
            }
        }
        active["review_seal"] = if commit_available {
            run["review_seal"].clone()
        } else {
            Value::Null
        };
        out["active_run"] = active;
    }
    out["verified_for_current_call"] = json!(verified);
    out["commit_available"] = json!(commit_available);
    out["new_run_available"] = json!(verified && clock["new_run_available"] == true);
    out["current_pause_proven"] = json!(false);
    out["harvest_goal_proven"] = json!(false);
    out["automatic_redispatch"] = json!(false);
    out
}

pub(super) fn augment(
    turn: &mut Value,
    result: &Value,
    reference: &Value,
    session: Option<SessionId>,
) {
    let verified = result["ok"] == true
        && result["retained_state_currently_verified"] == true
        && result["clock"]["verified"] == true;
    let clock = if verified {
        &result["clock"]
    } else {
        &reference["clock"]
    };
    if !clock.is_object() {
        return;
    }
    let run_state = clock["active_run"]["state"].as_str();
    let unsettled = clock["unresolved"] == true || clock["pending"] == true;
    let unattempted = matches!(run_state, Some("intent_recorded" | "prepared"));
    let reconcilable = matches!(
        run_state,
        Some("dispatch_started" | "tracking" | "cancel_requested")
    );
    let commit_available = verified
        && result["mode"] == "control"
        && clock["commit_available"] == true
        && clock["pending"] == true
        && clock["unresolved"] == false
        && clock["advancement_fenced"] != true
        && run_state == Some("prepared")
        && clock["active_run"]["review_seal"]
            .as_str()
            .is_some_and(|seal| !seal.is_empty());
    let retained = summary(clock, verified, commit_available);
    turn["briefing"]["clock"] = retained.clone();
    turn["active_work"]["clock"] = retained.clone();
    // A run receipt is historical time exposure. The Tree owner supplies the
    // separate fresh root/job/WOOD sample before claiming the harvest goal.
    // A configured clock can have exposed time even when its reply or this
    // publication failed. Retained receipt fields describe the known event;
    // neither an error nor missing current evidence establishes zero exposure.
    turn["briefing"]["clock_advanced"] = Value::Null;
    if clock["unresolved"] == true
        && let Some(hazards) = turn["active_work"]["indeterminate_effects"].as_array_mut()
    {
        hazards.push(json!({"scope":"clock", "original_tree_clock":retained}));
    }
    if let Some(uncertainty) = turn["uncertainty"].as_array_mut() {
        uncertainty.push(json!({"uncertainty_id":"tree_clock_exposure", "epistemic_state":"unknown",
            "statement":"A clock interval exposes the fortress to ordinary game evolution. Its receipt does not establish a felled tree, new usable wood, or causal provenance. Preserve both Tree and clock recovery references."}));
    }
    let Some(session) = session.filter(|_| result["session_closed"] != true) else {
        return;
    };
    if !verified || result["mode"] == "offline" {
        return;
    }
    let mut args = json!({"session_id":session.to_string()});
    let mut confirmation = false;
    let mut ticks = 0;
    let (id, tool, reason, risk) = if commit_available {
        args["scope"] = json!("clock");
        args["review_seal"] = clock["active_run"]["review_seal"].clone();
        args["confirmed"] = json!(true);
        confirmation = true;
        ticks = clock["active_run"]["game_ticks"].as_u64().unwrap_or(0);
        if let Some(confirmations) = turn["active_work"]["confirmations"].as_array_mut() {
            confirmations
                .push(json!({"scope":"clock", "review_seal":args["review_seal"], "required":true}));
        }
        if let Some(plans) = turn["active_work"]["pending_plans"].as_array_mut() {
            plans.push(json!({"scope":"clock", "target_id":clock["target_id"], "active_run":retained["active_run"]}));
        }
        (
            "confirm_original_tree_clock",
            "fortress_commit",
            "Review and confirm this exact bounded clock interval under the original Tree deadline.",
            "guarded",
        )
    } else if unsettled && run_state == Some("terminal") {
        (
            "inspect_terminal_tree_clock",
            "fortress_explain",
            "The retained clock has a terminal unresolved outcome. Inspect its original source-loss evidence; another wait or safety-stop request cannot restore continuity or prove a pause.",
            "read_only",
        )
    } else if unsettled
        && unattempted
        && result["mode"] == "control"
        && clock["cancel_available"] == true
    {
        args["scope"] = json!("clock");
        (
            "retire_unattempted_tree_clock",
            "fortress_cancel",
            "Retire the retained unattempted clock interval under current clock authority. Its earlier review cannot be restored; local retirement does not unpause the game.",
            "guarded",
        )
    } else if unsettled && unattempted {
        (
            "inspect_unattempted_tree_clock",
            "fortress_explain",
            "The original clock interval was never dispatched and has no current review or retirement authority. Preserve its custody; recovery cannot restore dispatch permission and repeated waits cannot commit it.",
            "read_only",
        )
    } else if unsettled && reconcilable {
        args["scope"] = json!("clock");
        (
            "reconcile_original_tree_clock",
            "fortress_wait",
            "Reconcile the original clock key once; after it settles, obtain a fresh original Tree progress sample.",
            "read_only",
        )
    } else if unsettled {
        (
            "inspect_blocked_tree_clock",
            "fortress_explain",
            "The retained clock is blocked without a reconcilable active interval. Inspect its original source and custody; repeated waits cannot clear this blocker or authorize another run.",
            "read_only",
        )
    } else if result["mode"] == "control"
        && clock["new_run_available"] == true
        && clock["clock_ready"] == true
        && clock["advancement_fenced"] != true
        && result["original"]["historical_supply_observed_tick"].is_null()
        && let (Some(target), Some(game_ticks)) = (
            clock["target_id"].as_str(),
            clock["recommended_game_ticks"].as_u64().filter(|ticks| {
                *ticks > 0
                    && *ticks <= 1200
                    && *ticks <= result["max_game_ticks"].as_u64().unwrap_or(0)
                    && *ticks <= clock["remaining_horizon_ticks"].as_u64().unwrap_or(0)
                    && clock["remaining_tree_samples"].as_u64().unwrap_or(0) >= 3
            }),
        )
    {
        args["request"] = json!(
            json!({"kind":"run_tree", "target_id":target,
            "game_ticks":game_ticks, "wall_millis":1000})
            .to_string()
        );
        (
            "prepare_original_tree_clock",
            "fortress_plan",
            "Prepare one separately reviewed interval for the unfinished original Tree goal; retain its deadline and remaining sample capacity.",
            "read_only",
        )
    } else if result["original"]["native_phase"] == 2
        && (clock["advancement_fenced"] == true
            || clock["remaining_tree_samples"]
                .as_u64()
                .is_some_and(|n| n < 3)
            || clock["remaining_horizon_ticks"] == 0
            || (clock["readiness"]["root_predicate"] == 2
                && clock["readiness"]["nearby_felling_jobs"] == 0
                && clock["readiness"]["wood_rows"] == 0))
    {
        (
            "inspect_original_tree_clock_blocker",
            "fortress_explain",
            "Inspect the original source, supply and remaining horizon/sample limits before another clock interval.",
            "read_only",
        )
    } else {
        return;
    };
    let mut next = recommendation(
        id,
        tool,
        reason,
        "high",
        "high",
        risk,
        if confirmation {
            "ordinary world evolution; no automatic undo"
        } else {
            "not_applicable"
        },
        confirmation,
        args,
    );
    next["estimated_cost"] = json!({"game_ticks":ticks,
        "native_writer_calls_max":u8::from(confirmation), "output_bytes_max":RESPONSE_BYTES});
    next["grants_authority"] = json!(false);
    next["confidence"]["evidence"] = json!([reference.clone()]);
    turn["recommendations"] = json!([next]);
    if reconcilable
        && clock["unresolved"] == true
        && result["mode"] == "control"
        && clock["cancel_available"] == true
    {
        let mut stop = recommendation(
            "stop_original_tree_clock",
            "fortress_cancel",
            "Request the independently authorized safety pause for the retained clock; preserve the original Tree and both recovery references.",
            "high",
            "high",
            "guarded",
            "stops future exposure; does not undo elapsed time",
            false,
            json!({"session_id":session.to_string(), "scope":"clock"}),
        );
        stop["grants_authority"] = json!(false);
        stop["estimated_cost"] = json!({"game_ticks":0, "native_writer_calls_max":1,
            "output_bytes_max":RESPONSE_BYTES});
        if let Some(recommendations) = turn["recommendations"].as_array_mut() {
            recommendations.push(stop);
        }
    }
}

#[cfg(test)]
#[path = "clock_presentation_tests.rs"]
mod tests;
