#[path = "../../dfmcp-intent/tests/support/civilian_protection.rs"]
#[allow(dead_code)]
mod support;

use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{Capability, CommitState, ErrorCode, IntentId, Result};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{PredicateEvidence, PredicateTruth, Value};
use support::*;

#[test]
fn full_protection_uses_real_prepare_commit_poll_and_idempotent_replay() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let compiled = compiled()?;
    let plan = compiled.prepare(IntentId::new(201), "protect".to_owned(), &PredicateEvidence::laboratory(adapter.snapshot())?, &context(adapter.snapshot()))?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    for _ in 0..3 {
        for row in &receipt.actions {
            let ctx = context(adapter.snapshot());
            assert_ne!(adapter.poll_action(row.action_id, &ctx)?.state, CommitState::Failed);
        }
    }
    for row in &receipt.actions {
        assert!(adapter.action_receipt(row.action_id).is_some_and(|r| r.state == CommitState::Verified));
    }
    assert_eq!(compiled.evaluate(&PredicateEvidence::laboratory(adapter.snapshot())?, &context(adapter.snapshot()))?.configuration, PredicateTruth::True);
    for id in [FIRST, SECOND, OTHER] {
        assert_eq!(adapter.snapshot().graph.entities.get(&id).and_then(|e| e.fields.get(&field_name())).and_then(|f| f.known_value()), Some(&Value::Bool(true)));
    }
    let hash = adapter.snapshot().state_hash;
    let replay = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(receipt.actions.iter().map(|a| a.action_id).collect::<Vec<_>>(), replay.actions.iter().map(|a| a.action_id).collect::<Vec<_>>());
    assert_eq!(hash, adapter.snapshot().state_hash);
    Ok(())
}

#[test]
fn current_mutation_authority_is_needed_at_initial_and_deferred_dispatch() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let plan = compiled()?.prepare(IntentId::new(202), "protect".to_owned(), &PredicateEvidence::laboratory(adapter.snapshot())?, &context(adapter.snapshot()))?;
    let before = adapter.snapshot().anchor();
    let mut denied = context(adapter.snapshot());
    denied.grants.retain(|g| g.capability != Capability::ConfigureLogistics);
    assert!(matches!(adapter.prepare(&plan, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied));
    assert_eq!(before, adapter.snapshot().anchor());
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let ctx = context(adapter.snapshot());
    adapter.poll_action(receipt.actions[0].action_id, &ctx)?;
    let before = adapter.snapshot().anchor();
    let mut denied = context(adapter.snapshot());
    denied.grants.retain(|g| g.capability != Capability::ConfigureLogistics);
    assert!(matches!(adapter.poll_action(receipt.actions[1].action_id, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied));
    assert_eq!(before, adapter.snapshot().anchor());
    Ok(())
}

#[test]
fn cancelling_undispatched_protection_never_dispatches_it_or_removes_existing_members() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world());
    let plan = compiled()?.prepare(IntentId::new(203), "protect".to_owned(), &PredicateEvidence::laboratory(adapter.snapshot())?, &context(adapter.snapshot()))?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let child = receipt.actions[1].action_id;
    let ctx = context(adapter.snapshot());
    adapter.request_cancel(child, CancelMode::CompensateReversible, &ctx)?;
    let ctx = context(adapter.snapshot());
    adapter.finalize_cancel(child, &ctx)?;
    for (id, expected) in [(FIRST, true), (SECOND, false), (OTHER, true)] {
        assert_eq!(adapter.snapshot().graph.entities.get(&id).and_then(|e| e.fields.get(&field_name())).and_then(|f| f.known_value()), Some(&Value::Bool(expected)));
    }
    assert!(adapter.snapshot().paused);
    Ok(())
}
