use super::*;
use crate::order_run::tests::{capture_bytes, context, plan, record_bytes};
use dfmcp_core::Capability;
use std::cell::{Cell, RefCell};
use std::io::{Read, Seek, Write};
use std::rc::Rc;

#[derive(Clone, Default)]
struct Memory {
    data: Rc<RefCell<Vec<u8>>>,
    pos: u64,
    fail_sync: Rc<Cell<bool>>,
    syncs: Rc<Cell<u32>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let data = self.data.borrow();
        let start = (self.pos as usize).min(data.len());
        let n = out.len().min(data.len() - start);
        out[..n].copy_from_slice(&data[start..start + n]);
        self.pos += n as u64;
        Ok(n)
    }
}
impl Write for Memory {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        let mut data = self.data.borrow_mut();
        let start = self.pos as usize;
        if start + input.len() > data.len() {
            data.resize(start + input.len(), 0);
        }
        data[start..start + input.len()].copy_from_slice(input);
        self.pos += input.len() as u64;
        Ok(input.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let next = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.data.borrow().len() as i128 + i128::from(n),
            SeekFrom::Current(n) => i128::from(self.pos) + i128::from(n),
        };
        self.pos = u64::try_from(next).map_err(|_| io::Error::other("seek"))?;
        Ok(self.pos)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        if self.fail_sync.get() {
            return Err(io::Error::other("sync"));
        }
        self.syncs.set(self.syncs.get() + 1);
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no truncation"))
    }
}
struct Native {
    manifest: OrderRunManifest,
    fortress: FortressIdentity,
    before: OrderCapture,
    record: Option<OrderRunRecord>,
    prepares: u32,
    commits: u32,
    cancels: u32,
    reads: u32,
    fail_prepare: bool,
    fail_commit: bool,
    fail_cancel: bool,
    storage: Memory,
    fail_receipt_sync: bool,
}
impl Native {
    fn new(storage: Memory) -> Result<Self> {
        Ok(Self {
            manifest: OrderRunManifest {
                generation: 41,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            fortress: FortressIdentity::new("region1", 7)?,
            before: plan()?.before().clone(),
            record: None,
            prepares: 0,
            commits: 0,
            cancels: 0,
            reads: 0,
            fail_prepare: false,
            fail_commit: false,
            fail_cancel: false,
            storage,
            fail_receipt_sync: false,
        })
    }
}
impl OrderRunSource for Native {
    fn manifest(&self) -> &OrderRunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(SocketAddr::from(([127, 0, 0, 1], 5000)))
    }
    fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    fn fence(&mut self) {}
    fn observe(&mut self, _: u32, _: &OperationContext, _: Duration) -> Result<OrderCapture> {
        self.reads += 1;
        Ok(self.before.clone())
    }
    fn prepare(
        &mut self,
        p: &OrderRunPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        assert!(self.storage.syncs.get() >= 3);
        self.prepares += 1;
        let r = OrderRunRecord::decode(&record_bytes(p, 0, 0, 0, None, 0))?;
        self.record = Some(r.clone());
        if self.fail_prepare {
            return Err(uncertain());
        }
        Ok(r)
    }
    fn commit(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        let durable = OrderRunJournal::open(
            self.storage.clone(),
            c,
            OrderRunMode::Offline,
            &self.fortress,
            None,
            false,
        )?;
        assert_eq!(
            durable.lookup(p.key(), p.digest())?.state(),
            OrderRunState::DispatchStarted
        );
        self.commits += 1;
        let r = OrderRunRecord::decode(&record_bytes(p, 1, 0, 0, None, 0))?;
        self.record = Some(r.clone());
        if self.fail_receipt_sync {
            self.storage.fail_sync.set(true);
        }
        if self.fail_commit {
            return Err(uncertain());
        }
        Ok(r)
    }
    fn query(
        &mut self,
        _: &OrderRunPlan,
        _: &OperationContext,
        _: Duration,
    ) -> Result<Option<OrderRunRecord>> {
        Ok(self.record.clone())
    }
    fn cancel(
        &mut self,
        p: &OrderRunPlan,
        c: &OperationContext,
        _: Duration,
    ) -> Result<OrderRunRecord> {
        let durable = OrderRunJournal::open(
            self.storage.clone(),
            c,
            OrderRunMode::Offline,
            &self.fortress,
            None,
            false,
        )?;
        assert_eq!(
            durable.lookup(p.key(), p.digest())?.state(),
            OrderRunState::CancelRequested
        );
        self.cancels += 1;
        if self.fail_cancel {
            return Err(uncertain());
        }
        let r = OrderRunRecord::decode(&record_bytes(p, 3, 3, 0, None, 0))?;
        self.record = Some(r.clone());
        Ok(r)
    }
}
fn open(
    memory: Memory,
    n: &Native,
    c: &OperationContext,
    mode: OrderRunMode,
    initialize: bool,
) -> Result<OrderRunJournal<Memory>> {
    OrderRunJournal::open(
        memory,
        c,
        mode,
        n.fortress(),
        if mode == OrderRunMode::Control {
            Some(OrderRunBinding::from_source(n)?)
        } else {
            None
        },
        initialize,
    )
}
#[test]
fn lifecycle_retains_predicate_receipt_and_offline_history() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    assert_eq!(j.prepare(&mut n, &p, &c)?.state(), OrderRunState::Prepared);
    assert_eq!(
        j.commit(&mut n, p.key(), p.digest(), &c)?.state(),
        OrderRunState::Tracking
    );
    let sample = capture_bytes("region1", 4, 105, false, 1);
    n.record = Some(OrderRunRecord::decode(&record_bytes(
        &p,
        3,
        3,
        1,
        Some(&sample),
        2,
    ))?);
    let terminal = j.reconcile(&mut n, p.key(), p.digest(), &c)?;
    assert!(terminal.settled());
    assert!(
        terminal
            .native()
            .is_some_and(OrderRunRecord::predicate_observed)
    );
    drop(j);
    let mut readonly = c.clone();
    readonly.session_id = SessionId::new(2);
    readonly
        .grants
        .retain(|g| g.capability == Capability::Query);
    let mut offline = open(memory, &n, &readonly, OrderRunMode::Offline, false)?;
    assert_eq!(offline.view(&readonly)?.entries, vec![terminal]);
    assert_eq!(n.commits, 1);
    Ok(())
}
#[test]
fn lost_prepare_recovers_without_repreparing_or_extending_lifetime() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    n.fail_prepare = true;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    assert!(j.prepare(&mut n, &p, &c).is_err());
    assert_eq!(j.view(&c)?.entries[0].state(), OrderRunState::Intent);
    drop(j);
    let mut recover = open(memory, &n, &c, OrderRunMode::Recover, false)?;
    assert_eq!(
        recover.reconcile(&mut n, p.key(), p.digest(), &c)?.state(),
        OrderRunState::Prepared
    );
    assert_eq!(n.prepares, 1);
    assert!(recover.commit(&mut n, p.key(), p.digest(), &c).is_err());
    Ok(())
}
#[test]
fn lost_commit_and_prepared_query_can_never_restore_dispatch_eligibility() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    j.prepare(&mut n, &p, &c)?;
    n.fail_commit = true;
    assert!(j.commit(&mut n, p.key(), p.digest(), &c).is_err());
    drop(j);
    let mut j = open(memory, &n, &c, OrderRunMode::Control, false)?;
    assert!(j.commit(&mut n, p.key(), p.digest(), &c).is_err());
    assert_eq!(n.commits, 1);
    n.record = Some(OrderRunRecord::decode(&record_bytes(&p, 0, 0, 0, None, 0))?);
    assert_eq!(
        j.reconcile(&mut n, p.key(), p.digest(), &c)?.state(),
        OrderRunState::Tracking
    );
    assert!(j.commit(&mut n, p.key(), p.digest(), &c).is_err());
    assert_eq!(n.commits, 1);
    Ok(())
}
#[test]
fn failed_receipt_sync_fences_but_reopen_never_redispatches() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    j.prepare(&mut n, &p, &c)?;
    n.fail_receipt_sync = true;
    assert!(j.commit(&mut n, p.key(), p.digest(), &c).is_err());
    assert!(j.view(&c).is_err());
    memory.fail_sync.set(false);
    drop(j);
    let mut j = open(memory, &n, &c, OrderRunMode::Control, false)?;
    assert!(j.commit(&mut n, p.key(), p.digest(), &c).is_err());
    assert_eq!(n.commits, 1);
    Ok(())
}
#[test]
fn intent_sync_failure_prevents_native_prepare() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    memory.fail_sync.set(true);
    assert!(j.prepare(&mut n, &p, &c).is_err());
    assert_eq!(n.prepares, 0);
    assert_eq!(n.commits, 0);
    Ok(())
}
#[test]
fn missing_receipt_and_source_loss_do_not_release_control_block() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory, &n, &c, OrderRunMode::Control, true)?;
    j.prepare(&mut n, &p, &c)?;
    j.commit(&mut n, p.key(), p.digest(), &c)?;
    n.record = None;
    assert!(j.reconcile(&mut n, p.key(), p.digest(), &c).is_err());
    n.record = Some(OrderRunRecord::decode(&record_bytes(&p, 5, 7, 6, None, 0))?);
    let lost = j.reconcile(&mut n, p.key(), p.digest(), &c)?;
    assert!(lost.unresolved() && !lost.settled());
    let next = OrderRunPlan::new("next", p.spec(), p.before().clone())?;
    assert!(j.prepare(&mut n, &next, &c).is_err());
    Ok(())
}
#[test]
fn local_cancel_has_no_native_edge_and_active_cancel_never_repeats_unpause() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory, &n, &c, OrderRunMode::Control, true)?;
    j.prepare(&mut n, &p, &c)?;
    assert!(j.cancel::<Native>(None, p.key(), p.digest(), &c)?.settled());
    assert_eq!(n.cancels, 0);
    let p = OrderRunPlan::new("second", p.spec(), p.before().clone())?;
    j.prepare(&mut n, &p, &c)?;
    j.commit(&mut n, p.key(), p.digest(), &c)?;
    n.fail_cancel = true;
    assert!(j.cancel(Some(&mut n), p.key(), p.digest(), &c).is_err());
    assert_eq!(
        j.view(&c)?.entries[1].state(),
        OrderRunState::CancelRequested
    );
    n.fail_cancel = false;
    assert!(j.cancel(Some(&mut n), p.key(), p.digest(), &c)?.settled());
    assert_eq!(n.commits, 1);
    assert_eq!(n.cancels, 2);
    Ok(())
}
#[test]
fn wrong_fortress_owner_mode_and_budget_are_pre_dispatch() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    let mut bad = c.clone();
    bad.session_id = SessionId::new(99);
    assert!(j.prepare(&mut n, &p, &bad).is_err());
    bad = c.clone();
    bad.budget.max_bytes = 1;
    assert!(j.prepare(&mut n, &p, &bad).is_err());
    bad = c.clone();
    bad.budget.max_game_ticks = 0;
    assert!(j.prepare(&mut n, &p, &bad).is_err());
    assert_eq!(n.prepares, 0);
    assert_eq!(n.reads, 0);
    drop(j);
    assert!(
        OrderRunJournal::open(
            memory.clone(),
            &c,
            OrderRunMode::Offline,
            &FortressIdentity::new("other", 7)?,
            None,
            false
        )
        .is_err()
    );
    let mut offline = open(memory, &n, &c, OrderRunMode::Offline, false)?;
    assert!(offline.prepare(&mut n, &p, &c).is_err());
    assert!(offline.observe(&mut n, 9, &c).is_err());
    Ok(())
}
#[test]
fn corruption_and_every_torn_prefix_fail_reopen_without_repair() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, true)?;
    let header_length = memory.data.borrow().len();
    j.prepare(&mut n, &p, &c)?;
    drop(j);
    let bytes = memory.data.borrow().clone();
    // The header alone is a valid empty journal; incomplete first-frame prefixes are not.
    let first_body = u32::from_be_bytes(
        bytes[header_length + 8..header_length + 12]
            .try_into()
            .map_err(|_| corrupt())?,
    ) as usize;
    for end in header_length + 1..header_length + 92 + first_body {
        let m = Memory::default();
        *m.data.borrow_mut() = bytes[..end].to_vec();
        assert!(open(m, &n, &c, OrderRunMode::Offline, false).is_err());
    }
    let mut j = open(memory.clone(), &n, &c, OrderRunMode::Control, false)?;
    memory.data.borrow_mut()[header_length + 20] ^= 1;
    assert!(j.view(&c).is_err());
    assert!(open(memory, &n, &c, OrderRunMode::Offline, false).is_err());
    Ok(())
}
#[test]
fn illegal_rehashed_transition_and_native_plan_substitution_are_refused() -> Result<()> {
    let p = plan()?;
    let intent = OrderRunEntry {
        plan: p.clone(),
        state: OrderRunState::Intent,
        native: None,
    };
    let prepared = OrderRunEntry {
        plan: p.clone(),
        state: OrderRunState::Prepared,
        native: Some(OrderRunRecord::decode(&record_bytes(&p, 0, 0, 0, None, 0))?),
    };
    assert!(transition(None, &prepared).is_err());
    transition(Some(&intent), &prepared)?;
    let dispatched = OrderRunEntry {
        state: OrderRunState::DispatchStarted,
        ..prepared.clone()
    };
    transition(Some(&prepared), &dispatched)?;
    assert!(transition(Some(&dispatched), &prepared).is_err());
    let other = OrderRunPlan::new("other", p.spec(), p.before().clone())?;
    let invalid = OrderRunEntry {
        plan: other,
        ..prepared
    };
    assert!(invalid.validate().is_err());
    Ok(())
}
#[test]
fn retention_leaves_cancellation_and_terminal_slots() -> Result<()> {
    let memory = Memory::default();
    let n = Native::new(memory.clone())?;
    let c = context()?;
    let mut j = open(memory, &n, &c, OrderRunMode::Control, true)?;
    j.transitions = MAX_TRANSITIONS - 3;
    assert!(j.room(4).is_err());
    j.room(3)?;
    j.transitions = MAX_TRANSITIONS - 1;
    j.room(1)?;
    assert!(j.room(2).is_err());
    Ok(())
}

#[test]
fn repeated_cancel_can_use_the_reserved_final_terminal_slot() -> Result<()> {
    let memory = Memory::default();
    let mut n = Native::new(memory.clone())?;
    let c = context()?;
    let p = plan()?;
    let mut j = open(memory, &n, &c, OrderRunMode::Control, true)?;
    j.prepare(&mut n, &p, &c)?;
    j.commit(&mut n, p.key(), p.digest(), &c)?;
    n.fail_cancel = true;
    assert!(j.cancel(Some(&mut n), p.key(), p.digest(), &c).is_err());
    j.transitions = MAX_TRANSITIONS - 1;
    n.fail_cancel = false;
    assert!(j.cancel(Some(&mut n), p.key(), p.digest(), &c)?.settled());
    assert_eq!(j.transitions, MAX_TRANSITIONS);
    assert_eq!(n.commits, 1);
    Ok(())
}
#[test]
fn independent_binary_journal_fixture_recovers_exact_terminal_predicate() -> Result<()> {
    let raw = include_str!("../../../tests/fixtures/order_run_journal_v1_14.hex").trim();
    let bytes = raw
        .as_bytes()
        .chunks(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| corrupt())?;
            u8::from_str_radix(text, 16).map_err(|_| corrupt())
        })
        .collect::<Result<Vec<_>>>()?;
    let memory = Memory::default();
    *memory.data.borrow_mut() = bytes;
    let n = Native::new(memory.clone())?;
    let c = context()?;
    let mut j = open(memory, &n, &c, OrderRunMode::Offline, false)?;
    let view = j.view(&c)?;
    assert_eq!(view.transitions, 4);
    assert_eq!(view.entries.len(), 1);
    assert!(
        view.entries[0]
            .native()
            .is_some_and(OrderRunRecord::predicate_observed)
    );
    assert_eq!(view.binding.fortress(), plan()?.before().fortress());
    Ok(())
}
#[test]
#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
fn private_file_ownership_reopen_links_and_same_length_corruption() -> Result<()> {
    use crate::order_run::private_file::open_private_order_journal;
    use std::fs::{self, OpenOptions};
    use std::os::unix::fs::{PermissionsExt, symlink};
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| corrupt())?
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "dfmcp-order-run-rust-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&dir).map_err(storage_error)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).map_err(storage_error)?;
    let dir = dir.canonicalize().map_err(storage_error)?;
    let path = dir.join("runs.bin");
    let n = Native::new(Memory::default())?;
    let c = context()?;
    let binding = OrderRunBinding::from_source(&n)?;
    let j = open_private_order_journal(
        &path,
        n.fortress(),
        &c,
        OrderRunMode::Control,
        Some(binding.clone()),
    )?;
    assert!(
        open_private_order_journal(&path, n.fortress(), &c, OrderRunMode::Offline, None).is_err()
    );
    drop(j);
    let bytes = fs::read(&path).map_err(storage_error)?;
    let mut offline =
        open_private_order_journal(&path, n.fortress(), &c, OrderRunMode::Offline, None)?;
    assert!(offline.view(&c)?.entries.is_empty());
    drop(offline);
    assert_eq!(fs::read(&path).map_err(storage_error)?, bytes);
    let link = dir.join("alias.bin");
    symlink(&path, &link).map_err(storage_error)?;
    assert!(
        open_private_order_journal(&link, n.fortress(), &c, OrderRunMode::Offline, None).is_err()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).map_err(storage_error)?;
    assert!(
        open_private_order_journal(
            &path,
            n.fortress(),
            &c,
            OrderRunMode::Control,
            Some(binding.clone())
        )
        .is_err()
    );
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(storage_error)?;
    let mut j = open_private_order_journal(
        &path,
        n.fortress(),
        &c,
        OrderRunMode::Control,
        Some(binding),
    )?;
    let mut other = OpenOptions::new()
        .write(true)
        .open(&path)
        .map_err(storage_error)?;
    other.seek(SeekFrom::Start(12)).map_err(storage_error)?;
    other.write_all(&[bytes[12] ^ 1]).map_err(storage_error)?;
    assert!(j.view(&c).is_err());
    // Keep artifacts for inspection; no recursive deletion or evidence repair.
    Ok(())
}
