//! Owned Task proof uses the same original project and exact reserve frontier
//! as selected foreground waiting, without capturing evidence itself.

use super::*;
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const HORIZON: u64 = 1_234;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str, scenario: &str) -> TestResult<String> {
    let opened = parsed(&open_session_in_scenario(
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
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        None,
        Some(16_384),
        Some(64),
        Some(scenario.to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn submit(session: &str, source: Value) -> TestResult<String> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(source.to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    validate(session, &digest)?;
    let committed = parsed(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(digest)
}

fn reserve(session: &str) -> TestResult<String> {
    submit(
        session,
        json!({
            "template":"production", "reserve_until_tick":HORIZON,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
        }),
    )
}

fn advance_to(session: &str, tick: u64) -> TestResult {
    let now = with_task_read(session, |guard| Ok(guard.adapter.snapshot().tick.0))?;
    let ticks = tick
        .checked_sub(now)
        .ok_or("cannot move fixture backwards")?;
    let result = parsed(&wait_with_ticks(Some(session.to_owned()), Some(ticks)))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["game_tick"], tick, "{result}");
    Ok(())
}

fn capture(session: &str, digest: &str) -> TestResult<Value> {
    let result = parsed(&wait_for_plan(
        Some(session.to_owned()),
        Some(0),
        digest.to_owned(),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["advanced_game_ticks"], 0);
    Ok(result["selected_plan_wait"].clone())
}

fn set_drink(session: &str, amount: Option<u64>) -> Result<()> {
    with_task_session(session, |guard| {
        let mut snapshot = guard.adapter.snapshot().clone();
        let ledger = effects::stock_ledger(&snapshot).ok_or_else(session_error)?;
        let stock = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(session_error)?;
        if let Some(amount) = amount {
            stock.fields.insert(
                effects::STOCK_DRINK_FIELD.to_owned(),
                Fact::known(
                    WorldValue::U64(amount),
                    snapshot.tick,
                    FactSource::Replay,
                    Digest32::of_bytes(b"task-reserve-stock-fixture"),
                ),
            );
        } else {
            stock.fields.remove(effects::STOCK_DRINK_FIELD);
        }
        stock.revision += 1;
        snapshot.cursor = snapshot.cursor.checked_next().ok_or_else(session_error)?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)
    })
}

#[test]
fn funded_future_reserve_task_waits_for_explicit_exact_endpoint_without_writing_it() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824301", "starter_fortress")?;
    let digest = reserve(&session)?;
    advance_to(&session, HORIZON - 1)?;
    let pending = view(&session, &digest)?;
    assert_eq!(
        pending.status,
        McpTaskStatus::Working,
        "{}",
        pending.payload
    );
    assert_eq!(pending.payload["action_proof_status"], "completed");
    assert_eq!(pending.payload["physical_quiescent"], true);
    assert_eq!(pending.payload["original_goal"]["predicate_truth"], "true");
    assert_eq!(pending.payload["original_goal_satisfied"], false);
    assert_eq!(pending.payload["original_goal_pending"], true);
    assert_eq!(pending.payload["proof_status"], "working");
    assert_eq!(
        pending.payload["next_step"]["arguments"]["plan_digest"],
        digest
    );
    assert!(cancellation_admission(&session, &digest)?);

    advance_to(&session, HORIZON)?;
    let before = with_task_read(&session, |guard| {
        Ok((guard.adapter.snapshot().clone(), guard.next_request_id))
    })?;
    let at_frontier = view(&session, &digest)?;
    assert_eq!(at_frontier.status, McpTaskStatus::Working);
    assert_eq!(
        at_frontier.payload["selected_plan_wait"]["reserve_capture_required"],
        true
    );
    assert_eq!(
        at_frontier.payload["next_step"]["arguments"]["max_game_ticks"],
        0
    );
    assert!(cancellation_admission(&session, &digest)?);
    with_task_read(&session, |guard| {
        assert_eq!(guard.adapter.snapshot(), &before.0);
        assert_eq!(guard.next_request_id, before.1);
        assert!(
            guard
                .objectives
                .iter()
                .all(|goal| goal.selected_wait_reserve_observation.is_none())
        );
        Ok(())
    })?;

    let selected = capture(&session, &digest)?;
    assert_eq!(selected["reserve_frontier"]["truth"], "true");
    let completed = view(&session, &digest)?;
    assert_eq!(
        completed.status,
        McpTaskStatus::Completed,
        "{}",
        completed.payload
    );
    assert_eq!(completed.payload["original_goal_pending"], false);
    assert_eq!(completed.payload["selected_plan_wait"]["complete"], true);
    assert_eq!(
        completed.payload["selected_plan_wait"]["reserve_frontier"]["reused_from_prior_wait"],
        true
    );
    assert!(cancellation_admission(&session, &digest).is_err());
    Ok(())
}

#[test]
fn missing_reserve_endpoint_fails_despite_true_later_stock() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824302", "starter_fortress")?;
    let digest = reserve(&session)?;
    advance_to(&session, HORIZON + 1)?;
    let failed = view(&session, &digest)?;
    assert_eq!(failed.status, McpTaskStatus::Failed, "{}", failed.payload);
    assert_eq!(failed.payload["original_goal"]["predicate_truth"], "true");
    assert_eq!(failed.payload["original_goal_satisfied"], false);
    assert_eq!(failed.payload["action_proof_status"], "completed");
    assert_eq!(failed.payload["physical_quiescent"], true);
    assert_eq!(
        failed.payload["selected_plan_wait"]["stop_reason"],
        "reserve_unobserved"
    );
    assert_eq!(
        failed.payload["selected_plan_wait"]["reserve_frontier"]["available"],
        false
    );
    assert_eq!(failed.payload["recovery_class"], "reconciliation_required");
    assert_eq!(failed.payload["blind_retry_allowed"], false);
    assert!(cancellation_admission(&session, &digest)?);
    set_drink(&session, Some(1_000))?;
    assert_eq!(view(&session, &digest)?.status, McpTaskStatus::Failed);
    with_task_read(&session, |guard| {
        assert!(
            guard
                .objectives
                .iter()
                .all(|goal| goal.selected_wait_reserve_observation.is_none())
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn failed_and_unknown_first_endpoints_cannot_be_repaired_by_current_task_stock() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, stock, truth, stop) in [
        ("7824303", Some(0), "false", "reserve_unmet"),
        ("7824304", None, "unknown", "reserve_unknown"),
    ] {
        let session = open(selector, "starter_fortress")?;
        let digest = reserve(&session)?;
        advance_to(&session, HORIZON)?;
        set_drink(&session, stock)?;
        let selected = capture(&session, &digest)?;
        assert_eq!(selected["reserve_frontier"]["truth"], truth, "{selected}");
        set_drink(&session, Some(1_000))?;
        let observed = view(&session, &digest)?;
        assert_eq!(
            observed.status,
            McpTaskStatus::Failed,
            "{}",
            observed.payload
        );
        assert_eq!(observed.payload["original_goal"]["predicate_truth"], "true");
        assert_eq!(observed.payload["original_goal_satisfied"], false);
        assert_eq!(observed.payload["selected_plan_wait"]["stop_reason"], stop);
        assert_eq!(
            observed.payload["selected_plan_wait"]["reserve_frontier"]["truth"],
            truth
        );
        assert_eq!(observed.payload["needs_replan"], stock.is_some());
        assert!(cancellation_admission(&session, &digest)?);
    }
    Ok(())
}

#[test]
fn cancelling_pending_reserve_monitor_proves_only_physical_drain() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824305", "starter_fortress")?;
    let digest = reserve(&session)?;
    advance_to(&session, HORIZON - 1)?;
    let before = view(&session, &digest)?;
    assert_eq!(before.status, McpTaskStatus::Working);
    assert!(cancellation_admission(&session, &digest)?);
    let requested = drain(&session, &digest, false)?;
    let finalized = drain(&session, &digest, true)?;
    assert_eq!(requested["proof_status"], "working");
    assert_eq!(finalized["proof_status"], "working");
    assert_eq!(finalized["original_goal_pending"], true);
    assert_eq!(finalized["drain_progress"]["quiescent"], true);
    assert_eq!(finalized["finalize_certificate"]["proof_status"], "working");
    assert_eq!(
        finalized["selected_plan_wait"]["reserve_frontier"]["available"],
        false
    );
    let after = view(&session, &digest)?;
    assert_eq!(after.payload["actions"], before.payload["actions"]);
    assert_eq!(
        after.payload["observed_anchor"],
        before.payload["observed_anchor"]
    );
    Ok(())
}

#[test]
fn revoked_query_refuses_task_admission_proof_and_cancellation_before_effects() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824306", "starter_fortress")?;
    let digest = reserve(&session)?;
    advance_to(&session, HORIZON - 1)?;
    let before = with_task_session(&session, |guard| {
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
        Ok(guard.adapter.snapshot().clone())
    })?;
    assert_eq!(
        validate(&session, &digest).err().map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(
        view(&session, &digest).err().map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(
        cancellation_admission(&session, &digest)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(
        drain(&session, &digest, true).err().map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );
    with_task_read(&session, |guard| {
        assert_eq!(guard.adapter.snapshot(), &before);
        assert!(
            guard
                .objectives
                .iter()
                .all(|goal| goal.selected_wait_reserve_observation.is_none())
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn completed_harvest_task_requires_the_complete_original_room_and_explicit_continuation()
-> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824307", "starter_fortress_resources")?;
    with_task_session(&session, |guard| {
        let mut snapshot = guard.adapter.snapshot().clone();
        let ledger = effects::stock_ledger(&snapshot).ok_or_else(session_error)?;
        let stock = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(session_error)?;
        stock
            .fields
            .get_mut(effects::STOCK_WOOD_FIELD)
            .ok_or_else(session_error)?
            .value = WorldValue::U64(0);
        stock.revision += 1;
        snapshot
            .graph
            .entities
            .remove(&crate::lab_world::starter::CARPENTERS);
        snapshot.cursor = snapshot.cursor.checked_next().ok_or_else(session_error)?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)
    })?;
    let digest = submit(
        &session,
        json!({
            "template":"provisioned_room",
            "layout":{"template":"bedroom_cluster", "origin":[2,6,10], "rooms":2, "room_size":[3,3]},
            "acquire_resources":{"assign_labor":false},
            "prerequisites":{"assign_labor":false,"workshops":[{
                "building":"workshop:Carpenters", "location":[0,0,10],
            }]},
        }),
    )?;
    let waited = parsed(&wait_for_plan(
        Some(session.clone()),
        Some(2_000),
        digest.clone(),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let observed = view(&session, &digest)?;
    assert_eq!(
        observed.status,
        McpTaskStatus::Failed,
        "{}",
        observed.payload
    );
    assert_eq!(observed.payload["action_proof_status"], "completed");
    assert_eq!(observed.payload["physical_quiescent"], true);
    assert_eq!(
        observed.payload["selected_plan_wait"]["stage_goal_truth"],
        "true"
    );
    assert_eq!(
        observed.payload["selected_plan_wait"]["goal_truth"],
        "false"
    );
    assert_eq!(
        observed.payload["selected_plan_wait"]["stop_reason"],
        "needs_continuation"
    );
    assert_eq!(observed.payload["needs_replan"], true);
    assert_eq!(
        observed.payload["next_step"],
        observed.payload["selected_plan_wait"]["continuation_request"]
    );
    assert_eq!(observed.payload["replacement_work_dispatched"], false);
    Ok(())
}
