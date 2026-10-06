use super::*;
use crate::build_placement::{BuildBinding, BuildCapture};
use crate::furniture_batch::FurniturePlan;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, EntityId, FortressId, GameTick, ObservationCursor, RequestId,
    StateAnchor, WorkBudget,
};
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::net::SocketAddr;
use std::rc::Rc;

fn capture() -> Result<BuildCapture> {
    let text =
        include_str!("../../../../../bridge/common/tests/fixtures/build_placement_v1_19.json");
    let needle = "\"capture\": \"";
    let start = text.find(needle).ok_or_else(corrupt)? + needle.len();
    let encoded = text[start..].split('"').next().ok_or_else(corrupt)?;
    let bytes = encoded
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let digits = std::str::from_utf8(pair).map_err(|_| corrupt())?;
            u8::from_str_radix(digits, 16).map_err(|_| corrupt())
        })
        .collect::<Result<Vec<_>>>()?;
    BuildCapture::decode(&bytes)
}
fn definition() -> Result<BatchDefinition> {
    let plan = FurniturePlan::decode(br#"{"schema":"dfmcp.furniture-plan/1","steps":[{"name":"bed","kind":"bed","item":42,"target":[15,15,2]},{"name":"chair","kind":"chair","item":43,"target":[18,15,2],"after":["bed"]}]}"#)?;
    let binding = BuildBinding::new(
        SocketAddr::from(([127, 0, 0, 1], 5000)),
        "df",
        "dfhack",
        &capture()?,
    )?;
    BatchDefinition::new(
        plan,
        binding,
        Digest32::of_bytes(b"original placement journal"),
    )
}
fn context() -> Result<OperationContext> {
    let capture = capture()?;
    let fortress_id = capture.fortress_id();
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(capture.tick()),
            state_hash: capture.witness(),
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
    let mut context = context()?;
    context
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    Ok(context)
}
fn io_failure() -> io::Error {
    io::Error::other("injected batch storage failure")
}
#[derive(Default)]
struct Disk {
    bytes: Vec<u8>,
    writes: usize,
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
            // Deterministically cross a one-millisecond cooperative deadline.
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
        if self.disk.borrow().fail_flush {
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
        BuildMode::Control,
        Some(definition()?),
        true,
    )
}

#[test]
fn immutable_parent_replays_and_query_only_stop_is_permanent_and_idempotent() -> Result<()> {
    let memory = Memory::default();
    let owner = create(memory.clone())?;
    assert_eq!(owner.definition().id(), definition()?.id());
    assert_eq!(owner.definition().journal_id(), definition()?.journal_id());
    assert!(!owner.stopped());
    assert_eq!(memory.disk.borrow().syncs, 1);
    let original = memory.disk.borrow().bytes.clone();
    drop(owner);
    let mut recovered = BatchStore::open(
        memory.clone(),
        &query_context()?,
        BuildMode::Recover,
        None,
        false,
    )?;
    recovered.stop(&query_context()?)?;
    recovered.verify(&query_context()?)?;
    assert!(recovered.stopped());
    assert_eq!(&memory.disk.borrow().bytes[..original.len()], original);
    assert_eq!(
        memory.disk.borrow().bytes.len(),
        original.len() + STOP_BYTES
    );
    assert_eq!(memory.disk.borrow().syncs, 2);
    recovered.stop(&query_context()?)?;
    assert_eq!(memory.disk.borrow().syncs, 2);
    drop(recovered);
    let mut offline = BatchStore::open(
        memory.clone(),
        &query_context()?,
        BuildMode::Offline,
        None,
        false,
    )?;
    offline.verify(&query_context()?)?;
    assert!(offline.stopped());
    assert!(offline.stop(&query_context()?).is_err());
    assert_eq!(memory.disk.borrow().syncs, 2);
    Ok(())
}

#[test]
fn creation_needs_query_plan_control_exact_definition_and_empty_storage() -> Result<()> {
    for mode in [BuildMode::Recover, BuildMode::Offline] {
        let memory = Memory::default();
        assert!(
            BatchStore::open(memory.clone(), &context()?, mode, Some(definition()?), true).is_err()
        );
        assert_eq!(memory.disk.borrow().writes, 0);
    }
    for (context, expected) in [(query_context()?, Some(definition()?)), (context()?, None)] {
        let memory = Memory::default();
        assert!(
            BatchStore::open(memory.clone(), &context, BuildMode::Control, expected, true).is_err()
        );
        assert_eq!(memory.disk.borrow().writes, 0);
    }
    let memory = Memory::default();
    assert!(
        BatchStore::open(
            memory.clone(),
            &context()?,
            BuildMode::Control,
            Some(definition()?),
            false
        )
        .is_err()
    );
    assert!(memory.disk.borrow().bytes.is_empty());
    let existing = Memory::with_bytes(vec![1]);
    assert!(create(existing.clone()).is_err());
    assert_eq!(existing.disk.borrow().bytes, [1]);
    Ok(())
}

#[test]
fn fresh_authority_owner_scope_and_cancellation_apply_to_cached_store() -> Result<()> {
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
    for context in variants {
        assert!(owner.verify(&context).is_err());
        assert!(owner.stop(&context).is_err());
        assert_eq!(memory.disk.borrow().bytes, before);
    }
    owner.verify(&base)?;
    owner.stop(&base)?;
    Ok(())
}

#[test]
fn definition_journal_and_source_substitution_are_refused_without_writes() -> Result<()> {
    let memory = Memory::default();
    drop(create(memory.clone())?);
    let original = definition()?;
    let different_journal = BatchDefinition::new(
        original.plan().clone(),
        original.binding().clone(),
        Digest32::of_bytes(b"other journal"),
    )?;
    let different_source = BatchDefinition::new(
        original.plan().clone(),
        BuildBinding::new(
            SocketAddr::from(([127, 0, 0, 1], 5001)),
            "df",
            "dfhack",
            &capture()?,
        )?,
        original.journal_id(),
    )?;
    for expected in [different_journal, different_source] {
        assert!(
            BatchStore::open(
                memory.clone(),
                &context()?,
                BuildMode::Control,
                Some(expected),
                false
            )
            .is_err()
        );
    }
    assert_eq!(memory.disk.borrow().writes, 1);
    Ok(())
}

#[test]
fn every_torn_header_and_stop_and_repeated_stop_are_rejected() -> Result<()> {
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let original = memory.disk.borrow().bytes.clone();
    for cut in 0..original.len() {
        assert!(
            BatchStore::open(
                Memory::with_bytes(original[..cut].to_vec()),
                &query_context()?,
                BuildMode::Offline,
                None,
                false
            )
            .is_err(),
            "header cut {cut}"
        );
    }
    owner.stop(&query_context()?)?;
    let complete = memory.disk.borrow().bytes.clone();
    for cut in original.len() + 1..complete.len() {
        assert!(
            BatchStore::open(
                Memory::with_bytes(complete[..cut].to_vec()),
                &query_context()?,
                BuildMode::Offline,
                None,
                false
            )
            .is_err(),
            "stop cut {cut}"
        );
    }
    let mut repeated = complete.clone();
    repeated.extend_from_slice(&complete[original.len()..]);
    assert!(
        BatchStore::open(
            Memory::with_bytes(repeated),
            &query_context()?,
            BuildMode::Offline,
            None,
            false
        )
        .is_err()
    );
    for offset in [
        0,
        8,
        12,
        original.len() - 9,
        original.len() - 1,
        original.len(),
        complete.len() - 1,
    ] {
        let mut mutated = complete.clone();
        mutated[offset] ^= 1;
        assert!(
            BatchStore::open(
                Memory::with_bytes(mutated),
                &query_context()?,
                BuildMode::Offline,
                None,
                false
            )
            .is_err(),
            "mutation {offset}"
        );
    }
    Ok(())
}

#[test]
fn sync_failure_does_not_acknowledge_creation_or_stop_and_reopen_verifies_actual_bytes()
-> Result<()> {
    let initial = Memory::default();
    initial.disk.borrow_mut().fail_sync = Some(1);
    assert!(create(initial.clone()).is_err());
    assert!(!initial.disk.borrow().bytes.is_empty());
    let reopened = BatchStore::open(
        initial.clone(),
        &query_context()?,
        BuildMode::Offline,
        None,
        false,
    )?;
    assert!(!reopened.stopped());
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    memory.disk.borrow_mut().fail_sync = Some(2);
    assert!(owner.stop(&query_context()?).is_err());
    assert!(owner.stopped());
    assert!(!owner.durable_stopped());
    assert!(owner.is_fenced());
    assert!(owner.verify(&query_context()?).is_err());
    assert!(owner.stop(&query_context()?).is_err());
    assert_eq!(memory.disk.borrow().syncs, 2);
    let mut reopened = BatchStore::open(
        memory.clone(),
        &query_context()?,
        BuildMode::Offline,
        None,
        false,
    )?;
    reopened.verify(&query_context()?)?;
    assert!(reopened.stopped());
    assert_eq!(memory.disk.borrow().syncs, 2);
    Ok(())
}

#[test]
fn partial_writes_flush_failure_and_post_sync_changes_fence_without_repair() -> Result<()> {
    let initial = Memory::default();
    initial.disk.borrow_mut().partial = Some(17);
    assert!(create(initial.clone()).is_err());
    assert_eq!(initial.disk.borrow().bytes.len(), 17);
    assert!(BatchStore::open(initial, &query_context()?, BuildMode::Offline, None, false).is_err());
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
        assert!(owner.verify(&query_context()?).is_err());
        let bytes = memory.disk.borrow().bytes.clone();
        assert!(owner.stop(&query_context()?).is_err());
        assert_eq!(memory.disk.borrow().bytes, bytes);
    }
    Ok(())
}

#[test]
fn removed_stop_or_same_length_replacement_fences_the_retained_owner() -> Result<()> {
    for stopped in [false, true] {
        let memory = Memory::default();
        let mut owner = create(memory.clone())?;
        if stopped {
            owner.stop(&query_context()?)?;
        }
        {
            let mut disk = memory.disk.borrow_mut();
            if stopped {
                let end = disk.bytes.len() - STOP_BYTES;
                disk.bytes.truncate(end);
            } else {
                disk.bytes[12] ^= 1;
            }
        }
        assert!(owner.verify(&query_context()?).is_err());
        assert!(owner.stopped());
        assert!(owner.stop(&query_context()?).is_err());
    }
    Ok(())
}

#[test]
fn storage_size_deadline_and_whole_stop_reservation_are_bounded() -> Result<()> {
    let over = Memory::with_bytes(vec![0; MAX_STORE_BYTES + 1]);
    assert!(BatchStore::open(over, &query_context()?, BuildMode::Offline, None, false).is_err());
    let memory = Memory::default();
    let mut owner = create(memory.clone())?;
    let before = memory.disk.borrow().bytes.clone();
    let mut small = query_context()?;
    small.budget.max_bytes = before.len() as u64 + 1;
    assert_eq!(
        owner.stop(&small).err().map(|e| e.code),
        Some(ErrorCode::BudgetExceeded)
    );
    assert_eq!(memory.disk.borrow().bytes, before);
    assert!(owner.stopped());
    assert!(owner.is_fenced());
    assert!(!owner.durable_stopped());
    let mut owner = BatchStore::open(
        memory.clone(),
        &query_context()?,
        BuildMode::Recover,
        None,
        false,
    )?;
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
