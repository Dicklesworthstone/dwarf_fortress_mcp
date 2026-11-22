//! Model mechanics use actual completed laboratory actions and a real epoch
//! reset. The facade companion exercises the durable custody verifier itself.

use super::*;
use server::selected_plan_wait::RecoveredStepCustody;

fn recovered_fixture() -> Result<(MemoryAdapter, PreparedPlan, Original, Project)> {
    recover_project(fixture()?, 816_501, beds()?)
}

fn recover_project(
    mut live: MemoryAdapter,
    id: u128,
    source: ProjectSource,
) -> Result<(MemoryAdapter, PreparedPlan, Original, Project)> {
    let (plan, original, project) = commit_project(&mut live, id, source)?;
    finish_original(&mut live, &original)?;
    let original_head = live.snapshot().anchor();
    let recovered = MemoryAdapter::recovered(live.snapshot().clone())?;
    server::goal_continuation::RecoveredGoalLineage::verify(
        live.snapshot(),
        original_head,
        recovered.snapshot().anchor(),
    )?;
    let digest = Digest32::of_bytes(b"actual completed original model fixture custody");
    let census = plan
        .steps
        .iter()
        .zip(&original.actions)
        .map(|(step, action)| {
            let receipt = live.action_receipt(*action).ok_or_else(|| {
                error(
                    ErrorCode::InvalidPlan,
                    "completed model fixture receipt missing",
                )
            })?;
            assert_eq!(receipt.state, CommitState::Verified);
            Ok(RecoveredStepCustody {
                step: step.id,
                recorded_state: "verified",
                state: "verified",
                recorded_anchor: original_head,
                proof_anchor: Some(original_head),
                recovery_anchor: recovered.snapshot().anchor(),
                custody_digest: digest,
                work: live.action_work_state(*action)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let historical = RecoveredOriginal::new(
        &plan,
        census,
        json!({"digest":digest.to_hex()}),
        recovered.snapshot(),
    )?;
    Ok((
        recovered,
        plan,
        Original {
            actions: Vec::new(),
            ..original
        },
        project.with_recovery(historical)?,
    ))
}

fn recovered_room_fixture(
    connected: bool,
) -> Result<(MemoryAdapter, PreparedPlan, Original, Project)> {
    let mut snapshot = fixture()?.snapshot().clone();
    let wood = snapshot
        .graph
        .entities
        .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_WOOD_FIELD))
        .ok_or_else(|| error(ErrorCode::InvalidPlan, "room fixture wood stock missing"))?;
    wood.value = dfmcp_world::Value::U64(0);
    snapshot.refresh_hash();
    let room = json!({
        "template":"provisioned_room","acquire_resources":{"assign_labor":false},
        "layout":{"template":"bedroom_cluster","origin":[2,6,10],"rooms":1,"room_size":[3,3]},
        "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1"},
    });
    let source = if connected {
        ProjectSource::ConnectedRoom(server::connected_room::ConnectedRoomRequest::parse_current(
            &json!({
                "template":"connected_room","room":room,
                "access":{"entry":[0,2,10],"region":{"min":[0,0,10],"max":[12,12,10]}},
            })
            .to_string(),
        )?)
    } else {
        ProjectSource::Room(
            server::room_provisioning::ProvisionedRoomRequest::parse_current(&room.to_string())?,
        )
    };
    recover_project(
        MemoryAdapter::new(snapshot),
        816_502 + u128::from(connected),
        source,
    )
}

fn assert_recovered_room_project(connected: bool) -> Result<()> {
    let (adapter, plan, original, project) = recovered_room_fixture(connected)?;
    let before = format!("{adapter:?}");
    assert_eq!(
        project.goal_truth(adapter.snapshot())?,
        PredicateTruth::False
    );
    let result = run(
        &adapter,
        &[plan.clone()],
        &[original],
        Some(vec![project]),
        &context(&adapter),
        8_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    let row = &result["originals"][0];
    assert_eq!(row["selection_origin"], "durable_recovery");
    assert_eq!(row["predicted_project_actions_complete"], true);
    assert!(
        row["future_stages"]
            .as_array()
            .is_some_and(|stages| !stages.is_empty())
    );
    assert!(
        row["steps"]
            .as_array()
            .is_some_and(|steps| steps.len() == plan.steps.len()
                && steps.iter().all(|step| step["action_id"].is_null()
                    && step["action_state_origin"] == "verified_recovered_history"))
    );
    assert_eq!(
        result["forecast_work"]["verified_recovered_original_steps"],
        plan.steps.len()
    );
    assert_eq!(format!("{adapter:?}"), before);
    for step in &plan.steps {
        assert!(adapter.step_receipt(plan.id, step.id).is_none());
    }
    Ok(())
}

#[test]
fn recovered_room_harvest_predicts_production_and_furnishing_without_old_handles() -> Result<()> {
    assert_recovered_room_project(false)
}

#[test]
fn recovered_connected_room_harvest_preserves_access_and_furnishing_requirements() -> Result<()> {
    assert_recovered_room_project(true)
}

#[test]
fn recovered_verified_harvest_predicts_residual_production_without_old_handles() -> Result<()> {
    let (adapter, plan, original, project) = recovered_fixture()?;
    assert!(original.actions.is_empty());
    let before = format!("{adapter:?}");
    let expected_steps = plan.steps.len();
    let result = run(
        &adapter,
        &[plan.clone()],
        &[original],
        Some(vec![project]),
        &context(&adapter),
        4_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    let root = &result["originals"][0];
    assert_eq!(root["selection_origin"], "durable_recovery");
    assert_eq!(root["dispatch_handles_restored"], false);
    assert_eq!(root["predicted_actions_complete"], true);
    assert_eq!(root["predicted_committed_actions_complete"], true);
    assert_eq!(root["predicted_project_actions_complete"], true);
    assert_eq!(root["steps"].as_array().map(Vec::len), Some(expected_steps));
    assert!(
        root["steps"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(|row| {
                row["action_id"].is_null()
                    && row["action_state_origin"] == "verified_recovered_history"
                    && row["dispatch_handle_restored"] == false
            }))
    );
    assert_eq!(root["future_stages"][0]["stage"], "production");
    assert_eq!(result["forecast_work"]["tracked_actions"], 0);
    assert_eq!(
        result["forecast_work"]["verified_recovered_original_steps"],
        expected_steps
    );
    assert_eq!(format!("{adapter:?}"), before);
    for step in &plan.steps {
        assert!(adapter.step_receipt(plan.id, step.id).is_none());
    }
    Ok(())
}

#[test]
fn recovered_steps_consume_the_same_cumulative_action_allowance_as_future_work() -> Result<()> {
    let (adapter, plan, original, project) = recovered_fixture()?;
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = plan.steps.len() as u32;
    let zero = run(
        &adapter,
        &[plan.clone()],
        &[original.clone()],
        Some(vec![project.clone()]),
        &ctx,
        0,
    )?;
    assert_eq!(zero["all_complete"], false);
    assert_eq!(zero["forecast_work"]["hypothetical_action_steps"], 0);
    let refused = run(
        &adapter,
        &[plan],
        &[original],
        Some(vec![project]),
        &ctx,
        4_000,
    );
    assert!(matches!(refused, Err(failure) if failure.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn recovered_forecast_cannot_repair_false_unknown_or_missing_past_reserves() -> Result<()> {
    for sample in [
        None,
        Some(PredicateTruth::False),
        Some(PredicateTruth::Unknown),
    ] {
        let (adapter, plan, mut original, project) = recovered_fixture()?;
        original.reserve_until = Some(adapter.snapshot().tick.0.saturating_sub(1));
        original.reserve_observation = sample;
        let result = run(
            &adapter,
            &[plan],
            &[original],
            Some(vec![project]),
            &context(&adapter),
            4_000,
        )?;
        assert_eq!(result["all_complete"], false, "{result}");
        assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
        assert_eq!(
            result["originals"][0]["predicted_outcome"],
            match sample {
                None => "reserve_unobserved",
                Some(PredicateTruth::False) => "reserve_unmet",
                _ => "reserve_unknown",
            }
        );
    }
    Ok(())
}

#[test]
fn recovered_originals_still_require_current_future_effect_authority() -> Result<()> {
    let (adapter, plan, original, project) = recovered_fixture()?;
    let mut ctx = context(&adapter);
    ctx.grants
        .retain(|grant| grant.capability != Capability::ConfigureProduction);
    let before = format!("{adapter:?}");
    let refused = run(
        &adapter,
        &[plan],
        &[original],
        Some(vec![project]),
        &ctx,
        4_000,
    );
    assert!(matches!(refused, Err(failure) if failure.code == ErrorCode::CapabilityDenied));
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn recovered_forecast_refuses_a_second_reset_or_unverified_original_census() -> Result<()> {
    let (adapter, plan, original, project) = recovered_fixture()?;
    let reset = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    assert!(
        run(
            &reset,
            &[plan.clone()],
            &[original],
            Some(vec![project]),
            &context(&reset),
            0
        )
        .is_err()
    );
    let digest = Digest32::of_bytes(b"unverified model custody");
    for state in ["dispatched", "failed", "cancelled", "not_dispatched"] {
        let rows = plan
            .steps
            .iter()
            .map(|step| RecoveredStepCustody {
                step: step.id,
                recorded_state: state,
                state,
                recorded_anchor: plan.anchor,
                proof_anchor: Some(plan.anchor),
                recovery_anchor: adapter.snapshot().anchor(),
                custody_digest: digest,
                work: EffectWorkState::NeverDispatched,
            })
            .collect();
        assert!(
            RecoveredOriginal::new(
                &plan,
                rows,
                json!({"digest":digest.to_hex()}),
                adapter.snapshot()
            )
            .is_err()
        );
    }
    Ok(())
}
