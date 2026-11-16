//! Regression coverage for observation-time eligibility at presentation boundaries.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EdgeId, FortressId, ObservationCursor, RequestId,
    SessionId, WorkBudget,
};
use dfmcp_world::{EdgeRecord, WorldGraph};

fn fact(value: WorldValue, tick: u64) -> Fact {
    Fact::known(
        value,
        GameTick(tick),
        FactSource::DfhackField("projection-test".to_owned()),
        Digest32::of_bytes(b"source-time-projection"),
    )
}

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        EntityId::new(1),
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: BTreeMap::from([
                (
                    "future".to_owned(),
                    fact(WorldValue::Text("future-payload".repeat(4096)), 11),
                ),
                ("rank".to_owned(), fact(WorldValue::U64(7), 10)),
                (
                    "position".to_owned(),
                    fact(WorldValue::Coord(MapCoord::new(1, 2, 3)), 10),
                ),
                (
                    "past".to_owned(),
                    fact(WorldValue::Text("prior-value".to_owned()), 9),
                ),
                (
                    "future_absence".to_owned(),
                    Fact::with_presence(
                        FactPresence::Absent,
                        GameTick(11),
                        FactSource::Replay,
                        Digest32::ZERO,
                    ),
                ),
            ]),
        },
    );
    graph.entities.insert(
        EntityId::new(2),
        EntityRecord {
            id: EntityId::new(2),
            generation: 1,
            revision: 1,
            kind: EntityKind::Item,
            label: "Tool".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    graph.edges.insert(
        EdgeId::new(1),
        EdgeRecord {
            id: EdgeId::new(1),
            revision: 1,
            kind: EdgeKind::Requires,
            from: EntityId::new(1),
            to: EntityId::new(2),
            fields: BTreeMap::new(),
        },
    );
    let snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    );
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 100_000,
            max_bytes: 8 * 1024 * 1024,
            max_output_tokens: 65_536,
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

fn request(query: Value) -> Value {
    json!({"schema":"dfmcp.query/1", "query":query})
}

#[test]
fn known_values_and_absence_are_unknown_until_their_own_observation_tick() {
    for observed in [9, 10, 11] {
        for (source, expected_state) in [
            (FactSource::DfhackField("field".to_owned()), "observed"),
            (FactSource::Derived("derivation".to_owned()), "inferred"),
            (FactSource::AgentAssertion("assertion".to_owned()), "assumed"),
            (FactSource::Replay, "assumed"),
        ] {
            for explicit in [false, true] {
                let mut value = fact(WorldValue::Text("future-payload".to_owned()), observed);
                value.source = source.clone();
                if explicit {
                    value.presence = Some(FactPresence::Known(value.value.clone()));
                }
                let projected = fact_json(&value, GameTick(10));
                assert_eq!(projected["observed_at_game_tick"], observed);
                assert_eq!(projected["source_digest"], value.source_digest.to_string());
                if observed > 10 {
                    assert_eq!(projected["presence"], "unknown");
                    assert_eq!(projected["epistemic_state"], "unknown");
                    assert!(projected["value"].is_null());
                    assert!(!projected.to_string().contains("future-payload"));
                } else {
                    assert_eq!(projected["presence"], "known");
                    assert_eq!(projected["epistemic_state"], expected_state);
                    assert_eq!(
                        projected["value"],
                        json!({"type":"text", "value":"future-payload"})
                    );
                }
                value.presence = Some(FactPresence::Absent);
                let absent = fact_json(&value, GameTick(10));
                assert!(absent["value"].is_null());
                assert_eq!(
                    absent["presence"],
                    if observed > 10 { "unknown" } else { "absent" }
                );
            }
        }
    }
}

#[test]
fn future_observation_preserves_unavailable_and_contradictory_classifications() {
    let (snapshot, _) = fixture();
    for (presence, expected, epistemic) in [
        (FactPresence::Redacted("policy".to_owned()), "redacted", "unknown"),
        (FactPresence::Omitted("profile".to_owned()), "omitted", "unknown"),
        (
            FactPresence::Unsupported("manifest".to_owned()),
            "unsupported",
            "unknown",
        ),
        (FactPresence::Unknown("not observed".to_owned()), "unknown", "unknown"),
        (FactPresence::Stale(snapshot.anchor()), "stale", "stale"),
        (
            FactPresence::Known(WorldValue::Bool(false)),
            "unknown",
            "contradicted",
        ),
    ] {
        let mut value = fact(WorldValue::Text("future-payload".to_owned()), 11);
        value.presence = Some(presence);
        let projected = fact_json(&value, GameTick(10));
        assert_eq!(projected["presence"], expected);
        assert_eq!(projected["epistemic_state"], epistemic);
        assert!(projected["value"].is_null());
        assert!(!projected.to_string().contains("future-payload"));
    }
}

#[test]
fn every_entity_query_family_masks_future_values_before_complete_row_fitting() -> Result<()> {
    let (snapshot, context) = fixture();
    let fields = json!(["future", "future_absence", "past"]);
    let mut queries = vec![
        json!({"kind":"entities", "kinds":["unit"], "limit":8}),
        json!({"kind":"inspect", "entity_id":"1", "generation":1}),
        json!({"kind":"spatial_entities", "kinds":["unit"], "limit":8,
            "region":{"min":[0,0,0], "max":[4,4,4]},
            "geometry":{"kind":"point", "field":"position"}}),
        json!({"kind":"related_entities", "kinds":["unit"], "limit":8,
            "relation":{"edge_kinds":["requires"], "target_kinds":["item"]}}),
        json!({"kind":"ranked_entities", "kinds":["unit"], "limit":8,
            "order_by":[{"field":"rank", "numeric_type":"u64", "direction":"descending"}]}),
    ];
    for query in &mut queries {
        query["fields"] = fields.clone();
        let input = request(query.clone());
        let result = match query["kind"].as_str() {
            Some("spatial_entities") => {
                super::super::spatial_entity_query::execute_with_output_budget(
                    &snapshot, &context, &input, 8192,
                )?
            }
            Some("related_entities") => {
                super::super::related_entity_query::execute_with_output_budget(
                    &snapshot, &context, &input, 8192,
                )?
            }
            Some("ranked_entities") => {
                super::super::ranked_entity_query::execute_with_output_budget(
                    &snapshot, &context, &input, 8192,
                )?
            }
            _ => execute_with_output_budget(&snapshot, &context, &input, 8192)?,
        };
        let row = if query["kind"] == "inspect" {
            &result["row"]
        } else {
            assert_eq!(result["rows"].as_array().map(Vec::len), Some(1));
            &result["rows"][0]
        };
        assert_eq!(row["entity_id"], "1");
        for field in ["future", "future_absence"] {
            assert_eq!(row["fields"][field]["presence"], "unknown");
            assert!(row["fields"][field]["value"].is_null());
            assert_eq!(row["fields"][field]["observed_at_game_tick"], 11);
        }
        assert_eq!(
            row["fields"]["past"]["value"],
            json!({"type":"text", "value":"prior-value"})
        );
        let rendered = result.to_string();
        assert!(!rendered.contains("future-payload"));
        assert!(rendered.len() <= 8192);
    }
    assert!(snapshot.hash_is_valid());
    Ok(())
}

#[test]
fn retained_sources_do_not_borrow_temporal_eligibility_from_later_worlds() -> Result<()> {
    let (snapshot, context) = fixture();
    let input = request(json!({
        "kind":"inspect", "entity_id":"1", "generation":1,
        "fields":["future_absence", "rank"]
    }));
    let initial = execute_with_output_budget(&snapshot, &context, &input, 8192)?;
    assert_eq!(initial["row"]["fields"]["future_absence"]["presence"], "unknown");

    let mut later = snapshot.clone();
    later.tick = GameTick(11);
    later.cursor.sequence += 1;
    later.refresh_hash();
    let mut current = context.clone();
    current.anchor = later.anchor();
    let latest = execute_with_output_budget(&later, &current, &input, 8192)?;
    assert_eq!(latest["row"]["fields"]["future_absence"]["presence"], "absent");
    let retained = execute_retained_with_output_budget(
        &snapshot, &current, &input, 8192, Instant::now(),
    )?;
    assert_eq!(retained, initial);
    assert_eq!(
        execute_with_output_budget(&snapshot, &current, &input, 8192)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::StaleAnchor)
    );
    current.grants.clear();
    assert!(
        execute_retained_with_output_budget(
            &snapshot, &current, &input, 8192, Instant::now(),
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn hidden_future_payload_consumes_source_budget_and_never_supplies_authority() -> Result<()> {
    let (snapshot, context) = fixture();
    let input = request(json!({
        "kind":"entities", "kinds":["unit"], "fields":["future"], "limit":1
    }));
    let rendered = execute_with_output_budget(&snapshot, &context, &input, 4096)?;
    assert_eq!(rendered["rows"][0]["fields"]["future"]["presence"], "unknown");
    assert!(!rendered.to_string().contains("future-payload"));

    let mut bounded = context.clone();
    bounded.budget.max_bytes = 4096;
    assert_eq!(
        execute_with_output_budget(&snapshot, &bounded, &input, 4096)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::BudgetExceeded)
    );
    let mut denied = context;
    denied.grants.clear();
    assert!(execute_with_output_budget(&snapshot, &denied, &input, 4096).is_err());
    Ok(())
}
