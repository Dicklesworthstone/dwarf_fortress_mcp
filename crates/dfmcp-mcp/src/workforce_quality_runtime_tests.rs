//! Exercise the real registered handler, not just a JSON rendering stand-in.
use super::*;
use dfmcp_adapter::live_spatial::citizens::citizen_entity_id;
use dfmcp_adapter::workforce_analysis::{quality as weighted, WorkforceDemand};

fn request(workers: u32) -> Value {
    let mut q = planned(workers); q["objective"] = json!("priority_skill_distance"); q
}
fn scored(ratings: &[(u32, u32)]) -> Result<Registered> {
    let s = register(ratings.len() as u32, 65_536)?;
    let mut state = LiveSpatialCitizenState::default();
    state.publish(observation_scored(3, ratings.len() as u32, Some(ratings))?)?;
    let handle = resolve(s.handle())?; lock(&handle)?.state = state; Ok(s)
}

#[test]
fn quality_query_prefers_observed_skill_without_changing_legacy_default() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = scored(&[(5,5), (9,8)])?;
    let legacy = ask(&s, planned(1))?;
    let mut explicit = planned(1); explicit["objective"] = json!("max_filled_slots");
    let old = ask(&s, explicit)?;
    for key in ["rows", "model_digest", "optimization", "work_units"] { assert_eq!(old[key], legacy[key]); }
    assert_eq!(legacy["rows"][0]["citizen"]["entity_id"], citizen_entity_id(10).get().to_string());
    let out = ask(&s, request(1))?;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["rows"][0]["citizen"]["entity_id"], citizen_entity_id(11).get().to_string());
    assert_eq!(out["optimization"]["achieved"]["effective_skill"], 8);
    assert_eq!(out["optimization"]["achieved"]["nominal_skill"], 9);
    assert_eq!(out["optimization"]["maximum_cardinality_checked"], true);
    assert_eq!(out["optimization"]["residual_optimality_checked"], true);
    assert_eq!(out["optimization"]["model_only"], true);
    assert_ne!(out["model_digest"], legacy["model_digest"]);
    for key in ["mutation_dispatched", "reservations_created", "unit_path_proven", "labor_eligibility_proven", "safety_proven"] { assert_eq!(out[key], false); }
    let again = ask(&s, request(1))?;
    for key in ["rows", "model_digest", "optimization", "work_units"] { assert_eq!(again[key], out[key]); }
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn priority_handles_shortage_and_global_rerouting_preserves_headcount() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(1, 65_536)?;
    let mut a = demand("a", "CARPENTRY", 1); a["priority"] = json!(0);
    let mut b = demand("b", "MINING", 1); b["priority"] = json!(1000);
    let mut q = json!({"kind":"workforce_plan","objective":"priority_skill_distance","demands":[a,b]});
    let out = ask(&s, q.clone())?;
    assert_eq!(out["ok"], true, "{out}"); assert_eq!(out["assigned_workers"], 1);
    assert_eq!(out["rows"][0]["demand_key"], "b");
    assert_eq!(out["optimization"]["achieved"]["priority"], 1000);
    assert_eq!(out["shortage"]["distinct_candidate_workers"], 1);
    assert_eq!(out["shortage"]["deficit"], 1);
    let second = register(2, 65_536)?;
    let filled = ask(&second, q.clone())?;
    assert_eq!(filled["assigned_workers"], 2); assert_eq!(filled["cut_capacity"], 2);
    assert_eq!(filled["rows"][0]["citizen"]["entity_id"], citizen_entity_id(11).get().to_string());
    assert_eq!(filled["rows"][1]["citizen"]["entity_id"], citizen_entity_id(10).get().to_string());
    q["demands"].as_array_mut().expect("test demands").reverse();
    let reordered = ask(&second, q)?;
    for key in ["rows", "model_digest", "optimization", "work_units"] { assert_eq!(reordered[key], filled[key]); }
    let route = decode(&fortress_query(second.handle(), None, Some(filled["rows"][0]["route_query"].clone())))?;
    assert_eq!(route["ok"], true, "{route}"); assert_eq!(route["anchor"], filled["anchor"]);
    assert_eq!(s.calls.load(Ordering::SeqCst) + second.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn quality_pages_keep_active_watches_and_bind_objective_priorities_and_capture() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(40, 2048)?;
    let watch = ask(&s, json!({"kind":"watch","key":"quality-review","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":2}))?;
    assert_eq!(watch["ok"], true, "{watch}");
    let mut q = request(40); let mut ids = BTreeSet::new(); let mut token = Value::Null; let mut pages = 0;
    loop {
        let raw = fortress_query(s.handle(), None, Some(json!({"schema":"dfmcp.query/1","query":q.clone()})));
        assert!(raw.len() <= 8192); let out = decode(&raw)?; assert_eq!(out["ok"], true, "{out}");
        assert_eq!(out["assigned_workers"], 40);
        assert_eq!(out["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len), Some(1));
        assert_eq!(out["agent_turn"]["active_work"]["obligations"][0]["stable_observations"], 1);
        for row in out["rows"].as_array().expect("test rows") {
            assert!(ids.insert(row["citizen"]["entity_id"].as_str().expect("test worker").to_owned()));
        }
        pages += 1; assert!(pages <= 40);
        if token.is_null() { token = out["continuation"].clone(); }
        if out["continuation"].is_null() { break; }
        assert!(out["continuation"].as_str().expect("test token").starts_with("wq1:"));
        q["continuation"] = out["continuation"].clone(); q["limit"] = json!(3);
        q["demands"][0]["priority"] = json!(0); // explicit default keeps the model
    }
    assert_eq!(ids.len(), 40); assert!(pages > 1);
    q["continuation"] = token;
    let mut changed = q.clone(); changed["demands"][0]["priority"] = json!(1);
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    let other = register(40, 2048)?;
    assert_eq!(ask(&other, q.clone())?["error"]["code"], "stale_anchor");
    let mut legacy = q.clone(); legacy["objective"] = json!("max_filled_slots");
    legacy["demands"][0].as_object_mut().expect("test demand").remove("priority");
    assert_eq!(ask(&s, legacy)?["ok"], false);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
    assert_eq!(ask(&s, q)?["error"]["code"], "stale_anchor");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    assert_eq!(ask(&s, json!({"kind":"cancel_watch","watch":watch["record"]["watch"]}))?["ok"], true);
    assert_eq!(ask(&s, json!({"kind":"release_watch","watch":watch["record"]["watch"]}))?["ok"], true);
    Ok(())
}

#[test]
fn schema_discovers_quality_and_runtime_refuses_invalid_or_ignored_priorities() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(2, 65_536)?;
    let schema = decode(&fortress_query(s.handle(), Some("schema".into()), None))?;
    let variants = schema["query_schema"]["$defs"]["query"]["oneOf"].as_array().expect("test variants");
    let plan = variants.iter().find(|v| v["properties"]["kind"]["const"] == "workforce_plan").expect("test plan variant");
    assert!(plan["properties"]["objective"]["enum"].as_array().expect("test objectives").contains(&json!("priority_skill_distance")));
    for bad in [json!(-1), json!(1001), json!(65536), json!(true), json!("1")] {
        let mut q = request(1); q["demands"][0]["priority"] = bad;
        assert_eq!(ask(&s, q)?["ok"], false);
    }
    let mut ignored = planned(1); ignored["demands"][0]["priority"] = json!(0);
    assert_eq!(ask(&s, ignored)?["error"]["code"], "invalid_request");
    let mut unknown = request(1); unknown["objective"] = json!("greedy");
    assert_eq!(ask(&s, unknown)?["error"]["code"], "invalid_request");
    let mut duplicate = request(1); duplicate["demands"] = json!([demand("x", "CARPENTRY", 1),demand("x", "MINING", 1)]);
    assert_eq!(ask(&s, duplicate)?["error"]["code"], "invalid_request");
    let mut exhausted = request(1); exhausted["max_work"] = json!(1);
    assert_eq!(ask(&s, exhausted)?["error"]["code"], "budget_exceeded");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn quality_adapter_keeps_authorization_cancellation_and_priority_key_checks() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(2, 65_536)?;
    let handle = resolve(s.handle())?; let mut session = lock(&handle)?; let context = session.context()?;
    let demands = [WorkforceDemand { key:"wood".into(),workers:1,target:[0,0,5],skill_key:"CARPENTRY".into(),
        min_effective_skill:1,preserve_social:true,adults_only:true }];
    let priorities = BTreeMap::new();
    for case in 0..4 {
        let mut c = context.clone();
        let expected = match case {
            0 => { c.grants.clear(); ErrorCode::CapabilityDenied },
            1 => { c.cancellation_requested = true; ErrorCode::CancellationRequested },
            2 => { c.anchor.cursor.sequence += 1; ErrorCode::StaleAnchor },
            _ => { c.budget.max_entities = 1; ErrorCode::BudgetExceeded },
        };
        assert!(matches!(weighted::plan(&session.state, &c, &demands, &priorities, 1_000_000), Err(e) if e.code == expected));
    }
    let unknown = BTreeMap::from([("missing".into(), 1)]);
    assert!(matches!(weighted::plan(&session.state, &context, &demands, &unknown, 1_000_000), Err(e) if e.code == ErrorCode::InvalidRequest));
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn quality_query_does_not_gain_authority_from_a_fenced_source() -> Result<()> {
    let _serial = lock(&SERIAL)?; let s = register(2, 8192)?;
    { let handle = resolve(s.handle())?; lock(&handle)?.source.fence(); }
    assert_eq!(ask(&s, request(1))?["error"]["code"], "adapter_unavailable");
    { let handle = resolve(s.handle())?; lock(&handle)?.grants.retain(|g| g.capability != Capability::Query); }
    assert_eq!(ask(&s, request(1))?["error"]["code"], "capability_denied");
    assert_eq!(decode(&fortress_plan(s.handle()))?["error"]["code"], "capability_denied");
    assert_eq!(decode(&fortress_commit(s.handle()))?["error"]["code"], "capability_denied");
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}
