//! Actual public laboratory consumers of observed network bottleneck analysis.
//! Owning bead: df-franken-storage-mvcc-54h.4.4.

use super::*;
use dfmcp_core::EdgeId;
use dfmcp_world::{
    EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph,
};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(text: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(text)?)
}

fn fixture() -> TestResult<(String, WorldSnapshot)> {
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
        ]),
        Some(60_000),
        None,
        Some(128),
        Some(1024 * 1024),
        Some(8192),
        None,
        Some("empty".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    let id = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    let session = resolve_session(Some(id.clone()))?;
    let source = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(10);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.graph = WorldGraph::default();
        for id in 1..=4 {
            source.graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: id,
                    kind: EntityKind::Stockpile,
                    label: format!("Network point {id}"),
                    fields: BTreeMap::new(),
                },
            );
        }
        for (id, from, to, capacity) in [
            (1, 1, 2, 3),
            (2, 1, 3, 2),
            (3, 2, 3, 1),
            (4, 2, 4, 2),
            (5, 3, 4, 2),
        ] {
            let fact = |value| {
                Fact::known(
                    value,
                    source.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                )
            };
            source.graph.edges.insert(
                EdgeId::new(id),
                EdgeRecord {
                    id: EdgeId::new(id),
                    revision: 1,
                    kind: EdgeKind::Uses,
                    from: EntityId::new(from),
                    to: EntityId::new(to),
                    fields: BTreeMap::from([
                        ("capacity".to_owned(), fact(WorldValue::U64(capacity))),
                        (
                            "capacity_unit".to_owned(),
                            fact(WorldValue::Text("items_per_tick".to_owned())),
                        ),
                    ]),
                },
            );
        }
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    Ok((id, source))
}

fn request() -> Value {
    json!({"mode":"structured", "request":{"schema":"dfmcp.query/1", "query":{
        "kind":"max_flow", "edge_kinds":["uses"], "capacity_field":"capacity",
        "unit_field":"capacity_unit", "source":{"entity_id":"1", "generation":1},
        "sink":{"entity_id":"4", "generation":1}
    }}})
}

fn query(session: &str, request: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(session.to_owned()),
        Some(request.to_string()),
    ))
}

fn refusal(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("edges").is_none());
    assert!(value.get("minimum_cut").is_none());
    assert!(value["agent_turn"].is_object());
}

#[test]
fn max_flow_public_query_explains_the_exact_bottleneck_without_effects() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let result = query(&session_id, request())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], 4);
    assert_eq!(result["unit"], "items_per_tick");
    assert_eq!(result["minimum_cut"]["capacity"], 4);
    assert_eq!(
        result["minimum_cut"]["edge_ids"],
        json!([EdgeId::new(4).to_string(), EdgeId::new(5).to_string()])
    );
    let edges = result["edges"].as_array().ok_or("edge flows missing")?;
    assert_eq!(edges.len(), 5);
    for edge in edges {
        assert!(
            edge["flow"].as_u64().ok_or("flow missing")?
                <= edge["capacity"].as_u64().ok_or("capacity missing")?
        );
        assert_eq!(edge["capacity_fact"]["presence"], "known");
        assert_eq!(edge["unit_fact"]["value"]["value"], "items_per_tick");
    }
    assert_eq!(result["model"]["exact_within_projection"], true);
    assert_eq!(result["model"]["actual_throughput_proven"], false);
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(
        result["observation_coverage"]["eligible_as_mutation_evidence"],
        false
    );
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["mutation_dispatched"], false);
    assert_eq!(result["truncated"], false);
    assert!(result["continuation"].is_null());
    assert_eq!(result["anchor"], anchor_json(&source.anchor()));
    assert_eq!(result["agent_turn"]["anchor"], result["anchor"]);
    assert_eq!(result["source_work"]["records"], 9);
    let again = query(&session_id, request())?;
    assert_eq!(again["edges"], result["edges"]);
    assert_eq!(again["witness"], result["witness"]);
    let session = resolve_session(Some(session_id))?;
    assert_eq!(
        session
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot(),
        &source
    );
    Ok(())
}

#[test]
fn max_flow_public_history_uses_original_handles_and_current_authority_after_restore() -> TestResult
{
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let first = query(&session_id, request())?;
    assert_eq!(first["ok"], true, "{first}");
    let session = resolve_session(Some(session_id.clone()))?;
    let current = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        remember_version(&mut guard);
        let mut current = source.clone();
        current.tick = GameTick(2);
        current.cursor.epoch += 1;
        current.cursor.sequence = 0;
        current
            .graph
            .entities
            .get_mut(&EntityId::new(4))
            .ok_or("sink missing")?
            .generation = 2;
        current.refresh_hash();
        guard.adapter.inject_snapshot(current.clone())?;
        guard
            .grants
            .iter_mut()
            .find(|g| g.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(GameTick(5));
        current
    };
    let mut retained = request();
    retained["at"] = json!(source.state_hash.to_string());
    retained["request"]["expected_anchor"] = anchor_json(&source.anchor());
    let result = query(&session_id, retained.clone())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], 4);
    assert_eq!(result["sink"]["generation"], 1);
    assert_eq!(result["edges"], first["edges"]);
    assert_eq!(result["anchor"], anchor_json(&source.anchor()));
    assert_eq!(result["current_anchor"], anchor_json(&current.anchor()));
    assert_eq!(result["agent_turn"]["anchor"], result["current_anchor"]);
    assert_eq!(result["historical"], true);
    refusal(&query(&session_id, request())?, "conflict");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .iter_mut()
            .find(|g| g.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(GameTick(1));
    }
    refusal(&query(&session_id, retained)?, "capability_denied");
    Ok(())
}

#[test]
fn max_flow_public_parser_and_publisher_refuse_ambiguity_and_partial_certificates() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    for raw in [
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"max_flow","edge_kinds":["uses"],"capacity_field":"capacity","capacity_field":"quantity","unit_field":"capacity_unit","source":{"entity_id":"1","generation":1},"sink":{"entity_id":"4","generation":1}}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"max_flow","edge_kinds":["uses"],"capacity_field":"capacity","unit_field":"capacity_unit","source":{"entity_id":"1","generation":1,"generation":2},"sink":{"entity_id":"4","generation":1}}}}"#,
    ] {
        refusal(
            &parsed(&crate::agent_facade::fortress_query(
                Some(session_id.clone()),
                Some(raw.to_owned()),
            ))?,
            "invalid_request",
        );
    }
    let session = resolve_session(Some(session_id.clone()))?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 600;
    refusal(&query(&session_id, request())?, "budget_exceeded");
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 8192;
    let result = query(&session_id, request())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["value"], 4);
    assert_eq!(
        session
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot(),
        &source
    );
    Ok(())
}
