use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_spatial.rs"]
mod fixture;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenState;
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};

fn state(stock: u32) -> Result<LiveSpatialCitizenState> {
    let mut state = LiveSpatialCitizenState::default();
    state.publish(fixture::observation(3, 2, stock, false)?)?; Ok(state)
}
fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let anchor = state.snapshot().ok_or_else(|| invalid("fixture snapshot absent"))?.anchor();
    Ok(OperationContext { session_id: SessionId::new(191), request_id: RequestId::new(192), anchor,
        budget: WorkBudget { max_entities: 100_000, max_bytes: 100_000, max_output_tokens: 25_000,
            max_wall_millis: 60_000, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query,
            scope: CapabilityScope { fortress_id: Some(anchor.fortress_id), ..CapabilityScope::default() },
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }], cancellation_requested: false })
}
fn query() -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"production_chain","quantity_unit":"stack_units",
        "resources":[{"key":"raw","item_types":["item_type_3"]},
            {"key":"component","item_types":["item_type_998"]},{"key":"product","item_types":["item_type_999"]}],
        "quotas":[{"resource":"product","minimum_stock":4},{"resource":"raw","minimum_stock":4}],
        "recipes":[{"output":"component","output_batch_size":2,"inputs":[{"resource":"raw","units":2}],
                "job_token":"DeclaredComponent","workshop":{"kind":"workshop","type_key":"DeclaredWorkshop"}},
            {"output":"product","output_batch_size":1,"inputs":[{"resource":"component","units":1}],
                "job_token":"DeclaredProduct","workshop":{"kind":"furnace","type_key":"DeclaredFurnace"}}],
        "limit":128}})
}
fn row<'a>(out: &'a Value, kind: &str, key: &str) -> Result<&'a Value> {
    out["rows"].as_array().and_then(|rows| rows.iter().find(|r| r["row_kind"] == kind
        && (r["key"] == key || r["output"] == key || r["resource"] == key)))
        .ok_or_else(|| invalid("fixture row absent"))
}

#[test]
fn multi_stage_chain_preserves_reserves_and_exposes_complete_deficits() -> Result<()> {
    let state = state(7)?; let c = context(&state)?; let before = state.snapshot().cloned();
    let out = execute(&state, &c, &query())?;
    assert!(handles(&query())); assert_eq!(out["kind"], "production_chain");
    assert_eq!(out["anchor"], anchor_json(c.anchor)); assert_eq!(out["model_feasible"], false);
    assert_eq!(out["observed_quotas_met"], false); assert_eq!(out["total_rows"], 8);
    let raw = row(&out, "resource", "raw")?;
    assert_eq!(raw["observed_stock"]["stack_units"], 7); assert_eq!(raw["observed_stock"]["eligible_items"], 1);
    assert_eq!(raw["observed_stock"]["examples"][0]["generation"], 1);
    assert_eq!(raw["modeled_balance"]["minimum_stock"], 4); assert_eq!(raw["modeled_balance"]["consumed_units"], 4);
    assert_eq!(row(&out, "shortage", "raw")?["missing_units"], 1);
    assert_eq!(row(&out, "step", "component")?["batches"], 2);
    assert_eq!(row(&out, "step", "product")?["depends_on"], json!([{"step_index":0,"output":"component"}]));
    assert_eq!(state.snapshot(), before.as_ref());
    for flag in ["plan_created","reservation_created","commit_compatible","mutation_dispatched",
        "native_recipe_verified","job_readiness_proven","completion_proven"] { assert_eq!(out[flag], false); }
    Ok(())
}

#[test]
fn enough_observed_supply_removes_deficit_but_does_not_claim_finished_products() -> Result<()> {
    let state = state(8)?; let c = context(&state)?; let mut q = query();
    q["query"]["section"] = json!("shortages");
    let out = execute(&state, &c, &q)?;
    assert_eq!(out["model_feasible"], true); assert_eq!(out["observed_quotas_met"], false);
    assert_eq!(out["total_rows"], 0); assert_eq!(out["rows"], json!([]));
    assert_eq!(out["truncated"], false); assert!(out["continuation"].is_null());
    Ok(())
}

#[test]
fn normalized_models_keep_identity_across_equivalent_declarations() -> Result<()> {
    let state = state(7)?; let c = context(&state)?; let q = query();
    let first = execute(&state, &c, &q)?; let mut changed = q;
    changed["query"]["resources"].as_array_mut().ok_or_else(|| invalid("resources"))?.reverse();
    changed["query"]["recipes"].as_array_mut().ok_or_else(|| invalid("recipes"))?.reverse();
    changed["query"]["quotas"].as_array_mut().ok_or_else(|| invalid("quotas"))?
        .push(json!({"resource":"raw","minimum_stock":3}));
    changed["query"]["recipes"][1]["inputs"] = json!([{"resource":"raw","units":1},{"resource":"raw","units":1}]);
    let second = execute(&state, &c, &changed)?;
    assert_eq!(first["model_digest"], second["model_digest"]);
    assert_eq!(first["analysis_digest"], second["analysis_digest"]);
    assert_eq!(first["rows"], second["rows"]);
    Ok(())
}

#[test]
fn whole_row_pages_allow_width_changes_and_repeated_continuations() -> Result<()> {
    let state = state(7)?; let c = context(&state)?;
    let full = execute(&state, &c, &query())?;
    let mut q = query(); q["query"]["limit"] = json!(1);
    let mut collected = Vec::new();
    for _ in 0..8 {
        let page = execute(&state, &c, &q)?;
        assert_eq!(page, execute(&state, &c, &q)?);
        assert_eq!(page["summary"], full["summary"]);
        assert_eq!(page["analysis_digest"], full["analysis_digest"]);
        collected.extend(page["rows"].as_array().ok_or_else(|| invalid("rows"))?.iter().cloned());
        if page["continuation"].is_null() { break; }
        q["query"]["continuation"] = page["continuation"].clone(); q["query"]["limit"] = json!(2);
    }
    assert_eq!(json!(collected), full["rows"]); Ok(())
}

#[test]
fn cursors_bind_every_model_capture_and_session_dimension() -> Result<()> {
    let mut state = state(7)?; let c = context(&state)?;
    let mut q = query(); q["query"]["limit"] = json!(1);
    let first = execute(&state, &c, &q)?; q["query"]["continuation"] = first["continuation"].clone();
    for case in 0..5 {
        let mut changed = q.clone(); let mut changed_c = c.clone();
        match case {
            0 => changed_c.session_id = SessionId::new(99),
            1 => changed["query"]["quotas"][0]["minimum_stock"] = json!(5),
            2 => changed["query"]["recipes"][0]["job_token"] = json!("DifferentJob"),
            3 => changed["query"]["section"] = json!("steps"),
            _ => changed["query"]["max_work"] = json!(MAX_ANALYSIS_WORK - 1),
        }
        assert!(execute(&state, &changed_c, &changed).is_err_and(|e| e.code == ErrorCode::StaleAnchor));
    }
    state.publish(fixture::observation(4, 2, 8, false)?)?;
    assert!(execute(&state, &context(&state)?, &q).is_err_and(|e| e.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn closed_inputs_and_adapter_semantic_refusals_remain_in_force() -> Result<()> {
    let state = state(7)?; let c = context(&state)?;
    for case in 0..9 {
        let mut q = query();
        match case {
            0 => q["query"]["commit"] = json!(true),
            1 => q["query"]["recipes"][0]["raw_lua"] = json!("return true"),
            2 => q["query"]["quantity_unit"] = json!("items"),
            3 => q["query"]["resources"][1]["item_types"] = json!(["item_type_3"]),
            4 => q["query"]["recipes"][0]["inputs"][0]["resource"] = json!("undefined"),
            5 => q["query"]["recipes"][0]["output_batch_size"] = json!(0),
            6 => q["query"]["recipes"][1]["output"] = json!("component"),
            7 => q["query"]["recipes"][0]["workshop"]["kind"] = json!("custom"),
            _ => q["query"]["recipes"][0]["inputs"][0]["units"] = json!(0),
        }
        assert!(execute(&state, &c, &q).is_err(), "case {case}");
    }
    Ok(())
}

#[test]
fn authority_work_and_output_fail_without_partial_results() -> Result<()> {
    let state = state(7)?; let c = context(&state)?;
    for case in 0..5 {
        let mut restricted = c.clone();
        match case { 0 => restricted.grants.clear(), 1 => restricted.cancellation_requested = true,
            2 => restricted.budget.max_entities = 1, 3 => restricted.budget.max_wall_millis = 0,
            _ => restricted.anchor.cursor.sequence += 1 }
        assert!(execute(&state, &restricted, &query()).is_err());
    }
    for limit in [0, 129] {
        let mut q = query(); q["query"]["limit"] = json!(limit); assert!(execute(&state, &c, &q).is_err());
    }
    for work in [0, 1, MAX_ANALYSIS_WORK + 1] {
        let mut q = query(); q["query"]["max_work"] = json!(work); assert!(execute(&state, &c, &q).is_err());
    }
    let mut q = query(); q["query"]["limit"] = json!(1); let page = execute(&state, &c, &q)?;
    let mut tight = c.clone(); tight.budget.max_bytes = page.to_string().len() as u64;
    assert_eq!(execute(&state, &tight, &q)?, page);
    tight.budget.max_bytes -= 1; assert!(execute(&state, &tight, &q).is_err());
    let mut bad = query(); bad["expected_anchor"] = json!({});
    assert!(execute(&state, &c, &bad).is_err_and(|e| e.code == ErrorCode::StaleAnchor));
    Ok(())
}

#[test]
fn schema_discovery_adds_only_the_new_query() -> Result<()> {
    let out = extend_schema(json!({"$defs":{"query":{"oneOf":[{"const":"old"}]}}}))?;
    assert_eq!(out["$defs"]["query"]["oneOf"][0], json!({"const":"old"}));
    assert_eq!(out["$defs"]["query"]["oneOf"][1]["properties"]["kind"]["const"], "production_chain");
    assert!(!handles(&json!({"query":{"kind":"create_work_order"}}))); Ok(())
}

#[cfg(unix)]
#[path = "production_chain_runtime_tests.rs"]
mod runtime;
