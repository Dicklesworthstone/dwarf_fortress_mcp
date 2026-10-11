//! Exercise whole-goal forecasts with the actual planner and effect timeline.

use super::*;
use crate::lab_world::{ProductionRequest, parse_steps, scenario_snapshot, starter};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId, GameTick,
    IntentId, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner, effects,
};
use dfmcp_world::{CompareOp, Fact, FactPresence, FactSource, Predicate, Value};

fn adapter_at(tick: u64) -> Result<MemoryAdapter> {
    let mut adapter = MemoryAdapter::new(scenario_snapshot(
        "starter_fortress",
        FortressId::new(746),
        false,
    )?);
    if tick > 1 {
        adapter.advance_ticks(tick - 1)?;
    }
    Ok(adapter)
}

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

fn production(adapter: &MemoryAdapter, raw: &str, current: bool) -> Result<PreparedPlan> {
    let request = if current {
        ProductionRequest::parse_current(raw)?
    } else {
        ProductionRequest::parse(raw)?
    };
    let compiled = request.compile(adapter.snapshot())?;
    let mut intent = Intent {
        id: IntentId::new(1),
        anchor: adapter.snapshot().anchor(),
        summary: "forecast the original reserve".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &context(adapter))
}

const DRINK: &str = r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#;

#[test]
fn legacy_completed_orders_do_not_predict_a_consumed_original_goal_as_complete() -> Result<()> {
    let adapter = adapter_at(1100)?;
    let plan = production(&adapter, DRINK, false)?;
    let before = adapter.snapshot().clone();
    let result = forecast_plan(&adapter, &plan, &context(&adapter));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["predicted_actions_complete"], true);
    assert_eq!(result["predicted_actions_completion_tick"], 1300);
    assert_eq!(result["predicted_physical_quiescent"], true);
    assert_eq!(result["predicted_goal_truth"], "false");
    assert_eq!(result["predicted_goal_complete"], false);
    assert_eq!(result["predicted_complete"], false);
    assert!(result["predicted_completion_tick"].is_null());
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}

#[test]
fn consumption_aware_plan_predicts_and_actually_reaches_the_original_goal() -> Result<()> {
    let mut adapter = adapter_at(1100)?;
    let plan = production(&adapter, DRINK, true)?;
    let before = adapter.snapshot().clone();
    let result = forecast_plan(&adapter, &plan, &context(&adapter));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["predicted_actions_complete"], true);
    assert_eq!(result["predicted_goal_truth"], "true");
    assert_eq!(result["predicted_physical_quiescent"], true);
    assert_eq!(result["predicted_complete"], true);
    assert_eq!(result["epistemic_state"], "predicted");
    assert_eq!(adapter.snapshot(), &before);
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    let receipt = adapter.commit(&plan, &prepared, &context(&adapter))?;
    let completion = result["predicted_completion_tick"]
        .as_u64()
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "forecast tick missing",
            )
        })?;
    adapter.advance_ticks(completion - adapter.snapshot().tick.0)?;
    for action in receipt.actions {
        assert_eq!(
            adapter
                .poll_action(action.action_id, &context(&adapter))?
                .state,
            CommitState::Verified
        );
        assert!(adapter.action_work_state(action.action_id)?.is_quiescent());
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn forecast_checks_every_original_quota_including_one_initially_satisfied() -> Result<()> {
    let adapter = adapter_at(1151)?;
    let raw = r#"{"template":"production","quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]}"#;
    let legacy = production(&adapter, raw, false)?;
    assert_eq!(legacy.steps.len(), 1);
    let old = forecast_plan(&adapter, &legacy, &context(&adapter));
    assert_eq!(old["predicted_actions_complete"], true, "{old}");
    assert_eq!(old["predicted_goal_truth"], "false");
    assert_eq!(old["predicted_complete"], false);
    let current = production(&adapter, raw, true)?;
    assert_eq!(current.steps.len(), 2);
    let new = forecast_plan(&adapter, &current, &context(&adapter));
    assert_eq!(new["predicted_actions_complete"], true, "{new}");
    assert_eq!(new["predicted_goal_truth"], "true");
    assert_eq!(new["predicted_complete"], true);
    Ok(())
}

#[test]
fn early_goal_and_action_proof_do_not_hide_physical_work_beyond_the_forecast_horizon() -> Result<()>
{
    let adapter = adapter_at(1)?;
    let goal = Predicate::FieldCompare {
        entity_id: starter::STOCK_LEDGER,
        field: effects::STOCK_DRINK_FIELD.to_owned(),
        op: CompareOp::Ge,
        value: Value::U64(45),
    };
    let intent = Intent {
        id: IntentId::new(2),
        anchor: adapter.snapshot().anchor(),
        summary: "early reserve proof with long physical work".to_owned(),
        terminal_condition: goal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::CreateWorkOrder {
                name: "continues after first useful batch".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 20,
                conditions: Vec::new(),
            },
            preconditions: vec![Predicate::True],
            postconditions: vec![goal.clone()],
            compensation: None,
            obligation: Some(ObligationSpec {
                terminal: goal,
                failure: None,
                deadline_tick: GameTick(101),
                poll_interval_ticks: 50,
                stable_for_observations: 1,
            }),
            depends_on: Vec::new(),
        }],
    };
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter),
    )?;
    let result = forecast_plan(&adapter, &plan, &context(&adapter));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["predicted_actions_complete"], true, "{result}");
    assert_eq!(result["predicted_goal_complete"], true);
    assert_eq!(result["predicted_physical_quiescent"], false);
    assert_eq!(result["predicted_frontier_tick"], 101);
    assert_eq!(result["predicted_complete"], false);
    assert!(result["predicted_completion_tick"].is_null());
    Ok(())
}

#[test]
fn unknown_original_goal_remains_unknown_after_a_verified_immediate_action() -> Result<()> {
    let mut snapshot = adapter_at(1)?.snapshot().clone();
    let mut fact = Fact::known(
        Value::Bool(false),
        snapshot.tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    );
    fact.presence = Some(FactPresence::Unknown("not observed".to_owned()));
    snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "fixture ledger missing",
            )
        })?
        .fields
        .insert("unknown_goal".to_owned(), fact);
    snapshot.refresh_hash();
    let adapter = MemoryAdapter::new(snapshot);
    let intent = Intent {
        id: IntentId::new(3),
        anchor: adapter.snapshot().anchor(),
        summary: "preserve unknown original goal".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: starter::STOCK_LEDGER,
            field: "unknown_goal".to_owned(),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause { paused: true },
            preconditions: vec![Predicate::Paused(false)],
            postconditions: vec![Predicate::Paused(true)],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter),
    )?;
    let result = forecast_plan(&adapter, &plan, &context(&adapter));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["predicted_actions_complete"], true);
    assert_eq!(result["predicted_physical_quiescent"], true);
    assert_eq!(result["predicted_goal_truth"], "unknown");
    assert_eq!(result["predicted_goal_complete"], false);
    assert_eq!(result["predicted_complete"], false);
    Ok(())
}

#[test]
fn paused_work_and_invalid_future_inputs_do_not_become_completion_predictions() -> Result<()> {
    let mut paused = adapter_at(1)?.snapshot().clone();
    paused.paused = true;
    paused.refresh_hash();
    let adapter = MemoryAdapter::new(paused);
    let plan = production(&adapter, DRINK, true)?;
    let result = forecast_plan(&adapter, &plan, &context(&adapter));
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["blocked_by_pause"], true);
    assert_eq!(result["predicted_complete"], false);
    assert_eq!(result["predicted_frontier_tick"], 1);

    let mut future = adapter_at(1)?.snapshot().clone();
    future
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::METABOLISM_TICKS_FIELD))
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "fixture metabolism missing",
            )
        })?
        .observed_at = GameTick(100);
    future.refresh_hash();
    let adapter = MemoryAdapter::new(future);
    let plan = production(&adapter, DRINK, false)?;
    let result = forecast_plan(&adapter, &plan, &context(&adapter));
    assert_eq!(result["available"], false, "{result}");
    assert!(result["predicted_complete"].is_null());
    assert_eq!(result["epistemic_state"], "predicted");
    Ok(())
}
