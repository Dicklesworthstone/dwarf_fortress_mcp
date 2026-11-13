//! Exact selected reserve evidence survives recovery without borrowing later stock.

use super::*;
use dfmcp_core::{DfmcpError, Digest32, ErrorCode, GameTick, StateAnchor};
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue};
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const HORIZON: u64 = 1_234;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-durable-selected-reserve-{label}-{}",
            std::process::id(),
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

fn open(selector: &str, production_authority: bool) -> TestResult<String> {
    let mut grants = vec![
        ("observe", "read_only"),
        ("query", "read_only"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("restore", "guarded"),
    ];
    if production_authority {
        grants.extend([
            ("plan", "reversible"),
            ("configure_production", "reversible"),
        ]);
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
    field(&opened, "session_id")
}

fn reserve(session: &str) -> TestResult<String> {
    let planned = payload(&fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(
            json!({
                "template":"production", "reserve_until_tick":HORIZON,
                "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
            })
            .to_string(),
        ),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = field(&planned, "plan_digest")?;
    let committed = payload(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(digest)
}

fn selected(session: &str, digest: &str, ticks: u64) -> TestResult<Value> {
    let waited = payload(&fortress_wait_for_plan(
        Some(session.to_owned()),
        Some(ticks),
        Some(digest.to_owned()),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    Ok(waited)
}

fn anchor(session: &str) -> TestResult<StateAnchor> {
    let handle = crate::server::lookup_session_str(session)?;
    let guard = handle.lock().map_err(|_| "test session lock poisoned")?;
    Ok(guard.adapter.snapshot().anchor())
}

fn wait(session: &str, ticks: u64) -> TestResult<Value> {
    let waited = payload(&fortress_wait(Some(session.to_owned()), Some(ticks)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    Ok(waited)
}

fn fixture_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

/// Change only the observed stock fixture, keeping the original ledger identity,
/// original effects and all sealed source bytes. The actual owner publishes the
/// resulting complete snapshot through its normal durable progress path.
fn stock_at(session: &str, drink: Option<u64>, tick: u64) -> TestResult {
    let handle = crate::server::lookup_session_str(session)?;
    crate::server::with_session(
        &handle,
        || Err(fixture_error("reserve fixture session unavailable")),
        |guard| {
            let mut snapshot = guard.adapter.snapshot().clone();
            assert!(tick >= snapshot.tick.0);
            let ledger = effects::stock_ledger(&snapshot)
                .ok_or_else(|| fixture_error("stock ledger missing"))?;
            let stock = snapshot
                .graph
                .entities
                .get_mut(&ledger)
                .ok_or_else(|| fixture_error("stock entity missing"))?;
            if let Some(amount) = drink {
                stock.fields.insert(
                    effects::STOCK_DRINK_FIELD.to_owned(),
                    Fact::known(
                        WorldValue::U64(amount),
                        GameTick(tick),
                        FactSource::Replay,
                        Digest32::of_bytes(b"selected-reserve-test-stock"),
                    ),
                );
            } else {
                stock.fields.remove(effects::STOCK_DRINK_FIELD);
            }
            stock.fields.insert(
                effects::STOCK_FOOD_FIELD.to_owned(),
                Fact::known(
                    WorldValue::U64(1_000),
                    GameTick(tick),
                    FactSource::Replay,
                    Digest32::of_bytes(b"selected-reserve-test-stock"),
                ),
            );
            stock.revision += 1;
            snapshot.tick = GameTick(tick);
            snapshot.cursor = snapshot
                .cursor
                .checked_next()
                .ok_or_else(|| fixture_error("fixture cursor exhausted"))?;
            snapshot.refresh_hash();
            guard.adapter.inject_snapshot(snapshot)
        },
    )?;
    Ok(())
}

#[test]
fn exact_reserve_survives_retired_actions_restart_and_refuses_restore() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("exact");
    let first = open("7855801", true)?;
    let checkpoint = payload(&fortress_checkpoint(
        Some(first.clone()),
        Some("before reserve".into()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let checkpoint_id = field(&checkpoint, "checkpoint_id")?;
    let digest = reserve(&first)?;
    let observed = selected(&first, &digest, 2_000)?;
    assert_eq!(
        observed["selected_plan_wait"]["stop_reason"], "complete",
        "{observed}"
    );
    assert_eq!(observed["game_tick"], HORIZON);
    let endpoint = observed["selected_plan_wait"]["reserve_frontier"].clone();
    assert_eq!(endpoint["truth"], "true");
    assert_eq!(endpoint["durable"], true);
    assert_eq!(endpoint["survives_restart"], true);
    assert_eq!(endpoint["survives_restore"], false);
    assert_eq!(endpoint["continuous_maintenance_proven"], false);
    assert_eq!(endpoint["observed_anchor"]["game_tick"], HORIZON);
    wait(&first, 1)?;
    directory.restart();

    let second = open("7855801", false)?;
    let before = anchor(&second)?;
    let recovered = selected(&second, &digest, 0)?;
    let outcome = &recovered["selected_plan_wait"];
    assert_eq!(outcome["selection_scope"], "exact_recovered_original");
    assert_eq!(outcome["stop_reason"], "complete", "{recovered}");
    assert_eq!(
        outcome["reserve_frontier"]["observed_anchor"],
        endpoint["observed_anchor"]
    );
    assert_eq!(outcome["reserve_frontier"]["truth"], "true");
    assert_eq!(outcome["reserve_frontier"]["durable"], true);
    assert_eq!(outcome["reserve_frontier"]["reused_from_prior_wait"], true);
    assert_eq!(
        outcome["reserve_frontier"]["retention"],
        "durable_original_exact_snapshot"
    );
    assert_eq!(outcome["owns_original_dispatch"], false);
    assert_eq!(outcome["dispatch_handles_restored"], false);
    assert_eq!(recovered["advanced_game_ticks"], 0);
    assert_eq!(recovered["polled_actions"], json!([]));
    assert_eq!(anchor(&second)?, before);
    assert_eq!(
        recovered["agent_turn"]["anchor"],
        recovered["observed_anchor"]
    );

    let restored = payload(&fortress_restore(Some(second.clone()), checkpoint_id))?;
    assert_eq!(restored["ok"], true, "{restored}");
    let before = anchor(&second)?;
    let refused = payload(&fortress_wait_for_plan(
        Some(second.clone()),
        Some(100),
        Some(digest),
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(anchor(&second)?, before);
    Ok(())
}

#[test]
fn restart_cannot_invent_an_endpoint_that_no_selected_wait_observed() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    let directory = StateDir::new("missed");
    let first = open("7855802", true)?;
    let digest = reserve(&first)?;
    wait(&first, 2_000)?;
    directory.restart();
    let second = open("7855802", false)?;
    let before = anchor(&second)?;
    let recovered = selected(&second, &digest, 100)?;
    let outcome = &recovered["selected_plan_wait"];
    assert_eq!(outcome["actions_verified"], true, "{recovered}");
    assert_eq!(outcome["physical_quiescent"], true);
    assert_eq!(outcome["stop_reason"], "reserve_unobserved", "{recovered}");
    assert_eq!(outcome["complete"], false);
    assert_eq!(outcome["reserve_frontier"]["available"], false);
    assert_eq!(outcome["reserve_frontier"]["durable"], false);
    assert_eq!(recovered["advanced_game_ticks"], 0);
    assert_eq!(anchor(&second)?, before);
    Ok(())
}

#[test]
fn recovered_false_and_unknown_endpoints_cannot_be_repaired_by_later_stock() -> TestResult {
    let _serial = crate::server::serialized_durable_tests();
    for (label, selector, quantity, truth, stop) in [
        ("false", "7855803", Some(0), "false", "reserve_unmet"),
        ("unknown", "7855804", None, "unknown", "reserve_unknown"),
    ] {
        let directory = StateDir::new(label);
        let first = open(selector, true)?;
        let digest = reserve(&first)?;
        wait(&first, HORIZON - anchor(&first)?.tick.0)?;
        stock_at(&first, quantity, HORIZON)?;
        // Snapshot injection invalidates live adapter handles. Reopen at this
        // exact tick and use the real archived receipts, never a fabricated
        // live receipt, for the first selected endpoint observation.
        directory.restart();
        let endpoint_owner = open(selector, false)?;
        let observed = selected(&endpoint_owner, &digest, 0)?;
        let endpoint = observed["selected_plan_wait"]["reserve_frontier"].clone();
        assert_eq!(endpoint["truth"], truth, "{observed}");
        assert_eq!(endpoint["durable"], true);
        assert_eq!(endpoint["observed_anchor"]["game_tick"], HORIZON);

        stock_at(&endpoint_owner, Some(1_000), HORIZON + 1)?;
        directory.restart();
        let second = open(selector, false)?;
        let before = anchor(&second)?;
        let recovered = selected(&second, &digest, 100)?;
        let outcome = &recovered["selected_plan_wait"];
        assert_eq!(outcome["goal_truth"], "true", "{recovered}");
        assert_eq!(outcome["actions_verified"], true);
        assert_eq!(outcome["physical_quiescent"], true);
        assert_eq!(outcome["complete"], false);
        assert_eq!(outcome["stop_reason"], stop, "{recovered}");
        assert_eq!(outcome["reserve_frontier"]["truth"], truth);
        assert_eq!(
            outcome["reserve_frontier"]["observed_anchor"],
            endpoint["observed_anchor"]
        );
        assert_eq!(outcome["reserve_frontier"]["durable"], true);
        assert_eq!(recovered["advanced_game_ticks"], 0);
        assert_eq!(anchor(&second)?, before);
    }
    Ok(())
}
