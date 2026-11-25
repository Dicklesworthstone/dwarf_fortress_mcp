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

fn request_original_cancellation(
    store: &LabTaskStore,
    id: &FinalTaskId,
    entry: usize,
) -> TestResult {
    match entry {
        0 => store.request_cancellation(id)?,
        1 => {
            let current = store.get_task_snapshot(id)?.ok_or("Task missing")?;
            assert!(store.request_cancellation_if_current(&current)?);
        }
        2 => cancel_transport_monitor(store, id)?,
        _ => return Err("unknown cancellation entry".into()),
    }
    Ok(())
}

#[test]
fn cancellation_after_a_supervisor_check_fences_late_original_dispatch() -> TestResult {
    let _serial = crate::test_serial();
    for entry in 0..3 {
        let session = open(&(7_872_460 + entry).to_string(), false, true)?;
        let digest = plan(&session)?;
        let store = LabTaskStore::new()?;
        let id = create(
            &store,
            &format!("cancel-before-dispatch-{entry}"),
            &session,
            &digest,
            COMMIT_SCHEMA,
        )?;
        let before = engine_footprint(&session)?;
        // Deterministic ordering: the supervisor passed its original check,
        // then cancellation completed before its dispatch admission.
        assert!(!store.is_cancellation_requested(&id)?);
        request_original_cancellation(&store, &id, entry)?;
        let finalized = store.task_detail(&session, &id)?["progress"].clone();
        assert_eq!(finalized["stage"], "finalized");
        assert_eq!(
            finalized["finalization"]["finalize_certificate"]["effects_dispatched"],
            false
        );
        let mut invoked = false;
        assert!(
            store
                .with_original_dispatch(&id, |_| {
                    invoked = true;
                    Ok(())
                })?
                .is_none()
        );
        assert!(!invoked, "accepted cancellation must fence the effect callback");
        assert!(store.mark_dispatch_started(&id).is_err());

        // A stale supervisor observation must not replace its cancellation
        // certificate after the transport intent has already been consumed.
        store.record_progress(&id, json!({"stage":"observed","obligation":null}))?;
        assert_eq!(store.task_detail(&session, &id)?["progress"], finalized);
        assert_eq!(engine_footprint(&session)?, before);
    }
    Ok(())
}

#[test]
fn dispatch_reservation_without_a_receipt_cannot_certify_queued_cancellation() -> TestResult {
    let _serial = crate::test_serial();
    for entry in 0..3 {
        let session = open(&(7_872_470 + entry).to_string(), false, true)?;
        let digest = plan(&session)?;
        let store = LabTaskStore::new()?;
        let id = create(
            &store,
            &format!("reserved-before-cancel-{entry}"),
            &session,
            &digest,
            COMMIT_SCHEMA,
        )?;
        let before = engine_footprint(&session)?;
        // Reproduce the old gap between reserving dispatch and publishing
        // dispatch_started progress, including recovery before a receipt exists.
        assert!(!store.mark_dispatch_started(&id)?);
        assert_eq!(store.task_detail(&session, &id)?["progress"]["stage"], "queued");
        let refusal = request_original_cancellation(&store, &id, entry)
            .expect_err("a reserved dispatch cannot receive a no-effect certificate");
        assert!(
            refusal.to_string().contains("without a retained receipt"),
            "{refusal}"
        );
        assert!(!store.is_cancellation_requested(&id)?);
        let unresolved = store.task_detail(&session, &id)?;
        assert!(unresolved["progress"]["finalization"]["finalize_certificate"].is_null());
        assert_eq!(engine_footprint(&session)?, before);

        // Once the actual original receipt arrives, cancellation drains those
        // exact actions instead of fabricating the zero-action queued proof.
        let committed = parsed(&crate::agent_facade::fortress_commit(
            Some(session.clone()),
            digest.clone(),
        ))?;
        assert_eq!(committed["ok"], true, "{committed}");
        request_original_cancellation(&store, &id, entry)?;
        let cancelled = store.task_detail(&session, &id)?;
        assert_eq!(cancelled["progress"]["stage"], "finalized");
        assert!(
            cancelled["progress"]["finalization"]["drain_progress"]["actions_total"]
                .as_u64()
                .is_some_and(|count| count > 0)
        );
        assert_eq!(
            cancelled["progress"]["finalization"]["drain_progress"]["quiescent"],
            true
        );
        assert!(
            cancelled["progress"]["finalization"]["finalize_certificate"]["effects_dispatched"]
                .is_null(),
            "a real original drain must retain its action proof"
        );
        assert!(store.mark_dispatch_started(&id).is_err());
    }
    Ok(())
}

#[test]
fn execution_expiry_before_dispatch_refuses_the_late_effect_callback() -> TestResult {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    let _serial = crate::test_serial();
    let session = open("7872480", false, true)?;
    let digest = plan(&session)?;
    let elapsed = Arc::new(AtomicU64::new(0));
    let origin = Instant::now();
    let clock_elapsed = elapsed.clone();
    let store = LabTaskStore::with_test_clock(Arc::new(move || {
        origin + Duration::from_secs(clock_elapsed.load(Ordering::Acquire))
    }))?;
    let id = create(
        &store,
        "expiry-before-dispatch",
        &session,
        &digest,
        COMMIT_SCHEMA,
    )?;
    let expected = store.get_task_snapshot(&id)?.ok_or("Task missing")?;
    let handoff = store
        .take_initial_work_handoff_for_owner_if_current(&expected, "expiry-before-dispatch-owner")?
        .ok_or("initial original handoff missing")?;
    assert!(
        store
            .begin_handoff_dispatch_for_owner_if_current(
                &id,
                expected.generation(),
                "expiry-before-dispatch-owner",
            )?
            .is_some()
    );
    let scope = store.monitor_scope(&id)?;
    let before = engine_footprint(&session)?;
    assert!(!store.is_cancellation_requested(&id)?);
    elapsed.store(3_601, Ordering::Release);
    assert!(store.expire_task_execution_if_current(&expected)?.is_some());

    let mut invoked = false;
    assert!(
        store
            .with_original_dispatch(&id, |_| {
                invoked = true;
                Ok(())
            })
            .is_err()
    );
    assert!(!invoked, "execution expiry must close original dispatch admission");
    assert!(store.mark_dispatch_started(&id).is_err());
    assert!(matches!(store.get_task(&id)?, Some(FinalTask::Failed { .. })));
    let failure = store.task_detail(&session, &id)?["execution_failure"].clone();
    assert_eq!(failure["dispatch_started"], false);
    assert_eq!(failure["effect_cancellation_requested"], false);
    assert_eq!(failure["physical_quiescence_certified"], false);
    assert_eq!(engine_footprint(&session)?, before);
    drop(handoff);
    drop(scope);
    Ok(())
}

#[test]
fn failed_cancellation_publication_rechecks_a_later_foreground_commit() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872481", false, true)?;
    let digest = plan(&session)?;
    let store = LabTaskStore::new()?;
    let id = create(
        &store,
        "unpublished-cancellation",
        &session,
        &digest,
        COMMIT_SCHEMA,
    )?;
    let before = engine_footprint(&session)?;
    let current = store.get_task_snapshot(&id)?.ok_or("Task missing")?;
    let cancelled = task_wire(id.as_str(), "cancelled");
    // The engine can finish its queued proof before the upstream handoff CAS
    // discovers a missing owner. Use that real rejection, without a test hook.
    assert!(
        store
            .replace_task_and_clear_input_for_handoff_if_current(
                &current,
                "missing-cancellation-owner",
                1,
                true,
                serde_json::from_value(cancelled.clone())?,
                notification(&cancelled)?,
            )?
            .is_none()
    );
    assert!(matches!(store.get_task(&id)?, Some(FinalTask::Working(_))));
    assert!(!store.is_cancellation_requested(&id)?);
    assert!(store.mark_dispatch_started(&id).is_err());
    let refused = store.task_detail(&session, &id)?["progress"].clone();
    assert_eq!(refused["stage"], "cancellation_reconciliation_required");
    assert_eq!(refused["physical_quiescence_certified"], false);
    assert!(refused["physical_quiescent"].is_null());
    let historical = refused["historical_finalization"].clone();
    assert_eq!(
        historical["finalization"]["finalize_certificate"]["effects_dispatched"],
        false
    );
    assert_eq!(engine_footprint(&session)?, before);

    // A separate explicit foreground request may still commit the pending plan.
    // The old failed Task cancellation must never certify that new work as quiet.
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(session.clone()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let grants = with_task_session(&session, |owner| {
        let grants = owner.grants.clone();
        owner
            .grants
            .retain(|grant| grant.capability != Capability::Query);
        Ok(grants)
    })?;
    assert!(cancel_transport_monitor(&store, &id).is_err());
    let denied = store.task_detail(&session, &id)?["progress"].clone();
    assert_eq!(denied["stage"], "cancellation_reconciliation_required");
    assert_eq!(denied["physical_quiescence_certified"], false);
    assert!(denied["physical_quiescent"].is_null());
    assert_eq!(denied["historical_finalization"], historical);
    assert!(!store.is_cancellation_requested(&id)?);
    with_task_session(&session, |owner| {
        owner.grants = grants;
        Ok(())
    })?;

    cancel_transport_monitor(&store, &id)?;
    let finalized = store.task_detail(&session, &id)?["progress"].clone();
    assert_eq!(finalized["stage"], "finalized");
    assert_eq!(finalized["finalization"]["plan_digest"], digest);
    assert!(
        finalized["finalization"]["drain_progress"]["actions_total"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert_eq!(finalized["finalization"]["drain_progress"]["quiescent"], true);
    assert_ne!(
        finalized["finalization"]["finalize_certificate"]["digest"],
        historical["finalization"]["finalize_certificate"]["digest"]
    );
    assert!(
        finalized["finalization"]["finalize_certificate"]["effects_dispatched"].is_null()
    );
    store.record_progress(&id, json!({"stage":"observed","obligation":null}))?;
    assert_eq!(store.task_detail(&session, &id)?["progress"], finalized);
    Ok(())
}

#[test]
fn task_gate_preserves_missing_record_compare_and_swap_results() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7872482", false, true)?;
    let digest = plan(&session)?;
    let donor = LabTaskStore::new()?;
    let id = create(
        &donor,
        "missing-gated-task",
        &session,
        &digest,
        COMMIT_SCHEMA,
    )?;
    let expected = donor.get_task_snapshot(&id)?.ok_or("Task missing")?;
    let store = LabTaskStore::new()?;
    let before = engine_footprint(&session)?;
    assert!(!store.request_cancellation_if_current(&expected)?);
    let cancelled = task_wire(id.as_str(), "cancelled");
    assert!(
        store
            .request_cancellation_and_clear_input_if_current(
                &expected,
                serde_json::from_value(cancelled.clone())?,
                notification(&cancelled)?,
            )?
            .is_none()
    );
    assert!(
        store
            .replace_task_and_clear_input_for_handoff_if_current(
                &expected,
                "missing-task-owner",
                1,
                true,
                serde_json::from_value(cancelled.clone())?,
                notification(&cancelled)?,
            )?
            .is_none()
    );
    assert!(store.expire_task_execution_if_current(&expected)?.is_none());
    assert_eq!(engine_footprint(&session)?, before);
    Ok(())
}
