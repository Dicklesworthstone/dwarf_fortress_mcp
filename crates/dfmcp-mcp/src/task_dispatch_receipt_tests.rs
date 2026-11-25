//! Initial and recovered Task dispatch must not infer no effects from lost custody.
//! Ownership: df-fastmcp-conformance-5pj.3 and df-tasks-projection-tests-yy4.
use super::*;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn payload(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn text_field(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn open(selector: &str) -> TestResult<String> {
    let opened = payload(&crate::agent_facade::fortress_open_session(
        Some(true),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
            ]
            .into_iter()
            .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
            .collect(),
        ),
        None,
        None,
        None,
        None,
        Some(8_192),
        None,
        None,
        None,
        None,
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    text_field(&opened, "session_id")
}

fn plan(session: &str) -> TestResult<TaskWork> {
    let planned = payload(&crate::agent_facade::fortress_plan(
        Some(session.to_owned()),
        Some("retain original dispatch identity across a restore".to_owned()),
        Some(false),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(TaskWork::parse(&json!({
        "schema": WORK_SCHEMA,
        "session_id": session,
        "plan_digest": text_field(&planned, "plan_digest")?,
    }))?)
}

fn assert_unresolved(value: &Value) {
    assert_eq!(value["status"], "failed");
    assert_eq!(value["proof_status"], "unknown");
    assert!(value["original_plan_dispatched"].is_null());
    assert!(value["remaining_work"].is_null());
    assert_eq!(value["physical_quiescent"], false);
    assert_eq!(value["physical_work"]["state"], "unknown");
    assert_eq!(value["indeterminate"], true);
    assert_eq!(value["blind_retry_allowed"], false);
    assert_eq!(value["recovery_class"], "reconciliation_required");
    assert_eq!(value["receipt_available"], false);
    for key in [
        "actions_total",
        "remaining_nonterminal",
        "remaining_work",
        "unknown_work",
        "remaining_actions",
        "drained",
    ] {
        assert!(value["drain_progress"][key].is_null(), "{key}: {value}");
    }
    assert_eq!(value["drain_progress"]["physical_quiescent"], false);
    assert_eq!(value["drain_progress"]["quiescent"], false);
    assert_eq!(
        value["commit_response_scope"],
        "historical_dispatch_attempt_response"
    );
    assert_eq!(value["next_step"]["tool"], "fortress.observe");
}

#[test]
fn initial_success_followed_by_restore_never_becomes_a_zero_effect_failure() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855311")?;
    let checkpoint = payload(&crate::agent_facade::fortress_checkpoint(
        Some(session.clone()),
        Some("before-original-task".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let work = plan(&session)?;
    let committed = payload(&crate::agent_facade::fortress_commit(
        Some(session.clone()),
        work.original_digest()?.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(committed["paused"], false);
    assert!(
        missing_original_receipt(&work, committed.clone(), false)?.is_none(),
        "retained successful custody must proceed to ordinary monitoring"
    );

    // Deterministic interleaving: the real effect returned successfully, then
    // restore retired its old-epoch receipt before the supervisor inspected it.
    let restored = payload(&crate::agent_facade::fortress_restore(
        Some(session.clone()),
        text_field(&checkpoint, "checkpoint_id")?,
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    assert!(!crate::server::task_session::has_original_receipt(
        &session,
        work.original_digest()?,
    )?);

    for recovering in [false, true] {
        let (_, failure) = missing_original_receipt(&work, committed.clone(), recovering)?
            .ok_or("missing custody must retain a failure")?;
        assert_unresolved(&failure);
        assert_eq!(failure["session_id"], session);
        assert_eq!(failure["plan_digest"], work.original_digest()?);
        assert_eq!(failure["monitor_recovery_attempt"], recovering);
        assert_eq!(failure["commit_response"], committed);
        assert_eq!(failure["commit_response"]["ok"], true);
        assert!(
            missing_original_receipt(&work, committed.clone(), recovering)?.is_some(),
            "classification cannot recreate the original receipt or redispatch"
        );
    }
    Ok(())
}

#[test]
fn initial_refusal_does_not_turn_receipt_absence_into_physical_proof() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855312")?;
    let work = plan(&session)?;
    // A rejected unrelated digest leaves the original pending. The response
    // is useful diagnosis but not current custody for this admitted original.
    let refused = payload(&crate::agent_facade::fortress_commit(
        Some(session.clone()),
        "f".repeat(64),
    ))?;
    assert_eq!(refused["ok"], false, "{refused}");
    let (_, failure) = missing_original_receipt(&work, refused.clone(), false)?
        .ok_or("missing original custody must remain unresolved")?;
    assert_unresolved(&failure);
    assert_eq!(failure["commit_response"], refused);
    assert_eq!(failure["monitor_recovery_attempt"], false);

    // Classification did not consume the pending plan or close its separate
    // foreground authority. The exact original remains explicitly committable.
    let committed = payload(&crate::agent_facade::fortress_commit(
        Some(session),
        work.original_digest()?.to_owned(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert!(missing_original_receipt(&work, committed, false)?.is_none());
    Ok(())
}
