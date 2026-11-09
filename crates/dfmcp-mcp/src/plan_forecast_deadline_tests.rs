//! Deadline and stability frontiers use the actual planner and effect runtime.

use super::tests::{adapter_at, context};
use super::*;
use std::time::Instant;

use crate::lab_world::starter;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, GameTick, IntentId, RiskTier};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner, effects,
};
use dfmcp_world::{CompareOp, Predicate, PredicateEvidence, Value};

fn stock_goal(field: &str, minimum: u64) -> Predicate {
    Predicate::FieldCompare {
        entity_id: starter::STOCK_LEDGER,
        field: field.to_owned(),
        op: CompareOp::Ge,
        value: Value::U64(minimum),
    }
}

fn forecast_context(adapter: &MemoryAdapter) -> OperationContext {
    let mut current = context(adapter);
    current.budget.max_wall_millis = 60_000;
    current.budget.max_game_ticks = 200_000;
    current.budget.max_entities = 10_000;
    current.budget.max_bytes = 4 * 1024 * 1024;
    current.grants.push(CapabilityGrant {
        capability: Capability::Query,
        max_risk: RiskTier::ReadOnly,
        scope: CapabilityScope {
            fortress_id: Some(adapter.snapshot().fortress_id),
            ..CapabilityScope::default()
        },
        expires_at_tick: None,
        remaining_uses: None,
    });
    current
}

fn mixed_fixture(
    short: (u64, u64, u32),
    long: (u64, u64, u32),
    dependent: bool,
) -> Result<(MemoryAdapter, PreparedPlan, OperationContext)> {
    let adapter = adapter_at(1)?;
    let current = forecast_context(&adapter);
    let drink = stock_goal(effects::STOCK_DRINK_FIELD, 45);
    let food = stock_goal(effects::STOCK_FOOD_FIELD, 65);
    let requests = [
        ("short brew", "BREW_DRINK", drink.clone(), short),
        ("long kitchen", "PREPARE_MEAL", food.clone(), long),
    ]
    .into_iter()
    .enumerate()
    .map(
        |(index, (name, job_token, terminal, (deadline, cadence, stable)))| RequestedAction {
            action: Action::CreateWorkOrder {
                name: name.to_owned(),
                job_token: job_token.to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            preconditions: vec![Predicate::True],
            postconditions: vec![terminal.clone()],
            compensation: None,
            obligation: Some(ObligationSpec {
                terminal,
                failure: None,
                deadline_tick: GameTick(deadline),
                poll_interval_ticks: cadence,
                stable_for_observations: stable,
            }),
            depends_on: if index == 1 && dependent {
                vec![0]
            } else {
                Vec::new()
            },
        },
    )
    .collect();
    let intent = Intent {
        id: IntentId::new(441),
        anchor: current.anchor,
        summary: "predict independent short and long production obligations".to_owned(),
        terminal_condition: Predicate::All(vec![drink, food]),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: requests,
    };
    let plan =
        StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &current)?;
    Ok((adapter, plan, current))
}

fn verify_execution_at(
    adapter: &mut MemoryAdapter,
    plan: &PreparedPlan,
    template: &OperationContext,
    ticks: &[u64],
) -> Result<()> {
    let current = |adapter: &MemoryAdapter| OperationContext {
        anchor: adapter.snapshot().anchor(),
        ..template.clone()
    };
    let prepared = adapter.prepare(plan, &current(adapter))?;
    let receipt = adapter.commit(plan, &prepared, &current(adapter))?;
    for tick in ticks {
        adapter.advance_ticks(*tick - adapter.snapshot().tick.0)?;
        for action in &receipt.actions {
            adapter.poll_action(action.action_id, &current(adapter))?;
        }
    }
    for action in &receipt.actions {
        assert_eq!(
            adapter
                .poll_action(action.action_id, &current(adapter))?
                .state,
            CommitState::Verified,
        );
        assert!(adapter.action_work_state(action.action_id)?.is_quiescent());
    }
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::True,
    );
    Ok(())
}

#[test]
fn a_long_step_cannot_make_a_short_step_miss_its_exact_deadline() -> Result<()> {
    let (mut adapter, plan, current) = mixed_fixture((61, 1000, 1), (100_001, 10, 1), false)?;
    let original = format!("{adapter:?}");
    let result = forecast_plan(&adapter, &plan, &current);
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["resolution_ticks"], 250);
    assert_eq!(result["predicted_complete"], true, "{result}");
    assert_eq!(result["predicted_completion_tick"], 61);
    assert_eq!(result["steps"][0]["predicted_terminal_tick"], 61);
    assert_eq!(result["simulation_slices"], 1);
    assert_eq!(format!("{adapter:?}"), original);
    // The old 250-tick stride really does lose this otherwise achievable
    // proof in the ordinary adapter; a late matching stock cannot repair it.
    let mut late = adapter.clone();
    let prepared = late.prepare(&plan, &current)?;
    let receipt = late.commit(&plan, &prepared, &current)?;
    late.advance_ticks(250)?;
    let late_context = OperationContext {
        anchor: late.snapshot().anchor(),
        ..current.clone()
    };
    assert_eq!(
        late.poll_action(receipt.actions[0].action_id, &late_context)?
            .state,
        CommitState::Failed,
    );
    verify_execution_at(&mut adapter, &plan, &current, &[61])
}

#[test]
fn final_stability_samples_survive_a_coarse_multi_step_forecast() -> Result<()> {
    let (mut adapter, plan, current) = mixed_fixture((71, 10, 3), (100_001, 10, 1), false)?;
    let result = forecast_plan(&adapter, &plan, &current);
    assert_eq!(result["predicted_complete"], true, "{result}");
    assert_eq!(result["predicted_completion_tick"], 71);
    assert_eq!(result["steps"][0]["predicted_terminal_tick"], 71);
    assert_eq!(result["simulation_slices"], 3);
    verify_execution_at(&mut adapter, &plan, &current, &[51, 61, 71])
}

#[test]
fn off_cadence_reads_do_not_shift_the_final_stability_window() -> Result<()> {
    let (mut adapter, plan, current) = mixed_fixture((121, 30, 3), (14_801, 10, 1), false)?;
    let result = forecast_plan(&adapter, &plan, &current);
    assert_eq!(result["resolution_ticks"], 37);
    assert_eq!(result["predicted_complete"], true, "{result}");
    assert_eq!(result["predicted_completion_tick"], 121);
    assert_eq!(result["simulation_slices"], 4);
    verify_execution_at(&mut adapter, &plan, &current, &[38, 68, 98, 121])
}

#[test]
fn deferred_observation_cadence_begins_at_actual_dispatch() -> Result<()> {
    let (mut adapter, plan, current) = mixed_fixture((61, 10, 1), (141, 10, 3), true)?;
    let result = forecast_plan(&adapter, &plan, &current);
    assert_eq!(result["predicted_complete"], true, "{result}");
    assert_eq!(result["steps"][0]["predicted_terminal_tick"], 51);
    assert_eq!(result["steps"][1]["predicted_terminal_tick"], 121);
    verify_execution_at(
        &mut adapter,
        &plan,
        &current,
        &[11, 21, 31, 41, 51, 61, 71, 81, 91, 101, 111, 121],
    )
}

#[test]
fn a_real_deadline_miss_stays_failed_even_when_later_stock_satisfies_the_goal() -> Result<()> {
    let (adapter, plan, current) = mixed_fixture((40, 10, 1), (100_001, 10, 1), false)?;
    let result = forecast_plan(&adapter, &plan, &current);
    assert_eq!(result["available"], true, "{result}");
    assert_eq!(result["steps"][0]["predicted_state"], "Failed");
    assert_eq!(result["steps"][0]["predicted_terminal_tick"], 40);
    assert_eq!(result["predicted_goal_truth"], "true");
    assert_eq!(result["predicted_physical_quiescent"], true);
    assert_eq!(result["predicted_complete"], false);
    Ok(())
}

#[test]
fn stability_frontiers_cannot_exceed_single_or_shared_forecast_budgets() -> Result<()> {
    let (adapter, plan, current) = mixed_fixture((501, 1, 501), (100_001, 10, 1), false)?;
    let original = format!("{adapter:?}");
    let single = forecast_plan(&adapter, &plan, &current);
    assert_eq!(single["available"], false, "{single}");
    assert_eq!(single["reason"]["code"], "budget_exceeded");
    assert!(single["predicted_complete"].is_null());
    let mut budget = ForecastBudget::new(&current, Instant::now(), 1000)?;
    assert!(matches!(
        forecast_plan_bounded(&adapter, &plan, &current, None, &mut budget),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    assert_eq!(budget.work_json()["simulation_slices"], MAX_FORECAST_SLICES);
    assert_eq!(
        budget.work_json()["simulated_game_ticks"],
        MAX_FORECAST_SLICES
    );
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

#[test]
fn reserve_fork_uses_the_same_hard_slice_bound_as_stability_observation() -> Result<()> {
    let adapter = adapter_at(1)?;
    let current = forecast_context(&adapter);
    let drink = stock_goal(effects::STOCK_DRINK_FIELD, 45);
    let intent = Intent {
        id: IntentId::new(442),
        anchor: current.anchor,
        summary: "one complete 400-sample proof plus a future reserve".to_owned(),
        terminal_condition: drink.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![
            RequestedAction {
                action: Action::Pause { paused: false },
                preconditions: vec![Predicate::True],
                postconditions: vec![Predicate::Paused(false)],
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal: Predicate::Paused(false),
                    failure: None,
                    deadline_tick: GameTick(401),
                    poll_interval_ticks: 1,
                    stable_for_observations: 400,
                }),
                depends_on: Vec::new(),
            },
            RequestedAction {
                action: Action::CreateWorkOrder {
                    name: "reserve brew".to_owned(),
                    job_token: "BREW_DRINK".to_owned(),
                    amount: 1,
                    conditions: Vec::new(),
                },
                preconditions: vec![Predicate::True],
                postconditions: vec![drink.clone()],
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal: drink,
                    failure: None,
                    deadline_tick: GameTick(401),
                    poll_interval_ticks: 10,
                    stable_for_observations: 1,
                }),
                depends_on: Vec::new(),
            },
        ],
    };
    let plan =
        StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &current)?;
    let original = format!("{adapter:?}");
    let complete = forecast_plan(&adapter, &plan, &current);
    assert_eq!(complete["predicted_complete"], true, "{complete}");
    assert_eq!(complete["simulation_slices"], MAX_FORECAST_SLICES);
    let reserve = forecast_plan_with_reserve(&adapter, &plan, &current, Some(501));
    assert_eq!(reserve["available"], false, "{reserve}");
    assert_eq!(reserve["reason"]["code"], "budget_exceeded");
    let mut budget = ForecastBudget::new(&current, Instant::now(), 500)?;
    assert!(matches!(
        forecast_plan_bounded(&adapter, &plan, &current, Some(501), &mut budget),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    assert_eq!(budget.work_json()["simulation_slices"], MAX_FORECAST_SLICES);
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}
