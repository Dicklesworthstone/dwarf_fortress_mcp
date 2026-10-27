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
        state_hash: Digest32::of_bytes(b"canonical original"),
    }
}
fn retained() -> Value {
    let a = anchor_value(original_anchor());
    json!({"ok":true,"mode":"control","anchor":a,"native_state":"prepared",
        "observation":{"seal":"a".repeat(64),"source_digest":"8".repeat(64),"citizens":3,"paused":true},
        "original_plan":{"plan_digest":"b".repeat(64),"anchor":a,
            "steps":[{"obligation":{"deadline_tick":140,"poll_interval_ticks":10,"stable_samples":2}}]},
        "review":{"paired_seal":"c".repeat(64),"native_plan_digest":"d".repeat(64),"semantic_plan_digest":"b".repeat(64)},
        "recovery":{"plan_digest":"b".repeat(64),"object_digest":"e".repeat(64),"anchor":a,"history_present":false},
        "insertion":{"verified":false,"native_order_id":null,"receipt":null},
        "current_goal":null,"current_goal_available":false,"first_satisfied_anchor":null,
        "history":{"events":0,"bytes":0,"fenced":false}})
}
fn success() -> Value {
    json!({"ok":true,"development_production_enabled":true})
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
    .expect("complete JSON")
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

#[test]
fn prepared_goal_exposes_exact_confirmation_and_the_full_agent_turn() {
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
    assert_eq!(p["agent_turn"]["phase"], "propose");
    assert_eq!(p["agent_turn"]["continuity"]["status"], "partial");
    assert!(p["agent_turn"]["turn_id"].is_null());
    assert!(enabled(&p, "fortress.commit"));
    let r = &p["agent_turn"]["recommendations"][0];
    assert_eq!(r["tool"], "fortress.commit");
    assert_eq!(r["requires_confirmation"], true);
    assert_eq!(r["arguments"]["confirmed"], true);
    assert_eq!(r["arguments"]["paired_seal"], view["review"]["paired_seal"]);
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
fn control_mode_alone_does_not_supply_authority_or_recovered_observation() {
    let p = output(
        "fortress.query",
        "control",
        json!({"ok":true}),
        retained(),
        false,
    );
    assert!(!enabled(&p, "fortress.commit"));
    let mut view = retained();
    view["observation"] = Value::Null;
    let p = output("fortress.query", "control", success(), view, false);
    assert!(!enabled(&p, "fortress.commit"));
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.observe"
    );
    let p = output("fortress.query", "reconcile", success(), retained(), false);
    assert!(!enabled(&p, "fortress.commit"));
}

#[test]
fn interrupted_preparation_exposes_only_an_unchanged_original_request_retry() {
    let mut view = retained();
    view["native_state"] = Value::Null;
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
fn insertion_current_goal_and_physical_quiescence_remain_distinct() {
    let mut view = retained();
    view["native_state"] = json!("created");
    view["insertion"] = json!({"verified":true,"native_order_id":99,"receipt":"9".repeat(64)});
    let p = output("fortress.commit", "control", success(), view.clone(), false);
    assert_eq!(
        p["agent_turn"]["briefing"]["insertion_verified_at_anchor"],
        true
    );
    assert!(p["agent_turn"]["briefing"]["original_goal_proven_at_anchor"].is_null());
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    view["current_goal"] = json!({"anchor":anchor_value(original_anchor()),
        "original_goal_proven":true,"receipt_linked":true,"physical_quiescence_proven":false,
        "semantic_completion_proven":true});
    let p = output("fortress.wait", "control", success(), view.clone(), false);
    assert_eq!(
        p["agent_turn"]["briefing"]["original_goal_proven_at_anchor"],
        true
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        false
    );
    view["current_goal"]["physical_quiescence_proven"] = json!(true);
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
}

#[test]
fn historical_success_and_offline_recovery_do_not_supply_current_truth() {
    let mut view = retained();
    view["native_state"] = json!("created");
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
fn failed_call_keeps_recovery_without_promoting_cached_preparation() {
    let view = retained();
    let p = output(
        "fortress.commit",
        "control",
        json!({"ok":false,
        "error":{"code":"effect_indeterminate"},"retained_state_currently_verified":false}),
        view.clone(),
        true,
    );
    assert_eq!(p["retained_state_currently_verified"], false);
    assert_eq!(p["agent_turn"]["continuity"]["status"], "indeterminate");
    assert_eq!(p["agent_turn"]["active_work"]["state_known"], false);
    assert!(!enabled(&p, "fortress.commit"));
    assert!(!enabled(&p, "fortress.plan"));
    assert!(
        p["agent_turn"]["active_work"]["pending_plans"]
            .as_array()
            .expect("plans")
            .is_empty()
    );
    assert_eq!(p["retained_state"]["recovery"], view["recovery"]);
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.query"
    );
    assert_eq!(
        p["agent_turn"]["recommendations"][1]["tool"],
        "fortress.doctor"
    );
    let p = output(
        "fortress.cancel",
        "control",
        json!({"ok":true,"closed":true,
        "retained_state_currently_verified":false}),
        view,
        false,
    );
    assert!(!enabled(&p, "fortress.commit"));
    assert!(
        p["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .is_empty()
    );
}

#[test]
fn native_uncertainty_is_not_reported_as_a_safe_non_effect() {
    let mut view = retained();
    view["native_state"] = json!("dispatch_started");
    let p = output("fortress.query", "reconcile", success(), view, false);
    assert_eq!(
        p["agent_turn"]["active_work"]["indeterminate_effects"][0]["effect_may_have_started"],
        true
    );
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert!(!enabled(&p, "fortress.commit"));
}

#[test]
fn overflow_remains_complete_and_preserves_the_exact_recovery_handoff() {
    let mut view = retained();
    view["unexpected_private_configuration"] = json!("must-not-survive-fallback");
    let raw = packet(
        "fortress.commit",
        Some(SessionId::new(9)),
        Some(original_anchor()),
        Some("control"),
        Some(RequestId::new(3)),
        None,
        json!({"ok":true,"huge":"x".repeat(2 * RESPONSE_RESERVE as usize)}),
        Some(view.clone()),
        true,
    );
    assert!(raw.len() as u64 <= RESPONSE_RESERVE);
    assert!(!raw.contains("must-not-survive-fallback"));
    let p: Value = serde_json::from_str(&raw).expect("whole JSON, never byte truncation");
    assert_eq!(p["result"]["ok"], false);
    assert_eq!(p["result"]["automatic_redispatch"], false);
    assert_eq!(p["result"]["effect_may_have_started"], true);
    assert_eq!(p["retained_state"]["recovery"], view["recovery"]);
    assert_eq!(p["retained_state"]["review"], view["review"]);
    assert_eq!(p["retained_state_currently_verified"], false);
    assert!(!enabled(&p, "fortress.commit"));
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.query"
    );
    let raw = packet(
        "fortress.cancel",
        Some(SessionId::new(9)),
        Some(original_anchor()),
        Some("control"),
        None,
        None,
        json!({"ok":true,"closed":true,"effects_cancelled":false,
            "huge":"x".repeat(2 * RESPONSE_RESERVE as usize)}),
        Some(view),
        false,
    );
    let p: Value = serde_json::from_str(&raw).expect("whole close refusal");
    assert_eq!(p["result"]["closed"], true);
    assert_eq!(p["result"]["effects_cancelled"], false);
    assert!(
        p["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .is_empty()
    );
}

#[test]
fn bootstrap_and_admission_refusal_never_fabricate_a_source_anchor() {
    let mut bootstrap = original_anchor();
    bootstrap.state_hash = Digest32::ZERO;
    let raw = packet(
        "fortress.open_session",
        None,
        Some(bootstrap),
        None,
        None,
        None,
        json!({"ok":false,"error":{"code":"capability_denied"}}),
        None,
        false,
    );
    let p: Value = serde_json::from_str(&raw).expect("complete refusal");
    assert!(p["agent_turn"]["anchor"].is_null());
    assert_eq!(p["agent_turn"]["active_work"]["state_known"], false);
    let mut turn = AgentTurnBuilder::new("fortress.open_session", AgentPhase::Bootstrap).build();
    turn["briefing"]["admission"] = json!({"state":"admitted","ticket_id":"ambient"});
    let p: Value =
        serde_json::from_str(&finish(turn, json!({"ok":false}), None, false)).expect("refusal");
    assert!(p["agent_turn"]["briefing"].get("admission").is_none());
}
