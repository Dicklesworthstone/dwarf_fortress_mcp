//! Exact item custody through the public laboratory transaction and recovery APIs.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, IntentId, MapCoord, MapCuboid, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_intent::{
    Constraint, FurnitureKind, Intent, ObligationSpec, RequestedAction, StaticPlanner,
    derive_step_idempotency_key,
};
use dfmcp_world::{
    ChunkCoord, EntityKind, EntityRecord, Fact, FactSource, Value, terrain::uniform_chunk,
    tile_codes,
};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const LEDGER: EntityId = EntityId::new(91);
const WORKER: EntityId = EntityId::new(11);
const WORKSHOP: EntityId = EntityId::new(51);
const SITE: MapCoord = MapCoord::new(2, 2, 10);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

struct StateDirectory(PathBuf);

impl StateDirectory {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "dfmcp-furniture-custody-{}-{}",
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

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::Checkpoint, RiskTier::Guarded),
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

fn record(id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        label: kind.as_str().to_owned(),
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

fn world() -> Result<MemoryAdapter> {
    let mut graph = WorldGraph::default();
    let chunk = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(chunk, uniform_chunk(chunk, tile_codes::FLOOR));
    graph.entities.insert(
        WORKER,
        record(
            WORKER,
            EntityKind::Unit,
            vec![
                ("alive", Value::Bool(true)),
                ("labor.CARPENTRY", Value::Bool(true)),
            ],
        ),
    );
    let workshop_site = MapCoord::new(12, 12, 10);
    let workshop_area = MapCuboid::new(workshop_site, workshop_site)?;
    graph.entities.insert(
        WORKSHOP,
        record(
            WORKSHOP,
            EntityKind::Building,
            vec![
                (
                    "building_kind",
                    Value::Text("workshop:Carpenters".to_owned()),
                ),
                (
                    effects::CONSTRUCTION_STAGE_FIELD,
                    Value::Text(effects::STAGE_COMPLETE.to_owned()),
                ),
                ("position", Value::Coord(workshop_site)),
                ("footprint_min", Value::Coord(workshop_area.min)),
                ("footprint_max", Value::Coord(workshop_area.max)),
            ],
        ),
    );
    graph.entities.insert(
        LEDGER,
        record(
            LEDGER,
            EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
            vec![
                (effects::STOCK_DRINK_FIELD, Value::U64(100)),
                (effects::STOCK_FOOD_FIELD, Value::U64(100)),
                (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
                (effects::STOCK_WOOD_FIELD, Value::U64(1)),
                (effects::STOCK_BED_FIELD, Value::U64(0)),
                (effects::STOCK_DOOR_FIELD, Value::U64(0)),
                (effects::STOCK_TABLE_FIELD, Value::U64(0)),
                (effects::STOCK_CHAIR_FIELD, Value::U64(0)),
            ],
        ),
    );
    Ok(MemoryAdapter::new(WorldSnapshot::new(
        FortressId::new(88),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )))
}

fn prepare_action(adapter: &MemoryAdapter, action: Action, id: u128) -> Result<PreparedPlan> {
    let intent_id = IntentId::new(id);
    let key = derive_step_idempotency_key(
        intent_id,
        adapter.snapshot().anchor(),
        StepId::new(0),
        &action,
    );
    let postconditions =
        effects::default_postconditions(&action, &key, adapter.snapshot().fortress_id);
    let terminal = Predicate::All(postconditions.clone()).normalized();
    let intent = Intent {
        id: intent_id,
        anchor: adapter.snapshot().anchor(),
        summary: "manufacture and place one exact furniture item".to_owned(),
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
    };
    StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &context(adapter))
}

fn dispatch(adapter: &mut MemoryAdapter, action: Action, id: u128) -> Result<(ActionId, EntityId)> {
    let plan = prepare_action(adapter, action, id)?;
    let entity = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let prepared = adapter.prepare(&plan, &context(adapter))?;
    let committed = adapter.commit(&plan, &prepared, &context(adapter))?;
    Ok((committed.actions[0].action_id, entity))
}

fn manufactured(job: &str) -> Result<(MemoryAdapter, EntityId)> {
    let mut adapter = world()?;
    let (action, order) = dispatch(
        &mut adapter,
        Action::CreateWorkOrder {
            name: "one exact item".to_owned(),
            job_token: job.to_owned(),
            amount: 1,
            conditions: Vec::new(),
        },
        100,
    )?;
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT)?;
    assert_eq!(
        adapter.poll_action(action, &context(&adapter))?.state,
        CommitState::Verified
    );
    let item = effects::produced_item_id(order, 1);
    assert_eq!(
        value(&adapter, LEDGER, effects::STOCK_WOOD_FIELD)?,
        &Value::U64(0)
    );
    assert_eq!(
        value(&adapter, item, effects::FURNITURE_INVENTORY_STATE_FIELD)?,
        &Value::Text("loose".to_owned())
    );
    Ok((adapter, item))
}

fn placement(kind: FurnitureKind, item: EntityId, location: MapCoord) -> Action {
    Action::PlaceFurniture {
        kind,
        item,
        location,
    }
}

fn value<'a>(adapter: &'a MemoryAdapter, id: EntityId, name: &str) -> Result<&'a Value> {
    adapter
        .snapshot()
        .graph
        .entities
        .get(&id)
        .and_then(|record| record.fields.get(name))
        .and_then(Fact::known_value)
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                format!("test expected established field {name} on {id}"),
            )
        })
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

#[test]
fn manufactured_furniture_is_debited_attached_and_proved_as_the_same_item() -> Result<()> {
    for (job, kind, stock) in [
        (
            "CONSTRUCT_BED",
            FurnitureKind::Bed,
            effects::STOCK_BED_FIELD,
        ),
        (
            "CONSTRUCT_DOOR",
            FurnitureKind::Door,
            effects::STOCK_DOOR_FIELD,
        ),
        (
            "CONSTRUCT_TABLE",
            FurnitureKind::Table,
            effects::STOCK_TABLE_FIELD,
        ),
        (
            "CONSTRUCT_THRONE",
            FurnitureKind::Chair,
            effects::STOCK_CHAIR_FIELD,
        ),
    ] {
        let (mut adapter, item) = manufactured(job)?;
        let (action, building) = dispatch(&mut adapter, placement(kind, item, SITE), 200)?;
        assert!(adapter.known_work_entity_ids().any(|id| id == building));
        assert_eq!(value(&adapter, LEDGER, stock)?, &Value::U64(0));
        assert_eq!(
            value(&adapter, item, effects::FURNITURE_ALLOCATED_TO_FIELD)?,
            &Value::Entity(building)
        );
        assert_eq!(
            value(&adapter, building, effects::FURNITURE_SOURCE_ITEM_FIELD)?,
            &Value::Entity(item)
        );
        assert_eq!(
            adapter.action_work_state(action)?,
            EffectWorkState::Active {
                entity_id: building
            }
        );
        adapter.advance_ticks(effects::BUILD_TICKS)?;
        assert_eq!(
            adapter.poll_action(action, &context(&adapter))?.state,
            CommitState::Verified
        );
        assert!(adapter.action_work_state(action)?.is_quiescent());
        assert_eq!(value(&adapter, item, "position")?, &Value::Coord(SITE));
        assert_eq!(
            value(&adapter, item, effects::FURNITURE_INVENTORY_STATE_FIELD)?,
            &Value::Text("installed".to_owned())
        );
        assert_eq!(
            adapter
                .snapshot()
                .graph
                .entities
                .values()
                .filter(|record| record.kind == EntityKind::Item)
                .count(),
            1
        );
        let before = adapter.snapshot().clone();
        let drain = adapter.drain_action_work(action, &context(&adapter))?;
        assert!(!drain.stopped_work);
        assert_eq!(adapter.snapshot(), &before);
        assert_eq!(
            refusal(adapter.request_cancel(
                action,
                CancelMode::StopFutureSteps,
                &context(&adapter)
            ))?
            .code,
            ErrorCode::Conflict
        );
        assert_eq!(adapter.snapshot(), &before);
    }
    Ok(())
}

#[test]
fn cancelled_placement_refunds_once_and_released_item_can_be_reused() -> Result<()> {
    let (mut adapter, item) = manufactured("CONSTRUCT_BED")?;
    let (first, old_building) =
        dispatch(&mut adapter, placement(FurnitureKind::Bed, item, SITE), 201)?;
    adapter.advance_ticks(100)?;
    adapter.request_cancel(first, CancelMode::StopFutureSteps, &context(&adapter))?;
    assert_eq!(
        adapter.finalize_cancel(first, &context(&adapter))?.state,
        CommitState::Cancelled
    );
    assert_eq!(
        value(&adapter, LEDGER, effects::STOCK_BED_FIELD)?,
        &Value::U64(1)
    );
    assert_eq!(
        value(&adapter, item, effects::FURNITURE_ALLOCATED_TO_FIELD)?,
        &Value::Null
    );
    assert_eq!(
        value(&adapter, old_building, effects::CONSTRUCTION_STAGE_FIELD)?,
        &Value::Text(effects::STAGE_CANCELLED.to_owned())
    );
    let refunded = adapter.snapshot().clone();
    adapter.finalize_cancel(first, &context(&adapter))?;
    assert_eq!(adapter.snapshot(), &refunded);

    let next_site = MapCoord::new(4, 2, 10);
    let (second, new_building) = dispatch(
        &mut adapter,
        placement(FurnitureKind::Bed, item, next_site),
        202,
    )?;
    assert_ne!(old_building, new_building);
    assert_eq!(
        value(&adapter, LEDGER, effects::STOCK_BED_FIELD)?,
        &Value::U64(0)
    );
    let reallocated = adapter.snapshot().clone();
    // A historical cancellation does not own an item released to a later action.
    adapter.finalize_cancel(first, &context(&adapter))?;
    assert_eq!(adapter.snapshot(), &reallocated);
    adapter.advance_ticks(effects::BUILD_TICKS)?;
    assert_eq!(
        adapter.poll_action(second, &context(&adapter))?.state,
        CommitState::Verified
    );
    assert_eq!(value(&adapter, item, "position")?, &Value::Coord(next_site));
    assert_eq!(
        value(&adapter, LEDGER, effects::STOCK_BED_FIELD)?,
        &Value::U64(0)
    );
    Ok(())
}

#[test]
fn fresh_second_plan_cannot_allocate_an_item_already_owned_by_running_work() -> Result<()> {
    let (mut adapter, item) = manufactured("CONSTRUCT_BED")?;
    let (first, building) = dispatch(&mut adapter, placement(FurnitureKind::Bed, item, SITE), 203)?;
    let before = adapter.snapshot().clone();
    // Compile against the current anchor so stale-anchor rejection cannot mask custody.
    let error = refusal(dispatch(
        &mut adapter,
        placement(FurnitureKind::Bed, item, MapCoord::new(3, 2, 10)),
        204,
    ))?;
    assert_eq!(error.code, ErrorCode::PreconditionsFailed);
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(
        adapter.action_work_state(first)?,
        EffectWorkState::Active {
            entity_id: building
        }
    );
    Ok(())
}

#[test]
fn placement_requires_authority_for_both_the_exact_item_and_destination() -> Result<()> {
    let (mut adapter, item) = manufactured("CONSTRUCT_BED")?;
    let plan = prepare_action(&adapter, placement(FurnitureKind::Bed, item, SITE), 205)?;
    let before = adapter.snapshot().clone();
    for (granted_item, granted_site) in
        [(EntityId::new(999), SITE), (item, MapCoord::new(8, 8, 10))]
    {
        let mut authority = context(&adapter);
        for grant in &mut authority.grants {
            if grant.capability == Capability::Construct {
                grant.scope.entity_ids = [granted_item].into_iter().collect();
                grant.scope.map_area = Some(MapCuboid::new(granted_site, granted_site)?);
            }
        }
        assert_eq!(
            refusal(adapter.prepare(&plan, &authority))?.code,
            ErrorCode::CapabilityDenied
        );
        assert_eq!(adapter.snapshot(), &before);
    }
    let mut authority = context(&adapter);
    for grant in &mut authority.grants {
        if grant.capability == Capability::Construct {
            grant.scope.entity_ids = [item].into_iter().collect();
            grant.scope.map_area = Some(MapCuboid::new(SITE, SITE)?);
        }
    }
    let prepared = adapter.prepare(&plan, &authority)?;
    adapter.commit(&plan, &prepared, &authority)?;
    assert_eq!(
        value(&adapter, LEDGER, effects::STOCK_BED_FIELD)?,
        &Value::U64(0)
    );
    Ok(())
}

#[test]
fn lost_reciprocal_custody_blocks_progress_and_refund_atomically() -> Result<()> {
    let (mut adapter, item) = manufactured("CONSTRUCT_BED")?;
    let (action, _) = dispatch(&mut adapter, placement(FurnitureKind::Bed, item, SITE), 206)?;
    let record = adapter
        .snapshot
        .graph
        .entities
        .get_mut(&item)
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "produced item missing",
            )
        })?;
    record.fields.remove(effects::FURNITURE_ALLOCATED_TO_FIELD);
    adapter.snapshot.refresh_hash();
    let before = adapter.snapshot().clone();
    assert!(adapter.advance_ticks(effects::BUILD_TICKS).is_err());
    assert_eq!(adapter.snapshot(), &before);
    adapter.request_cancel(action, CancelMode::StopFutureSteps, &context(&adapter))?;
    assert!(adapter.finalize_cancel(action, &context(&adapter)).is_err());
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(
        value(&adapter, LEDGER, effects::STOCK_BED_FIELD)?,
        &Value::U64(0)
    );
    Ok(())
}

#[test]
fn matching_completion_fields_do_not_verify_replacement_generations_or_broken_custody() -> Result<()>
{
    for mutation in 0..3 {
        let (mut adapter, item) = manufactured("CONSTRUCT_BED")?;
        let (action, building) =
            dispatch(&mut adapter, placement(FurnitureKind::Bed, item, SITE), 208)?;
        adapter.advance_ticks(effects::BUILD_TICKS)?;
        let receipt = adapter.action_receipt(action).cloned().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "pending receipt missing",
            )
        })?;
        assert_eq!(receipt.state, CommitState::AppliedAwaitingVerification);
        if mutation < 2 {
            let replaced = if mutation == 1 { building } else { item };
            let record = adapter
                .snapshot
                .graph
                .entities
                .get_mut(&replaced)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "placement record missing",
                    )
                })?;
            record.generation += 1;
        } else {
            // Preserve all original postcondition fields while another building
            // contradicts exclusive custody of this same item.
            let mut duplicate = adapter
                .snapshot
                .graph
                .entities
                .get(&building)
                .cloned()
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "placement record missing",
                    )
                })?;
            duplicate.id = EntityId::new(999);
            adapter
                .snapshot
                .graph
                .entities
                .insert(duplicate.id, duplicate);
        }
        adapter.snapshot.refresh_hash();
        let before = adapter.snapshot().clone();
        let step = adapter.action_step(action).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "placement step missing",
            )
        })?;
        assert!(
            PredicateEvidence::laboratory(adapter.snapshot())?
                .establishes(&Predicate::All(step.postconditions.clone()))?
        );
        assert_eq!(
            refusal(adapter.poll_action(action, &context(&adapter)))?.code,
            ErrorCode::PreconditionsFailed
        );
        assert_eq!(adapter.action_receipt(action), Some(&receipt));
        assert_eq!(adapter.snapshot(), &before);
        assert!(matches!(
            adapter.action_work_state(action)?,
            EffectWorkState::Unknown { .. }
        ));
    }
    Ok(())
}

#[test]
fn durable_restart_finishes_original_attachment_without_recreating_item_or_receipt() -> Result<()> {
    let (mut adapter, item) = manufactured("CONSTRUCT_BED")?;
    let (action, building) =
        dispatch(&mut adapter, placement(FurnitureKind::Bed, item, SITE), 207)?;
    adapter.advance_ticks(175)?;
    let interrupted = adapter.snapshot().clone();
    let directory = StateDirectory::new();
    {
        let mut store = durable::DurableLabStore::open(&directory.0)?;
        store.persist_head("furniture", &interrupted)?;
    }
    let store = durable::DurableLabStore::open(&directory.0)?;
    let head = store.head(interrupted.fortress_id).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "durable head missing",
        )
    })?;
    assert_eq!(head.anchor, interrupted.anchor());
    let reopened = store.load_snapshot(head.anchor.state_hash)?;
    assert_eq!(reopened, interrupted);
    let mut recovered = MemoryAdapter::recovered(reopened)?;
    assert!(recovered.snapshot().cursor.epoch > interrupted.cursor.epoch);
    assert!(recovered.action_receipt(action).is_none());
    assert_eq!(recovered.known_work_entity_ids().count(), 0);
    assert_eq!(
        value(&recovered, item, effects::FURNITURE_ALLOCATED_TO_FIELD)?,
        &Value::Entity(building)
    );
    recovered.advance_ticks(effects::BUILD_TICKS - 175)?;
    assert_eq!(
        value(&recovered, building, effects::CONSTRUCTION_STAGE_FIELD)?,
        &Value::Text(effects::STAGE_COMPLETE.to_owned())
    );
    assert_eq!(value(&recovered, item, "position")?, &Value::Coord(SITE));
    assert_eq!(
        value(&recovered, LEDGER, effects::STOCK_BED_FIELD)?,
        &Value::U64(0)
    );
    let installed = recovered.snapshot().graph.entities[&item].clone();
    recovered.advance_ticks(effects::BUILD_TICKS)?;
    assert_eq!(recovered.snapshot().graph.entities[&item], installed);
    assert_eq!(
        recovered
            .snapshot()
            .graph
            .entities
            .values()
            .filter(|record| record.kind == EntityKind::Item)
            .count(),
        1
    );
    Ok(())
}
