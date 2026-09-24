use super::super::archive::{ArchiveMode, ProgressArchive};
use super::super::{ProgressManifest, ProgressObservation};
use super::*;
use crate::operations_journal::JournalStorage;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, GameTick, ObservationCursor, OperationContext,
    RequestId, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

pub(super) struct Memory(pub Cursor<Vec<u8>>);
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0.read(out)
    }
}
impl Write for Memory {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Memory {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.0.seek(from)
    }
}
impl JournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("no truncation"))
    }
}
pub(super) fn observation(
    sequence: u64,
    tick: u64,
    left: i32,
    flags: u32,
) -> Result<ProgressObservation> {
    let raw = include_str!("../../tests/fixtures/work_order_progress_v1_12.hex");
    let mut bytes = raw
        .trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text =
                std::str::from_utf8(pair).map_err(|_| error(ErrorCode::InvalidRequest, "hex"))?;
            u8::from_str_radix(text, 16).map_err(|_| error(ErrorCode::InvalidRequest, "hex"))
        })
        .collect::<Result<Vec<u8>>>()?;
    bytes[16..24].copy_from_slice(&sequence.to_be_bytes());
    bytes[24..32].copy_from_slice(&tick.to_be_bytes());
    bytes[68..72].copy_from_slice(&left.to_be_bytes());
    bytes[76..80].copy_from_slice(&flags.to_be_bytes());
    ProgressObservation::decode(&bytes, &[3, 8])
}
pub(super) fn context() -> Result<OperationContext> {
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: StateAnchor {
            fortress_id: observation(1, 10, 3, 0)?.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_entities: 4096,
            max_game_ticks: MAX_WATCH_HORIZON,
            max_bytes: 96 * 1024 * 1024,
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
    })
}
pub(super) fn manifest() -> ProgressManifest {
    ProgressManifest {
        generation: 7,
        df_version: "df".into(),
        dfhack_version: "dfhack".into(),
    }
}
pub(super) fn archive() -> Result<ProgressArchive<Memory>> {
    ProgressArchive::open(
        Memory(Cursor::new(Vec::new())),
        ArchiveMode::Live,
        true,
        &context()?,
    )
}
pub(super) fn append(
    a: &mut ProgressArchive<Memory>,
    o: &ProgressObservation,
) -> Result<ArchivedProgress> {
    let e = a.append(&manifest(), o, &context()?)?;
    a.record(e.number, e.record_digest, &context()?)
}
fn samples(timeline: &[(u64, i32, u32)]) -> Result<(Digest32, Vec<ArchivedProgress>)> {
    let mut a = archive()?;
    let mut records = Vec::new();
    for (i, &(tick, left, flags)) in timeline.iter().enumerate() {
        records.push(append(
            &mut a,
            &observation(i as u64 + 1, tick, left, flags)?,
        )?);
    }
    Ok((a.summary(&context()?)?.archive_id, records))
}
fn watch(
    id: Digest32,
    record: &ArchivedProgress,
    goal: WatchGoal,
    deadline: u64,
    cadence: u64,
    stable: u8,
) -> Result<WatchEvaluation> {
    Ok(WatchEvaluation::new(WatchDefinition::new(
        WatchSpec::new("watch-1", 3, goal, deadline, cadence, stable)?,
        id,
        record,
    )?))
}

#[test]
fn bounded_spec_and_exact_recognized_origin_are_required() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 0)])?;
    for key in ["", "two keys", "../file", "λ", &"a".repeat(65)] {
        assert!(WatchSpec::new(key, 3, WatchGoal::Validated, 20, 1, 1).is_err());
    }
    for (order, goal, cadence, stable) in [
        (u32::MAX, WatchGoal::Validated, 1, 1),
        (3, WatchGoal::RemainingAtMost(101), 1, 1),
        (3, WatchGoal::Active, 0, 1),
        (3, WatchGoal::Active, 10_001, 1),
        (3, WatchGoal::Active, 1, 0),
        (3, WatchGoal::Active, 1, 17),
    ] {
        assert!(WatchSpec::new("key", order, goal, 20, cadence, stable).is_err());
    }
    for deadline in [10, 9, 10 + MAX_WATCH_HORIZON + 1] {
        assert!(watch(id, &r[0], WatchGoal::Validated, deadline, 1, 1).is_err());
    }
    assert!(watch(id, &r[0], WatchGoal::RemainingAtMost(6), 20, 1, 1).is_err());
    assert!(watch(id, &r[0], WatchGoal::Validated, 11, 1, 2).is_err());
    assert!(
        WatchDefinition::new(
            WatchSpec::new("absent", 8, WatchGoal::Validated, 20, 1, 1)?,
            id,
            &r[0]
        )
        .is_err()
    );
    assert!(watch(Digest32::ZERO, &r[0], WatchGoal::Active, 20, 1, 1).is_err());
    Ok(())
}
#[test]
fn definition_digest_binds_every_intent_and_origin_field() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 0), (11, 3, 0)])?;
    let spec = WatchSpec::new("key", 3, WatchGoal::Validated, 30, 1, 2)?;
    let d = WatchDefinition::new(spec.clone(), id, &r[0])?;
    assert_eq!(d, WatchDefinition::new(spec, id, &r[0])?);
    let alternatives = [
        WatchSpec::new("other", 3, WatchGoal::Validated, 30, 1, 2)?,
        WatchSpec::new("key", 3, WatchGoal::Active, 30, 1, 2)?,
        WatchSpec::new("key", 3, WatchGoal::Validated, 31, 1, 2)?,
        WatchSpec::new("key", 3, WatchGoal::Validated, 30, 2, 2)?,
        WatchSpec::new("key", 3, WatchGoal::Validated, 30, 1, 3)?,
    ];
    for s in alternatives {
        assert_ne!(d.digest(), WatchDefinition::new(s, id, &r[0])?.digest());
    }
    assert_ne!(
        d.digest(),
        WatchDefinition::new(d.spec().clone(), Digest32::of_bytes(b"other"), &r[0])?.digest()
    );
    assert_ne!(
        d.digest(),
        WatchDefinition::new(d.spec().clone(), id, &r[1])?.digest()
    );
    Ok(())
}
#[test]
fn baseline_and_same_tick_reads_never_manufacture_stability() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 3), (10, 3, 3), (11, 2, 3), (11, 2, 3), (12, 1, 3)])?;
    let mut w = watch(id, &r[0], WatchGoal::RemainingAtMost(3), 20, 1, 2)?;
    assert!(w.samples().is_empty());
    for (i, expected) in [(1, 0), (2, 1), (3, 1), (4, 2)] {
        w.advance(&r[i])?;
        assert_eq!(w.samples().len(), expected);
    }
    assert_eq!(w.state(), WatchState::SatisfiedObservation);
    assert_eq!(
        w.samples(),
        &[WatchRecordRef::of(&r[2]), WatchRecordRef::of(&r[4])]
    );
    assert_eq!(w.next_sample_tick(), None);
    Ok(())
}
#[test]
fn off_cadence_negative_observation_breaks_stability() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 0), (12, 3, 1), (13, 3, 0), (14, 3, 1), (16, 3, 1)])?;
    let mut w = watch(id, &r[0], WatchGoal::Validated, 30, 2, 2)?;
    w.advance(&r[1])?;
    assert_eq!(w.samples().len(), 1);
    w.advance(&r[2])?;
    assert!(w.samples().is_empty());
    w.advance(&r[3])?;
    assert_eq!(w.state(), WatchState::Pending);
    w.advance(&r[4])?;
    assert_eq!(w.state(), WatchState::SatisfiedObservation);
    Ok(())
}
#[test]
fn deadline_is_inclusive_for_satisfaction_and_never_renews() -> Result<()> {
    for final_flags in [0, 1] {
        let (id, r) = samples(&[(10, 3, 0), (11, 3, 1), (12, 3, final_flags)])?;
        let mut w = watch(id, &r[0], WatchGoal::Validated, 12, 1, 2)?;
        w.advance(&r[1])?;
        w.advance(&r[2])?;
        assert_eq!(
            w.state(),
            if final_flags == 1 {
                WatchState::SatisfiedObservation
            } else {
                WatchState::Expired
            }
        );
    }
    let (id, r) = samples(&[(10, 3, 0), (13, 3, 1)])?;
    let mut w = watch(id, &r[0], WatchGoal::Validated, 12, 1, 1)?;
    w.advance(&r[1])?;
    assert_eq!(w.state(), WatchState::Expired);
    Ok(())
}
#[test]
fn missing_order_and_zero_counter_have_distinct_non_goods_outcomes() -> Result<()> {
    let mut a = archive()?;
    let first = append(&mut a, &observation(1, 10, 3, 1)?)?;
    let id = a.summary(&context()?)?.archive_id;
    let mut missing = observation(2, 11, 0, 1)?.canonical_bytes()[..58].to_vec();
    missing[40..44].copy_from_slice(&0u32.to_be_bytes());
    for n in [3u32, 8] {
        missing.extend_from_slice(&n.to_be_bytes());
        missing.push(0);
    }
    let absent = append(&mut a, &ProgressObservation::decode(&missing, &[3, 8])?)?;
    let mut w = watch(id, &first, WatchGoal::RemainingAtMost(0), 20, 1, 1)?;
    w.advance(&absent)?;
    assert_eq!(w.state(), WatchState::MissingOutcomeUnknown);
    assert!(w.samples().is_empty());
    let (id, r) = samples(&[(10, 3, 1), (11, 0, 1)])?;
    let mut w = watch(id, &r[0], WatchGoal::RemainingAtMost(0), 20, 1, 1)?;
    w.advance(&r[1])?;
    assert_eq!(w.state(), WatchState::SatisfiedObservation);
    assert_eq!(w.definition().spec().goal().name(), "remaining_at_most");
    Ok(())
}
#[test]
fn configuration_edit_and_increased_counter_retire_the_watch() -> Result<()> {
    for kind in 0..3 {
        let mut a = archive()?;
        let first = append(&mut a, &observation(1, 10, 3, 0)?)?;
        let mut bytes = observation(2, 11, if kind == 0 { 4 } else { 2 }, 1)?
            .canonical_bytes()
            .to_vec();
        if kind == 1 {
            bytes[72..76].copy_from_slice(&4i32.to_be_bytes());
        }
        if kind == 2 {
            bytes[67] = 0;
        }
        let next = append(&mut a, &ProgressObservation::decode(&bytes, &[3, 8])?)?;
        let mut w = watch(
            a.summary(&context()?)?.archive_id,
            &first,
            WatchGoal::Validated,
            20,
            1,
            1,
        )?;
        w.advance(&next)?;
        assert_eq!(
            w.state(),
            if kind == 0 {
                WatchState::CounterIncreased
            } else {
                WatchState::ConfigurationChanged
            }
        );
    }
    Ok(())
}
#[test]
fn discontinuity_invalidates_and_cannot_hide_behind_compatible_endpoints() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 0), (11, 3, 1), (9, 3, 1), (12, 3, 1)])?;
    let mut w = watch(id, &r[0], WatchGoal::Validated, 20, 1, 2)?;
    w.advance(&r[1])?;
    w.advance(&r[2])?;
    assert_eq!(w.state(), WatchState::ContinuityLost);
    let retired = w.clone();
    w.advance(&r[3])?;
    assert_eq!(w, retired);
    Ok(())
}
#[test]
fn skipped_replayed_or_substituted_records_do_not_partially_update() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 0), (11, 3, 1), (12, 3, 1)])?;
    let original = watch(id, &r[0], WatchGoal::Validated, 20, 1, 2)?;
    for bad in [r[0].clone(), r[2].clone()] {
        let mut w = original.clone();
        assert!(w.advance(&bad).is_err());
        assert_eq!(w, original);
    }
    let mut bad = r[1].clone();
    bad.entry.witness = Digest32::ZERO;
    let mut w = original.clone();
    assert!(w.advance(&bad).is_err());
    assert_eq!(w, original);
    bad = r[1].clone();
    bad.entry.previous_digest = Digest32::ZERO;
    assert!(w.advance(&bad).is_err());
    assert_eq!(w, original);
    Ok(())
}
#[test]
fn terminal_evidence_is_immutable_and_cancel_requires_exact_pending_frontier() -> Result<()> {
    let (id, r) = samples(&[(10, 3, 0), (11, 3, 1), (12, 3, 0)])?;
    let mut pending = watch(id, &r[0], WatchGoal::Validated, 20, 1, 2)?;
    assert!(pending.cancel_at(WatchRecordRef::of(&r[1])).is_err());
    pending.cancel_at(WatchRecordRef::of(&r[0]))?;
    pending.cancel_at(WatchRecordRef::of(&r[0]))?;
    let cancelled = pending.clone();
    pending.advance(&r[1])?;
    assert_eq!(pending, cancelled);
    let mut success = watch(id, &r[0], WatchGoal::Validated, 20, 1, 1)?;
    success.advance(&r[1])?;
    assert!(success.cancel_at(WatchRecordRef::of(&r[1])).is_err());
    let terminal = success.clone();
    success.advance(&r[2])?;
    assert_eq!(success, terminal);
    Ok(())
}
#[test]
fn exhaustive_six_tick_truth_schedules_match_independent_run_length_oracle() -> Result<()> {
    for bits in 0..64u32 {
        let mut timeline = vec![(10, 3, 0)];
        for n in 0..6 {
            timeline.push((11 + n, 3, u32::from(bits & (1 << n) != 0)));
        }
        let (id, records) = samples(&timeline)?;
        for required in 1..=3 {
            let mut w = watch(id, &records[0], WatchGoal::Validated, 16, 1, required)?;
            let mut run = 0;
            let mut satisfied = None;
            for n in 0..6 {
                if bits & (1 << n) != 0 {
                    run += 1;
                } else {
                    run = 0;
                }
                if run >= required && satisfied.is_none() {
                    satisfied = Some(n + 1);
                }
                w.advance(&records[n + 1])?;
            }
            assert_eq!(
                w.state(),
                if satisfied.is_some() {
                    WatchState::SatisfiedObservation
                } else {
                    WatchState::Expired
                }
            );
            if let Some(index) = satisfied {
                assert_eq!(w.through(), WatchRecordRef::of(&records[index]));
            }
        }
    }
    Ok(())
}
