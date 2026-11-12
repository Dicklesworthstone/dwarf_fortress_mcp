//! Original gameplay observations replay through the actual eleven-tool facade.
//! Owning bead: df-lab-structured-query-f7q.3.

use super::*;
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
    let result = parsed(&crate::agent_facade::fortress_open_session(
        Some(false),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("configure_labor".into(), "reversible".into()),
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
    assert_eq!(result["ok"], true, "{result}");
    Ok(Owner(
        result["session_id"]
            .as_str()
            .ok_or("session missing")?
            .into(),
    ))
}

fn query(owner: &Owner, input: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(owner.0.clone()),
        Some(input.to_string()),
    ))
}

fn temporal(owner: &Owner, input: Value) -> TestResult<Value> {
    query(
        owner,
        json!({"mode":"temporal","request":{"schema":"dfmcp.query/1","query":input}}),
    )
}

fn source(owner: &Owner) -> TestResult<WorldSnapshot> {
    Ok(resolve_session(Some(owner.0.clone()))?
        .lock()
        .map_err(|_| "session lock")?
        .adapter
        .snapshot()
        .clone())
}

fn labor_condition() -> Value {
    json!({"op":"field","entity_id":"1001","generation":1,"field":"labor.MINING",
        "comparison":"eq","value":{"type":"bool","value":true}})
}

fn ask(first: &WorldSnapshot) -> Value {
    json!({"mode":"condition_replay","from":first.state_hash.to_string(),
        "definition":{"condition":labor_condition(),"deadline_tick":1000,
            "poll_interval_ticks":1,"stable_observations":2},"detail":"evidence"})
}

fn set_labor(owner: &Owner, enabled: bool) -> TestResult {
    let plan = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.0.clone()),
        None,
        None,
        Some(
            json!([{"action":{"kind":"set_labor","units":["1001"],
            "labor":"MINING","enabled":enabled}}])
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.0.clone()),
        plan["plan_digest"].as_str().ok_or("plan digest")?.into(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(())
}

fn refuse(value: &Value, code: &str) {
    assert_eq!(value["ok"], false, "{value}");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert!(value.get("replay_id").is_none(), "{value}");
    assert!(value.get("status_changes").is_none(), "{value}");
}

#[test]
fn condition_replay_public_real_labor_history_keeps_current_work_and_terminal_evidence_separate()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    let first = source(&owner)?;
    set_labor(&owner, true)?;
    for _ in 0..2 {
        let waited = parsed(&crate::agent_facade::fortress_wait(
            Some(owner.0.clone()),
            Some(1),
        ))?;
        assert_eq!(waited["ok"], true, "{waited}");
    }
    set_labor(&owner, false)?;
    let current = source(&owner)?;
    let watched = temporal(
        &owner,
        json!({"kind":"watch","key":"unrelated-future",
        "condition":{"op":"tick_at_least","value":100},"deadline_tick":1000,
        "stable_observations":2}),
    )?;
    assert_eq!(watched["ok"], true, "{watched}");
    let watch_id = watched["record"]["watch"].as_str().ok_or("watch handle")?;
    let baseline = temporal(
        &owner,
        json!({"kind":"capture","key":"unrelated-units",
        "select":{"kind":"entities","kinds":["unit"],"fields":["alive"]},
        "max_game_ticks":1000}),
    )?;
    assert_eq!(baseline["ok"], true, "{baseline}");
    let baseline_id = baseline["captured"]["baseline"]
        .as_str()
        .ok_or("baseline handle")?;
    let pending = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.0.clone()),
        Some("keep a separate pending review".into()),
        Some(true),
        None,
        None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");
    let watches_before = temporal(&owner, json!({"kind":"watches"}))?;
    let baselines_before = temporal(&owner, json!({"kind":"baselines"}))?;
    let retained_before = {
        let handle = resolve_session(Some(owner.0.clone()))?;
        let guard = handle.lock().map_err(|_| "session lock")?;
        format!("{:?}", guard.history)
    };
    let replayed = query(&owner, ask(&first))?;
    assert_eq!(replayed["ok"], true, "{replayed}");
    assert_eq!(replayed["status"], "satisfied", "{replayed}");
    // The starter world begins at tick 1. Its first sample consumes that
    // cadence slot, so the same-tick labor change earns no positive sample.
    // The two explicit one-tick waits supply stable samples at ticks 2 and 3.
    assert_eq!(replayed["last_evaluated_anchor"]["game_tick"], 3);
    assert!(
        replayed["records_after_terminal"]
            .as_u64()
            .is_some_and(|n| n >= 1)
    );
    assert_eq!(replayed["condition_truth"], "true");
    assert_eq!(replayed["historical"], true);
    assert_eq!(replayed["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(
        replayed["agent_turn"]["anchor"],
        anchor_json(&current.anchor())
    );
    assert_eq!(replayed["current_anchor"], anchor_json(&current.anchor()));
    assert_eq!(replayed["watch_registered"], false);
    assert_eq!(replayed["advances_game"], false);
    assert_eq!(replayed["eligible_as_mutation_evidence"], false);
    let current_work = &replayed["agent_turn"]["active_work"];
    assert_eq!(current_work["query_work_coverage"]["status"], "available");
    assert_eq!(
        current_work["query_work_coverage"]["anchor"],
        anchor_json(&current.anchor())
    );
    assert_eq!(
        current_work["query_watches"].as_array().map(Vec::len),
        Some(1),
        "{replayed}"
    );
    assert_eq!(current_work["query_watches"][0]["watch"], watch_id);
    assert_eq!(current_work["condition_watches"][0]["watch"], watch_id);
    assert_eq!(
        current_work["query_baselines"].as_array().map(Vec::len),
        Some(1),
        "{replayed}"
    );
    assert_eq!(current_work["query_baselines"][0]["baseline"], baseline_id);
    assert_eq!(
        current_work["pending_plans"][0]["plan_digest"],
        pending["plan_digest"]
    );
    let now = query(
        &owner,
        json!({"mode":"structured","request":{"schema":"dfmcp.query/1",
        "query":{"kind":"condition_evaluation","condition":labor_condition()}}}),
    )?;
    assert_eq!(now["evaluation"]["condition_truth"], "false", "{now}");
    assert_eq!(
        temporal(&owner, json!({"kind":"watches"}))?["records"],
        watches_before["records"]
    );
    assert_eq!(
        temporal(&owner, json!({"kind":"baselines"}))?["baselines"],
        baselines_before["baselines"]
    );
    let handle = resolve_session(Some(owner.0.clone()))?;
    let guard = handle.lock().map_err(|_| "session lock")?;
    assert_eq!(guard.adapter.snapshot(), &current);
    assert_eq!(format!("{:?}", guard.history), retained_before);
    assert_eq!(
        guard
            .pending
            .as_ref()
            .ok_or("pending review")?
            .plan
            .digest
            .to_string(),
        pending["plan_digest"].as_str().ok_or("pending digest")?
    );
    Ok(())
}

#[test]
fn condition_replay_public_authority_source_and_output_refusals_preserve_owned_state() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open()?;
    let first = source(&owner)?;
    set_labor(&owner, true)?;
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.0.clone()),
        Some(1),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let current = source(&owner)?;
    let full = query(&owner, ask(&first))?;
    assert_eq!(full["ok"], true, "{full}");
    let handle = resolve_session(Some(owner.0.clone()))?;
    let original = handle.lock().map_err(|_| "session lock")?.budget.clone();
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        guard.budget.max_entities = u32::try_from(
            current.graph.entities.len()
                + current.graph.edges.len()
                + current.graph.chunks.len()
                + current.graph.events.len(),
        )?;
    }
    refuse(&query(&owner, ask(&first))?, "budget_exceeded");
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        guard.budget = original.clone();
        guard.budget.max_output_tokens = 128;
    }
    refuse(&query(&owner, ask(&first))?, "budget_exceeded");
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        guard.budget = original;
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
    }
    let mut missing = ask(&first);
    missing["from"] = json!("f".repeat(64));
    refuse(&query(&owner, missing)?, "capability_denied");
    assert_eq!(source(&owner)?, current);
    Ok(())
}

#[test]
fn condition_replay_public_malformed_requests_and_unrecorded_frontiers_do_not_record_history()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open()?;
    let first = source(&owner)?;
    let handle = resolve_session(Some(owner.0.clone()))?;
    let retained_before = {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let mut changed = guard.adapter.snapshot().clone();
        changed.tick = GameTick(1);
        changed.cursor = changed.cursor.checked_next().ok_or("cursor")?;
        changed.refresh_hash();
        guard.adapter.inject_snapshot(changed)?;
        format!("{:?}", guard.history)
    };
    let mut malformed = ask(&first);
    malformed["definition"]["unrecognized"] = json!(true);
    refuse(&query(&owner, malformed)?, "invalid_request");
    assert_eq!(
        format!("{:?}", handle.lock().map_err(|_| "session lock")?.history),
        retained_before
    );
    refuse(&query(&owner, ask(&first))?, "stale_anchor");
    assert_eq!(
        format!("{:?}", handle.lock().map_err(|_| "session lock")?.history),
        retained_before
    );
    let observed = parsed(&crate::agent_facade::fortress_observe(Some(
        owner.0.clone(),
    )))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let replayed = query(&owner, ask(&first))?;
    assert_eq!(replayed["ok"], true, "{replayed}");
    assert_eq!(replayed["status"], "blocked_unknown");
    assert_eq!(replayed["range"]["records"], 2);
    Ok(())
}
