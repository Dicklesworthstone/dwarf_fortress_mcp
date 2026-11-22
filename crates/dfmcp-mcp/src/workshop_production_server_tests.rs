//! Pure public-boundary regressions for the staged eleven-tool profile.
//! Native construction, production and recovery transitions have separate tests.
use super::*;
use dfmcp_core::Digest32;

#[test]
fn transition_request_is_closed_bounded_and_cannot_hide_duplicate_kind() -> Result<()> {
    assert!(is_begin_production(r#"{"kind":"begin_production"}"#)?);
    assert!(!is_begin_production(
        r#"{"kind":"step","original_plan_digest":"original"}"#
    )?);
    assert!(!is_begin_production(
        r#"{"units":["1"],"labor":"CARPENTER"}"#
    )?);
    for raw in [
        r#"{"kind":"begin_production","directory":"/tmp/foreign"}"#,
        r#"{"kind":"begin_production","handoff_digest":"caller-controlled"}"#,
        r#"{"kind":"begin_production","confirmed":true}"#,
        r#"{"kind":"begin_production","kind":"step"}"#,
        r#"{"kind":"step","kind":"begin_production"}"#,
        r#"{"kind":"begin_production","kind":"begin_production"}"#,
        r#"{"kind":null}"#,
        r#"{"kind":123}"#,
        "null",
        "[]",
        "",
    ] {
        assert!(is_begin_production(raw).is_err(), "{raw}");
    }
    let mut at_limit = r#"{"kind":"begin_production"}"#.to_owned();
    at_limit.push_str(&" ".repeat(8192 - at_limit.len()));
    assert!(is_begin_production(&at_limit)?);
    at_limit.push(' ');
    assert!(is_begin_production(&at_limit).is_err());
    Ok(())
}

#[test]
fn process_scoped_parent_handles_roundtrip_and_reject_child_namespaces() -> Result<()> {
    for sequence in [1, 2, SEQUENCE_LIMIT - 1] {
        let issued = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        assert_eq!(session_id(&issued.to_string())?, issued);
    }
    for family in [0, 1u128 << 57, 24u128 << 57, 31u128 << 57] {
        let foreign = SessionId::new((1u128 << 127) | family | 1);
        assert_eq!(
            session_id(&foreign.to_string()).err().map(|e| e.code),
            Some(ErrorCode::SessionNotFound)
        );
    }
    let unscoped = format!("{:032x}", (1u128 << 127) | FAMILY | 1);
    assert!(session_id(&unscoped).is_err());
    for malformed in [
        "",
        "1",
        "0000000000000000000000000000000g",
        "FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF",
    ] {
        assert!(session_id(malformed).is_err());
    }
    Ok(())
}

#[test]
fn opening_reserves_complete_recovery_and_keeps_clock_budget_explicit() -> Result<()> {
    let (display, work) = budgets(None, None, None, None)?;
    assert_eq!(display.max_game_ticks, 0);
    assert_eq!(work.max_game_ticks, 0);
    assert_eq!(
        display.max_bytes - work.max_bytes,
        RESPONSE_RESERVE + OPEN_INSPECTION_BYTES
    );
    assert_eq!(display.max_output_tokens, 32_768);
    assert!(display.max_wall_millis <= 60_000);
    let minimum = RESPONSE_RESERVE + OPEN_INSPECTION_BYTES + 1024 * 1024 * 1024;
    assert!(budgets(None, Some(minimum), None, None).is_ok());
    assert!(budgets(None, Some(minimum - 1), None, None).is_err());
    for output in [0, 1, 32_767, 32_769] {
        assert!(budgets(None, None, Some(output), None).is_err());
    }
    assert!(budgets(Some(0), None, None, None).is_err());
    assert!(budgets(Some(60_001), None, None, None).is_err());
    assert!(budgets(None, Some(32 * 1024 * 1024 * 1024 + 1), None, None).is_err());
    assert_eq!(
        budgets(None, None, None, Some(1200))?.1.max_game_ticks,
        1200
    );
    assert!(budgets(None, None, None, Some(1201)).is_err());
    Ok(())
}

#[test]
fn final_publication_counts_jsonrpc_text_escaping_and_the_whole_budget() -> Result<()> {
    let (display, _) = budgets(None, None, None, None)?;
    let ordinary = json!({"text":"a".repeat(20_000)}).to_string();
    let escaped = json!({"text":"\"".repeat(20_000)}).to_string();
    assert!(ordinary.len() < MAX_WIRE_BYTES);
    assert!(escaped.len() < MAX_WIRE_BYTES);
    assert!(fits(&ordinary, display));
    assert!(
        !fits(&escaped, display),
        "the text-content JSON string adds another layer of escaping"
    );
    let wire = json!({"jsonrpc":"2.0","id":u64::MAX,
        "result":{"content":[{"type":"text","text":ordinary}],"isError":false}});
    let wire_bytes = wire.to_string().len() as u64;
    let mut exact = display;
    exact.max_bytes = wire_bytes;
    assert!(fits(&ordinary, exact));
    exact.max_bytes -= 1;
    assert!(!fits(&ordinary, exact));
    exact.max_bytes = display.max_bytes;
    exact.max_output_tokens = u32::try_from(wire_bytes.div_ceil(4))
        .map_err(|_| error(ErrorCode::InternalInvariantViolation, "fixture output size"))?;
    assert!(fits(&ordinary, exact));
    exact.max_output_tokens -= 1;
    assert!(!fits(&ordinary, exact));
    Ok(())
}

#[test]
fn refusal_retains_complete_small_workflow_without_current_proof_or_review() -> Result<()> {
    let (display, _) = budgets(None, None, None, None)?;
    let session = SessionId::new((1u128 << 127) | FAMILY | 1);
    let hash = Digest32::of_bytes(b"staged retained recovery").to_hex();
    let recovery = json!({
        "checkpoint":{"stage":"transition","handoff_digest":hash,"authority_tick_floor":120},
        "workshop":{"acknowledged":{"journal_id":hash,"head":hash,"frames":7},
            "clock":{"recovery":{"journal_id":hash,"head":hash}}},
        "production":{"workforce_journal_id":hash,"recovery":null}
    });
    let cause = error(
        ErrorCode::BudgetExceeded,
        "publication failed after the original effect",
    );
    let raw = fallback(
        "fortress.commit",
        Some(session),
        Some(RequestId::new(3)),
        display,
        Some(Stage::Transition),
        recovery.clone(),
        &cause,
        true,
    );
    assert!(fits(&raw, display));
    let packet: Value = serde_json::from_str(&raw).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "invalid fallback JSON",
        )
    })?;
    assert_eq!(packet["workflow"], recovery);
    assert_eq!(packet["result"]["effect_may_have_occurred"], true);
    assert_eq!(packet["result"]["original_goal_proven"], false);
    assert_eq!(packet["result"]["retained_state_currently_verified"], false);
    assert_eq!(
        packet["agent_turn"]["continuity"]["status"],
        "indeterminate"
    );
    assert_eq!(
        packet["agent_turn"]["active_work"]["confirmations"],
        json!([])
    );
    assert_eq!(
        packet["agent_turn"]["active_work"]["pending_plans"],
        json!([])
    );
    assert_eq!(packet["agent_turn"]["recommendations"], json!([]));
    Ok(())
}

#[test]
fn oversized_diagnostics_and_workflow_still_publish_a_bounded_honest_refusal() -> Result<()> {
    let (display, _) = budgets(None, None, None, None)?;
    for budget in [display, WorkBudget::default()] {
        for message in ["workshop \"🛠\" ".repeat(40_000), "\0".repeat(40_000)] {
            let cause = error(ErrorCode::CapabilityDenied, &message);
            let recovery = json!({"complete":true,"originals":"\"".repeat(80_000)});
            let raw = fallback(
                "fortress.wait",
                None,
                None,
                budget,
                Some(Stage::Production),
                recovery,
                &cause,
                true,
            );
            assert!(
                fits(&raw, budget),
                "the final fallback itself must satisfy escaped-wire bounds"
            );
            let packet: Value = serde_json::from_str(&raw).map_err(|_| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "invalid fallback JSON",
                )
            })?;
            assert_eq!(packet["result"]["ok"], false);
            assert_eq!(packet["result"]["error"]["code"], "capability_denied");
            assert_eq!(packet["result"]["effect_may_have_occurred"], true);
            assert_eq!(packet["result"]["original_goal_proven"], false);
            assert_eq!(packet["workflow"]["complete"], false);
            assert_eq!(packet["workflow"]["withheld_for_output"], true);
            assert_eq!(packet["agent_turn"]["schema"], "dfmcp.agent_turn/1");
            assert_eq!(
                packet["agent_turn"]["continuity"]["status"],
                "indeterminate"
            );
            assert_eq!(
                packet["agent_turn"]["active_work"]["confirmations"],
                json!([])
            );
            assert_eq!(
                packet["agent_turn"]["active_work"]["pending_plans"],
                json!([])
            );
            assert_eq!(packet["agent_turn"]["recommendations"], json!([]));
        }
    }
    Ok(())
}

#[test]
fn transition_custody_has_no_production_orientation_or_effect_advice() -> Result<()> {
    let (budget, _) = budgets(None, None, None, None)?;
    let session = SessionId::new((1u128 << 127) | FAMILY | 1);
    let request = RequestId::new(7);
    let hash = Digest32::of_bytes(b"preopened production custody").to_hex();
    // Transition inspection returns the inert combined owner's actual shape.
    // Its configured controls and custody anchor cannot orient a client toward
    // production before the exact Workshop import has completed.
    let production_custody = json!({
        "ok":true,"mode":"control","transition_pending":true,
        "anchor":{"fortress_id":"7","epoch":0,"sequence":0,"tick":0,
            "state_hash":Digest32::ZERO.to_hex()},
        "labor_control_enabled":true,"production_control_enabled":true,
        "original_plan":null,"review":null,
        "frontier":{"next_ready":null,"entries":[],"all_applied":false,"needs_reconciliation":false},
        "clock":{"enabled":true,"journal_id":hash,"active_run":null,"unresolved":false},
        "current_parent_goal":null,"selected_inventory_goal":null,"selected_order_goal":null,
        "original_goal_proven":false,"current_state_proven":false
    });
    let raw = transition_packet(
        "fortress.query",
        session,
        Some(request),
        budget,
        production_custody.clone(),
    );
    assert!(fits(&raw, budget));
    let packet: Value = serde_json::from_str(&raw).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "invalid transition JSON",
        )
    })?;
    assert_eq!(
        packet["result"], production_custody,
        "retained child custody is not rewritten into current evidence"
    );
    let turn = &packet["agent_turn"];
    assert_eq!(turn["schema"], "dfmcp.agent_turn/1");
    assert_eq!(turn["operation"], "fortress.query");
    assert_eq!(turn["session_id"], session.to_string());
    assert_eq!(turn["request_id"], request.to_string());
    assert!(
        turn["anchor"].is_null(),
        "an inert production custody anchor is not canonical orientation"
    );
    assert_eq!(turn["continuity"]["status"], "indeterminate");
    assert!(turn["continuity"]["basis"].is_null());
    assert_eq!(turn["briefing"]["workflow_stage"], "transition");
    assert_eq!(turn["briefing"]["current_goal_available"], false);
    assert_eq!(turn["briefing"]["canonical_join_between_stages"], false);
    assert_eq!(turn["briefing"]["gameplay_permission"], false);
    assert_eq!(turn["active_work"], empty_active_work());
    assert_eq!(turn["affordances"], json!([]));
    assert_eq!(turn["recommendations"], json!([]));
    assert_eq!(turn["coverage"]["complete_domains"], json!([]));
    assert_eq!(
        turn["uncertainty"][0]["current_workshop_completion_proven"],
        false
    );
    assert_eq!(turn["uncertainty"][0]["automatic_redispatch"], false);
    for field in [
        "phase",
        "turn_id",
        "profile",
        "changes",
        "attention",
        "references",
        "budget",
    ] {
        assert!(
            turn.get(field).is_some(),
            "complete Agent Turn spine: {field}"
        );
    }
    assert_eq!(turn["budget"]["admitted"]["max_bytes"], budget.max_bytes);
    Ok(())
}
