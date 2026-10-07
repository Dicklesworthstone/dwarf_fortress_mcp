//! Physical-work regressions for df-cancellation-progress-certificates-vvw.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, IntentId, MapCoord, MapCuboid, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_intent::{
    BuildingKind, Constraint, DigMode, Intent, MaterialSelector, ObligationSpec, RequestedAction,
    StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{ChunkCoord, FactSource, Value, terrain::uniform_chunk, tile_codes};

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn order() -> Action {
    Action::CreateWorkOrder {
        name: "bounded order".to_owned(),
        job_token: "MAKE_TEST".to_owned(),
        amount: 100,
        conditions: Vec::new(),
    }
}

fn refusal<T>(result: Result<T>) -> Result<DfmcpError> {
    match result {
        Err(error) => Ok(error),
        Ok(_) => Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "test expected the operation to be refused",
        )),
    }
}

fn started(
    action: Action,
    early_proof: bool,
    compensation: Option<Action>,
) -> Result<(MemoryAdapter, ActionId, EntityId)> {
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    let mut graph = WorldGraph::default();
    graph.chunks.insert(
        chunk,
        uniform_chunk(
            chunk,
            if matches!(action, Action::DesignateDig { .. }) {
                tile_codes::SOLID_WALL
            } else {
                tile_codes::FLOOR
            },
        ),
    );
    let snapshot = WorldSnapshot::new(
        FortressId::new(88),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    let intent_id = IntentId::new(771);
    let key = derive_step_idempotency_key(intent_id, snapshot.anchor(), StepId::new(0), &action);
    let entity_id = effects::created_entity_id(&key, 0);
    let postconditions = if early_proof {
        vec![Predicate::EntityExists(entity_id)]
    } else {
        effects::default_postconditions(&action, &key, snapshot.fortress_id)
    };
    let terminal = Predicate::All(postconditions.clone()).normalized();
    let intent = Intent {
        id: intent_id,
        anchor: snapshot.anchor(),
        summary: "work outlives goal proof".to_owned(),
        terminal_condition: terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions,
            compensation,
            obligation: Some(ObligationSpec {
                terminal,
                failure: None,
                deadline_tick: GameTick(110),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            }),
            depends_on: Vec::new(),
        }],
    };
    let mut adapter = MemoryAdapter::new(snapshot);
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter),
    )?;
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    let commit = adapter.commit(&plan, &prepared, &context(&adapter))?;
    let action_id = commit.actions[0].action_id;
    Ok((adapter, action_id, entity_id))
}

fn fail(adapter: &mut MemoryAdapter, action_id: ActionId) -> Result<ActionReceipt> {
    adapter.advance_ticks(10)?;
    let receipt = adapter.poll_action(action_id, &context(adapter))?;
    assert_eq!(receipt.state, CommitState::Failed);
    Ok(receipt)
}

#[test]
fn failed_goal_keeps_active_work_until_a_separately_authorized_drain() -> Result<()> {
    let (mut adapter, action_id, entity_id) = started(order(), false, None)?;
    let proof = fail(&mut adapter, action_id)?;
    assert_eq!(
        adapter.action_work_state(action_id)?,
        EffectWorkState::Active { entity_id }
    );
    // Failure itself must not secretly cancel ongoing work.
    adapter.advance_ticks(50)?;
    let remaining_before =
        adapter.snapshot.graph.entities[&entity_id].fields[effects::AMOUNT_REMAINING_FIELD].clone();
    assert!(matches!(remaining_before.value, Value::U64(value) if value < 100));
    let drain = adapter.drain_action_work(action_id, &context(&adapter))?;
    assert!(drain.stopped_work);
    assert!(drain.after.is_quiescent());
    assert_eq!(drain.observed_anchor, adapter.snapshot.anchor());
    assert_eq!(adapter.action_receipt(action_id), Some(&proof));
    let stopped = adapter.snapshot.graph.entities[&entity_id].clone();
    adapter.advance_ticks(1_000)?;
    assert_eq!(adapter.snapshot.graph.entities[&entity_id], stopped);
    assert_eq!(adapter.poll_action(action_id, &context(&adapter))?, proof);
    let duplicate = adapter.drain_action_work(action_id, &context(&adapter))?;
    assert!(!duplicate.stopped_work);
    assert!(duplicate.after.is_quiescent());
    Ok(())
}

#[test]
fn early_verified_goal_is_not_physical_quiescence_and_its_proof_is_immutable() -> Result<()> {
    let (mut adapter, action_id, entity_id) = started(order(), true, None)?;
    adapter.advance_ticks(1)?;
    let proof = adapter.poll_action(action_id, &context(&adapter))?;
    assert_eq!(proof.state, CommitState::Verified);
    assert_eq!(
        adapter.action_work_state(action_id)?,
        EffectWorkState::Active { entity_id }
    );
    let drain = adapter.drain_action_work(action_id, &context(&adapter))?;
    assert!(drain.stopped_work && drain.after.is_quiescent());
    assert_eq!(adapter.action_receipt(action_id), Some(&proof));
    adapter.advance_ticks(50)?;
    assert_eq!(
        adapter.snapshot.graph.entities[&entity_id].fields[effects::AMOUNT_REMAINING_FIELD].value,
        Value::U64(100)
    );
    assert_eq!(adapter.poll_action(action_id, &context(&adapter))?, proof);
    Ok(())
}

#[test]
fn every_temporal_action_family_stops_physical_progress_after_failure() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(1, 1, 10), MapCoord::new(2, 2, 10))?;
    let actions = [
        Action::DesignateDig {
            area,
            mode: DigMode::Mine,
        },
        Action::Build {
            kind: BuildingKind::Workshop("Still".to_owned()),
            location: area.min,
            footprint: area,
            material: MaterialSelector::default(),
        },
        order(),
    ];
    for action in actions {
        let (mut adapter, action_id, entity_id) = started(action, false, None)?;
        let proof = fail(&mut adapter, action_id)?;
        assert_eq!(
            adapter.action_work_state(action_id)?,
            EffectWorkState::Active { entity_id }
        );
        adapter.drain_action_work(action_id, &context(&adapter))?;
        let graph = adapter.snapshot.graph.clone();
        adapter.advance_ticks(500)?;
        assert_eq!(adapter.snapshot.graph, graph);
        assert_eq!(adapter.action_receipt(action_id), Some(&proof));
    }
    Ok(())
}

#[test]
fn missing_untrusted_or_replaced_dispatched_work_cannot_certify_or_mutate_a_drain() -> Result<()> {
    for mutation in 0..10 {
        let (mut adapter, action_id, entity_id) = started(order(), false, None)?;
        let proof = fail(&mut adapter, action_id)?;
        if mutation == 0 {
            adapter.snapshot.graph.entities.remove(&entity_id);
        } else if let Some(entity) = adapter.snapshot.graph.entities.get_mut(&entity_id) {
            match mutation {
                1 => {
                    entity.fields.remove(effects::STATUS_FIELD);
                }
                2 => {
                    if let Some(status) = entity.fields.get_mut(effects::STATUS_FIELD) {
                        status.value = Value::Text(effects::STATUS_CANCELLED.to_owned());
                        status.source = FactSource::AgentAssertion("claimed stopped".to_owned());
                    }
                }
                3 => {
                    if let Some(status) = entity.fields.get_mut(effects::STATUS_FIELD) {
                        status.observed_at = GameTick(500);
                    }
                }
                4 => {
                    entity.generation += 1;
                }
                5 => {
                    if let Some(job) = entity.fields.get_mut("job_token") {
                        job.value = Value::Text("OTHER_JOB".to_owned());
                    }
                }
                6 => {
                    entity.kind = dfmcp_world::EntityKind::Unit;
                }
                7 => {
                    if let Some(status) = entity.fields.get_mut(effects::STATUS_FIELD) {
                        status.presence = Some(dfmcp_world::FactPresence::Unknown(
                            "not observed".to_owned(),
                        ));
                    }
                }
                8 => {
                    if let Some(status) = entity.fields.get_mut(effects::STATUS_FIELD) {
                        status.source = FactSource::Replay;
                    }
                }
                9 => {
                    if let Some(status) = entity.fields.get_mut(effects::STATUS_FIELD) {
                        status.source = FactSource::Derived("unregistered".to_owned());
                    }
                }
                _ => {}
            }
        }
        adapter.snapshot.refresh_hash();
        let before = adapter.snapshot.clone();
        let transcript = adapter.transcript.clone();
        assert!(matches!(
            adapter.action_work_state(action_id)?,
            EffectWorkState::Unknown { .. }
        ));
        assert_eq!(
            refusal(adapter.drain_action_work(action_id, &context(&adapter)))?.code,
            ErrorCode::CancellationIncomplete
        );
        assert_eq!(adapter.snapshot, before);
        assert_eq!(adapter.transcript, transcript);
        assert_eq!(adapter.action_receipt(action_id), Some(&proof));
    }
    Ok(())
}

#[test]
fn terminal_work_cleanup_rechecks_authority_expiry_scope_and_action_budget() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(1, 1, 10), MapCoord::new(2, 2, 10))?;
    for denial in 0..4 {
        let (mut adapter, action_id, _) = started(
            Action::DesignateDig {
                area,
                mode: DigMode::Mine,
            },
            false,
            None,
        )?;
        let proof = fail(&mut adapter, action_id)?;
        let mut denied = context(&adapter);
        if denial == 0 {
            denied
                .grants
                .retain(|grant| grant.capability == Capability::Observe);
        } else if denial == 3 {
            denied.budget.max_actions = 0;
        } else if let Some(grant) = denied
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Designate)
        {
            if denial == 1 {
                grant.expires_at_tick = Some(GameTick(adapter.snapshot.tick.0 - 1));
            } else {
                grant.scope.map_area = Some(MapCuboid::new(
                    MapCoord::new(8, 8, 10),
                    MapCoord::new(9, 9, 10),
                )?);
            }
        }
        let before = adapter.snapshot.clone();
        let error = refusal(adapter.drain_action_work(action_id, &denied))?;
        assert_eq!(
            error.code,
            if denial == 3 {
                ErrorCode::InvalidRequest
            } else {
                ErrorCode::CapabilityDenied
            }
        );
        assert_eq!(adapter.snapshot, before);
        assert_eq!(adapter.action_receipt(action_id), Some(&proof));
        assert!(!adapter.action_work_state(action_id)?.is_quiescent());
    }
    Ok(())
}

#[test]
fn ongoing_work_requires_request_and_finalize_before_physical_drain() -> Result<()> {
    let (mut adapter, action_id, entity_id) = started(order(), false, None)?;
    assert_eq!(
        refusal(adapter.drain_action_work(action_id, &context(&adapter)))?.code,
        ErrorCode::Conflict
    );
    adapter.request_cancel(action_id, CancelMode::StopFutureSteps, &context(&adapter))?;
    assert_eq!(
        adapter.action_work_state(action_id)?,
        EffectWorkState::Active { entity_id }
    );
    adapter.finalize_cancel(action_id, &context(&adapter))?;
    assert!(adapter.action_work_state(action_id)?.is_quiescent());
    let graph = adapter.snapshot.graph.clone();
    adapter.advance_ticks(500)?;
    assert_eq!(adapter.snapshot.graph, graph);
    Ok(())
}

#[test]
fn a_failed_compensation_rolls_back_the_stop_and_keeps_cancellation_pending() -> Result<()> {
    let compensation = Action::SetLabor {
        units: vec![EntityId::new(999)],
        labor: "BREW".to_owned(),
        enabled: true,
    };
    let (mut adapter, action_id, entity_id) = started(order(), false, Some(compensation))?;
    adapter.request_cancel(
        action_id,
        CancelMode::CompensateReversible,
        &context(&adapter),
    )?;
    let before = adapter.snapshot.clone();
    let request = adapter.action_receipt(action_id).cloned();
    let transcript = adapter.transcript.clone();
    assert!(
        adapter
            .finalize_cancel(action_id, &context(&adapter))
            .is_err()
    );
    assert_eq!(adapter.snapshot, before);
    assert_eq!(adapter.action_receipt(action_id), request.as_ref());
    assert_eq!(adapter.transcript, transcript);
    assert_eq!(
        adapter.action_work_state(action_id)?,
        EffectWorkState::Active { entity_id }
    );
    Ok(())
}

#[test]
fn finalization_cannot_claim_a_missing_dispatch_was_drained_or_spawn_temporal_compensation()
-> Result<()> {
    for missing in [true, false] {
        let (mut adapter, action_id, entity_id) = started(order(), false, (!missing).then(order))?;
        adapter.request_cancel(
            action_id,
            CancelMode::CompensateReversible,
            &context(&adapter),
        )?;
        if missing {
            adapter.snapshot.graph.entities.remove(&entity_id);
            adapter.snapshot.refresh_hash();
        }
        let before = adapter.snapshot.clone();
        let request = adapter.action_receipt(action_id).cloned();
        let transcript = adapter.transcript.clone();
        assert_eq!(
            refusal(adapter.finalize_cancel(action_id, &context(&adapter)))?.code,
            ErrorCode::CancellationIncomplete
        );
        assert_eq!(adapter.snapshot, before);
        assert_eq!(adapter.action_receipt(action_id), request.as_ref());
        assert_eq!(adapter.transcript, transcript);
    }
    Ok(())
}

#[test]
fn terminal_drain_publication_failure_leaves_world_and_terminal_proof_unchanged() -> Result<()> {
    let (mut adapter, action_id, entity_id) = started(order(), false, None)?;
    let proof = fail(&mut adapter, action_id)?;
    adapter.snapshot.cursor.sequence = u64::MAX;
    adapter.snapshot.refresh_hash();
    let before = adapter.snapshot.clone();
    let transcript = adapter.transcript.clone();
    assert_eq!(
        refusal(adapter.drain_action_work(action_id, &context(&adapter)))?.code,
        ErrorCode::CursorGap
    );
    assert_eq!(adapter.snapshot, before);
    assert_eq!(adapter.transcript, transcript);
    assert_eq!(adapter.action_receipt(action_id), Some(&proof));
    assert_eq!(
        adapter.action_work_state(action_id)?,
        EffectWorkState::Active { entity_id }
    );
    Ok(())
}

#[test]
fn undispatched_is_distinct_from_missing_after_dispatch_and_immediate_effects_are_quiescent()
-> Result<()> {
    let (adapter, action_id, _) = started(order(), false, None)?;
    let step = adapter.action_step(action_id).ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "test action absent")
    })?;
    let mut empty = adapter.snapshot.clone();
    empty.graph.entities.clear();
    empty.refresh_hash();
    assert_eq!(
        inspect_effect_work(&empty, &step.action, &step.idempotency_key, false)?,
        EffectWorkState::NeverDispatched
    );
    assert!(matches!(
        inspect_effect_work(&empty, &step.action, &step.idempotency_key, true)?,
        EffectWorkState::Unknown { .. }
    ));
    assert!(
        inspect_effect_work(
            &empty,
            &Action::AssignSquad {
                units: Vec::new(),
                squad: EntityId::new(7)
            },
            "immediate",
            true
        )?
        .is_quiescent()
    );
    Ok(())
}
