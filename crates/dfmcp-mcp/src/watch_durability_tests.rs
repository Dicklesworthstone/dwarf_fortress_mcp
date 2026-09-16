use super::*;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicU64,Ordering};
use dfmcp_core::{CapabilityGrant,CapabilityScope,EntityId,RequestId,WorkBudget};
use dfmcp_world::{EntityKind,EntityRecord,Fact,FactSource,Value as WorldValue,WorldGraph};

static NEXT:AtomicU64=AtomicU64::new(100_000);
fn next()->u64{NEXT.fetch_add(1,Ordering::Relaxed)}
fn io_error(_:std::io::Error)->dfmcp_core::DfmcpError{corrupt("durable watch test filesystem error")}
struct Files{directory:std::path::PathBuf,path:std::path::PathBuf}
impl Files{
    fn new()->Result<Self>{
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?
            .join(format!("dfmcp-watch-durable-{}-{}",std::process::id(),next()));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self{path:directory.join("watches.bin"),directory})
    }
    fn bytes(&self)->Result<Vec<u8>>{fs::read(&self.path).map_err(io_error)}
}
impl Drop for Files{
    fn drop(&mut self){let _=fs::remove_file(&self.path);let _=fs::remove_dir(&self.directory);}
}
fn world(tick:u64,sequence:u64,healthy:bool)->WorldSnapshot{
    let mut graph=WorldGraph::default();
    graph.entities.insert(EntityId::new(1),EntityRecord{id:EntityId::new(1),generation:1,revision:sequence+1,
        kind:EntityKind::Unit,label:"Urist".into(),fields:BTreeMap::from([("sane".into(),
            Fact::known(WorldValue::Bool(healthy),GameTick(tick),FactSource::DfhackField("unit.sane".into()),
                Digest32::of_bytes(b"observed source")))])});
    WorldSnapshot::new(FortressId::new(1),GameTick(tick),ObservationCursor{epoch:0,sequence},true,graph)
}
fn context(snapshot:&WorldSnapshot,session:SessionId)->OperationContext{
    OperationContext{session_id:session,request_id:RequestId::new(1),anchor:snapshot.anchor(),
        budget:WorkBudget{max_game_ticks:1000,max_entities:10,max_bytes:262144,
            max_output_tokens:65536,max_wall_millis:60000,..WorkBudget::default()},cancellation_requested:false,
        grants:[Capability::Query,Capability::Observe].into_iter().map(|capability|CapabilityGrant{
            capability,scope:Default::default(),max_risk:RiskTier::ReadOnly,
            expires_at_tick:None,remaining_uses:None}).collect()}
}
fn archive()->Digest32{Digest32::of_bytes(b"exact retained spatial18 archive")}
fn parse(bytes:&str)->Result<Value>{serde_json::from_str(bytes).map_err(|_|invalid("test JSON"))}
fn open(files:&Files,snapshot:&WorldSnapshot,history:&[StateAnchor])->Result<(SessionId,Value,WatchJournalGuard)>{
    let session=SessionId::new(u128::from(next()));let c=context(snapshot,session);
    let (out,guard)=attach(snapshot,&c,&files.path,archive(),history,json!({"ok":true}),|v|Ok(v.to_string()))?;
    Ok((session,parse(&out)?,guard))
}
fn run(snapshot:&WorldSnapshot,session:SessionId,request:Value)->Result<Value>{
    let output=super::super::execute(snapshot,&context(snapshot,session),
        &json!({"schema":"dfmcp.query/1","query":request}),|v|Ok(v.to_string()))?;
    parse(&output)
}
fn create(stability:u32)->Value{json!({"kind":"watch","key":"healthy","condition":{
    "op":"field","entity_id":"1","generation":1,"field":"sane","comparison":"eq",
    "value":{"type":"bool","value":true}},"deadline_tick":100,"stable_observations":stability})}
fn handle(result:&Value)->Result<String>{result["record"]["watch"].as_str().map(str::to_owned).ok_or_else(||invalid("test watch handle"))}
fn listed_handle(result:&Value)->Result<String>{result["records"][0]["watch"].as_str().map(str::to_owned).ok_or_else(||invalid("listed handle"))}

#[test]
fn restart_restores_intent_but_requires_new_stability_evidence()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let boot=world(2,1,true);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    let created=run(&first,id,create(2))?;let old=handle(&created)?;
    assert_eq!(created["record"]["stable_observations"],1);assert_eq!(created["durable"],true);
    drop(guard);
    let (new_id,opened,guard)=open(&files,&boot,&[first.anchor(),boot.anchor()])?;
    assert_eq!(opened["watch_recovery"]["restored"],1);
    let listed=run(&boot,new_id,json!({"kind":"watches"}))?;let new=listed_handle(&listed)?;
    assert_ne!(new,old);assert_eq!(listed["records"][0]["status"],"blocked_unknown");
    assert_eq!(listed["records"][0]["stable_observations"],0);
    assert!(run(&boot,new_id,json!({"kind":"poll_watch","watch":old})).is_err());
    let same=run(&boot,new_id,json!({"kind":"poll_watch","watch":new}))?;
    assert_eq!(same["record"]["sample_count"],1);
    assert_eq!(same["record"]["evaluation"]["reason"],"restart_gap_requires_fresh_observation");
    let next=run(&world(3,2,true),new_id,json!({"kind":"poll_watch","watch":new}))?;
    assert_eq!(next["record"]["status"],"candidate");assert_eq!(next["record"]["stable_observations"],1);
    let done=run(&world(4,3,true),new_id,json!({"kind":"poll_watch","watch":new}))?;
    assert_eq!(done["record"]["status"],"satisfied");assert_eq!(done["record"]["sample_count"],3);
    drop(guard);Ok(())
}

#[test]
fn terminal_outcome_remains_historical_after_restart()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let boot=world(2,1,false);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    let old=run(&first,id,create(1))?;drop(guard);
    let (id,_,guard)=open(&files,&boot,&[first.anchor(),boot.anchor()])?;
    let list=run(&boot,id,json!({"kind":"watches"}))?;let watch=listed_handle(&list)?;
    let restored=run(&boot,id,json!({"kind":"poll_watch","watch":watch}))?;
    assert_eq!(restored["record"]["status"],"satisfied");assert_eq!(restored["record"]["evaluation_current"],false);
    assert_eq!(restored["record"]["sample_count"],1);
    assert_eq!(restored["record"]["recovery"]["prior_evidence_digest"],old["record"]["evidence_digest"]);
    assert_eq!(restored["_condition_watch_work"],json!([]));drop(guard);Ok(())
}

#[test]
fn cancellation_and_release_survive_restart_without_resurrection()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let second=world(2,1,true);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    let watch=handle(&run(&first,id,create(2))?)?;
    run(&first,id,json!({"kind":"cancel_watch","watch":watch}))?;drop(guard);
    let (id,_,guard)=open(&files,&second,&[first.anchor(),second.anchor()])?;
    let list=run(&second,id,json!({"kind":"watches"}))?;
    assert_eq!(list["records"][0]["status"],"cancelled");let watch=listed_handle(&list)?;
    run(&second,id,json!({"kind":"release_watch","watch":watch}))?;drop(guard);
    let (id,opened,guard)=open(&files,&second,&[first.anchor(),second.anchor()])?;
    assert_eq!(opened["watch_recovery"]["restored"],0);
    assert_eq!(run(&second,id,json!({"kind":"watches"}))?["records"],json!([]));drop(guard);Ok(())
}

#[test]
fn rendering_failure_and_identical_reads_never_append_or_publish()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    let before=files.bytes()?;
    let failed=super::super::execute(&first,&context(&first,id),&json!({"schema":"dfmcp.query/1","query":create(2)}),
        |_|Err(bounded("injected full-packet overflow")));
    assert!(failed.is_err());assert_eq!(files.bytes()?,before);
    assert_eq!(run(&first,id,json!({"kind":"watches"}))?["records"],json!([]));
    let created=run(&first,id,create(2))?;let watch=handle(&created)?;let saved=files.bytes()?;
    for _ in 0..4{
        run(&first,id,create(2))?;run(&first,id,json!({"kind":"watches"}))?;
        run(&first,id,json!({"kind":"poll_watch","watch":watch}))?;
    }
    assert_eq!(files.bytes()?,saved);drop(guard);Ok(())
}

#[test]
fn different_archive_or_missing_observation_proof_refuses_unchanged_file()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let second=world(2,1,true);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;run(&first,id,create(2))?;drop(guard);
    let before=files.bytes()?;let c=context(&second,SessionId::new(u128::from(next())));
    assert!(attach(&second,&c,&files.path,Digest32::of_bytes(b"other archive"),&[first.anchor(),second.anchor()],
        json!({}),|v|Ok(v.to_string())).is_err());
    assert!(attach(&second,&c,&files.path,archive(),&[second.anchor()],json!({}),|v|Ok(v.to_string())).is_err());
    assert_eq!(files.bytes()?,before);Ok(())
}

#[test]
fn epoch_reset_invalidates_and_downtime_cannot_extend_deadlines()->Result<()>{
    for epoch_reset in [true,false]{
        let files=Files::new()?;let first=world(1,0,true);
        let (id,_,guard)=open(&files,&first,&[first.anchor()])?;run(&first,id,create(2))?;drop(guard);
        let mut boot=world(101,1,true);
        if epoch_reset{boot.cursor=ObservationCursor{epoch:1,sequence:0};boot.tick=GameTick(0);boot.refresh_hash();}
        let (id,_,guard)=open(&files,&boot,&[first.anchor(),boot.anchor()])?;
        let list=run(&boot,id,json!({"kind":"watches"}))?;
        assert_eq!(list["records"][0]["status"],if epoch_reset{"invalidated"}else{"expired"});
        assert_eq!(list["records"][0]["stable_observations"],0);drop(guard);
    }
    Ok(())
}

#[test]
fn current_authority_and_custody_failures_do_not_acknowledge_watches()->Result<()>{
    use std::io::Write;
    let files=Files::new()?;let first=world(1,0,true);let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    let before=files.bytes()?;let mut c=context(&first,id);
    c.grants=vec![CapabilityGrant{capability:Capability::Query,scope:CapabilityScope::default(),
        max_risk:RiskTier::ReadOnly,expires_at_tick:Some(GameTick(0)),remaining_uses:None}];
    assert!(super::super::execute(&first,&c,&json!({"schema":"dfmcp.query/1","query":create(2)}),|v|Ok(v.to_string())).is_err());
    assert_eq!(files.bytes()?,before);
    fs::OpenOptions::new().append(true).open(&files.path).and_then(|mut f|f.write_all(b"x")).map_err(io_error)?;
    assert!(run(&first,id,json!({"kind":"watches"})).is_err());
    assert!(run(&first,id,create(2)).is_err());drop(guard);
    assert!(open(&files,&first,&[first.anchor()]).is_err());Ok(())
}

#[test]
fn failed_recovery_render_leaves_no_session_or_persistent_rebind()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let second=world(2,1,true);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;run(&first,id,create(2))?;drop(guard);
    let before=files.bytes()?;let c=context(&second,SessionId::new(u128::from(next())));
    assert!(attach(&second,&c,&files.path,archive(),&[first.anchor(),second.anchor()],json!({}),
        |_|Err(bounded("injected open response overflow"))).is_err());
    assert_eq!(files.bytes()?,before);
    assert!(!journals()?.contains_key(&c.session_id));
    assert!(!super::super::lock(&WATCHES)?.entries.keys().any(|(id,_)|*id==c.session_id));
    let (_,opened,guard)=open(&files,&second,&[first.anchor(),second.anchor()])?;
    assert_eq!(opened["watch_recovery"]["restored"],1);drop(guard);Ok(())
}
