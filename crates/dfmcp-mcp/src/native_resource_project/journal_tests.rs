use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, ObservationCursor, RequestId, SessionId,
    StateAnchor, WorkBudget,
};
use serde_json::json;
use std::cell::RefCell;
use std::io::{self, Read, Seek, Write};
use std::rc::Rc;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(Default)]
struct MemoryState {
    bytes: Vec<u8>,
    write_left: Option<usize>,
    fail_sync: bool,
    fail_flush: bool,
    invalid_identity: bool,
    reads: usize,
    writes: usize,
    syncs: usize,
    truncates: usize,
}
#[derive(Clone, Default)]
struct Memory {
    state: Rc<RefCell<MemoryState>>,
    offset: usize,
}
impl Memory {
    fn raw(&self) -> Vec<u8> {
        self.state.borrow().bytes.clone()
    }
    fn replace(&self, bytes: Vec<u8>) {
        self.state.borrow_mut().bytes = bytes;
    }
}
fn fault() -> io::Error {
    io::Error::other("injected storage fault")
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        state.reads += 1;
        let length = out.len().min(state.bytes.len().saturating_sub(self.offset));
        out[..length].copy_from_slice(&state.bytes[self.offset..self.offset + length]);
        self.offset += length;
        Ok(length)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self.state.borrow_mut();
        state.writes += 1;
        let length = match state.write_left {
            Some(0) => return Err(fault()),
            Some(left) => {
                let length = left.min(bytes.len());
                state.write_left = Some(left - length);
                length
            }
            None => bytes.len(),
        };
        if self.offset + length > state.bytes.len() {
            state.bytes.resize(self.offset + length, 0);
        }
        state.bytes[self.offset..self.offset + length].copy_from_slice(&bytes[..length]);
        self.offset += length;
        Ok(length)
    }
    fn flush(&mut self) -> io::Result<()> {
        if self.state.borrow().fail_flush { Err(fault()) } else { Ok(()) }
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let next = match position {
            SeekFrom::Start(value) => i128::from(value),
            SeekFrom::End(value) => self.state.borrow().bytes.len() as i128 + i128::from(value),
            SeekFrom::Current(value) => self.offset as i128 + i128::from(value),
        };
        self.offset = usize::try_from(next).map_err(|_| fault())?;
        Ok(self.offset as u64)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        let mut state = self.state.borrow_mut();
        state.syncs += 1;
        if state.fail_sync { Err(fault()) } else { Ok(()) }
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        let mut state = self.state.borrow_mut();
        state.truncates += 1;
        state.bytes.truncate(usize::try_from(length).map_err(|_| fault())?);
        Ok(())
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.state.borrow().invalid_identity { Err(fault()) } else { Ok(()) }
    }
}

fn context() -> OperationContext {
    let fortress = FortressId::new(91);
    OperationContext {
        session_id: SessionId::new(7001),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: fortress,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(100),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_bytes: WORK_BYTES,
            max_wall_millis: 60_000,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        },
        grants: [Capability::Query, Capability::Plan]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::Guarded,
                expires_at_tick: Some(GameTick(1200)),
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    }
}
fn original() -> Value {
    json!({"goal": {"recipe": "wooden_bed", "minimum_stack_units": 2, "deadline_tick": 1000},
        "workshop_policy": "original-workshop-policy", "production_policy": "original-production-policy"})
}
fn workshop() -> Checkpoint {
    Checkpoint {
        workshop: Some(json!({"acknowledged": "original-workshop-ack", "key": "original-workshop", "high_tick": 110})),
        ..Checkpoint::default()
    }
}
fn handoff() -> Checkpoint {
    let mut checkpoint = workshop();
    checkpoint.handoff = Some(json!({"original_key": "original-workshop", "original_digest": "original-digest", "acknowledged": "original-workshop-ack", "high_tick": 120}));
    checkpoint
}
fn production() -> Checkpoint {
    let mut checkpoint = handoff();
    checkpoint.production = Some(json!({"journal_id": "original-production", "head": "original-production-head", "authority_tick_floor": 130}));
    checkpoint
}
fn seeded() -> TestResult<(Journal<Memory, Memory>, Memory, Memory)> {
    let storage = Memory::default();
    let acknowledgements = Memory::default();
    let journal = Journal::open(
        storage.clone(),
        acknowledgements.clone(),
        &context(),
        Mode::Control,
        true,
        original(),
        None,
    )?;
    Ok((journal, storage, acknowledgements))
}
fn reopen(
    storage: &Memory,
    acknowledgements: &Memory,
    pin: Pin,
    mode: Mode,
) -> Result<Journal<Memory, Memory>> {
    let mut c = context();
    c.session_id = SessionId::new(7002);
    Journal::open(
        storage.clone(),
        acknowledgements.clone(),
        &c,
        mode,
        false,
        original(),
        Some(pin),
    )
}

#[test]
fn complete_stage_history_reopens_original_without_initialization_authority() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let c = context();
    let first = journal.pin();
    assert_eq!(journal.snapshot().stage, Stage::WorkshopOpening);
    journal.preflight_append(&c)?;
    journal.advance(&c, Stage::Workshop, workshop())?;
    journal.advance(&c, Stage::Transition, handoff())?;
    journal.advance(&c, Stage::ProductionOpening, handoff())?;
    journal.advance(&c, Stage::Production, production())?;
    assert_eq!(journal.pin().frames, 5);
    let retained = journal.snapshot().clone();
    let bytes = (storage.raw(), acknowledgements.raw());
    drop(journal);
    let mut recovered = reopen(&storage, &acknowledgements, first.clone(), Mode::Offline)?;
    assert_eq!(recovered.original(), &original());
    assert_eq!(recovered.snapshot(), &retained);
    assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
    let mut new_context = c;
    new_context.session_id = SessionId::new(7002);
    assert!(
        recovered
            .advance(&new_context, Stage::WorkshopOpening, Checkpoint::default())
            .is_err()
    );
    assert!(
        Journal::open(
            storage,
            acknowledgements,
            &context(),
            Mode::Control,
            true,
            original(),
            None,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn cannot_skip_rewind_replace_handoff_or_open_a_new_stage_in_recover_mode() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let c = context();
    assert!(journal.advance(&c, Stage::Production, production()).is_err());
    journal.advance(&c, Stage::Workshop, workshop())?;
    assert!(
        journal
            .advance(&c, Stage::WorkshopOpening, Checkpoint::default())
            .is_err()
    );
    let pin = journal.pin();
    drop(journal);
    let mut recovered = reopen(&storage, &acknowledgements, pin, Mode::Recover)?;
    let mut c = c;
    c.session_id = SessionId::new(7002);
    assert!(recovered.advance(&c, Stage::Transition, handoff()).is_err());
    recovered.advance(&c, Stage::Workshop, workshop())?;
    drop(recovered);
    let latest = replay(&storage.raw(), &Work::new(&c)?)?;
    let head = latest.heads.last().ok_or("missing head")?;
    let mut journal = Journal::open(
        storage.clone(),
        acknowledgements,
        &c,
        Mode::Control,
        false,
        original(),
        Some(Pin {
            journal_id: latest.journal_id.to_hex(),
            head: head.digest.to_hex(),
            frames: latest.heads.len() as u32,
            authority_tick_floor: head.tick,
        }),
    )?;
    journal.advance(&c, Stage::Transition, handoff())?;
    let mut forged = handoff();
    forged.handoff = Some(json!({"original_key": "replacement"}));
    assert!(
        journal
            .advance(&c, Stage::ProductionOpening, forged)
            .is_err()
    );
    let mut changed_donor = handoff();
    changed_donor.workshop = Some(json!({"acknowledged": "replacement"}));
    assert!(
        journal
            .advance(&c, Stage::ProductionOpening, changed_donor)
            .is_err()
    );
    journal.advance(&c, Stage::ProductionOpening, handoff())?;
    Ok(())
}

#[test]
fn cancellation_keeps_original_phase_and_allows_only_that_childs_drain() -> TestResult {
    for phase in [
        Stage::WorkshopOpening,
        Stage::Workshop,
        Stage::Transition,
        Stage::ProductionOpening,
        Stage::Production,
    ] {
        let (mut journal, storage, acknowledgements) = seeded()?;
        let c = context();
        if phase != Stage::WorkshopOpening {
            journal.advance(&c, Stage::Workshop, workshop())?;
        }
        if matches!(phase, Stage::Transition | Stage::ProductionOpening | Stage::Production) {
            journal.advance(&c, Stage::Transition, handoff())?;
        }
        if matches!(phase, Stage::ProductionOpening | Stage::Production) {
            journal.advance(&c, Stage::ProductionOpening, handoff())?;
        }
        if phase == Stage::Production {
            journal.advance(&c, Stage::Production, production())?;
        }
        let before = journal.snapshot().checkpoint.clone();
        journal.cancel(&c)?;
        assert_eq!(journal.snapshot().cancelled_from, Some(phase));
        assert_eq!(journal.snapshot().checkpoint, before);
        assert!(
            journal
                .advance(&c, Stage::WorkshopOpening, Checkpoint::default())
                .is_err()
        );
        assert!(journal.advance(&c, Stage::Production, production()).is_err());
        let mut drained = before.clone();
        if phase == Stage::Workshop {
            drained.workshop = Some(json!({"key": "original-workshop", "retired": true}));
        }
        if phase == Stage::Production {
            drained.production = Some(json!({"journal_id": "original-production", "drain_requested": true}));
        }
        journal.advance(&c, Stage::Cancelled, drained.clone())?;
        let pin = journal.pin();
        drop(journal);
        let recovered = reopen(&storage, &acknowledgements, pin, Mode::Offline)?;
        assert_eq!(recovered.snapshot().stage, Stage::Cancelled);
        assert_eq!(recovered.snapshot().cancelled_from, Some(phase));
        assert_eq!(recovered.snapshot().checkpoint, drained);
    }
    Ok(())
}

#[test]
fn independent_ack_rejects_valid_prefix_rollback_even_with_older_external_pin() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let initial = storage.raw();
    let old_pin = journal.pin();
    journal.advance(&context(), Stage::Workshop, workshop())?;
    drop(journal);
    storage.replace(initial);
    assert!(reopen(&storage, &acknowledgements, old_pin, Mode::Recover).is_err());
    assert_eq!(storage.state.borrow().truncates, 0);
    Ok(())
}

#[test]
fn acknowledged_external_head_rejects_rollback_of_both_local_stores() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let initial = (storage.raw(), acknowledgements.raw());
    journal.advance(&context(), Stage::Workshop, workshop())?;
    let pin = journal.pin();
    drop(journal);
    storage.replace(initial.0);
    acknowledgements.replace(initial.1);
    assert!(reopen(&storage, &acknowledgements, pin, Mode::Recover).is_err());
    Ok(())
}

#[test]
fn failed_journal_sync_fences_owner_and_surviving_opening_never_becomes_retry_permission() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    journal.advance(&context(), Stage::Workshop, workshop())?;
    journal.advance(&context(), Stage::Transition, handoff())?;
    let pin = journal.pin();
    let prior_ack = acknowledgements.raw();
    storage.state.borrow_mut().fail_sync = true;
    assert!(
        journal
            .advance(&context(), Stage::ProductionOpening, handoff())
            .is_err()
    );
    assert!(journal.is_fenced());
    assert_eq!(journal.pin(), pin);
    assert_eq!(acknowledgements.raw(), prior_ack);
    assert!(
        journal
            .advance(&context(), Stage::ProductionOpening, handoff())
            .is_err()
    );
    storage.state.borrow_mut().fail_sync = false;
    drop(journal);
    assert!(reopen(&storage, &acknowledgements, pin.clone(), Mode::Offline).is_err());
    let recovered = reopen(&storage, &acknowledgements, pin, Mode::Recover)?;
    assert_eq!(recovered.snapshot().stage, Stage::ProductionOpening);
    assert_eq!(recovered.snapshot().checkpoint.handoff, handoff().handoff);
    assert_eq!(recovered.snapshot().checkpoint.production, None);
    assert_eq!(storage.state.borrow().truncates, 0);
    assert_eq!(acknowledgements.state.borrow().truncates, 0);
    Ok(())
}

#[test]
fn every_torn_transition_prefix_is_refused_without_repair() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let base = storage.raw();
    let ack = acknowledgements.raw();
    let pin = journal.pin();
    journal.advance(&context(), Stage::Workshop, workshop())?;
    let complete = storage.raw();
    drop(journal);
    for length in base.len() + 1..complete.len() {
        storage.replace(complete[..length].to_vec());
        acknowledgements.replace(ack.clone());
        assert!(
            reopen(&storage, &acknowledgements, pin.clone(), Mode::Recover).is_err(),
            "accepted torn frame length {length}"
        );
        assert_eq!(storage.raw(), complete[..length]);
    }
    assert_eq!(storage.state.borrow().truncates, 0);
    Ok(())
}

#[test]
fn partial_journal_or_ack_write_remains_fenced_and_unrepaired() -> TestResult {
    for fail_ack in [false, true] {
        let (mut journal, storage, acknowledgements) = seeded()?;
        let pin = journal.pin();
        let selected = if fail_ack { &acknowledgements } else { &storage };
        selected.state.borrow_mut().write_left = Some(17);
        assert!(journal.advance(&context(), Stage::Workshop, workshop()).is_err());
        assert!(journal.is_fenced());
        let bytes = (storage.raw(), acknowledgements.raw());
        selected.state.borrow_mut().write_left = None;
        drop(journal);
        assert!(reopen(&storage, &acknowledgements, pin, Mode::Recover).is_err());
        assert_eq!((storage.raw(), acknowledgements.raw()), bytes);
        assert_eq!(
            storage.state.borrow().truncates + acknowledgements.state.borrow().truncates,
            0
        );
    }
    Ok(())
}

#[test]
fn failed_ack_sync_recovers_only_the_exact_complete_descendant() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let pin = journal.pin();
    acknowledgements.state.borrow_mut().fail_sync = true;
    assert!(journal.advance(&context(), Stage::Workshop, workshop()).is_err());
    assert!(journal.is_fenced());
    assert_eq!(journal.pin(), pin);
    acknowledgements.state.borrow_mut().fail_sync = false;
    drop(journal);
    let recovered = reopen(&storage, &acknowledgements, pin, Mode::Recover)?;
    assert_eq!(recovered.snapshot().stage, Stage::Workshop);
    assert_eq!(recovered.snapshot().checkpoint, workshop());
    assert_eq!(recovered.pin().frames, 2);
    Ok(())
}

#[test]
fn altered_original_goal_deadline_and_policy_refuse_before_recovery_publication() -> TestResult {
    let (journal, storage, acknowledgements) = seeded()?;
    let pin = journal.pin();
    drop(journal);
    for field in ["goal", "workshop_policy", "production_policy"] {
        let mut changed = original();
        changed[field] = json!({"deadline_tick": 9999, "replacement": true});
        let writes = (storage.state.borrow().writes, acknowledgements.state.borrow().writes);
        assert!(
            Journal::open(
                storage.clone(),
                acknowledgements.clone(),
                &context(),
                Mode::Recover,
                false,
                changed,
                Some(pin.clone()),
            )
            .is_err()
        );
        assert_eq!(
            (storage.state.borrow().writes, acknowledgements.state.borrow().writes),
            writes
        );
    }
    Ok(())
}

#[test]
fn checksummed_impossible_transition_is_not_authority() -> TestResult {
    let (journal, storage, acknowledgements) = seeded()?;
    let pin = journal.pin();
    let prior_ack_chain = journal.ack_chain;
    let impossible = Snapshot {
        stage: Stage::Production,
        checkpoint: production(),
        cancelled_from: None,
    };
    let mut raw = storage.raw();
    let appended = frame(parsed_digest(&pin.head)?, 2, &impossible, 100)?;
    raw.extend_from_slice(&appended);
    let head = Head {
        digest: Digest32::from_bytes(appended[appended.len() - 32..].try_into()?),
        tick: 100,
    };
    let mut ack = acknowledgements.raw();
    ack.extend_from_slice(&acknowledgement(
        parsed_digest(&pin.journal_id)?,
        head,
        2,
        prior_ack_chain,
    ));
    drop(journal);
    storage.replace(raw);
    acknowledgements.replace(ack);
    assert!(reopen(&storage, &acknowledgements, pin, Mode::Recover).is_err());
    Ok(())
}

#[test]
fn noncanonical_json_is_rejected_even_under_a_matching_frame_digest() -> TestResult {
    let (journal, storage, acknowledgements) = seeded()?;
    let pin = journal.pin();
    let mut raw = storage.raw();
    let body = serde_json::to_vec(&Body {
        high_tick: 100,
        snapshot: Snapshot {
            stage: Stage::Workshop,
            checkpoint: workshop(),
            cancelled_from: None,
        },
    })?;
    let mut noncanonical = b" ".to_vec();
    noncanonical.extend_from_slice(&body);
    let mut record = FRAME.to_vec();
    record.extend_from_slice(&(noncanonical.len() as u32).to_be_bytes());
    record.extend_from_slice(&2u32.to_be_bytes());
    record.extend_from_slice(parsed_digest(&pin.head)?.as_bytes());
    record.extend_from_slice(&noncanonical);
    let digest = hash(b"dfmcp-resource-project-frame/1", &record);
    record.extend_from_slice(digest.as_bytes());
    raw.extend_from_slice(&record);
    drop(journal);
    storage.replace(raw);
    assert!(reopen(&storage, &acknowledgements, pin, Mode::Recover).is_err());
    Ok(())
}

#[test]
fn bounded_shape_and_budget_fail_before_any_storage_access() -> TestResult {
    let storage = Memory::default();
    let acknowledgements = Memory::default();
    let mut deep = json!({});
    for _ in 0..18 {
        deep = json!({"next": deep});
    }
    let huge = json!({"values": vec![0; MAX_NODES + 1]});
    for original in [
        deep,
        huge,
        json!({"text": "x".repeat(16 * 1024 + 1)}),
        json!({"float": 1.25}),
    ] {
        assert!(
            Journal::open(
                storage.clone(),
                acknowledgements.clone(),
                &context(),
                Mode::Control,
                true,
                original,
                None,
            )
            .is_err()
        );
    }
    let mut c = context();
    c.budget.max_bytes = WORK_BYTES - 1;
    assert!(
        Journal::open(
            storage.clone(),
            acknowledgements.clone(),
            &c,
            Mode::Control,
            true,
            original(),
            None,
        )
        .is_err()
    );
    assert_eq!(storage.state.borrow().reads + storage.state.borrow().writes, 0);
    assert_eq!(
        acknowledgements.state.borrow().reads + acknowledgements.state.borrow().writes,
        0
    );
    Ok(())
}

#[test]
fn current_and_opening_authority_are_both_required_and_consumed_tick_does_not_regress() -> TestResult {
    let (mut journal, storage, acknowledgements) = seeded()?;
    let mut c = context();
    c.session_id = SessionId::new(19);
    assert!(journal.verify(&c).is_err());
    c = context();
    c.anchor.tick = GameTick(1201);
    assert!(journal.verify(&c).is_err());
    assert!(journal.verify(&context()).is_err());
    let pin = journal.pin();
    assert_eq!(pin.authority_tick_floor, 1201);
    let before_sync = (storage.state.borrow().syncs, acknowledgements.state.borrow().syncs);
    drop(journal);
    assert!(reopen(&storage, &acknowledgements, pin, Mode::Recover).is_err());
    assert_eq!(
        (storage.state.borrow().syncs, acknowledgements.state.borrow().syncs),
        before_sync
    );
    let (mut journal, _, _) = seeded()?;
    let mut revoked = context();
    revoked.grants.clear();
    assert!(journal.advance(&revoked, Stage::Workshop, workshop()).is_err());
    Ok(())
}

#[test]
fn identity_loss_and_flush_failure_fence_without_rewriting_history() -> TestResult {
    for identity in [false, true] {
        let (mut journal, storage, acknowledgements) = seeded()?;
        if identity {
            acknowledgements.state.borrow_mut().invalid_identity = true;
        } else {
            storage.state.borrow_mut().fail_flush = true;
        }
        assert!(journal.advance(&context(), Stage::Workshop, workshop()).is_err());
        assert!(journal.is_fenced());
        assert_eq!(
            storage.state.borrow().truncates + acknowledgements.state.borrow().truncates,
            0
        );
    }
    Ok(())
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
mod private_files {
    use super::*;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt, symlink};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    struct Directory(PathBuf);
    impl Directory {
        fn new() -> TestResult<Self> {
            let path = std::env::temp_dir().join(format!(
                "dfmcp-resource-parent-{}-{}",
                std::process::id(),
                SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::DirBuilder::new().mode(0o700).create(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn real_private_files_lock_sync_reopen_and_reject_prefix_rollback() -> TestResult {
        let directory = Directory::new()?;
        let c = context();
        let mut journal = open_private(&directory.0, &c, Mode::Control, true, original(), None)?;
        let old_pin = journal.pin();
        let initial = std::fs::read(directory.0.join("resource-project.journal"))?;
        assert_eq!(
            std::fs::metadata(directory.0.join("resource-project.journal"))?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert!(
            open_private(
                &directory.0,
                &c,
                Mode::Recover,
                false,
                original(),
                Some(old_pin.clone()),
            )
            .is_err()
        );
        journal.advance(&c, Stage::Workshop, workshop())?;
        let pin = journal.pin();
        drop(journal);
        let recovered = open_private(
            &directory.0,
            &c,
            Mode::Offline,
            false,
            original(),
            Some(pin.clone()),
        )?;
        assert_eq!(recovered.snapshot().stage, Stage::Workshop);
        drop(recovered);
        assert!(open_private(&directory.0, &c, Mode::Control, true, original(), None).is_err());
        std::fs::write(directory.0.join("resource-project.journal"), initial)?;
        assert!(
            open_private(
                &directory.0,
                &c,
                Mode::Recover,
                false,
                original(),
                Some(old_pin),
            )
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn real_private_factory_refuses_symlinks_wrong_modes_missing_recovery_and_cap_overflow() -> TestResult {
        let directory = Directory::new()?;
        let c = context();
        let path = directory.0.join("new.journal");
        assert!(
            open_private_resource_project_storage(&path, &c, Mode::Recover, false, MAX_BYTES)
                .is_err()
        );
        assert!(!path.exists());
        assert!(
            open_private_resource_project_storage(&path, &c, Mode::Control, true, MAX_BYTES + 1)
                .is_err()
        );
        assert!(!path.exists());
        let target = directory.0.join("target");
        std::fs::write(&target, b"untouched")?;
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600))?;
        symlink(&target, &path)?;
        assert!(
            open_private_resource_project_storage(&path, &c, Mode::Control, true, MAX_BYTES)
                .is_err()
        );
        assert_eq!(std::fs::read(&target)?, b"untouched");
        let bad = directory.0.join("wrong-mode");
        std::fs::write(&bad, b"untouched")?;
        std::fs::set_permissions(&bad, std::fs::Permissions::from_mode(0o640))?;
        assert!(
            open_private_resource_project_storage(&bad, &c, Mode::Recover, false, MAX_BYTES)
                .is_err()
        );
        Ok(())
    }
}
