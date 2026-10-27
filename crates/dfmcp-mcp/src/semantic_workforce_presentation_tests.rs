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

fn clock_budget(game_ticks: u64) -> WorkBudget {
    WorkBudget {
        max_wall_millis: 5000,
        max_bytes: 2 * 1024 * 1024 * 1024,
        max_output_tokens: 32768,
        max_entities: 8192,
        max_actions: 1,
        max_game_ticks: game_ticks,
    }
}
fn clock_success() -> Value {
    json!({"ok":true,"development_labor_enabled":false,"development_clock_enabled":true})
}
fn clock_view() -> Value {
    let mut view = applied();
    view["recovery"]["clock_journal_id"] = json!("2".repeat(64));
    view["current_goal"] = json!({"anchor":anchor_value(original_anchor()),"current_truth":"true",
        "original_goal_proven":false,"semantic_completion_proven":false});
    view["clock"] = json!({"journal_id":"2".repeat(64),"head":"3".repeat(64),
        "mode":"control","target_digest":"4".repeat(64),"original_paired_seal":"c".repeat(64),
        "run_count":0,"pending":false,"unresolved":false,"may_be_running":false,
        "needs_reconciliation":false,"high_tick":40,"high_source_sequence":0,
        "source_identity_semantics":"source_incarnation_only_not_named_fortress_lineage",
        "current_pause_proven":false,"original_goal_proven":false,"active_run":null,"runs":[]});
    view
}
fn prepared_run() -> Value {
    json!({"review_seal":"5".repeat(64),"key":"original-labor-run-001",
        "plan_digest":"6".repeat(64),"state":"prepared","before_tick":40,
        "source_generation":1001,"source_sequence":0,"game_ticks":10,"wall_millis":1000,
        "purpose":"advance_for_original_labor_stability","pending":true,"unresolved":false,
        "native":{"phase":"prepared","reason":"none","receipt":"7".repeat(64),
            "unpause_attempted":false,"historical_pause_verified":false,"observed_tick":40,
            "observed_tick_overshoot":0}})
}
fn clock_output(
    operation: &str,
    mode: &str,
    result: Value,
    view: Value,
    budget: WorkBudget,
    uncertain: bool,
) -> Value {
    serde_json::from_str(&packet(
        operation,
        Some(SessionId::new(9)),
        Some(original_anchor()),
        Some(mode),
        Some(RequestId::new(3)),
        Some(budget),
        result,
        Some(view),
        uncertain,
    ))
    .expect("complete clock Agent Turn JSON")
}
fn clock_enabled(value: &Value, action: &str) -> bool {
    value["agent_turn"]["affordances"]
        .as_array()
        .expect("affordances")
        .iter()
        .find(|v| v["action"] == action && v["scope"] == "run")
        .is_some_and(|v| v["enabled"] == true)
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

#[test]
fn current_true_labor_recommends_one_cadence_slice_with_independent_clock_authority() {
    let p = clock_output(
        "fortress.wait",
        "control",
        clock_success(),
        clock_view(),
        clock_budget(1200),
        false,
    );
    assert!(!enabled(&p, "fortress.commit"));
    assert!(clock_enabled(&p, "fortress.plan"));
    assert!(!clock_enabled(&p, "fortress.commit"));
    let next = &p["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.plan");
    assert_eq!(next["requires_confirmation"], false);
    let request: Value = serde_json::from_str(
        next["arguments"]["request"]
            .as_str()
            .expect("closed request"),
    )
    .expect("request JSON");
    assert_eq!(
        request,
        json!({"kind":"run","original_paired_seal":"c".repeat(64),
        "game_ticks":10,"wall_millis":5000})
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["development_clock_enabled"],
        true
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["development_labor_enabled"],
        false
    );
    assert_eq!(
        p["agent_turn"]["budget"]["admitted"]["max_game_ticks"],
        1200
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["clock_source_identifies_named_fortress"],
        false
    );
    assert_eq!(
        p["agent_turn"]["briefing"]["clock_receipt_is_goal_proof"],
        false
    );
}

#[test]
fn clock_suggestions_narrow_to_remaining_horizon_and_negotiated_allowance() {
    let mut view = clock_view();
    view["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(47);
    for (ticks, expected) in [(1200, 7), (4, 4)] {
        let p = clock_output(
            "fortress.query",
            "control",
            clock_success(),
            view.clone(),
            clock_budget(ticks),
            false,
        );
        let request: Value = serde_json::from_str(
            p["agent_turn"]["recommendations"][0]["arguments"]["request"]
                .as_str()
                .expect("request"),
        )
        .expect("JSON");
        assert_eq!(request["game_ticks"], expected);
    }
    for deadline in [0, 39, 40] {
        view["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(deadline);
        let p = clock_output(
            "fortress.query",
            "control",
            clock_success(),
            view.clone(),
            clock_budget(1200),
            false,
        );
        assert!(!clock_enabled(&p, "fortress.plan"));
    }
}

#[test]
fn clock_controls_require_independent_current_authority_budget_and_linked_original_custody() {
    for (mode, result, ticks) in [
        ("control", success(), 1200),
        ("control", clock_success(), 0),
        ("reconcile", clock_success(), 1200),
        ("offline", clock_success(), 1200),
    ] {
        let p = clock_output(
            "fortress.query",
            mode,
            result,
            clock_view(),
            clock_budget(ticks),
            false,
        );
        assert!(!clock_enabled(&p, "fortress.plan"));
        assert!(!clock_enabled(&p, "fortress.commit"));
    }
    for (field, value) in [
        ("mode", json!("reconcile")),
        ("original_paired_seal", json!("8".repeat(64))),
        ("target_digest", Value::Null),
        ("journal_id", json!("9".repeat(64))),
    ] {
        let mut view = clock_view();
        view["clock"][field] = value;
        let p = clock_output(
            "fortress.query",
            "control",
            clock_success(),
            view,
            clock_budget(1200),
            false,
        );
        assert!(!clock_enabled(&p, "fortress.plan"), "{field}");
        assert!(!clock_enabled(&p, "fortress.commit"), "{field}");
    }
}

#[test]
fn time_is_not_recommended_to_repair_false_unknown_or_unverified_labor() {
    for truth in ["false", "unknown"] {
        let mut view = clock_view();
        view["current_goal"]["current_truth"] = json!(truth);
        let p = clock_output(
            "fortress.wait",
            "control",
            clock_success(),
            view,
            clock_budget(1200),
            false,
        );
        assert!(!clock_enabled(&p, "fortress.plan"), "{truth}");
    }
    for action in ["unverified", "applied_outside_semantics"] {
        let mut view = clock_view();
        view["action_result"] = json!({"verified":false,"state":action,"receipt":"9".repeat(64)});
        let p = clock_output(
            "fortress.wait",
            "control",
            clock_success(),
            view,
            clock_budget(1200),
            false,
        );
        assert!(!clock_enabled(&p, "fortress.plan"), "{action}");
    }
    let mut view = clock_view();
    view["native_phase"] = json!("unknown");
    view["effect_uncertain"] = json!(true);
    let p = clock_output(
        "fortress.wait",
        "control",
        clock_success(),
        view,
        clock_budget(1200),
        false,
    );
    assert!(!clock_enabled(&p, "fortress.plan"));
    assert_eq!(
        p["agent_turn"]["recommendations"][0]["tool"],
        "fortress.explain"
    );
}

#[test]
fn a_fresh_matching_goal_sample_is_required_before_another_run() {
    for mutation in 0..4 {
        let mut view = clock_view();
        match mutation {
            0 => view["current_goal"] = Value::Null,
            1 => view["current_goal"]["anchor"]["tick"] = json!(39),
            2 => view["current_goal"]["anchor"]["state_hash"] = json!("8".repeat(64)),
            _ => view["clock"]["high_tick"] = json!(41),
        }
        let p = clock_output(
            "fortress.query",
            "control",
            clock_success(),
            view,
            clock_budget(1200),
            false,
        );
        assert!(!clock_enabled(&p, "fortress.plan"));
        assert_eq!(
            p["agent_turn"]["recommendations"][0]["tool"],
            "fortress.wait"
        );
    }
}

#[test]
fn prepared_clock_review_has_its_own_confirmation_and_original_custody() {
    let mut view = clock_view();
    let run = prepared_run();
    view["clock"]["active_run"] = run.clone();
    view["clock"]["run_count"] = json!(1);
    view["clock"]["pending"] = json!(true);
    let p = clock_output(
        "fortress.plan",
        "control",
        clock_success(),
        view.clone(),
        clock_budget(1200),
        false,
    );
    assert!(!clock_enabled(&p, "fortress.plan"));
    assert!(clock_enabled(&p, "fortress.commit"));
    assert!(clock_enabled(&p, "fortress.cancel"));
    let next = &p["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.commit");
    assert_eq!(next["arguments"]["scope"], "run");
    assert_eq!(next["arguments"]["paired_seal"], run["review_seal"]);
    assert_ne!(
        next["arguments"]["paired_seal"],
        view["review"]["paired_seal"]
    );
    assert_eq!(next["requires_confirmation"], true);
    assert_eq!(
        p["agent_turn"]["active_work"]["pending_plans"][0]["recovery"],
        view["recovery"]
    );
    assert_eq!(
        p["agent_turn"]["active_work"]["confirmations"][0]["scope"],
        "run"
    );
    assert_eq!(
        p["agent_turn"]["active_work"]["actions"][0]["action_result"]["verified"],
        true
    );
    assert_eq!(p["agent_turn"]["active_work"]["actions"][1]["scope"], "run");
}

#[test]
fn clock_commit_affordance_refuses_stale_witness_horizon_or_budget() {
    for mutation in 0..6 {
        let mut view = clock_view();
        view["clock"]["active_run"] = prepared_run();
        view["clock"]["pending"] = json!(true);
        view["clock"]["run_count"] = json!(1);
        let mut budget = clock_budget(1200);
        match mutation {
            0 => view["clock"]["active_run"]["before_tick"] = json!(39),
            1 => view["clock"]["high_tick"] = json!(41),
            2 => view["clock"]["high_source_sequence"] = json!(1),
            3 => budget.max_game_ticks = 9,
            4 => view["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(49),
            _ => view["current_goal"]["current_truth"] = json!("false"),
        }
        let p = clock_output(
            "fortress.query",
            "control",
            clock_success(),
            view,
            budget,
            false,
        );
        assert!(!clock_enabled(&p, "fortress.commit"), "case {mutation}");
        assert!(!clock_enabled(&p, "fortress.plan"), "case {mutation}");
        assert!(clock_enabled(&p, "fortress.cancel"), "case {mutation}");
    }
}

#[test]
fn unsettled_clock_is_reconciled_before_labor_sampling_or_new_control() {
    for (state, phase) in [
        ("dispatch_started", "prepared"),
        ("tracking", "running"),
        ("cancel_requested", "stopping"),
        ("terminal", "source_lost"),
    ] {
        let mut view = clock_view();
        let mut run = prepared_run();
        run["state"] = json!(state);
        run["native"]["phase"] = json!(phase);
        run["native"]["unpause_attempted"] = json!(true);
        run["unresolved"] = json!(true);
        view["clock"]["active_run"] = run;
        view["clock"]["pending"] = json!(true);
        view["clock"]["unresolved"] = json!(true);
        view["clock"]["may_be_running"] = json!(true);
        let p = clock_output(
            "fortress.wait",
            "control",
            clock_success(),
            view,
            clock_budget(1200),
            false,
        );
        assert_eq!(p["agent_turn"]["continuity"]["status"], "indeterminate");
        assert_eq!(p["agent_turn"]["briefing"]["clock_may_be_running"], true);
        assert_eq!(
            p["agent_turn"]["recommendations"][0]["tool"],
            "fortress.wait"
        );
        assert!(!clock_enabled(&p, "fortress.plan"));
        assert!(!clock_enabled(&p, "fortress.commit"));
        assert_eq!(
            p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
            false
        );
    }
}

#[test]
fn historical_clock_stop_never_substitutes_for_original_labor_evidence() {
    let mut view = clock_view();
    view["current_goal"] = Value::Null;
    view["clock"]["run_count"] = json!(1);
    view["clock"]["high_tick"] = json!(52);
    let mut run = prepared_run();
    run["state"] = json!("terminal");
    run["native"] = json!({"phase":"stopped","reason":"tick_limit","receipt":"7".repeat(64),
        "unpause_attempted":true,"historical_pause_verified":true,"observed_tick":52,"observed_tick_overshoot":2});
    view["clock"]["runs"] = json!([run]);
    let p = clock_output(
        "fortress.wait",
        "control",
        clock_success(),
        view,
        clock_budget(1200),
        false,
    );
    assert!(p["agent_turn"]["briefing"]["original_goal_proven_at_anchor"].is_null());
    assert!(p["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert!(p["agent_turn"]["briefing"]["clock_advanced"].is_null());
    assert_eq!(p["agent_turn"]["briefing"]["current_pause_proven"], false);
    assert_eq!(p["agent_turn"]["anchor"], anchor_value(original_anchor()));
    assert!(!clock_enabled(&p, "fortress.plan"));
}

#[test]
fn original_completion_and_clock_capacity_stop_further_time_exposure() {
    let mut view = clock_view();
    view["original_plan"]["steps"][0]["obligation"]["stable_samples"] = json!(64);
    view["clock"]["run_count"] = json!(MAX_RUNS - 1);
    let p = clock_output(
        "fortress.query",
        "control",
        clock_success(),
        view.clone(),
        clock_budget(1200),
        false,
    );
    assert!(clock_enabled(&p, "fortress.plan"));
    view["clock"]["run_count"] = json!(MAX_RUNS);
    let p = clock_output(
        "fortress.query",
        "control",
        clock_success(),
        view.clone(),
        clock_budget(1200),
        false,
    );
    assert!(!clock_enabled(&p, "fortress.plan"));
    view["clock"]["run_count"] = json!(0);
    view["current_goal"]["original_goal_proven"] = json!(true);
    view["current_goal"]["semantic_completion_proven"] = json!(true);
    let p = clock_output(
        "fortress.wait",
        "control",
        clock_success(),
        view,
        clock_budget(1200),
        false,
    );
    assert!(!clock_enabled(&p, "fortress.plan"));
    assert!(!clock_enabled(&p, "fortress.commit"));
    assert!(
        p["agent_turn"]["recommendations"]
            .as_array()
            .expect("recommendations")
            .is_empty()
    );
}

#[test]
fn clock_recovery_and_unpause_uncertainty_survive_error_close_and_overflow() {
    let mut view = clock_view();
    view["clock"]["active_run"] = prepared_run();
    view["clock_effect_may_have_started"] = json!(true);
    view["clock"]["unbounded_native_detail"] = json!("x".repeat(RESPONSE_RESERVE as usize));
    for (operation, result) in [
        (
            "fortress.commit",
            json!({"ok":false,"clock_effect_may_have_started":true,
            "retained_state_currently_verified":false}),
        ),
        (
            "fortress.cancel",
            json!({"ok":true,"closed":true,"effects_cancelled":false,
            "retained_state_currently_verified":false}),
        ),
    ] {
        let p = clock_output(
            operation,
            "control",
            result,
            view.clone(),
            clock_budget(1200),
            true,
        );
        assert!(p.to_string().len() as u64 <= RESPONSE_RESERVE);
        assert_eq!(p["result"]["details_omitted"], true);
        assert_eq!(p["retained_state"]["recovery"], view["recovery"]);
        assert_eq!(
            p["retained_state"]["clock"]["journal_id"],
            view["clock"]["journal_id"]
        );
        assert_eq!(
            p["retained_state"]["clock"]["active_run"]["key"],
            view["clock"]["active_run"]["key"]
        );
        assert_eq!(
            p["retained_state"]["clock"]["active_run"]["review_seal"],
            view["clock"]["active_run"]["review_seal"]
        );
        assert_eq!(p["retained_state"]["clock_effect_may_have_started"], true);
        assert!(
            p["retained_state"]["clock"]
                .get("unbounded_native_detail")
                .is_none()
        );
        assert_eq!(p["agent_turn"]["briefing"]["clock_may_be_running"], true);
        assert!(!clock_enabled(&p, "fortress.commit"));
        assert!(!clock_enabled(&p, "fortress.cancel"));
        assert!(p["agent_turn"]["briefing"].get("admission").is_none());
    }
}

#[test]
fn startup_clock_identity_survives_failure_without_an_original_goal_or_control() {
    let mut view = clock_view();
    view["ok"] = json!(false);
    for key in ["original_plan", "recovery", "review", "current_goal"] {
        view[key] = Value::Null;
    }
    view["clock"]["target_digest"] = Value::Null;
    view["clock"]["original_paired_seal"] = Value::Null;
    let p = clock_output(
        "fortress.open_session",
        "control",
        json!({"ok":false}),
        view.clone(),
        clock_budget(1200),
        false,
    );
    assert_eq!(
        p["agent_turn"]["references"][0]["clock"]["journal_id"],
        view["clock"]["journal_id"]
    );
    assert!(p["agent_turn"]["references"][0]["recovery"].is_null());
    assert_eq!(p["agent_turn"]["active_work"]["state_known"], false);
    assert!(!clock_enabled(&p, "fortress.plan"));
    assert!(!clock_enabled(&p, "fortress.commit"));
}
