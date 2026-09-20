use super::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::io::{Cursor, Read, Write, Seek};
use crate::workforce_control::tests::{capture, plan, context, record, unhex};
#[derive(Default)]
struct Memory { cursor: Cursor<Vec<u8>>, syncs: usize, fail_sync: Option<usize>, fail_write: bool }
#[derive(Clone, Default)]
struct Storage(Rc<RefCell<Memory>>);
impl Read for Storage { fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> { self.0.borrow_mut().cursor.read(bytes) } }
impl Seek for Storage { fn seek(&mut self, at: SeekFrom) -> io::Result<u64> { self.0.borrow_mut().cursor.seek(at) } }
impl Write for Storage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut data = self.0.borrow_mut(); if data.fail_write { return Err(io::Error::other("injected write")); }
        data.cursor.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl EffectJournalStorage for Storage {
    fn sync(&mut self) -> io::Result<()> {
        let mut data = self.0.borrow_mut(); data.syncs += 1;
        if data.fail_sync == Some(data.syncs) { Err(io::Error::other("injected sync")) } else { Ok(()) }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> { Err(io::Error::other("no repair")) }
}
fn binding() -> Result<WorkforceBinding> {
    WorkforceBinding::new(SocketAddr::from(([127,0,0,1],5000)),
        WorkforceManifest { generation: 42, df_version: "fake-df".into(), dfhack_version: "fake-dfhack".into() }, "region1".into(), 7)
}
struct Source {
    binding: WorkforceBinding, cap: WorkforceCapture, storage: Storage, native: Option<AssignmentEffect>,
    prepares: usize, commits: usize, queries: usize, cancels: usize, lost_prepare: bool, lost_commit: bool, unknown: bool, missing: bool,
}
impl Source {
    fn new(storage: Storage) -> Result<Self> { Ok(Self { binding: binding()?, cap: capture()?, storage, native: None,
        prepares: 0, commits: 0, queries: 0, cancels: 0, lost_prepare: false, lost_commit: false, unknown: false, missing: false }) }
    fn marker(&self, p: &AssignmentPlan) -> Result<AssignmentState> {
        let ctx = context()?;
        let mut check = WorkforceJournal::open(self.storage.clone(), &ctx, WorkforceMode::Offline, None)?;
        Ok(check.get(p.key(),p.digest(),&ctx)?.state())
    }
}
impl WorkforceSource for Source {
    fn manifest(&self) -> &WorkforceManifest { self.binding.manifest() }
    fn endpoint(&self) -> Option<SocketAddr> { Some(self.binding.endpoint()) }
    fn observe(&mut self, _: &[u32], _: &OperationContext) -> Result<WorkforceCapture> { Ok(self.cap.clone()) }
    fn prepare(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.marker(p)?, AssignmentState::Intent); self.prepares += 1;
        let value = AssignmentEffect::decode(&record(p,AssignmentPhase::Prepared,None)?,p)?; self.native = Some(value.clone());
        if self.lost_prepare { return Err(error(ErrorCode::AdapterRejected,"lost preparation reply")); } Ok(value)
    }
    fn commit(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.marker(p)?, AssignmentState::DispatchStarted); self.commits += 1;
        let value = if self.unknown { AssignmentEffect::decode(&record(p,AssignmentPhase::Unknown,None)?,p)? }
        else {
            let (mut after, changed) = p.before.expected(p.spec)?;
            for u in &mut after.citizens { if changed.contains(&u.id) && p.spec.assigned {
                for (bit, allowed) in u.labors.iter_mut().zip(&p.before.details[p.spec.detail as usize].labors) { *bit |= *allowed; }
            } }
            after.bytes = after.encode_values()?;
            let value = AssignmentEffect::decode(&record(p,AssignmentPhase::Applied,Some(&after))?,p)?; self.cap = after; value
        };
        self.native = Some(value.clone());
        if self.lost_commit { return Err(error(ErrorCode::AdapterRejected,"lost assignment reply")); } Ok(value)
    }
    fn query(&mut self, _: &AssignmentPlan, _: &OperationContext) -> Result<Option<AssignmentEffect>> {
        self.queries += 1; Ok(if self.missing { None } else { self.native.clone() })
    }
    fn cancel(&mut self, p: &AssignmentPlan, _: &OperationContext) -> Result<AssignmentEffect> {
        assert_eq!(self.marker(p)?, AssignmentState::CancelRequested); self.cancels += 1;
        let old = self.native.as_ref().ok_or_else(|| error(ErrorCode::AdapterRejected,"absent native record"))?;
        let value = if old.phase() == AssignmentPhase::Prepared { AssignmentEffect::decode(&record(p,AssignmentPhase::Cancelled,None)?,p)? }
            else { old.clone() };
        self.native = Some(value.clone()); Ok(value)
    }
}
fn setup() -> Result<(WorkforceJournal<Storage>, Source, OperationContext, AssignmentPlan)> {
    let ctx = context()?; let storage = Storage::default(); let source = Source::new(storage.clone())?;
    let journal = WorkforceJournal::open(storage,&ctx,WorkforceMode::Control,Some((binding()?,[b'n';32])))?;
    Ok((journal,source,ctx,plan()?))
}
#[test]
fn synced_lifecycle_and_offline_terminal_reopen() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?;
    assert_eq!(j.prepare(&mut s,&p,&ctx)?.state(),AssignmentState::Prepared);
    let done=j.commit(&mut s,p.key(),p.digest(),&ctx)?; assert_eq!(done.state(),AssignmentState::Terminal);
    assert_eq!(j.commit(&mut s,p.key(),p.digest(),&ctx)?,done); assert_eq!(s.commits,1);
    let mut read=ctx.clone(); read.session_id=SessionId::new(2); read.grants.retain(|g|g.capability==Capability::Query);
    let mut recovered=WorkforceJournal::open(j.storage.clone(),&read,WorkforceMode::Offline,None)?;
    assert_eq!(recovered.get(p.key(),p.digest(),&read)?,done); assert_eq!(recovered.view(&read)?.events,4);
    assert!(recovered.get(p.key(),p.digest(),&ctx).is_err()); Ok(())
}
#[test]
fn lost_preparation_can_only_recover_by_query_without_renewal() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; s.lost_prepare=true;
    assert!(j.prepare(&mut s,&p,&ctx).is_err()); assert_eq!(j.get(p.key(),p.digest(),&ctx)?.state(),AssignmentState::Intent);
    let mut recovered=WorkforceJournal::open(j.storage.clone(),&ctx,WorkforceMode::Recover,None)?;
    assert_eq!(recovered.reconcile(&mut s,p.key(),p.digest(),&ctx)?.state(),AssignmentState::Prepared);
    assert!(recovered.commit(&mut s,p.key(),p.digest(),&ctx).is_err()); assert_eq!(s.prepares,1); assert_eq!(s.commits,0); Ok(())
}
#[test]
fn lost_commit_reconciles_without_a_second_assignment() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; j.prepare(&mut s,&p,&ctx)?; s.lost_commit=true;
    assert!(j.commit(&mut s,p.key(),p.digest(),&ctx).is_err());
    assert!(j.commit(&mut s,p.key(),p.digest(),&ctx).is_err()); assert_eq!(s.commits,1);
    let mut recovered=WorkforceJournal::open(j.storage.clone(),&ctx,WorkforceMode::Recover,None)?;
    assert_eq!(recovered.reconcile(&mut s,p.key(),p.digest(),&ctx)?.state(),AssignmentState::Terminal);
    assert_eq!(s.commits,1); assert_eq!(s.queries,1); Ok(())
}
#[test]
fn uncertain_dispatch_sync_never_calls_setter_or_regains_prepared() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; j.prepare(&mut s,&p,&ctx)?;
    let next=j.storage.0.borrow().syncs+1; j.storage.0.borrow_mut().fail_sync=Some(next);
    assert!(j.commit(&mut s,p.key(),p.digest(),&ctx).is_err()); assert_eq!(s.commits,0); assert!(j.fenced);
    j.storage.0.borrow_mut().fail_sync=None;
    let mut recovered=WorkforceJournal::open(j.storage.clone(),&ctx,WorkforceMode::Control,None)?;
    assert_eq!(recovered.reconcile(&mut s,p.key(),p.digest(),&ctx)?.state(),AssignmentState::Tracking);
    assert!(recovered.commit(&mut s,p.key(),p.digest(),&ctx).is_err());
    assert_eq!(recovered.cancel(Some(&mut s),p.key(),p.digest(),&ctx)?.state(),AssignmentState::Terminal);
    assert_eq!(s.commits,0); assert_eq!(s.cancels,1); Ok(())
}
#[test]
fn failed_intent_write_cannot_prepare_native_work() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; j.storage.0.borrow_mut().fail_write=true;
    assert!(j.prepare(&mut s,&p,&ctx).is_err()); assert_eq!(s.prepares,0); assert_eq!(s.commits,0); assert!(j.fenced); Ok(())
}
#[test]
fn permanent_unknown_and_missing_record_block_new_intent() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; j.prepare(&mut s,&p,&ctx)?; s.unknown=true;
    let pending=j.commit(&mut s,p.key(),p.digest(),&ctx)?; assert!(pending.state().unresolved());
    assert_eq!(j.reconcile(&mut s,p.key(),p.digest(),&ctx)?,pending); assert_eq!(s.queries,0);
    let other=AssignmentPlan::new("other",p.spec(),p.before().clone())?;
    assert!(j.prepare(&mut s,&other,&ctx).is_err()); assert_eq!(s.prepares,1);
    let (mut j,mut s,ctx,p)=setup()?; j.prepare(&mut s,&p,&ctx)?; s.lost_commit=true;
    assert!(j.commit(&mut s,p.key(),p.digest(),&ctx).is_err()); s.missing=true;
    assert!(j.reconcile(&mut s,p.key(),p.digest(),&ctx).is_err());
    assert_eq!(j.get(p.key(),p.digest(),&ctx)?.state(),AssignmentState::DispatchStarted); Ok(())
}
#[test]
fn fixed_recovery_modes_and_local_cancellation() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; j.prepare(&mut s,&p,&ctx)?;
    for mode in [WorkforceMode::Offline,WorkforceMode::Recover] {
        let mut restricted=WorkforceJournal::open(j.storage.clone(),&ctx,mode,None)?;
        assert!(restricted.prepare(&mut s,&p,&ctx).is_err()); assert!(restricted.commit(&mut s,p.key(),p.digest(),&ctx).is_err());
        assert!(restricted.cancel::<Source>(None,p.key(),p.digest(),&ctx).is_err());
    }
    assert_eq!(j.cancel::<Source>(None,p.key(),p.digest(),&ctx)?.state(),AssignmentState::CancelledBeforeDispatch);
    assert_eq!(s.cancels,0); assert_eq!(s.commits,0); Ok(())
}
#[test]
fn exact_source_and_entity_budgets_are_enforced_before_dispatch() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?;
    s.cap.folder="other".into(); s.cap.bytes=s.cap.encode_values()?;
    assert!(j.prepare(&mut s,&p,&ctx).is_err()); assert!(j.view(&ctx)?.records.is_empty());
    s.cap=capture()?; let mut narrow=ctx.clone(); narrow.budget.max_entities=1;
    assert!(j.prepare(&mut s,&p,&narrow).is_err()); assert!(j.view(&ctx)?.records.is_empty());
    j.prepare(&mut s,&p,&ctx)?; s.binding.manifest.generation+=1;
    assert!(j.commit(&mut s,p.key(),p.digest(),&ctx).is_err()); assert_eq!(s.commits,0); Ok(())
}
#[test]
fn independently_encoded_binary_fixture_replays() -> Result<()> {
    let data=unhex(include_str!("../../../tests/fixtures/workforce_journal_v1_17.hex"))?;
    let storage=Storage::default(); storage.0.borrow_mut().cursor=Cursor::new(data);
    let ctx=context()?; let mut j=WorkforceJournal::open(storage,&ctx,WorkforceMode::Offline,None)?;
    let p=plan()?; let view=j.view(&ctx)?; assert_eq!(view.events,4);
    assert_eq!(j.get(p.key(),p.digest(),&ctx)?.effect().map(AssignmentEffect::phase),Some(AssignmentPhase::Applied)); Ok(())
}
#[test]
fn checksum_corruption_and_torn_frame_refuse_without_repair() -> Result<()> {
    let (mut j,mut s,ctx,p)=setup()?; j.prepare(&mut s,&p,&ctx)?; j.commit(&mut s,p.key(),p.digest(),&ctx)?;
    let raw=j.raw.clone();
    for at in 0..raw.len() {
        let storage=Storage::default(); let mut bytes=raw.clone(); bytes[at]^=1;
        storage.0.borrow_mut().cursor=Cursor::new(bytes);
        assert!(WorkforceJournal::open(storage,&ctx,WorkforceMode::Offline,None).is_err());
    }
    let storage=Storage::default(); storage.0.borrow_mut().cursor=Cursor::new(raw[..raw.len()-1].to_vec());
    assert!(WorkforceJournal::open(storage.clone(),&ctx,WorkforceMode::Recover,None).is_err());
    assert_eq!(storage.0.borrow().cursor.get_ref().len(),raw.len()-1); Ok(())
}
#[test]
fn transition_rules_never_restore_dispatch_eligibility() -> Result<()> {
    let p=plan()?;
    let prepared=AssignmentEffect::decode(&record(&p,AssignmentPhase::Prepared,None)?,&p)?;
    let old=AssignmentRecord {plan:p.clone(),state:AssignmentState::DispatchStarted,effect:Some(prepared.clone())};
    let next=AssignmentRecord {plan:p,state:AssignmentState::Prepared,effect:Some(prepared)};
    assert!(transition(Some(&old),&next).is_err());
    assert_eq!(reserve(AssignmentState::CancelRequested),1); assert_eq!(reserve(AssignmentState::Terminal),0);
    assert!(MAX_FRAME * 5 < MAX_BYTES); Ok(())
}
#[cfg(all(target_os="linux",any(target_arch="x86_64",target_arch="aarch64")))]
#[test]
fn real_private_file_lock_readonly_and_mode_custody() -> std::result::Result<(),Box<dyn std::error::Error>> {
    use std::fs; use std::os::unix::fs::PermissionsExt;
    let unique=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let dir=std::env::temp_dir().join(format!("dfmcp-workforce-{}-{unique}",std::process::id()));
    fs::create_dir(&dir)?; fs::set_permissions(&dir,fs::Permissions::from_mode(0o700))?;
    let path=dir.join("journal.bin"); let ctx=context()?;
    let mut j=crate::workforce_control::private_file::open_private_workforce(&path,&ctx,WorkforceMode::Control,Some(binding()?))?;
    assert!(crate::workforce_control::private_file::open_private_workforce(&path,&ctx,WorkforceMode::Offline,None).is_err());
    j.view(&ctx)?; drop(j);
    let mut offline=crate::workforce_control::private_file::open_private_workforce(&path,&ctx,WorkforceMode::Offline,None)?;
    assert!(offline.storage.write_all(b"must not write").is_err());
    fs::set_permissions(&path,fs::Permissions::from_mode(0o644))?;
    assert!(offline.view(&ctx).is_err());
    // Retain artifacts for inspection; no automatic removal or evidence repair.
    Ok(())
}
