//! Composition regressions use real canonical snapshots and independent
//! adjacency/ranking expectations, without a transport or authority substitute.

use super::*;
use dfmcp_core::EdgeId;
use dfmcp_world::{EdgeKind, EdgeRecord};

fn filter(field: &str) -> Value {
    json!({"op":"compare", "field":field, "comparison":"eq",
        "value":{"type":"bool", "value":true}})
}

fn edge(id: u128, from: u64, to: u64) -> EdgeRecord {
    EdgeRecord {
        id: EdgeId::new(id),
        revision: 23,
        kind: EdgeKind::Requires,
        from: EntityId::new(from),
        to: EntityId::new(to),
        fields: BTreeMap::new(),
    }
}

fn relation_request() -> Value {
    let mut input = request();
    input["query"]["where"] = filter("alive");
    input["query"]["relation"] = json!({"edge_kinds":["requires"],
        "target_kinds":["item"], "where":filter("eligible")});
    input
}

fn relation_fixture() -> (WorldSnapshot, OperationContext) {
    let (mut snapshot, mut context) = fixture(&[
        (30, 1),
        (100, 1),
        (80, 1),
        (90, 1),
        (20, 1),
        (200, 1),
        (300, 1),
        (10, 1),
        (1000, 1),
    ]);
    for entity in snapshot.graph.entities.values_mut() {
        if entity.id == EntityId::new(6) {
            entity
                .fields
                .insert("alive".to_owned(), known(WorldValue::Bool(false)));
        }
        if entity.id == EntityId::new(7) {
            let mut fact = known(WorldValue::Bool(true));
            fact.presence = Some(FactPresence::Unknown("unobserved worker".to_owned()));
            entity.fields.insert("alive".to_owned(), fact);
        }
        if entity.id == EntityId::new(8) {
            let mut fact = known(WorldValue::I64(10));
            fact.presence = Some(FactPresence::Redacted("private rank".to_owned()));
            entity.fields.insert("stress".to_owned(), fact);
        }
    }
    for id in [101, 102, 103, 201] {
        let mut eligible = known(WorldValue::Bool(id != 102));
        if id == 103 {
            eligible.presence = Some(FactPresence::Omitted("not projected".to_owned()));
        }
        snapshot.graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 3,
                revision: 5,
                kind: if id == 201 {
                    EntityKind::Building
                } else {
                    EntityKind::Item
                },
                label: format!("target {id}"),
                fields: BTreeMap::from([("eligible".to_owned(), eligible)]),
            },
        );
    }
    for (id, from, to) in [
        (1, 1, 101),
        (2, 1, 103),
        (3, 1, 999),
        (4, 1, 101),
        (5, 1, 101),
        (6, 1, 101),
        (7, 1, 101),
        (8, 1, 101),
        (9, 2, 102),
        (10, 3, 103),
        (11, 4, 999),
        (12, 5, 101),
        (13, 6, 101),
        (14, 7, 101),
        (15, 8, 101),
        (16, 9, 201),
    ] {
        let record = edge(id, from, to);
        snapshot.graph.edges.insert(record.id, record);
    }
    refresh(&mut snapshot, &mut context);
    (snapshot, context)
}

#[test]
fn positive_relation_filters_before_rank_with_bounded_witnesses_and_distinct_unknowns() -> TestResult
{
    let (snapshot, context) = relation_fixture();
    let result = execute(&snapshot, &context, &relation_request())?;
    assert_eq!(ids(&result), ["1", "5"]);
    assert_eq!(result["matched"], 2);
    let selection = &result["selection"];
    for (key, count) in [
        ("kind_selected", 9),
        ("excluded_filter_false", 1),
        ("excluded_filter_unknown", 1),
        ("relation_matched", 3),
        ("unknown_relation", 2),
        ("no_observed_match", 2),
        ("missing_target_records", 2),
        ("unknown_target_filters", 2),
        ("unknown_rank", 1),
        ("selected_edges", 16),
    ] {
        assert_eq!(selection[key], count, "{key}");
    }
    assert_eq!(
        result["coverage"]["selection_complete_within_projection"],
        false
    );
    assert_eq!(result["coverage"]["relation_domain_complete"], false);
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    let row = &result["rows"][0];
    assert_eq!(row["fields"], json!({}));
    assert_eq!(row["relations"]["matched_edges"], 6);
    assert_eq!(row["relations"]["witnesses_returned"], 4);
    assert_eq!(row["relations"]["witnesses_omitted"], 2);
    assert_eq!(row["relations"]["unknown_target_filters"], 1);
    assert_eq!(row["relations"]["missing_target_records"], 1);
    let witnesses = row["relations"]["witnesses"]
        .as_array()
        .ok_or("witnesses")?;
    assert_eq!(
        witnesses
            .iter()
            .map(|value| value["edge_id"].clone())
            .collect::<Vec<_>>(),
        [1_u128, 4, 5, 6].map(|id| json!(EdgeId::new(id).to_string()))
    );
    assert_eq!(witnesses[0]["edge_revision"], 23);
    assert_eq!(
        witnesses[0]["target"],
        json!({"entity_id":"101", "generation":3,"revision":5})
    );
    assert_eq!(
        row["ranking"]["facts"]["stress"]["value"],
        json!({"type":"i64","value":30})
    );
    assert_eq!(result["source_cost"]["maximum_retained_candidates"], 2);
    assert_eq!(result["source_cost"]["relation_source_candidates"], 7);
    Ok(())
}

#[test]
fn every_three_vertex_graph_composes_adjacency_membership_with_exact_top_k() -> TestResult {
    // Independent boolean adjacency oracle: target IDs 1 and 3 are selected;
    // rank 2 and 3 tie, so their IDs decide. Include every directed self-loop.
    for encoded in 0..512usize {
        let (mut snapshot, mut context) = fixture(&[(20, 1), (50, 1), (50, 1)]);
        for from in 0..3 {
            for to in 0..3 {
                if encoded & (1 << (from * 3 + to)) != 0 {
                    let record = edge((from * 3 + to + 1) as u128, from as u64 + 1, to as u64 + 1);
                    snapshot.graph.edges.insert(record.id, record);
                }
            }
        }
        refresh(&mut snapshot, &mut context);
        for direction in ["outgoing", "incoming", "undirected"] {
            let expected = [2usize, 3, 1]
                .into_iter()
                .filter(|subject| {
                    [1usize, 3].into_iter().any(|target| {
                        let outgoing = encoded & (1 << ((subject - 1) * 3 + target - 1)) != 0;
                        let incoming = encoded & (1 << ((target - 1) * 3 + subject - 1)) != 0;
                        match direction {
                            "outgoing" => outgoing,
                            "incoming" => incoming,
                            _ => outgoing || incoming,
                        }
                    })
                })
                .map(|id| id.to_string())
                .collect::<Vec<_>>();
            for width in [1usize, 2] {
                let mut input = request();
                input["query"]["relation"] = json!({"edge_kinds":["requires"],
                    "direction":direction,"target_ids":["1","3"]});
                input["query"]["limit"] = json!(width);
                let result = execute(&snapshot, &context, &input)?;
                assert_eq!(result["matched"], expected.len());
                assert_eq!(
                    ids(&result),
                    expected[..width.min(expected.len())],
                    "graph={encoded}, direction={direction}, width={width}"
                );
                assert_eq!(
                    result["source_cost"]["maximum_retained_candidates"],
                    width.min(expected.len())
                );
            }
        }
    }
    Ok(())
}

#[test]
fn relation_continuation_normalizes_sets_and_binds_every_selector_source_and_session() -> TestResult
{
    let (snapshot, context) = relation_fixture();
    let mut input = relation_request();
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    input["query"]["continuation"] = first["continuation"].clone();
    let mut normalized = input.clone();
    normalized["query"]["relation"]["edge_kinds"] =
        json!(["requires", "custom:requires", "requires"]);
    normalized["query"]["relation"]["target_kinds"] = json!(["item", "other:item"]);
    normalized["query"]["relation"]["direction"] = json!("outgoing");
    normalized["query"]["limit"] = json!(128);
    let final_page = execute(&snapshot, &context, &normalized)?;
    assert_eq!(ids(&final_page), ["5"]);
    assert_eq!(final_page["query_digest"], first["query_digest"]);
    assert!(final_page["continuation"].is_null());
    for case in 0..9 {
        let mut changed = input.clone();
        let mut source = snapshot.clone();
        let mut current = context.clone();
        match case {
            0 => changed["query"]["relation"] = Value::Null,
            1 => changed["query"]["relation"]["edge_kinds"] = json!(["uses"]),
            2 => changed["query"]["relation"]["direction"] = json!("incoming"),
            3 => changed["query"]["relation"]["target_kinds"] = json!([]),
            4 => changed["query"]["relation"]["target_ids"] = json!(["101"]),
            5 => changed["query"]["relation"]["where"] = Value::Null,
            6 => current.session_id = SessionId::new(6),
            7 => source.cursor.epoch += 1,
            _ => source.tick = GameTick(11),
        }
        refresh(&mut source, &mut current);
        assert!(
            matches!(execute(&source, &current, &changed),
            Err(error) if error.code == ErrorCode::StaleAnchor),
            "case {case}"
        );
    }
    let digest =
        Digest32::from_hex(first["query_digest"].as_str().ok_or("digest")?).ok_or("digest")?;
    for position in [
        (EntityId::new(2), 7),
        (EntityId::new(3), 7),
        (EntityId::new(8), 7),
        (EntityId::new(1), 99),
        (EntityId::new(5), 7),
    ] {
        input["query"]["continuation"] = json!(cursor(digest, position));
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::CursorGap));
    }
    Ok(())
}

#[test]
fn negated_unknown_target_stays_unknown_while_positive_matches_remain_decidable() -> TestResult {
    let (snapshot, context) = relation_fixture();
    let mut input = relation_request();
    input["query"]["relation"]["where"] = json!({"op":"not","arg":filter("eligible")});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), ["2"]);
    assert_eq!(result["selection"]["unknown_relation"], 3);
    assert_eq!(result["selection"]["unknown_target_filters"], 2);
    assert_eq!(result["selection"]["unknown_rank"], 0);
    assert_eq!(result["coverage"]["absence_proven"], false);
    Ok(())
}

#[test]
fn relation_only_affects_selection_when_present() -> TestResult {
    let (snapshot, context) = fixture(&[(3, 1), (2, 1), (1, 1)]);
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let original = execute(&snapshot, &context, &input)?;
    input["query"]["relation"] = Value::Null;
    assert_eq!(execute(&snapshot, &context, &input)?, original);
    assert!(original.get("relation").is_none());
    assert!(original["rows"][0].get("relations").is_none());
    assert!(original["selection"].get("unknown_relation").is_none());
    assert_eq!(original["source_cost"]["algorithm"], "bounded_top_k");

    // Freeze the original v1 serialization grammar independently of the new
    // optional-selector branches. Only the source anchor is fixture-generated.
    let append = |bytes: &mut Vec<u8>, value: &[u8]| {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value);
    };
    let mut legacy = b"dfmcp-ranked-entity-query-v1\0".to_vec();
    legacy.extend_from_slice(&context.session_id.get().to_be_bytes());
    legacy.extend_from_slice(&snapshot.fortress_id.get().to_be_bytes());
    legacy.extend_from_slice(&snapshot.cursor.epoch.to_be_bytes());
    legacy.extend_from_slice(&snapshot.cursor.sequence.to_be_bytes());
    legacy.extend_from_slice(&snapshot.tick.0.to_be_bytes());
    legacy.extend_from_slice(snapshot.state_hash.as_bytes());
    legacy.extend_from_slice(&1_u64.to_be_bytes());
    append(&mut legacy, b"unit");
    legacy.push(0);
    legacy.extend_from_slice(&0_u64.to_be_bytes());
    append(&mut legacy, br#"[{"field":"stress","numeric_type":"i64","direction":"descending","scale":null,"max_age_ticks":null},{"field":"age","numeric_type":"u64","direction":"ascending","scale":null,"max_age_ticks":null}]"#);
    assert_eq!(
        original["query_digest"],
        Digest32::of_bytes(&legacy).to_string()
    );
    Ok(())
}

#[test]
fn whole_projected_packets_keep_relation_witnesses_and_resume_without_skipping() -> TestResult {
    let (snapshot, context) = relation_fixture();
    let mut input = relation_request();
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    let packet = |result: &Value| {
        json!({"agent_turn":{"retained_work":["needed"],
        "current_anchor":core_query::anchor_json(snapshot.anchor()), "payload":result}})
    };
    input["query"]["limit"] = json!(128);
    let mut expected_prefix = execute(&snapshot, &context, &input)?;
    expected_prefix["rows"] = json!([first["rows"][0].clone()]);
    expected_prefix["returned"] = json!(1);
    expected_prefix["truncated"] = json!(true);
    expected_prefix["continuation"] = first["continuation"].clone();
    let maximum = serde_json::to_vec(&packet(&expected_prefix))?.len();
    let fits = |result: &Value| -> dfmcp_core::Result<bool> {
        let size = serde_json::to_vec(&packet(result))
            .map_err(|_| invalid("test packet encoding"))?
            .len();
        Ok(size <= maximum)
    };
    let fitted = execute_with_page_budget(&snapshot, &context, &input, u64::MAX, &fits)?;
    assert_eq!(ids(&fitted), ["1"]);
    assert_eq!(
        fitted["rows"][0]["relations"],
        first["rows"][0]["relations"]
    );
    assert_eq!(fitted["rows"][0]["ranking"], first["rows"][0]["ranking"]);
    input["query"]["continuation"] = fitted["continuation"].clone();
    let last = execute_with_page_budget(&snapshot, &context, &input, u64::MAX, &fits)?;
    assert_eq!(ids(&last), ["5"]);
    assert!(last["continuation"].is_null());
    assert!(serde_json::to_vec(&packet(&last))?.len() <= maximum);
    Ok(())
}

#[test]
fn positive_witness_dominates_other_unknown_targets_for_membership() -> TestResult {
    let (mut snapshot, mut context) = relation_fixture();
    snapshot
        .graph
        .entities
        .retain(|id, _| id.get() == 1 || id.get() >= 101);
    snapshot
        .graph
        .edges
        .retain(|_, edge| edge.from == EntityId::new(1));
    refresh(&mut snapshot, &mut context);
    let result = execute(&snapshot, &context, &relation_request())?;
    assert_eq!(ids(&result), ["1"]);
    assert_eq!(result["selection"]["unknown_relation"], 0);
    assert_eq!(result["selection"]["missing_target_records"], 1);
    assert_eq!(result["selection"]["unknown_target_filters"], 1);
    assert_eq!(
        result["coverage"]["selection_complete_within_projection"],
        true
    );
    assert_eq!(result["coverage"]["relation_domain_complete"], false);
    Ok(())
}

#[test]
fn full_source_and_authority_precede_relation_exclusion_and_publication() -> TestResult {
    use std::cell::Cell;
    let (mut snapshot, mut context) = relation_fixture();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(201))
        .ok_or("unselected target")?
        .fields
        .insert(
            "unselected_payload".to_owned(),
            known(WorldValue::Text("x".repeat(65_536))),
        );
    refresh(&mut snapshot, &mut context);
    let mut input = relation_request();
    input["query"]["limit"] = json!(1);
    let output = execute_with_output_budget(&snapshot, &context, &input, 8192)?;
    assert_eq!(ids(&output), ["1"]);
    assert!(
        output["source_cost"]["bounded_bytes"]
            .as_u64()
            .ok_or("source bytes")?
            > 8192
    );
    let callbacks = Cell::new(0);
    let fits = |_: &Value| {
        callbacks.set(callbacks.get() + 1);
        Ok(true)
    };
    let mut narrow = context.clone();
    narrow.budget.max_bytes = 8192;
    assert!(
        matches!(execute_with_page_budget(&snapshot,&narrow,&input,u64::MAX,&fits),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(callbacks.get(), 0);
    narrow = context.clone();
    narrow.grants.clear();
    assert!(execute_with_page_budget(&snapshot, &narrow, &input, u64::MAX, &fits).is_err());
    assert_eq!(callbacks.get(), 0);

    // Invalid edge identities remain invalid even when their kinds are not selected.
    snapshot
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge")?
        .id = EdgeId::new(1000);
    refresh(&mut snapshot, &mut context);
    input["query"]["relation"]["edge_kinds"] = json!(["uses"]);
    assert!(
        matches!(execute_with_page_budget(&snapshot,&context,&input,u64::MAX,&fits),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    assert_eq!(callbacks.get(), 0);
    Ok(())
}

#[test]
fn ranked_relation_keeps_the_closed_positive_selector_grammar() {
    let (snapshot, context) = relation_fixture();
    for relation in [
        json!({"edge_kinds":[]}),
        json!({"edge_kinds":["requires"],"exists":false}),
        json!({"edge_kinds":["requires"],"max_depth":2}),
        json!({"edge_kinds":["requires"],"target_ids":["01"]}),
        json!({"edge_kinds":["requires"],"target_ids":vec!["1";65]}),
        json!({"edge_kinds":["requires"],"direction":"sideways"}),
        json!({"edge_kinds":["nonsense"]}),
    ] {
        let mut input = request();
        input["query"]["relation"] = relation;
        assert!(execute(&snapshot, &context, &input).is_err());
    }
}
