use super::*;
use super::super::invalid;
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
        // semantic_query.rs is instantiated in several runtimes. Each test
        // module needs a separate namespace, not just its own atomic counter.
        let namespace=Digest32::of_bytes(module_path!().as_bytes());
        let directory=std::env::temp_dir().canonicalize().map_err(io_error)?
            .join(format!("dfmcp-watch-durable-{}-{namespace}-{}",std::process::id(),next()));
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
    assert_eq!(listed["records"][0]["evaluation_current"],false);
    assert_eq!(listed["records"][0]["fresh_observation_required"],true);
    assert_eq!(listed["_condition_watch_work"][0]["next_step"]["arguments"]["query"]["query"]["kind"],"await_watch");
    assert!(run(&boot,new_id,json!({"kind":"poll_watch","watch":old})).is_err());
    let same=run(&boot,new_id,json!({"kind":"poll_watch","watch":new}))?;
    assert_eq!(same["record"]["sample_count"],1);
    assert_eq!(same["record"]["evaluation"]["reason"],"restart_gap_requires_fresh_observation");
    let next=run(&world(3,2,true),new_id,json!({"kind":"poll_watch","watch":new}))?;
    assert_eq!(next["record"]["status"],"candidate");assert_eq!(next["record"]["stable_observations"],1);
    assert_eq!(next["record"]["fresh_observation_required"],false);
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
    assert_eq!(restored["record"]["historical_outcome"],true);
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

#[test]
fn identical_bootstrap_does_not_relabel_old_terminal_evidence_as_fresh()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;run(&first,id,create(1))?;drop(guard);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    let result=run(&first,id,json!({"kind":"watches"}))?;
    assert_eq!(result["records"][0]["historical_outcome"],true);
    assert_eq!(result["records"][0]["evaluation_current"],false);
    assert_eq!(result["records"][0]["status"],"satisfied");drop(guard);Ok(())
}

#[test]
fn recovery_cannot_restore_an_old_larger_time_horizon()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let second=world(2,1,true);
    let (id,_,guard)=open(&files,&first,&[first.anchor()])?;run(&first,id,create(2))?;drop(guard);
    let before=files.bytes()?;let mut c=context(&second,SessionId::new(u128::from(next())));
    c.budget.max_game_ticks=10;
    assert!(matches!(attach(&second,&c,&files.path,archive(),&[first.anchor(),second.anchor()],json!({}),
        |v|Ok(v.to_string())),Err(e)if e.code==ErrorCode::BudgetExceeded));
    assert_eq!(files.bytes()?,before);Ok(())
}

#[test]
fn invalid_evaluation_shapes_and_unbounded_json_are_rejected()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    run(&first,id,create(2))?;
    let saved=journals()?.get(&id).and_then(|e|e.saved.clone()).ok_or_else(||invalid("missing saved fixture"))?;
    let anchors=BTreeMap::from([(first.anchor(),0)]);
    for evaluation in [Value::Null,json!(true),json!("not an object"),json!([]),json!({"reason":"bad\u{0}"})]{
        let mut bad=saved.clone();bad.watches[0].evaluation=evaluation;
        let bytes=serde_json::to_vec(&bad).map_err(|_|invalid("fixture serialization"))?;
        assert!(decode_saved(&bytes,&anchors).is_err());
    }
    let mut deep=json!({"leaf":0});for _ in 0..33{deep=json!({"child":deep});}
    assert!(bounded_json(deep.to_string().as_bytes()).is_err());
    let wide=json!({"array":vec![Value::Null;32769]});
    assert!(bounded_json(wide.to_string().as_bytes()).is_err());
    let mut bad=saved;bad.watches[0].definition.deadline_tick=1;
    assert!(decode_saved(&serde_json::to_vec(&bad).map_err(|_|invalid("fixture serialization"))?,&anchors).is_err());
    drop(guard);Ok(())
}

#[test]
fn watch_persistence_never_claims_unrelated_baselines_are_durable()->Result<()>{
    let files=Files::new()?;let first=world(1,0,true);let (id,_,guard)=open(&files,&first,&[first.anchor()])?;
    run(&first,id,create(2))?;let before=files.bytes()?;
    let result=super::super::with_active_work(&context(&first,id),json!({"kind":"capture","durable":false}),
        |v|Ok(v.to_string()))?;
    let result=parse(&result)?;assert_eq!(result["durable"],false);
    assert_eq!(result["watch_persistence"]["scope"],"foreground_condition_watches");
    assert_eq!(result["watch_persistence"]["durable"],true);
    assert_eq!(files.bytes()?,before);drop(guard);Ok(())
}

#[test]
fn checkpoint_bytes_match_independent_python_vectors()->Result<()>{
    use std::io::{self,Read,Write,Seek,SeekFrom,Cursor};
    #[derive(Clone,Default)]
    struct Bytes(std::rc::Rc<std::cell::RefCell<Cursor<Vec<u8>>>>);
    impl Read for Bytes{fn read(&mut self,b:&mut[u8])->io::Result<usize>{self.0.borrow_mut().read(b)}}
    impl Write for Bytes{
        fn write(&mut self,b:&[u8])->io::Result<usize>{self.0.borrow_mut().write(b)}
        fn flush(&mut self)->io::Result<()>{Ok(())}
    }
    impl Seek for Bytes{fn seek(&mut self,p:SeekFrom)->io::Result<u64>{self.0.borrow_mut().seek(p)}}
    impl dfmcp_adapter::operations_journal::JournalStorage for Bytes{
        fn sync(&mut self)->io::Result<()>{Ok(())}
        fn truncate(&mut self,_:u64)->io::Result<()>{Err(io::Error::other("no repair"))}
    }
    let bytes=Bytes::default();let mut c=context(&world(1,0,true),SessionId::new(7));
    c.request_id=RequestId::new(3);c.anchor.state_hash=Digest32::of_bytes(b"world");
    let mut journal=checkpoint::Journal::open(bytes.clone(),&c,Digest32::of_bytes(b"spatial/1.8 archive"),true,|_|Ok(()))?;
    assert_eq!(journal.id().to_string(),"d94a9a394f027513b75f4f042667fe0123aa13de1e66afa1dd3b5c8c3c09edaf");
    assert_eq!(journal.head().to_string(),"d3404626c30fe42c90c400a4cb31cd574e77077f29415421c73fa11ed651f518");
    let first=journal.stage(b"first".to_vec(),&c)?;journal.commit(first,&c)?;
    assert_eq!(journal.head().to_string(),"06a925e21a650ae34c37cb00a6e3c2e414d468cf2476de849c14c2131aae4f80");
    let second=journal.stage(b"second".to_vec(),&c)?;journal.commit(second,&c)?;
    assert_eq!(journal.head().to_string(),"6e557d1732e84b9fbb9eb3628bf0a1650edd0af766597a7415cb16b2502a4063");
    assert_eq!(journal.count(),2);assert_eq!(journal.retained_bytes(),363);
    assert_eq!(Digest32::of_bytes(bytes.0.borrow().get_ref()).to_string(),
        "8166fa4dc5bc52ea47bf0919976e9f40b99f425ae0967d7acf48c806edd2c327");
    Ok(())
}
