use super::*;

fn source() -> Value {
    let anchor =
        json!({"fortress_id":"7","epoch":0,"sequence":3,"tick":100,"state_hash":"a".repeat(64)});
    json!({"ok":true,"retained_state_currently_verified":true,"mode":"control",
        "anchor":anchor,"journal_id":"b".repeat(64),
        "review":{"paired_seal":"c".repeat(64),"native_plan_digest":"d".repeat(64),
            "semantic_plan_digest":"e".repeat(64),"original_object_digest":"f".repeat(64)},
        "recovery":{"journal_id":"b".repeat(64),"plan_digest":"e".repeat(64),"object_digest":"f".repeat(64),
            "anchor":anchor,"history_present":false},
        "original_plan":{"plan_digest":"e".repeat(64),"anchor":anchor,
            "steps":[{"obligation":{"deadline_tick":200,"poll_interval_ticks":5,"stable_samples":2}}]},
        "development_placement_enabled":true,"prepared_in_this_opening":true,
        "native_state":"prepared","native_phase":"prepared","native_needs_query":true,"effect_uncertain":false,
        "placement":{"verified":false},"current_goal":null,"current_goal_available":false,"history":{"fenced":false}})
}
fn run(
    operation: &str,
    result: Value,
    retained: Option<Value>,
    uncertain: bool,
) -> Result<Value, serde_json::Error> {
    serde_json::from_str(&packet(
        operation,
        Some(SessionId::new(77)),
        None,
        Some("control"),
        Some(RequestId::new(8)),
        None,
        result,
        retained,
        uncertain,
    ))
}
#[test]
fn current_exact_preparation_recommends_separately_confirmed_paired_seal()
-> Result<(), serde_json::Error> {
    let packet = run("fortress.plan", source(), None, false)?;
    let recommendation = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(recommendation["tool"], "fortress.commit");
    assert_eq!(recommendation["requires_confirmation"], true);
    assert_eq!(recommendation["arguments"]["confirmed"], true);
    assert_eq!(recommendation["arguments"]["paired_seal"], "c".repeat(64));
    assert_eq!(
        packet["agent_turn"]["briefing"]["mutation_admissible"],
        false
    );
    Ok(())
}
#[test]
fn recovered_preparation_and_ambiguous_dispatch_never_recommend_redispatch()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    let reopened = run("fortress.query", view.clone(), None, false)?;
    assert_eq!(
        reopened["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    view["effect_uncertain"] = json!(true);
    view["native_state"] = json!("dispatch_started");
    let ambiguous = run("fortress.commit", view, None, true)?;
    assert_eq!(ambiguous["agent_turn"]["phase"], "reconcile");
    assert_eq!(
        ambiguous["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert!(
        !ambiguous["agent_turn"]["active_work"]["indeterminate_effects"]
            .as_array()
            .is_none_or(|a| a.is_empty())
    );
    Ok(())
}
#[test]
fn queried_recovered_preparation_recommends_explicit_retirement_with_original_identity()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    view["native_state"] = json!("tracking");
    let packet = run("fortress.wait", view, None, false)?;
    let next = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.cancel");
    assert_eq!(next["arguments"]["scope"], "goal");
    assert_eq!(packet["result"]["review"]["paired_seal"], "c".repeat(64));
    assert_eq!(
        packet["result"]["recovery"]["object_digest"],
        "f".repeat(64)
    );
    assert_eq!(packet["result"]["prepared_in_this_opening"], false);
    assert!(
        packet["agent_turn"]["active_work"]["confirmations"]
            .as_array()
            .is_some_and(|items| items.is_empty())
    );
    Ok(())
}
#[test]
fn placement_is_not_original_construction_completion() -> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("placed");
    view["native_needs_query"] = json!(false);
    view["placement"] = json!({"verified":true,"receipt":"a".repeat(64)});
    let packet = run("fortress.commit", view, None, false)?;
    assert_eq!(
        packet["agent_turn"]["briefing"]["placement_verified_at_anchor"],
        true
    );
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert_eq!(
        packet["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert_eq!(
        packet["agent_turn"]["briefing"]["placement_receipt_is_goal_proof"],
        false
    );
    assert!(packet["agent_turn"]["active_work"]["obligations"][0]["complete_at_anchor"].is_null());
    assert_eq!(
        packet["agent_turn"]["active_work"]["obligations"][0]["currently_verified"],
        false
    );
    Ok(())
}
#[test]
fn unavailable_custody_withdraws_current_goal_and_uses_executable_inspection_arguments()
-> Result<(), serde_json::Error> {
    let mut retained = source();
    retained["current_goal_available"] = json!(true);
    retained["current_goal"] =
        json!({"original_goal_proven":true,"semantic_completion_proven":true});
    let packet = run(
        "fortress.wait",
        json!({"ok":false,"error":{"code":"capability_denied"}}),
        Some(retained),
        false,
    )?;
    assert_eq!(packet["agent_turn"]["continuity"]["status"], "stale");
    assert!(packet["agent_turn"]["briefing"]["original_goal_proven_at_anchor"].is_null());
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    let next = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.query");
    assert!(next["arguments"].get("query").is_none());
    assert_eq!(packet["retained_reference_current"], false);
    Ok(())
}
#[test]
fn overflow_preserves_exact_recovery_and_suppresses_all_goal_claims()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["original_plan"]["summary"] = json!("oversized".repeat(50_000));
    let encoded = packet(
        "fortress.commit",
        Some(SessionId::new(77)),
        None,
        Some("control"),
        None,
        None,
        view,
        None,
        true,
    );
    assert!(encoded.len() as u64 <= RESPONSE_RESERVE);
    let packet: Value = serde_json::from_str(&encoded)?;
    assert_eq!(packet["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(
        packet["retained_reference"]["recovery"]["object_digest"],
        "f".repeat(64)
    );
    assert!(
        packet["retained_reference"]["recovery"]
            .get("archive_publication_unknown")
            .is_none()
    );
    assert!(
        packet["retained_reference"]["recovery"]
            .get("paired_preparation_present")
            .is_none()
    );
    assert_eq!(packet["result"]["current_goal_available"], false);
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    Ok(())
}
#[test]
fn every_result_has_complete_packet_and_never_claims_clock_advancement()
-> Result<(), serde_json::Error> {
    let packet = run("fortress.doctor", source(), None, false)?;
    for field in [
        "schema",
        "operation",
        "phase",
        "session_id",
        "request_id",
        "anchor",
        "continuity",
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
        assert!(packet["agent_turn"].get(field).is_some());
    }
    assert_eq!(packet["agent_turn"]["briefing"]["clock_advanced"], false);
    assert_eq!(packet["agent_turn"]["briefing"]["runtime_admitted"], false);
    Ok(())
}

#[test]
fn released_session_stays_closed_in_normal_and_overflow_packets() -> Result<(), serde_json::Error> {
    for oversized in [false, true] {
        let mut view = source();
        view["session_closed"] = json!(true);
        if oversized {
            view["original_plan"]["summary"] = json!("oversized".repeat(50_000));
        }
        let packet = run("fortress.cancel", view, None, false)?;
        assert_eq!(packet["result"]["session_closed"], true);
        assert_eq!(packet["agent_turn"]["briefing"]["session_closed"], true);
        assert!(
            packet["agent_turn"]["recommendations"]
                .as_array()
                .is_some_and(|items| items.is_empty())
        );
        assert!(
            packet["agent_turn"]["affordances"]
                .as_array()
                .is_some_and(|items| items.iter().all(|v| v["enabled"] != true))
        );
        assert_eq!(
            packet["agent_turn"]["references"][0]["recovery"]["object_digest"],
            "f".repeat(64)
        );
    }
    Ok(())
}

#[test]
fn fenced_goal_preserves_history_and_recommends_session_release_instead_of_more_waits()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("placed");
    view["native_needs_query"] = json!(false);
    view["prepared_in_this_opening"] = json!(false);
    view["placement"]["verified"] = json!(true);
    view["history"]["fenced"] = json!(true);
    view["current_goal_available"] = json!(true);
    view["current_goal"] = json!({"semantic_completion_proven":true,"original_goal_proven":true});
    view["first_satisfied_anchor"] = view["anchor"].clone();
    let packet = run("fortress.query", view.clone(), None, false)?;
    let next = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.cancel");
    assert_eq!(next["arguments"]["scope"], "session");
    assert!(packet["agent_turn"]["briefing"]["original_goal_proven_at_anchor"].is_null());
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert!(packet["agent_turn"]["active_work"]["obligations"][0]["complete_at_anchor"].is_null());
    assert_eq!(
        packet["agent_turn"]["active_work"]["obligations"][0]["currently_verified"],
        false
    );
    view["original_plan"]["summary"] = json!("oversized".repeat(50_000));
    let fallback = run("fortress.query", view, None, false)?;
    assert_eq!(
        fallback["retained_reference"]["first_satisfied_anchor"]["tick"],
        100
    );
    assert!(fallback["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    Ok(())
}

fn construction_clock_source() -> Value {
    let mut view = source();
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("placed");
    view["native_needs_query"] = json!(false);
    view["prepared_in_this_opening"] = json!(false);
    view["placement"] = json!({"verified":true,"receipt":"a".repeat(64)});
    view["development_clock_enabled"] = json!(true);
    view["recovery"]["history_present"] = json!(true);
    view["recovery"]["clock_journal_id"] = json!("1".repeat(64));
    view["current_goal_available"] = json!(true);
    view["current_goal"] = json!({"anchor":view["anchor"],"current_truth":"false",
        "receipt_linked":true,"construction_condition":"pending","construction_proven":false,
        "original_goal_proven":false,"semantic_completion_proven":false,"failure_predicate":null});
    view["clock"] = json!({"journal_id":"1".repeat(64),"head":"2".repeat(64),"mode":"control",
        "target_digest":"3".repeat(64),"original_paired_seal":"c".repeat(64),
        "run_count":0,"pending":false,"unresolved":false,"may_be_running":false,
        "needs_reconciliation":false,"high_tick":100,"high_source_sequence":4,"active_run":null});
    view
}
fn prepare_clock(view: &mut Value) {
    view["clock"]["pending"] = json!(true);
    view["clock"]["run_count"] = json!(1);
    view["clock"]["active_run"] = json!({"review_seal":"8".repeat(64),"plan_digest":"9".repeat(64),
        "key":format!("furniture-clock-{}","7".repeat(64)),"state":"prepared",
        "before_tick":100,"source_generation":7,"source_sequence":4,"game_ticks":5,"wall_millis":1000,
        "pending":true,"unresolved":false,"native":{"phase":"prepared","reason":"none",
            "receipt":"6".repeat(64),"unpause_attempted":false,"historical_pause_verified":false}});
}
fn clock_packet(operation: &str, view: Value, ticks: u64) -> Result<Value, serde_json::Error> {
    serde_json::from_str(&packet(
        operation,
        Some(SessionId::new(77)),
        None,
        Some("control"),
        Some(RequestId::new(8)),
        Some(WorkBudget {
            max_game_ticks: ticks,
            max_wall_millis: 5000,
            max_bytes: 2 * 1024 * 1024 * 1024,
            max_entities: 8192,
            max_actions: 1,
            max_output_tokens: 32768,
        }),
        view,
        None,
        false,
    ))
}
fn offers_clock(packet: &Value, tool: &str) -> bool {
    packet["agent_turn"]["affordances"]
        .as_array()
        .is_some_and(|items| {
            items
                .iter()
                .any(|v| v["scope"] == "run" && v["tool"] == tool && v["enabled"] == true)
        })
}

#[test]
fn viable_incomplete_construction_proposes_only_a_bounded_original_cadence_run()
-> Result<(), serde_json::Error> {
    for (ticks, expected) in [(3, 3), (1200, 5)] {
        let view = construction_clock_source();
        let out = clock_packet("fortress.wait", view, ticks)?;
        let next = &out["agent_turn"]["recommendations"][0];
        assert_eq!(next["tool"], "fortress.plan");
        let raw: String = serde_json::from_value(next["arguments"]["request"].clone())?;
        let request: Value = serde_json::from_str(&raw)?;
        assert_eq!(request["kind"], "run");
        assert_eq!(request["original_paired_seal"], "c".repeat(64));
        assert_eq!(request["game_ticks"], expected);
        assert_eq!(next["requires_confirmation"], false);
        assert!(offers_clock(&out, "fortress.plan"));
        assert_eq!(
            out["agent_turn"]["briefing"]["original_goal_proven_at_anchor"],
            false
        );
    }
    let mut deadline = construction_clock_source();
    deadline["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(102);
    let out = clock_packet("fortress.wait", deadline, 1200)?;
    let raw: String = serde_json::from_value(
        out["agent_turn"]["recommendations"][0]["arguments"]["request"].clone(),
    )?;
    let request: Value = serde_json::from_str(&raw)?;
    assert_eq!(request["game_ticks"], 2);
    Ok(())
}

#[test]
fn clock_offers_require_current_viable_evidence_scope_allowance_and_original_deadline()
-> Result<(), serde_json::Error> {
    let mut cases = Vec::new();
    for condition in [
        "suspended",
        "no_construction_job",
        "item_unverified",
        "removal_pending",
        "building_identity_mismatch",
    ] {
        let mut v = construction_clock_source();
        v["current_goal"]["construction_condition"] = json!(condition);
        cases.push(v);
    }
    for (field, value) in [
        ("current_truth", json!("unknown")),
        ("receipt_linked", json!(false)),
        ("failure_predicate", json!("true")),
        ("semantic_completion_proven", json!(true)),
    ] {
        let mut v = construction_clock_source();
        v["current_goal"][field] = value;
        cases.push(v);
    }
    let mut v = construction_clock_source();
    v["current_goal"]["anchor"]["sequence"] = json!(2);
    cases.push(v);
    let mut v = construction_clock_source();
    v["development_clock_enabled"] = json!(false);
    cases.push(v);
    let mut v = construction_clock_source();
    v["recovery"]["clock_journal_id"] = json!("4".repeat(64));
    cases.push(v);
    let mut v = construction_clock_source();
    v["clock"]["original_paired_seal"] = json!("4".repeat(64));
    cases.push(v);
    let mut v = construction_clock_source();
    v["clock"]["run_count"] = json!(64);
    cases.push(v);
    let mut v = construction_clock_source();
    v["history"]["fenced"] = json!(true);
    cases.push(v);
    let mut v = construction_clock_source();
    v["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(100);
    cases.push(v);
    for view in cases {
        let out = clock_packet("fortress.query", view, 1200)?;
        assert!(!offers_clock(&out, "fortress.plan"));
        assert!(!offers_clock(&out, "fortress.commit"));
    }
    assert!(!offers_clock(
        &clock_packet("fortress.query", construction_clock_source(), 0)?,
        "fortress.plan"
    ));
    Ok(())
}

#[test]
fn prepared_clock_uses_its_distinct_review_and_never_recommends_placement()
-> Result<(), serde_json::Error> {
    let mut view = construction_clock_source();
    prepare_clock(&mut view);
    let out = clock_packet("fortress.plan", view.clone(), 1200)?;
    let next = &out["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.commit");
    assert_eq!(next["arguments"]["scope"], "run");
    assert_eq!(next["arguments"]["paired_seal"], "8".repeat(64));
    assert_eq!(next["requires_confirmation"], true);
    assert!(offers_clock(&out, "fortress.commit"));
    assert_eq!(
        out["agent_turn"]["active_work"]["confirmations"][0]["scope"],
        "run"
    );
    assert_eq!(
        out["agent_turn"]["briefing"]["clock_receipt_is_goal_proof"],
        false
    );
    for (field, value) in [
        ("before_tick", 99),
        ("source_sequence", 3),
        ("game_ticks", 101),
    ] {
        let mut changed = view.clone();
        changed["clock"]["active_run"][field] = json!(value);
        let denied = clock_packet("fortress.query", changed, 1200)?;
        assert!(!offers_clock(&denied, "fortress.commit"));
        assert_eq!(
            denied["agent_turn"]["recommendations"][0]["tool"],
            "fortress.cancel"
        );
        assert_eq!(
            denied["agent_turn"]["recommendations"][0]["arguments"]["scope"],
            "run"
        );
    }
    assert!(!offers_clock(
        &clock_packet("fortress.query", view, 4)?,
        "fortress.commit"
    ));
    Ok(())
}

#[test]
fn unsettled_clock_hides_cached_completion_and_keeps_safety_pause_available()
-> Result<(), serde_json::Error> {
    let mut view = construction_clock_source();
    prepare_clock(&mut view);
    view["clock"]["unresolved"] = json!(true);
    view["clock"]["may_be_running"] = json!(true);
    view["clock"]["active_run"]["state"] = json!("tracking");
    view["clock"]["active_run"]["native"]["phase"] = json!("running");
    view["current_goal"]["semantic_completion_proven"] = json!(true);
    view["current_goal"]["original_goal_proven"] = json!(true);
    view["first_satisfied_anchor"] = view["anchor"].clone();
    // Safety cancellation remains available after the original deadline.
    view["original_plan"]["steps"][0]["obligation"]["deadline_tick"] = json!(99);
    let out = clock_packet("fortress.commit", view, 0)?;
    assert_eq!(out["agent_turn"]["phase"], "reconcile");
    assert_eq!(
        out["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert!(out["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert!(out["agent_turn"]["briefing"]["original_goal_proven_at_anchor"].is_null());
    assert_eq!(out["agent_turn"]["briefing"]["clock_may_be_running"], true);
    assert!(offers_clock(&out, "fortress.cancel"));
    assert!(!offers_clock(&out, "fortress.plan"));
    assert!(!offers_clock(&out, "fortress.commit"));
    assert_eq!(
        out["agent_turn"]["references"][0]["recovery"]["clock_journal_id"],
        "1".repeat(64)
    );
    assert_eq!(
        out["agent_turn"]["active_work"]["obligations"][0]["historical_first_satisfied_anchor"]["tick"],
        100
    );
    Ok(())
}

#[test]
fn stopped_clock_does_not_make_an_older_construction_sample_current()
-> Result<(), serde_json::Error> {
    let mut view = construction_clock_source();
    view["clock"]["run_count"] = json!(1);
    view["clock"]["high_tick"] = json!(110);
    view["current_goal"]["semantic_completion_proven"] = json!(true);
    let out = clock_packet("fortress.wait", view, 1200)?;
    assert!(out["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert_eq!(
        out["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert!(!offers_clock(&out, "fortress.plan"));
    assert_eq!(out["agent_turn"]["briefing"]["current_pause_proven"], false);
    assert!(out["agent_turn"]["briefing"]["clock_advanced"].is_null());
    Ok(())
}

#[test]
fn clock_overflow_preserves_exact_original_and_run_handoff_without_control_or_truth()
-> Result<(), serde_json::Error> {
    let mut view = construction_clock_source();
    prepare_clock(&mut view);
    view["clock"]["unresolved"] = json!(true);
    view["clock"]["may_be_running"] = json!(true);
    view["clock"]["active_run"]["state"] = json!("dispatch_started");
    view["clock"]["untrusted_extra"] = json!("x".repeat(RESPONSE_RESERVE as usize));
    view["original_plan"]["summary"] = json!("oversized".repeat(50_000));
    let encoded = packet(
        "fortress.commit",
        Some(SessionId::new(77)),
        None,
        Some("control"),
        None,
        None,
        view,
        None,
        true,
    );
    assert!(encoded.len() as u64 <= RESPONSE_RESERVE);
    let out: Value = serde_json::from_str(&encoded)?;
    assert_eq!(out["result"]["error"]["code"], "budget_exceeded");
    assert_eq!(
        out["retained_reference"]["recovery"]["object_digest"],
        "f".repeat(64)
    );
    assert_eq!(
        out["retained_reference"]["recovery"]["clock_journal_id"],
        "1".repeat(64)
    );
    assert_eq!(
        out["retained_reference"]["clock"]["active_run"]["review_seal"],
        "8".repeat(64)
    );
    assert!(
        out["retained_reference"]["clock"]
            .get("untrusted_extra")
            .is_none()
    );
    assert_eq!(out["result"]["clock_effect_may_have_started"], true);
    assert!(out["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert!(!offers_clock(&out, "fortress.commit"));
    Ok(())
}

#[test]
fn closing_a_clock_session_preserves_running_identity_without_offering_further_actions()
-> Result<(), serde_json::Error> {
    for oversized in [false, true] {
        let mut retained = construction_clock_source();
        prepare_clock(&mut retained);
        retained["clock"]["unresolved"] = json!(true);
        retained["clock"]["active_run"]["state"] = json!("tracking");
        if oversized {
            retained["original_plan"]["summary"] = json!("oversized".repeat(50_000));
        }
        let out = run(
            "fortress.cancel",
            json!({"ok":true,"closed":true,"session_closed":true,
            "effects_cancelled":false,"retained_state_currently_verified":false}),
            Some(retained),
            true,
        )?;
        assert_eq!(out["result"]["session_closed"], true);
        assert_eq!(out["agent_turn"]["briefing"]["session_closed"], true);
        assert!(
            out["agent_turn"]["recommendations"]
                .as_array()
                .is_some_and(|v| v.is_empty())
        );
        assert!(
            out["agent_turn"]["affordances"]
                .as_array()
                .is_some_and(|v| v.iter().all(|a| a["enabled"] != true))
        );
        assert_eq!(
            out["agent_turn"]["references"][0]["recovery"]["clock_journal_id"],
            "1".repeat(64)
        );
        assert!(out["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    }
    Ok(())
}
