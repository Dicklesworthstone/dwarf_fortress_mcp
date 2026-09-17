//! Real projection and private-journal acceptance, with deterministic elapsed
//! time and an injected capture source. No sleeps, DFHack or ambient environment.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::GameTick;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError { error(ErrorCode::CorruptLedger, "acceptance fixture I/O") }
struct Files { directory: PathBuf, journal: PathBuf }
impl Files {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-observation-acceptance-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self { journal: directory.join("observations.bin"), directory })
    }
}
impl Drop for Files {
    fn drop(&mut self) { let _=fs::remove_file(&self.journal); let _=fs::remove_dir(&self.directory); }
}
#[derive(Clone, Default)]
struct Calls { count: Arc<AtomicUsize>, allowances: Arc<Mutex<Vec<Duration>>> }
struct Script { values: VecDeque<LiveSpatialCitizenObservation>, calls: Calls, fenced: bool, corrupt: Option<PathBuf> }
impl Source for Script {
    fn read(&mut self, allowance: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.count.fetch_add(1,Ordering::SeqCst);
        lock(&self.calls.allowances)?.push(allowance);
        if let Some(path)=self.corrupt.take() {
            fs::OpenOptions::new().append(true).open(path)
                .and_then(|mut file|file.write_all(b"x")).map_err(io_error)?;
        }
        self.values.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"injected capture failure"))
    }
    fn poisoned(&self)->bool {self.fenced}
    fn fence(&mut self) {self.fenced=true;}
    fn pages(&self)->u32 {1}
}
fn session(files: Option<&Files>, ticks: &[u32]) -> Result<(Session, Calls)> {
    let first=fixture::observation(3,2,2,false)?;
    let region=first.spatial().terrain().map.region;
    let mut state=LiveSpatialCitizenState::default();state.publish(first)?;
    let anchor=state.snapshot().ok_or_else(||error(ErrorCode::InvalidRequest,"fixture snapshot"))?.anchor();
    let limits=CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),region},citizens:4096};
    let calls=Calls::default();
    let source=Script {values:ticks.iter().map(|tick|fixture::observation(*tick,2,2,false))
        .collect::<Result<VecDeque<_>>>()?,calls:calls.clone(),fenced:false,corrupt:None};
    let mut session=Session {id:next_id()?,source:Box::new(source),state,limits,journal:None,
        budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:65536,
            max_wall_millis:60000,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe,Capability::Doctor].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),
        request:0,_watch_journal:None,_slot:Slot::reserve()?};
    if let Some(files)=files {
        let c=session.context()?;history::attach(&mut session,&files.journal,TailRecovery::Refuse,&c)?;
    }
    Ok((session,calls))
}
fn clocked(s: &mut Session, c: &OperationContext, times: &[u64]) -> Result<JobPublication> {
    let mut times=times.iter().copied();
    refresh_with_clock(s,c,||Duration::from_millis(times.next().unwrap_or(60000)))
}
fn forbid_at_next_tick(s: &mut Session, capability: Capability) -> Result<()> {
    let expiry=GameTick(s.anchor()?.tick.0+1);
    for grant in &mut s.grants {if grant.capability==capability {grant.expires_at_tick=Some(expiry);}}
    Ok(())
}

#[test]
fn observe_expiry_preserves_both_journaled_and_memory_roots() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;
    for durable in [false,true] {
        let (mut s,calls)=session(durable.then_some(&files),&[5])?;
        forbid_at_next_tick(&mut s,Capability::Observe)?;
        let old=s.state.snapshot().cloned();let source=s.state.source_digest()?;
        let bytes=if durable {Some(fs::read(&files.journal).map_err(io_error)?)}else{None};
        let c=s.context()?;
        assert!(matches!(clocked(&mut s,&c,&[0,0,0]),Err(e)if e.code==ErrorCode::CapabilityDenied));
        assert_eq!(s.state.snapshot(),old.as_ref());assert_eq!(s.state.source_digest()?,source);
        assert!(s.source.poisoned());assert_eq!(calls.count.load(Ordering::SeqCst),1);
        if let Some(bytes)=bytes {assert_eq!(fs::read(&files.journal).map_err(io_error)?,bytes);}
    }
    Ok(())
}

#[test]
fn durable_capture_requires_query_at_the_target_but_memory_observe_does_not() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;
    let (mut s,_)=session(Some(&files),&[5])?;forbid_at_next_tick(&mut s,Capability::Query)?;
    let old=s.anchor()?;let before=fs::read(&files.journal).map_err(io_error)?;let c=s.context()?;
    assert!(matches!(clocked(&mut s,&c,&[0,0,0]),Err(e)if e.code==ErrorCode::CapabilityDenied));
    assert_eq!(s.anchor()?,old);assert_eq!(fs::read(&files.journal).map_err(io_error)?,before);drop(s);
    let (mut s,calls)=session(None,&[5])?;s.grants.retain(|g|g.capability==Capability::Observe);
    let old=s.anchor()?;let c=s.context()?;clocked(&mut s,&c,&[0,0,0])?;
    assert!(s.anchor()?.tick>old.tick);assert!(!s.source.poisoned());assert_eq!(calls.count.load(Ordering::SeqCst),1);
    Ok(())
}

#[test]
fn deadline_refusals_before_and_after_capture_never_publish() -> Result<()> {
    let _serial=lock(&SERIAL)?;
    for durable in [false,true] {
        for (times,read) in [(vec![10],false),(vec![1,10],true),(vec![1,2,10],true)] {
            let files=Files::new()?;let (mut s,calls)=session(durable.then_some(&files),&[4])?;
            let old=s.state.snapshot().cloned();let before=if durable {Some(fs::read(&files.journal).map_err(io_error)?)}else{None};
            let mut c=s.context()?;c.budget.max_wall_millis=10;
            assert!(matches!(clocked(&mut s,&c,&times),Err(e)if e.code==ErrorCode::BudgetExceeded));
            assert_eq!(s.state.snapshot(),old.as_ref());assert_eq!(s.source.poisoned(),read);
            assert_eq!(calls.count.load(Ordering::SeqCst),usize::from(read));
            if let Some(before)=before {assert_eq!(fs::read(&files.journal).map_err(io_error)?,before);}
        }
    }
    Ok(())
}

#[test]
fn source_receives_only_the_remaining_time_and_success_is_published_once() -> Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let (mut s,calls)=session(Some(&files),&[4,4])?;
    let c=s.context()?;let old=c.anchor;
    clocked(&mut s,&c,&[1000,3000,6000])?;
    assert_eq!(lock(&calls.allowances)?.as_slice(),&[Duration::from_secs(59)]);
    assert_eq!(s.anchor()?.cursor.sequence,old.cursor.sequence+1);
    let bytes=fs::read(&files.journal).map_err(io_error)?;let current=s.anchor()?;let c=s.context()?;
    assert_eq!(clocked(&mut s,&c,&[0,0,0])?,JobPublication::Heartbeat);
    assert_eq!(s.anchor()?,current);assert_eq!(fs::read(&files.journal).map_err(io_error)?,bytes);
    assert_eq!(s.journal.as_ref().map(|j|j.entries().len()),Some(2));Ok(())
}

#[test]
fn stale_cancelled_or_expired_callers_never_enter_the_source() -> Result<()> {
    let _serial=lock(&SERIAL)?;
    for case in 0..5 {
        let (mut s,calls)=session(None,&[2])?;let old=s.anchor()?;let mut c=s.context()?;
        match case {
            0=>c.session_id=SessionId::new(123),
            1=>c.anchor.cursor.sequence+=1,
            2=>c.cancellation_requested=true,
            3=>c.grants.clear(),
            _=>for grant in &mut c.grants {grant.expires_at_tick=Some(GameTick(old.tick.0-1));},
        }
        assert!(clocked(&mut s,&c,&[0,0,0]).is_err());assert_eq!(s.anchor()?,old);
        assert_eq!(calls.count.load(Ordering::SeqCst),0);assert!(!s.source.poisoned());
    }
    Ok(())
}

#[test]
fn narrowed_entity_and_acquisition_bounds_preserve_generation_history() -> Result<()> {
    let _serial=lock(&SERIAL)?;
    for count_bound in [false,true] {
        let (mut s,calls)=session(None,&[])?;let old=s.state.snapshot().cloned();let source=s.state.source_digest()?;
        s.source=Box::new(Script {values:VecDeque::from([fixture::observation(4,3,2,false)?]),
            calls:calls.clone(),fenced:false,corrupt:None});
        let mut c=s.context()?;
        if count_bound {s.limits.citizens=2;}else{c.budget.max_entities=old.as_ref().map_or(0,|s|s.graph.entities.len() as u32);}
        assert!(matches!(clocked(&mut s,&c,&[0,0,0]),Err(e)if e.code==ErrorCode::BudgetExceeded));
        assert_eq!(s.state.snapshot(),old.as_ref());assert_eq!(s.state.source_digest()?,source);
        assert!(s.source.poisoned());assert_eq!(calls.count.load(Ordering::SeqCst),1);
    }
    Ok(())
}

#[test]
fn changed_journal_custody_refuses_before_capture_or_after_source_without_append() -> Result<()> {
    let _serial=lock(&SERIAL)?;
    for during in [false,true] {
        let files=Files::new()?;let (mut s,calls)=session(Some(&files),&[4])?;let old=s.anchor()?;
        let before=fs::read(&files.journal).map_err(io_error)?;
        if during {
            s.source=Box::new(Script {values:VecDeque::from([fixture::observation(4,2,2,false)?]),
                calls:calls.clone(),fenced:false,corrupt:Some(files.journal.clone())});
        } else {fs::OpenOptions::new().append(true).open(&files.journal)
            .and_then(|mut file|file.write_all(b"x")).map_err(io_error)?;}
        let c=s.context()?;
        assert!(matches!(clocked(&mut s,&c,&[0,0,0]),Err(e)if e.code==ErrorCode::CorruptLedger));
        assert_eq!(s.anchor()?,old);assert!(s.source.poisoned());
        assert_eq!(calls.count.load(Ordering::SeqCst),usize::from(during));
        assert_eq!(fs::read(&files.journal).map_err(io_error)?,[before,b"x".to_vec()].concat());
    }
    Ok(())
}

#[test]
fn source_failure_and_epoch_reset_have_distinct_acceptance_outcomes() -> Result<()> {
    let _serial=lock(&SERIAL)?;let (mut s,calls)=session(None,&[])?;let old=s.anchor()?;let c=s.context()?;
    assert!(matches!(clocked(&mut s,&c,&[0]),Err(e)if e.code==ErrorCode::AdapterUnavailable));
    assert_eq!(s.anchor()?,old);assert!(s.source.poisoned());assert_eq!(calls.count.load(Ordering::SeqCst),1);drop(s);
    let (mut s,_)=session(None,&[2])?;let old=s.anchor()?;let c=s.context()?;
    assert_eq!(clocked(&mut s,&c,&[0,0,0])?,JobPublication::Reset);
    assert!(s.anchor()?.cursor.epoch>old.cursor.epoch);assert!(s.anchor()?.tick<old.tick);
    Ok(())
}
