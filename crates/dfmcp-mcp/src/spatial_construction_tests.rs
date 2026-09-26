use super::*;
use super::super::super::semantic_query;
use dfmcp_adapter::live_jobs::{LiveJob, LiveJobObservation};
use dfmcp_adapter::live_operations::{JobItemAttachment, LiveBuilding, LiveItem, LiveOperationsObservation, OperationsProfile};
use dfmcp_adapter::live_map::LiveMapObservation;
use dfmcp_adapter::live_spatial::{LiveSpatialObservation, LiveSpatialState};
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};
use std::sync::atomic::{AtomicU64, Ordering};

fn operations() -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7, df_version: "df".into(), dfhack_version: "dfhack".into(),
            year: 250, year_tick: 100, paused: true, site_id: 2, world_folder: "test-world".into(),
            next_job_id: 100, jobs: vec![],
        },
        next_building_id: 100, next_item_id: 100,
        buildings: vec![LiveBuilding {
            native_id: 10, building_type: 1, type_key: "Bed".into(),
            x1: 12, y1: 14, x2: 12, y2: 14, z: 3, build_stage: 3, max_build_stage: 3,
        }],
        items: vec![LiveItem {
            native_id: 20, item_type: 1, type_key: "BED".into(), subtype: -1,
            material_type: 0, material_index: 2, stack_size: 1, raw_position: MapCoord::new(12, 14, 3),
            flags: 256, container_native_id: None, holder_building_native_id: Some(10),
        }], attachments: vec![],
    }
}
fn job(kind: &str) -> LiveJob {
    LiveJob { native_id: 5, job_type: 1, type_key: kind.into(), reaction: String::new(),
        suspended: false, repeating: false, position: MapCoord::new(12, 14, 3),
        worker_native_id: None, holder_native_id: Some(10), completion_timer: -1,
        attached_item_count: 0, required_item_filter_count: 0 }
}
fn spatial(o: &LiveOperationsObservation) -> LiveSpatialObservation {
    // Actual decoder and publication, with a deliberately unavailable terrain
    // cell: construction analysis must not pretend to prove terrain or access.
    let mut map = b"DFMM1500".to_vec();
    for n in [o.jobs.year, o.jobs.year_tick] { map.extend_from_slice(&n.to_be_bytes()); }
    map.push(u8::from(o.jobs.paused));
    map.extend_from_slice(&(o.jobs.site_id as u32).to_be_bytes());
    map.extend_from_slice(&(o.jobs.world_folder.len() as u16).to_be_bytes());
    map.extend_from_slice(o.jobs.world_folder.as_bytes());
    for n in [128u32,128,16, 0,0,0, 1,1,1, 1] { map.extend_from_slice(&n.to_be_bytes()); }
    map.push(0);
    LiveMapObservation::decode_payload(&map, o.jobs.bridge_generation, "df".into(), "dfhack".into()).unwrap();
    let op = o.encode_profile(OperationsProfile::PagedV1_4).unwrap();
    let mut wire = b"DFMS1600".to_vec();
    for part in [&op, &map] {
        wire.extend_from_slice(&(part.len() as u32).to_be_bytes()); wire.extend_from_slice(part);
    }
    LiveSpatialObservation::decode_payload(&wire, o.jobs.bridge_generation, "df".into(), "dfhack".into()).unwrap()
}
fn state(o: &LiveOperationsObservation) -> LiveSpatialState {
    let mut state = LiveSpatialState::default(); state.publish(spatial(o)).unwrap(); state
}
fn context(state: &LiveSpatialState) -> OperationContext {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    OperationContext {
        session_id: SessionId::new((1u128 << 96) + u128::from(NEXT.fetch_add(1, Ordering::Relaxed))),
        request_id: RequestId::new(1), anchor: state.snapshot().unwrap().anchor(),
        budget: WorkBudget { max_wall_millis: 60_000, max_output_tokens: 65_536, ..WorkBudget::default() },
        grants: vec![CapabilityGrant { capability: Capability::Query, scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }],
        cancellation_requested: false,
    }
}
fn request(c: &OperationContext, monitor: bool) -> Value {
    let mut out = json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(c.anchor),
        "query":{"kind":"construction_progress","targets":[{"building_native_id":10,
            "expected_generation":1,"expected_type":"Bed","item_native_id":20}]}});
    if monitor { out["query"]["monitor"] = json!({"key_prefix":"bedrooms","deadline_tick":c.anchor.tick.0+100}); }
    out
}
fn query(state: &LiveSpatialState, c: &OperationContext, request: &Value) -> Value {
    assert!(super::super::handles(request));
    super::super::execute(state, c, request).unwrap()
}
fn run(state: &LiveSpatialState, c: &OperationContext, input: &Value) -> Value {
    let out = semantic_query::execute_with_publisher(state.snapshot().unwrap(), c, input, |value| Ok(value.to_string())).unwrap();
    serde_json::from_str(&out).unwrap()
}
fn evaluate(state: &LiveSpatialState, c: &OperationContext, watch: &Value) -> Value {
    semantic_query::execute(state.snapshot().unwrap(), c, &json!({"schema":"dfmcp.query/1",
        "query":{"kind":"condition_evaluation","condition":watch["query"]["condition"],
            "failure_condition":watch["query"]["failure_condition"]}})).unwrap()
}
fn close(c: &OperationContext) {
    semantic_query::release_session_resources(c.session_id, true, |value| Ok(value.to_string())).unwrap();
}

#[test]
fn routed_analysis_preserves_enclosing_anchor_and_does_not_register_or_read() {
    let state = state(&operations()); let c = context(&state);
    let before = state.snapshot().unwrap().clone();
    let result = query(&state, &c, &request(&c, true));
    assert_eq!(result["anchor"], anchor_json(c.anchor));
    assert_eq!(result["source_digest"], state.source_digest().unwrap().to_string());
    assert_eq!(result["rows"][0]["status"], "satisfied_at_observation");
    assert_eq!(result["coverage"]["native_captures"], 0);
    for name in ["watch_registered","mutation_dispatched","native_effect_completed_proven","placement_receipt_verified","safety_proven"] {
        assert_eq!(result[name], false);
    }
    assert_eq!(state.snapshot().unwrap(), &before);
    let watches = run(&state, &c, &json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}));
    assert_eq!(watches["records"], json!([])); close(&c);
}

#[test]
fn proposal_uses_real_condition_evaluator_and_explicit_stable_watch_lifecycle() {
    let mut o = operations(); o.buildings[0].build_stage = 0;
    o.items[0].flags = 66; o.items[0].holder_building_native_id = None;
    let mut j = job("ConstructBuilding"); j.attached_item_count = 1; o.jobs.jobs.push(j);
    o.attachments.push(JobItemAttachment { job_native_id:5,item_native_id:20,role:0,filter_index:-1 });
    let mut state = state(&o); let mut c = context(&state);
    let result = query(&state, &c, &request(&c, true));
    let watch = result["rows"][0]["monitoring"]["watch_request"].clone();
    assert_eq!(evaluate(&state, &c, &watch)["evaluation"]["status"], "condition_not_met");
    let registered = run(&state, &c, &watch);
    assert_eq!(registered["record"]["status"], "waiting");
    let poll = json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":registered["record"]["watch"]}});
    o.jobs.jobs.clear(); o.attachments.clear(); o.buildings[0].build_stage = 3;
    o.items[0].flags = 256; o.items[0].holder_building_native_id = Some(10); o.jobs.year_tick += 1;
    state.publish(spatial(&o)).unwrap(); c.anchor = state.snapshot().unwrap().anchor();
    let candidate = run(&state, &c, &poll);
    assert_eq!(candidate["record"]["status"], "candidate");
    assert_eq!(candidate["record"]["stable_observations"], 1);
    assert_eq!(run(&state, &c, &poll)["record"], candidate["record"]);
    o.jobs.year_tick += 1; state.publish(spatial(&o)).unwrap(); c.anchor = state.snapshot().unwrap().anchor();
    let finished = run(&state, &c, &poll);
    assert_eq!(finished["record"]["status"], "satisfied");
    assert_eq!(finished["record"]["stable_observations"], 2);
    close(&c);
}

#[test]
fn generated_condition_matches_the_adapter_across_every_supported_item_flag_word() {
    let original = operations(); let first = state(&original); let first_c = context(&first);
    let result = query(&first, &first_c, &request(&first_c, true));
    let watch = &result["rows"][0]["monitoring"]["watch_request"];
    for flags in 0..512 {
        let mut o = original.clone(); o.items[0].flags = flags;
        let state = state(&o); let c = context(&state);
        let result = query(&state, &c, &request(&c, false));
        assert_eq!(evaluate(&state, &c, watch)["evaluation"]["eligible_success_sample"],
            result["rows"][0]["condition_met_at_observation"], "flags={flags}");
    }
}

#[test]
fn removal_failure_and_exact_item_links_are_not_approximated_by_job_absence() {
    let original = operations(); let first = state(&original); let first_c = context(&first);
    let result = query(&first, &first_c, &request(&first_c, true));
    let watch = &result["rows"][0]["monitoring"]["watch_request"];
    for case in 0..9 {
        let mut o = original.clone();
        let expected = match case {
            0 => { o.items[0].holder_building_native_id = None; "condition_not_met" }
            1 => { o.items[0].type_key = "CHAIR".into(); "condition_not_met" }
            2 => { o.items.clear(); "condition_not_met" }
            3 => { o.buildings[0].build_stage = 2; "condition_not_met" }
            4 => { o.buildings[0].max_build_stage = 4; o.buildings[0].build_stage = 4; "condition_not_met" }
            5 => { o.jobs.jobs.push(job("DestroyBuilding")); "failure_condition_met" }
            6 => { o.jobs.jobs.push(job("ConstructBuilding")); "condition_not_met" }
            7 => {
                let mut j = job("StoreItemInStockpile"); j.holder_native_id = None; j.attached_item_count = 1;
                o.jobs.jobs.push(j);
                o.attachments.push(JobItemAttachment { job_native_id:5,item_native_id:20,role:0,filter_index:-1 });
                "condition_not_met"
            }
            _ => {
                let mut container = o.items[0].clone(); container.native_id = 21;
                container.holder_building_native_id = None; o.items.push(container);
                o.items[0].container_native_id = Some(21); "condition_not_met"
            }
        };
        let state = state(&o); let c = context(&state);
        assert_eq!(evaluate(&state, &c, watch)["evaluation"]["status"], expected, "case={case}");
    }
}

#[test]
fn stale_proposal_registration_and_recycled_entities_are_rejected() {
    let mut o = operations(); let mut state = state(&o); let mut c = context(&state);
    let result = query(&state, &c, &request(&c, true));
    let watch = result["rows"][0]["monitoring"]["watch_request"].clone();
    let mut missing = o.clone(); missing.buildings.clear(); missing.items[0].holder_building_native_id = None;
    missing.jobs.year_tick += 1; state.publish(spatial(&missing)).unwrap(); c.anchor = state.snapshot().unwrap().anchor();
    assert!(semantic_query::execute_with_publisher(state.snapshot().unwrap(), &c, &watch, |v| Ok(v.to_string())).is_err());
    assert_eq!(evaluate(&state, &c, &watch)["evaluation"]["status"], "blocked_unknown");
    o.jobs.year_tick += 2; state.publish(spatial(&o)).unwrap(); c.anchor = state.snapshot().unwrap().anchor();
    assert_eq!(evaluate(&state, &c, &watch)["evaluation"]["status"], "invalidated_reference");
    let result = query(&state, &c, &request(&c, true));
    assert_eq!(result["rows"][0]["status"], "identity_mismatch");
    assert_eq!(result["rows"][0]["monitoring"]["available"], false); close(&c);
}

#[test]
fn complete_summary_pagination_and_cursor_bindings_cover_every_target() {
    let mut o = operations();
    for id in 11..42 { let mut b = o.buildings[0].clone(); b.native_id = id; o.buildings.push(b); }
    o.buildings[31].build_stage = 0;
    let state = state(&o); let c = context(&state);
    let mut input = request(&c, true);
    input["query"]["targets"] = json!((10..42).map(|id|json!({"building_native_id":id})).collect::<Vec<_>>());
    input["query"]["limit"] = json!(1);
    let first = query(&state, &c, &input);
    assert_eq!(first["total_rows"], 32);
    assert_eq!(first["summary"]["by_status"]["no_construction_job"], 1);
    assert_eq!(first["summary"]["all_conditions_met_at_observation"], false);
    let mut next = input.clone(); next["query"]["continuation"] = first["continuation"].clone();
    next["query"]["limit"] = json!(8);
    assert_eq!(query(&state, &c, &next)["analysis_digest"], first["analysis_digest"]);
    next["query"]["targets"].as_array_mut().unwrap().reverse();
    assert_eq!(query(&state, &c, &next)["analysis_digest"], first["analysis_digest"]);
    let mut changed = next.clone(); changed["query"]["monitor"]["key_prefix"] = json!("other");
    assert!(super::super::execute(&state, &c, &changed).is_err());
    let mut changed_c = c.clone(); changed_c.session_id = SessionId::new(999);
    assert!(super::super::execute(&state, &changed_c, &next).is_err());
    let mut small = c.clone(); small.budget.max_bytes = 64; small.budget.max_output_tokens = 16;
    assert!(super::super::execute(&state, &small, &input).is_err());
    let mut bounded = c.clone(); bounded.budget.max_bytes = 8192; bounded.budget.max_output_tokens = 2048;
    let page = query(&state, &bounded, &input);
    assert!(page.to_string().len() <= 8192);
    assert_eq!(page["rows"][0]["monitoring"]["available"], true);
}

#[test]
fn missing_and_unsupported_targets_return_explanations_not_weaker_watch_proposals() {
    for case in 0..3 {
        let mut o = operations(); let mut input;
        match case {
            0 => { o.buildings.clear(); o.items[0].holder_building_native_id = None; }
            1 => { o.items.clear(); }
            _ => { o.buildings[0].type_key = "Workshop".into(); }
        }
        let state = state(&o); let c = context(&state); input = request(&c, true);
        input["query"]["targets"][0]["expected_type"] = Value::Null;
        let result = query(&state, &c, &input);
        assert_eq!(result["rows"][0]["monitoring"]["available"], false);
        assert!(result["rows"][0]["monitoring"].get("watch_request").is_none());
    }
}

#[test]
fn schema_discovery_and_closed_request_authority_budget_checks() {
    let state = state(&operations()); let c = context(&state); let input = request(&c, true);
    let schema = super::super::schema().unwrap();
    assert!(schema["$defs"]["query"]["oneOf"].as_array().unwrap().iter()
        .any(|s|s["properties"]["kind"]["const"]=="construction_progress"));
    for (field, value) in [("commit",json!(true)),("path",json!("/tmp/untrusted")),("protocol",json!("1.19"))] {
        let mut changed = input.clone(); changed["query"][field] = value;
        assert!(super::super::execute(&state, &c, &changed).is_err());
    }
    for case in 0..8 {
        let mut changed = input.clone();
        match case {
            0 => changed["query"]["monitor"]["key_prefix"] = json!(""),
            1 => changed["query"]["monitor"]["key_prefix"] = json!("x".repeat(33)),
            2 => changed["query"]["monitor"]["deadline_tick"] = json!(c.anchor.tick.0),
            3 => changed["query"]["monitor"]["deadline_tick"] = json!(c.anchor.tick.0+c.budget.max_game_ticks+1),
            4 => changed["query"]["monitor"]["poll_interval_ticks"] = json!(0),
            5 => changed["query"]["monitor"]["stable_observations"] = json!(65),
            6 => changed["query"]["limit"] = json!(33),
            _ => changed["query"]["max_work"] = json!(1),
        }
        assert!(super::super::execute(&state, &c, &changed).is_err(), "case={case}");
    }
    let mut denied = c.clone(); denied.grants.clear();
    assert!(super::super::execute(&state, &denied, &input).is_err());
    let mut cancelled = c; cancelled.cancellation_requested = true;
    assert!(super::super::execute(&state, &cancelled, &input).is_err());
}

#[test]
fn rejected_watch_publication_preserves_existing_registry() {
    let state = state(&operations()); let c = context(&state);
    let result = query(&state, &c, &request(&c, true));
    let watch = &result["rows"][0]["monitoring"]["watch_request"];
    assert!(semantic_query::execute_with_publisher(state.snapshot().unwrap(), &c, watch,
        |_|Err(budget("injected response rejection"))).is_err());
    assert_eq!(run(&state, &c, &json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))["records"], json!([]));
    close(&c);
}

#[test]
fn generated_predicates_match_the_independently_evaluated_fixture() {
    let state = state(&operations()); let c = context(&state);
    let result = query(&state, &c, &request(&c, true));
    let generated = &result["rows"][0]["monitoring"]["watch_request"]["query"];
    let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/construction_condition_v1.json")).unwrap();
    assert_eq!(generated["condition"], fixture["condition"]);
    assert_eq!(generated["failure_condition"], fixture["failure_condition"]);
}
