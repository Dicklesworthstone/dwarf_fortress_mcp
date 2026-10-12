//! Complete proposal generation and actual native handler watch lifecycle.
use super::*;
use crate::live_spatial_citizens_server as server;
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_core::{CapabilityGrant, CapabilityScope, SessionId, WorkBudget};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, atomic::{AtomicUsize, Ordering}};
use std::time::Duration;

#[path = "../../dfmcp-adapter/tests/support/shelter_fixture.rs"]
#[allow(dead_code)]
mod fixture;

fn state() -> Result<LiveSpatialCitizenState> {
    fixture::state(&[(10,[0,0,5],0x11f),(11,[2,0,5],0x11f)])
}
fn context(state: &LiveSpatialCitizenState) -> Result<OperationContext> {
    let mut c = fixture::context(state)?;
    c.budget.max_bytes = 1024 * 1024;
    c.budget.max_game_ticks = 1000;
    Ok(c)
}
fn request(c: &OperationContext) -> Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"shelter_status",
        "citizens":[{"entity_id":"3298534883338","generation":1},{"entity_id":"3298534883339","generation":1}],
        "shelters":[{"min":[0,0,5],"max":[1,1,5]}],"limit":1,
        "watch":{"key":"joint-arrival","deadline_tick":c.anchor.tick.0+100,
            "poll_interval_ticks":10,"stable_observations":2}}})
}
fn execute(state: &LiveSpatialCitizenState, c: &OperationContext, input: &Value) -> Result<Value> {
    super::super::execute(state, c, input)
}
fn decoded(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| invalid("test JSON response"))
}
fn arguments(value: &Value) -> Result<Value> {
    decoded(value["watch_proposal"]["arguments_json"].as_str()
        .ok_or_else(|| invalid("test complete watch arguments missing"))?)
}

#[test]
fn every_page_preserves_all_original_targets_and_the_exact_registration_arguments() -> Result<()> {
    let state = state()?; let c = context(&state)?;
    let input = request(&c);
    let first = execute(&state,&c,&input)?;
    assert_eq!(first["arrival_truth"],"false");
    assert_eq!(first["rows"][0]["status"],"inside");
    let full = arguments(&first)?;
    assert_eq!(full["session_id"],c.session_id.to_string());
    assert_eq!(full["query"]["expected_anchor"],anchor_json(c.anchor));
    assert_eq!(full["query"]["query"]["condition"]["citizens"],input["query"]["citizens"]);
    assert_eq!(full["query"]["query"]["condition"]["shelters"],input["query"]["shelters"]);
    let encoded = first["watch_proposal"]["arguments_json"].as_str().ok_or_else(|| invalid("test arguments"))?;
    assert_eq!(first["watch_proposal"]["request_digest"],Digest32::of_bytes(encoded.as_bytes()).to_string());
    assert_eq!(first["watch_proposal"]["watch_registered"],false);
    let mut next = input.clone(); next["query"]["continuation"] = first["continuation"].clone();
    let second = execute(&state,&c,&next)?;
    assert_eq!(second["rows"][0]["status"],"outside");
    assert_eq!(second["watch_proposal"],first["watch_proposal"]);
    let mut missing = input; missing["query"]["citizens"][1]["entity_id"] = json!("3298534883340");
    let missing = execute(&state,&c,&missing)?;
    assert_eq!(missing["arrival_truth"],"unknown");
    assert_eq!(arguments(&missing)?["query"]["query"]["condition"]["citizens"][1]["entity_id"],"3298534883340");
    Ok(())
}

#[test]
fn optional_handoff_preserves_old_status_identity_and_normalizes_input_order() -> Result<()> {
    let state = state()?; let c = context(&state)?;
    let mut input = request(&c);
    input["query"].as_object_mut().ok_or_else(|| invalid("test query"))?.remove("watch");
    let old = execute(&state,&c,&input)?;
    assert!(old.get("watch_proposal").is_none());
    let expected = Digest32::of_bytes(json!({"domain":"dfmcp-shelter-status-model/1",
        "policy":shelter::POLICY,"selection":old["selection_digest"],
        "anchor":old["anchor"],"source_digest":old["source_digest"]}).to_string().as_bytes());
    assert_eq!(old["model_digest"],expected.to_string());
    input["query"]["watch"] = Value::Null;
    assert_eq!(execute(&state,&c,&input)?,old);
    let original = request(&c);
    let mut reordered = original.clone();
    reordered["query"]["citizens"].as_array_mut().ok_or_else(|| invalid("test citizens"))?.reverse();
    assert_eq!(execute(&state,&c,&original)?,execute(&state,&c,&reordered)?);
    Ok(())
}

#[test]
fn continuation_binds_every_watch_policy_field_without_freezing_page_size() -> Result<()> {
    let state = state()?; let c = context(&state)?;
    let first = execute(&state,&c,&request(&c))?;
    let mut next = request(&c); next["query"]["continuation"] = first["continuation"].clone();
    for (key,value) in [("key",json!("other")),("label",json!("other")),
        ("deadline_tick",json!(c.anchor.tick.0+200)),("poll_interval_ticks",json!(11)),
        ("stable_observations",json!(3))] {
        let mut changed = next.clone(); changed["query"]["watch"][key] = value;
        assert!(matches!(execute(&state,&c,&changed),Err(e) if e.code==ErrorCode::StaleAnchor));
    }
    let mut removed = next.clone(); removed["query"]["watch"] = Value::Null;
    assert!(matches!(execute(&state,&c,&removed),Err(e) if e.code==ErrorCode::StaleAnchor));
    next["query"]["limit"] = json!(128);
    assert_eq!(execute(&state,&c,&next)?["watch_proposal"],first["watch_proposal"]);
    Ok(())
}

#[test]
fn invalid_policy_and_insufficient_output_never_return_a_partial_watch_request() -> Result<()> {
    let state = state()?; let c = context(&state)?;
    for (key,value) in [("key",json!("")),("key",json!("x".repeat(65))),
        ("label",json!("\0")),("label",json!("x".repeat(257))),
        ("deadline_tick",json!(c.anchor.tick.0)),("deadline_tick",json!(c.anchor.tick.0+1001)),
        ("poll_interval_ticks",json!(0)),("poll_interval_ticks",json!(1_000_001)),
        ("stable_observations",json!(0)),("stable_observations",json!(65)),
        ("authority",json!(true)),("positions",json!([]))] {
        let mut q = request(&c); q["query"]["watch"][key] = value;
        assert!(execute(&state,&c,&q).is_err(),"{q}");
    }
    let mut impossible = request(&c); impossible["query"]["watch"]["stable_observations"] = json!(64);
    assert!(execute(&state,&c,&impossible).is_err());
    let mut tiny = c; tiny.budget.max_output_tokens = 1;
    assert!(matches!(execute(&state,&tiny,&request(&tiny)),Err(e) if e.code==ErrorCode::BudgetExceeded));
    Ok(())
}

struct Script { values:VecDeque<LiveSpatialCitizenObservation>, calls:Arc<AtomicUsize>, fenced:bool }
impl server::Source for Script {
    fn read(&mut self, _:Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        self.values.pop_front().ok_or_else(|| invalid("test shelter captures exhausted"))
    }
    fn poisoned(&self) -> bool { self.fenced }
    fn fence(&mut self) { self.fenced = true; }
    fn pages(&self) -> u32 { 1 }
}
struct Registered { id:SessionId, calls:Arc<AtomicUsize>, initial:OperationContext }
impl Drop for Registered {
    fn drop(&mut self) {
        let _ = server::semantic_query::release_session_resources(self.id,true,|_| Ok(String::new()));
        if let Ok(mut sessions) = server::SESSIONS.lock() { sessions.remove(&self.id); }
    }
}
fn register() -> Result<Registered> {
    let state = state()?; let mut initial = context(&state)?;
    let region = state.observation_full().ok_or_else(|| invalid("test observation"))?.spatial().terrain().map.region;
    let limits = server::CitizenSpatialLimits { spatial:server::SpatialLimits {
        operations:server::PagedOperationsLimits::default(),region },citizens:4096 };
    let id = server::next_id()?; initial.session_id = id;
    let calls = Arc::new(AtomicUsize::new(0));
    let mut values = VecDeque::new();
    for (tick,outside) in [(10,false),(10,false),(20,false),(30,true)] {
        values.push_back(fixture::observation(&[(10,[0,0,5],0x11f),
            (11,if outside {[2,0,5]} else {[0,0,5]},0x11f)],tick,7)?);
    }
    let session = server::Session {
        id,state,limits,journal:None,request:0,_watch_journal:None,_slot:server::Slot::reserve()?,
        source:Box::new(Script {values,calls:calls.clone(),fenced:false}),
        budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,
            max_output_tokens:65_536,max_wall_millis:60_000,max_game_ticks:1000,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe,Capability::Doctor].into_iter().map(|capability| CapabilityGrant {
            capability,max_risk:RiskTier::ReadOnly,scope:CapabilityScope {
                fortress_id:Some(initial.anchor.fortress_id),..CapabilityScope::default()},
            expires_at_tick:None,remaining_uses:None }).collect(),
    };
    server::lock(&server::SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));
    Ok(Registered {id,calls,initial})
}
fn ask(s:&Registered, query:Value) -> Result<Value> {
    decoded(&server::fortress_query(Some(s.id.to_string()),None,Some(query)))
}
fn operation(s:&Registered,kind:&str,watch:&str) -> Result<Value> {
    ask(s,json!({"schema":"dfmcp.query/1","query":{"kind":kind,"watch":watch}}))
}
fn handle(value:&Value) -> Result<String> {
    value["record"]["watch"].as_str().map(str::to_owned).ok_or_else(|| invalid("test watch handle"))
}

#[test]
fn actual_native_query_register_await_and_current_inspection_preserve_the_original_group() -> Result<()> {
    let _serial = crate::test_serial(); let s = register()?;
    let before = ask(&s,json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))?;
    assert_eq!(before["records"],json!([]));
    let proposed = ask(&s,request(&s.initial))?;
    assert_eq!(proposed["ok"],true,"{proposed}");
    assert_eq!(proposed["returned"],1);
    assert_eq!(s.calls.load(Ordering::SeqCst),0);
    let unchanged = ask(&s,json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))?;
    assert_eq!(unchanged["records"],json!([]));
    let args = arguments(&proposed)?;
    let registered = ask(&s,args["query"].clone())?;
    assert_eq!(registered["ok"],true,"{registered}");
    assert_eq!(registered["record"]["status"],"waiting");
    let watch = handle(&registered)?;
    let replay = ask(&s,args["query"].clone())?;
    assert_eq!(replay["replayed"],true);
    assert_eq!(replay["record"],registered["record"]);
    assert_eq!(s.calls.load(Ordering::SeqCst),0);
    let first = operation(&s,"await_watch",&watch)?;
    assert_eq!(first["ok"],true,"{first}");
    assert_eq!(first["record"]["status"],"candidate");
    assert_eq!(first["record"]["stable_observations"],1);
    let repeated = operation(&s,"await_watch",&watch)?;
    assert_eq!(repeated["record"]["evidence_digest"],first["record"]["evidence_digest"]);
    assert_eq!(repeated["record"]["stable_observations"],1);
    let completed = operation(&s,"await_watch",&watch)?;
    assert_eq!(completed["record"]["status"],"satisfied","{completed}");
    assert_eq!(completed["record"]["stable_observations"],2);
    assert_eq!(s.calls.load(Ordering::SeqCst),3);
    assert_eq!(operation(&s,"await_watch",&watch)?["record"],completed["record"]);
    assert_eq!(s.calls.load(Ordering::SeqCst),3);
    let observed = decoded(&server::fortress_observe(Some(s.id.to_string())))?;
    assert_eq!(observed["ok"],true,"{observed}");
    let historical = operation(&s,"poll_watch",&watch)?;
    assert_eq!(historical["record"]["status"],"satisfied");
    assert_eq!(historical["record"]["evaluation_current"],false);
    let current = ask(&s,json!({"schema":"dfmcp.query/1","query":{"kind":"condition_evaluation",
        "condition":args["query"]["query"]["condition"]}}))?;
    assert_eq!(current["ok"],true,"{current}");
    assert_eq!(current["evaluation"]["condition_truth"],"false");
    assert_eq!(s.calls.load(Ordering::SeqCst),4);
    Ok(())
}

#[test]
fn actual_generated_registration_refuses_an_old_anchor_without_creating_a_watch() -> Result<()> {
    let _serial = crate::test_serial(); let s = register()?;
    let proposal = ask(&s,request(&s.initial))?;
    let args = arguments(&proposal)?;
    let observed = decoded(&server::fortress_observe(Some(s.id.to_string())))?;
    assert_eq!(observed["ok"],true,"{observed}");
    let refused = ask(&s,args["query"].clone())?;
    assert_eq!(refused["ok"],false,"{refused}");
    let retained = ask(&s,json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))?;
    assert_eq!(retained["records"],json!([]));
    assert_eq!(s.calls.load(Ordering::SeqCst),1);
    Ok(())
}

#[test]
fn actual_source_fencing_and_observe_authority_never_come_from_the_handoff() -> Result<()> {
    let _serial = crate::test_serial();
    for fence in [false,true] {
        let s = register()?;
        let args = arguments(&ask(&s,request(&s.initial))?)?;
        let watch = handle(&ask(&s,args["query"].clone())?)?;
        {
            let handle = server::resolve(Some(s.id.to_string()))?;
            let mut session = server::lock(&handle)?;
            if fence { session.source.fence(); }
            else { session.grants.retain(|g| g.capability != Capability::Observe); }
        }
        let refused = operation(&s,"await_watch",&watch)?;
        assert_eq!(refused["ok"],false,"{refused}");
        assert_eq!(s.calls.load(Ordering::SeqCst),0);
        let cancelled = operation(&s,"cancel_watch",&watch)?;
        assert_eq!(cancelled["ok"],true,"{cancelled}");
        assert_eq!(cancelled["record"]["status"],"cancelled");
        assert_eq!(s.calls.load(Ordering::SeqCst),0);
    }
    Ok(())
}

#[test]
fn maximum_generated_request_and_schema_discovery_preserve_all_targets() -> Result<()> {
    let rows:Vec<_> = (10..138).map(|id| (id,[0,0,5],0x11f)).collect();
    let state = fixture::state(&rows)?; let c = context(&state)?;
    let mut input = request(&c);
    input["query"]["citizens"] = json!((10..138).map(|id| json!({"entity_id":((3u64<<40)+id).to_string(),"generation":1})).collect::<Vec<_>>());
    input["query"]["shelters"] = json!((0..16).map(|x| json!({"min":[x,0,5],"max":[x,0,5]})).collect::<Vec<_>>());
    let out = execute(&state,&c,&input)?;
    let args = arguments(&out)?;
    assert_eq!(out["returned"],1);
    assert_eq!(out["watch_proposal"]["selected_citizens"],128);
    assert_eq!(args["query"]["query"]["condition"]["citizens"].as_array().map(Vec::len),Some(128));
    assert_eq!(args["query"]["query"]["condition"]["shelters"].as_array().map(Vec::len),Some(16));
    let _serial = crate::test_serial(); let s = register()?;
    let schema = decoded(&server::fortress_query(Some(s.id.to_string()),Some("schema".into()),None))?;
    assert_eq!(schema["ok"],true,"{schema}");
    let conditions = schema["query_schema"]["$defs"]["watch_condition"]["oneOf"].as_array().ok_or_else(|| invalid("test conditions"))?;
    assert!(conditions.iter().any(|v| v["properties"]["op"]["const"]=="shelter_set"));
    let queries = schema["query_schema"]["$defs"]["query"]["oneOf"].as_array().ok_or_else(|| invalid("test queries"))?;
    assert!(queries.iter().any(|v| v["properties"]["kind"]["const"]=="shelter_status" && v["properties"]["watch"].is_object()));
    Ok(())
}
