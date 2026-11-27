//! Real finite-world regressions for joint original forecasts.

use super::*;
use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId, RequestId,
    SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner, effects,
};
use dfmcp_world::{CompareOp, Predicate, Value as WorldValue};

fn fixture() -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(scenario_snapshot(
        "starter_fortress_provisions", FortressId::new(816_101), false,
    )?))
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(816_101), request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(), cancellation_requested: false,
        budget: WorkBudget {
            max_wall_millis: 60_000, max_game_ticks: 4_000, max_entities: 32_768,
            max_bytes: 4 * 1024 * 1024, max_actions: 64, max_output_tokens: 32_768,
            ..WorkBudget::default()
        },
        grants: [
            (Capability::Observe, RiskTier::ReadOnly), (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly), (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ConfigureLabor, RiskTier::Reversible), (Capability::ControlClock, RiskTier::Reversible),
        ].into_iter().map(|(capability,max_risk)| CapabilityGrant {
            capability,max_risk,scope:CapabilityScope {
                fortress_id:Some(adapter.snapshot().fortress_id),..CapabilityScope::default()
            },expires_at_tick:None,remaining_uses:None,
        }).collect(),
    }
}

fn requested(action: Action, depends_on: Vec<u32>) -> RequestedAction {
    RequestedAction {
        action, depends_on, preconditions:Vec::new(), postconditions:Vec::new(),
        compensation:None, obligation:None,
    }
}

fn order(name: &str, amount: u32) -> RequestedAction {
    // The real carpentry recipe shares finite workers and wood; generic jobs do not.
    requested(Action::CreateWorkOrder {
        name:name.to_owned(), job_token:"CONSTRUCT_BED".to_owned(), amount, conditions:Vec::new(),
    },Vec::new())
}

fn prepare(adapter: &MemoryAdapter, id: u128, mut actions: Vec<RequestedAction>, duration: u64) -> Result<PreparedPlan> {
    let mut terminal = Vec::new();
    for (index, step) in actions.iter_mut().enumerate() {
        if step.postconditions.is_empty() {
            let action = step.action.normalized();
            let key = dfmcp_intent::derive_step_idempotency_key(
                IntentId::new(id),adapter.snapshot().anchor(),StepId::new(index as u32),&action,
            );
            step.postconditions = effects::default_postconditions(&action,&key,adapter.snapshot().fortress_id);
        }
        if step.action.naturally_temporal() && step.obligation.is_none() {
            step.obligation = Some(ObligationSpec {
                terminal:Predicate::All(step.postconditions.clone()).normalized(), failure:None,
                deadline_tick:GameTick(adapter.snapshot().tick.0+duration),
                poll_interval_ticks:10,stable_for_observations:1,
            });
        }
        terminal.extend(step.postconditions.clone());
    }
    StaticPlanner::default().prepare_laboratory(adapter.snapshot(),&Intent {
        id:IntentId::new(id),anchor:adapter.snapshot().anchor(),
        summary:"joint original forecast fixture".to_owned(),
        terminal_condition:Predicate::All(terminal).normalized(),
        constraints:vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions:actions,
    },&context(adapter))
}

fn commit(adapter: &mut MemoryAdapter, plan: &PreparedPlan) -> Result<Original> {
    let prepared = adapter.prepare(plan,&context(adapter))?;
    let receipt = adapter.commit(plan,&prepared,&context(adapter))?;
    Ok(Original {
        requested:plan.digest,actual:plan.digest,
        actions:receipt.actions.into_iter().map(|row|row.action_id).collect(),
        reserve_until:None,reserve_observation:None,can_continue:false,
    })
}

fn predict(
    adapter: &MemoryAdapter, plans: &[&PreparedPlan], originals: &[Original],
    tracked: &[ActionId], ticks: u64, ctx: &OperationContext,
) -> Result<Value> {
    forecast(adapter, originals, tracked, ctx, ticks, Instant::now(),None,Vec::new(),
        |snapshot, _, _| {
            if snapshot.hash_is_valid() {Ok(())}
            else {Err(error(ErrorCode::StaleAnchor,"invalid fixture source"))}
        },
        |digest,snapshot| {
            let plan = plans.iter().find(|plan|plan.digest==digest)
                .ok_or_else(||error(ErrorCode::InvalidPlan,"fixture plan missing"))?;
            let truth = crate::lab_world::plan_goal_truth(snapshot,plan)?;
            Ok((truth,truth))
        },
    )
}

#[test]
fn two_independently_feasible_originals_compete_for_one_actual_carpenter() -> Result<()> {
    for id in [1,2] {
        let mut adapter=fixture()?;
        let plan=prepare(&adapter,id,vec![order("two beds",2)],110)?;
        let original=commit(&mut adapter,&plan)?;
        let tracked=original.actions.clone();
        let result=predict(&adapter,&[&plan],&[original],&tracked,110,&context(&adapter))?;
        assert_eq!(result["all_complete"],true,"{result}");
    }
    let mut adapter=fixture()?;
    let first_plan=prepare(&adapter,1,vec![order("first pair",2)],110)?;
    let first=commit(&mut adapter,&first_plan)?;
    let second_plan=prepare(&adapter,2,vec![order("second pair",2)],110)?;
    let second=commit(&mut adapter,&second_plan)?;
    let tracked=first.actions.iter().chain(&second.actions).copied().collect::<Vec<_>>();
    let before=format!("{adapter:?}");
    let result=predict(&adapter,&[&first_plan,&second_plan],&[first,second],&tracked,110,&context(&adapter))?;
    assert_eq!(result["all_complete"],false,"{result}");
    assert_eq!(result["stop_reason"],"attention_required");
    assert!(result["originals"].as_array().is_some_and(|rows|rows.iter().any(|row|row["predicted_outcome"]=="failed")));
    assert_eq!(result["forecast_work"]["source_forks"],1);
    assert_eq!(result["forecast_work"]["actual_game_ticks_advanced"],0);
    assert_eq!(format!("{adapter:?}"),before);
    Ok(())
}

#[test]
fn originals_share_the_finite_stock_ledger_as_well_as_the_workers() -> Result<()> {
    let mut adapter=fixture()?;
    let first_plan=prepare(&adapter,11,vec![order("eleven beds",11)],1_500)?;
    let first=commit(&mut adapter,&first_plan)?;
    let second_plan=prepare(&adapter,12,vec![order("eleven more beds",11)],1_500)?;
    let second=commit(&mut adapter,&second_plan)?;
    let tracked=first.actions.iter().chain(&second.actions).copied().collect::<Vec<_>>();
    let before=format!("{adapter:?}");
    let result=predict(&adapter,&[&first_plan,&second_plan],&[first,second],&tracked,1_500,&context(&adapter))?;
    assert_eq!(result["all_complete"],false,"{result}");
    assert_eq!(result["stop_reason"],"attention_required");
    assert_eq!(format!("{adapter:?}"),before);
    Ok(())
}

#[test]
fn original_stability_samples_and_deadlines_survive_the_fork() -> Result<()> {
    let mut adapter=fixture()?;
    let terminal=Predicate::FieldCompare {
        entity_id:starter::STOCK_LEDGER,field:effects::STOCK_DRINK_FIELD.to_owned(),
        op:CompareOp::Ge,value:WorldValue::U64(45),
    };
    let step=RequestedAction {
        action:Action::CreateWorkOrder {
            name:"existing stability".to_owned(),job_token:"BREW_DRINK".to_owned(),
            amount:1,conditions:Vec::new(),
        },
        preconditions:Vec::new(),postconditions:vec![terminal.clone()],compensation:None,
        obligation:Some(ObligationSpec {
            terminal,failure:None,deadline_tick:GameTick(81),
            poll_interval_ticks:30,stable_for_observations:2,
        }),depends_on:Vec::new(),
    };
    let plan=prepare(&adapter,21,vec![step],80)?;
    let original=commit(&mut adapter,&plan)?;
    adapter.advance_ticks(60)?;
    adapter.poll_action(original.actions[0],&context(&adapter))?;
    assert_eq!(adapter.action_receipt(original.actions[0]).map(|row|row.state),Some(CommitState::AppliedAwaitingVerification));
    let before=format!("{adapter:?}");
    let tracked=original.actions.clone();
    let result=predict(&adapter,&[&plan],&[original],&tracked,20,&context(&adapter))?;
    assert_eq!(result["all_complete"],true,"{result}");
    assert_eq!(result["predicted_frontier_tick"],81);
    assert_eq!(format!("{adapter:?}"),before);
    Ok(())
}

fn cadence_original(adapter: &MemoryAdapter, id: u128) -> Result<PreparedPlan> {
    let terminal = Predicate::FieldCompare {
        entity_id: dfmcp_core::EntityId::new(1007),
        field: "labor.BREW".to_owned(),
        op: CompareOp::Eq,
        value: WorldValue::Bool(true),
    };
    let step = RequestedAction {
        action: Action::SetLabor {
            units: vec![dfmcp_core::EntityId::new(1007)],
            labor: "BREW".to_owned(),
            enabled: true,
        },
        preconditions: Vec::new(),
        postconditions: vec![terminal.clone()],
        compensation: None,
        obligation: Some(ObligationSpec {
            terminal,
            failure: None,
            deadline_tick: GameTick(adapter.snapshot().tick.0 + 25),
            poll_interval_ticks: 10,
            stable_for_observations: 3,
        }),
        depends_on: Vec::new(),
    };
    prepare(adapter, id, vec![step], 25)
}

#[test]
fn short_prior_wait_cannot_postpone_original_forecast_samples_past_the_deadline() -> Result<()> {
    let mut adapter = fixture()?;
    let plan = cadence_original(&adapter, 22)?;
    let original = commit(&mut adapter, &plan)?;
    let tracked = original.actions.clone();
    let uninterrupted = predict(
        &adapter, &[&plan], &[original.clone()], &tracked, 25, &context(&adapter),
    )?;
    assert_eq!(uninterrupted["all_complete"], true, "{uninterrupted}");

    adapter.advance_ticks(7)?;
    let early = adapter.poll_action(tracked[0], &context(&adapter))?;
    assert_eq!(early.state, CommitState::AppliedAwaitingVerification);
    let before = format!("{adapter:?}");
    let resumed = predict(
        &adapter, &[&plan], &[original], &tracked, 18, &context(&adapter),
    )?;
    assert_eq!(resumed["all_complete"], true, "{resumed}");
    assert_eq!(resumed["predicted_frontier_tick"], uninterrupted["predicted_frontier_tick"]);
    assert_eq!(resumed["originals"][0]["steps"][0]["predicted_state"], "Verified");
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn another_originals_exact_reserve_frontier_cannot_move_the_sampling_clock() -> Result<()> {
    let mut adapter = fixture()?;
    let first_plan = cadence_original(&adapter, 23)?;
    let first = commit(&mut adapter, &first_plan)?;
    let second_plan = prepare(
        &adapter,
        24,
        vec![requested(Action::SetLabor {
            units: vec![dfmcp_core::EntityId::new(1006)],
            labor: "BREW".to_owned(),
            enabled: true,
        }, Vec::new())],
        25,
    )?;
    let mut second = commit(&mut adapter, &second_plan)?;
    second.reserve_until = Some(adapter.snapshot().tick.0 + 7);
    let tracked = first.actions.iter().chain(&second.actions).copied().collect::<Vec<_>>();
    let before = format!("{adapter:?}");
    let result = predict(
        &adapter, &[&first_plan, &second_plan], &[first, second], &tracked, 25, &context(&adapter),
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["predicted_frontier_tick"], adapter.snapshot().tick.0 + 25);
    assert_eq!(result["originals"][1]["reserve_frontier_truth"], "true");
    assert_eq!(result["originals"][0]["steps"][0]["predicted_state"], "Verified");
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

fn ready_pause(resume: bool) -> Result<(MemoryAdapter, PreparedPlan, Original, Vec<ActionId>)> {
    let mut adapter=fixture()?;
    let mut actions=vec![order("competitor",1),requested(Action::Pause {paused:true},vec![0])];
    if resume {actions.push(requested(Action::Pause {paused:false},vec![1]));}
    let competitor=prepare(&adapter,31,actions,1_000)?;
    let competing=commit(&mut adapter,&competitor)?;
    adapter.advance_ticks(50)?;
    adapter.poll_action(competing.actions[0],&context(&adapter))?;
    assert_eq!(adapter.action_receipt(competing.actions[1]).map(|row|row.state),Some(CommitState::Prepared));
    let plan=prepare(&adapter,32,vec![order("selected original",4)],1_000)?;
    let original=commit(&mut adapter,&plan)?;
    let tracked=competing.actions.iter().chain(&original.actions).copied().collect();
    Ok((adapter,plan,original,tracked))
}

#[test]
fn unselected_ready_deferred_pause_stops_prediction_at_the_same_current_tick() -> Result<()> {
    let (adapter,plan,original,tracked)=ready_pause(false)?;
    let before=format!("{adapter:?}");
    let result=predict(&adapter,&[&plan],&[original],&tracked,1_000,&context(&adapter))?;
    assert_eq!(result["stop_reason"],"paused","{result}");
    assert_eq!(result["predicted_frontier_tick"],adapter.snapshot().tick.0);
    assert_eq!(result["forecast_work"]["deferred_dispatches"],1);
    assert_eq!(format!("{adapter:?}"),before);
    Ok(())
}

#[test]
fn pause_and_resume_are_evaluated_as_one_complete_owned_dispatch_round() -> Result<()> {
    let (adapter,plan,original,tracked)=ready_pause(true)?;
    let result=predict(&adapter,&[&plan],&[original],&tracked,1_000,&context(&adapter))?;
    assert_eq!(result["all_complete"],true,"{result}");
    assert_eq!(result["predicted_paused"],false);
    assert_eq!(result["forecast_work"]["deferred_dispatches"],2);
    Ok(())
}

#[test]
fn zero_horizon_does_not_poll_even_an_already_ready_deferred_action() -> Result<()> {
    let (adapter,plan,original,tracked)=ready_pause(false)?;
    let result=predict(&adapter,&[&plan],&[original],&tracked,0,&context(&adapter))?;
    assert_eq!(result["stop_reason"],"horizon_reached","{result}");
    assert_eq!(result["predicted_paused"],false);
    assert_eq!(result["forecast_work"]["action_polls"],0);
    Ok(())
}

#[test]
fn prediction_needs_no_live_clock_grant_but_cannot_mint_deferred_effect_authority() -> Result<()> {
    let mut adapter=fixture()?;
    let plan=prepare(&adapter,41,vec![order("read-only model",1)],100)?;
    let original=commit(&mut adapter,&plan)?;
    let tracked=original.actions.clone();
    let mut ctx=context(&adapter);
    ctx.grants.retain(|grant|grant.capability!=Capability::ControlClock);
    assert_eq!(predict(&adapter,&[&plan],&[original],&tracked,100,&ctx)?["all_complete"],true);
    let (adapter,plan,original,tracked)=ready_pause(false)?;
    let before=format!("{adapter:?}");
    let mut ctx=context(&adapter);
    ctx.grants.retain(|grant|grant.capability!=Capability::ControlClock);
    assert_eq!(predict(&adapter,&[&plan],&[original],&tracked,1_000,&ctx).err().map(|error|error.code),Some(ErrorCode::CapabilityDenied));
    assert_eq!(format!("{adapter:?}"),before);
    Ok(())
}

#[test]
fn exact_reserve_endpoints_do_not_latch_a_goal_that_consumption_later_invalidates() -> Result<()> {
    let mut adapter=fixture()?;
    let first_plan=prepare(&adapter,51,vec![order("first proof",1)],1_000)?;
    let mut first=commit(&mut adapter,&first_plan)?;
    let second_plan=prepare(&adapter,52,vec![order("second proof",1)],1_000)?;
    let mut second=commit(&mut adapter,&second_plan)?;
    first.reserve_until=Some(101);
    second.reserve_until=Some(1_201);
    let tracked=first.actions.iter().chain(&second.actions).copied().collect::<Vec<_>>();
    let goal=Predicate::FieldCompare {
        entity_id:starter::STOCK_LEDGER,field:effects::STOCK_DRINK_FIELD.to_owned(),
        op:CompareOp::Ge,value:WorldValue::U64(40),
    };
    let result=forecast(&adapter,&[first,second],&tracked,&context(&adapter),1_500,Instant::now(),None,Vec::new(),
        |_, _, _|Ok(()),
        |_,snapshot| {
            let truth=dfmcp_world::PredicateEvidence::laboratory(snapshot)?.evaluate(&goal)?;
            Ok((truth,truth))
        })?;
    assert_eq!(result["all_complete"],false,"{result}");
    assert_eq!(result["predicted_frontier_tick"],1_201);
    assert_eq!(result["originals"][0]["reserve_frontier_truth"],"true");
    assert_eq!(result["originals"][0]["predicted_goal_truth"],"false");
    assert_eq!(result["originals"][1]["reserve_frontier_truth"],"false");
    Ok(())
}

#[test]
fn retained_false_and_missed_reserve_endpoints_are_never_repaired_by_later_stock() -> Result<()> {
    for (sample,until,expected) in [
        (Some(PredicateTruth::False),1,"reserve_unmet"),
        (Some(PredicateTruth::Unknown),1,"reserve_unknown"),
        (None,0,"reserve_unobserved"),
    ] {
        let mut adapter=fixture()?;
        let plan=prepare(&adapter,61,vec![order("retained reserve",1)],1_000)?;
        let mut original=commit(&mut adapter,&plan)?;
        original.reserve_until=Some(until);
        original.reserve_observation=sample;
        let tracked=original.actions.clone();
        let result=predict(&adapter,&[&plan],&[original],&tracked,1_000,&context(&adapter))?;
        assert_eq!(result["originals"][0]["predicted_outcome"],expected,"{result}");
        assert_eq!(result["forecast_work"]["action_polls"],0);
    }
    Ok(())
}

#[test]
fn source_admission_and_expired_wall_budget_refuse_before_any_private_work() -> Result<()> {
    let mut adapter=fixture()?;
    let plan=prepare(&adapter,71,vec![order("budget",1)],100)?;
    let original=commit(&mut adapter,&plan)?;
    let tracked=original.actions.clone();
    let before=format!("{adapter:?}");
    let ctx=context(&adapter);
    let result=forecast(&adapter,&[original],&tracked,&ctx,100,
        Instant::now()-Duration::from_millis(ctx.budget.max_wall_millis+1),None,Vec::new(),
        |_, _, _|Ok(()),|_,_|Ok((PredicateTruth::True,PredicateTruth::True)));
    assert_eq!(result.err().map(|error|error.code),Some(ErrorCode::BudgetExceeded));
    assert_eq!(format!("{adapter:?}"),before);
    Ok(())
}

