//! Compound inventory/workforce summaries over one immutable admitted source.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, MapCoord, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{EdgeKind, EdgeRecord, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField("aggregate-fixture".to_owned()),
        Digest32::of_bytes(b"compound-aggregate"),
    )
}

fn refresh(snapshot: &mut WorldSnapshot, context: &mut OperationContext) {
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
}

fn add_edge(graph: &mut WorldGraph, id: u128, from: u64, to: u64, kind: EdgeKind) {
    let id = EdgeId::new(id);
    graph.edges.insert(
        id,
        EdgeRecord {
            id,
            revision: 3,
            kind,
            from: EntityId::new(from),
            to: EntityId::new(to),
            fields: BTreeMap::new(),
        },
    );
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for number in 1..=9 {
        let id = EntityId::new(number);
        let mut fields = BTreeMap::from([
            ("eligible".to_owned(), known(WorldValue::Bool(number != 4))),
            (
                "category".to_owned(),
                known(WorldValue::Text("food".to_owned())),
            ),
            (
                "stock".to_owned(),
                known(WorldValue::U64(if number == 1 {
                    u64::MAX
                } else {
                    number * 10
                })),
            ),
            (
                "position".to_owned(),
                known(WorldValue::Coord(MapCoord::new(
                    if number == 2 { 50 } else { 1 },
                    0,
                    0,
                ))),
            ),
        ]);
        if number == 3 {
            fields.remove("position");
        }
        if number == 8 {
            fields.insert(
                "eligible".to_owned(),
                Fact::with_presence(
                    FactPresence::Omitted("not observed".to_owned()),
                    GameTick(10),
                    FactSource::Replay,
                    Digest32::ZERO,
                ),
            );
        }
        if number == 9 {
            let mut stock = known(WorldValue::U64(987_654_321));
            stock.presence = Some(FactPresence::Redacted("inventory policy".to_owned()));
            fields.insert("stock".to_owned(), stock);
        }
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 2,
                revision: number,
                kind: EntityKind::Item,
                label: format!("stack {number}"),
                fields,
            },
        );
    }
    for number in [100, 101] {
        let id = EntityId::new(number);
        let mut active = known(WorldValue::Bool(true));
        if number == 101 {
            active.presence = Some(FactPresence::Unknown("unread target".to_owned()));
        }
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 4,
                revision: 7,
                kind: EntityKind::Stockpile,
                label: format!("stockpile {number}"),
                // Targets deliberately have no geometry; the selector applies to
                // contributing stacks, not to their related stockpile records.
                fields: BTreeMap::from([("active".to_owned(), active)]),
            },
        );
    }
    for (index, (from, to)) in [
        (1, 100),
        (1, 100),
        (1, 999),
        (2, 100),
        (3, 100),
        (4, 100),
        (5, 101),
        (6, 999),
        (8, 100),
        (9, 100),
    ]
    .into_iter()
    .enumerate()
    {
        add_edge(
            &mut graph,
            index as u128 + 1,
            from,
            to,
            EdgeKind::ContainedIn,
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
            max_game_ticks: 1000,
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

fn boolean(field: &str) -> Value {
    json!({"op":"compare","field":field,"comparison":"eq",
        "value":{"type":"bool","value":true}})
}

fn request() -> Value {
    json!({"schema":"dfmcp.query/1","query":{
        "kind":"aggregate","kinds":["item"],"where":boolean("eligible"),
        "relation":{"edge_kinds":["contained_in"],"target_kinds":["stockpile"],
            "where":boolean("active")},
        "spatial":{"region":{"min":[0,0,0],"max":[2,0,0]},
            "geometry":{"kind":"point","field":"position"},"relation":"within"},
        "group_by":{"kind":"field","field":"category"},
        "metrics":[{"name":"stock","field":"stock","numeric_type":"u64"}]
    }})
}

fn remove(query: &mut Value, name: &str) -> std::result::Result<(), Box<dyn std::error::Error>> {
    query["query"]
        .as_object_mut()
        .ok_or("query object missing")?
        .remove(name);
    Ok(())
}

#[test]
fn compound_aggregate_selects_before_grouping_without_multiplying_stock_by_edges() -> TestResult {
    let (snapshot, context) = fixture();
    let before = snapshot.anchor();
    let input = request();
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result, execute(&snapshot, &context, &input)?);
    assert_eq!(result["matched"], 2);
    assert_eq!(result["groups"].as_array().map(Vec::len), Some(1));
    assert_eq!(result["groups"][0]["key"]["value"]["value"], "food");
    assert_eq!(result["groups"][0]["count"], 2);
    let metric = &result["groups"][0]["metrics"]["stock"];
    assert_eq!(metric["samples"], 1);
    assert_eq!(metric["sum"], u64::MAX.to_string());
    assert_eq!(metric["excluded"]["redacted"], 1);
    assert_eq!(metric["sources"]["observed"], 1);
    assert_eq!(result["selection"]["selected_entities"], 9);
    assert_eq!(result["selection"]["matched_source_filter"], 7);
    assert_eq!(result["selection"]["excluded_false"], 1);
    assert_eq!(result["selection"]["excluded_unknown"], 1);
    assert_eq!(result["selection"]["relation_matched"], 4);
    assert_eq!(result["selection"]["unknown_relation"], 2);
    assert_eq!(result["selection"]["no_observed_match"], 1);
    assert_eq!(result["selection"]["matched_edge_incidences"], 5);
    assert_eq!(result["selection"]["selected_edges"], 10);
    assert_eq!(result["selection"]["missing_target_records"], 2);
    assert_eq!(result["selection"]["unknown_target_filters"], 1);
    assert_eq!(result["selection"]["spatial_matched"], 2);
    assert_eq!(result["selection"]["outside_spatial_relation"], 1);
    assert_eq!(result["selection"]["unknown_geometry"], 1);
    assert_eq!(
        result["selection"]["unknown_geometry_reasons"]["missing_field"],
        1
    );
    assert_eq!(result["selection"]["complete_within_projection"], false);
    assert_eq!(
        result["coverage"]["selection_complete_within_projection"],
        false
    );
    assert_eq!(
        result["coverage"]["totals_scope"],
        "selected_known_members_only"
    );
    assert_eq!(result["coverage"]["world_domain_complete"], false);
    assert_eq!(result["coverage"]["relation_domain_complete"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(result["coverage"]["terrain_evaluated"], false);
    assert_eq!(result["coverage"]["accessibility_evaluated"], false);
    assert_eq!(result["source_cost"]["relation_target_records"], 11);
    assert!(result["relation"].get("witnesses_per_row_limit").is_none());
    assert!(!result.to_string().contains("987654321"));
    assert_eq!(snapshot.anchor(), before);
    assert!(snapshot.hash_is_valid());
    Ok(())
}

#[test]
fn either_selector_works_alone_and_plain_aggregate_shape_is_preserved() -> TestResult {
    let (snapshot, context) = fixture();
    let mut related = request();
    remove(&mut related, "spatial")?;
    let related = execute(&snapshot, &context, &related)?;
    assert_eq!(related["matched"], 4);
    assert_eq!(
        related["groups"][0]["metrics"]["stock"]["sum"],
        "18446744073709551665"
    );
    assert!(related.get("spatial").is_none());
    assert!(related["selection"].get("unknown_geometry").is_none());

    let mut spatial = request();
    remove(&mut spatial, "relation")?;
    let spatial = execute(&snapshot, &context, &spatial)?;
    assert_eq!(spatial["matched"], 5);
    assert_eq!(spatial["selection"]["unknown_geometry"], 1);
    assert_eq!(spatial["selection"]["outside_spatial_relation"], 1);
    assert!(spatial.get("relation").is_none());
    assert!(spatial["selection"].get("unknown_relation").is_none());

    let mut plain = request();
    remove(&mut plain, "relation")?;
    remove(&mut plain, "spatial")?;
    remove(&mut plain, "where")?;
    let prior = execute(&snapshot, &context, &plain)?;
    assert_eq!(prior["matched"], 9);
    assert!(prior.get("selection").is_none());
    assert!(prior.get("source_cost").is_none());
    assert_eq!(
        prior["coverage"],
        json!({"domain":"observed_projection","absence_proven":false,
        "note":"counts and matches describe the supplied projection, not complete fortress history or unobserved domains"})
    );
    plain["query"]["relation"] = Value::Null;
    plain["query"]["spatial"] = Value::Null;
    let null = execute(&snapshot, &context, &plain)?;
    // The existing request-based identity distinguishes explicit null fields;
    // no-selector execution and response data retain their established shape.
    for key in [
        "groups",
        "matched",
        "coverage",
        "scanned_entities",
        "metric_evaluations",
    ] {
        assert_eq!(null[key], prior[key]);
    }
    assert!(null.get("selection").is_none());
    assert!(null.get("source_cost").is_none());
    Ok(())
}

#[test]
fn grouped_counts_and_sums_match_independent_direction_and_geometry_oracles() -> TestResult {
    for mask in [0u16, 1, 7, 73, 84, 341, 511] {
        for direction in ["outgoing", "incoming", "undirected"] {
            let (mut snapshot, mut context) = fixture();
            snapshot.graph.entities.retain(|id, _| id.get() <= 3);
            snapshot.graph.edges.clear();
            for number in 1..=3 {
                let entity = snapshot
                    .graph
                    .entities
                    .get_mut(&EntityId::new(number))
                    .ok_or("entity")?;
                entity
                    .fields
                    .insert("stock".to_owned(), known(WorldValue::U64(number * 10)));
                entity.fields.insert(
                    "position".to_owned(),
                    known(WorldValue::Coord(MapCoord::new(number as i32 - 1, 0, 0))),
                );
            }
            let mut edges = Vec::new();
            for from in 1..=3u64 {
                for to in 1..=3u64 {
                    let bit = (from - 1) * 3 + to - 1;
                    if mask & (1 << bit) != 0 {
                        edges.push((from, to));
                        add_edge(
                            &mut snapshot.graph,
                            u128::from(bit + 1),
                            from,
                            to,
                            EdgeKind::Requires,
                        );
                    }
                }
            }
            // A parallel edge changes incidence counts, never entity totals.
            if edges.contains(&(1, 2)) {
                edges.push((1, 2));
                add_edge(&mut snapshot.graph, 100, 1, 2, EdgeKind::Requires);
            }
            refresh(&mut snapshot, &mut context);
            let selected: Vec<u64> = (1..=2)
                .filter(|subject| {
                    edges.iter().any(|&(from, to)| {
                        let outgoing = from == *subject && matches!(to, 2 | 3);
                        let incoming = to == *subject && matches!(from, 2 | 3);
                        match direction {
                            "outgoing" => outgoing,
                            "incoming" => incoming,
                            _ => outgoing || incoming,
                        }
                    })
                })
                .collect();
            let input = json!({"schema":"dfmcp.query/1","query":{
                "kind":"aggregate","kinds":["item"],
                "relation":{"edge_kinds":["requires"],"target_ids":["2","3"],"direction":direction},
                "spatial":{"region":{"min":[0,0,0],"max":[1,0,0]},
                    "geometry":{"kind":"point","field":"position"}},
                "metrics":[{"name":"stock","field":"stock","numeric_type":"u64"}]
            }});
            let result = execute(&snapshot, &context, &input)?;
            assert_eq!(result["matched"], selected.len(), "{mask}:{direction}");
            assert_eq!(result["groups"][0]["count"], selected.len());
            let expected = if selected.is_empty() {
                Value::Null
            } else {
                json!(selected.iter().map(|id| id * 10).sum::<u64>().to_string())
            };
            assert_eq!(
                result["groups"][0]["metrics"]["stock"]["sum"], expected,
                "{mask}:{direction}"
            );
            assert_eq!(result["selection"]["complete_within_projection"], true);
            assert_eq!(result["selection"]["predicate_evaluations"], 0);
        }
    }
    Ok(())
}

#[test]
fn geometry_unknowns_never_become_zero_inventory_or_known_outside() -> TestResult {
    for case in 0..10 {
        let (mut snapshot, mut context) = fixture();
        snapshot
            .graph
            .entities
            .retain(|id, _| matches!(id.get(), 1 | 100));
        snapshot.graph.edges.retain(|id, _| *id == EdgeId::new(1));
        let anchor = snapshot.anchor();
        let entity = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("entity")?;
        if case == 0 {
            entity.fields.remove("position");
        } else {
            let position = entity.fields.get_mut("position").ok_or("position")?;
            match case {
                1 => position.presence = Some(FactPresence::Absent),
                2 => position.presence = Some(FactPresence::Unknown("unread".to_owned())),
                3 => position.presence = Some(FactPresence::Omitted("unread".to_owned())),
                4 => position.presence = Some(FactPresence::Redacted("private".to_owned())),
                5 => position.presence = Some(FactPresence::Unsupported("version".to_owned())),
                6 => position.presence = Some(FactPresence::Stale(anchor)),
                7 => {
                    position.presence = Some(FactPresence::Known(WorldValue::Coord(MapCoord::new(
                        2, 0, 0,
                    ))))
                }
                8 => position.observed_at = GameTick(11),
                _ => *position = known(WorldValue::Text("concealed-location".to_owned())),
            }
        }
        refresh(&mut snapshot, &mut context);
        let mut input = request();
        remove(&mut input, "group_by")?;
        let result = execute(&snapshot, &context, &input)?;
        assert_eq!(result["matched"], 0, "case {case}");
        assert_eq!(result["selection"]["relation_matched"], 1);
        assert_eq!(result["selection"]["unknown_geometry"], 1);
        assert_eq!(result["selection"]["outside_spatial_relation"], 0);
        assert_eq!(result["selection"]["complete_within_projection"], false);
        assert_eq!(result["groups"][0]["count"], 0);
        assert!(result["groups"][0]["metrics"]["stock"]["sum"].is_null());
        assert!(!result.to_string().contains("concealed-location"));
    }
    Ok(())
}

#[test]
fn cuboid_boundaries_and_known_members_with_unavailable_metrics_stay_independent() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .retain(|id, _| matches!(id.get(), 1 | 9 | 100));
    snapshot
        .graph
        .edges
        .retain(|_, edge| edge.to == EntityId::new(100) && matches!(edge.from.get(), 1 | 9));
    for number in [1, 9] {
        let entity = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(number))
            .ok_or("entity")?;
        entity.fields.insert(
            "min".to_owned(),
            known(WorldValue::Coord(MapCoord::new(-2, -2, -2))),
        );
        entity.fields.insert(
            "max".to_owned(),
            known(WorldValue::Coord(MapCoord::new(2, 2, 2))),
        );
    }
    refresh(&mut snapshot, &mut context);
    for relation in ["contains", "intersects", "within"] {
        let mut input = request();
        input["query"]["spatial"] = json!({
            "region":{"min":[-2,-2,-2],"max":[2,2,2]},
            "geometry":{"kind":"cuboid","min_field":"min","max_field":"max"},
            "relation":relation
        });
        let result = execute(&snapshot, &context, &input)?;
        assert_eq!(result["matched"], 2);
        assert_eq!(result["selection"]["complete_within_projection"], true);
        assert_eq!(result["groups"][0]["metrics"]["stock"]["samples"], 1);
        assert_eq!(
            result["groups"][0]["metrics"]["stock"]["excluded"]["redacted"],
            1
        );
    }
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity")?
        .fields
        .insert(
            "min".to_owned(),
            known(WorldValue::Coord(MapCoord::new(3, 0, 0))),
        );
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["spatial"]["geometry"] =
        json!({"kind":"cuboid","min_field":"min","max_field":"max"});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(
        result["selection"]["unknown_geometry_reasons"]["inverted_cuboid"],
        1
    );
    assert_eq!(result["selection"]["outside_spatial_relation"], 1);
    assert_eq!(result["matched"], 0);
    Ok(())
}

#[test]
fn aggregate_composition_preserves_builtin_other_and_custom_kind_namespaces() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .retain(|id, _| matches!(id.get(), 1 | 2 | 100 | 101));
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or("item")?
        .kind = EntityKind::Other("item".to_owned());
    let custom_target = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(101))
        .ok_or("target")?;
    custom_target.kind = EntityKind::Other("stockpile".to_owned());
    custom_target
        .fields
        .insert("active".to_owned(), known(WorldValue::Bool(true)));
    snapshot.graph.edges.clear();
    add_edge(&mut snapshot.graph, 1, 1, 100, EdgeKind::ContainedIn);
    add_edge(
        &mut snapshot.graph,
        2,
        1,
        100,
        EdgeKind::Custom("contained_in".to_owned()),
    );
    add_edge(&mut snapshot.graph, 3, 2, 100, EdgeKind::ContainedIn);
    add_edge(&mut snapshot.graph, 4, 1, 101, EdgeKind::ContainedIn);
    refresh(&mut snapshot, &mut context);
    for (source_kind, edge_kind, target_kind) in [
        ("item", "contained_in", "stockpile"),
        ("other:item", "contained_in", "stockpile"),
        ("item", "custom:contained_in", "stockpile"),
        ("item", "contained_in", "other:stockpile"),
    ] {
        let mut input = request();
        remove(&mut input, "spatial")?;
        input["query"]["kinds"] = json!([source_kind]);
        input["query"]["group_by"] = json!({"kind":"entity_kind"});
        input["query"]["relation"]["edge_kinds"] = json!([edge_kind]);
        input["query"]["relation"]["target_kinds"] = json!([target_kind]);
        let result = execute(&snapshot, &context, &input)?;
        assert_eq!(
            result["matched"], 1,
            "{source_kind}/{edge_kind}/{target_kind}"
        );
        assert_eq!(result["selection"]["selected_entities"], 1);
        assert_eq!(result["selection"]["matched_edge_incidences"], 1);
        assert_eq!(result["groups"][0]["key"]["value"], source_kind);
        assert_eq!(result["relation"]["edge_kinds"], json!([edge_kind]));
        assert_eq!(result["relation"]["target_kinds"], json!([target_kind]));
    }
    // Existing relation/ranking consumers retain their canonical-name contract.
    let raw = json!({"edge_kinds":["custom:contained_in"],"target_kinds":["other:stockpile"]});
    let legacy = serde_json::from_value::<relation_selector::Request>(raw.clone())?.normalize()?;
    let exact = serde_json::from_value::<relation_selector::Request>(raw)?.normalize_exact()?;
    assert_eq!(legacy.json()["edge_kinds"], json!(["contained_in"]));
    assert_eq!(legacy.json()["target_kinds"], json!(["stockpile"]));
    assert_eq!(exact.json()["edge_kinds"], json!(["custom:contained_in"]));
    assert_eq!(exact.json()["target_kinds"], json!(["other:stockpile"]));
    Ok(())
}

#[test]
fn full_source_and_combined_work_are_admitted_before_any_narrow_selection() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    let mut input = request();
    input["query"]["kinds"] = json!(["unit"]);
    context.budget.max_entities =
        (snapshot.graph.entities.len() + snapshot.graph.edges.len() - 1) as u32;
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("complete-source record")));
    context.budget.max_entities = 100_000;
    let mut nested = WorldValue::Null;
    for _ in 0..66 {
        nested = WorldValue::List(vec![nested]);
    }
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(100))
        .ok_or("target")?
        .fields
        .insert("unselected".to_owned(), known(nested));
    refresh(&mut snapshot, &mut context);
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("nesting")));

    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge")?
        .fields
        .insert(
            "unselected".to_owned(),
            known(WorldValue::Text("x".repeat(200_000))),
        );
    refresh(&mut snapshot, &mut context);
    context.budget.max_bytes = 100_000;
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    context.budget.max_bytes = 8 * 1024 * 1024;
    context.budget.max_wall_millis = 0;
    let invalid_budget = execute(&snapshot, &context, &input);
    assert!(
        matches!(&invalid_budget,
        Err(error) if error.code == ErrorCode::InvalidRequest),
        "{invalid_budget:?}"
    );
    context.budget.max_wall_millis = 1;
    let expired = Instant::now()
        .checked_sub(std::time::Duration::from_millis(2))
        .ok_or("deadline fixture underflow")?;
    let deadline_error = source::preflight(&snapshot, &context, expired)
        .err()
        .ok_or("expired source deadline unexpectedly admitted")?;
    assert_eq!(
        deadline_error.code,
        ErrorCode::BudgetExceeded,
        "{deadline_error:?}"
    );

    let (mut snapshot, mut context) = fixture();
    let seed = snapshot
        .graph
        .entities
        .get(&EntityId::new(100))
        .ok_or("target")?
        .clone();
    for number in 1000..11000 {
        let mut entity = seed.clone();
        entity.id = EntityId::new(number);
        snapshot.graph.entities.insert(entity.id, entity);
    }
    refresh(&mut snapshot, &mut context);
    input["query"]["relation"]["where"] = json!({"op":"all","args":vec![boolean("active");140]});
    validate_input(&input)?;
    source::preflight(&snapshot, &context, Instant::now())?.check()?;
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("work")));
    Ok(())
}

#[test]
fn aggregate_output_and_group_limits_refuse_atomically_and_identity_binds_selection() -> TestResult
{
    let (mut snapshot, mut context) = fixture();
    let input = request();
    let exact = execute(&snapshot, &context, &input)?;
    let bytes = serde_json::to_vec(&exact)?.len() as u32;
    context.budget.max_output_tokens = bytes.div_ceil(4);
    assert_eq!(execute(&snapshot, &context, &input)?, exact);
    context.budget.max_output_tokens = (bytes - 1) / 4;
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    context.budget.max_output_tokens = 65_536;
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(9))
        .ok_or("item")?
        .fields
        .insert(
            "category".to_owned(),
            known(WorldValue::Text("drink".to_owned())),
        );
    refresh(&mut snapshot, &mut context);
    let mut too_many = input.clone();
    too_many["query"]["max_groups"] = json!(1);
    assert!(matches!(execute(&snapshot, &context, &too_many),
        Err(error) if error.code == ErrorCode::BudgetExceeded
            && error.message.contains("no partial summary")));
    for replacement in [
        json!({"relation":{"edge_kinds":["contained_in"],"target_ids":["100"]}}),
        json!({"spatial":{"region":{"min":[1,0,0],"max":[1,0,0]},
            "geometry":{"kind":"point","field":"position"}}}),
    ] {
        let mut changed = input.clone();
        for (key, value) in replacement.as_object().ok_or("replacement")? {
            changed["query"][key] = value.clone();
        }
        let changed = execute(&snapshot, &context, &changed)?;
        let original = execute(&snapshot, &context, &input)?;
        assert_ne!(changed["query_digest"], original["query_digest"]);
    }
    let mut other = context.clone();
    other.session_id = SessionId::new(18);
    assert_ne!(
        execute(&snapshot, &other, &input)?["query_digest"],
        execute(&snapshot, &context, &input)?["query_digest"]
    );
    assert!(snapshot.hash_is_valid());
    Ok(())
}

#[test]
fn malformed_compound_grammar_source_corruption_and_authority_are_refused() -> TestResult {
    let (snapshot, context) = fixture();
    for replacement in [
        json!({"relation":{"edge_kinds":[]}}),
        json!({"relation":{"edge_kinds":["contained_in"],"exists":false}}),
        json!({"relation":{"edge_kinds":["contained_in"],"target_ids":["01"]}}),
        json!({"relation":{"edge_kinds":["contained_in"],"where":{"op":"not","arg":boolean("active"),"extra":true}}}),
        json!({"spatial":{"region":{"min":[2,0,0],"max":[0,0,0]},
            "geometry":{"kind":"point","field":"position"}}}),
        json!({"spatial":{"region":{"min":[0,0,0],"max":[1,1,1]},
            "geometry":{"kind":"cuboid","min_field":"position","max_field":"position"}}}),
        json!({"spatial":{"region":{"min":[0,0,0],"max":[1,1,1]},
            "geometry":{"kind":"point","field":"position"},"distance":10}}),
    ] {
        let mut input = request();
        for (key, value) in replacement.as_object().ok_or("replacement")? {
            input["query"][key] = value.clone();
        }
        assert!(
            matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::InvalidRequest),
            "{input}"
        );
    }
    for restriction in 0..5 {
        let mut denied = context.clone();
        match restriction {
            0 => denied.grants.clear(),
            1 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(1)]),
            2 => denied.grants[0].expires_at_tick = Some(GameTick(9)),
            3 => denied.grants[0].remaining_uses = Some(0),
            _ => denied.cancellation_requested = true,
        }
        assert!(execute(&snapshot, &denied, &request()).is_err());
    }
    let mut corrupt = snapshot.clone();
    corrupt.paused = !corrupt.paused;
    assert!(matches!(execute(&corrupt, &context, &request()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    let mut inconsistent = snapshot.clone();
    inconsistent
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or("edge")?
        .id = EdgeId::new(999);
    inconsistent.refresh_hash();
    let mut context = context;
    context.anchor = inconsistent.anchor();
    assert!(matches!(execute(&inconsistent, &context, &request()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    Ok(())
}

#[test]
fn aggregate_schema_uses_the_same_closed_relationship_and_spatial_selectors() -> TestResult {
    let schema: Value = serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))?;
    let aggregate = schema["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("query schema")?
        .iter()
        .find(|query| query["properties"]["kind"]["const"] == "aggregate")
        .ok_or("aggregate schema")?;
    assert_eq!(aggregate["additionalProperties"], false);
    assert_eq!(
        aggregate["properties"]["relation"]["anyOf"][0]["$ref"],
        "#/$defs/relation_selector"
    );
    assert_eq!(
        aggregate["properties"]["spatial"]["anyOf"][0]["$ref"],
        "#/$defs/spatial_selector"
    );
    assert_eq!(
        schema["$defs"]["relation_selector"]["additionalProperties"],
        false
    );
    assert_eq!(
        schema["$defs"]["spatial_selector"]["additionalProperties"],
        false
    );
    Ok(())
}
