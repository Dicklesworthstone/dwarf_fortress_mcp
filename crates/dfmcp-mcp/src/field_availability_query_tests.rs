//! Source-bound availability selection through the shared JSON query boundary.
//! Owning bead: df-lab-structured-query-f7q.7.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, ObservationCursor, RequestId, SessionId,
    WorkBudget,
};
use dfmcp_world::WorldGraph;

fn fixture() -> (WorldSnapshot, OperationContext) {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    );
    for id in 1..=14 {
        let mut fact = Fact::known(
            WorldValue::Text("concealed-payload".to_owned()),
            GameTick(10),
            FactSource::DfhackField("fixture.reading".to_owned()),
            Digest32::of_bytes(b"field-availability-fixture"),
        );
        match id {
            1 => fact.value = WorldValue::Bool(true),
            2 => {
                fact.value = WorldValue::Bool(true);
                fact.observed_at = GameTick(7);
            }
            3 => fact.presence = Some(FactPresence::Absent),
            4 => fact.presence = Some(FactPresence::Unknown("not determined".to_owned())),
            5 => fact.presence = Some(FactPresence::Unsupported("version".to_owned())),
            6 => fact.presence = Some(FactPresence::Omitted("projection".to_owned())),
            7 | 13 => fact.presence = Some(FactPresence::Redacted("policy".to_owned())),
            8 => fact.presence = Some(FactPresence::Stale(snapshot.anchor())),
            10 => fact.presence = Some(FactPresence::Known(WorldValue::Bool(false))),
            12 => fact.presence = Some(FactPresence::Absent),
            14 => fact.value = WorldValue::Null,
            _ => {}
        }
        if matches!(id, 11..=13) {
            fact.observed_at = GameTick(11);
        }
        if matches!(id, 1 | 2 | 14) {
            fact.presence = Some(FactPresence::Known(fact.value.clone()));
        }
        let entity_id = EntityId::new(id);
        snapshot.graph.entities.insert(
            entity_id,
            EntityRecord {
                id: entity_id,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {id}"),
                fields: if id == 9 {
                    BTreeMap::new()
                } else {
                    BTreeMap::from([("reading".to_owned(), fact)])
                },
            },
        );
    }
    snapshot.refresh_hash();
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 128,
            max_bytes: 1024 * 1024,
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

fn request(filter: Value) -> Value {
    json!({"schema":"dfmcp.query/1","query":{
        "kind":"entities","kinds":["unit"],"fields":["reading"],
        "where":filter,"limit":32
    }})
}

fn presence(state: &str) -> Value {
    json!({"op":"presence","field":"reading","state":state})
}

fn freshness(max_age_ticks: u64) -> Value {
    json!({"op":"freshness","field":"reading","max_age_ticks":max_age_ticks})
}

fn ids(result: &Value) -> Vec<&str> {
    result["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| row["entity_id"].as_str())
        .collect()
}

#[test]
fn availability_json_selects_each_category_without_publishing_unavailable_values() -> Result<()> {
    let (snapshot, context) = fixture();
    for (state, expected) in [
        ("known", vec!["1", "2", "14"]),
        ("absent", vec!["3"]),
        ("unknown", vec!["4"]),
        ("unsupported", vec!["5"]),
        ("omitted", vec!["6"]),
        ("redacted", vec!["7", "13"]),
        ("stale", vec!["8"]),
        ("unobserved", vec!["9"]),
        ("contradicted", vec!["10"]),
        ("future", vec!["11", "12"]),
    ] {
        let result = execute(&snapshot, &context, &request(presence(state)))?;
        assert_eq!(ids(&result), expected, "{state}: {result}");
        assert!(!result.to_string().contains("concealed-payload"), "{state}");
        assert_eq!(result["anchor"], anchor_json(snapshot.anchor()));
        if state == "future" {
            for row in result["rows"].as_array().into_iter().flatten() {
                assert_eq!(row["fields"]["reading"]["presence"], "unknown");
                assert!(row["fields"]["reading"]["value"].is_null());
            }
        }
    }
    let combined = request(json!({"op":"all","args":[
        freshness(0),
        {"op":"compare","field":"reading","comparison":"eq",
            "value":{"type":"bool","value":true}}
    ]}));
    assert_eq!(ids(&execute(&snapshot, &context, &combined)?), ["1"]);
    let missing_or_redacted = request(json!({"op":"any","args":[
        presence("unobserved"), presence("redacted")
    ]}));
    assert_eq!(
        ids(&execute(&snapshot, &context, &missing_or_redacted)?),
        ["7", "9", "13"]
    );
    Ok(())
}

#[test]
fn freshness_uses_inclusive_selected_source_time_and_never_negates_unknown_into_known() -> Result<()>
{
    let (snapshot, context) = fixture();
    for (age, expected) in [
        (0, vec!["1", "14"]),
        (2, vec!["1", "14"]),
        (3, vec!["1", "2", "14"]),
        (u64::MAX, vec!["1", "2", "14"]),
    ] {
        assert_eq!(
            ids(&execute(&snapshot, &context, &request(freshness(age)))?),
            expected
        );
    }
    let old = request(json!({"op":"not","arg":freshness(2)}));
    assert_eq!(ids(&execute(&snapshot, &context, &old)?), ["2"]);

    let mut current = context.clone();
    current.anchor.tick = GameTick(100);
    current.grants[0].expires_at_tick = Some(GameTick(100));
    let result = execute_retained_with_output_budget(
        &snapshot,
        &current,
        &request(freshness(0)),
        current.budget.max_bytes,
        Instant::now(),
    )?;
    assert_eq!(ids(&result), ["1", "14"]);
    assert_eq!(result["anchor"], anchor_json(snapshot.anchor()));
    current.grants[0].expires_at_tick = Some(GameTick(99));
    assert!(
        execute_retained_with_output_budget(
            &snapshot,
            &current,
            &request(freshness(0)),
            current.budget.max_bytes,
            Instant::now(),
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn diagnostic_pages_bind_parameters_and_keep_complete_source_and_output_budgets() -> Result<()> {
    let (snapshot, context) = fixture();
    for filter in [presence("known"), freshness(3)] {
        let mut input = request(filter);
        input["query"]["limit"] = json!(1);
        let first = execute(&snapshot, &context, &input)?;
        assert_eq!(ids(&first), ["1"]);
        input["query"]["continuation"] = first["continuation"].clone();
        input["query"]["limit"] = json!(2);
        let next = execute(&snapshot, &context, &input)?;
        assert_eq!(ids(&next), ["2", "14"]);
        assert!(next["continuation"].is_null());
        if input["query"]["where"]["op"] == "presence" {
            input["query"]["where"]["state"] = json!("redacted");
        } else {
            input["query"]["where"]["max_age_ticks"] = json!(4);
        }
        assert_eq!(
            execute(&snapshot, &context, &input).err().map(|e| e.code),
            Some(ErrorCode::StaleAnchor)
        );
    }
    let mut narrow = context.clone();
    narrow.budget.max_entities = 1;
    assert_eq!(
        execute(&snapshot, &narrow, &request(presence("unobserved")))
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        execute_with_output_budget(&snapshot, &context, &request(presence("known")), 1)
            .err()
            .map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    let mut denied = context.clone();
    denied.grants.clear();
    assert!(execute(&snapshot, &denied, &request(presence("redacted"))).is_err());
    Ok(())
}

#[test]
fn diagnostic_filter_json_is_closed_and_bounded() {
    for filter in [
        json!({"op":"presence","field":"reading","state":"null"}),
        json!({"op":"presence","field":"reading"}),
        json!({"op":"presence","field":"reading","state":"known","extra":true}),
        json!({"op":"presence","field":"","state":"known"}),
        json!({"op":"presence","field":"é".repeat(65),"state":"known"}),
        json!({"op":"freshness","field":"reading"}),
        json!({"op":"freshness","field":"reading","max_age_ticks":null}),
        json!({"op":"freshness","field":"reading","max_age_ticks":-1}),
        json!({"op":"freshness","field":"reading","max_age_ticks":0.5}),
        json!({"op":"freshness","field":"reading","max_age_ticks":"0"}),
        json!({"op":"freshness","field":"reading\u{0}","max_age_ticks":0}),
        json!({"op":"freshness","field":"reading","max_age_ticks":0,"state":"known"}),
    ] {
        assert!(decode_filter(filter.clone()).is_err(), "{filter}");
    }
}

#[test]
fn provenance_selects_exact_sources_and_preserves_unknown_at_retained_ticks() -> Result<()> {
    let (mut snapshot, mut context) = fixture();
    for (id, source) in [
        (1, FactSource::Derived("scenario".to_owned())),
        (2, FactSource::AgentAssertion("scenario".to_owned())),
        (3, FactSource::Replay),
        (14, FactSource::Derived("other".to_owned())),
    ] {
        if let Some(fact) = snapshot.graph.entities.get_mut(&EntityId::new(id))
            .and_then(|row| row.fields.get_mut("reading"))
        {
            fact.source = source;
        }
    }
    snapshot.refresh_hash();
    context.anchor = snapshot.anchor();
    let filter = json!({"op":"provenance","field":"reading","source_kind":"derived"});
    for (kind, expected) in [
        ("derived", vec!["1", "14"]),
        ("agent_assertion", vec!["2"]),
        ("replay", vec!["3"]),
        ("dfhack_field", vec![]),
    ] {
        let mut by_kind = filter.clone();
        by_kind["source_kind"] = json!(kind);
        assert_eq!(ids(&execute(&snapshot, &context, &request(by_kind))?), expected);
    }
    let mut exact = filter.clone();
    exact["source_name"] = json!("scenario");
    exact["source_digest"] = json!(Digest32::of_bytes(b"field-availability-fixture").to_hex());
    assert_eq!(ids(&execute(&snapshot, &context, &request(exact.clone()))?), ["1"]);
    exact["source_digest"] = json!(Digest32::ZERO.to_hex());
    assert!(ids(&execute(&snapshot, &context, &request(exact))?).is_empty());
    context.anchor.tick = GameTick(100);
    let negated = request(json!({"op":"not","arg":filter}));
    let historical = execute_retained_with_output_budget(
        &snapshot, &context, &negated, context.budget.max_bytes, Instant::now(),
    )?;
    assert_eq!(ids(&historical), ["2", "3"]);
    Ok(())
}

#[test]
fn provenance_continuations_bind_optional_selectors_and_reject_invalid_sources() -> Result<()> {
    let (snapshot, context) = fixture();
    let mut filter = json!({"op":"provenance","field":"reading","source_kind":"dfhack_field"});
    let mut input = request(filter.clone());
    input["query"]["limit"] = json!(1);
    let first = execute(&snapshot, &context, &input)?;
    assert_eq!(ids(&first), ["1"]);
    input["query"]["continuation"] = first["continuation"].clone();
    assert_eq!(ids(&execute(&snapshot, &context, &input)?), ["2"]);
    for (key, value) in [
        ("source_kind", json!("derived")),
        ("source_name", json!("fixture.reading")),
        ("source_digest", json!(Digest32::of_bytes(b"field-availability-fixture").to_hex())),
    ] {
        let mut changed = input.clone();
        changed["query"]["where"][key] = value;
        assert!(matches!(execute(&snapshot, &context, &changed),
            Err(error) if error.code == ErrorCode::StaleAnchor));
    }
    for (key, value) in [
        ("source_kind", json!("observed")),
        ("source_name", json!("")),
        ("source_name", json!("é".repeat(129))),
        ("source_digest", json!("A".repeat(64))),
        ("source_digest", json!("wrong")),
    ] {
        let mut invalid = filter.clone();
        invalid[key] = value;
        assert!(decode_filter(invalid).is_err());
    }
    filter["source_kind"] = json!("replay");
    filter["source_name"] = json!("fixture.reading");
    assert!(decode_filter(filter).is_err());
    Ok(())
}
