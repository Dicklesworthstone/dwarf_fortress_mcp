//! Original stock objectives through the actual MCP publication and effect path.

use super::*;
use dfmcp_intent::effects;
use dfmcp_world::{PredicateTruth, Value as WorldValue, laboratory_fact_value};
use serde_json::Value;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Owner {
    id: String,
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(id) = u128::from_str_radix(&self.id, 16) {
            sessions().remove(&SessionId::new(id));
        }
    }
}

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn text(value: &Value, field: &str) -> TestResult<String> {
    Ok(value[field]
        .as_str()
        .ok_or_else(|| format!("missing {field}: {value}"))?
        .to_owned())
}

fn open(output_tokens: Option<u32>) -> TestResult<Owner> {
    let opened = parsed(&crate::agent_facade::fortress_open_session(
        Some(false),
        None,
        Some(vec![
            ("observe".into(), "read_only".into()),
            ("query".into(), "read_only".into()),
            ("plan".into(), "read_only".into()),
            ("designate".into(), "guarded".into()),
            ("checkpoint".into(), "guarded".into()),
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
        Some("starter_fortress_resources".into()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    Ok(Owner {
        id: text(&opened, "session_id")?,
    })
}

fn snapshot(owner: &Owner) -> TestResult<WorldSnapshot> {
    let session = resolve_session(Some(owner.id.clone()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    Ok(guard.adapter.snapshot().clone())
}

fn pending(owner: &Owner) -> TestResult<Option<String>> {
    let session = resolve_session(Some(owner.id.clone()))?;
    let guard = session.lock().map_err(|_| "session poisoned")?;
    Ok(guard.pending.as_ref().map(|plan| plan.digest.clone()))
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> TestResult<u64> {
    match snapshot
        .graph
        .entities
        .get(&crate::lab_world::starter::STOCK_LEDGER)
        .and_then(|ledger| ledger.fields.get(field))
        .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
    {
        Some(WorldValue::U64(value)) => Ok(*value),
        _ => Err("stock observation missing".into()),
    }
}

fn request(wood: u64, plants: Option<u64>, assign_labor: bool) -> Value {
    let mut quotas = vec![json!({"resource":"WOOD","minimum":wood})];
    if let Some(minimum) = plants {
        quotas.push(json!({"resource":"PLANT","minimum":minimum}));
    }
    json!({"template":"resource_acquisition","quotas":quotas,"assign_labor":assign_labor})
}

fn plan(owner: &Owner, source: Value) -> TestResult<Value> {
    parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        None,
        None,
        None,
        Some(source.to_string()),
    ))
}

fn pause_plan(owner: &Owner) -> TestResult<String> {
    let planned = parsed(&crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        Some("retain reviewed pause".into()),
        Some(true),
        None,
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    text(&planned, "plan_digest")
}

fn refusal(result: &Value, code: &str) {
    assert_eq!(result["ok"], false, "{result}");
    assert_eq!(result["error"]["code"], code, "{result}");
}

fn complete_step_reference(result: &Value) -> TestResult {
    let pending = &result["agent_turn"]["active_work"]["pending_plans"][0];
    let reference = &pending["steps_reference"];
    assert_eq!(reference["scope"], "same_response");
    assert_eq!(reference["complete"], true);
    assert_eq!(reference["plan_digest"], result["plan_digest"]);
    assert_eq!(pending["plan_digest"], result["plan_digest"]);
    let pointer = reference["json_pointer"]
        .as_str()
        .ok_or("step reference missing")?;
    assert_eq!(pointer, "/steps");
    let resolved = result
        .pointer(pointer)
        .ok_or("complete step array missing")?;
    assert_eq!(resolved, &result["steps"]);
    assert_eq!(
        resolved.as_array().map(Vec::len).map(|n| n as u64),
        pending["step_count"].as_u64()
    );
    assert!(
        pending.get("steps").is_none(),
        "catalog duplicated the full proof"
    );
    Ok(())
}

fn fresh_review_guidance(result: &Value, digest: &str, next: &Value) -> TestResult {
    let turn = &result["agent_turn"];
    let pending = &turn["active_work"]["pending_plans"][0];
    assert_eq!(pending["plan_digest"], digest);
    assert_eq!(pending["state"], "review_required");
    assert_eq!(pending["minimum_safe_next_step"], *next);
    let recommendations = turn["recommendations"]
        .as_array()
        .ok_or("recommendations missing")?;
    assert!(
        recommendations
            .iter()
            .any(|row| row["tool"] == "fortress.plan" && row["arguments"] == next["arguments"])
    );
    assert!(
        !recommendations
            .iter()
            .any(|row| row["tool"] == "fortress.commit")
    );
    let affordances = turn["affordances"]
        .as_array()
        .ok_or("affordances missing")?;
    assert!(
        !affordances
            .iter()
            .any(|row| row["tool"] == "fortress.commit" && row["enabled"] == true)
    );
    Ok(())
}

#[test]
fn optional_labor_assignment_is_reviewed_and_enables_real_acquisition() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(Some(16_384))?;
    {
        let session = resolve_session(Some(owner.id.clone()))?;
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        let mut snapshot = guard.adapter.snapshot().clone();
        for unit in snapshot.graph.entities.values_mut() {
            if let Some(fact) = unit.fields.get_mut("labor.WOODCUTTING") {
                fact.value = WorldValue::Bool(false);
                fact.presence = Some(dfmcp_world::FactPresence::Known(WorldValue::Bool(false)));
                unit.revision = unit.revision.checked_add(1).ok_or("revision exhausted")?;
            }
        }
        snapshot.cursor = snapshot.cursor.checked_next().ok_or("cursor exhausted")?;
        snapshot.refresh_hash();
        guard.adapter.inject_snapshot(snapshot)?;
        remember_version(&mut guard);
    }
    let before = snapshot(&owner)?;
    let refused = plan(&owner, request(22, None, false))?;
    assert_eq!(
        refused["ok"], false,
        "disabled labor was silently enabled: {refused}"
    );
    assert!(pending(&owner)?.is_none());
    let reviewed = plan(&owner, request(22, None, true))?;
    assert_eq!(reviewed["ok"], true, "{reviewed}");
    assert_eq!(reviewed["steps"][0]["kind"], "set_labor");
    assert_eq!(reviewed["steps"][1]["kind"], "harvest_resource");
    assert_eq!(
        reviewed["resource_acquisition"]["staffing"][0]["previously_enabled"],
        false
    );
    assert_eq!(
        reviewed["resource_acquisition"]["staffing"][0]["labor_enable_action_index"],
        0
    );
    assert_eq!(snapshot(&owner)?, before);
    let digest = text(&reviewed, "plan_digest")?;
    let handle = resolve_session(Some(owner.id.clone()))?;
    let grants = {
        let mut guard = handle.lock().map_err(|_| "session poisoned")?;
        let grants = guard.grants.clone();
        guard
            .grants
            .retain(|grant| grant.capability != Capability::ConfigureLabor);
        grants
    };
    refusal(
        &parsed(&crate::agent_facade::fortress_commit(
            Some(owner.id.clone()),
            digest.clone(),
        ))?,
        "capability_denied",
    );
    assert_eq!(snapshot(&owner)?, before);
    assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    handle.lock().map_err(|_| "session poisoned")?.grants = grants;
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        digest,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let finished = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(500),
    ))?;
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(
        finished["objectives"][0]["predicate_truth"], "true",
        "{finished}"
    );
    Ok(())
}

#[test]
fn declarative_acquisition_supplies_observed_inputs_for_previously_infeasible_production()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open(Some(16_384))?;
    let production = json!({
        "template":"production", "quotas":[{"item":"BED","minimum":22}]
    });
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?, 20);
    let initially_blocked = plan(&owner, production.clone())?;
    assert_eq!(
        initially_blocked["ok"], false,
        "production invented absent wood: {initially_blocked}"
    );
    assert!(pending(&owner)?.is_none());

    let acquisition = plan(&owner, request(22, None, false))?;
    assert_eq!(acquisition["ok"], true, "{acquisition}");
    let acquisition_digest = text(&acquisition, "plan_digest")?;
    let forecast_is_not_supply = plan(&owner, production.clone())?;
    assert_eq!(
        forecast_is_not_supply["ok"], false,
        "a discarded acquisition forecast became supply"
    );
    assert_eq!(
        pending(&owner)?.as_deref(),
        Some(acquisition_digest.as_str())
    );
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        acquisition_digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let unfinished_is_not_supply = plan(&owner, production.clone())?;
    assert_eq!(
        unfinished_is_not_supply["ok"], false,
        "unfinished harvest became supply"
    );
    let acquired = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(500),
    ))?;
    assert_eq!(acquired["ok"], true, "{acquired}");
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(
        acquired["objectives"][0]["predicate_truth"], "true",
        "{acquired}"
    );

    let planned = plan(&owner, production)?;
    assert_eq!(planned["ok"], true, "{planned}");
    let production_digest = text(&planned, "plan_digest")?;
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        production_digest.clone(),
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let finished = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(3_000),
    ))?;
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_BED_FIELD)?, 22);
    let objectives = finished["objectives"]
        .as_array()
        .ok_or("goal evidence missing")?;
    let produced = objectives
        .iter()
        .find(|goal| goal["plan_digest"] == production_digest)
        .ok_or("original production objective missing")?;
    assert_eq!(produced["predicate_truth"], "true", "{produced}");
    assert_eq!(produced["status"], "achieved", "{produced}");
    let consumed = objectives
        .iter()
        .find(|goal| goal["plan_digest"] == acquisition_digest)
        .ok_or("original acquisition objective missing")?;
    assert_eq!(
        consumed["predicate_truth"], "false",
        "consumed stock was still reported as current"
    );
    assert_eq!(
        consumed["historical_achievement"], "verified",
        "the original acquisition proof was rewritten"
    );
    Ok(())
}

#[test]
fn public_stock_goal_publishes_complete_review_and_proves_original_custody() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(None)?;
    let before = snapshot(&owner)?;
    let raw = crate::agent_facade::fortress_plan(
        Some(owner.id.clone()),
        None,
        None,
        None,
        Some(request(22, Some(22), false).to_string()),
    );
    assert!(raw.len() <= 4_096 * 4, "default response budget exceeded");
    let planned = parsed(&raw)?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(planned["kind"], "resource_acquisition");
    assert_eq!(planned["game_effect"], "none");
    assert_eq!(
        planned["publication"],
        "complete_before_pending_plan_change"
    );
    assert_eq!(planned["production"], Value::Null);
    assert_eq!(
        planned["resource_acquisition"]["planner"],
        "resource_acquisition_v1"
    );
    assert_eq!(
        planned["resource_acquisition"]["requirements"][0]["deficit"],
        2
    );
    assert_eq!(
        planned["resource_acquisition"]["requirements"][1]["deficit"],
        2
    );
    assert_eq!(
        planned["resource_acquisition"]["pending_harvest_credited_as_inventory"],
        false
    );
    assert_eq!(
        planned["resource_acquisition"]["native_execution_supported"],
        false
    );
    assert_eq!(planned["steps"].as_array().map(Vec::len), Some(2));
    complete_step_reference(&planned)?;
    assert_eq!(planned["requires_checkpoint"], true);
    let digest = text(&planned, "plan_digest")?;
    assert_eq!(planned["agent_turn"]["phase"], "propose");
    assert_eq!(
        planned["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
        digest
    );
    assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    assert_eq!(
        snapshot(&owner)?,
        before,
        "planning or its discarded forecast changed the world"
    );
    let prepared = {
        let handle = resolve_session(Some(owner.id.clone()))?;
        let guard = handle.lock().map_err(|_| "session poisoned")?;
        let pending = guard.pending.as_ref().ok_or("pending plan missing")?;
        let projection = temporal_handler::projection(&guard, &read_context(&guard))?;
        let entry =
            plan_comparison_response::pending_plan_entry(&pending.plan, guard.session_id, true);
        let catalogs = json!({"query_baselines":[],"query_watches":[],"condition_watches":[]});
        for field in ["count", "digest", "pointer", "complete"] {
            let mut invalid = entry.clone();
            match field {
                "count" => invalid["step_count"] = json!(1),
                "digest" => invalid["steps_reference"]["plan_digest"] = json!("0".repeat(64)),
                "pointer" => invalid["steps_reference"]["json_pointer"] = json!("/forecast"),
                _ => invalid["steps_reference"]["complete"] = json!(false),
            }
            assert!(
                plan_comparison_response::finish(
                    &projection,
                    planned.clone(),
                    catalogs.clone(),
                    Some(invalid)
                )
                .is_err(),
                "accepted a mismatched same-packet step reference: {field}"
            );
        }
        let durable = pending.source.durable();
        let restored = PlanSource::from_durable(&durable);
        let (replayed, _) = restored.compile(pending.plan.intent_id, &before)?;
        assert_eq!(replayed.terminal_condition, pending.plan.terminal_condition);
        let PlanSource::Blueprint { raw, .. } = restored else {
            return Err("retained original source changed families".into());
        };
        assert_eq!(parsed(&raw)?["planner"], "resource_acquisition_v1");
        assert_eq!(
            parsed(&raw)?["quotas"],
            json!([
                {"resource":"WOOD","minimum":22}, {"resource":"PLANT","minimum":22}
            ])
        );
        pending.plan.clone()
    };
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        digest,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?, 20);
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(500),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    // The ordinary Wait projection can explicitly omit full goal records at
    // this default budget. Its retained goal summary still names the actual
    // original plan; the complete original predicate is checked below.
    let observed_goal = &waited["agent_turn"]["briefing"]["objective_status"][0];
    assert_eq!(observed_goal["plan_digest"], prepared.digest.to_string());
    assert_eq!(observed_goal["status"], "achieved", "{waited}");
    if let Some(goals) = waited["objectives"].as_array() {
        assert_eq!(goals[0]["predicate_truth"], "true", "{waited}");
    } else {
        assert_eq!(waited["objectives"]["omitted_for_output_budget"], true);
    }
    let completed = snapshot(&owner)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(stock(&completed, effects::STOCK_PLANT_FIELD)?, 22);
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &prepared)?,
        PredicateTruth::True
    );
    for id in [EntityId::new(8001), crate::lab_world::starter::STOCK_LEDGER] {
        let mut replaced = completed.clone();
        replaced
            .graph
            .entities
            .get_mut(&id)
            .ok_or("original source missing")?
            .generation += 1;
        replaced.refresh_hash();
        assert!(
            !matches!(
                crate::lab_world::plan_goal_truth(&replaced, &prepared),
                Ok(PredicateTruth::True)
            ),
            "same-id replacement established the original goal"
        );
    }
    Ok(())
}

#[test]
fn public_stock_goal_admission_and_output_failures_preserve_reviewed_pending_work() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(Some(8_192))?;
    let prior = pause_plan(&owner)?;
    let before = snapshot(&owner)?;
    let session = resolve_session(Some(owner.id.clone()))?;
    let (budget, grants) = {
        let guard = session.lock().map_err(|_| "session poisoned")?;
        (guard.budget, guard.grants.clone())
    };
    for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
        {
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            guard.grants = grants.clone();
            guard.grants.retain(|grant| grant.capability != capability);
        }
        refusal(
            &plan(&owner, request(22, None, false))?,
            "capability_denied",
        );
        assert_eq!(pending(&owner)?.as_deref(), Some(prior.as_str()));
        assert_eq!(snapshot(&owner)?, before);
    }
    for limit in ["entities", "bytes", "output", "actions"] {
        {
            let mut guard = session.lock().map_err(|_| "session poisoned")?;
            guard.grants = grants.clone();
            guard.budget = budget;
            match limit {
                "entities" => guard.budget.max_entities = 1,
                "bytes" => guard.budget.max_bytes = 1_024,
                "output" => guard.budget.max_output_tokens = 600,
                _ => guard.budget.max_actions = 1,
            }
        }
        refusal(
            &plan(&owner, request(22, Some(22), false))?,
            "budget_exceeded",
        );
        assert_eq!(pending(&owner)?.as_deref(), Some(prior.as_str()), "{limit}");
        assert_eq!(snapshot(&owner)?, before);
    }
    {
        let mut guard = session.lock().map_err(|_| "session poisoned")?;
        guard.budget = budget;
        guard.grants = grants;
    }
    refusal(
        &plan(&owner, request(20, Some(20), false))?,
        "invalid_intent",
    );
    assert_eq!(pending(&owner)?.as_deref(), Some(prior.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    Ok(())
}

#[test]
fn public_stock_goal_commit_requires_ordinary_harvest_authority_and_checkpoint() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(Some(8_192))?;
    let planned = plan(&owner, request(22, None, false))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = text(&planned, "plan_digest")?;
    let before = snapshot(&owner)?;
    let handle = resolve_session(Some(owner.id.clone()))?;
    let grants = handle
        .lock()
        .map_err(|_| "session poisoned")?
        .grants
        .clone();
    for capability in [Capability::Designate, Capability::Checkpoint] {
        {
            let mut guard = handle.lock().map_err(|_| "session poisoned")?;
            guard.grants = grants.clone();
            guard.grants.retain(|grant| grant.capability != capability);
        }
        refusal(
            &parsed(&crate::agent_facade::fortress_commit(
                Some(owner.id.clone()),
                digest.clone(),
            ))?,
            "capability_denied",
        );
        assert_eq!(snapshot(&owner)?, before);
        assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    }
    Ok(())
}

#[test]
fn changed_stock_anchor_requires_fresh_review_and_preserves_the_original_pending_source()
-> TestResult {
    let _serial = crate::test_serial();
    let mut owner_with_hex_letter = None;
    for _ in 0..16 {
        let candidate = open(Some(8_192))?;
        if candidate
            .id
            .chars()
            .any(|character| ('a'..='f').contains(&character))
        {
            owner_with_hex_letter = Some(candidate);
            break;
        }
    }
    let owner = owner_with_hex_letter.ok_or("could not obtain a session with a hex letter")?;
    let planned = plan(&owner, request(24, None, false))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = text(&planned, "plan_digest")?;
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(1),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let changed = snapshot(&owner)?;
    let refused = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.to_uppercase()),
        digest.clone(),
    ))?;
    refusal(&refused, "stale_anchor");
    assert_eq!(refused["session_id"], owner.id);
    assert_eq!(
        refused["rebase"]["method"],
        "fresh_resource_acquisition_required"
    );
    assert_eq!(refused["rebase"]["pending_plan_preserved"], true);
    assert_eq!(refused["next_step"]["tool"], "fortress.plan");
    assert_eq!(refused["error"]["recovery"]["class"], "rebase_required");
    assert_eq!(
        refused["error"]["recovery"]["minimum_safe_next_step"],
        refused["next_step"]
    );
    fresh_review_guidance(&refused, &digest, &refused["next_step"])?;
    assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    assert_eq!(snapshot(&owner)?, changed);
    let original = parsed(&text(&refused["next_step"]["arguments"], "blueprint")?)?;
    assert_eq!(
        original,
        planned["resource_acquisition"]["original_request"]
    );
    assert_eq!(
        original["quotas"],
        json!([{"resource":"WOOD","minimum":24}])
    );
    let comparison = plan(
        &owner,
        json!({
            "template":"compare_candidates", "version":1, "horizon_ticks":1,
            "candidates":[
                {"key":"a","paused_target":true},
                {"key":"b","paused_target":true}
            ],
        }),
    )?;
    assert_eq!(comparison["ok"], true, "{comparison}");
    let projected = &comparison["agent_turn"]["active_work"]["pending_plans"][0];
    assert_eq!(projected["plan_digest"], digest);
    assert_eq!(projected["state"], "review_required");
    assert_eq!(projected["minimum_safe_next_step"], refused["next_step"]);
    assert_eq!(projected["steps"].as_array().map(Vec::len), Some(1));
    assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    let observed = parsed(&crate::agent_facade::fortress_observe(Some(
        owner.id.clone(),
    )))?;
    assert_eq!(observed["ok"], true, "{observed}");
    fresh_review_guidance(&observed, &digest, &refused["next_step"])?;
    assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(1),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    fresh_review_guidance(&waited, &digest, &refused["next_step"])?;
    assert_eq!(pending(&owner)?.as_deref(), Some(digest.as_str()));
    let reviewed = plan(&owner, original)?;
    assert_eq!(reviewed["ok"], true, "{reviewed}");
    assert_ne!(reviewed["plan_digest"], digest);
    assert_eq!(
        reviewed["resource_acquisition"]["requirements"][0]["deficit"],
        4
    );
    let next = parsed(&crate::agent_facade::fortress_observe(Some(
        owner.id.clone(),
    )))?;
    assert_eq!(next["ok"], true, "{next}");
    let pending = &next["agent_turn"]["active_work"]["pending_plans"][0];
    assert_eq!(pending["plan_digest"], reviewed["plan_digest"]);
    assert_eq!(pending["state"], "prepared");
    assert_eq!(pending["minimum_safe_next_step"]["tool"], "fortress.commit");
    assert_eq!(
        pending["minimum_safe_next_step"]["arguments"]["plan_digest"],
        reviewed["plan_digest"]
    );
    Ok(())
}

#[test]
fn resource_candidate_comparison_and_selection_keep_the_original_stock_source() -> TestResult {
    let _serial = crate::test_serial();
    let owner = open(None)?;
    let prior = pause_plan(&owner)?;
    let before = snapshot(&owner)?;
    let comparison = plan(
        &owner,
        json!({
            "template":"compare_candidates", "version":1, "horizon_ticks":500,
            "candidates":[
                {"key":"enabled_only","blueprint":request(22, Some(22), false)},
                {"key":"enable_if_needed","blueprint":request(22, Some(22), true)}
            ],
        }),
    )?;
    assert_eq!(comparison["ok"], true, "{comparison}");
    assert_eq!(pending(&owner)?.as_deref(), Some(prior.as_str()));
    assert_eq!(snapshot(&owner)?, before);
    let candidate = comparison["candidates"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["key"] == "enable_if_needed"))
        .ok_or("acquisition candidate missing")?;
    assert_eq!(candidate["status"], "evaluated", "{candidate}");
    assert_eq!(candidate["prediction"]["predicted_goal_truth"], "true");
    let selected = plan(
        &owner,
        parsed(&text(&candidate["next_step"]["arguments"], "blueprint")?)?,
    )?;
    assert_eq!(selected["ok"], true, "{selected}");
    assert_eq!(selected["kind"], "select_candidate");
    complete_step_reference(&selected)?;
    assert_eq!(selected["production"], Value::Null);
    assert_eq!(
        selected["resource_acquisition"]["original_request"]["assign_labor"],
        true
    );
    assert_eq!(
        selected["resource_acquisition"]["original_request"]["quotas"],
        json!([{"resource":"WOOD","minimum":22},{"resource":"PLANT","minimum":22}])
    );
    assert_eq!(snapshot(&owner)?, before);
    Ok(())
}

#[test]
fn cancelled_partial_acquisition_keeps_real_yield_and_explicit_replan_sizes_the_residual()
-> TestResult {
    let _serial = crate::test_serial();
    let owner = open(Some(16_384))?;
    let original = request(26, None, false);
    let planned = plan(&owner, original.clone())?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        text(&planned, "plan_digest")?,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let waited = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(100),
    ))?;
    assert_eq!(waited["ok"], true, "{waited}");
    let partial = stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?;
    assert!(
        (21..26).contains(&partial),
        "expected physically completed partial yield, got {partial}"
    );
    let blocked = plan(&owner, original.clone())?;
    assert_eq!(
        blocked["ok"], false,
        "active original harvest was double allocated: {blocked}"
    );
    let cancelled = parsed(&crate::agent_facade::fortress_cancel(
        Some(owner.id.clone()),
        Some("stop_future_steps".into()),
        Some("plan".into()),
    ))?;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    assert_eq!(
        stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?,
        partial
    );
    let reviewed = plan(&owner, original)?;
    assert_eq!(reviewed["ok"], true, "{reviewed}");
    assert_eq!(
        reviewed["resource_acquisition"]["original_request"]["quotas"][0]["minimum"],
        26
    );
    assert_eq!(
        reviewed["resource_acquisition"]["requirements"][0]["observed"],
        partial
    );
    assert_eq!(
        reviewed["resource_acquisition"]["requirements"][0]["deficit"],
        26 - partial
    );
    let committed = parsed(&crate::agent_facade::fortress_commit(
        Some(owner.id.clone()),
        text(&reviewed, "plan_digest")?,
    ))?;
    assert_eq!(committed["ok"], true, "{committed}");
    let finished = parsed(&crate::agent_facade::fortress_wait(
        Some(owner.id.clone()),
        Some(500),
    ))?;
    assert_eq!(finished["ok"], true, "{finished}");
    assert_eq!(stock(&snapshot(&owner)?, effects::STOCK_WOOD_FIELD)?, 26);
    Ok(())
}
