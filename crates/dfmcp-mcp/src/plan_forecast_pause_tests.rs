//! A forecast must stop game time when its own deferred work pauses the world.

use super::tests::{adapter_at, context};
use super::*;
use std::time::Instant;

use crate::lab_world::starter;
use crate::server::lab_progress::ForegroundProgress;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, GameTick, IntentId, RiskTier};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner, effects,
};
use dfmcp_world::{CompareOp, Predicate, PredicateEvidence, Value};

fn fixture(
    pause_samples: Option<u32>,
    resume: bool,
) -> Result<(MemoryAdapter, PreparedPlan, OperationContext)> {
    let adapter = adapter_at(1)?;
    let mut current = context(&adapter);
    current.budget.max_wall_millis = 60_000;
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
    let drink = Predicate::FieldCompare {
        entity_id: starter::STOCK_LEDGER,
        field: effects::STOCK_DRINK_FIELD.to_owned(),
        op: CompareOp::Ge,
        value: Value::U64(45),
    };
    let mut actions = vec![
        RequestedAction {
            action: Action::CreateWorkOrder {
                name: "brew before the reviewed pause".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            preconditions: vec![Predicate::True],
            postconditions: vec![drink.clone()],
            compensation: None,
            obligation: Some(ObligationSpec {
                terminal: drink.clone(),
                failure: None,
                deadline_tick: GameTick(1001),
                poll_interval_ticks: 10,
                stable_for_observations: 1,
            }),
            depends_on: Vec::new(),
        },
        RequestedAction {
            action: Action::Pause { paused: true },
            preconditions: vec![Predicate::Paused(false)],
            postconditions: vec![Predicate::Paused(true)],
            compensation: None,
            obligation: pause_samples.map(|samples| ObligationSpec {
                terminal: Predicate::Paused(true),
                failure: None,
                deadline_tick: GameTick(1001),
                poll_interval_ticks: 10,
                stable_for_observations: samples,
            }),
            depends_on: vec![0],
        },
    ];
    if resume {
        actions.push(RequestedAction {
            action: Action::Pause { paused: false },
            preconditions: vec![Predicate::True],
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: None,
            depends_on: vec![1],
        });
    }
    let intent = Intent {
        id: IntentId::new(443),
        anchor: current.anchor,
        summary: "forecast only game time the foreground owner can advance".to_owned(),
        terminal_condition: Predicate::All(vec![drink, Predicate::Paused(!resume)]),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: actions,
    };
    let plan =
        StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &current)?;
    Ok((adapter, plan, current))
}

#[test]
fn deferred_pause_cannot_manufacture_advancing_stability_samples() -> Result<()> {
    let (adapter, plan, current) = fixture(Some(2), false)?;
    let original = format!("{adapter:?}");
    let prediction = forecast_plan(&adapter, &plan, &current);
    assert_eq!(prediction["available"], true, "{prediction}");
    assert_eq!(prediction["blocked_by_pause"], true, "{prediction}");
    assert_eq!(prediction["predicted_frontier_tick"], 51);
    assert_eq!(prediction["predicted_goal_truth"], "true");
    assert_eq!(prediction["predicted_actions_complete"], false);
    assert_eq!(prediction["predicted_physical_quiescent"], true);
    assert_eq!(prediction["predicted_complete"], false);
    assert!(prediction["predicted_completion_tick"].is_null());
    assert_eq!(prediction["steps"][0]["predicted_state"], "Verified");
    assert_eq!(
        prediction["steps"][1]["predicted_state"],
        "AppliedAwaitingVerification"
    );
    assert!(prediction["steps"][1]["predicted_terminal_tick"].is_null());

    // Exercise the actual foreground wait owner, not a second forecast model.
    let mut actual = adapter.clone();
    let prepared = actual.prepare(&plan, &current)?;
    let committed = actual.commit(&plan, &prepared, &current)?;
    let tracked: Vec<_> = committed
        .actions
        .iter()
        .map(|action| action.action_id)
        .collect();
    let mut progress = ForegroundProgress::new(
        &actual,
        1000,
        &OperationContext {
            anchor: actual.snapshot().anchor(),
            ..current.clone()
        },
    )?;
    for _ in 0..102 {
        let step_context = OperationContext {
            anchor: actual.snapshot().anchor(),
            ..current.clone()
        };
        let polled = progress.step(&mut actual, &tracked, &step_context, None)?;
        if polled.is_none_or(|slice| slice.finished) {
            break;
        }
    }
    assert_eq!(actual.snapshot().tick.0, 51);
    assert!(actual.snapshot().paused);
    assert_eq!(progress.advanced_game_ticks(), 50);
    assert_eq!(progress.remaining_game_ticks(), 950);
    assert_eq!(
        PredicateEvidence::laboratory(actual.snapshot())?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::True,
    );
    for (index, action) in committed.actions.iter().enumerate() {
        let observed = actual.poll_action(
            action.action_id,
            &OperationContext {
                anchor: actual.snapshot().anchor(),
                ..current.clone()
            },
        )?;
        assert_eq!(
            format!("{:?}", observed.state),
            prediction["steps"][index]["predicted_state"]
        );
    }
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

#[test]
fn deferred_pause_shares_the_exact_stop_frontier_with_comparison_and_reserve_budgets() -> Result<()>
{
    let (adapter, plan, current) = fixture(Some(2), false)?;
    let original = format!("{adapter:?}");
    for reserve_tick in [51, 251] {
        let mut budget = ForecastBudget::new(&current, Instant::now(), 1000)?;
        let prediction =
            forecast_plan_bounded(&adapter, &plan, &current, Some(reserve_tick), &mut budget)?;
        assert_eq!(prediction["available"], true, "{prediction}");
        assert_eq!(prediction["blocked_by_pause"], true);
        assert_eq!(prediction["predicted_complete"], false);
        assert_eq!(prediction["predicted_frontier_tick"], 51);
        assert_eq!(
            prediction["reserve_forecast"]["available"],
            reserve_tick == 51
        );
        if reserve_tick == 51 {
            assert_eq!(prediction["reserve_forecast"]["predicted_tick"], 51);
            assert_eq!(
                prediction["reserve_forecast"]["predicted_goal_truth"],
                "true"
            );
        } else {
            assert!(prediction["reserve_forecast"]["predicted_tick"].is_null());
            assert!(prediction["reserve_forecast"]["predicted_goal_truth"].is_null());
        }
        assert_eq!(budget.work_json()["simulated_game_ticks"], 50);
        assert_eq!(budget.work_json()["simulation_slices"], 5);
        assert_eq!(budget.work_json()["source_forks"], 1);
    }
    assert_eq!(format!("{adapter:?}"), original);
    Ok(())
}

#[test]
fn immediate_pause_completion_and_same_round_resume_remain_eligible() -> Result<()> {
    for resume in [false, true] {
        let (adapter, plan, current) = fixture(None, resume)?;
        let prediction = forecast_plan(&adapter, &plan, &current);
        assert_eq!(prediction["available"], true, "{prediction}");
        assert_eq!(prediction["blocked_by_pause"], false);
        assert_eq!(prediction["predicted_complete"], true);
        assert_eq!(prediction["predicted_goal_truth"], "true");
        assert_eq!(prediction["predicted_actions_complete"], true);
        assert_eq!(prediction["predicted_completion_tick"], 51);
    }
    Ok(())
}
