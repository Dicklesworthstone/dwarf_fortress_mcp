use super::*;
use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{EdgeRecord, EntityKind, EntityRecord, WorldGraph};

fn fixture(size: u64, edges: &[(u64, u64)]) -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in 1..=size {
        graph.entities.insert(EntityId::new(id), EntityRecord {
            id: EntityId::new(id), generation: 1, revision: 1,
            kind: EntityKind::Building, label: format!("node {id}"), fields: BTreeMap::new(),
        });
    }
    for (offset, &(from, to)) in edges.iter().enumerate() {
        let id = EdgeId::new(offset as u128 + 1);
        graph.edges.insert(id, EdgeRecord {
            id, revision: 1, kind: EdgeKind::Requires, from: EntityId::new(from),
            to: EntityId::new(to), fields: BTreeMap::new(),
        });
    }
    let snapshot = WorldSnapshot::new(FortressId::new(1), GameTick(10), ObservationCursor::ORIGIN, true, graph);
    let context = OperationContext {
        session_id: SessionId::new(987001), request_id: RequestId::new(1), anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 2000, max_bytes: 262144, max_output_tokens: 65536,
            max_wall_millis: 60000, ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope { fortress_id: Some(snapshot.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    (snapshot, context)
}

fn diamond() -> (WorldSnapshot, OperationContext) {
    fixture(7, &[(1, 2), (1, 3), (2, 4), (3, 4), (4, 5), (6, 7), (7, 6), (7, 4)])
}

fn request(target: Option<u64>) -> Value {
    let mut input = json!({"schema":"dfmcp.query/1","query":{
        "kind":"dominators","root":"1","root_generation":1,"edge_kinds":["requires"]
    }});
    if let Some(target) = target {
        input["query"]["target"] = json!(target.to_string());
        input["query"]["target_generation"] = json!(1);
    }
    input
}

#[test]
fn semantic_dispatch_returns_the_complete_witnessed_tree() -> Result<()> {
    let (snapshot, context) = diamond();
    let input = request(None);
    let result = super::super::execute(&snapshot, &context, &input)?;
    assert_eq!(result["kind"], "dominators");
    assert_eq!(result["view"], "dominator_tree");
    assert_eq!(result["reachable_count"], 5);
    assert_eq!(result["unreachable"], json!(["6", "7"]));
    assert_eq!(result["vertices"][3]["entity_id"], "4");
    assert_eq!(result["vertices"][3]["immediate_dominator"], "1");
    assert_eq!(result["vertices"][3]["dominated_descendants"], 1);
    assert_eq!(result["witness"]["anchor"], anchor_json(snapshot.anchor()));
    assert_eq!(result["policy"], DOMINANCE_POLICY);
    assert_eq!(result["analysis_complete"], true);
    assert_eq!(result["mutation_dispatched"], false);
    assert_eq!(result, super::super::execute(&snapshot, &context, &input)?);
    Ok(())
}

#[test]
fn target_focus_is_not_a_partial_analysis_or_a_traversable_route() -> Result<()> {
    let (snapshot, context) = diamond();
    let tree = execute(&snapshot, &context, &request(None))?;
    let focused = execute(&snapshot, &context, &request(Some(5)))?;
    assert_eq!(focused["view"], "target_chain");
    assert_eq!(focused["dominator_chain"], json!(["1", "4", "5"]));
    assert_eq!(focused["returned"], 3);
    assert_eq!(focused["target_status"], "reachable");
    assert_eq!(focused["reachable_count"], tree["reachable_count"]);
    assert_eq!(focused["unreachable_count"], 2);
    assert!(focused["unreachable"].is_null());
    assert_eq!(focused["witness"], tree["witness"]);
    assert_eq!(focused["truncated"], false);
    assert_eq!(focused["coverage"]["walkability_proven"], false);
    assert!(focused.get("path").is_none());
    // There is no 1 -> 4 edge, despite their adjacency in the dominator chain.
    assert!(!snapshot.graph.edges.values().any(|e| e.from == EntityId::new(1) && e.to == EntityId::new(4)));
    let root = execute(&snapshot, &context, &request(Some(1)))?;
    assert_eq!(root["dominator_chain"], json!(["1"]));
    Ok(())
}

#[test]
fn unreachable_is_distinct_from_missing_and_never_world_absence() -> Result<()> {
    let (snapshot, context) = diamond();
    let result = execute(&snapshot, &context, &request(Some(6)))?;
    assert_eq!(result["target_status"], "unreachable_in_projection");
    assert_eq!(result["vertices"], json!([]));
    assert!(result["dominator_chain"].is_null());
    assert_eq!(result["analysis_complete"], true);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert!(matches!(execute(&snapshot, &context, &request(Some(99))), Err(e) if e.code == ErrorCode::InvalidRequest));
    Ok(())
}

#[test]
fn incoming_and_selected_relation_semantics_are_explicit() -> Result<()> {
    let (snapshot, context) = fixture(4, &[(1, 2), (2, 3), (3, 4)]);
    let mut input = request(Some(1));
    input["query"]["root"] = json!("4");
    input["query"]["direction"] = json!("incoming");
    let reverse = execute(&snapshot, &context, &input)?;
    assert_eq!(reverse["dominator_chain"], json!(["4", "3", "2", "1"]));
    input["query"]["edge_kinds"] = json!(["uses"]);
    let absent_relation = execute(&snapshot, &context, &input)?;
    assert_eq!(absent_relation["target_status"], "unreachable_in_projection");
    input["query"]["edge_kinds"] = json!(["uses", "requires", "requires"]);
    let mixed = execute(&snapshot, &context, &input)?;
    assert_eq!(mixed["edge_kinds"], json!(["requires", "uses"]));
    input["query"]["edge_kinds"] = json!(["requires", "uses"]);
    assert_eq!(mixed, execute(&snapshot, &context, &input)?);
    Ok(())
}

#[test]
fn endpoint_generations_and_exact_anchors_fence_every_view() -> Result<()> {
    let (snapshot, context) = diamond();
    for target in [None, Some(5), Some(6)] {
        for root_stale in [true, false] {
            let mut input = request(target);
            if root_stale {
                input["query"]["root_generation"] = json!(2);
            } else if target.is_some() {
                input["query"]["target_generation"] = json!(2);
            } else { continue; }
            assert!(matches!(execute(&snapshot, &context, &input), Err(e) if e.code == ErrorCode::Conflict));
        }
    }
    let mut input = request(Some(5));
    input["expected_anchor"] = anchor_json(context.anchor);
    execute(&snapshot, &context, &input)?;
    input["expected_anchor"]["game_tick"] = json!(11);
    assert!(matches!(execute(&snapshot, &context, &input), Err(e) if e.code == ErrorCode::StaleAnchor));
    let mut wrong = context.clone();
    wrong.anchor.tick = GameTick(11);
    assert!(matches!(execute(&snapshot, &wrong, &request(None)), Err(e) if e.code == ErrorCode::StaleAnchor));
    let mut corrupted = snapshot.clone();
    corrupted.paused = false;
    assert!(matches!(execute(&corrupted, &context, &request(None)), Err(e) if e.code == ErrorCode::InternalInvariantViolation));
    Ok(())
}

#[test]
fn authority_is_checked_before_parsing_or_graph_expansion() {
    let (snapshot, context) = diamond();
    for change in 0..6 {
        let mut denied = context.clone();
        match change {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            3 => denied.grants[0].expires_at_tick = Some(GameTick(0)),
            4 => denied.grants[0].remaining_uses = Some(1),
            _ => denied.grants[0].scope.fortress_id = Some(FortressId::new(99)),
        }
        assert!(super::super::execute(&snapshot, &denied, &request(Some(5))).is_err());
    }
    let mut no_authority = context;
    no_authority.grants.clear();
    let error = execute(&snapshot, &no_authority, &Value::Null).unwrap_err();
    assert_ne!(error.code, ErrorCode::InvalidRequest);
}

#[test]
fn typed_selector_and_shape_bounds_reject_malformed_requests() {
    let (snapshot, context) = diamond();
    for (field, value) in [
        ("root", json!(1)), ("root", json!("01")), ("root", json!("18446744073709551616")),
        ("root_generation", json!(-1)), ("root_generation", json!(4294967296u64)),
        ("edge_kinds", json!([])), ("edge_kinds", json!(["not_an_edge"])),
        ("edge_kinds", json!(["custom:"])), ("edge_kinds", json!(["custom:a\u{0000}b"])),
        ("edge_kinds", json!(vec!["requires"; 33])), ("direction", json!("undirected")),
        ("extra", json!(true)), ("target", json!("5")), ("target_generation", json!(1)),
        ("target", Value::Null), ("target_generation", Value::Null),
    ] {
        let mut input = request(None);
        input["query"][field] = value;
        assert!(execute(&snapshot, &context, &input).is_err(), "accepted {input}");
    }
    let mut missing = request(None);
    missing["query"].as_object_mut().unwrap().remove("root_generation");
    assert!(execute(&snapshot, &context, &missing).is_err());
    let mut huge = request(None);
    huge["query"]["edge_kinds"] = json!(["x".repeat(16384)]);
    assert!(matches!(execute(&snapshot, &context, &huge), Err(e) if e.code == ErrorCode::BudgetExceeded));
    let mut deep = json!(false);
    for _ in 0..10 { deep = json!([deep]); }
    huge["query"]["extra"] = deep;
    assert!(matches!(execute(&snapshot, &context, &huge), Err(e) if e.code == ErrorCode::BudgetExceeded));
}

#[test]
fn target_focus_fits_when_full_tree_cannot_and_bounds_never_truncate() -> Result<()> {
    let (snapshot, mut context) = fixture(100, &(2..=100).map(|id| (1, id)).collect::<Vec<_>>());
    context.budget.max_bytes = 4000;
    assert!(matches!(execute(&snapshot, &context, &request(None)), Err(e) if e.code == ErrorCode::BudgetExceeded));
    let focused = execute(&snapshot, &context, &request(Some(2)))?;
    assert_eq!(focused["reachable_count"], 100);
    assert_eq!(focused["returned"], 2);
    context.budget.max_bytes = serde_json::to_vec(&focused).unwrap().len() as u64;
    assert_eq!(focused, execute(&snapshot, &context, &request(Some(2)))?);
    context.budget.max_bytes -= 1;
    assert!(matches!(execute(&snapshot, &context, &request(Some(2))), Err(e) if e.code == ErrorCode::BudgetExceeded));
    context.budget.max_bytes = 262144;
    context.budget.max_output_tokens = 1;
    assert!(matches!(execute(&snapshot, &context, &request(None)), Err(e) if e.code == ErrorCode::BudgetExceeded));
    context.budget.max_output_tokens = 65536;
    context.budget.max_entities = 99;
    assert!(matches!(execute(&snapshot, &context, &request(Some(2))), Err(e) if e.code == ErrorCode::BudgetExceeded));
    context.budget.max_entities = 2000;
    context.budget.max_wall_millis = 0;
    assert!(matches!(execute(&snapshot, &context, &request(Some(2))), Err(e) if e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn publisher_failure_is_atomic_and_query_does_not_allocate_work() -> Result<()> {
    let (snapshot, context) = diamond();
    let before = snapshot.anchor();
    let input = request(Some(5));
    let mut calls = 0;
    let error = super::super::execute_with_publisher(&snapshot, &context, &input, |_| {
        calls += 1;
        Err(bounded("injected complete-turn rendering failure"))
    });
    assert!(matches!(error, Err(e) if e.code == ErrorCode::BudgetExceeded));
    assert_eq!(calls, 1);
    assert_eq!(snapshot.anchor(), before);
    let rendered = super::super::execute_with_publisher(&snapshot, &context, &input, |value| Ok(value.to_string()))?;
    let parsed: Value = serde_json::from_str(&rendered).unwrap();
    assert_eq!(parsed["kind"], "dominators");
    assert_eq!(parsed["mutation_dispatched"], false);
    assert_eq!(snapshot.anchor(), before);
    let mut other = context.clone();
    other.session_id = SessionId::new(987002);
    let first = execute(&snapshot, &context, &input)?;
    let second = execute(&snapshot, &other, &input)?;
    assert_eq!(first["vertices"], second["vertices"]);
    assert_ne!(first["witness"]["decision_digest"], second["witness"]["decision_digest"]);
    Ok(())
}

#[test]
fn runtime_schema_composition_advertises_the_new_query_variant() -> Result<()> {
    let base: Value = serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json")).unwrap();
    let schema = super::super::extend_watch_count_schema(base)?;
    let variants = schema["$defs"]["query"]["oneOf"].as_array().unwrap();
    let added: Vec<_> = variants.iter().filter(|variant| variant["properties"]["kind"]["const"] == "dominators").collect();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0]["dependentRequired"]["target"], json!(["target_generation"]));
    assert_eq!(added[0]["properties"]["edge_kinds"]["minItems"], 1);
    assert!(extend_schema(json!({})).is_err());
    Ok(())
}
