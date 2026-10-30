use super::*;
use crate::dig_blueprint::{BlueprintPart, Layout};
use crate::dig_designation::DigObservation;
use crate::dig_designation::journal::DigBinding;
use crate::dig_designation::rpc::DigManifest;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick, MapCoord, MapCuboid,
    ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn capture() -> Result<DigObservation> {
    let encoded =
        include_str!("../../../../../tests/native/dig_designation/vectors/observation.hex").trim();
    if encoded.len() % 2 != 0 {
        return Err(corrupt());
    }
    let bytes = encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let text = std::str::from_utf8(pair).map_err(|_| corrupt())?;
            u8::from_str_radix(text, 16).map_err(|_| corrupt())
        })
        .collect::<Result<Vec<_>>>()?;
    DigObservation::decode(&bytes)
}
fn binding(port: u16) -> Result<DigBinding> {
    DigBinding::new(
        SocketAddr::from(([127, 0, 0, 1], port)),
        DigManifest {
            generation: 7,
            df_version: "test-df".into(),
            dfhack_version: "test-dfhack".into(),
        },
        &capture()?,
        MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(63, 63, 7))?,
    )
}
fn definition() -> Result<BatchDefinition> {
    BatchDefinition::new(
        Layout::new(vec![BlueprintPart {
            origin: [15, 15, 2],
            size: [2, 2, 1],
        }])?,
        binding(5000)?,
        Digest32::of_bytes(b"exact original native dig journal"),
        capture()?,
    )
}
fn context() -> Result<OperationContext> {
    let before = capture()?;
    let fortress_id = before.fortress_id();
    let scope = binding(5000)?.scope();
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(before.tick()),
            state_hash: before.witness(),
        },
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: 4 * 1024 * 1024,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [Capability::Query, Capability::Plan]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress_id),
                    map_area: Some(scope),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    })
}
fn query_context() -> Result<OperationContext> {
    let mut c = context()?;
    c.grants
        .retain(|grant| grant.capability == Capability::Query);
    Ok(c)
}
fn io_failure() -> io::Error {
    io::Error::other("injected blueprint storage failure")
}
#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
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
    read_delay: bool,
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
}
impl Read for Memory {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let disk = self.disk.borrow();
        if disk.read_delay {
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(3) {
                std::hint::spin_loop();
            }
        }
        let start = self.position.min(disk.bytes.len());
        let count = output.len().min(disk.bytes.len() - start);
        output[..count].copy_from_slice(&disk.bytes[start..start + count]);
        self.position += count;
        Ok(count)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut disk = self.disk.borrow_mut();
        disk.writes += 1;
        if disk.broken_write || self.position != disk.bytes.len() {
            return Err(io_failure());
        }
        let count = disk.partial.take().map_or(bytes.len(), |n| {
            disk.broken_write = true;
            n.min(bytes.len())
        });
        disk.bytes.extend_from_slice(&bytes[..count]);
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
    fn seek(&mut self, seek: SeekFrom) -> io::Result<u64> {
        let next = match seek {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::End(n) => self.disk.borrow().bytes.len() as i128 + i128::from(n),
            SeekFrom::Current(n) => self.position as i128 + i128::from(n),
        };
        self.position = usize::try_from(next).map_err(|_| io_failure())?;
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
fn create(memory: Memory) -> Result<BatchStore<Memory>> {
    BatchStore::open(
        memory,
        &context()?,
        DigMode::Control,
        Some(definition()?),
        true,
        None,
    )
}
fn reopen(memory: Memory, mode: DigMode, recovery: Recovery) -> Result<BatchStore<Memory>> {
    BatchStore::open(memory, &query_context()?, mode, None, false, Some(recovery))
}

#[test]
fn exact_parent_replays_and_query_only_stop_is_permanent_and_idempotent() -> Result<()> {
    let memory = Memory::default();
    let owner = create(memory.clone())?;
    let initial = owner.recovery();
    assert_eq!(initial, initial_recovery(&definition()?)?);
    assert_eq!(owner.definition().journal_id(), definition()?.journal_id());
    assert!(!owner.stopped());
    let original = memory.disk.borrow().bytes.clone();
    assert_eq!(memory.disk.borrow().syncs, 1);
    drop(owner);
    let mut recovered = reopen(memory.clone(), DigMode::Recover, initial)?;
    assert_eq!(memory.disk.borrow().syncs, 2);
    recovered.stop(&query_context()?)?;
    recovered.verify(&query_context()?)?;
    let stopped = recovered.recovery();
    assert_ne!(stopped.head, initial.head);
    assert_eq!(stopped.blueprint_id, initial.blueprint_id);
    assert!(recovered.durable_stopped());
    assert_eq!(&memory.disk.borrow().bytes[..original.len()], original);
    assert_eq!(
        memory.disk.borrow().bytes.len(),
        original.len() + STOP_BYTES
    );
    assert_eq!(memory.disk.borrow().syncs, 3);
    recovered.stop(&query_context()?)?;
    assert_eq!(memory.disk.borrow().syncs, 3);
    drop(recovered);
    let mut offline = reopen(memory.clone(), DigMode::Offline, stopped)?;
    offline.verify(&query_context()?)?;
    assert!(offline.stopped());
    assert!(offline.stop(&query_context()?).is_err());
    assert_eq!(memory.disk.borrow().syncs, 3);
    assert_eq!(memory.disk.borrow().writes, 2);
    Ok(())
}

#[test]
fn creation_requires_exact_control_definition_and_never_reinitializes_existing_bytes() -> Result<()>
{
    let pin = initial_recovery(&definition()?)?;
    for mode in [DigMode::Recover, DigMode::Offline] {
        let memory = Memory::default();
        assert!(
            BatchStore::open(
                memory.clone(),
                &context()?,
                mode,
                Some(definition()?),
                true,
                None
            )
            .is_err()
        );
        assert_eq!(memory.disk.borrow().writes, 0);
    }
    for (c, def, recovery) in [
        (query_context()?, Some(definition()?), None),
        (context()?, None, None),
        (context()?, Some(definition()?), Some(pin)),
    ] {
        let memory = Memory::default();
        assert!(
            BatchStore::open(memory.clone(), &c, DigMode::Control, def, true, recovery).is_err()
        );
        assert_eq!(memory.disk.borrow().writes, 0);
    }
    let empty = Memory::default();
    assert!(reopen(empty.clone(), DigMode::Control, pin).is_err());
    assert!(empty.disk.borrow().bytes.is_empty());
    let existing = Memory::with_bytes(vec![1]);
    assert!(create(existing.clone()).is_err());
    assert_eq!(existing.disk.borrow().bytes, [1]);
    let memory = Memory::default();
    drop(create(memory.clone())?);
    let before = memory.disk.borrow().bytes.clone();
    assert!(create(memory.clone()).is_err());
    assert_eq!(memory.disk.borrow().bytes, before);
    Ok(())
}

#[test]
fn recovery_requires_original_identity_and_head_before_sync_or_writes() -> Result<()> {
    let memory = Memory::default();
    let owner = create(memory.clone())?;
    let pin = owner.recovery();
    let before = memory.disk.borrow().bytes.clone();
    drop(owner);
    for pin in [
        None,
        Some(Recovery {
            blueprint_id: Digest32::ZERO,
            ..pin
        }),
        Some(Recovery {
            head: Digest32::ZERO,
            ..pin
        }),
        Some(Recovery {
            blueprint_id: Digest32::of_bytes(b"another blueprint"),
            ..pin
        }),
        Some(Recovery {
            head: Digest32::of_bytes(b"another custody head"),
            ..pin
        }),
    ] {
        assert!(
            BatchStore::open(
                memory.clone(),
                &context()?,
                DigMode::Control,
                None,
                false,
                pin
            )
            .is_err()
        );
    }
    let original = definition()?;
    for different in [
        BatchDefinition::new(
            original.layout().clone(),
            original.binding().clone(),
            Digest32::of_bytes(b"other journal"),
            capture()?,
        )?,
        BatchDefinition::new(
            original.layout().clone(),
            binding(5001)?,
            original.journal_id(),
            capture()?,
        )?,
    ] {
        assert!(
            BatchStore::open(
                memory.clone(),
                &context()?,
                DigMode::Control,
                Some(different),
                false,
                Some(pin)
            )
            .is_err()
        );
    }
    assert_eq!(memory.disk.borrow().bytes, before);
    assert_eq!(memory.disk.borrow().writes, 1);
    assert_eq!(memory.disk.borrow().syncs, 1);
    Ok(())
}

#[test]
fn newer_stop_can_recover_lost_reply_but_acknowledged_stop_cannot_roll_back() -> Result<()> {
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let initial = owner.recovery();
    let original = memory.disk.borrow().bytes.clone();
    owner.stop(&query_context()?)?;
    let stopped = owner.recovery();
    drop(owner);
    let recovered = reopen(memory.clone(), DigMode::Offline, initial)?;
    assert!(recovered.stopped());
    assert_eq!(recovered.recovery(), stopped);
    drop(recovered);
    let rollback = Memory::with_bytes(original);
    assert!(reopen(rollback.clone(), DigMode::Control, stopped).is_err());
    assert_eq!(rollback.disk.borrow().writes, 0);
    assert_eq!(rollback.disk.borrow().syncs, 0);
    Ok(())
}

#[test]
fn every_torn_prefix_byte_corruption_and_repeated_stop_refuse_without_repair() -> Result<()> {
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let initial = owner.recovery();
    let header_len = memory.disk.borrow().bytes.len();
    owner.stop(&query_context()?)?;
    let stopped = owner.recovery();
    let complete = memory.disk.borrow().bytes.clone();
    for cut in 0..complete.len() {
        let raw = complete[..cut].to_vec();
        let damaged = Memory::with_bytes(raw.clone());
        assert!(
            reopen(damaged.clone(), DigMode::Offline, stopped).is_err(),
            "prefix {cut}"
        );
        assert_eq!(damaged.disk.borrow().bytes, raw);
    }
    // The one fully formed older header is accepted only with the older pin.
    assert!(
        !reopen(
            Memory::with_bytes(complete[..header_len].to_vec()),
            DigMode::Offline,
            initial
        )?
        .stopped()
    );
    for offset in 0..complete.len() {
        let mut raw = complete.clone();
        raw[offset] ^= 1;
        assert!(
            reopen(Memory::with_bytes(raw), DigMode::Offline, stopped).is_err(),
            "corruption {offset}"
        );
    }
    let mut repeated = complete.clone();
    repeated.extend_from_slice(&complete[header_len..]);
    assert!(reopen(Memory::with_bytes(repeated), DigMode::Offline, stopped).is_err());
    Ok(())
}

#[test]
fn creation_and_stop_sync_failures_keep_exact_recovery_without_acknowledgement() -> Result<()> {
    let initial = Memory::default();
    let pin = initial_recovery(&definition()?)?;
    initial.disk.borrow_mut().fail_sync = Some(1);
    assert!(create(initial.clone()).is_err());
    assert!(!initial.disk.borrow().bytes.is_empty());
    let offline = reopen(initial.clone(), DigMode::Offline, pin)?;
    assert!(!offline.stopped());
    assert_eq!(initial.disk.borrow().syncs, 1);
    drop(offline);
    drop(reopen(initial.clone(), DigMode::Recover, pin)?);
    assert_eq!(initial.disk.borrow().syncs, 2);
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let initial = owner.recovery();
    memory.disk.borrow_mut().fail_sync = Some(2);
    assert!(owner.stop(&query_context()?).is_err());
    assert!(owner.is_fenced());
    assert!(owner.stopped());
    assert!(!owner.durable_stopped());
    assert_eq!(owner.recovery(), initial);
    assert!(owner.verify(&query_context()?).is_err());
    assert!(owner.stop(&query_context()?).is_err());
    drop(owner);
    let mut recovered = reopen(memory.clone(), DigMode::Recover, initial)?;
    assert_eq!(memory.disk.borrow().syncs, 3);
    assert!(recovered.durable_stopped());
    recovered.verify(&query_context()?)?;
    assert_ne!(recovered.recovery(), initial);
    Ok(())
}

#[test]
fn partial_writes_flush_failures_and_post_sync_substitution_fence_without_repair() -> Result<()> {
    let initial = Memory::default();
    initial.disk.borrow_mut().partial = Some(17);
    assert!(create(initial.clone()).is_err());
    assert_eq!(initial.disk.borrow().bytes.len(), 17);
    assert!(reopen(initial, DigMode::Offline, initial_recovery(&definition()?)?).is_err());
    for fault in 0..4 {
        let memory = Memory::default();
        let mut owner = create(memory.clone())?;
        {
            let mut disk = memory.disk.borrow_mut();
            match fault {
                0 => disk.partial = Some(17),
                1 => disk.fail_flush = true,
                2 => disk.invalidate_on_sync = true,
                _ => disk.mutate_on_sync = true,
            }
        }
        assert!(owner.stop(&query_context()?).is_err(), "fault {fault}");
        assert!(owner.stopped());
        assert!(owner.is_fenced());
        assert!(owner.verify(&query_context()?).is_err());
        let bytes = memory.disk.borrow().bytes.clone();
        assert!(owner.stop(&query_context()?).is_err());
        assert_eq!(memory.disk.borrow().bytes, bytes);
    }
    Ok(())
}

#[test]
fn removed_stop_changed_extent_and_same_length_replacement_fence_current_owner() -> Result<()> {
    for fault in 0..4 {
        let memory = Memory::default();
        let mut owner = create(memory.clone())?;
        if fault == 0 {
            owner.stop(&query_context()?)?;
        }
        {
            let mut disk = memory.disk.borrow_mut();
            match fault {
                0 => {
                    let end = disk.bytes.len() - STOP_BYTES;
                    disk.bytes.truncate(end);
                }
                1 => disk.bytes[12] ^= 1,
                2 => disk.bytes.push(0),
                _ => disk.invalid = true,
            }
        }
        assert!(owner.verify(&query_context()?).is_err());
        assert!(owner.stopped());
        assert!(owner.is_fenced());
        assert!(owner.stop(&query_context()?).is_err());
    }
    Ok(())
}

#[test]
fn fresh_authority_session_scope_expiry_and_cancellation_gate_cached_custody() -> Result<()> {
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let before = memory.disk.borrow().bytes.clone();
    let base = query_context()?;
    let mut variants = Vec::new();
    let mut denied = base.clone();
    denied.grants.clear();
    variants.push(denied);
    let mut denied = base.clone();
    denied.session_id = SessionId::new(2);
    variants.push(denied);
    let mut denied = base.clone();
    denied.anchor.fortress_id = FortressId::new(987);
    variants.push(denied);
    let mut denied = base.clone();
    denied.cancellation_requested = true;
    variants.push(denied);
    let mut denied = base.clone();
    denied.grants[0].remaining_uses = Some(1);
    variants.push(denied);
    let mut denied = base.clone();
    denied.grants[0].scope.entity_ids.insert(EntityId::new(1));
    variants.push(denied);
    let mut denied = base.clone();
    denied.anchor.tick = GameTick(base.anchor.tick.get() + 1);
    denied.grants[0].expires_at_tick = Some(base.anchor.tick);
    variants.push(denied);
    for c in variants {
        assert!(owner.verify(&c).is_err());
        assert!(owner.stop(&c).is_err());
        assert_eq!(memory.disk.borrow().bytes, before);
    }
    owner.verify(&base)?;
    owner.stop(&base)?;
    Ok(())
}

#[test]
fn scoped_authority_and_retained_bootstrap_floor_cannot_be_bypassed_by_reopen() -> Result<()> {
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let pin = owner.recovery();
    let before = memory.disk.borrow().bytes.clone();
    let mut narrow = context()?;
    for grant in &mut narrow.grants {
        grant.scope.map_area = Some(MapCuboid::new(
            MapCoord::new(0, 0, 0),
            MapCoord::new(15, 15, 7),
        )?);
    }
    assert!(owner.verify(&narrow).is_err());
    assert!(owner.stop(&narrow).is_err());
    assert!(
        BatchStore::open(
            memory.clone(),
            &narrow,
            DigMode::Recover,
            None,
            false,
            Some(pin)
        )
        .is_err()
    );
    for capability in [Capability::Query, Capability::Plan] {
        let mut stale = context()?;
        stale.anchor.tick = GameTick(1);
        for grant in &mut stale.grants {
            if grant.capability == capability {
                grant.expires_at_tick = Some(GameTick(100));
            }
        }
        let blank = Memory::default();
        assert!(
            BatchStore::open(
                blank.clone(),
                &stale,
                DigMode::Control,
                Some(definition()?),
                true,
                None
            )
            .is_err()
        );
        assert!(blank.disk.borrow().bytes.is_empty());
        if capability == Capability::Query {
            assert!(owner.verify(&stale).is_err());
            assert!(owner.stop(&stale).is_err());
            assert!(
                BatchStore::open(
                    memory.clone(),
                    &stale,
                    DigMode::Recover,
                    None,
                    false,
                    Some(pin)
                )
                .is_err()
            );
        }
    }
    assert_eq!(memory.disk.borrow().bytes, before);
    assert_eq!(memory.disk.borrow().syncs, 1);
    owner.verify(&query_context()?)?;
    owner.stop(&query_context()?)?;
    Ok(())
}

#[test]
fn size_deadline_and_complete_stop_work_are_bounded_before_mutation() -> Result<()> {
    let pin = initial_recovery(&definition()?)?;
    assert!(
        reopen(
            Memory::with_bytes(vec![0; MAX_STORE_BYTES + 1]),
            DigMode::Offline,
            pin
        )
        .is_err()
    );
    let mut small = context()?;
    small.budget.max_bytes = 1;
    let memory = Memory::default();
    assert!(
        BatchStore::open(
            memory.clone(),
            &small,
            DigMode::Control,
            Some(definition()?),
            true,
            None
        )
        .is_err()
    );
    assert!(memory.disk.borrow().bytes.is_empty());
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let before = memory.disk.borrow().bytes.clone();
    let pin = owner.recovery();
    let mut small = query_context()?;
    small.budget.max_bytes = before.len() as u64 + 1;
    assert_eq!(
        owner.stop(&small).err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(memory.disk.borrow().bytes, before);
    assert!(owner.is_fenced());
    assert!(!owner.durable_stopped());
    drop(owner);
    let mut owner = reopen(memory.clone(), DigMode::Recover, pin)?;
    assert!(!owner.stopped());
    let mut late = query_context()?;
    late.budget.max_wall_millis = 1;
    memory.disk.borrow_mut().read_delay = true;
    assert_eq!(
        owner.verify(&late).err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    memory.disk.borrow_mut().read_delay = false;
    owner.verify(&query_context()?)?;
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
mod private;
