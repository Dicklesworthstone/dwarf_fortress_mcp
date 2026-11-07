//! Regressions for the actual default public query routes and retained sources.
//! Owning implementation bead: df-lab-structured-query-f7q.

use super::*;
use dfmcp_world::{EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph};
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
        ]),
        Some(60_000),
        None,
        Some(128),
        Some(1024 * 1024),
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
    let session = resolve_session(Some(id.clone()))?;
    let source = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut source = guard.adapter.snapshot().clone();
        source.tick = GameTick(10);
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.graph = WorldGraph::default();
        for n in 1..=12 {
            let known = |value| {
                Fact::known(
                    value,
                    source.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::of_bytes(b"query-budget-fixture"),
                )
            };
            let entity = EntityRecord {
                id: EntityId::new(n),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {n}"),
                fields: BTreeMap::from([
                    ("alive".to_owned(), known(WorldValue::Bool(true))),
                    ("stock".to_owned(), known(WorldValue::U64(n))),
                    (
                        "profession".to_owned(),
                        known(WorldValue::Text("Miner".to_owned())),
                    ),
                    // Escaped text makes the complete JSON page larger than
                    // its admitted canonical source, exercising publication.
                    (
                        "notes".to_owned(),
                        known(WorldValue::Text("\u{0001}".repeat(64))),
                    ),
                ]),
            };
            source.graph.entities.insert(entity.id, entity);
        }
        source.refresh_hash();
        guard.adapter.inject_snapshot(source.clone())?;
        remember_version(&mut guard);
        source
    };
    let initialized = parsed(&crate::agent_facade::fortress_query(
        Some(id.clone()),
        Some("summary".to_owned()),
    ))?;
    assert_eq!(initialized["ok"], true, "{initialized}");
    Ok((id, source))
}

fn query(session: &str, mode: Value) -> TestResult<Value> {
    let raw = mode
        .as_str()
        .map_or_else(|| mode.to_string(), str::to_owned);
    parsed(&crate::agent_facade::fortress_query(
        Some(session.to_owned()),
        Some(raw),
    ))
}

fn refused(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    for key in ["rows", "hits", "levels", "changes", "continuation"] {
        assert!(
            value.get(key).is_none(),
            "unpublished result retained: {value}"
        );
    }
    assert!(value["agent_turn"].is_object(), "{value}");
    assert_eq!(
        value["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
}

fn modes(hash: Digest32) -> Vec<Value> {
    vec![
        json!("summary"),
        json!("entities"),
        json!({"mode":"entities", "limit":1}),
        json!({"mode":"entities", "kind":"building", "limit":1}),
        json!({"mode":"search", "text":"Urist", "limit":1}),
        json!({"mode":"observation", "completeness_profile":"operations", "section":"entities", "limit":1}),
        json!({"mode":"observation", "completeness_profile":"control-minimum", "section":"chunks", "limit":1}),
        json!({"mode":"terrain", "min":[0,0,0], "max":[0,0,0]}),
        json!({"mode":"path", "from":[0,0,0], "to":[1,0,0]}),
        json!({"mode":"entities", "limit":1, "at":hash.to_string()}),
        json!({"mode":"changes", "since":hash.to_string()}),
        json!("civilian_recovery"),
    ]
}

#[test]
fn legacy_query_modes_cannot_bypass_complete_source_record_or_byte_budgets() -> TestResult {
    let _serial = crate::test_serial();
    let (id, source) = fixture()?;
    let session = resolve_session(Some(id.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 1;
    }
    for mode in modes(source.state_hash) {
        refused(&query(&id, mode)?, "budget_exceeded");
    }
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 128;
        guard.budget.max_bytes = source.canonical_bytes().len() as u64 - 1;
    }
    for mode in modes(source.state_hash) {
        let result = query(&id, mode)?;
        refused(&result, "budget_exceeded");
        assert!(result.to_string().len() < source.canonical_bytes().len());
    }
    assert_eq!(
        session.lock().map_err(|_| "session poisoned")?.adapter.snapshot(),
        &source
    );
    Ok(())
}

#[test]
fn legacy_changes_charge_both_endpoints_and_historical_reads_recheck_current_authority() -> TestResult
{
    let _serial = crate::test_serial();
    let (id, old) = fixture()?;
    let session = resolve_session(Some(id.clone()))?;
    let current = {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut current = old.clone();
        current.tick = GameTick(20);
        current.cursor = current.cursor.checked_next().ok_or("cursor exhausted")?;
        current
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("unit missing")?
            .label = "changed".to_owned();
        current.refresh_hash();
        guard.adapter.inject_snapshot(current.clone())?;
        guard.budget.max_entities = 12;
        current
    };
    let historical = json!({"mode":"entities", "limit":1, "at":old.state_hash.to_string()});
    let result = query(&id, historical.clone())?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["anchor"], anchor_json(&old.anchor()));
    assert_eq!(
        result["agent_turn"]["anchor"],
        anchor_json(&current.anchor())
    );
    let changes_request = json!({"mode":"changes", "since":old.state_hash.to_string()});
    refused(&query(&id, changes_request.clone())?, "budget_exceeded");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_entities = 24;
        guard.budget.max_bytes =
            (old.canonical_bytes().len() + current.canonical_bytes().len()) as u64 - 1;
    }
    refused(&query(&id, changes_request.clone())?, "budget_exceeded");
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_bytes = 1024 * 1024;
    }
    let changes = query(&id, changes_request)?;
    assert_eq!(changes["ok"], true, "{changes}");
    assert_eq!(changes["source_work"]["records"], 24);
    let same = query(
        &id,
        json!({"mode":"changes", "since":current.state_hash.to_string()}),
    )?;
    assert_eq!(same["ok"], true, "{same}");
    assert_eq!(same["source_work"]["records"], 12);
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let grant = guard
            .grants
            .iter_mut()
            .find(|grant| grant.capability == Capability::Query)
            .ok_or("query grant missing")?;
        grant.expires_at_tick = Some(GameTick(15));
    }
    refused(&query(&id, historical)?, "capability_denied");
    refused(
        &query(&id, json!({"mode":"entities", "at":"f".repeat(64)}))?,
        "capability_denied",
    );
    Ok(())
}

#[test]
fn legacy_pages_honor_narrowed_publication_budgets_and_resume_every_row() -> TestResult {
    let _serial = crate::test_serial();
    let (id, source) = fixture()?;
    assert!(source.canonical_bytes().len() < 8192);
    let session = resolve_session(Some(id.clone()))?;
    for mode in ["entities", "observation"] {
        {
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            guard.budget.max_bytes = 8192;
            guard.budget.max_output_tokens = 32_768;
        }
        let mut request = if mode == "entities" {
            json!({"mode":"entities", "limit":12})
        } else {
            json!({"mode":"observation", "completeness_profile":"operations", "section":"entities", "limit":12})
        };
        let mut ids = Vec::new();
        let mut clipped = false;
        for _ in 0..12 {
            let page = query(&id, request.clone())?;
            assert_eq!(page["ok"], true, "{page}");
            assert!(page.to_string().len() <= 8192, "{page}");
            let rows = page["rows"].as_array().ok_or("rows missing")?;
            assert!(!rows.is_empty(), "nonprogressing page: {page}");
            assert_eq!(page["returned"], rows.len());
            ids.extend(rows.iter().map(|row| row["entity_id"].clone()));
            clipped |= page["output_budget"]["tier"] == "truncated";
            if page["continuation"].is_null() {
                break;
            }
            assert_eq!(page["next_offset"], ids.len());
            assert_eq!(
                page["continuation"]["at"],
                source.state_hash.to_string()
            );
            assert_eq!(page["complete_domain"], false);
            request = page["continuation"].clone();
        }
        assert!(clipped, "fixture must exercise public row clipping");
        assert_eq!(
            ids,
            (1..=12).map(|n| json!(n.to_string())).collect::<Vec<_>>()
        );
    }
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_bytes = 1024 * 1024;
        guard.budget.max_output_tokens = 2048;
    }
    let response = query(&id, json!({"mode":"entities", "limit":1}))?;
    assert_eq!(response["ok"], true, "{response}");
    assert!(response.to_string().len() <= 8192);
    assert_eq!(response["query_budget"]["max_output_tokens"], 2048);
    let late = parsed(&query_budget::finish_response(
        response.to_string(),
        std::time::Duration::from_millis(60_000),
    ))?;
    refused(&late, "budget_exceeded");
    assert!(
        late["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("wall-time"))
    );
    Ok(())
}
