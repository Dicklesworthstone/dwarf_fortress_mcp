use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, EntityId, FortressId, IntentId,
    ObservationCursor, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner};
use dfmcp_world::{CompareOp, EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph};

const LEDGER: EntityId = EntityId::new(91);

fn put(snapshot: &mut WorldSnapshot, id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) {
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: "provisions dispatcher fixture".to_owned(),
            fields: fields
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
                .collect(),
        },
    );
    snapshot.refresh_hash();
}

fn source() -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(710),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
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
                effects::LABORATORY_PROVISIONS_MODEL_FIELD,
                Value::Text(effects::LABORATORY_PROVISIONS_MODEL.to_owned()),
            ),
        ],
    );
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_PLANT_FIELD, Value::U64(2)),
            (effects::STOCK_DRINK_FIELD, Value::U64(40)),
            (effects::STOCK_FOOD_FIELD, Value::U64(60)),
            (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    );
    put(
        &mut snapshot,
        EntityId::new(11),
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.BREW", Value::Bool(true)),
        ],
    );
    put(
        &mut snapshot,
        EntityId::new(21),
        EntityKind::Building,
        vec![
            ("building_kind", Value::Text("workshop:Still".to_owned())),
            (
                effects::CONSTRUCTION_STAGE_FIELD,
                Value::Text(effects::STAGE_COMPLETE.to_owned()),
            ),
            ("required_ticks", Value::U64(effects::BUILD_TICKS)),
            ("progress_ticks", Value::U64(effects::BUILD_TICKS)),
        ],
    );
    snapshot
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(710),
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

fn started(custom_terminal: bool) -> Result<(MutationDispatcher, PreparedPlan, WorldSnapshot)> {
    let mut snapshot = source();
    let intent = Intent {
        id: IntentId::new(710),
        anchor: snapshot.anchor(),
        summary: "produce two ingredient-backed brewing batches".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: LEDGER,
            field: effects::STOCK_DRINK_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(50),
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::CreateWorkOrder {
                name: "two brews".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 2,
                conditions: Vec::new(),
            },
            preconditions: Vec::new(),
            postconditions: if custom_terminal {
                vec![Predicate::True]
            } else {
                Vec::new()
            },
            obligation: custom_terminal.then_some(ObligationSpec {
                terminal: Predicate::True,
                failure: None,
                deadline_tick: GameTick(2_100),
                poll_interval_ticks: 10,
                stable_for_observations: 1,
            }),
            compensation: None,
            depends_on: Vec::new(),
        }],
    };
    let current = context(&snapshot);
    let plan = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &current)?;
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &current)?;
    let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &current)?;
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
fn provision_completion_rejects_replaced_generations_and_inventory_domains() -> Result<()> {
    let (pending, plan, mut completed) = started(false)?;
    advance(&mut completed, effects::WORK_ORDER_TICKS_PER_UNIT * 2)?;
    let order = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
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
            0 => record_mut(&mut replaced, order)?.generation = 2,
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
                    .remove(effects::LABORATORY_PROVISIONS_MODEL_FIELD);
            }
        }
        replaced.refresh_hash();
        assert!(
            plan.steps[0]
                .postconditions
                .iter()
                .all(|predicate| dfmcp_world::evaluate(&replaced, predicate)),
            "variant {variant} challenges identity outside ordinary completion fields"
        );
        let before = replaced.clone();
        let current = context(&replaced);
        assert!(
            dispatcher
                .reconcile(&plan, &mut replaced, &current)
                .is_err(),
            "variant {variant}"
        );
        assert_eq!(replaced, before);
        assert_eq!(dispatcher.journal().lookup(&key).cloned(), previous);
        let mut original = completed.clone();
        let current = context(&original);
        assert_eq!(
            dispatcher
                .reconcile(&plan, &mut original, &current)?
                .actions[0]
                .state,
            CommitState::Verified
        );
    }
    Ok(())
}

#[test]
fn caller_true_predicates_cannot_complete_an_unconsumed_provisions_order() -> Result<()> {
    let (mut dispatcher, plan, mut snapshot) = started(true)?;
    advance(&mut snapshot, 10)?;
    let current = context(&snapshot);
    assert_eq!(
        dispatcher
            .reconcile(&plan, &mut snapshot, &current)?
            .actions[0]
            .state,
        CommitState::AppliedAwaitingVerification
    );
    let report = effects::provisions_report(&snapshot)?.ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "provisions report missing",
        )
    })?;
    assert_eq!((report.available_plants, report.consumed_plants), (2, 0));
    advance(&mut snapshot, effects::WORK_ORDER_TICKS_PER_UNIT * 2 - 10)?;
    let current = context(&snapshot);
    assert_eq!(
        dispatcher
            .reconcile(&plan, &mut snapshot, &current)?
            .actions[0]
            .state,
        CommitState::Verified
    );
    let report = effects::provisions_report(&snapshot)?.ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "completed provisions report missing",
        )
    })?;
    assert_eq!((report.available_plants, report.consumed_plants), (0, 2));
    Ok(())
}
