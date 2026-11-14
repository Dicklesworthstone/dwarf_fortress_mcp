//! Real laboratory and Tasks-store replay regressions. These tests exercise
//! the normal facade, retained original receipts and existing drain engine.

use super::*;
use crate::agent_facade as f;
use crate::server::task_session;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{ActionId, Capability, CommitState, MapCoord, MapCuboid};
use fastmcp_rust::{
    FinalTask, FinalTaskId, FinalTaskStatusNotification, FinalTaskStore, FinalTaskWorkDescriptor,
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn field(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key].as_str().ok_or_else(|| format!("missing {key}: {value}"))?.to_owned())
}

fn open(selector: &str) -> TestResult<String> {
    let grants = [
        ("observe", "read_only"), ("query", "read_only"), ("plan", "guarded"),
        ("designate", "guarded"), ("checkpoint", "guarded"), ("control_clock", "reversible"),
    ];
    let opened = parsed(&f::fortress_open_session(
        Some(false), Some(selector.to_owned()),
        Some(grants.into_iter().map(|(capability, risk)| (capability.to_owned(), risk.to_owned())).collect()),
        Some(60_000), Some(2_000), Some(8_192), None, Some(16_384), Some(64),
        Some("starter_fortress".to_owned()), Some(false), Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    field(&opened, "session_id")
}

fn commit(session: &str, areas: &[(i32, i32)]) -> TestResult<(String, Vec<ActionId>)> {
    let actions = areas.iter().map(|(left, right)| json!({
        "action":{"kind":"designate_dig","min":[left,3,10],"max":[right,5,10],"mode":"mine"},
    })).collect::<Vec<_>>();
    let planned = parsed(&f::fortress_plan(
        Some(session.to_owned()), Some("replay exact Task cancellation".to_owned()), None,
        Some(json!(actions).to_string()), None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = field(&planned, "plan_digest")?;
    let committed = parsed(&f::fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let ids = committed["actions"].as_array().ok_or("missing actions")?.iter()
        .map(|row| {
            let raw = row["action_id"].as_str().ok_or("missing action id")?;
            Ok(ActionId::new(u128::from_str_radix(raw, 16)?))
        }).collect::<TestResult<Vec<_>>>()?;
    Ok((digest, ids))
}

fn rehash(bundle: &mut Value) {
    bundle["calls_digest"] = json!(if bundle["schema"] == TASK_BUNDLE_SCHEMA {
        task_calls_digest(&bundle["calls"])
    } else {
        calls_digest(&bundle["calls"])
    });
}

fn exact_roundtrip(session: &str) -> TestResult<Value> {
    let bundle = crate::server::replay_bundle_for(session)?;
    assert_eq!(bundle["schema"], TASK_BUNDLE_SCHEMA);
    assert_eq!(bundle["replayable"], true, "{bundle}");
    let report = replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    let replayed = field(&report, "replay_session_id")?;
    assert_eq!(crate::server::replay_bundle_for(&replayed)?, bundle);
    Ok(bundle)
}

#[test]
fn original_task_phases_replay_without_cancelling_later_work_or_losing_pending_review() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824501")?;
    let (original, original_ids) = commit(&session, &[(0, 3)])?;
    let (later, later_ids) = commit(&session, &[(5, 7)])?;
    let pending = parsed(&f::fortress_plan(
        Some(session.clone()), None, Some(true), None, None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");
    let pending_digest = field(&pending, "plan_digest")?;
    let requested = task_session::drain(&session, &original, false)?;
    assert_eq!(requested["stage"], "cancel_requested");
    let finalized = task_session::drain(&session, &original, true)?;
    assert_eq!(finalized["drain_progress"]["quiescent"], true);
    assert!(finalized["finalize_certificate"].is_object());
    assert_eq!(finalized["plan_digest"], original);
    assert_eq!(task_session::view(&session, &later)?.payload["physical_quiescent"], false);
    let observed = parsed(&f::fortress_observe(Some(session.clone())))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let bundle = exact_roundtrip(&session)?;
    assert_eq!(bundle["calls"].as_array().ok_or("calls")?.iter()
        .filter(|entry| entry["event"] == "task_cancel_phase").count(), 2);
    let handle = crate::server::lookup_session_str(&session)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    assert_eq!(guard.adapter.action_receipt(original_ids[0]).ok_or("original")?.state, CommitState::Cancelled);
    assert!(!guard.adapter.action_receipt(later_ids[0]).ok_or("later")?.state.is_terminal());
    // Custody survives the stop; an unchanged seal still needs ordinary
    // commit-time revalidation against its now-advanced anchor.
    assert_eq!(crate::server::handoff_json(&guard)["pending_plan"]["plan_digest"], pending_digest);
    Ok(())
}

fn create_task(
    store: &crate::task_store::LabTaskStore,
    id: &str,
    session: &str,
    digest: &str,
    schema: &str,
) -> TestResult<FinalTaskId> {
    let task = json!({
        "taskId":id,"status":"working","createdAt":"2026-07-28T12:00:00.000Z",
        "lastUpdatedAt":"2026-07-28T12:00:00.000Z","ttlMs":null,
    });
    let notification: FinalTaskStatusNotification = serde_json::from_value(
        json!({"jsonrpc":"2.0","method":"notifications/tasks","params":task}),
    )?;
    store.create_task_with_work(
        serde_json::from_value(task)?, notification,
        FinalTaskWorkDescriptor::new(json!({
            "schema":schema,"session_id":session,"plan_digest":digest,
        }))?,
    )?;
    Ok(FinalTaskId::parse(id)?)
}

#[test]
fn every_transport_cancellation_entry_records_both_real_engine_phases() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, mode) in [("7824502", "request"), ("7824503", "conditional"), ("7824504", "clear_input")] {
        let session = open(selector)?;
        let (digest, _) = commit(&session, &[(0, 7)])?;
        let store = crate::task_store::LabTaskStore::new()?;
        let id = create_task(&store, mode, &session, &digest, "dfmcp.lab-plan-task-work/1")?;
        match mode {
            "request" => store.request_cancellation(&id)?,
            "conditional" => {
                let snapshot = store.get_task_snapshot(&id)?.ok_or("task")?;
                assert!(store.request_cancellation_if_current(&snapshot)?);
            }
            _ => {
                let snapshot = store.get_task_snapshot(&id)?.ok_or("task")?;
                let cancelled = json!({
                    "taskId":mode,"status":"cancelled","createdAt":"2026-07-28T12:00:00.000Z",
                    "lastUpdatedAt":"2026-07-28T12:00:00.000Z","ttlMs":null,
                });
                let task: FinalTask = serde_json::from_value(cancelled.clone())?;
                let notification = serde_json::from_value(json!({
                    "jsonrpc":"2.0","method":"notifications/tasks","params":cancelled,
                }))?;
                assert!(store.request_cancellation_and_clear_input_if_current(
                    &snapshot, task, notification,
                )?.is_some());
            }
        }
        let detail = store.task_detail(&session, &id)?;
        assert_eq!(detail["progress"]["stage"], "finalized");
        assert_eq!(detail["progress"]["finalization"]["drain_progress"]["quiescent"], true);
        let bundle = exact_roundtrip(&session)?;
        assert_eq!(bundle["calls"].as_array().ok_or("calls")?.iter()
            .filter(|entry| entry["event"] == "task_cancel_phase").count(), 2);
        let before_retry = bundle;
        // The upstream store may report an already-terminal Task; either
        // response must retain the exact engine transcript on retry.
        let _retry = store.request_cancellation(&id);
        assert_eq!(crate::server::replay_bundle_for(&session)?, before_retry);
    }
    Ok(())
}

#[test]
fn passive_observer_cancellation_leaves_legacy_engine_transcript_unchanged() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824505")?;
    let (digest, _) = commit(&session, &[(0, 7)])?;
    let before = crate::server::replay_bundle_for(&session)?;
    let store = crate::task_store::LabTaskStore::new()?;
    let id = create_task(&store, "passive-replay", &session, &digest, "dfmcp.lab-original-observer-task-work/1")?;
    store.request_cancellation(&id)?;
    let after = crate::server::replay_bundle_for(&session)?;
    assert_eq!(before, after);
    assert_eq!(after["schema"], BUNDLE_SCHEMA);
    assert_eq!(replay_bundle(&after)["ok"], true);
    Ok(())
}

#[test]
fn refused_phase_is_recorded_and_replayed_without_fabricating_an_original() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824506")?;
    let unknown = "ab".repeat(32);
    assert!(task_session::drain(&session, &unknown, false).is_err());
    let bundle = exact_roundtrip(&session)?;
    let event = bundle["calls"].as_array().ok_or("calls")?.last().ok_or("event")?;
    assert_eq!(event["ok"], false);
    assert_eq!(event["before"], event["after"]);
    assert_eq!(event["error_code"], "conflict");
    Ok(())
}

#[test]
fn partial_authority_failure_retains_changed_receipts_and_refuses_unrecorded_grants() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824507")?;
    let (digest, ids) = commit(&session, &[(0, 3), (5, 7)])?;
    {
        let handle = crate::server::lookup_session_str(&session)?;
        let mut guard = handle.lock().map_err(|_| "session poisoned")?;
        let grant = guard.grants.iter_mut().find(|grant| grant.capability == Capability::Designate)
            .ok_or("designation grant")?;
        grant.scope.map_area = Some(MapCuboid::new(
            MapCoord::new(5, 3, 10), MapCoord::new(7, 5, 10),
        )?);
    }
    let error = task_session::drain(&session, &digest, false).err().ok_or("expected partial refusal")?;
    assert_eq!(error.code, dfmcp_core::ErrorCode::CapabilityDenied);
    let bundle = crate::server::replay_bundle_for(&session)?;
    let event = bundle["calls"].as_array().ok_or("calls")?.last().ok_or("event")?;
    assert_eq!(event["ok"], false);
    assert_ne!(event["before"]["next_request_id"], event["after"]["next_request_id"]);
    assert_ne!(event["before"]["original_actions_digest"], event["after"]["original_actions_digest"]);
    let handle = crate::server::lookup_session_str(&session)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    assert_eq!(guard.adapter.action_receipt(ids[1]).ok_or("later receipt")?.state, CommitState::CancelRequested);
    assert_ne!(guard.adapter.action_receipt(ids[0]).ok_or("earlier receipt")?.state, CommitState::CancelRequested);
    drop(guard);
    // Direct test-only grant mutation is deliberately absent from the request
    // log. The replayer diagnoses its input mismatch before draining anything.
    let report = replay_bundle(&bundle);
    assert_eq!(report["ok"], false, "{report}");
    assert_eq!(report["first_divergence"]["field"], "before");
    assert_eq!(report["event_effects_attempted"], false);
    Ok(())
}

#[test]
fn task_event_input_and_proof_tampering_is_localized_after_rehashing() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824508")?;
    let (digest, _) = commit(&session, &[(0, 7)])?;
    task_session::drain(&session, &digest, false)?;
    task_session::drain(&session, &digest, true)?;
    let bundle = exact_roundtrip(&session)?;
    let index = bundle["calls"].as_array().ok_or("calls")?.iter()
        .position(|entry| entry["event"] == "task_cancel_phase").ok_or("event")?;
    for pointer in [
        "/before/next_request_id", "/before/authority_digest",
        "/before/original_actions_digest", "/before/budget/max_bytes",
        "/after/next_request_id", "/evidence_digest",
    ] {
        let mut changed = bundle.clone();
        let event = &mut changed["calls"][index];
        let target = event.pointer_mut(pointer).ok_or("pointer")?;
        *target = if pointer.ends_with("max_bytes") {
            json!(target.as_u64().ok_or("bytes")? + 1)
        } else if pointer.ends_with("next_request_id") {
            json!("999999999")
        } else {
            json!("cd".repeat(32))
        };
        rehash(&mut changed);
        let report = replay_bundle(&changed);
        assert_eq!(report["ok"], false, "{pointer}: {report}");
        assert_eq!(report["first_divergence"]["seq"], index, "{pointer}: {report}");
        if pointer.starts_with("/before") {
            assert_eq!(report["event_effects_attempted"], false, "{report}");
        }
    }
    Ok(())
}

#[test]
fn full_prevalidation_rejects_persistent_and_malformed_later_entries_before_open() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824509")?;
    let (digest, _) = commit(&session, &[(0, 7)])?;
    task_session::drain(&session, &digest, false)?;
    let bundle = crate::server::replay_bundle_for(&session)?;
    for mutation in ["shared", "durable", "phase", "selector", "event", "counter", "sequence", "downgrade", "extra"] {
        let mut changed = bundle.clone();
        let last = changed["calls"].as_array().ok_or("calls")?.len() - 1;
        match mutation {
            "shared" | "durable" => changed["calls"][0]["arguments"][mutation] = json!(true),
            "phase" => changed["calls"][last]["arguments"]["phase"] = json!("dispatch"),
            "selector" => changed["calls"][last]["arguments"]["plan_digest"] = json!("AB".repeat(32)),
            "event" => changed["calls"][last]["event"] = json!("native_command"),
            "counter" => changed["calls"][last]["before"]["next_request_id"] = json!("01"),
            "sequence" => changed["calls"][last]["seq"] = json!(0),
            "downgrade" => changed["schema"] = json!(BUNDLE_SCHEMA),
            _ => changed["calls"][last]["arguments"]["session_id"] = json!(session),
        }
        rehash(&mut changed);
        let expected_refusal = validation::validate(&changed).expect_err("whole transcript refused");
        let report = replay_bundle(&changed);
        assert_eq!(report["ok"], false, "{mutation}: {report}");
        assert_eq!(report["replayed"], 0, "{mutation}: {report}");
        assert_eq!(report["error"], expected_refusal, "{mutation}: {report}");
    }
    Ok(())
}

#[test]
fn incomplete_order_and_capacity_never_claim_reproducible_task_effects() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824510")?;
    let (digest, _) = commit(&session, &[(0, 7)])?;
    {
        let handle = crate::server::lookup_session_str(&session)?;
        let mut guard = handle.lock().map_err(|_| "session poisoned")?;
        guard.replay.calls.pop();
    }
    task_session::drain(&session, &digest, false)?;
    let bundle = crate::server::replay_bundle_for(&session)?;
    assert_eq!(bundle["replayable"], false);
    assert_eq!(replay_bundle(&bundle)["ok"], false);
    let handle = crate::server::lookup_session_str(&session)?;
    let mut guard = handle.lock().map_err(|_| "session poisoned")?;
    let last = guard.replay.calls.last().cloned().ok_or("event")?;
    guard.replay.calls.resize(MAX_RECORDED_CALLS, last);
    drop(guard);
    task_session::drain(&session, &digest, true)?;
    let truncated = crate::server::replay_bundle_for(&session)?;
    assert_eq!(truncated["calls"].as_array().map(Vec::len), Some(MAX_RECORDED_CALLS));
    assert_eq!(truncated["truncated"], true);
    assert_eq!(truncated["replayable"], false);
    assert_eq!(replay_bundle(&truncated)["ok"], false);
    Ok(())
}


#[test]
fn withdrawn_original_task_replays_with_verified_owner_bound_withdrawal_proof() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824511")?;
    let (digest, actions) = commit(&session, &[(0, 7)])?;
    let store = crate::task_store::LabTaskStore::new()?;
    let id = create_task(&store, "withdrawn-replay", &session, &digest, "dfmcp.lab-plan-task-work/1")?;
    let stopped = parsed(&f::fortress_cancel(
        Some(session.clone()), Some("stop_future_steps".to_owned()),
        Some(format!("plan:{digest}")),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    let withdrawn = parsed(&f::fortress_cancel(
        Some(session.clone()), Some("withdraw_goal".to_owned()),
        Some(format!("goal:{digest}")),
    ))?;
    assert_eq!(withdrawn["ok"], true, "{withdrawn}");
    assert_eq!(withdrawn["withdrawal"]["requester_session_id"], session);
    let original_withdrawal = withdrawn["withdrawal"].clone();

    // A retained transport Task can still be cancelled after foreground drain
    // and explicit goal withdrawal. Its history must remain exact in replay.
    store.request_cancellation(&id)?;
    let detail = store.task_detail(&session, &id)?;
    let finalization = detail["progress"]["finalization"].clone();
    assert_eq!(finalization["original_goal"]["withdrawal"], original_withdrawal);
    assert_eq!(finalization["proof_status"], "cancelled");
    exact_roundtrip(&session)?;
    let handle = crate::server::lookup_session_str(&session)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    let (valid_digest, valid) =
        task_session::replay::session_evidence_digest(&Ok(finalization.clone()), &guard);
    assert!(valid, "retained withdrawal proof verifies before normalization");
    assert_eq!(guard.adapter.action_receipt(actions[0]).ok_or("action")?.state, CommitState::Cancelled);
    for field in ["proof_digest", "requester_session_id"] {
        let mut changed = finalization.clone();
        changed["original_goal"]["withdrawal"][field] = json!("00".repeat(32));
        let (changed_digest, verified) =
            task_session::replay::session_evidence_digest(&Ok(changed), &guard);
        assert!(!verified, "forged {field} must not be normalized");
        assert_ne!(valid_digest, changed_digest);
    }
    // Evidence comparison never rewrites the actual published withdrawal.
    assert_eq!(finalization["original_goal"]["withdrawal"], original_withdrawal);
    Ok(())
}

#[test]
fn checked_replay_phase_refuses_changed_input_without_publishing_or_draining() -> TestResult {
    use task_session::replay::{CheckedPhase, checked_drain, frontier_for};
    let _serial = crate::test_serial();
    let session = open("7824512")?;
    let (digest, _) = commit(&session, &[(0, 7)])?;
    let before = frontier_for(&session, &digest)?;
    let log_before = crate::server::replay_bundle_for(&session)?;
    let mut forged = before.clone();
    forged["next_request_id"] = json!("0");
    match checked_drain(&session, &digest, false, &forged)? {
        CheckedPhase::InputDivergence(observed) => assert_eq!(observed, before),
        CheckedPhase::Executed { .. } => panic!("changed input was drained"),
    }
    assert_eq!(frontier_for(&session, &digest)?, before);
    assert_eq!(crate::server::replay_bundle_for(&session)?, log_before);

    let (after, evidence) = match checked_drain(&session, &digest, false, &before)? {
        CheckedPhase::Executed { result, after, evidence_digest } => {
            assert_eq!(result?["stage"], "cancel_requested");
            (after, evidence_digest)
        }
        CheckedPhase::InputDivergence(_) => panic!("unchanged input was refused"),
    };
    let recorded = crate::server::replay_bundle_for(&session)?;
    let entry = recorded["calls"].as_array().ok_or("calls")?.last().ok_or("phase")?;
    assert_eq!(entry["after"], after);
    assert_eq!(entry["evidence_digest"], evidence);
    task_session::drain(&session, &digest, true)?;
    exact_roundtrip(&session)?;
    Ok(())
}

#[test]
fn evidence_normalization_preserves_user_text_and_invalid_certificate_identity() {
    use task_session::replay::evidence_digest;
    let first = json!({
        "original_goal":{"owner_session_id":"first","summary":"literal owner-like text"},
        "selected_plan_wait":{"original_owner_session_id":"first"},
        "finalize_certificate":null,
    });
    let mut second = first.clone();
    second["original_goal"]["owner_session_id"] = json!("second");
    second["selected_plan_wait"]["original_owner_session_id"] = json!("second");
    assert_eq!(evidence_digest(&Ok(first.clone()), "first"), evidence_digest(&Ok(second.clone()), "second"));
    second["original_goal"]["summary"] = json!("second");
    assert_ne!(evidence_digest(&Ok(first), "first"), evidence_digest(&Ok(second.clone()), "second"));
    second["finalize_certificate"] = json!({"digest":"11".repeat(32)});
    let before = evidence_digest(&Ok(second.clone()), "second");
    second["finalize_certificate"]["digest"] = json!("22".repeat(32));
    assert_ne!(before, evidence_digest(&Ok(second), "second"));
}
