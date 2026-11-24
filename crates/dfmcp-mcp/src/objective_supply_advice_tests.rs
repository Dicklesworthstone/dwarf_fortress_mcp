//! Retained supply advice uses real committed goals and the normal quiet gate.

use super::*;
use crate::server;
use dfmcp_intent::effects;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-supply-advice-recovery-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        server::simulate_durable_restart(Some(path.clone()));
        Self(path)
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

fn open(selector: &str, durable: bool) -> TestResult<String> {
    let opened = value(&server::open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
            ]
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
    string(&opened, "session_id")
}

fn commit(session: &str, blueprint: Value) -> TestResult<Digest32> {
    let planned = value(&server::plan_request(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(blueprint.to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = string(&planned, "plan_digest")?;
    let committed = value(&server::fortress_commit(
        Some(session.to_owned()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Digest32::from_hex(&digest).ok_or_else(|| "invalid plan digest".into())
}

fn wait(session: &str, ticks: u64) -> TestResult {
    let mut remaining = ticks;
    for _ in 0..128 {
        let waited = value(&server::wait_with_ticks(
            Some(session.to_owned()),
            Some(remaining),
        ))?;
        if waited["ok"] == true {
            return Ok(());
        }
        assert_eq!(waited["error"]["code"], "budget_exceeded", "{waited}");
        let advanced = waited["advanced_game_ticks"]
            .as_u64()
            .ok_or("bounded wait lost its progress frontier")?;
        let next = waited["wait_progress"]["remaining_game_ticks"]
            .as_u64()
            .ok_or("bounded wait lost its remaining allowance")?;
        assert!(advanced > 0 && next < remaining, "{waited}");
        assert_eq!(advanced + next, remaining);
        remaining = next;
    }
    Err("supply advice fixture exhausted bounded foreground waits".into())
}

fn proposed() -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":24}],"prerequisites":{"assign_labor":true}}"#,
    )
}

fn selected(session: &LabSession) -> TestResult<Option<goal_continuation::ProductionContinuation>> {
    match supply_recovery_continuation(session, &proposed()?, &read_context(session))? {
        Some(PlanSource::ProductionContinuation { raw, .. }) => Ok(Some(
            goal_continuation::ProductionContinuation::parse(&raw)?,
        )),
        None => Ok(None),
        Some(_) => Err("supply advice returned another source family".into()),
    }
}

fn original_request(session: &LabSession, digest: Digest32) -> TestResult<ProductionRequest> {
    let objective = session
        .objectives
        .iter()
        .find(|objective| objective.plan_digest == digest)
        .ok_or("missing original objective")?;
    let PlanSource::Production { raw, .. } = &objective.source else {
        return Err("missing original production source".into());
    };
    Ok(ProductionRequest::parse(raw)?)
}

fn consumed(session: &str) -> TestResult<Digest32> {
    let digest = commit(
        session,
        json!({"template":"production", "quotas":[{"item":"DRINK","minimum":60}]}),
    )?;
    wait(session, 1_200)?;
    inspect(session, |session| {
        let objective = session
            .objectives
            .iter()
            .find(|objective| objective.plan_digest == digest)
            .ok_or("missing consumed original")?;
        assert!(goal_work_quiescent(session, objective)?);
        assert_eq!(objective.current_truth(session)?, PredicateTruth::False);
        assert!(selected(session)?.is_some());
        Ok(())
    })?;
    Ok(digest)
}

#[test]
fn quiet_supply_advice_keeps_extra_quotas_and_original_permissions_without_effect_authority()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let session = open("7867101", false)?;
    let root = commit(
        &session,
        json!({
            "template":"production",
            "quotas":[{"item":"DRINK","minimum":60},{"item":"FOOD","minimum":65}],
            "prerequisites":{"assign_labor":false},
        }),
    )?;
    wait(&session, 1_200)?;
    let pending = value(&server::plan_request(
        Some(session.clone()),
        None,
        Some(true),
        None,
        None,
        None,
    ))?;
    assert_eq!(pending["ok"], true, "{pending}");
    mutate(&session, |session| {
        // Discovery preserves this exact pursuit even when its next effects
        // cannot be authorized. The caller must publish unavailable advice.
        session.grants.retain(|grant| {
            !matches!(
                grant.capability,
                Capability::ConfigureProduction | Capability::ConfigureLabor
            )
        });
        let before = (
            session.adapter.snapshot().clone(),
            session.grants.clone(),
            session.next_request_id,
            session
                .pending
                .as_ref()
                .map(|pending| pending.digest.clone()),
            format!("{:?}", session.objectives),
        );
        let original = original_request(session, root)?;
        let source = selected(session)?.ok_or("quiet supply goal was not retained")?;
        assert_eq!(source.parent, root);
        assert_eq!(source.root, root);
        assert_eq!(source.request(), &original);
        assert_eq!(source.request().quotas().get("FOOD"), Some(&65));
        assert_ne!(source.request(), &proposed()?);
        assert_eq!(
            (
                session.adapter.snapshot().clone(),
                session.grants.clone(),
                session.next_request_id,
                session
                    .pending
                    .as_ref()
                    .map(|pending| pending.digest.clone()),
                format!("{:?}", session.objectives),
            ),
            before,
            "advice cannot mutate the world, pending review, grants or original history"
        );
        Ok(())
    })
}

#[test]
fn unresolved_supply_goals_refuse_while_withdrawn_and_restore_abandoned_goals_are_skipped()
-> TestResult {
    let _serial = server::serialized_durable_tests();
    let session = open("7867102", false)?;
    let root = commit(
        &session,
        json!({"template":"production", "quotas":[{"item":"DRINK","minimum":60}]}),
    )?;
    inspect(&session, |session| {
        let error = supply_recovery_continuation(session, &proposed()?, &read_context(session))
            .expect_err("active work cannot be bypassed with a new supply root");
        assert_eq!(error.code, ErrorCode::PreconditionsFailed);
        Ok(())
    })?;
    wait(&session, 1_200)?;
    mutate(&session, |session| {
        assert!(selected(session)?.is_some());
        session.objective_restore.insert(root);
        assert!(
            selected(session)?.is_none(),
            "pending restore abandonment is binding"
        );
        session.objective_restore.remove(&root);

        let mut unknown = session.adapter.snapshot().clone();
        let ledger = effects::stock_ledger(&unknown).ok_or("missing stock ledger")?;
        let old = unknown
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or("missing stock ledger")?
            .fields
            .remove(effects::STOCK_DRINK_FIELD)
            .ok_or("missing original drink stock")?;
        unknown.cursor = unknown.cursor.checked_next().ok_or("cursor exhausted")?;
        unknown.refresh_hash();
        session.adapter.inject_snapshot(unknown)?;
        let error = supply_recovery_continuation(session, &proposed()?, &read_context(session))
            .expect_err("unknown stock cannot be bypassed with a new supply root");
        assert_eq!(error.code, ErrorCode::PreconditionsFailed);

        let mut restored = session.adapter.snapshot().clone();
        restored
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or("missing stock ledger")?
            .fields
            .insert(effects::STOCK_DRINK_FIELD.to_owned(), old);
        restored.cursor = restored.cursor.checked_next().ok_or("cursor exhausted")?;
        restored.refresh_hash();
        session.adapter.inject_snapshot(restored)?;
        assert!(selected(session)?.is_some());
        withdraw_goal(session, &root)?;
        assert!(
            selected(session)?.is_none(),
            "withdrawal cannot be undone by advice"
        );
        Ok(())
    })
}

#[test]
fn newest_quiet_continuation_is_selected_independent_of_goal_book_order() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let session = open("7867103", false)?;
    let root = consumed(&session)?;
    let child = commit(
        &session,
        json!({"template":"continue_goal", "plan_digest":root.to_hex()}),
    )?;
    inspect(&session, |session| {
        let error = supply_recovery_continuation(session, &proposed()?, &read_context(session))
            .expect_err("an active newer pursuit cannot fall back to its old root");
        assert_eq!(error.code, ErrorCode::PreconditionsFailed);
        Ok(())
    })?;
    wait(&session, 1_200)?;
    mutate(&session, |session| {
        let first = selected(session)?.ok_or("quiet child not offered")?;
        assert_eq!(first.parent, child);
        assert_eq!(first.root, root);
        session.objectives.reverse();
        assert_eq!(selected(session)?, Some(first));
        withdraw_goal(session, &child)?;
        let fallback = selected(session)?.ok_or("eligible original was not found")?;
        assert_eq!(fallback.parent, root);
        assert_eq!(fallback.root, root);
        Ok(())
    })
}

#[test]
fn discovery_refuses_incomplete_source_budgets_and_cancelled_or_foreign_contexts() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let session = open("7867104", false)?;
    consumed(&session)?;
    inspect(&session, |session| {
        let original = session.adapter.snapshot().clone();
        let proposed = proposed()?;
        for dimension in 0..6 {
            let mut context = read_context(session);
            let expected = match dimension {
                0 => {
                    context.budget.max_entities = 1;
                    ErrorCode::BudgetExceeded
                }
                1 => {
                    context.budget.max_bytes = proposed.canonical_json().len() as u64;
                    ErrorCode::BudgetExceeded
                }
                2 => {
                    context.cancellation_requested = true;
                    ErrorCode::CancellationRequested
                }
                3 => {
                    context.anchor.cursor.sequence =
                        context.anchor.cursor.sequence.saturating_add(1);
                    ErrorCode::InvalidRequest
                }
                4 => {
                    context
                        .grants
                        .retain(|grant| grant.capability != Capability::Observe);
                    ErrorCode::CapabilityDenied
                }
                _ => {
                    context
                        .grants
                        .retain(|grant| grant.capability != Capability::Plan);
                    ErrorCode::CapabilityDenied
                }
            };
            let error = supply_recovery_continuation(session, &proposed, &context)
                .expect_err("incomplete discovery cannot silently synthesize a new root");
            assert_eq!(error.code, expected, "dimension {dimension}: {error}");
        }
        assert_eq!(session.adapter.snapshot(), &original);
        Ok(())
    })
}

#[test]
fn quiet_original_reserve_requires_current_query_instead_of_becoming_a_new_plain_root() -> TestResult
{
    let _serial = server::serialized_durable_tests();
    let session = open("7867106", false)?;
    let root = commit(
        &session,
        json!({
            "template":"production", "reserve_until_tick":2401,
            "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
        }),
    )?;
    let stopped = value(&server::cancel_in_scope(
        Some(session.clone()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(stopped["ok"], true, "{stopped}");
    wait(&session, 1)?;
    inspect(&session, |session| {
        let source = selected(session)?.ok_or("quiet unfinished reserve was not retained")?;
        assert_eq!(source.parent, root);
        assert_eq!(source.request().reserve_until_tick(), Some(2401));
        let before = session.adapter.snapshot().clone();
        let mut context = read_context(session);
        context
            .grants
            .retain(|grant| grant.capability != Capability::Query);
        let error = supply_recovery_continuation(session, &proposed()?, &context)
            .expect_err("missing Query must not erase the retained reserve source");
        assert_eq!(error.code, ErrorCode::CapabilityDenied);
        assert_eq!(session.adapter.snapshot(), &before);
        Ok(())
    })
}

#[test]
fn verified_durable_supply_goal_remains_eligible_without_restoring_action_handles() -> TestResult {
    let _serial = server::serialized_durable_tests();
    let directory = StateDir::new();
    let first = open("7867105", true)?;
    let root = consumed(&first)?;
    let original = inspect(&first, |session| original_request(session, root))?;
    server::simulate_durable_restart(Some(directory.0.clone()));
    let recovered = open("7867105", true)?;
    inspect(&recovered, |session| {
        let objective = session
            .objectives
            .iter()
            .find(|objective| objective.plan_digest == root)
            .ok_or("missing recovered supply original")?;
        assert!(!objective.originating_process);
        assert!(!objective.original_handles_live);
        assert!(session.open_actions.is_empty());
        let before = session.adapter.snapshot().clone();
        let source = selected(session)?.ok_or("verified recovered supply goal was not offered")?;
        assert_eq!(source.parent, root);
        assert_eq!(source.root, root);
        assert_eq!(source.request(), &original);
        assert_eq!(session.adapter.snapshot(), &before);
        assert!(session.open_actions.is_empty());
        Ok(())
    })
}

#[test]
fn supply_matching_preserves_original_minima_without_importing_unrelated_outputs() -> Result<()> {
    let proposed = proposed()?;
    for (quotas, expected) in [
        (
            json!([{"item":"DRINK","minimum":60},{"item":"FOOD","minimum":65}]),
            true,
        ),
        (json!([{"item":"DRINK","minimum":23}]), false),
        (json!([{"item":"FOOD","minimum":65}]), false),
        (
            json!([{"item":"DRINK","minimum":60},{"item":"BED","minimum":2}]),
            false,
        ),
    ] {
        let original = ProductionRequest::parse_current(
            &json!({"template":"production", "quotas":quotas}).to_string(),
        )?;
        assert_eq!(covers(&original, &proposed), expected);
    }
    Ok(())
}
