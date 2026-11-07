use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{EntityKind, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField("fixture".to_owned()),
        Digest32::of_bytes(b"relationship-query-fixture"),
    )
}

fn entity(id: u64, kind: EntityKind) -> EntityRecord {
    EntityRecord {
        id: EntityId::new(id),
        generation: 7,
        revision: 9,
        kind,
        label: format!("record {id}"),
        fields: BTreeMap::new(),
    }
}

fn edge(id: u128, from: u64, to: u64) -> EdgeRecord {
    EdgeRecord {
        id: EdgeId::new(id),
        revision: 23,
        kind: dfmcp_world::EdgeKind::Requires,
        from: EntityId::new(from),
        to: EntityId::new(to),
        fields: BTreeMap::new(),
    }
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for id in [1, 2, 3, 4, u64::MAX] {
        let mut job = entity(id, EntityKind::Job);
        job.fields
            .insert("suspended".to_owned(), known(WorldValue::Bool(id != 2)));
        let mut concealed = known(WorldValue::Text("concealedpayload".to_owned()));
        concealed.presence = Some(FactPresence::Redacted("policy".to_owned()));
        job.fields.insert("concealed".to_owned(), concealed);
        graph.entities.insert(job.id, job);
    }
    for id in [101, 102, 103] {
        let mut item = entity(id, EntityKind::Item);
        let mut eligible = known(WorldValue::Bool(id != 102));
        if id == 103 {
            eligible.presence = Some(FactPresence::Omitted("not projected".to_owned()));
        }
        item.fields.insert("eligible".to_owned(), eligible);
        graph.entities.insert(item.id, item);
    }
    graph
        .entities
        .insert(EntityId::new(201), entity(201, EntityKind::Building));
    graph
        .entities
        .insert(EntityId::new(301), entity(301, EntityKind::Unit));
    for (id, from, to) in [
        (1, 1, 101),
        (2, 1, 102),
        (3, 2, 102),
        (4, 3, 103),
        (5, 4, 999),
        (7, 201, 1),
        (8, 1, 1),
        (9, u64::MAX, 101),
    ] {
        let edge = edge(id, from, to);
        graph.edges.insert(edge.id, edge);
    }
    let mut assigned = edge(6, 2, 301);
    assigned.kind = dfmcp_world::EdgeKind::AssignedTo;
    graph.edges.insert(assigned.id, assigned);
    let snapshot = WorldSnapshot::new(
        FortressId::new(42),
        GameTick(10),
        ObservationCursor {
            epoch: 2,
            sequence: 8,
        },
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(5),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_output_tokens: 32_768,
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
    json!({"schema":"dfmcp.query/1","query":{"kind":"related_entities","kinds":["job"],
        "relation":{"edge_kinds":["requires"],"target_kinds":["item"]},"limit":128}})
}

fn filter(field: &str) -> Value {
    json!({"op":"compare","field":field,"comparison":"eq","value":{"type":"bool","value":true}})
}

fn refresh(snapshot: &mut WorldSnapshot, context: &mut OperationContext) {
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
}

fn ids(result: &Value) -> Vec<Value> {
    result["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|row| row["entity_id"].clone())
        .collect()
}

#[test]
fn source_and_target_filters_return_exact_relation_and_endpoint_witnesses() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["where"] = filter("suspended");
    input["query"]["relation"]["where"] = filter("eligible");
    input["query"]["fields"] = json!(["suspended", "concealed"]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), vec![json!("1"), json!(u64::MAX.to_string())]);
    assert_eq!(result["selection"]["excluded_filter_false"], 1);
    assert_eq!(result["selection"]["unknown_relation"], 2);
    assert_eq!(result["selection"]["unknown_target_filters"], 1);
    assert_eq!(result["selection"]["missing_target_records"], 1);
    let row = &result["rows"][0];
    assert_eq!(row["relations"]["matched_edges"], 1);
    let witness = &row["relations"]["witnesses"][0];
    assert_eq!(witness["edge_id"], EdgeId::new(1).to_string());
    assert_eq!(witness["edge_revision"], 23);
    assert_eq!(
        witness["target"],
        json!({"entity_id":"101","generation":7,"revision":9})
    );
    assert_eq!(row["fields"]["suspended"]["epistemic_state"], "observed");
    assert_eq!(row["fields"]["concealed"]["presence"], "redacted");
    assert!(!result.to_string().contains("concealedpayload"));
    assert_eq!(result["coverage"]["relation_domain_complete"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(result, execute(&snapshot, &context, &input)?);
    Ok(())
}

#[test]
fn incoming_and_undirected_selection_keep_multiedges_but_count_self_loops_once() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    let mut input = request();
    input["query"]["relation"]["direction"] = json!("incoming");
    input["query"]["relation"]["target_kinds"] = json!(["building"]);
    let incoming = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&incoming), vec![json!("1")]);
    assert_eq!(
        incoming["rows"][0]["relations"]["witnesses"][0]["target"]["entity_id"],
        "201"
    );
    let parallel = edge(u128::MAX, 1, 101);
    snapshot.graph.edges.insert(parallel.id, parallel);
    refresh(&mut snapshot, &mut context);
    input["query"]["relation"]["direction"] = json!("undirected");
    input["query"]["relation"]["target_kinds"] = json!([]);
    let result = execute(&snapshot, &context, &input)?;
    let relations = &result["rows"][0]["relations"];
    assert_eq!(relations["matched_edges"], 5);
    assert_eq!(relations["witnesses_returned"], 4);
    assert_eq!(relations["witnesses_omitted"], 1);
    assert_eq!(
        relations["witnesses"][3]["edge_id"],
        EdgeId::new(8).to_string()
    );
    input["query"]["relation"]["direction"] = json!("outgoing");
    input["query"]["relation"]["target_ids"] = json!(["101"]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(
        result["rows"][0]["relations"]["witnesses"][1]["edge_id"],
        EdgeId::new(u128::MAX).to_string()
    );
    Ok(())
}

#[test]
fn every_three_vertex_graph_agrees_with_an_independent_incidence_oracle() -> TestResult {
    let (template, mut context) = fixture();
    for mask in 0_u16..512 {
        let mut graph = WorldGraph::default();
        for id in 1..=3 {
            graph
                .entities
                .insert(EntityId::new(id), entity(id, EntityKind::Unit));
        }
        let mut pairs = Vec::new();
        for from in 1..=3 {
            for to in 1..=3 {
                let bit = (from - 1) * 3 + to - 1;
                if mask & (1 << bit) != 0 {
                    let e = edge(u128::from(bit + 1), from, to);
                    graph.edges.insert(e.id, e);
                    pairs.push((from, to));
                }
            }
        }
        let snapshot = WorldSnapshot::new(
            template.fortress_id,
            template.tick,
            template.cursor,
            true,
            graph,
        );
        context.anchor = snapshot.anchor();
        for direction in ["outgoing", "incoming", "undirected"] {
            let mut input = request();
            input["query"]["kinds"] = json!(["unit"]);
            input["query"]["relation"]["target_kinds"] = json!([]);
            input["query"]["relation"]["direction"] = json!(direction);
            let result = execute(&snapshot, &context, &input)?;
            let expected = (1..=3)
                .filter_map(|id| {
                    let count = pairs
                        .iter()
                        .filter(|(from, to)| match direction {
                            "outgoing" => *from == id,
                            "incoming" => *to == id,
                            _ => *from == id || *to == id,
                        })
                        .count();
                    (count > 0).then_some((id.to_string(), count))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                ids(&result),
                expected.iter().map(|(id, _)| json!(id)).collect::<Vec<_>>(),
                "mask {mask}, {direction}"
            );
            for (index, (_, count)) in expected.iter().enumerate() {
                assert_eq!(result["rows"][index]["relations"]["matched_edges"], *count);
            }
        }
    }
    Ok(())
}

#[test]
fn unavailable_target_facts_remain_unknown_even_under_negation() -> TestResult {
    let (base, base_context) = fixture();
    for presence in [
        FactPresence::Absent,
        FactPresence::Unknown("unknown".to_owned()),
        FactPresence::Unsupported("unsupported".to_owned()),
        FactPresence::Omitted("omitted".to_owned()),
        FactPresence::Redacted("redacted".to_owned()),
        FactPresence::Stale(base.anchor()),
        FactPresence::Known(WorldValue::Bool(false)),
    ] {
        let mut snapshot = base.clone();
        let mut context = base_context.clone();
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(103))
            .ok_or("target")?
            .fields
            .get_mut("eligible")
            .ok_or("eligible")?
            .presence = Some(presence);
        refresh(&mut snapshot, &mut context);
        let mut input = request();
        input["query"]["relation"]["where"] = json!({"op":"not","arg":{
            "op":"compare","field":"eligible","comparison":"eq","value":{"type":"bool","value":false}}});
        let result = execute(&snapshot, &context, &input)?;
        assert!(!ids(&result).contains(&json!("3")));
        assert_eq!(result["selection"]["unknown_relation"], 2);
        assert_eq!(result["selection"]["unknown_target_filters"], 1);
        assert_eq!(
            result["coverage"]["selection_complete_within_projection"],
            false
        );
    }
    Ok(())
}

#[test]
fn a_positive_witness_survives_other_unknown_edges_without_claiming_complete_edges() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    let extra = edge(10, 1, 103);
    snapshot.graph.edges.insert(extra.id, extra);
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["relation"]["where"] = filter("eligible");
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["rows"][0]["entity_id"], "1");
    assert_eq!(result["rows"][0]["relations"]["matched_edges"], 1);
    assert_eq!(result["rows"][0]["relations"]["unknown_target_filters"], 1);
    assert_eq!(result["coverage"]["relation_domain_complete"], false);
    Ok(())
}

#[test]
fn missing_targets_and_no_observed_matches_never_become_world_absence() -> TestResult {
    let (snapshot, context) = fixture();
    let baseline = execute(&snapshot, &context, &request())?;
    assert_eq!(baseline["selection"]["unknown_relation"], 1);
    assert_eq!(baseline["selection"]["missing_target_records"], 1);
    let mut input = request();
    input["query"]["relation"]["target_ids"] = json!(["101"]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), vec![json!("1"), json!(u64::MAX.to_string())]);
    assert_eq!(result["selection"]["unknown_relation"], 0);
    assert_eq!(result["selection"]["no_observed_match"], 3);
    assert_eq!(result["coverage"]["absence_proven"], false);
    input["query"]["relation"]["target_ids"] = json!(["888"]);
    let empty = execute(&snapshot, &context, &input)?;
    assert_eq!(empty["matched"], 0);
    assert!(empty["continuation"].is_null());
    assert_eq!(empty["coverage"]["relation_domain_complete"], false);
    assert_eq!(empty["coverage"]["absence_proven"], false);
    Ok(())
}

#[test]
fn source_predicate_unknown_is_separate_from_relation_unknown() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(3))
        .ok_or("source")?
        .fields
        .get_mut("suspended")
        .ok_or("suspended")?
        .presence = Some(FactPresence::Omitted("unknown".to_owned()));
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["where"] = filter("suspended");
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["selection"]["excluded_filter_false"], 1);
    assert_eq!(result["selection"]["excluded_filter_unknown"], 1);
    assert_eq!(result["selection"]["unknown_relation"], 1);
    assert_eq!(ids(&result), vec![json!("1"), json!(u64::MAX.to_string())]);
    Ok(())
}

#[test]
fn pages_keep_whole_entities_in_both_orders_with_changed_width() -> TestResult {
    let (snapshot, context) = fixture();
    for order in ["id_ascending", "id_descending"] {
        let mut input = request();
        input["query"]["order"] = json!(order);
        input["query"]["limit"] = json!(1);
        let first = execute(&snapshot, &context, &input)?;
        let mut seen = ids(&first);
        assert_eq!(first["matched"], 4);
        input["query"]["continuation"] = first["continuation"].clone();
        input["query"]["limit"] = json!(2);
        for _ in 0..2 {
            let next = execute(&snapshot, &context, &input)?;
            seen.extend(ids(&next));
            input["query"]["continuation"] = next["continuation"].clone();
        }
        let mut expected = vec![
            json!("1"),
            json!("2"),
            json!("3"),
            json!(u64::MAX.to_string()),
        ];
        if order == "id_descending" {
            expected.reverse();
        }
        assert_eq!(seen, expected);
        assert!(input["query"]["continuation"].is_null());
    }
    Ok(())
}

#[test]
fn every_selector_source_and_session_binds_continuation_and_aliases_normalize() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    input["query"]["continuation"] = first["continuation"].clone();
    let mut normalized = input.clone();
    normalized["query"]["kinds"] = json!(["other:job", "job", "job"]);
    normalized["query"]["relation"]["edge_kinds"] = json!(["requires", "custom:requires"]);
    normalized["query"]["relation"]["target_kinds"] = json!(["item", "other:item"]);
    assert_eq!(
        execute(&snapshot, &context, &normalized)?["query_digest"],
        first["query_digest"]
    );
    for case in 0..12 {
        let mut changed = input.clone();
        let mut source = snapshot.clone();
        let mut current = context.clone();
        match case {
            0 => changed["query"]["kinds"] = json!([]),
            1 => changed["query"]["fields"] = json!(["suspended"]),
            2 => changed["query"]["where"] = json!({"op":"all","args":[]}),
            3 => changed["query"]["relation"]["edge_kinds"] = json!(["uses"]),
            4 => changed["query"]["relation"]["direction"] = json!("incoming"),
            5 => changed["query"]["relation"]["target_kinds"] = json!([]),
            6 => changed["query"]["relation"]["target_ids"] = json!(["101"]),
            7 => changed["query"]["relation"]["where"] = filter("eligible"),
            8 => changed["query"]["order"] = json!("id_descending"),
            9 => current.session_id = SessionId::new(6),
            10 => source.cursor.epoch += 1,
            _ => source.tick = GameTick(11),
        }
        refresh(&mut source, &mut current);
        assert!(
            matches!(execute(&source,&current,&changed),Err(error) if error.code == ErrorCode::StaleAnchor),
            "case {case}"
        );
    }
    let digest =
        Digest32::from_hex(first["query_digest"].as_str().ok_or("digest")?).ok_or("digest")?;
    for position in [
        (EntityId::new(888), 7),
        (EntityId::new(u64::MAX), 7),
        (EntityId::new(1), 99),
    ] {
        input["query"]["continuation"] = json!(cursor(digest, position));
        assert!(
            matches!(execute(&snapshot,&context,&input),Err(error) if error.code == ErrorCode::CursorGap)
        );
    }
    Ok(())
}

#[test]
fn same_hash_kind_aliases_leave_answers_and_edge_witnesses_unchanged() -> TestResult {
    let (mut snapshot, context) = fixture();
    let before = execute(&snapshot, &context, &request())?;
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("source")?
        .kind = EntityKind::Other("job".to_owned());
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(101))
        .ok_or("target")?
        .kind = EntityKind::Other("item".to_owned());
    snapshot
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge")?
        .kind = dfmcp_world::EdgeKind::Custom("requires".to_owned());
    snapshot.refresh_hash();
    assert_eq!(snapshot.anchor(), context.anchor);
    assert_eq!(execute(&snapshot, &context, &request())?, before);
    Ok(())
}

#[test]
fn complete_source_and_whole_output_rows_use_independent_budget_dimensions() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or("source")?
        .fields
        .insert(
            "large".to_owned(),
            known(WorldValue::List(vec![WorldValue::Null; 20_000])),
        );
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["fields"] = json!(["large"]);
    context.budget.max_output_tokens = 1500;
    let first = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&first), vec![json!("1")]);
    assert!(first.to_string().len() <= 6000);
    input["query"]["continuation"] = first["continuation"].clone();
    assert!(
        matches!(execute(&snapshot,&context,&input),Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    context.budget.max_bytes = 8192;
    let mut narrow = request();
    narrow["query"]["relation"]["target_ids"] = json!(["888"]);
    assert!(
        matches!(execute(&snapshot,&context,&narrow),Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    context.budget.max_bytes = 4 * 1024 * 1024;
    context.budget.max_entities = 10;
    assert!(
        matches!(execute(&snapshot,&context,&json!({"schema":"dfmcp.query/1","query":{
        "kind":"related_entities","relation":{"edge_kinds":["uses"]},"limit":1}})),Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn authority_anchor_and_source_identity_are_never_bypassed_by_an_empty_selection() -> TestResult {
    let (snapshot, context) = fixture();
    for case in 0..6 {
        let mut denied = context.clone();
        match case {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].expires_at_tick = Some(GameTick(9)),
            3 => {
                denied.grants[0].scope.entity_ids.insert(EntityId::new(1));
            }
            4 => denied.grants[0].remaining_uses = Some(1),
            _ => denied.anchor.cursor.sequence += 1,
        }
        assert!(execute(&snapshot, &denied, &request()).is_err());
    }
    let mut input = request();
    input["expected_anchor"] = core_query::anchor_json(snapshot.anchor());
    assert!(execute(&snapshot, &context, &input).is_ok());
    input["expected_anchor"]["sequence"] = json!(9);
    assert!(
        matches!(execute(&snapshot,&context,&input),Err(error) if error.code == ErrorCode::StaleAnchor)
    );
    let mut invalid = snapshot.clone();
    invalid.state_hash = Digest32::ZERO;
    let mut current = context.clone();
    current.anchor = invalid.anchor();
    assert!(
        matches!(execute(&invalid,&current,&request()),Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    invalid = snapshot.clone();
    invalid
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge")?
        .id = EdgeId::new(333);
    refresh(&mut invalid, &mut current);
    let mut empty = request();
    empty["query"]["relation"]["edge_kinds"] = json!(["supports"]);
    assert!(
        matches!(execute(&invalid,&current,&empty),Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    Ok(())
}

#[test]
fn closed_grammar_rejects_negated_relations_and_unbounded_or_ambiguous_selectors() {
    let (snapshot, context) = fixture();
    for invalid in [
        json!({"edge_kinds":[]}),
        json!({"edge_kinds":["requires"],"exists":false}),
        json!({"edge_kinds":["requires"],"max_depth":100}),
        json!({"edge_kinds":["requires"],"target_ids":["01"]}),
        json!({"edge_kinds":["requires"],"target_ids":["18446744073709551616"]}),
        json!({"edge_kinds":["requires"],"target_ids":vec!["1";65]}),
        json!({"edge_kinds":["nonsense"]}),
        json!({"edge_kinds":["requires"],"direction":"sideways"}),
    ] {
        let mut input = request();
        input["query"]["relation"] = invalid;
        assert!(execute(&snapshot, &context, &input).is_err());
    }
    let mut input = request();
    input["query"]["limit"] = json!(129);
    assert!(execute(&snapshot, &context, &input).is_err());
}


#[test]
fn reserved_output_preserves_source_allowance_and_still_enforces_both_caps() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?
        .fields.insert("source_only_payload".to_owned(),
            known(WorldValue::Text("x".repeat(65_536))));
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let maximum_output_bytes = 8192;
    let first = execute_with_output_budget(&snapshot, &context, &input, maximum_output_bytes)?;
    assert_eq!(ids(&first), vec![json!("1")]);
    assert!(first["source_cost"]["bounded_bytes"].as_u64().ok_or("source bytes")?
        > maximum_output_bytes);
    assert!(serde_json::to_vec(&first)?.len() as u64 <= maximum_output_bytes);
    input["query"]["continuation"] = first["continuation"].clone();
    let second = execute_with_output_budget(&snapshot, &context, &input, maximum_output_bytes)?;
    assert_eq!(ids(&second), vec![json!("2")]);
    assert_eq!(second["query_digest"], first["query_digest"]);

    let mut narrowed = context.clone();
    narrowed.budget.max_bytes = maximum_output_bytes;
    assert!(matches!(
        execute_with_output_budget(&snapshot, &narrowed, &input, u64::MAX),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    assert!(matches!(
        execute_with_output_budget(&snapshot, &context, &input, 1),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    let mut tiny_output = context.clone();
    tiny_output.budget.max_output_tokens = 1;
    assert!(matches!(
        execute_with_output_budget(&snapshot, &tiny_output, &input, u64::MAX),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    Ok(())
}
