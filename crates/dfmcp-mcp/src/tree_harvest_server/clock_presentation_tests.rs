use super::*;

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
