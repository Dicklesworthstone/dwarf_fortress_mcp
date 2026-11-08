//! Public laboratory entity pages fit the complete Agent Turn without losing
//! rows, authority, retained work, or the observation basis of a failed query.

use dfmcp_core::{Capability, Digest32, EntityId, GameTick};
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph, WorldSnapshot,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn decode(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn fixture(
    count: u64,
    selected_bytes: usize,
    hidden_bytes: usize,
    observe: bool,
) -> TestResult<(String, WorldSnapshot)> {
    let mut capabilities = vec![("query".to_owned(), "read_only".to_owned())];
    if observe {
        capabilities.extend([
            ("observe".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "read_only".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
        ]);
    }
    let opened = decode(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(capabilities),
        Some(60_000),
        Some(1000),
        Some(512),
        Some(2 * 1024 * 1024),
        Some(32_768),
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
    let session = crate::server::resolve_session(Some(id.clone()))?;
    let source = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(10);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.graph = WorldGraph::default();
        for entity in 1..=count {
            let known = |value| {
                Fact::known(
                    value,
                    source.tick,
                    FactSource::Replay,
                    Digest32::of_bytes(b"projected-entities-public-fixture"),
                )
            };
            source.graph.entities.insert(
                EntityId::new(entity),
                EntityRecord {
                    id: EntityId::new(entity),
                    generation: 1,
                    revision: entity,
                    kind: EntityKind::Unit,
                    label: format!("Urist {entity}"),
                    fields: BTreeMap::from([
                        (
                            "small".to_owned(),
                            known(WorldValue::Text(format!(
                                "{entity}:{}",
                                "s".repeat(selected_bytes)
                            ))),
                        ),
                        (
                            "unselected".to_owned(),
                            known(WorldValue::Text("hidden".repeat(hidden_bytes / 6))),
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

fn request(limit: u32) -> Value {
    json!({"mode":"structured", "request":{"schema":"dfmcp.query/1",
        "query":{"kind":"entities", "kinds":["unit"], "fields":["small"], "limit":limit}}})
}

fn query(session: &str, request: &Value) -> TestResult<(String, Value)> {
    let raw =
        crate::agent_facade::fortress_query(Some(session.to_owned()), Some(request.to_string()));
    let parsed = decode(&raw)?;
    Ok((raw, parsed))
}

fn turn(page: &Value) -> TestResult<u64> {
    Ok(page["agent_turn"]["turn_id"]
        .as_str()
        .ok_or("turn missing")?
        .strip_prefix("presentation-turn-")
        .ok_or("unexpected turn identity")?
        .parse()?)
}

fn narrowed_tokens(session: &str, one_complete_packet: &str) -> TestResult<u32> {
    let tokens = u32::try_from((one_complete_packet.len() + 128).div_ceil(4))?;
    crate::server::resolve_session(Some(session.to_owned()))?
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = tokens;
    Ok(tokens)
}

fn advance(session: &str) -> TestResult<WorldSnapshot> {
    let owner = crate::server::resolve_session(Some(session.to_owned()))?;
    let mut guard = owner.lock().map_err(|_| "session poisoned")?;
    let mut source = guard.adapter.snapshot().clone();
    source.tick = GameTick(source.tick.0 + 1);
    source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
    let entity = source
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or("entity missing")?;
    entity.label = "Urist after the next observation".to_owned();
    entity.revision += 1;
    source.refresh_hash();
    guard.adapter.inject_snapshot(source.clone())?;
    Ok(source)
}

fn refused(page: &Value, code: &str) {
    assert_eq!(page["ok"], false, "{page}");
    assert_eq!(page["error"]["code"], code, "{page}");
    assert!(page.get("rows").is_none());
    assert!(page.get("continuation").is_none());
    assert_eq!(page["error"]["effect_may_have_occurred"], false);
}

#[test]
fn projected_entities_facade_pages_fit_the_complete_turn_without_skipping_rows() -> TestResult {
    let _serial = crate::test_serial();
    let (session, source) = fixture(12, 1024, 16_384, false)?;
    let direct = decode(&crate::server::fortress_query(
        Some(session.clone()),
        Some(request(1).to_string()),
    ))?;
    assert_eq!(direct["ok"], true, "{direct}");
    assert_eq!(direct["returned"], 1);
    assert_eq!(
        direct["rows"]
            .as_array()
            .ok_or("direct rows missing")?
            .len(),
        1
    );
    assert_eq!(direct["rows"][0]["entity_id"], "1");
    assert!(direct["continuation"].is_string());
    let (one_raw, one) = query(&session, &request(1))?;
    assert_eq!(one["ok"], true, "{one}");
    let tokens = narrowed_tokens(&session, &one_raw)?;
    let mut previous_turn = turn(&one)?;
    let mut selected = request(12);
    let mut ids = Vec::new();
    for index in 0..12 {
        let (raw, page) = query(&session, &selected)?;
        assert_eq!(page["ok"], true, "{page}");
        assert!(raw.len() <= tokens as usize * 4);
        assert!(matches!(
            page["output_budget"]["tier"].as_str(),
            Some("full" | "compact_turn")
        ));
        assert_eq!(turn(&page)?, previous_turn + 1);
        previous_turn = turn(&page)?;
        let rows = page["rows"].as_array().ok_or("rows missing")?;
        assert!(!rows.is_empty());
        if index == 0 {
            assert!(rows.len() < 12);
        }
        assert_eq!(page["returned"], rows.len());
        assert_eq!(page["matched"], 12);
        for row in rows {
            assert!(row["fields"].get("unselected").is_none());
            ids.push(
                row["entity_id"]
                    .as_str()
                    .ok_or("entity missing")?
                    .to_owned(),
            );
        }
        assert_eq!(
            page["source_work"]["canonical_bytes"],
            source.canonical_bytes().len()
        );
        assert_eq!(
            page["observation_coverage"]["continuation"],
            page["continuation"]
        );
        assert_eq!(
            page["agent_turn"]["coverage"]["continuation"],
            page["continuation"]
        );
        if page["continuation"].is_null() {
            break;
        }
        selected["request"]["query"]["continuation"] = page["continuation"].clone();
    }
    assert_eq!(ids, (1..=12).map(|id| id.to_string()).collect::<Vec<_>>());
    assert_eq!(
        crate::server::resolve_session(Some(session))?
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot(),
        &source
    );
    Ok(())
}

#[test]
fn projected_entities_facade_preserves_pending_work_under_a_small_output_budget() -> TestResult {
    let _serial = crate::test_serial();
    let (session, _) = fixture(8, 1024, 0, true)?;
    assert_eq!(query(&session, &request(1))?.1["ok"], true);
    let plan = decode(&crate::agent_facade::fortress_plan(
        Some(session.clone()),
        Some("retain this reviewed pause plan".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let (one_raw, one) = query(&session, &request(1))?;
    assert_eq!(one["ok"], true, "{one}");
    let tokens = narrowed_tokens(&session, &one_raw)?;
    let (raw, page) = query(&session, &request(8))?;
    assert_eq!(page["ok"], true, "{page}");
    assert!(raw.len() <= tokens as usize * 4);
    assert_eq!(turn(&page)?, turn(&one)? + 1);
    assert_eq!(
        page["agent_turn"]["active_work"]["pending_plans"],
        one["agent_turn"]["active_work"]["pending_plans"]
    );
    assert_eq!(
        page["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        plan["plan_digest"]
    );
    assert_eq!(
        page["agent_turn"]["active_work"]["pending_plans"][0]["state"],
        "prepared"
    );
    Ok(())
}

#[test]
fn projected_entities_facade_refuses_when_context_and_one_whole_row_cannot_fit() -> TestResult {
    let _serial = crate::test_serial();
    let (session, _) = fixture(2, 2048, 16_384, false)?;
    let raw = crate::server::fortress_query(Some(session.clone()), Some(request(1).to_string()));
    assert_eq!(decode(&raw)?["ok"], true);
    let tokens = u32::try_from(raw.len().div_ceil(4) + 1)?;
    crate::server::resolve_session(Some(session.clone()))?
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = tokens;
    let (raw, page) = query(&session, &request(1))?;
    refused(&page, "budget_exceeded");
    assert!(raw.len() <= tokens as usize * 4);
    assert_eq!(page["agent_turn"]["changes"], json!([]));
    assert_ne!(page["agent_turn"]["continuity"]["status"], "continuous");
    Ok(())
}

#[test]
fn projected_entities_facade_refusals_preserve_the_previous_observation_basis() -> TestResult {
    let _serial = crate::test_serial();
    let (session, _) = fixture(3, 64, 0, true)?;
    let (_, before) = query(&session, &request(1))?;
    assert_eq!(before["ok"], true, "{before}");
    let current = advance(&session)?;
    let mut invalid = request(3);
    invalid["request"]["query"]["where"] = json!({"op":"unregistered_predicate"});
    let mut stale = request(3);
    stale["request"]["expected_anchor"] = before["anchor"].clone();
    let mut last_turn = turn(&before)?;
    for (request, code) in [(invalid, "invalid_request"), (stale, "stale_anchor")] {
        let (_, page) = query(&session, &request)?;
        refused(&page, code);
        assert_eq!(turn(&page)?, last_turn + 1);
        last_turn = turn(&page)?;
        assert_eq!(page["agent_turn"]["anchor"], before["agent_turn"]["anchor"]);
        assert_eq!(page["agent_turn"]["changes"], json!([]));
        assert_eq!(page["agent_turn"]["continuity"]["status"], "stale");
    }
    let (_, after) = query(&session, &request(3))?;
    assert_eq!(after["ok"], true, "{after}");
    assert_eq!(turn(&after)?, last_turn + 1);
    assert_eq!(
        after["agent_turn"]["anchor"]["state_hash"],
        current.state_hash.to_string()
    );
    assert_eq!(
        after["agent_turn"]["continuity"]["basis"],
        before["agent_turn"]["anchor"]
    );
    assert!(
        after["agent_turn"]["changes"]
            .as_array()
            .is_some_and(|changes| !changes.is_empty())
    );
    Ok(())
}

#[test]
fn projected_entities_facade_historical_pages_require_current_query_authority() -> TestResult {
    let _serial = crate::test_serial();
    let (session, source) = fixture(5, 512, 16_384, false)?;
    let (_, first) = query(&session, &request(1))?;
    assert_eq!(first["ok"], true, "{first}");
    let current = advance(&session)?;
    let mut retained = request(1);
    retained["request"]["query"]["continuation"] = first["continuation"].clone();
    retained["at"] = json!(source.state_hash.to_string());
    let (_, page) = query(&session, &retained)?;
    assert_eq!(page["ok"], true, "{page}");
    assert_eq!(page["historical"], true);
    assert_eq!(page["anchor"], first["anchor"]);
    assert_eq!(page["rows"][0]["entity_id"], "2");
    assert_eq!(
        page["agent_turn"]["anchor"]["state_hash"],
        current.state_hash.to_string()
    );
    crate::server::resolve_session(Some(session.clone()))?
        .lock()
        .map_err(|_| "session poisoned")?
        .grants
        .iter_mut()
        .find(|grant| grant.capability == Capability::Query)
        .ok_or("Query grant missing")?
        .expires_at_tick = Some(source.tick);
    refused(&query(&session, &retained)?.1, "capability_denied");
    Ok(())
}

#[test]
fn projected_entities_preparation_rechecks_context_without_a_second_semantic_request() -> TestResult
{
    let _serial = crate::test_serial();
    for case in 0..4 {
        let (session, _) = fixture(2, 0, 0, true)?;
        let owner = crate::server::resolve_session(Some(session.clone()))?;
        let before = crate::server::read_context(&owner.lock().map_err(|_| "session poisoned")?)
            .request_id
            .get();
        let mut started = Instant::now();
        let prepared = crate::server::fortress_query_publication(
            Some(session.clone()),
            Some(request(1).to_string()),
            started,
        );
        let crate::server::QueryPublication::Projected {
            query: prepared, ..
        } = prepared
        else {
            return Err("entity query was not deferred".into());
        };
        let after_prepare =
            crate::server::read_context(&owner.lock().map_err(|_| "session poisoned")?)
                .request_id
                .get();
        assert_eq!(after_prepare, before + 1);
        let expected = match case {
            0 => {
                owner.lock().map_err(|_| "session poisoned")?.grants.clear();
                "capability_denied"
            }
            1 => {
                owner
                    .lock()
                    .map_err(|_| "session poisoned")?
                    .budget
                    .max_output_tokens -= 1;
                "budget_exceeded"
            }
            2 => {
                advance(&session)?;
                "stale_anchor"
            }
            _ => {
                started = started
                    .checked_sub(Duration::from_millis(60_001))
                    .ok_or("clock underflow")?;
                "budget_exceeded"
            }
        };
        let calls = std::cell::Cell::new(0);
        let result = prepared.execute(started, &|_| {
            calls.set(calls.get() + 1);
            Ok(true)
        });
        refused(&result, expected);
        assert_eq!(calls.get(), 0);
        let after = crate::server::read_context(&owner.lock().map_err(|_| "session poisoned")?)
            .request_id
            .get();
        assert_eq!(after, after_prepare);
    }
    Ok(())
}
