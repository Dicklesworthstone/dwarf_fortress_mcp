//! Real session/normalizer/watch paths with a single injected reconnect factory.
//! No environment mutation, sleeps, live DFHack, or production admission.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::GameTick;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError { error(ErrorCode::CorruptLedger,"recovery fixture I/O") }
struct Files { directory: PathBuf, observations: PathBuf, watches: PathBuf }
impl Files {
    fn new() -> Result<Self> {
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-source-recovery-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self { observations:directory.join("observations.bin"),watches:directory.join("watches.bin"),directory })
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _=fs::remove_file(&self.observations);let _=fs::remove_file(&self.watches);
        let _=fs::remove_dir(&self.directory);
    }
}
struct Owned(Session);
impl Deref for Owned {type Target=Session;fn deref(&self)->&Session {&self.0}}
impl DerefMut for Owned {fn deref_mut(&mut self)->&mut Session {&mut self.0}}
impl Drop for Owned {
    fn drop(&mut self) {
        let _=semantic_query::release_session_resources(self.0.id,true,|v|Ok(v.to_string()));
        self.0._watch_journal.take();
    }
}
#[derive(Clone,Default)]
struct Calls {connections:Arc<AtomicUsize>,reads:Arc<AtomicUsize>,allowances:Arc<Mutex<Vec<Duration>>>}
struct Script {value:Option<LiveSpatialCitizenObservation>,calls:Calls,fenced:bool,archive:bool}
impl Source for Script {
    fn read(&mut self,allowance:Duration)->Result<LiveSpatialCitizenObservation> {
        self.calls.reads.fetch_add(1,Ordering::SeqCst);lock(&self.calls.allowances)?.push(allowance);
        self.value.take().ok_or_else(||error(ErrorCode::AdapterUnavailable,"injected read loss"))
    }
    fn poisoned(&self)->bool {self.fenced}
    fn fence(&mut self) {self.fenced=true;}
    fn pages(&self)->u32 {7}
    fn archive_only(&self)->bool {self.archive}
}
fn session(files:Option<&Files>,durable:bool)->Result<Owned> {
    let first=fixture::observation(3,2,2,false)?;let region=first.spatial().terrain().map.region;
    let mut state=LiveSpatialCitizenState::default();state.publish(first)?;
    let fortress=state.snapshot().ok_or_else(||error(ErrorCode::InvalidRequest,"fixture snapshot"))?.fortress_id;
    let limits=CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),region},citizens:4096};
    let mut s=Owned(Session {id:next_id()?,source:Box::new(Script {value:None,calls:Calls::default(),fenced:true,archive:false}),
        state,limits,journal:None,budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,
            max_output_tokens:65536,max_wall_millis:60000,..WorkBudget::default()},
        grants:[Capability::Observe,Capability::Query,Capability::Doctor].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope {fortress_id:Some(fortress),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),
        request:0,_watch_journal:None,_slot:Slot::reserve()?});
    if let Some(files)=files {
        let c=s.context()?;history::attach(&mut s,&files.observations,TailRecovery::Refuse,&c)?;
        if durable {let c=s.context()?;durable_watches::finish_open(&mut s,&c,Some(&files.watches),json!({"ok":true}))?;}
    }
    Ok(s)
}
fn call(s:&mut Session,input:Value)->Result<Value> {
    let c=s.context()?;let snapshot=s.state.snapshot().ok_or_else(||error(ErrorCode::InvalidRequest,"snapshot"))?;
    let encoded=semantic_query::execute_with_publisher(snapshot,&c,&input,|v|Ok(v.to_string()))?;
    decode(&encoded)
}
fn decode(value:&str)->Result<Value> {
    serde_json::from_str(value).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))
}
fn register(s:&mut Session)->Result<Value> {
    let deadline=s.anchor()?.tick.0+100;
    call(s,json!({"schema":"dfmcp.query/1","query":{"kind":"watch","key":"survive-outage",
        "condition":{"op":"tick_at_least","value":0},"deadline_tick":deadline,
        "poll_interval_ticks":1,"stable_observations":2}}))
}
fn poll(s:&mut Session,handle:&Value)->Result<Value> {
    call(s,json!({"schema":"dfmcp.query/1","query":{"kind":"poll_watch","watch":handle}}))
}
fn run(s:&mut Session,tick:Option<u32>,calls:&Calls,times:&[u64])->Result<Value> {
    let c=s.context()?;let calls=calls.clone();let mut times:VecDeque<_>=times.iter().copied().collect();
    let output=recover(s,&c,move |allowance| {
        calls.connections.fetch_add(1,Ordering::SeqCst);lock(&calls.allowances)?.push(allowance);
        let Some(tick)=tick else {return Err(error(ErrorCode::AdapterUnavailable,"injected reconnect failure"));};
        Ok(Box::new(Script {value:Some(fixture::observation(tick,2,2,false)?),calls,fenced:false,archive:false}) as Box<dyn Source>)
    },||Duration::from_millis(times.pop_front().unwrap_or(0)))?;
    decode(&output)
}
fn recovery_request(s:&Session)->Result<Value> {
    Ok(json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(s.anchor()?),"query":{"kind":KIND}}))
}

#[test]
fn one_reconnect_preserves_handles_and_resets_progress_before_a_fresh_sample()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;let created=register(&mut s)?;
    let old=s.anchor()?;let id=s.id;let handle=&created["record"]["watch"];let calls=Calls::default();
    let out=run(&mut s,Some(4),&calls,&[0,0,0,0])?;
    assert_eq!(out["ok"],true);assert!(!s.source.poisoned());assert_eq!(s.id,id);
    assert_eq!(calls.connections.load(Ordering::SeqCst),1);assert_eq!(calls.reads.load(Ordering::SeqCst),1);
    assert_eq!(out["source_recovery"]["capture_outcome"],"advanced");
    assert_eq!(s.anchor()?.cursor.sequence,old.cursor.sequence+1);
    assert_eq!(out["agent_turn"]["continuity"]["status"],"partial");
    assert_eq!(out["source_gap"]["records"][0]["watch"],*handle);
    assert_eq!(out["source_gap"]["changed_watches"],1);assert_eq!(out["samples_added"],0);
    assert_eq!(out["agent_turn"]["active_work"]["obligations"][0]["stable_observations"],0);
    let next=poll(&mut s,handle)?;
    assert_eq!(next["record"]["status"],"candidate");assert_eq!(next["record"]["stable_observations"],1);
    assert_eq!(next["record"]["definition"],created["record"]["definition"]);
    assert_eq!(next["record"]["sample_count"],2);Ok(())
}

#[test]
fn failed_and_unchanged_recovery_never_manufacture_success_samples()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;let created=register(&mut s)?;
    let handle=&created["record"]["watch"];let old=s.anchor()?;let calls=Calls::default();
    let failure=run(&mut s,None,&calls,&[0,0,0])?;
    assert_eq!(failure["ok"],false);assert_eq!(s.anchor()?,old);assert!(s.source.poisoned());
    assert_eq!(calls.reads.load(Ordering::SeqCst),0);assert_eq!(failure["source_recovery"]["transfer_pages"],0);
    let digest=failure["source_gap"]["records"][0]["evidence_digest"].clone();
    let again=run(&mut s,None,&calls,&[0,0,0])?;
    assert_eq!(again["source_gap"]["changed_watches"],0);
    assert_eq!(again["source_gap"]["records"][0]["evidence_digest"],digest);
    let recovered=run(&mut s,Some(3),&calls,&[0,0,0,0])?;
    assert_eq!(recovered["source_recovery"]["capture_outcome"],"heartbeat");
    assert_eq!(s.anchor()?,old);
    let same=poll(&mut s,handle)?;
    assert_eq!(same["record"]["status"],"blocked_unknown");assert_eq!(same["record"]["sample_count"],1);Ok(())
}

#[test]
fn recovery_revalidates_both_read_grants_before_memory_or_durable_publication()->Result<()> {
    let _serial=lock(&SERIAL)?;
    for durable in [false,true] {
        for capability in [Capability::Observe,Capability::Query] {
            let files=Files::new()?;let mut s=session(durable.then_some(&files),durable)?;register(&mut s)?;
            let old=s.anchor()?;let before=if durable {Some(fs::read(&files.observations).map_err(io_error)?)}else{None};
            for grant in &mut s.grants {if grant.capability==capability {grant.expires_at_tick=Some(GameTick(old.tick.0+1));}}
            let calls=Calls::default();let result=run(&mut s,Some(5),&calls,&[0,0,0,0])?;
            assert_eq!(result["ok"],false);assert_eq!(result["source_recovery"]["error"]["code"],ErrorCode::CapabilityDenied.as_str());
            assert_eq!(s.anchor()?,old);assert!(s.source.poisoned());assert_eq!(calls.reads.load(Ordering::SeqCst),1);
            if let Some(before)=before {assert_eq!(fs::read(&files.observations).map_err(io_error)?,before);}
        }
    }
    Ok(())
}

#[test]
fn malformed_or_unprivileged_requests_are_refused_before_gap_or_network()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;let created=register(&mut s)?;let c=s.context()?;
    let valid=recovery_request(&s)?;request(&s,&c,&valid)?;
    for field in ["endpoint","token","protocol","path","repair"] {
        let mut input=valid.clone();input["query"][field]=json!("arbitrary");assert!(request(&s,&c,&input).is_err());
    }
    for millis in [json!(0),json!(60001),json!(-1),json!("5"),json!(true),Value::Null] {
        let mut input=valid.clone();input["query"]["max_wall_millis"]=millis;assert!(request(&s,&c,&input).is_err());
    }
    let mut missing=valid.clone();missing.as_object_mut().ok_or_else(||error(ErrorCode::InvalidRequest,"object"))?.remove("expected_anchor");
    assert!(request(&s,&c,&missing).is_err());
    let mut stale=valid;stale["expected_anchor"]["sequence"]=json!(999);assert!(request(&s,&c,&stale).is_err());
    for case in 0..3 {
        let mut c=c.clone();match case {0=>c.grants.retain(|g|g.capability!=Capability::Observe),
            1=>c.grants.retain(|g|g.capability!=Capability::Query),_=>c.cancellation_requested=true}
        let mut connections=0;
        assert!(recover(&mut s,&c,|_| {connections+=1;Err(error(ErrorCode::AdapterUnavailable,"unexpected network"))},||Duration::ZERO).is_err());
        assert_eq!(connections,0);
    }
    let same=poll(&mut s,&created["record"]["watch"])?;
    assert_eq!(same["record"]["evidence_digest"],created["record"]["evidence_digest"]);Ok(())
}

#[test]
fn full_packet_reservation_refuses_without_interrupting_or_connecting()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;let created=register(&mut s)?;
    let mut c=s.context()?;let before=created["record"]["evidence_digest"].clone();
    // Force a refusal below even the fixed recovery renderer's minimum packet.
    c.budget.max_bytes=1024;let mut connections=0;
    assert!(matches!(recover(&mut s,&c,|_| {connections+=1;Err(error(ErrorCode::AdapterUnavailable,"unexpected"))},||Duration::ZERO),
        Err(e) if e.code==ErrorCode::BudgetExceeded));
    assert_eq!(connections,0);
    assert_eq!(poll(&mut s,&created["record"]["watch"])?["record"]["evidence_digest"],before);Ok(())
}

#[test]
fn connection_and_capture_share_the_remaining_budget_and_expiry_stops_capture()->Result<()> {
    let _serial=lock(&SERIAL)?;
    let mut s=session(None,false)?;let calls=Calls::default();
    let out=run(&mut s,Some(4),&calls,&[1000,2000,3000,4000])?;
    assert_eq!(out["ok"],true);let allowances=lock(&calls.allowances)?;
    assert_eq!(allowances[0],Duration::from_millis(57000));
    assert!(allowances[1]<=Duration::from_millis(56000));drop(allowances);drop(s);
    let mut s=session(None,false)?;let old=s.anchor()?;let calls=Calls::default();
    let out=run(&mut s,Some(4),&calls,&[0,0,0,60000])?;
    assert_eq!(out["ok"],false);assert_eq!(calls.connections.load(Ordering::SeqCst),1);
    assert_eq!(calls.reads.load(Ordering::SeqCst),0);assert_eq!(s.anchor()?,old);assert!(s.source.poisoned());Ok(())
}

#[test]
fn archive_and_healthy_sources_cannot_use_recovery_to_bypass_normal_lifecycle()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;
    for (fenced,archive) in [(false,false),(true,true)] {
        s.source=Box::new(Script {value:None,calls:Calls::default(),fenced,archive});let c=s.context()?;let mut calls=0;
        assert!(recover(&mut s,&c,|_| {calls+=1;Err(error(ErrorCode::AdapterUnavailable,"unexpected"))},||Duration::ZERO).is_err());
        assert_eq!(calls,0);
    }
    Ok(())
}

#[test]
fn journal_corruption_prevents_reconnect_without_repair_or_watch_changes()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let mut s=session(Some(&files),true)?;register(&mut s)?;
    let watches=fs::read(&files.watches).map_err(io_error)?;
    fs::OpenOptions::new().append(true).open(&files.observations)
        .and_then(|mut f|f.write_all(b"x")).map_err(io_error)?;
    let corrupt=fs::read(&files.observations).map_err(io_error)?;let old=s.anchor()?;let calls=Calls::default();
    assert!(matches!(run(&mut s,Some(4),&calls,&[0,0,0]),Err(e)if e.code==ErrorCode::CorruptLedger));
    assert_eq!(calls.connections.load(Ordering::SeqCst),0);assert_eq!(s.anchor()?,old);
    assert_eq!(fs::read(&files.observations).map_err(io_error)?,corrupt);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?,watches);Ok(())
}

#[test]
fn paired_journals_record_gap_before_reconnect_and_do_not_sample_on_success()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;let mut s=session(Some(&files),true)?;let created=register(&mut s)?;
    let previous=fs::read(&files.watches).map_err(io_error)?;let c=s.context()?;
    let out=recover(&mut s,&c,|_| {
        let gap=fs::read(&files.watches).map_err(io_error)?;
        assert!(gap.len()>previous.len());assert!(gap.windows(b"source_gap_requires_fresh_observation".len())
            .any(|w|w==b"source_gap_requires_fresh_observation"));
        Ok(Box::new(Script {value:Some(fixture::observation(4,2,2,false)?),calls:Calls::default(),fenced:false,archive:false}) as Box<dyn Source>)
    },||Duration::ZERO)?;
    let out=decode(&out)?;assert_eq!(out["ok"],true);
    assert_eq!(out["watch_persistence"]["checkpoint_changed"],false);
    assert_eq!(out["source_gap"]["records"][0]["watch"],created["record"]["watch"]);
    assert_eq!(s.journal.as_ref().map(|j|j.entries().len()),Some(2));
    assert_eq!(out["agent_turn"]["active_work"]["obligations"][0]["status"],"blocked_unknown");
    Ok(())
}

#[test]
fn reconnect_epoch_reset_stays_explicit_and_invalidates_the_old_watch_on_poll()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;let created=register(&mut s)?;let old=s.anchor()?;
    let out=run(&mut s,Some(2),&Calls::default(),&[0,0,0,0])?;
    assert_eq!(out["source_recovery"]["capture_outcome"],"reset");assert!(s.anchor()?.cursor.epoch>old.cursor.epoch);
    assert_eq!(out["agent_turn"]["continuity"]["status"],"partial");
    assert_eq!(poll(&mut s,&created["record"]["watch"])?["record"]["status"],"invalidated");Ok(())
}

#[test]
fn diagnostic_is_utf8_bounded_and_final_metadata_uses_the_reserved_shape()->Result<()> {
    let _serial=lock(&SERIAL)?;let mut s=session(None,false)?;register(&mut s)?;let c=s.context()?;
    let text="é".repeat(1000);let out=recover(&mut s,&c,|_|Err(error(ErrorCode::AdapterUnavailable,&text)),||Duration::ZERO)?;
    let out=decode(&out)?;let message=out["source_recovery"]["error"]["message"].as_str()
        .ok_or_else(||error(ErrorCode::InvalidRequest,"message"))?;
    assert_eq!(message.len(),256);assert_eq!(out["source_recovery"]["error"]["message_truncated"],true);
    assert_eq!(out["mutation_dispatched"],false);assert_eq!(out["samples_added"],0);
    assert_eq!(out["agent_turn"]["anchor"],anchor_json(s.anchor()?));
    assert!(out.to_string().len()<=c.budget.max_bytes as usize);Ok(())
}

#[test]
fn failed_reconnect_gap_is_recovered_from_the_same_paired_archive_after_restart()->Result<()> {
    let _serial=lock(&SERIAL)?;let files=Files::new()?;
    let mut s=session(Some(&files),true)?;let created=register(&mut s)?;
    let failed=run(&mut s,None,&Calls::default(),&[0,0,0])?;
    let gap_digest=failed["source_gap"]["records"][0]["evidence_digest"].clone();
    let old_handle=created["record"]["watch"].clone();drop(s);
    let mut recovered=session(Some(&files),true)?;
    let listing=call(&mut recovered,json!({"schema":"dfmcp.query/1","query":{"kind":"watches"}}))?;
    let handle=listing["records"][0]["watch"].clone();assert_ne!(handle,old_handle);
    assert_eq!(listing["records"][0]["status"],"blocked_unknown");
    assert_eq!(listing["records"][0]["recovery"]["prior_evidence_digest"],gap_digest);
    let record=poll(&mut recovered,&handle)?;
    assert_eq!(record["record"]["sample_count"],1);assert_eq!(record["record"]["stable_observations"],0);
    assert_eq!(record["record"]["definition"],created["record"]["definition"]);
    Ok(())
}
