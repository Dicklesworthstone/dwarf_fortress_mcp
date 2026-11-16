//! Exact multi-dimensional summaries through the existing aggregate executor.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, MapCoord, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{EdgeKind, EdgeRecord, WorldGraph};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField("group-fields-fixture".to_owned()),
        Digest32::of_bytes(b"group-fields-fixture"),
    )
}

fn world(rows: Vec<BTreeMap<String, Fact>>) -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for (index, fields) in rows.into_iter().enumerate() {
        let id = EntityId::new(index as u64 + 1);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 2,
                revision: 1,
                kind: EntityKind::Job,
                label: format!("job {}", id.get()),
                fields,
            },
        );
    }
    let snapshot = WorldSnapshot::new(
        FortressId::new(42),
        GameTick(10),
        ObservationCursor {
            epoch: 2,
            sequence: 3,
        },
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(17),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 1_000,
            max_entities: 100_000,
            max_bytes: 8 * 1024 * 1024,
            max_output_tokens: 65_536,
            max_actions: 1,
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

fn request(fields: &[&str]) -> Value {
    json!({"schema":"dfmcp.query/1","query":{
        "kind":"aggregate","kinds":["job"],
        "group_by":{"kind":"fields","fields":fields}
    }})
}

fn totals_fixture() -> (WorldSnapshot, OperationContext) {
    world(
        (1..=12)
            .map(|number| {
                BTreeMap::from([
                    (
                        "workshop".to_owned(),
                        known(WorldValue::Entity(EntityId::new(100 + number % 3))),
                    ),
                    (
                        "suspended".to_owned(),
                        known(WorldValue::Bool(number % 2 == 0)),
                    ),
                    (
                        "stock".to_owned(),
                        known(WorldValue::U64(if number <= 2 {
                            u64::MAX
                        } else {
                            number
                        })),
                    ),
                ])
            })
            .collect(),
    )
}

fn refresh(snapshot: &mut WorldSnapshot, context: &mut OperationContext) {
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
}

#[test]
fn tuple_groups_match_independent_count_and_exact_integer_sum_oracle() -> TestResult {
    let (snapshot, context) = totals_fixture();
    let original = snapshot.clone();
    let mut input = request(&["workshop", "suspended"]);
    input["query"]["metrics"] =
        json!([{"name":"stock","field":"stock","numeric_type":"u64"}]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result, execute(&snapshot, &context, &input)?);
    let mut expected = BTreeMap::<(u64, bool), (u64, u128)>::new();
    for number in 1..=12u64 {
        let row = expected.entry((100 + number % 3, number % 2 == 0)).or_default();
        row.0 += 1;
        row.1 += u128::from(if number <= 2 { u64::MAX } else { number });
    }
    let mut actual = BTreeMap::new();
    for row in result["groups"].as_array().ok_or("groups")? {
        let fields = row["key"]["fields"].as_array().ok_or("tuple")?;
        assert_eq!(row["key"]["kind"], "fields");
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0]["field"], "workshop");
        assert_eq!(fields[0]["presence"], "known");
        assert_eq!(fields[0]["value"]["type"], "entity");
        assert_eq!(fields[1]["field"], "suspended");
        assert_eq!(fields[1]["value"]["type"], "bool");
        let key = (
            fields[0]["value"]["value"]
                .as_str()
                .ok_or("workshop")?
                .parse::<u64>()?,
            fields[1]["value"]["value"].as_bool().ok_or("suspended")?,
        );
        let count = row["count"].as_u64().ok_or("count")?;
        let metric = &row["metrics"]["stock"];
        let sum = metric["sum"].as_str().ok_or("sum")?.parse::<u128>()?;
        assert_eq!(metric["samples"], count);
        assert_eq!(metric["mean"]["numerator"], sum.to_string());
        assert_eq!(metric["mean"]["denominator"], count);
        assert!(actual.insert(key, (count, sum)).is_none());
    }
    assert_eq!(actual, expected);
    assert_eq!(result["matched"], 12);
    assert_eq!(result["metric_evaluations"], 12);
    assert_eq!(result["selection"]["complete_within_projection"], true);
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(snapshot, original);
    Ok(())
}

#[test]
fn tuple_components_preserve_scalar_types_scales_and_delimiter_boundaries() -> TestResult {
    let values = vec![
        (WorldValue::Null, json!({"type":"null"})),
        (WorldValue::Bool(false), json!({"type":"bool","value":false})),
        (WorldValue::I64(1), json!({"type":"i64","value":1})),
        (WorldValue::U64(1), json!({"type":"u64","value":1})),
        (
            WorldValue::Fixed { units: 1, scale: 0 },
            json!({"type":"fixed","value":{"units":1,"scale":0}}),
        ),
        (
            WorldValue::Fixed { units: 10, scale: 1 },
            json!({"type":"fixed","value":{"units":10,"scale":1}}),
        ),
        (WorldValue::Text("1".to_owned()), json!({"type":"text","value":"1"})),
        (
            WorldValue::Entity(EntityId::new(1)),
            json!({"type":"entity","value":"1"}),
        ),
        (
            WorldValue::Coord(MapCoord::new(1, 0, 0)),
            json!({"type":"coord","value":{"x":1,"y":0,"z":0}}),
        ),
    ];
    let rows = values
        .iter()
        .flat_map(|(value, _)| {
            (0..2).map(move |_| {
                BTreeMap::from([
                    ("a".to_owned(), known(value.clone())),
                    ("b".to_owned(), known(WorldValue::Text("same".to_owned()))),
                ])
            })
        })
        .collect();
    let (snapshot, context) = world(rows);
    let result = execute(&snapshot, &context, &request(&["a", "b"]))?;
    let actual: BTreeSet<_> = result["groups"]
        .as_array()
        .ok_or("groups")?
        .iter()
        .map(|row| {
            assert_eq!(row["count"], 2);
            row["key"]["fields"][0]["value"].to_string()
        })
        .collect();
    let expected: BTreeSet<_> = values.iter().map(|(_, value)| value.to_string()).collect();
    assert_eq!(actual, expected);

    let (snapshot, context) = world(
        [("a|b", "c"), ("a", "b|c"), ("a", ""), ("", "a")]
            .into_iter()
            .map(|(a, b)| {
                BTreeMap::from([
                    ("a".to_owned(), known(WorldValue::Text(a.to_owned()))),
                    ("b".to_owned(), known(WorldValue::Text(b.to_owned()))),
                ])
            })
            .collect(),
    );
    let result = execute(&snapshot, &context, &request(&["a", "b"]))?;
    assert_eq!(result["groups"].as_array().map(Vec::len), Some(4));
    assert!(
        result["groups"]
            .as_array()
            .ok_or("groups")?
            .iter()
            .all(|row| row["count"] == 1)
    );
    Ok(())
}

#[test]
fn unavailable_tuple_dimensions_stay_distinct_without_leaking_backing_values() -> TestResult {
    let anchor = totals_fixture().0.anchor();
    let variants = [
        ("absent", FactPresence::Absent),
        ("unknown", FactPresence::Unknown("hidden-reason-a".to_owned())),
        ("unknown", FactPresence::Unknown("hidden-reason-b".to_owned())),
        ("unsupported", FactPresence::Unsupported("hidden-version".to_owned())),
        ("omitted", FactPresence::Omitted("hidden-projection".to_owned())),
        ("redacted", FactPresence::Redacted("hidden-policy".to_owned())),
        ("stale", FactPresence::Stale(anchor)),
        ("contradicted", FactPresence::Known(WorldValue::Bool(true))),
    ];
    let mut rows = vec![BTreeMap::from([(
        "b".to_owned(),
        known(WorldValue::Text("same".to_owned())),
    )])];
    for (_, presence) in &variants {
        let mut unavailable = known(WorldValue::Text("concealed-backing-value".to_owned()));
        unavailable.presence = Some(presence.clone());
        rows.push(BTreeMap::from([
            ("a".to_owned(), unavailable),
            ("b".to_owned(), known(WorldValue::Text("same".to_owned()))),
        ]));
    }
    rows.push(BTreeMap::from([
        ("a".to_owned(), known(WorldValue::Null)),
        ("b".to_owned(), known(WorldValue::Text("same".to_owned()))),
    ]));
    let (snapshot, context) = world(rows);
    let result = execute(&snapshot, &context, &request(&["a", "b"]))?;
    let mut counts = BTreeMap::new();
    for row in result["groups"].as_array().ok_or("groups")? {
        let component = &row["key"]["fields"][0];
        let state = component["presence"]
            .as_str()
            .ok_or("presence")?
            .to_owned();
        let count = row["count"].as_u64().ok_or("count")?;
        if state == "known" {
            assert_eq!(component["value"], json!({"type":"null"}));
        } else {
            assert!(component["value"].is_null());
        }
        assert!(counts.insert(state, count).is_none());
    }
    let mut expected = BTreeMap::from([("unobserved".to_owned(), 1), ("known".to_owned(), 1)]);
    for (state, _) in variants {
        *expected.entry(state.to_owned()).or_default() += 1;
    }
    assert_eq!(counts, expected);
    assert_eq!(result["matched"], 10);
    // Unknown grouping values are represented groups, not unknown membership.
    assert_eq!(result["selection"]["excluded_unknown"], 0);
    assert_eq!(result["selection"]["complete_within_projection"], true);
    let encoded = serde_json::to_string(&result)?;
    assert!(!encoded.contains("concealed-backing-value"));
    assert!(!encoded.contains("hidden-"));
    Ok(())
}

#[test]
fn dimension_order_controls_lexicographic_groups_and_query_identity() -> TestResult {
    let (snapshot, context) = world(
        [(1, 2), (2, 1)]
            .into_iter()
            .map(|(a, b)| {
                BTreeMap::from([
                    ("a".to_owned(), known(WorldValue::U64(a))),
                    ("b".to_owned(), known(WorldValue::U64(b))),
                ])
            })
            .collect(),
    );
    let ab = execute(&snapshot, &context, &request(&["a", "b"]))?;
    let ba = execute(&snapshot, &context, &request(&["b", "a"]))?;
    assert_ne!(ab["query_digest"], ba["query_digest"]);
    assert_eq!(ab["groups"][0]["key"]["fields"][0]["field"], "a");
    assert_eq!(ab["groups"][0]["key"]["fields"][0]["value"]["value"], 1);
    assert_eq!(ab["groups"][0]["key"]["fields"][1]["value"]["value"], 2);
    assert_eq!(ba["groups"][0]["key"]["fields"][0]["field"], "b");
    assert_eq!(ba["groups"][0]["key"]["fields"][0]["value"]["value"], 1);
    assert_eq!(ba["groups"][0]["key"]["fields"][1]["value"]["value"], 2);
    let mut other = context.clone();
    other.session_id = SessionId::new(18);
    assert_ne!(
        ab["query_digest"],
        execute(&snapshot, &other, &request(&["a", "b"]))?["query_digest"]
    );
    let mut changed = snapshot.clone();
    let mut changed_context = context.clone();
    changed.paused = false;
    refresh(&mut changed, &mut changed_context);
    assert_ne!(
        ab["query_digest"],
        execute(&changed, &changed_context, &request(&["a", "b"]))?["query_digest"]
    );
    Ok(())
}

#[test]
fn tuple_grammar_is_closed_ordered_distinct_and_bounded() -> TestResult {
    let (snapshot, context) = totals_fixture();
    for malformed in [
        json!({"kind":"fields","fields":[]}),
        json!({"kind":"fields","fields":["a","b","c","d","e"]}),
        json!({"kind":"fields","fields":["a","a"]}),
        json!({"kind":"fields","fields":[""]}),
        json!({"kind":"fields","fields":["a".repeat(129)]}),
        json!({"kind":"fields","fields":["bad\0name"]}),
        json!({"kind":"fields"}),
        json!({"kind":"fields","fields":"a"}),
        json!({"kind":"fields","fields":["a",1]}),
        json!({"kind":"fields","fields":["a"],"field":"b"}),
        json!({"kind":"fields","fields":["a"],"unknown":"value"}),
    ] {
        let mut input = request(&["workshop"]);
        input["query"]["group_by"] = malformed;
        assert!(matches!(
            execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::InvalidRequest
        ), "{input}");
    }
    for fields in [
        vec!["workshop"],
        vec!["workshop", "suspended"],
        vec!["workshop", "suspended", "missing"],
        vec!["workshop", "suspended", "missing", "stock"],
    ] {
        let result = execute(&snapshot, &context, &request(&fields))?;
        for row in result["groups"].as_array().ok_or("groups")? {
            let actual: Vec<_> = row["key"]["fields"]
                .as_array()
                .ok_or("tuple")?
                .iter()
                .map(|field| field["field"].as_str().ok_or("field"))
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(actual, fields);
        }
    }
    Ok(())
}

#[test]
fn tuple_groups_follow_filter_relation_geometry_and_metric_availability() -> TestResult {
    let rows = (1..=8)
        .map(|number| {
            let mut fields = BTreeMap::from([
                ("eligible".to_owned(), known(WorldValue::Bool(number != 3))),
                ("workshop".to_owned(), known(WorldValue::Entity(EntityId::new(100)))),
                ("suspended".to_owned(), known(WorldValue::Bool(false))),
                ("stock".to_owned(), known(WorldValue::U64(number * 10))),
                (
                    "position".to_owned(),
                    known(WorldValue::Coord(MapCoord::new(if number == 7 { 9 } else { 1 }, 0, 0))),
                ),
            ]);
            if number == 2 {
                if let Some(stock) = fields.get_mut("stock") {
                    stock.presence = Some(FactPresence::Redacted("private".to_owned()));
                }
            }
            if number == 4 {
                if let Some(eligible) = fields.get_mut("eligible") {
                    eligible.presence = Some(FactPresence::Unknown("unread".to_owned()));
                }
            }
            if matches!(number, 3 | 4) {
                // A rejected source row must never reach scalar grouping.
                fields.insert("workshop".to_owned(), known(WorldValue::List(Vec::new())));
            }
            if number == 8 {
                fields.remove("position");
            }
            fields
        })
        .collect();
    let (mut snapshot, mut context) = world(rows);
    let target = EntityId::new(100);
    snapshot.graph.entities.insert(
        target,
        EntityRecord {
            id: target,
            generation: 1,
            revision: 1,
            kind: EntityKind::Building,
            label: "workshop".to_owned(),
            fields: BTreeMap::from([("active".to_owned(), known(WorldValue::Bool(true)))]),
        },
    );
    for (index, (from, to)) in [
        (1, 100),
        (1, 100),
        (1, 999),
        (2, 100),
        (3, 100),
        (4, 100),
        (5, 999),
        (7, 100),
        (8, 100),
    ]
    .into_iter()
    .enumerate()
    {
        let id = EdgeId::new(index as u128 + 1);
        snapshot.graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 1,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::new(),
            },
        );
    }
    refresh(&mut snapshot, &mut context);
    let mut input = request(&["workshop", "suspended"]);
    input["query"]["where"] = json!({"op":"compare","field":"eligible","comparison":"eq",
        "value":{"type":"bool","value":true}});
    input["query"]["relation"] = json!({"edge_kinds":["requires"],"target_kinds":["building"],
        "where":{"op":"compare","field":"active","comparison":"eq",
            "value":{"type":"bool","value":true}}});
    input["query"]["spatial"] = json!({"region":{"min":[0,0,0],"max":[2,0,0]},
        "geometry":{"kind":"point","field":"position"}});
    input["query"]["metrics"] = json!([{"name":"stock","field":"stock","numeric_type":"u64"}]);
    input["query"]["max_groups"] = json!(1);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["matched"], 2);
    assert_eq!(result["groups"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["groups"][0]["count"], 2);
    assert_eq!(result["groups"][0]["metrics"]["stock"]["samples"], 1);
    assert_eq!(result["groups"][0]["metrics"]["stock"]["sum"], "10");
    assert_eq!(result["groups"][0]["metrics"]["stock"]["excluded"]["redacted"], 1);
    assert_eq!(result["selection"]["selected_entities"], 8);
    assert_eq!(result["selection"]["matched_source_filter"], 6);
    assert_eq!(result["selection"]["excluded_false"], 1);
    assert_eq!(result["selection"]["excluded_unknown"], 1);
    assert_eq!(result["selection"]["relation_matched"], 4);
    assert_eq!(result["selection"]["unknown_relation"], 1);
    assert_eq!(result["selection"]["no_observed_match"], 1);
    assert_eq!(result["selection"]["matched_edge_incidences"], 5);
    assert_eq!(result["selection"]["missing_target_records"], 2);
    assert_eq!(result["selection"]["selected_edges"], 9);
    assert_eq!(result["selection"]["spatial_matched"], 2);
    assert_eq!(result["selection"]["outside_spatial_relation"], 1);
    assert_eq!(result["selection"]["unknown_geometry"], 1);
    assert_eq!(result["selection"]["complete_within_projection"], false);
    assert_eq!(result["coverage"]["totals_scope"], "selected_known_members_only");
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    Ok(())
}

#[test]
fn tuple_limits_refuse_whole_results_and_keep_source_budgets_separate() -> TestResult {
    let (snapshot, context) = totals_fixture();
    let before = snapshot.clone();
    let input = request(&["workshop", "suspended"]);
    let result = execute(&snapshot, &context, &input)?;
    let bytes = serde_json::to_vec(&result)?.len() as u64;
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &input, bytes)?,
        result
    );
    assert!(matches!(
        execute_with_output_budget(&snapshot, &context, &input, bytes - 1),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    let mut token_limited = context.clone();
    token_limited.budget.max_output_tokens = ((bytes - 1) / 4) as u32;
    assert!(matches!(
        execute(&snapshot, &token_limited, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    let mut groups_limited = input.clone();
    // Each dimension fits three values, but their six distinct pairs do not.
    groups_limited["query"]["max_groups"] = json!(3);
    assert!(matches!(
        execute(&snapshot, &context, &groups_limited),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("no partial summary")
    ));
    let mut empty = input.clone();
    empty["query"]["kinds"] = json!(["unit"]);
    let empty = execute(&snapshot, &context, &empty)?;
    assert_eq!(empty["matched"], 0);
    assert_eq!(empty["groups"], json!([]));
    assert_eq!(empty["truncated"], false);
    assert!(empty["continuation"].is_null());
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn tuple_queries_admit_complete_sources_and_charge_every_dimension() -> TestResult {
    let (mut snapshot, mut context) = totals_fixture();
    let mut input = request(&["workshop", "suspended"]);
    input["query"]["kinds"] = json!(["unit"]);
    let id = EdgeId::new(1);
    snapshot.graph.edges.insert(
        id,
        EdgeRecord {
            id,
            revision: 1,
            kind: EdgeKind::Requires,
            from: EntityId::new(1),
            to: EntityId::new(2),
            fields: BTreeMap::new(),
        },
    );
    refresh(&mut snapshot, &mut context);
    context.budget.max_entities = snapshot.graph.entities.len() as u32;
    assert!(matches!(
        execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("complete-source record")
    ));
    context.budget.max_entities = 100_000;
    let mut nested = WorldValue::Null;
    for _ in 0..66 {
        nested = WorldValue::List(vec![nested]);
    }
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity")?
        .fields
        .insert("unselected".to_owned(), known(nested));
    refresh(&mut snapshot, &mut context);
    assert!(matches!(
        execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("nesting")
    ));

    let (snapshot, context) = totals_fixture();
    let one = execute(&snapshot, &context, &request(&["workshop"]))?;
    let four = execute(
        &snapshot,
        &context,
        &request(&["workshop", "suspended", "stock", "missing"]),
    )?;
    assert!(
        four["source_cost"]["estimated_scan_work_units"].as_u64().ok_or("four work")?
            > one["source_cost"]["estimated_scan_work_units"].as_u64().ok_or("one work")?
    );
    assert_eq!(one["matched"], four["matched"]);
    Ok(())
}

#[test]
fn unavailable_dimensions_do_not_hide_authority_or_invalid_scalar_failures() -> TestResult {
    let (snapshot, context) = totals_fixture();
    let input = request(&["missing", "workshop"]);
    for restriction in 0..5 {
        let mut denied = context.clone();
        match restriction {
            0 => denied.grants.clear(),
            1 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            2 => denied.grants[0].expires_at_tick = Some(GameTick(9)),
            3 => denied.grants[0].remaining_uses = Some(0),
            _ => denied.cancellation_requested = true,
        }
        assert!(execute(&snapshot, &denied, &input).is_err());
    }
    for value in [
        WorldValue::List(Vec::new()),
        WorldValue::Text("x".repeat(257)),
        WorldValue::Text("é".repeat(129)),
    ] {
        let (snapshot, context) = world(vec![BTreeMap::from([("a".to_owned(), known(value))])]);
        assert!(matches!(
            execute(&snapshot, &context, &request(&["a", "missing"])),
            Err(error) if error.code == ErrorCode::BudgetExceeded
        ));
    }
    let (snapshot, context) = world(vec![BTreeMap::from([(
        "a".to_owned(), known(WorldValue::Text("é".repeat(128))),
    )])]);
    let result = execute(&snapshot, &context, &request(&["a", "missing"]))?;
    assert_eq!(
        result["groups"][0]["key"]["fields"][0]["value"]["value"],
        "é".repeat(128)
    );
    assert_eq!(
        result["groups"][0]["key"]["fields"][1]["presence"],
        "unobserved"
    );
    Ok(())
}

#[test]
fn legacy_single_field_and_entity_kind_outputs_keep_their_original_shape() -> TestResult {
    let (snapshot, context) = totals_fixture();
    let mut input = request(&["suspended"]);
    input["query"]["group_by"] = json!({"kind":"field","field":"suspended"});
    let legacy = execute(&snapshot, &context, &input)?;
    assert_eq!(legacy["groups"], json!([
        {"key":{"kind":"field","presence":"known","value":{"type":"bool","value":false}},
         "count":6,"metrics":{}},
        {"key":{"kind":"field","presence":"known","value":{"type":"bool","value":true}},
         "count":6,"metrics":{}}
    ]));
    assert!(legacy.get("selection").is_none());
    assert!(legacy.get("source_cost").is_none());
    input["query"]["group_by"] = json!({"kind":"entity_kind"});
    let kinds = execute(&snapshot, &context, &input)?;
    assert_eq!(kinds["groups"], json!([
        {"key":{"kind":"entity_kind","value":"job"},"count":12,"metrics":{}}
    ]));
    assert!(kinds.get("selection").is_none());
    assert!(kinds.get("source_cost").is_none());
    input["query"]["group_by"] = Value::Null;
    let all = execute(&snapshot, &context, &input)?;
    assert_eq!(all["groups"], json!([
        {"key":{"kind":"all"},"count":12,"metrics":{}}
    ]));
    assert!(all.get("selection").is_none());
    assert!(all.get("source_cost").is_none());
    Ok(())
}

#[test]
fn future_dimensions_preserve_membership_and_exclude_values_and_metric_samples() -> TestResult {
    let rows = [(9, 5), (10, 7), (11, 9_876_543_210_123_456_789u64)]
        .into_iter()
        .map(|(tick, value)| {
            let mut stock = known(WorldValue::U64(value));
            stock.observed_at = GameTick(tick);
            BTreeMap::from([
                ("material".to_owned(), known(WorldValue::Text("WOOD".to_owned()))),
                ("stock".to_owned(), stock),
            ])
        })
        .collect();
    let (snapshot, context) = world(rows);
    let mut input = request(&["material", "stock"]);
    input["query"]["metrics"] =
        json!([{"name":"stock","field":"stock","numeric_type":"u64"}]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["matched"], 3);
    assert_eq!(result["selection"]["excluded_unknown"], 0);
    assert_eq!(result["selection"]["complete_within_projection"], true);
    let rows = result["groups"].as_array().ok_or("groups")?;
    assert_eq!(rows.len(), 3);
    let mut past_and_equal = BTreeSet::new();
    let mut future_count = 0;
    for row in rows {
        assert_eq!(row["count"], 1);
        assert_eq!(
            row["key"]["fields"][0],
            json!({"field":"material","presence":"known",
                "value":{"type":"text","value":"WOOD"}})
        );
        let component = &row["key"]["fields"][1];
        let metric = &row["metrics"]["stock"];
        if component["presence"] == "future" {
            future_count += 1;
            assert!(component["value"].is_null());
            assert_eq!(metric["samples"], 0);
            assert_eq!(metric["excluded"]["future"], 1);
            assert!(metric["sum"].is_null());
            assert!(metric["minimum"].is_null());
            assert!(metric["mean"].is_null());
        } else {
            assert_eq!(component["presence"], "known");
            assert_eq!(component["value"]["type"], "u64");
            let value = component["value"]["value"].as_u64().ok_or("stock")?;
            past_and_equal.insert(value);
            assert_eq!(metric["samples"], 1);
            assert_eq!(metric["sum"], value.to_string());
        }
    }
    assert_eq!(future_count, 1);
    assert_eq!(past_and_equal, BTreeSet::from([5, 7]));
    assert!(!serde_json::to_string(&result)?.contains("9876543210123456789"));
    Ok(())
}
