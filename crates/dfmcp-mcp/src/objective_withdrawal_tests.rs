//! Executable goal withdrawal workflows through real laboratory owners.
use super::*;
use crate::{agent_facade, server};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);
impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-goal-withdrawal-{label}-{}",
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
fn value(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}
fn string(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}
fn open(selector: &str, durable: bool) -> TestResult<String> {
    let result = value(&server::open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("designate", "guarded"),
                ("configure_production", "reversible"),
            ]
            .into_iter()
            .map(|(cap, risk)| (cap.to_owned(), risk.to_owned()))
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
    assert_eq!(result["ok"], true, "{result}");
    string(&result, "session_id")
}
fn inspect<T>(session: &str, body: impl FnOnce(&LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = server::lookup_session_str(session)?;
    let guard = handle.lock().map_err(|_| "session poisoned")?;
    body(&guard)
}
fn mutate<T>(session: &str, body: impl FnOnce(&mut LabSession) -> TestResult<T>) -> TestResult<T> {
    let handle = server::lookup_session_str(session)?;
    let mut guard = handle.lock().map_err(|_| "session poisoned")?;
    body(&mut guard)
}
fn anchor(session: &str) -> TestResult<StateAnchor> {
    inspect(session, |session| Ok(session.adapter.snapshot().anchor()))
}
fn original(session: &str, digest: Digest32) -> TestResult<Value> {
    inspect(session, |session| {
        Ok(original_goal_observation(session, &digest.to_hex())?.1)
    })
}
fn dig(session: &str, dependent: bool) -> TestResult<Digest32> {
    let mut actions = vec![
        json!({"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}),
    ];
    if dependent {
        actions.push(json!({"action":{"kind":"designate_dig","min":[0,6,10],"max":[1,6,10],"mode":"mine"},"depends_on":[0]}));
    }
    let planned = value(&server::plan_request(
        Some(session.to_owned()),
        Some("withdraw an unwanted excavation".to_owned()),
        None,
        Some(json!(actions).to_string()),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = string(&planned, "plan_digest")?;
    let committed = value(&server::fortress_commit(
        Some(session.to_owned()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Digest32::from_hex(&digest).ok_or_else(|| "invalid digest".into())
}
fn drain(session: &str, digest: Digest32) -> TestResult<Value> {
    let result = value(&server::cancel_in_scope(
        Some(session.to_owned()),
        Some("stop_future_steps".to_owned()),
        Some(format!("plan:{digest}")),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["drain_progress"]["quiescent"], true, "{result}");
    Ok(result)
}
fn withdraw(session: &str, digest: Digest32) -> TestResult<Value> {
    value(&agent_facade::fortress_cancel(
        Some(session.to_owned()),
        Some("withdraw_goal".to_owned()),
        Some(format!("goal:{digest}")),
    ))
}

#[test]
fn full_unmet_goal_book_can_withdraw_one_quiet_original_and_commit_its_pending_review() -> TestResult
{
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("capacity");
    let session = open("7856201", true)?;
    let mut originals = Vec::new();
    for _ in 0..MAX_OBJECTIVES_PER_FORTRESS {
        let digest = dig(&session, false)?;
        drain(&session, digest)?;
        originals.push(digest);
    }
    let first = originals[0];
    let prior_goal = original(&session, first)?;
    assert_eq!(prior_goal["predicate_truth"], "false");
    assert_eq!(prior_goal["first_satisfied_anchor"], Value::Null);
    let planned = value(&server::plan_request(
        Some(session.clone()),
        Some("pause after archive".to_owned()),
        Some(true),
        None,
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let pending = string(&planned, "plan_digest")?;
    let before = anchor(&session)?;
    let refused = value(&server::fortress_commit(
        Some(session.clone()),
        pending.clone(),
    ))?;
    assert_eq!(refused["error"]["code"], "budget_exceeded", "{refused}");
    assert_eq!(anchor(&session)?, before);
    let history = with_durable_store(|store| {
        Ok((
            store.objective(before.fortress_id, first).cloned(),
            store.retired_commit(before.fortress_id, first).cloned(),
        ))
    })?;
    let archived = withdraw(&session, first)?;
    assert_eq!(archived["ok"], true, "{archived}");
    assert_eq!(archived["status"], "withdrawn");
    assert_eq!(archived["goal_success_claimed"], false);
    assert_eq!(archived["active_objectives"], 63);
    assert_eq!(archived["archived_objectives"], 1);
    assert_eq!(anchor(&session)?, before);
    inspect(&session, |session| {
        assert_eq!(
            session
                .pending
                .as_ref()
                .map(|pending| pending.plan.digest.to_hex()),
            Some(pending.clone())
        );
        assert_eq!(session.objectives.len(), MAX_OBJECTIVES_PER_FORTRESS);
        Ok(())
    })?;
    with_durable_store(|store| {
        assert_eq!(
            store.objective(before.fortress_id, first),
            history.0.as_ref()
        );
        assert_eq!(
            store.retired_commit(before.fortress_id, first),
            history.1.as_ref()
        );
        Ok(())
    })?;
    let committed = value(&server::fortress_commit(Some(session.clone()), pending))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let kept = original(&session, first)?;
    assert_eq!(kept["status"], "withdrawn");
    assert_eq!(kept["original_source"], prior_goal["original_source"]);
    assert_eq!(
        kept["first_satisfied_anchor"],
        prior_goal["first_satisfied_anchor"]
    );
    assert_eq!(kept["needs_replan"], false);
    assert_eq!(kept["pursuit_active"], false);
    Ok(())
}

#[test]
fn withdrawal_requires_current_mutating_plan_authority_quiet_custody_and_complete_budgets()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("preflight");
    let session = open("7856202", false)?;
    let digest = dig(&session, false)?;
    let before = anchor(&session)?;
    assert_eq!(withdraw(&session, digest)?["ok"], false);
    assert_eq!(anchor(&session)?, before);
    drain(&session, digest)?;
    let before = anchor(&session)?;
    let original_grants = mutate(&session, |session| {
        let old = session.grants.clone();
        for grant in &mut session.grants {
            if grant.capability == Capability::Plan {
                grant.max_risk = RiskTier::ReadOnly;
            }
        }
        Ok(old)
    })?;
    let denied = withdraw(&session, digest)?;
    assert_eq!(denied["error"]["code"], "capability_denied", "{denied}");
    mutate(&session, |session| {
        session.grants = original_grants;
        Ok(())
    })?;
    for output in [false, true] {
        mutate(&session, |session| {
            let old = session.budget;
            if output {
                session.budget.max_output_tokens = 1;
            } else {
                session.budget.max_bytes = 1;
            }
            assert!(withdraw_goal(session, &digest).is_err());
            assert!(
                !session
                    .objectives
                    .iter()
                    .find(|goal| goal.plan_digest == digest)
                    .ok_or("original missing")?
                    .is_withdrawn()
            );
            session.budget = old;
            Ok(())
        })?;
    }
    inspect(&session, |session| {
        let request = session.next_request_id;
        let offered = withdrawal_offer(session, digest);
        assert_eq!(offered["available"], true, "{offered}");
        assert_eq!(session.next_request_id, request);
        assert_eq!(session.adapter.snapshot().anchor(), before);
        Ok(())
    })?;
    let archived = withdraw(&session, digest)?;
    assert_eq!(archived["ok"], true, "{archived}");
    let repeated = withdraw(&session, digest)?;
    assert_eq!(repeated["ok"], true, "{repeated}");
    assert_eq!(repeated["already_withdrawn"], true);
    assert_eq!(repeated["withdrawal"], archived["withdrawal"]);
    assert_eq!(anchor(&session)?, before);
    inspect(&session, |session| {
        let error = continuation_source(session, digest, None, None)
            .err()
            .ok_or("withdrawn continuation accepted")?;
        assert_eq!(error.code, ErrorCode::PreconditionsFailed);
        Ok(())
    })?;
    mutate(&session, |session| {
        let original = session
            .objectives
            .iter_mut()
            .find(|goal| goal.plan_digest == digest)
            .ok_or("missing original")?;
        original.verification_error =
            Some("withdrawal quiet frontier cannot be verified".to_owned());
        let report = original_goal_observation(session, &digest.to_hex())?.1;
        assert_eq!(report["status"], "indeterminate");
        assert_eq!(report["withdrawal"]["verification"], "unverifiable");
        assert_eq!(
            report["withdrawal"]["physical_quiescent_at_withdrawal"],
            Value::Null
        );
        assert_eq!(withdrawal_offer(session, digest)["available"], false);
        assert!(objective_evictions(session, Digest32::of_bytes(b"new pursuit")).is_err());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn withdrawn_deferred_cancellation_survives_compaction_and_repeated_reopen_without_dispatch()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("deferred");
    let first = open("7856203", true)?;
    let digest = dig(&first, true)?;
    drain(&first, digest)?;
    let before = original(&first, digest)?;
    let archived = withdraw(&first, digest)?;
    assert_eq!(archived["ok"], true, "{archived}");
    assert_eq!(
        archived["withdrawal"]["never_dispatched_steps"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let withdrawal = archived["withdrawal"].clone();
    let mut previous = anchor(&first)?;
    for _ in 0..2 {
        directory.restart();
        {
            let mut store = DurableLabStore::open(&directory.0)?;
            store.compact()?;
        }
        let session = open("7856203", true)?;
        let current = anchor(&session)?;
        assert!(current.cursor.epoch > previous.cursor.epoch);
        let retained = original(&session, digest)?;
        assert_eq!(retained["status"], "withdrawn", "{retained}");
        assert_eq!(retained["withdrawal"], withdrawal);
        assert_eq!(retained["original_source"], before["original_source"]);
        assert_eq!(
            retained["first_satisfied_anchor"],
            before["first_satisfied_anchor"]
        );
        inspect(&session, |session| {
            let original = session
                .objectives
                .iter()
                .find(|goal| goal.plan_digest == digest)
                .ok_or("missing archive")?;
            assert!(
                original.verification_error.is_none(),
                "{:?}",
                original.verification_error
            );
            assert!(!original.original_handles_live);
            assert!(original.active_observation_anchor.is_none());
            assert!(session.open_actions.is_empty());
            Ok(())
        })?;
        let waited = value(&server::wait_for_plan(
            Some(session.clone()),
            Some(100),
            digest.to_hex(),
        ))?;
        assert_eq!(waited["ok"], false, "{waited}");
        assert_eq!(anchor(&session)?, current);
        previous = current;
    }
    Ok(())
}

#[test]
fn recovered_quiet_original_can_be_explicitly_withdrawn_with_fresh_authority() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("recovered");
    let first = open("7856204", true)?;
    let digest = dig(&first, false)?;
    drain(&first, digest)?;
    directory.restart();
    let session = open("7856204", true)?;
    let before = anchor(&session)?;
    let archived = withdraw(&session, digest)?;
    assert_eq!(archived["ok"], true, "{archived}");
    assert_eq!(archived["withdrawal"]["requester_session_id"], session);
    assert_eq!(archived["withdrawal"]["dispatch_authority_restored"], false);
    assert_eq!(anchor(&session)?, before);
    Ok(())
}

#[test]
fn withdrawal_write_failure_retains_active_custody_and_installs_no_marker() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let _directory = StateDir::new("failure");
    let session = open("7856205", true)?;
    let digest = dig(&session, false)?;
    drain(&session, digest)?;
    let before = anchor(&session)?;
    with_durable_store(|store| {
        store.set_append_budget(Some(0));
        Ok(())
    })?;
    mutate(&session, |session| {
        assert!(withdraw_goal(session, &digest).is_err());
        assert!(session.durability_fault.is_some());
        assert!(
            !session
                .objectives
                .iter()
                .find(|goal| goal.plan_digest == digest)
                .ok_or("missing original")?
                .is_withdrawn()
        );
        Ok(())
    })?;
    with_durable_store(|store| {
        assert!(
            store
                .objective_withdrawal(before.fortress_id, digest)
                .is_none()
        );
        store.set_append_budget(None);
        Ok(())
    })?;
    assert_eq!(anchor(&session)?, before);
    let retried = withdraw(&session, digest)?;
    assert_eq!(retried["ok"], true, "{retried}");
    Ok(())
}

#[test]
fn explicit_withdrawal_preserves_a_missing_original_reserve_endpoint_across_reopen() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("missing-reserve");
    let first = open("7856206", true)?;
    let planned = value(&server::plan_request(
        Some(first.clone()),
        None,
        None,
        None,
        Some(
            json!({"template":"production","reserve_until_tick":1234,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]})
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = Digest32::from_hex(&string(&planned, "plan_digest")?).ok_or("invalid digest")?;
    let committed = value(&server::fortress_commit(
        Some(first.clone()),
        digest.to_hex(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    drain(&first, digest)?;
    let prior = original(&first, digest)?;
    let archived = withdraw(&first, digest)?;
    assert_eq!(archived["ok"], true, "{archived}");
    let fortress = anchor(&first)?.fortress_id;
    with_durable_store(|store| {
        assert!(
            store
                .objective_reserve_observation(fortress, digest)
                .is_none()
        );
        assert_eq!(
            store
                .objective(fortress, digest)
                .and_then(|goal| goal.first_satisfied_anchor),
            None
        );
        Ok(())
    })?;
    directory.restart();
    let second = open("7856206", true)?;
    let retained = original(&second, digest)?;
    assert_eq!(retained["status"], "withdrawn", "{retained}");
    assert_eq!(
        retained["first_satisfied_anchor"],
        prior["first_satisfied_anchor"]
    );
    assert_eq!(retained["retained_reserve_endpoint"], Value::Null);
    assert_eq!(retained["withdrawal"], archived["withdrawal"]);
    Ok(())
}

#[test]
fn withdrawing_a_false_reserve_keeps_the_exact_negative_endpoint_as_history() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new("false-reserve");
    let first = open("7856207", true)?;
    let planned = value(&server::plan_request(
        Some(first.clone()),
        None,
        None,
        None,
        Some(
            json!({"template":"production","reserve_until_tick":1234,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]})
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = Digest32::from_hex(&string(&planned, "plan_digest")?).ok_or("invalid digest")?;
    let committed = value(&server::fortress_commit(
        Some(first.clone()),
        digest.to_hex(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    drain(&first, digest)?;
    let ticks = 1234 - anchor(&first)?.tick.0;
    let waited = value(&server::wait_with_ticks(Some(first.clone()), Some(ticks)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let endpoint = anchor(&first)?;
    mutate(&first, |session| {
        let truth = session
            .objectives
            .iter()
            .find(|goal| goal.plan_digest == digest)
            .ok_or("original missing")?
            .resource_output_truth_at(session.adapter.snapshot())?;
        assert_eq!(truth, PredicateTruth::False);
        retain_reserve_observation(session, digest, endpoint, truth)?;
        Ok(())
    })?;
    let archived = withdraw(&first, digest)?;
    assert_eq!(archived["ok"], true, "{archived}");
    directory.restart();
    let second = open("7856207", true)?;
    let retained = original(&second, digest)?;
    assert_eq!(retained["status"], "withdrawn", "{retained}");
    assert_eq!(retained["retained_reserve_endpoint"]["truth"], "false");
    assert_eq!(
        retained["retained_reserve_endpoint"]["anchor"],
        anchor_json(&endpoint)
    );
    assert_eq!(
        retained["retained_reserve_endpoint"]["historical_only"],
        true
    );
    assert_eq!(retained["first_satisfied_anchor"], Value::Null);
    assert_eq!(retained["withdrawal"]["goal_success_claimed"], false);
    Ok(())
}
