use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_spatial.rs"]
mod fixture;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenState;
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};

fn state() -> Result<LiveSpatialCitizenState> {
    let mut s = LiveSpatialCitizenState::default(); s.publish(fixture::observation(3, 2, 7, false)?)?; Ok(s)
}
fn context(s: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let anchor = s.snapshot().ok_or_else(|| invalid("comparison snapshot absent"))?.anchor();
    Ok(OperationContext { session_id: SessionId::new(261), request_id: RequestId::new(262), anchor,
        budget: WorkBudget { max_entities: 100_000, max_bytes: 100_000, max_output_tokens: 25_000,
            max_wall_millis: 60_000, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false })
}
fn request() -> Value {
    let recipe = |yield_units| json!({"output":"product","output_batch_size":yield_units,
        "inputs":[{"resource":"raw","units":1}],"job_token":"DeclaredProduct",
        "workshop":{"kind":"workshop","type_key":"DeclaredWorkshop"}});
    json!({"schema":"dfmcp.query/1","query":{"kind":"production_chain_compare","quantity_unit":"stack_units",
        "resources":[{"key":"raw","item_types":["item_type_3"]},{"key":"product","item_types":["item_type_999"]}],
        "quotas":[{"resource":"raw","minimum_stock":4},{"resource":"product","minimum_stock":4}],
        "candidates":[{"key":"demanding","recipes":[recipe(1)]},{"key":"efficient","recipes":[recipe(2)]}]}})
}

#[test]
fn common_stock_and_final_reserves_distinguish_feasible_alternatives() -> Result<()> {
    let s = state()?; let c = context(&s)?; let before = s.snapshot().cloned();
    let out = execute(&s, &c, &request())?;
    assert_eq!(out["summary"]["feasible_candidates"], json!(["efficient"]));
    assert_eq!(out["summary"]["deficit_frontier"], json!(["efficient"]));
    assert_eq!(out["rows"][0]["candidate"], "demanding");
    assert_eq!(out["rows"][0]["dominated_by"], json!(["efficient"]));
    assert_eq!(out["rows"][0]["balances"][1]["resource"], "raw");
    assert_eq!(out["rows"][0]["balances"][1]["missing_units"], 1);
    assert_eq!(out["rows"][1]["model_feasible"], true);
    assert_eq!(out["rows"][1]["balances"][1]["consumed_units"], 2);
    assert_eq!(out["summary"]["observed_stock"][1]["stock_units"], 7);
    assert_eq!(out["anchor"], anchor_json(c.anchor)); assert_eq!(out["native_captures"], 0);
    assert_eq!(s.snapshot(), before.as_ref());
    for flag in ["plan_created","reservation_created","commit_compatible","mutation_dispatched",
        "global_optimum_proven","native_recipe_verified","completion_proven"] { assert_eq!(out[flag], false); }
    Ok(())
}

#[test]
fn equal_and_incomparable_deficits_survive_without_scalar_scoring() -> Result<()> {
    let s = state()?; let c = context(&s)?; let mut q = request();
    q["query"]["candidates"][1]["recipes"] = q["query"]["candidates"][0]["recipes"].clone();
    let equal = execute(&s, &c, &q)?;
    assert_eq!(equal["summary"]["deficit_frontier"], json!(["demanding","efficient"]));
    assert_eq!(equal["rows"][0]["model_digest"], equal["rows"][1]["model_digest"]);
    q["query"]["candidates"][1]["recipes"] = json!([]);
    let incomparable = execute(&s, &c, &q)?;
    assert_eq!(incomparable["summary"]["feasible_candidates"], json!([]));
    assert_eq!(incomparable["summary"]["deficit_frontier"], json!(["demanding","efficient"]));
    assert_eq!(incomparable["rows"][1]["balances"][0]["resource"], "product");
    assert_eq!(incomparable["rows"][1]["balances"][0]["missing_units"], 4);
    Ok(())
}

#[test]
fn deficit_dominance_matches_the_integer_lower_rectangle_oracle() {
    for ax in 0..3 { for ay in 0..3 { for az in 0..3 {
        for bx in 0..3 { for by in 0..3 { for bz in 0..3 {
            let a = [ax, ay, az]; let b = [bx, by, bz];
            let mut lower = BTreeSet::new();
            for x in 0..=bx { for y in 0..=by { for z in 0..=bz { lower.insert([x,y,z]); } } }
            lower.remove(&b);
            assert_eq!(dominates(&a, &b), lower.contains(&a));
        } } }
    } } }
    assert!(!dominates(&[0], &[1,2])); assert!(!dominates(&[], &[]));
}

#[test]
fn reordering_normalized_inputs_preserves_comparison_identity_and_frontier() -> Result<()> {
    let s = state()?; let c = context(&s)?; let q = request();
    let before = execute(&s, &c, &q)?; let mut after = q;
    after["query"]["candidates"].as_array_mut().ok_or_else(|| invalid("candidates"))?.reverse();
    after["query"]["resources"].as_array_mut().ok_or_else(|| invalid("resources"))?.reverse();
    after["query"]["quotas"].as_array_mut().ok_or_else(|| invalid("quotas"))?.reverse();
    let after = execute(&s, &c, &after)?;
    assert_eq!(before["comparison_digest"], after["comparison_digest"]);
    assert_eq!(before["analysis_digest"], after["analysis_digest"]);
    assert_eq!(before["rows"], after["rows"]); assert_eq!(before["summary"], after["summary"]);
    Ok(())
}

#[test]
fn work_budget_is_shared_by_every_candidate_and_frontier_comparison() -> Result<()> {
    let s = state()?; let c = context(&s)?;
    let out = execute(&s, &c, &request())?;
    let used = out["work_units"].as_u64().ok_or_else(|| invalid("work counter"))?;
    let plans = out["rows"].as_array().ok_or_else(|| invalid("rows"))?.iter()
        .map(|r| r["planner_work_units"].as_u64().unwrap_or(0)).sum::<u64>();
    assert!(used > plans);
    let mut q = request(); q["query"]["max_work"] = json!(used);
    assert_eq!(execute(&s, &c, &q)?["rows"], out["rows"]);
    q["query"]["max_work"] = json!(used - 1);
    assert!(execute(&s, &c, &q).is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn pagination_binds_every_alternative_not_only_the_current_page() -> Result<()> {
    let mut s = state()?; let c = context(&s)?; let mut q = request(); q["query"]["limit"] = json!(1);
    let first = execute(&s, &c, &q)?; q["query"]["continuation"] = first["continuation"].clone();
    q["query"]["limit"] = json!(128); let second = execute(&s, &c, &q)?;
    assert_eq!(second["rows"][0]["candidate"], "efficient"); assert_eq!(first["summary"], second["summary"]);
    for case in 0..4 {
        let mut bad = q.clone(); let mut other = c.clone();
        match case { 0 => bad["query"]["candidates"][0]["recipes"][0]["job_token"] = json!("Changed"),
            1 => bad["query"]["candidates"][0]["key"] = json!("renamed"),
            2 => bad["query"]["max_work"] = json!(MAX_ANALYSIS_WORK - 1),
            _ => other.session_id = SessionId::new(999) }
        assert!(execute(&s, &other, &bad).is_err_and(|e| e.code == ErrorCode::StaleAnchor));
    }
    s.publish(fixture::observation(4, 2, 8, false)?)?;
    assert!(execute(&s, &context(&s)?, &q).is_err_and(|e| e.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn invalid_alternatives_never_yield_an_incomplete_frontier() -> Result<()> {
    let s = state()?; let c = context(&s)?;
    for case in 0..8 {
        let mut q = request();
        match case { 0 => q["query"]["candidates"] = json!([]),
            1 => q["query"]["candidates"][1]["key"] = json!("demanding"),
            2 => q["query"]["candidates"][1]["recipes"][0]["output_batch_size"] = json!(0),
            3 => q["query"]["candidates"][1]["recipes"][0]["inputs"][0]["resource"] = json!("missing"),
            4 => q["query"]["candidates"][1]["quotas"] = json!([]),
            5 => q["query"]["candidates"][1]["key"] = json!("x".repeat(65)),
            6 => q["query"]["commit"] = json!(true),
            _ => q["query"]["resources"][1]["item_types"] = json!(["item_type_3"]) }
        assert!(execute(&s, &c, &q).is_err(), "case {case}");
    }
    let mut q = request();
    let recipes = q["query"]["candidates"][1]["recipes"].clone();
    q["query"]["candidates"] = json!((0..8).map(|i| json!({"key":format!("c{i}"),"recipes":recipes})).collect::<Vec<_>>());
    assert_eq!(execute(&s, &c, &q)?["total_rows"], 8);
    q["query"]["candidates"].as_array_mut().ok_or_else(|| invalid("candidates"))?.push(json!({"key":"c8","recipes":recipes}));
    assert!(execute(&s, &c, &q).is_err()); Ok(())
}

#[test]
fn authority_and_complete_result_budgets_apply_to_comparison() -> Result<()> {
    let s = state()?; let c = context(&s)?;
    let mut denied = c.clone(); denied.grants.clear();
    assert!(execute(&s, &denied, &request()).is_err_and(|e| e.code == ErrorCode::CapabilityDenied));
    denied = c.clone(); denied.cancellation_requested = true;
    assert!(execute(&s, &denied, &request()).is_err());
    let mut tiny = c; tiny.budget.max_bytes = 1;
    assert!(execute(&s, &tiny, &request()).is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    Ok(())
}
