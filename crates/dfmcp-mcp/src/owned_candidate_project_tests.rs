//! Complete candidates reuse the real shared model, finite resources and peers.

use super::*;
use crate::lab_world::{ProductionRequest, starter};
use dfmcp_world::{CompareOp, EntityKind, Value as WorldValue};

fn project_fixture() -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(crate::lab_world::scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(819_401),
        false,
    )?))
}

fn project_context(adapter: &MemoryAdapter) -> OperationContext {
    let mut current = context(adapter);
    for capability in [
        Capability::Designate,
        Capability::Construct,
        Capability::Checkpoint,
    ] {
        current.grants.push(CapabilityGrant {
            capability,
            max_risk: RiskTier::Guarded,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        });
    }
    current
}

fn bed_project() -> Result<ProjectSource> {
    Ok(ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production",
            "quotas":[{"item":"BED","minimum":24}],
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?))
}

fn prepare_project(adapter: &MemoryAdapter, source: &ProjectSource) -> Result<PreparedPlan> {
    source.prepare_joint_stage(
        Digest32::of_bytes(b"owned candidate complete-project fixture"),
        1,
        adapter.snapshot(),
        &project_context(adapter),
    )
}

fn predict_project(
    adapter: &MemoryAdapter,
    source: &ProjectSource,
    plan: &PreparedPlan,
    peers: &OwnedWork,
    current: &OperationContext,
    horizon: u64,
) -> Result<(Value, Value)> {
    let started = Instant::now();
    let mut budget = ForecastBudget::new_project(current, started, horizon)?;
    let result = peers.forecast_project(
        adapter,
        source,
        plan,
        current,
        source.reserve_until_tick(),
        started,
        &mut budget,
    )?;
    Ok((result, budget.work_json()))
}

fn project_with_deferred_pause(
    resume: bool,
) -> Result<(MemoryAdapter, ProjectSource, PreparedPlan, OwnedWork)> {
    let mut adapter = project_fixture()?;
    let mut actions = vec![
        bed("earlier completed bed", 1),
        requested(Action::Pause { paused: true }, vec![0]),
    ];
    if resume {
        actions.push(requested(Action::Pause { paused: false }, vec![1]));
    }
    let peer = prepare(&adapter, 819_402, actions, 10, 1, None)?;
    let actions = commit(&mut adapter, &peer)?;
    adapter.advance_ticks(50)?;
    adapter.poll_action(actions[0], &project_context(&adapter))?;
    assert_eq!(
        adapter
            .action_receipt(actions[0])
            .map(|receipt| receipt.state),
        Some(CommitState::Verified)
    );
    let source = bed_project()?;
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, actions, Vec::new());
    Ok((adapter, source, plan, peers))
}

#[test]
fn explicit_complete_project_follows_harvest_into_real_production() -> Result<()> {
    let adapter = project_fixture()?;
    let source = bed_project()?;
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, Vec::new(), Vec::new());
    let current = project_context(&adapter);
    let before = format!("{adapter:?}");
    let (stage, _) = predict(&adapter, &plan, &peers, &current, 4_000, None)?;
    assert_eq!(stage["predicted_complete"], false, "{stage}");
    assert_eq!(stage["predicted_actions_complete"], true);
    let (project, work) = predict_project(&adapter, &source, &plan, &peers, &current, 4_000)?;
    assert_eq!(project["schema"], "dfmcp.project-forecast/2");
    assert_eq!(project["predicted_complete"], true, "{project}");
    assert_eq!(project["predicted_goal_truth"], "true");
    assert_eq!(project["predicted_actions_complete"], true);
    assert_eq!(project["predicted_physical_quiescent"], true);
    assert_eq!(project["horizon_tick"], current.anchor.tick.0 + 4_000);
    assert_eq!(project["stages"].as_array().map(Vec::len), Some(2));
    assert_eq!(project["stages"][0]["stage"], "acquire_resources");
    assert_eq!(project["stages"][1]["stage"], "production");
    assert_eq!(project["project_max_risk"], "guarded");
    assert!(
        project["project_required_capabilities"]
            .as_array()
            .is_some_and(|caps| caps.contains(&json!("configure_production")))
    );
    assert_eq!(work["project_stages"], 2);
    assert_eq!(work["source_forks"], 2);
    assert_eq!(
        work["candidate_action_steps"],
        project["project_action_count"]
    );
    assert!(
        work["generated_project_custody_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0)
    );
    assert!(
        work["generated_project_custody_records"]
            .as_u64()
            .is_some_and(|records| records > 0)
    );
    assert_eq!(project["future_stage_plans_are_executable"], false);
    for stage in project["stages"]
        .as_array()
        .ok_or_else(|| invalid("missing stages"))?
    {
        assert_eq!(stage["model_actions_are_live_handles"], false);
        for step in stage["steps"]
            .as_array()
            .ok_or_else(|| invalid("missing steps"))?
        {
            assert!(step.get("action_id").is_none());
            assert!(step.get("model_action_id").is_some());
        }
    }
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn complete_projects_obey_owned_deferred_pause_and_resume() -> Result<()> {
    for resume in [false, true] {
        let (adapter, source, plan, peers) = project_with_deferred_pause(resume)?;
        let before = format!("{adapter:?}");
        let current = project_context(&adapter);
        let (project, work) = predict_project(&adapter, &source, &plan, &peers, &current, 4_000)?;
        assert_eq!(project["available"], true, "{project}");
        assert_eq!(project["predicted_complete"], resume, "{project}");
        assert_eq!(project["blocked_by_pause"], !resume);
        assert_eq!(work["competing_action_steps"], peers.actions.len());
        assert_eq!(
            project["shared_progression_work"]["deferred_dispatches"],
            if resume { 2 } else { 1 }
        );
        if !resume {
            assert_eq!(project["stages"].as_array().map(Vec::len), Some(1));
            assert_eq!(work["simulated_game_ticks"], 0);
        }
        assert_eq!(format!("{adapter:?}"), before);
    }
    Ok(())
}

#[test]
fn successor_authority_precedes_aggregate_action_limits() -> Result<()> {
    let adapter = project_fixture()?;
    let source = bed_project()?;
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, Vec::new(), Vec::new());
    let before = format!("{adapter:?}");
    for expired in [false, true] {
        let mut current = project_context(&adapter);
        if expired {
            for grant in &mut current.grants {
                if grant.capability == Capability::ConfigureProduction {
                    grant.expires_at_tick = Some(current.anchor.tick);
                }
            }
        } else {
            current
                .grants
                .retain(|grant| grant.capability != Capability::ConfigureProduction);
        }
        current.budget.max_actions = plan.steps.len() as u32;
        let (refused, work) = predict_project(&adapter, &source, &plan, &peers, &current, 4_000)?;
        assert_eq!(refused["available"], false, "{refused}");
        assert_eq!(refused["reason"]["code"], "capability_denied");
        assert_eq!(work["project_stages"], 1);
        assert_eq!(work["candidate_action_steps"], plan.steps.len());
    }
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn every_future_stage_shares_candidate_peer_and_source_custody_bounds() -> Result<()> {
    let (adapter, source, plan, peers) = project_with_deferred_pause(true)?;
    let before = format!("{adapter:?}");
    let current = project_context(&adapter);
    let (full, _) = predict_project(&adapter, &source, &plan, &peers, &current, 4_000)?;
    assert_eq!(full["predicted_complete"], true, "{full}");
    let total = full["project_action_count"]
        .as_u64()
        .ok_or_else(|| invalid("missing project actions"))?
        + peers.actions.len() as u64;
    let mut narrow = current.clone();
    narrow.budget.max_actions =
        u32::try_from(total - 1).map_err(|_| invalid("fixture action bound"))?;
    assert!(matches!(
        predict_project(&adapter, &source, &plan, &peers, &narrow, 4_000),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    // The encoded future plan shares current world and original custody bytes.
    let started = Instant::now();
    let mut budget = ForecastBudget::new_project(&current, started, 4_000)?;
    budget.begin_project(adapter.snapshot(), &plan, &current)?;
    budget.admit_owned_candidate(peers.actions.len() as u64, 0, 0, &current)?;
    assert!(matches!(
        budget.admit_owned_project_stage(adapter.snapshot(), &plan, current.budget.max_bytes, &current),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn complete_project_retains_exact_original_reserve_and_zero_horizon() -> Result<()> {
    let mut snapshot = project_fixture()?.snapshot().clone();
    snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_PLANT_FIELD))
        .ok_or_else(|| invalid("fixture plant stock missing"))?
        .value = WorldValue::U64(0);
    snapshot.refresh_hash();
    let adapter = MemoryAdapter::new(snapshot);
    let source = ProjectSource::Production(ProductionRequest::parse_current(
        &json!({
            "template":"production", "reserve_until_tick":2401,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )?);
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, Vec::new(), Vec::new());
    let current = project_context(&adapter);
    let before = format!("{adapter:?}");
    let (zero, work) = predict_project(&adapter, &source, &plan, &peers, &current, 0)?;
    assert_eq!(zero["predicted_complete"], false);
    assert_eq!(work["project_stages"], 1);
    assert_eq!(work["action_polls"], 0);
    assert_eq!(work["simulated_game_ticks"], 0);
    let (project, _) = predict_project(&adapter, &source, &plan, &peers, &current, 4_000)?;
    assert_eq!(project["predicted_complete"], true, "{project}");
    assert_eq!(project["reserve_forecast"]["predicted_tick"], 2401);
    assert_eq!(project["reserve_forecast"]["predicted_goal_truth"], "true");
    assert_eq!(
        project["reserve_forecast"]["continuous_maintenance_proven"],
        false
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn successor_commit_refreshes_and_rejects_failed_recovered_peer_before_completion() -> Result<()> {
    let mut live = project_fixture()?;
    let failure = Predicate::EntityQuantity {
        kind: EntityKind::WorkOrder,
        filter_field: "job_token".to_owned(),
        filter_value: WorldValue::Text("CONSTRUCT_BED".to_owned()),
        quantity_field: None,
        op: CompareOp::Ge,
        value: 2,
    };
    let peer = prepare(
        &live,
        819_403,
        vec![bed("retained recovered bed", 1)],
        45,
        100,
        Some(failure),
    )?;
    let actions = commit(&mut live, &peer)?;
    live.advance_ticks(1_000)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let mut carried = Vec::new();
    for (step, action) in peer.steps.iter().zip(actions) {
        assert_eq!(
            live.action_receipt(action).map(|row| row.state),
            Some(CommitState::AppliedAwaitingVerification)
        );
        let mut spec = step
            .obligation
            .clone()
            .ok_or_else(|| invalid("fixture obligation missing"))?;
        let proof = Predicate::All(
            step.postconditions
                .iter()
                .cloned()
                .chain([spec.terminal.clone()])
                .collect(),
        )
        .normalized();
        spec.terminal = proof.clone();
        let monitor = RecoveredObligation::new_laboratory(
            recovered_step_progress::proof_id(peer.digest, step.id),
            spec.clone(),
            peer.anchor.tick,
            adapter.snapshot(),
        )?;
        carried.push(CarriedStep {
            plan_digest: peer.digest,
            step: Some(step.id),
            kind: crate::lab_world::action_kind(&step.action),
            proof,
            failure: spec.failure,
            deadline: Some(spec.deadline_tick),
            monitor: Some(monitor),
            recorded_state: "dispatched".to_owned(),
            state: "dispatched".to_owned(),
            proof_anchor: Some(live.snapshot().anchor()),
            observation_error: None,
            failure_reason: None,
            work_step: Some(step.clone()),
            work_state: dfmcp_intent::inspect_effect_work(
                adapter.snapshot(),
                &step.action,
                &step.idempotency_key,
                true,
            )?,
            work_anchor: Some(adapter.snapshot().anchor()),
            recovered_drain: None,
        });
    }
    let source = bed_project()?;
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, Vec::new(), carried);
    let before = format!("{adapter:?}|{:?}", peers.carried);
    let (refused, work) = predict_project(
        &adapter,
        &source,
        &plan,
        &peers,
        &project_context(&adapter),
        4_000,
    )?;
    assert_eq!(refused["available"], false, "{refused}");
    assert!(
        refused["reason"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("candidate successor")
                && message.contains("recovered peer"))
    );
    assert_eq!(
        work["project_stages"], 2,
        "the initial harvest did not create another work order"
    );
    assert_eq!(refused["stages"].as_array().map(Vec::len), Some(2));
    assert_eq!(refused["stage_outcomes_available"], false);
    assert_eq!(
        refused["project_action_count"],
        work["candidate_action_steps"]
    );
    assert_eq!(format!("{adapter:?}|{:?}", peers.carried), before);
    Ok(())
}

#[test]
fn complete_room_keeps_every_furnishing_and_stage_cost() -> Result<()> {
    let adapter = project_fixture()?;
    let source = ProjectSource::parse_room(
        &json!({
            "template":"provisioned_room","acquire_resources":{"assign_labor":false},
            "layout":{"template":"bedroom_cluster","origin":[2,6,10],"rooms":1,"room_size":[3,3]},
            "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1"},
        })
        .to_string(),
    )?;
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, Vec::new(), Vec::new());
    let before = format!("{adapter:?}");
    let (project, work) = predict_project(
        &adapter,
        &source,
        &plan,
        &peers,
        &project_context(&adapter),
        4_000,
    )?;
    assert_eq!(project["predicted_complete"], true, "{project}");
    assert_eq!(project["semantic_goal"]["goal"]["kind"], "provisioned_room");
    assert_eq!(project["predicted_goal_truth"], "true");
    assert_eq!(project["predicted_actions_complete"], true);
    assert_eq!(project["predicted_physical_quiescent"], true);
    let stages = project["stages"]
        .as_array()
        .ok_or_else(|| invalid("missing room stages"))?;
    assert!(stages.iter().any(|stage| stage["stage"] == "room"));
    assert_eq!(work["project_stages"], stages.len());
    assert_eq!(
        work["candidate_action_steps"],
        project["project_action_count"]
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn generated_stage_archives_share_one_byte_budget_across_candidates() -> Result<()> {
    let adapter = project_fixture()?;
    let source = bed_project()?;
    let plan = prepare_project(&adapter, &source)?;
    let peers = owned(&adapter, Vec::new(), Vec::new());
    let current = project_context(&adapter);
    let before = format!("{adapter:?}");
    let (prediction, baseline) =
        predict_project(&adapter, &source, &plan, &peers, &current, 4_000)?;
    assert_eq!(prediction["predicted_complete"], true, "{prediction}");
    let measure = |work: &Value, key: &str| {
        work[key]
            .as_u64()
            .ok_or_else(|| invalid("missing forecast byte accounting"))
    };
    let copied_source = measure(&baseline, "forked_source_canonical_bytes")?;
    let copied_custody = measure(&baseline, "forked_retained_custody_bytes")?;
    let generated = measure(&baseline, "generated_project_custody_bytes")?;
    assert!(generated > 0);
    let one_candidate = copied_source
        .checked_add(copied_custody)
        .and_then(|bytes| bytes.checked_add(generated))
        .ok_or_else(|| invalid("fixture byte accounting overflow"))?;
    let mut narrow = current.clone();
    narrow.budget.max_bytes = one_candidate
        .checked_mul(2)
        .and_then(|bytes| bytes.checked_sub(1))
        .ok_or_else(|| invalid("fixture byte allowance overflow"))?;
    assert!(narrow.budget.max_bytes <= current.budget.max_bytes);
    let started = Instant::now();
    let mut budget = ForecastBudget::new_project(&narrow, started, 4_000)?;
    let first = peers.forecast_project(
        &adapter,
        &source,
        &plan,
        &narrow,
        None,
        started,
        &mut budget,
    )?;
    assert_eq!(first["predicted_complete"], true, "{first}");
    let first_work = budget.work_json();
    assert_eq!(first_work["source_forks"], 2);
    assert_eq!(first_work["project_stages"], 2);
    assert_eq!(
        measure(&first_work, "generated_project_custody_bytes")?,
        generated
    );
    let second = peers.forecast_project(
        &adapter,
        &source,
        &plan,
        &narrow,
        None,
        started,
        &mut budget,
    );
    assert!(matches!(second, Err(error)
        if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("shared copied and generated custody budget")));
    let refused_work = budget.work_json();
    assert_eq!(refused_work["source_forks"], 4);
    assert_eq!(refused_work["project_stages"], 3);
    assert_eq!(refused_work["candidates"], 2);
    assert_eq!(
        measure(&refused_work, "generated_project_custody_bytes")?,
        generated
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}
