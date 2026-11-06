//! The public laboratory loop must own recovery history across calls and joins.

use super::*;
use crate::agent_facade as facade;
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}
fn id(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}
fn open(selector: &str, shared: bool, durable: bool) -> TestResult<String> {
    let opened = parsed(&facade::fortress_open_session(
        Some(true),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("configure_logistics", "guarded"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
            ]
            .into_iter()
            .map(|(cap, risk)| (cap.to_owned(), risk.to_owned()))
            .collect(),
        ),
        Some(30_000),
        Some(2_000),
        None,
        None,
        Some(8_192),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(shared),
        Some(durable),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    id(&opened, "session_id")
}
fn commit(session: &str, planned: &Value) -> TestResult<Value> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        id(planned, "plan_digest")?,
    ))
}
fn recovery(session: &str) -> TestResult<Value> {
    parsed(&facade::fortress_query(
        Some(session.to_owned()),
        Some("civilian_recovery".to_owned()),
    ))
}
fn wait(session: &str, ticks: u64) -> TestResult<Value> {
    let value = parsed(&facade::fortress_wait(
        Some(session.to_owned()),
        Some(ticks),
    ))?;
    assert_eq!(value["ok"], true, "{value}");
    Ok(value)
}
fn protect(session: &str) -> TestResult<()> {
    let planned = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        Some(
            json!({"workflow":"civilian_protection_v1","burrow":"3001", "civilians":
            (1_001..=1_007).map(|id| id.to_string()).collect::<Vec<_>>()})
            .to_string(),
        ),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(commit(session, &planned)?["ok"], true);
    wait(session, 0)?;
    Ok(())
}
fn unpause(session: &str) -> TestResult<Value> {
    let plan = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        Some(false),
        None,
        None,
    ))?;
    commit(session, &plan)
}
fn release(session: &str) -> TestResult<Value> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        Some("review civilian release".to_owned()),
        None,
        None,
        Some(json!({"template":"civilian_release","burrow":"3001"}).to_string()),
    ))
}

#[test]
fn public_loop_requires_stable_clearance_then_releases_exact_civilians() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("982001", false, false)?;
    protect(&session)?;
    for _ in 0..3 {
        assert_eq!(
            parsed(&facade::fortress_observe(Some(session.clone())))?["ok"],
            true
        );
    }
    assert_eq!(recovery(&session)?["burrows"][0]["release_ready"], false);
    assert_eq!(release(&session)?["ok"], false);
    assert_eq!(unpause(&session)?["ok"], true);
    wait(&session, 100)?;
    let recovered = recovery(&session)?;
    assert_eq!(
        recovered["burrows"][0]["release_ready"], true,
        "{recovered}"
    );
    let plan = release(&session)?;
    assert_eq!(plan["ok"], true, "{plan}");
    assert_eq!(plan["steps"].as_array().map(Vec::len), Some(2));
    let committed = commit(&session, &plan)?;
    assert_eq!(committed["ok"], true, "{committed}");
    wait(&session, 0)?;
    let released = recovery(&session)?;
    assert_eq!(
        released["burrows"][0]["released_configuration"], "true",
        "{released}"
    );
    assert_eq!(released["burrows"][0]["release_ready"], false);
    Ok(())
}

#[test]
fn a_release_plan_cannot_rebase_or_outlive_revoked_effect_authority() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("982002", false, false)?;
    protect(&session)?;
    assert_eq!(unpause(&session)?["ok"], true);
    wait(&session, 100)?;
    let stale = release(&session)?;
    wait(&session, 1)?;
    let refused = commit(&session, &stale)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert!(refused.get("rebased_plan").is_none());
    let fresh = release(&session)?;
    assert_eq!(fresh["ok"], true, "{fresh}");
    let handle = resolve_session(Some(session.clone()))?;
    with_session(
        &handle,
        || (),
        |guard| {
            guard
                .grants
                .retain(|grant| grant.capability != Capability::ConfigureLogistics);
        },
    );
    let refused = commit(&session, &fresh)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(
        recovery(&session)?["burrows"][0]["protection_observed"],
        "true"
    );
    Ok(())
}

#[test]
fn shared_sessions_consume_one_world_clear_interval() -> TestResult {
    let _serial = crate::test_serial();
    let first = open("982003", true, false)?;
    protect(&first)?;
    assert_eq!(unpause(&first)?["ok"], true);
    wait(&first, 100)?;
    let second = open("982003", true, false)?;
    let a = recovery(&first)?;
    let b = recovery(&second)?;
    assert_eq!(a["burrows"], b["burrows"]);
    assert_eq!(b["burrows"][0]["release_ready"], true, "{b}");
    let plan = release(&second)?;
    assert_eq!(plan["ok"], true, "{plan}");
    assert_eq!(commit(&second, &plan)?["ok"], true);
    assert_eq!(
        recovery(&first)?["burrows"][0]["released_configuration"],
        "true"
    );
    Ok(())
}

#[test]
fn durable_restart_keeps_protection_but_rebuilds_clearance() -> TestResult {
    let _serial = crate::test_serial();
    let _durable = serialized_durable_tests();
    let directory =
        std::env::temp_dir().join(format!("dfmcp-civilian-recovery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    simulate_durable_restart(Some(directory.clone()));
    let outcome = (|| -> TestResult {
        let session = open("982004", false, true)?;
        protect(&session)?;
        assert_eq!(unpause(&session)?["ok"], true);
        wait(&session, 100)?;
        assert_eq!(recovery(&session)?["burrows"][0]["release_ready"], true);
        let old = release(&session)?;
        simulate_durable_restart(Some(directory.clone()));
        let resumed = open("982004", false, true)?;
        let current = recovery(&resumed)?;
        assert_eq!(
            current["burrows"][0]["protection_observed"], "true",
            "{current}"
        );
        assert_eq!(current["burrows"][0]["release_ready"], false);
        assert_eq!(release(&resumed)?["ok"], false);
        assert_eq!(commit(&resumed, &old)?["ok"], false);
        wait(&resumed, 100)?;
        let plan = release(&resumed)?;
        assert_eq!(plan["ok"], true, "{plan}");
        assert_eq!(commit(&resumed, &plan)?["ok"], true);
        simulate_durable_restart(Some(directory.clone()));
        let released = open("982004", false, true)?;
        assert_eq!(
            recovery(&released)?["burrows"][0]["released_configuration"],
            "true"
        );
        Ok(())
    })();
    simulate_durable_restart(None);
    let _ = std::fs::remove_dir_all(directory);
    outcome
}

#[test]
fn public_history_claims_and_historical_recovery_queries_refuse() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("982005", false, false)?;
    let request = json!({"template":"civilian_release","burrow":"3001","clear_samples":100});
    let refused = parsed(&facade::fortress_plan(
        Some(session.clone()),
        None,
        None,
        None,
        Some(request.to_string()),
    ))?;
    assert_eq!(refused["ok"], false);
    let refused = parsed(&facade::fortress_query(
        Some(session),
        Some(json!({"mode":"civilian_recovery","at":"old"}).to_string()),
    ))?;
    assert_eq!(refused["ok"], false);
    Ok(())
}

#[test]
fn complete_census_and_compiled_work_require_current_budget_at_plan_and_commit() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("982006", false, false)?;
    protect(&session)?;
    assert_eq!(unpause(&session)?["ok"], true);
    wait(&session, 100)?;
    let handle = resolve_session(Some(session.clone()))?;
    let budget = with_session(&handle, WorkBudget::default, |guard| {
        let prior = guard.budget;
        guard.budget.max_entities = 7; // Seven named civilians fit; the complete domain does not.
        prior
    });
    let refused = release(&session)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    assert_eq!(recovery(&session)?["ok"], false);
    with_session(
        &handle,
        || (),
        |guard| {
            guard.budget = budget;
        },
    );
    let planned = release(&session)?;
    assert_eq!(planned["ok"], true, "{planned}");
    with_session(
        &handle,
        || (),
        |guard| {
            guard.budget.max_bytes = 1;
        },
    );
    let refused = commit(&session, &planned)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    with_session(
        &handle,
        || (),
        |guard| {
            guard.budget = budget;
        },
    );
    assert_eq!(
        recovery(&session)?["burrows"][0]["protection_observed"],
        "true"
    );
    assert_eq!(commit(&session, &planned)?["ok"], true);
    Ok(())
}
