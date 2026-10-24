//! Finite-ingredient forecasts and original-goal continuation across recovery.

use super::super::goal_continuation::{
    ProductionContinuation, RecoveredGoalLineage, validate_goal_evidence_in_active_lineage,
};
use super::*;
use crate::lab_world::{
    laboratory_provisions_tests::{commit, context, finish, request, seal, source, write},
    plan_goal_truth, provisions_action_json, provisions_json, starter,
};
use dfmcp_adapter::CancelMode;
use dfmcp_core::{EntityId, IntentId, Result, RiskTier};
use dfmcp_intent::{Action, Constraint, Intent, RequestedAction, StaticPlanner, effects};
use dfmcp_world::{CompareOp, Predicate, Value, WorldSnapshot};

fn missing(message: &str) -> DfmcpError {
    DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation, message)
}

#[test]
fn provisions_forecast_matches_actual_consumption_without_publishing_source_changes() -> Result<()>
{
    let mut adapter = MemoryAdapter::new(source(2)?);
    let compiled = request(json!([
        {"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65},
        {"item":"BED","minimum":1},
    ]))?
    .compile(adapter.snapshot())?;
    let plan = seal(adapter.snapshot(), &compiled)?;
    let before = adapter.snapshot().clone();
    let forecast = forecast_plan(&adapter, &plan, &context(adapter.snapshot()));
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    let predicted = &forecast["predicted_provisions"];
    assert_eq!(predicted["epistemic_state"], "predicted");
    assert_eq!(predicted["evidence_scope"], "discarded_laboratory_forecast");
    assert!(predicted.get("source_anchor").is_none());
    assert!(predicted.get("predicted_anchor").is_some());
    assert_eq!(predicted["available_plants"], 0);
    assert_eq!(predicted["consumed_plants"], 2);
    assert_eq!(predicted["remaining_demand_plants"], 0);
    assert_eq!(
        adapter.snapshot(),
        &before,
        "forecast changed current inventory or history"
    );
    assert_eq!(
        forecast_plan(&adapter, &plan, &context(adapter.snapshot())),
        forecast
    );
    let current =
        provisions_json(adapter.snapshot())?.ok_or_else(|| missing("current view missing"))?;
    assert_eq!(current["consumed_plants"], 0);
    assert_eq!(current["available_plants"], 2);
    let actions = commit(&mut adapter, &plan)?;
    let completion = forecast["predicted_completion_tick"]
        .as_u64()
        .ok_or_else(|| missing("completion missing"))?;
    let slice = forecast["resolution_ticks"]
        .as_u64()
        .ok_or_else(|| missing("slice missing"))?;
    while adapter.snapshot().tick.0 < completion {
        adapter.advance_ticks(slice.min(completion - adapter.snapshot().tick.0))?;
        for action in &actions {
            adapter.poll_action(*action, &context(adapter.snapshot()))?;
        }
    }
    finish(&mut adapter, &plan, &actions)?;
    let actual =
        provisions_json(adapter.snapshot())?.ok_or_else(|| missing("actual view missing"))?;
    for field in [
        "available_plants",
        "consumed_plants",
        "remaining_demand_plants",
        "orders",
    ] {
        assert_eq!(
            actual[field], predicted[field],
            "forecast mismatch for {field}"
        );
    }
    assert_eq!(actual["epistemic_state"], "certified_derived");
    Ok(())
}

#[test]
fn early_stock_goal_remains_true_while_original_input_accounted_work_continues() -> Result<()> {
    let snapshot = source(2)?;
    let intent = Intent {
        id: IntentId::new(998),
        anchor: snapshot.anchor(),
        summary: "stock target can hold before the entire finite order finishes".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: starter::STOCK_LEDGER,
            field: effects::STOCK_DRINK_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(45),
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::CreateWorkOrder {
                name: "two finite brewing batches".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 2,
                conditions: Vec::new(),
            },
            preconditions: vec![Predicate::True],
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let forecast = forecast_plan(&adapter, &plan, &context(adapter.snapshot()));
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    let actions = commit(&mut adapter, &plan)?;
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT)?;
    assert_eq!(
        plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::True
    );
    let report = effects::provisions_report(adapter.snapshot())?
        .ok_or_else(|| missing("partial report missing"))?;
    assert_eq!(report.consumed_plants, 1);
    assert_eq!(report.remaining_demand_plants, 1);
    assert!(!adapter.action_work_state(actions[0])?.is_quiescent());
    assert_ne!(
        adapter
            .poll_action(actions[0], &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    finish(&mut adapter, &plan, &actions)?;
    Ok(())
}

#[test]
fn cancelled_consumption_survives_reopen_and_original_quota_continuation_spends_only_residual_input()
-> Result<()> {
    let original_request = request(json!([{"item":"DRINK","minimum":50}]))?;
    let snapshot = source(2)?;
    let compiled = original_request.compile(&snapshot)?;
    let original = seal(&snapshot, &compiled)?;
    let immutable = original.clone();
    let mut adapter = MemoryAdapter::new(snapshot);
    let actions = commit(&mut adapter, &original)?;
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT)?;
    adapter.poll_action(actions[0], &context(adapter.snapshot()))?;
    adapter.request_cancel(
        actions[0],
        CancelMode::StopFutureSteps,
        &context(adapter.snapshot()),
    )?;
    assert_eq!(
        adapter
            .finalize_cancel(actions[0], &context(adapter.snapshot()))?
            .state,
        CommitState::Cancelled
    );
    assert!(adapter.action_work_state(actions[0])?.is_quiescent());
    let cancelled = effects::provisions_report(adapter.snapshot())?
        .ok_or_else(|| missing("cancelled history missing"))?;
    assert_eq!(cancelled.available_plants, 1);
    assert_eq!(cancelled.consumed_plants, 1);
    assert_eq!(cancelled.remaining_demand_plants, 0);
    adapter.finalize_cancel(actions[0], &context(adapter.snapshot()))?;
    assert_eq!(
        effects::provisions_report(adapter.snapshot())?,
        Some(cancelled.clone())
    );
    let persisted = WorldSnapshot::from_canonical_bytes(&adapter.snapshot().canonical_bytes())?;
    let mut recovered = MemoryAdapter::recovered(persisted.clone())?;
    let lineage = RecoveredGoalLineage::verify(
        &persisted,
        persisted.anchor(),
        recovered.snapshot().anchor(),
    )?;
    assert!(
        recovered
            .poll_action(actions[0], &context(recovered.snapshot()))
            .is_err()
    );
    assert_eq!(
        effects::provisions_report(recovered.snapshot())?,
        Some(cancelled)
    );
    validate_goal_evidence_in_active_lineage(
        &original,
        lineage.active_anchor(),
        recovered.snapshot(),
        &context(recovered.snapshot()),
        true,
    )?;
    let continuation =
        ProductionContinuation::new(original.digest, original.digest, original_request)?;
    let restored = ProductionContinuation::parse(&continuation.canonical_json())?;
    assert_eq!(restored, continuation);
    let mut short = recovered.snapshot().clone();
    write(
        &mut short,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(0),
    )?;
    assert!(
        restored
            .compile(IntentId::new(999), &short, "residual shortage")
            .is_err()
    );
    let before = recovered.snapshot().clone();
    let (intent, analysis) = restored.compile(
        IntentId::new(999),
        recovered.snapshot(),
        "finish the original drink quota",
    )?;
    assert_eq!(recovered.snapshot(), &before);
    assert_eq!(intent.terminal_condition, original.terminal_condition);
    assert_eq!(analysis["materials"]["PLANT"]["observed_stock"], 1);
    assert_eq!(
        analysis["materials"]["PLANT"]["reserved_for_existing_orders"],
        0
    );
    assert_eq!(intent.requested_actions.len(), 1);
    assert!(matches!(
        intent.requested_actions[0].action,
        Action::CreateWorkOrder { amount: 1, .. }
    ));
    let resumed = StaticPlanner::default().prepare_laboratory(
        recovered.snapshot(),
        &intent,
        &context(recovered.snapshot()),
    )?;
    assert_ne!(resumed.digest, original.digest);
    let resumed_actions = commit(&mut recovered, &resumed)?;
    finish(&mut recovered, &resumed, &resumed_actions)?;
    let completed = effects::provisions_report(recovered.snapshot())?
        .ok_or_else(|| missing("completed history missing"))?;
    assert_eq!(completed.available_plants, 0);
    assert_eq!(completed.consumed_plants, 2);
    assert_eq!(completed.remaining_demand_plants, 0);
    assert_eq!(completed.orders.len(), 2);
    assert!(
        completed
            .orders
            .iter()
            .any(|order| order.status == effects::STATUS_CANCELLED && order.consumed_plants == 1)
    );
    assert!(
        completed
            .orders
            .iter()
            .any(|order| order.status == effects::STATUS_COMPLETE && order.consumed_plants == 1)
    );
    assert_eq!(
        plan_goal_truth(recovered.snapshot(), &original)?,
        PredicateTruth::True
    );
    assert_eq!(original, immutable);
    Ok(())
}

#[test]
fn ingredient_shortage_after_interference_blocks_until_qualified_supply_is_restored() -> Result<()>
{
    let snapshot = source(2)?;
    let compiled = request(json!([{"item":"DRINK","minimum":50}]))?.compile(&snapshot)?;
    let plan = seal(&snapshot, &compiled)?;
    let order = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let mut adapter = MemoryAdapter::new(snapshot);
    commit(&mut adapter, &plan)?;
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT)?;
    // A newly observed external stock change invalidates the forecast's input
    // assumption. The fixture does not imply a resource-gathering MCP action.
    let mut depleted = adapter.snapshot().clone();
    write(
        &mut depleted,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(0),
    )?;
    let mut blocked = MemoryAdapter::recovered(depleted)?;
    blocked.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT * 2)?;
    let report = effects::provisions_report(blocked.snapshot())?
        .ok_or_else(|| missing("blocked history missing"))?;
    assert_eq!(report.consumed_plants, 1);
    assert_eq!(report.remaining_demand_plants, 1);
    let view = provisions_action_json(blocked.snapshot(), order)?
        .ok_or_else(|| missing("blocked action missing"))?;
    assert!(
        view["order"]["input_blocker"]
            .as_str()
            .is_some_and(|reason| reason.contains("PLANT"))
    );
    assert_eq!(
        effects::work_order_blocker(blocked.snapshot(), "BREW_DRINK"),
        None
    );
    let mut replenished = blocked.snapshot().clone();
    write(
        &mut replenished,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(1),
    )?;
    let mut resumed = MemoryAdapter::recovered(replenished)?;
    resumed.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT)?;
    let completed = effects::provisions_report(resumed.snapshot())?
        .ok_or_else(|| missing("resumed history missing"))?;
    assert_eq!(completed.available_plants, 0);
    assert_eq!(completed.consumed_plants, 2);
    assert_eq!(completed.remaining_demand_plants, 0);
    assert_eq!(
        plan_goal_truth(resumed.snapshot(), &plan)?,
        PredicateTruth::True
    );
    assert!(provisions_action_json(resumed.snapshot(), EntityId::new(0))?.is_none());
    Ok(())
}
