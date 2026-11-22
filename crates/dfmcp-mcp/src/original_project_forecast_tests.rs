//! Complete original projects use real finite compilers and a single adapter.

use super::super::super as server;
use super::*;
use crate::lab_world::{ProductionGoalLedger, ProductionRequest, scenario_snapshot};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, IntentId, RequestId, SessionId, WorkBudget,
};
use dfmcp_intent::{PreparedPlan, StaticPlanner};
use server::project_forecast::ProjectSource;

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(816_201),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        cancellation_requested: false,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 8_000,
            max_entities: 32_768,
            max_bytes: 4 * 1024 * 1024,
            max_actions: 128,
            max_output_tokens: 32_768,
            ..WorkBudget::default()
        },
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ConfigureProduction, RiskTier::Reversible),
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

fn fixture() -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(816_201),
        false,
    )?))
}

fn beds() -> Result<ProjectSource> {
    Ok(ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production","quotas":[{"item":"BED","minimum":24}],
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?))
}

fn commit_project(
    adapter: &mut MemoryAdapter,
    id: u128,
    source: ProjectSource,
) -> Result<(PreparedPlan, Original, Project)> {
    let ledger = ProductionGoalLedger::capture(adapter.snapshot())?;
    let until = match &source {
        ProjectSource::Production(request) => request.reserve_until_tick(),
        _ => None,
    };
    let (intent, _) = match &source {
        ProjectSource::Production(request) => server::production_source::compile(
            request,
            IntentId::new(id),
            adapter.snapshot(),
            "original shared project",
        )?,
        ProjectSource::Room(request) => request.compile(
            IntentId::new(id),
            adapter.snapshot(),
            "original shared room",
        )?,
        ProjectSource::ConnectedRoom(request) => request.compile(
            IntentId::new(id),
            adapter.snapshot(),
            "original shared room",
        )?,
    };
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter),
    )?;
    ledger.validate_plan(&plan)?;
    let prepared = adapter.prepare(&plan, &context(adapter))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter))?;
    let original = Original {
        requested: plan.digest,
        actual: plan.digest,
        actions: receipt
            .actions
            .into_iter()
            .map(|action| action.action_id)
            .collect(),
        reserve_until: until,
        reserve_observation: None,
        can_continue: true,
    };
    let project = Project::new(plan.digest, source, ledger);
    Ok((plan, original, project))
}

fn run(
    adapter: &MemoryAdapter,
    plans: &[PreparedPlan],
    originals: &[Original],
    projects: Option<Vec<Project>>,
    ctx: &OperationContext,
    ticks: u64,
) -> Result<Value> {
    let tracked = originals
        .iter()
        .flat_map(|original| original.actions.iter().copied())
        .collect::<Vec<_>>();
    forecast(
        adapter,
        originals,
        &tracked,
        ctx,
        ticks,
        Instant::now(),
        projects,
        |snapshot, extra_records, extra_bytes| {
            let work = server::query_budget::admit_sources(&[snapshot], ctx)?;
            if work["records"]
                .as_u64()
                .map_or(u64::MAX, |n| n.saturating_add(extra_records))
                > u64::from(ctx.budget.max_entities)
                || work["canonical_bytes"]
                    .as_u64()
                    .map_or(u64::MAX, |n| n.saturating_add(extra_bytes))
                    > ctx.budget.max_bytes
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "complete project fixture source budget",
                ));
            }
            Ok(())
        },
        |digest, snapshot| {
            let plan = plans
                .iter()
                .find(|plan| plan.digest == digest)
                .ok_or_else(|| error(ErrorCode::InvalidPlan, "missing fixture plan"))?;
            let truth = crate::lab_world::plan_goal_truth(snapshot, plan)?;
            Ok((truth, truth))
        },
    )
}

#[test]
fn harvested_input_becomes_a_complete_original_project_only_in_explicit_scope() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, original, project) = commit_project(&mut adapter, 1, beds()?)?;
    let before = format!("{adapter:?}");
    let ctx = context(&adapter);
    let stages = forecast(
        &adapter,
        std::slice::from_ref(&original),
        &original.actions,
        &ctx,
        4_000,
        Instant::now(),
        None,
        |_, _, _| Ok(()),
        |_, snapshot| {
            Ok((
                project.goal_truth(snapshot)?,
                crate::lab_world::plan_goal_truth(snapshot, &plan)?,
            ))
        },
    )?;
    assert_eq!(
        stages["originals"][0]["predicted_outcome"], "needs_continuation",
        "{stages}"
    );
    let result = run(
        &adapter,
        &[plan],
        &[original],
        Some(vec![project]),
        &ctx,
        4_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(
        result["originals"][0]["future_stages"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        result["originals"][0]["future_stages"][0]["stage"],
        "production"
    );
    assert_eq!(
        result["originals"][0]["predicted_project_actions_complete"],
        true
    );
    assert_eq!(result["forecast_work"]["source_forks"], 1);
    assert_eq!(result["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn production_and_room_projects_progress_on_the_same_finite_source() -> Result<()> {
    let mut adapter = fixture()?;
    let (first, a, pa) = commit_project(&mut adapter, 2, beds()?)?;
    let room=ProjectSource::Room(server::room_provisioning::ProvisionedRoomRequest::parse_current(&json!({
        "template":"provisioned_room","acquire_resources":{"assign_labor":false},
        "layout":{"template":"bedroom_cluster","origin":[2,6,10],"rooms":1,"room_size":[3,3]},
        "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1"},
    }).to_string())?);
    let (second, b, pb) = commit_project(&mut adapter, 3, room)?;
    let before = format!("{adapter:?}");
    let ctx = context(&adapter);
    let result = run(
        &adapter,
        &[first, second],
        &[a, b],
        Some(vec![pa, pb]),
        &ctx,
        8_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["source_forks"], 1);
    assert_eq!(result["originals"].as_array().map(Vec::len), Some(2));
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn current_true_stock_still_funds_the_original_future_reserve_after_harvesting() -> Result<()> {
    use crate::lab_world::starter;
    use dfmcp_intent::effects;
    use dfmcp_world::Value as WorldValue;

    let mut snapshot = fixture()?.snapshot().clone();
    let plants = snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_PLANT_FIELD))
        .ok_or_else(|| {
            error(
                ErrorCode::InvalidPlan,
                "reserve fixture plant stock missing",
            )
        })?;
    plants.value = WorldValue::U64(0);
    snapshot.refresh_hash();
    let mut adapter = MemoryAdapter::new(snapshot);
    let source = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production", "reserve_until_tick":2401,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?);
    let (plan, original, project) = commit_project(&mut adapter, 30, source)?;
    assert_eq!(
        project.goal_truth(adapter.snapshot())?,
        PredicateTruth::True
    );
    let before = format!("{adapter:?}");
    let mut witnessed_current_true_reserve_deficit = false;
    let prediction = forecast(
        &adapter,
        std::slice::from_ref(&original),
        &original.actions,
        &context(&adapter),
        4_000,
        Instant::now(),
        Some(vec![project.clone()]),
        |_, _, _| Ok(()),
        |_, snapshot| {
            let goal = project.goal_truth(snapshot)?;
            let stage = crate::lab_world::plan_goal_truth(snapshot, &plan)?;
            if snapshot.tick > plan.anchor.tick
                && goal == PredicateTruth::True
                && stage == PredicateTruth::True
            {
                witnessed_current_true_reserve_deficit = true;
            }
            Ok((goal, stage))
        },
    )?;
    assert!(witnessed_current_true_reserve_deficit, "{prediction}");
    assert_eq!(prediction["all_complete"], true, "{prediction}");
    assert_eq!(prediction["originals"][0]["reserve_until_tick"], 2401);
    assert_eq!(prediction["originals"][0]["reserve_frontier_truth"], "true");
    assert_eq!(
        prediction["originals"][0]["future_stages"][0]["stage"],
        "production"
    );
    assert_eq!(prediction["predicted_frontier_tick"], 2401);
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn future_effect_authority_refuses_before_shared_action_budget_charging() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, original, project) = commit_project(&mut adapter, 4, beds()?)?;
    let before = format!("{adapter:?}");
    let mut ctx = context(&adapter);
    ctx.grants
        .retain(|grant| grant.capability != Capability::ConfigureProduction);
    ctx.budget.max_actions = original.actions.len() as u32;
    let refused = run(
        &adapter,
        &[plan],
        &[original],
        Some(vec![project]),
        &ctx,
        4_000,
    );
    assert_eq!(
        refused.err().map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn future_plan_authority_expiry_cannot_be_replaced_by_current_grants() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, original, project) = commit_project(&mut adapter, 5, beds()?)?;
    let mut ctx = context(&adapter);
    for grant in &mut ctx.grants {
        if grant.capability == Capability::Plan {
            grant.expires_at_tick = Some(dfmcp_core::GameTick(adapter.snapshot().tick.0));
        }
    }
    let refused = run(
        &adapter,
        &[plan],
        &[original],
        Some(vec![project]),
        &ctx,
        4_000,
    );
    assert_eq!(
        refused.err().map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    Ok(())
}

#[test]
fn future_query_authority_expiry_refuses_predicted_observations() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, original, project) = commit_project(&mut adapter, 31, beds()?)?;
    let mut ctx = context(&adapter);
    for grant in &mut ctx.grants {
        if grant.capability == Capability::Query {
            grant.expires_at_tick = Some(dfmcp_core::GameTick(adapter.snapshot().tick.0));
        }
    }
    assert_eq!(
        run(
            &adapter,
            &[plan],
            &[original],
            Some(vec![project]),
            &ctx,
            4_000
        )
        .err()
        .map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    Ok(())
}

#[test]
fn modeled_stages_share_the_original_action_and_source_bounds() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, original, project) = commit_project(&mut adapter, 6, beds()?)?;
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = original.actions.len() as u32;
    assert_eq!(
        run(
            &adapter,
            &[plan],
            &[original],
            Some(vec![project]),
            &ctx,
            4_000
        )
        .err()
        .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn zero_horizon_and_paused_world_do_not_invent_clock_progress() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, original, project) = commit_project(&mut adapter, 7, beds()?)?;
    let ctx = context(&adapter);
    let zero = run(
        &adapter,
        &[plan.clone()],
        &[original.clone()],
        Some(vec![project.clone()]),
        &ctx,
        0,
    )?;
    assert_eq!(zero["forecast_work"]["hypothetical_stages"], 0);
    assert_eq!(zero["forecast_work"]["action_polls"], 0);
    let paused = MemoryAdapter::new(scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(816_202),
        true,
    )?);
    let mut paused = paused;
    let (plan, original, project) = commit_project(&mut paused, 8, beds()?)?;
    let result = run(
        &paused,
        &[plan],
        &[original],
        Some(vec![project]),
        &context(&paused),
        4_000,
    )?;
    assert_eq!(result["all_complete"], false);
    assert_eq!(result["stop_reason"], "paused");
    assert_eq!(result["forecast_work"]["simulated_game_ticks"], 0);
    Ok(())
}

#[test]
fn failed_original_reserve_sample_is_not_rescued_by_a_future_stage() -> Result<()> {
    let mut adapter = fixture()?;
    let (plan, mut original, project) = commit_project(&mut adapter, 9, beds()?)?;
    original.reserve_until = Some(adapter.snapshot().tick.0);
    let result = run(
        &adapter,
        &[plan],
        &[original],
        Some(vec![project]),
        &context(&adapter),
        4_000,
    )?;
    assert_eq!(result["all_complete"], false, "{result}");
    assert_eq!(result["originals"][0]["reserve_frontier_truth"], "false");
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    Ok(())
}
