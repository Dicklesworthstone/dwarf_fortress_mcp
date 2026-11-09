//! Finite harvest through actual plan, dispatch, cancellation and durable-world APIs.
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, IntentId, MapCoord, RequestId, SessionId,
    WorkBudget,
};
use dfmcp_intent::{
    Constraint, HarvestResource, Intent, ObligationSpec, RequestedAction, StaticPlanner,
};
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LEDGER: EntityId = EntityId::new(91);
const SOURCE: EntityId = EntityId::new(71);
const WORKER: EntityId = EntityId::new(11);
const UNIT_TICKS: u64 = 50;
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct StateDirectory(PathBuf);
impl StateDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "dfmcp-harvest-custody-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        )))
    }
}
impl Drop for StateDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn record(id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        label: "finite harvest lifecycle fixture".to_owned(),
        kind,
        fields: fields
            .into_iter()
            .map(|(name, value)| {
                (
                    name.to_owned(),
                    Fact::known(
                        value,
                        GameTick(100),
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )
            })
            .collect(),
    }
}

fn stock_field(resource: HarvestResource) -> &'static str {
    match resource {
        HarvestResource::Wood => effects::STOCK_WOOD_FIELD,
        HarvestResource::Plant => effects::STOCK_PLANT_FIELD,
    }
}

fn world(resource: HarvestResource) -> MemoryAdapter {
    let fortress = FortressId::new(718);
    let settings = effects::fortress_settings_entity_id(fortress);
    let mut graph = WorldGraph::default();
    for entity in [
        record(
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
        ),
        record(
            LEDGER,
            EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            vec![
                (effects::STOCK_WOOD_FIELD, Value::U64(0)),
                (effects::STOCK_PLANT_FIELD, Value::U64(0)),
                (effects::STOCK_DRINK_FIELD, Value::U64(100)),
                (effects::STOCK_FOOD_FIELD, Value::U64(100)),
                (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
            ],
        ),
        record(
            SOURCE,
            EntityKind::Plant,
            vec![
                (
                    effects::HARVEST_SOURCE_MODEL_FIELD,
                    Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
                ),
                (
                    effects::HARVEST_RESOURCE_FIELD,
                    Value::Text(match resource {
                        HarvestResource::Wood => "WOOD".to_owned(),
                        HarvestResource::Plant => "PLANT".to_owned(),
                    }),
                ),
                (effects::HARVEST_INITIAL_FIELD, Value::U64(4)),
                (effects::HARVEST_REMAINING_FIELD, Value::U64(4)),
                (effects::HARVEST_YIELDED_FIELD, Value::U64(0)),
                ("position", Value::Coord(MapCoord::new(1, 1, 10))),
            ],
        ),
        record(
            WORKER,
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.WOODCUTTING", Value::Bool(true)),
                ("labor.PLANT_GATHERING", Value::Bool(true)),
            ],
        ),
    ] {
        graph.entities.insert(entity.id, entity);
    }
    MemoryAdapter::new(WorldSnapshot::new(
        fortress,
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    ))
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(718),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Designate, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(adapter.snapshot().fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn plan(adapter: &MemoryAdapter, resource: HarvestResource, deadline: u64) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &Intent {
            id: IntentId::new(718),
            anchor: adapter.snapshot().anchor(),
            summary: "harvest three units from one finite original resource".to_owned(),
            terminal_condition: Predicate::FieldCompare {
                entity_id: SOURCE,
                field: effects::HARVEST_YIELDED_FIELD.to_owned(),
                op: dfmcp_world::CompareOp::Ge,
                value: Value::U64(3),
            },
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action: Action::HarvestResource {
                    source: SOURCE,
                    source_generation: 1,
                    resource,
                    amount: 3,
                },
                preconditions: Vec::new(),
                // The model's mandatory identity/yield proof must survive even
                // when a caller supplies a trivially true terminal predicate.
                postconditions: vec![Predicate::True],
                compensation: None,
                depends_on: Vec::new(),
                obligation: Some(ObligationSpec {
                    terminal: Predicate::True,
                    failure: None,
                    deadline_tick: GameTick(deadline),
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                }),
            }],
        },
        &context(adapter),
    )
}

fn started(
    resource: HarvestResource,
    deadline: u64,
) -> Result<(MemoryAdapter, ActionId, EntityId)> {
    let mut adapter = world(resource);
    let plan = plan(&adapter, resource, deadline)?;
    let task = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let prepared = adapter.prepare(&plan, &context(&adapter))?;
    let receipt = adapter.commit(&plan, &prepared, &context(&adapter))?;
    assert_eq!(
        receipt.actions[0].state,
        CommitState::AppliedAwaitingVerification
    );
    Ok((adapter, receipt.actions[0].action_id, task))
}

fn quantity(adapter: &MemoryAdapter, entity: EntityId, field: &str) -> u64 {
    match adapter.snapshot().graph.entities[&entity].fields[field].known_value() {
        Some(Value::U64(value)) => *value,
        other => panic!("expected qualified quantity {field}, found {other:?}"),
    }
}

#[test]
fn harvest_retry_is_one_task_and_caller_true_does_not_prove_unearned_yield() -> Result<()> {
    for resource in [HarvestResource::Wood, HarvestResource::Plant] {
        let mut adapter = world(resource);
        let plan = plan(&adapter, resource, 500)?;
        let task = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
        let prepared = adapter.prepare(&plan, &context(&adapter))?;
        let first = adapter.commit(&plan, &prepared, &context(&adapter))?;
        let action = first.actions[0].action_id;
        let before_retry = adapter.snapshot().clone();
        assert_eq!(adapter.commit(&plan, &prepared, &context(&adapter))?, first);
        assert_eq!(adapter.snapshot(), &before_retry);
        assert_eq!(adapter.known_work_entity_ids().collect::<Vec<_>>(), [task]);
        adapter.advance_ticks(UNIT_TICKS)?;
        assert_eq!(quantity(&adapter, LEDGER, stock_field(resource)), 1);
        assert_eq!(
            quantity(&adapter, SOURCE, effects::HARVEST_REMAINING_FIELD),
            3
        );
        assert_eq!(
            adapter.poll_action(action, &context(&adapter))?.state,
            CommitState::AppliedAwaitingVerification
        );
        let partial = adapter.snapshot().clone();
        adapter.commit(&plan, &prepared, &context(&adapter))?;
        assert_eq!(adapter.snapshot(), &partial);
        adapter.advance_ticks(UNIT_TICKS * 2)?;
        assert_eq!(
            adapter.poll_action(action, &context(&adapter))?.state,
            CommitState::Verified
        );
        assert!(adapter.action_work_state(action)?.is_quiescent());
        assert_eq!(quantity(&adapter, LEDGER, stock_field(resource)), 3);
        assert_eq!(
            quantity(&adapter, SOURCE, effects::HARVEST_REMAINING_FIELD),
            1
        );
        assert_eq!(
            adapter
                .snapshot()
                .graph
                .entities
                .values()
                .filter(|entity| entity.kind
                    == EntityKind::Other(effects::HARVEST_TASK_KIND.to_owned()))
                .count(),
            1
        );
    }
    Ok(())
}

#[test]
fn harvest_cancellation_preserves_earned_yield_and_never_refunds_the_source() -> Result<()> {
    for resource in [HarvestResource::Wood, HarvestResource::Plant] {
        let (mut adapter, action, task) = started(resource, 500)?;
        adapter.advance_ticks(UNIT_TICKS + UNIT_TICKS / 2)?;
        assert_eq!(quantity(&adapter, LEDGER, stock_field(resource)), 1);
        assert_eq!(
            quantity(&adapter, SOURCE, effects::HARVEST_YIELDED_FIELD),
            1
        );
        adapter.request_cancel(action, CancelMode::StopFutureSteps, &context(&adapter))?;
        let cancelled = adapter.finalize_cancel(action, &context(&adapter))?;
        assert_eq!(cancelled.state, CommitState::Cancelled);
        assert!(adapter.action_work_state(action)?.is_quiescent());
        let history = adapter.snapshot().graph.entities[&task].clone();
        adapter.advance_ticks(UNIT_TICKS * 6)?;
        assert_eq!(adapter.snapshot().graph.entities[&task], history);
        assert_eq!(quantity(&adapter, LEDGER, stock_field(resource)), 1);
        assert_eq!(
            quantity(&adapter, SOURCE, effects::HARVEST_REMAINING_FIELD),
            3
        );
        assert_eq!(
            adapter.finalize_cancel(action, &context(&adapter))?,
            cancelled
        );
    }
    Ok(())
}

#[test]
fn failed_harvest_keeps_physical_work_until_a_fresh_source_scoped_drain() -> Result<()> {
    let (mut adapter, action, task) = started(HarvestResource::Wood, 160)?;
    adapter.advance_ticks(60)?;
    let failed = adapter.poll_action(action, &context(&adapter))?;
    assert_eq!(failed.state, CommitState::Failed);
    assert_eq!(
        adapter.action_work_state(action)?,
        EffectWorkState::Active { entity_id: task }
    );
    let mut wrong_source = context(&adapter);
    for grant in &mut wrong_source.grants {
        if grant.capability == Capability::Designate {
            grant.scope.entity_ids = [EntityId::new(72)].into_iter().collect();
        }
    }
    let before = adapter.snapshot().clone();
    assert_eq!(
        adapter
            .drain_action_work(action, &wrong_source)
            .expect_err("source scope must be current")
            .code,
        ErrorCode::CapabilityDenied
    );
    assert_eq!(adapter.snapshot(), &before);
    let drain = adapter.drain_action_work(action, &context(&adapter))?;
    assert!(drain.stopped_work && drain.after.is_quiescent());
    assert_eq!(adapter.action_receipt(action), Some(&failed));
    adapter.advance_ticks(200)?;
    assert_eq!(quantity(&adapter, LEDGER, effects::STOCK_WOOD_FIELD), 1);
    assert_eq!(
        quantity(&adapter, SOURCE, effects::HARVEST_REMAINING_FIELD),
        3
    );
    assert_eq!(adapter.poll_action(action, &context(&adapter))?, failed);
    Ok(())
}

#[test]
fn harvest_completion_refuses_replacement_task_source_or_ledger() -> Result<()> {
    for replaced in [SOURCE, LEDGER, EntityId::NIL] {
        let (mut adapter, action, task) = started(HarvestResource::Plant, 500)?;
        adapter.advance_ticks(UNIT_TICKS * 3)?;
        let pending = adapter.action_receipt(action).cloned();
        let replaced = if replaced == EntityId::NIL {
            task
        } else {
            replaced
        };
        adapter
            .snapshot
            .graph
            .entities
            .get_mut(&replaced)
            .expect("fixture identity")
            .generation += 1;
        adapter.snapshot.refresh_hash();
        let before = adapter.snapshot().clone();
        let step = adapter.action_step(action).expect("original harvest step");
        assert!(
            PredicateEvidence::laboratory(adapter.snapshot())?
                .establishes(&Predicate::All(step.postconditions.clone()))?
        );
        assert!(adapter.poll_action(action, &context(&adapter)).is_err());
        assert_eq!(adapter.snapshot(), &before);
        assert_eq!(adapter.action_receipt(action).cloned(), pending);
        assert!(matches!(
            adapter.action_work_state(action)?,
            EffectWorkState::Unknown { .. }
        ));
    }
    Ok(())
}

#[test]
fn durable_harvest_resume_keeps_depletion_and_never_restores_old_dispatch_permission() -> Result<()>
{
    let (mut adapter, action, task) = started(HarvestResource::Plant, 500)?;
    let original_step = adapter
        .action_step(action)
        .cloned()
        .expect("original harvest");
    adapter.advance_ticks(75)?;
    let interrupted = adapter.snapshot().clone();
    let directory = StateDirectory::new();
    {
        let mut store = durable::DurableLabStore::open(&directory.0)?;
        store.persist_head("finite_harvest", &interrupted)?;
    }
    let store = durable::DurableLabStore::open(&directory.0)?;
    let head = store.head(interrupted.fortress_id).expect("durable head");
    let reopened = store.load_snapshot(head.anchor.state_hash)?;
    assert_eq!(reopened, interrupted);
    let mut recovered = MemoryAdapter::recovered(reopened)?;
    assert!(recovered.snapshot().cursor.epoch > interrupted.cursor.epoch);
    assert!(recovered.action_receipt(action).is_none());
    assert_eq!(recovered.known_work_entity_ids().count(), 0);
    assert_eq!(
        inspect_effect_work(
            recovered.snapshot(),
            &original_step.action,
            &original_step.idempotency_key,
            true
        )?,
        EffectWorkState::Active { entity_id: task }
    );
    assert!(
        recovered
            .request_cancel(action, CancelMode::StopFutureSteps, &context(&recovered))
            .is_err()
    );
    recovered.advance_ticks(75)?;
    effects::validate_harvest_completion(recovered.snapshot(), task, &original_step.action)?;
    assert_eq!(quantity(&recovered, LEDGER, effects::STOCK_PLANT_FIELD), 3);
    assert_eq!(
        quantity(&recovered, SOURCE, effects::HARVEST_REMAINING_FIELD),
        1
    );
    let completed = recovered.snapshot().graph.entities[&task].clone();
    recovered.advance_ticks(200)?;
    assert_eq!(recovered.snapshot().graph.entities[&task], completed);
    assert_eq!(quantity(&recovered, LEDGER, effects::STOCK_PLANT_FIELD), 3);
    Ok(())
}
