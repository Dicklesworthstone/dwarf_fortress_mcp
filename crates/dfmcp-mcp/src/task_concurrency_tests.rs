//! Bounded concurrent Tasks retain independent original and session ownership.
//! Regression scope: df-fastmcp-conformance-5pj.3.
use super::*;
use crate::task_store::{LabTaskStore, MAX_ACTIVE_TASKS};

const COMMIT_SCHEMA: &str = "dfmcp.lab-plan-task-work/1";
const SET_SCHEMA: &str = "dfmcp.lab-original-set-observer-task-work/1";

fn engine_footprint(session: &str) -> TestResult<String> {
    Ok(with_task_read(session, |owner| {
        Ok(format!(
            "{:?}|{}|{:?}|{:?}|{:?}|{:?}",
            owner.adapter,
            owner.next_request_id,
            owner.commit_receipts,
            owner.open_actions,
            owner.last_plan_actions,
            owner
                .pending
                .as_ref()
                .map(|pending| (&pending.digest, &pending.plan, &pending.source,)),
        ))
    })?)
}

fn create_set_task(
    store: &LabTaskStore,
    id: &str,
    session: &str,
    digest: &str,
) -> TestResult<FinalTaskId> {
    let task = task_wire(id, "working");
    store.create_task_with_work(
        serde_json::from_value(task.clone())?,
        notification(&task)?,
        FinalTaskWorkDescriptor::new(json!({
            "schema":SET_SCHEMA,"session_id":session,
            "plan_digests":[digest],"plan_policy":"all",
        }))?,
    )?;
    Ok(FinalTaskId::parse(id)?)
}

fn cancel_transport_monitor(store: &LabTaskStore, id: &FinalTaskId) -> TestResult {
    let current = store.get_task_snapshot(id)?.ok_or("Task missing")?;
    let cancelled = task_wire(id.as_str(), "cancelled");
    assert!(
        store
            .request_cancellation_and_clear_input_if_current(
                &current,
                serde_json::from_value(cancelled.clone())?,
                notification(&cancelled)?,
            )?
            .is_some()
    );
    Ok(())
}

#[test]
fn mixed_monitors_share_eight_slots_and_terminal_supervisors_keep_their_capacity() -> TestResult {
    let _serial = crate::test_serial();
    assert_eq!(MAX_ACTIVE_TASKS, 8);
    let store = LabTaskStore::new()?;
    let mut admitted = Vec::new();
    for index in 0..MAX_ACTIVE_TASKS {
        let session = open(&(7_872_400 + index).to_string(), false, true)?;
        let digest = if index % 3 == 0 {
            plan(&session)?
        } else {
            commit(&session)?
        };
        let before = engine_footprint(&session)?;
        let name = format!("concurrent-original-{index}");
        let id = match index % 3 {
            0 => create(&store, &name, &session, &digest, COMMIT_SCHEMA)?,
            1 => create(&store, &name, &session, &digest, OBSERVER_SCHEMA)?,
            _ => create_set_task(&store, &name, &session, &digest)?,
        };
        assert_eq!(engine_footprint(&session)?, before);
        let page = store.session_page(&session, 0)?;
        assert_eq!(page["total"], 1);
        assert_eq!(page["tasks"][0]["task_id"], id.as_str());
        assert_eq!(page["active_coverage_complete"], true);
        admitted.push((session, digest, id));
    }

    let ninth = open("7872410", false, true)?;
    let pending = plan(&ninth)?;
    let untouched = engine_footprint(&ninth)?;
    assert!(create(&store, "capacity-refused", &ninth, &pending, COMMIT_SCHEMA).is_err());
    assert_eq!(store.session_page(&ninth, 0)?["total"], 0);
    assert_eq!(engine_footprint(&ninth)?, untouched);

    // A cancellation can publish terminal wire state before its owning future
    // returns. That interval still occupies one execution and admission slot.
    let first = &admitted[0];
    let first_before = engine_footprint(&first.0)?;
    let scope = store.monitor_scope(&first.2)?;
    cancel_transport_monitor(&store, &first.2)?;
    assert!(matches!(
        store.get_task(&first.2)?,
        Some(FinalTask::Cancelled(_))
    ));
    assert_eq!(engine_footprint(&first.0)?, first_before);
    assert!(
        create(
            &store,
            "terminal-tail-refused",
            &ninth,
            &pending,
            COMMIT_SCHEMA
        )
        .is_err()
    );
    assert_eq!(engine_footprint(&ninth)?, untouched);
    assert!(store.with_session_idle(&first.0, |_| Ok(())).is_err());

    drop(scope);
    assert_eq!(store.with_session_idle(&first.0, |count| Ok(count))?, 1);
    let replacement = create(
        &store,
        "replacement-after-return",
        &ninth,
        &pending,
        COMMIT_SCHEMA,
    )?;
    assert_eq!(engine_footprint(&ninth)?, untouched);
    assert!(matches!(
        store.get_task(&replacement)?,
        Some(FinalTask::Working(_))
    ));
    for (session, _, id) in admitted.iter().skip(1) {
        assert!(matches!(store.get_task(id)?, Some(FinalTask::Working(_))));
        assert!(store.with_session_idle(session, |_| Ok(())).is_err());
    }

    let tenth = open("7872411", false, true)?;
    let last_pending = plan(&tenth)?;
    let last_before = engine_footprint(&tenth)?;
    assert!(
        create(
            &store,
            "only-one-slot-returned",
            &tenth,
            &last_pending,
            COMMIT_SCHEMA
        )
        .is_err()
    );
    assert_eq!(engine_footprint(&tenth)?, last_before);
    assert_eq!(store.session_page(&tenth, 0)?["total"], 0);
    Ok(())
}

#[test]
fn the_per_session_slot_uses_canonical_identity_until_its_supervisor_returns() -> TestResult {
    let _serial = crate::test_serial();
    // Session IDs are counters; at most sixteen real openings are needed to
    // exercise a nontrivial uppercase spelling without fabricating a handle.
    let mut selected = None;
    for index in 0..16 {
        let session = open(&(7_872_420 + index).to_string(), false, true)?;
        if session.to_ascii_uppercase() != session {
            selected = Some(session);
            break;
        }
        let closed = parsed(&crate::agent_facade::fortress_cancel(
            Some(session),
            Some("close_session".to_owned()),
            Some("session".to_owned()),
        ))?;
        assert_eq!(closed["closed"], true, "{closed}");
    }
    let session = selected.ok_or("no case-bearing canonical session ID")?;
    let digest = plan(&session)?;
    let store = LabTaskStore::new()?;
    let id = create(&store, "canonical-owner", &session, &digest, COMMIT_SCHEMA)?;
    let scope = store.monitor_scope(&id)?;
    let uppercase = session.to_ascii_uppercase();
    let before = engine_footprint(&session)?;
    assert!(
        create(
            &store,
            "same-owner-uppercase",
            &uppercase,
            &digest,
            COMMIT_SCHEMA
        )
        .is_err()
    );
    cancel_transport_monitor(&store, &id)?;
    assert!(
        create(
            &store,
            "same-owner-terminal-tail",
            &uppercase,
            &digest,
            COMMIT_SCHEMA
        )
        .is_err()
    );
    assert_eq!(engine_footprint(&session)?, before);
    assert_eq!(store.session_page(&session, 0)?["total"], 1);

    drop(scope);
    let replacement = create(
        &store,
        "same-owner-after-return",
        &uppercase,
        &digest,
        COMMIT_SCHEMA,
    )?;
    assert_eq!(engine_footprint(&session)?, before);
    assert_eq!(store.session_page(&session, 0)?["total"], 2);
    assert_eq!(
        store.task_detail(&session, &replacement)?["plan_digest"],
        digest
    );
    assert!(store.with_session_idle(&session, |_| Ok(())).is_err());
    Ok(())
}

#[test]
fn cancelling_an_observer_releases_only_its_session_and_preserves_a_running_sibling() -> TestResult
{
    let _serial = crate::test_serial();
    let first = open("7872440", false, true)?;
    let first_digest = commit(&first)?;
    let second = open("7872441", false, true)?;
    let second_digest = commit(&second)?;
    let store = LabTaskStore::new()?;
    let observer = create(
        &store,
        "independent-observer",
        &first,
        &first_digest,
        OBSERVER_SCHEMA,
    )?;
    let original = create(
        &store,
        "independent-commit",
        &second,
        &second_digest,
        COMMIT_SCHEMA,
    )?;
    let observer_scope = store.monitor_scope(&observer)?;
    let original_scope = store.monitor_scope(&original)?;
    let first_before = engine_footprint(&first)?;
    let second_before = engine_footprint(&second)?;
    let sibling_before = store.task_detail(&second, &original)?;

    cancel_transport_monitor(&store, &observer)?;
    assert_eq!(engine_footprint(&first)?, first_before);
    assert_eq!(engine_footprint(&second)?, second_before);
    assert_eq!(store.task_detail(&second, &original)?, sibling_before);
    let stopped = store.task_detail(&first, &observer)?;
    assert_eq!(stopped["progress"]["stage"], "monitor_finalized");
    assert_eq!(stopped["progress"]["physical_work_changed"], false);
    assert_eq!(stopped["progress"]["effect_cancellation_requested"], false);
    assert_eq!(stopped["progress"]["physical_quiescence_certified"], false);
    assert!(store.with_session_idle(&first, |_| Ok(())).is_err());
    assert!(store.with_session_idle(&second, |_| Ok(())).is_err());

    drop(observer_scope);
    assert_eq!(store.with_session_idle(&first, |count| Ok(count))?, 1);
    assert!(store.with_session_idle(&second, |_| Ok(())).is_err());
    assert!(matches!(
        store.get_task(&original)?,
        Some(FinalTask::Working(_))
    ));
    assert_eq!(
        observation_view(&first, &first_digest)?.status,
        McpTaskStatus::Working
    );
    finish(&second, &second_digest)?;
    assert_eq!(
        view(&second, &second_digest)?.status,
        McpTaskStatus::Completed
    );
    assert_eq!(engine_footprint(&first)?, first_before);
    drop(original_scope);
    assert!(
        store.with_session_idle(&second, |_| Ok(())).is_err(),
        "an upstream Working record retains ownership after a supervisor returns"
    );
    Ok(())
}

#[test]
fn execution_expiry_preserves_original_work_and_retained_terminal_evidence() -> TestResult {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    let _serial = crate::test_serial();
    for completed in [false, true] {
        let session = open(if completed { "7872451" } else { "7872450" }, false, true)?;
        let digest = commit(&session)?;
        if completed {
            finish(&session, &digest)?;
        }
        let observed = view(&session, &digest)?;
        assert_eq!(
            observed.status,
            if completed {
                McpTaskStatus::Completed
            } else {
                McpTaskStatus::Working
            }
        );
        let elapsed = Arc::new(AtomicU64::new(0));
        let origin = Instant::now();
        let clock_elapsed = elapsed.clone();
        let store = LabTaskStore::with_test_clock(Arc::new(move || {
            origin + Duration::from_secs(clock_elapsed.load(Ordering::Acquire))
        }))?;
        let id = create(
            &store,
            "expiring-original",
            &session,
            &digest,
            COMMIT_SCHEMA,
        )?;
        let expected = store.get_task_snapshot(&id)?.ok_or("Task missing")?;
        let handoff = store
            .take_initial_work_handoff_for_owner_if_current(&expected, "concurrency-expiry-owner")?
            .ok_or("initial original handoff missing")?;
        let fence = store.begin_handoff_dispatch_for_owner_if_current(
            &id,
            expected.generation(),
            "concurrency-expiry-owner",
        )?;
        assert!(fence.is_some());
        let scope = store.monitor_scope(&id)?;
        store.record_progress(
            &id,
            json!({"stage":"observed","obligation":observed.payload}),
        )?;
        if completed {
            store.retain_final_evidence(&id, &observed.payload, None)?;
        }
        let retained = store.terminal_evidence(&id)?;
        let before = engine_footprint(&session)?;
        assert!(store.expire_task_execution_if_current(&expected)?.is_none());
        assert!(store.task_detail(&session, &id)?["execution_failure"].is_null());

        elapsed.store(3_601, Ordering::Release);
        assert!(store.expire_task_execution_if_current(&expected)?.is_some());
        assert!(matches!(
            store.get_task(&id)?,
            Some(FinalTask::Failed { .. })
        ));
        assert_eq!(engine_footprint(&session)?, before);
        assert_eq!(store.terminal_evidence(&id)?, retained);
        let detail = store.task_detail(&session, &id)?;
        assert_eq!(detail["plan_digest"], digest);
        let failure = &detail["execution_failure"];
        assert_eq!(failure["effect_cancellation_requested"], false);
        assert_eq!(failure["original_receipts_changed"], false);
        assert_eq!(failure["physical_quiescence_certified"], false);
        assert!(failure["physical_quiescent"].is_null());
        assert_eq!(failure["blind_retry_allowed"], false);
        assert!(store.with_session_idle(&session, |_| Ok(())).is_err());
        assert!(store.expire_task_execution_if_current(&expected)?.is_none());
        assert_eq!(
            store.task_detail(&session, &id)?["execution_failure"],
            *failure
        );
        assert_eq!(engine_footprint(&session)?, before);

        drop(handoff);
        drop(scope);
        assert_eq!(store.with_session_idle(&session, |count| Ok(count))?, 1);
    }
    Ok(())
}
