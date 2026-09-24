use super::super::super::{Condition, Definition, Store, execute_in, validate_definition};
use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, ErrorCode, FortressId, MapCoord,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{Fact, WorldGraph};
use std::sync::Mutex;

fn area() -> Area {
    Area {
        min: [2, 2, 5],
        max: [4, 2, 5],
    }
}
fn snapshot(sequence: u64) -> Result<WorldSnapshot> {
    let tick = GameTick(100 + sequence);
    let source = Digest32::of_bytes(&sequence.to_be_bytes());
    let fact = |suffix: &str, value| {
        Fact::known(
            value,
            tick,
            FactSource::DfhackField(format!("spatial/1.8.map.{suffix}")),
            source,
        )
    };
    let coordinate = |p: [i32; 3]| WorldValue::Coord(MapCoord::new(p[0], p[1], p[2]));
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: sequence + 1,
            kind: EntityKind::Fortress,
            label: "Test fortress".into(),
            fields: BTreeMap::from([
                (
                    "region_origin".into(),
                    fact("requested_region", coordinate([2, 2, 5])),
                ),
                (
                    "region_size".into(),
                    fact("requested_region", coordinate([3, 1, 1])),
                ),
                (
                    "map_dimensions".into(),
                    fact("Maps.getTileSize", coordinate([16, 16, 10])),
                ),
            ]),
        },
    );
    for x in 2..=4 {
        let id = tile_entity_id([x, 2, 5])?;
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: sequence + 1,
                kind: EntityKind::TileFeature,
                label: "Tile".into(),
                fields: BTreeMap::from([
                    (
                        "position".into(),
                        fact("tile_position", coordinate([x as i32, 2, 5])),
                    ),
                    (
                        "visibility".into(),
                        fact("tile_visibility", WorldValue::Text("visible".into())),
                    ),
                    (
                        "shape".into(),
                        fact("shape", WorldValue::Text("floor".into())),
                    ),
                    (
                        "liquid_depth".into(),
                        fact("liquid_depth", WorldValue::U64(0)),
                    ),
                ]),
            },
        );
    }
    Ok(WorldSnapshot::new(
        FortressId::new(1),
        tick,
        ObservationCursor { epoch: 1, sequence },
        true,
        graph,
    ))
}
fn field<'a>(snapshot: &'a mut WorldSnapshot, x: u32, field: &str) -> Result<&'a mut Fact> {
    snapshot
        .graph
        .entities
        .get_mut(&tile_entity_id([x, 2, 5])?)
        .and_then(|e| e.fields.get_mut(field))
        .ok_or_else(|| invalid("fixture field absent"))
}
fn change(snapshot: &mut WorldSnapshot, x: u32, key: &str, value: WorldValue) -> Result<()> {
    let fact = field(snapshot, x, key)?;
    fact.value = value.clone();
    fact.presence = Some(FactPresence::Known(value));
    Ok(())
}
fn floor() -> Predicate {
    Predicate::Field {
        field: "shape".into(),
        comparison: Comparison::Eq,
        value: super::super::super::Literal::Text("floor".into()),
    }
}
fn condition() -> Condition {
    Condition::TerrainCount {
        areas: vec![area()],
        predicate: floor(),
        comparison: Comparison::Eq,
        value: 3,
    }
}
fn proof(
    snapshot: &WorldSnapshot,
    areas: &[Area],
    predicate: &Predicate,
    comparison: Comparison,
    value: u64,
) -> Result<(Truth, Value)> {
    let mut probe = Probe::default();
    let truth = evaluate(
        &mut probe,
        snapshot,
        areas,
        predicate,
        comparison,
        value,
        &mut EvaluationBudget::new(60_000),
    )?;
    Ok((truth, probe.facts.remove(0)))
}
fn reseal(s: WorldSnapshot) -> WorldSnapshot {
    WorldSnapshot::new(s.fortress_id, s.tick, s.cursor, s.paused, s.graph)
}
fn context(s: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(71),
        request_id: RequestId::new(72),
        anchor: s.anchor(),
        budget: WorkBudget {
            max_entities: 100,
            max_bytes: 100_000,
            max_output_tokens: 25_000,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        grants: vec![CapabilityGrant {
            capability: Capability::Query,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}
fn input(kind: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":kind})
}
fn register() -> Value {
    input(
        json!({"kind":"watch","key":"excavation","condition":condition(),
        "deadline_tick":200,"poll_interval_ticks":1,"stable_observations":2}),
    )
}
fn call(store: &Mutex<Store>, s: &WorldSnapshot, input: Value) -> Result<Value> {
    let text = execute_in(store, s, &context(s), &input, |value| {
        serde_json::to_string(&value).map_err(|_| invalid("test result encoding"))
    })?;
    serde_json::from_str(&text).map_err(|_| invalid("test result decoding"))
}

#[test]
fn exact_mask_counts_and_permutations_are_deterministic() -> Result<()> {
    let s = snapshot(0)?;
    let (truth, all) = proof(&s, &[area()], &floor(), Comparison::Eq, 3)?;
    assert_eq!(truth, Truth::True);
    assert_eq!(all["matched_min"], 3);
    assert_eq!(all["matched_max"], 3);
    assert_eq!(all["all_positions_visible"], true);
    let first = Area {
        min: [2, 2, 5],
        max: [2, 2, 5],
    };
    let second = Area {
        min: [3, 2, 5],
        max: [4, 2, 5],
    };
    assert_eq!(
        proof(
            &s,
            &[first.clone(), second.clone()],
            &floor(),
            Comparison::Eq,
            3
        )?,
        proof(&s, &[second, first], &floor(), Comparison::Eq, 3)?
    );
    for flag in [
        "native_job_completion_proven",
        "mutation_cause_proven",
        "safety_proven",
    ] {
        assert_eq!(all[flag], false);
    }
    Ok(())
}

#[test]
fn missing_or_uncaptured_coordinates_never_disappear_from_the_denominator() -> Result<()> {
    let mut s = snapshot(0)?;
    s.graph.entities.remove(&tile_entity_id([4, 2, 5])?);
    let (truth, out) = proof(&s, &[area()], &floor(), Comparison::Eq, 3)?;
    assert_eq!(truth, Truth::Unknown);
    assert_eq!(out["requested_tiles"], 3);
    assert_eq!(out["matched_min"], 2);
    assert_eq!(out["matched_max"], 3);
    assert_eq!(out["unestablished_reasons"]["tile_not_observed"], 1);
    let (_, out) = proof(
        &s,
        &[Area {
            min: [15, 2, 5],
            max: [16, 2, 5],
        }],
        &floor(),
        Comparison::Eq,
        0,
    )?;
    assert_eq!(out["unestablished_reasons"]["outside_capture"], 1);
    assert_eq!(out["unestablished_reasons"]["outside_map"], 1);
    assert_eq!(out["truth"], "unknown");
    Ok(())
}

#[test]
fn hidden_and_unallocated_remain_unknown_even_with_populated_attributes() -> Result<()> {
    let mut s = snapshot(0)?;
    change(&mut s, 3, "visibility", WorldValue::Text("hidden".into()))?;
    change(
        &mut s,
        4,
        "visibility",
        WorldValue::Text("unallocated".into()),
    )?;
    change(
        &mut s,
        3,
        "shape",
        WorldValue::Text("SECRET_UNDISCOVERED_SHAPE".into()),
    )?;
    for predicate in [
        floor(),
        Predicate::Always {},
        Predicate::Not {
            arg: Box::new(floor()),
        },
    ] {
        let (_, out) = proof(&s, &[area()], &predicate, Comparison::Eq, 3)?;
        assert_eq!(out["unestablished"], 2);
        assert_eq!(out["visible_tiles"], 1);
        assert_eq!(out["unestablished_reasons"]["hidden"], 1);
        assert_eq!(out["unestablished_reasons"]["unallocated"], 1);
        assert!(!out.to_string().contains("SECRET_UNDISCOVERED_SHAPE"));
    }
    Ok(())
}

#[test]
fn provenance_ticks_presence_types_and_position_must_agree() -> Result<()> {
    for case in 0..8 {
        let mut s = snapshot(0)?;
        let fact = field(&mut s, 4, "shape")?;
        match case {
            0 => fact.source_digest = Digest32::ZERO,
            1 => fact.source_digest = Digest32::of_bytes(b"another_capture"),
            2 => fact.observed_at = GameTick(99),
            3 => fact.source = FactSource::DfhackField("spatial/1.8.map.unregistered".into()),
            4 => fact.presence = Some(FactPresence::Redacted("hidden".into())),
            5 => fact.presence = Some(FactPresence::Known(WorldValue::Text("wall".into()))),
            6 => {
                fact.value = WorldValue::U64(3);
                fact.presence = None;
            }
            _ => change(
                &mut s,
                4,
                "position",
                WorldValue::Coord(MapCoord::new(3, 2, 5)),
            )?,
        }
        let (truth, out) = proof(&s, &[area()], &floor(), Comparison::Eq, 3)?;
        assert_eq!(truth, Truth::Unknown, "case {case}: {out}");
        assert_eq!(out["matched_min"], 2);
        assert_eq!(out["matched_max"], 3);
    }
    let mut s = snapshot(0)?;
    s.graph.entities.remove(&EntityId::new(1));
    assert_eq!(
        proof(&s, &[area()], &floor(), Comparison::Ge, 0)?.0,
        Truth::Unknown
    );
    Ok(())
}

#[test]
fn all_comparisons_match_every_completion_of_a_partial_mask() -> Result<()> {
    for encoding in 0..27 {
        let mut code = encoding;
        let mut s = snapshot(0)?;
        let mut low = 0;
        let mut unknown = 0;
        for x in 2..=4 {
            match code % 3 {
                0 => low += 1,
                1 => change(&mut s, x, "shape", WorldValue::Text("wall".into()))?,
                _ => {
                    unknown += 1;
                    change(&mut s, x, "visibility", WorldValue::Text("hidden".into()))?;
                }
            }
            code /= 3;
        }
        for comparison in [
            Comparison::Eq,
            Comparison::Ne,
            Comparison::Lt,
            Comparison::Le,
            Comparison::Gt,
            Comparison::Ge,
        ] {
            for target in 0..=4 {
                let possibilities: Vec<bool> = (low..=low + unknown)
                    .map(|n| match comparison {
                        Comparison::Eq => n == target,
                        Comparison::Ne => n != target,
                        Comparison::Lt => n < target,
                        Comparison::Le => n <= target,
                        Comparison::Gt => n > target,
                        Comparison::Ge => n >= target,
                    })
                    .collect();
                let expected = if possibilities.iter().all(|v| *v) {
                    Truth::True
                } else if possibilities.iter().all(|v| !v) {
                    Truth::False
                } else {
                    Truth::Unknown
                };
                assert_eq!(
                    proof(&s, &[area()], &floor(), comparison, target)?.0,
                    expected
                );
            }
        }
    }
    Ok(())
}

#[test]
fn overlap_volume_coordinate_and_joint_predicate_bounds_fail_before_evaluation() -> Result<()> {
    for areas in [
        vec![],
        vec![area(); 65],
        vec![area(), area()],
        vec![Area {
            min: [0, 0, 0],
            max: [128, 128, 0],
        }],
        vec![Area {
            min: [5, 0, 0],
            max: [4, 0, 0],
        }],
        vec![Area {
            min: [0, 0, 0],
            max: [u32::MAX, 0, 0],
        }],
    ] {
        assert!(validate(&areas).is_err());
    }
    assert_eq!(
        validate(&[Area {
            min: [0, 0, 0],
            max: [127, 127, 0]
        }])?,
        16_384
    );
    let mut definition = Definition {
        key: "test".into(),
        label: "test".into(),
        condition: condition(),
        failure_condition: None,
        deadline_tick: 200,
        poll_interval_ticks: 1,
        stable_observations: 2,
    };
    validate_definition(&definition)?;
    definition.failure_condition = Some(Condition::All {
        args: vec![condition(); 32],
    });
    assert!(validate_definition(&definition).is_err());
    let mut budget = EvaluationBudget::new(60_000);
    budget.used = super::super::MAX_EVALUATION_WORK;
    assert!(
        matches!(evaluate(&mut Probe::default(), &snapshot(0)?, &[area()], &floor(),
        Comparison::Eq, 3, &mut budget), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn mask_conditions_round_trip_in_durable_definitions_and_schema() -> Result<()> {
    let definition = Definition {
        key: "persist".into(),
        label: "persist".into(),
        condition: condition(),
        failure_condition: None,
        deadline_tick: 200,
        poll_interval_ticks: 2,
        stable_observations: 3,
    };
    let value = serde_json::to_value(&definition).map_err(|_| invalid("encode definition"))?;
    let recovered: Definition =
        serde_json::from_value(value).map_err(|_| invalid("decode definition"))?;
    assert_eq!(definition, recovered);
    validate_definition(&recovered)?;
    let schema = extend_schema(json!({"$defs":{"watch_condition":{"oneOf":[]}}}))?;
    assert_eq!(
        schema["$defs"]["watch_condition"]["oneOf"][0]["properties"]["op"]["const"],
        "terrain_count"
    );
    let mut invalid = json!(condition());
    invalid["execute"] = json!(true);
    assert!(serde_json::from_value::<Condition>(invalid).is_err());
    Ok(())
}

#[test]
fn foreground_watch_requires_distinct_stable_captures_and_unknown_resets_progress() -> Result<()> {
    let store = Mutex::new(Store::default());
    let first = snapshot(0)?;
    let created = call(&store, &first, register())?;
    assert_eq!(created["record"]["status"], "candidate");
    let handle = created["record"]["watch"].clone();
    let poll = || input(json!({"kind":"poll_watch","watch":handle}));
    assert_eq!(call(&store, &first, poll())?["record"]["sample_count"], 1);
    let mut incomplete = snapshot(1)?;
    change(
        &mut incomplete,
        4,
        "visibility",
        WorldValue::Text("hidden".into()),
    )?;
    let blocked = call(&store, &reseal(incomplete), poll())?;
    assert_eq!(blocked["record"]["status"], "blocked_unknown");
    assert_eq!(blocked["record"]["stable_observations"], 0);
    assert_eq!(
        call(&store, &snapshot(2)?, poll())?["record"]["status"],
        "candidate"
    );
    let done = call(&store, &snapshot(3)?, poll())?;
    assert_eq!(done["record"]["status"], "satisfied");
    assert_eq!(
        done["record"]["evaluation"]["facts"][0]["mutation_cause_proven"],
        false
    );
    assert_eq!(
        call(&store, &snapshot(4)?, poll())?["record"]["evidence_digest"],
        done["record"]["evidence_digest"]
    );
    Ok(())
}

#[test]
fn pure_inspection_does_not_register_or_sample_a_watch() -> Result<()> {
    let s = snapshot(0)?;
    let result = super::super::super::inspection::query(
        &s,
        &context(&s),
        &input(json!({"kind":"condition_evaluation","condition":condition()})),
    )?;
    assert_eq!(result["evaluation"]["status"], "condition_met");
    assert_eq!(result["watch_registered"], false);
    assert_eq!(result["evaluation"]["watch_completion_proven"], false);
    Ok(())
}

#[test]
fn output_refusal_and_denied_authority_do_not_publish_watch_state() -> Result<()> {
    let store = Mutex::new(Store::default());
    let s = snapshot(0)?;
    let mut c = context(&s);
    c.budget.max_bytes = 1;
    assert!(execute_in(&store, &s, &c, &register(), |v| Ok(v.to_string())).is_err());
    c = context(&s);
    c.grants.clear();
    assert!(
        matches!(execute_in(&store, &s, &c, &register(), |v| Ok(v.to_string())),
        Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    let list = call(&store, &s, input(json!({"kind":"watches"})))?;
    assert_eq!(list["records"], json!([]));
    Ok(())
}
