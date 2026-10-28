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
        "development_designation_enabled":true,"native_retirement_enabled":true,"prepared_in_this_opening":true,
        "native_state":"prepared","native_phase":"prepared","native_needs_query":true,"effect_uncertain":false,
        "designation":{"verified":false},"current_goal":null,"current_goal_available":false,"history":{"fenced":false}})
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
fn designation_is_not_original_excavation_completion() -> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("designated");
    view["native_needs_query"] = json!(false);
    view["designation"] = json!({"verified":true,"receipt":"a".repeat(64)});
    let packet = run("fortress.commit", view, None, false)?;
    assert_eq!(
        packet["agent_turn"]["briefing"]["designation_verified_at_anchor"],
        true
    );
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    assert_eq!(
        packet["agent_turn"]["recommendations"][0]["tool"],
        "fortress.wait"
    );
    assert_eq!(
        packet["agent_turn"]["briefing"]["designation_receipt_is_goal_proof"],
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
    view["native_phase"] = json!("designated");
    view["native_needs_query"] = json!(false);
    view["prepared_in_this_opening"] = json!(false);
    view["designation"]["verified"] = json!(true);
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

#[test]
fn permanent_native_unknown_preserves_handoff_and_does_not_recommend_endless_queries()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    view["effect_uncertain"] = json!(true);
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("unknown");
    view["native_needs_query"] = json!(false);
    let packet = run("fortress.wait", view, None, true)?;
    let next = &packet["agent_turn"]["recommendations"][0];
    assert_eq!(next["tool"], "fortress.cancel");
    assert_eq!(next["arguments"]["scope"], "session");
    assert_eq!(
        packet["agent_turn"]["references"][0]["recovery"]["object_digest"],
        "f".repeat(64)
    );
    assert!(
        packet["agent_turn"]["active_work"]["confirmations"]
            .as_array()
            .is_some_and(Vec::is_empty)
    );
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    Ok(())
}

#[test]
fn prepared_retirement_requires_current_native_authority() -> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    view["native_state"] = json!("tracking");
    for grant in [Value::Null, json!(false)] {
        view["native_retirement_enabled"] = grant;
        let packet = run("fortress.wait", view.clone(), None, false)?;
        assert_eq!(
            packet["agent_turn"]["recommendations"][0]["tool"],
            "fortress.explain"
        );
        assert!(
            packet["agent_turn"]["active_work"]["confirmations"]
                .as_array()
                .is_some_and(Vec::is_empty)
        );
    }
    Ok(())
}

#[test]
fn original_current_truth_and_historical_achievement_remain_distinct()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["prepared_in_this_opening"] = json!(false);
    view["native_state"] = json!("terminal");
    view["native_phase"] = json!("designated");
    view["native_needs_query"] = json!(false);
    view["designation"] = json!({"verified":true,"receipt":"a".repeat(64),"designated_count":4});
    view["current_goal_available"] = json!(true);
    view["first_satisfied_anchor"] = view["anchor"].clone();
    for proven in [true, false] {
        view["current_goal"] = json!({"original_goal_proven":proven,"excavation_proven":proven,"semantic_completion_proven":proven});
        let packet = run("fortress.wait", view.clone(), None, false)?;
        assert_eq!(
            packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
            proven
        );
        assert_eq!(
            packet["agent_turn"]["active_work"]["obligations"][0]["historical_first_satisfied_anchor"]
                ["tick"],
            100
        );
        if !proven {
            assert_eq!(
                packet["agent_turn"]["recommendations"][0]["tool"],
                "fortress.wait"
            );
        }
    }
    view["effect_uncertain"] = json!(true);
    view["current_goal"] = json!({"original_goal_proven":true,"semantic_completion_proven":true});
    let packet = run("fortress.wait", view, None, true)?;
    assert!(packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"].is_null());
    Ok(())
}

#[test]
fn every_packet_removes_ambient_admission_and_withholds_unverified_designation()
-> Result<(), serde_json::Error> {
    let mut view = source();
    view["native_phase"] = json!("designated");
    view["designation"] = json!({"verified":false});
    view["current_goal_available"] = json!(true);
    view["current_goal"] = json!({"original_goal_proven":true,"semantic_completion_proven":true});
    let packet = run("fortress.wait", view, None, false)?;
    assert!(packet["agent_turn"]["briefing"].get("admission").is_none());
    assert_eq!(packet["agent_turn"]["briefing"]["runtime_admitted"], false);
    assert_eq!(
        packet["agent_turn"]["briefing"]["semantic_completion_proven_at_anchor"],
        false
    );
    Ok(())
}
