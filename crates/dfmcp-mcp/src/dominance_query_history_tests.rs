//! Exact retained-source authority, complete input custody, and output bounds.
//! Owning bead: df-lab-directed-bottlenecks-r3k.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{
    EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph,
};
use std::collections::{BTreeMap, BTreeSet};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in 1..=5 {
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: id,
                kind: EntityKind::Building,
                label: format!("Junction {id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (id, from, to) in [(1, 1, 2), (2, 1, 3), (3, 2, 4), (4, 3, 4)] {
        graph.edges.insert(
            EdgeId::new(id),
            EdgeRecord {
                id: EdgeId::new(id),
                revision: 1,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::new(),
            },
        );
    }
    let snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(987010),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 128,
            max_bytes: 1024 * 1024,
            max_output_tokens: 8192,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    (snapshot, context)
}

fn request() -> Value {
    json!({"schema":"dfmcp.query/1", "query":{
        "kind":"dominators", "root":"1", "root_generation":1,
        "edge_kinds":["requires"], "target":"4", "target_generation":1
    }})
}

#[test]
fn retained_dominators_authorize_at_the_actual_current_tick_after_clock_reset() -> TestResult {
    let (source, mut current) = fixture();
    let original = source.canonical_bytes();
    let mut restored = source.clone();
    restored.tick = GameTick(2);
    restored.cursor.epoch += 1;
    restored
        .graph
        .entities
        .get_mut(&EntityId::new(4))
        .ok_or("target missing")?
        .generation = 2;
    restored.refresh_hash();
    current.anchor = restored.anchor();
    current.grants[0].expires_at_tick = Some(GameTick(5));
    let mut input = request();
    input["expected_anchor"] = anchor_json(source.anchor());
    let result = execute_laboratory_source(&source, &current, &input, 60_000)?;
    assert_eq!(result["anchor"], anchor_json(source.anchor()));
    assert_eq!(result["witness"]["anchor"], result["anchor"]);
    assert_eq!(result["dominator_chain"], json!(["1", "4"]));
    assert_eq!(result["vertices"][1]["generation"], 1);
    assert_eq!(source.canonical_bytes(), original);
    assert!(matches!(
        execute(&source, &current, &input),
        Err(error) if error.code == ErrorCode::StaleAnchor
    ));
    input["expected_anchor"] = anchor_json(current.anchor);
    assert!(matches!(
        execute_laboratory_source(&source, &current, &input, 60_000),
        Err(error) if error.code == ErrorCode::StaleAnchor
    ));
    input["expected_anchor"] = anchor_json(source.anchor());
    input["query"]["target_generation"] = json!(2);
    assert!(matches!(
        execute_laboratory_source(&source, &current, &input, 60_000),
        Err(error) if error.code == ErrorCode::Conflict
    ));
    current.grants[0].expires_at_tick = Some(GameTick(1));
    assert!(matches!(
        execute_laboratory_source(&source, &current, &request(), 60_000),
        Err(error) if error.code == ErrorCode::CapabilityDenied
    ));
    Ok(())
}

#[test]
fn retained_dominators_do_not_revive_expired_scoped_or_foreign_authority() -> TestResult {
    let (source, context) = fixture();
    let mut current = context.clone();
    current.anchor.tick = GameTick(30);
    current.grants[0].expires_at_tick = Some(GameTick(20));
    assert!(matches!(
        execute_laboratory_source(&source, &current, &request(), 60_000),
        Err(error) if error.code == ErrorCode::CapabilityDenied
    ));
    for change in 0..4 {
        let mut restricted = context.clone();
        match change {
            0 => restricted.grants.clear(),
            1 => restricted.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            2 => restricted.grants[0].remaining_uses = Some(0),
            _ => restricted.cancellation_requested = true,
        }
        assert!(execute_laboratory_source(&source, &restricted, &request(), 60_000).is_err());
    }
    let mut foreign = context.clone();
    foreign.anchor.fortress_id = FortressId::new(2);
    foreign.grants[0].scope.fortress_id = Some(FortressId::new(2));
    assert!(matches!(
        execute_laboratory_source(&source, &foreign, &request(), 60_000),
        Err(error) if error.code == ErrorCode::StaleAnchor
    ));
    assert!(matches!(
        execute_laboratory_source(&source, &context, &request(), 0),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    Ok(())
}

#[test]
fn dominance_input_and_output_allowances_remain_independent_for_focused_history() -> TestResult {
    let (mut source, mut context) = fixture();
    source
        .graph
        .entities
        .get_mut(&EntityId::new(5))
        .ok_or("unreachable vertex missing")?
        .fields
        .insert(
            "unused_payload".to_owned(),
            Fact::known(
                WorldValue::Text("x".repeat(32_768)),
                source.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    source.refresh_hash();
    context.anchor = source.anchor();
    let output = execute_with_output_budget(&source, &context, &request(), 8192)?;
    assert_eq!(output["returned"], 2);
    assert_eq!(output["reachable_count"], 4);
    assert_eq!(output["unreachable_count"], 1);
    let output_size = serde_json::to_vec(&output)?.len() as u64;
    let source_size = source.canonical_bytes().len() as u64;
    assert!(source_size > output_size);
    assert_eq!(
        execute_with_output_budget(&source, &context, &request(), output_size)?,
        output
    );
    assert!(matches!(
        execute_with_output_budget(&source, &context, &request(), output_size - 1),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    context.budget.max_bytes = source_size - 1;
    assert!(matches!(
        execute_laboratory_source(&source, &context, &request(), 60_000),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    context.budget.max_bytes = source_size;
    assert_eq!(
        execute_with_output_budget(&source, &context, &request(), output_size)?,
        output
    );
    context.budget.max_entities = 8;
    assert!(matches!(
        execute_laboratory_source(&source, &context, &request(), 60_000),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    Ok(())
}

#[test]
fn unselected_overdeep_source_values_refuse_before_canonical_graph_analysis() -> TestResult {
    let (mut source, context) = fixture();
    let mut nested = WorldValue::Bool(true);
    for _ in 0..65 {
        nested = WorldValue::List(vec![nested]);
    }
    source
        .graph
        .entities
        .get_mut(&EntityId::new(5))
        .ok_or("unreachable vertex missing")?
        .fields
        .insert(
            "unselected_nested_payload".to_owned(),
            Fact::known(
                nested,
                source.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    // Deliberately retain the old hash. Structural admission must reject the
    // oversized source before any canonical serialization or hash comparison.
    assert!(matches!(
        execute(&source, &context, &request()),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    Ok(())
}
