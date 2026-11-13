//! Exact original observation Tasks do not acquire mutation or drain ownership.

use super::*;
use fastmcp_rust::{
    FinalTask, FinalTaskId, FinalTaskStatusNotification, FinalTaskStore, FinalTaskWorkDescriptor,
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const OBSERVER_SCHEMA: &str = "dfmcp.lab-original-observer-task-work/1";

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn field(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}

fn open(selector: &str, durable: bool, effects: bool) -> TestResult<String> {
    let mut grants = vec![
        ("observe", "read_only"),
        ("query", "read_only"),
        ("control_clock", "reversible"),
        ("checkpoint", "guarded"),
        ("restore", "guarded"),
    ];
    if effects {
        grants.extend([("plan", "guarded"), ("designate", "guarded")]);
    }
    let opened = parsed(&open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            grants
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
    field(&opened, "session_id")
}

fn plan(session: &str) -> TestResult<String> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()), Some("observe the exact original excavation".to_owned()), None,
        Some(json!([
            {"action":{"kind":"pause","paused":false}},
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"},"depends_on":[0]},
        ]).to_string()), None, None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    field(&planned, "plan_digest")
}

fn commit(session: &str) -> TestResult<String> {
    let digest = plan(session)?;
    let committed = parsed(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let progressed = parsed(&wait_with_ticks(Some(session.to_owned()), Some(20)))?;
    assert_eq!(progressed["ok"], true, "{progressed}");
    Ok(digest)
}

fn finish(session: &str, digest: &str) -> TestResult {
    let result = parsed(&wait_for_plan(
        Some(session.to_owned()),
        Some(2_000),
        digest.to_owned(),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["selected_plan_wait"]["complete"], true, "{result}");
    Ok(())
}

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-observer-task-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }

    fn restart(&self) {
        simulate_durable_restart(Some(self.0.clone()));
    }
}
impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn owned_observer_requires_only_current_read_grants_and_never_dispatches() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824401", false, true)?;
    let digest = commit(&session)?;
    let (grants, before, next_request) = with_task_session(&session, |guard| {
        let grants = guard.grants.clone();
        guard
            .grants
            .retain(|grant| matches!(grant.capability, Capability::Observe | Capability::Query));
        Ok((
            grants,
            guard.adapter.snapshot().clone(),
            guard.next_request_id,
        ))
    })?;
    validate_observation(&session, &digest)?;
    let observed = observation_view(&session, &digest)?;
    assert_eq!(
        observed.status,
        McpTaskStatus::Working,
        "{}",
        observed.payload
    );
    assert_eq!(observed.payload["monitor_kind"], "observe_original");
    assert_eq!(observed.payload["cancellation_scope"], "monitor_only");
    assert_eq!(observed.payload["owns_original_dispatch"], false);
    assert_eq!(observed.payload["effects_dispatched_by_task"], false);
    assert_eq!(
        observed.payload["selected_plan_wait"]["selection_origin"],
        "current_session_commit"
    );
    with_task_session(&session, |guard| {
        assert_eq!(guard.adapter.snapshot(), &before);
        assert_eq!(guard.next_request_id, next_request);
        assert_eq!(guard.commit_receipts.len(), 1);
        guard.grants = grants;
        Ok(())
    })?;
    finish(&session, &digest)?;
    let completed = observation_view(&session, &digest)?;
    assert_eq!(
        completed.status,
        McpTaskStatus::Completed,
        "{}",
        completed.payload
    );
    assert_eq!(completed.payload["action_proof_status"], "completed");
    assert_eq!(completed.payload["selected_plan_wait"]["complete"], true);
    assert_eq!(completed.payload["physical_quiescent"], true);
    assert_eq!(completed.payload["effect_cancellation_allowed"], false);
    Ok(())
}

#[test]
fn recovered_original_observer_completes_without_recreating_any_action_handles() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("recovery");
    let first = open("7824402", true, true)?;
    let digest = commit(&first)?;
    directory.restart();
    let second = open("7824402", true, false)?;
    let before = with_task_read(&second, |guard| Ok(guard.adapter.snapshot().clone()))?;
    validate_observation(&second, &digest)?;
    let observed = observation_view(&second, &digest)?;
    assert_eq!(
        observed.status,
        McpTaskStatus::Working,
        "{}",
        observed.payload
    );
    assert_eq!(
        observed.payload["selected_plan_wait"]["selection_origin"],
        "durable_recovery"
    );
    assert_eq!(
        observed.payload["selected_plan_wait"]["original_owner_session_id"],
        first
    );
    assert_eq!(
        observed.payload["selected_plan_wait"]["owns_original_dispatch"],
        false
    );
    assert_eq!(observed.payload["dispatch_handles_restored"], false);
    with_task_read(&second, |guard| {
        assert_eq!(guard.adapter.snapshot(), &before);
        assert!(guard.commit_receipts.is_empty());
        assert!(guard.last_plan_actions.is_empty());
        Ok(())
    })?;
    finish(&second, &digest)?;
    let completed = observation_view(&second, &digest)?;
    assert_eq!(
        completed.status,
        McpTaskStatus::Completed,
        "{}",
        completed.payload
    );
    assert_eq!(
        completed.payload["selected_plan_wait"]["verified_actions"],
        2
    );
    assert_eq!(completed.payload["owns_original_dispatch"], false);
    assert_eq!(completed.payload["effects_dispatched_by_task"], false);
    with_task_read(&second, |guard| {
        assert!(guard.commit_receipts.is_empty());
        assert!(guard.last_plan_actions.is_empty());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn observer_admission_refuses_pending_incomplete_and_unauthorized_originals() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824403", false, true)?;
    let pending = plan(&session)?;
    assert!(validate_observation(&session, &pending).is_err());
    let committed = parsed(&fortress_commit(Some(session.clone()), pending.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    validate_observation(&session, &pending)?;
    with_task_session(&session, |guard| {
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
        Ok(())
    })?;
    assert_eq!(
        validate_observation(&session, &pending)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::CapabilityDenied)
    );

    let session = open("7824404", false, true)?;
    let digest = commit(&session)?;
    with_task_session(&session, |guard| {
        let raw = guard
            .commit_receipts
            .get_mut(&digest)
            .ok_or_else(session_error)?;
        let mut receipt: Value = serde_json::from_str(raw).map_err(|_| session_error())?;
        receipt["actions"]
            .as_array_mut()
            .ok_or_else(session_error)?
            .pop();
        *raw = receipt.to_string();
        Ok(())
    })?;
    assert!(validate_observation(&session, &digest).is_err());
    assert!(validate_observation(&session, &"0".repeat(64)).is_err());
    Ok(())
}

fn task_wire(id: &str, status: &str) -> Value {
    json!({"taskId":id,"status":status,"createdAt":"2026-07-28T12:00:00.000Z",
        "lastUpdatedAt":"2026-07-28T12:00:00.000Z","ttlMs":null})
}

fn notification(task: &Value) -> TestResult<FinalTaskStatusNotification> {
    Ok(serde_json::from_value(
        json!({"jsonrpc":"2.0","method":"notifications/tasks","params":task}),
    )?)
}

fn create(
    store: &crate::task_store::LabTaskStore,
    id: &str,
    session: &str,
    digest: &str,
    schema: &str,
) -> TestResult<FinalTaskId> {
    let task = task_wire(id, "working");
    store.create_task_with_work(
        serde_json::from_value(task.clone())?,
        notification(&task)?,
        FinalTaskWorkDescriptor::new(
            json!({"schema":schema,"session_id":session,"plan_digest":digest}),
        )?,
    )?;
    Ok(FinalTaskId::parse(id)?)
}

#[test]
fn every_observer_cancellation_entry_stops_only_the_monitor_and_preserves_work() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, mode) in [
        ("7824405", "request"),
        ("7824406", "conditional"),
        ("7824407", "clear_input"),
    ] {
        let session = open(selector, false, true)?;
        let digest = commit(&session)?;
        let store = crate::task_store::LabTaskStore::new()?;
        let id = create(&store, mode, &session, &digest, OBSERVER_SCHEMA)?;
        assert!(store.mark_dispatch_started(&id).is_err());
        let before = with_task_session(&session, |guard| {
            guard
                .grants
                .retain(|grant| grant.capability == Capability::Observe);
            let receipts = guard
                .last_plan_actions
                .iter()
                .map(|id| {
                    guard
                        .adapter
                        .action_receipt(*id)
                        .cloned()
                        .ok_or_else(session_error)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok((
                guard.adapter.snapshot().clone(),
                guard.next_request_id,
                receipts,
            ))
        })?;
        match mode {
            "request" => store.request_cancellation(&id)?,
            "conditional" => {
                let snapshot = store.get_task_snapshot(&id)?.ok_or("Task missing")?;
                assert!(store.request_cancellation_if_current(&snapshot)?);
            }
            _ => {
                let snapshot = store.get_task_snapshot(&id)?.ok_or("Task missing")?;
                let cancelled = task_wire(mode, "cancelled");
                let task: FinalTask = serde_json::from_value(cancelled.clone())?;
                assert!(
                    store
                        .request_cancellation_and_clear_input_if_current(
                            &snapshot,
                            task,
                            notification(&cancelled)?
                        )?
                        .is_some()
                );
            }
        }
        let detail = store.task_detail(&session, &id)?;
        assert_eq!(detail["progress"]["stage"], "monitor_finalized");
        assert_eq!(detail["progress"]["effect_cancellation_requested"], false);
        assert_eq!(detail["progress"]["physical_quiescence_certified"], false);
        assert!(detail["progress"]["finalize_certificate"].is_null());
        with_task_read(&session, |guard| {
            assert_eq!(guard.adapter.snapshot(), &before.0);
            assert_eq!(guard.next_request_id, before.1);
            let receipts = guard
                .last_plan_actions
                .iter()
                .map(|id| {
                    guard
                        .adapter
                        .action_receipt(*id)
                        .cloned()
                        .ok_or_else(session_error)
                })
                .collect::<Result<Vec<_>>>()?;
            assert_eq!(receipts, before.2);
            assert!(guard.last_plan_actions.iter().any(|id| {
                matches!(
                    guard.adapter.action_work_state(*id),
                    Ok(EffectWorkState::Active { .. })
                )
            }));
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn observer_and_commit_tasks_share_one_active_capacity_and_recheck_custody_at_creation()
-> TestResult {
    let _serial = crate::test_serial();
    let session = open("7824408", false, true)?;
    let digest = commit(&session)?;
    let store = crate::task_store::LabTaskStore::new()?;
    let id = create(&store, "first-observer", &session, &digest, OBSERVER_SCHEMA)?;
    assert!(
        create(
            &store,
            "second-observer",
            &session,
            &digest,
            OBSERVER_SCHEMA
        )
        .is_err()
    );
    assert!(
        create(
            &store,
            "commit-monitor",
            &session,
            &digest,
            "dfmcp.lab-plan-task-work/1"
        )
        .is_err()
    );
    assert_eq!(store.session_page(&session, 0)?["total"], 1);
    assert_eq!(
        store.task_detail(&session, &id)?["cancellation_scope"],
        "monitor_only"
    );
    let other = crate::task_store::LabTaskStore::new()?;
    with_task_session(&session, |guard| {
        guard
            .grants
            .retain(|grant| grant.capability != Capability::Query);
        Ok(())
    })?;
    assert!(create(&other, "unqualified", &session, &digest, OBSERVER_SCHEMA).is_err());
    assert_eq!(other.session_page(&session, 0)?["total"], 0);
    Ok(())
}
