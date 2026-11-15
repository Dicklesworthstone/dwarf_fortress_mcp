use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, EntityId, FortressId, GameTick, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource,
    Value as WorldValue, WorldGraph,
};
use std::collections::BTreeMap;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    // Two prerequisites converge on the goal: the shorter graph branch is
    // slower, so hop-count critical paths cannot answer this schedule.
    for (id, duration) in [(1, 10), (2, 2), (3, 2), (4, 1)] {
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Job,
                label: format!("Work {id}"),
                fields: BTreeMap::from([(
                    "duration".to_owned(),
                    Fact::known(
                        WorldValue::U64(duration),
                        GameTick(10),
                        FactSource::DfhackField("job.duration".to_owned()),
                        Digest32::of_bytes(b"schedule-source"),
                    ),
                )]),
            },
        );
    }
    // This unrelated record has no duration. Target closure must leave it out.
    graph.entities.insert(
        EntityId::new(5),
        EntityRecord {
            id: EntityId::new(5),
            generation: 1,
            revision: 1,
            kind: EntityKind::Fortress,
            label: "Fortress".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    for (id, from, to) in [(1, 4, 1), (2, 4, 3), (3, 3, 2)] {
        graph.edges.insert(
            EdgeId::new(id),
            EdgeRecord {
                id: EdgeId::new(id),
                revision: 1,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::new(),
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
    json!({"schema":"dfmcp.query/1","query":{"kind":"schedule",
        "edge_kinds":["requires"],"duration_field":"duration",
        "targets":[{"entity_id":"4","generation":1}]}})
}

#[test]
fn public_schedule_executor_follows_all_prerequisites_and_reports_duration_slack() -> TestResult {
    let (source, context) = fixture();
    let before = source.canonical_bytes();
    let result = execute(&source, &context, &request())?;
    assert_eq!(result["makespan"], 11);
    assert_eq!(result["critical_chain"], json!(["1", "4"]));
    assert_eq!(result["dependency_order"], json!(["1", "2", "3", "4"]));
    assert_eq!(result["returned"], 4);
    assert_eq!(result["entries"][1]["total_slack"], 6);
    assert_eq!(result["entries"][2]["earliest_start"], 2);
    assert_eq!(result["entries"][2]["latest_start"], 8);
    assert_eq!(result["entries"][3]["earliest_start"], 10);
    assert_eq!(
        result["entries"][0]["duration_fact"]["source_digest"],
        Digest32::of_bytes(b"schedule-source").to_string()
    );
    assert_eq!(
        result["parallel_fronts"][0],
        json!({"earliest_start":0,"entities":["1","2"]})
    );
    assert_eq!(
        result["coverage"]["selection"],
        "complete_selected_prerequisite_closure"
    );
    assert_eq!(result["model"]["epistemic_state"], "predicted");
    assert_eq!(result["model"]["time_unit"], "selected_field_units");
    assert_eq!(result["coverage"]["mutation_authorized"], false);
    assert_eq!(result["witness"]["anchor"], anchor_json(source.anchor()));
    assert_eq!(execute(&source, &context, &request())?, result);
    assert_eq!(source.canonical_bytes(), before);
    Ok(())
}

#[test]
fn schedule_canonicalizes_selection_but_rejects_duplicate_and_stale_target_handles() -> TestResult {
    let (source, context) = fixture();
    let mut first = request();
    first["query"]["targets"] =
        json!([{"entity_id":"4","generation":1}, {"entity_id":"2","generation":1}]);
    first["query"]["edge_kinds"] = json!(["uses", "requires", "requires"]);
    let mut second = first.clone();
    second["query"]["targets"]
        .as_array_mut()
        .ok_or("targets absent")?
        .reverse();
    second["query"]["edge_kinds"] = json!(["requires", "uses"]);
    assert_eq!(
        execute(&source, &context, &first)?,
        execute(&source, &context, &second)?
    );
    first["query"]["targets"] =
        json!([{"entity_id":"4","generation":1}, {"entity_id":"4","generation":1}]);
    assert_eq!(
        execute(&source, &context, &first).map_err(|e| e.code),
        Err(ErrorCode::InvalidRequest)
    );
    first["query"]["targets"] = json!([{"entity_id":"4","generation":2}]);
    assert_eq!(
        execute(&source, &context, &first).map_err(|e| e.code),
        Err(ErrorCode::Conflict)
    );
    Ok(())
}

#[test]
fn schedule_never_guesses_missing_duration_or_drops_a_cyclic_prerequisite() -> TestResult {
    let (mut source, mut context) = fixture();
    source
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or("job absent")?
        .fields
        .remove("duration");
    source.refresh_hash();
    context.anchor = source.anchor();
    assert!(execute(&source, &context, &request()).is_err());
    let (mut source, mut context) = fixture();
    source.graph.edges.insert(
        EdgeId::new(9),
        EdgeRecord {
            id: EdgeId::new(9),
            revision: 1,
            kind: EdgeKind::Requires,
            from: EntityId::new(2),
            to: EntityId::new(4),
            fields: BTreeMap::new(),
        },
    );
    source.refresh_hash();
    context.anchor = source.anchor();
    assert_eq!(
        execute(&source, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::Conflict)
    );
    let (source, context) = fixture();
    let mut all = request();
    all["query"]
        .as_object_mut()
        .ok_or("query absent")?
        .remove("targets");
    assert!(execute(&source, &context, &all).is_err());
    Ok(())
}

#[test]
fn schedule_rejects_unavailable_or_asserted_duration_without_partial_output() -> TestResult {
    for variant in 0..5 {
        let (mut source, mut context) = fixture();
        let fact = source
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("job absent")?
            .fields
            .get_mut("duration")
            .ok_or("duration absent")?;
        match variant {
            0 => fact.presence = Some(FactPresence::Unknown("unobserved".to_owned())),
            1 => fact.source = FactSource::AgentAssertion("agent estimate".to_owned()),
            2 => fact.source_digest = Digest32::ZERO,
            3 => fact.observed_at = GameTick(11),
            _ => fact.presence = Some(FactPresence::Known(WorldValue::U64(999))),
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
fn schedule_requires_current_authority_anchor_and_foreground_allowance() -> TestResult {
    let (source, context) = fixture();
    for variant in 0..7 {
        let mut denied = context.clone();
        match variant {
            0 => denied.grants.clear(),
            1 => denied.grants[0].scope.entity_ids = BTreeSet::from([EntityId::new(4)]),
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
    let mut stale = request();
    stale["expected_anchor"] = anchor_json(source.anchor());
    stale["expected_anchor"]["game_tick"] = json!(9);
    assert_eq!(
        execute(&source, &context, &stale).map_err(|e| e.code),
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
fn schedule_admits_complete_source_and_complete_result_separately() -> TestResult {
    let (source, mut context) = fixture();
    context.budget.max_entities = 7; // five vertices plus three edges
    assert_eq!(
        execute(&source, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_entities = 128;
    context.budget.max_bytes = source.canonical_bytes().len() as u64 - 1;
    assert_eq!(
        execute(&source, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    context.budget.max_bytes = 1024 * 1024;
    context.budget.max_output_tokens = 10;
    assert_eq!(
        execute(&source, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    let (mut source, mut context) = fixture();
    source
        .graph
        .entities
        .get_mut(&EntityId::new(5))
        .ok_or("unrelated vertex absent")?
        .fields
        .insert(
            "unselected".to_owned(),
            Fact::known(
                WorldValue::Bytes(vec![0; 4096]),
                source.tick,
                FactSource::Replay,
                Digest32::ZERO,
            ),
        );
    source.refresh_hash();
    context.anchor = source.anchor();
    context.budget.max_bytes = 2048;
    assert_eq!(
        execute(&source, &context, &request()).map_err(|e| e.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn schedule_closed_grammar_refuses_malformed_selectors_and_duplicate_fields() -> TestResult {
    let (source, context) = fixture();
    for (key, value) in [
        ("duration_field", json!("")),
        ("duration_field", json!("x".repeat(129))),
        ("duration_field", json!("x\0y")),
        ("duration_field", json!(" ")),
        ("duration_field", json!("x\ny")),
        ("targets", Value::Null),
        ("targets", json!([{"entity_id":"04","generation":1}])),
        (
            "targets",
            json!([{"entity_id":"4","generation":1,"extra":true}]),
        ),
        ("targets", json!([{"entity_id":"4"}])),
        ("edge_kinds", json!([])),
        ("edge_kinds", json!(["unknown"])),
        ("capacity", json!(5)),
        (
            "targets",
            json!(vec![json!({"entity_id":"4","generation":1}); 65]),
        ),
    ] {
        let mut input = request();
        input["query"][key] = value;
        assert!(
            execute(&source, &context, &input).is_err(),
            "accepted {key}: {input}"
        );
    }
    let duplicate = r#"{"schema":"dfmcp.query/1","query":{"kind":"schedule","edge_kinds":["requires"],"duration_field":"a","duration_field":"b"}}"#;
    assert!(serde_json::from_str::<Envelope>(duplicate).is_err());
    Ok(())
}

#[test]
fn schedule_schema_extension_preserves_all_existing_variants() -> TestResult {
    let base: Value = serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))?;
    let old = base["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("variants absent")?;
    let result = extend_schema(base.clone())?;
    let new = result["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or("variants absent")?;
    assert_eq!(&new[..old.len()], old.as_slice());
    assert_eq!(new.len(), old.len() + 1);
    assert_eq!(
        new.last().ok_or("extension absent")?["properties"]["kind"]["const"],
        "schedule"
    );
    Ok(())
}

#[test]
fn schedule_output_reservation_does_not_shrink_complete_source_allowance() -> TestResult {
    let (mut source, mut context) = fixture();
    source
        .graph
        .entities
        .get_mut(&EntityId::new(5))
        .ok_or("fortress absent")?
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
    let mut selected = request();
    selected["query"]["targets"] = json!([{"entity_id":"1","generation":1}]);
    let result = execute_with_output_budget(&source, &context, &selected, 8192)?;
    assert_eq!(result["returned"], 1);
    assert!(serde_json::to_vec(&result)?.len() <= 8192);
    assert!(source.canonical_bytes().len() > 65_536);
    context.budget.max_bytes = source.canonical_bytes().len() as u64 - 1;
    assert_eq!(
        execute_with_output_budget(&source, &context, &selected, 8192).map_err(|error| error.code),
        Err(ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn retained_schedule_authorizes_at_current_tick_after_restore_not_historical_tick() -> TestResult {
    let (source, mut current) = fixture();
    current.anchor.tick = GameTick(2);
    current.anchor.cursor.epoch = 1;
    current.grants[0].expires_at_tick = Some(GameTick(5));
    assert_eq!(
        execute(&source, &current, &request()).map_err(|error| error.code),
        Err(ErrorCode::StaleAnchor)
    );
    let result = execute_laboratory_source(&source, &current, &request(), 60_000)?;
    assert_eq!(result["anchor"], anchor_json(source.anchor()));
    assert_eq!(result["makespan"], 11);
    assert_eq!(current.grants[0].expires_at_tick, Some(GameTick(5)));
    current.grants[0].expires_at_tick = Some(GameTick(1));
    assert_eq!(
        execute_laboratory_source(&source, &current, &request(), 60_000)
            .map_err(|error| error.code),
        Err(ErrorCode::CapabilityDenied)
    );
    Ok(())
}

#[test]
fn retained_schedule_cannot_cross_fortresses_or_renew_elapsed_source_work_budget() -> TestResult {
    let (source, mut current) = fixture();
    assert_eq!(
        execute_laboratory_source(&source, &current, &request(), 0).map_err(|error| error.code),
        Err(ErrorCode::BudgetExceeded)
    );
    current.anchor.fortress_id = FortressId::new(2);
    current.grants[0].scope.fortress_id = Some(FortressId::new(2));
    assert_eq!(
        execute_laboratory_source(&source, &current, &request(), 60_000)
            .map_err(|error| error.code),
        Err(ErrorCode::StaleAnchor)
    );
    Ok(())
}
