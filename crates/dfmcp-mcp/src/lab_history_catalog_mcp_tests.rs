//! Public history discovery resumes exact reads without transcript-held hashes.
//! Owning bead: df-lab-structured-query-f7q.4.

use super::*;
use dfmcp_core::ObservationCursor;
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner(String);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.0, 16) {
            sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open() -> TestResult<Owner> {
    let value = parsed(&crate::agent_facade::fortress_open_session(
        Some(false),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
        ]),
        Some(60_000),
        Some(5_000),
        Some(16_384),
        Some(4 * 1024 * 1024),
        Some(16_384),
        Some(64),
        Some("starter_fortress".into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(Owner(value["session_id"].as_str().ok_or("session missing")?.into()))
}

fn query(owner: &Owner, input: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(owner.0.clone()),
        Some(input.to_string()),
    ))
}

fn temporal(owner: &Owner, query_input: Value) -> TestResult<Value> {
    query(owner, json!({"mode":"temporal","request":{
        "schema":"dfmcp.query/1","query":query_input}}))
}

fn source(owner: &Owner) -> TestResult<WorldSnapshot> {
    Ok(resolve_session(Some(owner.0.clone()))?
        .lock().map_err(|_| "session lock")?
        .adapter.snapshot().clone())
}

fn advance(owner: &Owner) -> TestResult {
    let value = parsed(&crate::agent_facade::fortress_wait(Some(owner.0.clone()), Some(1)))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(())
}

fn replay_request(window: &Value, stable: u32) -> Value {
    json!({"mode":"condition_replay","from":window["from"],"to":window["to"],
        "definition":{"condition":{"op":"tick_at_least","value":0},
        "deadline_tick":10_000,"poll_interval_ticks":1,"stable_observations":stable},
        "detail":"evidence"})
}

fn refusal(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("records").is_none(), "{value}");
    assert!(value.get("replay_windows").is_none(), "{value}");
    assert!(value.get("continuation").is_none_or(Value::is_null), "{value}");
}

#[test]
fn history_public_discovery_supplies_replay_comparison_and_at_endpoints_after_context_loss()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    advance(&owner)?;
    advance(&owner)?;
    let current = source(&owner)?;
    let watched = temporal(&owner, json!({"kind":"watch","key":"unrelated-future",
        "condition":{"op":"tick_at_least","value":100},"deadline_tick":1000,
        "stable_observations":2}))?;
    assert_eq!(watched["ok"], true, "{watched}");
    let baseline = temporal(&owner, json!({"kind":"capture","key":"unrelated-units",
        "select":{"kind":"entities","kinds":["unit"],"fields":["alive"]},
        "max_game_ticks":1000}))?;
    assert_eq!(baseline["ok"], true, "{baseline}");
    let pending = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.0.clone()), Some("retain independent pending review".into()),
        Some(true), None, None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");
    let handle = resolve_session(Some(owner.0.clone()))?;
    let retained_before = format!("{:?}", handle.lock().map_err(|_| "session lock")?.history);
    let watches_before = temporal(&owner, json!({"kind":"watches"}))?["records"].clone();
    let baselines_before = temporal(&owner, json!({"kind":"baselines"}))?["baselines"].clone();

    // No earlier state hash is supplied. A fresh caller follows the handoff.
    let discover = {
        let guard = handle.lock().map_err(|_| "session lock")?;
        handoff_json(&guard)["orientation"]["retained_history"].clone()
    };
    assert_eq!(discover["tool"], "fortress.query");
    assert_eq!(discover["arguments"]["session_id"], owner.0);
    let discovery_mode = discover["arguments"]["mode"].as_str().ok_or("history next step")?;
    let catalog = query(&owner, serde_json::from_str(discovery_mode)?)?;
    assert_eq!(catalog["ok"], true, "{catalog}");
    assert_eq!(catalog["mode"], "history");
    assert_eq!(catalog["current_anchor_retained"], true);
    assert_eq!(catalog["anchor"], anchor_json(&current.anchor()));
    assert_eq!(catalog["agent_turn"]["anchor"], anchor_json(&current.anchor()));
    assert_eq!(catalog["world_contents_validated"], false);
    let work = &catalog["agent_turn"]["active_work"];
    assert_eq!(work["query_work_coverage"]["status"], "available");
    assert_eq!(work["query_watches"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["query_baselines"].as_array().map(Vec::len), Some(1));
    assert_eq!(work["pending_plans"][0]["plan_digest"], pending["plan_digest"]);
    let window = catalog["replay_windows"].as_array().and_then(|rows| rows.first())
        .ok_or("history replay window")?;

    let replay = query(&owner, replay_request(window, 2))?;
    assert_eq!(replay["ok"], true, "{replay}");
    assert_eq!(replay["status"], "satisfied", "{replay}");
    assert_eq!(replay["range"]["from"], window["from"]);
    assert_eq!(replay["range"]["to"], window["to"]);
    let at = query(&owner, json!({"mode":"structured","at":window["from"],
        "request":{"schema":"dfmcp.query/1","query":{"kind":"entities",
        "kinds":["unit"],"fields":["alive"],"limit":1}}}))?;
    assert_eq!(at["ok"], true, "{at}");
    assert_eq!(at["anchor"]["state_hash"], window["from"]);
    let compared = query(&owner, json!({"mode":"query_changes","before":window["from"],
        "after":window["to"],"select":{"kind":"entities","kinds":["unit"],
        "fields":["alive"]},"limit":1}))?;
    assert_eq!(compared["ok"], true, "{compared}");
    assert_eq!(compared["basis"]["state_hash"], window["from"]);
    assert_eq!(compared["anchor"]["state_hash"], window["to"]);
    assert_eq!(source(&owner)?, current);
    assert_eq!(temporal(&owner, json!({"kind":"watches"}))?["records"], watches_before);
    assert_eq!(temporal(&owner, json!({"kind":"baselines"}))?["baselines"], baselines_before);
    assert_eq!(format!("{:?}", handle.lock().map_err(|_| "session lock")?.history), retained_before);
    Ok(())
}

#[test]
fn history_public_collected_interiors_and_restore_epochs_never_become_replay_windows()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    let mut sources = vec![source(&owner)?];
    for _ in 0..3 {
        advance(&owner)?;
        sources.push(source(&owner)?);
    }
    let handle = resolve_session(Some(owner.0.clone()))?;
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let mut history = dfmcp_world::retention::VersionRetention::new(1, 2);
        for source in &sources {
            history.record(source);
        }
        history.collect(&[sources[0].state_hash, sources[2].state_hash].into_iter().collect());
        guard.history = history;
    }
    let catalog = query(&owner, json!({"mode":"history","order":"oldest_first","limit":32}))?;
    assert_eq!(catalog["ok"], true, "{catalog}");
    assert_eq!(catalog["records"][1]["recording_gap_before"]["count"], 1);
    assert_eq!(catalog["replay_windows"].as_array().map(Vec::len), Some(2));
    for window in catalog["replay_windows"].as_array().ok_or("windows")? {
        let replayed = query(&owner, replay_request(window, 1))?;
        assert_eq!(replayed["ok"], true, "{replayed}");
        assert_eq!(replayed["status"], "satisfied", "{replayed}");
    }
    let crossing = json!({"from":catalog["records"][0]["anchor"]["state_hash"],
        "to":catalog["records"][2]["anchor"]["state_hash"]});
    refusal(&query(&owner, replay_request(&crossing, 1))?, "stale_anchor");
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let mut restored = guard.adapter.snapshot().clone();
        restored.cursor = ObservationCursor { epoch:restored.cursor.epoch + 1, sequence:0 };
        // Preserve the complete world's fact times; only epoch identity is
        // changed in this boundary test, not a simulated checkpoint restore.
        restored.refresh_hash();
        guard.adapter.inject_snapshot(restored.clone())?;
        guard.history.record(&restored);
    }
    let catalog = query(&owner, json!({"mode":"history","order":"oldest_first","limit":32}))?;
    assert_eq!(catalog["ok"], true, "{catalog}");
    assert_eq!(catalog["current_epoch_candidate_versions"], 1);
    assert_eq!(catalog["records"][0]["condition_replay_eligibility"], "different_epoch");
    assert_eq!(catalog["replay_windows"].as_array().map(Vec::len), Some(1));
    assert_eq!(catalog["replay_windows"][0]["versions"], 1);
    let replayed = query(&owner, replay_request(&catalog["replay_windows"][0], 1))?;
    assert_eq!(replayed["ok"], true, "{replayed}");
    let crossing = json!({"from":catalog["records"][0]["anchor"]["state_hash"],
        "to":catalog["replay_windows"][0]["to"]});
    refusal(&query(&owner, replay_request(&crossing, 1))?, "stale_anchor");
    Ok(())
}

#[test]
fn history_public_budget_and_authority_refusals_preserve_catalog_and_unrecorded_frontier()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    advance(&owner)?;
    let handle = resolve_session(Some(owner.0.clone()))?;
    let (retained_before, original_budget) = {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let mut changed = guard.adapter.snapshot().clone();
        changed.cursor = changed.cursor.checked_next().ok_or("cursor")?;
        changed.tick = GameTick(changed.tick.0 + 1);
        changed.refresh_hash();
        guard.adapter.inject_snapshot(changed)?;
        (format!("{:?}", guard.history), guard.budget)
    };
    let catalog = query(&owner, json!({"mode":"history"}))?;
    assert_eq!(catalog["ok"], true, "{catalog}");
    assert_eq!(catalog["current_anchor_retained"], false);
    assert_eq!(format!("{:?}", handle.lock().map_err(|_| "session lock")?.history), retained_before);
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        guard.budget.max_output_tokens = 256;
        guard.budget.max_bytes = 1024;
    }
    let raw = crate::agent_facade::fortress_query(
        Some(owner.0.clone()), Some(json!({"mode":"history"}).to_string()));
    let value = parsed(&raw)?;
    refusal(&value, "budget_exceeded");
    assert!(raw.len() <= 1024, "{raw}");
    assert_eq!(format!("{:?}", handle.lock().map_err(|_| "session lock")?.history), retained_before);
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        guard.budget = original_budget;
        guard.grants.retain(|grant| grant.capability != Capability::Query);
    }
    refusal(&query(&owner, json!({"mode":"history"}))?, "capability_denied");
    assert_eq!(format!("{:?}", handle.lock().map_err(|_| "session lock")?.history), retained_before);
    Ok(())
}

#[test]
fn history_public_consumed_cursor_marks_replay_nonportable_but_first_page_does_not()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    advance(&owner)?;
    advance(&owner)?;
    let first = query(&owner, json!({"mode":"history","limit":1}))?;
    assert_eq!(first["ok"], true, "{first}");
    assert!(first["continuation"].is_string());
    assert_eq!(replay_bundle_for(&owner.0)?["replayable"], true);
    let next = query(&owner, first["next_query"].clone())?;
    assert_eq!(next["ok"], true, "{next}");
    assert_ne!(next["records"][0]["anchor"], first["records"][0]["anchor"]);
    let bundle = replay_bundle_for(&owner.0)?;
    assert_eq!(bundle["replayable"], false);
    assert!(bundle["not_replayable_reason"].as_str()
        .is_some_and(|reason| reason.contains("continuations")));
    Ok(())
}
