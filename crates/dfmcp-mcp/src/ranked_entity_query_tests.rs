use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{EntityKind, Fact, FactPresence, FactSource, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField("ranking-fixture".to_owned()),
        Digest32::of_bytes(b"ranked-query-test"),
    )
}

fn fixture(values: &[(i64, u64)]) -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for (index, (stress, age)) in values.iter().enumerate() {
        let id = EntityId::new(index as u64 + 1);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 7,
                revision: 9,
                kind: EntityKind::Unit,
                label: format!("Urist {id}"),
                fields: BTreeMap::from([
                    ("stress".to_owned(), known(WorldValue::I64(*stress))),
                    ("age".to_owned(), known(WorldValue::U64(*age))),
                    ("alive".to_owned(), known(WorldValue::Bool(true))),
                    (
                        "score".to_owned(),
                        known(WorldValue::Fixed {
                            units: index as i64 % 3 - 1,
                            scale: 2,
                        }),
                    ),
                ]),
            },
        );
    }
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
            max_entities: 10_000,
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
    json!({"schema":"dfmcp.query/1","query":{"kind":"ranked_entities","kinds":["unit"],
        "order_by":[
            {"field":"stress","numeric_type":"i64","direction":"descending"},
            {"field":"age","numeric_type":"u64","direction":"ascending"}
        ],"limit":128}})
}

fn ids(value: &Value) -> Vec<String> {
    value["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["entity_id"].as_str().map(str::to_owned))
        .collect()
}

fn refresh(snapshot: &mut WorldSnapshot, context: &mut OperationContext) {
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
}

#[test]
fn mixed_keys_preserve_exact_order_ties_and_ranking_provenance() -> TestResult {
    let (snapshot, context) = fixture(&[(5, 20), (5, 10), (-8, 1), (100, 99), (5, 10)]);
    let result = execute(&snapshot, &context, &request())?;
    assert_eq!(ids(&result), ["4", "2", "5", "1", "3"]);
    assert_eq!(result["matched"], 5);
    assert_eq!(result["rows"][0]["generation"], 7);
    assert_eq!(result["rows"][0]["fields"], json!({}));
    let fact = &result["rows"][0]["ranking"]["facts"]["stress"];
    assert_eq!(fact["value"], json!({"type":"i64","value":100}));
    assert_eq!(fact["presence"], "known");
    assert_eq!(fact["epistemic_state"], "observed");
    assert_eq!(fact["observed_at_game_tick"], 10);
    assert_eq!(
        fact["source_digest"],
        Digest32::of_bytes(b"ranked-query-test").to_string()
    );
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(
        result["coverage"]["selection_complete_within_projection"],
        true
    );
    Ok(())
}

#[test]
fn exhaustive_small_three_key_orders_match_an_independent_full_sort() -> TestResult {
    // Every four-record assignment of the two binary primary dimensions,
    // all eight direction combinations, ties, and two bounded page widths.
    for encoded in 0..256usize {
        let values = (0..4)
            .map(|index| {
                let value = (encoded >> (2 * index)) & 3;
                ((value & 1) as i64 * 2 - 1, (value >> 1) as u64)
            })
            .collect::<Vec<_>>();
        let (snapshot, context) = fixture(&values);
        for directions in 0..8 {
            let mut input = request();
            input["query"]["order_by"] = json!([
                {"field":"stress","numeric_type":"i64","direction":if directions & 1 == 0 {"ascending"} else {"descending"}},
                {"field":"age","numeric_type":"u64","direction":if directions & 2 == 0 {"ascending"} else {"descending"}},
                {"field":"score","numeric_type":"fixed","scale":2,"direction":if directions & 4 == 0 {"ascending"} else {"descending"}}
            ]);
            let mut expected = (0..4usize).collect::<Vec<_>>();
            expected.sort_by(|a, b| {
                let comparisons = [
                    values[*a].0.cmp(&values[*b].0),
                    values[*a].1.cmp(&values[*b].1),
                    (*a % 3).cmp(&(*b % 3)),
                ];
                for (key, comparison) in comparisons.into_iter().enumerate() {
                    let comparison = if directions & (1 << key) == 0 {
                        comparison
                    } else {
                        comparison.reverse()
                    };
                    if comparison != Ordering::Equal {
                        return comparison;
                    }
                }
                a.cmp(b)
            });
            for width in [1, 3] {
                input["query"]["limit"] = json!(width);
                let result = execute(&snapshot, &context, &input)?;
                assert_eq!(
                    ids(&result),
                    expected[..width]
                        .iter()
                        .map(|id| (id + 1).to_string())
                        .collect::<Vec<_>>(),
                    "values={values:?}, directions={directions}, width={width}"
                );
                assert_eq!(result["source_cost"]["maximum_retained_candidates"], width);
            }
        }
    }
    Ok(())
}

#[test]
fn signed_unsigned_and_fixed_extrema_are_not_rounded_or_negated() -> TestResult {
    let (mut snapshot, mut context) = fixture(&[
        (i64::MIN, 0),
        (i64::MAX, u64::MAX),
        (i64::MAX - 1, u64::MAX - 1),
        (0, 1),
    ]);
    assert_eq!(
        ids(&execute(&snapshot, &context, &request())?),
        ["2", "3", "4", "1"]
    );
    let mut input = request();
    input["query"]["order_by"] = json!([
        {"field":"age","numeric_type":"u64","direction":"descending"}
    ]);
    assert_eq!(
        ids(&execute(&snapshot, &context, &input)?),
        ["2", "3", "4", "1"]
    );
    for (id, units, scale) in [(1, i64::MIN, 2), (2, i64::MAX, 2), (3, 0, 3), (4, -1, 2)] {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(id))
            .ok_or("unit")?
            .fields
            .insert("score".into(), known(WorldValue::Fixed { units, scale }));
    }
    refresh(&mut snapshot, &mut context);
    input["query"]["order_by"] = json!([
        {"field":"score","numeric_type":"fixed","scale":2,"direction":"ascending"}
    ]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), ["1", "4", "2"]);
    assert_eq!(

        result["selection"]["unknown_rank_reasons"]["scale_mismatch"],
        1
    );
    input["query"]["order_by"][0]["numeric_type"] = json!("i64");
    input["query"]["order_by"][0].as_object_mut().ok_or("key")?.remove("scale");
    let mismatched = execute(&snapshot, &context, &input)?;
    assert_eq!(mismatched["matched"], 0);
    assert_eq!(mismatched["selection"]["unknown_rank"], 4);
    Ok(())
}

#[test]
fn unavailable_future_old_and_mistyped_keys_stay_unknown_without_leaking_values() -> TestResult {
    let (mut snapshot, mut context) = fixture(&vec![(5, 1); 13]);
    let anchor = snapshot.anchor();
    snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or("unit")?
        .fields.remove("stress");
    for (index, presence) in [
        FactPresence::Absent,
        FactPresence::Unknown("unknown".to_owned()),
        FactPresence::Unsupported("unsupported".to_owned()),
        FactPresence::Omitted("omitted".to_owned()),
        FactPresence::Redacted("redacted".to_owned()),
        FactPresence::Stale(anchor),
        FactPresence::Known(WorldValue::I64(-1000)),
    ].into_iter().enumerate() {
        let mut fact = known(WorldValue::Text("hidden-rank-payload".to_owned()));
        fact.presence = Some(presence);
        snapshot.graph.entities.get_mut(&EntityId::new(index as u64 + 3))
            .ok_or("unit")?.fields.insert("stress".to_owned(), fact);
    }
    for (id, tick) in [(10, 11), (11, 0)] {
        snapshot.graph.entities.get_mut(&EntityId::new(id)).ok_or("unit")?
            .fields.get_mut("stress").ok_or("fact")?.observed_at = GameTick(tick);
    }
    for (id, value) in [(12, WorldValue::U64(5)), (13, WorldValue::Null)] {
        snapshot.graph.entities.get_mut(&EntityId::new(id)).ok_or("unit")?
            .fields.insert("stress".to_owned(), known(value));
    }
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["order_by"][0]["max_age_ticks"] = json!(2);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), ["1"]);
    assert_eq!(result["selection"]["unknown_rank"], 12);
    assert_eq!(
        result["selection"]["unknown_rank_reasons"],
        json!({"missing":1,"unavailable":7,"future":1,"too_old":1,"type_mismatch":2})
    );
    assert_eq!(result["coverage"]["selection_complete_within_projection"], false);
    assert!(!result.to_string().contains("hidden-rank-payload"));
    Ok(())
}

#[test]
fn filter_negation_preserves_unknown_separately_from_false_and_unusable_ranks() -> TestResult {
    let (mut snapshot, mut context) = fixture(&[(3, 1), (2, 1), (1, 1)]);
    let second = snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or("unit")?;
    second.fields.insert("alive".to_owned(), known(WorldValue::Bool(false)));
    second.fields.remove("stress");
    snapshot.graph.entities.get_mut(&EntityId::new(3)).ok_or("unit")?
        .fields.get_mut("alive").ok_or("fact")?.presence =
        Some(FactPresence::Unknown("not observed".to_owned()));
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["where"] = json!({"op":"not","arg":{
        "op":"compare","field":"alive","comparison":"eq","value":{"type":"bool","value":true}
    }});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["matched"], 0);
    assert_eq!(result["selection"]["excluded_filter_false"], 1);
    assert_eq!(result["selection"]["excluded_filter_unknown"], 1);
    assert_eq!(result["selection"]["unknown_rank"], 1);
    assert_eq!(result["coverage"]["selection_complete_within_projection"], false);
    Ok(())
}

#[test]
fn pages_resume_exact_ranks_when_width_changes_and_ties_cross_pages() -> TestResult {
    let (snapshot, context) = fixture(&[(5,20),(5,10),(-8,1),(100,99),(5,10),(5,10)]);
    let mut input = request();
    let mut actual = Vec::new();
    let mut digest = None;
    for width in [1, 3, 2] {
        input["query"]["limit"] = json!(width);
        let page = execute(&snapshot, &context, &input)?;
        if let Some(previous) = &digest {
            assert_eq!(&page["query_digest"], previous);
        }
        digest = Some(page["query_digest"].clone());
        actual.extend(ids(&page));
        input["query"]["continuation"] = page["continuation"].clone();
    }
    assert_eq!(actual, ["4","2","5","6","1","3"]);
    assert!(input["query"]["continuation"].is_null());
    Ok(())
}

#[test]
fn continuations_bind_every_sort_selector_projection_source_and_session() -> TestResult {
    let (snapshot, context) = fixture(&[(3,1),(2,1),(1,1)]);
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    input["query"]["continuation"] = first["continuation"].clone();
    let changes = [
        ("direction", json!("ascending")),
        ("field", json!("score")),
        ("max_age_ticks", json!(10)),
    ];
    for (name, value) in changes {
        let mut changed = input.clone();
        changed["query"]["order_by"][0][name] = value;
        assert!(matches!(execute(&snapshot, &context, &changed),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    let mut changes = Vec::new();
    let mut fixed = input.clone();
    fixed["query"]["order_by"][0] =
        json!({"field":"stress","numeric_type":"fixed","scale":2,"direction":"descending"});
    changes.push(fixed);
    let mut unsigned = input.clone();
    unsigned["query"]["order_by"][0]["numeric_type"] = json!("u64");
    changes.push(unsigned);
    let mut reversed = input.clone();
    reversed["query"]["order_by"].as_array_mut().ok_or("keys")?.reverse();
    changes.push(reversed);
    for (name, value) in [
        ("fields", json!(["alive"])),
        ("kinds", json!(["item"])),
        ("where", json!({"op":"all","args":[]})),
    ] {
        let mut changed = input.clone();
        changed["query"][name] = value;
        changes.push(changed);
    }
    for changed in changes {
        assert!(matches!(execute(&snapshot, &context, &changed),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    let mut other = context.clone();
    other.session_id = SessionId::new(6);
    assert!(matches!(execute(&snapshot, &other, &input),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    let mut source = snapshot.clone();
    source.graph.entities.get_mut(&EntityId::new(1)).ok_or("unit")?.generation += 1;
    refresh(&mut source, &mut other);
    other.session_id = context.session_id;
    assert!(matches!(execute(&source, &other, &input),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn edited_missing_ineligible_and_final_cursor_targets_refuse() -> TestResult {
    let (mut snapshot, mut context) = fixture(&[(3,1),(2,1),(1,1),(0,1)]);
    snapshot.graph.entities.get_mut(&EntityId::new(4)).ok_or("unit")?
        .fields.remove("stress");
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    let token = first["continuation"].as_str().ok_or("cursor")?;
    let parts = token.split(':').collect::<Vec<_>>();
    for (id, generation) in [(99,7),(4,7),(3,7),(1,8)] {
        let body = format!("rk1:{}:{id}:{generation}", parts[1]);
        input["query"]["continuation"] = json!(format!(
            "{body}:{}", Digest32::of_bytes(body.as_bytes())
        ));
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::CursorGap));
    }
    let mut edited = token.to_owned();
    edited.pop();
    edited.push(if token.ends_with('0') { '1' } else { '0' });
    input["query"]["continuation"] = json!(edited);
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn authority_cancellation_source_hash_and_complete_source_budgets_are_enforced() -> TestResult {
    let (snapshot, context) = fixture(&[(3,1),(2,1),(1,1)]);
    for mutation in 0..5 {
        let mut denied = context.clone();
        match mutation {
            0 => denied.grants.clear(),
            1 => denied.cancellation_requested = true,
            2 => denied.grants[0].expires_at_tick = Some(GameTick(9)),
            3 => denied.budget.max_entities = 1,
            _ => denied.budget.max_wall_millis = 0,
        }
        let mut input = request();
        input["query"]["limit"] = json!(1);
        assert!(execute(&snapshot, &denied, &input).is_err());
    }
    let mut corrupt = snapshot.clone();
    corrupt.graph.entities.get_mut(&EntityId::new(1)).ok_or("unit")?.label.push('x');
    assert!(matches!(execute(&corrupt, &context, &request()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    let mut huge = snapshot.clone();
    huge.graph.entities.get_mut(&EntityId::new(3)).ok_or("unit")?
        .fields.insert("unselected".to_owned(), known(WorldValue::Text("x".repeat(100_000))));
    let mut small = context.clone();
    refresh(&mut huge, &mut small);
    small.budget.max_bytes = 8192;
    let mut input = request();
    input["query"]["limit"] = json!(1);
    input["query"]["where"] = json!({
        "op":"compare","field":"stress","comparison":"eq","value":{"type":"i64","value":3}
    });
    assert!(matches!(execute(&huge, &small, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn whole_row_paging_never_skips_an_oversized_next_rank() -> TestResult {
    let (mut snapshot, mut context) = fixture(&[(3,1),(2,1),(1,1)]);
    for id in 1..=3 {
        snapshot.graph.entities.get_mut(&EntityId::new(id)).ok_or("unit")?
            .fields.insert("detail".to_owned(), known(WorldValue::Text(
                if id == 2 { "x".repeat(20_000) } else { "small".to_owned() }
            )));
    }
    refresh(&mut snapshot, &mut context);
    // Source admission is independent of the much smaller output-token budget.
    context.budget.max_output_tokens = 2048;
    let mut input = request();
    input["query"]["limit"] = json!(3);
    input["query"]["fields"] = json!(["detail"]);
    let first = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&first), ["1"]);
    assert_eq!(first["truncated"], true);
    input["query"]["continuation"] = first["continuation"].clone();
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    context.budget.max_output_tokens = 32_768;
    let resumed = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&resumed), ["2","3"]);
    assert!(resumed["continuation"].is_null());
    Ok(())
}

#[test]
fn closed_numeric_order_grammar_rejects_ambiguous_or_unsupported_keys() -> TestResult {
    let (snapshot, context) = fixture(&[(1,1)]);
    for keys in [
        json!([]),
        json!([{"field":"stress","numeric_type":"f64","direction":"ascending"}]),
        json!([{"field":"score","numeric_type":"fixed","direction":"ascending"}]),
        json!([{"field":"stress","numeric_type":"i64","direction":"ascending","scale":0}]),
        json!([{"field":"","numeric_type":"i64","direction":"ascending"}]),
        json!([{"field":"stress","numeric_type":"i64","direction":"up"}]),
        json!([{"field":"stress","numeric_type":"i64","direction":"ascending","unknown":true}]),
        json!([
            {"field":"stress","numeric_type":"i64","direction":"ascending"},
            {"field":"stress","numeric_type":"i64","direction":"descending"}
        ]),
    ] {
        let mut input = request();
        input["query"]["order_by"] = keys;
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::InvalidRequest));
    }
    for limit in [0,129] {
        let mut input = request();
        input["query"]["limit"] = json!(limit);
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::InvalidRequest));
    }
    let mut input = request();
    input["query"]["unexpected"] = json!(true);
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::InvalidRequest));
    Ok(())
}

#[test]
fn equal_canonical_kind_encodings_and_selector_sets_keep_identical_results() -> TestResult {
    let (snapshot, context) = fixture(&[(3,1),(2,1),(1,1)]);
    let mut equivalent = snapshot.clone();
    for entity in equivalent.graph.entities.values_mut() {
        entity.kind = EntityKind::Other("unit".to_owned());
    }
    assert_eq!(snapshot.compute_hash(), equivalent.compute_hash());
    let original = execute(&snapshot, &context, &request())?;
    assert_eq!(original, execute(&equivalent, &context, &request())?);
    let mut aliases = request();
    aliases["query"]["kinds"] = json!(["unit","unit"]);
    assert_eq!(original, execute(&snapshot, &context, &aliases)?);
    Ok(())
}

#[test]
fn derived_and_agent_asserted_rank_facts_keep_their_epistemic_classes() -> TestResult {
    let (mut snapshot, mut context) = fixture(&[(3,1),(2,1)]);
    for (id, source) in [
        (1, FactSource::Derived("advisory".to_owned())),
        (2, FactSource::AgentAssertion("untrusted".to_owned())),
    ] {
        snapshot.graph.entities.get_mut(&EntityId::new(id)).ok_or("unit")?
            .fields.get_mut("stress").ok_or("fact")?.source = source;
    }
    refresh(&mut snapshot, &mut context);
    let result = execute(&snapshot, &context, &request())?;
    for (index, epistemic) in ["inferred","assumed"].into_iter().enumerate() {
        assert_eq!(result["rows"][index]["ranking"]["facts"]["stress"]["epistemic_state"], epistemic);
        assert_eq!(result["rows"][index]["ranking"]["eligible_as_mutation_evidence"], false);
    }
    Ok(())
}

#[test]
fn large_population_retains_only_the_requested_heap_page() -> TestResult {
    let values = (0..4096u64)
        .map(|id| (((id * 7919) % 997) as i64 - 500, (id * 13) % 17))
        .collect::<Vec<_>>();
    let (snapshot, mut context) = fixture(&values);
    context.budget.max_bytes = 16 * 1024 * 1024;
    let mut input = request();
    input["query"]["limit"] = json!(7);
    let result = execute(&snapshot, &context, &input)?;
    let mut expected = (0..values.len()).collect::<Vec<_>>();
    expected.sort_by(|a, b| {
        values[*b]
            .0
            .cmp(&values[*a].0)
            .then_with(|| values[*a].1.cmp(&values[*b].1))
            .then_with(|| a.cmp(b))
    });
    assert_eq!(
        ids(&result),
        expected[..7]
            .iter()
            .map(|id| (id + 1).to_string())
            .collect::<Vec<_>>()
    );
    assert_eq!(result["matched"], 4096);
    assert_eq!(result["source_cost"]["maximum_retained_candidates"], 7);
    Ok(())
}

#[test]
fn reserved_output_keeps_complete_source_admission_independent() -> TestResult {
    let (snapshot, context) = fixture(&vec![(3, 1); 100]);
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let maximum_output_bytes = 8192;
    let result = execute_with_output_budget(
        &snapshot,
        &context,
        &input,
        maximum_output_bytes,
    )?;
    assert_eq!(result["matched"], 100);
    assert_eq!(result["returned"], 1);
    assert_eq!(ids(&result), ["1"]);
    assert!(
        result["source_cost"]["bounded_bytes"].as_u64().ok_or("source bytes")?
            > maximum_output_bytes
    );
    assert!(serde_json::to_vec(&result)?.len() as u64 <= maximum_output_bytes);

    let mut narrowed = context.clone();
    narrowed.budget.max_bytes = maximum_output_bytes;
    assert!(matches!(
        execute(&snapshot, &narrowed, &input),
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

#[path = "ranked_relation_query_tests.rs"]
mod relations;
