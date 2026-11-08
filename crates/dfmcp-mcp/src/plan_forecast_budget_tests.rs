//! Shared comparison budgets exercise the existing planner and reference model.

use super::*;
use std::time::{Duration, Instant};

use crate::lab_world::{scenario_snapshot, starter};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, GameTick, IntentId, RequestId,
    RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner, effects};
use dfmcp_world::{CompareOp, Predicate, Value as WorldValue};

fn fixture(amount: u32, deadline: u64, paused: bool)
-> Result<(MemoryAdapter, PreparedPlan, OperationContext)> {
    let adapter = MemoryAdapter::new(scenario_snapshot(
        "starter_fortress", FortressId::new(796), paused,
    )?);
    let current = OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(), cancellation_requested: false,
        budget: WorkBudget {
            max_wall_millis:60_000, max_game_ticks:20_000, max_entities:10_000,
            max_bytes:4*1024*1024, max_output_tokens:32_768,
            ..WorkBudget::default()
        },
        grants:[
            (Capability::Plan,RiskTier::ReadOnly), (Capability::Observe,RiskTier::ReadOnly),
            (Capability::Query,RiskTier::ReadOnly),
            (Capability::ConfigureProduction,RiskTier::Reversible),
            (Capability::ControlClock,RiskTier::Reversible),
        ].into_iter().map(|(capability,max_risk)| CapabilityGrant {
            capability,max_risk,scope:CapabilityScope {
                fortress_id:Some(adapter.snapshot().fortress_id), ..CapabilityScope::default()
            }, expires_at_tick:None,remaining_uses:None,
        }).collect(),
    };
    let goal = Predicate::FieldCompare {
        entity_id:starter::STOCK_LEDGER,field:effects::STOCK_DRINK_FIELD.to_owned(),
        op:CompareOp::Ge,value:WorldValue::U64(45),
    };
    let intent = Intent {
        id:IntentId::new(1),anchor:current.anchor,summary:"same reserve objective".to_owned(),
        terminal_condition:goal.clone(),constraints:vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions:vec![RequestedAction {
            action:Action::CreateWorkOrder {
                name:"comparison forecast work".to_owned(),job_token:"BREW_DRINK".to_owned(),
                amount,conditions:Vec::new(),
            },preconditions:vec![Predicate::True],postconditions:vec![goal.clone()],
            compensation:None,obligation:Some(ObligationSpec {
                terminal:goal,failure:None,deadline_tick:GameTick(deadline),
                poll_interval_ticks:10,stable_for_observations:1,
            }),depends_on:Vec::new(),
        }],
    };
    let plan = StaticPlanner::default().prepare_laboratory(adapter.snapshot(),&intent,&current)?;
    Ok((adapter,plan,current))
}

#[test]
fn bounded_forecast_reuses_the_exact_model_without_mutating_the_adapter() -> Result<()> {
    let (adapter,plan,current) = fixture(1,1001,false)?;
    let original = format!("{adapter:?}");
    let legacy = forecast_plan(&adapter,&plan,&current);
    let mut budget = ForecastBudget::new(&current,Instant::now(),1000)?;
    let source_work = budget.admit_source(adapter.snapshot(),&current)?;
    let mut bounded = forecast_plan_bounded(&adapter,&plan,&current,None,&mut budget)?;
    assert_eq!(bounded["available"],true,"{bounded}");
    assert_eq!(bounded["predicted_complete"],true,"{bounded}");
    assert_eq!(bounded["simulation_horizon_tick"],1001);
    assert_eq!(bounded["simulation_horizon_truncated"],false);
    for key in ["simulation_horizon_tick","simulation_horizon_reached","simulation_horizon_truncated"] {
        bounded.as_object_mut().ok_or_else(|| DfmcpError::new(
            ErrorCode::InternalInvariantViolation,"forecast is not an object"))?.remove(key);
    }
    assert_eq!(bounded,legacy);
    let work = budget.work_json();
    assert_eq!(work["candidates"],1);
    assert_eq!(work["source_forks"],1);
    assert_eq!(work["forked_source_records"],source_work["records"]);
    assert_eq!(work["forked_source_canonical_bytes"],source_work["canonical_bytes"]);
    assert_eq!(work["candidate_action_steps"],1);
    assert!(work["action_polls"].as_u64().is_some_and(|polls|polls>0));
    assert!(work["simulated_game_ticks"].as_u64().is_some_and(|ticks|ticks>0));
    assert_eq!(work["actual_game_ticks_advanced"],0);
    assert_eq!(format!("{adapter:?}"),original);
    Ok(())
}

#[test]
fn bounded_forecast_shares_simulated_ticks_and_preserves_original_deadlines() -> Result<()> {
    let (adapter,plan,mut current) = fixture(20,10_001,false)?;
    current.budget.max_game_ticks=75;
    let original = format!("{adapter:?}");
    let mut budget=ForecastBudget::new(&current,Instant::now(),50)?;
    let first=forecast_plan_bounded(&adapter,&plan,&current,None,&mut budget)?;
    assert_eq!(first["available"],true,"{first}");
    assert_eq!(first["predicted_complete"],false);
    assert_eq!(first["predicted_frontier_tick"],51);
    assert_eq!(first["simulation_horizon_tick"],51);
    assert_eq!(first["simulation_horizon_reached"],true);
    assert_eq!(first["simulation_horizon_truncated"],true);
    assert_eq!(first["horizon_tick"],10_001);
    assert_eq!(budget.work_json()["simulated_game_ticks"],50);
    assert!(matches!(forecast_plan_bounded(&adapter,&plan,&current,None,&mut budget),
        Err(error) if error.code==ErrorCode::BudgetExceeded));
    assert!(budget.work_json()["simulated_game_ticks"].as_u64().is_some_and(|ticks|ticks<=75));
    assert_eq!(plan.steps[0].obligation.as_ref().map(|spec|spec.deadline_tick),Some(GameTick(10_001)));
    assert_eq!(format!("{adapter:?}"),original);
    Ok(())
}

#[test]
fn bounded_forecast_candidates_use_one_common_sampling_resolution() -> Result<()> {
    let (adapter,short,current)=fixture(20,1001,false)?;
    let (_,long,_)=fixture(20,100_001,false)?;
    let mut budget=ForecastBudget::new(&current,Instant::now(),5000)?;
    let shorter=forecast_plan_bounded(&adapter,&short,&current,None,&mut budget)?;
    let longer=forecast_plan_bounded(&adapter,&long,&current,None,&mut budget)?;
    assert_eq!(shorter["available"],true,"{shorter}");
    assert_eq!(longer["available"],true,"{longer}");
    assert_eq!(shorter["resolution_ticks"],longer["resolution_ticks"]);
    assert_eq!(shorter["resolution_ticks"],13);
    assert_eq!(shorter["horizon_tick"],1001);
    assert_eq!(longer["horizon_tick"],100_001);
    assert_eq!(shorter["simulation_horizon_tick"],longer["simulation_horizon_tick"]);
    Ok(())
}

#[test]
fn bounded_forecast_source_forks_and_actions_share_one_aggregate_allowance() -> Result<()> {
    for case in 0..3 {
        let (adapter,plan,mut current)=fixture(20,1001,true)?;
        let records=adapter.snapshot().graph.entities.len()+adapter.snapshot().graph.edges.len()
            +adapter.snapshot().graph.chunks.len()+adapter.snapshot().graph.events.len();
        match case {
            0=>current.budget.max_entities=records as u32,
            1=>current.budget.max_bytes=adapter.snapshot().canonical_bytes().len() as u64,
            _=>current.budget.max_actions=1,
        }
        let mut budget=ForecastBudget::new(&current,Instant::now(),500)?;
        let first=forecast_plan_bounded(&adapter,&plan,&current,None,&mut budget)?;
        assert_eq!(first["available"],true,"{first}");
        assert_eq!(first["blocked_by_pause"],true);
        assert!(matches!(forecast_plan_bounded(&adapter,&plan,&current,None,&mut budget),
            Err(error) if error.code==ErrorCode::BudgetExceeded));
        assert_eq!(budget.work_json()["source_forks"],1);
        assert_eq!(budget.work_json()["simulated_game_ticks"],0);
    }
    let (adapter,plan,mut current)=fixture(20,1001,true)?;
    current.budget.max_bytes=adapter.snapshot().canonical_bytes().len() as u64-1;
    let mut budget=ForecastBudget::new(&current,Instant::now(),500)?;
    assert!(matches!(forecast_plan_bounded(&adapter,&plan,&current,None,&mut budget),
        Err(error) if error.code==ErrorCode::BudgetExceeded));
    assert_eq!(budget.work_json()["source_forks"],0);
    Ok(())
}

#[test]
fn bounded_forecast_deadline_cancellation_and_current_authority_precede_forks() -> Result<()> {
    let (adapter,plan,current)=fixture(20,1001,false)?;
    let expired=Instant::now().checked_sub(Duration::from_millis(current.budget.max_wall_millis+1))
        .ok_or_else(|| DfmcpError::new(ErrorCode::InternalInvariantViolation,"clock underflow"))?;
    assert!(matches!(ForecastBudget::new(&current,expired,500),
        Err(error) if error.code==ErrorCode::BudgetExceeded));
    for case in 0..4 {
        let mut budget=ForecastBudget::new(&current,Instant::now(),500)?;
        let mut changed=current.clone();
        let expected=match case {
            0=>{changed.cancellation_requested=true;ErrorCode::CancellationRequested}
            1=>{changed.grants.retain(|grant|grant.capability!=Capability::Observe);ErrorCode::CapabilityDenied}
            2=>{changed.anchor.tick=GameTick(changed.anchor.tick.0+1);ErrorCode::StaleAnchor}
            _=>{changed.budget.max_bytes-=1;ErrorCode::BudgetExceeded}
        };
        assert!(matches!(forecast_plan_bounded(&adapter,&plan,&changed,None,&mut budget),
            Err(error) if error.code==expected));
        assert_eq!(budget.work_json()["source_forks"],0);
    }
    // Missing effect authority makes a model unavailable; it is never supplied
    // synthetically merely because the branch will later be discarded.
    let mut restricted=current.clone();
    restricted.grants.retain(|grant|grant.capability!=Capability::ConfigureProduction);
    let mut budget=ForecastBudget::new(&restricted,Instant::now(),500)?;
    let result=forecast_plan_bounded(&adapter,&plan,&restricted,None,&mut budget)?;
    assert_eq!(result["available"],false,"{result}");
    assert_eq!(result["reason"]["code"],"capability_denied");
    Ok(())
}

#[test]
fn bounded_forecast_reserve_frontiers_obey_the_common_horizon_and_charge_their_fork() -> Result<()> {
    let (adapter,plan,current)=fixture(1,1001,false)?;
    let original=format!("{adapter:?}");
    let mut budget=ForecastBudget::new(&current,Instant::now(),500)?;
    let result=forecast_plan_bounded(&adapter,&plan,&current,Some(251),&mut budget)?;
    assert_eq!(result["predicted_complete"],true,"{result}");
    assert_eq!(result["reserve_forecast"]["available"],true);
    assert_eq!(result["reserve_forecast"]["predicted_tick"],251);
    assert_eq!(result["reserve_forecast"]["changes_action_deadlines"],false);
    assert_eq!(budget.work_json()["source_forks"],2);
    assert_eq!(budget.work_json()["simulated_game_ticks"],250);
    let mut budget=ForecastBudget::new(&current,Instant::now(),500)?;
    let limited=forecast_plan_bounded(&adapter,&plan,&current,Some(1501),&mut budget)?;
    assert_eq!(limited["predicted_complete"],true,"{limited}");
    assert_eq!(limited["reserve_forecast"]["available"],false);
    assert_eq!(limited["reserve_forecast"]["reserve_until_tick"],1501);
    assert!(limited["reserve_forecast"]["reason"].as_str().is_some_and(|reason|reason.contains("horizon")));
    assert_eq!(budget.work_json()["source_forks"],1);
    assert_eq!(format!("{adapter:?}"),original);
    Ok(())
}
