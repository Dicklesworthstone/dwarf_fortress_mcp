//! Original finite harvest proof through the actual in-memory dispatcher.
use dfmcp_adapter::{PrepareReceipt, dispatcher::MutationDispatcher, live_routing::route_step};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId, Result,
    RiskTier, SessionId, StepId, WorkBudget,
};
use dfmcp_intent::{
    Action, Constraint, HarvestResource, Intent, ObligationSpec, PlanPolicy, PlanStep,
    PreparedPlan, RequestedAction, StaticPlanner, effects,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactSource, Predicate, Value, WorldGraph, WorldSnapshot,
};

const SOURCE: EntityId = EntityId::new(71);
const LEDGER: EntityId = EntityId::new(91);

#[test]
fn finite_harvest_has_no_implicit_native_tree_or_gathering_route() {
    for resource in [HarvestResource::Wood, HarvestResource::Plant] {
        let route = route_step(&PlanStep {
            id: StepId::new(1),
            required_capability: Capability::Designate,
            risk: RiskTier::Guarded,
            action: Action::HarvestResource {
                source: SOURCE,
                source_generation: 1,
                resource,
                amount: 2,
            },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
            idempotency_key: "harvest-native-refusal".to_owned(),
        });
        let refusal = route
            .outcome
            .expect_err("laboratory harvest must refuse live routing");
        assert!(refusal.reason.contains("finite-harvest/1"));
        assert!(refusal.reason.contains("no reviewed native identity"));
    }
}

fn put(snapshot: &mut WorldSnapshot, id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) {
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: "finite harvest dispatcher fixture".to_owned(),
            fields: fields
                .into_iter()
                .map(|(name, value)| {
                    (
                        name.to_owned(),
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
        FortressId::new(719),
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
                effects::LABORATORY_HARVEST_MODEL_FIELD,
                Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
        ],
    );
    put(
        &mut snapshot,
        SOURCE,
        EntityKind::Plant,
        vec![
            (
                effects::HARVEST_SOURCE_MODEL_FIELD,
                Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            (
                effects::HARVEST_RESOURCE_FIELD,
                Value::Text("PLANT".to_owned()),
            ),
            (effects::HARVEST_INITIAL_FIELD, Value::U64(4)),
            (effects::HARVEST_REMAINING_FIELD, Value::U64(4)),
            (effects::HARVEST_YIELDED_FIELD, Value::U64(0)),
        ],
    );
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_PLANT_FIELD, Value::U64(0)),
            (effects::STOCK_WOOD_FIELD, Value::U64(0)),
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
            ("labor.PLANT_GATHERING", Value::Bool(true)),
        ],
    );
    snapshot
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(719),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        cancellation_requested: false,
        grants: [
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
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

fn started() -> Result<(
    MutationDispatcher,
    PreparedPlan,
    PrepareReceipt,
    WorldSnapshot,
)> {
    let mut snapshot = source();
    let current = context(&snapshot);
    let intent = Intent {
        id: IntentId::new(719),
        anchor: snapshot.anchor(),
        summary: "harvest two finite plant units".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: SOURCE,
            field: effects::HARVEST_YIELDED_FIELD.to_owned(),
            op: dfmcp_world::CompareOp::Ge,
            value: Value::U64(2),
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![RequestedAction {
            action: Action::HarvestResource {
                source: SOURCE,
                source_generation: 1,
                resource: HarvestResource::Plant,
                amount: 2,
            },
            preconditions: Vec::new(),
            postconditions: vec![Predicate::True],
            obligation: Some(ObligationSpec {
                terminal: Predicate::True,
                failure: None,
                deadline_tick: GameTick(500),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            }),
            compensation: None,
            depends_on: Vec::new(),
        }],
    };
    let guarded = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &current)?;
    assert!(guarded.requires_checkpoint);
    assert_eq!(
        MutationDispatcher::new()
            .prepare_mutation(&guarded, &snapshot, &current)
            .expect_err("this isolated dispatcher has no checkpoint implementation")
            .code,
        ErrorCode::AdapterRejected,
    );
    // This component has no checkpoint backend. The actual MemoryAdapter and
    // public MCP tests use the unchanged default guarded checkpoint policy.
    let plan = StaticPlanner::new(PlanPolicy {
        require_checkpoint_at_or_above: RiskTier::Irreversible,
        ..PlanPolicy::default()
    })
    .prepare_laboratory(&snapshot, &intent, &current)?;
    let mut dispatcher = MutationDispatcher::new();
    let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &current)?;
    let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &current)?;
    assert_eq!(
        committed.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    Ok((dispatcher, plan, prepared, snapshot))
}

fn advance(snapshot: &mut WorldSnapshot, elapsed: u64) -> Result<()> {
    snapshot.tick = GameTick(snapshot.tick.0 + elapsed);
    effects::advance_effects(snapshot, elapsed)?;
    snapshot.cursor = snapshot.cursor.checked_next().expect("bounded test cursor");
    snapshot.refresh_hash();
    Ok(())
}

#[test]
fn dispatched_harvest_replays_without_yield_and_requires_complete_original_work() -> Result<()> {
    let (mut dispatcher, plan, prepared, mut snapshot) = started()?;
    let task = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let before = snapshot.clone();
    let current = context(&snapshot);
    let replay = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &current)?;
    assert_eq!(
        replay.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    assert_eq!(snapshot, before);
    advance(&mut snapshot, 50)?;
    let current = context(&snapshot);
    assert_eq!(
        dispatcher
            .reconcile(&plan, &mut snapshot, &current)?
            .actions[0]
            .state,
        CommitState::AppliedAwaitingVerification
    );
    assert_eq!(
        snapshot.graph.entities[&LEDGER].fields[effects::STOCK_PLANT_FIELD].known_value(),
        Some(&Value::U64(1))
    );
    advance(&mut snapshot, 50)?;
    let current = context(&snapshot);
    assert_eq!(
        dispatcher
            .reconcile(&plan, &mut snapshot, &current)?
            .actions[0]
            .state,
        CommitState::Verified
    );
    assert_eq!(
        snapshot.graph.entities[&LEDGER].fields[effects::STOCK_PLANT_FIELD].known_value(),
        Some(&Value::U64(2))
    );
    assert_eq!(
        snapshot.graph.entities[&SOURCE].fields[effects::HARVEST_REMAINING_FIELD].known_value(),
        Some(&Value::U64(2))
    );
    effects::validate_harvest_completion(&snapshot, task, &plan.steps[0].action)?;
    Ok(())
}

#[test]
fn harvest_dispatcher_preserves_pending_custody_when_matching_fields_hide_replaced_identity()
-> Result<()> {
    let (pending, plan, _, mut completed) = started()?;
    advance(&mut completed, 100)?;
    let task = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let transaction = format!(
        "dfmcp_tx_{}_{}",
        context(&completed).session_id.get(),
        plan.digest
    );
    let prior = pending.journal().lookup(&transaction).cloned();
    for replaced in [SOURCE, LEDGER, task] {
        let mut dispatcher = pending.clone();
        let mut altered = completed.clone();
        altered
            .graph
            .entities
            .get_mut(&replaced)
            .expect("original identity")
            .generation += 1;
        altered.refresh_hash();
        assert!(
            plan.steps[0]
                .postconditions
                .iter()
                .all(|predicate| dfmcp_world::evaluate(&altered, predicate))
        );
        let before = altered.clone();
        let current = context(&altered);
        assert!(dispatcher.reconcile(&plan, &mut altered, &current).is_err());
        assert_eq!(altered, before);
        assert_eq!(dispatcher.journal().lookup(&transaction).cloned(), prior);
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
