//! Actual adapter boundaries with independently observed changes between the
//! two release effects. Test-only snapshot access preserves pending actions;
//! public scenario injection intentionally abandons all active work instead.

#[path = "../../dfmcp-intent/tests/support/civilian_protection.rs"]
#[allow(dead_code)]
mod support;

use super::MemoryAdapter;
use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{
    Capability, CommitState, EntityId, ErrorCode, GameTick, IntentId, Result, RiskTier,
};
use dfmcp_intent::{
    CivilianReleasePlan, Constraint, Intent, ObligationSpec, PreparedPlan, StaticPlanner, effects,
};
use dfmcp_world::{
    EntityKind, FactSource, LaboratoryEntityIdentity, PredicateEvidence, PredicateTruth, Value,
};
use support::*;

fn exact(id: EntityId) -> LaboratoryEntityIdentity {
    LaboratoryEntityIdentity { id, generation: 1 }
}
fn release() -> Result<CivilianReleasePlan> {
    CivilianReleasePlan::compile(exact(BURROW), &[exact(FIRST), exact(SECOND), exact(OTHER)])
}

fn deferred(adapter: &MemoryAdapter, id: u128) -> Result<PreparedPlan> {
    let release = release()?;
    let mut actions = release.requested_actions_for(adapter.snapshot().fortress_id);
    actions[0].obligation = Some(ObligationSpec {
        terminal: actions[0].postconditions[0].clone(),
        failure: None,
        deadline_tick: GameTick(20),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    let intent = Intent {
        id: IntentId::new(id),
        anchor: adapter.snapshot().anchor(),
        summary: "reviewed deferred civilian release".into(),
        terminal_condition: release.configuration_goal(adapter.snapshot().fortress_id),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: actions,
    };
    StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter.snapshot()),
    )
}

fn active(adapter: &MemoryAdapter) -> bool {
    adapter
        .snapshot()
        .graph
        .entities
        .get(&effects::fortress_settings_entity_id(
            adapter.snapshot().fortress_id,
        ))
        .and_then(|record| {
            record
                .fields
                .get("standing_order.CIVILIAN_BURROW_RESTRICTION")
        })
        .and_then(|fact| fact.known_value())
        == Some(&Value::Text("ACTIVE".into()))
}

fn memberships(adapter: &MemoryAdapter, assigned: bool) -> bool {
    [FIRST, SECOND, OTHER].iter().all(|id| {
        adapter
            .snapshot()
            .graph
            .entities
            .get(id)
            .and_then(|record| record.fields.get(&field_name()))
            .and_then(|fact| fact.known_value())
            == Some(&Value::Bool(assigned))
    })
}

#[test]
fn reviewed_running_release_completes_real_actions_and_replay_has_no_effect() -> Result<()> {
    let mut snapshot = complete(world())?;
    snapshot.paused = false;
    snapshot.refresh_hash();
    let mut adapter = MemoryAdapter::new(snapshot);
    let release = release()?;
    let plan = release.prepare(
        IntentId::new(401),
        "release after owned clearance".into(),
        &PredicateEvidence::laboratory(adapter.snapshot())?,
        &context(adapter.snapshot()),
    )?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    for row in &receipt.actions {
        assert_eq!(
            adapter
                .poll_action(row.action_id, &context(adapter.snapshot()))?
                .state,
            CommitState::Verified
        );
    }
    assert!(memberships(&adapter, false));
    assert!(!active(&adapter));
    assert!(!adapter.snapshot().paused);
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::True
    );
    let before = adapter.snapshot().clone();
    let replay = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(
        receipt
            .actions
            .iter()
            .map(|row| row.action_id)
            .collect::<Vec<_>>(),
        replay
            .actions
            .iter()
            .map(|row| row.action_id)
            .collect::<Vec<_>>()
    );
    assert_eq!(&before, adapter.snapshot());
    Ok(())
}

#[test]
fn returned_threat_unknown_inputs_new_civilian_and_aba_refuse_deferred_release() -> Result<()> {
    for change in 0..6 {
        let mut adapter = MemoryAdapter::new(complete(world())?);
        let plan = deferred(&adapter, 402 + change)?;
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
        let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
        assert_eq!(
            receipt.actions[0].state,
            CommitState::AppliedAwaitingVerification
        );
        assert_eq!(receipt.actions[1].state, CommitState::Prepared);
        adapter.advance_ticks(1)?;
        let prior =
            adapter.poll_action(receipt.actions[0].action_id, &context(adapter.snapshot()))?;
        assert_eq!(prior.state, CommitState::Verified);
        match change {
            0 => {
                let mut hostile = record(EntityId::new(30), EntityKind::Creature);
                hostile
                    .fields
                    .insert("hostile".into(), fact(Value::Bool(true)));
                hostile.fields.insert(
                    "threat_status".into(),
                    fact(Value::Text("approaching".into())),
                );
                adapter.snapshot.graph.entities.insert(hostile.id, hostile);
            }
            1 => {
                let mut alive = fact(Value::Bool(true));
                alive.source = FactSource::Replay;
                set(&mut adapter.snapshot, FIRST, "alive", alive)?;
            }
            2 => {
                let mut newcomer = record(EntityId::new(14), EntityKind::Unit);
                newcomer
                    .fields
                    .insert("alive".into(), fact(Value::Bool(true)));
                adapter
                    .snapshot
                    .graph
                    .entities
                    .insert(newcomer.id, newcomer);
            }
            3 | 4 => {
                let id = if change == 3 { FIRST } else { BURROW };
                adapter
                    .snapshot
                    .graph
                    .entities
                    .get_mut(&id)
                    .ok_or_else(|| {
                        dfmcp_core::DfmcpError::new(
                            ErrorCode::InvalidRequest,
                            "fixture entity missing",
                        )
                    })?
                    .generation += 1;
            }
            _ => {
                // A present but unobserved military role cannot silently remove
                // a member from the complete civilian domain.
                let mut role = fact(Value::Null);
                role.source = FactSource::AgentAssertion("civilian".into());
                set(&mut adapter.snapshot, FIRST, "squad", role)?;
            }
        }
        adapter.snapshot.cursor = adapter.snapshot.cursor.checked_next().ok_or_else(|| {
            dfmcp_core::DfmcpError::new(ErrorCode::BudgetExceeded, "fixture cursor exhausted")
        })?;
        adapter.snapshot.refresh_hash();
        let before = adapter.snapshot().clone();
        let failed =
            adapter.poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?;
        assert_eq!(failed.state, CommitState::Failed, "change {change}");
        assert_eq!(&before, adapter.snapshot());
        assert!(active(&adapter));
        assert!(
            !adapter
                .actions
                .get(&failed.action_id)
                .is_some_and(|action| action.dispatched)
        );
        assert_eq!(adapter.action_receipt(prior.action_id), Some(&prior));
        assert_ne!(
            PredicateEvidence::laboratory(adapter.snapshot())?
                .evaluate(&plan.terminal_condition)?,
            PredicateTruth::True
        );
    }
    Ok(())
}

#[test]
fn deferred_release_still_needs_current_effect_authority() -> Result<()> {
    let mut adapter = MemoryAdapter::new(complete(world())?);
    let plan = deferred(&adapter, 410)?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    adapter.advance_ticks(1)?;
    assert_eq!(
        adapter
            .poll_action(receipt.actions[0].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    let before = adapter.snapshot().clone();
    let mut denied = context(adapter.snapshot());
    denied
        .grants
        .retain(|grant| grant.capability != Capability::ConfigureLogistics);
    assert!(
        matches!(adapter.poll_action(receipt.actions[1].action_id, &denied), Err(error) if error.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(&before, adapter.snapshot());
    assert!(active(&adapter));
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    assert!(!active(&adapter));
    Ok(())
}

#[test]
fn cancellation_restores_memberships_even_when_current_threat_no_longer_permits_release()
-> Result<()> {
    let mut adapter = MemoryAdapter::new(complete(world())?);
    let plan = deferred(&adapter, 411)?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert!(memberships(&adapter, false));
    let mut hostile = record(EntityId::new(30), EntityKind::Creature);
    hostile
        .fields
        .insert("hostile".into(), fact(Value::Bool(true)));
    hostile.fields.insert(
        "threat_status".into(),
        fact(Value::Text("attacking".into())),
    );
    adapter.snapshot.graph.entities.insert(hostile.id, hostile);
    adapter.snapshot.cursor = adapter.snapshot.cursor.checked_next().ok_or_else(|| {
        dfmcp_core::DfmcpError::new(ErrorCode::BudgetExceeded, "fixture cursor exhausted")
    })?;
    adapter.snapshot.refresh_hash();
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?
            .evaluate(release()?.safety_predicate())?,
        PredicateTruth::False
    );
    adapter.request_cancel(
        receipt.actions[0].action_id,
        CancelMode::CompensateReversible,
        &context(adapter.snapshot()),
    )?;
    let restored =
        adapter.finalize_cancel(receipt.actions[0].action_id, &context(adapter.snapshot()))?;
    assert_eq!(restored.state, CommitState::Compensated);
    assert!(memberships(&adapter, true));
    assert!(active(&adapter));
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Failed
    );
    assert!(memberships(&adapter, true));
    assert!(active(&adapter));
    Ok(())
}

#[test]
fn cancellation_never_reassigns_a_reused_civilian_or_burrow_identity() -> Result<()> {
    for id in [FIRST, BURROW] {
        let mut adapter = MemoryAdapter::new(complete(world())?);
        let plan = deferred(&adapter, 412 + u128::from(id.get()))?;
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
        let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
        adapter
            .snapshot
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| {
                dfmcp_core::DfmcpError::new(ErrorCode::InvalidRequest, "fixture entity missing")
            })?
            .generation += 1;
        adapter.snapshot.cursor = adapter.snapshot.cursor.checked_next().ok_or_else(|| {
            dfmcp_core::DfmcpError::new(ErrorCode::BudgetExceeded, "fixture cursor exhausted")
        })?;
        adapter.snapshot.refresh_hash();
        adapter.request_cancel(
            receipt.actions[0].action_id,
            CancelMode::CompensateReversible,
            &context(adapter.snapshot()),
        )?;
        let before = adapter.snapshot().clone();
        assert!(
            matches!(adapter.finalize_cancel(receipt.actions[0].action_id, &context(adapter.snapshot())),
            Err(error) if error.code == ErrorCode::CancellationIncomplete)
        );
        assert_eq!(&before, adapter.snapshot());
        assert!(memberships(&adapter, false));
        assert!(active(&adapter));
        adapter.request_cancel(
            receipt.actions[0].action_id,
            CancelMode::StopFutureSteps,
            &context(adapter.snapshot()),
        )?;
        assert_eq!(
            adapter
                .finalize_cancel(receipt.actions[0].action_id, &context(adapter.snapshot()))?
                .state,
            CommitState::Cancelled
        );
        assert!(memberships(&adapter, false));
    }
    Ok(())
}
