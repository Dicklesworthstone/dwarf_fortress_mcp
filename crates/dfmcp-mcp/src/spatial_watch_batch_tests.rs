//! Actual spatial query handlers with injected captures and private journals.
use super::*;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::GameTick;

static SERIAL:Mutex<()>=Mutex::new(());
static FILE_ID:AtomicUsize=AtomicUsize::new(0);
fn io_error(_:std::io::Error)->DfmcpError {error(ErrorCode::CorruptLedger,"watch batch fixture I/O")}
struct Files {directory:PathBuf,observations:PathBuf,watches:PathBuf}
impl Files {
    fn new()->Result<Self> {
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-watch-batch-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self {observations:directory.join("observations.bin"),watches:directory.join("watches.bin"),directory})
    }
}
impl Drop for Files {fn drop(&mut self) {
    let _=fs::remove_file(&self.watches);let _=fs::remove_file(&self.observations);let _=fs::remove_dir(&self.directory);
}}
struct Script {values:VecDeque<LiveSpatialCitizenObservation>,calls:Arc<AtomicUsize>,fenced:bool,corrupt:Option<PathBuf>}
impl Source for Script {
    fn read(&mut self,_:Duration)->Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        let value=self.values.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"injected capture exhaustion"))?;
        if let Some(path)=self.corrupt.take() {
            fs::OpenOptions::new().append(true).open(path).and_then(|mut f|f.write_all(b"x")).map_err(io_error)?;
        }
        Ok(value)
    }
    fn poisoned(&self)->bool {self.fenced}
    fn fence(&mut self) {self.fenced=true;}
    fn pages(&self)->u32 {1}
}
fn part(out:&mut Vec<u8>,bytes:&[u8]) {out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());out.extend_from_slice(bytes);}
fn observation(tick:u32)->Result<LiveSpatialCitizenObservation> {
    let hex=include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes=(0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)
        .map_err(|_|error(ErrorCode::InvalidRequest,"fixture hex"))).collect::<Result<Vec<_>>>()?;
    let base=LiveSpatialObservation::decode_payload(&bytes,7,"df".into(),"dfhack".into())?;
    let mut operations=base.operations().clone();let mut terrain=base.terrain().clone();
    operations.jobs.year_tick=tick;terrain.year_tick=tick;operations.jobs.paused=true;terrain.paused=true;
    let mut spatial=b"DFMS1600".to_vec();part(&mut spatial,&operations.encode_profile(OperationsProfile::PagedV1_4)?);
    part(&mut spatial,&terrain.encode_payload()?);
    let mut citizens=b"DFMC1800".to_vec();citizens.extend_from_slice(&0u32.to_be_bytes());
    let mut combined=b"DFMS1800".to_vec();part(&mut combined,&spatial);part(&mut combined,&citizens);
    LiveSpatialCitizenObservation::decode_payload(&combined,7,"df".into(),"dfhack".into())
}
struct Registered {id:SessionId,calls:Arc<AtomicUsize>}
impl Registered {fn handle(&self)->Option<String> {Some(self.id.to_string())}}
impl Drop for Registered {fn drop(&mut self) {if let Ok(mut sessions)=SESSIONS.lock() {sessions.remove(&self.id);}}}
fn decode(text:&str)->Result<Value> {serde_json::from_str(text).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))}
fn register(files:&Files,tick:u32,next:&[u32],corrupt:bool)->Result<Registered> {
    let initial=observation(tick)?;let region=initial.spatial().terrain().map.region;
    let limits=CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),region},citizens:4096};
    let mut state=LiveSpatialCitizenState::default();state.publish(initial)?;
    let anchor=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"fixture snapshot"))?.anchor();
    let calls=Arc::new(AtomicUsize::new(0));
    let source=Script {values:next.iter().map(|n|observation(*n)).collect::<Result<VecDeque<_>>>()?,
        calls:calls.clone(),fenced:false,corrupt:corrupt.then(||files.watches.clone())};
    let mut s=Session {id:next_id()?,source:Box::new(source),state,limits,journal:None,
        budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:65536,
            max_wall_millis:60000,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe,Capability::Doctor].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),
        request:0,_watch_journal:None,_slot:Slot::reserve()?};
    let mut c=s.context()?;history::attach(&mut s,&files.observations,TailRecovery::Refuse,&c)?;c.anchor=s.anchor()?;
    durable_watches::finish_open(&mut s,&c,Some(&files.watches),json!({"ok":true}))?;
    let id=s.id;lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(s)));Ok(Registered {id,calls})
}
fn ask(s:&Registered,q:Value)->Result<Value> {
    decode(&fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":q}))))
}
fn watch(s:&Registered,key:&str,stable:u32)->Result<Value> {
    let result=ask(s,json!({"kind":"watch","key":key,"condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":stable}))?;
    assert_eq!(result["ok"],true,"{result}");Ok(result["record"]["watch"].clone())
}
fn checkpoint(value:&Value)->Result<u64> {value["watch_persistence"]["checkpoint"].as_u64()
    .ok_or_else(||error(ErrorCode::InvalidRequest,"test checkpoint missing"))}
fn require_success(value:&Value)->Result<()> {
    assert_eq!(value["ok"],true,"{value}");Ok(())
}

#[test]
fn mcp_await_batch_uses_one_capture_and_one_checkpoint_for_all_watches()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[4,5],false)?;
    watch(&s,"a",3)?;watch(&s,"b",3)?;
    let prior=ask(&s,json!({"kind":"watches"}))?;let mut count=checkpoint(&prior)?;
    for step in 1..=2 {
        let result=ask(&s,json!({"kind":"await_watches"}))?;require_success(&result)?;
        assert_eq!(result["native_captures"],1);assert_eq!(result["sampled"],2);assert_eq!(result["durable"],true);
        assert_eq!(checkpoint(&result)?,count+1);count+=1;
        for row in result["records"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows"))? {
            assert_eq!(row["stable_observations"],step+1);assert_eq!(row["last_evaluated_anchor"],result["anchor"]);
        }
        assert_eq!(s.calls.load(Ordering::SeqCst),step as usize);assert_eq!(result["all_satisfied"],step==2);
    }
    let bytes=fs::read(&files.watches).map_err(io_error)?;
    let replay=ask(&s,json!({"kind":"await_watches"}))?;require_success(&replay)?;
    assert_eq!(replay["native_captures"],0);assert_eq!(replay["advanced"],0);assert_eq!(checkpoint(&replay)?,count);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?,bytes);assert_eq!(s.calls.load(Ordering::SeqCst),2);Ok(())
}

#[test]
fn mcp_poll_batch_evaluates_existing_capture_without_another_read()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[4],false)?;
    watch(&s,"a",2)?;watch(&s,"b",2)?;
    require_success(&decode(&fortress_observe(s.handle()))?)?;
    let result=ask(&s,json!({"kind":"poll_watches"}))?;require_success(&result)?;
    assert_eq!(result["all_satisfied"],true);assert_eq!(result["native_captures"],0);
    assert_eq!(s.calls.load(Ordering::SeqCst),1);Ok(())
}

#[test]
fn invalid_selection_and_inadequate_preflight_budget_do_not_capture_or_write()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[4],false)?;
    let a=watch(&s,"a",3)?;let before=fs::read(&files.watches).map_err(io_error)?;
    for selection in [json!([]),json!([a.clone(),a]),json!([format!("watch:{}","f".repeat(64))])] {
        assert_eq!(ask(&s,json!({"kind":"await_watches","watches":selection}))?["ok"],false);
    }
    {let h=resolve(s.handle())?;let mut session=lock(&h)?;session.budget.max_output_tokens=1;}
    let result=ask(&s,json!({"kind":"await_watches"}))?;assert_eq!(result["ok"],false);
    assert_eq!(s.calls.load(Ordering::SeqCst),0);assert_eq!(fs::read(&files.watches).map_err(io_error)?,before);Ok(())
}

#[test]
fn bridge_failure_preserves_all_watches_and_restart_batch_restores_stability()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[],false)?;
    watch(&s,"a",2)?;watch(&s,"b",2)?;
    let before=fs::read(&files.watches).map_err(io_error)?;
    assert_eq!(ask(&s,json!({"kind":"await_watches"}))?["error"]["code"],"adapter_unavailable");
    assert_eq!(fs::read(&files.watches).map_err(io_error)?,before);drop(s);
    let s=register(&files,4,&[5,6],false)?;
    for step in 1..=2 {
        let result=ask(&s,json!({"kind":"await_watches"}))?;require_success(&result)?;
        for row in result["records"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows"))? {
            assert_eq!(row["stable_observations"],step);assert_eq!(row["fresh_observation_required"],false);
        }
        assert_eq!(result["all_satisfied"],step==2);
    }
    assert_eq!(s.calls.load(Ordering::SeqCst),2);Ok(())
}

#[test]
fn watch_storage_changed_during_capture_cannot_publish_the_candidate_set()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[4],true)?;
    watch(&s,"a",2)?;watch(&s,"b",2)?;
    let before=fs::read(&files.watches).map_err(io_error)?;
    let result=ask(&s,json!({"kind":"await_watches"}))?;
    assert_eq!(result["ok"],false);assert_eq!(result["error"]["code"],"corrupt_ledger");
    assert_eq!(fs::read(&files.watches).map_err(io_error)?,[before,b"x".to_vec()].concat());
    assert_eq!(s.calls.load(Ordering::SeqCst),1);Ok(())
}

#[test]
fn authority_expiring_at_new_capture_does_not_commit_watch_progress()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[5],false)?;
    watch(&s,"a",2)?;watch(&s,"b",2)?;let before=fs::read(&files.watches).map_err(io_error)?;
    {let h=resolve(s.handle())?;let mut session=lock(&h)?;
        let expiration=GameTick(session.anchor()?.tick.0+1);
        for grant in &mut session.grants {grant.expires_at_tick=Some(expiration);}}
    let result=ask(&s,json!({"kind":"await_watches"}))?;
    assert_eq!(result["ok"],false);assert_eq!(s.calls.load(Ordering::SeqCst),1);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?,before);Ok(())
}

#[test]
fn schema_discovers_batches_but_archive_and_historical_queries_refuse_them()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[],false)?;
    let schema=decode(&fortress_query(s.handle(),Some("schema".into()),None))?;require_success(&schema)?;
    let variants=schema["query_schema"]["$defs"]["query"]["oneOf"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"variants"))?;
    for kind in ["poll_watches","await_watches"] {
        assert!(variants.iter().any(|v|v["properties"]["kind"]["const"]==kind));
        let list=ask(&s,json!({"kind":"history"}))?;let row=&list["rows"][0];
        assert_eq!(ask(&s,json!({"kind":"historical_query","record":row["record"],"record_digest":row["record_digest"],
            "query":{"kind":kind}}))?["ok"],false);
    }
    let (limits,budget)={let h=resolve(s.handle())?;let session=lock(&h)?;(session.limits,session.budget)};drop(s);
    let mut session=archive::open(next_id()?,Slot::reserve()?,&files.observations,limits,budget,&[Capability::Query])?;
    let c=session.context()?;
    for kind in ["poll_watches","await_watches"] {
        assert!(matches!(execute(&mut session,&c,&json!({"schema":"dfmcp.query/1","query":{"kind":kind}})),
            Err(e)if e.code==ErrorCode::CapabilityDenied));
    }
    Ok(())
}

#[test]
fn full_eight_watch_batch_fits_the_default_8192_token_budget()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let s=register(&files,3,&[4],false)?;
    for index in 0..8 {watch(&s,&format!("batch-{index}"),2)?;}
    {let handle=resolve(s.handle())?;let mut session=lock(&handle)?;session.budget.max_output_tokens=8192;}
    let raw=fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":{"kind":"await_watches"}})));
    assert!(raw.len()<=32768);let result=decode(&raw)?;require_success(&result)?;
    assert_eq!(result["selected"],8);assert_eq!(result["sampled"],8);assert_eq!(result["all_satisfied"],true);
    assert_eq!(result["records"].as_array().map(Vec::len),Some(8));assert_eq!(s.calls.load(Ordering::SeqCst),1);Ok(())
}

#[path="spatial_watch_count_tests.rs"]
mod count_tests;
