//! Public journal APIs checked against independently computed binary vectors.
//! In-memory storage does not establish real filesystem or power-loss durability.
use dfmcp_adapter::control_effect_journal::{
    ControlEffectJournal, DurablePauseState, EffectJournalStorage, EffectTailRecovery,
};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode, FortressId,
    GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId,
    StateAnchor, WorkBudget,
};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::sync::{Arc, Mutex, MutexGuard};

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Cursor<Vec<u8>>>>);
impl Memory {
    fn lock(&self) -> io::Result<MutexGuard<'_, Cursor<Vec<u8>>>> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("test memory poisoned"))
    }
    fn from_bytes(bytes: Vec<u8>) -> Self {
        Self(Arc::new(Mutex::new(Cursor::new(bytes))))
    }
    fn bytes(&self) -> Result<Vec<u8>> {
        Ok(self
            .lock()
            .map_err(|_| DfmcpError::new(ErrorCode::CorruptLedger, "test memory unavailable"))?
            .get_ref()
            .clone())
    }
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.lock()?.read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.lock()?.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.lock()?.seek(from)
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.lock()?.get_mut().truncate(length as usize);
        Ok(())
    }
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(81),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: FortressId::new(1),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(10),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [Capability::ControlClock, Capability::Query]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    }
}
fn fixture() -> Result<(ControlEffectJournal<Memory>, Memory)> {
    let memory = Memory::default();
    let mut journal = ControlEffectJournal::open(
        memory.clone(),
        &context(),
        true,
        7,
        EffectTailRecovery::Refuse,
    )?;
    journal.record_prepared(
        "key".into(),
        Digest32::of_bytes(b"cancel-this-pause-plan"),
        true,
        10,
        7,
        [1; 16],
        &context(),
    )?;
    journal.cancel_prepared(
        "key",
        Digest32::of_bytes(b"cancel-this-pause-plan"),
        &context(),
    )?;
    Ok((journal, memory))
}

#[test]
fn public_cancellation_bytes_match_the_independent_sha256_vector() -> Result<()> {
    let (journal, memory) = fixture()?;
    assert_eq!(
        journal.id().to_string(),
        "64ff0324a202dcd9922c7cd60f088195c236588f5158765b178f9fa68bb8a61e"
    );
    assert_eq!(
        journal.head().to_string(),
        "661f25bdf9e66e16bcd738562c52ebfb8db2e94db8e297985fedbc0f1fc0c5ee"
    );
    assert_eq!(journal.retained_bytes(), 564);
    let record = journal
        .lookup("key")
        .ok_or_else(|| DfmcpError::new(ErrorCode::CorruptLedger, "vector key absent"))?;
    assert_eq!(
        record.previous_digest.to_string(),
        "887fb94aad95b4a4c2d270b27dacfb0eba4f1fc32b5169f961ec810f6bcef354"
    );
    assert_eq!(record.state, DurablePauseState::CancelledBeforeDispatch);
    let bytes = memory.bytes()?;
    assert_eq!(
        Digest32::of_bytes(&bytes).to_string(),
        "25f115661f385a23817e59069ac9c7317bc53002244af4671a4ef27e39c46c9d"
    );
    let recovered = ControlEffectJournal::open_read_only(Memory::from_bytes(bytes), &context())?;
    assert_eq!(recovered.lookup("key"), Some(record));
    Ok(())
}

#[test]
fn cancellation_replay_rejects_every_corrupted_byte_and_incomplete_prefix() -> Result<()> {
    let (_, memory) = fixture()?;
    let bytes = memory.bytes()?;
    for index in 0..bytes.len() {
        let mut changed = bytes.clone();
        changed[index] ^= 1;
        assert!(
            ControlEffectJournal::open_read_only(Memory::from_bytes(changed), &context()).is_err(),
            "byte={index}"
        );
    }
    for length in 0..bytes.len() {
        let result = ControlEffectJournal::open_read_only(
            Memory::from_bytes(bytes[..length].to_vec()),
            &context(),
        );
        // Empty valid header and exact end of Prepared are complete prefixes.
        assert_eq!(
            result.is_ok(),
            length == 72 || length == 318,
            "prefix={length}"
        );
    }
    Ok(())
}
