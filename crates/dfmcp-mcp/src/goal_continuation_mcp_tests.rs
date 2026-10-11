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
    let waited = parsed(&wait_with_ticks(Some(session.to_owned()), Some(ticks)))?;
    assert_eq!(waited["ok"], true, "{waited}");
    Ok(waited)
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
    assert_history_unchanged(&original_history, &goal(&second, &root)?);
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
