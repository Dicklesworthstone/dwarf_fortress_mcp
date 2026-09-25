use super::*;
use crate::excavation_run::tests::{plan, prepared, stopped};
use dfmcp_core::{CapabilityGrant, CapabilityScope, ObservationCursor, RequestId,
    SessionId, StateAnchor, WorkBudget};
use std::cell::{Cell, RefCell};
use std::io::{self, Read, Seek, Write};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Memory {
    bytes: Rc<RefCell<Vec<u8>>>,
    position: u64,
    syncs: Rc<Cell<u32>>,
    fail_at: Rc<Cell<u32>>,
    partial: Rc<Cell<bool>>,
    invalid: Rc<Cell<bool>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let bytes = self.bytes.borrow();
        let start = (self.position as usize).min(bytes.len());
        let n = out.len().min(bytes.len() - start);
        out[..n].copy_from_slice(&bytes[start..start + n]);
        self.position += n as u64;
        Ok(n)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.partial.get() {
            self.bytes.borrow_mut().extend_from_slice(&bytes[..bytes.len().min(3)]);
            return Err(io::Error::other("partial write"));
        }
        let mut out = self.bytes.borrow_mut();
        let start = self.position as usize;
        let end = out.len().max(start + bytes.len());
        out.resize(end, 0);
        out[start..start + bytes.len()].copy_from_slice(bytes);
        self.position += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> { Ok(()) }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.bytes.borrow().len() as i128 + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
        };
        self.position = u64::try_from(next).map_err(|_| io::Error::other("negative seek"))?;
        Ok(self.position)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.syncs.set(self.syncs.get() + 1);
        if self.syncs.get() == self.fail_at.get() { return Err(io::Error::other("sync failure")); }
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> { panic!("coordinator must never truncate") }
    fn validate_identity(&self) -> io::Result<()> {
        if self.invalid.get() { return Err(io::Error::other("custody changed")); }
        Ok(())
    }
}
fn context() -> Result<OperationContext> {
    let p = plan()?;
    let fortress_id = p.before().fortress().fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1), request_id: RequestId::new(2),
        anchor: StateAnchor { fortress_id, cursor: ObservationCursor::ORIGIN,
            tick: GameTick(p.before().tick()), state_hash: p.before().witness() },
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        grants: [Capability::Query, Capability::Plan, Capability::ControlClock].into_iter()
            .map(|capability| CapabilityGrant { capability,
                scope: CapabilityScope { fortress_id: Some(fortress_id), ..CapabilityScope::default() },
                max_risk: RiskTier::Guarded, expires_at_tick: None, remaining_uses: None }).collect(),
        cancellation_requested: false,
    })
}
fn binding() -> Result<ExcavationBinding> {
    ExcavationBinding::new(SocketAddr::from(([127, 0, 0, 1], 5000)), "df", "dfhack", plan()?.before())
}
fn native_variant(phase: u8, reason: u8) -> Result<ExcavationRunRecord> {
    let mut raw = (if phase == 4 { prepared()? } else { stopped()? }).canonical_bytes().to_vec();
    let p = plan()?;
    let offset = 8 + 2 + p.key().len() + 24 + 2 + p.before().canonical_bytes().len() + 48;
    if phase == 4 { raw[offset..offset + 5].copy_from_slice(&[4, reason, 0, 0, 0]); }
    else if phase == 5 {
        raw[offset..offset + 5].copy_from_slice(&[5, reason, 1, 0, 0]);
        raw[offset + 5..offset + 13].fill(0);
    }
    let n = raw.len() - 32;
    let checksum = hash(b"dfmcp-excavation-run-receipt/1", &raw[..n]);
    raw[n..].copy_from_slice(checksum.as_bytes());
    ExcavationRunRecord::decode(&raw)
}
struct Native {
    binding: ExcavationBinding,
    storage: Memory,
    calls: Vec<&'static str>,
    record: Option<ExcavationRunRecord>,
    lost_commit: bool,
    wrong_capture: bool,
    mutate_on_query: bool,
    fenced: bool,
}
impl Native {
    fn new(storage: Memory) -> Result<Self> {
        Ok(Self { binding: binding()?, storage, calls: Vec::new(), record: None,
            lost_commit: false, wrong_capture: false, mutate_on_query: false, fenced: false })
    }
}
impl ExcavationRunSource for Native {
    fn binding(&self) -> &ExcavationBinding { &self.binding }
    fn fence(&mut self) { self.fenced = true; }
    fn observe(&mut self, _: ExcavationRegion, _: &OperationContext, _: Duration) -> Result<ExcavationCapture> {
        self.calls.push("observe");
        let mut bytes = plan()?.before().canonical_bytes().to_vec();
        if self.wrong_capture { bytes[39] += 1; }
        ExcavationCapture::decode(&bytes)
    }
    fn prepare(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("prepare");
        assert!(self.storage.syncs.get() >= 2, "intent must be synced first");
        let r = prepared()?; self.record = Some(r.clone()); Ok(r)
    }
    fn commit(&mut self, permit: ExcavationDispatch<'_>, c: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        self.calls.push("commit");
        assert!(self.storage.syncs.get() >= 4, "dispatch must be synced first");
        let reopened = ExcavationCoordinator::open(self.storage.clone(), permit.plan().before().fortress(), c)?;
        assert!(reopened.entry(permit.plan().key()).unwrap().dispatch_started());
        assert_eq!(reopened.head, permit.journal_head());
        let r = stopped()?; self.record = Some(r.clone());
        if self.lost_commit { return Err(uncertain()); }
        Ok(r)
    }
    fn query(&mut self, _: &ExcavationRunPlan, _: &OperationContext, _: Duration) -> Result<Option<ExcavationRunRecord>> {
        self.calls.push("query");
        if self.mutate_on_query { self.storage.bytes.borrow_mut().push(0); }
        Ok(self.record.clone())
    }
    fn cancel(&mut self, p: &ExcavationRunPlan, c: &OperationContext, _: Duration) -> Result<ExcavationRunRecord> {
        let reopened = ExcavationCoordinator::open(self.storage.clone(), p.before().fortress(), c)?;
        assert!(reopened.entry(p.key()).unwrap().cancel_requested(), "cancel intent must be durable first");
        self.calls.push("cancel");
        let r = native_variant(4, 3)?; self.record = Some(r.clone()); Ok(r)
    }
}
fn setup() -> Result<(ExcavationCoordinator<Memory>, Native, Memory, OperationContext)> {
    let c = context()?;
    let storage = Memory::default();
    let native = Native::new(storage.clone())?;
    let journal = ExcavationCoordinator::create(storage.clone(), binding()?, &c)?;
    Ok((journal, native, storage, c))
}

#[test]
fn start_syncs_dispatch_before_one_commit_and_offline_reopen() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c)?.resolved());
    assert_eq!(native.calls, ["observe", "query", "prepare", "commit"]);
    assert_eq!(storage.syncs.get(), 5);
    let mut reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
    assert_eq!(reopened.pending_count(), 0);
    let calls = native.calls.len();
    assert!(reopened.recover(&mut native, p.key(), false, &c)?.unwrap().resolved());
    assert_eq!(native.calls.len(), calls, "terminal query must be offline");
    assert!(reopened.start(&mut native, p.clone(), p.digest(), &c).is_err());
    Ok(())
}

#[test]
fn lost_commit_reopens_for_query_without_a_second_commit() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    native.lost_commit = true;
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    assert!(native.fenced);
    let mut reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
    assert_eq!(reopened.pending_count(), 1);
    assert!(reopened.start(&mut native, p.clone(), p.digest(), &c).is_err());
    assert!(reopened.recover(&mut native, p.key(), false, &c)?.unwrap().resolved());
    assert_eq!(native.calls.iter().filter(|n| **n == "commit").count(), 1);
    Ok(())
}

#[test]
fn each_predispatch_sync_failure_blocks_unpause() -> Result<()> {
    for fail in 2..=4 {
        let (mut journal, mut native, storage, c) = setup()?;
        storage.fail_at.set(fail);
        let p = plan()?;
        assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
        assert!(!native.calls.contains(&"commit"));
        assert!(journal.is_fenced());
        let mut reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
        assert_eq!(reopened.pending_count(), 1);
        assert!(reopened.start(&mut native, p.clone(), p.digest(), &c).is_err());
    }
    Ok(())
}

#[test]
fn terminal_sync_failure_never_retries_and_reopen_checks_surviving_bytes() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    storage.fail_at.set(5);
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    assert!(journal.is_fenced());
    let reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
    assert!(reopened.entry(p.key()).unwrap().native().unwrap().resolved());
    assert_eq!(native.calls.iter().filter(|n| **n == "commit").count(), 1);
    Ok(())
}

#[test]
fn pending_and_source_loss_block_new_keys() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    native.lost_commit = true;
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    let mut reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
    let other = ExcavationRunPlan::new("new-key", p.spec(), p.before().clone())?;
    assert!(reopened.start(&mut native, other.clone(), other.digest(), &c).is_err());
    native.record = Some(native_variant(5, 7)?);
    let lost = reopened.recover(&mut native, p.key(), false, &c)?.unwrap();
    assert!(lost.terminal() && !lost.resolved());
    assert_eq!(reopened.pending_count(), 1);
    assert!(reopened.start(&mut native, other.clone(), other.digest(), &c).is_err());
    Ok(())
}

#[test]
fn absent_native_record_keeps_pending_and_cannot_grant_permit() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    native.lost_commit = true;
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    let mut reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
    native.record = None;
    assert!(reopened.recover(&mut native, p.key(), false, &c)?.is_none());
    assert_eq!(reopened.pending_count(), 1);
    assert!(reopened.start(&mut native, p.clone(), p.digest(), &c).is_err());
    Ok(())
}

#[test]
fn stale_confirmation_retained_key_and_capture_change_prevent_prepare() -> Result<()> {
    for scenario in 0..3 {
        let (mut journal, mut native, _, c) = setup()?;
        let p = plan()?;
        if scenario == 1 { native.record = Some(prepared()?); }
        if scenario == 2 { native.wrong_capture = true; }
        let confirmed = if scenario == 0 { Digest32::ZERO } else { p.digest() };
        assert!(journal.start(&mut native, p, confirmed, &c).is_err());
        assert!(!native.calls.contains(&"prepare") && !native.calls.contains(&"commit"));
    }
    Ok(())
}

#[test]
fn authority_horizon_and_budget_fail_before_native_work() -> Result<()> {
    for scenario in 0..5 {
        let (mut journal, mut native, _, mut c) = setup()?;
        let p = plan()?;
        match scenario {
            0 => c.cancellation_requested = true,
            1 => c.grants.retain(|g| g.capability != Capability::ControlClock),
            2 => for grant in &mut c.grants { grant.expires_at_tick = Some(GameTick(p.before().tick() + 50)); },
            3 => c.budget.max_game_ticks = 99,
            _ => c.budget.max_bytes = 100,
        }
        assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
        assert!(native.calls.is_empty());
    }
    Ok(())
}

#[test]
fn partial_write_is_preserved_and_refuses_reopen() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    let before = storage.bytes.borrow().len();
    storage.partial.set(true);
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    assert_eq!(storage.bytes.borrow().len(), before + 3);
    assert!(!native.calls.contains(&"prepare"));
    assert!(ExcavationCoordinator::open(storage, p.before().fortress(), &c).is_err());
    Ok(())
}

#[test]
fn query_only_recovery_uses_current_grants_and_checks_post_query_custody() -> Result<()> {
    let (mut journal, mut native, storage, mut c) = setup()?;
    native.lost_commit = true;
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    c.grants.retain(|g| g.capability == Capability::Query);
    let mut reopened = ExcavationCoordinator::open(storage, p.before().fortress(), &c)?;
    assert!(reopened.recover(&mut native, p.key(), true, &c).is_err());
    native.record = None; native.mutate_on_query = true;
    assert!(reopened.recover(&mut native, p.key(), false, &c).is_err());
    assert!(reopened.is_fenced());
    Ok(())
}

#[test]
fn cancellation_is_durable_before_call_and_terminal_replay_is_offline() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    // Fail dispatch sync: native is only prepared, but it still needs explicit recovery.
    storage.fail_at.set(4);
    let p = plan()?;
    assert!(journal.start(&mut native, p.clone(), p.digest(), &c).is_err());
    let mut reopened = ExcavationCoordinator::open(storage.clone(), p.before().fortress(), &c)?;
    assert!(reopened.recover(&mut native, p.key(), true, &c)?.unwrap().resolved());
    assert!(reopened.entry(p.key()).unwrap().cancel_requested());
    let calls = native.calls.len();
    reopened.recover(&mut native, p.key(), true, &c)?;
    assert_eq!(native.calls.len(), calls);
    assert!(!native.calls.contains(&"commit"));
    Ok(())
}

#[test]
fn immutable_journal_corruption_and_empty_recovery_are_rejected() -> Result<()> {
    let (mut journal, mut native, storage, c) = setup()?;
    let p = plan()?;
    journal.start(&mut native, p.clone(), p.digest(), &c)?;
    let original = storage.bytes.borrow().clone();
    for i in 0..original.len() {
        let copy = Memory::default();
        let mut bad = original.clone(); bad[i] ^= 128;
        *copy.bytes.borrow_mut() = bad;
        assert!(ExcavationCoordinator::open(copy, p.before().fortress(), &c).is_err(), "byte {i}");
    }
    assert!(ExcavationCoordinator::open(Memory::default(), p.before().fortress(), &c).is_err());
    Ok(())
}

#[test]
fn independent_journal_fixture_reopens_without_native_contact() -> Result<()> {
    let raw = include_str!("../../../tests/fixtures/excavation_run_journal_v1_18.hex").trim();
    let bytes = (0..raw.len()).step_by(2)
        .map(|i| u8::from_str_radix(&raw[i..i + 2], 16).unwrap()).collect();
    let storage = Memory::default();
    *storage.bytes.borrow_mut() = bytes;
    let p = plan()?;
    let journal = ExcavationCoordinator::open(storage, p.before().fortress(), &context()?)?;
    assert_eq!(journal.frames, 5);
    assert_eq!(journal.pending_count(), 0);
    assert_eq!(journal.entry(p.key()).unwrap().native(), Some(&stopped()?));
    Ok(())
}
