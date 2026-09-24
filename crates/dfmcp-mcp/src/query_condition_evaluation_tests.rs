use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    WorkBudget,
};
use dfmcp_world::{EntityKind, EntityRecord, Fact, WorldGraph};

fn world(sane: bool) -> WorldSnapshot {
    let source = Digest32::of_bytes(b"condition-inspection-fixture");
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Urist".into(),
            fields: BTreeMap::from([(
                "sane".into(),
                Fact::known(
                    WorldValue::Bool(sane),
                    GameTick(10),
                    FactSource::DfhackField("unit.sane".into()),
                    source,
                ),
            )]),
        },
    );
    graph.entities.insert(
        EntityId::new(2),
        EntityRecord {
            id: EntityId::new(2),
            generation: 1,
            revision: 1,
            kind: EntityKind::Item,
            label: "stock".into(),
            fields: BTreeMap::from([(
                "stack_size".into(),
                Fact::known(
                    WorldValue::U64(7),
                    GameTick(10),
                    FactSource::DfhackField("item.stack_size".into()),
                    source,
                ),
            )]),
        },
    );
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor {
            epoch: 0,
            sequence: 1,
        },
        true,
        graph,
    )
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(98301),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_entities: 100,
            max_bytes: 262144,
            max_output_tokens: 65536,
            max_wall_millis: 60000,
            max_game_ticks: 1000,
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
fn field() -> Value {
    json!({"op":"field","entity_id":"1","generation":1,"field":"sane",
    "comparison":"eq","value":{"type":"bool","value":true}})
}
fn request(condition: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":{
    "kind":"condition_evaluation","condition":condition}})
}
fn inspect(snapshot: &WorldSnapshot, value: &Value) -> Result<Value> {
    query(snapshot, &context(snapshot), value)
}
fn parsed(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| invalid("test JSON"))
}

#[test]
fn inspection_uses_watch_precedence_without_claiming_stability() -> Result<()> {
    for sane in [false, true] {
        for failed in [false, true] {
            let snapshot = world(sane);
            let c = context(&snapshot);
            let mut input = request(field());
            input["query"]["failure_condition"] = json!({"op":"paused","value":failed});
            let result = query(&snapshot, &c, &input)?;
            let store = Mutex::new(Store::default());
            let watch_input = json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"parity",
                "condition":field(),"failure_condition":input["query"]["failure_condition"],
                "deadline_tick":100,"stable_observations":1}});
            let watch = parsed(&execute_in(&store, &snapshot, &c, &watch_input, |v| {
                Ok(v.to_string())
            })?)?;
            let expected = if failed {
                "failure_condition_met"
            } else if sane {
                "condition_met"
            } else {
                "condition_not_met"
            };
            assert_eq!(result["evaluation"]["status"], expected);
            assert_eq!(
                result["evaluation"]["condition_truth"],
                watch["record"]["evaluation"]["condition"]
            );
            assert_eq!(
                result["evaluation"]["failure_condition_truth"],
                watch["record"]["evaluation"]["failure_condition"]
            );
            assert_eq!(
                result["evaluation"]["facts"],
                watch["record"]["evaluation"]["facts"]
            );
            assert_eq!(result["evaluation"]["watch_completion_proven"], false);
        }
    }
    Ok(())
}

#[test]
fn generation_mismatch_cannot_hide_behind_a_decisive_boolean_branch() -> Result<()> {
    let snapshot = world(true);
    let mut stale = field();
    stale["generation"] = json!(2);
    for group in ["all", "any"] {
        let value = inspect(
            &snapshot,
            &request(json!({"op":group,"args":[
            {"op":"paused","value":group=="any"}, stale.clone()]})),
        )?;
        assert_eq!(value["evaluation"]["status"], "invalidated_reference");
        assert_eq!(value["evaluation"]["eligible_success_sample"], false);
        assert_eq!(value["evaluation"]["generation_mismatch"], true);
    }
    Ok(())
}

#[test]
fn unknown_success_or_failure_does_not_become_a_successful_sample() -> Result<()> {
    let snapshot = world(true);
    let mut missing = field();
    missing["field"] = json!("not_observed");
    let value = inspect(
        &snapshot,
        &request(json!({"op":"not","arg":missing.clone()})),
    )?;
    assert_eq!(value["evaluation"]["condition_truth"], "unknown");
    assert_eq!(value["evaluation"]["status"], "blocked_unknown");
    let mut input = request(field());
    input["query"]["failure_condition"] = missing;
    let value = inspect(&snapshot, &input)?;
    assert_eq!(value["evaluation"]["condition_truth"], "true");
    assert_eq!(value["evaluation"]["failure_condition_truth"], "unknown");
    assert_eq!(value["evaluation"]["eligible_success_sample"], false);
    Ok(())
}

#[test]
fn compound_population_and_quantity_evidence_uses_existing_measurements() -> Result<()> {
    let snapshot = world(true);
    let condition = json!({"op":"all","args":[
        {"op":"entity_count","scope":"observed_projection","kind":"unit","predicate":{"op":"always"},"comparison":"eq","value":1},
        {"op":"item_quantity","scope":"observed_projection","quantity_unit":"stack_units","predicate":{"op":"always"},"comparison":"ge","value":7}]});
    let out = inspect(&snapshot, &request(condition))?;
    assert_eq!(out["evaluation"]["status"], "condition_met");
    assert_eq!(out["evaluation"]["facts"][1]["quantity_min"], 7);
    assert_eq!(out["evaluation"]["facts"][1]["quantity_max"], 7);
    assert_eq!(out["watch_evaluated"], false);
    Ok(())
}

#[test]
fn repeated_inspection_preserves_watch_progress_and_normalizes_null_guard() -> Result<()> {
    let snapshot = world(true);
    let c = context(&snapshot);
    let store = Mutex::new(Store::default());
    let create = json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"retained",
        "condition":field(),"deadline_tick":100,"stable_observations":5}});
    let before = execute_in(&store, &snapshot, &c, &create, |v| Ok(v.to_string()))?;
    let input = request(field());
    let first = query(&snapshot, &c, &input)?;
    for _ in 0..10 {
        assert_eq!(query(&snapshot, &c, &input)?, first);
    }
    let mut explicit = input;
    explicit["query"]["failure_condition"] = Value::Null;
    assert_eq!(query(&snapshot, &c, &explicit)?, first);
    let after = execute_in(&store, &snapshot, &c, &create, |v| Ok(v.to_string()))?;
    assert_eq!(parsed(&before)?["record"], parsed(&after)?["record"]);
    assert_eq!(snapshot.anchor(), c.anchor);
    Ok(())
}

#[test]
fn shared_predicate_bounds_and_complete_output_are_enforced() -> Result<()> {
    let snapshot = world(true);
    let mut c = context(&snapshot);
    let leaf = json!({"op":"paused","value":true});
    let mut input = request(json!({"op":"all","args":vec![leaf.clone();32]}));
    input["query"]["failure_condition"] = json!({"op":"all","args":vec![leaf;32]});
    assert!(matches!(query(&snapshot, &c, &input), Err(e) if e.code==ErrorCode::BudgetExceeded));
    c.budget.max_bytes = 1;
    assert!(
        matches!(query(&snapshot, &c, &request(field())), Err(e) if e.code==ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn authority_anchor_corrupt_state_and_stateful_arguments_are_refused() -> Result<()> {
    let mut snapshot = world(true);
    let c = context(&snapshot);
    let input = request(field());
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(
        matches!(query(&snapshot, &denied, &input), Err(e) if e.code==ErrorCode::CapabilityDenied)
    );
    let mut cancelled = c.clone();
    cancelled.cancellation_requested = true;
    assert!(
        matches!(query(&snapshot, &cancelled, &input), Err(e) if e.code==ErrorCode::CancellationRequested)
    );
    let mut stale = c.clone();
    stale.anchor.cursor.sequence += 1;
    assert!(matches!(query(&snapshot, &stale, &input), Err(e) if e.code==ErrorCode::StaleAnchor));
    for key in [
        "watch",
        "key",
        "deadline_tick",
        "stable_observations",
        "continuation",
    ] {
        let mut bad = input.clone();
        bad["query"][key] = json!(1);
        assert!(matches!(query(&snapshot, &c, &bad), Err(e) if e.code==ErrorCode::InvalidRequest));
    }
    snapshot.paused = false;
    assert!(query(&snapshot, &c, &input).is_err());
    Ok(())
}

#[test]
fn stale_or_untrusted_fields_remain_unknown_without_disclosing_backing_values() -> Result<()> {
    for mode in 0..3 {
        let mut snapshot = world(true);
        let fact = snapshot
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .and_then(|e| e.fields.get_mut("sane"))
            .ok_or_else(|| invalid("fixture field"))?;
        match mode {
            0 => fact.observed_at = GameTick(9),
            1 => fact.source_digest = Digest32::ZERO,
            _ => fact.presence = Some(FactPresence::Known(WorldValue::Bool(false))),
        }
        snapshot.refresh_hash();
        let value = inspect(&snapshot, &request(field()))?;
        assert_eq!(value["evaluation"]["condition_truth"], "unknown");
        assert!(value["evaluation"]["facts"][0].get("value").is_none());
    }
    Ok(())
}
