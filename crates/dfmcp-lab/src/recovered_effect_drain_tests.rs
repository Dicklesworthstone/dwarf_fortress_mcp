//! Real recovered effects, exact identity, current authority and atomic refusal.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, IntentId, MapCoord, MapCuboid, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_intent::{
    BuildingKind, Constraint, DigMode, FurnitureKind, HarvestResource, Intent, MaterialSelector,
    ObligationSpec, RequestedAction, StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{
    ChunkCoord, EntityKind, FactPresence, FactSource, Value, terrain::uniform_chunk, tile_codes,
};

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
            (Capability::Restore, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
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
        name: "recovered order".to_owned(),
        job_token: "MAKE_TEST".to_owned(),
        amount: 100,
        conditions: Vec::new(),
    }
}

fn world(solid: bool) -> MemoryAdapter {
    let mut graph = WorldGraph::default();
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    graph.chunks.insert(
        chunk,
        uniform_chunk(
            chunk,
            if solid {
                tile_codes::SOLID_WALL
            } else {
                tile_codes::FLOOR
            },
        ),
    );
    MemoryAdapter::new(WorldSnapshot::new(
        FortressId::new(88),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    ))
}

struct Original {
    plan: PreparedPlan,
    recorded_anchor: StateAnchor,
    recovery_anchor: StateAnchor,
    action_id: ActionId,
    entity_id: EntityId,
}

impl Original {
    fn reference(&self) -> RecoveredWorkRef<'_> {
        RecoveredWorkRef {
            plan: &self.plan,
            step_id: StepId::new(0),
            recorded_anchor: self.recorded_anchor,
            recovery_anchor: self.recovery_anchor,
            custody_digest: Digest32::of_bytes(b"verified original custody fixture"),
        }
    }
}

fn started(
    mut adapter: MemoryAdapter,
    action: Action,
    advance: u64,
) -> Result<(MemoryAdapter, Original)> {
    let key = derive_step_idempotency_key(
        IntentId::new(771),
        adapter.snapshot().anchor(),
        StepId::new(0),
        &action,
    );
    let entity_id = effects::created_entity_id(&key, 0);
    let postconditions =
        effects::default_postconditions(&action, &key, adapter.snapshot().fortress_id);
    let terminal = Predicate::All(postconditions.clone()).normalized();
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &Intent {
            id: IntentId::new(771),
            anchor: adapter.snapshot().anchor(),
            summary: "retain an exact original across recovery".to_owned(),
            terminal_condition: terminal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action,
                preconditions: Vec::new(),
                postconditions,
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal,
                    failure: None,
                    deadline_tick: GameTick(adapter.snapshot().tick.0 + 4_000),
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                }),
                depends_on: Vec::new(),
            }],
        },
        &context(&adapter),
    )?;
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    let committed = adapter.commit(&plan, &prepared, &context(&adapter))?;
    let action_id = committed.actions[0].action_id;
    let recorded_anchor = committed.actions[0].observed_anchor;
    if advance > 0 {
        adapter.advance_ticks(advance)?;
    }
    let recovered = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    let original = Original {
        plan,
        recorded_anchor,
        recovery_anchor: recovered.snapshot().anchor(),
        action_id,
        entity_id,
    };
    Ok((recovered, original))
}

fn refusal<T>(result: Result<T>) -> Result<DfmcpError> {
    match result {
        Err(error) => Ok(error),
        Ok(_) => Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "expected cleanup refusal",
        )),
    }
}

fn assert_refusal_is_atomic(
    adapter: &mut MemoryAdapter,
    reference: &RecoveredWorkRef<'_>,
    context: &OperationContext,
) -> Result<DfmcpError> {
    let before = adapter.snapshot().clone();
    let transcript = adapter.transcript().clone();
    let error = refusal(adapter.drain_recovered_work(reference, context))?;
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(adapter.transcript(), &transcript);
    assert!(adapter.actions.is_empty());
    assert!(adapter.plans.is_empty());
    assert!(adapter.commits.is_empty());
    Ok(error)
}

#[test]
fn recovered_cleanup_stops_every_temporal_family_without_restoring_handles() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(1, 1, 10), MapCoord::new(2, 2, 10))?;
    let (furniture, item) = furniture_lifecycle_tests::manufactured("CONSTRUCT_BED")?;
    let fixtures = vec![
        (
            world(true),
            Action::DesignateDig {
                area,
                mode: DigMode::Mine,
            },
            1,
        ),
        (
            world(false),
            Action::Build {
                kind: BuildingKind::Workshop("Still".to_owned()),
                location: area.min,
                footprint: area,
                material: MaterialSelector::default(),
            },
            1,
        ),
        (world(false), order(), 1),
        (
            harvest_lifecycle_tests::world(HarvestResource::Wood),
            Action::HarvestResource {
                source: EntityId::new(71),
                source_generation: 1,
                resource: HarvestResource::Wood,
                amount: 3,
            },
            50,
        ),
        (
            harvest_lifecycle_tests::world(HarvestResource::Plant),
            Action::HarvestResource {
                source: EntityId::new(71),
                source_generation: 1,
                resource: HarvestResource::Plant,
                amount: 3,
            },
            50,
        ),
        (
            furniture,
            Action::PlaceFurniture {
                kind: FurnitureKind::Bed,
                item,
                location: MapCoord::new(2, 2, 10),
            },
            1,
        ),
    ];
    for (adapter, action, advance) in fixtures {
        let (mut adapter, original) = started(adapter, action, advance)?;
        let reference = original.reference();
        let before = adapter.snapshot().anchor();
        assert_eq!(
            adapter.inspect_recovered_work_drain(&reference, &context(&adapter))?,
            EffectWorkState::Active {
                entity_id: original.entity_id
            }
        );
        assert_eq!(adapter.snapshot().anchor(), before);
        let receipt = adapter.drain_recovered_work(&reference, &context(&adapter))?;
        assert_eq!(receipt.before_anchor, before);
        assert_eq!(receipt.observed_anchor, adapter.snapshot().anchor());
        assert_eq!(receipt.recorded_anchor, original.recorded_anchor);
        assert_eq!(receipt.recovery_anchor, original.recovery_anchor);
        assert_eq!(receipt.custody_digest, reference.custody_digest);
        assert_eq!(receipt.plan_digest, original.plan.digest);
        assert!(receipt.stopped_work && receipt.after.is_quiescent());
        assert_eq!(receipt.observed_anchor.tick, before.tick);
        assert_eq!(
            receipt.observed_anchor.cursor.sequence,
            before.cursor.sequence + 1
        );
        assert!(adapter.action_receipt(original.action_id).is_none());
        assert!(adapter.known_work_entity_ids().next().is_none());
        assert!(
            adapter.actions.is_empty() && adapter.prepared.is_empty() && adapter.plans.is_empty()
        );
        let stopped = adapter.snapshot().graph.entities[&original.entity_id].clone();
        let terrain = adapter.snapshot().graph.chunks.clone();
        adapter.advance_ticks(1_000)?;
        assert_eq!(
            adapter.snapshot().graph.entities[&original.entity_id],
            stopped
        );
        assert_eq!(adapter.snapshot().graph.chunks, terrain);
        let quiet = adapter.snapshot().clone();
        let transcript = adapter.transcript().clone();
        let repeated = adapter.drain_recovered_work(&reference, &context(&adapter))?;
        assert!(!repeated.stopped_work && repeated.after.is_quiescent());
        assert_eq!(repeated.before_anchor, repeated.observed_anchor);
        assert_eq!(adapter.snapshot(), &quiet);
        assert_eq!(adapter.transcript(), &transcript);
    }
    Ok(())
}

#[test]
fn recovered_cleanup_requires_fresh_grants_even_when_original_plan_has_expired() -> Result<()> {
    let (mut adapter, original) = started(world(false), order(), 0)?;
    let elapsed = original
        .plan
        .expires_at_tick
        .0
        .saturating_sub(adapter.snapshot().tick.0)
        + 1;
    adapter.advance_ticks(elapsed)?;
    assert!(adapter.snapshot().tick > original.plan.expires_at_tick);
    let reference = original.reference();
    for mutation in 0..8 {
        let mut request = context(&adapter);
        match mutation {
            0 => request
                .grants
                .retain(|grant| grant.capability != Capability::Observe),
            1 => request
                .grants
                .retain(|grant| grant.capability != Capability::Query),
            2 => request
                .grants
                .retain(|grant| grant.capability != Capability::ConfigureProduction),
            3..=6 => {
                for grant in &mut request.grants {
                    if grant.capability == Capability::ConfigureProduction {
                        match mutation {
                            3 => grant.expires_at_tick = Some(GameTick(request.anchor.tick.0 - 1)),
                            4 => grant.max_risk = RiskTier::ReadOnly,
                            5 => grant.remaining_uses = Some(1),
                            _ => grant.scope.fortress_id = Some(FortressId::new(999)),
                        }
                    }
                }
            }
            _ => request.cancellation_requested = true,
        }
        let error = assert_refusal_is_atomic(&mut adapter, &reference, &request)?;
        assert_eq!(
            error.code,
            if mutation == 7 {
                ErrorCode::CancellationRequested
            } else {
                ErrorCode::CapabilityDenied
            }
        );
    }
    let mut fresh = context(&adapter);
    // Historical prepare authority is not authority for current cleanup.
    fresh
        .grants
        .retain(|grant| !matches!(grant.capability, Capability::Plan | Capability::Checkpoint));
    let receipt = adapter.drain_recovered_work(&reference, &fresh)?;
    assert!(receipt.stopped_work && receipt.after.is_quiescent());
    Ok(())
}

#[test]
fn recovered_cleanup_rechecks_exact_entity_and_spatial_scope() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(1, 1, 10), MapCoord::new(2, 2, 10))?;
    let cases = [
        started(
            world(true),
            Action::DesignateDig {
                area,
                mode: DigMode::Mine,
            },
            0,
        )?,
        started(
            harvest_lifecycle_tests::world(HarvestResource::Wood),
            Action::HarvestResource {
                source: EntityId::new(71),
                source_generation: 1,
                resource: HarvestResource::Wood,
                amount: 3,
            },
            0,
        )?,
    ];
    for (index, (mut adapter, original)) in cases.into_iter().enumerate() {
        let mut request = context(&adapter);
        for grant in &mut request.grants {
            if grant.capability == Capability::Designate {
                if index == 0 {
                    grant.scope.map_area = Some(MapCuboid::new(area.min, area.min)?);
                } else {
                    grant.scope.entity_ids.insert(EntityId::new(999));
                }
            }
        }
        assert_eq!(
            assert_refusal_is_atomic(&mut adapter, &original.reference(), &request)?.code,
            ErrorCode::CapabilityDenied
        );
    }
    Ok(())
}

#[test]
fn recovered_cleanup_refuses_missing_replaced_or_unqualified_work() -> Result<()> {
    for mutation in 0..9 {
        let (mut adapter, original) = started(world(false), order(), 0)?;
        if mutation == 0 {
            adapter.snapshot.graph.entities.remove(&original.entity_id);
        } else if let Some(entity) = adapter.snapshot.graph.entities.get_mut(&original.entity_id) {
            match mutation {
                1 => entity.generation += 1,
                2 => entity.kind = EntityKind::Unit,
                3 => entity.id = EntityId::new(999),
                _ => {
                    let name = if mutation == 4 {
                        "job_token"
                    } else {
                        effects::STATUS_FIELD
                    };
                    if let Some(fact) = entity.fields.get_mut(name) {
                        match mutation {
                            4 => fact.value = Value::Text("FOREIGN_JOB".to_owned()),
                            5 => {
                                fact.source = FactSource::AgentAssertion("not observed".to_owned())
                            }
                            6 => {
                                fact.presence =
                                    Some(FactPresence::Unknown("not observed".to_owned()))
                            }
                            7 => fact.observed_at = GameTick(10_000),
                            _ => fact.source = FactSource::Replay,
                        }
                    }
                }
            }
        }
        adapter.snapshot.cursor.sequence += 1;
        adapter.snapshot.refresh_hash();
        let request = context(&adapter);
        assert_refusal_is_atomic(&mut adapter, &original.reference(), &request)?;
    }
    Ok(())
}

#[test]
fn recovered_cleanup_rejects_wrong_original_or_recovery_lineage() -> Result<()> {
    let (mut adapter, original) = started(world(false), order(), 0)?;
    for mutation in 0..5 {
        let mut reference = original.reference();
        match mutation {
            0 => reference.custody_digest = Digest32::ZERO,
            1 => reference.step_id = StepId::new(999),
            2 => reference.recorded_anchor.state_hash = Digest32::of_bytes(b"wrong receipt source"),
            3 => {
                reference.recovery_anchor.state_hash = Digest32::of_bytes(b"wrong recovery source")
            }
            _ => reference.recovery_anchor.cursor.epoch += 1,
        }
        let request = context(&adapter);
        assert_refusal_is_atomic(&mut adapter, &reference, &request)?;
    }
    let mut changed = original.plan.clone();
    changed.steps[0].idempotency_key = "forged identity".to_owned();
    let reference = RecoveredWorkRef {
        plan: &changed,
        ..original.reference()
    };
    let request = context(&adapter);
    assert_eq!(
        assert_refusal_is_atomic(&mut adapter, &reference, &request)?.code,
        ErrorCode::InvalidPlan
    );
    let mut stale = context(&adapter);
    stale.anchor = original.recorded_anchor;
    assert_eq!(
        assert_refusal_is_atomic(&mut adapter, &original.reference(), &stale)?.code,
        ErrorCode::StaleAnchor
    );
    let mut ordinary = MemoryAdapter::new(adapter.snapshot().clone());
    let request = context(&ordinary);
    assert_refusal_is_atomic(&mut ordinary, &original.reference(), &request)?;
    let mut restored = adapter.clone();
    let checkpoint = restored.checkpoint("recovered-work", &context(&restored))?;
    restored.restore(checkpoint.checkpoint_id, &context(&restored))?;
    let request = context(&restored);
    assert_refusal_is_atomic(&mut restored, &original.reference(), &request)?;
    let mut recovered_again = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    let request = context(&recovered_again);
    assert_refusal_is_atomic(&mut recovered_again, &original.reference(), &request)?;
    let mut injected = adapter.snapshot().clone();
    injected.cursor.sequence += 1;
    injected.refresh_hash();
    adapter.inject_snapshot(injected)?;
    let request = context(&adapter);
    assert_refusal_is_atomic(&mut adapter, &original.reference(), &request)?;
    Ok(())
}

#[test]
fn recovered_cleanup_bounds_every_source_before_hashing_and_is_atomic_on_publication_failure()
-> Result<()> {
    let (mut adapter, original) = started(world(false), order(), 0)?;
    for mutation in 0..3 {
        let mut request = context(&adapter);
        match mutation {
            0 => request.budget.max_entities = 1,
            1 => request.budget.max_bytes = 64,
            _ => request.budget.max_actions = 0,
        }
        assert_eq!(
            assert_refusal_is_atomic(&mut adapter, &original.reference(), &request)?.code,
            if mutation == 2 {
                // A zero action dimension is an invalid budget, not exhaustion.
                ErrorCode::InvalidRequest
            } else {
                ErrorCode::BudgetExceeded
            }
        );
    }
    let mut oversized = original.plan.clone();
    oversized.summary = "x".repeat(65_537);
    let reference = RecoveredWorkRef {
        plan: &oversized,
        ..original.reference()
    };
    let request = context(&adapter);
    assert_eq!(
        assert_refusal_is_atomic(&mut adapter, &reference, &request)?.code,
        ErrorCode::BudgetExceeded
    );

    let clean = adapter.snapshot().clone();
    if let Some(entity) = adapter.snapshot.graph.entities.get_mut(&original.entity_id)
        && let Some(status) = entity.fields.get_mut(effects::STATUS_FIELD)
    {
        // Poison a compatibility value hidden by an otherwise valid presence.
        let known = status.known_value().cloned();
        let mut deep = Value::Null;
        for _ in 0..66 {
            deep = Value::List(vec![deep]);
        }
        status.value = deep;
        status.presence = known.map(FactPresence::Known);
    }
    adapter.snapshot.cursor.sequence += 1;
    adapter.snapshot.refresh_hash();
    let request = context(&adapter);
    assert_eq!(
        assert_refusal_is_atomic(&mut adapter, &original.reference(), &request)?.code,
        ErrorCode::BudgetExceeded
    );
    adapter.snapshot = clean;
    adapter.snapshot.cursor.sequence = u64::MAX;
    adapter.snapshot.refresh_hash();
    let request = context(&adapter);
    assert_eq!(
        assert_refusal_is_atomic(&mut adapter, &original.reference(), &request)?.code,
        ErrorCode::CursorGap
    );
    Ok(())
}
