//! Reviewed continuation crosses the public plan/commit boundary without
//! replacing original goals, importing authority, or duplicating lineage work.
use super::*;
use crate::agent_facade as facade;
use crate::lab_world::{ProductionRequest, starter};
use dfmcp_intent::effects;
use dfmcp_world::{Fact, FactSource, Value as WorldValue};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct StateDir(std::path::PathBuf);

impl StateDir {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "dfmcp-goal-continuation-{label}-{}",
            std::process::id()
        ));
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
        || Err(fixture_error("continuation fixture session unavailable")),
        body,
    )?)
}

fn open(selector: &str, shared: bool, durable: bool) -> TestResult<String> {
    let opened = parsed(&facade::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "read_only"),
                ("control_clock", "reversible"),
                ("configure_production", "reversible"),
                ("configure_labor", "reversible"),
                ("checkpoint", "guarded"),
                ("restore", "guarded"),
            ]
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
    identity(&opened, "session_id")
}

fn production(session: &str, quotas: Value) -> TestResult<Value> {
    let planned = parsed(&plan_request(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(json!({"template":"production", "quotas":quotas}).to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    Ok(planned)
}

fn continuation(session: &str, parent: &str, summary: &str) -> TestResult<Value> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        Some(summary.to_owned()),
        None,
        None,
        Some(json!({"template":"continue_goal", "plan_digest":parent}).to_string()),
    ))
}

fn commit(session: &str, planned: &Value) -> TestResult<Value> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        identity(planned, "plan_digest")?,
    ))
}

fn commit_ok(session: &str, planned: &Value) -> TestResult<Value> {
    let committed = commit(session, planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(committed)
}

fn wait(session: &str, ticks: u64) -> TestResult<Value> {
    let mut remaining = ticks;
    for _ in 0..128 {
        let waited = parsed(&wait_with_ticks(Some(session.to_owned()), Some(remaining)))?;
        if waited["ok"] == true {
            return Ok(waited);
        }
        assert_eq!(waited["error"]["code"], "budget_exceeded", "{waited}");
        let advanced = waited["advanced_game_ticks"]
            .as_u64()
            .ok_or("bounded wait refused without a progress frontier")?;
        let next = waited["wait_progress"]["remaining_game_ticks"]
            .as_u64()
            .ok_or("bounded wait refused without remaining ticks")?;
        assert!(advanced > 0 && next < remaining, "{waited}");
        assert_eq!(advanced + next, remaining);
        // This is a new explicit foreground request for only the unconsumed
        // ticks; no call's wall-time or game-time allowance is increased.
        remaining = next;
    }
    Err("wait exceeded 128 bounded continuation requests".into())
}

fn production_with_unfinished_setup(session: &str) -> TestResult<(Value, PreparedPlan)> {
    in_session(session, |guard| {
        let mut snapshot = guard.adapter.snapshot().clone();
        snapshot.graph.entities.remove(&starter::STILL);
        snapshot.cursor = snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("fixture cursor exhausted"))?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)?;
        guard.grants.push(CapabilityGrant {
            capability: Capability::Construct,
            max_risk: RiskTier::Guarded,
            scope: CapabilityScope {
                fortress_id: Some(guard.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        });
        Ok(())
    })?;
    let planned = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        None,
        None,
        None,
        Some(
            json!({
                "template":"production", "quotas":[{"item":"DRINK","minimum":50}],
                "prerequisites":{"workshops":[{"building":"workshop:Still","location":[0,0,10]}]},
            })
            .to_string(),
        ),
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let original = in_session(session, |guard| {
        guard
            .pending
            .as_ref()
            .map(|pending| pending.plan.clone())
            .ok_or_else(|| fixture_error("original setup plan missing"))
    })?;
    assert_eq!(original.steps.len(), 2);
    Ok((planned, original))
}

fn goal(session: &str, digest: &str) -> TestResult<Value> {
    in_session(session, |guard| {
        original_goal_observation(guard, digest).map(|(_, value)| value)
    })
}

#[derive(Debug, PartialEq)]
struct Effects {
    snapshot: WorldSnapshot,
    commit_receipts: BTreeMap<String, String>,
    objectives: Vec<Digest32>,
    last_action: Option<ActionId>,
    open_actions: Vec<ActionId>,
    action_leases: usize,
}

fn effects_before(session: &str) -> TestResult<Effects> {
    in_session(session, |guard| {
        Ok(Effects {
            snapshot: guard.adapter.snapshot().clone(),
            commit_receipts: guard.commit_receipts.clone(),
            objectives: guard
                .objectives
                .iter()
                .map(|objective| objective.plan_digest)
                .collect(),
            last_action: guard.last_action,
            open_actions: guard.open_actions.clone(),
            action_leases: guard.leases.by_action.len(),
        })
    })
}

fn assert_history_unchanged(before: &Value, after: &Value) {
    for field in [
        "plan_digest",
        "owner_session_id",
        "original_source",
        "sealed_anchor",
        "terminal_condition",
        "first_satisfied_anchor",
        "historical_achievement",
        "restore_abandoned_anchor",
    ] {
        assert_eq!(before[field], after[field], "history field {field}");
    }
}

fn assert_lineage(value: &Value, parent: &str, root: &str) {
    assert_eq!(value["parent_plan_digest"], parent, "{value}");
    assert_eq!(value["root_plan_digest"], root, "{value}");
    assert_eq!(value["requires_explicit_commit"], true, "{value}");
}

fn consumed_goal(session: &str) -> TestResult<(Value, Value)> {
    let planned = production(session, json!([{"item":"DRINK","minimum":60}]))?;
    let digest = identity(&planned, "plan_digest")?;
    commit_ok(session, &planned)?;
    wait(session, 200)?;
    let achieved = goal(session, &digest)?;
    assert_eq!(achieved["predicate_truth"], "true", "{achieved}");
    assert_eq!(achieved["historical_achievement"], "verified");
    wait(session, 1_000)?;
    let consumed = goal(session, &digest)?;
    assert_eq!(consumed["predicate_truth"], "false", "{consumed}");
    assert_eq!(consumed["physical_quiescent"], true, "{consumed}");
    assert_history_unchanged(&achieved, &consumed);
    Ok((planned, consumed))
}

#[test]
fn consumed_goal_continuation_requires_a_new_explicit_commit_and_keeps_original_history()
-> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7844001", false, false)?;
    let (original, history) = consumed_goal(&session)?;
    let root = identity(&original, "plan_digest")?;
    let before = effects_before(&session)?;
    let proposal = continuation(&session, &root, "replenish the original drink reserve")?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    let child = identity(&proposal, "plan_digest")?;
    assert_ne!(child, root);
    assert_lineage(&proposal["production"]["continuation"], &root, &root);
    assert_eq!(
        proposal["production"]["continuation"]["replacement_work_dispatched"],
        false
    );
    assert_eq!(
        effects_before(&session)?,
        before,
        "planning performs no effects"
    );
    in_session(&session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("reviewed continuation missing"))?;
        assert_eq!(pending.digest, child);
        assert!(
            !guard.commit_receipts.contains_key(&child),
            "a continuation proposal cannot dispatch itself"
        );
        Ok(())
    })?;
    assert_history_unchanged(&history, &goal(&session, &root)?);
    commit_ok(&session, &proposal)?;
    let child_goal = goal(&session, &child)?;
    assert_lineage(&child_goal["continuation"], &root, &root);
    assert_eq!(child_goal["predicate_truth"], "false");
    assert_eq!(
        child_goal["terminal_condition"],
        history["terminal_condition"]
    );
    wait(&session, 100)?;
    assert_eq!(goal(&session, &child)?["predicate_truth"], "true");
    let current = goal(&session, &root)?;
    assert_eq!(current["predicate_truth"], "true");
    assert_history_unchanged(&history, &current);
    in_session(&session, |guard| {
        assert_eq!(
            guard.commit_receipts.get(&root),
            before.commit_receipts.get(&root)
        );
        assert!(guard.commit_receipts.contains_key(&child));
        Ok(())
    })?;
    Ok(())
}

#[test]
fn continuation_keeps_a_quota_that_needed_no_original_order() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7844002", false, false)?;
    let original = production(
        &session,
        json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]),
    )?;
    let root = identity(&original, "plan_digest")?;
    assert_eq!(original["steps"].as_array().map(Vec::len), Some(1));
    commit_ok(&session, &original)?;
    wait(&session, 50)?;
    assert_eq!(goal(&session, &root)?["predicate_truth"], "true");
    wait(&session, 1_150)?; // A later meal consumes a reserve that originally needed no work.
    let parent = goal(&session, &root)?;
    assert_eq!(parent["predicate_truth"], "false");
    assert_eq!(parent["physical_quiescent"], true);
    let proposal = continuation(&session, &root, "retain both original reserves")?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    let requirements = proposal["production"]["requirements"]
        .as_array()
        .ok_or("production requirements missing")?;
    for (item, minimum) in [("DRINK", 40), ("FOOD", 65)] {
        let requirement = requirements
            .iter()
            .find(|requirement| requirement["item"] == item)
            .ok_or_else(|| format!("lost original quota {item}: {proposal}"))?;
        assert_eq!(requirement["minimum_stock"], minimum);
    }
    in_session(&session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("continuation plan missing"))?;
        assert!(pending.plan.steps.iter().any(|step| matches!(
            &step.action,
            Action::CreateWorkOrder { job_token, .. } if job_token == "BREW_DRINK"
        )));
        assert_eq!(
            crate::lab_world::predicate_json(&pending.plan.terminal_condition),
            parent["terminal_condition"]
        );
        Ok(())
    })?;
    commit_ok(&session, &proposal)?;
    let child = goal(&session, &identity(&proposal, "plan_digest")?)?;
    assert_eq!(child["terminal_condition"], parent["terminal_condition"]);
    assert_eq!(
        child["original_source"]["request"]["production"]["quotas"],
        parent["original_source"]["request"]["quotas"]
    );
    wait(&session, 100)?;
    assert_eq!(goal(&session, &root)?["predicate_truth"], "true");
    Ok(())
}

#[test]
fn active_true_and_unknown_original_goals_cannot_be_continued() -> TestResult {
    let _serial = serialized_durable_tests();
    let session = open("7844003", false, false)?;
    let original = production(&session, json!([{"item":"DRINK","minimum":60}]))?;
    let root = identity(&original, "plan_digest")?;
    commit_ok(&session, &original)?;
    let active = goal(&session, &root)?;
    assert_eq!(active["predicate_truth"], "false");
    assert_eq!(active["physical_quiescent"], false);
    let before = effects_before(&session)?;
    let refused = continuation(&session, &root, "cannot duplicate active work")?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "preconditions_failed");
    assert_eq!(effects_before(&session)?, before);
    wait(&session, 200)?;
    assert_eq!(goal(&session, &root)?["predicate_truth"], "true");
    let before = effects_before(&session)?;
    let satisfied = continuation(&session, &root, "no new work while the reserve holds")?;
    assert_eq!(satisfied["ok"], false, "{satisfied}");
    assert_eq!(satisfied["error"]["code"], "invalid_intent");
    assert_eq!(effects_before(&session)?, before);
    wait(&session, 1_000)?;
    in_session(&session, |guard| {
        let mut injected = guard.adapter.snapshot().clone();
        injected
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .ok_or_else(|| fixture_error("stock ledger missing"))?
            .fields
            .insert(
                effects::STOCK_DRINK_FIELD.to_owned(),
                Fact::known(
                    WorldValue::U64(53),
                    injected.tick,
                    FactSource::AgentAssertion("unobserved remaining drink".to_owned()),
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
    assert_eq!(goal(&session, &root)?["predicate_truth"], "unknown");
    let before = effects_before(&session)?;
    let unknown = continuation(&session, &root, "uncertainty cannot size replacement work")?;
    assert_eq!(unknown["ok"], false, "{unknown}");
    assert_eq!(unknown["error"]["code"], "preconditions_failed");
    assert_eq!(effects_before(&session)?, before);
    Ok(())
}

#[test]
fn continuation_rechecks_current_observe_and_plan_on_proposal_and_commit() -> TestResult {
    let _serial = serialized_durable_tests();
    for (index, capability) in [Capability::Observe, Capability::Plan]
        .into_iter()
        .enumerate()
    {
        let session = open(&format!("784401{index}"), false, false)?;
        let (original, _) = consumed_goal(&session)?;
        let root = identity(&original, "plan_digest")?;
        let proposal = continuation(&session, &root, "reviewed before authority expires")?;
        assert_eq!(proposal["ok"], true, "{proposal}");
        in_session(&session, |guard| {
            for grant in &mut guard.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let denied_plan = continuation(&session, &root, "expired authority")?;
        assert_eq!(denied_plan["ok"], false, "{denied_plan}");
        assert_eq!(denied_plan["error"]["code"], "capability_denied");
        let denied_commit = commit(&session, &proposal)?;
        assert_eq!(denied_commit["ok"], false, "{denied_commit}");
        assert_eq!(denied_commit["error"]["code"], "capability_denied");
        assert_eq!(effects_before(&session)?, before);
        in_session(&session, |guard| {
            assert_eq!(
                guard
                    .pending
                    .as_ref()
                    .map(|pending| pending.digest.as_str()),
                proposal["plan_digest"].as_str()
            );
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn shared_competing_proposals_cannot_commit_or_plan_around_an_active_sibling() -> TestResult {
    let _serial = serialized_durable_tests();
    let first = open("7844004", true, false)?;
    let (original, _) = consumed_goal(&first)?;
    let root = identity(&original, "plan_digest")?;
    let second = open("7844004", true, false)?;
    let first_proposal = continuation(&first, &root, "first continuation proposal")?;
    let peer_proposal = continuation(&second, &root, "competing peer proposal")?;
    assert_eq!(first_proposal["ok"], true, "{first_proposal}");
    assert_eq!(peer_proposal["ok"], true, "{peer_proposal}");
    assert_ne!(first_proposal["plan_digest"], peer_proposal["plan_digest"]);
    commit_ok(&first, &first_proposal)?;
    let parent = goal(&second, &root)?;
    assert_eq!(parent["predicate_truth"], "false");
    assert_eq!(parent["physical_quiescent"], true);
    let child = identity(&first_proposal, "plan_digest")?;
    assert_eq!(goal(&second, &child)?["physical_quiescent"], false);
    let before = effects_before(&second)?;
    let denied_commit = commit(&second, &peer_proposal)?;
    assert_eq!(denied_commit["ok"], false, "{denied_commit}");
    assert_eq!(denied_commit["error"]["code"], "preconditions_failed");
    assert!(denied_commit["rebased_plan"].is_null(), "{denied_commit}");
    let denied_plan = continuation(
        &second,
        &root,
        "another sibling is already pursuing the root",
    )?;
    assert_eq!(denied_plan["ok"], false, "{denied_plan}");
    assert_eq!(denied_plan["error"]["code"], "preconditions_failed");
    assert_eq!(effects_before(&second)?, before);
    in_session(&second, |guard| {
        assert_eq!(
            guard
                .pending
                .as_ref()
                .map(|pending| pending.digest.as_str()),
            peer_proposal["plan_digest"].as_str()
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn unknown_sibling_work_blocks_planning_and_a_previously_reviewed_peer_commit() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("unknown-sibling");
    let first = open("7844005", true, true)?;
    let (original, _) = consumed_goal(&first)?;
    let root = identity(&original, "plan_digest")?;
    simulate_durable_restart(Some(directory.0.clone()));
    let first = open("7844005", true, true)?;
    let second = open("7844005", true, true)?;
    let first_proposal = continuation(&first, &root, "new work from recovered original goal")?;
    let peer_proposal = continuation(&second, &root, "peer review before evidence loss")?;
    assert_eq!(first_proposal["ok"], true, "{first_proposal}");
    assert_eq!(peer_proposal["ok"], true, "{peer_proposal}");
    let child_step = in_session(&first, |guard| {
        guard
            .pending
            .as_ref()
            .and_then(|pending| {
                pending
                    .plan
                    .steps
                    .iter()
                    .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
            })
            .cloned()
            .ok_or_else(|| fixture_error("child production order missing"))
    })?;
    commit_ok(&first, &first_proposal)?;
    in_session(&first, |guard| {
        let mut injected = guard.adapter.snapshot().clone();
        let entity = effects::created_entity_id(&child_step.idempotency_key, 0);
        injected
            .graph
            .entities
            .get_mut(&entity)
            .ok_or_else(|| fixture_error("child order effect missing"))?
            .fields
            .insert(
                effects::STATUS_FIELD.to_owned(),
                Fact::known(
                    WorldValue::Text(effects::STATUS_CANCELLED.to_owned()),
                    injected.tick,
                    FactSource::AgentAssertion("uncertified stopped sibling".to_owned()),
                    Digest32::ZERO,
                ),
            );
        injected.cursor = injected
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("fixture cursor exhausted"))?;
        injected.refresh_hash();
        guard.adapter.inject_snapshot(injected)?;
        assert!(matches!(
            dfmcp_intent::inspect_effect_work(
                guard.adapter.snapshot(),
                &child_step.action,
                &child_step.idempotency_key,
                true,
            )?,
            EffectWorkState::Unknown { .. }
        ));
        Ok(())
    })?;
    let parent = goal(&second, &root)?;
    assert_eq!(parent["predicate_truth"], "false");
    assert_eq!(parent["physical_quiescent"], true, "{parent}");
    let before = effects_before(&second)?;
    let denied_plan = continuation(&second, &root, "unknown sibling cannot be retried")?;
    assert_eq!(denied_plan["ok"], false, "{denied_plan}");
    assert_eq!(denied_plan["error"]["code"], "preconditions_failed");
    let denied_commit = commit(&second, &peer_proposal)?;
    assert_eq!(denied_commit["ok"], false, "{denied_commit}");
    assert_eq!(denied_commit["error"]["code"], "preconditions_failed");
    assert!(denied_commit["rebased_plan"].is_null(), "{denied_commit}");
    assert_eq!(effects_before(&second)?, before);
    Ok(())
}

#[test]
fn durable_reopen_continues_the_flat_original_goal_and_retains_root_history() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("reopen");
    let first = open("7844006", false, true)?;
    let (original, original_history) = consumed_goal(&first)?;
    let root = identity(&original, "plan_digest")?;
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("7844006", false, true)?;
    let recovered_root = goal(&second, &root)?;
    assert_history_unchanged(&original_history, &recovered_root);
    assert_ne!(
        recovered_root["sealed_anchor"]["epoch"], recovered_root["observed_anchor"]["epoch"],
        "durable reopen preserves the old seal in a new observation epoch"
    );
    let before = effects_before(&second)?;
    assert!(
        before.commit_receipts.is_empty(),
        "recovery cannot recreate dispatch handles"
    );
    let proposal = continuation(&second, &root, "continue after durable reopen")?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    assert_eq!(effects_before(&second)?, before);
    let child = identity(&proposal, "plan_digest")?;
    commit_ok(&second, &proposal)?;
    wait(&second, 100)?;
    let child_history = goal(&second, &child)?;
    assert_eq!(child_history["predicate_truth"], "true");
    assert_lineage(&child_history["continuation"], &root, &root);
    wait(&second, 1_100)?;
    assert_eq!(goal(&second, &child)?["predicate_truth"], "false");
    with_durable_store(|store| store.compact())?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open("7844006", false, true)?;
    let recovered_child = goal(&third, &child)?;
    assert_history_unchanged(&child_history, &recovered_child);
    assert_ne!(
        recovered_child["sealed_anchor"]["epoch"], recovered_child["observed_anchor"]["epoch"],
        "each durable reopen needs its own owner-verified active lineage"
    );
    assert_history_unchanged(&original_history, &goal(&third, &root)?);
    assert_eq!(
        recovered_child["original_source"]["request"]["production"],
        original_history["original_source"]["request"]
    );
    let proposal = continuation(
        &third,
        &child,
        "second generation retains the original reserve",
    )?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    assert_lineage(&proposal["production"]["continuation"], &child, &root);
    commit_ok(&third, &proposal)?;
    let grandchild = goal(&third, &identity(&proposal, "plan_digest")?)?;
    assert_lineage(&grandchild["continuation"], &child, &root);
    assert_eq!(
        grandchild["original_source"]["request"]["production"],
        original_history["original_source"]["request"]
    );
    assert!(
        grandchild["original_source"]["request"]["production"]["production"].is_null(),
        "continuations must retain one complete flat request"
    );
    wait(&third, 100)?;
    assert_eq!(goal(&third, &root)?["predicate_truth"], "true");
    assert_history_unchanged(&original_history, &goal(&third, &root)?);
    Ok(())
}

#[test]
fn unfinished_original_setup_can_continue_on_both_recovery_openings() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("retired-setup");
    let first = open("7844020", false, true)?;
    let (planned, original) = production_with_unfinished_setup(&first)?;
    let root = identity(&planned, "plan_digest")?;
    let order = original
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
        .ok_or("original order missing")?;
    let old_key = order.idempotency_key.clone();
    let order_step = order.id;
    commit_ok(&first, &planned)?;
    assert_eq!(goal(&first, &root)?["physical_quiescent"], false);
    simulate_durable_restart(Some(directory.0.clone()));
    let second = open("7844020", false, true)?;
    in_session(&second, |guard| {
        let row = guard
            .carried
            .iter()
            .find(|row| row.step == Some(order_step))
            .ok_or_else(|| fixture_error("first recovery lost nondispatch custody"))?;
        assert!(row.is_final());
        assert_eq!(row.work_state, EffectWorkState::NeverDispatched);
        assert!(row.proof_anchor.is_some());
        assert!(guard.last_action.is_none());
        Ok(())
    })?;
    let blocked = continuation(&second, &root, "construction must finish first")?;
    assert_eq!(blocked["ok"], false, "{blocked}");
    assert_eq!(blocked["error"]["code"], "preconditions_failed");
    wait(&second, effects::BUILD_TICKS)?;
    let after_build = goal(&second, &root)?;
    assert_eq!(after_build["predicate_truth"], "false", "{after_build}");
    assert_eq!(after_build["physical_quiescent"], true, "{after_build}");
    let before = effects_before(&second)?;
    let first_review = continuation(&second, &root, "review after first recovery")?;
    assert_eq!(first_review["ok"], true, "{first_review}");
    assert_eq!(effects_before(&second)?, before);
    with_durable_store(|store| {
        let retained = store
            .objective(original.anchor.fortress_id, original.digest)
            .ok_or_else(|| fixture_error("durable original goal missing"))?;
        assert!(
            retained
                .retired_never_dispatched
                .contains_key(&order_step.get())
        );
        assert!(
            store
                .commit(original.anchor.fortress_id, original.digest)
                .is_none()
        );
        store.compact()
    })?;
    simulate_durable_restart(Some(directory.0.clone()));
    let third = open("7844020", false, true)?;
    let recovered = goal(&third, &root)?;
    assert_eq!(recovered["physical_quiescent"], true, "{recovered}");
    assert_eq!(recovered["predicate_truth"], "false");
    assert_eq!(
        recovered["retired_undispatched_steps"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        recovered["retired_undispatched_steps"][0]["step"],
        order_step.get()
    );
    assert_eq!(recovered["original_source"], after_build["original_source"]);
    in_session(&third, |guard| {
        assert!(
            guard.carried.is_empty(),
            "retired goal custody must survive without live commit recovery"
        );
        assert!(guard.commit_receipts.is_empty());
        assert!(
            !guard
                .adapter
                .snapshot()
                .graph
                .entities
                .contains_key(&effects::created_entity_id(&old_key, 0))
        );
        Ok(())
    })?;
    let next = continuation(&third, &root, "finish the original stock goal")?;
    assert_eq!(next["ok"], true, "{next}");
    commit_ok(&third, &next)?;
    wait(&third, 200)?;
    assert_eq!(goal(&third, &root)?["predicate_truth"], "true");
    in_session(&third, |guard| {
        assert!(
            !guard
                .adapter
                .snapshot()
                .graph
                .entities
                .contains_key(&effects::created_entity_id(&old_key, 0)),
            "continuation must never redispatch the original deferred key"
        );
        Ok(())
    })?;
    Ok(())
}

#[test]
fn first_recovery_rejects_unanchored_future_and_contradicted_nondispatch() -> TestResult {
    let _serial = serialized_durable_tests();
    for (index, mode) in [
        "unanchored",
        "future",
        "archived_creator",
        "current_creator",
    ]
    .into_iter()
    .enumerate()
    {
        let directory = StateDir::new(mode);
        let selector = format!("784403{index}");
        let first = open(&selector, false, true)?;
        let (planned, original) = production_with_unfinished_setup(&first)?;
        let root = identity(&planned, "plan_digest")?;
        let order = original
            .steps
            .iter()
            .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
            .ok_or("original order missing")?;
        let (sealed, source) = in_session(&first, |guard| {
            let pending = guard
                .pending
                .as_ref()
                .ok_or_else(|| fixture_error("original source missing"))?;
            Ok((guard.adapter.snapshot().clone(), pending.source.durable()))
        })?;
        let mut changed = sealed.clone();
        changed.cursor = changed
            .cursor
            .checked_next()
            .ok_or("fixture cursor exhausted")?;
        if mode == "future" {
            changed.tick = GameTick(changed.tick.0 + 10);
        } else {
            let mut entity = changed
                .graph
                .entities
                .get(&starter::STOCK_LEDGER)
                .cloned()
                .ok_or("fixture ledger missing")?;
            entity.id = effects::created_entity_id(&order.idempotency_key, 0);
            changed.graph.entities.insert(entity.id, entity);
        }
        changed.refresh_hash();
        with_durable_store(|store| {
            store.persist_objective_commit(
                &sealed,
                original.digest,
                original.intent_id.get(),
                source,
                SessionId::new(99),
                &[],
            )?;
            if mode == "unanchored" {
                store.persist_step(
                    sealed.fortress_id,
                    original.digest,
                    order.id.get(),
                    "not_dispatched",
                )?;
            } else {
                let updates: Vec<_> = original
                    .steps
                    .iter()
                    .map(|step| dfmcp_lab::durable::DurableStepUpdate {
                        plan_digest: original.digest,
                        step: step.id.get(),
                        state: "not_dispatched".to_owned(),
                    })
                    .collect();
                let archive = if mode == "current_creator" {
                    &sealed
                } else {
                    &changed
                };
                store.persist_progress("starter_fortress", archive, &updates, &[])?;
                if mode == "future" {
                    store.persist_head("starter_fortress", &sealed)?;
                } else if mode == "current_creator" {
                    store.persist_head("starter_fortress", &changed)?;
                }
            }
            Ok(())
        })?;
        simulate_durable_restart(Some(directory.0.clone()));
        let second = open(&selector, false, true)?;
        in_session(&second, |guard| {
            let carried = guard
                .carried
                .iter()
                .find(|row| row.step == Some(order.id))
                .ok_or_else(|| fixture_error("uncertain original step disappeared"))?;
            assert_eq!(carried.state, "indeterminate", "{mode}: {carried:?}");
            assert!(!carried.is_final());
            Ok(())
        })?;
        assert_eq!(goal(&second, &root)?["physical_quiescent"], false, "{mode}");
        let proposal = continuation(&second, &root, "unknown original work cannot be duplicated")?;
        assert_eq!(proposal["ok"], false, "{mode}: {proposal}");
        assert_eq!(proposal["error"]["code"], "preconditions_failed");
    }
    Ok(())
}

#[test]
fn contradicted_custody_cannot_fall_back_to_completed_original_work() -> TestResult {
    let _serial = serialized_durable_tests();
    for (index, mode) in ["carried", "indeterminate", "retired"]
        .into_iter()
        .enumerate()
    {
        let directory = StateDir::new(&format!("completed-contradiction-{mode}"));
        let selector = format!("784404{index}");
        let first = open(&selector, false, true)?;
        let (planned, original) = production_with_unfinished_setup(&first)?;
        let root = identity(&planned, "plan_digest")?;
        let order = original
            .steps
            .iter()
            .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
            .ok_or("original order missing")?;
        commit_ok(&first, &planned)?;
        simulate_durable_restart(Some(directory.0.clone()));
        let second = open(&selector, false, true)?;
        wait(&second, effects::BUILD_TICKS)?;
        assert_eq!(goal(&second, &root)?["physical_quiescent"], true);
        let session = if mode == "retired" {
            simulate_durable_restart(Some(directory.0.clone()));
            open(&selector, false, true)?
        } else {
            second
        };
        in_session(&session, |guard| {
            let mut shadow = guard.adapter.snapshot().clone();
            // Inject an actual completed creator under the old key, as a
            // contradiction of retained nondispatch. This is a fault fixture,
            // not a malformed entity whose work inspection happens to fail.
            effects::apply_effect(&mut shadow, &order.action, &order.idempotency_key)?;
            let amount = match order.action {
                Action::CreateWorkOrder { amount, .. } => u64::from(amount),
                _ => return Err(fixture_error("fixture original action is not a work order")),
            };
            let elapsed =
                amount * effects::WORK_ORDER_TICKS_PER_UNIT + effects::DRINK_INTERVAL_TICKS;
            shadow.tick = GameTick(shadow.tick.0 + elapsed);
            effects::advance_effects(&mut shadow, elapsed)?;
            shadow.cursor = shadow
                .cursor
                .checked_next()
                .ok_or_else(|| fixture_error("fixture cursor exhausted"))?;
            shadow.refresh_hash();
            // The quota planner includes consumption reserve. Advance actual
            // reference metabolism until that reserve is exhausted instead of
            // fabricating a low stock count in this fault fixture.
            for _ in 0..16 {
                shadow.refresh_hash();
                if PredicateEvidence::laboratory(&shadow)?.evaluate(&original.terminal_condition)?
                    == PredicateTruth::False
                {
                    break;
                }
                shadow.tick = GameTick(shadow.tick.0 + effects::DRINK_INTERVAL_TICKS);
                effects::advance_effects(&mut shadow, effects::DRINK_INTERVAL_TICKS)?;
            }
            for step in &original.steps {
                assert!(
                    dfmcp_intent::inspect_effect_work(
                        &shadow,
                        &step.action,
                        &step.idempotency_key,
                        true,
                    )?
                    .is_quiescent(),
                    "every actual creator is physically complete"
                );
            }
            assert_eq!(
                PredicateEvidence::laboratory(&shadow)?.evaluate(&original.terminal_condition)?,
                PredicateTruth::False,
                "consumption leaves the original goal unmet again"
            );
            if mode == "indeterminate" {
                let carried = guard
                    .carried
                    .iter_mut()
                    .find(|row| row.step == Some(order.id))
                    .ok_or_else(|| fixture_error("original carried step missing"))?;
                carried.state = "indeterminate".to_owned();
                carried.proof_anchor = None;
                carried.work_state = EffectWorkState::Unknown {
                    entity_id: Some(effects::created_entity_id(&order.idempotency_key, 0)),
                    reason: "injected contradictory original dispatch custody".to_owned(),
                };
            }
            guard.adapter.inject_snapshot(shadow)
        })?;
        let observed = goal(&session, &root)?;
        assert_eq!(observed["predicate_truth"], "false", "{mode}: {observed}");
        assert_eq!(observed["physical_quiescent"], false, "{mode}: {observed}");
        let before = effects_before(&session)?;
        let refused = continuation(
            &session,
            &root,
            "contradictory custody requires reconciliation",
        )?;
        assert_eq!(refused["ok"], false, "{mode}: {refused}");
        assert_eq!(refused["error"]["code"], "preconditions_failed");
        assert_eq!(effects_before(&session)?, before);
    }
    Ok(())
}

#[test]
fn arbitrary_epoch_changes_cannot_rebind_live_or_recovered_original_goals() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("unestablished-epoch");
    for (index, recover) in [false, true].into_iter().enumerate() {
        let selector = format!("784402{index}");
        let first = open(&selector, false, recover)?;
        let (original, history) = consumed_goal(&first)?;
        let root = identity(&original, "plan_digest")?;
        let session = if recover {
            simulate_durable_restart(Some(directory.0.clone()));
            open(&selector, false, true)?
        } else {
            first
        };
        let proposal = continuation(&session, &root, "review before an unestablished epoch")?;
        assert_eq!(proposal["ok"], true, "{proposal}");
        in_session(&session, |guard| {
            let mut changed = guard.adapter.snapshot().clone();
            changed.cursor = changed
                .cursor
                .checked_reset_epoch()
                .ok_or_else(|| fixture_error("fixture epoch exhausted"))?;
            changed.refresh_hash();
            // This is not the durable-recovery owner path and not an explicit
            // checkpoint restore. No original goal is rebound to this epoch.
            guard.adapter.inject_snapshot(changed)
        })?;
        let observed = goal(&session, &root)?;
        assert_history_unchanged(&history, &observed);
        assert_eq!(observed["predicate_truth"], "false");
        // Injecting a snapshot clears live action handles. Recovered goals can
        // still inspect their retained physical identities; live goals must
        // conservatively keep that lost handle evidence unresolved.
        assert_eq!(observed["physical_quiescent"], recover);
        assert!(observed["restore_abandoned_anchor"].is_null());
        if recover {
            assert_eq!(observed["continuation_refused"]["code"], "stale_anchor");
        } else {
            assert!(observed["continuation_request"].is_null());
            assert!(observed["continuation_refused"].is_null());
        }
        let before = effects_before(&session)?;
        let denied_plan = continuation(&session, &root, "old history is not an epoch issuer")?;
        assert_eq!(denied_plan["ok"], false, "{denied_plan}");
        assert_eq!(denied_plan["error"]["code"], "stale_anchor");
        let denied_commit = commit(&session, &proposal)?;
        assert_eq!(denied_commit["ok"], false, "{denied_commit}");
        assert_eq!(denied_commit["error"]["code"], "stale_anchor");
        assert!(denied_commit["rebased_plan"].is_null());
        assert_eq!(effects_before(&session)?, before);
    }
    Ok(())
}

#[test]
fn recovered_goal_binding_never_restores_expired_observe_or_plan_authority() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("recovered-authority");
    let first = open("7844022", false, true)?;
    let (original, history) = consumed_goal(&first)?;
    let root = identity(&original, "plan_digest")?;
    simulate_durable_restart(Some(directory.0.clone()));
    let session = open("7844022", false, true)?;
    let proposal = continuation(
        &session,
        &root,
        "current authority is required after recovery",
    )?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    for capability in [Capability::Observe, Capability::Plan] {
        in_session(&session, |guard| {
            for grant in &mut guard.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = Some(GameTick(0));
                }
            }
            Ok(())
        })?;
        let before = effects_before(&session)?;
        let refused = continuation(&session, &root, "history is not current authority")?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "capability_denied");
        let refused = commit(&session, &proposal)?;
        assert_eq!(refused["ok"], false, "{refused}");
        assert_eq!(refused["error"]["code"], "capability_denied");
        assert_eq!(effects_before(&session)?, before);
        in_session(&session, |guard| {
            for grant in &mut guard.grants {
                if grant.capability == capability {
                    grant.expires_at_tick = None;
                }
            }
            Ok(())
        })?;
    }
    assert_history_unchanged(&history, &goal(&session, &root)?);
    commit_ok(&session, &proposal)?;
    wait(&session, 100)?;
    assert_eq!(goal(&session, &root)?["predicate_truth"], "true");
    assert_history_unchanged(&history, &goal(&session, &root)?);
    Ok(())
}

#[test]
fn restore_abandonment_cannot_be_reopened_as_a_continuation() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("abandoned");
    let session = open("7844007", false, true)?;
    let checkpoint = parsed(&fortress_checkpoint(
        Some(session.clone()),
        Some("before the original production goal".to_owned()),
    ))?;
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let (original, history) = consumed_goal(&session)?;
    let root = identity(&original, "plan_digest")?;
    let restored = parsed(&fortress_restore(
        Some(session.clone()),
        identity(&checkpoint, "checkpoint_id")?,
    ))?;
    assert_eq!(restored["ok"], true, "{restored}");
    simulate_durable_restart(Some(directory.0.clone()));
    let reopened = open("7844007", false, true)?;
    let abandoned = goal(&reopened, &root)?;
    assert_eq!(abandoned["status"], "abandoned");
    assert_eq!(abandoned["predicate_truth"], "unknown");
    assert_eq!(
        abandoned["first_satisfied_anchor"],
        history["first_satisfied_anchor"]
    );
    assert!(!abandoned["restore_abandoned_anchor"].is_null());
    let before = effects_before(&reopened)?;
    let refused = continuation(
        &reopened,
        &root,
        "restore did not authorize another pursuit",
    )?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "preconditions_failed");
    assert_eq!(effects_before(&reopened)?, before);
    Ok(())
}

#[test]
fn legacy_lowered_actions_never_acquire_an_invented_production_source() -> TestResult {
    let _serial = serialized_durable_tests();
    let directory = StateDir::new("legacy-actions");
    let session = open("7844008", false, true)?;
    let raw = in_session(&session, |guard| {
        ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"DRINK","minimum":60}]}"#,
        )?
        .compile(guard.adapter.snapshot())
        .map(|compiled| compiled.actions)
    })?;
    let original = parsed(&plan_request(
        Some(session.clone()),
        Some("legacy lowered production actions".to_owned()),
        None,
        Some(raw),
        None,
        None,
    ))?;
    assert_eq!(original["ok"], true, "{original}");
    let digest = identity(&original, "plan_digest")?;
    commit_ok(&session, &original)?;
    wait(&session, 200)?;
    wait(&session, 1_000)?;
    simulate_durable_restart(Some(directory.0.clone()));
    let reopened = open("7844008", false, true)?;
    let legacy = goal(&reopened, &digest)?;
    assert_eq!(legacy["original_source"]["kind"], "actions");
    assert!(legacy["original_source"]["request"].is_array());
    assert!(legacy["continuation"].is_null());
    let before = effects_before(&reopened)?;
    let refused = continuation(
        &reopened,
        &digest,
        "action receipts cannot restore missing quotas",
    )?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(refused["production"].is_null());
    assert_eq!(effects_before(&reopened)?, before);
    assert_eq!(
        goal(&reopened, &digest)?["original_source"],
        legacy["original_source"]
    );
    Ok(())
}

#[test]
fn admitted_continuation_exclusion_cannot_escape_into_a_new_seal_after_anchor_change() -> TestResult
{
    let _serial = serialized_durable_tests();
    let session = open("7844009", false, false)?;
    let (original, _) = consumed_goal(&session)?;
    let root = identity(&original, "plan_digest")?;
    let proposal = continuation(
        &session,
        &root,
        "retry only the exact admitted continuation",
    )?;
    assert_eq!(proposal["ok"], true, "{proposal}");
    let child = identity(&proposal, "plan_digest")?;
    let (admitted_plan, admitted_source) = in_session(&session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("pending continuation missing"))?;
        let plan = pending.plan.clone();
        let source = pending.source.clone();
        assert_eq!(plan.anchor, guard.adapter.snapshot().anchor());
        // This is the real objective installation used by durable admission
        // before dispatch. No live action handles or effect receipt exist yet.
        objectives::install_objective(guard, &plan, source.clone(), &[]);
        for step in &plan.steps {
            assert!(guard.adapter.step_receipt(plan.id, step.id).is_none());
        }
        objectives::validate_continuation(guard, &source, Some(plan.digest))?;
        let blocked = objectives::validate_continuation(guard, &source, None)
            .err()
            .ok_or_else(|| fixture_error("unresolved admitted candidate was not fenced"))?;
        assert_eq!(blocked.code, ErrorCode::PreconditionsFailed);
        assert!(blocked.message.contains(&child));
        Ok((plan, source))
    })?;
    let parent = goal(&session, &root)?;
    assert_eq!(parent["predicate_truth"], "false");
    assert_eq!(parent["physical_quiescent"], true);
    assert_eq!(goal(&session, &child)?["physical_quiescent"], false);

    // A new anchor requires a new seal. The old admitted pursuit is now a
    // separate unresolved lineage member, so it must participate in the gate.
    wait(&session, 1)?;
    let before = effects_before(&session)?;
    assert_ne!(before.snapshot.anchor(), admitted_plan.anchor);
    assert!(!before.commit_receipts.contains_key(&child));
    let refused = commit(&session, &proposal)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["error"]["code"], "preconditions_failed");
    assert!(
        refused["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains(&child)),
        "the unresolved admitted candidate must cause the refusal: {refused}"
    );
    assert!(refused["rebased_plan"].is_null(), "{refused}");
    assert_eq!(effects_before(&session)?, before);
    in_session(&session, |guard| {
        let pending = guard
            .pending
            .as_ref()
            .ok_or_else(|| fixture_error("refusal lost the pending continuation"))?;
        assert_eq!(pending.digest, child);
        assert_eq!(pending.plan, admitted_plan);
        assert_eq!(
            pending.source.continuation()?,
            admitted_source.continuation()?
        );
        for step in &admitted_plan.steps {
            assert!(
                guard
                    .adapter
                    .step_receipt(admitted_plan.id, step.id)
                    .is_none()
            );
        }
        Ok(())
    })?;
    Ok(())
}
