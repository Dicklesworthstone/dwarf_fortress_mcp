//! Exercises the actual archive/store APIs. No native bridge is constructed.
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::rc::Rc;

use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::plan_archive::PlanArchiveStore;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, ErrorCode,
    FortressId, GameTick, OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget};
use dfmcp_intent::PreparedPlan;
use dfmcp_intent::plan_archive::{self as codec, ArchiveLimits, PlanArchiveSeal};

fn fixture() -> Result<(PreparedPlan, Vec<u8>, PlanArchiveSeal)> {
    let raw = include_str!("../../dfmcp-intent/tests/fixtures/plan_archive_pause_v1.hex").trim();
    let bytes = (0..raw.len()).step_by(2).map(|i| u8::from_str_radix(&raw[i..i+2],16)
        .map_err(|_| dfmcp_core::DfmcpError::new(ErrorCode::InvalidPlan,"fixture")))
        .collect::<Result<Vec<_>>>()?;
    let mut semantic = [0;32]; semantic.copy_from_slice(&bytes[bytes.len()-64..bytes.len()-32]);
    let mut object = [0;32]; object.copy_from_slice(&bytes[bytes.len()-32..]);
    // TEST fixture only: production expected identities must come from the
    // independently retained original handoff, not this file-trailer shortcut.
    let seal = PlanArchiveSeal { plan_digest: Digest32::from_bytes(semantic), object_digest: Digest32::from_bytes(object) };
    let plan = codec::decode(&bytes, seal, limits())?;
    Ok((plan, bytes, seal))
}
fn limits() -> ArchiveLimits {
    ArchiveLimits { max_bytes: 4096, max_steps: 8, max_nodes: 1024, max_depth: 16 }
}
fn context(plan: &PreparedPlan, write: bool) -> OperationContext {
    OperationContext { session_id: SessionId::new(701), request_id: RequestId::new(1), anchor: plan.anchor,
        budget: WorkBudget { max_bytes: 1_048_576, max_wall_millis: 60_000, max_entities: 256,
            ..WorkBudget::default() }, cancellation_requested: false,
        grants: [Capability::Query, Capability::Plan].into_iter().take(if write { 2 } else { 1 })
            .map(|capability| CapabilityGrant { capability, scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly, expires_at_tick: None, remaining_uses: None }).collect() }
}
struct Data {
    bytes: Cursor<Vec<u8>>, reads: usize, writes: usize, flushes: usize, syncs: usize,
    truncates: usize, identities: usize, valid: bool, fail_after: Option<usize>,
    written: usize, sync_failure: bool, corrupt_on_sync: bool,
}
#[derive(Clone)]
struct Memory(Rc<RefCell<Data>>);
impl Memory {
    fn new(bytes: Vec<u8>) -> Self {
        Self(Rc::new(RefCell::new(Data { bytes: Cursor::new(bytes), reads: 0, writes: 0,
            flushes: 0, syncs: 0, truncates: 0, identities: 0, valid: true,
            fail_after: None, written: 0, sync_failure: false, corrupt_on_sync: false })))
    }
    fn raw(&self) -> Vec<u8> { self.0.borrow().bytes.get_ref().clone() }
    fn no_writes(&self) {
        let d = self.0.borrow();
        assert_eq!((d.writes,d.flushes,d.syncs,d.truncates), (0,0,0,0));
    }
    fn no_io(&self) {
        self.no_writes();
        let d = self.0.borrow(); assert_eq!((d.reads,d.identities), (0,0));
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut d = self.0.borrow_mut(); d.reads += 1; d.bytes.read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> { self.0.borrow_mut().bytes.seek(pos) }
}
impl Write for Memory {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let mut d = self.0.borrow_mut(); d.writes += 1;
        let count = match d.fail_after {
            Some(n) if d.written >= n => return Err(io::Error::other("injected partial write")),
            Some(n) => input.len().min(n - d.written),
            None => input.len(),
        };
        let written = d.bytes.write(&input[..count])?; d.written += written; Ok(written)
    }
    fn flush(&mut self) -> io::Result<()> { self.0.borrow_mut().flushes += 1; Ok(()) }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut d = self.0.borrow_mut(); d.syncs += 1;
        if d.corrupt_on_sync { d.bytes.get_mut()[0] ^= 1; }
        if d.sync_failure { Err(io::Error::other("injected sync acknowledgment loss")) } else { Ok(()) }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        self.0.borrow_mut().truncates += 1; Err(io::Error::other("archive must not truncate"))
    }
    fn validate_identity(&self) -> io::Result<()> {
        let mut d = self.0.borrow_mut(); d.identities += 1;
        if d.valid { Ok(()) } else { Err(io::Error::other("replaced backing")) }
    }
}

#[test]
fn publication_syncs_before_return_and_recovery_preserves_original_expiry_and_keys() -> Result<()> {
    let (plan, raw, seal) = fixture()?;
    let memory = Memory::new(Vec::new());
    let store = PlanArchiveStore::create(memory.clone(), &plan, limits(), &context(&plan,true))?;
    assert_eq!(store.seal(), seal); assert_eq!(memory.raw(), raw);
    assert_eq!(memory.0.borrow().syncs, 1); assert_eq!(memory.0.borrow().truncates, 0);
    drop(store);
    let mut now = context(&plan,false);
    now.session_id = SessionId::new(702); now.anchor.cursor.sequence += 1; now.anchor.tick = GameTick(300);
    let mut reopened = PlanArchiveStore::open(memory.clone(), seal, limits(), &now)?;
    let original = reopened.recover(&now)?;
    assert_eq!(original, plan); assert_eq!(original.expires_at_tick, GameTick(200));
    assert_eq!(memory.0.borrow().syncs, 1);
    Ok(())
}
#[test]
fn archive_publication_requires_plan_authority_before_any_storage_operation() -> Result<()> {
    let (plan,_,_) = fixture()?; let memory = Memory::new(Vec::new());
    assert!(PlanArchiveStore::create(memory.clone(), &plan, limits(), &context(&plan,false)).is_err());
    memory.no_io(); assert!(memory.raw().is_empty()); Ok(())
}
#[test]
fn read_only_reopen_never_writes_flushes_syncs_or_repairs() -> Result<()> {
    let (plan,raw,seal) = fixture()?; let memory = Memory::new(raw);
    let c = context(&plan,false);
    let mut store = PlanArchiveStore::open(memory.clone(),seal,limits(),&c)?;
    assert_eq!(store.recover(&c)?,plan); memory.no_writes(); Ok(())
}
#[test]
fn existing_complete_or_partial_bytes_are_never_overwritten_by_create() -> Result<()> {
    let (plan,raw,_) = fixture()?;
    for bytes in [raw.clone(), raw[..20].to_vec(), vec![0]] {
        let memory = Memory::new(bytes.clone());
        assert!(matches!(PlanArchiveStore::create(memory.clone(),&plan,limits(),&context(&plan,true)),Err(e) if e.code==ErrorCode::Conflict));
        assert_eq!(memory.raw(),bytes); memory.no_writes();
    } Ok(())
}
#[test]
fn every_torn_prefix_and_corrupted_byte_refuses_without_repair() -> Result<()> {
    let (plan,raw,seal) = fixture()?; let c = context(&plan,false);
    for end in 0..raw.len() {
        let bytes = raw[..end].to_vec(); let memory = Memory::new(bytes.clone());
        assert!(PlanArchiveStore::open(memory.clone(),seal,limits(),&c).is_err());
        assert_eq!(memory.raw(),bytes); memory.no_writes();
    }
    for at in 0..raw.len() {
        let mut bytes = raw.clone(); bytes[at] ^= 1; let memory = Memory::new(bytes.clone());
        assert!(PlanArchiveStore::open(memory.clone(),seal,limits(),&c).is_err());
        assert_eq!(memory.raw(),bytes); memory.no_writes();
    } Ok(())
}
#[test]
fn partial_write_never_publishes_a_recoverable_archive_or_repairs_the_prefix() -> Result<()> {
    let (plan,raw,seal) = fixture()?;
    for limit in [0,1,43,raw.len()-1] {
        let memory = Memory::new(Vec::new()); memory.0.borrow_mut().fail_after=Some(limit);
        assert!(PlanArchiveStore::create(memory.clone(),&plan,limits(),&context(&plan,true)).is_err());
        assert_eq!(memory.raw(),raw[..limit]);
        assert!(PlanArchiveStore::open(memory.clone(),seal,limits(),&context(&plan,false)).is_err());
        assert_eq!(memory.0.borrow().truncates,0);
    } Ok(())
}
#[test]
fn lost_sync_ack_preserves_complete_bytes_for_explicit_read_only_reopen() -> Result<()> {
    let (plan,raw,seal)=fixture()?; let memory=Memory::new(Vec::new());
    memory.0.borrow_mut().sync_failure=true;
    assert!(PlanArchiveStore::create(memory.clone(),&plan,limits(),&context(&plan,true)).is_err());
    assert_eq!(memory.raw(),raw);
    let mut store=PlanArchiveStore::open(memory.clone(),seal,limits(),&context(&plan,false))?;
    assert_eq!(store.recover(&context(&plan,false))?,plan);
    assert_eq!(memory.0.borrow().syncs,1); assert_eq!(memory.0.borrow().truncates,0); Ok(())
}
#[test]
fn success_from_sync_is_insufficient_when_readback_differs() -> Result<()> {
    let (plan,_,seal)=fixture()?; let memory=Memory::new(Vec::new());
    memory.0.borrow_mut().corrupt_on_sync=true;
    assert!(matches!(PlanArchiveStore::create(memory.clone(),&plan,limits(),&context(&plan,true)),Err(e) if e.code==ErrorCode::CorruptLedger));
    assert!(PlanArchiveStore::open(memory.clone(),seal,limits(),&context(&plan,false)).is_err()); Ok(())
}
#[test]
fn changed_backing_fences_cached_plan_even_if_original_bytes_are_put_back() -> Result<()> {
    let (plan,raw,seal)=fixture()?; let c=context(&plan,false);
    for which in 0..3 {
        let memory=Memory::new(raw.clone()); let mut store=PlanArchiveStore::open(memory.clone(),seal,limits(),&c)?;
        match which {
            0=>memory.0.borrow_mut().bytes.get_mut()[20]^=1,
            1=>memory.0.borrow_mut().bytes.get_mut().push(0),
            _=>memory.0.borrow_mut().valid=false,
        }
        assert!(store.recover(&c).is_err()); assert!(store.is_fenced());
        memory.0.borrow_mut().bytes=Cursor::new(raw.clone()); memory.0.borrow_mut().valid=true;
        assert!(store.recover(&c).is_err()); drop(store);
        assert_eq!(PlanArchiveStore::open(memory.clone(),seal,limits(),&c)?.recover(&c)?,plan);
    } Ok(())
}
#[test]
fn current_authority_owner_and_fortress_gate_cached_reads() -> Result<()> {
    let (plan,raw,seal)=fixture()?; let c=context(&plan,false);
    let memory=Memory::new(raw); let mut store=PlanArchiveStore::open(memory.clone(),seal,limits(),&c)?;
    for which in 0..7 {
        let mut denied=c.clone();
        match which {
            0=>denied.grants.clear(), 1=>denied.cancellation_requested=true,
            2=>denied.session_id=SessionId::new(703),
            3=>denied.anchor.fortress_id=FortressId::new(99),
            4=>denied.grants[0].expires_at_tick=Some(GameTick(99)),
            5=>denied.grants[0].remaining_uses=Some(1),
            _=>denied.grants[0].scope.entity_ids=BTreeSet::from([EntityId::new(1)]),
        }
        let reads=memory.0.borrow().reads;
        assert!(store.recover(&denied).is_err()); assert_eq!(memory.0.borrow().reads,reads);
        assert!(!store.is_fenced());
    } Ok(())
}
#[test]
fn historical_anchor_does_not_revive_an_expired_current_query_grant() -> Result<()> {
    let (plan,raw,seal)=fixture()?; let memory=Memory::new(raw);
    let mut c=context(&plan,false); c.anchor.cursor.sequence+=1; c.anchor.tick=GameTick(300);
    c.grants[0].expires_at_tick=Some(GameTick(200));
    assert!(PlanArchiveStore::open(memory.clone(),seal,limits(),&c).is_err()); memory.no_io(); Ok(())
}
#[test]
fn current_context_floor_never_moves_backward_within_an_open_owner() -> Result<()> {
    let (plan,raw,seal)=fixture()?; let c=context(&plan,false); let memory=Memory::new(raw);
    let mut store=PlanArchiveStore::open(memory,seal,limits(),&c)?;
    let mut later=c.clone(); later.anchor.cursor.sequence+=1; later.anchor.tick=GameTick(300);
    store.recover(&later)?;
    assert!(matches!(store.recover(&c),Err(e) if e.code==ErrorCode::StaleAnchor));
    let mut restored=later.clone(); restored.anchor.cursor.epoch+=1;
    restored.anchor.cursor.sequence=0; restored.anchor.tick=GameTick(1);
    assert_eq!(store.recover(&restored)?,plan);
    assert!(store.recover(&later).is_err()); Ok(())
}
#[test]
fn whole_operation_byte_work_is_reserved_before_io_and_has_an_exact_boundary() -> Result<()> {
    let (plan,raw,seal)=fixture()?;
    let cost=4*limits().max_bytes as u64+2*raw.len() as u64;
    let mut c=context(&plan,true); c.budget.max_bytes=cost+1;
    let memory=Memory::new(Vec::new());
    PlanArchiveStore::create(memory.clone(),&plan,limits(),&c)?;
    let memory=Memory::new(Vec::new()); c.budget.max_bytes=cost;
    assert!(matches!(PlanArchiveStore::create(memory.clone(),&plan,limits(),&c),Err(e) if e.code==ErrorCode::BudgetExceeded));
    memory.no_io();
    let memory=Memory::new(raw); c.budget.max_bytes=4*limits().max_bytes as u64;
    assert!(PlanArchiveStore::open(memory.clone(),seal,limits(),&c).is_err()); memory.no_io(); Ok(())
}
#[test]
fn invalid_profile_or_unsealed_plan_does_not_touch_storage() -> Result<()> {
    let (mut plan,raw,seal)=fixture()?; let c=context(&plan,true);
    for invalid in [ArchiveLimits{max_depth:65,..limits()},ArchiveLimits{max_bytes:usize::MAX,..limits()}] {
        let memory=Memory::new(raw.clone());
        assert!(PlanArchiveStore::open(memory.clone(),seal,invalid,&c).is_err()); memory.no_io();
    }
    plan.digest=Digest32::ZERO; let memory=Memory::new(Vec::new());
    assert!(PlanArchiveStore::create(memory.clone(),&plan,limits(),&c).is_err()); memory.no_io(); Ok(())
}
#[test]
fn wrong_external_object_or_plan_identity_cannot_adopt_an_archive() -> Result<()> {
    let (plan,raw,seal)=fixture()?;
    for wrong in [PlanArchiveSeal{object_digest:Digest32::of_bytes(b"other"),..seal},
        PlanArchiveSeal{plan_digest:Digest32::of_bytes(b"other"),..seal}]
    {
        let memory=Memory::new(raw.clone());
        assert!(PlanArchiveStore::open(memory.clone(),wrong,limits(),&context(&plan,false)).is_err());
        memory.no_writes();
    } Ok(())
}

#[cfg(all(target_os="linux",any(target_arch="x86_64",target_arch="aarch64")))]
#[test]
fn real_private_file_is_exclusive_read_only_on_reopen_and_never_overwritten() -> Result<()> {
    use dfmcp_adapter::plan_archive::{create_private_plan_archive,open_private_plan_archive};
    use std::os::unix::fs::DirBuilderExt;
    let (plan,raw,seal)=fixture()?; let c=context(&plan,true);
    let io_error=|_:io::Error|dfmcp_core::DfmcpError::new(ErrorCode::CorruptLedger,"private archive fixture I/O");
    let directory=std::env::temp_dir().canonicalize().map_err(io_error)?
        .join(format!("dfmcp-plan-archive-{}",std::process::id()));
    std::fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
    let path=directory.join("original.bin");
    let store=create_private_plan_archive(&path,&plan,limits(),&c)?;
    assert_eq!(store.seal(),seal);
    assert!(open_private_plan_archive(&path,seal,limits(),&context(&plan,false)).is_err());
    drop(store);
    assert_eq!(std::fs::read(&path).map_err(io_error)?,raw);
    let mut store=open_private_plan_archive(&path,seal,limits(),&context(&plan,false))?;
    assert_eq!(store.recover(&context(&plan,false))?,plan); drop(store);
    assert!(create_private_plan_archive(&path,&plan,limits(),&c).is_err());
    assert_eq!(std::fs::read(&path).map_err(io_error)?,raw);
    let mut invalid=plan.clone(); invalid.digest=Digest32::ZERO;
    let invalid_path=directory.join("invalid.bin");
    assert!(create_private_plan_archive(&invalid_path,&invalid,limits(),&c).is_err());
    assert!(!invalid_path.exists());
    std::fs::remove_file(path).map_err(io_error)?;
    std::fs::remove_dir(directory).map_err(io_error)?; Ok(())
}
