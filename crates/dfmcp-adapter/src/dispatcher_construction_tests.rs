use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, EntityId, FortressId, IntentId, MapCoord,
    MapCuboid, ObservationCursor, RequestId, RiskTier, SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, BuildingKind, Constraint, Intent, MaterialSelector, ObligationSpec, PlanPolicy,
    RequestedAction, StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{ChunkCoord, EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph};

const LEDGER: EntityId = EntityId::new(91);

fn put(snapshot: &mut WorldSnapshot, id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) {
    let fields = fields
        .into_iter()
        .map(|(field, value)| {
            (
                field.to_owned(),
                Fact::known(
                    value,
                    snapshot.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            )
        })
        .collect();
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: "material dispatcher fixture".to_owned(),
            fields,
        },
    );
    snapshot.refresh_hash();
}

fn source() -> WorldSnapshot {
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    let mut graph = WorldGraph::default();
    graph.chunks.insert(
        chunk,
        dfmcp_world::terrain::uniform_chunk(chunk, dfmcp_world::tile_codes::FLOOR),
    );
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(708),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    let settings = effects::fortress_settings_entity_id(snapshot.fortress_id);
    put(
        &mut snapshot,
        settings,
        EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
        vec![
            (
                effects::LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
            ),
            (
                effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD,
                Value::Text(effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL.to_owned()),
            ),
        ],
    );
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_WOOD_FIELD, Value::U64(1)),
            (effects::STOCK_DRINK_FIELD, Value::U64(100)),
            (effects::STOCK_FOOD_FIELD, Value::U64(100)),
            (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    );
    put(
        &mut snapshot,
        EntityId::new(11),
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.CONSTRUCTION", Value::Bool(true)),
        ],
    );
    snapshot
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(708),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        cancellation_requested: false,
        grants: [
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Construct, RiskTier::Guarded),
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

fn plan(snapshot: &WorldSnapshot, custom_terminal: bool) -> Result<PreparedPlan> {
    let location = MapCoord::new(4, 4, 10);
    let action = Action::Build {
        kind: BuildingKind::Workshop("Still".to_owned()),
        location,
        footprint: MapCuboid {
            min: location,
            max: location,
        },
        material: MaterialSelector::default(),
    };
    let intent_id = IntentId::new(708);
    let key = derive_step_idempotency_key(intent_id, snapshot.anchor(), StepId::new(0), &action);
    let intent = Intent {
        id: intent_id,
        anchor: snapshot.anchor(),
        summary: "build a material-backed workshop".to_owned(),
        terminal_condition: Predicate::EntityExists(effects::created_entity_id(&key, 0)),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions: if custom_terminal {
                vec![Predicate::True]
            } else {
                Vec::new()
            },
            obligation: if custom_terminal {
                Some(ObligationSpec {
                    terminal: Predicate::True,
                    failure: None,
                    deadline_tick: GameTick(snapshot.tick.0 + 2_000),
                    poll_interval_ticks: 10,
                    stable_for_observations: 1,
                })
            } else {
                None
            },
            compensation: None,
            depends_on: Vec::new(),
        }],
    };
    StaticPlanner::new(PlanPolicy {
        require_checkpoint_at_or_above: RiskTier::Irreversible,
        ..PlanPolicy::default()
    })
    .prepare_laboratory(snapshot, &intent, &context(snapshot))
}

fn started(custom_terminal: bool) -> Result<(MutationDispatcher, PreparedPlan, WorldSnapshot)> {
    let mut snapshot = source();
    let plan = plan(&snapshot, custom_terminal)?;
    let context = context(&snapshot);
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
    let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context)?;
    assert_eq!(
        committed.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    Ok((dispatcher, plan, snapshot))
}

fn advance(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<()> {
    snapshot.tick = GameTick(snapshot.tick.0 + elapsed);
    effects::advance_effects(snapshot, elapsed)?;
    snapshot.cursor = snapshot
        .cursor
        .checked_next()
        .ok_or_else(|| DfmcpError::new(ErrorCode::CursorGap, "test cursor exhausted"))?;
    snapshot.refresh_hash();
    Ok(())
}

fn record_mut(snapshot: &mut WorldSnapshot, id: EntityId) -> Result<&mut EntityRecord> {
    snapshot.graph.entities.get_mut(&id).ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "test record missing")
    })
}

#[test]
fn material_completion_cannot_publish_for_replaced_generations_or_custody_domains() -> Result<()> {
    let (pending, plan, mut completed) = started(false)?;
    advance(&mut completed, effects::BUILD_TICKS)?;
    let building = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let key = format!(
        "dfmcp_tx_{}_{}",
        context(&completed).session_id.get(),
        plan.digest
    );
    let previous = pending.journal().lookup(&key).cloned();
    for variant in 0..4 {
        let mut dispatcher = pending.clone();
        let mut replaced = completed.clone();
        match variant {
            0 => record_mut(&mut replaced, building)?.generation = 2,
            1 => record_mut(&mut replaced, LEDGER)?.generation = 2,
            2 => {
                let mut duplicate =
                    replaced
                        .graph
                        .entities
                        .get(&LEDGER)
                        .cloned()
                        .ok_or_else(|| {
                            DfmcpError::new(ErrorCode::InternalInvariantViolation, "ledger missing")
                        })?;
                duplicate.id = EntityId::new(92);
                replaced.graph.entities.insert(duplicate.id, duplicate);
            }
            _ => {
                let settings = effects::fortress_settings_entity_id(replaced.fortress_id);
                record_mut(&mut replaced, settings)?
                    .fields
                    .remove(effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL_FIELD);
            }
        }
        replaced.refresh_hash();
        assert!(
            plan.steps[0]
                .postconditions
                .iter()
                .all(|predicate| dfmcp_world::evaluate(&replaced, predicate)),
            "variant {variant} must challenge identity outside the field predicates"
        );
        let before = replaced.clone();
        let current = context(&replaced);
        assert!(
            dispatcher
                .reconcile(&plan, &mut replaced, &current)
                .is_err(),
            "variant {variant}"
        );
        assert_eq!(replaced, before, "variant {variant} changed the source");
        assert_eq!(
            dispatcher.journal().lookup(&key).cloned(),
            previous,
            "variant {variant} changed the original receipt"
        );
        let mut original = completed.clone();
        let current = context(&original);
        let valid = dispatcher.reconcile(&plan, &mut original, &current)?;
        assert_eq!(valid.actions[0].state, CommitState::Verified);
    }
    Ok(())
}

#[test]
fn supplied_true_action_proof_still_waits_for_actual_material_consumption() -> Result<()> {
    let (mut dispatcher, plan, mut snapshot) = started(true)?;
    advance(&mut snapshot, 10)?;
    let current = context(&snapshot);
    let waiting = dispatcher.reconcile(&plan, &mut snapshot, &current)?;
    assert_eq!(
        waiting.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    let report = effects::construction_material_report(&snapshot)?.ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "model report missing",
        )
    })?;
    assert_eq!(
        (
            report.available_wood,
            report.held_wood,
            report.consumed_wood
        ),
        (0, 1, 0)
    );
    advance(&mut snapshot, effects::BUILD_TICKS - 10)?;
    let current = context(&snapshot);
    let completed = dispatcher.reconcile(&plan, &mut snapshot, &current)?;
    assert_eq!(completed.actions[0].state, CommitState::Verified);
    Ok(())
}
