//! Real planner, finite effects and adapter tests for one bounded foreground wait.

use super::*;
use crate::lab_world::{parse_steps, scenario_snapshot, starter};
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, Digest32, FortressId, IntentId, RequestId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
    effects,
};
use dfmcp_world::{
    CompareOp, Fact, FactPresence, FactSource, Predicate, PredicateEvidence, PredicateTruth, Value,
};

fn new_adapter() -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(scenario_snapshot(
        "starter_fortress",
        FortressId::new(752),
        false,
    )?))
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(752),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
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

fn plan(adapter: &MemoryAdapter, actions: Vec<RequestedAction>) -> Result<PreparedPlan> {
    let mut terminal = Vec::new();
    for (index, requested) in actions.iter().enumerate() {
        if requested.postconditions.is_empty() {
            let action = requested.action.normalized();
            let key = dfmcp_intent::derive_step_idempotency_key(
                IntentId::new(752),
                adapter.snapshot().anchor(),
                dfmcp_core::StepId::new(index as u32),
                &action,
            );
            terminal.extend(effects::default_postconditions(
                &action,
                &key,
                adapter.snapshot().fortress_id,
            ));
        } else {
            terminal.extend(requested.postconditions.clone());
        }
    }
    StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &Intent {
            id: IntentId::new(752),
            anchor: adapter.snapshot().anchor(),
            summary: "complete committed work during one bounded foreground wait".to_owned(),
            terminal_condition: Predicate::All(terminal).normalized(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: actions,
        },
        &context(adapter),
    )
}

fn commit(adapter: &mut MemoryAdapter, plan: &PreparedPlan) -> Result<Vec<ActionId>> {
    let prepared = adapter.prepare(plan, &context(adapter))?;
    Ok(adapter
        .commit(plan, &prepared, &context(adapter))?
        .actions
        .into_iter()
        .map(|receipt| receipt.action_id)
        .collect())
}

fn take_slice(
    progress: &mut ForegroundProgress,
    adapter: &mut MemoryAdapter,
    actions: &[ActionId],
    ceiling: Option<u64>,
) -> Result<Option<ProgressSlice>> {
    let ctx = context(adapter);
    progress.step(adapter, actions, &ctx, ceiling)
}

fn run(
    adapter: &mut MemoryAdapter,
    actions: &[ActionId],
    ticks: u64,
) -> Result<ForegroundProgress> {
    let mut progress = ForegroundProgress::new(adapter, ticks, &context(adapter))?;
    while let Some(slice) = take_slice(&mut progress, adapter, actions, None)? {
        if slice.finished {
            break;
        }
    }
    Ok(progress)
}

const WORKSHOP_PLAN: &str = r#"[
  {"action":{"kind":"designate_dig","min":[0,3,10],"max":[4,5,10],"mode":"mine"}},
  {"action":{"kind":"build","building":"workshop:Still","location":[2,4,10],"min":[1,3,10],"max":[3,5,10]},"depends_on":[0]},
  {"action":{"kind":"create_work_order","name":"brew","job_token":"BREW_DRINK","amount":2},"depends_on":[1]},
  {"action":{"kind":"set_labor","units":["1003"],"labor":"BREW","enabled":true}}
]"#;

#[test]
fn one_coarse_wait_executes_dig_build_brew_and_agrees_with_explicit_split_waits() -> Result<()> {
    let mut coarse = new_adapter()?;
    let sealed = plan(&coarse, parse_steps(WORKSHOP_PLAN)?)?;
    let original = sealed.clone();
    let actions = commit(&mut coarse, &sealed)?;
    let mut old = coarse.clone();
    let mut split = coarse.clone();

    old.advance_ticks(2_000)?;
    for action in &actions {
        old.poll_action(*action, &context(&old))?;
    }
    assert!(actions.iter().any(|id| {
        old.action_receipt(*id)
            .is_some_and(|receipt| receipt.state != CommitState::Verified)
    }));

    let progress = run(&mut coarse, &actions, 2_000)?;
    for _ in 0..20 {
        run(&mut split, &actions, 100)?;
    }
    assert_eq!(progress.advanced_game_ticks(), 2_000);
    assert_eq!(progress.remaining_game_ticks(), 0);
    assert_eq!(
        progress.dispatched_actions(),
        2,
        "only already committed deferred successors dispatch"
    );
    for action in &actions {
        let completed = coarse
            .action_receipt(*action)
            .ok_or_else(|| refusal(ErrorCode::InvalidRequest, "missing completed receipt"))?;
        let split_receipt = split
            .action_receipt(*action)
            .ok_or_else(|| refusal(ErrorCode::InvalidRequest, "missing split receipt"))?;
        assert_eq!(completed.state, CommitState::Verified);
        assert_eq!(split_receipt.state, CommitState::Verified);
        assert_eq!(
            completed.observed_anchor.tick,
            split_receipt.observed_anchor.tick
        );
        assert!(coarse.action_work_state(*action)?.is_quiescent());
        assert!(split.action_work_state(*action)?.is_quiescent());
    }
    assert_eq!(coarse.snapshot().tick, split.snapshot().tick);
    for step in &sealed.steps {
        if matches!(
            step.action,
            Action::Build { .. } | Action::CreateWorkOrder { .. }
        ) {
            let entity = effects::created_entity_id(&step.idempotency_key, 0);
            assert_eq!(
                coarse.snapshot().graph.entities.get(&entity),
                split.snapshot().graph.entities.get(&entity)
            );
        }
    }
    assert_eq!(
        sealed, original,
        "original identity, goals and deadlines cannot be rewritten"
    );
    Ok(())
}

fn stock_goal(minimum: u64) -> Predicate {
    Predicate::FieldCompare {
        entity_id: starter::STOCK_LEDGER,
        field: effects::STOCK_DRINK_FIELD.to_owned(),
        op: CompareOp::Ge,
        value: Value::U64(minimum),
    }
}

fn temporal_order(
    terminal: Predicate,
    deadline: u64,
    cadence: u64,
    stability: u32,
) -> RequestedAction {
    RequestedAction {
        action: Action::CreateWorkOrder {
            name: "original temporal order".to_owned(),
            job_token: "BREW_DRINK".to_owned(),
            amount: 1,
            conditions: Vec::new(),
        },
        preconditions: vec![Predicate::True],
        postconditions: vec![terminal.clone()],
        compensation: None,
        obligation: Some(ObligationSpec {
            terminal,
            failure: None,
            deadline_tick: GameTick(deadline),
            poll_interval_ticks: cadence,
            stable_for_observations: stability,
        }),
        depends_on: Vec::new(),
    }
}

#[test]
fn deadline_clipping_and_original_cadence_create_only_distinct_real_samples() -> Result<()> {
    let mut adapter = new_adapter()?;
    let sealed = plan(&adapter, vec![temporal_order(stock_goal(45), 81, 30, 2)])?;
    let actions = commit(&mut adapter, &sealed)?;
    let mut progress = ForegroundProgress::new(&adapter, 101, &context(&adapter))?;
    let mut observed = Vec::new();
    while let Some(slice) = take_slice(&mut progress, &mut adapter, &actions, None)? {
        observed.push((adapter.snapshot().tick.0, slice.receipts[0].state));
        if slice.finished {
            break;
        }
    }
    assert_eq!(
        observed.iter().map(|row| row.0).collect::<Vec<_>>(),
        vec![1, 31, 61, 81, 102]
    );
    assert_eq!(observed[0].1, CommitState::AppliedAwaitingVerification);
    assert_eq!(observed[2].1, CommitState::AppliedAwaitingVerification);
    assert_eq!(
        observed[3].1,
        CommitState::Verified,
        "the deadline is a final real sample"
    );
    let original = adapter.action_receipt(actions[0]).cloned();
    run(&mut adapter, &actions, 0)?;
    run(&mut adapter, &actions, 200)?;
    assert_eq!(
        adapter.action_receipt(actions[0]).cloned(),
        original,
        "later polls retain first proof"
    );
    Ok(())
}

#[test]
fn pause_dispatched_by_a_successor_halts_the_same_wait_without_forced_ticks() -> Result<()> {
    let mut adapter = new_adapter()?;
    let paused = RequestedAction {
        action: Action::Pause { paused: true },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on: vec![0],
    };
    let sealed = plan(
        &adapter,
        vec![temporal_order(stock_goal(45), 201, 10, 1), paused.clone()],
    )?;
    let actions = commit(&mut adapter, &sealed)?;
    let progress = run(&mut adapter, &actions, 2_000)?;
    assert!(adapter.snapshot().paused);
    assert_eq!(progress.advanced_game_ticks(), 50);
    assert_eq!(progress.remaining_game_ticks(), 1_950);
    for action in &actions {
        assert_eq!(
            adapter.action_receipt(*action).map(|receipt| receipt.state),
            Some(CommitState::Verified)
        );
    }
    let before = adapter.snapshot().clone();
    let mut zero = ForegroundProgress::new(&adapter, 0, &context(&adapter))?;
    let slice = take_slice(&mut zero, &mut adapter, &actions, None)?
        .ok_or_else(|| refusal(ErrorCode::InvalidRequest, "missing zero-time slice"))?;
    assert_eq!(zero.advanced_game_ticks(), 0);
    assert!(adapter.snapshot().paused && slice.finished);
    assert_eq!(adapter.snapshot(), &before);
    // A positive paused wait is also observational; it never forces the clock.
    assert_eq!(run(&mut adapter, &actions, 2_000)?.advanced_game_ticks(), 0);
    Ok(())
}

#[test]
fn expired_deferred_authority_rolls_back_only_the_current_slice_and_preserves_work() -> Result<()> {
    let mut adapter = new_adapter()?;
    let sealed = plan(&adapter, parse_steps(WORKSHOP_PLAN)?)?;
    let actions = commit(&mut adapter, &sealed)?;
    let mut progress = ForegroundProgress::new(&adapter, 1_000, &context(&adapter))?;
    let mut failed = false;
    loop {
        let mut ctx = context(&adapter);
        for grant in &mut ctx.grants {
            if grant.capability == Capability::Construct {
                grant.expires_at_tick = Some(GameTick(100));
            }
        }
        let before = adapter.clone();
        let ticks = progress.advanced_game_ticks();
        match progress.step(&mut adapter, &actions, &ctx, None) {
            Err(error) => {
                assert_eq!(error.code, ErrorCode::CapabilityDenied);
                assert_eq!(adapter.snapshot(), before.snapshot());
                assert_eq!(adapter.transcript(), before.transcript());
                assert_eq!(progress.advanced_game_ticks(), ticks);
                assert!(ticks > 0 && ticks < 1_000);
                assert_eq!(
                    adapter
                        .action_receipt(actions[1])
                        .map(|receipt| receipt.state),
                    Some(CommitState::Prepared)
                );
                failed = true;
                break;
            }
            Ok(Some(slice)) if !slice.finished => {}
            _ => break,
        }
    }
    assert!(
        failed,
        "expired construction cannot dispatch during an apparently read-only poll"
    );
    let before = adapter.snapshot().clone();
    let mut cancelled = context(&adapter);
    cancelled.cancellation_requested = true;
    assert_eq!(
        progress
            .step(&mut adapter, &actions, &cancelled, None)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::CancellationRequested)
    );
    assert_eq!(adapter.snapshot(), &before);
    run(&mut adapter, &actions, 1_000)?;
    assert!(actions.iter().all(|id| {
        adapter
            .action_receipt(*id)
            .is_some_and(|receipt| receipt.state == CommitState::Verified)
    }));
    Ok(())
}

#[test]
fn unknown_postconditions_never_release_dependent_work() -> Result<()> {
    let mut adapter = new_adapter()?;
    let mut snapshot = adapter.snapshot().clone();
    let mut unknown = Fact::known(
        Value::Bool(true),
        snapshot.tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    );
    unknown.presence = Some(FactPresence::Unknown("not observed".to_owned()));
    snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| refusal(ErrorCode::InvalidRequest, "missing fixture ledger"))?
        .fields
        .insert("progress_unknown".to_owned(), unknown);
    snapshot.refresh_hash();
    adapter = MemoryAdapter::new(snapshot);
    let unknown = Predicate::FieldCompare {
        entity_id: starter::STOCK_LEDGER,
        field: "progress_unknown".to_owned(),
        op: CompareOp::Eq,
        value: Value::Bool(true),
    };
    let paused = RequestedAction {
        action: Action::Pause { paused: true },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on: vec![0],
    };
    let sealed = plan(
        &adapter,
        vec![temporal_order(unknown.clone(), 81, 10, 1), paused],
    )?;
    let actions = commit(&mut adapter, &sealed)?;
    run(&mut adapter, &actions, 100)?;
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&unknown)?,
        PredicateTruth::Unknown
    );
    assert!(
        !adapter.snapshot().paused,
        "unknown proof cannot unconditionally dispatch the successor"
    );
    assert_eq!(
        adapter
            .action_receipt(actions[0])
            .map(|receipt| receipt.state),
        Some(CommitState::Failed)
    );
    assert_eq!(
        adapter
            .action_receipt(actions[1])
            .map(|receipt| receipt.state),
        Some(CommitState::Failed)
    );
    assert_eq!(
        adapter.action_work_state(actions[1])?,
        dfmcp_intent::EffectWorkState::NeverDispatched
    );
    Ok(())
}

#[test]
fn per_request_dispatch_budget_is_not_renewed_by_each_poll_or_slice() -> Result<()> {
    let mut adapter = new_adapter()?;
    let sealed = plan(&adapter, parse_steps(WORKSHOP_PLAN)?)?;
    let actions = commit(&mut adapter, &sealed)?;
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = 1;
    let mut progress = ForegroundProgress::new(&adapter, 1_000, &ctx)?;
    loop {
        ctx.anchor = adapter.snapshot().anchor();
        let before = adapter.snapshot().clone();
        match progress.step(&mut adapter, &actions, &ctx, None) {
            Ok(Some(slice)) if !slice.finished => {}
            Err(error) => {
                assert_eq!(error.code, ErrorCode::BudgetExceeded);
                assert_eq!(adapter.snapshot(), &before);
                assert_eq!(progress.dispatched_actions(), 1);
                assert_eq!(
                    adapter
                        .action_receipt(actions[2])
                        .map(|receipt| receipt.state),
                    Some(CommitState::Prepared)
                );
                break;
            }
            _ => {
                return Err(refusal(
                    ErrorCode::InvalidRequest,
                    "the aggregate action budget was not enforced",
                ));
            }
        }
    }
    Ok(())
}

#[test]
fn unknown_id_reordered_dependencies_source_fault_and_context_changes_do_not_advance() -> Result<()>
{
    let mut adapter = new_adapter()?;
    let sealed = plan(&adapter, parse_steps(WORKSHOP_PLAN)?)?;
    let actions = commit(&mut adapter, &sealed)?;
    let mut progress = ForegroundProgress::new(&adapter, 100, &context(&adapter))?;
    let original = adapter.snapshot().clone();
    let mut bad = actions.clone();
    bad[0] = ActionId::new(u128::MAX);
    assert!(take_slice(&mut progress, &mut adapter, &bad, None).is_err());
    bad = actions.iter().rev().copied().collect();
    assert!(take_slice(&mut progress, &mut adapter, &bad, None).is_err());
    bad = actions.clone();
    bad.push(actions[0]);
    assert!(take_slice(&mut progress, &mut adapter, &bad, None).is_err());
    let mut changed = context(&adapter);
    changed.session_id = SessionId::new(999);
    assert!(
        progress
            .step(&mut adapter, &actions, &changed, None)
            .is_err()
    );
    assert!(take_slice(&mut progress, &mut adapter, &actions, Some(0)).is_err());
    assert_eq!(adapter.snapshot(), &original);
    assert_eq!(progress.slices(), 0);
    take_slice(&mut progress, &mut adapter, &actions, None)?;
    let mut fork = adapter.snapshot().clone();
    fork.cursor = fork
        .cursor
        .checked_reset_epoch()
        .ok_or_else(|| refusal(ErrorCode::CursorGap, "fixture epoch exhausted"))?;
    fork.refresh_hash();
    adapter.inject_snapshot(fork)?;
    let before = adapter.snapshot().clone();
    assert_eq!(
        take_slice(&mut progress, &mut adapter, &actions, None)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::StaleAnchor)
    );
    assert_eq!(adapter.snapshot(), &before);

    // A future producer fact remains ineligible even when the requested wait
    // would move its timestamp into the apparent present.
    let mut poisoned = new_adapter()?;
    let mut future = poisoned.snapshot().clone();
    let fact = future
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(effects::STOCK_DRINK_FIELD))
        .ok_or_else(|| refusal(ErrorCode::InvalidRequest, "missing fixture count"))?;
    fact.observed_at = GameTick(2);
    future.refresh_hash();
    poisoned = MemoryAdapter::new(future);
    let mut progress = ForegroundProgress::new(&poisoned, 100, &context(&poisoned))?;
    take_slice(&mut progress, &mut poisoned, &[], None)?;
    let before = poisoned.snapshot().clone();
    assert!(take_slice(&mut progress, &mut poisoned, &[], None).is_err());
    assert_eq!(poisoned.snapshot(), &before);
    Ok(())
}

#[test]
fn hard_work_bounds_and_owner_ceiling_return_an_exact_resumable_prefix() -> Result<()> {
    let mut adapter = new_adapter()?;
    let mut progress = ForegroundProgress::new(&adapter, 100, &context(&adapter))?;
    take_slice(&mut progress, &mut adapter, &[], Some(7))?;
    let slice = take_slice(&mut progress, &mut adapter, &[], Some(7))?
        .ok_or_else(|| refusal(ErrorCode::InvalidRequest, "missing owner-clipped slice"))?;
    assert!(!slice.finished);
    assert_eq!(progress.advanced_game_ticks(), 7);
    assert_eq!(progress.remaining_game_ticks(), 93);
    assert_eq!(progress.polls(), 0);
    let before = adapter.snapshot().clone();
    progress.slices = MAX_PROGRESS_SLICES;
    assert_eq!(
        take_slice(&mut progress, &mut adapter, &[], None)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(progress.remaining_game_ticks(), 93);
    assert_eq!(run(&mut adapter, &[], 93)?.advanced_game_ticks(), 93);
    assert_eq!(adapter.snapshot().tick.0, 101);

    let mut denied = context(&adapter);
    for grant in &mut denied.grants {
        if grant.capability == Capability::ControlClock {
            grant.expires_at_tick = Some(GameTick(102));
        }
    }
    assert_eq!(
        ForegroundProgress::new(&adapter, 2, &denied)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::CapabilityDenied)
    );
    Ok(())
}
