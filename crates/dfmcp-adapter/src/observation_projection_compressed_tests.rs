//! Exercise the actual multi-record reader against mixed spatial/1.8 storage.
//! The native payload fixture is decoded and normalized by the real profile.
use super::*;
use crate::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenObservation};
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};
use std::cell::Cell;
use std::io::Cursor;
use std::rc::Rc;

#[derive(Default)]
struct Memory {
    bytes: Cursor<Vec<u8>>,
    reads: usize,
    writes: usize,
    syncs: usize,
    identity_failed: Rc<Cell<bool>>,
}
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let count = self.bytes.read(out)?;
        self.reads += count;
        Ok(count)
    }
}
impl Seek for Memory {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(position)
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
impl JournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        self.syncs += 1;
        Ok(())
    }
    fn truncate(&mut self, length: u64) -> io::Result<()> {
        self.bytes.get_mut().truncate(length as usize);
        Ok(())
    }
    fn validate_identity(&self) -> io::Result<()> {
        if self.identity_failed.get() {
            Err(io::Error::other("injected custody loss"))
        } else {
            Ok(())
        }
    }
}
fn put(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_be_bytes());
}
fn text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}
fn source(tick: u32, workers: u32, generation: u64) -> Result<LiveSpatialCitizenObservation> {
    let spatial = super::tests::observation(tick)?.encode_payload()?;
    let mut citizens = b"DFMC1800".to_vec();
    put(&mut citizens, workers);
    for i in 0..workers {
        put(&mut citizens, 10 + i);
        text(&mut citizens, "Urist");
        text(&mut citizens, "DWARF");
        for n in [0, 2, 2, 5] {
            put(&mut citizens, n);
        }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());
        put(&mut citizens, 6);
        citizens.extend_from_slice(&[1, 1]);
        citizens.extend_from_slice(&1u16.to_be_bytes());
        put(&mut citizens, 0);
        text(&mut citizens, "CARPENTRY");
        for n in [5, 5, 1] {
            put(&mut citizens, n);
        }
    }
    let mut payload = b"DFMS1800".to_vec();
    for part in [spatial, citizens] {
        put(&mut payload, part.len() as u32);
        payload.extend(part);
    }
    LiveSpatialCitizenObservation::decode_payload(
        &payload,
        generation,
        "df".into(),
        "dfhack".into(),
    )
}
fn fixture(count: u32) -> Result<(SpatialCitizenJournal<Memory>, OperationContext)> {
    let initial = source(3, 8, 7)?;
    let mut state = Spatial18::empty();
    Spatial18::publish(&mut state, initial)?;
    let anchor = Spatial18::snapshot(&state)
        .ok_or_else(|| corrupt("initial snapshot"))?
        .anchor();
    let mut context = OperationContext {
        session_id: SessionId::new(18_933),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 16 * 1024 * 1024,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        cancellation_requested: false,
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    };
    let mut journal = SpatialCitizenJournal::open(
        Memory::default(),
        &context,
        JournalLimits::default(),
        true,
        TailRecovery::Refuse,
    )?;
    for n in 0..count {
        journal.append(source(3 + n, 8, 7)?, &context)?;
    }
    context.anchor = journal
        .state()
        .snapshot()
        .ok_or_else(|| corrupt("latest snapshot"))?
        .anchor();
    journal.storage.reads = 0;
    Ok((journal, context))
}
fn selection(j: &SpatialCitizenJournal<Memory>, numbers: &[usize]) -> Vec<(u64, Digest32)> {
    numbers
        .iter()
        .map(|n| (j.entries()[n - 1].number, j.entries()[n - 1].record_digest))
        .collect()
}

#[test]
fn sparse_selection_carries_every_delta_base_and_reads_the_prefix_once() -> Result<()> {
    let (mut j, c) = fixture(5)?;
    assert_eq!(j.storage_stats().raw_records, 1);
    assert_eq!(j.storage_stats().delta_records, 4);
    let wanted = selection(&j, &[2, 4]);
    let expected = j.entries()[3].offset + u64::from(j.entries()[3].encoded_bytes);
    let bytes = j.storage.bytes.get_ref().clone();
    let head = j.head();
    let stats = j.storage_stats();
    let mutations = (j.storage.writes, j.storage.syncs);
    let values = j.project_records(&wanted, &c, |_, s| {
        Ok(s.snapshot()
            .ok_or_else(|| corrupt("projected snapshot"))?
            .anchor())
    })?;
    assert_eq!(j.storage.reads as u64, expected);
    assert_eq!(values.len(), 2);
    for (entry, anchor) in values {
        assert_eq!(entry.anchor, anchor);
        assert_eq!(
            j.state_at(entry.number, entry.record_digest, &c)?
                .snapshot()
                .map(WorldSnapshot::anchor),
            Some(anchor)
        );
    }
    assert_eq!(
        j.state().snapshot().map(WorldSnapshot::anchor),
        Some(c.anchor)
    );
    assert_eq!(j.head(), head);
    assert_eq!(j.storage_stats(), stats);
    assert_eq!(j.storage.bytes.get_ref(), &bytes);
    assert_eq!((j.storage.writes, j.storage.syncs), mutations);
    Ok(())
}

#[test]
fn independent_pages_and_reopen_never_use_the_current_live_payload_as_a_base() -> Result<()> {
    let (j, c) = fixture(5)?;
    let bytes = j.storage.bytes.into_inner();
    let mut reopened = SpatialCitizenJournal::open(
        Memory {
            bytes: Cursor::new(bytes),
            ..Memory::default()
        },
        &c,
        JournalLimits::default(),
        false,
        TailRecovery::Refuse,
    )?;
    for numbers in [&[4, 5][..], &[1, 2][..], &[2, 3][..]] {
        let wanted = selection(&reopened, numbers);
        let values = reopened.project_records(&wanted, &c, |_, s| {
            Ok(s.snapshot()
                .ok_or_else(|| corrupt("projected snapshot"))?
                .anchor())
        })?;
        for (entry, anchor) in values {
            assert_eq!(entry.anchor, anchor);
        }
        assert_eq!(
            reopened.state().snapshot().map(WorldSnapshot::anchor),
            Some(c.anchor)
        );
    }
    Ok(())
}

#[test]
fn mixed_keyframes_and_epoch_reset_project_the_recorded_generation_universe() -> Result<()> {
    let (mut j, mut c) = fixture(65)?;
    j.append(source(68, 7, 7)?, &c)?;
    j.append(source(69, 8, 7)?, &c)?;
    j.append(source(3, 8, 8)?, &c)?;
    j.append(source(4, 8, 8)?, &c)?;
    c.anchor = j
        .state()
        .snapshot()
        .ok_or_else(|| corrupt("latest"))?
        .anchor();
    assert_eq!(j.storage_stats().raw_records, 3);
    let wanted = selection(&j, &[64, 65, 66, 67, 68, 69]);
    let values = j.project_records(&wanted, &c, |_, s| {
        Ok(s.snapshot().ok_or_else(|| corrupt("projected"))?.clone())
    })?;
    for (entry, snapshot) in values {
        assert_eq!(snapshot.anchor(), entry.anchor);
        assert_eq!(
            j.state_at(entry.number, entry.record_digest, &c)?
                .snapshot(),
            Some(&snapshot)
        );
    }
    assert!(!j.fenced());
    Ok(())
}

#[test]
fn corruption_of_an_unselected_delta_fences_without_returning_partial_rows() -> Result<()> {
    let (mut j, c) = fixture(4)?;
    let wanted = selection(&j, &[4]);
    let offset = j.entries()[1].offset as usize + FRAME_HEADER_BYTES + 12;
    j.storage.bytes.get_mut()[offset] ^= 1;
    let mut calls = 0;
    assert!(
        matches!(j.project_records(&wanted, &c, |_, _| { calls += 1; Ok(()) }),
        Err(e) if e.code == ErrorCode::CorruptLedger)
    );
    assert_eq!(calls, 0);
    assert!(j.fenced());
    assert_eq!(
        j.state().snapshot().map(WorldSnapshot::anchor),
        Some(c.anchor)
    );
    Ok(())
}

#[test]
fn expanded_delta_budget_is_checked_before_projecting_the_target() -> Result<()> {
    let (mut j, mut c) = fixture(1)?;
    let maximum = source(3, 8, 7)?.encode_payload()?.len();
    j.append(source(4, 32, 7)?, &c)?;
    assert!(j.storage_stats().delta_records > 0);
    c.budget.max_bytes = maximum as u64;
    let wanted = selection(&j, &[2]);
    let mut calls = 0;
    assert!(
        matches!(j.project_records(&wanted, &c, |_, _| { calls += 1; Ok(()) }),
        Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(calls, 0);
    assert!(!j.fenced());
    Ok(())
}

#[test]
fn late_callback_refusal_does_not_change_the_next_append_compression_base() -> Result<()> {
    let (mut j, c) = fixture(4)?;
    let wanted = selection(&j, &[1, 2, 3]);
    let bytes = j.storage.bytes.get_ref().clone();
    let stats = j.storage_stats();
    assert!(matches!(j.project_records(&wanted, &c, |e, _| {
        if e.number == 3 { Err(budget("late output refusal")) } else { Ok(e.number) }
    }), Err(e) if e.code == ErrorCode::BudgetExceeded));
    assert_eq!(j.storage.bytes.get_ref(), &bytes);
    assert_eq!(j.storage_stats(), stats);
    assert!(!j.fenced());
    j.append(source(7, 8, 7)?, &c)?;
    let last = j
        .entries()
        .last()
        .cloned()
        .ok_or_else(|| corrupt("last entry"))?;
    assert_eq!(
        j.state_at(last.number, last.record_digest, &c)?
            .snapshot()
            .map(WorldSnapshot::anchor),
        Some(last.anchor)
    );
    Ok(())
}

#[test]
fn final_custody_failure_discards_all_projected_results() -> Result<()> {
    let (mut j, c) = fixture(3)?;
    let wanted = selection(&j, &[3]);
    let loss = Rc::clone(&j.storage.identity_failed);
    let mut calls = 0;
    assert!(matches!(j.project_records(&wanted, &c, |_, _| {
        calls += 1; loss.set(true); Ok(())
    }), Err(e) if e.code == ErrorCode::CorruptLedger));
    assert_eq!(calls, 1);
    assert!(j.fenced());
    assert_eq!(
        j.state().snapshot().map(WorldSnapshot::anchor),
        Some(c.anchor)
    );
    Ok(())
}

#[test]
fn a_bad_later_reference_is_rejected_before_any_prefix_read_or_projection() -> Result<()> {
    let (mut j, c) = fixture(4)?;
    let mut wanted = selection(&j, &[1, 4]);
    wanted[1].1 = Digest32::ZERO;
    let mut calls = 0;
    assert!(
        matches!(j.project_records(&wanted, &c, |_, _| { calls += 1; Ok(()) }),
        Err(e) if e.code == ErrorCode::StaleAnchor)
    );
    assert_eq!(calls, 0);
    assert_eq!(j.storage.reads, 0);
    assert!(!j.fenced());
    Ok(())
}
