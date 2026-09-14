use super::*;
use std::io::Cursor;
use dfmcp_core::{CapabilityGrant,CapabilityScope,MapCoord,RequestId,SessionId,WorkBudget};
use crate::live_jobs::{LiveJob,LiveJobObservation};

#[derive(Default)]
struct Memory { bytes:Cursor<Vec<u8>>, fail_after:Option<usize>, sync_fails:bool, syncs:usize }
impl Read for Memory {fn read(&mut self,out:&mut [u8])->io::Result<usize>{self.bytes.read(out)}}
impl Seek for Memory {fn seek(&mut self,from:SeekFrom)->io::Result<u64>{self.bytes.seek(from)}}
impl Write for Memory {
    fn write(&mut self,bytes:&[u8])->io::Result<usize>{
        let count=match &mut self.fail_after {
            Some(0)=>return Err(io::Error::other("injected write failure")),
            Some(left)=>{let n=(*left).min(bytes.len());*left-=n;n},None=>bytes.len(),
        };
        self.bytes.write(&bytes[..count])
    }
    fn flush(&mut self)->io::Result<()>{Ok(())}
}
impl JournalStorage for Memory {
    fn sync(&mut self)->io::Result<()>{self.syncs+=1;if self.sync_fails{Err(io::Error::other("injected sync failure"))}else{Ok(())}}
    fn truncate(&mut self,len:u64)->io::Result<()>{self.bytes.get_mut().truncate(len as usize);Ok(())}
}
impl<T:JournalStorage> JournalStorage for &mut T {
    fn sync(&mut self)->io::Result<()>{(**self).sync()}
    fn truncate(&mut self,len:u64)->io::Result<()>{(**self).truncate(len)}
    fn validate_identity(&self)->io::Result<()>{(**self).validate_identity()}
}
fn source(tick:u32)->LiveOperationsObservation {
    LiveOperationsObservation {jobs:LiveJobObservation {bridge_generation:7,df_version:"df".to_owned(),dfhack_version:"dfhack".to_owned(),
        year:105,year_tick:tick,paused:true,site_id:1,world_folder:"region1".to_owned(),next_job_id:1,
        jobs:vec![LiveJob {native_id:0,job_type:5,type_key:"Dig".to_owned(),reaction:String::new(),suspended:tick%2==0,
            repeating:false,position:MapCoord::new(1,2,3),worker_native_id:None,holder_native_id:None,
            completion_timer:-1,attached_item_count:0,required_item_filter_count:1}]},
        next_building_id:0,next_item_id:0,buildings:Vec::new(),items:Vec::new(),attachments:Vec::new()}
}
fn context()->Result<OperationContext> {
    let mut state=LiveOperationsState::default();state.publish(source(1))?;
    let anchor=state.snapshot().ok_or_else(||corrupt("fixture state"))?.anchor();
    Ok(OperationContext {session_id:SessionId::new(17),request_id:RequestId::new(1),anchor,
        budget:WorkBudget{max_wall_millis:60000,max_entities:64,max_bytes:MAX_OPERATIONS_BYTES as u64,max_output_tokens:8192,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe].into_iter().map(|capability|CapabilityGrant {capability,
            scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},max_risk:RiskTier::ReadOnly,
            expires_at_tick:None,remaining_uses:None}).collect(),cancellation_requested:false})
}
fn open(bytes:Vec<u8>,context:&OperationContext,recovery:TailRecovery)->Result<OperationsJournal<Memory>> {
    let new=bytes.is_empty();OperationsJournal::open(Memory{bytes:Cursor::new(bytes),..Memory::default()},context,JournalLimits::default(),new,recovery)
}
fn filled()->Result<(OperationsJournal<Memory>,OperationContext)> {
    let context=context()?;let mut j=open(Vec::new(),&context,TailRecovery::Refuse)?;
    j.append(source(1),&context)?;j.append(source(2),&context)?;Ok((j,context))
}

#[test]
fn synced_reopen_replays_exact_anchors_and_ignores_heartbeats()->Result<()> {
    let (mut j,c)=filled()?;let before=j.entries.clone();let root=j.state.snapshot().cloned();
    assert_eq!(j.append(source(2),&c)?,JobPublication::Heartbeat);assert_eq!(j.entries,before);
    let mut recovered=open(j.storage.bytes.into_inner(),&c,TailRecovery::Refuse)?;
    assert_eq!(recovered.entries,before);assert_eq!(recovered.state.snapshot(),root.as_ref());
    for entry in before {assert_eq!(recovered.snapshot_at(entry.number,entry.record_digest,&c)?.anchor(),entry.anchor);}
    Ok(())
}

#[test]
fn every_torn_record_prefix_recovers_only_with_explicit_repair()->Result<()> {
    let (j,c)=filled()?;let bytes=j.storage.bytes.into_inner();let second=&j.entries[1];
    for end in second.offset as usize+1..bytes.len() {
        assert!(open(bytes[..end].to_vec(),&c,TailRecovery::Refuse).is_err());
        let recovered=open(bytes[..end].to_vec(),&c,TailRecovery::TruncateIncomplete)?;
        assert_eq!(recovered.entries.len(),1,"prefix {end}");
        assert_eq!(recovered.length,second.offset);assert_eq!(recovered.repaired_tail_bytes,end as u64-second.offset);
        assert_eq!(recovered.storage.bytes.get_ref(),&bytes[..second.offset as usize]);
    }
    Ok(())
}

#[test]
fn partial_headers_and_complete_corrupt_frames_are_not_tail_repairs()->Result<()> {
    let (j,c)=filled()?;let bytes=j.storage.bytes.into_inner();
    for end in 1..HEADER_BYTES {assert!(open(bytes[..end].to_vec(),&c,TailRecovery::TruncateIncomplete).is_err());}
    for i in 0..bytes.len() {
        let mut corrupt_bytes=bytes.clone();corrupt_bytes[i]^=1;
        assert!(open(corrupt_bytes,&c,TailRecovery::Refuse).is_err(),"byte {i}");
    }
    for i in j.entries[1].offset as usize..bytes.len() {
        let mut corrupt_bytes=bytes.clone();corrupt_bytes[i]^=1;
        assert!(open(corrupt_bytes,&c,TailRecovery::TruncateIncomplete).is_err(),"byte {i}");
    }
    Ok(())
}

#[test]
fn write_and_sync_failures_never_advance_the_visible_root()->Result<()> {
    let (j,c)=filled()?;let bytes=j.storage.bytes.into_inner();
    let frame=encode_frame(j.id,3,j.head,j.entries[1].anchor,&source(3))?;
    for count in [0,1,4,frame.len()/2,frame.len()-1] {
        let mut j=open(bytes.clone(),&c,TailRecovery::Refuse)?;let anchor=j.state.snapshot().map(WorldSnapshot::anchor);
        j.storage.fail_after=Some(count);assert!(j.append(source(3),&c).is_err());
        assert_eq!(j.state.snapshot().map(WorldSnapshot::anchor),anchor);assert_eq!(j.entries.len(),2);assert!(j.fenced);
        j.storage.fail_after=None;assert!(j.append(source(3),&c).is_err());
    }
    let mut j=open(bytes,&c,TailRecovery::Refuse)?;j.storage.sync_fails=true;
    assert!(j.append(source(3),&c).is_err());assert_eq!(j.entries.len(),2);assert!(j.fenced);
    let recovered=open(j.storage.bytes.into_inner(),&c,TailRecovery::Refuse)?;
    assert_eq!(recovered.entries.len(),3);
    Ok(())
}

#[test]
fn replay_preserves_generation_and_epoch_transitions()->Result<()> {
    let c=context()?;let mut j=open(Vec::new(),&c,TailRecovery::Refuse)?;
    j.append(source(1),&c)?;let mut missing=source(2);missing.jobs.jobs.clear();j.append(missing,&c)?;
    j.append(source(3),&c)?;let mut reset=source(1);reset.jobs.bridge_generation=8;j.append(reset,&c)?;
    let original=j.state.snapshot().cloned();let recovered=open(j.storage.bytes.into_inner(),&c,TailRecovery::Refuse)?;
    assert_eq!(recovered.state.snapshot(),original.as_ref());
    let root=recovered.state.snapshot().ok_or_else(||corrupt("fixture"))?;
    assert_eq!(root.cursor.epoch,1);assert_eq!(root.graph.entities[&dfmcp_core::EntityId::new(2)].generation,3);
    Ok(())
}

#[test]
fn authority_and_capacity_fail_without_writing()->Result<()> {
    let (mut j,mut c)=filled()?;let bytes=j.storage.bytes.get_ref().clone();
    c.grants.retain(|g|g.capability==Capability::Query);
    assert!(j.append(source(3),&c).is_err());assert_eq!(j.storage.bytes.get_ref(),&bytes);
    c.grants.clear();assert!(j.snapshot_at(1,j.entries[0].record_digest,&c).is_err());
    let c=context()?;let mut j=open(bytes.clone(),&c,TailRecovery::Refuse)?;
    j.limits.max_records=2;assert!(j.append(source(3),&c).is_err());assert!(!j.fenced);
    assert_eq!(j.storage.bytes.get_ref(),&bytes);j.limits.max_records=3;j.limits.max_bytes=bytes.len() as u64;
    assert!(j.append(source(3),&c).is_err());assert_eq!(j.storage.bytes.get_ref(),&bytes);Ok(())
}

#[test]
fn historical_access_uses_present_time_authority_and_exact_record_identity()->Result<()> {
    let (mut j,mut c)=filled()?;let entry=j.entries[0].clone();
    assert!(j.snapshot_at(0,entry.record_digest,&c).is_err());assert!(j.snapshot_at(9,entry.record_digest,&c).is_err());
    assert!(j.snapshot_at(1,Digest32::ZERO,&c).is_err());
    for g in &mut c.grants {g.expires_at_tick=Some(c.anchor.tick);}
    c.anchor.tick=GameTick(c.anchor.tick.0+1);
    assert!(j.snapshot_at(1,entry.record_digest,&c).is_err());Ok(())
}

#[test]
fn incomplete_tail_default_preserves_every_byte()->Result<()> {
    let (j,c)=filled()?;let mut bytes=j.storage.bytes.into_inner();bytes.extend_from_slice(&RECORD[..3]);
    let mut memory=Memory{bytes:Cursor::new(bytes.clone()),..Memory::default()};
    assert!(OperationsJournal::open(&mut memory,&c,JournalLimits::default(),false,TailRecovery::Refuse).is_err());
    assert_eq!(memory.bytes.into_inner(),bytes);Ok(())
}

#[test]
fn validly_hashed_wrong_projector_anchor_is_rejected()->Result<()> {
    let c=context()?;let mut j=open(Vec::new(),&c,TailRecovery::Refuse)?;
    let mut bad=c.anchor;bad.cursor.sequence=42;
    let frame=encode_frame(j.id,1,j.head,bad,&source(1))?;
    j.storage.bytes.get_mut().extend(frame);
    assert!(open(j.storage.bytes.into_inner(),&c,TailRecovery::TruncateIncomplete).is_err());Ok(())
}

#[cfg(unix)]
#[test]
fn real_file_reopen_and_exclusive_lock_require_private_custody()->Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let c=context()?;
    let dir=std::env::temp_dir().join(format!("dfmcp-journal-{}-{}",std::process::id(),c.session_id));
    std::fs::create_dir(&dir).map_err(storage_error)?;
    std::fs::set_permissions(&dir,std::fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
    let dir=dir.canonicalize().map_err(storage_error)?;let path=dir.join("history.bin");
    let mut j=open_private_journal(&path,&c,JournalLimits::default(),TailRecovery::Refuse)?;
    j.append(source(1),&c)?;
    assert!(open_private_journal(&path,&c,JournalLimits::default(),TailRecovery::Refuse).is_err());
    let expected=j.state.snapshot().cloned();drop(j);
    let recovered=open_private_journal(&path,&c,JournalLimits::default(),TailRecovery::Refuse)?;
    assert_eq!(recovered.state.snapshot(),expected.as_ref());drop(recovered);
    std::fs::set_permissions(&path,std::fs::Permissions::from_mode(0o644)).map_err(storage_error)?;
    assert!(open_private_journal(&path,&c,JournalLimits::default(),TailRecovery::Refuse).is_err());
    std::fs::remove_file(&path).map_err(storage_error)?;std::fs::remove_dir(&dir).map_err(storage_error)?;Ok(())
}
