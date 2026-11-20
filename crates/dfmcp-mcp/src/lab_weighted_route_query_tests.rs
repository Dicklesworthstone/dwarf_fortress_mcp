//! Actual default-facade consumers of bounded weighted graph routes.

use super::*;
use dfmcp_core::EdgeId;
use dfmcp_world::{
    EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph,
};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn fixture() -> TestResult<(String, WorldSnapshot)> {
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "read_only".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
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
    let session_id = opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned();
    let session = resolve_session(Some(session_id.clone()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    let mut source = guard.adapter.snapshot().clone();
    source.tick = GameTick(10);
    source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
    source.graph = WorldGraph::default();
    for id in 1..=5 {
        source.graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Building,
                label: format!("Station {id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (id, from, to, cost) in [(1, 1, 4, 9), (2, 1, 2, 1), (3, 2, 3, 1), (4, 3, 4, 1)] {
        source.graph.edges.insert(
            EdgeId::new(id),
            EdgeRecord {
                id: EdgeId::new(id),
                revision: 1,
                kind: EdgeKind::Uses,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::from([(
                    "cost".to_owned(),
                    Fact::known(
                        WorldValue::U64(cost),
                        source.tick,
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )]),
            },
        );
    }
    source.refresh_hash();
    guard.adapter.inject_snapshot(source.clone())?;
    drop(guard);
    Ok((session_id, source))
}

fn request(at: Option<Digest32>) -> Value {
    let mut value = json!({"mode":"structured","request":{"schema":"dfmcp.query/1",
        "query":{"kind":"weighted_routes","roots":[{"entity_id":"1","generation":1}],
            "edge_kinds":["uses"],"cost_field":"cost",
            "targets":[{"entity_id":"4","generation":1}]}}});
    if let Some(hash) = at {
        value["at"] = json!(hash.to_string());
    }
    value
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
    assert!(value.get("entries").is_none(), "{value}");
    assert!(value.get("paths").is_none(), "{value}");
    assert!(value["agent_turn"].is_object(), "{value}");
}

#[test]
fn lab_weighted_routes_reach_the_public_facade_and_preserve_prepared_work() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let first = query(&session_id, request(None))?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["entries"][0]["total_cost"], 3);
    assert_eq!(first["paths"][0]["vertices"], json!(["1", "2", "3", "4"]));
    assert_eq!(first["anchor"], anchor_json(&source.anchor()));
    assert_eq!(first["agent_turn"]["anchor"], first["anchor"]);
    assert_eq!(
        first["observation_coverage"]["eligible_as_mutation_evidence"],
        false
    );
    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(session_id.clone()),
        Some("retain reviewed pause change".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let after = query(&session_id, request(None))?;
    assert_eq!(after["ok"], true, "{after}");
    assert_eq!(after["entries"], first["entries"]);
    assert_eq!(
        after["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        plan["plan_digest"]
    );
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
fn lab_weighted_routes_read_exact_history_and_keep_current_generation_and_grants() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, before) = fixture()?;
    let session = resolve_session(Some(session_id.clone()))?;
    let after = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        remember_version(&mut guard);
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(20);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("root missing")?
            .generation += 1;
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    refusal(&query(&session_id, request(None))?, "conflict");
    let history = query(&session_id, request(Some(before.state_hash)))?;
    assert_eq!(history["ok"], true, "{history}");
    assert_eq!(history["anchor"], anchor_json(&before.anchor()));
    assert_eq!(history["current_anchor"], anchor_json(&after.anchor()));
    assert_eq!(history["historical"], true);
    assert_eq!(history["paths"][0]["total_cost"], 3);
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        for grant in &mut guard.grants {
            if grant.capability == Capability::Query {
                grant.expires_at_tick = Some(before.tick);
            }
        }
    }
    refusal(
        &query(&session_id, request(Some(before.state_hash)))?,
        "capability_denied",
    );
    Ok(())
}

#[test]
fn lab_weighted_routes_refuse_duplicate_raw_selectors_and_complete_packet_overflow() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    for raw in [
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"weighted_routes","roots":[{"entity_id":"1","generation":1}],"edge_kinds":["uses"],"cost_field":"cost","cost_field":"other"}}}"#,
        r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"weighted_routes","roots":[{"entity_id":"1","entity_id":"2","generation":1}],"edge_kinds":["uses"],"cost_field":"cost"}}}"#,
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
        .max_output_tokens = 64;
    refusal(&query(&session_id, request(None))?, "budget_exceeded");
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
