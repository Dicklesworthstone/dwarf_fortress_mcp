use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource,
    Value as WorldValue, WorldGraph,
};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in 1..=6 {
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: id,
                kind: EntityKind::Building,
                label: format!("Station {id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    // The three-edge route beats the direct edge. The second origin wins a
    // cost tie by fewer hops; the last station is absent from every route.
    for (id, from, to, cost) in [
        (1, 1, 4, 9),
        (2, 1, 2, 1),
        (3, 2, 3, 1),
        (4, 3, 4, 1),
        (5, 5, 4, 3),
    ] {
        graph.edges.insert(
            EdgeId::new(id),
            EdgeRecord {
                id: EdgeId::new(id),
                revision: 1,
                kind: EdgeKind::Uses,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::from([(
                    "cost".to_owned(),
                    Fact::known(
                        WorldValue::U64(cost),
                        GameTick(10),
                        FactSource::DfhackField("route.cost".to_owned()),
                        Digest32::of_bytes(b"weighted-source"),
                    ),
                )]),
            },
        );
    }
    let source = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: source.anchor(),
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
                fortress_id: Some(source.fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    (source, context)
}

fn request() -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"weighted_routes",
        "roots":[{"entity_id":"1","generation":1}],
        "edge_kinds":["uses"],"cost_field":"cost"}})
}

#[test]
fn weighted_routes_choose_total_cost_and_preserve_exact_edge_evidence() -> TestResult {
    let (source, context) = fixture();
    let before = source.canonical_bytes();
    let result = execute(&source, &context, &request())?;
    assert_eq!(result["returned"], 4);
    assert_eq!(result["entries"][3]["total_cost"], 3);
    assert_eq!(result["entries"][3]["hops"], 3);
    assert_eq!(result["entries"][3]["parent"], "3");
    assert_eq!(result["entries"][3]["via_edge"], EdgeId::new(4).to_string());
    assert_eq!(
        result["entries"][3]["edge_cost_fact"]["source_digest"],
        Digest32::of_bytes(b"weighted-source").to_string()
    );
    assert_eq!(result["unreachable"], json!(["5", "6"]));
    assert_eq!(result["model"]["native_walkability_proven"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["witness"]["anchor"], anchor_json(source.anchor()));
    assert_eq!(execute(&source, &context, &request())?, result);
    assert_eq!(source.canonical_bytes(), before);
    Ok(())
}

#[test]
fn weighted_targets_return_complete_paths_and_explicit_projection_unreachability() -> TestResult {
    let (source, context) = fixture();
    let mut query = request();
    query["query"]["targets"] =
        json!([{"entity_id":"6","generation":1},{"entity_id":"4","generation":1}]);
    let result = execute(&source, &context, &query)?;
    assert_eq!(result["returned"], 1);
    assert_eq!(result["reachable_vertices"], 4);
    assert_eq!(result["paths"][0]["vertices"], json!(["1", "2", "3", "4"]));
    assert_eq!(result["paths"][0]["total_cost"], 3);
    assert_eq!(result["paths"][0]["edges"][0]["cost"], 1);
    assert_eq!(result["paths"][1]["status"], "unreachable_in_projection");
    assert_eq!(result["paths"][1]["total_cost"], Value::Null);
    assert_eq!(result["unreachable"], json!(["6"]));
    query["query"]["roots"] = json!([{"entity_id":"4","generation":1}]);
    query["query"]["targets"] = json!([{"entity_id":"1","generation":1}]);
    query["query"]["direction"] = json!("incoming");
    let incoming = execute(&source, &context, &query)?;
    assert_eq!(
        incoming["paths"][0]["vertices"],
        json!(["4", "3", "2", "1"])
    );
    assert_eq!(incoming["paths"][0]["total_cost"], 3);
    assert_eq!(
        incoming["paths"][0]["edges"][0]["edge_id"],
        EdgeId::new(4).to_string()
    );
    Ok(())
}

#[test]
fn weighted_multi_source_ties_and_selector_order_are_canonical() -> TestResult {
    let (source, context) = fixture();
    let mut first = request();
    first["query"]["roots"] =
        json!([{"entity_id":"5","generation":1},{"entity_id":"1","generation":1}]);
    first["query"]["edge_kinds"] = json!(["uses", "requires", "uses"]);
    let mut second = first.clone();
    second["query"]["roots"]
        .as_array_mut()
        .ok_or("roots absent")?
        .reverse();
    second["query"]["edge_kinds"] = json!(["requires", "uses"]);
    let result = execute(&source, &context, &first)?;
    assert_eq!(result, execute(&source, &context, &second)?);
    assert_eq!(result["entries"][3]["root"], "5");
    assert_eq!(result["entries"][3]["hops"], 1);
    first["query"]["roots"] =
        json!([{"entity_id":"1","generation":1},{"entity_id":"1","generation":1}]);
    assert_eq!(
        execute(&source, &context, &first).map_err(|e| e.code),
        Err(ErrorCode::InvalidRequest)
    );
    for key in ["roots", "targets"] {
        let mut stale = request();
        stale["query"][key] = json!([{"entity_id":"1","generation":2}]);
        assert_eq!(
            execute(&source, &context, &stale).map_err(|e| e.code),
            Err(ErrorCode::Conflict)
        );
    }
    Ok(())
}

#[test]
fn weighted_routes_refuse_unknown_costs_even_outside_the_reachable_component() -> TestResult {
    for variant in 0..8 {
        let (mut source, mut context) = fixture();
        let fact = source
            .graph
            .edges
            .get_mut(&EdgeId::new(5))
            .ok_or("edge absent")?
            .fields
            .get_mut("cost")
            .ok_or("cost absent")?;
        match variant {
            0 => fact.presence = Some(FactPresence::Unknown("not measured".to_owned())),
            1 => fact.source = FactSource::AgentAssertion("estimate".to_owned()),
            2 => fact.source_digest = Digest32::ZERO,
            3 => fact.observed_at = GameTick(11),
            4 => fact.presence = Some(FactPresence::Known(WorldValue::U64(99))),
            5 => fact.value = WorldValue::I64(-1),
            6 => fact.value = WorldValue::Text("3".to_owned()),
            _ => fact.source = FactSource::Replay,
        }
        source.refresh_hash();
        context.anchor = source.anchor();
        assert!(
            execute(&source, &context, &request()).is_err(),
            "variant {variant}"
        );
    }
    Ok(())
}

#[test]
fn weighted_routes_require_current_whole_projection_authority_and_exact_source() -> TestResult {
    let (source, context) = fixture();
    for variant in 0..7 {
        let mut denied = context.clone();
        match variant {
            0 => denied.grants.clear(),
            1 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            2 => denied.grants[0].expires_at_tick = Some(GameTick(9)),
            3 => denied.grants[0].remaining_uses = Some(0),
            4 => denied.cancellation_requested = true,
            5 => denied.budget.max_wall_millis = 0,
            _ => denied.anchor.cursor.sequence += 1,
        }
        assert!(
            execute(&source, &denied, &request()).is_err(),
            "variant {variant}"
        );
    }
    let mut wrong = request();
    wrong["expected_anchor"] = anchor_json(source.anchor());
    wrong["expected_anchor"]["game_tick"] = json!(9);
    assert_eq!(
        execute(&source, &context, &wrong).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    let mut other = context.clone();
    other.session_id = SessionId::new(2);
    assert_ne!(
        execute(&source, &context, &request())?["witness"]["authorization_scope_digest"],
        execute(&source, &other, &request())?["witness"]["authorization_scope_digest"]
    );
    Ok(())
}

#[test]
fn weighted_routes_keep_source_allowance_separate_from_selected_output() -> TestResult {
    let (mut source, mut context) = fixture();
    source
        .graph
        .entities
        .get_mut(&EntityId::new(6))
        .ok_or("vertex absent")?
        .fields
        .insert(
            "unselected".to_owned(),
            Fact::known(
                WorldValue::Bytes(vec![0; 65_536]),
                source.tick,
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
    source.refresh_hash();
    context.anchor = source.anchor();
    let mut query = request();
    query["query"]["targets"] = json!([{"entity_id":"4","generation":1}]);
    let result = execute_with_output_budget(&source, &context, &query, 8192)?;
    assert_eq!(result["returned"], 1);
    assert!(serde_json::to_vec(&result)?.len() <= 8192);
    assert!(source.canonical_bytes().len() > 65_536);
    assert_eq!(
        execute_with_output_budget(&source, &context, &query, 64).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_bytes = source.canonical_bytes().len() as u64 - 1;
    assert_eq!(
        execute_with_output_budget(&source, &context, &query, 8192).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_bytes = 1024 * 1024;
    context.budget.max_entities = 10;
    assert_eq!(
        execute(&source, &context, &query).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_entities = 128;
    context.budget.max_output_tokens = 10;
    assert_eq!(
        execute(&source, &context, &query).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn retained_weighted_routes_keep_current_authority_after_a_clock_reset() -> TestResult {
    let (source, mut current) = fixture();
    current.anchor.tick = GameTick(2);
    current.anchor.cursor.epoch = 1;
    current.grants[0].expires_at_tick = Some(GameTick(5));
    assert_eq!(
        execute(&source, &current, &request()).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    let result = execute_laboratory_source(&source, &current, &request(), 60_000)?;
    assert_eq!(result["anchor"], anchor_json(source.anchor()));
    assert_eq!(result["entries"][3]["total_cost"], 3);
    assert_eq!(
        execute_laboratory_source(&source, &current, &request(), 0).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    current.grants[0].expires_at_tick = Some(GameTick(1));
    assert_eq!(
        execute_laboratory_source(&source, &current, &request(), 60_000).map_err(|e| e.code),
        Err(ErrorCode::CapabilityDenied)
    );
    current.grants[0].expires_at_tick = None;
    current.anchor.fortress_id = FortressId::new(2);
    current.grants[0].scope.fortress_id = Some(FortressId::new(2));
    assert_eq!(
        execute_laboratory_source(&source, &current, &request(), 60_000).map_err(|e| e.code),
        Err(ErrorCode::StaleAnchor)
    );
    Ok(())
}

#[test]
fn weighted_routes_have_closed_grammar_and_additive_discovery() -> TestResult {
    let (source, context) = fixture();
    for (key, value) in [
        ("cost_field", json!("")),
        ("cost_field", json!("x".repeat(129))),
        ("cost_field", json!("x\0y")),
        ("cost_field", json!(" ")),
        ("direction", json!("undirected")),
        ("roots", json!([])),
        ("roots", json!([{"entity_id":"01","generation":1}])),
        (
            "roots",
            json!([{"entity_id":"1","generation":1,"extra":true}]),
        ),
        ("targets", Value::Null),
        ("edge_kinds", json!([])),
        ("edge_kinds", json!(["unknown"])),
        ("weights", json!([1, 2, 3])),
        (
            "targets",
            json!(vec![json!({"entity_id":"4","generation":1}); 65]),
        ),
    ] {
        let mut malformed = request();
        malformed["query"][key] = value;
        assert!(
            execute(&source, &context, &malformed).is_err(),
            "accepted {malformed}"
        );
    }
    let duplicate = r#"{"schema":"dfmcp.query/1","query":{"kind":"weighted_routes","roots":[{"entity_id":"1","generation":1}],"edge_kinds":["uses"],"cost_field":"a","cost_field":"b"}}"#;
    assert!(serde_json::from_str::<Envelope>(duplicate).is_err());
    let base: Value = serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))?;
    let result = extend_schema(base.clone())?;
    let old = base["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("variants absent")?;
    let new = result["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("variants absent")?;
    assert_eq!(&new[..old.len()], old.as_slice());
    assert_eq!(new.len(), old.len() + 1);
    assert_eq!(
        new.last().ok_or("extension absent")?["properties"]["kind"]["const"],
        "weighted_routes"
    );
    Ok(())
}
