//! Actual spatial query handlers with injected read source and real private
//! compressed observation/watch journals. Run with --test-threads=1, like the
//! existing spatial handler fixtures sharing the two-slot process registry.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::GameTick;

static FILE_NUMBER: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError { error(ErrorCode::CorruptLedger,"monitor fixture I/O") }
struct Files { directory:PathBuf, observation:PathBuf, watch:PathBuf }
impl Files {
    fn new() -> Result<Self> {
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-historical-monitor-{}-{}",std::process::id(),FILE_NUMBER.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self {observation:directory.join("observations.bin"),watch:directory.join("watches.bin"),directory})
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _=fs::remove_file(&self.watch); let _=fs::remove_file(&self.observation); let _=fs::remove_dir(&self.directory);
    }
}
struct NoRead { calls:Arc<AtomicUsize>, fenced:bool }
impl Source for NoRead {
    fn read(&mut self,_:Duration)->Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        Err(error(ErrorCode::AdapterUnavailable,"historical analysis must not enter source"))
    }
    fn poisoned(&self)->bool{self.fenced}
    fn fence(&mut self){self.fenced=true;}
    fn pages(&self)->u32{0}
}
struct Registered { id:SessionId, session:Arc<Mutex<Session>>, calls:Arc<AtomicUsize> }
impl Registered {
    fn install(session:Session,calls:Arc<AtomicUsize>)->Result<Self> {
        let id=session.id;let handle=Arc::new(Mutex::new(session));
        lock(&SESSIONS)?.insert(id,Arc::clone(&handle));
        Ok(Self {id,session:handle,calls})
    }
    fn query(&self,input:Value)->Result<Value> {
        serde_json::from_str(&fortress_query(Some(self.id.to_string()),None,Some(input)))
            .map_err(|_|invalid("handler returned invalid JSON"))
    }
    fn schema(&self)->Result<Value> {
        serde_json::from_str(&fortress_query(Some(self.id.to_string()),Some("schema".into()),None))
            .map_err(|_|invalid("schema handler returned invalid JSON"))
    }
    fn current(&self)->Result<StateAnchor>{lock(&self.session)?.anchor()}
}
impl Drop for Registered {
    fn drop(&mut self){let _=fortress_cancel(Some(self.id.to_string()),Some("session".into()),Some(true));}
}
fn live(files:&Files)->Result<Registered> {
    let first=fixture::observation(3,2,2,false)?;
    let region=first.spatial().terrain().map.region;
    let mut state=LiveSpatialCitizenState::default();state.publish(first)?;
    let anchor=state.snapshot().ok_or_else(||invalid("fixture snapshot"))?.anchor();
    let limits=CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),region},citizens:4096};
    let calls=Arc::new(AtomicUsize::new(0));
    let mut s=Session {id:next_id()?,source:Box::new(NoRead {calls:Arc::clone(&calls),fenced:false}),state,limits,journal:None,
        budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:65536,
            max_wall_millis:60000,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe,Capability::Doctor].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),
        request:0,_watch_journal:None,_slot:Slot::reserve()?};
    let c=s.context()?;history::attach(&mut s,&files.observation,TailRecovery::Refuse,&c)?;
    for tick in 4..=6 {
        let c=s.context()?;let read=history::replay_context(&s,&c);
        let journal=s.journal.as_mut().ok_or_else(||invalid("fixture journal"))?;
        journal.append(fixture::observation(tick,2,2,false)?,&read)?;
        s.state=journal.state().clone();
    }
    let c=s.context()?;
    durable_watches::finish_open(&mut s,&c,Some(&files.watch),json!({"ok":true}))?;
    Registered::install(s,calls)
}
fn request(session:&Registered,stable:u32,details:bool)->Result<Value> {
    let s=lock(&session.session)?;
    let entries=s.journal.as_ref().ok_or_else(||invalid("fixture journal"))?.entries();
    let first=entries.first().ok_or_else(||invalid("first record"))?;
    let last=entries.last().ok_or_else(||invalid("last record"))?;
    Ok(json!({"schema":"dfmcp.query/1","query":{"kind":"historical_watch_replay",
        "from":{"record":first.number,"record_digest":first.record_digest.to_string()},
        "to":{"record":last.number,"record_digest":last.record_digest.to_string()},
        "definition":{"condition":{"op":"paused","value":true},"deadline_tick":first.anchor.tick.0+50,
            "stable_observations":stable,"poll_interval_ticks":1},
        "detail":if details{"evidence"}else{"summary"}}}))
}
fn bytes(files:&Files)->Result<(Vec<u8>,Vec<u8>)>{Ok((fs::read(&files.observation).map_err(io_error)?,fs::read(&files.watch).map_err(io_error)?))}

#[test]
fn live_handler_keeps_paired_watches_unchanged_and_accepts_fenced_sources() -> Result<()> {
    let files=Files::new()?;let live=live(&files)?;let current=live.current()?;
    let created=live.query(json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"current-monitor",
        "condition":{"op":"paused","value":true},"deadline_tick":current.tick.0+50,"stable_observations":2}}))?;
    assert_eq!(created["record"]["status"],"candidate");
    let watch=created["record"]["watch"].clone();
    let before=bytes(&files)?;
    lock(&live.session)?.source.fence();
    let result=live.query(request(&live,2,true)?)?;
    assert_eq!(result["kind"],"historical_watch_replay");assert_eq!(result["status"],"satisfied");
    assert_eq!(result["first_terminal_record"],2);assert_eq!(result["records_verified"],4);
    assert_eq!(result["records_after_terminal"],2);assert_eq!(result["agent_turn"]["continuity"]["status"],"partial");
    assert_eq!(result["native_captures"],0);assert_eq!(result["watch_evaluated"],false);
    let work=result["agent_turn"]["active_work"]["obligations"].as_array().ok_or_else(||invalid("active work missing"))?;
    assert!(work.iter().any(|w|w["watch"]==watch&&w["evidence_digest"]==created["record"]["evidence_digest"]));
    assert_eq!(bytes(&files)?,before);assert_eq!(live.current()?,current);assert_eq!(live.calls.load(Ordering::SeqCst),0);
    let mut s=lock(&live.session)?;let c=s.context()?;let input=request_without_lock(&result)?;
    // Deliberately tiny final output cannot commit even a monitoring transition.
    let mut narrow=c;narrow.budget.max_bytes=1;
    assert!(execute(&mut s,&narrow,&input).is_err());drop(s);
    assert_eq!(bytes(&files)?,before);Ok(())
}
fn request_without_lock(result:&Value)->Result<Value>{
    let from=result.pointer("/range/from").ok_or_else(||invalid("range from"))?;
    let to=result.pointer("/range/to").ok_or_else(||invalid("range to"))?;
    Ok(json!({"schema":"dfmcp.query/1","query":{"kind":"historical_watch_replay",
        "from":{"record":from["record"],"record_digest":from["record_digest"]},
        "to":{"record":to["record"],"record_digest":to["record_digest"]},
        "definition":{"condition":{"op":"paused","value":true},"deadline_tick":result["deadline_tick"]}}}))
}

#[test]
fn archive_reopen_reproduces_identity_without_observe_or_loading_watch_journal() -> Result<()> {
    let files=Files::new()?;let live=live(&files)?;let input=request(&live,2,false)?;
    let expected=live.query(input.clone())?;
    let (limits,budget)={let s=lock(&live.session)?;(s.limits,s.budget)};
    drop(live);let before=bytes(&files)?;
    let s=archive::open(next_id()?,Slot::reserve()?,&files.observation,limits,budget,&[Capability::Query])?;
    let archive=Registered::install(s,Arc::new(AtomicUsize::new(0)))?;
    let result=archive.query(input.clone())?;
    assert_eq!(result["status"],"satisfied");assert_eq!(result["evidence_digest"],expected["evidence_digest"]);
    assert_eq!(result["replay_id"],expected["replay_id"]);assert_eq!(result["archive_only"],true);
    assert_eq!(result["bridge_connection_present"],false);
    assert_eq!(result["agent_turn"]["active_work"]["obligations"],json!([]));
    assert_eq!(bytes(&files)?,before);
    let schema=archive.schema()?;
    assert!(schema["query_schema"]["$defs"]["query"]["oneOf"].as_array().ok_or_else(||invalid("schema variants"))?
        .iter().any(|v|v["properties"]["kind"]["const"]=="historical_watch_replay"));
    let nested=archive.query(json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query",
        "record":input["query"]["from"]["record"],"record_digest":input["query"]["from"]["record_digest"],
        "query":input["query"]}}))?;
    assert_eq!(nested["ok"],false);assert_eq!(bytes(&files)?,before);Ok(())
}

#[test]
fn exact_references_current_grants_and_range_bounds_refuse_without_writes() -> Result<()> {
    let files=Files::new()?;let live=live(&files)?;let input=request(&live,2,false)?;let before=bytes(&files)?;
    for case in 0..4 {
        let mut bad=input.clone();
        match case {0=>bad["query"]["to"]["record_digest"]=json!("f".repeat(64)),
            1=>bad["query"]["to"]["record"]=json!(33),
            2=>bad["expected_anchor"]=json!({}),
            _=>bad["query"]["definition"]["unknown"]=json!(true)}
        assert_eq!(live.query(bad)?["ok"],false);assert_eq!(bytes(&files)?,before);
    }
    {
        let mut s=lock(&live.session)?;let current=s.anchor()?;
        for grant in &mut s.grants {grant.expires_at_tick=Some(GameTick(current.tick.0-1));}
    }
    assert_eq!(live.query(input)?["ok"],false);assert_eq!(bytes(&files)?,before);
    assert_eq!(live.calls.load(Ordering::SeqCst),0);Ok(())
}

#[test]
fn corruption_after_first_terminal_sample_still_refuses_the_complete_replay() -> Result<()> {
    let files=Files::new()?;let live=live(&files)?;let input=request(&live,1,false)?;let current=live.current()?;
    let offset={let s=lock(&live.session)?;let entries=s.journal.as_ref().ok_or_else(||invalid("journal"))?.entries();
        entries.last().ok_or_else(||invalid("last entry"))?.offset+60};
    let mut file=fs::OpenOptions::new().read(true).write(true).open(&files.observation).map_err(io_error)?;
    file.seek(SeekFrom::Start(offset)).map_err(io_error)?;let mut byte=[0];file.read_exact(&mut byte).map_err(io_error)?;
    byte[0]^=1;file.seek(SeekFrom::Start(offset)).map_err(io_error)?;file.write_all(&byte).map_err(io_error)?;
    let before=bytes(&files)?;let result=live.query(input)?;
    assert_eq!(result["ok"],false);assert_eq!(result["error"]["code"],"corrupt_ledger");
    assert_eq!(bytes(&files)?,before);assert_eq!(live.current()?,current);
    assert_eq!(live.calls.load(Ordering::SeqCst),0);Ok(())
}

#[test]
fn summary_fits_small_packet_or_refuses_whole_evidence_without_truncation() -> Result<()> {
    let files=Files::new()?;let live=live(&files)?;let input=request(&live,2,false)?;let before=bytes(&files)?;
    {let mut s=lock(&live.session)?;s.budget.max_bytes=8192;s.budget.max_output_tokens=2048;}
    let result=live.query(input)?;
    assert_eq!(result["status"],"satisfied");assert_eq!(result["truncated"],false);
    assert!(result.get("evaluation").is_none());assert!(result.to_string().len()<=8192);
    assert_eq!(bytes(&files)?,before);Ok(())
}

#[test]
fn schema_discovery_and_failure_guard_are_wired_through_the_live_handler() -> Result<()> {
    let files=Files::new()?;let live=live(&files)?;let discovery=live.schema()?;
    assert!(discovery["query_schema"]["$defs"]["query"]["oneOf"].as_array().ok_or_else(||invalid("schema variants"))?
        .iter().any(|v|v["properties"]["kind"]["const"]=="historical_watch_replay"));
    let mut input=request(&live,1,true)?;
    input["query"]["definition"]["failure_condition"]=json!({"op":"paused","value":true});
    let result=live.query(input)?;
    assert_eq!(result["status"],"failed");assert_eq!(result["first_terminal_record"],1);
    assert_eq!(result["records_after_terminal"],3);assert_eq!(live.calls.load(Ordering::SeqCst),0);Ok(())
}
