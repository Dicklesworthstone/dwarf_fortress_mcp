use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, FortressId, GameTick, ObservationCursor, RequestId,
    SessionId, StateAnchor, WorkBudget,
};
use std::io::Cursor;

#[derive(Default)]
struct Memory {
    bytes: Cursor<Vec<u8>>,
    writes: usize,
    syncs: usize,
    truncates: usize,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.bytes.read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(from)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        self.bytes.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.syncs += 1;
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.truncates += 1;
        self.bytes.get_mut().truncate(length as usize);
        Ok(())
    }
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(91),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: FortressId::NIL,
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: vec![
            CapabilityGrant {
                capability: Capability::Query,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::ControlClock,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
        ],
    }
}
fn fixture() -> Result<ControlEffectJournal<Memory>> {
    let mut journal = ControlEffectJournal::open(
        Memory::default(),
        &context(),
        true,
        7,
        EffectTailRecovery::Refuse,
    )?;
    for key in ["z-started", "a-prepared", "m-applied", "n-unknown"] {
        let plan = Digest32::of_bytes(key.as_bytes());
        journal.record_prepared(key.to_owned(), plan, true, 10, 7, [1; 16], &context())?;
        if key != "a-prepared" {
            journal.begin_commit(key, plan, 7, &context())?;
        }
        if key == "m-applied" {
            journal.record_reconciliation(
                key,
                plan,
                7,
                true,
                true,
                true,
                11,
                Some(Digest32::of_bytes(b"receipt")),
                &context(),
            )?;
        }
        if key == "n-unknown" {
            journal.mark_indeterminate(key, plan, &context())?;
        }
    }
    Ok(journal)
}
fn memory(bytes: Vec<u8>) -> Memory {
    Memory {
        bytes: Cursor::new(bytes),
        ..Memory::default()
    }
}

#[test]
fn recovery_enumerates_exact_sorted_effects_without_writes_or_a_bridge() -> Result<()> {
    let journal = fixture()?;
    let id = journal.id();
    let head = journal.head();
    let expected: Vec<_> = journal.records.values().cloned().collect();
    let bytes = journal.storage.bytes.into_inner();
    let mut read_context = context();
    read_context
        .grants
        .retain(|grant| grant.capability == Capability::Query);
    let mut recovered = ControlEffectJournal::open_read_only(memory(bytes.clone()), &read_context)?;
    let actual: Vec<_> = recovered.records(&read_context)?.cloned().collect();
    assert_eq!(actual, expected);
    assert_eq!(
        actual
            .iter()
            .map(|r| r.idempotency_key.as_str())
            .collect::<Vec<_>>(),
        ["a-prepared", "m-applied", "n-unknown", "z-started"]
    );
    assert!(recovered.read_only());
    assert_eq!((recovered.id(), recovered.head()), (id, head));
    assert_eq!(
        (
            recovered.storage.writes,
            recovered.storage.syncs,
            recovered.storage.truncates
        ),
        (0, 0, 0)
    );
    assert_eq!(recovered.storage.bytes.into_inner(), bytes);
    Ok(())
}

#[test]
fn recovery_refuses_all_effect_transitions_even_with_clock_authority() -> Result<()> {
    let bytes = fixture()?.storage.bytes.into_inner();
    let mut journal = ControlEffectJournal::open_read_only(memory(bytes.clone()), &context())?;
    let plan = Digest32::of_bytes(b"a-prepared");
    assert!(
        matches!(journal.record_prepared("a-prepared".into(), plan, true, 10, 7, [1; 16], &context()),
        Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert!(
        matches!(journal.begin_commit("a-prepared", plan, 7, &context()),
        Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert!(
        matches!(journal.mark_indeterminate("z-started", Digest32::of_bytes(b"z-started"), &context()),
        Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert!(
        matches!(journal.record_reconciliation("m-applied", Digest32::of_bytes(b"m-applied"),
        7, true, true, true, 11, Some(Digest32::of_bytes(b"receipt")), &context()),
        Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert!(!journal.fenced());
    assert_eq!(
        (
            journal.storage.writes,
            journal.storage.syncs,
            journal.storage.truncates
        ),
        (0, 0, 0)
    );
    assert_eq!(journal.storage.bytes.into_inner(), bytes);
    Ok(())
}

#[test]
fn recovery_rechecks_current_authority_cancellation_and_storage() -> Result<()> {
    let bytes = fixture()?.storage.bytes.into_inner();
    let mut denied = context();
    denied.grants.clear();
    assert!(
        matches!(ControlEffectJournal::open_read_only(memory(bytes.clone()), &denied),
        Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    let mut journal = ControlEffectJournal::open_read_only(memory(bytes), &context())?;
    assert!(matches!(journal.records(&denied), Err(e) if e.code == ErrorCode::CapabilityDenied));
    let mut cancelled = context();
    cancelled.cancellation_requested = true;
    assert!(
        matches!(journal.records(&cancelled), Err(e) if e.code == ErrorCode::CancellationRequested)
    );
    journal.storage.bytes.get_mut().push(0);
    assert!(matches!(journal.records(&context()), Err(e) if e.code == ErrorCode::CorruptLedger));
    assert!(journal.fenced());
    Ok(())
}

#[test]
fn recovery_never_initializes_or_repairs_an_incomplete_journal() -> Result<()> {
    assert!(ControlEffectJournal::open_read_only(Memory::default(), &context()).is_err());
    let bytes = fixture()?.storage.bytes.into_inner();
    for tail in [&RECORD[..3], &b"invalid"[..]] {
        let mut incomplete = bytes.clone();
        incomplete.extend_from_slice(tail);
        assert!(
            matches!(ControlEffectJournal::open_read_only(memory(incomplete), &context()),
            Err(e) if e.code == ErrorCode::CorruptLedger)
        );
    }
    Ok(())
}

#[test]
fn replay_rejects_hashed_applied_record_with_opposite_pause_evidence() -> Result<()> {
    let mut journal = fixture()?;
    let mut next = journal
        .lookup("z-started")
        .cloned()
        .ok_or_else(|| invalid("fixture"))?;
    next.state = DurablePauseState::VerifiedApplied;
    next.revision += 1;
    next.transition_number = journal.transition_count() as u64 + 1;
    next.previous_digest = journal.head();
    next.effect_known = true;
    next.effect_applied = true;
    next.observed_paused = Some(false);
    next.observed_game_tick = Some(11);
    next.receipt_digest = Some(Digest32::of_bytes(b"receipt"));
    let frame = encode_frame(journal.id(), &next)?;
    journal.storage.bytes.get_mut().extend_from_slice(&frame);
    assert!(
        matches!(ControlEffectJournal::open_read_only(memory(journal.storage.bytes.into_inner()), &context()),
        Err(e) if e.code == ErrorCode::CorruptLedger)
    );
    Ok(())
}

#[test]
fn replay_enforces_the_same_distinct_effect_bound_as_append() -> Result<()> {
    let mut journal = ControlEffectJournal::open(
        Memory::default(),
        &context(),
        true,
        7,
        EffectTailRecovery::Refuse,
    )?;
    let prototype = journal.record_prepared(
        "k0".into(),
        Digest32::of_bytes(b"plan"),
        true,
        10,
        7,
        [1; 16],
        &context(),
    )?;
    let mut head = journal.head();
    for index in 1..=MAX_EFFECTS {
        let mut record = prototype.clone();
        record.idempotency_key = format!("k{index}");
        record.transition_number = index as u64 + 1;
        record.previous_digest = head;
        let frame = encode_frame(journal.id(), &record)?;
        head = decode_frame(&frame, journal.id())?.record_digest;
        journal.storage.bytes.get_mut().extend_from_slice(&frame);
    }
    assert!(
        matches!(ControlEffectJournal::open_read_only(memory(journal.storage.bytes.into_inner()), &context()),
        Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn private_recovery_uses_existing_read_only_custody_and_never_creates() -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let serial = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let directory = std::env::temp_dir()
        .canonicalize()
        .map_err(storage_error)?
        .join(format!(
            "dfmcp-control-recovery-{}-{serial}",
            std::process::id()
        ));
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&directory)
        .map_err(storage_error)?;
    let path = directory.join("effects.bin");
    // Only this test's exclusively created directory is eligible for cleanup.
    let result = (|| -> Result<()> {
        assert!(open_private_control_recovery(&path, &context()).is_err());
        assert!(!path.exists());
        let mut live =
            open_private_control_journal(&path, &context(), 7, EffectTailRecovery::Refuse)?;
        live.record_prepared(
            "key".into(),
            Digest32::of_bytes(b"plan"),
            true,
            10,
            7,
            [1; 16],
            &context(),
        )?;
        assert!(open_private_control_recovery(&path, &context()).is_err());
        drop(live);
        let original = fs::read(&path).map_err(storage_error)?;
        let mut recovered = open_private_control_recovery(&path, &context())?;
        assert_eq!(recovered.records(&context())?.len(), 1);
        assert!(recovered.storage.write(b"bad").is_err());
        assert!(recovered.storage.sync().is_err());
        assert!(recovered.storage.truncate(0).is_err());
        drop(recovered);
        assert_eq!(fs::read(&path).map_err(storage_error)?, original);
        Ok(())
    })();
    if path.exists() {
        fs::remove_file(&path).map_err(storage_error)?;
    }
    fs::remove_dir(&directory).map_err(storage_error)?;
    result
}
