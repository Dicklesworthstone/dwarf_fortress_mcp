//! A recovered prefix retains proof and reserves across real continuation work.

use super::super::retained;
use super::*;

fn recovered_chain(
    reserve_until: Option<u64>,
    retain_samples: (bool, bool),
) -> Result<(MemoryAdapter, PreparedPlan, Original, Project, PreparedPlan, Original)> {
    let mut live = fixture()?;
    let mut request = json!({
        "template":"production","quotas":[{"item":"BED","minimum":24}],
        "acquire_resources":{"assign_labor":false},
    });
    if let Some(until) = reserve_until {
        request["reserve_until_tick"] = json!(until);
    }
    let request = ProductionRequest::parse_current(&request.to_string())?;
    let source = ProjectSource::Production(request.clone());
    let ledger = ProductionGoalLedger::capture(live.snapshot())?;
    let (root_plan, mut root, mut project) = commit_project(&mut live, 816_511, source.clone())?;
    retained::finish(&mut live, &root.actions)?;
    let (child_plan, mut child, _) =
        retained::continue_production(&mut live, root.actual, request, &mut project)?;
    retained::finish(&mut live, &child.actions)?;
    if let Some(until) = reserve_until {
        assert!(live.snapshot().tick.0 < until);
        live.advance_ticks(until - live.snapshot().tick.0)?;
        let truth = source.goal_truth(live.snapshot(), ledger)?;
        assert_eq!(truth, PredicateTruth::True);
        root.reserve_observation = retain_samples.0.then_some(truth);
        child.reserve_observation = retain_samples.1.then_some(truth);
        live.advance_ticks(1)?;
    }
    let recovered = MemoryAdapter::recovered(live.snapshot().clone())?;
    let root_history = recover_steps(&live, &recovered, &root_plan, &root)?;
    let child_history = recover_steps(&live, &recovered, &child_plan, &child)?;
    root.actions.clear();
    child.actions.clear();
    let mut project = Project::new(root.actual, source, ledger).with_recovery(root_history)?;
    project.retain_recovered_stage(
        child_plan.clone(), root.actual, child.clone(), child_history, recovered.snapshot(),
    )?;
    Ok((recovered, root_plan, root, project, child_plan, child))
}

#[test]
fn recovered_committed_chain_counts_historical_steps_without_dispatch_or_polls() -> Result<()> {
    let (adapter, root_plan, root, project, child_plan, child) = recovered_chain(None, (false, false))?;
    let before = format!("{adapter:?}");
    let result = retained::forecast_chain(
        &adapter, &root_plan, &root, &child, project, &context(&adapter), 0,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["action_polls"], 0);
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    assert_eq!(result["forecast_work"]["tracked_actions"], 0);
    assert_eq!(result["forecast_work"]["retained_continuation_stages"], 1);
    assert_eq!(result["forecast_work"]["retained_continuation_actions"], child_plan.steps.len());
    assert_eq!(result["forecast_work"]["verified_recovered_continuation_steps"], child_plan.steps.len());
    assert_eq!(result["forecast_work"]["verified_recovered_original_steps"], root_plan.steps.len() + child_plan.steps.len());
    let retained = &result["originals"][0]["retained_stages"][0];
    assert_eq!(retained["actual_plan_digest"], child.actual.to_hex());
    assert_eq!(retained["custody"], "recovered_verified_original");
    assert_eq!(retained["selection_origin"], "durable_recovery");
    assert_eq!(retained["predicted_actions_complete"], true);
    assert_eq!(retained["dispatch_handles_restored"], false);
    assert_eq!(retained["recommitted_by_forecast"], false);
    assert!(retained["steps"].as_array().is_some_and(|rows| {
        rows.len() == child_plan.steps.len()
            && rows.iter().all(|row| row["action_id"].is_null()
                && row["action_state_origin"] == "verified_recovered_history")
    }));
    for plan in [&root_plan, &child_plan] {
        for step in &plan.steps {
            assert!(adapter.step_receipt(plan.id, step.id).is_none());
        }
    }
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn recovered_root_and_live_tip_keep_the_existing_jobs_and_deadlines() -> Result<()> {
    let (mut adapter, root_plan, root, mut project) = recovered_fixture()?;
    let ProjectSource::Production(request) = beds()? else {
        return Err(error(ErrorCode::InternalInvariantViolation, "production fixture missing"));
    };
    let (child_plan, child, _) =
        retained::continue_production(&mut adapter, root.actual, request, &mut project)?;
    let before = format!("{adapter:?}");
    for horizon in [0, 4_000] {
        let result = retained::forecast_chain(
            &adapter, &root_plan, &root, &child, project.clone(), &context(&adapter), horizon,
        )?;
        assert_eq!(result["all_complete"], horizon > 0, "{result}");
        assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
        let row = &result["originals"][0]["retained_stages"][0];
        assert_eq!(row["custody"], "already_committed_original");
        assert_eq!(row["actual_plan_digest"], child.actual.to_hex());
        assert_eq!(row["recommitted_by_forecast"], false);
        assert_eq!(row["source_anchor"], server::anchor_json(&child_plan.anchor));
        assert_eq!(row["steps"].as_array().map(|steps| steps.iter()
            .filter_map(|step| step["action_id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()),
            Some(child.actions.iter().map(ToString::to_string).collect::<Vec<_>>()));
        let expected: Vec<_> = child_plan.steps.iter().filter_map(|step|
            step.obligation.as_ref().map(|obligation| obligation.deadline_tick.0)
        ).collect();
        let deadlines = row["obligation_deadlines"].as_array().ok_or_else(||
            error(ErrorCode::InternalInvariantViolation, "retained deadlines missing"))?;
        assert_eq!(deadlines.iter().filter_map(|row| row["deadline_tick"].as_u64())
            .collect::<Vec<_>>(), expected);
        assert_eq!(format!("{adapter:?}"), before);
    }
    Ok(())
}

#[test]
fn recovered_descendants_share_the_total_action_budget() -> Result<()> {
    let (adapter, root_plan, root, project, child_plan, child) = recovered_chain(None, (false, false))?;
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = (root_plan.steps.len() + child_plan.steps.len() - 1) as u32;
    let refused = retained::forecast_chain(
        &adapter, &root_plan, &root, &child, project, &ctx, 0,
    );
    assert!(matches!(refused, Err(failure) if failure.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn recovered_committed_chain_predicts_new_stages_after_real_stock_consumption() -> Result<()> {
    let (mut adapter, root_plan, root, project, _, child) = recovered_chain(None, (false, false))?;
    retained::consume_bed(&mut adapter)?;
    assert_eq!(project.goal_truth(adapter.snapshot())?, PredicateTruth::False);
    let before = format!("{adapter:?}");
    let result = retained::forecast_chain(
        &adapter, &root_plan, &root, &child, project, &context(&adapter), 4_000,
    )?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 2);
    assert_eq!(result["originals"][0]["future_stages"][0]["index"], 3);
    assert_eq!(result["originals"][0]["future_stages"][1]["index"], 4);
    assert_eq!(result["originals"][0]["retained_stages"][0]["custody"],
        "recovered_verified_original");
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn recovered_chain_preserves_each_original_exact_reserve_endpoint() -> Result<()> {
    for samples in [(true, false), (false, true), (true, true)] {
        let (adapter, root_plan, root, project, _, child) = recovered_chain(Some(2_001), samples)?;
        let before = format!("{adapter:?}");
        let result = retained::forecast_chain(
            &adapter, &root_plan, &root, &child, project, &context(&adapter), 100,
        )?;
        assert_eq!(result["all_complete"], samples.0 && samples.1, "{result}");
        assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
        if !samples.0 || !samples.1 {
            assert_eq!(result["originals"][0]["predicted_outcome"], "reserve_unobserved");
        }
        if !samples.1 {
            assert_eq!(result["originals"][0]["retained_stages"][0]["predicted_outcome"], "reserve_unobserved");
        }
        assert_eq!(format!("{adapter:?}"), before);
    }
    Ok(())
}

#[test]
fn recovered_continuations_cannot_cross_reset_custody_or_reuse_pre_reset_handles() -> Result<()> {
    let (mut live, root_plan, root, _, child_plan, child) = retained::continued_beds()?;
    retained::finish(&mut live, &child.actions)?;
    let recovered = MemoryAdapter::recovered(live.snapshot().clone())?;
    let later = MemoryAdapter::recovered(recovered.snapshot().clone())?;
    let root_history = recover_steps(&live, &recovered, &root_plan, &root)?;
    let child_history = recover_steps(&live, &later, &child_plan, &child)?;
    let ledger = ProductionGoalLedger::capture(fixture()?.snapshot())?;
    let mut project = Project::new(root.actual, beds()?, ledger).with_recovery(root_history)?;
    let refused = project.retain_recovered_stage(
        child_plan.clone(), root.actual,
        Original { actions: Vec::new(), ..child.clone() }, child_history, later.snapshot(),
    );
    assert!(matches!(refused, Err(failure) if failure.code == ErrorCode::Conflict));
    let refused = project.retain_stage(child_plan, root.actual, child, live.snapshot());
    assert!(matches!(refused, Err(failure) if failure.code == ErrorCode::Conflict));
    Ok(())
}
