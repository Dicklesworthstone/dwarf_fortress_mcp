//! Active recovery uses real finite work and the real retained proof runtime.
//! Durable admission and publication are covered by the facade companion.

use super::*;
use super::super::retained;
use dfmcp_core::GameTick;
use dfmcp_intent::{ObligationStatus, RecoveredObligation};
use server::{CarriedStep, recovered_step_progress};

#[path = "original_current_recovery_forecast_tests.rs"]
mod current;

fn direct_beds() -> Result<ProjectSource> {
    Ok(ProjectSource::Production(ProductionRequest::parse_current(
        &json!({"template":"production","quotas":[{"item":"BED","minimum":4}]}).to_string(),
    )?))
}

fn custom_original(
    live: &mut MemoryAdapter,
    id: u128,
    item: &str,
    deadline: u64,
    cadence: u64,
    stable: u32,
) -> Result<(PreparedPlan, Original, Project)> {
    let request = ProductionRequest::parse_current(
        &json!({"template":"production","quotas":[{"item":item,"minimum":1}]}).to_string(),
    )?;
    let ledger = ProductionGoalLedger::capture(live.snapshot())?;
    let (mut intent, _) = server::production_source::compile(
        &request, IntentId::new(id), live.snapshot(), "bounded recovered monitor fixture",
    )?;
    let mut temporal = 0;
    for (index, action) in intent.requested_actions.iter_mut().enumerate() {
        let normalized = action.action.normalized();
        let key = dfmcp_intent::derive_step_idempotency_key(
            intent.id, intent.anchor, StepId::new(index as u32), &normalized,
        );
        if action.postconditions.is_empty() {
            action.postconditions = effects::default_postconditions(&normalized, &key, intent.anchor.fortress_id);
        }
        let original = match action.obligation.take() {
            Some(spec) => Some(spec),
            None => effects::default_obligation(&normalized, &key, intent.anchor.fortress_id, intent.anchor.tick)?,
        };
        if let Some(mut spec) = original {
            spec.deadline_tick = GameTick(deadline);
            spec.poll_interval_ticks = cadence;
            spec.stable_for_observations = stable;
            action.obligation = Some(spec);
            temporal += 1;
        }
    }
    assert!(temporal > 0);
    let plan = StaticPlanner::default().prepare_laboratory(live.snapshot(), &intent, &context(live))?;
    ledger.validate_plan(&plan)?;
    let prepared = live.prepare(&plan, &context(live))?;
    let receipt = live.commit(&plan, &prepared, &context(live))?;
    let original = Original {
        requested: plan.digest, actual: plan.digest,
        actions: receipt.actions.into_iter().map(|action| action.action_id).collect(),
        reserve_until: None, reserve_observation: None, can_continue: true,
    };
    Ok((plan.clone(), original, Project::new(plan.digest, ProjectSource::Production(request), ledger)))
}

/// Construct the same observation-only runtime as real durable recovery. The
/// fixture owns real completed/dispatched action receipts and a real epoch reset;
/// production forecasting is never allowed to construct these monitors.
fn recovered_census(
    live: &MemoryAdapter,
    recovered: &MemoryAdapter,
    plan: &PreparedPlan,
    original: &Original,
) -> Result<(Vec<RecoveredStepCustody>, Vec<CarriedStep>)> {
    let custody = Digest32::of_bytes(b"active recovered original model fixture custody");
    let recorded = live.snapshot().anchor();
    let mut census = Vec::new();
    let mut carried = Vec::new();
    for (step, action) in plan.steps.iter().zip(&original.actions) {
        let receipt = live.action_receipt(*action)
            .ok_or_else(|| error(ErrorCode::InvalidPlan, "active fixture receipt missing"))?;
        let state = match receipt.state {
            CommitState::Verified => "verified",
            CommitState::AppliedAwaitingVerification => "dispatched",
            _ => return Err(error(ErrorCode::InvalidPlan, "fixture requires already-dispatched work")),
        };
        let work = dfmcp_intent::inspect_effect_work(
            recovered.snapshot(), &step.action, &step.idempotency_key, true,
        )?;
        if state == "dispatched" {
            let mut spec = step.obligation.clone()
                .ok_or_else(|| error(ErrorCode::InvalidPlan, "active fixture temporal specification missing"))?;
            let mut predicates = step.postconditions.clone();
            predicates.push(spec.terminal.clone());
            let proof = Predicate::All(predicates).normalized();
            spec.terminal = proof.clone();
            let monitor = RecoveredObligation::new_laboratory(
                recovered_step_progress::proof_id(plan.digest, step.id),
                spec.clone(), plan.anchor.tick, recovered.snapshot(),
            )?;
            carried.push(CarriedStep {
                plan_digest: plan.digest, step: Some(step.id),
                kind: crate::lab_world::action_kind(&step.action), proof,
                failure: spec.failure, deadline: Some(spec.deadline_tick), monitor: Some(monitor),
                recorded_state: state.to_owned(), proof_anchor: Some(recorded),
                observation_error: None, failure_reason: None,
                work_step: Some(step.clone()), work_state: work.clone(),
                work_anchor: Some(recovered.snapshot().anchor()), recovered_drain: None,
                state: state.to_owned(),
            });
        }
        census.push(RecoveredStepCustody {
            step: step.id, recorded_state: state, state, recorded_anchor: recorded,
            proof_anchor: Some(recorded), recovery_anchor: recovered.snapshot().anchor(),
            custody_digest: custody, work,
        });
    }
    assert_eq!(census.len(), plan.steps.len());
    assert!(!carried.is_empty());
    Ok((census, carried))
}

fn history(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    census: Vec<RecoveredStepCustody>,
    carried: &[CarriedStep],
) -> Result<RecoveredOriginal> {
    let custody = census.first().ok_or_else(|| error(ErrorCode::InvalidPlan, "empty fixture census"))?.custody_digest;
    RecoveredOriginal::new(plan, census, json!({"digest":custody.to_hex()}), adapter.snapshot(), carried)
}

fn predict(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    original: &Original,
    project: Project,
    carried: &[CarriedStep],
    ctx: &OperationContext,
    ticks: u64,
) -> Result<Value> {
    forecast(adapter, std::slice::from_ref(original), &[], ctx, ticks, Instant::now(),
        Some(vec![project]), carried.to_vec(),
        |snapshot, _, _| server::query_budget::admit_sources(&[snapshot], ctx).map(|_| ()),
        |_, snapshot| {
            let stage = crate::lab_world::plan_goal_truth(snapshot, plan)?;
            Ok((stage, stage))
        },
    )
}

#[test]
fn active_recovered_original_matches_real_observation_progress_without_mutating_custody() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = commit_project(&mut live, 816_601, direct_beds()?)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, carried) = recovered_census(&live, &adapter, &plan, &original)?;
    let historical_proof = server::anchor_json(&census[0].recorded_anchor);
    let project = project.with_recovery(history(&adapter, &plan, census, &carried)?)?;
    original.actions.clear();
    let before = format!("{adapter:?}|{carried:?}");
    let zero = predict(&adapter, &plan, &original, project.clone(), &carried, &context(&adapter), 0)?;
    assert_eq!(zero["all_complete"], false);
    assert_eq!(zero["forecast_work"]["recovered_observation_polls"], 0);
    let result = predict(&adapter, &plan, &original, project, &carried, &context(&adapter), 4_000)?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    assert_eq!(result["forecast_work"]["action_polls"], 0);
    assert!(result["forecast_work"]["recovered_observation_polls"].as_u64().is_some_and(|count| count > 0));
    let row = &result["originals"][0]["steps"][0];
    assert!(row["action_id"].is_null());
    assert_eq!(row["action_state_origin"], "cloned_recovered_observation_monitor");
    assert_eq!(row["predicted_state"], "Verified");
    assert_eq!(row["proof_anchor"], historical_proof);
    assert_eq!(row["predicted_proof_is_canonical"], false);
    assert!(row["predicted_proof_anchor"].is_object());
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    for step in &plan.steps {
        assert!(adapter.step_receipt(plan.id, step.id).is_none());
    }
    let mut actual = adapter.clone();
    let mut observed = carried.clone();
    for _ in 0..200 {
        observed = recovered_step_progress::observe(actual.snapshot(), &observed)?;
        if observed.iter().all(CarriedStep::is_final) {
            break;
        }
        let interval = recovered_step_progress::ceiling(actual.snapshot().tick, &observed).unwrap_or(100).min(100);
        actual.advance_ticks(interval)?;
    }
    assert!(observed.iter().all(|row| row.state == "verified" && row.is_final()));
    assert_eq!(result["predicted_frontier_tick"], actual.snapshot().tick.0);
    Ok(())
}

#[test]
fn active_recovered_child_finishes_after_immutable_historical_predecessors() -> Result<()> {
    let (live, root_plan, mut root, _, child_plan, mut child) = retained::continued_beds()?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let historical_root = recover_steps(&live, &adapter, &root_plan, &root)?;
    let (census, carried) = recovered_census(&live, &adapter, &child_plan, &child)?;
    let historical_child = history(&adapter, &child_plan, census, &carried)?;
    let ledger = ProductionGoalLedger::capture(fixture()?.snapshot())?;
    let mut project = Project::new(root.actual, beds()?, ledger).with_recovery(historical_root)?;
    root.actions.clear();
    child.actions.clear();
    project.retain_recovered_stage(child_plan.clone(), root.actual, child.clone(), historical_child, adapter.snapshot())?;
    let before = format!("{adapter:?}|{carried:?}");
    let zero = predict(&adapter, &root_plan, &root, project.clone(), &carried, &context(&adapter), 0)?;
    assert_eq!(zero["all_complete"], false);
    assert_eq!(zero["originals"][0]["predicted_committed_actions_complete"], false);
    let result = predict(&adapter, &root_plan, &root, project, &carried, &context(&adapter), 4_000)?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    assert_eq!(result["forecast_work"]["verified_recovered_original_steps"], root_plan.steps.len());
    assert_eq!(result["forecast_work"]["recovered_original_steps"], root_plan.steps.len() + child_plan.steps.len());
    let row = &result["originals"][0]["retained_stages"][0];
    assert_eq!(row["custody"], "recovered_observation_original");
    assert_eq!(row["actual_plan_digest"], child.actual.to_hex());
    assert_eq!(row["predicted_actions_complete"], true);
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    Ok(())
}

#[test]
fn private_observation_keeps_the_real_recovery_streak_and_ignores_duplicate_ticks() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = custom_original(&mut live, 816_602, "BED", 5_000, 70, 3)?;
    live.advance_ticks(2_000)?;
    let mut adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, mut carried) = recovered_census(&live, &adapter, &plan, &original)?;
    assert!(census.iter().all(|row| row.work.is_quiescent()));
    adapter.advance_ticks(70)?;
    carried = recovered_step_progress::observe(adapter.snapshot(), &carried)?;
    carried = recovered_step_progress::observe(adapter.snapshot(), &carried)?;
    assert!(carried.iter().all(|row| matches!(row.monitor.as_ref().and_then(|monitor| monitor.status()),
        Some(ObligationStatus::Active { consecutive_stable_observations: 1, .. }))));
    let project = project.with_recovery(history(&adapter, &plan, census, &carried)?)?;
    original.actions.clear();
    let before = format!("{adapter:?}|{carried:?}");
    for (ticks, complete) in [(0, false), (70, false), (140, true)] {
        let result = predict(&adapter, &plan, &original, project.clone(), &carried, &context(&adapter), ticks)?;
        assert_eq!(result["all_complete"], complete, "{result}");
        assert_eq!(result["predicted_frontier_tick"], adapter.snapshot().tick.0 + ticks);
    }
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    Ok(())
}

#[test]
fn recovered_deadline_cannot_be_extended_to_finish_a_missing_stability_window() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = custom_original(&mut live, 816_603, "BED", 2_100, 70, 3)?;
    live.advance_ticks(2_099 - live.snapshot().tick.0)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, carried) = recovered_census(&live, &adapter, &plan, &original)?;
    let project = project.with_recovery(history(&adapter, &plan, census, &carried)?)?;
    original.actions.clear();
    let result = predict(&adapter, &plan, &original, project, &carried, &context(&adapter), 100)?;
    assert_eq!(result["all_complete"], false);
    assert_eq!(result["predicted_frontier_tick"], 2_100);
    assert_eq!(result["originals"][0]["predicted_outcome"], "failed");
    assert_eq!(result["originals"][0]["predicted_physical_quiescent"], true);
    assert_eq!(result["originals"][0]["steps"][0]["obligation_deadline_tick"], 2_100);
    assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
    Ok(())
}

#[test]
fn recovered_peers_share_cadence_and_count_against_the_complete_action_budget() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = custom_original(&mut live, 816_604, "BED", 5_000, 70, 3)?;
    let (peer_plan, peer, _) = custom_original(&mut live, 816_605, "CHAIR", 5_000, 10, 30)?;
    live.advance_ticks(2_000)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, mut carried) = recovered_census(&live, &adapter, &plan, &original)?;
    let (_, peer_rows) = recovered_census(&live, &adapter, &peer_plan, &peer)?;
    carried.extend(peer_rows);
    let project = project.with_recovery(history(&adapter, &plan, census, &carried)?)?;
    original.actions.clear();
    let mut tight = context(&adapter);
    tight.budget.max_actions = plan.steps.len() as u32;
    assert!(matches!(predict(&adapter, &plan, &original, project.clone(), &carried, &tight, 0),
        Err(failure) if failure.code == ErrorCode::BudgetExceeded));
    let result = predict(&adapter, &plan, &original, project, &carried, &context(&adapter), 300)?;
    assert_eq!(result["all_complete"], true, "{result}");
    assert_eq!(result["predicted_frontier_tick"], adapter.snapshot().tick.0 + 210);
    assert_eq!(result["forecast_work"]["competing_recovered_steps"], peer_plan.steps.len());
    assert!(result["forecast_work"]["recovered_observation_polls"].as_u64().is_some_and(|count| count > 20));
    assert!(result["forecast_work"]["recovered_predicate_node_visits_bound"].as_u64().is_some_and(|count| count > 0));
    Ok(())
}

#[test]
fn active_recovery_requires_the_exact_existing_monitor_and_cannot_be_a_predecessor() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = commit_project(&mut live, 816_606, direct_beds()?)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, carried) = recovered_census(&live, &adapter, &plan, &original)?;
    assert!(history(&adapter, &plan, census.clone(), &[]).is_err());
    let mut missing = carried.clone();
    missing[0].monitor = None;
    assert!(history(&adapter, &plan, census.clone(), &missing).is_err());
    let mut altered = carried.clone();
    altered[0].plan_digest = Digest32::of_bytes(b"foreign original");
    assert!(history(&adapter, &plan, census.clone(), &altered).is_err());
    let reset = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    assert!(history(&reset, &plan, census.clone(), &carried).is_err());
    let mut unknown = census.clone();
    unknown[0].work = EffectWorkState::Unknown { entity_id: None, reason: "unavailable fixture work".to_owned() };
    assert!(history(&adapter, &plan, unknown, &carried).is_err());
    let recovered = history(&adapter, &plan, census, &carried)?;
    original.actions.clear();
    let mut root = project.with_recovery(recovered)?;
    // A separate real original has a valid seal but cannot be retained after
    // an unfinished recovered predecessor, even if the caller supplies a parent.
    let mut next = adapter.clone();
    let (child_plan, child, _) = custom_original(&mut next, 816_607, "CHAIR", 5_000, 10, 1)?;
    assert!(root.retain_stage(child_plan, original.actual, child, adapter.snapshot()).is_err());
    Ok(())
}

#[test]
fn unfinished_recovery_cannot_replace_missing_false_or_unknown_original_reserves() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = commit_project(&mut live, 816_608, direct_beds()?)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, carried) = recovered_census(&live, &adapter, &plan, &original)?;
    let project = project.with_recovery(history(&adapter, &plan, census, &carried)?)?;
    original.actions.clear();
    original.reserve_until = Some(adapter.snapshot().tick.0.saturating_sub(1));
    for (sample, expected) in [
        (None, "reserve_unobserved"),
        (Some(PredicateTruth::False), "reserve_unmet"),
        (Some(PredicateTruth::Unknown), "reserve_unknown"),
    ] {
        original.reserve_observation = sample;
        let result = predict(&adapter, &plan, &original, project.clone(), &carried, &context(&adapter), 4_000)?;
        assert_eq!(result["all_complete"], false, "{result}");
        assert_eq!(result["originals"][0]["predicted_outcome"], expected);
        assert_eq!(result["forecast_work"]["hypothetical_stages"], 0);
        assert_eq!(result["forecast_work"]["recovered_observation_polls"], 0);
    }
    Ok(())
}

#[test]
fn active_recovered_forecast_rechecks_observe_expiry_before_private_proof() -> Result<()> {
    let mut live = fixture()?;
    let (plan, mut original, project) = commit_project(&mut live, 816_609, direct_beds()?)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let (census, carried) = recovered_census(&live, &adapter, &plan, &original)?;
    let project = project.with_recovery(history(&adapter, &plan, census, &carried)?)?;
    original.actions.clear();
    let mut ctx = context(&adapter);
    let grant = ctx.grants.iter_mut().find(|grant| grant.capability == Capability::Observe)
        .ok_or_else(|| error(ErrorCode::InvalidPlan, "fixture Observe grant missing"))?;
    grant.expires_at_tick = Some(GameTick(adapter.snapshot().tick.0 + 1));
    let before = format!("{adapter:?}|{carried:?}");
    let refused = predict(&adapter, &plan, &original, project, &carried, &ctx, 4_000);
    assert!(matches!(refused, Err(failure) if failure.code == ErrorCode::CapabilityDenied));
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    Ok(())
}

#[test]
fn current_stage_forecast_preserves_recovered_peer_cadence_and_shared_admission() -> Result<()> {
    let mut previous = fixture()?;
    let (peer_plan, peer, _) =
        custom_original(&mut previous, 816_610, "CHAIR", 10_000, 45, 100)?;
    previous.advance_ticks(2_000)?;
    let mut adapter = MemoryAdapter::recovered(previous.snapshot().clone())?;
    let (_, mut carried) = recovered_census(&previous, &adapter, &peer_plan, &peer)?;
    let (plan, original, _) =
        custom_original(&mut adapter, 816_611, "BED", 9_000, 70, 3)?;
    adapter.advance_ticks(2_000)?;
    carried = recovered_step_progress::observe(adapter.snapshot(), &carried)?;
    assert!(carried.iter().all(|row| row.state == "dispatched" && row.work_state.is_quiescent()));
    for action in &original.actions {
        assert!(adapter.action_work_state(*action)?.is_quiescent());
        assert!(adapter.action_receipt(*action).is_some_and(|receipt| {
            matches!(receipt.state, CommitState::Verified | CommitState::AppliedAwaitingVerification)
        }));
    }
    assert!(original.actions.iter().any(|action| adapter.action_receipt(*action)
        .is_some_and(|receipt| receipt.state == CommitState::AppliedAwaitingVerification)));
    let source_tick = adapter.snapshot().tick.0;
    let before = format!("{adapter:?}|{carried:?}");
    let ctx = context(&adapter);
    let predict_stage = |ctx: &OperationContext, peers: Vec<CarriedStep>| {
        forecast(
            &adapter,
            std::slice::from_ref(&original),
            &original.actions,
            ctx,
            4_000,
            Instant::now(),
            None,
            peers,
            |snapshot, _, _| server::query_budget::admit_sources(&[snapshot], ctx).map(|_| ()),
            |_, snapshot| {
                let goal = crate::lab_world::plan_goal_truth(snapshot, &plan)?;
                Ok((goal, goal))
            },
        )
    };
    let predicted = predict_stage(&ctx, carried.clone())?;
    assert_eq!(predicted["all_complete"], true, "{predicted}");
    assert_eq!(predicted["predicted_frontier_tick"], source_tick + 180);
    assert_eq!(predicted["forecast_work"]["competing_recovered_steps"], carried.len());
    assert!(predicted["forecast_work"]["recovered_observation_polls"]
        .as_u64().is_some_and(|count| count > 0));
    // Omitting the passive peer used to predict an earlier completion: the
    // selected obligation samples at 70/140 instead of 90/180 after its first
    // eligible source-frontier observation. The peer's own proof stays active.
    let omitted = predict_stage(&ctx, Vec::new())?;
    assert_eq!(omitted["predicted_frontier_tick"], source_tick + 140);
    let mut narrow = ctx.clone();
    narrow.budget.max_actions = u32::try_from(original.actions.len()).map_err(|_| {
        error(ErrorCode::InternalInvariantViolation, "fixture action count overflow")
    })?;
    assert!(matches!(predict_stage(&narrow, carried.clone()),
        Err(failure) if failure.code == ErrorCode::BudgetExceeded));
    let mut expires = ctx.clone();
    expires.grants.iter_mut().find(|grant| grant.capability == Capability::Observe)
        .ok_or_else(|| error(ErrorCode::InvalidPlan, "fixture Observe grant missing"))?
        .expires_at_tick = Some(GameTick(source_tick + 150));
    assert_eq!(predict_stage(&expires, Vec::new())?["all_complete"], true);
    assert!(matches!(predict_stage(&expires, carried.clone()),
        Err(failure) if failure.code == ErrorCode::CapabilityDenied));
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);

    // Compare against the actual foreground owner and shared recovered helper,
    // not a second implementation of cadence or obligation stability.
    let mut actual = adapter.clone();
    let mut observed = carried.clone();
    let mut actual_context = context(&actual);
    actual_context.grants.push(CapabilityGrant {
        capability: Capability::ControlClock,
        max_risk: RiskTier::Reversible,
        scope: CapabilityScope {
            fortress_id: Some(actual.snapshot().fortress_id),
            ..CapabilityScope::default()
        },
        expires_at_tick: None,
        remaining_uses: None,
    });
    let mut progress = server::lab_progress::ForegroundProgress::new(&actual, 4_000, &actual_context)?;
    let mut pending = original.actions.clone();
    for _ in 0..20 {
        let ceiling = recovered_step_progress::ceiling(actual.snapshot().tick, &observed);
        let Some(slice) = progress.step(&mut actual, &pending, &actual_context, ceiling)? else {
            break;
        };
        observed = recovered_step_progress::observe(actual.snapshot(), &observed)?;
        let mut unfinished = Vec::new();
        for action in pending {
            let receipt = actual.action_receipt(action).ok_or_else(|| {
                error(ErrorCode::InvalidPlan, "actual original receipt disappeared")
            })?;
            if !receipt.state.is_terminal() || !actual.action_work_state(action)?.is_quiescent() {
                unfinished.push(action);
            }
        }
        pending = unfinished;
        actual_context.anchor = actual.snapshot().anchor();
        if pending.is_empty() || slice.finished {
            break;
        }
    }
    assert!(pending.is_empty());
    assert!(original.actions.iter().all(|action| actual.action_receipt(*action)
        .is_some_and(|receipt| receipt.state == CommitState::Verified)));
    assert!(observed.iter().all(|row| row.state == "dispatched"));
    assert_eq!(predicted["predicted_frontier_tick"], actual.snapshot().tick.0);
    assert_eq!(format!("{adapter:?}|{carried:?}"), before);
    Ok(())
}
