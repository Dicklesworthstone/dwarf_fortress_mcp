//! Foreground waits cross dependency, publication, goal-history and Task views.
use super::*;
use crate::agent_facade as facade;
use dfmcp_intent::effects;
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-foreground-wait-{label}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str, durable: bool) -> TestResult<String> {
    let result = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("checkpoint", "guarded"),
                ("designate", "guarded"),
                ("construct", "guarded"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(c, r)| (c.to_owned(), r.to_owned()))
            .collect(),
        ),
        Some(30_000),
        Some(2_000),
        None,
        None,
        Some(8_192),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(durable),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn submit(session: &str, actions: Option<Value>, blueprint: Option<Value>) -> TestResult<Value> {
    let planned = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        actions.map(|value| value.to_string()),
        blueprint.map(|value| value.to_string()),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        planned["plan_digest"]
            .as_str()
            .ok_or("plan digest missing")?
            .to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(committed)
}

fn chain() -> Value {
    json!([
        {"action": {"kind":"designate_dig", "mode":"mine", "min":[0,3,10], "max":[1,3,10]}},
        {"action": {"kind":"build", "building":"workshop:Still", "location":[0,3,10], "min":[0,3,10], "max":[0,3,10]}, "depends_on":[0]},
        {"action": {"kind":"create_work_order", "name":"foreground brewing", "job_token":"BREW_DRINK", "amount":2}, "depends_on":[1]},
    ])
}

#[test]
fn one_wait_runs_dig_build_and_production_and_projects_the_final_task() -> TestResult {
    let session = open("7855101", false)?;
    submit(&session, Some(chain()), None)?;
    let waited = parsed(&facade::fortress_wait(Some(session.clone()), Some(2_000)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["advanced_game_ticks"], 2_000);
    assert_eq!(waited["open_actions_remaining"], 0);
    assert_eq!(waited["status"], "completed", "{waited}");
    let rows = waited["polled_actions"]
        .as_array()
        .ok_or("action results missing")?;
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row["observed_anchor"].is_object()));
    assert!(
        rows.iter().any(|row| {
            row["observed_anchor"]["game_tick"].as_u64()
                < waited["observed_anchor"]["game_tick"].as_u64()
        }),
        "intermediate proof anchors must survive the later final frontier: {waited}"
    );
    assert!(
        rows.iter().all(|row| row["state"] == "Verified"),
        "{waited}"
    );
    assert!(
        waited["wait_progress"]["observation_rounds"]
            .as_u64()
            .ok_or("rounds missing")?
            > 1
    );
    let handle = resolve_session(Some(session))?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    let action = guard.last_action.ok_or("action missing")?;
    let receipt = guard
        .adapter
        .action_receipt(action)
        .ok_or("receipt missing")?;
    assert_eq!(receipt.state, CommitState::Verified);
    let stock = guard
        .adapter
        .snapshot()
        .graph
        .entities
        .get(&crate::lab_world::starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get(effects::STOCK_DRINK_FIELD))
        .and_then(|fact| fact.known_value());
    assert!(
        matches!(stock, Some(dfmcp_world::Value::U64(held)) if *held > 40),
        "the dependent brewing order must receive service during the same wait"
    );
    Ok(())
}

#[test]
fn a_deferred_pause_stops_time_inside_the_wait() -> TestResult {
    let session = open("7855102", false)?;
    submit(
        &session,
        Some(json!([
            {"action": {"kind":"designate_dig", "mode":"mine", "min":[0,3,10], "max":[0,3,10]}},
            {"action": {"kind":"pause", "paused":true}, "depends_on":[0]},
        ])),
        None,
    )?;
    let waited = parsed(&facade::fortress_wait(Some(session), Some(2_000)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(waited["paused"], true);
    let advanced = waited["advanced_game_ticks"]
        .as_u64()
        .ok_or("advanced ticks missing")?;
    assert!(advanced > 0 && advanced < 2_000, "{waited}");
    assert_eq!(
        waited["wait_progress"]["remaining_game_ticks"],
        2_000 - advanced
    );
    assert!(waited["blocked"].is_string());
    assert_eq!(waited["open_actions_remaining"], 0);
    Ok(())
}

#[test]
fn wait_retains_achievement_before_later_consumption_and_durable_reopen() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("achievement");
    let session = open("7855103", true)?;
    let committed = submit(
        &session,
        None,
        Some(json!({
            "template":"production", "quotas":[{"item":"DRINK", "minimum":50}],
        })),
    )?;
    let digest = committed["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let waited = parsed(&facade::fortress_wait(Some(session.clone()), Some(2_000)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let (_, current) = {
        let handle = resolve_session(Some(session))?;
        let guard = handle.lock().map_err(|_| "session poisoned")?;
        original_goal_observation(&guard, &digest)?
    };
    assert_eq!(current["predicate_truth"], "false", "{current}");
    assert_eq!(current["status"], "no_longer_holds", "{current}");
    let achieved = current["achieved_tick"]
        .as_u64()
        .ok_or("intermediate achievement was lost")?;
    assert!(
        achieved < 1_201,
        "achievement must precede consumption: {current}"
    );
    simulate_durable_restart(Some(directory.0.clone()));
    let resumed = open("7855103", true)?;
    let handle = resolve_session(Some(resumed))?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    let (_, recovered) = original_goal_observation(&guard, &digest)?;
    assert_eq!(recovered["achieved_tick"], current["achieved_tick"]);
    assert_eq!(recovered["predicate_truth"], "false");
    assert_eq!(recovered["status"], "no_longer_holds");
    Ok(())
}

#[test]
fn durable_publication_failure_fences_remaining_wait_time_and_dispatch() -> TestResult {
    let _serial = serialized_durable_tests();
    let _directory = StateDir::new("publish-failure");
    let session = open("7855104", true)?;
    submit(&session, Some(chain()), None)?;
    inject_durable_crash_after(0);
    let waited = parsed(&facade::fortress_wait(Some(session.clone()), Some(2_000)))?;
    assert_eq!(waited["ok"], false, "{waited}");
    assert_eq!(waited["error"]["code"], "adapter_unavailable", "{waited}");
    let advanced = waited["advanced_game_ticks"]
        .as_u64()
        .ok_or("partial progress missing")?;
    assert!(advanced < 2_000, "{waited}");
    assert_eq!(waited["partial_progress"], true);
    assert_eq!(waited["durability_pending"], true);
    assert_eq!(waited["agent_turn"]["anchor"], waited["observed_anchor"]);
    assert_eq!(waited["agent_turn"]["profile"], "pulse");
    assert!(
        waited["polled_actions"]
            .as_array()
            .is_some_and(|rows| rows.len() == 3)
    );
    let (anchor, receipts) = {
        let handle = resolve_session(Some(session.clone()))?;
        let guard = handle.lock().map_err(|_| "session poisoned")?;
        assert!(guard.durability_fault.is_some());
        assert_eq!(waited["paused"], guard.adapter.snapshot().paused);
        (
            guard.adapter.snapshot().anchor(),
            guard
                .last_plan_actions
                .iter()
                .map(|id| guard.adapter.action_receipt(*id).cloned())
                .collect::<Vec<_>>(),
        )
    };
    assert_eq!(waited["observed_anchor"], anchor_json(&anchor));
    let retry = parsed(&wait_with_ticks(Some(session.clone()), Some(2_000)))?;
    assert_eq!(retry["ok"], false, "{retry}");
    let handle = resolve_session(Some(session))?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    assert_eq!(guard.adapter.snapshot().anchor(), anchor);
    assert_eq!(
        guard
            .last_plan_actions
            .iter()
            .map(|id| guard.adapter.action_receipt(*id).cloned())
            .collect::<Vec<_>>(),
        receipts
    );
    Ok(())
}
