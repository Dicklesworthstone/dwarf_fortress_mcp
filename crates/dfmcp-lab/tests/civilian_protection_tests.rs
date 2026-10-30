#[path = "../../dfmcp-intent/tests/support/civilian_protection.rs"]
#[allow(dead_code)]
mod support;

use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{Capability, CommitState, ErrorCode, GameTick, IntentId, Result, RiskTier};
use dfmcp_intent::{Constraint, Intent, ObligationSpec, PreparedPlan, StaticPlanner, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Predicate, PredicateEvidence, PredicateTruth, Value};
use support::*;

// Keep the original two protection actions and dependency, while making the
// first action's proof wait for a later laboratory observation tick.
fn deferred_plan(adapter: &MemoryAdapter, id: u128) -> Result<PreparedPlan> {
    let compiled = compiled()?;
    let goal = compiled.configuration_goal(adapter.snapshot().fortress_id);
    let mut actions = compiled.into_requested_actions();
    let mut first_proof =
        effects::default_postconditions(&actions[0].action, "", adapter.snapshot().fortress_id);
    first_proof.push(Predicate::Paused(true));
    actions[0].obligation = Some(ObligationSpec {
        terminal: Predicate::All(first_proof).normalized(),
        failure: None,
        deadline_tick: GameTick(20),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    actions[1].postconditions = vec![goal.clone()];
    let intent = Intent {
        id: IntentId::new(id),
        anchor: adapter.snapshot().anchor(),
        summary: "protect with deferred assignment".to_owned(),
        terminal_condition: goal,
        constraints: vec![
            Constraint::MaxRisk(RiskTier::Guarded),
            Constraint::KeepPaused,
        ],
        requested_actions: actions,
    };
    StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter.snapshot()),
    )
}

#[test]
fn full_protection_uses_real_prepare_commit_poll_and_idempotent_replay() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let compiled = compiled()?;
    let plan = compiled.prepare(
        IntentId::new(201),
        "protect".to_owned(),
        &PredicateEvidence::laboratory(adapter.snapshot())?,
        &context(adapter.snapshot()),
    )?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    for _ in 0..3 {
        for row in &receipt.actions {
            let ctx = context(adapter.snapshot());
            assert_ne!(
                adapter.poll_action(row.action_id, &ctx)?.state,
                CommitState::Failed
            );
        }
    }
    for row in &receipt.actions {
        assert!(
            adapter
                .action_receipt(row.action_id)
                .is_some_and(|r| r.state == CommitState::Verified)
        );
    }
    assert_eq!(
        compiled
            .evaluate(
                &PredicateEvidence::laboratory(adapter.snapshot())?,
                &context(adapter.snapshot())
            )?
            .configuration,
        PredicateTruth::True
    );
    for id in [FIRST, SECOND, OTHER] {
        assert_eq!(
            adapter
                .snapshot()
                .graph
                .entities
                .get(&id)
                .and_then(|e| e.fields.get(&field_name()))
                .and_then(|f| f.known_value()),
            Some(&Value::Bool(true))
        );
    }
    let hash = adapter.snapshot().state_hash;
    let replay = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(
        receipt
            .actions
            .iter()
            .map(|a| a.action_id)
            .collect::<Vec<_>>(),
        replay
            .actions
            .iter()
            .map(|a| a.action_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(hash, adapter.snapshot().state_hash);
    Ok(())
}

#[test]
fn current_mutation_authority_is_needed_at_initial_and_deferred_dispatch() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let plan = deferred_plan(&adapter, 202)?;
    let before = adapter.snapshot().anchor();
    let mut denied = context(adapter.snapshot());
    denied
        .grants
        .retain(|g| g.capability != Capability::ConfigureLogistics);
    assert!(
        matches!(adapter.prepare(&plan, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(before, adapter.snapshot().anchor());
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    assert!(
        matches!(adapter.commit(&plan, &prepared, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(before, adapter.snapshot().anchor());
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(
        receipt.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    assert_eq!(receipt.actions[1].state, CommitState::Prepared);
    assert_eq!(
        adapter
            .action_receipt(receipt.actions[1].action_id)
            .map(|r| r.state),
        Some(CommitState::Prepared)
    );
    // This is a reference-lab clock advance while paused, not live game time.
    adapter.advance_ticks(1)?;
    let ctx = context(adapter.snapshot());
    assert_eq!(
        adapter
            .poll_action(receipt.actions[0].action_id, &ctx)?
            .state,
        CommitState::Verified
    );
    let before = adapter.snapshot().clone();
    let prior = adapter
        .action_receipt(receipt.actions[1].action_id)
        .cloned();
    let mut denied = context(adapter.snapshot());
    denied
        .grants
        .retain(|g| g.capability != Capability::ConfigureLogistics);
    assert!(
        matches!(adapter.poll_action(receipt.actions[1].action_id, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(&before, adapter.snapshot());
    assert_eq!(
        adapter.action_receipt(receipt.actions[1].action_id),
        prior.as_ref()
    );
    let ctx = context(adapter.snapshot());
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &ctx)?
            .state,
        CommitState::Verified
    );
    assert_eq!(
        compiled()?
            .evaluate(
                &PredicateEvidence::laboratory(adapter.snapshot())?,
                &context(adapter.snapshot())
            )?
            .configuration,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn cancelling_undispatched_protection_never_dispatches_it_or_removes_existing_members() -> Result<()>
{
    let mut adapter = MemoryAdapter::new(world());
    let plan = deferred_plan(&adapter, 203)?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let child = receipt.actions[1].action_id;
    assert_eq!(
        receipt.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    assert_eq!(receipt.actions[1].state, CommitState::Prepared);
    assert_eq!(
        adapter.action_receipt(child).map(|r| r.state),
        Some(CommitState::Prepared)
    );
    let ctx = context(adapter.snapshot());
    adapter.request_cancel(child, CancelMode::CompensateReversible, &ctx)?;
    let ctx = context(adapter.snapshot());
    adapter.finalize_cancel(child, &ctx)?;
    assert_eq!(
        adapter.action_receipt(child).map(|r| r.state),
        Some(CommitState::Cancelled)
    );
    for (id, expected) in [(FIRST, true), (SECOND, false), (OTHER, true)] {
        assert_eq!(
            adapter
                .snapshot()
                .graph
                .entities
                .get(&id)
                .and_then(|e| e.fields.get(&field_name()))
                .and_then(|f| f.known_value()),
            Some(&Value::Bool(expected))
        );
    }
    assert!(adapter.snapshot().paused);
    Ok(())
}
