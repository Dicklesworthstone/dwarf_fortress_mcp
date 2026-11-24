//! Actual durable active originals use private observation monitors and retain
//! the same scheduling frontier as the real selected-plan wait.

use super::*;

fn production(owner: &Owner, item: &str) -> TestResult<String> {
    commit(
        owner,
        None,
        Some(json!({
            "template": "production",
            "quotas": [{"item": item, "minimum": 4}],
        })),
    )
}

fn advance_one(owner: &Owner) -> TestResult {
    let result = parsed(&facade::fortress_wait(Some(owner.0.clone()), Some(1)))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(())
}

/// Deliberately corrupt only the in-memory fixture. The ordinary write wrapper
/// would observe and publish progress, hiding the admission error being tested.
fn replace_carried(owner: &Owner, rows: Vec<server::CarriedStep>) -> TestResult {
    let handle = server::resolve_session(Some(owner.0.clone()))?;
    let mut session = handle.lock().map_err(|_| "fixture session poisoned")?;
    session.carried = rows;
    Ok(())
}

#[test]
fn active_recovered_production_forecasts_without_publication_and_matches_real_wait()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("active-production");
    let first = open_durable("8161507")?;
    let digest = production(&first, "BED")?;
    advance_one(&first)?;
    directory.restart();
    drop(first);

    let owner = open_durable("8161507")?;
    let original = server::selected_plan_wait::parse_digest(&digest)?;
    let proofs = view(&owner, |session| {
        assert!(session.open_actions.is_empty());
        assert!(session.commit_receipts.is_empty());
        let proofs = session.carried.iter()
            .filter(|row| row.plan_digest == original && row.state == "dispatched")
            .map(|row| {
                assert!(row.monitor.is_some());
                Ok((
                    row.step.ok_or("active original step missing")?.get(),
                    row.proof_anchor.as_ref().map(server::anchor_json).unwrap_or(Value::Null),
                ))
            })
            .collect::<TestResult<Vec<_>>>()?;
        assert!(!proofs.is_empty(), "fixture must retain unfinished original work");
        Ok(proofs)
    })?;
    let before = state(&owner)?;
    let zero = forecast(&owner, &digest, 0)?;
    assert_eq!(zero["ok"], true, "{zero}");
    assert_eq!(zero["schema"], "dfmcp.lab-original-forecast/4");
    assert_eq!(zero["prediction"]["all_complete"], false, "{zero}");
    assert_eq!(state(&owner)?, before);

    let predicted = forecast(&owner, &digest, 4_000)?;
    assert_eq!(predicted["ok"], true, "{predicted}");
    assert_eq!(predicted["schema"], "dfmcp.lab-original-forecast/4");
    assert_eq!(predicted["prediction"]["all_complete"], true, "{predicted}");
    assert_eq!(predicted["prediction"]["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(predicted["prediction"]["forecast_work"]["hypothetical_stages"], 0);
    let original = &predicted["prediction"]["originals"][0];
    assert_eq!(original["actual_plan_digest"], digest);
    assert_eq!(original["dispatch_handles_restored"], false);
    assert_eq!(original["predicted_actions_complete"], true);
    assert_eq!(original["predicted_physical_quiescent"], true);
    let steps = original["steps"].as_array().ok_or("predicted original steps missing")?;
    assert!(steps.iter().all(|row| row["action_id"].is_null()));
    for (step, proof) in proofs {
        let row = steps.iter().find(|row| row["step"] == step)
            .ok_or("active original step omitted")?;
        assert_eq!(row["action_state_origin"], "cloned_recovered_observation_monitor");
        assert_eq!(row["predicted_state"], "Verified");
        assert_eq!(row["proof_anchor"], proof);
        assert!(row["predicted_proof_anchor"].is_object());
        assert_eq!(row["predicted_proof_is_canonical"], false);
        assert_eq!(row["dispatch_handle_restored"], false);
    }
    assert_eq!(predicted["agent_turn"]["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(state(&owner)?, before);

    let waited = parsed(&facade::fortress_wait_for_plan(
        Some(owner.0.clone()), Some(4_000), Some(digest),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["selected_plan_wait"]["stop_reason"], "complete", "{waited}");
    assert_eq!(waited["selected_plan_wait"]["actions_verified"], true);
    assert_eq!(waited["selected_plan_wait"]["physical_quiescent"], true);
    assert_eq!(waited["game_tick"], predicted["prediction"]["predicted_frontier_tick"]);
    Ok(())
}

#[test]
fn active_recovered_forecast_refuses_missing_foreign_interrupted_or_undispatched_monitors()
-> TestResult {
    let _serial = crate::test_serial();
    let _durable = server::serialized_durable_tests();
    let directory = StateDir::new("active-monitor-custody");
    let first = open_durable("8161508")?;
    let digest = production(&first, "BED")?;
    let peer = production(&first, "CHAIR")?;
    advance_one(&first)?;
    directory.restart();
    drop(first);

    let owner = open_durable("8161508")?;
    let original = server::selected_plan_wait::parse_digest(&digest)?;
    let other = server::selected_plan_wait::parse_digest(&peer)?;
    let baseline = view(&owner, |session| Ok(session.carried.clone()))?;
    let selected = baseline.iter().position(|row| {
        row.plan_digest == original && row.state == "dispatched" && row.monitor.is_some()
    }).ok_or("active selected monitor missing")?;
    let foreign = baseline.iter().find(|row| {
        row.plan_digest == other && row.state == "dispatched" && row.monitor.is_some()
    }).and_then(|row| row.monitor.clone()).ok_or("active foreign monitor missing")?;
    assert_eq!(
        baseline[selected].monitor.as_ref().ok_or("selected monitor missing")?.recovery_anchor(),
        foreign.recovery_anchor(),
    );

    for corruption in ["missing", "same_recovery_foreign", "interrupted", "not_dispatched"] {
        let mut rows = baseline.clone();
        match corruption {
            "missing" => rows[selected].monitor = None,
            "same_recovery_foreign" => rows[selected].monitor = Some(foreign.clone()),
            "interrupted" => rows[selected].observation_error = Some("interrupted fixture".to_owned()),
            _ => rows[selected].state = "not_dispatched".to_owned(),
        }
        replace_carried(&owner, rows)?;
        let before = state(&owner)?;
        let refused = forecast(&owner, &digest, 4_000)?;
        assert_eq!(refused["ok"], false, "{corruption}: {refused}");
        assert_eq!(state(&owner)?, before, "{corruption}");
    }

    replace_carried(&owner, baseline)?;
    let before = state(&owner)?;
    let valid = forecast(&owner, &digest, 4_000)?;
    assert_eq!(valid["ok"], true, "{valid}");
    assert_eq!(valid["prediction"]["all_complete"], true, "{valid}");
    assert_eq!(state(&owner)?, before);
    Ok(())
}
