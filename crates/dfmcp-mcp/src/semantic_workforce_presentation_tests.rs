use super::*;
use dfmcp_core::{FortressId, GameTick, ObservationCursor};

fn original_anchor() -> StateAnchor {
    StateAnchor {
        fortress_id: FortressId::new(7),
        cursor: ObservationCursor {
            epoch: 1,
            sequence: 2,
        },
        tick: GameTick(40),
        state_hash: Digest32::of_bytes(b"canonical original workforce"),
    }
}
fn retained() -> Value {
    let a = anchor_value(original_anchor());
    json!({"ok":true,"mode":"control","anchor":a,"journal_id":"1".repeat(64),"identity_digest":"f".repeat(64),
        "native_state":"prepared","native_phase":"prepared","native_needs_query":false,"effect_uncertain":false,
        "observation":{"seal":"a".repeat(64),"source_digest":"8".repeat(64),"citizens":3,"paused":true,
            "units":[{"entity_id":"12","generation":1,"native_unit_id":2,"name":"Urist"}],
            "offset":0,"limit":32,"returned":3,"total":3,"complete":true,"next_query":null},
        "original_plan":{"plan_digest":"b".repeat(64),"anchor":a,
            "steps":[{"obligation":{"deadline_tick":140,"poll_interval_ticks":10,"stable_samples":2}}]},
        "review":{"paired_seal":"c".repeat(64),"native_plan_digest":"d".repeat(64),"semantic_plan_digest":"b".repeat(64)},
        "recovery":{"plan_digest":"b".repeat(64),"object_digest":"e".repeat(64),"anchor":a,
            "identity_digest":"f".repeat(64),"journal_id":"1".repeat(64),"history_present":false},
        "action_result":{"verified":false,"state":"unverified","receipt":null},
        "current_goal":null,"current_goal_available":false,"first_satisfied_anchor":null,
        "history":{"events":0,"bytes":0,"fenced":false}})
}
fn success() -> Value {
    json!({"ok":true,"development_labor_enabled":true})
}
fn output(operation: &str, mode: &str, result: Value, view: Value, uncertain: bool) -> Value {
    serde_json::from_str(&packet(
        operation,
        Some(SessionId::new(9)),
        Some(original_anchor()),
        Some(mode),
        Some(RequestId::new(3)),
        None,
        result,
        Some(view),
        uncertain,
    ))
    .expect("complete Agent Turn JSON")
}
fn enabled(value: &Value, action: &str) -> bool {
    value["agent_turn"]["affordances"]
        .as_array()
        .expect("affordances")
        .iter()
        .find(|v| v["action"] == action)
        .expect("known action")["enabled"]
        == true
}
fn applied() -> Value {
    let mut view = retained();
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("applied");
    view["action_result"] = json!({"verified":true,"state":"verified","receipt":"9".repeat(64)});
    view
}

#[test]
fn original_labor_review_has_complete_orientation_and_exact_confirmation() {
    let view = retained();
    let p = output("fortress.plan", "control", success(), view.clone(), false);
    for key in [
        "schema",
        "operation",
        "phase",
        "session_id",
        "turn_id",
        "request_id",
        "anchor",
        "continuity",
        "profile",
        "briefing",
        "changes",
        "attention",
        "active_work",
        "affordances",
        "recommendations",
        "uncertainty",
        "coverage",
        "budget",
        "references",
    ] {
        assert!(p["agent_turn"].get(key).is_some(), "missing {key}");
    }
    assert!(p["agent_turn"]["turn_id"].is_null());
    assert_eq!(p["agent_turn"]["phase"], "propose");
    assert!(enabled(&p, "fortress.commit"));
    let next = &p["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.commit");
    assert_eq!(next["requires_confirmation"], true);
    assert_eq!(next["arguments"]["confirmed"], true);
    assert_eq!(
        next["arguments"]["paired_seal"],
        view["review"]["paired_seal"]
    );
    assert_eq!(
        p["agent_turn"]["active_work"]["pending_plans"][0]["recovery"],
        view["recovery"]
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["current_freshness_proven"],
        false
    );
    assert!(p["agent_turn"]["briefing"].get("admission").is_none());
}

#[test]
fn current_authority_exact_observation_and_control_mode_all_gate_confirmation() {
    for (mode, result) in [
        ("control", json!({"ok":true})),
        ("reconcile", success()),
        ("offline", success()),
    ] {
        let p = output("fortress.query", mode, result, retained(), false);
        assert!(!enabled(&p, "fortress.commit"));
    }
    let mut view = retained();
    view["observation"] = Value::Null;
    let p = output("fortress.query", "control", success(), view, false);
    assert!(!enabled(&p, "fortress.commit"));
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.observe"
    );
}

#[test]
fn interrupted_original_plan_only_offers_unchanged_request_without_new_goal() {
    let mut view = retained();
    view["native_state"] = Value::Null;
    view["native_phase"] = Value::Null;
    view["review"] = Value::Null;
    let p = output("fortress.query", "control", success(), view, false);
    assert!(enabled(&p, "fortress.plan"));
    assert_eq!(
        p["agent_turn"]["affordances"][0]["purpose"],
        "resume_only_the_unchanged_original_request"
    );
    assert!(!enabled(&p, "fortress.commit"));
}

#[test]
fn historical_assignment_and_fresh_labor_goal_are_separate_claims() {
    let mut view = applied();
    let p = output("fortress.commit", "control", success(), view.clone(), false);
    assert_eq!(
        p["agent_turn"]["briefing"]["assignment_verified_at_anchor"],
        true
    );
    assert!(p["agent_turn"]["briefing"]["original_goal_proven_at_anchor"].is_null());
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    view["current_goal"] = json!({"anchor":anchor_value(original_anchor()),
        "original_goal_proven":true,"semantic_completion_proven":true,"current_truth":"true"});
    let p = output("fortress.wait", "control", success(), view, false);
    assert_eq!(
        p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        true
    );
    assert!(
        p["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .is_empty()
    );
    assert!(
        p["agent_turn"]["briefing"]
            .get("physical_quiescence_proven_at_anchor")
            .is_none()
    );
}

#[test]
fn true_goal_cannot_repair_unknown_native_action() {
    let mut view = applied();
    view["native_phase"] = json!("unknown");
    view["effect_uncertain"] = json!(true);
    view["action_result"] = json!({"verified":false,"state":"unverified","receipt":null});
    view["current_goal"] = json!({"original_goal_proven":true,"semantic_completion_proven":true});
    let p = output("fortress.wait", "reconcile", success(), view, false);
    assert_eq!(
        p["agent_turn"]["briefing"]["original_goal_proven_at_anchor"],
        true
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        false
    );
    assert_eq!(p["agent_turn"]["continuity"]["status"], "indeterminate");
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.explain"
    );
    assert!(!enabled(&p, "fortress.commit"));
}

#[test]
fn applied_outside_single_labor_semantics_is_visible_without_completion() {
    let mut view = applied();
    view["action_result"]["verified"] = json!(false);
    view["action_result"]["state"] = json!("applied_outside_semantics");
    view["current_goal"] = json!({"original_goal_proven":true,"semantic_completion_proven":false});
    let p = output("fortress.wait", "control", success(), view, false);
    assert_eq!(
        p["agent_turn"]["active_work"]["actions"][0]["action_result"]["state"],
        "applied_outside_semantics"
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        false
    );
}

#[test]
fn first_achievement_recovered_offline_does_not_imply_current_truth() {
    let mut view = applied();
    view["first_satisfied_anchor"] = anchor_value(original_anchor());
    let p = output("fortress.query", "offline", success(), view, false);
    assert_eq!(
        p["agent_turn"]["briefing"]["historical_first_satisfied_anchor"],
        anchor_value(original_anchor())
    );
    assert!(p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert!(!enabled(&p, "fortress.wait"));
    assert!(!enabled(&p, "fortress.commit"));
}

#[test]
fn preparation_reply_loss_reconciles_without_assignment_or_preparation_retry() {
    let mut view = retained();
    view["native_state"] = json!("intent");
    view["native_phase"] = Value::Null;
    view["native_needs_query"] = json!(true);
    let p = output("fortress.plan", "control", success(), view, false);
    assert!(!enabled(&p, "fortress.plan"));
    assert!(!enabled(&p, "fortress.commit"));
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
}

#[test]
fn failure_and_close_preserve_exact_identity_without_promoting_cached_review() {
    let view = retained();
    let p = output(
        "fortress.commit",
        "control",
        json!({"ok":false,"error":{"code":"effect_indeterminate"},
        "retained_state_currently_verified":false}),
        view.clone(),
        true,
    );
    assert_eq!(p["retained_state_currently_verified"], false);
    assert_eq!(p["agent_turn"]["active_work"]["state_known"], false);
    assert!(!enabled(&p, "fortress.commit"));
    assert!(!enabled(&p, "fortress.plan"));
    assert_eq!(p["retained_state"]["recovery"], view["recovery"]);
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.query"
    );
    let p = output(
        "fortress.cancel",
        "control",
        json!({"ok":true,"closed":true,
        "effects_cancelled":false,"retained_state_currently_verified":false}),
        view,
        true,
    );
    assert_eq!(p["agent_turn"]["phase"], "handoff");
    assert!(!enabled(&p, "fortress.commit"));
    assert!(
        p["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .is_empty()
    );
}

#[test]
fn bounded_citizen_continuation_remains_bound_to_one_exact_observation() {
    let mut view = retained();
    for key in [
        "original_plan",
        "review",
        "recovery",
        "native_state",
        "native_phase",
    ] {
        view[key] = Value::Null;
    }
    view["observation"]["total"] = json!(70);
    view["observation"]["returned"] = json!(32);
    view["observation"]["complete"] = json!(false);
    let continuation =
        json!({"mode":"citizens","expected_observation":"a".repeat(64),"offset":32,"limit":32});
    view["observation"]["next_query"] = continuation.clone();
    let p = output("fortress.observe", "control", success(), view, false);
    let next = &p["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.query");
    assert_eq!(next["arguments"]["query"], continuation.to_string());
    assert_eq!(
        p["agent_turn"]["coverage"]["citizen_page"]["complete"],
        false
    );
    assert_eq!(p["agent_turn"]["coverage"]["continuation"], continuation);
}

#[test]
fn overflow_is_complete_and_retains_every_original_recovery_identity() {
    let mut view = applied();
    view["unexpected_configuration"] = json!("discard this");
    view["original_plan"]["unbounded_details"] = json!("x".repeat(RESPONSE_RESERVE as usize));
    let p = output("fortress.wait", "control", success(), view.clone(), true);
    assert!(p.to_string().len() as u64 <= RESPONSE_RESERVE);
    assert_eq!(p["result"]["details_omitted"], true);
    assert_eq!(p["retained_state"]["recovery"], view["recovery"]);
    assert_eq!(p["retained_state"]["journal_id"], view["journal_id"]);
    assert_eq!(p["retained_state"]["review"], view["review"]);
    assert!(
        p["retained_state"]
            .get("unexpected_configuration")
            .is_none()
    );
    assert!(p["retained_state"]["current_goal"].is_null());
    assert!(!enabled(&p, "fortress.commit"));
    assert!(p["agent_turn"]["briefing"].get("admission").is_none());
}

#[test]
fn failed_opening_keeps_new_journal_identity_without_inventing_an_original_goal() {
    let view =
        json!({"ok":false,"journal_id":"7".repeat(64),"retained_state_currently_verified":false});
    let p = output(
        "fortress.open_session",
        "control",
        json!({"ok":false}),
        view,
        false,
    );
    assert_eq!(
        p["agent_turn"]["active_work"]["retained_identity"]["journal_id"],
        "7".repeat(64)
    );
    assert!(p["agent_turn"]["active_work"]["retained_identity"]["recovery"].is_null());
    assert!(!enabled(&p, "fortress.commit"));
}

#[test]
fn partial_identity_publication_survives_error_close_and_output_overflow() {
    let view = json!({"ok":false,"journal_id":"7".repeat(64),"identity_digest":"8".repeat(64),
        "partial_goal_publication":true,"recovery":null,
        "discarded_detail":"x".repeat(RESPONSE_RESERVE as usize)});
    let p = output(
        "fortress.cancel",
        "control",
        json!({"ok":true,"closed":true,
        "effects_cancelled":false,"retained_state_currently_verified":false}),
        view,
        false,
    );
    assert!(p.to_string().len() as u64 <= RESPONSE_RESERVE);
    assert_eq!(p["result"]["closed"], true);
    assert_eq!(p["retained_state"]["identity_digest"], "8".repeat(64));
    assert_eq!(p["retained_state"]["partial_goal_publication"], true);
    assert!(p["retained_state"]["recovery"].is_null());
    assert!(p["retained_state"].get("discarded_detail").is_none());
    assert_eq!(
        p["agent_turn"]["references"][0]["identity_digest"],
        "8".repeat(64)
    );
    assert!(!enabled(&p, "fortress.commit"));
}
