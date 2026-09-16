//! Actual spatial MCP handlers with an injected, coherent native capture source.
use super::super::*;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_world::map_region::{Cell,MapRegion,Shape,Tile};

static SERIAL:Mutex<()>=Mutex::new(());
static NEXT_FILE:AtomicUsize=AtomicUsize::new(0);
struct Script {values:VecDeque<LiveSpatialCitizenObservation>,calls:Arc<AtomicUsize>,fenced:bool}
impl Source for Script {
    fn read(&mut self,_:Duration)->Result<LiveSpatialCitizenObservation>{
        self.calls.fetch_add(1,Ordering::SeqCst);
        self.values.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"situation fixture exhausted"))
    }
    fn poisoned(&self)->bool{self.fenced}
    fn fence(&mut self){self.fenced=true;}
    fn pages(&self)->u32{1}
}
fn put(out:&mut Vec<u8>,n:u32){out.extend_from_slice(&n.to_be_bytes());}
fn text(out:&mut Vec<u8>,value:&str){out.extend_from_slice(&(value.len() as u16).to_be_bytes());out.extend_from_slice(value.as_bytes());}
fn part(out:&mut Vec<u8>,value:&[u8]){put(out,value.len() as u32);out.extend_from_slice(value);}
fn observation(tick:u32,warnings:bool,generation:u64)->Result<LiveSpatialCitizenObservation>{
    let hex=include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)
        .map_err(|_|error(ErrorCode::InvalidRequest,"situation fixture hex"))).collect::<Result<Vec<_>>>()?;
    let base=LiveSpatialObservation::decode_payload(&bytes,7,"df".into(),"dfhack".into())?;
    let mut op=base.operations().clone();let mut map=base.terrain().clone();
    op.jobs.jobs.clear();op.attachments.clear();op.items.clear();op.buildings.clear();
    op.jobs.year_tick=tick;map.year_tick=tick;op.jobs.paused=true;map.paused=true;
    let tile=Tile{native_tiletype:1,shape:Shape::Floor,liquid_depth:0,magma:false,traffic:0,dig_designation:0,
        building_occupancy:0,unit_occupancy:0,walkable_region:1,temperature_1:10015,temperature_2:10015};
    map.map=MapRegion{region:Region{origin:[0,0,5],size:[3,3,1]},cells:vec![Cell::Visible(tile);9]};
    let mut spatial=b"DFMS1600".to_vec();part(&mut spatial,&op.encode_profile(OperationsProfile::PagedV1_4)?);part(&mut spatial,&map.encode_payload()?);
    let mut citizens=b"DFMC1800".to_vec();put(&mut citizens,40);
    for i in 0..40{
        put(&mut citizens,10+i);text(&mut citizens,"Urist");text(&mut citizens,"DWARF");
        for n in [0,1,0,5]{put(&mut citizens,n);}
        let flags:u16=if warnings&&i==0{0x11e}else if warnings&&i==1{0x11d}else{0x11f};
        citizens.extend_from_slice(&flags.to_be_bytes());put(&mut citizens,6);citizens.extend_from_slice(&[1,1]);
        citizens.extend_from_slice(&0u16.to_be_bytes());
    }
    let mut bytes=b"DFMS1800".to_vec();part(&mut bytes,&spatial);part(&mut bytes,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&bytes,generation,"df".into(),"dfhack".into())
}
struct Registered{id:SessionId,calls:Arc<AtomicUsize>}
impl Registered{fn handle(&self)->Option<String>{Some(self.id.to_string())}}
impl Drop for Registered{fn drop(&mut self){if let Ok(mut sessions)=SESSIONS.lock(){sessions.remove(&self.id);}}}
fn register(warnings:bool,next:&[(u32,bool,u64)],tokens:u32)->Result<(Registered,Value)>{
    let initial=observation(3,warnings,7)?;let region=initial.spatial().terrain().map.region;
    let mut state=LiveSpatialCitizenState::default();state.publish(initial)?;
    let anchor=state.snapshot().ok_or_else(||error(ErrorCode::InvalidRequest,"fixture state"))?.anchor();
    let calls=Arc::new(AtomicUsize::new(0));let id=next_id()?;
    let limits=CitizenSpatialLimits{spatial:SpatialLimits{operations:PagedOperationsLimits::default(),region},citizens:4096};
    let mut session=Session{id,source:Box::new(Script{values:next.iter().map(|&(t,w,g)|observation(t,w,g)).collect::<Result<VecDeque<_>>>()?,
        calls:calls.clone(),fenced:false}),state,limits,journal:None,request:0,_watch_journal:None,_slot:Slot::reserve()?,
        budget:WorkBudget{max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:tokens,max_wall_millis:60_000,..WorkBudget::default()},
        grants:[Capability::Observe,Capability::Query,Capability::Doctor].into_iter().map(|capability|CapabilityGrant{
            capability,scope:CapabilityScope{fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect()};
    let context=session.context()?;let opened=packet(Some(&session),Some(&context),"fortress.open_session",json!({"ok":true}))?;
    lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));Ok((Registered{id,calls},decode(&opened)?))
}
fn decode(raw:&str)->Result<Value>{serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"handler JSON"))}
fn ask(s:&Registered,query:Value)->Result<Value>{decode(&fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query}))))}
fn watch()->Value{json!({"kind":"watch","key":"situation-monitor","condition":{"op":"tick_at_least","value":105u64*403200+100},
    "deadline_tick":105u64*403200+500,"poll_interval_ticks":1,"stable_observations":2})}
fn release_watch(s:&Registered,handle:&Value)->Result<()>{
    assert_eq!(ask(s,json!({"kind":"cancel_watch","watch":handle}))?["ok"],true);
    assert_eq!(ask(s,json!({"kind":"release_watch","watch":handle}))?["ok"],true);Ok(())
}

#[test]
fn live_bootstrap_and_query_attention_drill_into_exact_observed_entities()->Result<()>{
    let _serial=lock(&SERIAL)?;let(s,opened)=register(true,&[(4,false,7)],65536)?;
    assert_eq!(opened["agent_turn"]["briefing"]["situation"]["signals"]["citizen_not_alive"]["observed"],1);
    let compact=decode(&fortress_query(s.handle(),Some("summary".into()),None))?;
    assert_eq!(compact["ok"],true,"{compact}");assert_eq!(compact["agent_turn"]["attention"].as_array().map(Vec::len),Some(1));
    assert_eq!(compact["agent_turn"]["briefing"]["situation"]["detail_query"]["arguments"]["mode"],"situation");
    let summary=decode(&fortress_query(s.handle(),Some("situation".into()),None))?;
    assert_eq!(summary["ok"],true,"{summary}");assert_eq!(summary["agent_turn"]["attention"].as_array().map(Vec::len),Some(2));
    let next=summary["agent_turn"]["attention"][0]["next_step"]["arguments"]["query"].clone();
    let detail=decode(&fortress_query(s.handle(),None,Some(next.clone())))?;
    assert_eq!(detail["ok"],true,"{detail}");assert_eq!(detail["anchor"],summary["anchor"]);
    assert_eq!(s.calls.load(Ordering::SeqCst),0);
    let explained=decode(&fortress_explain(s.handle()))?;
    assert_eq!(explained["ok"],true);assert_eq!(explained["situation_policy"]["rules"].as_array().map(Vec::len),Some(7));
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"],true);
    assert_eq!(decode(&fortress_query(s.handle(),None,Some(next)))?["error"]["code"],"stale_anchor");
    Ok(())
}

#[test]
fn observation_count_changes_preserve_watches_and_refuse_cross_epoch_comparisons()->Result<()>{
    let _serial=lock(&SERIAL)?;let(s,_)=register(true,&[(4,false,7),(4,false,7),(5,true,8)],65536)?;
    let created=ask(&s,watch())?;assert_eq!(created["ok"],true,"{created}");let handle=created["record"]["watch"].clone();
    let observed=decode(&fortress_observe(s.handle()))?;assert_eq!(observed["ok"],true,"{observed}");
    assert_eq!(observed["situation_comparison"]["status"],"compared");
    assert_eq!(observed["situation_comparison"]["continuous_between_observations"],false);
    assert!(observed["agent_turn"]["changes"].as_array().is_some_and(|v|v.iter().any(|c|c["rule"]=="citizen_not_alive")));
    let listed=ask(&s,json!({"kind":"watches"}))?;assert_eq!(listed["ok"],true,"{listed}");
    assert_eq!(listed["records"][0]["evidence_digest"],created["record"]["evidence_digest"]);
    assert_eq!(listed["records"][0]["last_evaluated_anchor"],created["record"]["last_evaluated_anchor"]);
    let heartbeat=decode(&fortress_wait(s.handle()))?;assert_eq!(heartbeat["ok"],true);
    assert_eq!(heartbeat["situation_comparison"]["status"],"heartbeat");assert_eq!(heartbeat["agent_turn"]["changes"],json!([]));
    let reset=decode(&fortress_observe(s.handle()))?;assert_eq!(reset["ok"],true,"{reset}");
    assert_eq!(reset["situation_comparison"]["status"],"reset");assert_eq!(reset["agent_turn"]["changes"],json!([]));
    assert_eq!(s.calls.load(Ordering::SeqCst),3);release_watch(&s,&handle)?;Ok(())
}

#[test]
fn entity_pages_keep_attention_and_watch_context_within_8192_bytes()->Result<()>{
    let _serial=lock(&SERIAL)?;let(s,_)=register(true,&[],2048)?;
    let created=ask(&s,watch())?;assert_eq!(created["ok"],true,"{created}");let handle=created["record"]["watch"].clone();
    let mut q=json!({"kind":"entities","kinds":["unit"],"fields":["alive"],"limit":128});
    let mut ids=BTreeSet::new();let mut pages=0;
    loop{
        let raw=fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":q.clone()})));
        assert!(raw.len()<=8192);let v=decode(&raw)?;assert_eq!(v["ok"],true,"{v}");
        assert_eq!(v["agent_turn"]["attention"].as_array().map(Vec::len),Some(1));
        assert_eq!(v["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len),Some(1));
        for row in v["rows"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows missing"))?{
            assert!(ids.insert(row["entity_id"].as_str().unwrap_or_default().to_owned()));
        }
        pages+=1;assert!(pages<=40);if v["continuation"].is_null(){break;}
        q["continuation"]=v["continuation"].clone();q["limit"]=json!(3);
    }
    assert!(pages>1);assert_eq!(ids.len(),40);assert_eq!(s.calls.load(Ordering::SeqCst),0);
    release_watch(&s,&handle)?;Ok(())
}

#[test]
fn watch_specific_reservation_still_refuses_overflow_before_registration()->Result<()>{
    let _serial=lock(&SERIAL)?;let(s,_)=register(true,&[],2048)?;
    {let h=resolve(s.handle())?;let mut session=lock(&h)?;let c=session.context()?;
        let projection=situation_view(&session,&c)?;
        let input=json!({"schema":"dfmcp.query/1","query":watch()});
        let allowance=situation_presentation::result_budget(&projection,&input)?;
        assert!(allowance>projection.result_byte_budget()?);
        assert!(allowance<projection.maximum_bytes);
        let payload=json!({"kind":"watch","record":{"watch":"watch:test"},"padding":"x".repeat(projection.maximum_bytes)});
        assert!(matches!(finish(&projection,payload),Err(e)if e.code==ErrorCode::BudgetExceeded));
        session.budget.max_output_tokens=1;
    }
    assert_eq!(ask(&s,watch())?["error"]["code"],"budget_exceeded");
    {let h=resolve(s.handle())?;lock(&h)?.budget.max_output_tokens=2048;}
    let listed=ask(&s,json!({"kind":"watches"}))?;assert_eq!(listed["ok"],true,"{listed}");
    assert_eq!(listed["records"],json!([]));assert_eq!(s.calls.load(Ordering::SeqCst),0);
    let created=ask(&s,watch())?;assert_eq!(created["ok"],true,"{created}");
    release_watch(&s,&created["record"]["watch"])?;Ok(())
}

#[test]
fn expired_query_authority_and_poisoned_sources_do_not_publish_current_attention()->Result<()>{
    let _serial=lock(&SERIAL)?;let(s,_)=register(true,&[(4,false,7)],65536)?;
    {let h=resolve(s.handle())?;let mut session=lock(&h)?;let tick=session.anchor()?.tick;
        for grant in &mut session.grants{if grant.capability==Capability::Query{grant.expires_at_tick=Some(tick);}}}
    let observed=decode(&fortress_observe(s.handle()))?;assert_eq!(observed["ok"],true,"{observed}");
    assert_eq!(observed["agent_turn"]["attention"],json!([]));
    assert_eq!(observed["agent_turn"]["briefing"]["situation"]["status"],"unavailable");
    assert!(observed.get("situation_comparison").is_none());drop(s);
    let(s,_)=register(true,&[],65536)?;
    let failed=decode(&fortress_observe(s.handle()))?;assert_eq!(failed["ok"],false);
    assert_eq!(failed["agent_turn"]["attention"],json!([]));
    assert_eq!(failed["agent_turn"]["continuity"]["status"],"stale");
    assert_eq!(failed["agent_turn"]["briefing"]["situation"]["status"],"unavailable");Ok(())
}

struct Files{directory:PathBuf,path:PathBuf}
impl Files{
    fn new()->Result<Self>{let directory=std::env::temp_dir().canonicalize().map_err(io_error)?
        .join(format!("dfmcp-situation-history-{}-{}",std::process::id(),NEXT_FILE.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;Ok(Self{path:directory.join("archive.bin"),directory})}
}
impl Drop for Files{fn drop(&mut self){let _=fs::remove_file(&self.path);let _=fs::remove_dir(&self.directory);}}
fn io_error(_:std::io::Error)->DfmcpError{error(ErrorCode::CorruptLedger,"situation fixture filesystem error")}

#[test]
fn historical_queries_and_offline_sessions_never_inherit_current_attention()->Result<()>{
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let(s,_)=register(false,&[(4,true,7)],65536)?;
    let(first,limits,budget)={let h=resolve(s.handle())?;let mut session=lock(&h)?;let c=session.context()?;
        history::attach(&mut session,&files.path,TailRecovery::Refuse,&c)?;
        let first=session.journal.as_ref().and_then(|j|j.entries().first()).cloned().ok_or_else(||error(ErrorCode::InvalidRequest,"first record"))?;
        (first,session.limits,session.budget)};
    let latest=decode(&fortress_observe(s.handle()))?;assert_eq!(latest["ok"],true,"{latest}");
    assert_eq!(latest["agent_turn"]["attention"].as_array().map(Vec::len),Some(2));
    let old=ask(&s,json!({"kind":"historical_query","record":first.number,"record_digest":first.record_digest.to_string(),
        "query":{"kind":"aggregate","group_by":{"kind":"entity_kind"}}}))?;
    assert_eq!(old["ok"],true,"{old}");assert_eq!(old["historical"],true);assert_eq!(old["agent_turn"]["attention"],json!([]));
    assert!(old["agent_turn"]["briefing"].get("situation").is_none());drop(s);
    let bytes=fs::read(&files.path).map_err(io_error)?;
    let id=next_id()?;let session=archive::open(id,Slot::reserve()?,&files.path,limits,budget,&[Capability::Query,Capability::Doctor])?;
    lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));let s=Registered{id,calls:Arc::new(AtomicUsize::new(0))};
    let old=decode(&fortress_query(s.handle(),Some("summary".into()),None))?;
    assert_eq!(old["ok"],true);assert_eq!(old["archive_only"],true);assert_eq!(old["agent_turn"]["attention"],json!([]));
    assert_eq!(decode(&fortress_query(s.handle(),Some("situation".into()),None))?["error"]["code"],"capability_denied");
    drop(s);assert_eq!(fs::read(&files.path).map_err(io_error)?,bytes);Ok(())
}
