use super::*;
use std::collections::VecDeque;
use dfmcp_adapter::live_map::tile_entity_id;
use dfmcp_world::map_region::{MapRegion,Shape,Tile};

static SERIAL:Mutex<()>=Mutex::new(());
struct Script{values:VecDeque<LiveMapObservation>,fenced:bool,calls:Arc<AtomicUsize>}
impl MapSource for Script{
    fn read(&mut self,_:Duration)->Result<LiveMapObservation>{self.calls.fetch_add(1,Ordering::SeqCst);
        self.values.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"test map source exhausted"))}
    fn poisoned(&self)->bool{self.fenced}fn fence(&mut self){self.fenced=true;}
}
struct Registered{id:SessionId,calls:Arc<AtomicUsize>}
impl Registered{fn handle(&self)->Option<String>{Some(self.id.to_string())}}
impl Drop for Registered{fn drop(&mut self){if let Ok(mut registry)=SESSIONS.lock(){registry.remove(&self.id);}}}
fn fixture()->Result<LiveMapObservation>{let hex=include_str!("../../dfmcp-adapter/tests/fixtures/map_v1_5.hex").trim();
    let bytes=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"test hex"))).collect::<Result<Vec<_>>>()?;
    LiveMapObservation::decode_payload(&bytes,7,"df".to_owned(),"dfhack".to_owned())}
fn register(first:LiveMapObservation,tokens:u32,granted:Vec<Capability>)->Result<Registered>{
    let id=next_id()?;let slot=Slot::reserve()?;let mut second=first.clone();second.year_tick+=1;
    if let Some(Cell::Visible(t))=second.map.cells.get_mut(2){t.liquid_depth=0;}
    let volume=first.map.cells.len();let mut state=LiveMapState::default();state.publish(first)?;
    let fortress=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"test snapshot"))?.fortress_id;
    let calls=Arc::new(AtomicUsize::new(0));
    let s=MapSession{id,source:Box::new(Script{values:VecDeque::from([second]),fenced:false,calls:Arc::clone(&calls)}),
        state,budget:WorkBudget{max_entities:volume as u32+1,max_bytes:MAX_MAP_BYTES as u64,max_output_tokens:tokens,..WorkBudget::default()},
        grants:granted.into_iter().map(|capability|CapabilityGrant{capability,scope:CapabilityScope{fortress_id:Some(fortress),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),request:0,_slot:slot};
    lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(s)));Ok(Registered{id,calls})
}
fn full()->Result<Registered>{register(fixture()?,8192,vec![Capability::Observe,Capability::Query,Capability::Doctor])}
fn decode(text:&str)->Result<Value>{serde_json::from_str(text).map_err(|_|error(ErrorCode::InternalInvariantViolation,"test JSON"))}
fn ask(s:&Registered,q:Value)->Result<Value>{decode(&fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":q}))))}
fn route(start:[u32;3],goal:[u32;3],limit:u32)->Value{json!({"kind":"map_route","start":start,"goal":goal,"limit":limit})}

#[test]
fn native_tile_queries_and_complementary_stair_route_use_actual_handlers()->Result<()>{
    let _serial=lock(&SERIAL)?;let s=full()?;
    let tiles=decode(&fortress_query(s.handle(),Some("tiles".to_owned()),None))?;
    assert_eq!(tiles["ok"],true);assert_eq!(tiles["matched"],24);
    assert_eq!(tiles["agent_turn"]["briefing"]["visible_tiles"],19);
    assert_eq!(tiles["agent_turn"]["briefing"]["hidden_tiles"],1);
    assert_eq!(tiles["agent_turn"]["briefing"]["unallocated_tiles"],4);
    assert_eq!(tiles["agent_turn"]["briefing"]["bridge_protocol"],"1.5");
    let hidden=ask(&s,json!({"kind":"inspect","entity_id":tile_entity_id([15,15,1])?.to_string(),"generation":1,"fields":["shape"]}))?;
    assert_eq!(hidden["ok"],true);assert!(hidden["row"]["fields"]["shape"].to_string().contains("redacted"));
    let path=ask(&s,route([14,15,1],[14,15,2],64))?;
    assert_eq!(path["ok"],true);assert_eq!(path["status"],"candidate_found");assert_eq!(path["model_steps"],1);
    assert_eq!(path["rows"][1]["position"],json!([14,15,2]));assert_eq!(path["unit_path_proven"],false);
    assert_eq!(path["global_unreachability_proven"],false);assert_eq!(path["safety_proven"],false);
    assert_eq!(ask(&s,route([14,15,1],[16,15,1],64))?["status"],"endpoint_excluded");
    assert_eq!(s.calls.load(Ordering::SeqCst),0);Ok(())
}

#[test]
fn liquid_watch_and_baseline_share_one_real_refresh_without_claiming_safe_terrain()->Result<()>{
    let _serial=lock(&SERIAL)?;let s=full()?;
    let capture=ask(&s,json!({"kind":"capture","key":"liquid","max_game_ticks":100,
        "select":{"kind":"entities","kinds":["tile_feature"],"fields":["liquid_depth"]}}))?;
    assert_eq!(capture["ok"],true);
    let watch=ask(&s,json!({"kind":"watch","key":"dry-tile","label":"Observed zero liquid depth",
        "condition":{"op":"field","entity_id":tile_entity_id([16,15,1])?.to_string(),"generation":1,
            "field":"liquid_depth","comparison":"eq","value":{"type":"u64","value":0}},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(watch["ok"],true);assert_eq!(watch["record"]["terminal"],false);
    let request=json!({"kind":"await_watch","watch":watch["record"]["watch"]});
    let completed=ask(&s,request.clone())?;assert_eq!(completed["ok"],true);assert_eq!(completed["record"]["terminal"],true);
    assert_eq!(completed["observation_refresh"]["native_observations"],1);assert_eq!(s.calls.load(Ordering::SeqCst),1);
    assert_eq!(ask(&s,request)?["record"],completed["record"]);assert_eq!(s.calls.load(Ordering::SeqCst),1);
    let changes=ask(&s,json!({"kind":"changes","baseline":capture["captured"]["baseline"]}))?;
    assert_eq!(changes["ok"],true);assert_eq!(changes["change_count"],1);
    assert_eq!(changes["changes"][0]["entity_id"],tile_entity_id([16,15,1])?.to_string());
    assert_eq!(ask(&s,json!({"kind":"release_watch","watch":watch["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(&s,json!({"kind":"release_baseline","baseline":capture["captured"]["baseline"]}))?["ok"],true);Ok(())
}
fn flat()->Result<LiveMapObservation>{let mut v=fixture()?;v.map=MapRegion{region:Region{origin:[0,0,0],size:[16,16,1]},
    cells:vec![Cell::Visible(Tile{native_tiletype:3,shape:Shape::Floor,liquid_depth:0,magma:false,traffic:0,dig_designation:0,
        building_occupancy:0,unit_occupancy:0,walkable_region:1,temperature_1:10015,temperature_2:10015});256]};Ok(v)}
#[test]
fn route_paging_keeps_active_work_and_binds_session_snapshot_and_endpoints()->Result<()>{
    let _serial=lock(&SERIAL)?;let caps=vec![Capability::Observe,Capability::Query,Capability::Doctor];
    let s=register(flat()?,2048,caps.clone())?;let other=register(flat()?,2048,caps)?;
    let created=ask(&s,json!({"kind":"watch","key":"review","label":"Wait for map review",
        "condition":{"op":"tick_at_least","value":105u64*403200+50},"deadline_tick":105u64*403200+100,
        "poll_interval_ticks":1,"stable_observations":1}))?;assert_eq!(created["ok"],true);
    let mut q=route([0,0,0],[15,15,0],2);let first=ask(&s,q.clone())?;assert_eq!(first["ok"],true);
    let mut saved=q.clone();saved["continuation"]=first["continuation"].clone();
    assert_eq!(ask(&other,saved.clone())?["error"]["code"],"stale_anchor");
    let mut changed=saved.clone();changed["goal"]=json!([14,15,0]);assert_eq!(ask(&s,changed)?["error"]["code"],"stale_anchor");
    let mut positions=Vec::new();let mut pages=0;
    loop{let text=fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":q})));
        assert!(text.len()<=8192);let v=decode(&text)?;assert_eq!(v["ok"],true,"{text}");
        assert_eq!(v["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len),Some(1));
        let rows=v["rows"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"route rows"))?;
        assert!(!rows.is_empty());positions.extend(rows.iter().map(|r|r["position"].clone()));pages+=1;assert!(pages<=31);
        if v["continuation"].is_null(){break;}q["continuation"]=v["continuation"].clone();q["limit"]=json!(3);
    }
    assert_eq!(positions.len(),31);assert_eq!(positions[0],json!([0,0,0]));assert_eq!(positions[30],json!([15,15,0]));
    assert_eq!(s.calls.load(Ordering::SeqCst),0);assert_eq!(decode(&fortress_observe(s.handle()))?["ok"],true);
    assert_eq!(ask(&s,saved)?["error"]["code"],"stale_anchor");
    assert_eq!(ask(&s,json!({"kind":"cancel_watch","watch":created["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(&s,json!({"kind":"release_watch","watch":created["record"]["watch"]}))?["ok"],true);Ok(())
}
#[test]
fn map_schema_adds_only_its_route_and_does_not_widen_base_variants()->Result<()>{
    let _serial=lock(&SERIAL)?;let s=full()?;let out=decode(&fortress_query(s.handle(),Some("schema".to_owned()),None))?;
    assert_eq!(out["ok"],true);let base:Value=serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"schema"))?;
    let variants=out["query_schema"]["$defs"]["query"]["oneOf"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"variants"))?;
    assert_eq!(variants.len(),17);assert_eq!(json!(&variants[..16]),base["$defs"]["query"]["oneOf"]);
    assert_eq!(variants[16]["properties"]["kind"]["const"],"map_route");assert_eq!(s.calls.load(Ordering::SeqCst),0);Ok(())
}
#[test]
fn source_failure_retains_previous_anchor_and_local_watch_management()->Result<()>{
    let _serial=lock(&SERIAL)?;let s=full()?;
    let w=ask(&s,json!({"kind":"watch","key":"future","label":"Later observation","condition":{"op":"tick_at_least","value":105u64*403200+50},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;assert_eq!(w["ok"],true);
    let observed=decode(&fortress_observe(s.handle()))?;assert_eq!(observed["ok"],true);
    let failed=decode(&fortress_observe(s.handle()))?;assert_eq!(failed["ok"],false);assert_eq!(failed["anchor"],observed["anchor"]);
    assert_eq!(failed["agent_turn"]["continuity"]["status"],"stale");
    assert_eq!(ask(&s,route([14,15,1],[14,15,2],1))?["error"]["code"],"adapter_unavailable");
    assert_eq!(ask(&s,json!({"kind":"watches"}))?["source_stale"],true);
    assert_eq!(ask(&s,json!({"kind":"cancel_watch","watch":w["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(&s,json!({"kind":"release_watch","watch":w["record"]["watch"]}))?["ok"],true);
    assert_eq!(decode(&fortress_doctor(s.handle()))?["status"],"source_fenced");Ok(())
}
#[test]
fn authority_region_and_malformed_routes_fail_without_io()->Result<()>{
    let _serial=lock(&SERIAL)?;let s=register(fixture()?,8192,vec![Capability::Doctor])?;
    let denied=ask(&s,route([14,15,1],[14,15,2],1))?;assert_eq!(denied["error"]["code"],"capability_denied");assert!(denied.get("anchor").is_none());
    assert_eq!(s.calls.load(Ordering::SeqCst),0);let allowed=full()?;
    for q in [json!({"kind":"map_route","start":[0,0],"goal":[0,0,0]}),route([0,0,0],[14,15,2],1),
        json!({"kind":"map_route","start":[14,15,1],"goal":[14,15,2],"max_work":1}),
        json!({"kind":"map_route","start":[14,15,1],"goal":[14,15,2],"reveal":true})]{assert_eq!(ask(&allowed,q)?["ok"],false);}
    assert!(map_queries::parse_region(&json!({"origin":[0,0,0],"size":[128,128,128]})).is_err());
    assert!(resolve(Some(format!("{:032x}",(1u128<<127)|FAMILY|1))).is_err());
    assert!(!allowed_environment("DFMCP_ADMISSION_TICKET"));assert!(!allowed_environment("DFMCP_OPERATIONS_TOKEN"));
    assert!(capabilities(Some(vec!["control_clock".to_owned()])).is_err());
    assert_eq!(decode(&fortress_commit(allowed.handle()))?["error"]["code"],"capability_denied");
    assert_eq!(allowed.calls.load(Ordering::SeqCst),0);Ok(())
}
