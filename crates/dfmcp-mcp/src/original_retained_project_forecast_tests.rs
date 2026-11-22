//! Already committed continuations retain real finite work and proof deadlines.

use super::*;

fn finish(adapter: &mut MemoryAdapter, actions: &[ActionId]) -> Result<()> {
    for _ in 0..160 {
        for action in actions {
            adapter.poll_action(*action, &context(adapter))?;
        }
        if actions.iter().all(|action| {
            adapter
                .action_receipt(*action)
                .is_some_and(|receipt| receipt.state == CommitState::Verified)
                && adapter
                    .action_work_state(*action)
                    .is_ok_and(|work| work.is_quiescent())
        }) {
            return Ok(());
        }
        adapter.advance_ticks(25)?;
    }
    Err(error(
        ErrorCode::VerificationTimeout,
        "fixture did not finish actual committed work",
    ))
}

fn continue_production(
    adapter: &mut MemoryAdapter,
    root: Digest32,
    request: ProductionRequest,
    project: &mut Project,
) -> Result<(PreparedPlan, Original, WorldSnapshot)> {
    let sealed = adapter.snapshot().clone();
    let source = server::goal_continuation::ProductionContinuation::new(root, root, request)?;
    let (intent, _) = source.compile(
        IntentId::new(816_310),
        adapter.snapshot(),
        "actual separately reviewed continuation",
    )?;
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter),
    )?;
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
        reserve_until: source.reserve_until_tick(),
        reserve_observation: None,
        can_continue: true,
    };
    project.retain_stage(plan.clone(), root, original.clone(), &sealed)?;
    Ok((plan, original, sealed))
}

fn continued_beds() -> Result<(
    MemoryAdapter,
    PreparedPlan,
    Original,
    Project,
    PreparedPlan,
    Original,
)> {
    let mut adapter = fixture()?;
    let ProjectSource::Production(request) = beds()? else {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "production fixture missing",
        ));
    };
    let (root_plan, root, mut project) = commit_project(
        &mut adapter,
        816_301,
        ProjectSource::Production(request.clone()),
    )?;
    finish(&mut adapter, &root.actions)?;
    let (child_plan, child, _) =
        continue_production(&mut adapter, root.actual, request, &mut project)?;
    Ok((adapter, root_plan, root, project, child_plan, child))
}

fn forecast_chain(
    adapter: &MemoryAdapter,
    root_plan: &PreparedPlan,
    root: &Original,
    child: &Original,
    project: Project,
    ctx: &OperationContext,
    ticks: u64,
) -> Result<Value> {
    forecast(
        adapter,
        std::slice::from_ref(root),
        &child.actions,
        ctx,
        ticks,
        Instant::now(),
        Some(vec![project]),
        |_, _, _| Ok(()),
        |_, snapshot| {
            let stage = crate::lab_world::plan_goal_truth(snapshot, root_plan)?;
            Ok((stage, stage))
        },
    )
}

#[test]
fn committed_continuation_uses_existing_receipts_work_and_absolute_deadlines() -> Result<()> {
    let (mut adapter, root_plan, root, project, child_plan, child) = continued_beds()?;
    adapter.advance_ticks(100)?;
    for action in &child.actions {
        adapter.poll_action(*action, &context(&adapter))?;
    }
    assert!(child.actions.iter().any(|action| matches!(
        adapter.action_work_state(*action),
        Ok(EffectWorkState::Active { .. })
    )));
    let before = format!("{adapter:?}");
    let result = forecast_chain(
        &adapter,
        &root_plan,
        &root,
        &child,
        project,
        &context(&adapter),
        4_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    assert_eq!(result["forecast_work"]["retained_continuation_stages"], 1);
    let retained = &result["originals"][0]["retained_stages"][0];
    assert_eq!(retained["actual_plan_digest"], child.actual.to_hex());
    assert_eq!(retained["parent_plan_digest"], root.actual.to_hex());
    assert_eq!(
        retained["source_anchor"]["game_tick"],
        child_plan.anchor.tick.0
    );
    assert_eq!(retained["recommitted_by_forecast"], false);
    assert_eq!(retained["predicted_actions_complete"], true);
    let rows = retained["steps"].as_array().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "retained steps missing",
        )
    })?;
    assert_eq!(
        rows.iter()
            .filter_map(|row| row["action_id"].as_str())
            .collect::<Vec<_>>(),
        child
            .actions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
    );
    let expected: Vec<_> = child_plan
        .steps
        .iter()
        .filter_map(|step| {
            step.obligation
                .as_ref()
                .map(|obligation| obligation.deadline_tick.0)
        })
        .collect();
    assert_eq!(
        retained["obligation_deadlines"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "deadlines missing"))?
            .iter()
            .filter_map(|row| row["deadline_tick"].as_u64())
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn active_retained_tip_prevents_early_completion_and_zero_horizon_dispatch() -> Result<()> {
    let (adapter, root_plan, root, project, _, child) = continued_beds()?;
    let before = format!("{adapter:?}");
    let result = forecast_chain(
        &adapter,
        &root_plan,
        &root,
        &child,
        project,
        &context(&adapter),
        0,
    )?;
    assert_eq!(result["all_complete"], false, "{result}");
    assert_eq!(result["stop_reason"], "horizon_reached");
    assert_eq!(result["forecast_work"]["action_polls"], 0);
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    assert_eq!(result["originals"][0]["predicted_actions_complete"], true);
    assert_eq!(
        result["originals"][0]["predicted_committed_actions_complete"],
        false
    );
    assert_eq!(
        result["originals"][0]["retained_stages"][0]["predicted_actions_complete"],
        false
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn quiet_predecessor_and_retained_tip_share_the_complete_action_budget() -> Result<()> {
    let (adapter, root_plan, root, project, _, child) = continued_beds()?;
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = (root.actions.len() + child.actions.len() - 1) as u32;
    let before = format!("{adapter:?}");
    let failure = forecast_chain(&adapter, &root_plan, &root, &child, project, &ctx, 4_000)
        .err()
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "aggregate budget unexpectedly accepted",
            )
        })?;
    assert_eq!(failure.code, ErrorCode::BudgetExceeded);
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn committed_chain_can_predict_further_stages_after_real_stock_consumption() -> Result<()> {
    let (mut adapter, root_plan, root, project, _, child) = continued_beds()?;
    finish(&mut adapter, &child.actions)?;
    assert_eq!(
        project.goal_truth(adapter.snapshot())?,
        PredicateTruth::True
    );
    let item =
        effects::loose_furniture_items(adapter.snapshot(), dfmcp_intent::FurnitureKind::Bed)?
            .into_iter()
            .next()
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "produced bed is missing",
                )
            })?;
    let action = Action::PlaceFurniture {
        kind: dfmcp_intent::FurnitureKind::Bed,
        item,
        location: dfmcp_core::MapCoord::new(0, 2, 10),
    };
    let id = IntentId::new(816_311);
    let key = dfmcp_intent::derive_step_idempotency_key(
        id,
        adapter.snapshot().anchor(),
        StepId::new(0),
        &action.normalized(),
    );
    let postconditions =
        effects::default_postconditions(&action, &key, adapter.snapshot().fortress_id);
    let intent = Intent {
        id,
        anchor: adapter.snapshot().anchor(),
        summary: "allocate a produced bed to another real project".to_owned(),
        terminal_condition: Predicate::All(postconditions.clone()),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions,
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };
    let furnishing = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter),
    )?;
    let prepared = adapter.prepare(&furnishing, &context(&adapter))?;
    let allocation = adapter.commit(&furnishing, &prepared, &context(&adapter))?;
    let allocated_actions: Vec<_> = allocation
        .actions
        .into_iter()
        .map(|action| action.action_id)
        .collect();
    finish(&mut adapter, &allocated_actions)?;
    assert_eq!(
        project.goal_truth(adapter.snapshot())?,
        PredicateTruth::False
    );
    let before = format!("{adapter:?}");
    let result = forecast_chain(
        &adapter,
        &root_plan,
        &root,
        &child,
        project,
        &context(&adapter),
        4_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["retained_continuation_stages"], 1);
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 2);
    let original = &result["originals"][0];
    assert_eq!(
        original["retained_stages"][0]["actual_plan_digest"],
        child.actual.to_hex()
    );
    assert_eq!(
        original["retained_stages"][0]["recommitted_by_forecast"],
        false
    );
    assert_eq!(original["predicted_committed_actions_complete"], true);
    assert_eq!(original["predicted_project_actions_complete"], true);
    assert_eq!(original["future_stages"][0]["index"], 3);
    assert_eq!(original["future_stages"][1]["index"], 4);
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn root_and_retained_tip_cannot_borrow_each_others_historical_reserve_sample() -> Result<()> {
    let mut adapter = fixture()?;
    let request = ProductionRequest::parse_current(
        &json!({
            "template":"production","quotas":[{"item":"BED","minimum":24}],
            "acquire_resources":{"assign_labor":false},"reserve_until_tick":2_001,
        })
        .to_string(),
    )?;
    let source = ProjectSource::Production(request.clone());
    let ledger = ProductionGoalLedger::capture(adapter.snapshot())?;
    let (root_plan, mut root, mut project) = commit_project(&mut adapter, 816_302, source.clone())?;
    finish(&mut adapter, &root.actions)?;
    let (child_plan, mut child, child_source) =
        continue_production(&mut adapter, root.actual, request, &mut project)?;
    finish(&mut adapter, &child.actions)?;
    assert!(adapter.snapshot().tick.0 < 2_001);
    adapter.advance_ticks(2_001 - adapter.snapshot().tick.0)?;
    let endpoint = source.goal_truth(adapter.snapshot(), ledger)?;
    assert_eq!(endpoint, PredicateTruth::True);
    adapter.advance_ticks(1)?;
    let before = format!("{adapter:?}");
    for missing_root in [false, true] {
        root.reserve_observation = (!missing_root).then_some(endpoint);
        child.reserve_observation = missing_root.then_some(endpoint);
        let mut project = Project::new(root.actual, source.clone(), ledger);
        project.retain_stage(
            child_plan.clone(),
            root.actual,
            child.clone(),
            &child_source,
        )?;
        let result = forecast_chain(
            &adapter,
            &root_plan,
            &root,
            &child,
            project,
            &context(&adapter),
            100,
        )?;
        assert_eq!(result["all_complete"], false, "{result}");
        assert_eq!(result["stop_reason"], "attention_required");
        assert_eq!(
            result["originals"][0]["predicted_outcome"],
            "reserve_unobserved"
        );
        assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
        if !missing_root {
            assert_eq!(
                result["originals"][0]["retained_stages"][0]["predicted_outcome"],
                "reserve_unobserved"
            );
        }
    }
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}
