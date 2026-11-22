//! A generic completed order cannot inherit another action's terminal proof.

use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, FortressId, IntentId, ObservationCursor,
    RequestId, RiskTier, SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner,
    derive_step_idempotency_key,
};
use dfmcp_world::{EntityKind, Fact, FactSource, Value, WorldGraph};

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(805),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        cancellation_requested: false,
        grants: [
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope::default(),
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

#[test]
fn generic_order_identity_refusal_preserves_dispatcher_journal_and_proof_monitor() -> Result<()> {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(805),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    let action = Action::CreateWorkOrder {
        name: "original quantity".to_owned(),
        job_token: "MAKE_TEST".to_owned(),
        amount: 2,
        conditions: Vec::new(),
    };
    let intent_id = IntentId::new(805);
    let effect_key =
        derive_step_idempotency_key(intent_id, snapshot.anchor(), StepId::new(0), &action);
    let terminal = Predicate::All(effects::default_postconditions(
        &action,
        &effect_key,
        snapshot.fortress_id,
    ))
    .normalized();
    let intent = Intent {
        id: intent_id,
        anchor: snapshot.anchor(),
        summary: "prove the original generic order".to_owned(),
        terminal_condition: terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            obligation: Some(ObligationSpec {
                terminal,
                failure: None,
                deadline_tick: GameTick(5_000),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            }),
            compensation: None,
            depends_on: Vec::new(),
        }],
    };
    let current = context(&snapshot);
    let plan = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &current)?;
    let mut pending = MutationDispatcher::new();
    let prepared = pending.prepare_mutation(&plan, &snapshot, &current)?;
    let committed = pending.commit_mutation(&plan, &prepared, &mut snapshot, &current)?;
    assert_eq!(
        committed.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    let elapsed = effects::WORK_ORDER_TICKS_PER_UNIT * 2;
    snapshot.tick = GameTick(snapshot.tick.0 + elapsed);
    effects::advance_effects(&mut snapshot, elapsed)?;
    snapshot.cursor = snapshot.cursor.checked_next().ok_or_else(|| {
        DfmcpError::new(ErrorCode::CursorGap, "test observation cursor exhausted")
    })?;
    snapshot.refresh_hash();
    let entity_id = effects::created_entity_id(&effect_key, 0);
    let journal_key = format!("dfmcp_tx_{}_{}", current.session_id.get(), plan.digest);

    for variant in 0..4 {
        let mut dispatcher = pending.clone();
        let mut candidate = snapshot.clone();
        let entity = candidate
            .graph
            .entities
            .get_mut(&entity_id)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "test work order missing",
                )
            })?;
        match variant {
            0 => entity.generation = 2,
            1 => entity.kind = EntityKind::Unit,
            2 => entity.kind = EntityKind::Other("work_order".to_owned()),
            _ => {
                entity.fields.insert(
                    "amount_total".to_owned(),
                    Fact::known(
                        Value::U64(1),
                        candidate.tick,
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                );
            }
        }
        candidate.refresh_hash();
        let before = candidate.clone();
        let current = context(&candidate);
        assert!(matches!(
            dispatcher.reconcile(&plan, &mut candidate, &current),
            Err(error) if error.code == ErrorCode::PreconditionsFailed
        ));
        assert_eq!(candidate, before);
        assert_eq!(
            dispatcher.journal().lookup(&journal_key),
            pending.journal().lookup(&journal_key)
        );

        candidate = snapshot.clone();
        let current = context(&candidate);
        let proven = dispatcher.reconcile(&plan, &mut candidate, &current)?;
        assert_eq!(proven.actions[0].state, CommitState::Verified);
        if let Some(entity) = candidate.graph.entities.get_mut(&entity_id) {
            entity.generation = 2;
        }
        candidate.refresh_hash();
        let current = context(&candidate);
        assert_eq!(
            dispatcher
                .reconcile(&plan, &mut candidate, &current)?
                .actions,
            proven.actions
        );
    }
    Ok(())
}
