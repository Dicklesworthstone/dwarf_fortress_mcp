//! Endpoint publication failures cannot become reusable original-goal proof.

use super::*;
use crate::server;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-objective-reserve-{label}-{}",
            std::process::id(),
        ));
        let _ = std::fs::remove_dir_all(&path);
        server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn payload(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str) -> TestResult<String> {
    let opened = payload(&server::open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        None,
        Some(16_384),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(true),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn reserve(session: &str) -> TestResult<Digest32> {
    let planned = payload(&server::plan_request(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(
            serde_json::json!({ "template":"production", "reserve_until_tick":1234,
                "quotas":[{"item":"DRINK", "minimum":40},{"item":"FOOD", "minimum":60}],
            })
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"].as_str().ok_or("digest missing")?;
    let committed = payload(&server::fortress_commit(
        Some(session.to_owned()),
        digest.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Digest32::from_hex(digest).ok_or_else(|| "invalid digest".into())
}

fn anchor(session: &str) -> TestResult<StateAnchor> {
    let handle = server::lookup_session_str(session)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    Ok(guard.adapter.snapshot().anchor())
}

#[test]
fn failed_endpoint_publication_installs_no_cache_and_retries_only_the_exact_frontier() -> TestResult
{
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("write-failure");
    let session = open("7855901")?;
    let digest = reserve(&session)?;
    let elapsed = 1234 - anchor(&session)?.tick.0;
    let advanced = payload(&server::wait_with_ticks(
        Some(session.clone()),
        Some(elapsed),
    ))?;
    assert_eq!(advanced["ok"], true, "{advanced}");
    let endpoint = anchor(&session)?;
    with_durable_store(|store| {
        assert_eq!(
            store.objective_reserve_observation(endpoint.fortress_id, digest),
            None
        );
        store.set_append_budget(Some(0));
        Ok(())
    })?;
    let refused = payload(&server::wait_for_plan(
        Some(session.clone()),
        Some(0),
        digest.to_hex(),
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "adapter_unavailable", "{refused}");
    assert_eq!(anchor(&session)?, endpoint);
    {
        let handle = server::lookup_session_str(&session)?;
        let guard = handle.lock().map_err(|_| "session poisoned")?;
        let original = guard
            .objectives
            .iter()
            .find(|original| original.plan_digest == digest)
            .ok_or("original missing")?;
        assert!(original.selected_wait_reserve_observation.is_none());
        assert!(!original.selected_wait_reserve_durable);
    }
    with_durable_store(|store| {
        assert_eq!(
            store.objective_reserve_observation(endpoint.fortress_id, digest),
            None
        );
        store.set_append_budget(None);
        Ok(())
    })?;
    let retried = payload(&server::wait_for_plan(
        Some(session.clone()),
        Some(0),
        digest.to_hex(),
    ))?;
    assert_eq!(retried["ok"], true, "{retried}");
    assert_eq!(
        retried["selected_plan_wait"]["reserve_frontier"]["durable"],
        true
    );
    assert_eq!(
        retried["selected_plan_wait"]["reserve_frontier"]["observed_anchor"],
        anchor_json(&endpoint)
    );
    assert_eq!(anchor(&session)?, endpoint);
    with_durable_store(|store| {
        assert_eq!(
            store.objective_reserve_observation(endpoint.fortress_id, digest),
            Some(endpoint)
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn a_well_formed_durable_snapshot_at_the_wrong_horizon_cannot_become_original_proof() -> TestResult
{
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("wrong-horizon");
    let first = open("7855902")?;
    let digest = reserve(&first)?;
    let snapshot = {
        let handle = server::lookup_session_str(&first)?;
        let guard = handle.lock().map_err(|_| "session poisoned")?;
        guard.adapter.snapshot().clone()
    };
    assert!(snapshot.tick.0 < 1234);
    // Storage admits immutable history; source/horizon semantics belong to the
    // original-goal verifier and cannot be supplied by this record alone.
    with_durable_store(|store| store.persist_objective_reserve_observation(digest, &snapshot))?;
    server::simulate_durable_restart(Some(directory.0.clone()));
    let second = open("7855902")?;
    let before = anchor(&second)?;
    let refused = payload(&server::wait_for_plan(
        Some(second.clone()),
        Some(100),
        digest.to_hex(),
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(anchor(&second)?, before);
    let handle = server::lookup_session_str(&second)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    let original = guard
        .objectives
        .iter()
        .find(|original| original.plan_digest == digest)
        .ok_or("original missing")?;
    assert!(original.verification_error.is_some());
    assert!(original.selected_wait_reserve_observation.is_none());
    Ok(())
}
