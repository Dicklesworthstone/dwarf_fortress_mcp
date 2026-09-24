//! End-to-end subquery dispatch, using the parent harness's injected source.
use super::*;

fn comparison() -> Value {
    let recipe = |yield_units| {
        json!({"output":"product","output_batch_size":yield_units,
        "inputs":[{"resource":"raw","units":1}],"job_token":"DeclaredProduct",
        "workshop":{"kind":"workshop","type_key":"DeclaredWorkshop"}})
    };
    envelope(
        json!({"kind":"production_chain_compare","quantity_unit":"stack_units",
        "resources":[{"key":"raw","item_types":["item_type_3"]},
            {"key":"product","item_types":["item_type_999"]}],
        "quotas":[{"resource":"raw","minimum_stock":4},{"resource":"product","minimum_stock":4}],
        "candidates":[{"key":"demanding","recipes":[recipe(1)]},
            {"key":"efficient","recipes":[recipe(2)]}],"limit":1}),
    )
}

#[test]
fn compare_models_in_live_handler_without_sampling_existing_watches() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(4096)?;
    let watched = ask(
        &s,
        envelope(json!({"kind":"watch","key":"comparison-review",
        "condition":{"op":"paused","value":true},"deadline_tick":105u64*403200+100,
        "stable_observations":2})),
    )?;
    assert_eq!(watched["ok"], true, "{watched}");
    let before = ask(&s, envelope(json!({"kind":"watches"})))?;
    let first = ask(&s, comparison())?;
    assert_eq!(first["ok"], true, "{first}");
    assert_eq!(
        first["summary"]["feasible_candidates"],
        json!(["efficient"])
    );
    assert_eq!(first["summary"]["deficit_frontier"], json!(["efficient"]));
    assert_eq!(first["all_declared_alternatives_evaluated"], true);
    assert_eq!(first["agent_turn"]["briefing"]["bridge_protocol"], "1.8");
    assert_eq!(first["rows"][0]["candidate"], "demanding");
    assert_eq!(first["rows"][0]["dominated_by"], json!(["efficient"]));
    assert!(first["continuation"].is_string());
    let mut q = comparison();
    q["query"]["continuation"] = first["continuation"].clone();
    q["query"]["limit"] = json!(128);
    let raw = fortress_query(s.handle(), None, Some(q.clone()));
    assert!(raw.len() <= 16_384);
    let second = decode(&raw)?;
    assert_eq!(second["ok"], true, "{raw}");
    assert_eq!(second["rows"][0]["candidate"], "efficient");
    assert_eq!(second["summary"], first["summary"]);
    assert_eq!(second["analysis_digest"], first["analysis_digest"]);
    assert_eq!(
        second["agent_turn"]["active_work"]["obligations"][0]["stable_observations"],
        1
    );
    assert_eq!(
        ask(&s, envelope(json!({"kind":"watches"})))?["records"],
        before["records"]
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        decode(&fortress_commit(s.handle()))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    assert_eq!(ask(&s, q)?["error"]["code"], "stale_anchor");
    let current = ask(&s, comparison())?;
    assert_eq!(current["ok"], true, "{current}");
    assert_eq!(
        current["summary"]["feasible_candidates"],
        json!(["demanding", "efficient"])
    );
    assert_eq!(
        current["summary"]["deficit_frontier"],
        json!(["demanding", "efficient"])
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn invalid_candidate_and_source_fencing_never_yield_a_partial_comparison() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(65_536)?;
    let schema = decode(&fortress_query(s.handle(), Some("schema".into()), None))?;
    assert_eq!(schema["ok"], true, "{schema}");
    let variants = schema["query_schema"]["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "comparison schema variants absent",
            )
        })?;
    assert!(
        variants
            .iter()
            .any(|v| v["properties"]["kind"]["const"] == "production_chain_compare")
    );
    let mut bad = comparison();
    bad["query"]["candidates"][1]["recipes"][0]["inputs"][0]["resource"] = json!("missing");
    let denied = ask(&s, bad)?;
    assert_eq!(denied["ok"], false);
    assert!(denied.get("comparison_digest").is_none());
    assert!(denied.get("rows").is_none());
    let mut bad = comparison();
    bad["query"]["max_work"] = json!(1);
    assert_eq!(ask(&s, bad)?["error"]["code"], "budget_exceeded");
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.source.fence();
    }
    assert_eq!(
        ask(&s, comparison())?["error"]["code"],
        "adapter_unavailable"
    );
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.grants.clear();
    }
    assert_eq!(ask(&s, comparison())?["error"]["code"], "capability_denied");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
