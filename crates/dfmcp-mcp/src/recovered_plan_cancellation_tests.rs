//! Real facade stops of durable originals, with no restored dispatch handles.
use super::*;
use crate::{agent_facade as facade, server, server::CarriedStep};
use dfmcp_core::{SessionId, WorkBudget};
use dfmcp_lab::durable::DurableLabStore;
use dfmcp_world::{Predicate, WorldSnapshot};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[path = "recovered_scope_cancellation_tests.rs"]
mod scopes;

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-recovered-stop-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
    fn restart(&self) {
        server::simulate_durable_restart(Some(self.0.clone()));
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
fn field(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}
fn open(selector: &str) -> TestResult<String> {
    let opened = payload(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "guarded"),
                ("designate", "guarded"),
                ("configure_production", "reversible"),
                ("control_clock", "reversible"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(60_000),
        Some(2_000),
        None,
        None,
        Some(16_384),
        Some(64),
        Some("starter_fortress".into()),
        Some(false),
        Some(true),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    field(&opened, "session_id")
}
fn inspect<T>(session: &str, body: impl FnOnce(&mut LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = server::lookup_session_str(session)?;
    let mut guard = handle.lock().map_err(|_| "session lock poisoned")?;
    body(&mut guard)
}
fn world(session: &str) -> TestResult<WorldSnapshot> {
    inspect(session, |session| Ok(session.adapter.snapshot().clone()))
}
fn plan(session: &str, actions: Value) -> TestResult<Value> {
    let planned = payload(&facade::fortress_plan(
        Some(session.to_owned()),
        Some("recover then stop this exact original".into()),
        None,
        Some(actions.to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned)
}
fn commit(session: &str, actions: Value) -> TestResult<(String, Value)> {
    let planned = plan(session, actions)?;
    let digest = field(&planned, "plan_digest")?;
    let committed = payload(&facade::fortress_commit(
        Some(session.to_owned()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok((digest, planned))
}
fn wait(session: &str, ticks: u64) -> TestResult<Value> {
    payload(&facade::fortress_wait(
        Some(session.to_owned()),
        Some(ticks),
    ))
}
fn selected_wait(session: &str, digest: &str) -> TestResult<Value> {
    payload(&facade::fortress_wait_for_plan(
        Some(session.to_owned()),
        Some(0),
        Some(digest.to_owned()),
    ))
}
fn stop(session: &str, digest: &str, mode: &str) -> TestResult<Value> {
    payload(&facade::fortress_cancel(
        Some(session.to_owned()),
        Some(mode.to_owned()),
        Some(format!("plan:{digest}")),
    ))
}
fn dig() -> Value {
    json!({"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}})
}
fn blocked_order() -> Value {
    json!({"action":{"kind":"create_work_order","name":"held original production",
        "job_token":"BREW_DRINK","amount":1,
        "conditions":[{"kind":"completed_order","order_name":"unavailable predecessor"}]}})
}
fn assert_quiet(value: &Value) {
    assert_eq!(value["ok"], true, "{value}");
    assert_eq!(
        value["selection_scope"], "exact_recovered_original",
        "{value}"
    );
    assert_eq!(value["dispatch_handles_restored"], false);
    assert_eq!(value["drain_progress"]["remaining_nonterminal"], 0);
    assert_eq!(value["drain_progress"]["remaining_work"], 0);
    assert_eq!(value["drain_progress"]["quiescent"], true);
    assert_eq!(value["publication_confirmed"], true);
    assert_eq!(
        value["finalize_certificate"]["digest"]
            .as_str()
            .map(str::len),
        Some(64)
    );
}

#[test]
fn recovered_stop_retires_exact_original_then_waits_withdraws_closes_and_reopens() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("lifetime");
    let first = open("7864001")?;
    let (digest, _) = commit(
        &first,
        json!([
            {"action":{"kind":"pause","paused":false}},
            {"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"},"depends_on":[0]},
            {"action":{"kind":"designate_dig","min":[0,6,10],"max":[1,6,10],"mode":"mine"},"depends_on":[1]}
        ]),
    )?;
    assert_eq!(wait(&first, 20)?["ok"], true);
    directory.restart();
    let session = open("7864001")?;
    let before = world(&session)?;
    let explanation = payload(&facade::fortress_explain(
        Some(session.clone()),
        Some(format!("goal:{digest}")),
    ))?;
    assert_eq!(explanation["ok"], true, "{explanation}");
    assert_eq!(
        explanation["cancellation"]["available"], true,
        "{explanation}"
    );
    let result = stop(&session, &digest, "stop_future_steps")?;
    assert_quiet(&result);
    assert_eq!(result["drain_progress"]["actions_total"], 3);
    assert_eq!(result["drain_progress"]["cancelled"], 1);
    assert_eq!(result["steps"][0]["state"], "verified");
    assert_eq!(result["steps"][1]["state"], "cancelled");
    assert_eq!(result["steps"][2]["state"], "not_dispatched");
    assert_eq!(
        result["steps"][0]["proof_anchor"],
        result["steps"][0]["recorded_anchor"]
    );
    let after = world(&session)?;
    assert_eq!(after.tick, before.tick);
    assert_eq!(
        after.graph.chunks, before.graph.chunks,
        "completed excavation must survive stop"
    );
    inspect(&session, |session| {
        assert!(session.adapter.retained_action_ids().next().is_none());
        assert!(session.last_action.is_none());
        assert!(session.commit_receipts.is_empty());
        assert!(session.carried.iter().all(CarriedStep::is_final));
        Ok(())
    })?;
    let repeated = stop(&session, &digest, "stop_future_steps")?;
    assert_quiet(&repeated);
    assert_eq!(repeated["drain_progress"]["drained"], 0);
    assert_eq!(world(&session)?, after);
    let observed = selected_wait(&session, &digest)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["selected_plan_wait"]["stop_reason"], "cancelled",
        "{observed}"
    );
    assert_eq!(observed["selected_plan_wait"]["physical_quiescent"], true);
    assert_eq!(observed["selected_plan_wait"]["not_dispatched_actions"], 1);
    let withdrawn = payload(&facade::fortress_cancel(
        Some(session.clone()),
        Some("withdraw_goal".into()),
        Some(format!("goal:{digest}")),
    ))?;
    assert_eq!(withdrawn["ok"], true, "{withdrawn}");
    assert_eq!(
        withdrawn["withdrawal"]["never_dispatched_steps"],
        json!([2])
    );
    let closed = payload(&facade::fortress_cancel(
        Some(session),
        Some("close_session".into()),
        Some("session".into()),
    ))?;
    assert_eq!(closed["ok"], true, "{closed}");
    directory.restart();
    {
        let mut store = DurableLabStore::open(&directory.0)?;
        let digest = Digest32::from_hex(&digest).ok_or("digest invalid")?;
        assert!(store.commit(after.fortress_id, digest).is_none());
        let archived = store
            .retired_commit(after.fortress_id, digest)
            .ok_or("retired receipts missing")?;
        assert_eq!(
            archived.steps.values().cloned().collect::<Vec<_>>(),
            vec!["verified", "cancelled", "not_dispatched"]
        );
        store.compact()?;
    }
    let reopened = open("7864001")?;
    let explained = payload(&facade::fortress_explain(
        Some(reopened.clone()),
        Some(format!("goal:{digest}")),
    ))?;
    assert_eq!(explained["ok"], true, "{explained}");
    assert_eq!(explained["goal"]["status"], "withdrawn", "{explained}");
    assert_eq!(explained["goal"]["withdrawal"], withdrawn["withdrawal"]);
    assert_eq!(stop(&reopened, &digest, "stop_future_steps")?["ok"], false);
    assert_eq!(world(&reopened)?.tick, after.tick);
    Ok(())
}

#[test]
fn recovered_failed_proof_remains_immutable_while_its_work_is_stopped() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("failed-proof");
    let first = open("7864002")?;
    let (digest, planned) = commit(&first, json!([blocked_order()]))?;
    let deadline = planned["steps"][0]["obligation"]["deadline_tick"]
        .as_u64()
        .ok_or("deadline missing")?;
    let failed = wait(&first, deadline - world(&first)?.tick.0 + 1)?;
    assert_eq!(failed["commit_state"], "Failed", "{failed}");
    assert_eq!(failed["work_state"]["state"], "active");
    directory.restart();
    let session = open("7864002")?;
    let proof = inspect(&session, |session| Ok(session.carried[0].proof_anchor))?;
    let before = world(&session)?;
    let result = stop(&session, &digest, "stop_future_steps")?;
    assert_quiet(&result);
    assert_eq!(result["drain_progress"]["cancelled"], 0);
    assert_eq!(result["drain_progress"]["terminal_work_stopped"], 1);
    assert_eq!(result["steps"][0]["state"], "failed");
    inspect(&session, |session| {
        assert_eq!(session.carried[0].state, "failed");
        assert_eq!(session.carried[0].proof_anchor, proof);
        assert!(session.carried[0].recovered_drain.is_some());
        Ok(())
    })?;
    assert_eq!(world(&session)?.tick, before.tick);
    let observed = selected_wait(&session, &digest)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(observed["selected_plan_wait"]["stop_reason"], "failed");
    directory.restart();
    DurableLabStore::open(&directory.0)?.compact()?;
    let reopened = open("7864002")?;
    let observed = selected_wait(&reopened, &digest)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(observed["selected_plan_wait"]["stop_reason"], "failed");
    assert_eq!(observed["selected_plan_wait"]["physical_quiescent"], true);
    Ok(())
}

#[test]
fn complete_recovered_preflight_refuses_authority_bounds_leases_and_bad_custody_without_stops()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("preflight");
    let first = open("7864003")?;
    let (digest, _) = commit(&first, json!([dig(), blocked_order()]))?;
    let mut unrelated = blocked_order();
    unrelated["action"]["name"] = json!("leave this recovered original running");
    let (other_digest, _) = commit(&first, json!([unrelated]))?;
    directory.restart();
    let session = open("7864003")?;
    let pending = plan(&session, json!([{"action":{"kind":"pause","paused":true}}]))?;
    let before = world(&session)?;
    let grants = inspect(&session, |session| {
        let grants = session.grants.clone();
        session
            .grants
            .retain(|grant| grant.capability != Capability::Designate);
        Ok(grants)
    })?;
    assert_eq!(
        stop(&session, &digest, "stop_future_steps")?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(world(&session)?, before);
    let budget = inspect(&session, |session| {
        session.grants = grants;
        Ok(session.budget)
    })?;
    for narrowed in [
        WorkBudget {
            max_actions: 1,
            ..budget
        },
        WorkBudget {
            max_entities: 1,
            ..budget
        },
        WorkBudget {
            max_bytes: 6_000,
            ..budget
        },
        WorkBudget {
            max_output_tokens: 1_499,
            ..budget
        },
    ] {
        inspect(&session, |session| {
            session.budget = narrowed;
            Ok(())
        })?;
        let result = stop(&session, &digest, "stop_future_steps")?;
        assert_eq!(result["ok"], false, "{result}");
        assert_eq!(result["error"]["code"], "budget_exceeded", "{result}");
        assert_eq!(world(&session)?, before);
    }
    inspect(&session, |session| {
        session.budget = budget;
        let scope = session
            .carried
            .iter()
            .find_map(|row| {
                row.work_step
                    .as_ref()
                    .and_then(|step| step.action.scope().map_area)
            })
            .ok_or("dig scope missing")?;
        session.leases.manager.acquire_spatial_lease(
            SessionId::new(999),
            scope,
            true,
            before.tick,
            100,
        )?;
        Ok(())
    })?;
    assert_eq!(
        stop(&session, &digest, "stop_future_steps")?["error"]["code"],
        "conflict"
    );
    assert_eq!(world(&session)?, before);
    let retained = inspect(&session, |session| {
        session
            .leases
            .manager
            .release_session_leases(SessionId::new(999));
        let retained = session.carried.clone();
        session
            .carried
            .iter_mut()
            .find(|row| row.plan_digest.to_hex() == digest)
            .ok_or("selected carried original missing")?
            .proof = Predicate::False;
        Ok(retained)
    })?;
    assert_eq!(
        stop(&session, &digest, "stop_future_steps")?["error"]["code"],
        "corrupt_ledger"
    );
    assert_eq!(world(&session)?, before);
    inspect(&session, |session| {
        session.carried = retained;
        session.commit_receipts.insert(digest.clone(), "{}".into());
        Ok(())
    })?;
    assert_eq!(
        stop(&session, &digest, "stop_future_steps")?["error"]["code"],
        "conflict"
    );
    assert_eq!(world(&session)?, before);
    inspect(&session, |session| {
        session.commit_receipts.remove(&digest);
        Ok(())
    })?;
    assert_eq!(
        stop(&session, &digest, "compensate_reversible")?["error"]["code"],
        "conflict"
    );
    assert_eq!(world(&session)?, before);
    assert_quiet(&stop(&session, &digest, "stop_future_steps")?);
    inspect(&session, |session| {
        assert_eq!(
            session.pending.as_ref().map(|plan| plan.digest.as_str()),
            pending["plan_digest"].as_str()
        );
        let other = session
            .carried
            .iter()
            .find(|row| row.plan_digest.to_hex() == other_digest)
            .ok_or("unrelated carried original missing")?;
        assert!(!other.is_final());
        let step = other
            .work_step
            .as_ref()
            .ok_or("unrelated original step missing")?;
        let entity = dfmcp_intent::effects::created_entity_id(&step.idempotency_key, 0);
        assert_eq!(
            session.adapter.snapshot().graph.entities.get(&entity),
            before.graph.entities.get(&entity)
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn stopped_work_with_failed_append_remains_visible_and_fences_further_effects() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("append-failure");
    let first = open("7864004")?;
    let (digest, _) = commit(&first, json!([dig(), blocked_order()]))?;
    directory.restart();
    let session = open("7864004")?;
    let before = world(&session)?;
    server::inject_durable_crash_after(0);
    let refused = stop(&session, &digest, "stop_future_steps")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "adapter_unavailable");
    assert_eq!(refused["durability_pending"], true);
    assert_eq!(refused["publication_confirmed"], false);
    assert_eq!(refused["drain_progress"]["drained"], 1);
    assert_eq!(refused["drain_progress"]["remaining_work"], 1);
    assert_eq!(refused["drain_progress"]["quiescent"], false);
    assert!(refused["finalize_certificate"].is_null());
    let partial = world(&session)?;
    assert_ne!(partial, before);
    assert_eq!(partial.tick, before.tick);
    let denied = stop(&session, &digest, "stop_future_steps")?;
    assert_eq!(denied["error"]["code"], "adapter_unavailable", "{denied}");
    assert_eq!(world(&session)?, partial);
    inspect(&session, |session| {
        assert!(session.durability_fault.is_some());
        assert_eq!(
            session
                .carried
                .iter()
                .filter(|row| row.state == "cancelled")
                .count(),
            1
        );
        Ok(())
    })?;
    // Once the same atomic publication succeeds, a fresh explicit request can
    // drain the remaining step without repeating the already performed stop.
    server::inject_durable_crash_after(100_000);
    let completed = stop(&session, &digest, "stop_future_steps")?;
    assert_quiet(&completed);
    assert_eq!(completed["drain_progress"]["drained"], 1);
    directory.restart();
    let reopened = open("7864004")?;
    let observed = selected_wait(&reopened, &digest)?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(observed["selected_plan_wait"]["stop_reason"], "cancelled");
    assert_eq!(observed["selected_plan_wait"]["physical_quiescent"], true);
    Ok(())
}

#[test]
fn recovered_emergency_requires_whole_budget_and_publishes_pause_before_stops() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("emergency");
    let first = open("7864005")?;
    let (digest, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864005")?;
    let before = world(&session)?;
    inspect(&session, |session| {
        session.budget.max_actions = 1;
        Ok(())
    })?;
    assert_eq!(
        stop(&session, &digest, "emergency_pause_and_drain")?["error"]["code"],
        "budget_exceeded"
    );
    assert_eq!(world(&session)?, before);
    inspect(&session, |session| {
        session.budget.max_actions = 2;
        Ok(())
    })?;
    let stopped = stop(&session, &digest, "emergency_pause_and_drain")?;
    assert_quiet(&stopped);
    assert_eq!(stopped["paused"], true);
    assert_eq!(stopped["emergency_pause"]["publication_confirmed"], true);
    assert_eq!(
        stopped["physical_drains"][0]["before_anchor"],
        stopped["emergency_pause"]["observed_anchor"]
    );
    assert_eq!(world(&session)?.tick, before.tick);
    Ok(())
}

#[test]
fn tampered_fresh_cancellation_evidence_cannot_certify_selected_wait_or_repeat() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("tampered-drain");
    let first = open("7864006")?;
    let (digest, _) = commit(&first, json!([dig()]))?;
    directory.restart();
    let session = open("7864006")?;
    assert_quiet(&stop(&session, &digest, "stop_future_steps")?);
    let quiet = world(&session)?;
    let retained = inspect(&session, |session| {
        let retained = session.carried[0].clone();
        session.carried[0]
            .recovered_drain
            .as_mut()
            .ok_or("drain receipt missing")?
            .custody_digest = Digest32::of_bytes(b"wrong custody");
        Ok(retained)
    })?;
    assert_eq!(
        stop(&session, &digest, "stop_future_steps")?["error"]["code"],
        "corrupt_ledger"
    );
    assert_eq!(selected_wait(&session, &digest)?["ok"], false);
    assert_eq!(world(&session)?, quiet);
    inspect(&session, |session| {
        session.carried[0] = retained;
        Ok(())
    })?;
    assert_quiet(&stop(&session, &digest, "stop_future_steps")?);
    Ok(())
}

#[test]
fn minimum_output_preserves_complete_multi_step_recovered_stop_outcome() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("minimum-output");
    let first = open("7864007")?;
    let actions = (0..3)
        .map(|index| {
            let mut order = blocked_order();
            order["action"]["name"] = json!(format!("retained original order {index}"));
            order
        })
        .collect::<Vec<_>>();
    let (digest, _) = commit(&first, json!(actions))?;
    directory.restart();
    let session = open("7864007")?;
    inspect(&session, |session| {
        session.budget.max_output_tokens = 1_500;
        Ok(())
    })?;
    let raw = facade::fortress_cancel(
        Some(session.clone()),
        Some("stop_future_steps".into()),
        Some(format!("plan:{digest}")),
    );
    assert!(raw.len() <= 6_000, "{} bytes: {raw}", raw.len());
    let stopped = payload(&raw)?;
    assert_quiet(&stopped);
    assert_eq!(stopped["drain_progress"]["actions_total"], 3);
    assert_eq!(stopped["drain_progress"]["cancelled"], 3);
    assert_eq!(stopped["drain_progress"]["drained"], 3);
    assert_eq!(stopped["selection_origin"], "durable_recovery");
    assert_eq!(
        stopped["plan_object_digest"].as_str().map(str::len),
        Some(64)
    );
    assert_eq!(stopped["output_budget"]["max_output_tokens"], 1_500);
    assert_ne!(stopped["output_budget"]["tier"], "full");
    assert_eq!(stopped["agent_turn"]["anchor"], stopped["observed_anchor"]);
    inspect(&session, |session| {
        assert_eq!(session.carried.len(), 3);
        assert!(
            session
                .carried
                .iter()
                .all(|row| row.state == "cancelled" && row.is_final())
        );
        assert!(session.adapter.retained_action_ids().next().is_none());
        Ok(())
    })?;
    Ok(())
}
