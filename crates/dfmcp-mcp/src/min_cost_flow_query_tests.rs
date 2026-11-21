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

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn fact(value: WorldValue, field: &str) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField(field.to_owned()),
        Digest32::of_bytes(b"cost-flow-source"),
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
    for (id, from, to, capacity, cost) in [
        (1, 1, 2, 3, 1),
        (2, 1, 3, 2, 5),
        (3, 2, 3, 1, 1),
        (4, 2, 4, 2, 1),
        (5, 3, 4, 3, 1),
        (6, 5, 5, 7, 0),
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
                        fact(WorldValue::Text("items".to_owned()), "edge.unit"),
                    ),
                    ("cost".to_owned(), fact(WorldValue::U64(cost), "edge.cost")),
                    (
                        "cost_unit".to_owned(),
                        fact(WorldValue::Text("tile_steps".to_owned()), "edge.cost_unit"),
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
    json!({"schema":"dfmcp.query/1","query":{"kind":"min_cost_flow","edge_kinds":["uses"],
        "capacity_field":"capacity","unit_field":"unit",
        "cost_field":"cost","cost_unit_field":"cost_unit","demand":3,
        "source":{"entity_id":"1","generation":1},
        "sink":{"entity_id":"4","generation":1}}})
}

#[test]
fn cost_flow_executor_returns_cheapest_allocation_and_complete_dual_certificate() -> TestResult {
    let (snapshot, context) = fixture();
    let before = snapshot.canonical_bytes();
    let result = execute(&snapshot, &context, &request())?;
    assert_eq!(result["value"], 3);
    assert_eq!(result["unit"], "items");
    assert_eq!(result["returned_edges"], 6);
    assert_eq!(result["demand"], 3);
    assert_eq!(result["unmet_demand"], 0);
    assert_eq!(result["demand_met"], true);
    assert_eq!(result["total_cost"], "7");
    assert_eq!(result["cost_unit"], "tile_steps");
    assert!(result["minimum_cut"].is_null());
    let edges = result["edges"].as_array().ok_or("edges absent")?;
    for (index, expected) in [3, 0, 1, 2, 1, 0].iter().enumerate() {
        assert_eq!(edges[index]["flow"], *expected);
        assert_eq!(edges[index]["unit_fact"]["value"]["value"], "items");
        assert_eq!(edges[index]["capacity_fact"]["presence"], "known");
        assert_eq!(edges[index]["cost_fact"]["presence"], "known");
        assert_eq!(
            edges[index]["cost_unit_fact"]["value"]["value"],
            "tile_steps"
        );
        assert_eq!(
            edges[index]["capacity_fact"]["source_digest"],
            Digest32::of_bytes(b"cost-flow-source").to_string()
        );
        assert_eq!(edges[index]["from"]["generation"], 1);
        assert_eq!(edges[index]["to"]["generation"], 1);
    }
    assert_eq!(result["model"]["exact_within_projection"], true);
    assert_eq!(result["model"]["epistemic_state"], "inferred");
    assert_eq!(result["model"]["conservation_verified"], true);
    assert_eq!(result["model"]["residual_dual_verified"], true);
    assert_eq!(result["model"]["shortfall_cut_verified"], false);
    verify_dual(&result)?;
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
fn cost_flow_canonicalizes_kind_sets_but_preserves_direction_and_source_scope() -> TestResult {
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
    assert_eq!(reverse["unit"], "items");
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
fn cost_flow_rejects_missing_unavailable_or_mixed_units_even_in_unreachable_edges() -> TestResult {
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
fn cost_flow_fences_both_endpoint_generations_and_checks_complete_expected_anchor() -> TestResult {
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
    assert_eq!(execute(&snapshot, &context, &input)?["value"], 3);
    input["expected_anchor"]["game_tick"] = json!(9);
    assert_eq!(
        execute(&snapshot, &context, &input).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn cost_flow_requires_current_query_authority_and_a_live_foreground_allowance() -> TestResult {
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
fn retained_cost_flow_keeps_current_authority_after_restore_and_selected_source_time() -> TestResult
{
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
    assert_eq!(result["value"], 3);
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
fn cost_flow_complete_source_and_output_budgets_are_independent_and_atomic() -> TestResult {
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
        3
    );
    context.budget.max_bytes = snapshot.canonical_bytes().len() as u64 - 1;
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &request(), 16_384).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn cost_flow_wide_total_uses_exact_decimal_string_without_infinity() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot.graph.edges.retain(|id, _| *id == EdgeId::new(1));
    let edge = snapshot
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge absent")?;
    edge.to = EntityId::new(4);
    for field in ["capacity", "cost"] {
        edge.fields
            .insert(field.to_owned(), fact(WorldValue::U64(u64::MAX), field));
    }
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
    let mut input = request();
    input["query"]["demand"] = json!(u64::MAX);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["value"], json!(u64::MAX));
    assert_eq!(
        result["total_cost"],
        (u128::from(u64::MAX) * u128::from(u64::MAX)).to_string()
    );
    assert!(result["minimum_cut"].is_null());
    verify_dual(&result)?;
    Ok(())
}

#[test]
fn cost_flow_closed_grammar_rejects_ambiguous_handles_and_effect_or_capacity_injection()
-> TestResult {
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
        ("cost_field", json!("unit")),
        ("cost_unit_field", json!("cost")),
        ("cost_field", json!(" ")),
        ("cost_unit_field", json!("x".repeat(129))),
        ("demand", json!(0)),
        ("demand", json!(-1)),
        ("demand", json!(1.5)),
        ("demand", json!(null)),
        ("demand", json!(true)),
        ("demand", json!("3")),
        ("cost", json!(0)),
        ("cost_unit", json!("tile_steps")),
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
        ("unit", json!("items")),
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
        r#"{"schema":"dfmcp.query/1","query":{"kind":"min_cost_flow","edge_kinds":["uses"],"capacity_field":"capacity","capacity_field":"other","unit_field":"unit","cost_field":"cost","cost_unit_field":"cost_unit","demand":3,"source":{"entity_id":"1","generation":1},"sink":{"entity_id":"4","generation":1}}}"#,
        r#"{"schema":"dfmcp.query/1","query":{"kind":"min_cost_flow","edge_kinds":["uses"],"capacity_field":"capacity","unit_field":"unit","cost_field":"cost","cost_unit_field":"cost_unit","demand":3,"source":{"entity_id":"1","generation":1,"generation":2},"sink":{"entity_id":"4","generation":1}}}"#,
    ] {
        assert!(serde_json::from_str::<Envelope>(raw).is_err());
    }
    Ok(())
}

#[test]
fn cost_flow_schema_extension_preserves_existing_queries_and_requires_units() -> TestResult {
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
    assert_eq!(extension["properties"]["kind"]["const"], "min_cost_flow");
    assert!(
        extension["required"]
            .as_array()
            .ok_or("requirements absent")?
            .contains(&json!("unit_field"))
    );
    assert_eq!(extension["additionalProperties"], false);
    for field in ["demand", "cost_field", "cost_unit_field"] {
        assert!(
            extension["required"]
                .as_array()
                .ok_or("requirements absent")?
                .contains(&json!(field))
        );
    }
    Ok(())
}

#[test]
fn cost_flow_original_json_rejects_duplicate_demand_and_cost_selectors() -> TestResult {
    let raw = serde_json::to_string(&request())?;
    for (original, duplicate) in [
        ("\"demand\":3", "\"demand\":3,\"demand\":4"),
        (
            "\"cost_field\":\"cost\"",
            "\"cost_field\":\"cost\",\"cost_field\":\"other\"",
        ),
        (
            "\"cost_unit_field\":\"cost_unit\"",
            "\"cost_unit_field\":\"cost_unit\",\"cost_unit_field\":\"other\"",
        ),
    ] {
        assert!(raw.contains(original));
        assert!(serde_json::from_str::<Envelope>(&raw.replacen(original, duplicate, 1)).is_err());
    }
    Ok(())
}

fn verify_dual(result: &Value) -> TestResult {
    let potentials: BTreeMap<_, _> = result["potentials"]
        .as_array()
        .ok_or("potentials absent")?
        .iter()
        .map(|entry| -> TestResult<(String, i128)> {
            Ok((
                entry["entity"]["entity_id"]
                    .as_str()
                    .ok_or("potential handle")?
                    .to_owned(),
                entry["potential"]
                    .as_str()
                    .ok_or("potential not decimal")?
                    .parse()?,
            ))
        })
        .collect::<std::result::Result<_, _>>()?;
    for edge in result["edges"].as_array().ok_or("edges absent")? {
        let from = edge["from"]["entity_id"].as_str().ok_or("from absent")?;
        let to = edge["to"]["entity_id"].as_str().ok_or("to absent")?;
        let cost = i128::from(edge["cost"].as_u64().ok_or("cost absent")?);
        let flow = edge["flow"].as_u64().ok_or("flow absent")?;
        let capacity = edge["capacity"].as_u64().ok_or("capacity absent")?;
        let reduced = cost + potentials[from] - potentials[to];
        if flow < capacity {
            assert!(reduced >= 0, "{edge}");
        }
        if flow > 0 {
            assert!(reduced <= 0, "{edge}");
        }
    }
    Ok(())
}

#[test]
fn cost_flow_shortfall_has_a_complete_equal_capacity_cut() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["demand"] = json!(9);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["demand"], 9);
    assert_eq!(result["value"], 5);
    assert_eq!(result["unmet_demand"], 4);
    assert_eq!(result["demand_met"], false);
    assert_eq!(result["total_cost"], "19");
    assert_eq!(result["minimum_cut"]["capacity"], 5);
    assert_eq!(result["model"]["shortfall_cut_verified"], true);
    assert_eq!(
        result["minimum_cut"]["source_side"],
        json!([{"entity_id":"1","generation":1,"revision":1}])
    );
    assert_eq!(
        result["minimum_cut"]["edge_ids"],
        json!([EdgeId::new(1).to_string(), EdgeId::new(2).to_string()])
    );
    verify_dual(&result)?;
    Ok(())
}

#[test]
fn cost_flow_requires_qualified_costs_and_common_units_in_disconnected_edges() -> TestResult {
    for field in ["cost", "cost_unit"] {
        for variant in 0..8 {
            let (mut snapshot, mut context) = fixture();
            let edge = snapshot
                .graph
                .edges
                .get_mut(&EdgeId::new(6))
                .ok_or("edge absent")?;
            let original = edge.fields.get_mut(field).ok_or("fact absent")?;
            match variant {
                0 => original.observed_at = GameTick(11),
                1 => original.source_digest = Digest32::ZERO,
                2 => original.source = FactSource::AgentAssertion("guess".to_owned()),
                3 => original.presence = Some(FactPresence::Unknown("unobserved".to_owned())),
                4 => {
                    edge.fields.remove(field);
                }
                5 => {
                    edge.fields
                        .insert(field.to_owned(), fact(WorldValue::I64(-1), field));
                }
                6 => {
                    edge.fields.insert(
                        field.to_owned(),
                        fact(WorldValue::Text("wrong_unit".to_owned()), field),
                    );
                }
                _ => {
                    edge.fields.insert(
                        field.to_owned(),
                        fact(WorldValue::Text(" ".to_owned()), field),
                    );
                }
            }
            snapshot.refresh_hash();
            context.anchor = snapshot.anchor();
            assert_eq!(
                execute(&snapshot, &context, &request()).map_err(|error| error.code),
                Err(ErrorCode::PreconditionsFailed),
                "accepted {field}/{variant}"
            );
        }
    }
    Ok(())
}
