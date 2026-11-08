//! Selected endpoint changes through the actual default eleven-tool facade.
//! Owning bead: df-lab-structured-query-f7q.1.

use super::*;
use dfmcp_world::{
    EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value as WorldValue, WorldGraph,
};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(scenario: &str) -> TestResult<String> {
    let result = parsed(&crate::agent_facade::fortress_open_session(
        Some(true),
        None,
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "reversible".to_owned()),
            ("configure_labor".to_owned(), "reversible".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
        ]),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some(scenario.to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["session_id"]
        .as_str()
        .ok_or("session ID missing")?
        .to_owned())
}

fn fixture() -> TestResult<(String, WorldSnapshot)> {
    let id = open("empty")?;
    let session = resolve_session(Some(id.clone()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    let mut snapshot = guard.adapter.snapshot().clone();
    snapshot.tick = GameTick(10);
    snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
    snapshot.graph = WorldGraph::default();
    for number in 1..=3 {
        snapshot.graph.entities.insert(
            EntityId::new(number),
            EntityRecord {
                id: EntityId::new(number),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: format!("Urist {number}"),
                fields: BTreeMap::from([(
                    "stock".to_owned(),
                    Fact::known(
                        WorldValue::U64(number),
                        snapshot.tick,
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::of_bytes(b"comparison-public-fixture"),
                    ),
                )]),
            },
        );
    }
    snapshot.refresh_hash();
    guard.adapter.inject_snapshot(snapshot.clone())?;
    drop(guard);
    Ok((id, snapshot))
}

fn advance(id: &str) -> TestResult<WorldSnapshot> {
    let session = resolve_session(Some(id.to_owned()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    remember_version(&mut guard);
    let mut after = guard.adapter.snapshot().clone();
    after.tick = GameTick(after.tick.0 + 10);
    after.cursor = after.cursor.checked_next().ok_or("cursor exhausted")?;
    for entity in after.graph.entities.values_mut() {
        entity.revision += 1;
        let fact = entity.fields.get_mut("stock").ok_or("stock missing")?;
        fact.observed_at = after.tick;
        fact.value = WorldValue::U64(100);
    }
    after
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or("unit missing")?
        .generation += 1;
    after.refresh_hash();
    guard.adapter.inject_snapshot(after.clone())?;
    Ok(after)
}

fn request(before: &WorldSnapshot) -> Value {
    json!({"mode":"query_changes", "before":before.state_hash.to_string(),
        "select":{"kind":"entities","kinds":["unit"],"fields":["stock"]}})
}

fn query(id: &str, value: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(id.to_owned()),
        Some(value.to_string()),
    ))
}

fn refusal(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("changes").is_none(), "{value}");
    assert!(
        value.get("continuation").is_none_or(Value::is_null),
        "{value}"
    );
    assert!(value["agent_turn"].is_object(), "{value}");
}

#[test]
fn lab_comparison_public_plan_commit_query_loop_proves_selected_labor_change() -> TestResult {
    let _serial = crate::test_serial();
    let id = open("starter_fortress")?;
    let before = resolve_session(Some(id.clone()))?
        .lock()
        .map_err(|_| "session poisoned")?
        .adapter
        .snapshot()
        .clone();
    let current = before
        .graph
        .entities
        .get(&EntityId::new(1001))
        .and_then(|entity| entity.fields.get("labor.MINING"))
        .is_some_and(|fact| fact.value == WorldValue::Bool(true));
    let planned = parsed(&crate::agent_facade::fortress_plan(
        Some(id.clone()),
        None,
        None,
        Some(
            json!([{"action":{"kind":"set_labor","units":["1001"],
            "labor":"MINING","enabled":!current}}])
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(id.clone()),
        planned["plan_digest"]
            .as_str()
            .ok_or("plan digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let result = query(
        &id,
        json!({"mode":"query_changes",
        "before":before.state_hash.to_string(),
        "select":{"kind":"entities","kinds":["unit"],"fields":["labor.MINING"]}}),
    )?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["change_count"], 1, "{result}");
    assert_eq!(result["changes"][0]["entity_id"], "1001");
    assert_eq!(result["changes"][0]["kind"], "changed_in_result");
    assert_eq!(
        result["changes"][0]["after"]["fields"]["labor.MINING"]["value"]["value"],
        !current
    );
    assert_eq!(result["basis"], anchor_json(&before.anchor()));
    assert_eq!(result["agent_turn"]["anchor"], result["current_anchor"]);
    assert_eq!(result["eligible_as_mutation_evidence"], false);
    assert_eq!(
        result["coverage"]["intermediate_observations_evaluated"],
        false
    );
    Ok(())
}

#[test]
fn lab_comparison_public_pages_preserve_generation_changes_and_bind_selection() -> TestResult {
    let _serial = crate::test_serial();
    let (id, before) = fixture()?;
    let after = advance(&id)?;
    let mut input = request(&before);
    input["limit"] = json!(1);
    let first = query(&id, input.clone())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(first["change_count"], 4);
    assert_eq!(first["source_work"]["records"], 6);
    assert_eq!(first["returned"], 1);
    assert_eq!(first["agent_turn"]["continuity"]["basis"], first["basis"]);
    let mut changes = first["changes"]
        .as_array()
        .ok_or("changes missing")?
        .clone();
    let mut token = first["continuation"].clone();
    while !token.is_null() {
        input["continuation"] = token;
        let page = query(&id, input.clone())?;
        assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["changes"], query(&id, input.clone())?["changes"]);
        changes.extend(page["changes"].as_array().ok_or("changes missing")?.clone());
        token = page["continuation"].clone();
    }
    assert_eq!(changes.len(), 4);
    assert_eq!(changes[1]["kind"], "left_result");
    assert_eq!(changes[1]["generation"], 1);
    assert_eq!(changes[2]["kind"], "entered_result");
    assert_eq!(changes[2]["generation"], 2);
    input["continuation"] = first["continuation"].clone();
    input["select"]["fields"] = json!([]);
    refusal(&query(&id, input)?, "stale_anchor");
    assert_eq!(
        resolve_session(Some(id))?
            .lock()
            .map_err(|_| "session poisoned")?
            .adapter
            .snapshot()
            .anchor(),
        after.anchor()
    );
    Ok(())
}

#[test]
fn lab_comparison_public_historical_heartbeat_keeps_current_anchor_and_one_source_charge()
-> TestResult {
    let _serial = crate::test_serial();
    let (id, before) = fixture()?;
    let after = advance(&id)?;
    let session = resolve_session(Some(id.clone()))?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_entities = 3;
    let mut input = request(&before);
    input["after"] = json!(before.state_hash.to_string());
    let result = query(&id, input)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["change_count"], 0);
    assert_eq!(result["source_work"]["records"], 3);
    assert_eq!(result["historical"], true);
    assert_eq!(result["anchor"], anchor_json(&before.anchor()));
    assert_eq!(result["current_anchor"], anchor_json(&after.anchor()));
    assert_eq!(result["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(result["agent_turn"]["anchor"], result["current_anchor"]);
    let refused = query(&id, request(&before))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert!(refused.get("changes").is_none());
    assert!(refused.get("continuation").is_none_or(Value::is_null));
    Ok(())
}

#[test]
fn lab_comparison_public_permission_precedes_retention_lookup() -> TestResult {
    let _serial = crate::test_serial();
    let (id, before) = fixture()?;
    let mut input = request(&before);
    input["before"] = json!("f".repeat(64));
    refusal(&query(&id, input.clone())?, "stale_anchor");
    let session = resolve_session(Some(id.clone()))?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .grants
        .clear();
    refusal(&query(&id, input)?, "capability_denied");
    Ok(())
}

#[test]
fn lab_comparison_public_redaction_never_exposes_underlying_value() -> TestResult {
    let _serial = crate::test_serial();
    let (id, before) = fixture()?;
    let session = resolve_session(Some(id.clone()))?;
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        remember_version(&mut guard);
        let mut after = before.clone();
        after.cursor = after.cursor.checked_next().ok_or("cursor exhausted")?;
        let field = after
            .graph
            .entities
            .get_mut(&EntityId::new(1))
            .ok_or("unit missing")?
            .fields
            .get_mut("stock")
            .ok_or("stock missing")?;
        field.value = WorldValue::Text("concealed-comparison-payload".to_owned());
        field.presence = Some(FactPresence::Redacted("policy".to_owned()));
        after.refresh_hash();
        guard.adapter.inject_snapshot(after)?;
    }
    let result = query(&id, request(&before))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(
        result["changes"][0]["after"]["fields"]["stock"]["presence"],
        "redacted"
    );
    assert!(!result.to_string().contains("concealed-comparison-payload"));
    Ok(())
}

#[test]
fn lab_comparison_public_overflow_refuses_the_complete_page_without_a_cursor() -> TestResult {
    let _serial = crate::test_serial();
    let (id, before) = fixture()?;
    advance(&id)?;
    let full = query(&id, request(&before))?;
    assert_eq!(full["ok"], true, "{full}");
    let session = resolve_session(Some(id.clone()))?;
    session
        .lock()
        .map_err(|_| "session poisoned")?
        .budget
        .max_output_tokens = 128;
    let refused = query(&id, request(&before))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert!(refused.get("changes").is_none());
    assert!(refused.get("continuation").is_none_or(Value::is_null));
    Ok(())
}
