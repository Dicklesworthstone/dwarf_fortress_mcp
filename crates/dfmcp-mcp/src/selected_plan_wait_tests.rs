//! Exact original selection crosses real commit, progression and goal custody.

use super::*;
use crate::server;
use dfmcp_intent::effects;
use dfmcp_world::Value as WorldValue;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn fixture_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn in_session<T>(session: &str, body: impl FnOnce(&mut LabSession) -> Result<T>) -> TestResult<T> {
    let handle = server::resolve_session(Some(session.to_owned()))?;
    Ok(server::with_session(
        &handle,
        || Err(fixture_error("selected wait fixture session unavailable")),
        body,
    )?)
}

fn open(selector: &str, scenario: &str) -> TestResult<String> {
    let opened = parsed(&server::open_session_in_scenario(
        Some(false),
        Some(selector.to_owned()),
        Some(
            [
                ("observe", "read_only"),
                ("query", "read_only"),
                ("plan", "reversible"),
                ("control_clock", "reversible"),
                ("checkpoint", "guarded"),
                ("designate", "guarded"),
                ("construct", "guarded"),
                ("configure_production", "reversible"),
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
        Some(scenario.to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(opened["session_id"]
        .as_str()
        .ok_or("session missing")?
        .to_owned())
}

fn submit(session: &str, actions: Option<Value>, blueprint: Option<Value>) -> TestResult<String> {
    let planned = parsed(&server::plan_request(
        Some(session.to_owned()),
        None,
        None,
        actions.map(|value| value.to_string()),
        blueprint.map(|value| value.to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = planned["plan_digest"]
        .as_str()
        .ok_or("digest missing")?
        .to_owned();
    let committed = parsed(&server::fortress_commit(
        Some(session.to_owned()),
        digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    Ok(digest)
}

fn wait(session: &str, digest: &str, ticks: u64) -> TestResult<Value> {
    let result = parsed(&server::wait_for_plan(
        Some(session.to_owned()),
        Some(ticks),
        digest.to_owned(),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

fn brew(name: &str, amount: u32) -> Value {
    json!([{"action":{"kind":"create_work_order", "name":name,
        "job_token":"BREW_DRINK", "amount":amount}}])
}

#[test]
fn an_older_selected_plan_finishes_while_other_retained_work_keeps_running() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855301", "starter_fortress")?;
    let original = submit(&session, Some(brew("selected original", 1)), None)?;
    let later = submit(&session, Some(brew("unrelated larger order", 20)), None)?;
    let latest = in_session(&session, |guard| {
        guard
            .last_action
            .ok_or_else(|| fixture_error("latest action missing"))
    })?;
    let waited = wait(&session, &original, 2_000)?;
    let selected = &waited["selected_plan_wait"];
    assert_eq!(selected["requested_plan_digest"], original);
    assert_ne!(selected["plan_digest"], later);
    assert_eq!(selected["stop_reason"], "complete", "{waited}");
    assert_eq!(selected["actions_verified"], true);
    assert_eq!(selected["physical_quiescent"], true);
    assert_eq!(selected["selection_scope"], "exact_original_commit");
    assert_eq!(selected["descendant_plans_included"], false);
    assert!(waited["advanced_game_ticks"].as_u64().is_some_and(|ticks| ticks < 2_000));
    assert_eq!(waited["action_id"], latest.to_string());
    in_session(&session, |guard| {
        assert_eq!(guard.last_action, Some(latest));
        assert!(guard.open_actions.contains(&latest));
        assert!(matches!(
            guard.adapter.action_work_state(latest)?,
            EffectWorkState::Active { .. }
        ));
        assert_ne!(
            guard.adapter.action_receipt(latest).map(|receipt| receipt.state),
            Some(CommitState::Cancelled)
        );
        Ok(())
    })?;
    let already_complete = wait(&session, &original, 2_000)?;
    assert_eq!(already_complete["advanced_game_ticks"], 0);
    assert_eq!(already_complete["wait_progress"]["observation_rounds"], 0);
    assert_eq!(already_complete["selected_plan_wait"]["stop_reason"], "complete");
    Ok(())
}

#[test]
fn reserve_wait_observes_the_exact_original_frontier_and_never_uses_late_stock() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855302", "starter_fortress")?;
    let until = 1_234;
    let original = submit(&session, None, Some(json!({
        "template":"production", "reserve_until_tick":until,
        "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
    })))?;
    let initial_tick = in_session(&session, |guard| Ok(guard.adapter.snapshot().tick.0))?;
    let bounded = wait(&session, &original, until - initial_tick - 1)?;
    assert_eq!(bounded["selected_plan_wait"]["stop_reason"], "tick_limit", "{bounded}");
    assert_eq!(bounded["selected_plan_wait"]["reserve_frontier"]["available"], false);
    assert_eq!(bounded["game_tick"], until - 1);
    let waited = wait(&session, &original, 2_000)?;
    let selected = &waited["selected_plan_wait"];
    assert_eq!(selected["stop_reason"], "complete", "{waited}");
    assert_eq!(waited["game_tick"], until);
    assert_eq!(waited["advanced_game_ticks"], 1);
    assert_eq!(selected["reserve_frontier"]["observed_anchor"]["game_tick"], until);
    assert_eq!(selected["reserve_frontier"]["truth"], "true");
    assert_eq!(selected["reserve_frontier"]["continuous_maintenance_proven"], false);
    assert_eq!(selected["reserve_frontier"]["changes_action_deadlines"], false);

    let later = parsed(&server::wait_with_ticks(Some(session.clone()), Some(1)))?;
    assert_eq!(later["ok"], true, "{later}");
    let late = wait(&session, &original, 100)?;
    assert_eq!(late["advanced_game_ticks"], 0);
    assert_eq!(late["selected_plan_wait"]["stop_reason"], "complete", "{late}");
    assert_eq!(late["selected_plan_wait"]["goal_truth"], "true");
    assert_eq!(late["selected_plan_wait"]["reserve_frontier"]["reused_from_prior_wait"], true);
    assert_eq!(
        late["selected_plan_wait"]["reserve_frontier"]["observed_anchor"],
        selected["reserve_frontier"]["observed_anchor"]
    );

    // An original without a witnessed endpoint cannot borrow the later stock
    // or the other plan's retained observation, even in the same fortress model.
    let unwitnessed = open("7855305", "starter_fortress")?;
    let original = submit(&unwitnessed, None, Some(json!({
        "template":"production", "reserve_until_tick":until,
        "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
    })))?;
    let elapsed = parsed(&server::wait_with_ticks(Some(unwitnessed.clone()), Some(2_000)))?;
    assert_eq!(elapsed["ok"], true, "{elapsed}");
    let late = wait(&unwitnessed, &original, 100)?;
    assert_eq!(late["advanced_game_ticks"], 0);
    assert_eq!(late["selected_plan_wait"]["stop_reason"], "reserve_unobserved", "{late}");
    assert_eq!(late["selected_plan_wait"]["goal_truth"], "true");
    assert_eq!(late["selected_plan_wait"]["complete"], false);
    assert_eq!(late["selected_plan_wait"]["reserve_frontier"]["available"], false);
    Ok(())
}

#[test]
fn verified_room_inputs_stop_for_review_without_claiming_a_complete_room() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855303", "starter_fortress_resources")?;
    in_session(&session, |guard| {
        let mut snapshot = guard.adapter.snapshot().clone();
        let ledger = effects::stock_ledger(&snapshot)
            .ok_or_else(|| fixture_error("stock ledger missing"))?;
        let stock = snapshot
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| fixture_error("stock missing"))?;
        stock
            .fields
            .get_mut(effects::STOCK_WOOD_FIELD)
            .ok_or_else(|| fixture_error("wood missing"))?
            .value = WorldValue::U64(0);
        stock.revision += 1;
        snapshot
            .graph
            .entities
            .remove(&crate::lab_world::starter::CARPENTERS);
        snapshot.cursor = snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| fixture_error("cursor exhausted"))?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)
    })?;
    let original = submit(&session, None, Some(json!({
        "template":"provisioned_room",
        "layout":{"template":"bedroom_cluster", "origin":[2,6,10], "rooms":2, "room_size":[3,3]},
        "acquire_resources":{"assign_labor":false},
        "prerequisites":{"assign_labor":false,"workshops":[{
            "building":"workshop:Carpenters", "location":[0,0,10],
        }]},
    })))?;
    let waited = wait(&session, &original, 2_000)?;
    let selected = &waited["selected_plan_wait"];
    assert_eq!(selected["stop_reason"], "needs_continuation", "{waited}");
    assert_eq!(selected["stage_goal_truth"], "true");
    assert_eq!(selected["goal_truth"], "false");
    assert_eq!(selected["actions_verified"], true);
    assert_eq!(selected["physical_quiescent"], true);
    assert_eq!(selected["complete"], false);
    assert_eq!(selected["automatic_continuation"], false);
    assert_eq!(selected["continuation_request"], selected["next_step"]);
    assert_eq!(selected["continuation_request"]["tool"], "fortress.plan");
    assert!(waited["advanced_game_ticks"].as_u64().is_some_and(|ticks| ticks < 2_000));
    in_session(&session, |guard| {
        assert!(guard.pending.is_none(), "waiting cannot install an unreviewed successor");
        assert_eq!(guard.objectives.len(), 1);
        Ok(())
    })?;
    Ok(())
}

#[test]
fn selected_authority_complete_source_and_commit_custody_are_checked_before_time() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7855304", "starter_fortress")?;
    let original = submit(&session, Some(brew("unchanged on refusal", 2)), None)?;
    let before = in_session(&session, |guard| Ok(guard.adapter.snapshot().clone()))?;
    let (grants, budget, receipts) = in_session(&session, |guard| {
        Ok((guard.grants.clone(), guard.budget, guard.commit_receipts.clone()))
    })?;
    for (defect, code) in [
        ("query", "capability_denied"),
        ("entities", "budget_exceeded"),
        ("bytes", "budget_exceeded"),
        ("custody", "conflict"),
    ] {
        in_session(&session, |guard| {
            guard.grants = grants.clone();
            guard.budget = budget;
            guard.commit_receipts = receipts.clone();
            match defect {
                "query" => guard.grants.retain(|grant| grant.capability != Capability::Query),
                "entities" => guard.budget.max_entities = 1,
                "bytes" => guard.budget.max_bytes = 1_024,
                "custody" => guard.commit_receipts.clear(),
                _ => return Err(fixture_error("invalid fixture defect")),
            }
            Ok(())
        })?;
        let refused = parsed(&server::wait_for_plan(
            Some(session.clone()),
            Some(2_000),
            original.clone(),
        ))?;
        assert_eq!(refused["ok"], false, "{defect}: {refused}");
        assert_eq!(refused["error"]["code"], code, "{defect}: {refused}");
        assert_eq!(
            in_session(&session, |guard| Ok(guard.adapter.snapshot().clone()))?,
            before
        );
    }
    in_session(&session, |guard| {
        guard.grants = grants;
        guard.budget = budget;
        guard.commit_receipts = receipts;
        Ok(())
    })?;
    assert_eq!(wait(&session, &original, 2_000)?["selected_plan_wait"]["stop_reason"], "complete");
    Ok(())
}

fn verified() -> Actions {
    Actions { total: 2, terminal: 2, verified: 2, ..Actions::default() }
}

#[test]
fn goal_proof_and_physical_quiet_are_independent_completion_requirements() {
    let complete = verified();
    assert_eq!(
        stop_reason(complete, PredicateTruth::True, None, None, None, false, 1),
        Some("complete")
    );
    for incomplete in [
        Actions { terminal: 1, verified: 1, ..complete },
        Actions { active: 1, ..complete },
    ] {
        assert_eq!(
            stop_reason(incomplete, PredicateTruth::True, None, None, None, false, 1),
            None
        );
    }
    assert_eq!(
        stop_reason(complete, PredicateTruth::Unknown, None, None, None, false, 1),
        Some("goal_unknown")
    );
    assert_eq!(
        stop_reason(complete, PredicateTruth::False, None, None, None, false, 1),
        Some("goal_unmet")
    );
    for (actions, reason) in [
        (Actions { failed: true, active: 1, ..complete }, "failed"),
        (Actions { cancelled: true, ..complete }, "cancelled"),
        (Actions { cancelling: true, active: 1, ..complete }, "cancellation_pending"),
        (Actions { indeterminate: true, ..complete }, "indeterminate"),
        (Actions { unknown: 1, ..complete }, "work_unknown"),
    ] {
        assert_eq!(
            stop_reason(actions, PredicateTruth::True, None, None, None, false, 1),
            Some(reason)
        );
    }
}

#[test]
fn reserve_forecasts_and_later_truth_cannot_replace_the_exact_endpoint() {
    let actions = verified();
    assert_eq!(
        stop_reason(actions, PredicateTruth::True, Some(100), None, Some(false), false, 10),
        None
    );
    assert_eq!(
        stop_reason(actions, PredicateTruth::True, Some(100), None, Some(true), false, 10),
        Some("needs_continuation")
    );
    assert_eq!(
        stop_reason(actions, PredicateTruth::True, Some(100), None, None, true, 10),
        Some("reserve_unknown")
    );
    assert_eq!(
        stop_reason(actions, PredicateTruth::True, Some(100), None, None, false, 101),
        Some("reserve_unobserved")
    );
    for (endpoint, reason) in [
        (PredicateTruth::False, "reserve_unmet"),
        (PredicateTruth::Unknown, "reserve_unknown"),
        (PredicateTruth::True, "complete"),
    ] {
        assert_eq!(
            stop_reason(actions, PredicateTruth::True, Some(100), Some(endpoint), None, false, 101),
            Some(reason)
        );
    }
}
