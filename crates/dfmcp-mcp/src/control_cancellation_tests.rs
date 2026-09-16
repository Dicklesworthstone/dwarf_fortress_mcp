//! Actual handlers over private files. The writable test session deliberately
//! has no connection: cancellation and its explanations must not contact DFHack.
use super::*;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use dfmcp_adapter::pause_reconciliation::{PauseReconciliationSource, reconcile_batch};

static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_:std::io::Error)->DfmcpError {err(ErrorCode::CorruptLedger,"cancellation fixture I/O")}
fn context()->OperationContext {
    OperationContext {session_id:SessionId::new(710),request_id:RequestId::new(1),anchor:coordinator_anchor(),
        budget:WorkBudget {max_bytes:65536,max_output_tokens:16384,max_wall_millis:60000,..WorkBudget::default()},
        cancellation_requested:false,grants:[Capability::Query,Capability::ControlClock].into_iter()
            .map(|capability|CapabilityGrant {capability,scope:CapabilityScope::default(),max_risk:RiskTier::Reversible,
                expires_at_tick:None,remaining_uses:None}).collect()}
}
fn plan(key:&str)->String {Digest32::of_bytes(key.as_bytes()).to_string()}
struct Fixture {directory:PathBuf,path:PathBuf}
impl Fixture {
    fn new()->Result<Self> {
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-pause-cancel-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        let f=Self {path:directory.join("effects.bin"),directory};
        let mut j=open_private_control_journal(&f.path,&context(),7,EffectTailRecovery::Refuse)?;
        for key in ["a-prepared","b-prepared","c-started","d-applied"] {
            let d=Digest32::of_bytes(key.as_bytes());
            j.record_prepared(key.into(),d,true,10,7,[1;16],&context())?;
            if key=="c-started"||key=="d-applied" {j.begin_commit(key,d,7,&context())?;}
            if key=="d-applied" {j.record_reconciliation(key,d,7,true,true,true,11,Some(d),&context())?;}
        }
        Ok(f)
    }
}
impl Drop for Fixture {fn drop(&mut self) {
    let _=fs::remove_file(&self.path);let _=fs::remove_dir(&self.directory);
}}
struct Registered {id:SessionId}
impl Registered {fn handle(&self)->Option<String>{Some(self.id.to_string())}}
impl Drop for Registered {fn drop(&mut self) {
    if let Ok(mut sessions)=SESSIONS.lock(){sessions.remove(&self.id);}
}}
fn register(f:&Fixture,read_only:bool)->Result<Registered> {
    let id=next_id()?;let c=context();
    let journal=if read_only {open_private_control_recovery(&f.path,&c)?}
        else {open_private_control_journal(&f.path,&c,7,EffectTailRecovery::Refuse)?};
    let grants=if read_only {c.grants.into_iter().filter(|g|g.capability==Capability::Query).collect()} else {c.grants};
    let s=ControlSession {id,connection:None,journal,request:0,budget:c.budget,grants,_slot:Slot::reserve()?};
    lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(s)));Ok(Registered{id})
}
fn decode(raw:String)->Result<Value>{serde_json::from_str(&raw).map_err(|_|err(ErrorCode::InvalidRequest,"test JSON"))}
fn request(s:&Registered,key:&str)->Result<Value>{
    decode(fortress_cancel(s.handle(),Some(key.into()),Some(plan(key)),None,None))
}
fn success(value:Value)->Value {assert_eq!(value["result"]["ok"],true,"{value}");value}
fn list(s:&Registered,state:&str)->Result<Value>{
    decode(fortress_query(s.handle(),Some(state.into()),Some(128),None,None,None))
}

#[test]
fn cancellation_is_queryable_explainable_and_never_commit_compatible() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,false)?;
    let first=success(request(&s,"a-prepared")?);
    assert_eq!(first["result"]["effect"]["state"],"cancelled_before_dispatch");
    assert_eq!(first["result"]["effect"]["effect_known"],false);
    assert_eq!(first["result"]["global_effect_absence_proven"],false);
    assert_eq!(first["result"]["native_cancellation_performed"],false);
    let bytes=fs::read(&f.path).map_err(io_error)?;
    let replay=success(request(&s,"a-prepared")?);assert_eq!(replay["result"]["replayed"],true);
    assert_eq!(request(&s,"a-prepared")?,replay);
    let listed=success(list(&s,"cancelled_before_dispatch")?);
    assert_eq!(listed["result"]["matched_effects"],1);
    assert_eq!(listed["result"]["state_counts"]["cancelled_before_dispatch"],1);
    assert_eq!(success(list(&s,"nonterminal")?)["result"]["matched_effects"],2);
    let explained=success(decode(fortress_explain(s.handle(),"a-prepared".into(),plan("a-prepared")))?);
    assert_eq!(explained["result"]["cancelled"],true);assert_eq!(explained["result"]["commit_permitted"],false);
    let replanned=success(decode(fortress_plan(s.handle(),"a-prepared".into(),plan("a-prepared"),true,10))?);
    assert_eq!(replanned["result"]["effect"]["state"],"cancelled_before_dispatch");
    let committed=decode(fortress_commit(s.handle(),"a-prepared".into(),plan("a-prepared"),"01".repeat(16)))?;
    assert_eq!(committed["result"]["error"]["code"],"conflict");
    assert_eq!(fs::read(&f.path).map_err(io_error)?,bytes);
    assert!(lock(&resolve(s.handle())?)?.connection.is_none());
    Ok(())
}

#[test]
fn cancelled_keys_remain_retired_after_writable_and_offline_recovery() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,false)?;
    success(request(&s,"a-prepared")?);drop(s);
    let bytes=fs::read(&f.path).map_err(io_error)?;
    for read_only in [true,false] {
        let s=register(&f,read_only)?;
        let found=success(list(&s,"cancelled_before_dispatch")?);
        assert_eq!(found["result"]["effects"][0]["idempotency_key"],"a-prepared");
        let explained=success(decode(fortress_explain(s.handle(),"a-prepared".into(),plan("a-prepared")))?);
        assert_eq!(explained["result"]["cancelled"],true);
        assert_eq!(explained["result"]["reconciliation_performed"],false);
        if read_only {assert_eq!(request(&s,"b-prepared")?["result"]["error"]["code"],"capability_denied");}
        else {success(request(&s,"a-prepared")?);}
    }
    assert_eq!(fs::read(&f.path).map_err(io_error)?,bytes);Ok(())
}

#[test]
fn cancellation_never_erases_dispatched_or_verified_effects() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,false)?;
    let before=fs::read(&f.path).map_err(io_error)?;
    let started=request(&s,"c-started")?;
    assert_eq!(started["result"]["error"]["code"],"effect_indeterminate");
    assert_eq!(started["result"]["error"]["mutation_dispatched"],false);
    assert_eq!(started["result"]["error"]["reconciliation_required"],true);
    assert_eq!(request(&s,"d-applied")?["result"]["error"]["code"],"conflict");
    assert_eq!(success(list(&s,"reconciliation_required")?)["result"]["matched_effects"],1);
    assert_eq!(fs::read(&f.path).map_err(io_error)?,before);Ok(())
}

#[test]
fn cancellation_budget_authority_and_identity_refusals_leave_the_file_unchanged() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,false)?;
    let before=fs::read(&f.path).map_err(io_error)?;
    for bytes in [0,1,32,128,65537] {
        let v=decode(fortress_cancel(s.handle(),Some("a-prepared".into()),Some(plan("a-prepared")),Some(bytes),None))?;
        assert_eq!(v["result"]["error"]["code"],"budget_exceeded");
    }
    assert_eq!(decode(fortress_cancel(s.handle(),Some("a-prepared".into()),Some(plan("other")),None,None))?
        ["result"]["error"]["code"],"conflict");
    assert_eq!(decode(fortress_cancel(s.handle(),None,None,None,None))?["result"]["error"]["code"],"capability_denied");
    assert_eq!(decode(fortress_cancel(s.handle(),Some("a-prepared".into()),None,None,None))?["result"]["error"]["code"],"invalid_request");
    let h=resolve(s.handle())?;let mut guard=lock(&h)?;guard.grants.clear();drop(guard);
    assert_eq!(request(&s,"a-prepared")?["result"]["error"]["code"],"capability_denied");
    assert_eq!(fs::read(&f.path).map_err(io_error)?,before);Ok(())
}

#[test]
fn injected_clock_authority_cannot_write_an_offline_recovery_journal() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,true)?;
    let before=fs::read(&f.path).map_err(io_error)?;
    {let h=resolve(s.handle())?;lock(&h)?.grants=context().grants;}
    assert_eq!(request(&s,"a-prepared")?["result"]["error"]["code"],"capability_denied");
    assert_eq!(fs::read(&f.path).map_err(io_error)?,before);Ok(())
}

#[test]
fn new_cancellation_invalidates_history_pages_without_rewinding_later_heads() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,false)?;
    let before=success(decode(fortress_query(s.handle(),None,Some(1),None,None,None))?);
    let token=before["result"]["continuation"].as_str().ok_or_else(||err(ErrorCode::InvalidRequest,"token"))?.to_owned();
    success(request(&s,"a-prepared")?);let later=success(request(&s,"b-prepared")?);
    let replay=success(request(&s,"a-prepared")?);
    assert_eq!(replay["result"]["durable_effect_journal"],later["result"]["durable_effect_journal"]);
    assert_ne!(replay["result"]["effect"]["record_digest"],replay["result"]["durable_effect_journal"]["head"]);
    assert_eq!(decode(fortress_query(s.handle(),None,Some(1),Some(token),None,None))?["result"]["error"]["code"],"conflict");
    Ok(())
}

#[test]
fn cancellation_and_commit_start_races_have_only_one_durable_winner() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;
    for _ in 0..8 {
        let f=Fixture::new()?;let s=register(&f,false)?;
        let barrier=Arc::new(std::sync::Barrier::new(2));let start=barrier.clone();let id=s.handle();
        let cancel_thread=std::thread::spawn(move||{
            start.wait();fortress_cancel(id,Some("a-prepared".into()),Some(plan("a-prepared")),None,None)
        });
        barrier.wait();
        let dispatched={let h=resolve(s.handle())?;let mut session=lock(&h)?;let c=session.context()?;
            session.journal.begin_commit("a-prepared",Digest32::of_bytes(b"a-prepared"),7,&c)};
        let cancelled=decode(cancel_thread.join().map_err(|_|err(ErrorCode::InternalInvariantViolation,"cancel thread panicked"))?)?;
        let h=resolve(s.handle())?;let guard=lock(&h)?;
        let state=guard.journal.lookup("a-prepared").ok_or_else(||err(ErrorCode::InvalidRequest,"missing key"))?.state;
        match state {
            DurablePauseState::CancelledBeforeDispatch=>{assert!(matches!(dispatched,Err(e)if e.code==ErrorCode::Conflict));success(cancelled);}
            DurablePauseState::CommitStarted=>{assert!(dispatched.is_ok());assert_eq!(cancelled["result"]["error"]["code"],"effect_indeterminate");}
            _=>return Err(err(ErrorCode::InternalInvariantViolation,"race produced a third outcome")),
        }
    }
    Ok(())
}

struct NoQueries(usize);
impl PauseReconciliationSource for NoQueries {
    fn query_effect(&mut self,_:&DurablePauseRecord,_:Duration,_:&OperationContext)->Result<PauseEffect> {
        self.0+=1;Err(err(ErrorCode::AdapterUnavailable,"must not query cancelled work"))
    }
}
#[test]
fn recovery_batches_skip_cancelled_effects_without_native_reads() -> Result<()> {
    let _serial=lock(&SESSION_TESTS)?;let f=Fixture::new()?;let s=register(&f,false)?;
    success(request(&s,"a-prepared")?);
    let h=resolve(s.handle())?;let mut session=lock(&h)?;let c=session.context()?;
    let head=session.journal.head();let mut source=NoQueries(0);
    let batch=reconcile_batch(&mut session.journal,&mut source,&["a-prepared".into()],&c)?;
    assert_eq!(source.0,0);assert_eq!(batch.head_after,head);
    assert!(batch.items[0].record.state.terminal());assert!(!batch.items[0].queried);
    assert!(!batch.items[0].record.effect_known);assert!(batch.stopped.is_none());Ok(())
}
