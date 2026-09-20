use super::*;
use std::io::{self, Cursor, Read, Write, Seek, SeekFrom};
use std::cell::RefCell;
use std::rc::Rc;
use dfmcp_adapter::work_order_progress::{ProgressObservation,ProgressManifest};
use dfmcp_adapter::work_order_progress::archive::{ArchiveMode,ArchivedProgress};
use dfmcp_core::{Capability,CapabilityGrant,CapabilityScope,GameTick,ObservationCursor,
    RequestId,RiskTier,StateAnchor,WorkBudget};
use super::super::{packet_value,BASE_RESERVE,ROW_RESERVE,WATCH_BYTES};

#[derive(Clone,Default)]
struct Memory {bytes:Rc<RefCell<Vec<u8>>>,position:u64}
impl Read for Memory {fn read(&mut self,out:&mut[u8])->io::Result<usize>{let bytes=self.bytes.borrow();
    let mut c=Cursor::new(bytes.as_slice());c.set_position(self.position);let n=c.read(out)?;self.position=c.position();Ok(n)}}
impl Write for Memory {fn write(&mut self,data:&[u8])->io::Result<usize>{let mut bytes=self.bytes.borrow_mut();
    let mut c=Cursor::new(&mut *bytes);c.set_position(self.position);let n=c.write(data)?;self.position=c.position();Ok(n)}
    fn flush(&mut self)->io::Result<()>{Ok(())}}
impl Seek for Memory {fn seek(&mut self,from:SeekFrom)->io::Result<u64>{let bytes=self.bytes.borrow();let mut c=Cursor::new(bytes.as_slice());
    c.set_position(self.position);self.position=c.seek(from)?;Ok(self.position)}}
impl JournalStorage for Memory {fn sync(&mut self)->io::Result<()>{Ok(())}
    fn truncate(&mut self,_:u64)->io::Result<()>{Err(io::Error::other("no repair"))}}
fn hex(raw:&str)->Result<Vec<u8>>{raw.trim().as_bytes().chunks_exact(2).map(|p|{
    u8::from_str_radix(std::str::from_utf8(p).map_err(|_|error(ErrorCode::InvalidRequest,"hex"))?,16)
        .map_err(|_|error(ErrorCode::InvalidRequest,"hex"))}).collect()}
fn observation(seq:u64,tick:u64,flags:u32)->Result<ProgressObservation>{
    let mut b=hex(include_str!("../../dfmcp-adapter/tests/fixtures/work_order_progress_v1_12.hex"))?;
    b[16..24].copy_from_slice(&seq.to_be_bytes());b[24..32].copy_from_slice(&tick.to_be_bytes());
    b[68..72].copy_from_slice(&3i32.to_be_bytes());b[76..80].copy_from_slice(&flags.to_be_bytes());
    ProgressObservation::decode(&b,&[3,8])
}
fn context()->Result<OperationContext>{Ok(OperationContext{session_id:SessionId::new(1),request_id:RequestId::new(1),
    anchor:StateAnchor{fortress_id:observation(1,10,0)?.fortress_id(),cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO},
    budget:WorkBudget{max_entities:4096,max_bytes:WATCH_BYTES,max_output_tokens:65_536,max_game_ticks:120_000,..WorkBudget::default()},
    grants:[Capability::Query,Capability::Observe].into_iter().map(|capability|CapabilityGrant{capability,
        scope:CapabilityScope::default(),max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),cancellation_requested:false})}
fn append(a:&mut ProgressArchive<Memory>,seq:u64,tick:u64,flags:u32)->Result<ArchivedProgress>{let c=context()?;
    let e=a.append(&ProgressManifest{generation:7,df_version:"df".into(),dfhack_version:"dfhack".into()},&observation(seq,tick,flags)?,&c)?;
    a.record(e.number,e.record_digest,&c)}
fn setup()->Result<(ProgressArchive<Memory>,WatchBook<Memory>,Memory,ArchivedProgress)>{let c=context()?;
    let mut a=ProgressArchive::open(Memory::default(),ArchiveMode::Live,true,&c)?;
    let first=append(&mut a,1,10,0)?;let storage=Memory::default();
    let b=WatchBook::open(storage.clone(),ArchiveMode::Live,true,&mut a,&c)?;Ok((a,b,storage,first))}
fn register(a:&mut ProgressArchive<Memory>,key:&str,origin:&ArchivedProgress)->Result<String>{
    Ok(json!({"mode":"watch_register","archive_id":a.summary(&context()?)?.archive_id.to_string(),
        "key":key,"native_order_id":3,"goal":"validated","deadline_game_tick":30,
        "cadence_game_ticks":1,"stable_samples":2,"origin_number":origin.entry.number,
        "origin_digest":origin.entry.record_digest.to_string()}).to_string())
}
#[test]
fn closed_request_grammar_refuses_duplicate_keys_mixed_thresholds_and_unbounded_values()->Result<()>{
    let (mut a,_,_,origin)=setup()?;let raw=register(&mut a,"test",&origin)?;
    assert!(Request::parse(&raw).is_ok());assert!(Request::parse("{\"mode\":\"watch_list\"}").is_ok());
    for raw in ["{}","[]","{\"mode\":\"watch_list\",\"x\":1}","{\"mode\":\"watch_list\",\"mode\":\"watch_list\"}"]{
        assert!(Request::parse(raw).is_err(),"{raw}");}
    let value:Value=serde_json::from_str(&raw).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))?;
    for (field,bad) in [("threshold",json!(1)),("key",json!("../bad")),("key",json!("x".repeat(65))),
        ("native_order_id",json!(u32::MAX)),("stable_samples",json!(0)),("stable_samples",json!(17)),
        ("cadence_game_ticks",json!(0)),("origin_number",json!(0)),("origin_number",json!(4097)),
        ("origin_digest",json!("A".repeat(64))),("unknown",json!(true)),("goal",json!("completed_goods"))]{
        let mut bad_value=value.clone();bad_value[field]=bad;assert!(Request::parse(&bad_value.to_string()).is_err(),"{field}");}
    assert!(Request::parse(&" ".repeat(2049)).is_err());Ok(())
}
#[test]
fn dispatcher_register_discover_and_status_prove_only_sampled_predicates()->Result<()>{
    let (mut a,mut b,storage,origin)=setup()?;let c=context()?;let request=register(&mut a,"test",&origin)?;
    let answer=query(&mut b,&mut a,Request::parse(&request)?,&c)?;
    assert_eq!(answer["native_calls"],0);let before=storage.bytes.borrow().clone();
    query(&mut b,&mut a,Request::parse(&request)?,&c)?;assert_eq!(*storage.bytes.borrow(),before);
    let first=query(&mut b,&mut a,Request::List,&c)?;assert_eq!(first["watch_result"]["pending"],1);
    append(&mut a,2,11,1)?;append(&mut a,3,12,1)?;
    let result=query(&mut b,&mut a,Request::List,&c)?;let watched=&result["watch_result"]["watches"][0];
    assert_eq!(watched["state"],"satisfied_observation");assert_eq!(watched["positive_samples"].as_array().map(Vec::len),Some(2));
    assert_eq!(watched["production_completion_proven"],false);assert_eq!(result["game_mutation_dispatched"],false);
    let d=b.summary(&mut a,&c)?.definitions[0].1;let id=a.summary(&c)?.archive_id;
    let status=query(&mut b,&mut a,Request::Status{archive:id,key:"test".into(),digest:d},&c)?;
    assert_eq!(status["watch_result"]["watch"],*watched);Ok(())
}
#[test]
fn local_cancel_and_offline_inspection_never_reach_a_native_source()->Result<()>{
    let (mut a,mut b,storage,origin)=setup()?;let c=context()?;let request=register(&mut a,"key",&origin)?;
    query(&mut b,&mut a,Request::parse(&request)?,&c)?;
    let d=b.summary(&mut a,&c)?.definitions[0].1;let summary=a.summary(&c)?;
    let answer=query(&mut b,&mut a,Request::Cancel{archive:summary.archive_id,key:"key".into(),digest:d,head:summary.head},&c)?;
    assert_eq!(answer["watch_result"]["state"],"cancelled");assert_eq!(answer["watch_result"]["orders_cancelled"],false);
    let before=storage.bytes.borrow().clone();let mut offline=WatchBook::open(storage.clone(),ArchiveMode::Offline,false,&mut a,&c)?;
    assert!(query(&mut offline,&mut a,Request::parse(&request)?,&c).is_err());
    let result=query(&mut offline,&mut a,Request::List,&c)?;
    assert_eq!(result["watch_result"]["watches"][0]["state"],"cancelled");assert_eq!(*storage.bytes.borrow(),before);Ok(())
}
#[test]
fn all_thirty_two_maximal_stability_proofs_and_index_fit_reserved_response()->Result<()>{
    let (mut a,mut b,_,origin)=setup()?;let c=context()?;
    for i in 0..32{let key=format!("{i:02}{}","x".repeat(62));
        let spec=WatchSpec::new(&key,3,WatchGoal::Validated,40,1,16)?;b.register(spec,WatchRecordRef::of(&origin),&mut a,&c)?;}
    for seq in 2..=17{append(&mut a,seq,seq+9,1)?;}
    let result=query(&mut b,&mut a,Request::List,&c)?;
    assert_eq!(result["watch_result"]["watches"].as_array().map(Vec::len),Some(32));
    let summary=b.summary(&mut a,&c)?;let archive=a.summary(&c)?;
    let mut packet=packet_value("fortress.query",result,Some(&c),None,None,true,Some(&archive));
    attach(&mut packet,Some(&summary),c.session_id);
    assert!(packet.to_string().len() as u64<=BASE_RESERVE+32*ROW_RESERVE);
    assert_eq!(packet["agent_turn"]["active_work"]["progress_watches"]["definition_count"],32);Ok(())
}
#[test]
fn authority_budget_and_wrong_archive_fail_without_changing_watch_bytes()->Result<()>{
    let (mut a,mut b,storage,origin)=setup()?;let c=context()?;let request=register(&mut a,"key",&origin)?;
    let before=storage.bytes.borrow().clone();let mut denied=c.clone();denied.grants.retain(|g|g.capability==Capability::Query);
    assert!(query(&mut b,&mut a,Request::parse(&request)?,&denied).is_err());
    let mut wrong:Value=serde_json::from_str(&request).map_err(|_|error(ErrorCode::InvalidRequest,"test JSON"))?;
    wrong["archive_id"]=json!("0".repeat(64));assert!(query(&mut b,&mut a,Request::parse(&wrong.to_string())?,&c).is_err());
    let mut tiny=c;tiny.budget.max_output_tokens=1;assert!(super::super::reserve(tiny,32).is_err());
    assert_eq!(*storage.bytes.borrow(),before);Ok(())
}
#[test]
fn omitted_or_unhealthy_books_do_not_establish_absence_of_pending_work()->Result<()>{
    let c=context()?;let mut packet=packet_value("fortress.query",json!({"ok":true}),Some(&c),None,None,true,None);
    attach(&mut packet,None,c.session_id);assert_eq!(packet["agent_turn"]["active_work"]["progress_watches"]["pending_absence_proven"],false);
    unavailable(&mut packet,true);assert_eq!(packet["agent_turn"]["active_work"]["progress_watches"]["status"],"unavailable");
    assert_eq!(packet["agent_turn"]["active_work"]["progress_watches"]["pending_absence_proven"],false);Ok(())
}
#[test]
fn operator_watch_path_requires_a_distinct_archive_and_output_reserve_remains_bounded(){
    use std::path::Path;
    assert!(super::super::validate_watch_path(None,Path::new("/private/watch")).is_err());
    assert!(super::super::validate_watch_path(Some(Path::new("/private/watch")),Path::new("/private/watch")).is_err());
    assert!(super::super::validate_watch_path(Some(Path::new("/private/archive")),Path::new("/private/watch")).is_ok());
    assert!(WATCH_BYTES > dfmcp_adapter::work_order_progress::watches::BOOK_OPEN_RESERVE
        + dfmcp_adapter::work_order_progress::archive::MAX_ARCHIVE_BYTES + 2*1024*1024);
}
#[cfg(unix)]
#[test]
fn offline_runtime_publishes_watch_discovery_and_suppresses_lost_book_custody()->std::result::Result<(),Box<dyn std::error::Error>>{
    use std::fs;use std::os::unix::fs::{DirBuilderExt,PermissionsExt};
    use super::super::{offline_session,render_checked,Projection};
    use dfmcp_adapter::work_order_progress::archive::open_progress_archive;
    use dfmcp_adapter::work_order_progress::watches::open_watch_book;
    let root=std::env::temp_dir().canonicalize()?.join(format!("dfmcp-watch-mcp-{}-{}",std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos()));
    fs::DirBuilder::new().mode(0o700).create(&root)?;struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup{fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}let _cleanup=Cleanup(root.clone());
    let archive_path=root.join("archive");let book_path=root.join("watches");
    fs::write(&archive_path,hex(include_str!("../../dfmcp-adapter/tests/fixtures/progress_watch_archive_v1.hex"))?)?;
    fs::write(&book_path,hex(include_str!("../../dfmcp-adapter/tests/fixtures/progress_watch_book_v1.hex"))?)?;
    for p in [&archive_path,&book_path]{fs::set_permissions(p,fs::Permissions::from_mode(0o600))?;}
    let mut c=context()?;c.grants.retain(|g|g.capability==Capability::Query);
    let mut a=open_progress_archive(&archive_path,ArchiveMode::Offline,&c)?;
    let b=open_watch_book(&book_path,ArchiveMode::Offline,&mut a,&c)?;
    let mut session=offline_session(a,&c,c.budget)?;session.watch_book=Some(b);assert!(session.reader.is_none());
    let out=render_checked("fortress.query",Projection::plain(json!({"ok":true}),true),&mut session,&c)?;
    let value:Value=serde_json::from_str(&out)?;assert_eq!(value["agent_turn"]["active_work"]["progress_watches"]["definition_count"],1);
    fs::rename(&book_path,root.join("retained"))?;
    assert!(render_checked("fortress.query",Projection::plain(json!({"ok":true}),true),&mut session,&c).is_err());
    let out=super::super::render("fortress.query",Projection::plain(json!({"ok":true}),true),&mut session,&c);
    let value:Value=serde_json::from_str(&out)?;assert_eq!(value["agent_turn"]["active_work"]["progress_watches"]["status"],"unavailable");Ok(())
}
