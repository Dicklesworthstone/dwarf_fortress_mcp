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

fn clock_retained(prepared: bool) -> Value {
    let mut view = retained();
    view["native_state"] = json!("created");
    view["insertion"] = json!({"verified":true,"native_order_id":99,"receipt":"9".repeat(64)});
    view["recovery"]["clock_journal_id"] = json!("1".repeat(64));
    view["development_clock_enabled"] = json!(true);
    view["current_goal_available"] = json!(true);
    view["current_goal"] = json!({"anchor":anchor_value(original_anchor()),"current_truth":"false",
        "original_goal_proven":false,"receipt_linked":true,"physical_quiescence_proven":false,
        "semantic_completion_proven":false});
    let run = json!({"review_seal":"2".repeat(64),"key":"semantic-clock-original-key",
        "plan_digest":"3".repeat(64),"state":"prepared","native_order_id":99,
        "before_tick":40,"game_ticks":20,"wall_millis":1000,"pending":true,"unresolved":false,
        "purpose":"advance_original_order","stop_predicate":"remaining_at_most","native":null});
    view["clock"] = json!({"journal_id":"1".repeat(64),"head":"4".repeat(64),"mode":"control",
        "target_digest":"5".repeat(64),"original_paired_seal":"c".repeat(64),
        "run_count":u32::from(prepared),"pending":prepared,"unresolved":false,"may_be_running":false,
        "needs_reconciliation":prepared,"high_tick":40,
        "active_run":if prepared {run.clone()} else {Value::Null},
        "runs":if prepared {vec![run]} else {vec![]}});
    view
}
fn clock_success() -> Value {
    json!({"ok":true,"development_production_enabled":false,"development_clock_enabled":true})
}
fn clock_output(mode: &str, result: Value, view: Value, ticks: u64, uncertain: bool) -> Value {
    serde_json::from_str(&packet(
        "fortress.query",
        Some(SessionId::new(9)),
        Some(original_anchor()),
        Some(mode),
        Some(RequestId::new(3)),
        Some(WorkBudget {
            max_game_ticks: ticks,
            ..WorkBudget::default()
        }),
        result,
        Some(view),
        uncertain,
    ))
    .expect("complete clock JSON")
}
fn clock_enabled(value: &Value, action: &str) -> bool {
    value["agent_turn"]["affordances"]
        .as_array()
        .expect("affordances")
        .iter()
        .find(|v| v["action"] == action && v["scope"] == "run")
        .expect("clock affordance")["enabled"]
        == true
}

#[test]
fn clock_only_control_proposes_closed_bounded_run_without_production_grant() {
    let view = clock_retained(false);
    let p = clock_output("control", clock_success(), view, 1200, false);
    assert!(clock_enabled(&p, "fortress.plan"));
    assert!(!enabled(&p, "fortress.plan"));
    assert_eq!(
        p["agent_turn"]["briefing"]["development_production_enabled"],
        false
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["development_clock_enabled"],
        true
    );
    let proposal = &p["agent_turn"]["recommendations"][0];
    assert_eq!(proposal["tool"], "fortress.plan");
    let request: Value = serde_json::from_str(
        proposal["arguments"]["request"]
            .as_str()
            .expect("request JSON"),
    )
    .expect("closed request");
    assert_eq!(request["kind"], "run");
    assert_eq!(request["original_paired_seal"], "c".repeat(64));
    assert_eq!(request["game_ticks"], 100);
    assert_eq!(request.as_object().expect("closed request").len(), 4);
    assert_eq!(proposal["requires_confirmation"], false);
    assert_eq!(
        p["agent_turn"]["budget"]["admitted"]["max_game_ticks"],
        1200
    );
}

#[test]
fn prepared_clock_exposes_separate_review_and_confirmation_scope() {
    let view = clock_retained(true);
    let p = clock_output("control", clock_success(), view.clone(), 1200, false);
    assert!(clock_enabled(&p, "fortress.commit"));
    assert!(!enabled(&p, "fortress.commit"));
    assert!(!clock_enabled(&p, "fortress.plan"));
    let next = &p["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.commit");
    assert_eq!(next["arguments"]["scope"], "run");
    assert_eq!(next["arguments"]["paired_seal"], "2".repeat(64));
    assert_eq!(next["arguments"]["confirmed"], true);
    let confirmation = &p["agent_turn"]["active_work"]["confirmations"][0];
    assert_eq!(confirmation["scope"], "run");
    assert_eq!(confirmation["commit_available"], true);
    assert_eq!(
        p["agent_turn"]["active_work"]["retained_identity"]["recovery"],
        view["recovery"]
    );
}

#[test]
fn clock_grant_budget_source_and_mode_are_required_for_commit_affordance() {
    for case in 0..7 {
        let mut view = clock_retained(true);
        let mut result = clock_success();
        let mut ticks = 1200;
        let mut mode = "control";
        match case {
            0 => result["development_clock_enabled"] = json!(false),
            1 => ticks = 0,
            2 => mode = "reconcile",
            3 => view["clock"]["mode"] = json!("reconcile"),
            4 => view["recovery"]["clock_journal_id"] = json!("6".repeat(64)),
            5 => view["clock"]["active_run"]["before_tick"] = json!(39),
            _ => view["clock"]["high_tick"] = json!(41),
        }
        let p = clock_output(mode, result, view, ticks, false);
        assert!(!clock_enabled(&p, "fortress.commit"), "case {case}");
    }
}

#[test]
fn clock_receipts_never_complete_the_original_goal_or_prove_present_pause() {
    let mut view = clock_retained(false);
    view["clock"]["run_count"] = json!(1);
    view["clock"]["runs"] = json!([{"native":{"phase":"stopped","historical_pause_verified":true,
        "predicate_observed":true,"original_goal_proven":true}}]);
    let p = clock_output("control", clock_success(), view, 1200, false);
    assert_eq!(
        p["agent_turn"]["briefing"]["original_goal_proven_at_anchor"],
        false
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        false
    );
    assert_eq!(p["agent_turn"]["briefing"]["current_pause_proven"], false);
    assert_eq!(
        p["agent_turn"]["briefing"]["clock_receipt_is_goal_proof"],
        false
    );
    assert!(p["agent_turn"]["briefing"]["clock_advanced"].is_null());
}

#[test]
fn unresolved_clock_requires_reconciliation_and_never_another_dispatch() {
    let mut view = clock_retained(true);
    view["clock"]["active_run"]["state"] = json!("tracking");
    view["clock"]["active_run"]["unresolved"] = json!(true);
    view["clock"]["unresolved"] = json!(true);
    view["clock"]["may_be_running"] = json!(true);
    let p = clock_output("control", clock_success(), view, 1200, false);
    assert_eq!(p["agent_turn"]["continuity"]["status"], "indeterminate");
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert!(!clock_enabled(&p, "fortress.commit"));
    assert!(!clock_enabled(&p, "fortress.plan"));
    assert!(clock_enabled(&p, "fortress.cancel"));
    assert_eq!(
        p["agent_turn"]["active_work"]["indeterminate_effects"][0]["effect_may_have_started"],
        true
    );
    assert_eq!(
        p["agent_turn"]["active_work"]["indeterminate_effects"][0]["clock_may_be_running"],
        true
    );
}

#[test]
fn cached_clock_failure_and_session_close_keep_handoff_without_authority() {
    let view = clock_retained(true);
    for closed in [false, true] {
        let result = json!({"ok":closed,"closed":closed,"retained_state_currently_verified":false,"development_clock_enabled":true});
        let p = clock_output("control", result, view.clone(), 1200, true);
        assert!(!clock_enabled(&p, "fortress.commit"));
        assert!(!clock_enabled(&p, "fortress.plan"));
        assert!(!clock_enabled(&p, "fortress.cancel"));
        assert_eq!(
            p["retained_state"]["recovery"]["clock_journal_id"],
            "1".repeat(64)
        );
        assert!(
            p["agent_turn"]["active_work"]["pending_plans"]
                .as_array()
                .expect("plans")
                .is_empty()
        );
        if closed {
            assert!(
                p["agent_turn"]["recommendations"]
                    .as_array()
                    .expect("recommendations")
                    .is_empty()
            );
        } else {
            assert_eq!(
                p["agent_turn"]["recommendations"][0]["tool"],
                "fortress.query"
            );
        }
    }
}

#[test]
fn original_completion_and_deadline_prevent_unnecessary_clock_proposals() {
    for case in 0..4 {
        let mut view = clock_retained(false);
        let mut ticks = 1200;
        match case {
            0 => {
                view["current_goal"]["original_goal_proven"] = json!(true);
                view["current_goal"]["physical_quiescence_proven"] = json!(true);
                view["current_goal"]["semantic_completion_proven"] = json!(true);
            }
            1 => view["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(40),
            2 => ticks = 0,
            _ => view["current_goal"]["current_truth"] = json!("unknown"),
        }
        let p = clock_output("control", clock_success(), view, ticks, false);
        assert!(!clock_enabled(&p, "fortress.plan"), "case {case}");
        assert!(!clock_enabled(&p, "fortress.commit"));
    }
}

#[test]
fn clock_overflow_preserves_exact_journal_and_active_review_without_false_proof() {
    let mut view = clock_retained(true);
    view["clock"]["unexpected_private_configuration"] = json!("must-not-survive-clock-fallback");
    let raw = packet(
        "fortress.commit",
        Some(SessionId::new(9)),
        Some(original_anchor()),
        Some("control"),
        Some(RequestId::new(3)),
        Some(WorkBudget {
            max_game_ticks: 1200,
            ..WorkBudget::default()
        }),
        json!({"ok":true,"development_clock_enabled":true,"huge":"x".repeat(2*RESPONSE_RESERVE as usize)}),
        Some(view.clone()),
        true,
    );
    assert!(raw.len() as u64 <= RESPONSE_RESERVE);
    assert!(!raw.contains("must-not-survive-clock-fallback"));
    let p: Value = serde_json::from_str(&raw).expect("complete bounded clock handoff");
    assert_eq!(p["retained_state"]["recovery"], view["recovery"]);
    let clock = &p["retained_state"]["clock"];
    assert_eq!(clock["journal_id"], view["clock"]["journal_id"]);
    for key in ["review_seal", "key", "plan_digest"] {
        assert_eq!(clock["active_run"][key], view["clock"]["active_run"][key]);
    }
    assert!(!clock_enabled(&p, "fortress.commit"));
    assert!(!clock_enabled(&p, "fortress.plan"));
    assert_eq!(p["result"]["effect_may_have_started"], true);
    assert!(p["retained_state"]["current_goal"].is_null());
    assert!(
        !recovery(&retained())
            .expect("legacy handoff")
            .as_object()
            .expect("object")
            .contains_key("clock_journal_id")
    );
}
