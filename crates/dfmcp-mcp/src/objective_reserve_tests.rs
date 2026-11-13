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
    open_with_durability(selector, true)
}

fn open_with_durability(selector: &str, durable: bool) -> TestResult<String> {
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
        Some(durable),
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

fn with_original<T>(
    session: &str,
    digest: Digest32,
    body: impl FnOnce(&mut LabSession, &Objective) -> Result<T>,
) -> TestResult<T> {
    let handle = server::lookup_session_str(session)?;
    let mut guard = handle.lock().map_err(|_| "session poisoned")?;
    let original = guard
        .objectives
        .iter()
        .find(|original| original.plan_digest == digest)
        .cloned()
        .ok_or("original missing")?;
    Ok(body(&mut guard, &original)?)
}

fn wait_ticks(session: &str, ticks: u64) -> TestResult {
    let advanced = payload(&server::wait_with_ticks(
        Some(session.to_owned()),
        Some(ticks),
    ))?;
    assert_eq!(advanced["ok"], true, "{advanced}");
    Ok(())
}

fn capacity_refuses_pause(session: &str) -> TestResult {
    let planned = payload(&server::plan_request(
        Some(session.to_owned()),
        None,
        Some(true),
        None,
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"]
        .as_str()
        .ok_or("pause digest missing")?;
    let before = anchor(session)?;
    let refused = payload(&server::fortress_commit(
        Some(session.to_owned()),
        digest.to_owned(),
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert_eq!(anchor(session)?, before);
    let handle = server::lookup_session_str(session)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    assert_eq!(guard.objectives.len(), MAX_OBJECTIVES_PER_FORTRESS);
    assert_eq!(
        guard
            .pending
            .as_ref()
            .map(|pending| pending.digest.as_str()),
        Some(digest)
    );
    Ok(())
}

#[test]
fn funded_and_missed_reserves_cannot_release_goal_capacity_before_a_new_effect() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("reserve-capacity");
    let session = open("7855903")?;
    let mut originals = Vec::new();
    for index in 0..MAX_OBJECTIVES_PER_FORTRESS {
        originals.push(reserve(&session)?);
        if index + 1 < MAX_OBJECTIVES_PER_FORTRESS {
            // Retain each original while avoiding duplicate active producers.
            // The final independently committed order supplies the shared stock.
            let stopped = payload(&server::cancel_in_scope(
                Some(session.clone()),
                Some("stop_future_steps".to_owned()),
                Some("plan".to_owned()),
            ))?;
            assert_eq!(stopped["ok"], true, "{stopped}");
            assert_eq!(stopped["drain_progress"]["quiescent"], true, "{stopped}");
        }
    }
    wait_ticks(&session, 1233 - anchor(&session)?.tick.0)?;
    for digest in &originals {
        with_original(&session, *digest, |guard, original| {
            assert!(original.historical_proof_verified);
            assert_eq!(original.current_truth(guard)?, PredicateTruth::True);
            assert!(goal_work_quiescent(guard, original)?);
            let source = original
                .future_reserve_source()?
                .ok_or_else(|| conflict("fixture reserve source missing"))?;
            assert_eq!(
                original.future_reserve_status(guard, &source)?,
                ProductionReserveStatus::FundedFromCurrentStock
            );
            assert!(original.selected_wait_reserve_observation.is_none());
            Ok(())
        })?;
    }
    capacity_refuses_pause(&session)?;

    // Ordinary progression does not capture the exact reserve endpoint.
    wait_ticks(&session, 2)?;
    for digest in &originals {
        with_original(&session, *digest, |guard, original| {
            assert_eq!(original.current_truth(guard)?, PredicateTruth::True);
            assert!(goal_work_quiescent(guard, original)?);
            assert!(original.selected_wait_reserve_observation.is_none());
            let source = original
                .future_reserve_source()?
                .ok_or_else(|| conflict("fixture reserve source missing"))?;
            assert_eq!(
                original.future_reserve_status(guard, &source)?,
                ProductionReserveStatus::Expired
            );
            Ok(())
        })?;
    }
    capacity_refuses_pause(&session)?;
    Ok(())
}

#[test]
fn exact_true_reserve_history_is_eligible_locally_and_after_verified_recovery() -> TestResult {
    let _serial = server::serialized_durable_tests();
    for (durable, selector) in [(false, "7855904"), (true, "7855905")] {
        let directory = StateDir::new(if durable {
            "eligible-durable"
        } else {
            "eligible-local"
        });
        let first = open_with_durability(selector, durable)?;
        let digest = reserve(&first)?;
        let observed = payload(&server::wait_for_plan(
            Some(first.clone()),
            Some(2000),
            digest.to_hex(),
        ))?;
        assert_eq!(observed["ok"], true, "{observed}");
        assert_eq!(
            observed["selected_plan_wait"]["complete"], true,
            "{observed}"
        );
        assert_eq!(observed["game_tick"], 1234);
        assert_eq!(
            observed["selected_plan_wait"]["reserve_frontier"]["durable"],
            durable
        );
        wait_ticks(&first, 1)?;
        let session = if durable {
            server::simulate_durable_restart(Some(directory.0.clone()));
            open(selector)?
        } else {
            first
        };
        with_original(&session, digest, |guard, original| {
            assert_eq!(original.current_truth(guard)?, PredicateTruth::True);
            assert!(goal_work_quiescent(guard, original)?);
            assert!(eviction_is_complete(original, guard)?);
            assert_eq!(original.selected_wait_reserve_durable, durable);
            let endpoint = original
                .selected_wait_reserve_observation
                .ok_or_else(|| conflict("fixture endpoint missing"))?
                .0;
            assert_eq!(endpoint.tick.0, 1234);
            assert_eq!(
                endpoint.cursor.epoch != guard.adapter.snapshot().cursor.epoch,
                durable
            );

            let mut missing = original.clone();
            missing.selected_wait_reserve_observation = None;
            assert!(!eviction_is_complete(&missing, guard)?);
            for truth in [PredicateTruth::False, PredicateTruth::Unknown] {
                let mut unfinished = original.clone();
                unfinished.selected_wait_reserve_observation = Some((endpoint, truth));
                assert!(!eviction_is_complete(&unfinished, guard)?);
            }
            let mut wrong_horizon = original.clone();
            let mut wrong_anchor = endpoint;
            wrong_anchor.tick.0 += 1;
            wrong_horizon.selected_wait_reserve_observation =
                Some((wrong_anchor, PredicateTruth::True));
            assert!(eviction_is_complete(&wrong_horizon, guard).is_err());
            let mut wrong_lineage = original.clone();
            let mut active = wrong_lineage
                .active_observation_anchor
                .ok_or_else(|| conflict("fixture active lineage missing"))?;
            active.cursor.epoch += 1;
            wrong_lineage.active_observation_anchor = Some(active);
            assert!(eviction_is_complete(&wrong_lineage, guard).is_err());
            if durable {
                let mut unverified_restart = original.clone();
                unverified_restart.selected_wait_reserve_durable = false;
                assert!(eviction_is_complete(&unverified_restart, guard).is_err());
            }
            let grants = guard.grants.clone();
            guard
                .grants
                .retain(|grant| grant.capability != Capability::Query);
            assert!(eviction_is_complete(original, guard).is_err());
            guard.grants = grants;
            let budget = guard.budget;
            guard.budget.max_bytes = 1;
            assert!(eviction_is_complete(original, guard).is_err());
            guard.budget = budget;
            assert!(eviction_is_complete(original, guard)?);
            Ok(())
        })?;
    }
    Ok(())
}
