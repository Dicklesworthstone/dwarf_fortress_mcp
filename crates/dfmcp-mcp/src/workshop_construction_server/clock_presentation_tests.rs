//! Behavioral projections for one construction goal and its separate run owner.
use super::*;

fn original() -> Value {
    json!({"key":"original-carpenter","plan_digest":"a".repeat(64),
        "review_seal":"b".repeat(64),"state":"tracking","native_phase":"placed",
        "prepared_in_this_connection":false,"dispatch_started":true,"unresolved":false,
        "placed_at_anchor":true,"historical_goal":"in_progress",
        "fresh_goal":"in_progress","fresh_completion_proven":false,
        "source":{"profile":"workshop/1.20","generation":4,"tick":100},
        "sample_tick":100,"high_tick":100,"high_generation":4})
}
fn clock() -> Value {
    json!({"enabled":true,"target_id":"workshop-clock-target:exact-original",
        "deadline_tick":150,"recommended_game_ticks":50,
        "recovery":{"journal_id":"c".repeat(64),"head":"d".repeat(64)},
        "verified":true,"pending":false,"unresolved":false,"active_run":null,
        "clock_ready":true,"new_run_available":true,"commit_available":false,
        "cancel_available":true})
}
fn run(state: &str, phase: &str) -> Value {
    json!({"key":"original-workshop-run-1","plan_digest":"e".repeat(64),
        "review_seal":"f".repeat(64),"state":state,"native_phase":phase,
        "game_ticks":50,"wall_millis":5000,"start_tick":100,
        "observed_tick":100,"stopped_with_pause":false})
}
fn ready() -> Value {
    json!({"ok":true,"mode":"control","retained_state_currently_verified":true,
        "acknowledged":{"journal_id":"1".repeat(64),"head":"2".repeat(64),"frames":5},
        "original":original(),"clock":clock(),"current_goal_available":true,
        "clock_advanced":false,"clock_control_attempted":false,"native_unpause_attempted":false})
}
fn shown(operation: &str, result: Value, reference: Value, ticks: u64) -> Result<Value> {
    serde_json::from_str(&presentation::packet(
        operation,
        Some(SessionId::new((1u128 << 127) | FAMILY | 21)),
        Some(RequestId::new(9)),
        Some(budgets(None, None, None, Some(ticks))?),
        result,
        reference,
    ))
    .map_err(|_| error(ErrorCode::InvalidRequest, "invalid clock presentation JSON"))
}
fn next(value: &Value) -> &Value {
    &value["agent_turn"]["recommendations"][0]
}

#[test]
fn workshop_wait_and_cancel_keep_legacy_goal_default_and_explicit_clock_scope() -> Result<()> {
    assert!(matches!(scope_action(None, false)?, Action::Wait));
    assert!(matches!(scope_action(Some("goal"), false)?, Action::Wait));
    assert!(matches!(
        scope_action(Some("clock"), false)?,
        Action::WaitClock
    ));
    assert!(matches!(scope_action(None, true)?, Action::Cancel));
    assert!(matches!(scope_action(Some("goal"), true)?, Action::Cancel));
    assert!(matches!(
        scope_action(Some("clock"), true)?,
        Action::CancelClock
    ));
    for scope in ["", "effect", "native", "Clock"] {
        assert!(scope_action(Some(scope), false).is_err());
        assert!(scope_action(Some(scope), true).is_err());
    }
    assert!(scope_action(Some("session"), false).is_err());
    Ok(())
}

#[test]
fn pending_workshop_recommends_exact_separately_reviewed_bounded_interval() -> Result<()> {
    let result = ready();
    let value = shown("fortress.wait", result.clone(), retained(&result), 1200)?;
    let recommendation = next(&value);
    assert_eq!(recommendation["tool"], "fortress_plan");
    let request: Value = serde_json::from_str(
        recommendation["arguments"]["request"]
            .as_str()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "clock request missing"))?,
    )
    .map_err(|_| error(ErrorCode::InvalidRequest, "clock request invalid"))?;
    assert_eq!(
        request,
        json!({"kind":"run_workshop",
        "target_id":"workshop-clock-target:exact-original","game_ticks":50,"wall_millis":5000})
    );
    assert_eq!(recommendation["grants_authority"], false);
    assert_eq!(recommendation["requires_confirmation"], false);
    assert_eq!(recommendation["estimated_cost"]["game_ticks"], 0);
    assert_eq!(
        recommendation["estimated_cost"]["native_clock_unpause_calls_max"],
        0
    );
    assert_eq!(value["agent_turn"]["budget"]["max_game_ticks"], 1200);
    assert_eq!(
        value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
    assert!(value["agent_turn"]["anchor"].is_null());
    Ok(())
}

#[test]
fn clock_recommendation_requires_current_readiness_explicit_budget_and_original_deadline()
-> Result<()> {
    for case in 0..12 {
        let mut result = ready();
        let mut ticks = 1200;
        match case {
            0 => result["clock"]["clock_ready"] = json!(false),
            1 => result["clock"]["new_run_available"] = json!(false),
            2 => result["clock"]["verified"] = json!(false),
            3 => result["clock"]["enabled"] = json!(false),
            4 => ticks = 0,
            5 => ticks = 49,
            6 => result["clock"]["recommended_game_ticks"] = json!(51),
            7 => result["clock"]["recommended_game_ticks"] = json!(0),
            8 => result["original"]["sample_tick"] = Value::Null,
            9 => result["mode"] = json!("recover"),
            10 => result["clock"]["target_id"] = Value::Null,
            _ => result["clock"]["advancement_fenced"] = json!(true),
        }
        let value = shown("fortress.query", result.clone(), retained(&result), ticks)?;
        assert_ne!(next(&value)["tool"], "fortress_plan", "case {case}");
        assert_ne!(next(&value)["tool"], "fortress_commit", "case {case}");
        assert_eq!(
            value["agent_turn"]["briefing"]["development_commit_available"],
            false
        );
    }
    Ok(())
}

#[test]
fn clock_confirmation_uses_its_own_seal_and_reports_its_real_tick_allowance() -> Result<()> {
    let mut result = ready();
    result["clock"]["active_run"] = run("prepared", "prepared");
    result["clock"]["pending"] = json!(true);
    result["clock"]["commit_available"] = json!(true);
    result["clock"]["new_run_available"] = json!(false);
    let value = shown("fortress.plan", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&value)["tool"], "fortress_commit");
    assert_eq!(next(&value)["arguments"]["review_seal"], "f".repeat(64));
    assert_ne!(
        next(&value)["arguments"]["review_seal"],
        result["original"]["review_seal"]
    );
    assert_eq!(next(&value)["arguments"]["confirmed"], true);
    assert_eq!(next(&value)["requires_confirmation"], true);
    assert_eq!(next(&value)["risk"], "guarded");
    assert_eq!(next(&value)["estimated_cost"]["game_ticks"], 50);
    assert_eq!(
        next(&value)["estimated_cost"]["native_clock_unpause_calls_max"],
        1
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["confirmations"][0]["scope"],
        "clock"
    );
    assert_eq!(value["agent_turn"]["briefing"]["clock_advanced"], false);
    assert_eq!(
        value["agent_turn"]["briefing"]["clock_receipt_proves_workshop_completion"],
        false
    );
    for (field, replacement) in [
        ("commit_available", json!(false)),
        ("verified", json!(false)),
        ("unresolved", json!(true)),
        ("advancement_fenced", json!(true)),
    ] {
        let mut recovered = result.clone();
        recovered["clock"][field] = replacement;
        let recovered = shown(
            "fortress.query",
            recovered.clone(),
            retained(&recovered),
            1200,
        )?;
        assert_ne!(next(&recovered)["tool"], "fortress_commit");
        assert_eq!(
            recovered["agent_turn"]["briefing"]["development_commit_available"],
            false
        );
    }
    for mode in ["recover", "offline"] {
        let mut recovered = result.clone();
        recovered["mode"] = json!(mode);
        let recovered = shown(
            "fortress.query",
            recovered.clone(),
            retained(&recovered),
            1200,
        )?;
        assert_ne!(next(&recovered)["tool"], "fortress_commit");
    }
    Ok(())
}

#[test]
fn lost_review_retires_unattempted_clock_only_with_current_control_authority() -> Result<()> {
    for state in ["intent", "prepared"] {
        for fenced in [false, true] {
            let mut result = ready();
            result["clock"]["active_run"] = run(state, "prepared");
            result["clock"]["active_run"]["review_seal"] = Value::Null;
            result["clock"]["pending"] = json!(true);
            result["clock"]["new_run_available"] = json!(false);
            result["clock"]["advancement_fenced"] = json!(fenced);
            let value = shown("fortress.query", result.clone(), retained(&result), 1200)?;
            assert_eq!(next(&value)["tool"], "fortress_cancel");
            assert_eq!(next(&value)["arguments"]["scope"], "clock");
            assert_eq!(next(&value)["risk"], "guarded");
            assert_eq!(next(&value)["estimated_cost"]["game_ticks"], 0);
            assert_eq!(
                next(&value)["estimated_cost"]["native_clock_unpause_calls_max"],
                0
            );
            assert_eq!(next(&value)["grants_authority"], false);
            assert_eq!(
                value["agent_turn"]["briefing"]["development_commit_available"],
                false
            );
            assert_eq!(
                value["agent_turn"]["references"][0]["clock"]["cancel_available"],
                false
            );

            result["clock"]["cancel_available"] = json!(false);
            for mode in ["control", "recover"] {
                result["mode"] = json!(mode);
                let unavailable = shown("fortress.query", result.clone(), retained(&result), 1200)?;
                assert_ne!(next(&unavailable)["tool"], "fortress_commit");
                assert_ne!(next(&unavailable)["tool"], "fortress_cancel");
                if state == "prepared" {
                    assert_eq!(next(&unavailable)["tool"], "fortress_explain");
                    assert!(next(&unavailable)["reason"].as_str().is_some_and(|reason| {
                        reason.contains("Recovery cannot restore dispatch permission")
                    }));
                } else {
                    assert_eq!(next(&unavailable)["tool"], "fortress_wait");
                    assert_eq!(next(&unavailable)["arguments"]["scope"], "clock");
                }
            }
            result["mode"] = json!("offline");
            let offline = shown("fortress.query", result.clone(), retained(&result), 1200)?;
            assert_eq!(next(&offline)["tool"], "fortress_query");
        }
    }
    Ok(())
}

#[test]
fn terminal_unknown_and_settled_advancement_fence_explain_without_phantom_waiting() -> Result<()> {
    let mut result = ready();
    result["clock"]["active_run"] = run("terminal", "source_lost");
    result["clock"]["pending"] = json!(true);
    result["clock"]["unresolved"] = json!(true);
    result["clock"]["new_run_available"] = json!(false);
    result["clock"]["advancement_fenced"] = json!(true);
    let unknown = shown("fortress.wait", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&unknown)["tool"], "fortress_explain");
    assert!(
        next(&unknown)["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("terminal unresolved"))
    );
    assert_eq!(unknown["agent_turn"]["briefing"]["clock_unresolved"], true);
    assert_eq!(
        unknown["agent_turn"]["active_work"]["indeterminate_effects"][0]["scope"],
        "clock"
    );
    assert_eq!(
        unknown["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
    assert_eq!(
        unknown["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );

    result["clock"]["active_run"] = Value::Null;
    result["clock"]["pending"] = json!(false);
    result["clock"]["unresolved"] = json!(false);
    let settled = shown("fortress.cancel", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&settled)["tool"], "fortress_explain");
    assert!(
        next(&settled)["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("permanently fenced"))
    );
    assert_eq!(
        settled["agent_turn"]["briefing"]["clock_advancement_fenced"],
        true
    );
    assert_eq!(next(&settled)["estimated_cost"]["game_ticks"], 0);

    // A fence stops new advancement; a still-running uncertain interval keeps
    // its useful reconciliation path instead of being hidden by that blocker.
    result["clock"]["active_run"] = run("tracking", "running");
    result["clock"]["pending"] = json!(true);
    result["clock"]["unresolved"] = json!(true);
    let running = shown("fortress.query", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&running)["tool"], "fortress_wait");
    assert_eq!(next(&running)["arguments"]["scope"], "clock");
    Ok(())
}

#[test]
fn active_or_uncertain_clock_reconciles_before_fresh_workshop_claims() -> Result<()> {
    for unresolved in [false, true] {
        let mut result = ready();
        result["clock"]["active_run"] = run("tracking", "running");
        result["clock"]["pending"] = json!(true);
        result["clock"]["unresolved"] = json!(unresolved);
        result["clock"]["new_run_available"] = json!(false);
        result["original"]["fresh_completion_proven"] = json!(true);
        result["original"]["historical_goal"] = json!("complete");
        result["clock_control_attempted"] = json!(true);
        result["native_unpause_attempted"] = json!(true);
        result["clock_advanced"] = Value::Null;
        let value = shown("fortress.commit", result.clone(), retained(&result), 1200)?;
        assert_eq!(next(&value)["tool"], "fortress_wait");
        assert_eq!(next(&value)["arguments"]["scope"], "clock");
        assert_eq!(next(&value)["estimated_cost"]["game_ticks"], 0);
        assert_eq!(
            value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
            false
        );
        assert_eq!(
            value["agent_turn"]["briefing"]["current_pause_proven"],
            false
        );
        assert!(value["agent_turn"]["briefing"]["clock_advanced"].is_null());
        assert_eq!(
            value["agent_turn"]["briefing"]["native_unpause_attempted"],
            true
        );
        assert_eq!(
            value["agent_turn"]["active_work"]["actions"][1]["scope"],
            "clock"
        );
        if unresolved {
            assert!(
                !value["agent_turn"]["active_work"]["indeterminate_effects"]
                    .as_array()
                    .ok_or_else(|| error(ErrorCode::InvalidRequest, "missing hazards"))?
                    .is_empty()
            );
        }
    }
    Ok(())
}

#[test]
fn stopped_receipt_needs_new_workshop_evidence_and_does_not_claim_current_pause() -> Result<()> {
    let mut result = ready();
    result["clock"]["active_run"] = run("terminal", "stopped");
    result["clock"]["active_run"]["observed_tick"] = json!(150);
    result["clock"]["active_run"]["stopped_with_pause"] = json!(true);
    result["clock"]["new_run_available"] = json!(false);
    result["clock"]["clock_ready"] = json!(false);
    result["current_goal_available"] = json!(false);
    result["original"]["fresh_goal"] = Value::Null;
    result["clock_advanced"] = json!(true);
    let value = shown("fortress.wait", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&value)["tool"], "fortress_wait");
    assert_eq!(next(&value)["arguments"]["scope"], "goal");
    assert_eq!(value["agent_turn"]["briefing"]["clock_advanced"], true);
    assert_eq!(
        value["agent_turn"]["briefing"]["clock_advance_evidence_scope"],
        "historical_run_receipt_only"
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
    result["current_goal_available"] = json!(true);
    result["original"]["historical_goal"] = json!("complete");
    result["original"]["fresh_goal"] = json!("complete");
    result["original"]["fresh_completion_proven"] = json!(true);
    let value = shown("fortress.wait", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&value)["tool"], "fortress_explain");
    assert_eq!(
        value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        true
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["clock_receipt_proves_workshop_completion"],
        false
    );
    Ok(())
}

#[test]
fn same_wait_can_recommend_another_review_only_after_fresh_incomplete_workshop_sample() -> Result<()>
{
    let mut result = ready();
    result["clock"]["active_run"] = run("terminal", "stopped");
    result["clock"]["active_run"]["observed_tick"] = json!(120);
    result["clock"]["active_run"]["stopped_with_pause"] = json!(true);
    result["original"]["sample_tick"] = json!(120);
    result["original"]["high_tick"] = json!(120);
    result["clock"]["recommended_game_ticks"] = json!(30);
    result["clock_advanced"] = json!(true);
    let value = shown("fortress.wait", result.clone(), retained(&result), 1200)?;
    assert_eq!(next(&value)["tool"], "fortress_plan");
    let request: Value = serde_json::from_str(
        next(&value)["arguments"]["request"]
            .as_str()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "request absent"))?,
    )
    .map_err(|_| error(ErrorCode::InvalidRequest, "request invalid"))?;
    assert_eq!(request["game_ticks"], 30);
    assert_eq!(request["target_id"], result["clock"]["target_id"]);
    assert_eq!(
        value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["development_commit_available"],
        false
    );
    Ok(())
}

#[test]
fn failure_keeps_latest_uncertain_run_and_both_recovery_roots_without_dispatch_advice() -> Result<()>
{
    let mut before = ready();
    before["clock"]["active_run"] = run("prepared", "prepared");
    before["clock"]["pending"] = json!(true);
    before["clock"]["commit_available"] = json!(true);
    let mut latest = retained(&before);
    latest["clock"]["active_run"]["state"] = json!("dispatch_started");
    latest["clock"]["unresolved"] = json!(true);
    latest["clock"]["recovery"]["head"] = json!("9".repeat(64));
    let reference = json!({"last_complete_reference":retained(&before),
        "latest_unverified_reference":latest});
    let value: Value = serde_json::from_str(&failure_with_request(
        "fortress.commit",
        error(ErrorCode::EffectIndeterminate, "run reply lost"),
        Some(SessionId::new((1u128 << 127) | FAMILY | 21)),
        Some(RequestId::new(9)),
        Some(budgets(None, None, None, Some(1200))?),
        reference,
    ))
    .map_err(|_| error(ErrorCode::InvalidRequest, "invalid failure JSON"))?;
    let saved = &value["agent_turn"]["references"][0]["retained_reference"];
    assert_eq!(saved["acknowledged"]["head"], "2".repeat(64));
    assert_eq!(saved["clock"]["recovery"]["journal_id"], "c".repeat(64));
    assert_eq!(saved["clock"]["recovery"]["head"], "9".repeat(64));
    assert_eq!(saved["clock"]["active_run"]["state"], "dispatch_started");
    assert_eq!(
        saved["clock"]["active_run"]["key"],
        "original-workshop-run-1"
    );
    assert_eq!(saved["clock"]["unresolved"], true);
    assert_eq!(saved["clock"]["verified"], false);
    assert_eq!(saved["clock"]["commit_available"], false);
    assert_eq!(next(&value)["tool"], "fortress_query");
    assert_eq!(
        value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );
    assert_eq!(value["agent_turn"]["briefing"]["clock_unresolved"], true);
    assert!(value["agent_turn"]["briefing"].get("admission").is_none());
    Ok(())
}

#[test]
fn closing_keeps_unsettled_clock_visible_without_claiming_effect_cancellation() -> Result<()> {
    let mut prior = ready();
    prior["clock"]["active_run"] = run("tracking", "running");
    prior["clock"]["pending"] = json!(true);
    prior["clock"]["unresolved"] = json!(true);
    let value = shown(
        "fortress.cancel",
        json!({"ok":true,"session_closed":true,
        "effects_cancelled":false,"custody_deleted":false,"current_goal_available":false}),
        json!({"last_complete_reference":retained(&prior),"latest_unverified_reference":retained(&prior)}),
        1200,
    )?;
    assert_eq!(value["result"]["effects_cancelled"], false);
    assert_eq!(value["agent_turn"]["recommendations"], json!([]));
    assert_eq!(value["agent_turn"]["briefing"]["clock_pending"], true);
    assert_eq!(
        value["agent_turn"]["briefing"]["clock_recovery"]["journal_id"],
        "c".repeat(64)
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["actions"][0]["scope"],
        "clock"
    );
    assert_eq!(
        value["agent_turn"]["active_work"]["indeterminate_effects"][0]["scope"],
        "clock"
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["clock_custody_currently_verified"],
        false
    );
    assert_eq!(
        value["agent_turn"]["briefing"]["current_pause_proven"],
        false
    );
    Ok(())
}

#[test]
fn overflow_retains_original_and_clock_hazards_with_all_agent_turn_fields() -> Result<()> {
    let mut result = ready();
    result["clock"]["active_run"] = run("dispatch_started", "prepared");
    result["clock"]["pending"] = json!(true);
    result["clock"]["unresolved"] = json!(true);
    result["oversized"] = json!("x".repeat(RESPONSE_BYTES as usize + 1));
    let value = shown("fortress.commit", result.clone(), retained(&result), 1200)?;
    assert_eq!(value["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(
        value["retained_reference"]["original_key"],
        "original-carpenter"
    );
    assert_eq!(
        value["retained_reference"]["clock"]["active_run"]["key"],
        "original-workshop-run-1"
    );
    assert_eq!(
        value["retained_reference"]["clock"]["active_run"]["plan_digest"],
        "e".repeat(64)
    );
    assert_eq!(
        value["retained_reference"]["clock"]["recovery"]["head"],
        "d".repeat(64)
    );
    assert_eq!(value["agent_turn"]["briefing"]["clock_unresolved"], true);
    assert_eq!(
        value["agent_turn"]["briefing"]["fresh_original_completion_proven"],
        false
    );
    assert_eq!(value["agent_turn"]["recommendations"], json!([]));
    assert!(value["agent_turn"]["anchor"].is_null());
    let contract: Value = serde_json::from_str(include_str!(
        "../../../../architecture/agent_turn_contract.json"
    ))
    .map_err(|_| error(ErrorCode::InvalidRequest, "invalid turn contract"))?;
    for field in contract["field_order"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "field order absent"))?
    {
        assert!(
            value["agent_turn"]
                .get(
                    field
                        .as_str()
                        .ok_or_else(|| error(ErrorCode::InvalidRequest, "field name invalid"))?
                )
                .is_some()
        );
    }
    assert!(value.to_string().len() < RESPONSE_BYTES as usize);
    let without_reference = shown("fortress.commit", result, Value::Null, 1200)?;
    assert_eq!(
        without_reference["retained_reference"]["clock"]["active_run"]["key"],
        "original-workshop-run-1"
    );
    Ok(())
}
