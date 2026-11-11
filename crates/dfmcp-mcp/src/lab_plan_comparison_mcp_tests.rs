//! Candidate comparison and fresh selection through the actual public facade.
//!
//! These exercise the default laboratory tool callbacks and their complete
//! Agent Turn publication. Pure forecast tests do not substitute for this path.

use super::*;
use dfmcp_world::Value as WorldValue;
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner {
    id: String,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(number) = u128::from_str_radix(&self.id, 16) {
            sessions().remove(&SessionId::new(number));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn text_field(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn open(paused: bool, output_tokens: Option<u32>) -> TestResult<Owner> {
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(paused),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "read_only".into()),
            ("configure_labor".into(), "reversible".into()),
            ("configure_production".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
        ]),
        Some(60_000),
        Some(12_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        output_tokens,
        Some(64),
        Some("starter_fortress".into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner {
        id: text_field(&opened, "session_id")?,
    })
}

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    let session = resolve_session(Some(owner.id.clone()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    Ok(guard.adapter.snapshot().clone())
}

fn pending_digest(owner: &Owner) -> TestResult<Option<String>> {
    let session = resolve_session(Some(owner.id.clone()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    Ok(guard.pending.as_ref().map(|pending| pending.digest.clone()))
}

fn output_budget(owner: &Owner, tokens: u32) -> TestResult {
    let session = resolve_session(Some(owner.id.clone()))?;
    let mut guard = session.lock().map_err(|_| "session poisoned")?;
    guard.budget.max_output_tokens = tokens;
    Ok(())
}

fn wrapper(candidates: Value, horizon: u64) -> Value {
    json!({
        "template": "compare_candidates",
        "version": 1,
        "horizon_ticks": horizon,
        "candidates": candidates,
    })
}

fn production_candidates() -> Value {
    json!([
        {"key": "ordinary", "blueprint": {
            "template": "production",
            "quotas": [{"item": "DRINK", "minimum": 60}],
        }},
        {"key": "conditional", "blueprint": {
            "template": "production",
            "quotas": [{"item": "DRINK", "minimum": 60}],
            "orders": "dependency_preserving_v1",
        }},
    ])
}

fn pause_candidates() -> Value {
    json!([
        {"key": "alpha", "paused_target": true},
        {"key": "zulu", "paused_target": true},
    ])
}

fn plan_blueprint(owner: &Owner, raw: &str) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        None,
        None,
        None,
        Some(raw.to_owned()),
    ))
}

fn compare(owner: &Owner, candidates: Value, horizon: u64) -> TestResult<Value> {
    plan_blueprint(owner, &wrapper(candidates, horizon).to_string())
}

fn candidate<'a>(report: &'a Value, key: &str) -> TestResult<&'a Value> {
    report["candidates"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["key"] == key))
        .ok_or_else(|| format!("missing candidate {key}: {report}").into())
}

fn selection(report: &Value, key: &str) -> TestResult<String> {
    let row = candidate(report, key)?;
    assert_eq!(row["next_step"]["tool"], "fortress.plan", "{row}");
    text_field(&row["next_step"]["arguments"], "blueprint")
}

fn pause_plan(owner: &Owner) -> TestResult<Value> {
    let result = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        Some("retain this reviewed pause plan".into()),
        Some(true),
        None,
        None,
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}

fn temporal(owner: &Owner, query: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_query(
        Some(owner.id.clone()),
        Some(
            json!({
                "mode": "temporal",
                "request": {"schema": "dfmcp.query/1", "query": query},
            })
            .to_string(),
        ),
    ))
}

fn refusal(result: &Value, code: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], code, "{result}");
    // The bounded minimal refusal may omit the Agent Turn entirely.
    if result["output_budget"]["tier"] == "refused" {
        assert_eq!(code, "budget_exceeded", "{result}");
    } else {
        assert!(result["agent_turn"].is_object(), "{result}");
    }
}

#[test]
fn public_candidate_production_compares_at_default_budget_then_selects_commits_and_proves_goal()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, None)?;
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        let guard = session.lock().map_err(|_| "session poisoned")?;
        assert_eq!(guard.budget.max_output_tokens, 4_096);
    }
    let before = snapshot(&owner)?;
    assert!(pending_digest(&owner)?.is_none());
    let raw = crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        None,
        None,
        None,
        Some(wrapper(production_candidates(), 1_000).to_string()),
    );
    assert!(
        raw.len() <= 4_096 * 4,
        "comparison exceeded its complete packet bound"
    );
    let report = parsed(&raw)?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["kind"], "compare_candidates");
    assert_eq!(report["agent_turn"]["phase"], "compare");
    assert_eq!(report["anchor"], anchor_json(&before.anchor()));
    assert_eq!(report["agent_turn"]["anchor"], report["anchor"]);
    assert_eq!(report["pending_plan_changed"], false);
    assert_eq!(report["effect_evidence"], false);
    assert_eq!(report["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(
        report["agent_turn"]["active_work"]["pending_plans"],
        json!([])
    );
    assert_eq!(report["candidates"].as_array().map(Vec::len), Some(2));
    for key in ["ordinary", "conditional"] {
        let row = candidate(&report, key)?;
        assert_eq!(row["status"], "evaluated", "{row}");
        assert_eq!(row["prediction"]["predicted_complete"], true, "{row}");
        assert_eq!(row["prediction"]["predicted_goal_truth"], "true");
        assert!(row.get("plan_digest").is_none(), "{row}");
    }
    assert_eq!(snapshot(&owner)?, before);
    assert!(pending_digest(&owner)?.is_none());

    let selected = plan_blueprint(&owner, &selection(&report, "conditional")?)?;
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(selected["kind"], "select_candidate");
    assert_eq!(selected["agent_turn"]["phase"], "propose");
    assert_eq!(selected["pending_plan_changed"], true);
    let digest = text_field(&selected, "plan_digest")?;
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(digest.as_str()));
    assert_eq!(
        selected["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        digest
    );
    assert_eq!(snapshot(&owner)?, before);
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        let guard = session.lock().map_err(|_| "session poisoned")?;
        let pending = guard.pending.as_ref().ok_or("selected plan missing")?;
        let PlanSource::Production { raw, .. } = &pending.source else {
            return Err("selection changed the original production source family".into());
        };
        let source = parsed(raw)?;
        assert_eq!(source["orders"], "dependency_preserving_v1");
        assert_eq!(source["quotas"], json!([{"item": "DRINK", "minimum": 60}]));
    }
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        digest,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(500),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(
        waited["objectives"][0]["predicate_truth"], "true",
        "{waited}"
    );
    assert_eq!(waited["objectives"][0]["status"], "achieved", "{waited}");
    Ok(())
}

#[test]
fn public_candidate_comparison_preserves_short_deadlines_beside_long_work() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget.max_game_ticks = 200_000;
    }
    let pending = pause_plan(&owner)?;
    let prior_digest = text_field(&pending, "plan_digest")?;
    let before = snapshot(&owner)?;
    let actions = json!([
        {"action": {
            "kind": "create_work_order", "name": "short proof", "job_token": "BREW_DRINK", "amount": 1,
        }},
        {"action": {
            "kind": "create_work_order", "name": "long work", "job_token": "PREPARE_MEAL", "amount": 1000,
        }},
    ]);
    let report = compare(
        &owner,
        json!([
            {"key": "alpha", "actions": actions.clone()},
            {"key": "zulu", "actions": actions},
        ]),
        200_000,
    )?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["recommended_candidate"], "alpha", "{report}");
    for key in ["alpha", "zulu"] {
        let prediction = &candidate(&report, key)?["prediction"];
        assert_eq!(prediction["available"], true, "{prediction}");
        assert_eq!(prediction["predicted_complete"], true, "{prediction}");
        // The 500-tick nominal stride exceeds the first order's sealed
        // 200-tick deadline. Its inserted observation must preserve that
        // action's proof before the 50,000-tick second order completes.
        assert_eq!(prediction["resolution_ticks"], 500);
        assert!(prediction["sampling_policy"].is_string());
        assert!(
            prediction["simulation_slices"]
                .as_u64()
                .is_some_and(|n| n <= 400)
        );
    }
    assert_eq!(report["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(
        pending_digest(&owner)?.as_deref(),
        Some(prior_digest.as_str())
    );
    assert_eq!(report["pending_plan_changed"], false);
    Ok(())
}

#[test]
fn public_candidate_comparison_preserves_pending_plan_baseline_and_watch_catalogs() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let pause = pause_plan(&owner)?;
    let digest = text_field(&pause, "plan_digest")?;
    let captured = temporal(
        &owner,
        json!({
            "kind": "capture", "key": "candidate-roster", "max_game_ticks": 200,
            "select": {"kind": "entities", "kinds": ["unit"], "fields": ["alive"]},
        }),
    )?;
    assert_eq!(captured["ok"], true, "{captured}");
    let baseline = text_field(&captured["captured"], "baseline")?;
    let before = snapshot(&owner)?;
    let watched = temporal(
        &owner,
        json!({
            "kind": "watch", "key": "candidate-clock",
            "condition": {"op": "tick_at_least", "value": before.tick.0 + 100},
            "deadline_tick": before.tick.0 + 200,
            "poll_interval_ticks": 1, "stable_observations": 1,
        }),
    )?;
    assert_eq!(watched["ok"], true, "{watched}");
    assert_eq!(watched["record"]["status"], "waiting");
    let watch = text_field(&watched["record"], "watch")?;
    let discovered = temporal(&owner, json!({"kind": "watches"}))?;
    assert_eq!(discovered["ok"], true, "{discovered}");

    let report = compare(&owner, production_candidates(), 1_000)?;
    assert_eq!(report["ok"], true, "{report}");
    let work = &report["agent_turn"]["active_work"];
    assert_eq!(work["pending_plans"][0]["plan_digest"], digest);
    assert_eq!(work["query_baselines"][0]["baseline"], baseline);
    assert_eq!(work["query_watches"][0]["watch"], watch);
    assert_eq!(work["condition_watches"][0]["watch"], watch);
    for key in ["query_baselines", "query_watches", "condition_watches"] {
        assert_eq!(
            work[key], discovered["agent_turn"]["active_work"][key],
            "{key}"
        );
    }
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(digest.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    let observed = parsed(&crate::agent_facade::fortress_observe(Some(
        owner.id.clone(),
    )))?;
    assert_eq!(observed["ok"], true, "{observed}");
    assert_eq!(
        observed["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        digest
    );
    assert_eq!(
        observed["agent_turn"]["active_work"]["query_baselines"][0]["baseline"],
        baseline
    );
    assert_eq!(
        observed["agent_turn"]["active_work"]["query_watches"][0]["watch"],
        watch
    );
    Ok(())
}

#[test]
fn public_candidate_selection_refuses_tampered_summary_and_moved_anchor() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let prior = text_field(&pause_plan(&owner)?, "plan_digest")?;
    let before = snapshot(&owner)?;
    let report = compare(&owner, production_candidates(), 1_000)?;
    assert_eq!(report["ok"], true, "{report}");
    let exact = selection(&report, "conditional")?;
    let mut changed = parsed(&exact)?;
    changed["candidate"]["summary"] = json!("an unreviewed replacement purpose");
    refusal(
        &plan_blueprint(&owner, &changed.to_string())?,
        "invalid_request",
    );
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));

    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(1),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(snapshot(&owner)?.tick.0, before.tick.0 + 1);
    let moved = snapshot(&owner)?;
    refusal(&plan_blueprint(&owner, &exact)?, "stale_anchor");
    assert_eq!(snapshot(&owner)?, moved);
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
    Ok(())
}

#[test]
fn public_candidate_output_refusals_leave_the_prior_pending_plan_intact() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let prior = text_field(&pause_plan(&owner)?, "plan_digest")?;
    let before = snapshot(&owner)?;
    let report = compare(&owner, production_candidates(), 1_000)?;
    assert_eq!(report["ok"], true, "{report}");
    let exact = selection(&report, "conditional")?;
    output_budget(&owner, 600)?;
    refusal(
        &compare(&owner, production_candidates(), 1_000)?,
        "budget_exceeded",
    );
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    refusal(&plan_blueprint(&owner, &exact)?, "budget_exceeded");
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
    assert_eq!(snapshot(&owner)?, before);

    output_budget(&owner, 8_192)?;
    let retried = compare(&owner, production_candidates(), 1_000)?;
    assert_eq!(retried["ok"], true, "{retried}");
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
    let selected = plan_blueprint(&owner, &exact)?;
    assert_eq!(selected["ok"], true, "{selected}");
    let digest = text_field(&selected, "plan_digest")?;
    assert_ne!(digest, prior);
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(digest.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    Ok(())
}

#[test]
fn public_candidate_compare_and_select_require_current_read_authority_and_shared_action_budget()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let before = snapshot(&owner)?;
    let report = compare(&owner, pause_candidates(), 100)?;
    assert_eq!(report["ok"], true, "{report}");
    let exact = selection(&report, "alpha")?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let grants = session
        .lock()
        .map_err(|_| "session poisoned")?
        .grants
        .clone();
    assert!(
        grants
            .iter()
            .any(|grant| grant.capability == Capability::Plan
                && grant.max_risk == RiskTier::ReadOnly)
    );
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        {
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            guard.grants = grants.clone();
            guard.grants.retain(|grant| grant.capability != capability);
        }
        refusal(
            &compare(&owner, pause_candidates(), 100)?,
            "capability_denied",
        );
        refusal(&plan_blueprint(&owner, &exact)?, "capability_denied");
        assert!(pending_digest(&owner)?.is_none());
        assert_eq!(snapshot(&owner)?, before);
    }
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.grants = grants;
        guard.budget.max_actions = 1;
    }
    refusal(
        &compare(&owner, pause_candidates(), 100)?,
        "budget_exceeded",
    );
    assert!(pending_digest(&owner)?.is_none());
    assert_eq!(snapshot(&owner)?, before);
    Ok(())
}

#[test]
fn public_candidate_unlike_goals_are_refused_and_paused_predictions_are_not_recommended()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let before = snapshot(&owner)?;
    refusal(
        &compare(
            &owner,
            json!([
                {"key": "pause", "paused_target": true},
                {"key": "already_unpaused", "paused_target": false},
            ]),
            100,
        )?,
        "invalid_request",
    );
    assert!(pending_digest(&owner)?.is_none());
    assert_eq!(snapshot(&owner)?, before);

    let paused = open(true, Some(8_192))?;
    let paused_before = snapshot(&paused)?;
    let report = compare(&paused, production_candidates(), 1_000)?;
    assert_eq!(report["ok"], true, "{report}");
    assert!(report["recommended_candidate"].is_null(), "{report}");
    // A paused world has a known blocked prediction, not fabricated progress.
    for key in ["ordinary", "conditional"] {
        let row = candidate(&report, key)?;
        assert_eq!(row["status"], "evaluated", "{row}");
        assert_eq!(row["prediction"]["available"], true, "{row}");
        assert_eq!(row["prediction"]["blocked_by_pause"], true, "{row}");
        assert_eq!(row["prediction"]["predicted_complete"], false, "{row}");
        assert!(row["prediction"]["predicted_completion_tick"].is_null());
        assert_eq!(
            row["prediction"]["predicted_frontier_tick"],
            paused_before.tick.0
        );
    }
    assert_eq!(report["forecast_work"]["simulated_game_ticks"], 0);
    assert!(pending_digest(&paused)?.is_none());
    assert_eq!(snapshot(&paused)?, paused_before);
    Ok(())
}

#[test]
fn public_candidate_original_bytes_reject_duplicate_keys_and_outer_summary_ambiguity() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let prior = text_field(&pause_plan(&owner)?, "plan_digest")?;
    let before = snapshot(&owner)?;
    for raw in [
        r#"{"template":"compare_candidates","version":1,"version":1,"horizon_ticks":100,"candidates":[{"key":"a","paused_target":true},{"key":"b","paused_target":true}]}"#,
        r#"{"template":"compare_candidates","version":1,"horizon_ticks":100,"candidates":[{"key":"a","actions":[{"action":{"kind":"pause","paused":true,"paused":false}}]},{"key":"b","paused_target":true}]}"#,
    ] {
        refusal(&plan_blueprint(&owner, raw)?, "invalid_request");
        assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
        assert_eq!(snapshot(&owner)?, before);
    }
    let result = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        Some("outer summary must not override candidate identities".into()),
        None,
        None,
        Some(wrapper(pause_candidates(), 100).to_string()),
    ))?;
    refusal(&result, "invalid_request");
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    Ok(())
}

#[test]
fn public_candidate_packets_keep_unfinished_recovered_work_visible_without_resampling() -> TestResult
{
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let prior = text_field(&pause_plan(&owner)?, "plan_digest")?;
    let before = snapshot(&owner)?;
    let retained = {
        let session = resolve_session(Some(owner.id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        // A recovered Verified goal does not establish physical quiescence.
        // This fixture represents the retained recovery boundary, not a new
        // observation or a claim that a native effect was executed.
        let carried = CarriedStep {
            plan_digest: Digest32::of_bytes(b"candidate-retained-work-fixture"),
            step: Some(dfmcp_core::StepId::new(0)),
            kind: "recovered_work",
            proof: Predicate::True,
            failure: None,
            deadline: Some(GameTick(before.tick.0 + 200)),
            monitor: None,
            recorded_state: "verified".to_owned(),
            proof_anchor: Some(before.anchor()),
            observation_error: None,
            failure_reason: None,
            work_step: None,
            work_state: EffectWorkState::Unknown {
                entity_id: None,
                reason: "retained physical work has not been reobserved".to_owned(),
            },
            work_anchor: None,
            state: "verified".to_owned(),
        };
        let retained = carried.to_json();
        let mut quiet = carried.clone();
        quiet.step = Some(dfmcp_core::StepId::new(1));
        quiet.work_state = EffectWorkState::Quiescent { entity_id: None };
        guard.carried.extend([carried, quiet]);
        retained
    };
    let report = compare(&owner, pause_candidates(), 100)?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(
        report["agent_turn"]["active_work"]["carried_obligations"],
        json!([retained])
    );
    assert_eq!(
        report["agent_turn"]["briefing"]["recovered_work_observation"],
        "retained_not_reobserved"
    );
    assert!(
        !report["agent_turn"]["coverage"]["complete_domains"]
            .as_array()
            .ok_or("complete domains missing")?
            .contains(&json!("laboratory.current_session_work"))
    );
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(prior.as_str()));
    let selected = plan_blueprint(&owner, &selection(&report, "alpha")?)?;
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(
        selected["agent_turn"]["active_work"]["carried_obligations"],
        json!([retained])
    );
    assert_eq!(snapshot(&owner)?, before);
    let session = resolve_session(Some(owner.id.clone()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    assert_eq!(guard.carried.len(), 2);
    assert_eq!(guard.carried[0].to_json(), retained);
    assert!(guard.carried[1].is_final());
    Ok(())
}

#[test]
fn public_candidate_ranking_prefers_one_equivalent_labor_batch_before_lexical_key() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(false, Some(8_192))?;
    let before = snapshot(&owner)?;
    for number in [1_001, 1_002] {
        let unit = before
            .graph
            .entities
            .get(&EntityId::new(number))
            .ok_or("starter unit")?;
        assert!(
            unit.fields
                .get("labor.MINING")
                .is_none_or(|fact| fact.value != WorldValue::Bool(true)),
            "fixture already satisfies the requested labor for {number}",
        );
    }
    let report = compare(
        &owner,
        json!([
            {"key": "alpha_two_steps", "actions": [
                {"action": {"kind": "set_labor", "units": ["1001"], "labor": "MINING", "enabled": true}},
                {"action": {"kind": "set_labor", "units": ["1002"], "labor": "MINING", "enabled": true}},
            ]},
            {"key": "zulu_one_batch", "actions": [
                {"action": {"kind": "set_labor", "units": ["1001", "1002"], "labor": "MINING", "enabled": true}},
            ]},
        ]),
        100,
    )?;
    assert_eq!(report["ok"], true, "{report}");
    let separate = candidate(&report, "alpha_two_steps")?;
    let batched = candidate(&report, "zulu_one_batch")?;
    assert_eq!(separate["action_count"], 2);
    assert_eq!(batched["action_count"], 1);
    for row in [separate, batched] {
        assert_eq!(row["prediction"]["predicted_complete"], true, "{row}");
        assert_eq!(row["prediction"]["predicted_goal_truth"], "true");
        assert_eq!(
            row["prediction"]["predicted_completion_tick"],
            before.tick.0
        );
    }
    assert_eq!(separate["max_risk"], batched["max_risk"]);
    assert_eq!(
        separate["requires_checkpoint"],
        batched["requires_checkpoint"]
    );
    assert_eq!(report["recommendation"]["evidence_complete"], true);
    assert_eq!(
        report["recommended_candidate"], "zulu_one_batch",
        "{report}"
    );
    assert_eq!(snapshot(&owner)?, before);
    assert!(pending_digest(&owner)?.is_none());

    let selected = plan_blueprint(&owner, &selection(&report, "zulu_one_batch")?)?;
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(selected["steps"].as_array().map(Vec::len), Some(1));
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        text_field(&selected, "plan_digest")?,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let after = snapshot(&owner)?;
    for number in [1_001, 1_002] {
        let unit = after
            .graph
            .entities
            .get(&EntityId::new(number))
            .ok_or("starter unit")?;
        assert_eq!(
            unit.fields.get("labor.MINING").map(|fact| &fact.value),
            Some(&WorldValue::Bool(true)),
        );
    }
    Ok(())
}

fn open_project_resources() -> TestResult<Owner> {
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(false),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "read_only".into()),
            ("configure_labor".into(), "reversible".into()),
            ("configure_production".into(), "reversible".into()),
            ("control_clock".into(), "reversible".into()),
            ("designate".into(), "guarded".into()),
            ("construct".into(), "guarded".into()),
            ("checkpoint".into(), "guarded".into()),
        ]),
        Some(60_000),
        Some(20_000),
        Some(8_192),
        Some(4 * 1024 * 1024),
        Some(20_000),
        Some(64),
        Some("starter_fortress_resources".into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner {
        id: text_field(&opened, "session_id")?,
    })
}

#[test]
fn public_complete_project_comparison_predicts_later_production_but_selects_only_current_harvest()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open_project_resources()?;
    let previous = pause_plan(&owner)?;
    let old_pending = text_field(&previous, "plan_digest")?;
    let before = snapshot(&owner)?;
    let blueprint = json!({
        "template":"compare_candidates", "version":2,
        "forecast_scope":"complete_project", "horizon_ticks":5_000,
        "candidates":[
            {"key":"alpha","blueprint":{
                "template":"production", "quotas":[{"item":"BED","minimum":24}],
                "acquire_resources":{"assign_labor":false},
            }},
            {"key":"staffed","blueprint":{
                "template":"production", "quotas":[{"item":"BED","minimum":24}],
                "acquire_resources":{"assign_labor":true},
            }},
        ],
    });
    let report = plan_blueprint(&owner, &blueprint.to_string())?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["schema"], "dfmcp.plan-comparison/2");
    assert_eq!(report["agent_turn"]["phase"], "compare");
    assert_eq!(report["forecast_scope"], "complete_project");
    assert_eq!(report["selection_scope"], "current_reviewed_stage_only");
    assert_eq!(report["forecast_work"]["actual_game_ticks_advanced"], 0);
    assert_eq!(report["pending_plan_changed"], false);
    assert_eq!(report["effect_evidence"], false);
    assert!(report["recommended_candidate"].is_string(), "{report}");
    for key in ["alpha", "staffed"] {
        let row = candidate(&report, key)?;
        assert_eq!(row["prediction"]["predicted_complete"], true, "{row}");
        assert_eq!(row["prediction"]["predicted_goal_truth"], "true", "{row}");
        assert_eq!(
            row["prediction"]["stages"].as_array().map(Vec::len),
            Some(2),
            "{row}"
        );
        assert_eq!(
            row["prediction"]["stages"][0]["stage"], "acquire_resources",
            "{row}"
        );
        assert_eq!(
            row["prediction"]["stages"][1]["stage"], "production",
            "{row}"
        );
        assert_eq!(row["max_risk"], row["prediction"]["project_max_risk"]);
        assert_eq!(
            row["action_count"],
            row["prediction"]["project_action_count"]
        );
    }
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(
        pending_digest(&owner)?.as_deref(),
        Some(old_pending.as_str())
    );

    // Whole-packet refusal cannot replace the existing pending plan with even
    // the first stage of an otherwise successful model.
    output_budget(&owner, 256)?;
    let refused = plan_blueprint(&owner, &blueprint.to_string())?;
    refusal(&refused, "budget_exceeded");
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(
        pending_digest(&owner)?.as_deref(),
        Some(old_pending.as_str())
    );
    output_budget(&owner, 20_000)?;

    let selected_raw = selection(&report, "alpha")?;
    let selected_request = parsed(&selected_raw)?;
    assert_eq!(selected_request["version"], 1);
    let selected = plan_blueprint(&owner, &selected_raw)?;
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(selected["kind"], "select_candidate");
    assert_eq!(
        selected["production"]["resource_workflow"]["stage"], "acquire_resources",
        "{selected}"
    );
    let digest = text_field(&selected, "plan_digest")?;
    assert_eq!(pending_digest(&owner)?.as_deref(), Some(digest.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        let guard = session.lock().map_err(|_| "session poisoned")?;
        let pending = guard
            .pending
            .as_ref()
            .ok_or("selected current stage missing")?;
        assert_eq!(
            pending.plan.steps.len() as u64,
            candidate(&report, "alpha")?["current_stage"]["action_count"]
                .as_u64()
                .ok_or("current-stage count missing")?
        );
        assert!(
            guard.open_actions.is_empty(),
            "forecast dispatched canonical actions"
        );
        let PlanSource::Production { raw, .. } = &pending.source else {
            return Err("project selection changed source family".into());
        };
        assert_eq!(parsed(raw)?["quotas"], json!([{"item":"BED","minimum":24}]));
    }
    Ok(())
}
