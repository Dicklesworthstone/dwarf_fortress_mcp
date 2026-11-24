//! Real deferred effects, recovered proof cadence and aggregate model limits.
//! The companion facade tests cover production custody admission/publication.

use super::super::super::{lab_progress::ForegroundProgress, plan_forecast::forecast_plan_bounded};
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, CommitState, ErrorCode, FortressId, GameTick, IntentId,
    RequestId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, RecoveredObligation, RequestedAction,
    StaticPlanner, effects,
};
use dfmcp_world::PredicateTruth;

fn fixture() -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(crate::lab_world::scenario_snapshot(
        "starter_fortress_provisions",
        FortressId::new(819_301),
        false,
    )?))
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(819_301),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        cancellation_requested: false,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 8_000,
            max_entities: 32_768,
            max_bytes: 16 * 1024 * 1024,
            max_actions: 64,
            max_output_tokens: 32_768,
            ..WorkBudget::default()
        },
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
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

fn requested(action: Action, depends_on: Vec<u32>) -> RequestedAction {
    RequestedAction {
        action,
        depends_on,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
    }
}

fn bed(name: &str, amount: u32) -> RequestedAction {
    requested(
        Action::CreateWorkOrder {
            name: name.to_owned(),
            job_token: "CONSTRUCT_BED".to_owned(),
            amount,
            conditions: Vec::new(),
        },
        Vec::new(),
    )
}

fn prepare(
    adapter: &MemoryAdapter,
    id: u128,
    mut actions: Vec<RequestedAction>,
    cadence: u64,
    stable: u32,
    failure: Option<Predicate>,
) -> Result<PreparedPlan> {
    let mut terminal = Vec::new();
    for (index, step) in actions.iter_mut().enumerate() {
        let action = step.action.normalized();
        let key = dfmcp_intent::derive_step_idempotency_key(
            IntentId::new(id),
            adapter.snapshot().anchor(),
            StepId::new(index as u32),
            &action,
        );
        step.postconditions =
            effects::default_postconditions(&action, &key, adapter.snapshot().fortress_id);
        if action.naturally_temporal() {
            step.obligation = Some(ObligationSpec {
                terminal: Predicate::All(step.postconditions.clone()).normalized(),
                failure: failure.clone(),
                deadline_tick: GameTick(adapter.snapshot().tick.0 + 10_000),
                poll_interval_ticks: cadence,
                stable_for_observations: stable,
            });
        }
        terminal.extend(step.postconditions.clone());
    }
    StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &Intent {
            id: IntentId::new(id),
            anchor: adapter.snapshot().anchor(),
            summary: "owned-session counterfactual fixture".to_owned(),
            terminal_condition: Predicate::All(terminal).normalized(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: actions,
        },
        &context(adapter),
    )
}

fn commit(adapter: &mut MemoryAdapter, plan: &PreparedPlan) -> Result<Vec<ActionId>> {
    let current = context(adapter);
    let prepared = adapter.prepare(plan, &current)?;
    Ok(adapter
        .commit(plan, &prepared, &current)?
        .actions
        .into_iter()
        .map(|action| action.action_id)
        .collect())
}

// Tests below already own exact fixture plans and actual receipts. Production
// entry must instead call OwnedWork::admit's complete session custody checks.
fn owned(adapter: &MemoryAdapter, actions: Vec<ActionId>, carried: Vec<CarriedStep>) -> OwnedWork {
    OwnedWork {
        session: context(adapter).session_id,
        anchor: adapter.snapshot().anchor(),
        actions,
        carried,
        live_originals: Vec::new(),
        recovered_originals: Vec::new(),
        records: 0,
        bytes: 0,
    }
}

fn predict(
    adapter: &MemoryAdapter,
    plan: &PreparedPlan,
    peers: &OwnedWork,
    current: &OperationContext,
    horizon: u64,
    reserve: Option<u64>,
) -> Result<(Value, Value)> {
    let started = Instant::now();
    let mut budget = ForecastBudget::new(current, started, horizon)?;
    let prediction = peers.forecast(adapter, plan, current, reserve, started, &mut budget)?;
    Ok((prediction, budget.work_json()))
}

fn ready_pause(resume: bool) -> Result<(MemoryAdapter, PreparedPlan, OwnedWork)> {
    let mut adapter = fixture()?;
    let mut actions = vec![
        bed("finished prerequisite", 1),
        requested(Action::Pause { paused: true }, vec![0]),
    ];
    if resume {
        actions.push(requested(Action::Pause { paused: false }, vec![1]));
    }
    let peer = prepare(&adapter, 1, actions, 10, 1, None)?;
    let live = commit(&mut adapter, &peer)?;
    adapter.advance_ticks(50)?;
    adapter.poll_action(live[0], &context(&adapter))?;
    assert_eq!(
        adapter.action_receipt(live[1]).map(|row| row.state),
        Some(CommitState::Prepared)
    );
    let candidate = prepare(&adapter, 2, vec![bed("candidate", 4)], 10, 1, None)?;
    let peers = owned(&adapter, live, Vec::new());
    Ok((adapter, candidate, peers))
}

fn actual_completion(
    source: &MemoryAdapter,
    plan: &PreparedPlan,
    peers: &OwnedWork,
    horizon: u64,
) -> Result<(u64, bool)> {
    let mut actual = source.clone();
    let candidate = commit(&mut actual, plan)?;
    let mut tracked = peers.actions.clone();
    tracked.extend(&candidate);
    let mut carried = recovered_step_progress::observe(actual.snapshot(), &peers.carried)?;
    let mut current = context(&actual);
    let mut progress = ForegroundProgress::new(&actual, horizon, &current)?;
    for _ in 0..400 {
        let ceiling = recovered_step_progress::ceiling(actual.snapshot().tick, &carried);
        let Some(slice) = progress.step(&mut actual, &tracked, &current, ceiling)? else {
            break;
        };
        carried = recovered_step_progress::observe(actual.snapshot(), &carried)?;
        let mut unfinished = Vec::new();
        for action in tracked {
            if !actual
                .action_receipt(action)
                .is_some_and(|row| row.state.is_terminal())
                || !actual.action_work_state(action)?.is_quiescent()
            {
                unfinished.push(action);
            }
        }
        tracked = unfinished;
        current.anchor = actual.snapshot().anchor();
        let complete = candidate.iter().all(|id| {
            actual
                .action_receipt(*id)
                .is_some_and(|row| row.state == CommitState::Verified)
        }) && candidate.iter().all(|id| {
            actual
                .action_work_state(*id)
                .is_ok_and(|work| work.is_quiescent())
        }) && crate::lab_world::plan_goal_truth(actual.snapshot(), plan)?
            == PredicateTruth::True;
        if complete || slice.finished {
            return Ok((actual.snapshot().tick.0, complete));
        }
    }
    Ok((actual.snapshot().tick.0, false))
}

#[test]
fn owned_deferred_pause_changes_candidate_prediction_and_matches_real_foreground() -> Result<()> {
    for resume in [false, true] {
        let (adapter, plan, peers) = ready_pause(resume)?;
        let before = format!("{adapter:?}");
        let current = context(&adapter);
        let (prediction, work) = predict(&adapter, &plan, &peers, &current, 1_000, None)?;
        assert_eq!(prediction["available"], true, "{prediction}");
        assert_eq!(prediction["predicted_complete"], resume, "{prediction}");
        assert_eq!(prediction["blocked_by_pause"], !resume);
        assert_eq!(
            prediction["shared_progression_work"]["deferred_dispatches"],
            if resume { 2 } else { 1 }
        );
        let (tick, complete) = actual_completion(&adapter, &plan, &peers, 1_000)?;
        assert_eq!(prediction["predicted_frontier_tick"], tick);
        assert_eq!(prediction["predicted_complete"], complete);
        assert_eq!(work["source_forks"], 2);
        assert_eq!(work["competing_action_steps"], peers.actions.len());
        assert_eq!(work["actual_game_ticks_advanced"], 0);
        // The published legacy mode deliberately excludes deferred peers.
        let mut legacy = ForecastBudget::new(&current, Instant::now(), 1_000)?;
        assert_eq!(
            forecast_plan_bounded(&adapter, &plan, &current, None, &mut legacy)?["predicted_complete"],
            true
        );
        assert_eq!(format!("{adapter:?}"), before);
    }
    Ok(())
}

#[test]
fn zero_horizon_never_polls_and_modeled_time_never_mints_deferred_authority() -> Result<()> {
    let (adapter, plan, peers) = ready_pause(false)?;
    let before = format!("{adapter:?}");
    let mut current = context(&adapter);
    let (zero, work) = predict(&adapter, &plan, &peers, &current, 0, None)?;
    assert_eq!(zero["available"], true, "{zero}");
    assert_eq!(zero["blocked_by_pause"], false);
    assert_eq!(work["action_polls"], 0);
    assert_eq!(work["simulation_slices"], 0);
    current
        .grants
        .retain(|grant| grant.capability != Capability::ControlClock);
    let (refused, _) = predict(&adapter, &plan, &peers, &current, 1_000, None)?;
    assert_eq!(refused["available"], false, "{refused}");
    assert_eq!(refused["reason"]["code"], "capability_denied");
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

fn recovered_peer(failure: Option<Predicate>) -> Result<(MemoryAdapter, OwnedWork)> {
    let mut live = fixture()?;
    let plan = prepare(&live, 11, vec![bed("recovered peer", 1)], 45, 100, failure)?;
    let actions = commit(&mut live, &plan)?;
    live.advance_ticks(1_000)?;
    let adapter = MemoryAdapter::recovered(live.snapshot().clone())?;
    let mut carried = Vec::new();
    for (step, action) in plan.steps.iter().zip(actions) {
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
            recovered_step_progress::proof_id(plan.digest, step.id),
            spec.clone(),
            plan.anchor.tick,
            adapter.snapshot(),
        )?;
        carried.push(CarriedStep {
            plan_digest: plan.digest,
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
    let peers = owned(&adapter, Vec::new(), carried);
    Ok((adapter, peers))
}

#[test]
fn recovered_peer_cadence_changes_stability_timing_without_restoring_dispatch_handles() -> Result<()>
{
    let (adapter, peers) = recovered_peer(None)?;
    let plan = prepare(
        &adapter,
        12,
        vec![bed("candidate stability", 1)],
        70,
        3,
        None,
    )?;
    let before = format!("{adapter:?}|{:?}", peers.carried);
    let current = context(&adapter);
    let (prediction, work) = predict(&adapter, &plan, &peers, &current, 1_000, None)?;
    assert_eq!(prediction["predicted_complete"], true, "{prediction}");
    let (omitted, _) = predict(
        &adapter,
        &plan,
        &owned(&adapter, Vec::new(), Vec::new()),
        &current,
        1_000,
        None,
    )?;
    assert!(
        prediction["predicted_completion_tick"].as_u64()
            > omitted["predicted_completion_tick"].as_u64()
    );
    let (tick, complete) = actual_completion(&adapter, &plan, &peers, 1_000)?;
    assert!(complete);
    assert_eq!(prediction["predicted_completion_tick"], tick);
    assert_eq!(prediction["recovered_commit_observation_polls"], 1);
    assert_eq!(
        prediction["shared_progression_work"]["action_polls"]
            .as_u64()
            .unwrap_or(0)
            + prediction["shared_progression_work"]["recovered_observation_polls"]
                .as_u64()
                .unwrap_or(0)
            + 1,
        work["action_polls"].as_u64().unwrap_or(0)
    );
    assert_eq!(
        prediction["competing_work"]["recovered_dispatch_handles_restored"],
        false
    );
    assert_eq!(format!("{adapter:?}|{:?}", peers.carried), before);
    Ok(())
}

#[test]
fn same_tick_candidate_effect_refreshes_recovered_failure_before_complete_shortcut() -> Result<()> {
    let (adapter, peers) = recovered_peer(Some(Predicate::Paused(true)))?;
    let plan = prepare(
        &adapter,
        13,
        vec![requested(Action::Pause { paused: true }, Vec::new())],
        10,
        1,
        None,
    )?;
    let before = format!("{adapter:?}|{:?}", peers.carried);
    let (prediction, work) = predict(&adapter, &plan, &peers, &context(&adapter), 0, None)?;
    assert_eq!(prediction["available"], false, "{prediction}");
    assert!(
        prediction["reason"]["message"]
            .as_str()
            .is_some_and(|reason| reason.contains("recovered peer"))
    );
    assert_eq!(
        work["action_polls"], 1,
        "only same-tick recovered commit observation is charged"
    );
    assert_eq!(work["simulation_slices"], 0);
    assert_eq!(work["simulated_game_ticks"], 0);
    assert_eq!(format!("{adapter:?}|{:?}", peers.carried), before);
    Ok(())
}

#[test]
fn original_reserve_endpoint_and_all_candidate_work_share_the_same_aggregate_budget() -> Result<()>
{
    let (adapter, plan, peers) = ready_pause(true)?;
    let before = format!("{adapter:?}");
    let current = context(&adapter);
    let until = adapter.snapshot().tick.0 + 701;
    let (prediction, work) = predict(&adapter, &plan, &peers, &current, 1_000, Some(until))?;
    assert_eq!(prediction["predicted_complete"], true, "{prediction}");
    assert_eq!(prediction["reserve_forecast"]["predicted_tick"], until);
    assert_eq!(
        prediction["reserve_forecast"]["predicted_goal_truth"],
        "true"
    );
    assert_eq!(work["simulated_game_ticks"], 701);
    let mut narrow = current.clone();
    narrow.budget.max_actions = (plan.steps.len() + peers.actions.len()) as u32;
    let started = Instant::now();
    let mut budget = ForecastBudget::new(&narrow, started, 1_000)?;
    peers.forecast(&adapter, &plan, &narrow, None, started, &mut budget)?;
    let prior = budget.work_json();
    assert!(
        matches!(peers.forecast(&adapter, &plan, &narrow, None, started, &mut budget),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(budget.work_json()["source_forks"], prior["source_forks"]);
    assert_eq!(
        budget.work_json()["simulated_game_ticks"],
        prior["simulated_game_ticks"]
    );
    assert_eq!(format!("{adapter:?}"), before);
    Ok(())
}

#[test]
fn version_three_requires_explicit_current_stage_and_owned_session_scopes() -> Result<()> {
    let valid = json!({"template":"compare_candidates", "version":3,
        "forecast_scope":"current_stage", "progression_scope":"owned_session", "horizon_ticks":100,
        "candidates":[{"key":"a","paused_target":true},{"key":"b","paused_target":true}]});
    let request =
        super::super::parse(&valid.to_string())?.ok_or_else(|| invalid("fixture parse missing"))?;
    assert!(super::super::uses_owned_progression(&request));
    for (key, value) in [
        ("version", json!(1)),
        ("version", json!(2)),
        ("forecast_scope", json!("complete_project")),
        ("forecast_scope", Value::Null),
        ("progression_scope", json!("candidate_only")),
        ("progression_scope", Value::Null),
        ("horizon_ticks", json!(0)),
        ("restore_recovered_actions", json!(true)),
    ] {
        let mut bad = valid.clone();
        bad[key] = value;
        assert!(super::super::parse(&bad.to_string()).is_err(), "{bad}");
    }
    for key in ["forecast_scope", "progression_scope"] {
        let mut bad = valid.clone();
        bad.as_object_mut()
            .ok_or_else(|| invalid("fixture object missing"))?
            .remove(key);
        assert!(super::super::parse(&bad.to_string()).is_err(), "{bad}");
    }
    Ok(())
}
