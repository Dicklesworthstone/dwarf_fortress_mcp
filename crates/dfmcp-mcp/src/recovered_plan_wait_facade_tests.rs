//! Public recovery waits preserve original proof without restoring dispatch.
use super::*;
use dfmcp_core::Capability;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-recovered-plan-wait-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        crate::server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
    fn restart(&self) {
        crate::server::simulate_durable_restart(Some(self.0.clone()));
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        crate::server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn payload(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}
fn field(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}
fn open(selector: &str, effect_authority: bool) -> TestResult<Value> {
    let mut grants = vec![
        ("observe", "read_only"),
        ("query", "read_only"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("restore", "guarded"),
    ];
    if effect_authority {
        grants.extend([("plan", "guarded"), ("designate", "guarded")]);
    }
    let opened = payload(&fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            grants
                .into_iter()
                .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
                .collect(),
        ),
        Some(30_000),
        Some(2_000),
        None,
        None,
        Some(16_384),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(true),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened)
}
fn commit(session: &str, actions: Value) -> TestResult<String> {
    let planned = payload(&fortress_plan(
        Some(session.to_owned()),
        Some("retain original excavation through restart".to_owned()),
        None,
        Some(actions.to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = field(&planned, "plan_digest")?;
    let committed = payload(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let progress = payload(&fortress_wait(Some(session.to_owned()), Some(20)))?;
    assert_eq!(progress["ok"], true, "{progress}");
    Ok(digest)
}
fn selected_wait(session: &str, digest: &str, ticks: u64) -> TestResult<Value> {
    payload(&fortress_wait_for_plan(
        Some(session.to_owned()),
        Some(ticks),
        Some(digest.to_owned()),
    ))
}
fn anchor(session: &str) -> TestResult<dfmcp_core::StateAnchor> {
    let handle = crate::server::lookup_session_str(session)?;
    let guard = handle.lock().map_err(|_| "test session lock poisoned")?;
    Ok(guard.adapter.snapshot().anchor())
}

#[test]
fn recovered_mixed_plan_finishes_without_recreating_dispatch_authority() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("mixed");
    let first = field(&open("7855701", true)?, "session_id")?;
    let digest = commit(
        &first,
        json!([
            {"action":{"kind":"pause","paused":false}},
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"},
             "depends_on":[0]}
        ]),
    )?;
    let prior = anchor(&first)?;
    directory.restart();
    // No Plan or Designate grant: only existing physical work can progress.
    let reopened = open("7855701", false)?;
    assert_eq!(reopened["durable"]["resumed"], true);
    let second = field(&reopened, "session_id")?;
    assert!(anchor(&second)?.cursor.epoch > prior.cursor.epoch);
    let waited = selected_wait(&second, &digest, 2_000)?;
    assert_eq!(waited["ok"], true, "{waited}");
    let selected = &waited["selected_plan_wait"];
    assert_eq!(selected["selection_scope"], "exact_recovered_original");
    assert_eq!(selected["selection_origin"], "durable_recovery");
    assert_eq!(selected["dispatch_handles_restored"], false);
    assert_eq!(selected["owns_original_dispatch"], false);
    assert_eq!(selected["plan_digest"], digest);
    assert_eq!(selected["stop_reason"], "complete", "{waited}");
    assert_eq!(selected["actions_verified"], true);
    assert_eq!(selected["action_count"], 2);
    assert_eq!(selected["verified_actions"], 2);
    assert_eq!(selected["physical_quiescent"], true);
    assert_eq!(selected["goal_truth"], "true");
    assert_eq!(waited["wait_progress"]["deferred_actions_dispatched"], 0);
    assert_eq!(waited["polled_actions"], json!([]));
    let advanced = waited["advanced_game_ticks"]
        .as_u64()
        .ok_or("elapsed ticks missing")?;
    assert!(advanced > 0 && advanced < 2_000, "{waited}");
    assert_eq!(waited["agent_turn"]["anchor"], waited["observed_anchor"]);

    // A completed recovered monitor remains inspectable without a new commit.
    let finished = anchor(&second)?;
    let repeated = selected_wait(&second, &digest, 0)?;
    assert_eq!(repeated["ok"], true, "{repeated}");
    assert_eq!(repeated["selected_plan_wait"]["stop_reason"], "complete");
    assert_eq!(repeated["advanced_game_ticks"], 0);
    assert_eq!(anchor(&second)?, finished);
    assert_eq!(payload(&fortress_observe(Some(first)))?["ok"], false);
    Ok(())
}

#[test]
fn recovered_wait_does_not_dispatch_an_original_dependent_step() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("deferred");
    let first = field(&open("7855702", true)?, "session_id")?;
    let digest = commit(
        &first,
        json!([
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}},
            {"action":{"kind":"designate_dig","min":[0,6,10],"max":[1,6,10],"mode":"mine"},
             "depends_on":[0]}
        ]),
    )?;
    directory.restart();
    let second = field(&open("7855702", false)?, "session_id")?;
    let waited = selected_wait(&second, &digest, 2_000)?;
    assert_eq!(waited["ok"], true, "{waited}");
    let selected = &waited["selected_plan_wait"];
    assert_eq!(selected["complete"], false);
    assert_eq!(selected["stop_reason"], "goal_unmet", "{waited}");
    assert_eq!(selected["goal_truth"], "false");
    assert_eq!(selected["verified_actions"], 1);
    assert_eq!(selected["not_dispatched_actions"], 1);
    assert_eq!(selected["physical_quiescent"], true);
    assert_eq!(waited["wait_progress"]["deferred_actions_dispatched"], 0);
    assert_eq!(waited["polled_actions"], json!([]));
    let rows = payload(&fortress_query(
        Some(second),
        Some(r#"{"mode":"terrain","min":[0,6,10],"max":[1,6,10]}"#.to_owned()),
    ))?;
    assert_eq!(rows["ok"], true, "{rows}");
    assert_eq!(rows["levels"][0]["rows"], json!(["##"]));
    Ok(())
}

#[test]
fn recovered_selection_requires_current_grants_and_refuses_restored_history() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("authority");
    let first = field(&open("7855703", true)?, "session_id")?;
    let checkpoint = payload(&fortress_checkpoint(
        Some(first.clone()),
        Some("before dig".into()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let checkpoint_id = field(&checkpoint, "checkpoint_id")?;
    let digest = commit(
        &first,
        json!([
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}
        ]),
    )?;
    directory.restart();
    let second = field(&open("7855703", false)?, "session_id")?;
    let handle = crate::server::lookup_session_str(&second)?;
    for capability in [
        Capability::Observe,
        Capability::Query,
        Capability::ControlClock,
    ] {
        let before = anchor(&second)?;
        let original = {
            let mut guard = handle.lock().map_err(|_| "test session lock poisoned")?;
            let original = guard.grants.clone();
            guard.grants.retain(|grant| grant.capability != capability);
            original
        };
        let refused = selected_wait(&second, &digest, 100)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "capability_denied", "{refused}");
        assert_eq!(anchor(&second)?, before);
        handle
            .lock()
            .map_err(|_| "test session lock poisoned")?
            .grants = original;
    }
    let before = anchor(&second)?;
    assert_eq!(selected_wait(&second, &"ab".repeat(32), 100)?["ok"], false);
    assert_eq!(anchor(&second)?, before);
    let observed = selected_wait(&second, &digest, 0)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(observed["advanced_game_ticks"], 0);
    let restored = payload(&fortress_restore(Some(second.clone()), checkpoint_id))?;
    assert_eq!(restored["ok"], true, "{restored}");
    let restored_anchor = anchor(&second)?;
    let refused = selected_wait(&second, &digest, 100)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(anchor(&second)?, restored_anchor);
    Ok(())
}
