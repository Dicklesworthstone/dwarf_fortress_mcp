use super::super::*;
use crate::live_map::LiveMapObservation;
use crate::live_operations::{LiveOperationsObservation, OperationsProfile, item_entity_id};
use crate::live_spatial::LiveSpatialObservation;
use dfmcp_core::{CapabilityGrant, CapabilityScope, EntityId, RequestId, SessionId, WorkBudget};
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
fn fixture() -> Result<LiveSpatialObservation> {
    let hex = include_str!("../tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| corrupt("fixture hex")))
        .collect::<Result<Vec<_>>>()?;
    LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())
}
fn compose(
    op: LiveOperationsObservation,
    map: LiveMapObservation,
) -> Result<LiveSpatialObservation> {
    let mut out = b"DFMS1600".to_vec();
    for part in [
        op.encode_profile(OperationsProfile::PagedV1_4)?,
        map.encode_payload()?,
    ] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(&part);
    }
    LiveSpatialObservation::decode_payload(
        &out,
        map.bridge_generation,
        map.df_version,
        map.dfhack_version,
    )
}
fn changed(tick: u32) -> Result<LiveSpatialObservation> {
    let base = fixture()?;
    let mut op = base.operations().clone();
    let mut map = base.terrain().clone();
    op.jobs.year_tick = tick;
    map.year_tick = tick;
    op.items[2].stack_size = tick;
    compose(op, map)
}
fn context<P: JournalProfile>(observation: &P::Observation) -> Result<OperationContext> {
    let mut state = P::empty();
    P::publish(&mut state, observation.clone())?;
    let anchor = P::snapshot(&state)
        .ok_or_else(|| corrupt("test snapshot"))?
        .anchor();
    Ok(OperationContext {
        session_id: SessionId::new(117),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_entities: 100_000,
            max_bytes: P::MAX_PAYLOAD as u64,
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
fn open<P: JournalProfile>(
    bytes: Vec<u8>,
    c: &OperationContext,
    repair: TailRecovery,
) -> Result<ObservationJournal<Memory, P>> {
    let new = bytes.is_empty();
    ObservationJournal::open(
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

#[test]
fn spatial_reopen_recovers_the_combined_anchor_and_native_query_state() -> Result<()> {
    let first = fixture()?;
    let c = context::<Spatial16>(&first)?;
    let mut journal = open::<Spatial16>(Vec::new(), &c, TailRecovery::Refuse)?;
    journal.append(first.clone(), &c)?;
    journal.append(changed(4)?, &c)?;
    let records = journal.entries().to_vec();
    let latest = journal.state().snapshot().cloned();
    let bytes = journal.storage.bytes.into_inner();
    let mut recovered = open::<Spatial16>(bytes, &c, TailRecovery::Refuse)?;
    assert_eq!(recovered.entries(), records);
    assert_eq!(recovered.state().snapshot(), latest.as_ref());
    let archived = recovered.state_at(1, records[0].record_digest, &c)?;
    assert_eq!(archived.observation(), Some(&first));
    assert_eq!(
        archived.snapshot().map(WorldSnapshot::anchor),
        Some(records[0].anchor)
    );
    assert_eq!(recovered.state().snapshot(), latest.as_ref());
    assert_eq!(
        recovered.append(changed(4)?, &c)?,
        JobPublication::Heartbeat
    );
    assert_eq!(recovered.entries().len(), 2);
    Ok(())
}

#[test]
fn same_fortress_files_cannot_be_reinterpreted_or_repaired_as_another_profile() -> Result<()> {
    let first = fixture()?;
    let c = context::<Spatial16>(&first)?;
    let mut spatial = open::<Spatial16>(Vec::new(), &c, TailRecovery::Refuse)?;
    spatial.append(first.clone(), &c)?;
    let mut paged = open::<Operations14>(Vec::new(), &c, TailRecovery::Refuse)?;
    paged.append(first.operations().clone(), &c)?;
    let mut legacy = open::<Operations13>(Vec::new(), &c, TailRecovery::Refuse)?;
    legacy.append(first.operations().clone(), &c)?;
    let files = [
        legacy.storage.bytes.into_inner(),
        paged.storage.bytes.into_inner(),
        spatial.storage.bytes.into_inner(),
    ];
    for (index, original) in files.iter().enumerate() {
        assert_eq!(
            open::<Operations13>(original.clone(), &c, TailRecovery::Refuse).is_ok(),
            index == 0
        );
        assert_eq!(
            open::<Operations14>(original.clone(), &c, TailRecovery::Refuse).is_ok(),
            index == 1
        );
        assert_eq!(
            open::<Spatial16>(original.clone(), &c, TailRecovery::Refuse).is_ok(),
            index == 2
        );
        let mut storage = Memory {
            bytes: Cursor::new(original.clone()),
            ..Memory::default()
        };
        // The existing journal suite supplies the blanket borrowed-storage adapter.
        if index != 2 {
            assert!(
                SpatialJournal::open(
                    &mut storage,
                    &c,
                    JournalLimits::default(),
                    false,
                    TailRecovery::TruncateIncomplete
                )
                .is_err()
            );
            assert_eq!(storage.bytes.get_ref(), original);
        }
    }
    Ok(())
}

#[test]
fn spatial_retirement_reappearance_and_reset_survive_restart() -> Result<()> {
    let first = fixture()?;
    let c = context::<Spatial16>(&first)?;
    let mut j = open::<Spatial16>(Vec::new(), &c, TailRecovery::Refuse)?;
    j.append(first.clone(), &c)?;
    let mut op = first.operations().clone();
    let mut map = first.terrain().clone();
    let removed = op.items[2].native_id;
    op.items.remove(2);
    op.jobs.year_tick = 4;
    map.year_tick = 4;
    j.append(compose(op, map)?, &c)?;
    j.append(changed(5)?, &c)?;
    let before = j.state().snapshot().ok_or_else(|| corrupt("test state"))?;
    assert_eq!(
        before.graph.entities[&item_entity_id(removed)].generation,
        2
    );
    let mut op = first.operations().clone();
    let mut map = first.terrain().clone();
    op.jobs.bridge_generation = 8;
    map.bridge_generation = 8;
    j.append(compose(op, map)?, &c)?;
    let expected = j.state().snapshot().cloned();
    let recovered = open::<Spatial16>(j.storage.bytes.into_inner(), &c, TailRecovery::Refuse)?;
    assert_eq!(recovered.state().snapshot(), expected.as_ref());
    let snapshot = recovered
        .state()
        .snapshot()
        .ok_or_else(|| corrupt("test state"))?;
    assert_eq!(snapshot.cursor.epoch, 1);
    assert_eq!(
        snapshot.graph.entities[&item_entity_id(removed)].generation,
        3
    );
    assert_eq!(snapshot.graph.entities[&EntityId::new(1)].generation, 2);
    Ok(())
}

#[test]
fn changed_region_does_not_append_to_or_replace_existing_spatial_history() -> Result<()> {
    let first = fixture()?;
    let c = context::<Spatial16>(&first)?;
    let mut j = open::<Spatial16>(Vec::new(), &c, TailRecovery::Refuse)?;
    j.append(first.clone(), &c)?;
    let original = j.storage.bytes.get_ref().clone();
    let anchor = j.entries()[0].anchor;
    let mut map = first.terrain().clone();
    map.map.region.origin[0] += 1;
    let changed = compose(first.operations().clone(), map)?;
    assert!(j.append(changed, &c).is_err());
    assert_eq!(j.storage.bytes.get_ref(), &original);
    assert_eq!(
        j.state().snapshot().map(WorldSnapshot::anchor),
        Some(anchor)
    );
    Ok(())
}

#[test]
fn partial_write_and_uncertain_sync_do_not_publish_but_can_be_recovered() -> Result<()> {
    let first = fixture()?;
    let c = context::<Spatial16>(&first)?;
    for failing_sync in [false, true] {
        let mut j = open::<Spatial16>(Vec::new(), &c, TailRecovery::Refuse)?;
        j.append(first.clone(), &c)?;
        let anchor = j.entries()[0].anchor;
        if failing_sync {
            j.storage.fail_sync = true;
        } else {
            j.storage.fail_after = Some(100);
        }
        assert!(j.append(changed(4)?, &c).is_err());
        assert!(j.fenced());
        assert_eq!(
            j.state().snapshot().map(WorldSnapshot::anchor),
            Some(anchor)
        );
        assert_eq!(j.entries().len(), 1);
        let bytes = j.storage.bytes.into_inner();
        if !failing_sync {
            assert!(open::<Spatial16>(bytes.clone(), &c, TailRecovery::Refuse).is_err());
        }
        let recovered = open::<Spatial16>(bytes, &c, TailRecovery::TruncateIncomplete)?;
        assert_eq!(recovered.entries().len(), if failing_sync { 2 } else { 1 });
    }
    Ok(())
}

#[test]
fn replay_checks_current_authority_record_identity_and_acquisition_budget() -> Result<()> {
    let first = fixture()?;
    let c = context::<Spatial16>(&first)?;
    let mut j = open::<Spatial16>(Vec::new(), &c, TailRecovery::Refuse)?;
    j.append(first, &c)?;
    let digest = j.entries()[0].record_digest;
    assert!(j.state_at(1, Digest32::ZERO, &c).is_err());
    let mut denied = c.clone();
    denied.grants.clear();
    assert!(
        matches!(j.state_at(1, digest, &denied), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    let mut narrow = c.clone();
    narrow.budget.max_entities = 1;
    assert!(
        matches!(j.state_at(1, digest, &narrow), Err(e) if e.code == ErrorCode::BudgetExceeded)
    );
    let mut expired = c.clone();
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(c.anchor.tick);
    }
    expired.anchor.tick.0 += 1;
    assert!(
        matches!(j.state_at(1, digest, &expired), Err(e) if e.code == ErrorCode::CapabilityDenied)
    );
    assert!(!j.fenced());
    assert_eq!(j.entries().len(), 1);
    Ok(())
}

#[test]
fn paged_archive_accepts_large_native_captures_without_widening_legacy_decoding() -> Result<()> {
    let first = fixture()?;
    let mut op = first.operations().clone();
    op.jobs.jobs.clear();
    op.attachments.clear();
    let template = op.items[2].clone();
    op.items = (0..40_000u32)
        .map(|n| {
            let mut item = template.clone();
            item.native_id = n;
            item
        })
        .collect();
    op.next_item_id = 40_000;
    assert!(op.encode_payload().is_err());
    assert!(op.encode_profile(OperationsProfile::PagedV1_4)?.len() > 2 * 1024 * 1024);
    let c = context::<Operations14>(&op)?;
    let mut journal = open::<Operations14>(Vec::new(), &c, TailRecovery::Refuse)?;
    journal.append(op, &c)?;
    let entry = journal.entries()[0].clone();
    assert!(entry.encoded_bytes as usize > 2 * 1024 * 1024);
    let mut recovered =
        open::<Operations14>(journal.storage.bytes.into_inner(), &c, TailRecovery::Refuse)?;
    let state = recovered.state_at(1, entry.record_digest, &c)?;
    assert_eq!(state.profile(), OperationsProfile::PagedV1_4);
    assert_eq!(state.observation().map(|o| o.items.len()), Some(40_000));
    Ok(())
}

#[test]
fn default_archive_header_and_frames_match_the_legacy_layout() -> Result<()> {
    let op = fixture()?.operations().clone();
    let c = context::<Operations13>(&op)?;
    let mut j = open::<Operations13>(Vec::new(), &c, TailRecovery::Refuse)?;
    let mut identity = b"dfmcp-operations-journal-incarnation/1\0".to_vec();
    identity.extend_from_slice(&c.session_id.get().to_be_bytes());
    identity.extend_from_slice(&c.request_id.get().to_be_bytes());
    identity.extend_from_slice(c.anchor.state_hash.as_bytes());
    let id = Digest32::of_bytes(&identity);
    let mut expected = b"DFMOJ001".to_vec();
    expected.extend_from_slice(&c.anchor.fortress_id.get().to_be_bytes());
    expected.extend_from_slice(id.as_bytes());
    let mut header_input = b"dfmcp-operations-journal-header/1\0".to_vec();
    header_input.extend_from_slice(&expected);
    let previous = Digest32::of_bytes(&header_input);
    expected.extend_from_slice(previous.as_bytes());
    assert_eq!(j.storage.bytes.get_ref(), &expected);
    let mut body = 1u64.to_be_bytes().to_vec();
    body.extend_from_slice(previous.as_bytes());
    for n in [
        c.anchor.fortress_id.get(),
        c.anchor.cursor.epoch,
        c.anchor.cursor.sequence,
        c.anchor.tick.0,
    ] {
        body.extend_from_slice(&n.to_be_bytes());
    }
    body.extend_from_slice(c.anchor.state_hash.as_bytes());
    body.extend_from_slice(op.source_digest()?.as_bytes());
    body.extend_from_slice(&op.jobs.bridge_generation.to_be_bytes());
    for text in [&op.jobs.df_version, &op.jobs.dfhack_version] {
        body.extend_from_slice(&(text.len() as u16).to_be_bytes());
        body.extend_from_slice(text.as_bytes());
    }
    let payload = op.encode_payload()?;
    body.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    body.extend_from_slice(&payload);
    let mut frame = b"DFMOREC1".to_vec();
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    let mut length_input = b"dfmcp-operations-journal-frame-header/1\0".to_vec();
    length_input.extend_from_slice(id.as_bytes());
    length_input.extend_from_slice(&frame);
    frame.extend_from_slice(Digest32::of_bytes(&length_input).as_bytes());
    frame.extend_from_slice(&body);
    let mut frame_input = b"dfmcp-operations-journal-record/1\0".to_vec();
    frame_input.extend_from_slice(id.as_bytes());
    frame_input.extend_from_slice(&frame);
    frame.extend_from_slice(Digest32::of_bytes(&frame_input).as_bytes());
    frame.extend_from_slice(b"DFMOEND1");
    expected.extend_from_slice(&frame);
    j.append(op, &c)?;
    assert_eq!(j.storage.bytes.get_ref(), &expected);
    Ok(())
}
