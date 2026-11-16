//! Source-time masking for operational facts (df-lab-structured-query-f7q.6).
use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::WorldGraph;

fn known(value: WorldValue, tick: u64) -> Fact {
    Fact::known(
        value,
        GameTick(tick),
        FactSource::DfhackField("source-time-fixture".to_owned()),
        Digest32::of_bytes(b"source-time-fixture"),
    )
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    for (id, tick, quantity, bucket) in [
        (1, 11, 1000, "future-secret"),
        (2, 10, 7, "equal"),
        (3, 9, 5, "older"),
        (4, 11, 9000, "redacted-secret"),
    ] {
        let mut fields = BTreeMap::from([
            ("stock".to_owned(), known(WorldValue::U64(quantity), tick)),
            ("bucket".to_owned(), known(WorldValue::Text(bucket.to_owned()), tick)),
            ("message".to_owned(), known(WorldValue::Text("needle".to_owned()), tick)),
            ("selected".to_owned(), known(WorldValue::Bool(true), tick)),
        ]);
        for fact in fields.values_mut() {
            if id == 1 {
                fact.presence = Some(FactPresence::Known(fact.value.clone()));
            } else if id == 3 {
                fact.presence = None;
            } else if id == 4 {
                fact.presence = Some(FactPresence::Redacted("policy".to_owned()));
            }
        }
        let id = EntityId::new(id);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Item,
                label: "item".to_owned(),
                fields,
            },
        );
    }
    let snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(9),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 5000,
            max_game_ticks: 1000,
            max_entities: 1000,
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
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

fn envelope(query: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":query})
}

fn aggregate() -> Value {
    envelope(json!({
        "kind":"aggregate",
        "metrics":[{"name":"stock","field":"stock","numeric_type":"u64"}]
    }))
}

fn lexical() -> Value {
    envelope(json!({
        "kind":"search","text":"needle","text_fields":["message"],
        "include_labels":false,"limit":10
    }))
}

fn selected() -> Value {
    json!({"op":"compare","field":"selected","comparison":"eq",
        "value":{"type":"bool","value":true}})
}

#[test]
fn future_numeric_samples_are_excluded_without_changing_population_counts() -> Result<()> {
    let (snapshot, context) = fixture();
    let result = execute(&snapshot, &context, &aggregate())?;
    let metric = &result["groups"][0]["metrics"]["stock"];
    assert_eq!(result["matched"], 4);
    assert_eq!(metric["samples"], 2);
    assert_eq!(metric["sum"], "12");
    assert_eq!(metric["minimum"], "5");
    assert_eq!(metric["maximum"], "7");
    assert_eq!(metric["mean"], json!({"numerator":"12","denominator":2}));
    assert_eq!(metric["excluded"]["future"], 1);
    assert_eq!(metric["excluded"]["redacted"], 1);
    assert_eq!(metric["sources"]["observed"], 2);
    Ok(())
}

#[test]
fn future_group_values_are_masked_and_keep_distinct_presence_counts() -> Result<()> {
    let (snapshot, context) = fixture();
    let result = execute(
        &snapshot,
        &context,
        &envelope(json!({
            "kind":"aggregate","group_by":{"kind":"field","field":"bucket"}
        })),
    )?;
    let groups = result["groups"]
        .as_array()
        .ok_or_else(|| invalid("missing fixture groups"))?;
    assert_eq!(groups.len(), 4);
    for state in ["future", "redacted"] {
        let group = groups
            .iter()
            .find(|group| group["key"]["presence"] == state)
            .ok_or_else(|| invalid("missing unavailable fixture group"))?;
        assert!(group["key"]["value"].is_null());
        assert_eq!(group["count"], 1);
    }
    for name in ["equal", "older"] {
        assert!(groups.iter().any(|group| group["key"]["value"]["value"] == name));
    }
    let encoded = serde_json::to_string(&result)
        .map_err(|_| invalid("cannot encode fixture result"))?;
    assert!(!encoded.contains("future-secret"));
    assert!(!encoded.contains("redacted-secret"));
    Ok(())
}

#[test]
fn lexical_matches_and_provenance_exclude_future_text() -> Result<()> {
    let (snapshot, context) = fixture();
    let result = execute(&snapshot, &context, &lexical())?;
    assert_eq!(result["matched"], 2);
    assert_eq!(result["returned"], 2);
    assert_eq!(result["rows"][0]["entity_id"], "2");
    assert_eq!(result["rows"][1]["entity_id"], "3");
    assert_eq!(result["scanned_text_bytes"], 2 * "needle".len());
    assert_eq!(result["unavailable_or_nontext_fields"], 2);
    assert!(result["continuation"].is_null());
    Ok(())
}

#[test]
fn future_field_filters_stay_unknown_for_aggregate_and_lexical_negation() -> Result<()> {
    let (snapshot, context) = fixture();
    for original in [aggregate(), lexical()] {
        for negated in [false, true] {
            let mut request = original.clone();
            request["query"]["where"] = if negated {
                json!({"op":"not","arg":selected()})
            } else {
                selected()
            };
            let result = execute(&snapshot, &context, &request)?;
            assert_eq!(result["matched"], if negated { 0 } else { 2 });
            assert_eq!(result["selection"]["excluded_unknown"], 2);
            assert_eq!(result["selection"]["excluded_false"], if negated { 2 } else { 0 });
            assert_eq!(result["selection"]["complete_within_projection"], false);
        }
    }
    Ok(())
}

#[test]
fn source_tick_is_local_to_each_selected_snapshot() -> Result<()> {
    let (retained, old_context) = fixture();
    let original_anchor = retained.anchor();
    let mut later = retained.clone();
    later.tick = GameTick(11);
    later.cursor.sequence += 1;
    later.refresh_hash();
    let mut later_context = old_context.clone();
    later_context.anchor = later.anchor();
    let current = execute(&later, &later_context, &aggregate())?;
    assert_eq!(current["groups"][0]["metrics"]["stock"]["sum"], "1012");
    assert_eq!(execute(&later, &later_context, &lexical())?["matched"], 3);
    let historical = execute(&retained, &old_context, &aggregate())?;
    assert_eq!(historical["groups"][0]["metrics"]["stock"]["sum"], "12");
    assert_eq!(historical["groups"][0]["metrics"]["stock"]["excluded"]["future"], 1);
    assert_eq!(execute(&retained, &old_context, &lexical())?["matched"], 2);
    assert_eq!(retained.anchor(), original_anchor);
    assert!(retained.hash_is_valid());
    Ok(())
}

#[test]
fn future_mask_does_not_replace_other_unavailable_or_contradicted_states() {
    for (presence_state, expected) in [
        (FactPresence::Absent, "absent"),
        (FactPresence::Unknown("unread".to_owned()), "unknown"),
        (FactPresence::Unsupported("version".to_owned()), "unsupported"),
        (FactPresence::Omitted("projection".to_owned()), "omitted"),
        (FactPresence::Redacted("policy".to_owned()), "redacted"),
        (FactPresence::Stale(fixture().0.anchor()), "stale"),
        (FactPresence::Known(WorldValue::U64(4)), "contradicted"),
    ] {
        let mut fact = known(WorldValue::U64(3), 11);
        fact.presence = Some(presence_state);
        assert_eq!(presence(Some(&fact), GameTick(10)), (expected, None));
    }
    for explicit in [false, true] {
        let mut fact = known(WorldValue::U64(3), 11);
        fact.presence = explicit.then_some(FactPresence::Known(WorldValue::U64(3)));
        assert_eq!(presence(Some(&fact), GameTick(10)), ("future", None));
        assert_eq!(
            presence(Some(&fact), GameTick(11)),
            ("known", Some(&WorldValue::U64(3)))
        );
        assert_eq!(
            presence(Some(&fact), GameTick(12)),
            ("known", Some(&WorldValue::U64(3)))
        );
    }
}
