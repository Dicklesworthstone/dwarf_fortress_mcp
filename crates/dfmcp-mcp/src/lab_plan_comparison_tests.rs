//! Actual compiler/planner/fork coverage for bounded alternative selection.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, RequestId, SessionId, WorkBudget,
};

fn adapter() -> Result<MemoryAdapter> {
    Ok(MemoryAdapter::new(crate::lab_world::scenario_snapshot(
        "starter_fortress", FortressId::new(1917), false,
    )?))
}

fn context(adapter: &MemoryAdapter) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1917),
        request_id: RequestId::new(1),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Query, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::ConfigureProduction, RiskTier::Reversible),
        ].into_iter().map(|(capability, max_risk)| CapabilityGrant {
            capability, max_risk,
            scope: CapabilityScope { fortress_id: Some(adapter.snapshot().fortress_id), ..CapabilityScope::default() },
            expires_at_tick: None,
            remaining_uses: None,
        }).collect(),
    }
}

fn request(candidates: Value, horizon: u64) -> Result<Request> {
    parse(&json!({"template":"compare_candidates", "version":1, "horizon_ticks":horizon, "candidates":candidates}).to_string())?
        .ok_or_else(|| invalid("test comparison not recognized"))
}

fn pause_candidates() -> Value {
    json!([
        {"key":"zulu", "paused_target":true},
        {"key":"alpha", "paused_target":true},
    ])
}

fn production_candidates() -> Value {
    json!([
        {"key":"ordinary", "blueprint":{"template":"production", "quotas":[{"item":"DRINK","minimum":60}]}},
        {"key":"conditional", "blueprint":{"template":"production", "quotas":[{"item":"DRINK","minimum":60}], "orders":"dependency_preserving_v1"}},
    ])
}

fn run(adapter: &MemoryAdapter, input: &Request) -> Result<Value> {
    compare(adapter, &context(adapter), IntentId::new(1), input, Instant::now())
}

fn selection(row: &Value) -> Result<Request> {
    let raw = row["next_step"]["arguments"]["blueprint"].as_str()
        .ok_or_else(|| invalid("comparison omitted executable selection"))?;
    parse(raw)?.ok_or_else(|| invalid("selection not recognized"))
}

#[test]
fn closed_comparison_grammar_rejects_ambiguous_or_unbounded_alternatives() -> Result<()> {
    assert!(parse(r#"{"template":"dining_hall","origin":[0,0,10],"width":3,"height":3}"#)?.is_none());
    let valid = json!({"template":"compare_candidates","version":1,"horizon_ticks":100,"candidates":pause_candidates()});
    assert!(matches!(parse(&valid.to_string())?, Some(Request::Compare(_))));
    let mut cases = Vec::new();
    let mut value = valid.clone(); value["version"] = json!(2); cases.push(value);
    let mut value = valid.clone(); value["horizon_ticks"] = json!(0); cases.push(value);
    let mut value = valid.clone(); value["horizon_ticks"] = json!(-1); cases.push(value);
    let mut value = valid.clone(); value["goal"] = json!(true); cases.push(value);
    let mut value = valid.clone(); value["candidates"] = json!([{ "key":"only", "paused_target":true }]); cases.push(value);
    let mut value = valid.clone(); value["candidates"] = json!((0..5).map(|i| json!({"key":format!("c{i}"),"paused_target":true})).collect::<Vec<_>>()); cases.push(value);
    let mut value = valid.clone(); value["candidates"][1]["key"] = json!("zulu"); cases.push(value);
    let mut value = valid.clone(); value["candidates"][0]["key"] = json!("has space"); cases.push(value);
    let mut value = valid.clone(); value["candidates"][0]["actions"] = json!([]); cases.push(value);
    let mut value = valid.clone(); value["candidates"][0]["summary"] = Value::Null; cases.push(value);
    let mut value = valid.clone(); value["candidates"][0]["extra"] = json!(true); cases.push(value);
    for value in cases {
        assert_eq!(parse(&value.to_string()).expect_err("closed grammar accepted malformed input").code, ErrorCode::InvalidRequest, "{value}");
    }
    Ok(())
}

#[test]
fn original_json_rejects_duplicate_keys_even_inside_nested_source_actions() {
    for raw in [
        r#"{"template":"compare_candidates","version":1,"version":1,"horizon_ticks":100,"candidates":[{"key":"a","paused_target":true},{"key":"b","paused_target":true}]}"#,
        r#"{"template":"compare_candidates","version":1,"horizon_ticks":100,"candidates":[{"key":"a","actions":[{"action":{"kind":"pause","paused":true,"paused":false}}]},{"key":"b","paused_target":true}]}"#,
        r#"{"template":"compare_candidates","version":1,"horizon_ticks":100,"candidates":[{"key":"a","blueprint":{"template":"production","quotas":[{"item":"DRINK","minimum":60,"minimum":80}]}},{"key":"b","paused_target":true}]}"#,
    ] {
        assert_eq!(parse(raw).expect_err("duplicate accepted").code, ErrorCode::InvalidRequest);
    }
}

#[test]
fn equivalent_pause_candidates_use_real_forks_and_leave_canonical_state_unchanged() -> Result<()> {
    let adapter = adapter()?;
    let before = adapter.snapshot().clone();
    let report = run(&adapter, &request(pause_candidates(), 100)?)?;
    assert_eq!(report["recommended_candidate"], "alpha", "{report}");
    assert_eq!(report["recommendation"]["evidence_complete"], true);
    assert_eq!(report["candidates"].as_array().map(Vec::len), Some(2));
    for candidate in report["candidates"].as_array().ok_or_else(|| invalid("rows missing"))? {
        assert_eq!(candidate["status"], "evaluated", "{candidate}");
        assert_eq!(candidate["prediction"]["predicted_complete"], true, "{candidate}");
        assert_eq!(candidate["prediction"]["predicted_goal_truth"], "true");
        assert_eq!(candidate["prediction"]["predicted_physical_quiescent"], true);
        assert!(candidate.get("plan_digest").is_none());
        assert_eq!(candidate["next_step"]["tool"], "fortress.plan");
    }
    assert_eq!(report["pending_plan_changed"], false);
    assert_eq!(report["effect_evidence"], false);
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}

#[test]
fn real_production_lowerings_compare_one_original_quota_and_select_the_exact_source() -> Result<()> {
    let adapter = adapter()?;
    let before = adapter.snapshot().clone();
    let report = run(&adapter, &request(production_candidates(), 1000)?)?;
    assert_eq!(report["recommendation"]["evidence_complete"], true, "{report}");
    assert!(report["recommended_candidate"].is_string(), "{report}");
    assert_eq!(report["candidates"][0]["key"], "conditional");
    assert_eq!(report["candidates"][1]["key"], "ordinary");
    for row in report["candidates"].as_array().ok_or_else(|| invalid("rows missing"))? {
        assert_eq!(row["prediction"]["predicted_complete"], true, "{row}");
        let selected = selection(row)?;
        let source = selected_source(&selected, adapter.snapshot(), &context(&adapter))?
            .ok_or_else(|| invalid("selection returned no source"))?;
        let (intent, _) = source.compile(IntentId::new(90), adapter.snapshot())?;
        let prepared = StaticPlanner::default().prepare_laboratory(adapter.snapshot(), &intent, &context(&adapter))?;
        assert_eq!(prepared.steps.len(), row["action_count"].as_u64().unwrap_or_default() as usize);
        let PlanSource::Production { raw, .. } = source else { return Err(invalid("production source changed kind")); };
        let raw: Value = serde_json::from_str(&raw).map_err(|e| invalid(e.to_string()))?;
        assert_eq!(raw["planner"], "consumption_aware_v1");
        if row["key"] == "conditional" {
            assert_eq!(raw["orders"], "dependency_preserving_v1");
        }
    }
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}

#[test]
fn candidate_order_and_real_request_counter_do_not_change_predictions_or_source_digests() -> Result<()> {
    let adapter = adapter()?;
    let candidates = production_candidates();
    let first = run(&adapter, &request(candidates.clone(), 1000)?)?;
    let mut reversed = candidates.as_array().cloned().ok_or_else(|| invalid("candidate array missing"))?;
    reversed.reverse();
    let second = compare(&adapter, &context(&adapter), IntentId::new(999), &request(json!(reversed), 1000)?, Instant::now())?;
    assert_eq!(first["recommended_candidate"], second["recommended_candidate"]);
    assert_eq!(first["original_terminal_digest"], second["original_terminal_digest"]);
    assert_eq!(first["candidates"], second["candidates"]);
    Ok(())
}

#[test]
fn different_goals_are_rejected_before_a_failed_preparation_can_hide_the_difference() -> Result<()> {
    let adapter = adapter()?;
    // The second pause intent is already true. Its preparation would fail,
    // but that must not excuse comparing a different original objective.
    let input = request(json!([
        {"key":"pause","paused_target":true},
        {"key":"already_unpaused","paused_target":false},
    ]), 100)?;
    let error = run(&adapter, &input).expect_err("different original goals compared");
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("same normalized original"));
    Ok(())
}

#[test]
fn reserve_frontiers_are_part_of_the_shared_objective() -> Result<()> {
    let adapter = adapter()?;
    let mut candidates = production_candidates();
    candidates[0]["blueprint"]["reserve_until_tick"] = json!(1201);
    candidates[1]["blueprint"]["reserve_until_tick"] = json!(2401);
    let error = run(&adapter, &request(candidates, 3000)?).expect_err("different reserve frontiers compared");
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error:?}");
    assert!(error.message.contains("reserve frontier"));
    Ok(())
}

#[test]
fn an_unavailable_alternative_is_not_scored_as_a_loss_or_hidden_from_recommendation() -> Result<()> {
    let adapter = adapter()?;
    let input = request(json!([
        {"key":"valid", "paused_target":true},
        {"key":"invalid", "actions":[{"action":{"kind":"not_an_action"}}]},
    ]), 100)?;
    let report = run(&adapter, &input)?;
    assert_eq!(report["candidates"][0]["key"], "invalid");
    assert_eq!(report["candidates"][0]["status"], "unavailable");
    assert_eq!(report["candidates"][0]["stage"], "compile");
    assert_eq!(report["candidates"][0]["selection_available"], false);
    assert_eq!(report["candidates"][1]["prediction"]["predicted_complete"], true);
    assert!(report["recommended_candidate"].is_null());
    assert_eq!(report["recommendation"]["evidence_complete"], false);
    Ok(())
}

#[test]
fn incomplete_horizon_is_reported_and_never_recommended_as_complete() -> Result<()> {
    let adapter = adapter()?;
    let report = run(&adapter, &request(production_candidates(), 1)?)?;
    for row in report["candidates"].as_array().ok_or_else(|| invalid("rows missing"))? {
        assert_eq!(row["prediction"]["available"], true, "{row}");
        assert_eq!(row["prediction"]["predicted_complete"], false, "{row}");
        assert_eq!(row["prediction"]["simulation_horizon_truncated"], true, "{row}");
        assert!(row["prediction"]["predicted_completion_tick"].is_null());
    }
    assert!(report["recommended_candidate"].is_null());
    Ok(())
}

#[test]
fn aggregate_action_source_tick_and_wall_allowances_cannot_be_multiplied_by_candidates() -> Result<()> {
    let adapter = adapter()?;
    let input = request(pause_candidates(), 100)?;
    let mut ctx = context(&adapter);
    ctx.budget.max_actions = 1;
    assert_eq!(compare(&adapter, &ctx, IntentId::new(1), &input, Instant::now()).expect_err("per-candidate action allowance multiplied").code, ErrorCode::BudgetExceeded);
    ctx = context(&adapter); ctx.budget.max_entities = 1;
    assert_eq!(compare(&adapter, &ctx, IntentId::new(1), &input, Instant::now()).expect_err("whole-source budget bypassed").code, ErrorCode::BudgetExceeded);
    ctx = context(&adapter); ctx.budget.max_game_ticks = 1;
    assert_eq!(compare(&adapter, &ctx, IntentId::new(1), &input, Instant::now()).expect_err("horizon beyond game-time allowance accepted").code, ErrorCode::BudgetExceeded);
    ctx = context(&adapter); ctx.budget.max_wall_millis = 1;
    let past = Instant::now().checked_sub(Duration::from_secs(1)).ok_or_else(|| invalid("clock cannot represent test past"))?;
    assert_eq!(compare(&adapter, &ctx, IntentId::new(1), &input, past).expect_err("wall-time deadline restarted").code, ErrorCode::BudgetExceeded);
    Ok(())
}

#[test]
fn current_authority_and_cancellation_gate_comparison_and_selection() -> Result<()> {
    let adapter = adapter()?;
    let input = request(pause_candidates(), 100)?;
    let report = run(&adapter, &input)?;
    let selected = selection(&report["candidates"][0])?;
    for capability in [Capability::Plan, Capability::Observe, Capability::Query] {
        let mut ctx = context(&adapter);
        ctx.grants.retain(|grant| grant.capability != capability);
        assert_eq!(compare(&adapter, &ctx, IntentId::new(1), &input, Instant::now()).expect_err("missing current authority accepted").code, ErrorCode::CapabilityDenied);
        assert_eq!(selected_source(&selected, adapter.snapshot(), &ctx).expect_err("selection reused old authority").code, ErrorCode::CapabilityDenied);
    }
    let mut ctx = context(&adapter); ctx.cancellation_requested = true;
    assert!(compare(&adapter, &ctx, IntentId::new(1), &input, Instant::now()).is_err());
    assert!(selected_source(&selected, adapter.snapshot(), &ctx).is_err());
    Ok(())
}

#[test]
fn candidate_selection_rejects_changed_source_summary_digest_and_restore_epoch() -> Result<()> {
    let mut adapter = adapter()?;
    let report = run(&adapter, &request(pause_candidates(), 100)?)?;
    let row = &report["candidates"][0];
    let selected = selection(row)?;
    assert!(selected_source(&selected, adapter.snapshot(), &context(&adapter))?.is_some());
    let raw = row["next_step"]["arguments"]["blueprint"].as_str().ok_or_else(|| invalid("selection missing"))?;
    let wrapper: Value = serde_json::from_str(raw).map_err(|e| invalid(e.to_string()))?;
    for fortress in ["01917", "+1917", "77d", "18446744073709551616"] {
        let mut changed = wrapper.clone();
        changed["expected_anchor"]["fortress_id"] = json!(fortress);
        assert_eq!(
            parse(&changed.to_string())
                .expect_err("noncanonical fortress identity accepted")
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for field in ["summary", "paused_target"] {
        let mut changed = wrapper.clone();
        changed["candidate"][field] = if field == "summary" { json!("different reviewed purpose") } else { json!(false) };
        let parsed = parse(&changed.to_string())?.ok_or_else(|| invalid("changed selection missing"))?;
        assert_eq!(selected_source(&parsed, adapter.snapshot(), &context(&adapter)).expect_err("tampered candidate accepted").code, ErrorCode::InvalidRequest);
    }
    let mut changed = wrapper.clone(); changed["candidate_digest"] = json!("0".repeat(64));
    let parsed = parse(&changed.to_string())?.ok_or_else(|| invalid("changed selection missing"))?;
    assert_eq!(selected_source(&parsed, adapter.snapshot(), &context(&adapter)).expect_err("tampered digest accepted").code, ErrorCode::InvalidRequest);
    let mut next = adapter.snapshot().clone();
    next.cursor.epoch += 1;
    next.cursor.sequence = 0;
    next.refresh_hash();
    adapter.inject_snapshot(next)?;
    assert_eq!(selected_source(&selected, adapter.snapshot(), &context(&adapter)).expect_err("restore epoch reused old candidate").code, ErrorCode::StaleAnchor);
    Ok(())
}

#[test]
fn missing_effect_authority_produces_unavailable_forecasts_without_predicted_dispatch() -> Result<()> {
    let adapter = adapter()?;
    let before = adapter.snapshot().clone();
    let input = request(production_candidates(), 1000)?;
    let mut ctx = context(&adapter);
    ctx.grants.retain(|grant| grant.capability != Capability::ConfigureProduction);
    let report = compare(&adapter, &ctx, IntentId::new(1), &input, Instant::now())?;
    assert!(report["recommended_candidate"].is_null());
    assert_eq!(report["recommendation"]["evidence_complete"], false);
    for row in report["candidates"].as_array().ok_or_else(|| invalid("rows missing"))? {
        assert_eq!(row["status"], "unavailable", "{row}");
        assert!(row["reason"]["code"] == "capability_denied" || row["prediction"]["reason"]["code"] == "capability_denied", "{row}");
    }
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}
