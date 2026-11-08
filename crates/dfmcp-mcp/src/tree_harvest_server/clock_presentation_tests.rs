use super::*;
use dfmcp_adapter::bounded_run::RunPhase;
use dfmcp_adapter::bounded_run::journal::RunState;

fn ready() -> Value {
    json!({"ok":true, "mode":"control", "retained_state_currently_verified":true,
        "max_game_ticks":1200, "clock_advanced":false,
        "original":{"key":"tree-original", "review_seal":"a".repeat(64),
            "native_phase":2, "prepared_in_this_connection":false,
            "historical_supply_observed_tick":null, "continuity_lost":false},
        "clock":{"enabled":true,"verified":true, "target_id":"b".repeat(64),
            "recovery":{"journal_id":"c".repeat(64), "head":"d".repeat(64)},
            "deadline_tick":300, "high_tick":100, "consumed_tick_floor":100,
            "pending":false, "unresolved":false, "active_run":null,
            "clock_ready":true, "new_run_available":true, "commit_available":false,
            "cancel_available":true, "remaining_tree_samples":15,
            "remaining_horizon_ticks":200, "recommended_game_ticks":100}})
}

fn reference(value: &Value) -> Value {
    json!({"original_key":"tree-original", "uncertain_effect":false,
        "acknowledged":{"journal_id":"e".repeat(64),"head":"f".repeat(64),"frames":5},
        "clock":value["clock"].clone()})
}

fn shown(value: Value, retained: Value) -> Value {
    serde_json::from_str(&packet(
        "fortress.wait",
        Some(SessionId::new((1u128 << 127) | FAMILY | 19)),
        Some(RequestId::new(2)),
        value,
        retained,
    ))
    .expect("valid Tree packet")
}

fn prepared() -> Value {
    let mut value = ready();
    value["clock"]["new_run_available"] = json!(false);
    value["clock"]["commit_available"] = json!(true);
    value["clock"]["pending"] = json!(true);
    value["clock"]["active_run"] = json!({"key":"run-original-1", "review_seal":"1".repeat(64),
        "game_ticks":100, "wall_millis":1000, "state":"prepared", "native_unpause_attempted":false});
    value
}

#[test]
fn original_tree_clock_recommends_separate_bounded_review_without_claiming_wood() {
    let value = ready();
    let packet = shown(value.clone(), reference(&value));
    let next = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_plan");
    let request: Value =
        serde_json::from_str(next["arguments"]["request"].as_str().unwrap()).unwrap();
    assert_eq!(
        request,
        json!({"kind":"run_tree", "target_id":"b".repeat(64),
        "game_ticks":100,"wall_millis":1000})
    );
    assert_eq!(next["requires_confirmation"], false);
    assert_eq!(next["estimated_cost"]["game_ticks"], 0);
    assert_eq!(next["grants_authority"], false);
    assert_eq!(
        packet["agent_turn"]["briefing"]["clock"]["harvest_goal_proven"],
        false
    );
    assert_eq!(packet["agent_turn"]["budget"]["max_game_ticks"], 1200);
}

#[test]
fn tree_clock_confirmation_selects_only_the_exact_clock_seal() {
    let value = prepared();
    let packet = shown(value.clone(), reference(&value));
    let next = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress_commit");
    assert_eq!(next["arguments"]["scope"], "clock");
    assert_eq!(next["arguments"]["review_seal"], "1".repeat(64));
    assert_eq!(next["requires_confirmation"], true);
    assert_eq!(next["estimated_cost"]["game_ticks"], 100);
    assert_eq!(
        packet["agent_turn"]["active_work"]["confirmations"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        packet["agent_turn"]["active_work"]["confirmations"][0]["scope"],
        "clock"
    );
}

#[test]
fn unresolved_tree_clock_preserves_both_custodies_and_offers_reconcile_and_safety_stop() {
    let mut value = prepared();
    value["clock"]["commit_available"] = json!(false);
    value["clock"]["unresolved"] = json!(true);
    value["clock"]["active_run"]["state"] = json!(RunState::DispatchStarted.as_str());
    value["clock"]["active_run"]["review_seal"] = Value::Null;
    value["clock"]["active_run"]["native_unpause_attempted"] = json!(true);
    let packet = shown(value.clone(), reference(&value));
    let next = &packet["agent_turn"]["recommendations"];
    assert_eq!(next[0]["tool"], "fortress_wait");
    assert_eq!(next[0]["arguments"]["scope"], "clock");
    assert_eq!(next[1]["tool"], "fortress_cancel");
    assert_eq!(next[1]["arguments"]["scope"], "clock");
    assert!(
        !packet["agent_turn"]["active_work"]["indeterminate_effects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        packet["retained_reference"]["acknowledged"]["journal_id"],
        "e".repeat(64)
    );
    assert_eq!(
        packet["retained_reference"]["clock"]["recovery"]["journal_id"],
        "c".repeat(64)
    );
}

#[test]
fn lost_tree_clock_review_retires_the_real_unattempted_states_without_another_unpause() {
    for state in [RunState::Intent, RunState::Prepared] {
        for fenced in [false, true] {
            let mut value = prepared();
            value["clock"]["commit_available"] = json!(false);
            value["clock"]["active_run"]["state"] = json!(state.as_str());
            value["clock"]["active_run"]["review_seal"] = Value::Null;
            value["clock"]["advancement_fenced"] = json!(fenced);
            let retained = reference(&value);
            let packet = shown(value.clone(), retained.clone());
            let next = &packet["agent_turn"]["recommendations"];
            assert_eq!(next.as_array().unwrap().len(), 1);
            assert_eq!(next[0]["tool"], "fortress_cancel");
            assert_eq!(next[0]["arguments"]["scope"], "clock");
            assert_eq!(next[0]["risk"], "guarded");
            assert_eq!(next[0]["requires_confirmation"], false);
            assert_eq!(next[0]["estimated_cost"]["game_ticks"], 0);
            assert_eq!(next[0]["estimated_cost"]["native_writer_calls_max"], 0);
            assert_eq!(next[0]["grants_authority"], false);
            assert_eq!(packet["retained_reference"], retained);
            assert_eq!(
                packet["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
            assert!(
                packet["agent_turn"]["briefing"]["clock"]["active_run"]["review_seal"].is_null()
            );

            for (mode, allowed) in [("control", false), ("recover", false), ("recover", true)] {
                value["mode"] = json!(mode);
                value["clock"]["cancel_available"] = json!(allowed);
                let recovered = shown(value.clone(), reference(&value));
                let next = &recovered["agent_turn"]["recommendations"];
                assert_eq!(next.as_array().unwrap().len(), 1);
                assert_eq!(next[0]["tool"], "fortress_explain");
                assert_eq!(
                    recovered["agent_turn"]["active_work"]["confirmations"],
                    json!([])
                );
                assert_eq!(
                    recovered["agent_turn"]["active_work"]["clock"]["active_run"]["key"],
                    "run-original-1"
                );
            }
        }
    }
}

#[test]
fn terminal_tree_clock_source_loss_keeps_uncertainty_without_wait_or_ineffective_stop() {
    for mode in ["control", "recover"] {
        let mut value = prepared();
        value["mode"] = json!(mode);
        value["clock"]["active_run"]["state"] = json!(RunState::Terminal.as_str());
        value["clock"]["active_run"]["native_phase"] = json!(RunPhase::SourceLost.as_str());
        value["clock"]["active_run"]["native_unpause_attempted"] = json!(true);
        value["clock"]["unresolved"] = json!(true);
        value["clock"]["advancement_fenced"] = json!(true);
        // A stale availability flag/seal cannot override terminal evidence.
        let retained = reference(&value);
        let packet = shown(value, retained.clone());
        let next = &packet["agent_turn"]["recommendations"];
        assert_eq!(next.as_array().unwrap().len(), 1);
        assert_eq!(next[0]["tool"], "fortress_explain");
        assert_eq!(packet["retained_reference"], retained);
        assert_eq!(
            packet["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            packet["agent_turn"]["briefing"]["clock"]["commit_available"],
            false
        );
        assert_eq!(
            packet["agent_turn"]["briefing"]["clock"]["current_pause_proven"],
            false
        );
        assert_eq!(
            packet["agent_turn"]["briefing"]["clock"]["harvest_goal_proven"],
            false
        );
        assert!(packet["agent_turn"]["briefing"]["clock_advanced"].is_null());
        assert!(packet["agent_turn"]["briefing"]["clock"]["active_run"]["review_seal"].is_null());
        assert!(
            !packet["agent_turn"]["active_work"]["indeterminate_effects"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn blocked_tree_clock_without_an_active_interval_does_not_recommend_phantom_waits() {
    let mut value = ready();
    value["clock"]["pending"] = json!(true);
    value["clock"]["new_run_available"] = json!(false);
    value["clock"]["advancement_fenced"] = json!(true);
    let packet = shown(value.clone(), reference(&value));
    let next = &packet["agent_turn"]["recommendations"];
    assert_eq!(next.as_array().unwrap().len(), 1);
    assert_eq!(next[0]["tool"], "fortress_explain");
    assert_eq!(
        packet["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
}

#[test]
fn in_flight_tree_clock_keeps_reconciliation_and_only_current_control_can_stop_it() {
    for state in [
        RunState::DispatchStarted,
        RunState::Tracking,
        RunState::CancelRequested,
    ] {
        for (mode, allowed) in [("control", true), ("control", false), ("recover", true)] {
            let mut value = prepared();
            value["mode"] = json!(mode);
            value["clock"]["active_run"]["state"] = json!(state.as_str());
            value["clock"]["active_run"]["review_seal"] = Value::Null;
            value["clock"]["commit_available"] = json!(false);
            value["clock"]["unresolved"] = json!(true);
            value["clock"]["cancel_available"] = json!(allowed);
            value["clock"]["advancement_fenced"] = json!(true);
            let packet = shown(value.clone(), reference(&value));
            let next = packet["agent_turn"]["recommendations"].as_array().unwrap();
            assert_eq!(next[0]["tool"], "fortress_wait");
            assert_eq!(next[0]["arguments"]["scope"], "clock");
            assert_eq!(next[0]["estimated_cost"]["game_ticks"], 0);
            assert_eq!(next[0]["estimated_cost"]["native_writer_calls_max"], 0);
            if mode == "control" && allowed {
                assert_eq!(next.len(), 2);
                assert_eq!(next[1]["tool"], "fortress_cancel");
                assert_eq!(next[1]["arguments"]["scope"], "clock");
                assert_eq!(next[1]["estimated_cost"]["native_writer_calls_max"], 1);
            } else {
                assert_eq!(next.len(), 1);
            }
            assert_eq!(
                packet["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
        }
    }
}

#[test]
fn tree_clock_confirmation_requires_a_live_prepared_review() {
    for case in 0..7 {
        let mut value = prepared();
        match case {
            0 => value["clock"]["active_run"]["state"] = json!(RunState::Intent.as_str()),
            1 => value["clock"]["active_run"]["review_seal"] = json!(""),
            2 => value["clock"]["pending"] = json!(false),
            3 => value["clock"]["unresolved"] = json!(true),
            4 => value["clock"]["advancement_fenced"] = json!(true),
            5 => value["mode"] = json!("recover"),
            _ => value["clock"]["active_run"]["state"] = json!(RunState::DispatchStarted.as_str()),
        }
        let packet = shown(value.clone(), reference(&value));
        assert_ne!(
            packet["agent_turn"]["recommendations"][0]["tool"], "fortress_commit",
            "case {case}"
        );
        assert_eq!(
            packet["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            packet["agent_turn"]["briefing"]["clock"]["commit_available"],
            false
        );
        assert!(packet["agent_turn"]["briefing"]["clock"]["active_run"]["review_seal"].is_null());
    }
}

#[test]
fn tree_clock_plan_guidance_requires_current_readiness_budget_and_remaining_original_capacity() {
    for case in 0..8 {
        let mut value = ready();
        match case {
            0 => value["max_game_ticks"] = json!(0),
            1 => value["clock"]["clock_ready"] = json!(false),
            2 => value["clock"]["advancement_fenced"] = json!(true),
            3 => value["clock"]["remaining_tree_samples"] = json!(2),
            4 => value["clock"]["remaining_horizon_ticks"] = json!(99),
            5 => value["mode"] = json!("recover"),
            6 => value["clock"]["verified"] = json!(false),
            _ => value["original"]["historical_supply_observed_tick"] = json!(100),
        }
        let packet = shown(value.clone(), reference(&value));
        assert_ne!(
            packet["agent_turn"]["recommendations"][0]["tool"], "fortress_plan",
            "case{case}"
        );
        assert_eq!(
            packet["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
    }
}

#[test]
fn tree_clock_failure_offline_and_close_cannot_restore_confirmation_or_current_proof() {
    for case in 0..3 {
        let mut value = prepared();
        let mut retained = reference(&value);
        retained["clock"]["unresolved"] = json!(true);
        match case {
            0 => value = json!({"ok":false}),
            1 => {
                value["mode"] = json!("offline");
                value["clock"]["verified"] = json!(false);
            }
            _ => value = json!({"ok":true,"session_closed":true}),
        }
        let packet = shown(value, retained.clone());
        assert_eq!(packet["retained_reference"], retained);
        assert_eq!(
            packet["agent_turn"]["active_work"]["confirmations"],
            json!([])
        );
        assert_eq!(
            packet["agent_turn"]["briefing"]["clock"]["commit_available"],
            false
        );
        assert_eq!(
            packet["agent_turn"]["briefing"]["clock"]["current_pause_proven"],
            false
        );
        assert!(packet["agent_turn"]["briefing"]["clock_advanced"].is_null());
        assert!(packet["agent_turn"]["briefing"]["clock"]["active_run"]["review_seal"].is_null());
        assert!(
            !packet["agent_turn"]["active_work"]["indeterminate_effects"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn tree_clock_overflow_keeps_recovery_without_a_dispatch_recommendation() {
    let mut value = prepared();
    value["oversized_evidence"] = json!("x".repeat(RESPONSE_BYTES as usize + 1));
    let mut retained = reference(&value);
    retained["clock"]["unresolved"] = json!(true);
    let packet = shown(value, retained.clone());
    assert_eq!(packet["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(packet["retained_reference"], retained);
    assert_eq!(
        packet["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(packet["agent_turn"]["recommendations"], json!([]));
    assert_eq!(
        packet["agent_turn"]["briefing"]["clock"]["commit_available"],
        false
    );
    assert!(
        !packet["agent_turn"]["active_work"]["indeterminate_effects"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
