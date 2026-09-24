//! Actual journal append/replay paths with native-codec fixtures and injected I/O.
use super::*;
use crate::live_operations::OperationsProfile;
use crate::live_spatial::{
    LiveSpatialObservation, SpatialStateView, citizens::LiveSpatialCitizenObservation,
};
use dfmcp_core::{CapabilityGrant, CapabilityScope, RequestId, SessionId, WorkBudget};
use dfmcp_world::{EntityKind, Value};
use std::io::Cursor;

#[derive(Default)]
struct Memory {
    bytes: Cursor<Vec<u8>>,
    fail_after: Option<usize>,
    fail_sync: bool,
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
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let count = match &mut self.fail_after {
            Some(0) => return Err(io::Error::other("injected write failure")),
            Some(left) => {
                let n = (*left).min(data.len());
                *left -= n;
                n
            }
            None => data.len(),
        };
        self.bytes.write(&data[..count])
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl JournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        if self.fail_sync {
            Err(io::Error::other("injected sync failure"))
        } else {
            Ok(())
        }
    }
    fn truncate(&mut self, n: u64) -> io::Result<()> {
        self.bytes.get_mut().truncate(n as usize);
        Ok(())
    }
}
fn put(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}
fn text(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}
fn part(out: &mut Vec<u8>, bytes: &[u8]) {
    put(out, bytes.len() as u32);
    out.extend_from_slice(bytes);
}
fn observation(tick: u32, workers: u32, generation: u64) -> Result<LiveSpatialCitizenObservation> {
    let hex = include_str!("../tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| corrupt("fixture hex")))
        .collect::<Result<Vec<_>>>()?;
    let first =
        LiveSpatialObservation::decode_payload(&bytes, generation, "df".into(), "dfhack".into())?;
    let mut operations = first.operations().clone();
    let mut terrain = first.terrain().clone();
    operations.jobs.year_tick = tick;
    terrain.year_tick = tick;
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &terrain.encode_payload()?);
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
    let mut combined = b"DFMS1800".to_vec();
    part(&mut combined, &spatial);
    part(&mut combined, &citizens);
    LiveSpatialCitizenObservation::decode_payload(
        &combined,
        generation,
        "df".into(),
        "dfhack".into(),
    )
}
fn context() -> Result<OperationContext> {
    let mut state = Spatial18::empty();
    Spatial18::publish(&mut state, observation(3, 8, 7)?)?;
    let anchor = Spatial18::snapshot(&state)
        .ok_or_else(|| corrupt("test snapshot"))?
        .anchor();
    Ok(OperationContext {
        session_id: SessionId::new(1881),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 100_000,
            max_bytes: Spatial18::MAX_PAYLOAD as u64,
            max_output_tokens: 8192,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        cancellation_requested: false,
    })
}
fn open(
    bytes: Vec<u8>,
    c: &OperationContext,
    repair: TailRecovery,
) -> Result<SpatialCitizenJournal<Memory>> {
    let new = bytes.is_empty();
    SpatialCitizenJournal::open(
        Memory {
            bytes: Cursor::new(bytes),
            ..Memory::default()
        },
        c,
        JournalLimits::default(),
        new,
        repair,
    )
}
fn filled() -> Result<(SpatialCitizenJournal<Memory>, OperationContext)> {
    let c = context()?;
    let mut j = open(Vec::new(), &c, TailRecovery::Refuse)?;
    j.append(observation(3, 8, 7)?, &c)?;
    j.append(observation(4, 8, 7)?, &c)?;
    Ok((j, c))
}
fn record<'a>(bytes: &'a [u8], entry: &JournalEntry) -> &'a [u8] {
    &bytes[entry.offset as usize..entry.offset as usize + entry.encoded_bytes as usize]
}
fn reseal(bytes: &mut [u8], id: Digest32) {
    let length = bytes.len() - 40;
    let hash = frame_hash(id, &bytes[..length]);
    bytes[length..length + 32].copy_from_slice(hash.as_bytes());
}

#[test]
fn mixed_frames_reopen_and_historical_queries_reproduce_exact_anchors() -> Result<()> {
    let (j, c) = filled()?;
    let entries = j.entries().to_vec();
    let stats = j.storage_stats();
    assert_eq!(stats.raw_records, 1);
    assert_eq!(stats.delta_records, 1);
    assert!(stats.stored_payload_bytes + 128 <= stats.expanded_payload_bytes);
    let expected = j.state().snapshot().cloned();
    let bytes = j.storage.bytes.into_inner();
    assert_eq!(&record(&bytes, &entries[0])[..8], RECORD);
    assert_eq!(&record(&bytes, &entries[1])[..8], compression::DELTA_RECORD);
    let mut reopened = open(bytes, &c, TailRecovery::Refuse)?;
    assert_eq!(reopened.entries(), entries);
    assert_eq!(reopened.storage_stats(), stats);
    assert_eq!(reopened.state().snapshot(), expected.as_ref());
    for entry in entries {
        assert_eq!(
            reopened
                .snapshot_at(entry.number, entry.record_digest, &c)?
                .anchor(),
            entry.anchor
        );
    }
    assert_eq!(reopened.state().snapshot(), expected.as_ref());
    let length = reopened.retained_bytes();
    assert_eq!(
        reopened.append(observation(4, 8, 7)?, &c)?,
        JobPublication::Heartbeat
    );
    assert_eq!(reopened.retained_bytes(), length);
    assert_eq!(reopened.storage_stats(), stats);
    Ok(())
}

#[test]
fn old_raw_prefixes_append_deltas_without_rewriting_any_retained_byte() -> Result<()> {
    let c = context()?;
    let mut j = open(Vec::new(), &c, TailRecovery::Refuse)?;
    j.append(observation(3, 8, 7)?, &c)?;
    j.payload_base = None; // Reproduce the original raw writer for the second record.
    j.append(observation(4, 8, 7)?, &c)?;
    let prefix = j.storage.bytes.get_ref().clone();
    let old = j.entries().to_vec();
    let mut recovered = open(prefix.clone(), &c, TailRecovery::Refuse)?;
    recovered.append(observation(5, 8, 7)?, &c)?;
    assert_eq!(
        &recovered.storage.bytes.get_ref()[..prefix.len()],
        prefix.as_slice()
    );
    assert_eq!(&recovered.entries()[..2], old);
    assert_eq!(recovered.storage_stats().raw_records, 2);
    assert_eq!(recovered.storage_stats().delta_records, 1);
    Ok(())
}

#[test]
fn resets_and_periodic_records_are_raw_but_generations_survive_compression() -> Result<()> {
    let c = context()?;
    let mut j = open(Vec::new(), &c, TailRecovery::Refuse)?;
    for n in 0..65 {
        j.append(observation(3 + n, 8, 7)?, &c)?;
    }
    assert_eq!(j.storage_stats().raw_records, 2);
    assert_eq!(j.storage_stats().delta_records, 63);
    assert_eq!(
        &record(j.storage.bytes.get_ref(), &j.entries()[64])[..8],
        RECORD
    );
    j.append(observation(68, 7, 7)?, &c)?;
    j.append(observation(69, 8, 7)?, &c)?;
    let snapshot = j.state().snapshot().ok_or_else(|| corrupt("test state"))?;
    let citizen = snapshot
        .graph
        .entities
        .values()
        .find(|e| {
            e.kind == EntityKind::Unit
                && e.fields
                    .get("native_unit_id")
                    .is_some_and(|f| f.value == Value::U64(17))
        })
        .ok_or_else(|| corrupt("fixture citizen missing"))?;
    assert_eq!(citizen.generation, 2);
    assert_eq!(j.append(observation(3, 8, 8)?, &c)?, JobPublication::Reset);
    let expected = j.state().snapshot().cloned();
    let entries = j.entries().to_vec();
    assert_eq!(
        &record(j.storage.bytes.get_ref(), &entries[67])[..8],
        RECORD
    );
    let recovered = open(j.storage.bytes.into_inner(), &c, TailRecovery::Refuse)?;
    assert_eq!(recovered.state().snapshot(), expected.as_ref());
    Ok(())
}

#[test]
fn compressed_capacity_extends_retention_without_widening_limits() -> Result<()> {
    let c = context()?;
    let first = observation(3, 8, 7)?;
    let mut j = open(Vec::new(), &c, TailRecovery::Refuse)?;
    j.append(first.clone(), &c)?;
    let raw_length = j.entries()[0].encoded_bytes as u64;
    j.limits.max_bytes = HEADER_BYTES as u64 + raw_length * 3;
    for tick in 4..9 {
        j.append(observation(tick, 8, 7)?, &c)?;
    }
    assert_eq!(j.entries().len(), 6);
    assert!(j.retained_bytes() < j.limits.max_bytes);
    let before = j.storage.bytes.get_ref().clone();
    let stats = j.storage_stats();
    j.limits.max_records = 6;
    assert!(
        matches!(j.append(observation(9, 8, 7)?, &c), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(j.storage.bytes.get_ref(), &before);
    assert_eq!(j.storage_stats(), stats);
    assert!(!j.fenced());
    Ok(())
}

#[test]
fn every_incomplete_delta_prefix_requires_explicit_tail_repair() -> Result<()> {
    let (j, c) = filled()?;
    let second = j.entries()[1].clone();
    let bytes = j.storage.bytes.into_inner();
    for end in second.offset as usize + 1..bytes.len() {
        let mut memory = Memory {
            bytes: Cursor::new(bytes[..end].to_vec()),
            ..Memory::default()
        };
        assert!(
            SpatialCitizenJournal::open(
                &mut memory,
                &c,
                JournalLimits::default(),
                false,
                TailRecovery::Refuse
            )
            .is_err()
        );
        assert_eq!(memory.bytes.get_ref(), &bytes[..end]);
        let recovered = open(bytes[..end].to_vec(), &c, TailRecovery::TruncateIncomplete)?;
        assert_eq!(recovered.entries().len(), 1);
        assert_eq!(recovered.storage_stats().delta_records, 0);
        assert_eq!(recovered.retained_bytes(), second.offset);
    }
    Ok(())
}

#[test]
fn every_corrupt_delta_byte_is_refused_without_repair() -> Result<()> {
    let (j, c) = filled()?;
    let start = j.entries()[1].offset as usize;
    let bytes = j.storage.bytes.into_inner();
    for i in start..bytes.len() {
        let mut changed = bytes.clone();
        changed[i] ^= 1;
        let mut memory = Memory {
            bytes: Cursor::new(changed.clone()),
            ..Memory::default()
        };
        assert!(
            SpatialCitizenJournal::open(
                &mut memory,
                &c,
                JournalLimits::default(),
                false,
                TailRecovery::TruncateIncomplete
            )
            .is_err(),
            "byte {i}"
        );
        assert_eq!(memory.bytes.get_ref(), &changed);
    }
    Ok(())
}

#[test]
fn checksummed_wrong_predecessor_epoch_source_and_keyframe_are_not_trusted() -> Result<()> {
    let (j, c) = filled()?;
    let start = j.entries()[1].offset as usize;
    let id = j.id();
    let bytes = j.storage.bytes.into_inner();
    for case in 0..4 {
        let mut changed = bytes.clone();
        let frame = &mut changed[start..];
        let field = FRAME_HEADER_BYTES
            + match case {
                0 => 8,
                1 => 48,
                2 => 104,
                _ => 0,
            };
        match case {
            0 | 2 => frame[field..field + 32].fill(0),
            1 => frame[field..field + 8].copy_from_slice(&2u64.to_be_bytes()),
            _ => frame[field..field + 8].copy_from_slice(&65u64.to_be_bytes()),
        }
        reseal(frame, id);
        let mut memory = Memory {
            bytes: Cursor::new(changed.clone()),
            ..Memory::default()
        };
        assert!(
            matches!(SpatialCitizenJournal::open(&mut memory, &c, JournalLimits::default(), false,
            TailRecovery::TruncateIncomplete), Err(e) if e.code == ErrorCode::CorruptLedger)
        );
        assert_eq!(memory.bytes.get_ref(), &changed);
    }
    Ok(())
}

#[test]
fn partial_writes_and_uncertain_sync_preserve_state_stats_and_delta_base() -> Result<()> {
    let (j, c) = filled()?;
    let original = j.storage.bytes.into_inner();
    for failure in [Some(0), Some(1), Some(100), None] {
        let mut j = open(original.clone(), &c, TailRecovery::Refuse)?;
        let old = j.state().snapshot().cloned();
        let stats = j.storage_stats();
        let base = j.payload_base.as_ref().map(|b| b.digest);
        j.storage.fail_after = failure;
        j.storage.fail_sync = failure.is_none();
        assert!(j.append(observation(5, 8, 7)?, &c).is_err());
        assert!(j.fenced());
        assert_eq!(j.state().snapshot(), old.as_ref());
        assert_eq!(j.storage_stats(), stats);
        assert_eq!(j.payload_base.as_ref().map(|b| b.digest), base);
        let reopened = open(
            j.storage.bytes.into_inner(),
            &c,
            TailRecovery::TruncateIncomplete,
        )?;
        assert_eq!(
            reopened.entries().len(),
            if failure.is_none() { 3 } else { 2 }
        );
    }
    Ok(())
}

#[test]
fn expanded_budget_and_current_authority_apply_to_compressed_history() -> Result<()> {
    let (mut j, c) = filled()?;
    let entry = j.entries()[1].clone();
    let first = compression::decode::<Spatial18>(
        record(j.storage.bytes.get_ref(), &j.entries()[0]),
        j.id(),
        HEADER_BYTES as u64,
        None,
        Spatial18::MAX_PAYLOAD,
    )?;
    let encoded = record(j.storage.bytes.get_ref(), &entry);
    assert!(
        matches!(compression::decode::<Spatial18>(encoded, j.id(), entry.offset,
        first.payload_base.as_ref(), 1), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(
        matches!(j.state_at(2, entry.record_digest, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    let mut cancelled = c.clone();
    cancelled.cancellation_requested = true;
    assert!(j.state_at(2, entry.record_digest, &cancelled).is_err());
    let mut narrow = c.clone();
    narrow.budget.max_bytes = 1;
    assert!(
        matches!(j.state_at(2, entry.record_digest, &narrow), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    assert!(!j.fenced());
    assert_eq!(j.entries().len(), 2);
    Ok(())
}

#[test]
fn legacy_profiles_keep_raw_bytes_and_reject_delta_markers_before_tail_repair() -> Result<()> {
    let (j, c) = filled()?;
    let source = observation(4, 8, 7)?;
    let op = source.spatial().operations();
    let anchor = j.entries()[1].anchor;
    let expected = encode_profile_frame::<Operations13>(j.id(), 2, j.head(), anchor, op)?;
    assert_eq!(
        decode_profile_frame::<Operations13>(&expected, j.id(), 0)?.1,
        *op
    );
    assert_eq!(
        compression::encode::<Operations13>(
            j.id(),
            2,
            j.head(),
            anchor,
            op,
            j.payload_base.as_ref()
        )?,
        expected
    );
    for marker in [RECORD, compression::DELTA_RECORD] {
        let mut prefix = marker.to_vec();
        prefix.extend_from_slice(&200u32.to_be_bytes());
        prefix.extend_from_slice(frame_header_hash(j.id(), &prefix).as_bytes());
        let raw = marker == RECORD;
        assert_eq!(
            decode_frame_header::<Operations13>(&prefix, j.id()).is_ok(),
            raw
        );
        assert_eq!(
            decode_frame_header::<Operations14>(&prefix, j.id()).is_ok(),
            raw
        );
        assert_eq!(
            decode_frame_header::<Spatial16>(&prefix, j.id()).is_ok(),
            raw
        );
        assert!(decode_frame_header::<Spatial18>(&prefix, j.id()).is_ok());
    }
    // Changing only the file profile cannot smuggle a delta into a legacy file.
    let mut bytes = j.storage.bytes.into_inner();
    bytes[..8].copy_from_slice(Spatial16::MAGIC);
    let hash = header_hash(&bytes[..48]);
    bytes[48..80].copy_from_slice(hash.as_bytes());
    let mut memory = Memory {
        bytes: Cursor::new(bytes.clone()),
        ..Memory::default()
    };
    assert!(
        SpatialJournal::open(
            &mut memory,
            &c,
            JournalLimits::default(),
            false,
            TailRecovery::TruncateIncomplete
        )
        .is_err()
    );
    assert_eq!(memory.bytes.get_ref(), &bytes);
    Ok(())
}
