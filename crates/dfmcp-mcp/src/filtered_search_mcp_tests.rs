//! Typed lexical selection through the real laboratory public facade.
//! Owning implementation bead: df-lab-structured-query-f7q.

use std::collections::BTreeMap;

use dfmcp_core::{Capability, Digest32, EntityId, GameTick};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph,
    WorldSnapshot,
};
use serde_json::{Value, json};

use crate::agent_facade::{fortress_open_session, fortress_query};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open_fixture() -> TestResult<(String, WorldSnapshot)> {
    let opened = parsed(&fortress_open_session(
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
    let session_id = opened["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned();
    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    let snapshot = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(10);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.graph = WorldGraph::default();
        for (id, label) in [
            (1, "Urist brewer"),
            (2, "Urist miner"),
            (3, "Urist unavailable"),
            (4, "Urist carpenter"),
            (5, "Domas carpenter"),
            (6, "Urist report"),
        ] {
            let mut alive = Fact::known(
                WorldValue::Bool(id != 2 && id != 3),
                source.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::of_bytes(b"filtered-search-fixture"),
            );
            if id == 3 {
                alive.presence = Some(FactPresence::Redacted("fixture policy".to_owned()));
            }
            source.graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: 1,
                    kind: if id == 6 {
                        EntityKind::Announcement
                    } else {
                        EntityKind::Unit
                    },
                    label: label.to_owned(),
                    fields: BTreeMap::from([("alive".to_owned(), alive)]),
                },
            );
        }
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    Ok((session_id, snapshot))
}

fn request() -> Value {
    json!({"mode":"structured","request":{"schema":"dfmcp.query/1","query":{
        "kind":"search","text":"urist","kinds":["unit"],"limit":1,
        "where":{"op":"compare","field":"alive","comparison":"eq",
            "value":{"type":"bool","value":true}}
    }}})
}

fn query(session_id: &str, request: &Value) -> TestResult<Value> {
    parsed(&fortress_query(
        Some(session_id.to_owned()),
        Some(request.to_string()),
    ))
}

fn refusal(result: &Value, code: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], code, "{result}");
    assert!(result.get("rows").is_none(), "{result}");
    assert!(result.get("selection").is_none(), "{result}");
    assert_eq!(
        result["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
}

#[test]
fn public_filtered_search_pages_and_retained_reads_preserve_current_authority() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, before) = open_fixture()?;
    let mut request = request();
    let first = query(&session_id, &request)?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["matched"], 2);
    assert_eq!(first["returned"], 1);
    assert_eq!(first["rows"][0]["entity_id"], "1");
    assert_eq!(first["selection"]["selected_entities"], 5);
    assert_eq!(first["selection"]["matched_predicate"], 3);
    assert_eq!(first["selection"]["excluded_false"], 1);
    assert_eq!(first["selection"]["excluded_unknown"], 1);
    assert_eq!(first["selection"]["complete_within_projection"], false);
    assert_eq!(first["coverage"]["absence_proven"], false);
    assert_eq!(first["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(
        first["agent_turn"]["anchor"],
        crate::observation_projection::anchor_json(&before.anchor())
    );
    request["request"]["query"]["continuation"] = first["continuation"].clone();
    request["request"]["query"]["limit"] = json!(8);
    let next = query(&session_id, &request)?;
    assert_eq!(next["ok"], true, "{next}");
    assert_eq!(next["rows"][0]["entity_id"], "4");
    assert_eq!(next["returned"], 1);
    assert_eq!(next["selection"], first["selection"]);
    assert_eq!(next["query_digest"], first["query_digest"]);

    let mut changed_filter = request.clone();
    changed_filter["request"]["query"]["where"]["value"]["value"] = json!(false);
    refusal(&query(&session_id, &changed_filter)?, "stale_anchor");

    // Prior public calls retain their exact source. Publish a newer world in
    // the adapter, then resume the old filtered page only by explicitly naming it.
    let session = crate::server::resolve_session(Some(session_id.clone()))?;
    let after = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.adapter.snapshot(), &before);
        let mut source = before.clone();
        source.tick = GameTick(20);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        let entity = source
            .graph
            .entities
            .get_mut(&EntityId::new(4))
            .ok_or("fixture citizen missing")?;
        entity.revision += 1;
        entity
            .fields
            .get_mut("alive")
            .ok_or("fixture alive missing")?
            .value = WorldValue::Bool(false);
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        source
    };
    refusal(&query(&session_id, &request)?, "stale_anchor");
    request["at"] = json!(before.state_hash.to_string());
    let historical = query(&session_id, &request)?;
    assert_eq!(historical["ok"], true, "{historical}");
    assert_eq!(historical["rows"][0]["entity_id"], "4");
    assert_eq!(historical["selection"], first["selection"]);
    assert_eq!(historical["historical"], true);
    assert_eq!(
        historical["anchor"],
        crate::observation_projection::anchor_json(&before.anchor())
    );
    assert_eq!(
        historical["agent_turn"]["anchor"],
        crate::observation_projection::anchor_json(&after.anchor())
    );
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.adapter.snapshot(), &after);
        guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("Query grant missing")?
            .expires_at_tick = Some(before.tick);
    }
    refusal(&query(&session_id, &request)?, "capability_denied");
    Ok(())
}

#[test]
fn public_filtered_search_negation_retains_unknown_and_never_dispatches() -> TestResult {
    let _serial = crate::test_serial();
    let (session_id, source) = open_fixture()?;
    let mut request = request();
    let predicate = request["request"]["query"]["where"].clone();
    request["request"]["query"]["where"] = json!({"op":"not","arg":predicate});
    let result = query(&session_id, &request)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["matched"], 1);
    assert_eq!(result["rows"][0]["entity_id"], "2");
    assert_eq!(result["selection"]["matched_predicate"], 1);
    assert_eq!(result["selection"]["excluded_false"], 3);
    assert_eq!(result["selection"]["excluded_unknown"], 1);
    assert_eq!(
        result["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
    assert_eq!(
        crate::server::resolve_session(Some(session_id))?
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot(),
        &source
    );
    Ok(())
}
