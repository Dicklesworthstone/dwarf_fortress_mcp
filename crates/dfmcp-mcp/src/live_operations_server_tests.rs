use super::*;
use std::collections::VecDeque;
use dfmcp_adapter::live_operations::{building_entity_id,item_entity_id};

struct Script { observations:VecDeque<LiveOperationsObservation>,fenced:bool,calls:Arc<AtomicUsize> }
impl OperationsSource for Script {
    fn read(&mut self,_:Duration)->Result<LiveOperationsObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        self.observations.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"test source exhausted"))
    }
    fn poisoned(&self)->bool{self.fenced}
    fn fence(&mut self){self.fenced=true;}
}
struct Registered {id:SessionId,calls:Arc<AtomicUsize>}
impl Registered {
    fn handle(&self)->Option<String>{Some(self.id.to_string())}
}
impl Drop for Registered {
    fn drop(&mut self) {if let Ok(mut registry)=SESSIONS.lock(){registry.remove(&self.id);}}
}
fn observation()->Result<LiveOperationsObservation> {
    let hex=include_str!("../../dfmcp-adapter/tests/fixtures/operations_v1_3.hex").trim();
    let bytes=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"bad test hex"))).collect::<Result<Vec<_>>>()?;
    LiveOperationsObservation::decode_payload(&bytes,7,"df".to_owned(),"dfhack".to_owned())
}
fn register(tokens:u32,granted:Vec<Capability>)->Result<Registered> {
    let id=next_id()?;let slot=SessionSlot::reserve()?;
    let first=observation()?;let mut second=first.clone();second.jobs.year_tick+=1;
    second.items[0].flags|=1;second.items[0].stack_size=8;second.buildings[0].build_stage=2;
    let mut state=LiveOperationsState::default();state.publish(first)?;
    let fortress=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"test snapshot"))?.fortress_id;
    let calls=Arc::new(AtomicUsize::new(0));let limits=OperationsLimits::default();
    let session=OperationsSession {id,source:Box::new(Script {observations:VecDeque::from([second]),fenced:false,calls:Arc::clone(&calls)}),
        state,journal:None,limits,budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:MAX_OPERATIONS_BYTES as u64,
            max_output_tokens:tokens,..WorkBudget::default()},
        grants:granted.into_iter().map(|capability|CapabilityGrant {capability,
            scope:CapabilityScope {fortress_id:Some(fortress),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),request:0,_slot:slot};
    lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));Ok(Registered {id,calls})
}
fn decode(raw:&str)->Result<Value>{serde_json::from_str(raw).map_err(|_|error(ErrorCode::InternalInvariantViolation,"bad test JSON"))}
fn ask(session:&Registered,query:Value)->Result<Value> {
    decode(&fortress_query(session.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query}))))
}
fn full()->Result<Registered>{register(8192,vec![Capability::Observe,Capability::Query,Capability::Doctor])}

#[test]
fn connected_operations_queries_inspect_real_items_buildings_and_paths()->Result<()> {
    let session=full()?;
    let items=decode(&fortress_query(session.handle(),Some("items".to_owned()),None))?;
    assert_eq!(items["ok"],true);assert_eq!(items["matched"],3);
    assert_eq!(items["agent_turn"]["briefing"]["bridge_protocol"],"1.3");
    assert_eq!(items["agent_turn"]["briefing"]["runtime_admitted"],false);
    assert_eq!(items["rows"][0]["fields"]["stack_size"]["value"]["value"],5);
    assert_eq!(items["rows"][0]["fields"]["forbidden"]["value"]["value"],false);
    let building=ask(&session,json!({"kind":"inspect","entity_id":building_entity_id(20).to_string(),
        "generation":1,"fields":["build_stage","max_build_stage"]}))?;
    assert_eq!(building["ok"],true);assert_eq!(building["row"]["fields"]["build_stage"]["value"]["value"],1);
    let path=ask(&session,json!({"kind":"traverse","roots":["9"],"edge_kinds":["uses","contained_in"],
        "max_depth":4,"target":item_entity_id(31).to_string()}))?;
    assert_eq!(path["ok"],true);
    assert_eq!(path["path"]["vertices"],json!(["9",item_entity_id(30).to_string(),item_entity_id(31).to_string()]));
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    let schema=decode(&fortress_query(session.handle(),Some("schema".to_owned()),None))?;
    assert_eq!(schema["ok"],true);
    assert_eq!(schema["query_schema"]["$defs"]["query"]["oneOf"].as_array().map(Vec::len),Some(20));
    Ok(())
}

#[test]
fn native_inventory_baseline_and_construction_watch_share_one_refresh()->Result<()> {
    let session=full()?;
    let captured=ask(&session,json!({"kind":"capture","key":"inventory","max_game_ticks":100,
        "select":{"kind":"entities","kinds":["item"],"fields":["stack_size","forbidden"]}}))?;
    assert_eq!(captured["ok"],true);
    let watch=ask(&session,json!({"kind":"watch","key":"building-stage","label":"Observe construction stage 2",
        "condition":{"op":"field","entity_id":building_entity_id(20).to_string(),"generation":1,
            "field":"build_stage","comparison":"ge","value":{"type":"i64","value":2}},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(watch["ok"],true);assert_eq!(watch["record"]["terminal"],false);
    let request=json!({"kind":"await_watch","watch":watch["record"]["watch"]});
    let satisfied=ask(&session,request.clone())?;
    assert_eq!(satisfied["ok"],true);assert_eq!(satisfied["record"]["terminal"],true);
    assert_eq!(satisfied["observation_refresh"]["native_observations"],1);
    assert_eq!(session.calls.load(Ordering::SeqCst),1);
    assert_eq!(ask(&session,request)?["record"],satisfied["record"]);
    assert_eq!(session.calls.load(Ordering::SeqCst),1);
    let changed=ask(&session,json!({"kind":"changes","baseline":captured["captured"]["baseline"]}))?;
    assert_eq!(changed["ok"],true);assert_eq!(changed["change_count"],1);
    assert_eq!(changed["changes"][0]["entity_id"],item_entity_id(30).to_string());
    assert_eq!(changed["agent_turn"]["continuity"]["status"],"partial");
    assert_eq!(ask(&session,json!({"kind":"release_watch","watch":watch["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(&session,json!({"kind":"release_baseline","baseline":captured["captured"]["baseline"]}))?["ok"],true);
    Ok(())
}

#[test]
fn source_failure_preserves_anchor_and_permits_local_watch_cleanup()->Result<()> {
    let session=full()?;
    let created=ask(&session,json!({"kind":"watch","key":"future","label":"Wait for future tick",
        "condition":{"op":"tick_at_least","value":105u64*403200+50},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(created["ok"],true);
    let observed=decode(&fortress_observe(session.handle()))?;assert_eq!(observed["ok"],true);
    let failure=decode(&fortress_observe(session.handle()))?;
    assert_eq!(failure["ok"],false);assert_eq!(failure["agent_turn"]["anchor"],observed["anchor"]);
    assert_eq!(failure["agent_turn"]["continuity"]["status"],"stale");
    assert_eq!(ask(&session,json!({"kind":"entities"}))?["ok"],false);
    assert_eq!(ask(&session,json!({"kind":"watches"}))?["source_stale"],true);
    assert_eq!(ask(&session,json!({"kind":"cancel_watch","watch":created["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(&session,json!({"kind":"release_watch","watch":created["record"]["watch"]}))?["ok"],true);
    assert_eq!(decode(&fortress_doctor(session.handle()))?["status"],"source_fenced");
    Ok(())
}

#[test]
fn missing_authority_and_foreign_handles_never_reach_observation()->Result<()> {
    let session=register(8192,vec![Capability::Query])?;
    let denied=decode(&fortress_observe(session.handle()))?;
    assert_eq!(denied["error"]["code"],"capability_denied");assert!(denied.get("anchor").is_none());
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    for raw in [(1u128<<127)|FAMILY|1,SessionId::new((1u128<<127)|(1u128<<60)|1).get(),0] {
        assert!(resolve(Some(format!("{raw:032x}"))).is_err());
    }
    let malformed=decode(&fortress_query(session.handle(),None,Some(json!({"schema":"dfmcp.query/1",
        "query":{"kind":"await_watch","watch":format!("watch:{}","0".repeat(64))}}))))?;
    assert_eq!(malformed["ok"],false);assert_eq!(session.calls.load(Ordering::SeqCst),0);
    assert_eq!(decode(&fortress_commit(session.handle()))?["error"]["code"],"capability_denied");
    assert!(capabilities(Some(vec!["control_clock".to_owned()])).is_err());
    assert!(!allowed_environment("DFMCP_ADMITTED_BRIDGE_PROTOCOL"));
    assert!(!allowed_environment("DFMCP_JOBS_TOKEN"));
    assert!(allowed_environment("DFMCP_OPERATIONS_TOKEN"));Ok(())
}

#[test]
fn complete_operations_queries_respect_the_minimum_packet_budget()->Result<()> {
    let session=register(2048,vec![Capability::Observe,Capability::Query,Capability::Doctor])?;
    for kind in ["jobs","items","buildings","summary"] {
        let raw=fortress_query(session.handle(),Some(kind.to_owned()),None);
        assert!(raw.len()<=8192);let value=decode(&raw)?;
        assert_eq!(value["ok"],true,"mode {kind}: {raw}");
        assert_eq!(value["agent_turn"]["briefing"]["mutation_admissible"],false);
        assert_eq!(value["agent_turn"]["coverage"]["status"],"partial");
    }
    Ok(())
}

mod production_cases {
    include!("operations_production_tests.rs");
}

#[cfg(unix)]
mod history_cases {
    include!("operations_history_tests.rs");
}
