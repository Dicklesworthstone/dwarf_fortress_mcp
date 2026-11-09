use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, GameTick, MapCoord, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{EdgeKind, EdgeRecord, EntityKind, Fact, FactPresence, FactSource, WorldGraph};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn known(value: WorldValue) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField("compound-fixture".to_owned()),
        Digest32::of_bytes(b"ranked-spatial-query-test"),
    )
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for (id, stress, x, alive) in [
        (1, 90, 0, true),
        (2, 100, 10, true),
        (3, 80, 1, true),
        (4, 70, 2, false),
        (5, 60, 3, true),
        (6, 50, 4, true),
    ] {
        let id = EntityId::new(id);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 7,
                revision: 9,
                kind: EntityKind::Unit,
                label: format!("Urist {id}"),
                fields: BTreeMap::from([
                    ("stress".to_owned(), known(WorldValue::I64(stress))),
                    ("alive".to_owned(), known(WorldValue::Bool(alive))),
                    (
                        "position".to_owned(),
                        known(WorldValue::Coord(MapCoord::new(x, 0, 0))),
                    ),
                ]),
            },
        );
    }
    for id in [101, 102] {
        let id = EntityId::new(id);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 2,
                revision: 4,
                kind: EntityKind::Squad,
                label: format!("squad {id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (from, to) in [(1, 101), (2, 101), (3, 101), (4, 101), (5, 102)] {
        let id = EdgeId::new(u128::from(from));
        graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 23,
                kind: EdgeKind::AssignedTo,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::new(),
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
        "spatial":{"region":{"min":[0,0,0],"max":[4,0,0]},
            "geometry":{"kind":"point","field":"position"}},
        "order_by":[{"field":"stress","numeric_type":"i64","direction":"descending"}],
        "limit":128}})
}

fn broad_request() -> Value {
    let mut input = request();
    input["query"]["spatial"]["region"] = json!({
        "min":[i32::MIN,i32::MIN,i32::MIN],"max":[i32::MAX,i32::MAX,i32::MAX]
    });
    input
}

fn refresh(snapshot: &mut WorldSnapshot, context: &mut OperationContext) {
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
}

fn ids(value: &Value) -> Vec<String> {
    value["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["entity_id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn source_filter_relationship_spatial_selection_and_ranking_compose_with_all_witnesses()
-> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(5))
        .ok_or("entity")?
        .fields
        .insert("stress".to_owned(), known(WorldValue::I64(99)));
    for id in [4, 6] {
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(id))
            .ok_or("entity")?
            .fields
            .get_mut("position")
            .ok_or("position")?
            .presence = Some(FactPresence::Redacted("not projected".to_owned()));
    }
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(101))
        .ok_or("squad")?
        .fields
        .insert(
            "position".to_owned(),
            known(WorldValue::Coord(MapCoord::new(-1000, -1000, -1000))),
        );
    refresh(&mut snapshot, &mut context);
    let original = snapshot.anchor();
    let mut input = request();
    input["query"]["where"] = json!({"op":"compare","field":"alive","comparison":"eq",
        "value":{"type":"bool","value":true}});
    input["query"]["relation"] = json!({"edge_kinds":["assigned_to"],
        "target_kinds":["squad"],"target_ids":["101"]});
    input["query"]["limit"] = json!(1);
    let result = execute(&snapshot, &context, &input)?;
    // The highest rank is outside the region; the next rank belongs to another
    // squad. Both filters must run before retaining the one best candidate.
    assert_eq!(ids(&result), ["1"]);
    assert_eq!(result["matched"], 2);
    assert_eq!(result["selection"]["kind_selected"], 6);
    assert_eq!(result["selection"]["excluded_filter_false"], 1);
    assert_eq!(result["selection"]["relation_matched"], 3);
    assert_eq!(result["selection"]["no_observed_match"], 2);
    assert_eq!(result["selection"]["spatial_matched"], 2);
    assert_eq!(result["selection"]["outside_spatial_relation"], 1);
    assert_eq!(result["selection"]["unknown_geometry"], 0);
    assert_eq!(
        result["coverage"]["selection_complete_within_projection"],
        true
    );
    assert_eq!(result["coverage"]["eligible_as_mutation_evidence"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(result["coverage"]["terrain_evaluated"], false);
    assert_eq!(result["coverage"]["accessibility_evaluated"], false);
    let first = &result["rows"][0];
    assert_eq!(first["fields"], json!({}));
    assert_eq!(first["ranking"]["facts"]["stress"]["value"]["value"], 90);
    assert_eq!(first["geometry"]["min"], json!([0, 0, 0]));
    assert_eq!(
        first["geometry"]["facts"]["position"]["epistemic_state"],
        "observed"
    );
    assert_eq!(
        first["relations"]["witnesses"][0]["target"]["entity_id"],
        "101"
    );
    assert_eq!(first["relations"]["witnesses"][0]["edge_revision"], 23);
    input["query"]["continuation"] = result["continuation"].clone();
    let second = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&second), ["3"]);
    assert!(second["continuation"].is_null());
    let digest =
        Digest32::from_hex(result["query_digest"].as_str().ok_or("digest")?).ok_or("digest")?;
    for id in [2, 5] {
        input["query"]["continuation"] = json!(cursor(digest, (EntityId::new(id), 7)));
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::CursorGap));
    }
    assert_eq!(snapshot.anchor(), original);
    Ok(())
}

#[test]
fn compound_cuboid_selection_and_ranked_pages_match_an_independent_coordinate_oracle() -> TestResult
{
    let (mut snapshot, mut context) = fixture();
    let bounds = [
        ([-2, -2, -2], [-1, 0, 0]),
        ([0, 0, 0], [0, 0, 0]),
        ([0, 0, 0], [2, 2, 2]),
        ([-1, -1, -1], [3, 3, 3]),
        ([3, 0, 0], [4, 1, 1]),
        ([0, 0, 3], [0, 0, 3]),
    ];
    for (index, (min, max)) in bounds.iter().enumerate() {
        let entity = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(index as u64 + 1))
            .ok_or("entity")?;
        entity.fields.insert(
            "min".to_owned(),
            known(WorldValue::Coord(MapCoord::new(min[0], min[1], min[2]))),
        );
        entity.fields.insert(
            "max".to_owned(),
            known(WorldValue::Coord(MapCoord::new(max[0], max[1], max[2]))),
        );
    }
    refresh(&mut snapshot, &mut context);
    for (min, max) in [
        ([0, 0, 0], [2, 2, 2]),
        ([-1, -1, -1], [0, 0, 0]),
        ([i32::MIN; 3], [i32::MAX; 3]),
    ] {
        for relation in ["intersects", "within", "contains"] {
            let expected = [2usize, 1, 3, 4, 5, 6]
                .into_iter()
                .filter(|id| {
                    let (low, high) = bounds[*id - 1];
                    (0..3).all(|axis| match relation {
                        "within" => min[axis] <= low[axis] && high[axis] <= max[axis],
                        "contains" => low[axis] <= min[axis] && max[axis] <= high[axis],
                        _ => low[axis].max(min[axis]) <= high[axis].min(max[axis]),
                    })
                })
                .map(|id| id.to_string())
                .collect::<Vec<_>>();
            let mut input = request();
            input["query"]["spatial"] = json!({
                "region":{"min":min,"max":max}, "relation":relation,
                "geometry":{"kind":"cuboid","min_field":"min","max_field":"max"}
            });
            input["query"]["limit"] = json!(2);
            let mut actual = Vec::new();
            for _ in 0..4 {
                let result = execute(&snapshot, &context, &input)?;
                assert_eq!(result["matched"], expected.len());
                assert_eq!(result["selection"]["unknown_geometry"], 0);
                actual.extend(ids(&result));
                input["query"]["continuation"] = result["continuation"].clone();
                if input["query"]["continuation"].is_null() {
                    break;
                }
            }
            assert!(input["query"]["continuation"].is_null());
            assert_eq!(actual, expected, "{relation}: {min:?}..{max:?}");
        }
    }
    Ok(())
}

#[test]
fn geometry_unknowns_remain_separate_from_outside_filter_and_rank_exclusions() -> TestResult {
    for case in 0..10 {
        let (mut snapshot, mut context) = fixture();
        let anchor = snapshot.anchor();
        let entity = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("entity")?;
        if case == 0 {
            entity.fields.remove("position");
        } else {
            let fact = entity.fields.get_mut("position").ok_or("position")?;
            match case {
                1 => fact.presence = Some(FactPresence::Absent),
                2 => fact.presence = Some(FactPresence::Unknown("unavailable".to_owned())),
                3 => fact.presence = Some(FactPresence::Omitted("unavailable".to_owned())),
                4 => fact.presence = Some(FactPresence::Redacted("unavailable".to_owned())),
                5 => fact.presence = Some(FactPresence::Unsupported("unavailable".to_owned())),
                6 => fact.presence = Some(FactPresence::Stale(anchor)),
                7 => {
                    fact.presence = Some(FactPresence::Known(WorldValue::Coord(MapCoord::new(
                        77, 77, 77,
                    ))))
                }
                8 => fact.observed_at = GameTick(11),
                _ => *fact = known(WorldValue::Text("concealed-payload".to_owned())),
            }
        }
        snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(3))
            .ok_or("entity")?
            .fields
            .get_mut("stress")
            .ok_or("stress")?
            .presence = Some(FactPresence::Unknown("not observed".to_owned()));
        refresh(&mut snapshot, &mut context);
        let mut input = request();
        input["query"]["where"] = json!({"op":"compare","field":"alive","comparison":"eq",
            "value":{"type":"bool","value":true}});
        let result = execute(&snapshot, &context, &input)?;
        assert_eq!(ids(&result), ["5", "6"], "case {case}");
        assert_eq!(result["selection"]["excluded_filter_false"], 1);
        assert_eq!(result["selection"]["outside_spatial_relation"], 1);
        assert_eq!(result["selection"]["unknown_geometry"], 1);
        assert_eq!(result["selection"]["spatial_matched"], 3);
        assert_eq!(result["selection"]["unknown_rank"], 1);
        assert_eq!(
            result["coverage"]["selection_complete_within_projection"],
            false
        );
        assert!(!result.to_string().contains("concealed-payload"));
    }
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity")?
        .fields
        .remove("position");
    let id = EdgeId::new(999);
    snapshot.graph.edges.insert(
        id,
        EdgeRecord {
            id,
            revision: 1,
            kind: EdgeKind::AssignedTo,
            from: EntityId::new(1),
            to: EntityId::new(999),
            fields: BTreeMap::new(),
        },
    );
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["relation"] = json!({"edge_kinds":["assigned_to"],"target_kinds":["squad"]});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), ["3", "4", "5"]);
    assert_eq!(result["selection"]["relation_matched"], 5);
    assert_eq!(result["selection"]["missing_target_records"], 1);
    assert_eq!(result["selection"]["unknown_relation"], 0);
    assert_eq!(result["selection"]["unknown_geometry"], 1);
    assert_eq!(result["selection"]["outside_spatial_relation"], 1);
    assert_eq!(
        result["coverage"]["selection_complete_within_projection"],
        false
    );

    let (mut snapshot, mut context) = fixture();
    for entity in snapshot.graph.entities.values_mut() {
        if let Some(position) = entity.fields.get("position").cloned() {
            entity.fields.insert("min".to_owned(), position.clone());
            entity.fields.insert("max".to_owned(), position);
        }
    }
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity")?
        .fields
        .insert(
            "min".to_owned(),
            known(WorldValue::Coord(MapCoord::new(1, 0, 0))),
        );
    refresh(&mut snapshot, &mut context);
    let mut input = request();
    input["query"]["spatial"]["geometry"] =
        json!({"kind":"cuboid","min_field":"min","max_field":"max"});
    let result = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&result), ["3", "4", "5", "6"]);
    assert_eq!(result["selection"]["outside_spatial_relation"], 1);
    assert_eq!(result["selection"]["unknown_geometry"], 1);
    assert_eq!(
        result["selection"]["unknown_geometry_reasons"]["inverted_cuboid"],
        1
    );
    Ok(())
}

#[test]
fn compound_geometry_keeps_inferred_provenance_and_earlier_observation_tick() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    let fact = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity")?
        .fields
        .get_mut("position")
        .ok_or("position")?;
    fact.source = FactSource::Derived("estimated position".to_owned());
    fact.observed_at = GameTick(1);
    refresh(&mut snapshot, &mut context);
    let result = execute(&snapshot, &context, &request())?;
    assert_eq!(ids(&result)[0], "1");
    let geometry = &result["rows"][0]["geometry"];
    assert_eq!(geometry["facts"]["position"]["epistemic_state"], "inferred");
    assert_eq!(geometry["facts"]["position"]["observed_at_game_tick"], 1);
    assert_eq!(geometry["eligible_as_mutation_evidence"], false);
    Ok(())
}

#[test]
fn rank_cursors_bind_every_spatial_selector_and_reject_ineligible_positions() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = request();
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&first), ["1"]);
    input["query"]["continuation"] = first["continuation"].clone();
    let mut explicit = input.clone();
    explicit["query"]["spatial"]["relation"] = json!("intersects");
    assert_eq!(
        execute(&snapshot, &context, &input)?,
        execute(&snapshot, &context, &explicit)?
    );
    for case in 0..6 {
        let mut changed = input.clone();
        match case {
            0 => changed["query"]["spatial"]["region"]["min"][0] = json!(1),
            1 => changed["query"]["spatial"]["region"]["max"][2] = json!(1),
            2 => changed["query"]["spatial"]["geometry"]["field"] = json!("elsewhere"),
            3 => changed["query"]["spatial"]["relation"] = json!("within"),
            4 => changed["query"]["spatial"] = Value::Null,
            _ => {
                changed["query"]["spatial"]["geometry"] =
                    json!({"kind":"cuboid","min_field":"min","max_field":"max"})
            }
        }
        assert!(
            matches!(execute(&snapshot, &context, &changed),
            Err(error) if error.code == ErrorCode::StaleAnchor),
            "case {case}"
        );
    }
    let digest =
        Digest32::from_hex(first["query_digest"].as_str().ok_or("digest")?).ok_or("digest")?;
    for (id, generation) in [(2, 7), (6, 7), (1, 999)] {
        input["query"]["continuation"] = json!(cursor(digest, (EntityId::new(id), generation)));
        assert!(matches!(execute(&snapshot, &context, &input),
            Err(error) if error.code == ErrorCode::CursorGap));
    }
    let mut ordinary = request();
    ordinary["query"]
        .as_object_mut()
        .ok_or("query")?
        .remove("spatial");
    let absent = execute(&snapshot, &context, &ordinary)?;
    ordinary["query"]["spatial"] = Value::Null;
    assert_eq!(execute(&snapshot, &context, &ordinary)?, absent);
    assert!(absent.get("spatial").is_none());
    assert!(absent["selection"].get("unknown_geometry").is_none());
    Ok(())
}

#[test]
fn mandatory_geometry_provenance_participates_in_whole_row_paging() -> TestResult {
    let (mut snapshot, mut context) = fixture();
    snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity")?
        .fields
        .get_mut("position")
        .ok_or("position")?
        .source = FactSource::DfhackField("x".repeat(32_768));
    refresh(&mut snapshot, &mut context);
    let mut input = broad_request();
    let first = execute_with_output_budget(&snapshot, &context, &input, 8192)?;
    assert_eq!(ids(&first), ["2"]);
    assert_eq!(first["truncated"], true);
    assert!(!first["continuation"].is_null());
    assert!(first["rows"][0]["geometry"]["facts"]["position"].is_object());
    input["query"]["continuation"] = first["continuation"].clone();
    assert!(
        matches!(execute_with_output_budget(&snapshot, &context, &input, 8192),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    // A narrow output page still admits all canonical source bytes, including
    // the required geometry of the next row and unrelated records.
    let mut denied = context.clone();
    denied.budget.max_bytes = 8192;
    assert!(matches!(execute(&snapshot, &denied, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn spatial_composition_preserves_complete_packet_prefixes_and_empty_result_budgets() -> TestResult {
    let (snapshot, context) = fixture();
    let mut input = broad_request();
    input["query"]["relation"] = json!({"edge_kinds":["assigned_to"],"target_ids":["101"]});
    let fits = |value: &Value| -> Result<bool> {
        Ok(value["rows"].as_array().is_some_and(|rows| rows.len() <= 1))
    };
    let first = execute_with_page_budget(&snapshot, &context, &input, u64::MAX, &fits)?;
    assert_eq!(ids(&first), ["2"]);
    assert!(first["rows"][0]["geometry"].is_object());
    assert!(first["rows"][0]["relations"].is_object());
    assert!(first["rows"][0]["ranking"].is_object());
    input["query"]["continuation"] = first["continuation"].clone();
    let second = execute_with_page_budget(&snapshot, &context, &input, u64::MAX, &fits)?;
    assert_eq!(ids(&second), ["1"]);
    input["query"]["continuation"] = Value::Null;
    input["query"]["spatial"]["region"] = json!({"min":[99,99,99],"max":[99,99,99]});
    let empty = execute(&snapshot, &context, &input)?;
    assert_eq!(empty["matched"], 0);
    assert_eq!(empty["coverage"]["absence_proven"], false);
    assert!(
        matches!(execute_with_page_budget(&snapshot, &context, &input, u64::MAX, &|_| Ok(false)),
        Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn compound_spatial_request_is_closed_and_cannot_narrow_complete_source_admission() -> TestResult {
    let (snapshot, context) = fixture();
    for value in [
        json!({"region":{"min":[0,0,0],"max":[4,0,0]},"geometry":{"kind":"point","field":"position"},"extra":true}),
        json!({"region":{"min":[4,0,0],"max":[0,0,0]},"geometry":{"kind":"point","field":"position"}}),
        json!({"region":{"min":[0,0,0],"max":[i64::MAX,0,0]},"geometry":{"kind":"point","field":"position"}}),
        json!({"region":{"min":[0,0,0],"max":[4,0,0]},"geometry":{"kind":"point","field":""}}),
        json!({"region":{"min":[0,0,0],"max":[4,0,0]},"geometry":{"kind":"cuboid","min_field":"position","max_field":"position"}}),
        json!({"region":{"min":[0,0,0],"max":[4,0,0]},"geometry":{"kind":"point","field":"position"},"relation":"nearby"}),
    ] {
        let mut input = request();
        input["query"]["spatial"] = value;
        assert!(execute(&snapshot, &context, &input).is_err());
    }
    let mut denied = context.clone();
    denied.budget.max_entities = 8; // The full source also contains five edges.
    let mut input = request();
    input["query"]["limit"] = json!(1);
    input["query"]["spatial"]["region"] = json!({"min":[0,0,0],"max":[0,0,0]});
    assert!(matches!(execute(&snapshot, &denied, &input),
        Err(error) if error.code == ErrorCode::BudgetExceeded));
    Ok(())
}
