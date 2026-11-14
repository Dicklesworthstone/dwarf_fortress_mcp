//! Group Task custody uses one monitor and never acquires selected effects.
//! Regression scope: df-fastmcp-conformance-5pj.3.1.
use super::*;

const SET_SCHEMA: &str = "dfmcp.lab-original-set-observer-task-work/1";

fn live_pair(session: &str) -> TestResult<Vec<String>> {
    let first = commit(session)?;
    let second = parsed(&plan_request(
        Some(session.to_owned()),
        Some("independent foreground clock state".to_owned()),
        None,
        Some(json!([{"action":{"kind":"pause","paused":false}}]).to_string()),
        None,
        None,
    ))?;
    assert_eq!(second["ok"], true, "{second}");
    let second = field(&second, "plan_digest")?;
    let committed = parsed(&fortress_commit(Some(session.to_owned()), second.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_ne!(first, second);
    Ok(vec![second, first])
}

fn create_set(
    store: &crate::task_store::LabTaskStore,
    id: &str,
    session: &str,
    digests: &[String],
    policy: &str,
) -> TestResult<FinalTaskId> {
    let task = task_wire(id, "working");
    store.create_task_with_work(
        serde_json::from_value(task.clone())?,
        notification(&task)?,
        FinalTaskWorkDescriptor::new(json!({
            "schema":SET_SCHEMA,"session_id":session,
            "plan_digests":digests,"plan_policy":policy,
        }))?,
    )?;
    Ok(FinalTaskId::parse(id)?)
}

#[test]
fn every_group_cancellation_entry_preserves_all_original_work_and_identity() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, mode) in [
        ("7872301", "request"),
        ("7872302", "conditional"),
        ("7872303", "clear_input"),
    ] {
        let session = open(selector, false, true)?;
        let digests = live_pair(&session)?;
        let observed = observation_set_view(&session, &digests, "all")?;
        assert_eq!(
            observed.status,
            McpTaskStatus::Working,
            "{}",
            observed.payload
        );
        assert_eq!(observed.payload["physical_quiescent"], false);
        let store = crate::task_store::LabTaskStore::new()?;
        let id = create_set(&store, mode, &session, &digests, "all")?;
        assert!(store.mark_dispatch_started(&id).is_err());
        let before = with_task_session(&session, |guard| {
            guard
                .grants
                .retain(|grant| grant.capability == Capability::Observe);
            Ok((
                guard.adapter.snapshot().clone(),
                guard.next_request_id,
                guard.commit_receipts.clone(),
                guard.open_actions.clone(),
                guard.last_plan_actions.clone(),
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
                assert!(
                    store
                        .request_cancellation_and_clear_input_if_current(
                            &snapshot,
                            serde_json::from_value(cancelled.clone())?,
                            notification(&cancelled)?,
                        )?
                        .is_some()
                );
            }
        }
        let detail = store.task_detail(&session, &id)?;
        assert!(detail.get("plan_digest").is_none());
        assert_eq!(detail["plan_digests"], json!(digests));
        let progress = &detail["progress"];
        assert_eq!(progress["stage"], "monitor_finalized");
        assert_eq!(progress["monitor_kind"], "observe_original_set");
        assert_eq!(progress["cancellation_scope"], "monitor_only");
        assert_eq!(progress["plan_digests"], json!(digests));
        assert!(progress.get("plan_digest").is_none());
        assert_eq!(progress["physical_work_changed"], false);
        assert_eq!(progress["effect_cancellation_requested"], false);
        assert_eq!(progress["physical_quiescence_certified"], false);
        assert!(progress["finalize_certificate"].is_null());
        assert_eq!(
            progress["observation_binding"]["plans"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        let mut actual = digests.clone();
        actual.sort();
        assert_eq!(progress["actual_plan_digests"], json!(actual));
        with_task_read(&session, |guard| {
            assert_eq!(guard.adapter.snapshot(), &before.0);
            assert_eq!(guard.next_request_id, before.1);
            assert_eq!(guard.commit_receipts, before.2);
            assert_eq!(guard.open_actions, before.3);
            assert_eq!(guard.last_plan_actions, before.4);
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn group_creation_rechecks_every_member_before_taking_the_single_monitor_slot() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872304", false, true)?;
    let digests = live_pair(&session)?;
    let store = crate::task_store::LabTaskStore::new()?;
    let before = with_task_read(&session, |guard| Ok(guard.adapter.snapshot().clone()))?;
    let mut unavailable = digests.clone();
    unavailable[1] = "0".repeat(64);
    assert!(create_set(&store, "unavailable", &session, &unavailable, "all").is_err());
    assert_eq!(store.session_page(&session, 0)?["total"], 0);
    with_task_read(&session, |guard| {
        assert_eq!(guard.adapter.snapshot(), &before);
        Ok(())
    })?;
    let id = create_set(&store, "selected-originals", &session, &digests, "all")?;
    assert!(create_set(&store, "second-group", &session, &digests, "any").is_err());
    assert!(
        create(
            &store,
            "single-observer",
            &session,
            &digests[0],
            OBSERVER_SCHEMA
        )
        .is_err()
    );
    assert!(
        create(
            &store,
            "commit-monitor",
            &session,
            &digests[0],
            "dfmcp.lab-plan-task-work/1"
        )
        .is_err()
    );
    let page = store.session_page(&session, 0)?;
    assert_eq!(page["total"], 1);
    assert_eq!(page["tasks"][0]["plan_digests"], json!(digests));
    assert_eq!(page["tasks"][0]["monitor_kind"], "observe_original_set");
    assert!(page["tasks"][0].get("plan_digest").is_none());
    assert!(store.with_session_idle(&session, |_| Ok(())).is_err());
    let scope = store.monitor_scope(&id)?;
    assert!(store.monitor_scope(&id).is_err());
    let snapshot = store.get_task_snapshot(&id)?.ok_or("Task missing")?;
    let cancelled = task_wire("selected-originals", "cancelled");
    assert!(
        store
            .request_cancellation_and_clear_input_if_current(
                &snapshot,
                serde_json::from_value(cancelled.clone())?,
                notification(&cancelled)?,
            )?
            .is_some()
    );
    assert!(
        store.with_session_idle(&session, |_| Ok(())).is_err(),
        "terminal wire state cannot release a still-owned supervisor"
    );
    drop(scope);
    assert_eq!(
        store.with_session_idle(&session, |terminal| Ok(terminal))?,
        1
    );
    Ok(())
}

#[test]
fn admitted_group_binding_survives_later_custody_loss_without_claiming_current_truth() -> TestResult
{
    let _serial = crate::test_serial();
    let session = open("7872305", false, true)?;
    let digests = live_pair(&session)?;
    let store = crate::task_store::LabTaskStore::new()?;
    let id = create_set(&store, "bound-originals", &session, &digests, "all")?;
    let binding = store.observation_binding(&id)?.ok_or("binding missing")?;
    assert_eq!(binding["plans"].as_array().map(Vec::len), Some(2));
    assert!(binding.get("group_complete").is_none());
    assert!(binding["plans"][0].get("goal_truth").is_none());
    with_task_session(&session, |guard| {
        guard.commit_receipts.remove(&digests[1]);
        Ok(())
    })?;
    assert!(observation_set_view(&session, &digests, "all").is_err());
    assert_eq!(store.observation_binding(&id)?, Some(binding.clone()));
    assert_eq!(
        store.task_detail(&session, &id)?["observation_binding"],
        binding
    );
    let refused = crate::task_store::LabTaskStore::new()?;
    assert!(create_set(&refused, "lost-custody", &session, &digests, "all").is_err());
    assert_eq!(refused.session_page(&session, 0)?["total"], 0);
    Ok(())
}
