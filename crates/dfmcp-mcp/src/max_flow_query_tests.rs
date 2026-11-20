use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{
    EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue,
    WorldGraph,
};
use std::collections::{BTreeMap, BTreeSet};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn fact(value: WorldValue, field: &str) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField(field.to_owned()),
        Digest32::of_bytes(b"flow-source"),
    )
}

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
                label: format!("Network {id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (id, from, to, capacity) in [
        (1, 1, 2, 3),
        (2, 1, 3, 2),
        (3, 2, 3, 1),
        (4, 2, 4, 2),
        (5, 3, 4, 3),
        (6, 5, 5, 7),
    ] {
        graph.edges.insert(
            EdgeId::new(id),
            EdgeRecord {
                id: EdgeId::new(id),
                revision: 1,
                kind: EdgeKind::Uses,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::from([
                    (
                        "capacity".to_owned(),
                        fact(WorldValue::U64(capacity), "edge.capacity"),
                    ),
                    (
                        "unit".to_owned(),
                        fact(WorldValue::Text("items/tick".to_owned()), "edge.unit"),
                    ),
                ]),
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
        session_id: SessionId::new(1),
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
    json!({"schema":"dfmcp.query/1","query":{"kind":"max_flow","edge_kinds":["uses"],
        "capacity_field":"capacity","unit_field":"unit",
        "source":{"entity_id":"1","generation":1},
        "sink":{"entity_id":"4","generation":1}}})
}

#[test]
fn flow_executor_returns_complete_conserved_network_and_equal_capacity_cut() -> TestResult {
    let (snapshot, context) = fixture();
    let before = snapshot.canonical_bytes();
    let result = execute(&snapshot, &context, &request())?;
    assert_eq!(result["value"], 5);
    assert_eq!(result["unit"], "items/tick");
    assert_eq!(result["returned_edges"], 6);
    assert_eq!(result["minimum_cut"]["capacity"], 5);
    assert_eq!(
        result["minimum_cut"]["edge_ids"],
        json!([EdgeId::new(1).to_string(), EdgeId::new(2).to_string()])
    );
    assert_eq!(
        result["minimum_cut"]["source_side"],
        json!([
        {"entity_id":"1","generation":1,"revision":1}])
    );
    let edges = result["edges"].as_array().ok_or("edges absent")?;
    for (index, expected) in [3, 2, 1, 2, 3, 0].iter().enumerate() {
        assert_eq!(edges[index]["flow"], *expected);
        assert_eq!(edges[index]["unit_fact"]["value"]["value"], "items/tick");
        assert_eq!(edges[index]["capacity_fact"]["presence"], "known");
        assert_eq!(
            edges[index]["capacity_fact"]["source_digest"],
            Digest32::of_bytes(b"flow-source").to_string()
        );
        assert_eq!(edges[index]["from"]["generation"], 1);
        assert_eq!(edges[index]["to"]["generation"], 1);
    }
    assert_eq!(result["model"]["exact_within_projection"], true);
    assert_eq!(result["model"]["epistemic_state"], "inferred");
    assert_eq!(result["model"]["conservation_verified"], true);
    assert_eq!(result["model"]["flow_cut_equality_verified"], true);
    assert_eq!(result["model"]["actual_throughput_proven"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["coverage"]["mutation_authorized"], false);
    assert_eq!(result["mutation_dispatched"], false);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["witness"]["anchor"], anchor_json(snapshot.anchor()));
    assert_eq!(execute(&snapshot, &context, &request())?, result);
    assert_eq!(snapshot.canonical_bytes(), before);
    Ok(())
}

#[test]
fn flow_canonicalizes_kind_sets_but_preserves_direction_and_source_scope() -> TestResult {
    let (snapshot, context) = fixture();
    let mut first = request();
    first["query"]["edge_kinds"] = json!(["requires", "uses", "uses"]);
    let mut reordered = first.clone();
    reordered["query"]["edge_kinds"] = json!(["uses", "requires"]);
    let result = execute(&snapshot, &context, &first)?;
    assert_eq!(execute(&snapshot, &context, &reordered)?, result);
    let mut reversed = request();
    reversed["query"]["source"] = json!({"entity_id":"4","generation":1});
    reversed["query"]["sink"] = json!({"entity_id":"1","generation":1});
    let reverse = execute(&snapshot, &context, &reversed)?;
    assert_eq!(reverse["value"], 0);
    assert_eq!(reverse["unit"], "items/tick");
    assert_ne!(
        result["witness"]["decision_digest"],
        reverse["witness"]["decision_digest"]
    );
    let mut other = context.clone();
    other.session_id = SessionId::new(2);
    assert_ne!(
        execute(&snapshot, &other, &first)?["witness"]["authorization_scope_digest"],
        result["witness"]["authorization_scope_digest"]
    );
    Ok(())
}

#[test]
fn empty_selected_network_reports_zero_and_unknown_units_without_absence_claim() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["edge_kinds"] = json!(["produces"]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["value"], 0);
    assert_eq!(result["minimum_cut"]["capacity"], 0);
    assert_eq!(result["returned_edges"], 0);
    assert!(result["unit"].is_null());
    assert_eq!(result["model"]["unit_established"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(
        result["minimum_cut"]["source_side"]
            .as_array()
            .ok_or("partition absent")?
            .len(),
        1
    );
    assert_eq!(
        result["minimum_cut"]["sink_side"]
            .as_array()
            .ok_or("partition absent")?
            .len(),
        4
    );
    Ok(())
}

#[test]
fn flow_rejects_missing_unavailable_or_mixed_units_even_in_unreachable_edges() -> TestResult {
    for variant in 0..9 {
        let (mut snapshot, mut context) = fixture();
        let edge = snapshot
            .graph
            .edges
            .get_mut(&EdgeId::new(6))
            .ok_or("edge absent")?;
        match variant {
            0 => {
                edge.fields.remove("capacity");
            }
            1 => {
                edge.fields.remove("unit");
            }
            2 => {
                edge.fields
                    .get_mut("capacity")
                    .ok_or("capacity absent")?
                    .observed_at = GameTick(11)
            }
            3 => {
                edge.fields
                    .get_mut("unit")
                    .ok_or("unit absent")?
                    .observed_at = GameTick(11)
            }
            4 => {
                edge.fields
                    .get_mut("capacity")
                    .ok_or("capacity absent")?
                    .source_digest = Digest32::ZERO
            }
            5 => {
                edge.fields.get_mut("unit").ok_or("unit absent")?.source =
                    FactSource::AgentAssertion("guess".to_owned())
            }
            6 => {
                edge.fields.insert(
                    "unit".to_owned(),
                    fact(WorldValue::Text("power".to_owned()), "edge.unit"),
                );
            }
            7 => {
                edge.fields
                    .get_mut("capacity")
                    .ok_or("capacity absent")?
                    .presence = Some(FactPresence::Unknown("unobserved".to_owned()))
            }
            _ => {
                edge.fields.insert(
                    "capacity".to_owned(),
                    fact(WorldValue::I64(-1), "edge.capacity"),
                );
            }
        }
        snapshot.refresh_hash();
        context.anchor = snapshot.anchor();
        assert!(
            execute(&snapshot, &context, &request()).is_err(),
            "accepted variant {variant}"
        );
    }
    Ok(())
}

#[test]
fn flow_fences_both_endpoint_generations_and_checks_complete_expected_anchor() -> TestResult {
    let (snapshot, context) = fixture();
    for endpoint in ["source", "sink"] {
        let mut input = request();
        input["query"][endpoint]["generation"] = json!(2);
        assert_eq!(
            execute(&snapshot, &context, &input).map_err(|e| e.code),
            Err(ErrorCode::Conflict)
        );
        input["query"][endpoint] = json!({"entity_id":"99","generation":1});
        assert_eq!(
            execute(&snapshot, &context, &input).map_err(|e| e.code),
            Err(ErrorCode::InvalidRequest)
        );
    }
    let mut input = request();
    input["expected_anchor"] = anchor_json(snapshot.anchor());
    assert_eq!(execute(&snapshot, &context, &input)?["value"], 5);
    input["expected_anchor"]["game_tick"] = json!(9);
    assert_eq!(
        execute(&snapshot, &context, &input).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn flow_requires_current_query_authority_and_a_live_foreground_allowance() -> TestResult {
    let (snapshot, context) = fixture();
    for variant in 0..7 {
        let mut denied = context.clone();
        match variant {
            0 => denied.grants.clear(),
            1 => {
                denied.grants[0].scope.entity_ids =
                    BTreeSet::from([EntityId::new(1), EntityId::new(4)])
            }
            2 => denied.grants[0].expires_at_tick = Some(GameTick(9)),
            3 => denied.grants[0].remaining_uses = Some(0),
            4 => denied.cancellation_requested = true,
            5 => denied.budget.max_wall_millis = 0,
            _ => denied.anchor.cursor.sequence += 1,
        }
        assert!(
            execute(&snapshot, &denied, &request()).is_err(),
            "accepted variant {variant}"
        );
    }
    assert_eq!(
        execute_laboratory_source(&snapshot, &context, &request(), 0).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn retained_flow_keeps_current_authority_after_restore_and_selected_source_time() -> TestResult {
    let (snapshot, mut current) = fixture();
    current.anchor.tick = GameTick(2);
    current.anchor.cursor.epoch = 1;
    current.grants[0].expires_at_tick = Some(GameTick(5));
    assert_eq!(
        execute(&snapshot, &current, &request()).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    let result = execute_laboratory_source(&snapshot, &current, &request(), 60_000)?;
    assert_eq!(result["anchor"], anchor_json(snapshot.anchor()));
    assert_eq!(result["value"], 5);
    current.grants[0].expires_at_tick = Some(GameTick(1));
    assert_eq!(
        execute_laboratory_source(&snapshot, &current, &request(), 60_000).map_err(|e| e.code),
        Err(ErrorCode::CapabilityDenied)
    );
    current.grants[0].expires_at_tick = None;
    current.anchor.fortress_id = FortressId::new(2);
    current.grants[0].scope.fortress_id = Some(FortressId::new(2));
    assert_eq!(
        execute_laboratory_source(&snapshot, &current, &request(), 60_000).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn flow_complete_source_and_output_budgets_are_independent_and_atomic() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    let result = execute(&snapshot, &context, &request())?;
    let exact = serde_json::to_vec(&result)?.len() as u64;
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &request(), exact)?,
        result
    );
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &request(), exact - 1).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_entities = 10; // five vertices and six edges
    assert_eq!(
        execute(&snapshot, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_entities = 128;
    context.budget.max_output_tokens = 10;
    let refused = execute(&snapshot, &context, &request())
        .err()
        .ok_or("tiny output allowance retained a complete network")?;
    assert_eq!(refused.code, ErrorCode::BudgetExceeded);
    // Refuse as soon as the next child would exceed the response allowance,
    // before allocating every edge/partition row and the complete JSON tree.
    assert!(refused.message.contains("output materialization"));
    context.budget.max_output_tokens = 8192;
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(5))
        .ok_or("unrelated entity absent")?
        .fields
        .insert(
            "large_unselected".to_owned(),
            fact(WorldValue::Bytes(vec![0; 65_536]), "unrelated.bytes"),
        );
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &request(), 16_384)?["value"],
        5
    );
    context.budget.max_bytes = snapshot.canonical_bytes().len() as u64 - 1;
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &request(), 16_384).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn flow_exact_u64_capacity_never_wraps_total_or_invents_infinity() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot.graph.edges.retain(|id, _| *id == EdgeId::new(1));
    let edge = snapshot
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge absent")?;
    edge.to = EntityId::new(4);
    edge.fields.insert(
        "capacity".to_owned(),
        fact(WorldValue::U64(u64::MAX), "edge.capacity"),
    );
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
    let result = execute(&snapshot, &context, &request())?;
    assert_eq!(result["value"], json!(u64::MAX));
    assert_eq!(result["minimum_cut"]["capacity"], json!(u64::MAX));
    let mut second = snapshot
        .graph
        .edges
        .get(&EdgeId::new(1))
        .ok_or("edge absent")?
        .clone();
    second.id = EdgeId::new(2);
    snapshot.graph.edges.insert(second.id, second);
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
    assert_eq!(
        execute(&snapshot, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn flow_closed_grammar_rejects_ambiguous_handles_and_effect_or_capacity_injection() -> TestResult {
    let (snapshot, context) = fixture();
    for (key, value) in [
        ("edge_kinds", json!([])),
        ("edge_kinds", json!(["unknown"])),
        ("edge_kinds", json!(vec!["uses"; 33])),
        ("capacity_field", json!("")),
        ("capacity_field", json!(" ")),
        ("capacity_field", json!("x\ny")),
        ("capacity_field", json!("x".repeat(129))),
        ("unit_field", json!("capacity")),
        ("unit_field", json!(null)),
        ("source", json!({"entity_id":"01","generation":1})),
        (
            "source",
            json!({"entity_id":"1","generation":1,"extra":true}),
        ),
        ("source", json!({"entity_id":"1"})),
        ("sink", json!({"entity_id":"1","generation":1})),
        ("sink", json!({"entity_id":"0","generation":1})),
        ("direction", json!("undirected")),
        ("capacity", json!(100)),
        ("unit", json!("items/tick")),
        ("commit", json!(true)),
    ] {
        let mut input = request();
        input["query"][key] = value;
        assert_eq!(
            execute(&snapshot, &context, &input).map_err(|e| e.code),
            Err(ErrorCode::InvalidRequest),
            "accepted {key}: {input}"
        );
    }
    for raw in [
        r#"{"schema":"dfmcp.query/1","query":{"kind":"max_flow","edge_kinds":["uses"],"capacity_field":"capacity","capacity_field":"other","unit_field":"unit","source":{"entity_id":"1","generation":1},"sink":{"entity_id":"4","generation":1}}}"#,
        r#"{"schema":"dfmcp.query/1","query":{"kind":"max_flow","edge_kinds":["uses"],"capacity_field":"capacity","unit_field":"unit","source":{"entity_id":"1","generation":1,"generation":2},"sink":{"entity_id":"4","generation":1}}}"#,
    ] {
        assert!(serde_json::from_str::<Envelope>(raw).is_err());
    }
    Ok(())
}

#[test]
fn flow_schema_extension_preserves_existing_queries_and_requires_units() -> TestResult {
    let base: Value = serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))?;
    let old = base["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("variants absent")?;
    let extended = extend_schema(base.clone())?;
    let new = extended["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("variants absent")?;
    assert_eq!(&new[..old.len()], old.as_slice());
    assert_eq!(new.len(), old.len() + 1);
    let extension = new.last().ok_or("extension absent")?;
    assert_eq!(extension["properties"]["kind"]["const"], "max_flow");
    assert!(
        extension["required"]
            .as_array()
            .ok_or("requirements absent")?
            .contains(&json!("unit_field"))
    );
    assert_eq!(extension["additionalProperties"], false);
    Ok(())
}
