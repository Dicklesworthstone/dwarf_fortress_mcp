//! Archive bootstrap helper and actual MCP handlers over real private journals.
//! No environment mutation, native service, network mock or bridge credentials.
use super::*;
use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_adapter::operations_journal::open_profile_journal;
use dfmcp_world::map_region::{Cell,MapRegion,Shape,Tile};

static SERIAL:Mutex<()>=Mutex::new(());
static NEXT_FILE:AtomicUsize=AtomicUsize::new(0);
fn io_error(_:std::io::Error)->DfmcpError {error(ErrorCode::CorruptLedger,"archive fixture I/O")}
struct Files {directory:PathBuf,path:PathBuf}
impl Files {
    fn new()->Result<Self> {
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?
            .join(format!("dfmcp-archive-handlers-{}-{}",std::process::id(),NEXT_FILE.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self {path:directory.join("observations.bin"),directory})
    }
    fn populate(&self,workers:u32)->Result<()> {
        let first=observation(3,workers)?;let mut state=LiveSpatialCitizenState::default();state.publish(first.clone())?;
        let anchor=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"fixture snapshot"))?.anchor();
        let c=OperationContext {session_id:next_id()?,request_id:RequestId::new(1),anchor,budget:WorkBudget {
            max_entities:100_000,max_bytes:16*1024*1024,max_wall_millis:60_000,..WorkBudget::default()},
            grants:grants(&[Capability::Observe,Capability::Query],Some(anchor.fortress_id)),cancellation_requested:false};
        let mut journal=open_profile_journal::<Spatial18>(&self.path,&c,JournalLimits::default(),TailRecovery::Refuse)?;
        journal.append(first,&c)?;journal.append(observation(4,workers.saturating_sub(1))?,&c)?;Ok(())
    }
}
impl Drop for Files {
    fn drop(&mut self) {let _=fs::remove_file(&self.path);let _=fs::remove_dir(&self.directory);}
}
fn put(out:&mut Vec<u8>,n:u32) {out.extend_from_slice(&n.to_be_bytes());}
fn text(out:&mut Vec<u8>,s:&str) {out.extend_from_slice(&(s.len() as u16).to_be_bytes());out.extend_from_slice(s.as_bytes());}
fn part(out:&mut Vec<u8>,bytes:&[u8]) {put(out,bytes.len() as u32);out.extend_from_slice(bytes);}
fn observation(tick:u32,workers:u32)->Result<LiveSpatialCitizenObservation> {
    let hex=include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)
        .map_err(|_|error(ErrorCode::InvalidRequest,"fixture hex"))).collect::<Result<Vec<_>>>()?;
    let base=LiveSpatialObservation::decode_payload(&bytes,7,"df".into(),"dfhack".into())?;
    let mut op=base.operations().clone();let mut map=base.terrain().clone();
    op.jobs.jobs.clear();op.buildings.clear();op.items.clear();op.attachments.clear();
    op.jobs.year_tick=tick;map.year_tick=tick;op.jobs.paused=true;map.paused=true;
    let tile=Tile {native_tiletype:1,shape:Shape::Floor,liquid_depth:0,magma:false,traffic:0,dig_designation:0,
        building_occupancy:0,unit_occupancy:0,walkable_region:1,temperature_1:10015,temperature_2:10015};
    map.map=MapRegion {region:Region {origin:[0,0,5],size:[3,3,1]},cells:vec![Cell::Visible(tile);9]};
    let mut spatial=b"DFMS1600".to_vec();part(&mut spatial,&op.encode_profile(OperationsProfile::PagedV1_4)?);
    part(&mut spatial,&map.encode_payload()?);
    let mut citizens=b"DFMC1800".to_vec();put(&mut citizens,workers);
    for i in 0..workers {
        put(&mut citizens,10+i);text(&mut citizens,"Urist");text(&mut citizens,"DWARF");
        for n in [0,1,0,5] {put(&mut citizens,n);}
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());put(&mut citizens,6);citizens.extend_from_slice(&[1,1]);
        citizens.extend_from_slice(&1u16.to_be_bytes());put(&mut citizens,0);text(&mut citizens,"CARPENTRY");
        for n in [5,5,1] {put(&mut citizens,n);}
    }
    let mut combined=b"DFMS1800".to_vec();part(&mut combined,&spatial);part(&mut combined,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&combined,7,"df".into(),"dfhack".into())
}
fn limits()->CitizenSpatialLimits {
    CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),
        region:Region {origin:[0,0,5],size:[3,3,1]}},citizens:4096}
}
fn budget(tokens:u32)->WorkBudget {
    WorkBudget {max_entities:limits().entity_limit(),max_bytes:1024*1024,max_output_tokens:tokens,
        max_wall_millis:60_000,..WorkBudget::default()}
}
struct Registered {id:SessionId}
impl Registered {fn handle(&self)->Option<String> {Some(self.id.to_string())}}
impl Drop for Registered {fn drop(&mut self) {if let Ok(mut sessions)=SESSIONS.lock() {sessions.remove(&self.id);}}}
fn decode(raw:&str)->Result<Value> {serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"handler JSON"))}
fn register(files:&Files,tokens:u32)->Result<(Registered,Value)> {
    let mut session=open(next_id()?,Slot::reserve()?,&files.path,limits(),budget(tokens),&requested_capabilities(None)?)?;
    assert!(session.source.archive_only());assert_eq!(session.source.pages(),0);assert!(session._watch_journal.is_none());
    assert!(session.grants.iter().all(|g|matches!(g.capability,Capability::Query|Capability::Doctor)));
    let c=session.context()?;let opened=packet(&session,&c,"fortress.open_session",None,json!({"ok":true}))?;
    let id=session.id;lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));
    Ok((Registered {id},decode(&opened)?))
}
fn ask(s:&Registered,q:Value)->Result<Value> {
    decode(&fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":q}))))
}
fn plan(workers:u32)->Value {json!({"kind":"workforce_plan","demands":[{"key":"wood","workers":workers,
    "target":[0,0,5],"skill_key":"CARPENTRY"}],"limit":128})}
fn historical(entry:&Value,q:Value)->Value {json!({"kind":"historical_query","record":entry["record"],
    "record_digest":entry["record_digest"],"query":q})}

#[test]
fn archive_bootstrap_and_actual_queries_never_claim_live_freshness()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(3)?;
    let before=fs::read(&files.path).map_err(io_error)?;let (s,opened)=register(&files,65536)?;
    assert_eq!(opened["archive_only"],true);assert_eq!(opened["live"],false);
    assert_eq!(opened["agent_turn"]["continuity"]["status"],"partial");
    for mode in ["summary","citizens","jobs","buildings","items","tiles","history","production"] {
        let result=decode(&fortress_query(s.handle(),Some(mode.into()),None))?;
        assert_eq!(result["ok"],true,"mode={mode}: {result}");
        assert_eq!(result["historical"],true);assert_eq!(result["native_captures"],0);
        assert_eq!(result["current_freshness_proven"],false);assert_eq!(result["bridge_connection_present"],false);
    }
    let p=ask(&s,plan(2))?;assert_eq!(p["ok"],true,"{p}");assert_eq!(p["assigned_workers"],2);
    let doctor=decode(&fortress_doctor(s.handle()))?;assert_eq!(doctor["status"],"archive_only");
    assert_eq!(doctor["agent_turn"]["briefing"]["watch_evidence_loaded"],false);
    drop(s);assert_eq!(fs::read(&files.path).map_err(io_error)?,before);Ok(())
}

#[test]
fn archived_workforce_and_route_drilldowns_stay_on_the_selected_record()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(2)?;let (s,_)=register(&files,65536)?;
    let history=ask(&s,json!({"kind":"history"}))?;let first=history["rows"][0].clone();
    let newest=ask(&s,plan(2))?;assert_eq!(newest["assigned_workers"],1);
    let old=ask(&s,historical(&first,plan(2)))?;assert_eq!(old["ok"],true,"{old}");
    assert_eq!(old["assigned_workers"],2);assert_eq!(old["anchor"],first["anchor"]);
    let route=old["rows"][0]["route_query"].clone();
    assert_eq!(route["query"]["kind"],"historical_query");assert_eq!(route["query"]["record"],1);
    let drilled=decode(&fortress_query(s.handle(),None,Some(route)))?;
    assert_eq!(drilled["ok"],true,"{drilled}");assert_eq!(drilled["anchor"],first["anchor"]);
    assert_eq!(drilled["journal_record"]["record_digest"],first["record_digest"]);
    assert_eq!(ask(&s,plan(2))?["anchor"],newest["anchor"]);Ok(())
}

#[test]
fn archive_pages_fit_full_packet_budgets_and_never_repeat_workers()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(41)?;let (s,_)=register(&files,2048)?;
    let mut q=plan(40);let mut ids=BTreeSet::new();let mut pages=0;
    loop {
        let raw=fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":q.clone()})));
        assert!(raw.len()<=8192);let page=decode(&raw)?;assert_eq!(page["ok"],true,"{page}");
        assert_eq!(page["assigned_workers"],40);assert_eq!(page["archive_only"],true);
        for row in page["rows"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows absent"))? {
            assert!(ids.insert(row["citizen"]["entity_id"].as_str().unwrap_or_default().to_owned()));
        }
        pages+=1;assert!(pages<=40);
        if page["continuation"].is_null() {break;}
        assert_eq!(page["agent_turn"]["coverage"]["continuation"],page["continuation"]);
        q["continuation"]=page["continuation"].clone();q["limit"]=json!(3);
    }
    assert_eq!(ids.len(),40);assert!(pages>1);Ok(())
}

#[test]
fn archive_rejects_monitoring_baselines_refresh_and_all_effect_tools()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(2)?;let (s,_)=register(&files,65536)?;
    let before=fs::read(&files.path).map_err(io_error)?;
    for name in ["watch","poll_watch","await_watch","watches","cancel_watch","release_watch","capture","changes","baselines","release_baseline"] {
        assert_eq!(ask(&s,json!({"kind":name}))?["error"]["code"],"capability_denied","{name}");
    }
    for call in [fortress_observe,fortress_wait,fortress_plan,fortress_commit,fortress_checkpoint,fortress_restore] {
        assert_eq!(decode(&call(s.handle()))?["error"]["code"],"capability_denied");
    }
    assert_eq!(decode(&fortress_cancel(s.handle(),None,None))?["error"]["code"],"capability_denied");
    {let handle=resolve(s.handle())?;let mut session=lock(&handle)?;
        session.grants.push(grants(&[Capability::Observe],None).remove(0));let c=session.context()?;
        assert!(matches!(session.refresh(&c),Err(e)if e.code==ErrorCode::CapabilityDenied));
        assert!(session.source.read(Duration::from_millis(1)).is_err());}
    drop(s);assert_eq!(fs::read(&files.path).map_err(io_error)?,before);Ok(())
}

#[test]
fn archive_history_continuations_bind_session_and_record_identity()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(2)?;let (s,_)=register(&files,65536)?;
    let first=ask(&s,json!({"kind":"history","limit":1}))?;
    let token=first["continuation"].clone();assert!(token.is_string());
    let second=ask(&s,json!({"kind":"history","limit":1,"continuation":token}))?;
    assert_eq!(second["rows"][0]["record"],2);
    let mut selection=historical(&first["rows"][0],plan(1));selection["record_digest"]=json!("00".repeat(32));
    assert_eq!(ask(&s,selection)?["error"]["code"],"stale_anchor");
    assert_eq!(ask(&s,historical(&first["rows"][0],json!({"kind":"watch"})))?["error"]["code"],"capability_denied");
    drop(s);let (next,_)=register(&files,65536)?;
    assert_eq!(ask(&next,json!({"kind":"history","continuation":token}))?["error"]["code"],"stale_anchor");
    Ok(())
}

#[test]
fn archive_query_and_doctor_fence_changed_storage_without_hiding_the_error()->Result<()> {
    use std::io::Write;
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(2)?;let (s,_)=register(&files,65536)?;
    fs::OpenOptions::new().append(true).open(&files.path).and_then(|mut f|f.write_all(b"x")).map_err(io_error)?;
    assert_eq!(ask(&s,plan(1))?["error"]["code"],"corrupt_ledger");
    assert_eq!(decode(&fortress_doctor(s.handle()))?["error"]["code"],"corrupt_ledger");
    Ok(())
}

#[test]
fn archive_schema_advertises_nineteen_read_variants_including_condition_inspection()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;files.populate(2)?;let (s,_)=register(&files,65536)?;
    let result=decode(&fortress_query(s.handle(),Some("schema".into()),None))?;
    assert_eq!(result["ok"],true,"{result}");
    let variants=result["query_schema"]["$defs"]["query"]["oneOf"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"schema variants"))?;
    assert_eq!(variants.len(),19);
    for kind in ["historical_changes","historical_series","production_diagnosis","inventory_plan","item_quantity","production_portfolio","condition_evaluation"] {
        assert!(variants.iter().any(|v|v["properties"]["kind"]["const"]==kind));
    }
    assert_eq!(result["query_schema"]["$defs"]["archive_stateless"]["oneOf"].as_array().map(Vec::len),Some(15));
    for variant in variants {assert_ne!(variant["properties"]["kind"]["const"],"watch");}
    Ok(())
}

#[test]
fn archive_configuration_and_bootstrap_fail_without_creating_replacement_history()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;
    assert!(validate_configuration(None,TailRecovery::Refuse,None).is_err());
    assert!(validate_configuration(Some(&files.path),TailRecovery::TruncateIncomplete,None).is_err());
    assert!(validate_configuration(Some(&files.path),TailRecovery::Refuse,Some(&files.path)).is_err());
    for caps in [vec!["observe".into(),"query".into()],vec!["doctor".into()],vec!["control_clock".into()]] {
        assert!(requested_capabilities(Some(caps)).is_err());
    }
    assert!(open(next_id()?,Slot::reserve()?,&files.path,limits(),budget(65536),&[Capability::Query]).is_err());
    assert!(!files.path.exists());
    files.populate(2)?;let before=fs::read(&files.path).map_err(io_error)?;
    let mut wrong=limits();wrong.spatial.region.origin[0]=1;
    assert!(open(next_id()?,Slot::reserve()?,&files.path,wrong,budget(65536),&[Capability::Query]).is_err());
    assert_eq!(fs::read(&files.path).map_err(io_error)?,before);Ok(())
}

#[path = "spatial_history_changes_tests.rs"]
mod comparisons;
