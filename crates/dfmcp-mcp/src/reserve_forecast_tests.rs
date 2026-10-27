//! Explicit future stock frontiers use the real sealed-plan reference timeline.

use super::*;
use crate::lab_world::{ProductionRequest, scenario_snapshot};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, IntentId, RequestId, Result,
    RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::StaticPlanner;

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

fn fixture(until: u64, minimum: u32, paused: bool) -> Result<(MemoryAdapter, PreparedPlan)> {
    let adapter = MemoryAdapter::new(scenario_snapshot(
        "starter_fortress_provisions",
        FortressId::new(782),
        paused,
    )?);
    let request = ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": [{"item": "DRINK", "minimum": minimum}, {"item": "FOOD", "minimum": 60}],
            "reserve_until_tick": until,
        })
        .to_string(),
    )?;
    let (intent, _) = crate::production_source::compile(
        &request,
        IntentId::new(1),
        adapter.snapshot(),
        "explicit reserve forecast",
    )?;
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter),
    )?;
    Ok((adapter, plan))
}

#[test]
fn future_stock_forecast_matches_execution_without_extending_work_deadlines() -> Result<()> {
    let (mut adapter, plan) = fixture(2401, 40, false)?;
    let before = adapter.snapshot().clone();
    let result = forecast_plan_with_reserve(&adapter, &plan, &context(&adapter), Some(2401));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["predicted_complete"], true, "{result}");
    let reserve = &result["reserve_forecast"];
    assert_eq!(reserve["available"], true, "{result}");
    assert_eq!(reserve["predicted_tick"], 2401);
    assert_eq!(reserve["predicted_goal_truth"], "true");
    assert_eq!(reserve["predicted_physical_quiescent"], true);
    assert_eq!(reserve["epistemic_state"], "predicted");
    assert_eq!(reserve["continuous_maintenance_proven"], false);
    assert_eq!(reserve["changes_action_deadlines"], false);
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(
        forecast_plan_with_reserve(&adapter, &plan, &context(&adapter), Some(2401)),
        result
    );
    let completion = result["predicted_completion_tick"]
        .as_u64()
        .ok_or_else(|| {
            DfmcpError::new(
                dfmcp_core::ErrorCode::InternalInvariantViolation,
                "completion missing",
            )
        })?;
    assert!(completion < 2401);
    assert!(
        plan.steps
            .iter()
            .filter_map(|step| step.obligation.as_ref())
            .all(|o| o.deadline_tick.0 < 2401)
    );
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    let receipt = adapter.commit(&plan, &prepared, &context(&adapter))?;
    let slice = result["resolution_ticks"].as_u64().ok_or_else(|| {
        DfmcpError::new(
            dfmcp_core::ErrorCode::InternalInvariantViolation,
            "cadence missing",
        )
    })?;
    while adapter.snapshot().tick.0 < completion {
        adapter.advance_ticks(slice.min(completion - adapter.snapshot().tick.0))?;
        for action in &receipt.actions {
            adapter.poll_action(action.action_id, &context(&adapter))?;
        }
    }
    adapter.advance_ticks(2401 - adapter.snapshot().tick.0)?;
    assert_eq!(
        crate::lab_world::plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::True
    );
    assert_eq!(
        adapter.snapshot().state_hash.to_hex(),
        reserve["predicted_state_hash"]
    );
    Ok(())
}

#[test]
fn a_reserve_frontier_before_completion_never_borrows_later_production() -> Result<()> {
    let (adapter, plan) = fixture(2, 100, false)?;
    let result = forecast_plan_with_reserve(&adapter, &plan, &context(&adapter), Some(2));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["reserve_forecast"]["available"], true);
    assert_eq!(result["reserve_forecast"]["predicted_tick"], 2);
    assert_eq!(result["reserve_forecast"]["predicted_goal_truth"], "false");
    assert_eq!(
        result["reserve_forecast"]["predicted_physical_quiescent"],
        false
    );
    assert_eq!(result["predicted_goal_truth"], "true");
    assert!(
        result["predicted_frontier_tick"]
            .as_u64()
            .is_some_and(|tick| tick > 2)
    );
    Ok(())
}

#[test]
fn paused_fortress_does_not_predict_future_stock_as_observed_or_available() -> Result<()> {
    let (adapter, plan) = fixture(2401, 40, true)?;
    let before = adapter.snapshot().clone();
    let result = forecast_plan_with_reserve(&adapter, &plan, &context(&adapter), Some(2401));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["blocked_by_pause"], true);
    assert_eq!(result["reserve_forecast"]["available"], false);
    assert!(result["reserve_forecast"]["predicted_goal_truth"].is_null());
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}

#[test]
fn no_horizon_keeps_existing_forecast_bytes_and_bad_horizons_refuse() -> Result<()> {
    let (adapter, plan) = fixture(2401, 40, false)?;
    assert_eq!(
        forecast_plan_with_reserve(&adapter, &plan, &context(&adapter), None),
        forecast_plan(&adapter, &plan, &context(&adapter)),
    );
    for tick in [0, 1, u64::MAX] {
        let result = forecast_plan_with_reserve(&adapter, &plan, &context(&adapter), Some(tick));
        assert_eq!(result["available"], false, "{result}");
        assert_eq!(result["reason"]["code"], "invalid_request");
    }
    Ok(())
}
