use super::*;
use crate::dig_blueprint::monitor::tests as fixtures;
use dfmcp_world::map_region::Cell;
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::rc::Rc;

#[cfg(target_os = "linux")]
mod private;

pub(super) fn definition() -> Result<MonitorSpec> {
    fixtures::spec(fixtures::parts())
}
pub(super) fn context(spec: &MonitorSpec) -> OperationContext {
    fixtures::context(spec)
}
fn io_failure() -> io::Error {
    io::Error::other("injected monitor history failure")
}

#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    reads: usize,
    writes: usize,
    flushes: usize,
    syncs: usize,
    fail_sync: Option<usize>,
    fail_flush: bool,
    partial: Option<usize>,
    broken_write: bool,
    invalid: bool,
    invalidate_on_sync: bool,
    mutate_on_sync: bool,
}
#[derive(Clone, Default)]
struct Memory {
    disk: Rc<RefCell<Disk>>,
    position: usize,
}
impl Memory {
    fn with_bytes(bytes: Vec<u8>) -> Self {
        let memory = Self::default();
        memory.disk.borrow_mut().bytes = bytes;
        memory
    }
    fn snapshot_bytes(&self) -> Vec<u8> {
        self.disk.borrow().bytes.clone()
    }
    fn effects(&self) -> (usize, usize, usize) {
        let disk = self.disk.borrow();
        (disk.writes, disk.flushes, disk.syncs)
    }
}
impl Read for Memory {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.reads += 1;
        let start = self.position.min(disk.bytes.len());
        let count = output.len().min(disk.bytes.len() - start);
        output[..count].copy_from_slice(&disk.bytes[start..start + count]);
        self.position += count;
        Ok(count)
    }
}
impl Write for Memory {
    fn write(&mut self, raw: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.writes += 1;
        if disk.broken_write || self.position != disk.bytes.len() {
            return Err(io_failure());
        }
        let count = disk.partial.take().map_or(raw.len(), |n| {
            disk.broken_write = true;
            n.min(raw.len())
        });
        disk.bytes.extend_from_slice(&raw[..count]);
        self.position += count;
        Ok(count)
    }
    fn flush(&mut self) -> io::Result<()> {
        let mut disk = self.disk.borrow_mut();
        disk.flushes += 1;
        if disk.fail_flush {
            Err(io_failure())
        } else {
            Ok(())
        }
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let n = match position {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
        };
        self.position = usize::try_from(n).map_err(|_| io_failure())?;
        Ok(self.position as u64)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut disk = self.disk.borrow_mut();
        disk.syncs += 1;
        if disk.invalidate_on_sync {
            disk.invalid = true;
        }
        if disk.mutate_on_sync {
            let last = disk.bytes.last_mut().ok_or_else(io_failure)?;
            *last ^= 1;
        }
        if disk.fail_sync == Some(disk.syncs) {
            Err(io_failure())
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io_failure())
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.disk.borrow().invalid {
            Err(io_failure())
        } else {
            Ok(())
        }
    }
}
fn create(
    memory: Memory,
    spec: &MonitorSpec,
    c: &OperationContext,
) -> Result<MonitorStore<Memory>> {
    MonitorStore::open(memory, c, DigMode::Recover, Some(spec.clone()), true, None)
}
fn reopen(
    memory: Memory,
    c: &OperationContext,
    pin: Recovery,
    mode: DigMode,
) -> Result<MonitorStore<Memory>> {
    MonitorStore::open(memory, c, mode, None, false, Some(pin))
}
fn sample(store: &mut MonitorStore<Memory>, c: &OperationContext, tick: u64) -> Result<()> {
    let token = store.begin_attempt(c)?;
    store.capture(&token, fixtures::capture(store.spec(), tick)?, c)?;
    store.publish(&token, c)
}
fn code<T>(result: Result<T>) -> Option<ErrorCode> {
    result.err().map(|error| error.code)
}

#[test]
fn exact_creation_and_recovery_never_recreate_or_replace_history() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    assert_eq!(store.recovery(), initial_recovery(&spec)?);
    assert_eq!(store.state().status(), Status::Pending);
    store.verify(&c)?;
    let old = store.recovery();
    let header = memory.snapshot_bytes();
    let effects = memory.effects();
    assert!(create(memory.clone(), &spec, &c).is_err());
    assert!(MonitorStore::open(memory.clone(), &c, DigMode::Recover, None, false, None).is_err());
    assert!(
        MonitorStore::open(
            memory.clone(),
            &c,
            DigMode::Recover,
            Some(spec.clone()),
            true,
            Some(old)
        )
        .is_err()
    );
    for pin in [
        Recovery {
            monitor_id: Digest32::of_bytes(b"other monitor"),
            ..old
        },
        Recovery {
            head: Digest32::of_bytes(b"other history"),
            ..old
        },
        Recovery {
            head: Digest32::ZERO,
            ..old
        },
    ] {
        assert!(reopen(memory.clone(), &c, pin, DigMode::Recover).is_err());
    }
    assert!(reopen(Memory::default(), &c, old, DigMode::Recover).is_err());
    assert_eq!(memory.effects(), effects);
    assert_eq!(memory.snapshot_bytes(), header);
    sample(&mut store, &c, 1000)?;
    let newer = store.recovery();
    let recovered = reopen(memory.clone(), &c, old, DigMode::Recover)?;
    assert_eq!(recovered.recovery(), newer);
    assert_eq!(recovered.state().status(), Status::Unknown);
    assert!(reopen(Memory::with_bytes(header), &c, newer, DigMode::Offline).is_err());
    Ok(())
}

#[test]
fn read_intent_is_durable_and_tokens_bind_owner_request_and_order() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    let before = memory.effects();
    let token = store.begin_attempt(&c)?;
    assert!(memory.effects().2 > before.2);
    assert!(decode(&memory.snapshot_bytes())?.pending());
    assert!(store.is_fenced());
    assert_eq!(store.state().attempts(), 1);
    assert!(store.begin_attempt(&c).is_err());
    let mut wrong = c.clone();
    wrong.request_id = RequestId::new(2);
    assert_eq!(
        code(store.capture(&token, fixtures::capture(&spec, 1000)?, &wrong)),
        Some(ErrorCode::CapabilityDenied)
    );
    wrong = c.clone();
    wrong.session_id = SessionId::new(2);
    assert_eq!(
        code(store.interrupt(&token, &wrong)),
        Some(ErrorCode::CapabilityDenied)
    );
    store.capture(&token, fixtures::capture(&spec, 1000)?, &c)?;
    assert!(
        store
            .capture(&token, fixtures::capture(&spec, 1001)?, &c)
            .is_err()
    );
    store.publish(&token, &c)?;
    assert_eq!(store.state().status(), Status::Stabilizing);
    assert!(!store.is_fenced());
    assert!(store.publish(&token, &c).is_err());
    assert!(store.interrupt(&token, &c).is_err());
    Ok(())
}

#[test]
fn only_published_full_samples_contribute_to_stability_and_historical_success() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    sample(&mut store, &c, 1000)?;
    sample(&mut store, &c, 1000)?;
    assert_eq!(store.state().streak(), 1);
    let token = store.begin_attempt(&c)?;
    store.capture(&token, fixtures::capture(&spec, 1010)?, &c)?;
    assert_ne!(store.state().status(), Status::Satisfied);
    assert!(store.state().first_satisfied().is_none());
    store.publish(&token, &c)?;
    assert_eq!(store.state().status(), Status::Satisfied);
    assert_eq!(
        store.state().first_satisfied().map(|raw| raw.tick().get()),
        Some(1010)
    );
    let pin = store.recovery();
    let bytes = memory.snapshot_bytes();
    assert!(store.begin_attempt(&c).is_err());
    store.cancel(&c)?; // Idempotent local close cannot rewrite terminal history.
    assert_eq!(memory.snapshot_bytes(), bytes);
    let mut offline = reopen(memory.clone(), &c, pin, DigMode::Offline)?;
    let effects = memory.effects();
    offline.verify(&c)?;
    assert_eq!(offline.state().status(), Status::Satisfied);
    assert_eq!(
        offline.state().first_satisfied(),
        store.state().first_satisfied()
    );
    assert_eq!(memory.effects(), effects);
    assert!(offline.cancel(&c).is_err());
    assert_eq!(memory.effects(), effects);
    Ok(())
}

#[test]
fn replay_breaks_nonterminal_stability_and_persists_reset_before_next_read() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    sample(&mut store, &c, 1000)?;
    let mut reopened = reopen(memory.clone(), &c, store.recovery(), DigMode::Recover)?;
    assert_eq!(reopened.state().status(), Status::Unknown);
    assert_eq!(reopened.state().streak(), 0);
    assert_eq!(reopened.spec().policy(), spec.policy());
    assert_eq!(reopened.state().attempts(), 1);
    sample(&mut reopened, &c, 1010)?;
    assert_eq!(reopened.state().status(), Status::Stabilizing);
    assert_eq!(reopened.state().since_tick(), Some(1010));
    assert_eq!(decode(&memory.snapshot_bytes())?.state.streak(), 1);
    sample(&mut reopened, &c, 1020)?;
    assert_eq!(reopened.state().status(), Status::Satisfied);
    Ok(())
}

#[test]
fn pending_started_or_captured_reopen_fences_sampling_but_allows_query_cancel() -> Result<()> {
    for captured in [false, true] {
        let spec = definition()?;
        let c = context(&spec);
        let memory = Memory::default();
        let mut store = create(memory.clone(), &spec, &c)?;
        sample(&mut store, &c, 1000)?;
        let token = store.begin_attempt(&c)?;
        if captured {
            store.capture(&token, fixtures::capture(&spec, 1010)?, &c)?;
        }
        let mut query = c.clone();
        query.anchor.tick = GameTick(0);
        query
            .grants
            .retain(|grant| grant.capability == Capability::Query);
        let mut reopened = reopen(memory.clone(), &query, store.recovery(), DigMode::Recover)?;
        assert!(reopened.is_fenced());
        assert_eq!(reopened.state().status(), Status::Unknown);
        assert!(reopened.state().first_satisfied().is_none());
        assert_eq!(reopened.high_tick(), if captured { 1010 } else { 1000 });
        reopened.verify(&query)?;
        assert!(reopened.begin_attempt(&c).is_err());
        assert!(reopened.publish(&token, &c).is_err());
        reopened.cancel(&query)?;
        assert_eq!(reopened.state().status(), Status::Cancelled);
        assert!(!reopened.is_fenced());
        let recovered = reopen(memory, &query, reopened.recovery(), DigMode::Offline)?;
        assert_eq!(recovered.state().status(), Status::Cancelled);
        assert_eq!(recovered.high_tick(), reopened.high_tick());
    }
    Ok(())
}

#[test]
fn consumed_clock_precedes_new_authority_check_and_cannot_be_revived_by_old_anchor() -> Result<()> {
    let spec = definition()?;
    let mut c = context(&spec);
    for grant in &mut c.grants {
        grant.expires_at_tick = Some(GameTick(1005));
    }
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    let token = store.begin_attempt(&c)?;
    store.capture(&token, fixtures::capture(&spec, 1010)?, &c)?;
    assert_eq!(store.high_tick(), 1010);
    assert_eq!(decode(&memory.snapshot_bytes())?.high_tick, 1010);
    assert_eq!(
        code(store.publish(&token, &c)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert!(store.is_fenced());
    assert_eq!(code(store.verify(&c)), Some(ErrorCode::CapabilityDenied));
    assert_eq!(code(store.cancel(&c)), Some(ErrorCode::CapabilityDenied));
    let pin = store.recovery();
    let effects = memory.effects();
    c.anchor.tick = GameTick(0);
    assert_eq!(
        code(reopen(memory.clone(), &c, pin, DigMode::Recover)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert_eq!(memory.effects(), effects); // Failed retained-floor authority did not sync.
    let mut fresh = context(&spec);
    fresh.anchor.tick = GameTick(0);
    let mut recovered = reopen(memory.clone(), &fresh, pin, DigMode::Recover)?;
    assert!(recovered.is_fenced());
    assert_eq!(recovered.high_tick(), 1010);
    recovered.cancel(&fresh)?;
    assert_eq!(recovered.state().status(), Status::Cancelled);
    Ok(())
}

#[test]
fn source_changes_and_regressed_clocks_are_retained_terminal_negatives() -> Result<()> {
    for changed in 0..3 {
        let spec = definition()?;
        let c = context(&spec);
        let memory = Memory::default();
        let mut store = create(memory.clone(), &spec, &c)?;
        sample(&mut store, &c, 1010)?;
        let mut current = c.clone();
        current.anchor.tick = GameTick(1030);
        let token = store.begin_attempt(&current)?;
        let mut raw = fixtures::capture(&spec, if changed == 0 { 1020 } else { 1040 })?;
        if changed == 1 {
            raw.bridge_generation += 1;
        }
        if changed == 2 {
            raw.paused = false;
        }
        store.capture(&token, raw, &current)?;
        if changed == 1 {
            store.interrupt(&token, &current)?;
        } else if changed == 2 {
            store.cancel(&current)?;
        } else {
            store.publish(&token, &current)?;
        }
        assert_eq!(store.state().status(), Status::Invalidated);
        assert_eq!(
            store.state().latest().map(|capture| capture.tick().get()),
            Some(1010)
        );
        assert!(store.state().first_satisfied().is_none());
        assert_eq!(store.high_tick(), if changed == 0 { 1030 } else { 1040 });
        assert!(store.begin_attempt(&current).is_err());
        let recovered = reopen(memory, &c, store.recovery(), DigMode::Offline)?;
        assert_eq!(recovered.state().status(), Status::Invalidated);
        assert_eq!(recovered.high_tick(), store.high_tick());
    }
    Ok(())
}

#[test]
fn expired_capture_and_io_source_change_never_turn_into_retryable_gaps() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    for cancel in [false, true] {
        let memory = Memory::default();
        let mut expired = create(memory.clone(), &spec, &c)?;
        let token = expired.begin_attempt(&c)?;
        expired.capture(&token, fixtures::capture(&spec, 2001)?, &c)?;
        if cancel {
            expired.cancel(&c)?;
        } else {
            expired.interrupt(&token, &c)?;
        }
        assert_eq!(expired.state().status(), Status::Expired);
        assert_eq!(expired.high_tick(), 2001);
        let recovered = reopen(memory, &c, expired.recovery(), DigMode::Offline)?;
        assert_eq!(recovered.state().status(), Status::Expired);
    }
    let mut changed = create(Memory::default(), &spec, &c)?;
    let token = changed.begin_attempt(&c)?;
    changed.invalidate(&token, &c)?;
    assert_eq!(changed.state().status(), Status::Invalidated);
    assert!(changed.begin_attempt(&c).is_err());
    Ok(())
}

#[test]
fn interruption_clears_whole_goal_streak_without_refunding_attempt_or_deadline() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let mut store = create(Memory::default(), &spec, &c)?;
    sample(&mut store, &c, 1000)?;
    let token = store.begin_attempt(&c)?;
    store.interrupt(&token, &c)?;
    assert_eq!(store.state().attempts(), 2);
    assert_eq!(store.state().status(), Status::Unknown);
    assert_eq!(store.state().streak(), 0);
    sample(&mut store, &c, 1010)?;
    assert_eq!(store.state().status(), Status::Stabilizing);
    assert_eq!(store.state().since_tick(), Some(1010));
    assert_eq!(store.spec().policy().deadline_tick(), 2000);
    sample(&mut store, &c, 1020)?;
    assert_eq!(store.state().status(), Status::Satisfied);
    Ok(())
}

#[test]
fn exact_scoped_read_grants_are_sufficient_and_no_permission_is_restored_from_bytes() -> Result<()>
{
    let spec = definition()?;
    let c = context(&spec);
    assert!(c.grants.iter().all(|grant| grant.scope.map_area.is_some()));
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    let mut query = c.clone();
    query
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let effects = memory.effects();
    assert_eq!(
        code(store.begin_attempt(&query)),
        Some(ErrorCode::CapabilityDenied)
    );
    assert!(create(Memory::default(), &spec, &query).is_err());
    assert_eq!(memory.effects(), effects);
    store.verify(&query)?;
    let mut foreign = c.clone();
    foreign.anchor.fortress_id = dfmcp_core::FortressId::new(c.anchor.fortress_id.get() + 1);
    assert!(store.verify(&foreign).is_err());
    let mut narrow = c.clone();
    for grant in &mut narrow.grants {
        grant.scope.map_area = Some(MapCuboid::new(
            dfmcp_core::MapCoord::new(2, 2, 2),
            dfmcp_core::MapCoord::new(2, 2, 2),
        )?);
    }
    assert!(store.begin_attempt(&narrow).is_err());
    store.cancel(&query)?;
    let mut none = query.clone();
    none.grants.clear();
    assert!(reopen(memory, &none, store.recovery(), DigMode::Offline).is_err());
    Ok(())
}

#[test]
fn whole_attempt_preflight_and_absolute_deadline_precede_every_native_read() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    let mut small = c.clone();
    small.budget.max_bytes = store.operation_reserve() - 1;
    let before = memory.effects();
    let reads = memory.disk.borrow().reads;
    assert_eq!(
        code(store.begin_attempt(&small)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(memory.effects(), before);
    assert_eq!(memory.disk.borrow().reads, reads);
    assert_eq!(store.state().attempts(), 0);
    let token = store.begin_attempt(&c)?;
    store.active.as_mut().ok_or_else(corrupt)?.work.deadline = Instant::now();
    let after_start = memory.effects();
    assert_eq!(
        code(store.capture(&token, fixtures::capture(&spec, 1110)?, &c)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(store.high_tick(), 1110);
    assert!(store.is_fenced());
    assert_eq!(memory.effects(), after_start);
    // A fresh cancellation operation may close the read but cannot erase its
    // consumed clock merely because its original allowance has elapsed.
    store.cancel(&c)?;
    let recovered = reopen(memory, &c, store.recovery(), DigMode::Offline)?;
    assert_eq!(recovered.high_tick(), 1110);
    assert_eq!(recovered.state().status(), Status::Cancelled);
    Ok(())
}

#[test]
fn malformed_or_unpublished_candidates_never_serialize_positive_progress() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    let token = store.begin_attempt(&c)?;
    let before = memory.snapshot_bytes();
    let mut malformed = fixtures::capture(&spec, 1010)?;
    malformed.map.cells.pop();
    assert!(store.capture(&token, malformed, &c).is_err());
    assert_eq!(memory.snapshot_bytes(), before);
    store.interrupt(&token, &c)?;
    assert_eq!(store.state().status(), Status::Unknown);
    let initial = header(&spec)?;
    let forged = frame(&Event::Published, header_head(&initial), 1)?;
    let mut bytes = initial;
    bytes.extend_from_slice(&forged);
    assert!(
        reopen(
            Memory::with_bytes(bytes),
            &c,
            initial_recovery(&spec)?,
            DigMode::Offline
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn checksum_torn_frame_and_acknowledged_rollback_are_refused_without_repair() -> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    let initial = store.recovery();
    let header_len = store.byte_len();
    sample(&mut store, &c, 1000)?;
    let complete = memory.snapshot_bytes();
    let current = store.recovery();
    for cut in [0, 1, header_len - 1, header_len + 1, complete.len() - 1] {
        let broken = Memory::with_bytes(complete[..cut].to_vec());
        let before = broken.snapshot_bytes();
        assert!(reopen(broken.clone(), &c, current, DigMode::Recover).is_err());
        assert_eq!(broken.snapshot_bytes(), before);
        assert_eq!(broken.effects(), (0, 0, 0));
    }
    for at in [
        0,
        12,
        header_len - 9,
        header_len + 16,
        complete.len() - 9,
        complete.len() - 1,
    ] {
        let mut raw = complete.clone();
        raw[at] ^= 1;
        let broken = Memory::with_bytes(raw);
        assert!(reopen(broken.clone(), &c, initial, DigMode::Recover).is_err());
        assert_eq!(broken.effects(), (0, 0, 0));
    }
    assert!(
        reopen(
            Memory::with_bytes(complete[..header_len].to_vec()),
            &c,
            current,
            DigMode::Recover
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn ambiguous_capture_writes_fence_and_complete_survivors_remain_unpublished() -> Result<()> {
    for failure in 0..5 {
        let spec = definition()?;
        let c = context(&spec);
        let memory = Memory::default();
        let mut store = create(memory.clone(), &spec, &c)?;
        let token = store.begin_attempt(&c)?;
        let old = store.recovery();
        {
            let mut disk = memory.disk.borrow_mut();
            match failure {
                0 => disk.partial = Some(10),
                1 => disk.fail_flush = true,
                2 => disk.fail_sync = Some(disk.syncs + 1),
                3 => disk.invalidate_on_sync = true,
                _ => disk.mutate_on_sync = true,
            }
        }
        assert!(
            store
                .capture(&token, fixtures::capture(&spec, 1010)?, &c)
                .is_err()
        );
        assert!(store.is_fenced());
        assert_eq!(store.recovery(), old);
        assert_eq!(store.high_tick(), 1010);
        assert!(store.verify(&c).is_err());
        assert!(store.cancel(&c).is_err());
        let surviving = Memory::with_bytes(memory.snapshot_bytes());
        let recovered = reopen(surviving, &c, old, DigMode::Recover);
        if matches!(failure, 1..=3) {
            let recovered = recovered?;
            assert!(recovered.is_fenced());
            assert_eq!(recovered.high_tick(), 1010);
            assert!(recovered.state().first_satisfied().is_none());
        } else {
            assert!(recovered.is_err());
        }
    }
    Ok(())
}

#[test]
fn terminal_sync_failure_never_acknowledges_current_proof_and_exact_reopen_replays_survivor()
-> Result<()> {
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    sample(&mut store, &c, 1000)?;
    let token = store.begin_attempt(&c)?;
    store.capture(&token, fixtures::capture(&spec, 1010)?, &c)?;
    let pin = store.recovery();
    {
        let mut disk = memory.disk.borrow_mut();
        disk.fail_sync = Some(disk.syncs + 1);
    }
    assert!(store.publish(&token, &c).is_err());
    assert!(store.is_fenced());
    assert_eq!(store.recovery(), pin);
    assert_ne!(store.state().status(), Status::Satisfied);
    assert!(store.state().first_satisfied().is_none());
    let recovered = reopen(
        Memory::with_bytes(memory.snapshot_bytes()),
        &c,
        pin,
        DigMode::Recover,
    )?;
    assert_eq!(recovered.state().status(), Status::Satisfied);
    assert_eq!(
        recovered
            .state()
            .first_satisfied()
            .map(|raw| raw.tick().get()),
        Some(1010)
    );
    assert_ne!(recovered.head(), pin.head);
    Ok(())
}

#[test]
fn current_file_mutation_and_identity_loss_fence_all_cached_claims() -> Result<()> {
    for identity in [false, true] {
        let spec = definition()?;
        let c = context(&spec);
        let memory = Memory::default();
        let mut store = create(memory.clone(), &spec, &c)?;
        let pin = store.recovery();
        if identity {
            memory.disk.borrow_mut().invalid = true;
        } else {
            let mut disk = memory.disk.borrow_mut();
            let last = disk.bytes.last_mut().ok_or_else(corrupt)?;
            *last ^= 1;
        }
        let before = memory.effects();
        assert!(store.verify(&c).is_err());
        assert!(store.is_fenced());
        assert!(store.begin_attempt(&c).is_err());
        assert!(store.cancel(&c).is_err());
        assert_eq!(store.recovery(), pin);
        assert_eq!(memory.effects(), before);
    }
    Ok(())
}

#[test]
fn every_read_attempt_is_bounded_across_reopens_and_cancellation_keeps_reserved_room() -> Result<()>
{
    let spec = definition()?;
    let c = context(&spec);
    let memory = Memory::default();
    let mut store = create(memory.clone(), &spec, &c)?;
    for n in 0..MAX_ATTEMPTS {
        if n != 0 {
            store = reopen(memory.clone(), &c, store.recovery(), DigMode::Recover)?;
        }
        let token = store.begin_attempt(&c)?;
        let mut capture = fixtures::capture(&spec, 1000 + u64::from(n))?;
        capture.map.cells[0] = Cell::Hidden;
        store.capture(&token, capture, &c)?;
        store.publish(&token, &c)?;
        assert_eq!(store.state().attempts(), n + 1);
        assert_eq!(store.state().status(), Status::Unknown);
    }
    let before = memory.effects();
    assert_eq!(
        code(store.begin_attempt(&c)),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(memory.effects(), before);
    store.cancel(&c)?;
    assert_eq!(store.state().status(), Status::Cancelled);
    assert!(store.byte_len() < MAX_STORE_BYTES);
    let recovered = reopen(memory, &c, store.recovery(), DigMode::Offline)?;
    assert_eq!(recovered.state().attempts(), MAX_ATTEMPTS);
    assert_eq!(recovered.state().status(), Status::Cancelled);
    Ok(())
}
