//! Public close behavior, ownership races and publication refusal.
use super::*;
use crate::agent_facade as facade;
use fastmcp_rust::{
    FinalTask, FinalTaskBase, FinalTaskId, FinalTaskStatus, FinalTaskStatusNotification,
    FinalTaskStatusNotificationParams, FinalTaskStore, FinalTaskTimestamp, FinalTaskWorkDescriptor,
};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn open(selector: &str, shared: bool, durable: bool, scenario: &str) -> TestResult<String> {
    let value = parsed(&facade::fortress_open_session(
        Some(false), Some(selector.to_owned()),
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
            ("designate".into(), "guarded".into()),
        ]),
        Some(60_000), Some(2_000), Some(8_192), Some(4 * 1024 * 1024),
        Some(16_384), Some(64), Some(scenario.to_owned()), Some(shared), Some(durable),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(value["session_id"].as_str().ok_or("session ID missing")?.to_owned())
}

fn close_public(id: &str) -> TestResult<Value> {
    parsed(&facade::fortress_cancel(
        Some(id.to_owned()), Some("close_session".into()), Some("session".into()),
    ))
}

fn query(id: &str, query: Value) -> TestResult<Value> {
    parsed(&facade::fortress_query(
        Some(id.to_owned()),
        Some(json!({"mode":"temporal","request":{"schema":"dfmcp.query/1","query":query}}).to_string()),
    ))
}

fn pending(id: &str) -> TestResult<Value> {
    let plan = parsed(&facade::fortress_plan(
        Some(id.to_owned()), None, Some(true), None, None,
    ))?;
    assert_eq!(plan["ok"], true, "{plan}");
    Ok(plan)
}

#[test]
fn close_releases_review_query_handles_slots_and_stale_arc_authority() -> TestResult {
    let _serial = crate::test_serial();
    let id = open("73901", false, false, "starter_fortress")?;
    let handle = resolve_session(Some(id.clone()))?;
    assert!(facade::has_session_orientation(&id));
    let baseline = query(&id, json!({
        "kind":"capture","key":"before-close","max_game_ticks":100,
        "select":{"kind":"entities","kinds":["unit"],"fields":["alive"]},
    }))?;
    assert_eq!(baseline["ok"], true, "{baseline}");
    let tick = handle.lock().map_err(|_| "session lock")?.adapter.snapshot().tick.0;
    let watch = query(&id, json!({
        "kind":"watch","key":"waiting-at-close",
        "condition":{"op":"field","entity_id":"5001","generation":1,
            "field":dfmcp_intent::effects::STOCK_DRINK_FIELD,"comparison":"ge","value":{"type":"u64","value":100000}},
        "deadline_tick":tick+100,"poll_interval_ticks":1,"stable_observations":2,
    }))?;
    assert_eq!(watch["ok"], true, "{watch}");
    pending(&id)?;
    let result = close_public(&id)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["closed"], true);
    assert_eq!(result["pending_reviews_discarded"], 1);
    assert_eq!(result["process_local_resources"]["process_local_baselines_discarded"], 1);
    assert_eq!(result["process_local_resources"]["watch_handles_released"], 1);
    assert_eq!(result["session_slot_released"], true);
    assert_eq!(result["orientation_slot_released"], true);
    assert_eq!(result["game_effects_cancelled"], false);
    assert!(result["agent_turn"]["references"].as_array().is_some_and(Vec::is_empty));
    assert!(!facade::has_session_orientation(&id));
    assert!(lookup_session_str(&id).is_err());
    assert!(ensure_durable_owner(&handle).is_err());
    assert!(handle.lock().map_err(|_| "session lock")?.grants.is_empty());
    assert!(!with_session(&handle, || false, |_| true));
    assert!(!with_session_read(&handle, |_| false, |_| true));
    assert_eq!(close_public(&id)?, result, "recent close is byte-stable and idempotent");
    let stale = parsed(&facade::fortress_observe(Some(id)))?;
    assert_eq!(stale["ok"], false, "{stale}");
    assert_eq!(stale["error"]["code"], "session_not_found");
    Ok(())
}

#[test]
fn explicit_scope_authority_and_output_bound_refuse_before_resource_release() -> TestResult {
    let _serial = crate::test_serial();
    let id = open("73902", false, false, "empty")?;
    let handle = resolve_session(Some(id.clone()))?;
    pending(&id)?;
    let wrong_scope = parsed(&facade::fortress_cancel(
        Some(id.clone()), Some("close_session".into()), Some("plan".into()),
    ))?;
    assert_eq!(wrong_scope["ok"], false, "{wrong_scope}");
    let original = {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let original = guard.grants.clone();
        guard.grants.retain(|grant| grant.capability != Capability::Observe);
        original
    };
    let denied = close_public(&id)?;
    assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        guard.grants = original;
        guard.budget.max_output_tokens = 1;
    }
    let too_small = close_public(&id)?;
    assert_eq!(too_small["error"]["code"], "budget_exceeded", "{too_small}");
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        assert!(!guard.closed);
        assert!(guard.pending.is_some());
        guard.budget.max_output_tokens = 16_384;
        guard.grants.retain(|grant| grant.capability != Capability::Query);
    }
    assert!(facade::has_session_orientation(&id));
    assert!(recent_receipt(parse_session_id_arg(&id)?).is_err());
    assert_eq!(close_public(&id)?["ok"], true);
    Ok(())
}

#[test]
fn a_small_final_receipt_does_not_bypass_complete_quiet_world_source_admission() -> TestResult {
    let _serial = crate::test_serial();
    let id = open("73910", false, false, "empty")?;
    let handle = resolve_session(Some(id.clone()))?;
    pending(&id)?;
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let mut source = guard.adapter.snapshot().clone();
        let entity = EntityId::new(1);
        source.graph.entities.insert(entity, dfmcp_world::EntityRecord {
            id:entity, generation:1, revision:1, kind:dfmcp_world::EntityKind::Unit,
            label:"large quiet source ".repeat(2_048), fields:BTreeMap::new(),
        });
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.refresh_hash();
        guard.adapter.inject_snapshot(source)?;
        guard.budget.max_bytes = 16 * 1024;
    }
    let refused = close_public(&id)?;
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert!(refused["error"]["message"].as_str()
        .is_some_and(|message| message.contains("source")));
    assert!(facade::has_session_orientation(&id));
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        assert!(!guard.closed);
        assert!(guard.pending.is_some());
        guard.budget.max_bytes = 4 * 1024 * 1024;
    }
    assert_eq!(close_public(&id)?["ok"], true);
    Ok(())
}

/// Proof of designation acceptance completes while excavation still runs.
fn commit_early_dig(id: &str) -> TestResult<ActionId> {
    use dfmcp_core::{MapCoord, StepId};
    use dfmcp_intent::{DigMode, ObligationSpec, derive_step_idempotency_key, effects};
    let handle = resolve_session(Some(id.to_owned()))?;
    let digest = with_session(&handle, || Err(unavailable("fixture unavailable")), |guard| {
        let snapshot = guard.adapter.snapshot();
        let action = Action::DesignateDig {
            area: MapCuboid::new(MapCoord::new(1, 3, 10), MapCoord::new(10, 5, 10))?,
            mode: DigMode::Mine,
        };
        let intent_id = IntentId::new(73903);
        let key = derive_step_idempotency_key(intent_id, snapshot.anchor(), StepId::new(0), &action);
        let proof = Predicate::EntityExists(effects::created_entity_id(&key, 0));
        let intent = Intent {
            id:intent_id, anchor:snapshot.anchor(), summary:"accept a long excavation".into(),
            terminal_condition:proof.clone(), constraints:vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions:vec![RequestedAction {
                action, preconditions:vec![], postconditions:vec![proof.clone()],
                compensation:None, depends_on:vec![],
                obligation:Some(ObligationSpec {
                    terminal:proof, failure:None, deadline_tick:GameTick(snapshot.tick.0+10),
                    poll_interval_ticks:1, stable_for_observations:1,
                }),
            }],
        };
        let plan = StaticPlanner::default().prepare_laboratory(snapshot, &intent, &read_context(guard))?;
        let digest = plan.digest.to_hex();
        guard.pending = Some(PendingPlan {
            plan, digest:digest.clone(),
            source:PlanSource::Actions {
                summary:"accept a long excavation".into(),
                raw:r#"[{"action":{"kind":"designate_dig","min":[1,3,10],"max":[10,5,10],"mode":"mine"}}]"#.into(),
            },
        });
        Ok(digest)
    })?;
    let committed = parsed(&facade::fortress_commit(Some(id.to_owned()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&facade::fortress_wait(Some(id.to_owned()), Some(1)))?;
    assert_eq!(waited["commit_state"], "Verified", "{waited}");
    assert_eq!(waited["work_state"]["state"], "active", "{waited}");
    let action = handle.lock().map_err(|_| "session lock")?.last_action.ok_or("action missing")?;
    Ok(action)
}

#[test]
fn terminal_proof_and_unknown_physical_work_cannot_close_or_erase_original_receipts() -> TestResult {
    let _serial = crate::test_serial();
    let id = open("73903", false, false, "starter_fortress")?;
    let action = commit_early_dig(&id)?;
    let handle = resolve_session(Some(id.clone()))?;
    let before = handle.lock().map_err(|_| "session lock")?.adapter.action_receipt(action)
        .cloned().ok_or("receipt missing")?;
    let refused = close_public(&id)?;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    {
        let guard = handle.lock().map_err(|_| "session lock")?;
        assert_eq!(guard.adapter.action_receipt(action), Some(&before));
        assert!(!guard.closed);
    }
    let original = {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let original = guard.adapter.snapshot().clone();
        let mut unknown = original.clone();
        let step = guard.adapter.action_step(action).ok_or("step missing")?;
        let entity = dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0);
        unknown.graph.entities.get_mut(&entity).ok_or("designation missing")?.fields.remove("status");
        unknown.cursor = unknown.cursor.checked_next().ok_or("cursor exhausted")?;
        unknown.refresh_hash();
        guard.adapter.inject_snapshot(unknown)?;
        original
    };
    assert_eq!(close_public(&id)?["ok"], false);
    {
        let mut guard = handle.lock().map_err(|_| "session lock")?;
        let mut restored = original;
        restored.cursor = guard.adapter.snapshot().cursor.checked_next().ok_or("cursor exhausted")?;
        restored.refresh_hash();
        guard.adapter.inject_snapshot(restored)?;
    }
    let drained = parsed(&facade::fortress_cancel(
        Some(id.clone()), Some("stop_future_steps".into()), Some("session".into()),
    ))?;
    assert_eq!(drained["drain_progress"]["quiescent"], true, "{drained}");
    assert!(!handle.lock().map_err(|_| "session lock")?.closed,
        "ordinary scope=session drains and preserves the session");
    assert_eq!(handle.lock().map_err(|_| "session lock")?.adapter.action_receipt(action), Some(&before));
    assert_eq!(close_public(&id)?["ok"], true);
    Ok(())
}

#[test]
fn shared_member_close_preserves_other_work_and_releases_last_world_slot() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open("73904", true, false, "starter_fortress")?;
    let departing = open("73904", true, false, "starter_fortress")?;
    let action = commit_early_dig(&owner)?;
    let handle = resolve_session(Some(owner.clone()))?;
    let departing_id = parse_session_id_arg(&departing)?;
    with_session(&handle, || (), |guard| {
        guard.leases.unpause_consent.insert(departing_id);
    });
    let result = close_public(&departing)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["shared_members_remaining"], 1);
    assert_eq!(result["shared_world_released"], false);
    assert_eq!(result["finalize_certificate"]["scope"], "owned_session_actions");
    with_session_read(&handle, |_| panic!("owner unavailable"), |guard| {
        assert!(!guard.leases.unpause_consent.contains(&departing_id));
        assert!(!action_fully_drained(&guard.adapter, action));
    });
    assert_eq!(close_public(&owner)?["ok"], false, "last owner cannot orphan work");
    let drained = parsed(&facade::fortress_cancel(
        Some(owner.clone()), Some("stop_future_steps".into()), Some("session".into()),
    ))?;
    assert_eq!(drained["drain_progress"]["quiescent"], true, "{drained}");
    assert_eq!(close_public(&owner)?["shared_world_released"], true);
    assert!(!SHARED_WORLDS.lock().map_err(|_| "world registry lock")?
        .contains_key(&FortressId::new(73904)));
    let replacement = open("73904", true, false, "empty")?;
    assert_eq!(close_public(&replacement)?["ok"], true);
    Ok(())
}

#[test]
fn a_quiet_verified_prerequisite_does_not_hide_a_deferred_successor() -> TestResult {
    use dfmcp_core::StepId;
    use dfmcp_intent::ObligationSpec;
    let _serial = crate::test_serial();
    let id = open("73909", false, false, "empty")?;
    let handle = resolve_session(Some(id.clone()))?;
    let digest = with_session(&handle, || Err(unavailable("fixture unavailable")), |guard| {
        let source = guard.adapter.snapshot();
        let intent = Intent {
            id:IntentId::new(73909), anchor:source.anchor(), summary:"deferred close fixture".into(),
            terminal_condition:Predicate::Paused(true),
            constraints:vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions:vec![
                RequestedAction {
                    action:Action::Pause { paused:false },
                    preconditions:vec![], postconditions:vec![Predicate::Paused(false)],
                    compensation:None, depends_on:vec![],
                    obligation:Some(ObligationSpec {
                        terminal:Predicate::Paused(false), failure:None,
                        deadline_tick:GameTick(source.tick.0+100), poll_interval_ticks:1,
                        stable_for_observations:2,
                    }),
                },
                RequestedAction {
                    action:Action::Pause { paused:true },
                    preconditions:vec![], postconditions:vec![Predicate::Paused(true)],
                    compensation:None, depends_on:vec![StepId::new(0)], obligation:None,
                },
            ],
        };
        let plan = StaticPlanner::default().prepare_laboratory(source, &intent, &read_context(guard))?;
        let digest = plan.digest.to_hex();
        guard.pending = Some(PendingPlan {
            plan, digest:digest.clone(),
            source:PlanSource::Actions {
                summary:"deferred close fixture".into(),
                raw:r#"[{"action":{"kind":"pause","paused":false}},{"action":{"kind":"pause","paused":true},"depends_on":[0]}]"#.into(),
            },
        });
        Ok(digest)
    })?;
    let committed = parsed(&facade::fortress_commit(Some(id.clone()), digest))?;
    assert_eq!(committed["ok"], true, "{committed}");
    with_session(&handle, || Err(unavailable("fixture unavailable")), |guard| {
        let first = guard.last_plan_actions[0];
        let second = guard.last_plan_actions[1];
        // Registration establishes a cadence floor, not a positive sample.
        // Poll only the prerequisite at two fresh eligible ticks; ordinary
        // fortress.wait would also dispatch its now-ready successor.
        for _ in 0..2 {
            guard.adapter.advance_ticks(1)?;
            let current = read_context(guard);
            guard.adapter.poll_action(first, &current)?;
        }
        assert!(action_fully_drained(&guard.adapter, first));
        assert_eq!(guard.adapter.action_receipt(second).map(|r| r.state), Some(CommitState::Prepared));
        assert!(guard.adapter.action_work_state(second)?.is_quiescent());
        Ok(())
    })?;
    let refused = close_public(&id)?;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert!(handle.lock().map_err(|_| "session lock")?.open_actions.len() >= 1);
    let drained = parsed(&facade::fortress_cancel(
        Some(id.clone()), Some("stop_future_steps".into()), Some("session".into()),
    ))?;
    assert_eq!(drained["drain_progress"]["quiescent"], true, "{drained}");
    let closed = close_public(&id)?;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(closed["finalize_certificate"]["actions_total"], 2);
    assert_eq!(closed["finalize_certificate"]["census_digest"].as_str().map(str::len), Some(64));
    Ok(())
}

fn task(id: &str) -> TestResult<FinalTask> {
    let timestamp = FinalTaskTimestamp::parse("2026-07-28T12:00:00.000Z")?;
    Ok(FinalTask::Working(FinalTaskBase {
        task_id:FinalTaskId::parse(id)?, status:FinalTaskStatus::Working,
        status_message:None, created_at:timestamp.clone(), last_updated_at:timestamp,
        ttl_ms:None, poll_interval_ms:None,
    }))
}

fn notification(task: &FinalTask) -> FinalTaskStatusNotification {
    FinalTaskStatusNotification::new(FinalTaskStatusNotificationParams {
        task:task.clone(), meta:None, additional:BTreeMap::new(),
    })
}

fn descriptor(id: &str) -> TestResult<FinalTaskWorkDescriptor> {
    Ok(FinalTaskWorkDescriptor::new(json!({
        "schema":"dfmcp.lab-plan-task-work/1","session_id":id,"plan_digest":"b".repeat(64),
    }))?)
}

#[test]
fn task_close_fence_covers_queued_terminal_running_and_delayed_create() -> TestResult {
    let _serial = crate::test_serial();
    let id = open("73905", false, false, "empty")?;
    let handle = resolve_session(Some(id.clone()))?;
    let store = crate::task_store::LabTaskStore::new()?;
    let initial = task("close-monitor")?;
    let task_id = initial.base().task_id.clone();
    store.create_task_with_work(initial.clone(), notification(&initial), descriptor(&id)?)?;
    assert!(store.with_session_idle(&id, |_| Ok(())).is_err());
    let monitor = store.monitor_scope(&task_id)?;
    let mut base = initial.base().clone();
    base.status = FinalTaskStatus::Cancelled;
    let terminal = FinalTask::Cancelled(base);
    store.replace_task(terminal.clone(), notification(&terminal))?;
    assert!(store.with_session_idle(&id, |_| Ok(())).is_err(),
        "terminal wire state cannot hide a still-owned supervisor future");
    drop(monitor);
    let admission = LAB_ADMISSION.write().map_err(|_| "admission lock")?;
    let delayed_store = store.clone();
    let delayed_id = id.clone();
    let (sender, receiver) = std::sync::mpsc::channel();
    let mut worker = None;
    let output = store.with_session_idle(&id, |retained| {
        assert_eq!(retained, 1);
        let delayed = task("delayed-after-close").map_err(|e| unavailable(&e.to_string()))?;
        let work = descriptor(&delayed_id).map_err(|e| unavailable(&e.to_string()))?;
        worker = Some(std::thread::spawn(move || {
            let _ = sender.send(());
            delayed_store.create_task_with_work(delayed.clone(), notification(&delayed), work)
        }));
        receiver.recv_timeout(std::time::Duration::from_secs(2))
            .map_err(|_| unavailable("delayed task fixture did not start"))?;
        close_admitted(&handle, retained)
    })?;
    drop(admission);
    assert_eq!(parsed(&output)?["closed"], true);
    assert!(worker.ok_or("worker missing")?.join().map_err(|_| "worker panicked")?.is_err(),
        "creation validated before close must recheck registration after the custody fence");
    assert!(store.get_task(&task_id).is_err(),
        "retained terminal Task history confers no stale session authority");
    Ok(())
}

#[test]
fn durable_close_publishes_head_and_reopen_recovers_with_new_authority() -> TestResult {
    let _serial = crate::test_serial();
    let _durable = serialized_durable_tests();
    let root = std::env::temp_dir().join(format!("dfmcp-session-close-{}", std::process::id()));
    if root.exists() {
        std::fs::remove_dir_all(&root)?;
    }
    simulate_durable_restart(Some(root.clone()));
    let id = open("73906", false, true, "empty")?;
    let old = resolve_session(Some(id.clone()))?;
    let anchor = old.lock().map_err(|_| "session lock")?.adapter.snapshot().anchor();
    // Leave an unpublished but valid local frontier, then refuse the final save.
    {
        let mut guard = old.lock().map_err(|_| "session lock")?;
        let mut source = guard.adapter.snapshot().clone();
        source.paused = !source.paused;
        source.cursor = source.cursor.checked_next().ok_or("cursor exhausted")?;
        source.refresh_hash();
        guard.adapter.inject_snapshot(source)?;
    }
    with_durable_store(|store| { store.set_append_budget(Some(0)); Ok(()) })?;
    let refused = close_public(&id)?;
    assert_eq!(refused["error"]["code"], "adapter_unavailable", "{refused}");
    assert_eq!(durable_lab().owners.get(&FortressId::new(73906)), Some(&parse_session_id_arg(&id)?));
    assert!(!old.lock().map_err(|_| "session lock")?.closed);
    assert!(facade::has_session_orientation(&id));
    with_durable_store(|store| { store.set_append_budget(None); Ok(()) })?;
    let closed = close_public(&id)?;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(closed["durable_head_published"], true);
    assert_eq!(closed["durable_private_owner_released"], true);
    assert!(!durable_lab().owners.contains_key(&FortressId::new(73906)));
    let next = open("73906", false, true, "empty")?;
    let fresh = resolve_session(Some(next.clone()))?;
    assert!(fresh.lock().map_err(|_| "session lock")?.adapter.snapshot().cursor.epoch > anchor.cursor.epoch);
    assert!(!with_session(&old, || false, |_| true));
    assert_eq!(close_public(&next)?["ok"], true);

    // The durable admission boundary precedes adapter commit. Reproduce the
    // retained source + review after an adapter refusal with no action handle.
    let admitted_id = open("73908", false, true, "empty")?;
    let review = pending(&admitted_id)?;
    let admitted = resolve_session(Some(admitted_id.clone()))?;
    {
        let mut guard = admitted.lock().map_err(|_| "session lock")?;
        let pending = guard.pending.as_ref().ok_or("pending review missing")?;
        let plan = pending.plan.clone();
        let source = pending.source.clone();
        with_durable_store(|store| store.persist_objective_commit(
            guard.adapter.snapshot(), plan.digest, plan.intent_id.get(), source.durable(),
            guard.session_id, &[],
        ))?;
        objectives::install_objective(&mut guard, &plan, source, &[], None);
    }
    let refused = close_public(&admitted_id)?;
    assert_eq!(refused["error"]["code"], "conflict", "{refused}");
    assert!(admitted.lock().map_err(|_| "session lock")?.pending.is_some());
    with_durable_store(|store| {
        assert_eq!(store.commits(FortressId::new(73908)).count(), 1);
        Ok(())
    })?;
    let committed = parsed(&facade::fortress_commit(
        Some(admitted_id.clone()), review["plan_digest"].as_str().ok_or("plan digest")?.into(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(close_public(&admitted_id)?["ok"], true);

    simulate_durable_restart(None);
    std::fs::remove_dir_all(root)?;
    Ok(())
}
