#![forbid(unsafe_code)]

#[path = "../../dfmcp-intent/tests/support/labor_execution.rs"]
#[allow(dead_code)]
mod support;

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{Capability, CommitState, ErrorCode, IntentId, Result};
use dfmcp_intent::{LaborAllocationLimits, LaborAllocator};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::PredicateEvidence;
use support::{context, demands, profiles, world};

#[test]
fn guarded_staffing_runs_through_real_adapter_prepare_commit_and_poll() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(&profiles(), &demands()));
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let plan = compiled.prepare(IntentId::new(51), "staff".to_owned(), &PredicateEvidence::laboratory(adapter.snapshot())?, &context(adapter.snapshot()))?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    for _ in 0..=plan.steps.len() {
        for action in &receipt.actions {
            let ctx = context(adapter.snapshot());
            assert_ne!(adapter.poll_action(action.action_id, &ctx)?.state, CommitState::Failed);
        }
    }
    for action in &receipt.actions {
        assert!(adapter.action_receipt(action.action_id).is_some_and(|r| r.state == CommitState::Verified));
    }
    assert!(PredicateEvidence::laboratory(adapter.snapshot())?.establishes(&plan.terminal_condition)?);
    let hash = adapter.snapshot().state_hash;
    let replay = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(receipt.actions.iter().map(|a| a.action_id).collect::<Vec<_>>(), replay.actions.iter().map(|a| a.action_id).collect::<Vec<_>>());
    assert_eq!(hash, adapter.snapshot().state_hash);
    Ok(())
}

#[test]
fn a_model_and_sealed_plan_do_not_supply_mutation_authority() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(&profiles(), &demands()));
    let compiled = LaborAllocator.compile_execution(&profiles(), &demands(), LaborAllocationLimits::default())?;
    let plan = compiled.prepare(IntentId::new(52), "staff".to_owned(), &PredicateEvidence::laboratory(adapter.snapshot())?, &context(adapter.snapshot()))?;
    let before = adapter.snapshot().anchor();
    let mut denied = context(adapter.snapshot());
    denied.grants.retain(|g| g.capability != Capability::ConfigureLabor);
    assert!(matches!(adapter.prepare(&plan, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied));
    assert_eq!(before, adapter.snapshot().anchor());
    Ok(())
}
