//! Selected current-stage recovery reuses real monitor cadence and work.
//! No complete-project source, future stage or restored dispatch is supplied.

use super::*;

fn predict_current(
    adapter: &MemoryAdapter,
    plans: &[PreparedPlan],
    originals: &[Original],
    tracked: &[dfmcp_core::ActionId],
    recovered: Vec<RecoveredOriginal>,
    carried: &[CarriedStep],
    ctx: &OperationContext,
    ticks: u64,
) -> Result<Value> {
    forecast_with_recovered(
        adapter, originals, tracked, ctx, ticks, Instant::now(), None,
        recovered, carried.to_vec(),
        |snapshot, _, _| server::query_budget::admit_sources(&[snapshot], ctx).map(|_| ()),
        |digest, snapshot| {
            let plan = plans.iter().find(|plan| plan.digest == digest)
                .ok_or_else(|| error(ErrorCode::InvalidPlan, "selected fixture seal missing"))?;
            let truth = crate::lab_world::plan_goal_truth(snapshot, plan)?;
            Ok((truth, truth))
        },
    )
}

#[test]
fn selected_recovered_current_stage_preserves_streak_cadence_and_original_proof() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, _) = custom_original(&mut live, 816_620, "BED", 5_000, 70, 3)?;
    live.advance_ticks(2_000)?;
    let mut adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, mut carried) = recovered_census(&live, &adapter, &plan, &original)?;
    adapter.advance_ticks(70)?;
    carried = recovered_step_progress::observe(adapter.snapshot(), &carried)?;
    carried = recovered_step_progress::observe(adapter.snapshot(), &carried)?;
    let recovered = history(&adapter, &plan, census, &carried)?;
    original.actions.clear();
    let before = format!("{adapter:?}|{carried:?}");
    for (ticks, complete) in [(0, false), (70, false), (140, true)] {
        let predicted = predict_current(
            &adapter, std::slice::from_ref(&plan), std::slice::from_ref(&original), &[],
            vec![recovered.clone()], &carried, &context(&adapter), ticks,
        )?;
        assert_eq!(predicted["all_complete"], complete, "{predicted}");
        assert_eq!(predicted["predicted_frontier_tick"], adapter.snapshot().tick.0 + ticks);
        assert_eq!(predicted["forecast_work"]["action_polls"], 0);
        assert_eq!(predicted["forecast_work"]["deferred_dispatches"], 0);
        assert_eq!(predicted["forecast_work"]["competing_recovered_steps"], 0);
        assert_eq!(predicted["forecast_work"]["recovered_original_steps"], plan.steps.len());
        assert!(predicted["forecast_work"]["hypothetical_stages"].is_null());
        let row = &predicted["originals"][0];
        assert_eq!(row["selection_origin"], "durable_recovery");
        assert_eq!(row["dispatch_handles_restored"], false);
        assert!(row["future_stages"].is_null());
        let step = &row["steps"][0];
        assert_eq!(step["action_id"], Value::Null);
        assert_eq!(step["obligation_deadline_tick"], 5_000);
        assert_eq!(step["proof_anchor"],
            carried[0].proof_anchor.as_ref().map(server::anchor_json).unwrap_or(Value::Null));
        assert_eq!(step["predicted_proof_is_canonical"], false);
    }
    for capability in [Capability::Observe, Capability::Query] {
        let mut expires = context(&adapter);
        expires.grants.iter_mut().find(|grant| grant.capability == capability)
            .ok_or_else(|| error(ErrorCode::InvalidPlan, "fixture grant missing"))?
            .expires_at_tick = Some(GameTick(adapter.snapshot().tick.0 + 100));
        assert!(predict_current(
            &adapter, std::slice::from_ref(&plan), std::slice::from_ref(&original), &[],
            vec![recovered.clone()], &carried, &expires, 0,
        ).is_ok());
        assert!(matches!(predict_current(
            &adapter, std::slice::from_ref(&plan), std::slice::from_ref(&original), &[],
            vec![recovered.clone()], &carried, &expires, 140,
        ), Err(failure) if failure.code == ErrorCode::CapabilityDenied));
    }
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    assert!(plan.steps.iter().all(|step| adapter.step_receipt(plan.id, step.id).is_none()));
    Ok(())
}

#[test]
fn mixed_current_stage_selection_shares_real_work_and_counts_recovered_steps_once() -> Result<()> {
    let mut previous = fixture()?;
    let (old_plan, mut old, _) = custom_original(&mut previous, 816_621, "BED", 6_000, 70, 2)?;
    let mut adapter = MemoryAdapter::recovered(previous.snapshot().clone())?;
    let (census, carried) = recovered_census(&previous, &adapter, &old_plan, &old)?;
    let recovered = history(&adapter, &old_plan, census, &carried)?;
    old.actions.clear();
    let (new_plan, new, _) = custom_original(&mut adapter, 816_622, "CHAIR", 6_000, 45, 2)?;
    let originals = [old, new.clone()];
    let plans = [old_plan, new_plan];
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = (plans[0].steps.len() + plans[1].steps.len()) as u32;
    let before = format!("{adapter:?}|{carried:?}");
    let predicted = predict_current(
        &adapter, &plans, &originals, &new.actions, vec![recovered.clone()],
        &carried, &ctx, 4_000,
    )?;
    assert_eq!(predicted["all_complete"], true, "{predicted}");
    assert_eq!(predicted["forecast_work"]["competing_recovered_steps"], 0);
    assert_eq!(predicted["forecast_work"]["recovered_original_steps"], plans[0].steps.len());
    assert!(predicted["forecast_work"]["action_polls"].as_u64().is_some_and(|n| n > 0));
    assert!(predicted["forecast_work"]["recovered_observation_polls"].as_u64().is_some_and(|n| n > 0));
    ctx.budget.max_actions -= 1;
    assert!(matches!(predict_current(
        &adapter, &plans, &originals, &new.actions, vec![recovered],
        &carried, &ctx, 0,
    ), Err(failure) if failure.code == ErrorCode::BudgetExceeded));
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);

    let mut actual = adapter.clone();
    let mut observed = carried.clone();
    let mut actual_context = context(&actual);
    actual_context.grants.push(CapabilityGrant {
        capability: Capability::ControlClock, max_risk: RiskTier::Reversible,
        scope: CapabilityScope {
            fortress_id: Some(actual.snapshot().fortress_id), ..CapabilityScope::default()
        },
        expires_at_tick: None, remaining_uses: None,
    });
    let mut progress = server::lab_progress::ForegroundProgress::new(&actual, 4_000, &actual_context)?;
    let mut pending = new.actions.clone();
    for _ in 0..200 {
        let ceiling = recovered_step_progress::ceiling(actual.snapshot().tick, &observed);
        let Some(slice) = progress.step(&mut actual, &pending, &actual_context, ceiling)? else {
            break;
        };
        observed = recovered_step_progress::observe(actual.snapshot(), &observed)?;
        let mut unfinished = Vec::new();
        for action in pending {
            let receipt = actual.action_receipt(action)
                .ok_or_else(|| error(ErrorCode::InvalidPlan, "live fixture action disappeared"))?;
            if !receipt.state.is_terminal() || !actual.action_work_state(action)?.is_quiescent() {
                unfinished.push(action);
            }
        }
        pending = unfinished;
        actual_context.anchor = actual.snapshot().anchor();
        if (pending.is_empty() && observed.iter().all(CarriedStep::is_final)) || slice.finished {
            break;
        }
    }
    assert!(pending.is_empty());
    assert!(observed.iter().all(|row| row.state == "verified"));
    assert_eq!(predicted["predicted_frontier_tick"], actual.snapshot().tick.0);
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    Ok(())
}

#[test]
fn recovered_current_stage_keeps_deadline_failure_and_current_observation_authority() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, _) = custom_original(&mut live, 816_623, "BED", 2_100, 70, 3)?;
    live.advance_ticks(2_099 - live.snapshot().tick.0)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, carried) = recovered_census(&live, &adapter, &plan, &original)?;
    let recovered = history(&adapter, &plan, census, &carried)?;
    original.actions.clear();
    let before = format!("{adapter:?}|{carried:?}");
    let baseline = context(&adapter);
    assert!(!baseline.grants.iter().any(|grant| grant.capability == Capability::ControlClock));
    let call = |ctx: &OperationContext| predict_current(
        &adapter, std::slice::from_ref(&plan), std::slice::from_ref(&original), &[],
        vec![recovered.clone()], &carried, ctx, 100,
    );
    let predicted = call(&baseline)?;
    assert_eq!(predicted["predicted_frontier_tick"], 2_100);
    assert_eq!(predicted["originals"][0]["predicted_outcome"], "failed");
    assert_eq!(predicted["originals"][0]["predicted_physical_quiescent"], true);
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        let mut denied = baseline.clone();
        denied.grants.retain(|grant| grant.capability != capability);
        assert!(matches!(call(&denied),
            Err(failure) if failure.code == ErrorCode::CapabilityDenied));
    }
    let reset = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    assert!(predict_current(
        &reset, std::slice::from_ref(&plan), std::slice::from_ref(&original), &[],
        vec![recovered.clone()], &carried, &context(&reset), 0,
    ).is_err());
    assert!(predict_current(
        &adapter, std::slice::from_ref(&plan), std::slice::from_ref(&original), &[],
        vec![recovered.clone(), recovered], &carried, &baseline, 0,
    ).is_err());
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    Ok(())
}
