use super::*;

fn budget() -> WorkBudget {
    WorkBudget {
        max_wall_millis: 0,
        max_game_ticks: 0,
        max_entities: 0,
        max_bytes: 64 * 1024,
        max_output_tokens: 16 * 1024,
        max_actions: 0,
    }
}

#[test]
fn release_receipt_has_no_game_fact_or_production_admission_claim() -> Result<()> {
    let output = render(
        SessionId::new(19),
        budget(),
        "read/1.0",
        false,
        true,
        json!({"process_local_baselines_discarded": 0}),
    )?;
    let value: Value = serde_json::from_str(&output).map_err(|error| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, error.to_string())
    })?;
    assert_eq!(value["closed"], true);
    assert_eq!(value["release_only"], true);
    assert_eq!(value["native_captures"], 0);
    assert_eq!(value["game_state_observed"], false);
    assert_eq!(value["game_effects_cancelled"], false);
    assert_eq!(value["mutation_dispatched"], false);
    assert_eq!(value["files_modified"], false);
    assert_eq!(value["session_mutex_poisoned"], true);
    assert_eq!(value["receipt_durable"], false);
    assert_eq!(value["close_receipt_retention"], 32);
    assert_eq!(value["agent_turn"]["operation"], "fortress.cancel");
    assert_eq!(value["agent_turn"]["continuity"]["status"], "partial");
    assert!(!output.contains("runtime_admitted"));
    assert!(!output.contains("state_hash"));
    assert!(!output.contains("calendar_year"));
    Ok(())
}

#[test]
fn development_release_preserves_explicit_unadmitted_posture() -> Result<()> {
    let output = render(
        SessionId::new(20),
        budget(),
        "announcements/1.1",
        true,
        false,
        json!({}),
    )?;
    let value: Value = serde_json::from_str(&output).map_err(|error| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, error.to_string())
    })?;
    assert_eq!(value["runtime"], "unadmitted_development");
    assert_eq!(value["runtime_admitted"], false);
    assert_eq!(value["agent_turn"]["briefing"]["runtime_admitted"], false);
    assert_eq!(value["agent_turn"]["briefing"]["compatibility_admitted"], false);
    Ok(())
}

#[test]
fn complete_agent_turn_must_fit_both_byte_and_token_allowances() -> Result<()> {
    let id = SessionId::new(21);
    let release = json!({"process_local_baselines_discarded": 8});
    let output = render(id, budget(), "read/1.0", false, false, release.clone())?;
    let exact = WorkBudget {
        max_bytes: output.len() as u64,
        ..budget()
    };
    assert_eq!(
        render(id, exact, "read/1.0", false, false, release.clone())?,
        output
    );
    for too_small in [
        WorkBudget {
            max_bytes: exact.max_bytes - 1,
            ..budget()
        },
        WorkBudget {
            max_output_tokens: ((output.len() - 1) / 4) as u32,
            ..budget()
        },
    ] {
        assert!(matches!(
            render(id, too_small, "read/1.0", false, false, release.clone()),
            Err(failure) if failure.code == ErrorCode::BudgetExceeded
        ));
    }
    Ok(())
}
