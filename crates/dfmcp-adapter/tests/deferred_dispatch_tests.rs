#![forbid(unsafe_code)]

//! The dispatcher obeys the same deferred-step proof gates as the laboratory.
//! Beads: df-fastmcp-conformance-5pj.3, df-cx-authority-budget-threading-lmu.

use dfmcp_adapter::dispatcher::MutationDispatcher;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, GameTick, IntentId,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

fn world() -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(14),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    )
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(4),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: [
            Capability::Plan,
            Capability::Observe,
            Capability::ControlClock,
        ]
        .into_iter()
        .map(|capability| CapabilityGrant {
            capability,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Reversible,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn obligation(paused: bool, deadline: u64, stable: u32) -> ObligationSpec {
    ObligationSpec {
        terminal: Predicate::Paused(paused),
        failure: None,
        deadline_tick: GameTick(deadline),
        poll_interval_ticks: 1,
        stable_for_observations: stable,
    }
}

fn plan(
    snapshot: &WorldSnapshot,
    child_precondition: bool,
    child_deadline: Option<u64>,
) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare(
        snapshot,
        &Intent {
            id: IntentId::new(1),
            anchor: snapshot.anchor(),
            summary: "unpause, prove stability, then pause".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![
                RequestedAction {
                    action: Action::Pause { paused: false },
                    preconditions: Vec::new(),
                    postconditions: Vec::new(),
                    compensation: None,
                    obligation: Some(obligation(false, 110, 2)),
                    depends_on: Vec::new(),
                },
                RequestedAction {
                    action: Action::Pause { paused: true },
                    preconditions: if child_precondition {
                        vec![Predicate::Paused(true)]
                    } else {
                        Vec::new()
                    },
                    postconditions: Vec::new(),
                    compensation: None,
                    obligation: child_deadline.map(|deadline| obligation(true, deadline, 1)),
                    depends_on: vec![0],
                },
            ],
        },
        &context(snapshot),
    )
}

fn at_tick(snapshot: &mut WorldSnapshot, tick: u64) {
    snapshot.tick = GameTick(tick);
    snapshot.cursor.sequence += 1;
    snapshot.refresh_hash();
}

#[test]
fn failed_parent_closes_deferred_child_without_effect_and_replay_stays_exact() -> Result<()> {
    let mut snapshot = world();
    let plan = plan(&snapshot, false, None)?;
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context(&snapshot))?;
    let ctx = context(&snapshot);
    let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &ctx)?;
    assert_eq!(committed.actions[1].state, CommitState::Prepared);
    at_tick(&mut snapshot, 111);
    let before = snapshot.clone();
    let ctx = context(&snapshot);
    let result = dispatcher.reconcile(&plan, &mut snapshot, &ctx)?;
    assert_eq!(
        result.actions.iter().map(|a| a.state).collect::<Vec<_>>(),
        vec![CommitState::Failed; 2]
    );
    assert_eq!(snapshot, before);
    assert!(result.actions[1].message.contains("not dispatched"));
    assert!(result.actions.iter().all(|a| !a.evidence.is_empty()));
    at_tick(&mut snapshot, 120);
    let ctx = context(&snapshot);
    let later = dispatcher.reconcile(&plan, &mut snapshot, &ctx)?;
    assert_eq!(later.actions, result.actions);
    assert_eq!(
        dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &ctx)?,
        later
    );
    Ok(())
}

#[test]
fn expired_waiting_child_fails_before_a_new_effect_even_when_parent_just_verified() -> Result<()> {
    let mut snapshot = world();
    let plan = plan(&snapshot, false, Some(101))?;
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context(&snapshot))?;
    let ctx = context(&snapshot);
    dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &ctx)?;
    at_tick(&mut snapshot, 101);
    let before = snapshot.clone();
    let ctx = context(&snapshot);
    let result = dispatcher.reconcile(&plan, &mut snapshot, &ctx)?;
    assert_eq!(result.actions[0].state, CommitState::Verified);
    assert_eq!(result.actions[1].state, CommitState::Failed);
    assert!(result.actions[1].message.contains("deadline"));
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn changed_child_precondition_records_failure_without_rolling_back_parent_proof() -> Result<()> {
    let mut snapshot = world();
    let plan = plan(&snapshot, true, None)?;
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context(&snapshot))?;
    let ctx = context(&snapshot);
    dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &ctx)?;
    at_tick(&mut snapshot, 101);
    let before = snapshot.clone();
    let ctx = context(&snapshot);
    let result = dispatcher.reconcile(&plan, &mut snapshot, &ctx)?;
    assert_eq!(result.actions[0].state, CommitState::Verified);
    assert_eq!(result.actions[1].state, CommitState::Failed);
    assert!(result.actions[1].message.contains("preconditions"));
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn exact_deadline_can_prove_a_dispatched_obligation() -> Result<()> {
    let mut snapshot = world();
    let plan = plan(&snapshot, false, None)?;
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context(&snapshot))?;
    let ctx = context(&snapshot);
    dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &ctx)?;
    at_tick(&mut snapshot, 110);
    let ctx = context(&snapshot);
    let result = dispatcher.reconcile(&plan, &mut snapshot, &ctx)?;
    assert!(
        result
            .actions
            .iter()
            .all(|a| a.state == CommitState::Verified)
    );
    assert!(snapshot.paused);
    Ok(())
}
