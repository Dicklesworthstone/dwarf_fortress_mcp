#![forbid(unsafe_code)]

//! A canonical-name alias must never remove an entity from physical progress
//! or publish a partial time advance, even with committed work in flight.

use std::collections::BTreeMap;

use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId, Result,
    RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::effects::{self, AMOUNT_REMAINING_FIELD, WORK_ORDER_TICKS_PER_UNIT};
use dfmcp_intent::{Action, Constraint, Intent, RequestedAction, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactSource, Predicate, Value, WorldGraph, WorldSnapshot,
};

const SOURCE_ENTITY: EntityId = EntityId::new(u64::MAX);

fn world(kind: EntityKind) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        SOURCE_ENTITY,
        EntityRecord {
            id: SOURCE_ENTITY,
            generation: 1,
            revision: 1,
            kind,
            label: "source citizen".to_owned(),
            fields: BTreeMap::from([(
                "alive".to_owned(),
                Fact::known(
                    Value::Bool(true),
                    GameTick(1_000),
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    )
}

fn context(adapter: &MemoryAdapter, request: u128) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(request),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ConfigureProduction, RiskTier::Reversible),
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

fn commit_order(adapter: &mut MemoryAdapter) -> Result<(ActionId, EntityId)> {
    let intent = Intent {
        id: IntentId::new(1),
        anchor: adapter.snapshot().anchor(),
        summary: "complete one reference work unit".to_owned(),
        terminal_condition: Predicate::Paused(true),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::CreateWorkOrder {
                name: "barrel".to_owned(),
                job_token: "MAKE_BARREL".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter, 1),
    )?;
    let prepared = adapter.prepare(&plan, &context(adapter, 2))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter, 3))?;
    Ok((
        receipt.actions[0].action_id,
        effects::created_entity_id(&plan.steps[0].idempotency_key, 0),
    ))
}

#[test]
fn canonical_source_alias_refusal_preserves_clock_work_receipts_and_transcript() -> Result<()> {
    let source = world(EntityKind::Unit);
    let aliased = world(EntityKind::Other("unit".to_owned()));
    assert_eq!(source.canonical_bytes(), aliased.canonical_bytes());
    assert_eq!(source.anchor(), aliased.anchor());
    assert_eq!(
        WorldSnapshot::from_canonical_bytes(&aliased.canonical_bytes())?,
        source
    );

    let mut adapter = MemoryAdapter::new(aliased);
    let (action, order) = commit_order(&mut adapter)?;
    let before = adapter.snapshot().clone();
    let transcript = adapter.transcript().clone();
    let action_receipt = adapter.action_receipt(action).cloned();
    let plan_receipt = adapter.action_plan_receipt(action).cloned();
    assert!(action_receipt.is_some());
    assert!(plan_receipt.is_some());
    assert!(order < SOURCE_ENTITY);
    assert_eq!(
        before.graph.entities[&order].fields[AMOUNT_REMAINING_FIELD].value,
        Value::U64(1)
    );

    for _ in 0..2 {
        let refused = adapter.advance_ticks(WORK_ORDER_TICKS_PER_UNIT).err();
        assert_eq!(
            refused.as_ref().map(|error| error.code),
            Some(ErrorCode::PreconditionsFailed)
        );
        assert!(refused.is_some_and(|error| error.message.contains("canonical entity kind")));
        // Snapshot equality includes the game tick, cursor, hash, work records
        // and every fact. Neither the failed advance nor a retry publishes.
        assert_eq!(adapter.snapshot(), &before);
        assert_eq!(adapter.transcript(), &transcript);
        assert_eq!(adapter.action_receipt(action), action_receipt.as_ref());
        assert_eq!(adapter.action_plan_receipt(action), plan_receipt.as_ref());
    }
    Ok(())
}

#[test]
fn canonical_named_units_and_unreserved_other_kinds_keep_progressing() -> Result<()> {
    for kind in [EntityKind::Unit, EntityKind::Other("mod.unit".to_owned())] {
        let mut adapter = MemoryAdapter::new(world(kind));
        let (_, order) = commit_order(&mut adapter)?;
        adapter.advance_ticks(WORK_ORDER_TICKS_PER_UNIT)?;
        assert_eq!(adapter.snapshot().tick, GameTick(1_050));
        assert_eq!(
            adapter.snapshot().graph.entities[&order].fields[AMOUNT_REMAINING_FIELD].value,
            Value::U64(0)
        );
    }
    Ok(())
}
