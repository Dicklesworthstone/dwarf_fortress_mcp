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

    // Completed action evidence must survive another process, including
    // compaction. Exact retired receipts remain read-only; no action handles
    // or old session ownership return with the historical original goal.
    let object_digest = selected["plan_object_digest"].clone();
    let owner = selected["original_owner_session_id"].clone();
    let mut prior = finished;
    for _ in 0..2 {
        directory.restart();
        {
            let mut store = dfmcp_lab::durable::DurableLabStore::open(&directory.0)?;
            let plan = dfmcp_core::Digest32::from_hex(&digest).ok_or("invalid plan digest")?;
            assert!(store.commit(prior.fortress_id, plan).is_none());
            let retired = store
                .retired_commit(prior.fortress_id, plan)
                .ok_or("retired original receipt map missing")?;
            assert_eq!(retired.steps.len(), 2);
            assert!(retired.steps.values().all(|state| state == "verified"));
            store.compact()?;
        }
        let next = field(&open("7855701", false)?, "session_id")?;
        let before = anchor(&next)?;
        assert!(before.cursor.epoch > prior.cursor.epoch);
        let archived = selected_wait(&next, &digest, 0)?;
        assert_eq!(archived["ok"], true, "{archived}");
        let observed = &archived["selected_plan_wait"];
        assert_eq!(observed["stop_reason"], "complete", "{archived}");
        assert_eq!(observed["plan_object_digest"], object_digest);
        assert_eq!(observed["original_owner_session_id"], owner);
        assert_eq!(observed["actions_verified"], true);
        assert_eq!(observed["action_count"], 2);
        assert_eq!(observed["physical_quiescent"], true);
        assert_eq!(observed["owns_original_dispatch"], false);
        assert_eq!(observed["dispatch_handles_restored"], false);
        assert_eq!(
            observed["recovery_evidence"]["action_record_origin"],
            "archived_retired_commit",
        );
        assert_eq!(archived["advanced_game_ticks"], 0);
        assert_eq!(anchor(&next)?, before);
        prior = before;
    }
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

fn current_world(session: &str) -> TestResult<dfmcp_world::WorldSnapshot> {
    let handle = crate::server::lookup_session_str(session)?;
    let guard = handle.lock().map_err(|_| "test session lock poisoned")?;
    Ok(guard.adapter.snapshot().clone())
}

fn cancel_session(session: &str, mode: &str) -> TestResult<Value> {
    payload(&fortress_cancel(
        Some(session.to_owned()),
        Some(mode.to_owned()),
        Some("session".to_owned()),
    ))
}

#[test]
fn recovered_emergency_pause_is_durable_without_claiming_carried_work_drained() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("emergency-pause");
    let first = field(&open("7855704", true)?, "session_id")?;
    let digest = commit(
        &first,
        json!([
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}
        ]),
    )?;
    directory.restart();
    // Recovery restores physical work, but no action handle or Designate grant.
    let second = field(&open("7855704", false)?, "session_id")?;
    let before = current_world(&second)?;
    assert!(!before.paused);
    let ordinary = cancel_session(&second, "stop_future_steps")?;
    assert_eq!(ordinary["ok"], true, "{ordinary}");
    assert_eq!(ordinary["drain_progress"]["quiescent"], false);
    assert!(ordinary["finalize_certificate"].is_null());
    assert_eq!(current_world(&second)?, before);

    let paused = cancel_session(&second, "emergency_pause_and_drain")?;
    assert_eq!(paused["ok"], true, "{paused}");
    assert_eq!(paused["paused"], true);
    assert_eq!(paused["emergency_pause"]["changed"], true);
    assert_eq!(paused["emergency_pause"]["durable"], true);
    assert_eq!(paused["emergency_pause"]["publication_confirmed"], true);
    assert_eq!(paused["emergency_pause"]["action_handles_created"], false);
    assert_eq!(paused["emergency_pause"]["physical_work_drained"], false);
    assert_eq!(paused["drain_progress"]["actions_total"], 0);
    assert_eq!(paused["drain_progress"]["remaining_carried"], 1);
    assert_eq!(paused["drain_progress"]["quiescent"], false);
    assert!(paused["finalize_certificate"].is_null());
    let after = current_world(&second)?;
    assert!(after.paused);
    assert_eq!(after.tick, before.tick);
    assert_eq!(after.graph.entities, before.graph.entities);
    assert_eq!(after.graph.chunks, before.graph.chunks);
    let observed = selected_wait(&second, &digest, 100)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(observed["advanced_game_ticks"], 0);
    assert_eq!(observed["selected_plan_wait"]["physical_quiescent"], false);
    assert_eq!(
        observed["selected_plan_wait"]["dispatch_handles_restored"],
        false
    );
    assert_eq!(current_world(&second)?, after);

    // Pause is part of the durable world frontier, including after compaction.
    let mut prior = after.anchor();
    for _ in 0..2 {
        directory.restart();
        dfmcp_lab::durable::DurableLabStore::open(&directory.0)?.compact()?;
        let next = field(&open("7855704", false)?, "session_id")?;
        let world = current_world(&next)?;
        assert!(world.paused);
        assert_eq!(world.tick, before.tick);
        assert!(world.cursor.epoch > prior.cursor.epoch);
        assert_eq!(world.graph.entities, before.graph.entities);
        let remaining = selected_wait(&next, &digest, 0)?;
        assert_eq!(remaining["ok"], true, "{remaining}");
        assert_eq!(remaining["selected_plan_wait"]["physical_quiescent"], false);
        assert_eq!(
            remaining["selected_plan_wait"]["dispatch_handles_restored"],
            false
        );
        assert_eq!(remaining["advanced_game_ticks"], 0);
        prior = world.anchor();
    }
    Ok(())
}

#[test]
fn emergency_pause_publication_failure_stops_before_any_physical_drain() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("emergency-publication-failure");
    let first = field(&open("7855705", true)?, "session_id")?;
    let digest = commit(
        &first,
        json!([
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}
        ]),
    )?;
    let before = current_world(&first)?;
    assert!(!before.paused);
    crate::server::inject_durable_crash_after(0);
    let refused = cancel_session(&first, "emergency_pause_and_drain")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "adapter_unavailable");
    assert_eq!(refused["paused"], true);
    assert_eq!(refused["emergency_pause"]["changed"], true);
    assert_eq!(refused["emergency_pause"]["publication_confirmed"], false);
    assert_eq!(refused["drain_progress"]["drained"], 0);
    assert_eq!(refused["drain_progress"]["quiescent"], false);
    assert!(refused["finalize_certificate"].is_null());
    let memory = current_world(&first)?;
    assert!(memory.paused);
    assert_eq!(memory.tick, before.tick);
    assert_eq!(memory.graph.entities, before.graph.entities);
    assert_eq!(memory.graph.chunks, before.graph.chunks);

    // The refusal did not claim this in-memory pause survived a failed append.
    directory.restart();
    let next = field(&open("7855705", false)?, "session_id")?;
    let recovered = current_world(&next)?;
    assert!(!recovered.paused);
    assert_eq!(recovered.tick, before.tick);
    assert_eq!(recovered.graph.entities, before.graph.entities);
    let remaining = selected_wait(&next, &digest, 0)?;
    assert_eq!(remaining["ok"], true, "{remaining}");
    assert_eq!(remaining["selected_plan_wait"]["physical_quiescent"], false);
    assert_eq!(
        remaining["selected_plan_wait"]["dispatch_handles_restored"],
        false
    );
    Ok(())
}
