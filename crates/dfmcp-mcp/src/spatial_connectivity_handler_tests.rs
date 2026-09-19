//! Actual spatial/1.8 dispatcher and private paired journals, with no native read.
//! Run with --test-threads=1, as the development session registry has two slots.
use super::super::*;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_world::map_region::{Cell, MapRegion, Shape, Tile};
#[path="../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;

static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError { error(ErrorCode::CorruptLedger,"connectivity fixture I/O") }
struct Files { directory:PathBuf, observations:PathBuf, watches:PathBuf }
impl Files {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-connectivity-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self { observations:directory.join("observations.bin"),watches:directory.join("watches.bin"),directory })
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _=fs::remove_file(&self.observations); let _=fs::remove_file(&self.watches);
        let _=fs::remove_dir(&self.directory);
    }
}
struct NeverRead { calls:Arc<AtomicUsize>, fenced:bool }
impl Source for NeverRead {
    fn read(&mut self,_:Duration)->Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        Err(error(ErrorCode::AdapterUnavailable,"connectivity must not read the source"))
    }
    fn poisoned(&self)->bool {self.fenced}
    fn fence(&mut self) {self.fenced=true;}
    fn pages(&self)->u32 {0}
}
fn observation(tick:u32,split:bool)->Result<LiveSpatialCitizenObservation> {
    let base=fixture::observation(tick,0,1,false)?;
    let mut operations=base.spatial().operations().clone();let mut terrain=base.spatial().terrain().clone();
    operations.jobs.jobs.clear();operations.attachments.clear();operations.buildings.clear();
    let floor=Cell::Visible(Tile {native_tiletype:1,shape:Shape::Floor,liquid_depth:0,magma:false,
        traffic:0,dig_designation:0,building_occupancy:0,unit_occupancy:0,walkable_region:1,
        temperature_1:10015,temperature_2:10015});
    terrain.map=MapRegion {region:Region {origin:[0,1,5],size:[5,1,1]},cells:vec![floor;5]};
    if split {terrain.map.cells[2]=Cell::Hidden;}
    let part=|out:&mut Vec<u8>,value:&[u8]| {out.extend_from_slice(&(value.len() as u32).to_be_bytes());out.extend_from_slice(value);};
    let mut spatial=b"DFMS1600".to_vec();
    part(&mut spatial,&operations.encode_profile(OperationsProfile::PagedV1_4)?);
    part(&mut spatial,&terrain.encode_payload()?);
    let mut citizens=b"DFMC1800".to_vec();citizens.extend_from_slice(&0u32.to_be_bytes());
    let mut bytes=b"DFMS1800".to_vec();part(&mut bytes,&spatial);part(&mut bytes,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&bytes,7,"df".into(),"dfhack".into())
}
struct Registered(SessionId);
impl Registered {
    fn new(s:Session)->Result<Self> {
        let id=s.id;lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(s)));Ok(Self(id))
    }
    fn call(&self,input:Value)->Result<Value> {
        serde_json::from_str(&fortress_query(Some(self.0.to_string()),None,Some(input)))
            .map_err(|_|error(ErrorCode::InvalidRequest,"connectivity packet"))
    }
}
impl Drop for Registered {
    fn drop(&mut self) {let _=session_release::close(Some(self.0.to_string()),true);}
}
fn input(section:&str)->Value {
    json!({"schema":"dfmcp.query/1","query":{"kind":"map_connectivity","section":section,"limit":1}})
}

#[test]
fn actual_dispatch_preserves_watches_and_supports_fenced_history_and_archive_reopen()->Result<()> {
    let files=Files::new()?;let calls=Arc::new(AtomicUsize::new(0));
    let mut state=LiveSpatialCitizenState::default();state.publish(observation(3,false)?)?;
    let anchor=state.snapshot().ok_or_else(||error(ErrorCode::InvalidRequest,"fixture anchor"))?.anchor();
    let limits=CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),
        region:Region {origin:[0,1,5],size:[5,1,1]}},citizens:4096};
    let budget=WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,
        max_output_tokens:65536,max_wall_millis:60000,..WorkBudget::default()};
    let mut s=Session {id:next_id()?,source:Box::new(NeverRead {calls:calls.clone(),fenced:false}),state,
        limits,journal:None,budget,request:0,_watch_journal:None,_slot:Slot::reserve()?,
        grants:[Capability::Observe,Capability::Query,Capability::Doctor].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect()};
    let c=s.context()?;history::attach(&mut s,&files.observations,TailRecovery::Refuse,&c)?;
    let rc=history::replay_context(&s,&c);
    let journal=s.journal.as_mut().ok_or_else(||error(ErrorCode::CorruptLedger,"fixture journal"))?;
    journal.append(observation(4,true)?,&rc)?;
    let first=journal.entries()[0].clone();s.state=journal.state().clone();
    let c=s.context()?;
    durable_watches::finish_open(&mut s,&c,Some(&files.watches),json!({"ok":true}))?;
    let current=s.anchor()?;
    let registered=Registered::new(s)?;
    let watched=registered.call(json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"retain-me",
        "condition":{"op":"tick_at_least","value":u64::MAX},"deadline_tick":current.tick.0+100}}))?;
    assert_eq!(watched["record"]["status"],"waiting");
    let watches=registered.call(json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))?["records"].clone();
    let watch_bytes=fs::read(&files.watches).map_err(io_error)?;
    let observation_bytes=fs::read(&files.observations).map_err(io_error)?;
    {
        let handle=resolve(Some(registered.0.to_string()))?;let mut s=lock(&handle)?;
        s.budget.max_bytes=8192;s.budget.max_output_tokens=2048;
    }
    let out=registered.call(input("all"))?;
    assert_eq!(out["kind"],"map_connectivity");assert_eq!(out["summary"]["components"],2);
    assert_eq!(out["native_captures"],0);assert!(out.to_string().len()<=8192);
    assert_eq!(out["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len),Some(1));
    let historical=json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query",
        "record":first.number,"record_digest":first.record_digest.to_string(),"query":{
            "kind":"map_connectivity","section":"bottlenecks","limit":1}}});
    let old=registered.call(historical.clone())?;
    assert_eq!(old["summary"]["components"],1);assert_eq!(old["rows"][0]["position"],json!([2,1,5]));
    assert_eq!(old["anchor"],anchor_json(first.anchor));assert_eq!(old["historical"],true);
    assert!(old.to_string().len()<=8192);
    let after=registered.call(json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))?;
    assert_eq!(after["records"],watches);assert_eq!(fs::read(&files.watches).map_err(io_error)?,watch_bytes);
    assert_eq!(fs::read(&files.observations).map_err(io_error)?,observation_bytes);
    {
        let handle=resolve(Some(registered.0.to_string()))?;let mut s=lock(&handle)?;s.source.fence();
    }
    assert_eq!(registered.call(input("all"))?["error"]["code"],"adapter_unavailable");
    assert_eq!(registered.call(historical.clone())?["structural_digest"],old["structural_digest"]);
    assert_eq!(calls.load(Ordering::SeqCst),0);
    drop(registered);
    let archive=archive::open(next_id()?,Slot::reserve()?,&files.observations,limits,budget,&[Capability::Query])?;
    let recovered=Registered::new(archive)?;
    let out=recovered.call(historical.clone())?;
    assert_eq!(out["archive_only"],true);assert_eq!(out["current_freshness_proven"],false);
    assert_eq!(out["rows"],old["rows"]);assert_eq!(out["structural_digest"],old["structural_digest"]);
    assert_eq!(out["bridge_connection_present"],false);
    assert_eq!(fs::read(&files.observations).map_err(io_error)?,observation_bytes);
    let mut corrupted=observation_bytes;corrupted[first.offset as usize+50]^=1;
    fs::write(&files.observations,&corrupted).map_err(io_error)?;
    assert_eq!(recovered.call(historical)?["error"]["code"],"corrupt_ledger");
    assert_eq!(fs::read(&files.observations).map_err(io_error)?,corrupted);
    assert_eq!(calls.load(Ordering::SeqCst),0);
    Ok(())
}
