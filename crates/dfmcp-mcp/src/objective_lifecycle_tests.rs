//! Original goals survive their effects, owners, checkpoints and process loss.
use super::*;
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("dfmcp-objective-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
}

impl Drop for StateDir {
    fn drop(&mut self) {
        simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn identity(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn fixture_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn in_session<T>(session: &str, body: impl FnOnce(&mut LabSession) -> Result<T>) -> TestResult<T> {
    let handle = resolve_session(Some(session.to_owned()))?;
    Ok(with_session(
        &handle,
        || Err(fixture_error("goal fixture session unavailable")),
        body,
    )?)
}

fn open(selector: &str, shared: bool, durable: bool, observe: bool) -> TestResult<Value> {
    let mut capabilities = vec![
        ("query", "read_only"),
        ("plan", "read_only"),
        ("control_clock", "reversible"),
        ("configure_production", "reversible"),
        ("configure_labor", "reversible"),
        ("checkpoint", "guarded"),
        ("restore", "guarded"),
        ("doctor", "read_only"),
    ];
    if observe {
        capabilities.push(("observe", "read_only"));
    }
    let opened = parsed(&open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            capabilities
                .into_iter()
                .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
                .collect(),
        ),
        None,
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
    Ok(opened)
}

fn actions(session: &str, raw: &str) -> TestResult<(String, Value)> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()),
        None,
        None,
        Some(raw.to_owned()),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = identity(&planned, "plan_digest")?;
    let committed = parsed(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok((digest, committed))
}

const BREW_OFF: &str =
    r#"[{"action":{"kind":"set_labor","units":["1003"],"labor":"BREW","enabled":false}}]"#;
const BREW_ON: &str =
    r#"[{"action":{"kind":"set_labor","units":["1003"],"labor":"BREW","enabled":true}}]"#;

fn production_plan(session: &str) -> TestResult<Value> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#.to_owned()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned)
}

fn production(session: &str) -> TestResult<String> {
    let digest = identity(&production_plan(session)?, "plan_digest")?;
    let committed = parsed(&fortress_commit(Some(session.to_owned()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(digest)
}

fn wait(session: &str, ticks: u64) -> TestResult<Value> {
    let waited = parsed(&wait_with_ticks(Some(session.to_owned()), Some(ticks)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    Ok(waited)
}

fn goal(session: &str, digest: &str) -> TestResult<Value> {
    in_session(session, |guard| {
        original_goal_observation(guard, digest).map(|(_, value)| value)
    })
}

#[test]
fn original_goal_survives_done_compaction_and_restart_with_its_owner() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("done");
    let first = identity(&open("7843001", false, true, true)?, "session_id")?;
    let (digest, _) = actions(&first, BREW_OFF)?;
    let before = goal(&first, &digest)?;
    assert_eq!(before["predicate_truth"], "true");
    assert_eq!(before["historical_achievement"], "verified");
    in_session(&first, |guard| {
        for grant in &mut guard.grants {
            if grant.capability == Capability::Observe {
                grant.expires_at_tick = Some(GameTick(0));
            }
        }
        let hidden = objectives_json(guard);
        assert_eq!(hidden[0]["status"], "unavailable");
        assert!(hidden[0].get("first_satisfied_anchor").is_none());
        assert!(hidden[0].get("original_source").is_none());
        for grant in &mut guard.grants {
            if grant.capability == Capability::Observe {
                grant.expires_at_tick = None;
            }
        }
        Ok(())
    })?;
    let fortress = in_session(&first, |guard| Ok(guard.fortress_id))?;
    let plan = Digest32::from_hex(&digest).ok_or("digest")?;
    with_durable_store(|store| {
        assert!(
            store.commit(fortress, plan).is_none(),
            "the finished action commit must retire"
        );
        assert!(
            store.objective(fortress, plan).is_some(),
            "goal history is independent of Done"
        );
        store.compact()
    })?;
    simulate_durable_restart(Some(directory.0.clone()));
    let reopened = open("7843001", false, true, true)?;
    let second = identity(&reopened, "session_id")?;
    let after = goal(&second, &digest)?;
    assert_eq!(
        after["first_satisfied_anchor"],
        before["first_satisfied_anchor"]
    );
    assert_eq!(after["owner_session_id"], first);
    assert_eq!(after["owned_by_current_session"], false);
    let historical_owner = parse_session_id_arg(&first)?;
    in_session(&second, |guard| {
        let current_id = guard.session_id;
        guard.session_id = historical_owner;
        let collision = objectives_json(guard);
        guard.session_id = current_id;
        assert_eq!(collision[0]["owner_session_id"], first);
        assert_eq!(
            collision[0]["owned_by_current_session"], false,
            "a real restart may reuse a process-local session counter"
        );
        Ok(())
    })?;
    assert_eq!(after["original_source"], before["original_source"]);
    assert_eq!(after["terminal_condition"], before["terminal_condition"]);
    assert_eq!(after["predicate_truth"], "true");
    assert_eq!(reopened["durable"]["retained_objectives"], 1);
    assert_eq!(reopened["durable"]["recovered_commits"], json!([]));
    in_session(&second, |guard| {
        assert!(guard.last_action.is_none());
        assert!(guard.open_actions.is_empty());
        assert!(guard.pending.is_none());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn original_production_first_proof_remains_true_history_when_current_stock_falls() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("consumption");
    let first = identity(&open("7843002", false, true, true)?, "session_id")?;
    let digest = production(&first)?;
    wait(&first, 200)?;
    let achieved = goal(&first, &digest)?;
    assert_eq!(achieved["predicate_truth"], "true", "{achieved}");
    assert_eq!(achieved["historical_achievement"], "verified");
    // Metabolism counts elapsed ticks: 1200 elapsed means game tick 1201.
    wait(&first, 1_000)?;
    let consumed = goal(&first, &digest)?;
    assert_eq!(consumed["predicate_truth"], "false", "{consumed}");
    assert_eq!(consumed["status"], "no_longer_holds");
    assert_eq!(
        consumed["first_satisfied_anchor"],
        achieved["first_satisfied_anchor"]
    );
    assert_eq!(consumed["physical_quiescent"], true);
    assert_eq!(consumed["needs_replan"], true);
    assert_eq!(consumed["replacement_work_dispatched"], false);
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = identity(&open("7843002", false, true, true)?, "session_id")?;
    let recovered = goal(&second, &digest)?;
    assert_eq!(recovered["predicate_truth"], "false");
    assert_eq!(recovered["historical_achievement"], "verified");
    assert_eq!(
        recovered["first_satisfied_anchor"],
        achieved["first_satisfied_anchor"]
    );
    assert_eq!(recovered["original_source"]["kind"], "production");
    assert_eq!(
        recovered["original_source"]["request"]["quotas"][0]["minimum"],
        60
    );
    assert_eq!(recovered["physical_quiescent"], true);
    Ok(())
}

#[test]
fn shared_durable_goal_owner_and_first_proof_follow_the_canonical_fortress() -> TestResult {
    let _serial = serialized_durable_tests();
    let _directory = StateDir::new("shared");
    let first = identity(&open("7843003", true, true, true)?, "session_id")?;
    let digest = production(&first)?;
    wait(&first, 100)?;
    let before_join = goal(&first, &digest)?;
    assert_eq!(before_join["predicate_truth"], "false");
    let joined = open("7843003", true, true, true)?;
    let second = identity(&joined, "session_id")?;
    assert_eq!(joined["shared_world"]["joined_existing"], true);
    assert_eq!(joined["objectives"][0]["owner_session_id"], first);
    assert_eq!(joined["anchor"], before_join["observed_anchor"]);
    wait(&second, 100)?;
    let by_peer = goal(&second, &digest)?;
    assert_eq!(by_peer["predicate_truth"], "true", "{by_peer}");
    assert_eq!(by_peer["owner_session_id"], first);
    assert_eq!(by_peer["owned_by_current_session"], false);
    assert_eq!(by_peer["historical_achievement"], "verified");
    let by_owner = goal(&first, &digest)?;
    assert_eq!(
        by_owner["first_satisfied_anchor"],
        by_peer["first_satisfied_anchor"]
    );
    assert_eq!(by_owner["owned_by_current_session"], true);
    assert_eq!(by_owner["original_source"], by_peer["original_source"]);
    Ok(())
}

#[test]
fn expired_observe_cannot_certify_goal_history_before_or_after_restart() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("authority");
    let first = identity(&open("7843004", false, true, true)?, "session_id")?;
    let digest = production(&first)?;
    in_session(&first, |guard| {
        for grant in &mut guard.grants {
            if grant.capability == Capability::Observe {
                grant.expires_at_tick = Some(GameTick(0));
            }
        }
        // Independent reference work can finish while this caller has no
        // Observe grant. Persistence must save that world without certifying it.
        guard.adapter.advance_ticks(200)
    })?;
    let denied = in_session(&first, |guard| Ok(objectives_json(guard)))?;
    assert_eq!(denied[0]["predicate_truth"], "unknown");
    assert!(denied[0]["first_satisfied_anchor"].is_null());
    assert_eq!(denied[0]["status"], "unavailable");
    assert!(denied[0].get("plan_digest").is_none());
    assert!(denied[0].get("original_source").is_none());
    assert!(denied[0].get("owner_session_id").is_none());
    assert!(denied[0].get("recorded_first_satisfied_anchor").is_none());
    simulate_durable_restart(Some(directory.0.clone()));
    let unauthorized = open("7843004", false, true, false)?;
    assert_eq!(unauthorized["objectives"][0]["predicate_truth"], "unknown");
    assert!(unauthorized["objectives"][0]["first_satisfied_anchor"].is_null());
    assert!(
        unauthorized["objectives"][0]
            .get("original_source")
            .is_none()
    );
    assert!(unauthorized["objectives"][0].get("sealed_anchor").is_none());
    assert!(unauthorized["durable"]["retained_objectives"].is_null());
    let authorized = open("7843004", false, true, true)?;
    let second = identity(&authorized, "session_id")?;
    let proven = goal(&second, &digest)?;
    assert_eq!(proven["predicate_truth"], "true");
    assert_eq!(proven["historical_achievement"], "verified");
    assert_eq!(proven["first_satisfied_anchor"], authorized["anchor"]);
    assert_ne!(proven["first_satisfied_anchor"], unauthorized["anchor"]);
    Ok(())
}

#[test]
fn unresolved_goal_capacity_refuses_before_effects_and_preserves_pending_plan() -> TestResult {
    let session = identity(&open("7843005", false, false, false)?, "session_id")?;
    for index in 0..dfmcp_lab::durable::MAX_OBJECTIVES_PER_FORTRESS {
        actions(&session, if index % 2 == 0 { BREW_OFF } else { BREW_ON })?;
    }
    // Every receipt is terminal, but no Observe authority certified any
    // original goal. Receipt completion alone must not make history evictable.
    let planned = production_plan(&session)?;
    let digest = identity(&planned, "plan_digest")?;
    let before = in_session(&session, |guard| {
        Ok((
            guard.adapter.snapshot().clone(),
            guard.last_action,
            guard.commit_receipts.len(),
            guard.leases.by_action.len(),
        ))
    })?;
    let refused = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    in_session(&session, |guard| {
        assert_eq!(guard.adapter.snapshot(), &before.0);
        assert_eq!(guard.last_action, before.1);
        assert_eq!(guard.commit_receipts.len(), before.2);
        assert_eq!(guard.leases.by_action.len(), before.3);
        assert_eq!(
            guard
                .pending
                .as_ref()
                .map(|pending| pending.digest.as_str()),
            Some(digest.as_str())
        );
        assert_eq!(
            guard.objectives.len(),
            dfmcp_lab::durable::MAX_OBJECTIVES_PER_FORTRESS
        );
        assert!(objectives::newly_satisfied_objectives(guard).is_empty());
        Ok(())
    })?;
    Ok(())
}

#[test]
fn restore_abandonment_and_world_share_a_crash_boundary_without_reanchoring_history() -> TestResult
{
    let _serial = serialized_durable_tests();
    for budget in [0, 1] {
        let directory = StateDir::new(&format!("restore-{budget}"));
        let selector = format!("784301{budget}");
        let first = identity(&open(&selector, false, true, true)?, "session_id")?;
        let checkpoint = parsed(&fortress_checkpoint(
            Some(first.clone()),
            Some("before goal".to_owned()),
        ))?;
        assert_eq!(checkpoint["ok"], true, "{checkpoint}");
        let (digest, _) = actions(&first, BREW_OFF)?;
        let achieved = goal(&first, &digest)?;
        inject_durable_crash_after(budget);
        let restored = parsed(&fortress_restore(
            Some(first),
            identity(&checkpoint, "checkpoint_id")?,
        ))?;
        assert_eq!(restored["ok"], true, "{restored}");
        simulate_durable_restart(Some(directory.0.clone()));
        let second = identity(&open(&selector, false, true, true)?, "session_id")?;
        let recovered = goal(&second, &digest)?;
        assert_eq!(
            recovered["first_satisfied_anchor"],
            achieved["first_satisfied_anchor"]
        );
        assert_eq!(recovered["historical_achievement"], "verified");
        assert_eq!(recovered["replacement_work_dispatched"], false);
        if budget == 0 {
            assert!(recovered["restore_abandoned_anchor"].is_null());
            assert_eq!(recovered["predicate_truth"], "true");
        } else {
            assert_eq!(
                recovered["restore_abandoned_anchor"],
                restored["restored_anchor"]
            );
            assert_eq!(recovered["status"], "abandoned");
            assert_eq!(recovered["predicate_truth"], "unknown");
            assert_eq!(recovered["pursuit_active"], false);
            assert_eq!(recovered["needs_replan"], false);
            assert_eq!(recovered["abandonment_publication_pending"], false);
        }
    }
    Ok(())
}

#[test]
fn unverifiable_archived_goal_or_first_proof_remains_indeterminate() -> TestResult {
    let _serial = serialized_durable_tests();
    let _directory = StateDir::new("invalid");
    let session = identity(&open("7843006", false, true, true)?, "session_id")?;
    let (digest, _) = actions(&session, BREW_OFF)?;
    let fortress = in_session(&session, |guard| Ok(guard.fortress_id))?;
    let digest = Digest32::from_hex(&digest).ok_or("digest")?;
    for corrupt_source in [false, true] {
        let invalid = with_durable_store(|store| {
            let mut retained = store
                .objective(fortress, digest)
                .cloned()
                .ok_or_else(|| fixture_error("retained objective missing"))?;
            if corrupt_source {
                retained.source = dfmcp_lab::durable::DurablePlanSource::Actions {
                    summary: "wrong source".to_owned(),
                    raw: "[]".to_owned(),
                };
            } else {
                // BREW was still enabled at the seal: this anchor cannot
                // establish the original off predicate despite valid bytes.
                retained.first_satisfied_anchor =
                    Some(store.load_snapshot(retained.sealed_state_hash)?.anchor());
            }
            Ok(objectives::recover_objective(store, &retained))
        })?;
        let shown = in_session(&session, |guard| {
            let original = std::mem::replace(&mut guard.objectives, vec![invalid]);
            let shown = objectives_json(guard);
            assert!(objectives::newly_satisfied_objectives(guard).is_empty());
            guard.objectives = original;
            Ok(shown)
        })?;
        assert_eq!(shown[0]["status"], "indeterminate");
        assert_eq!(shown[0]["predicate_truth"], "unknown");
        assert!(shown[0]["first_satisfied_anchor"].is_null());
        assert_eq!(shown[0]["historical_achievement"], "unverifiable");
        assert_eq!(shown[0]["replacement_work_dispatched"], false);
        assert_eq!(shown[0]["blind_retry_allowed"], false);
    }
    Ok(())
}

#[test]
fn achieved_quiet_goals_cannot_free_capacity_after_consumption_or_lost_stock_evidence() -> TestResult
{
    use dfmcp_intent::effects;
    use dfmcp_world::{Fact, FactSource, Value as WorldValue};

    let _serial = serialized_durable_tests();
    let directory = StateDir::new("unmet-capacity");
    let first = identity(&open("7843007", false, true, true)?, "session_id")?;
    let mut digests = Vec::new();
    // Distinct seals preserve distinct original effect identities, even when
    // every request shares the same quota. Nothing is produced before time
    // advances, so all 64 goals are initially unmet.
    for _ in 0..dfmcp_lab::durable::MAX_OBJECTIVES_PER_FORTRESS {
        digests.push(production(&first)?);
    }
    wait(&first, 200)?;
    let drained = parsed(&cancel_in_scope(
        Some(first.clone()),
        Some("stop_future_steps".to_owned()),
        Some("session".to_owned()),
    ))?;
    assert_eq!(drained["ok"], true, "{drained}");
    assert_eq!(drained["drain_progress"]["quiescent"], true, "{drained}");
    let achieved = in_session(&first, |guard| Ok(objectives_json(guard)))?;
    for row in achieved.as_array().ok_or("goals")? {
        assert_eq!(row["predicate_truth"], "true", "{row}");
        assert_eq!(row["historical_achievement"], "verified", "{row}");
        assert_eq!(row["physical_quiescent"], true, "{row}");
    }
    // Every original effect is now stopped. At 1200 elapsed ticks the dwarves
    // consume drink, making all of those historically proven quotas false.
    wait(&first, 1_000)?;
    simulate_durable_restart(Some(directory.0.clone()));
    let session = identity(&open("7843007", false, true, true)?, "session_id")?;
    for unknown in [false, true] {
        if unknown {
            in_session(&session, |guard| {
                let mut injected = guard.adapter.snapshot().clone();
                injected
                    .graph
                    .entities
                    .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
                    .ok_or_else(|| fixture_error("stock ledger missing"))?
                    .fields
                    .insert(
                        effects::STOCK_DRINK_FIELD.to_owned(),
                        Fact::known(
                            WorldValue::U64(60),
                            injected.tick,
                            FactSource::AgentAssertion(
                                "stock count is not certified observation".to_owned(),
                            ),
                            Digest32::ZERO,
                        ),
                    );
                injected.cursor = injected
                    .cursor
                    .checked_next()
                    .ok_or_else(|| fixture_error("fixture cursor exhausted"))?;
                injected.refresh_hash();
                guard.adapter.inject_snapshot(injected)
            })?;
        }
        let now = in_session(&session, |guard| Ok(objectives_json(guard)))?;
        assert_eq!(now.as_array().map(Vec::len), Some(digests.len()));
        for row in now.as_array().ok_or("goals")? {
            assert_eq!(
                row["predicate_truth"],
                if unknown { "unknown" } else { "false" },
                "{row}"
            );
            assert_eq!(row["historical_achievement"], "verified", "{row}");
            assert_eq!(row["physical_quiescent"], true, "{row}");
        }
        // An unrelated plan remains plannable with unknown stock, but goal
        // admission must still refuse it before any clock effect or receipt.
        let planned = parsed(&plan_request(
            Some(session.clone()),
            None,
            Some(true),
            None,
            None,
            None,
        ))?;
        assert_eq!(planned["ok"], true, "{planned}");
        let digest = identity(&planned, "plan_digest")?;
        let before = in_session(&session, |guard| Ok(guard.adapter.snapshot().clone()))?;
        let refused = parsed(&fortress_commit(Some(session.clone()), digest.clone()))?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "budget_exceeded");
        in_session(&session, |guard| {
            assert_eq!(guard.adapter.snapshot(), &before);
            assert_eq!(guard.objectives.len(), digests.len());
            assert_eq!(
                guard
                    .pending
                    .as_ref()
                    .map(|pending| pending.digest.as_str()),
                Some(digest.as_str())
            );
            assert!(
                guard.commit_receipts.is_empty(),
                "recovery must not redispatch any original plan"
            );
            Ok(())
        })?;
    }
    Ok(())
}
