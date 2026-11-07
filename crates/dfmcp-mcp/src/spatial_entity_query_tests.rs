use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{Fact, FactPresence, FactSource, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value, GameTick(10), FactSource::DfhackField("fixture".to_owned()),
        Digest32::of_bytes(b"spatial-fixture"),
    )
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for (id, position) in [
        (1, [-2, -2, 0]), (2, [0, 0, 0]), (3, [2, 2, 0]),
        (4, [1, 1, 1]), (u64::MAX, [9, 9, 9]),
    ] {
        graph.entities.insert(EntityId::new(id), EntityRecord {
            id: EntityId::new(id), generation: 3, revision: 7, kind: EntityKind::Unit,
            label: format!("Urist {id}"),
            fields: BTreeMap::from([
                ("position".to_owned(), known(WorldValue::Coord(coord(position)))),
                ("alive".to_owned(), known(WorldValue::Bool(id % 2 == 0))),
            ]),
        });
    }
    let snapshot = WorldSnapshot::new(
        FortressId::new(42), GameTick(10),
        ObservationCursor { epoch: 2, sequence: 8 }, true, graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(5), request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_output_tokens: 32_768, max_wall_millis: 60_000, ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id), ..CapabilityScope::default()
            },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    (snapshot, context)
}

fn request() -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"spatial_entities",
        "region":{"min":[-100,-100,-100],"max":[100,100,100]},
        "geometry":{"kind":"point","field":"position"},"limit":128}})
}

fn refresh(snapshot: &mut WorldSnapshot, context: &mut OperationContext) {
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
}

fn ids(result: &Value) -> Vec<Value> {
    result["rows"].as_array().into_iter().flatten()
        .map(|row| row["entity_id"].clone()).collect()
}

#[test]
fn inclusive_three_dimensional_selection_preserves_named_fact_provenance() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["region"] = json!({"min":[-2,-2,0],"max":[2,2,0]});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), vec![json!("1"), json!("2"), json!("3")]);
    assert_eq!(result["matched"], 3);
    assert_eq!(result["selection"]["outside_relation"], 2);
    let fact = &result["rows"][0]["geometry"]["facts"]["position"];
    assert_eq!(fact["epistemic_state"], "observed");
    assert_eq!(fact["observed_at_game_tick"], 10);
    assert_eq!(fact["source_digest"], Digest32::of_bytes(b"spatial-fixture").to_string());
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(result["coverage"]["terrain_evaluated"], false);
    assert_eq!(result["rows"][0]["fields"], json!({}));
    Ok(())
}

#[test]
fn cuboid_relations_match_an_independent_inclusive_coordinate_oracle() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot.graph.entities.clear();
    let bounds = [
        ([-1,-1,-1], [0,0,0]), ([0,0,0], [2,2,2]), ([1,1,1], [1,1,1]),
        ([-2,-2,-2], [3,3,3]), ([3,0,0], [4,1,1]),
    ];
    for (index, (min, max)) in bounds.iter().enumerate() {
        let id = EntityId::new(index as u64 + 1);
        snapshot.graph.entities.insert(id, EntityRecord {
            id, generation: 1, revision: 1, kind: EntityKind::Building,
            label: "building".to_owned(),
            fields: BTreeMap::from([
                ("min".to_owned(), known(WorldValue::Coord(coord(*min)))),
                ("max".to_owned(), known(WorldValue::Coord(coord(*max)))),
            ]),
        });
    }
    refresh(&mut snapshot, &mut context);
    for relation in ["intersects", "within", "contains"] {
        let mut input = request();
        input["query"]["geometry"] =
            json!({"kind":"cuboid","min_field":"min","max_field":"max"});
        input["query"]["region"] = json!({"min":[0,0,0],"max":[2,2,2]});
        input["query"]["relation"] = json!(relation);
        let expected = bounds.iter().enumerate().filter(|(_, (min, max))| {
            (0..3).all(|axis| match relation {
                "within" => min[axis] >= 0 && max[axis] <= 2,
                "contains" => min[axis] <= 0 && max[axis] >= 2,
                _ => (0..=2).any(|point| (min[axis]..=max[axis]).contains(&point)),
            })
        }).map(|(index, _)| json!((index + 1).to_string())).collect::<Vec<_>>();
        assert_eq!(ids(&execute(&snapshot, &context, &input)?), expected, "{relation}");
    }
    let mut input = request();
    input["query"]["geometry"] =
        json!({"kind":"cuboid","min_field":"min","max_field":"max"});
    input["query"]["region"] = json!({
        "min":[i32::MIN,i32::MIN,i32::MIN],"max":[i32::MAX,i32::MAX,i32::MAX]
    });
    assert_eq!(execute(&snapshot, &context, &input)?["matched"], 5);
    Ok(())
}

#[test]
fn unavailable_and_contradictory_geometry_is_unknown_never_outside() -> TestResult {
    let (base, base_context) = fixture();
    let states = [
        FactPresence::Absent, FactPresence::Unknown("unknown".to_owned()),
        FactPresence::Unsupported("unsupported".to_owned()),
        FactPresence::Omitted("omitted".to_owned()),
        FactPresence::Redacted("redacted".to_owned()), FactPresence::Stale(base.anchor()),
        FactPresence::Known(WorldValue::Coord(coord([77,77,77]))),
    ];
    for presence in states {
        let mut snapshot = base.clone();
        let mut context = base_context.clone();
        snapshot.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?
            .fields.get_mut("position").ok_or("position")?.presence = Some(presence);
        refresh(&mut snapshot, &mut context);
        let result = execute(&snapshot, &context, &request())?;
        assert_eq!(result["matched"], 4);
        assert_eq!(result["selection"]["unknown_geometry"], 1);
        assert_eq!(result["selection"]["outside_relation"], 0);
        assert_eq!(result["coverage"]["selection_complete_within_projection"], false);
        assert!(!ids(&result).contains(&json!("1")));
    }
    Ok(())
}

#[test]
fn missing_future_mistyped_and_inverted_geometry_remain_explicit_unknowns() -> TestResult {
    for case in 0..4 {
        let (mut snapshot, mut context) = fixture();
        let entity = snapshot.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?;
        let mut input = request();
        let reason = match case {
            0 => {
                entity.fields.remove("position");
                "missing_field"
            }
            1 => {
                entity.fields.get_mut("position").ok_or("position")?.observed_at = GameTick(11);
                "future_geometry"
            }
            2 => {
                entity.fields.insert(
                    "position".to_owned(), known(WorldValue::Text("concealedpayload".to_owned())),
                );
                "non_coordinate_field"
            }
            _ => {
                entity.fields.insert("min".to_owned(), known(WorldValue::Coord(coord([2,0,0]))));
                entity.fields.insert("max".to_owned(), known(WorldValue::Coord(coord([1,0,0]))));
                input["query"]["geometry"] =
                    json!({"kind":"cuboid","min_field":"min","max_field":"max"});
                "inverted_cuboid"
            }
        };
        refresh(&mut snapshot, &mut context);
        let result = execute(&snapshot, &context, &input)?;
        assert_eq!(result["selection"]["unknown_geometry_reasons"][reason], 1);
        assert!(!result.to_string().contains("concealedpayload"));
    }
    Ok(())
}

#[test]
fn inferred_geometry_retains_its_epistemic_class_and_filters_use_three_valued_truth() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    let entity = snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or("entity")?;
    entity.fields.get_mut("position").ok_or("position")?.source =
        FactSource::Derived("estimated".to_owned());
    snapshot.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?
        .fields.get_mut("alive").ok_or("alive")?.presence =
        Some(FactPresence::Redacted("hidden".to_owned()));
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["fields"] = json!(["alive"]);
    input["query"]["where"] = json!({"op":"not","arg":{
        "op":"compare","field":"alive","comparison":"eq","value":{"type":"bool","value":false}
    }});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), vec![json!("2"), json!("4")]);
    assert_eq!(result["selection"]["excluded_filter_false"], 2);
    assert_eq!(result["selection"]["excluded_filter_unknown"], 1);
    assert_eq!(result["coverage"]["selection_complete_within_projection"], false);
    assert_eq!(
        result["rows"][0]["geometry"]["facts"]["position"]["epistemic_state"], "inferred",
    );
    assert_eq!(result["rows"][0]["fields"]["alive"]["value"], json!({"type":"bool","value":true}));
    Ok(())
}

#[test]
fn pagination_has_no_gaps_in_both_orders_and_allows_changed_page_width() -> TestResult {
    let (snapshot, context) = fixture();
    for order in ["id_ascending", "id_descending"] {
        let mut input = request();
        input["query"]["order"] = json!(order);
        input["query"]["limit"] = json!(1);
        let first = execute(&snapshot, &context, &input)?;
        assert_eq!(first["coverage"]["selection_complete_within_projection"], true);
        assert_eq!(first["truncated"], true);
        let mut seen = ids(&first);
        input["query"]["continuation"] = first["continuation"].clone();
        input["query"]["limit"] = json!(2);
        for _ in 0..2 {
            let page = execute(&snapshot, &context, &input)?;
            seen.extend(ids(&page));
            input["query"]["continuation"] = page["continuation"].clone();
        }
        assert!(input["query"]["continuation"].is_null());
        let mut expected =
            vec![json!("1"), json!("2"), json!("3"), json!("4"), json!(u64::MAX.to_string())];
        if order == "id_descending" {
            expected.reverse();
        }
        assert_eq!(seen, expected);
    }
    Ok(())
}

#[test]
fn continuations_bind_every_selection_source_and_session_component() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["limit"] = json!(1);
    input["query"]["kinds"] = json!(["unit"]);
    input["query"]["fields"] = json!(["alive","position"]);
    let first = execute(&snapshot, &context, &input)?;
    input["query"]["continuation"] = first["continuation"].clone();
    let mut normalized = input.clone();
    normalized["query"]["kinds"] = json!(["unit","unit"]);
    normalized["query"]["fields"] = json!(["position","alive","position"]);
    assert_eq!(
        execute(&snapshot, &context, &normalized)?["query_digest"], first["query_digest"],
    );
    for case in 0..11 {
        let mut changed = input.clone();
        let mut source = snapshot.clone();
        let mut current = context.clone();
        match case {
            0 => changed["query"]["region"]["min"][0] = json!(-99),
            1 => changed["query"]["geometry"]["field"] = json!("elsewhere"),
            2 => changed["query"]["relation"] = json!("within"),
            3 => changed["query"]["fields"] = json!(["alive"]),
            4 => changed["query"]["kinds"] = json!([]),
            5 => changed["query"]["where"] = json!({"op":"all","args":[]}),
            6 => changed["query"]["order"] = json!("id_descending"),
            7 => current.session_id = SessionId::new(99),
            8 => source.cursor.sequence += 1,
            9 => source.cursor.epoch += 1,
            _ => source.tick = GameTick(12),
        }
        refresh(&mut source, &mut current);
        assert!(
            matches!(execute(&source, &current, &changed),
                Err(error) if error.code == ErrorCode::StaleAnchor),
            "case {case}",
        );
    }
    let digest = Digest32::from_hex(first["query_digest"].as_str().ok_or("digest")?)
        .ok_or("digest")?;
    for position in [
        (EntityId::new(777), 3), (EntityId::new(u64::MAX), 3), (EntityId::new(1), 99),
    ] {
        input["query"]["continuation"] = json!(cursor(digest, position));
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::CursorGap));
    }
    Ok(())
}

#[test]
fn output_budget_preserves_a_whole_row_prefix_and_never_skips_the_large_next_row() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot.graph.entities.get_mut(&EntityId::new(2)).ok_or("entity")?
        .fields.insert(
            "large".to_owned(), known(WorldValue::List(vec![WorldValue::Null; 20_000])),
        );
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["fields"] = json!(["large"]);
    context.budget.max_output_tokens = 1500;
    let first = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&first), vec![json!("1")]);
    assert_eq!(first["truncated"], true);
    assert!(first.to_string().len() <= 6000);
    input["query"]["continuation"] = first["continuation"].clone();
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    context.budget.max_bytes = 1;
    assert!(matches!(execute(&snapshot, &context, &request()),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn authority_expected_anchor_and_source_integrity_are_checked_before_returning_rows() -> TestResult {
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
    input["expected_anchor"] = super::super::core_query::anchor_json(snapshot.anchor());
    assert!(execute(&snapshot, &context, &input).is_ok());
    input["expected_anchor"]["game_tick"] = json!(9);
    assert!(matches!(execute(&snapshot, &context, &input),
        Err(error) if error.code == ErrorCode::StaleAnchor));
    let mut corrupt = snapshot.clone();
    corrupt.state_hash = Digest32::ZERO;
    let mut current = context.clone();
    current.anchor = corrupt.anchor();
    assert!(matches!(execute(&corrupt, &current, &request()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    let mut inconsistent = snapshot.clone();
    inconsistent.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?.id =
        EntityId::new(555);
    refresh(&mut inconsistent, &mut current);
    assert!(matches!(execute(&inconsistent, &current, &request()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    Ok(())
}

#[test]
fn closed_input_and_complete_source_limits_cannot_be_bypassed_by_a_narrow_region() -> TestResult {
    let (snapshot, context) = fixture();
    for malformed in [
        json!({"kind":"point","field":""}),
        json!({"kind":"point","field":"position","fallback":[0,0,0]}),
        json!({"kind":"cuboid","min_field":"position","max_field":"position"}),
    ] {
        let mut input = request();
        input["query"]["geometry"] = malformed;
        assert!(execute(&snapshot, &context, &input).is_err());
    }
    for case in 0..5 {
        let mut input = request();
        match case {
            0 => input["query"]["region"]["min"][0] = json!(101),
            1 => input["query"]["region"]["max"][0] = json!(i64::MAX),
            2 => input["query"]["limit"] = json!(129),
            3 => input["query"]["order"] = json!("label_ascending"),
            _ => input["query"]["extra"] = json!(true),
        }
        assert!(execute(&snapshot, &context, &input).is_err());
    }
    let mut constrained = context.clone();
    constrained.budget.max_entities = 4;
    let mut input = request();
    input["query"]["limit"] = json!(1);
    input["query"]["region"] = json!({"min":[99,99,99],"max":[99,99,99]});
    assert!(matches!(execute(&snapshot, &constrained, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    let mut source = snapshot.clone();
    let mut deep = WorldValue::Null;
    for _ in 0..66 {
        deep = WorldValue::List(vec![deep]);
    }
    source.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?
        .fields.insert("unselected".to_owned(), known(deep));
    constrained = context.clone();
    refresh(&mut source, &mut constrained);
    assert!(matches!(execute(&source, &constrained, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn empty_results_still_cannot_prove_world_absence() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["kinds"] = json!(["building"]);
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(result["rows"], json!([]));
    assert_eq!(result["coverage"]["selection_complete_within_projection"], true);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert!(result["continuation"].is_null());
    Ok(())
}

#[test]
fn canonically_equivalent_kind_encodings_produce_the_same_spatial_answer() -> TestResult {
    let (mut snapshot, context) = fixture();
    let mut input = request();
    input["query"]["kinds"] = json!(["unit"]);
    let before = execute(&snapshot, &context, &input)?;
    let hash = snapshot.state_hash;
    snapshot.graph.entities.get_mut(&EntityId::new(1)).ok_or("entity")?.kind =
        EntityKind::Other("unit".to_owned());
    snapshot.refresh_hash();
    assert_eq!(snapshot.state_hash, hash);
    assert_eq!(execute(&snapshot, &context, &input)?, before);
    Ok(())
}

#[test]
fn negotiated_source_bytes_include_unselected_geometry_outside_the_region() -> TestResult {
    let (snapshot, mut context) = fixture();
    context.budget.max_bytes = 8192;
    let mut input = request();
    input["query"]["limit"] = json!(1);
    input["query"]["region"] = json!({"min":[-2,-2,0],"max":[-2,-2,0]});
    assert_eq!(execute(&snapshot, &context, &input)?["matched"], 1);
    for value in [
        WorldValue::Text("x".repeat(32_768)),
        WorldValue::List(vec![WorldValue::Null; 1024]),
    ] {
        let mut source = snapshot.clone();
        let mut current = context.clone();
        source.graph.entities.get_mut(&EntityId::new(u64::MAX)).ok_or("entity")?
            .fields.insert("unselected".to_owned(), known(value));
        refresh(&mut source, &mut current);
        assert!(matches!(execute(&source, &current, &input),
            Err(error) if error.code == ErrorCode::BudgetExceeded));
    }
    Ok(())
}
