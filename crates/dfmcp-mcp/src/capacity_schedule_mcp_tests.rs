//! Existing-tool consumers of the finite-capacity dependency model.
//! Owning bead: df-franken-storage-mvcc-54h.4.2.

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
        for (id, duration) in [(1, 10), (2, 2), (3, 2), (4, 1)] {
            source.graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: id,
                    kind: EntityKind::Job,
                    label: format!("Work {id}"),
                    fields: BTreeMap::from([(
                        "duration".to_owned(),
                        Fact::known(
                            WorldValue::U64(duration),
                            source.tick,
                            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                            Digest32::ZERO,
                        ),
                    )]),
                },
            );
        }
        for (id, from, to) in [(1, 4, 1), (2, 4, 3), (3, 3, 2)] {
            source.graph.edges.insert(
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
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    Ok((id, source))
}

fn request(capacity: Option<u64>) -> Value {
    let mut input = json!({"mode":"structured","request":{
        "schema":"dfmcp.query/1","query":{"kind":"schedule",
            "edge_kinds":["requires"],"duration_field":"duration",
            "targets":[{"entity_id":"4","generation":1}]}}});
    if let Some(capacity) = capacity {
        input["request"]["query"]["capacity"] = json!(capacity);
    }
    input
}

fn query(session: &str, input: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(session.to_owned()),
        Some(input.to_string()),
    ))
}

fn refusal(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("entries").is_none(), "{value}");
    assert!(value.get("baseline").is_none(), "{value}");
    assert!(value["agent_turn"].is_object(), "{value}");
}

#[test]
fn capacity_schedule_public_facade_compares_slots_without_effects() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let unlimited = query(&session_id, request(None))?;
    let finite = query(&session_id, request(Some(1)))?;
    let parallel = query(&session_id, request(Some(2)))?;
    for result in [&unlimited, &finite, &parallel] {
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["anchor"], anchor_json(&source.anchor()));
        assert_eq!(result["agent_turn"]["anchor"], result["anchor"]);
        assert_eq!(result["native_captures"], 0);
        assert_eq!(result["mutation_dispatched"], false);
        assert_eq!(
            result["observation_coverage"]["eligible_as_mutation_evidence"],
            false
        );
        assert_eq!(result["source_work"]["records"], 7);
    }
    assert_eq!(unlimited["makespan"], 11);
    assert_eq!(finite["makespan"], 15);
    assert_eq!(parallel["makespan"], 11);
    assert_eq!(finite["entries"][1]["queue_delay"], 10);
    assert_eq!(finite["entries"][3]["start"], 14);
    assert_eq!(finite["dispatch_order"], json!(["1", "2", "3", "4"]));
    assert_eq!(finite["baseline"]["entries"], unlimited["entries"]);
    assert_eq!(finite["baseline"]["witness"], unlimited["witness"]);
    assert_eq!(finite["model"]["capacity"]["epistemic_state"], "assumed");
    assert_eq!(finite["model"]["capacity"]["slots"], 1);
    assert_eq!(finite["model"]["epistemic_state"], "predicted");
    assert_eq!(finite["model"]["optimality_proven"], false);
    assert_ne!(
        finite["witness"]["decision_digest"],
        parallel["witness"]["decision_digest"]
    );
    assert!(finite.get("critical_chain").is_none());
    let session = resolve_session(Some(session_id))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    assert_eq!(guard.adapter.snapshot(), &source);
    Ok(())
}

#[test]
fn capacity_schedule_retained_source_keeps_current_authority_after_clock_reset() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
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
            .ok_or("goal missing")?
            .generation = 2;
        current.refresh_hash();
        guard.adapter.inject_snapshot(current.clone())?;
        guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("query grant missing")?
            .expires_at_tick = Some(GameTick(5));
        current
    };
    let mut retained = request(Some(1));
    retained["at"] = json!(source.state_hash.to_string());
    retained["request"]["expected_anchor"] = anchor_json(&source.anchor());
    let result = query(&session_id, retained.clone())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["makespan"], 15);
    assert_eq!(result["entries"][3]["generation"], 1);
    assert_eq!(result["anchor"], anchor_json(&source.anchor()));
    assert_eq!(result["current_anchor"], anchor_json(&current.anchor()));
    assert_eq!(result["agent_turn"]["anchor"], result["current_anchor"]);
    assert_eq!(result["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(result["historical"], true);
    refusal(&query(&session_id, request(Some(1)))?, "conflict");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("query grant missing")?
            .expires_at_tick = Some(GameTick(1));
    }
    refusal(&query(&session_id, retained)?, "capability_denied");
    Ok(())
}

#[test]
fn capacity_schedule_facade_rejects_ambiguous_capacity_and_whole_packet_overflow() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = fixture()?;
    let duplicate = r#"{"mode":"structured","request":{"schema":"dfmcp.query/1","query":{"kind":"schedule","edge_kinds":["requires"],"duration_field":"duration","capacity":1,"capacity":2}}}"#;
    refusal(
        &parsed(&crate::agent_facade::fortress_query(
            Some(session_id.clone()),
            Some(duplicate.to_owned()),
        ))?,
        "invalid_request",
    );
    for capacity in [Value::Null, json!(0), json!(65), json!(1.5)] {
        let mut invalid = request(Some(1));
        invalid["request"]["query"]["capacity"] = capacity;
        refusal(&query(&session_id, invalid)?, "invalid_request");
    }
    let session = resolve_session(Some(session_id.clone()))?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 600;
    refusal(&query(&session_id, request(Some(1)))?, "budget_exceeded");
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
