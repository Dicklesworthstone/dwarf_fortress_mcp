use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, ObservationCursor, RequestId, StateAnchor, WorkBudget,
};
use std::cell::Cell;
use std::collections::VecDeque;
use std::rc::Rc;

const VECTOR: &str = include_str!("../../tests/fixtures/work_order_progress_v1_12.hex");
fn hex(s: &str) -> Result<Vec<u8>> {
    s.trim()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let s = std::str::from_utf8(pair)
                .map_err(|_| error(ErrorCode::InvalidRequest, "test hex"))?;
            u8::from_str_radix(s, 16).map_err(|_| error(ErrorCode::InvalidRequest, "test hex"))
        })
        .collect()
}
fn fixture() -> Result<Vec<u8>> {
    hex(VECTOR)
}
fn observation(sequence: u64, tick: u64, left: i32) -> Result<ProgressObservation> {
    let mut data = fixture()?;
    data[16..24].copy_from_slice(&sequence.to_be_bytes());
    data[24..32].copy_from_slice(&tick.to_be_bytes());
    data[68..72].copy_from_slice(&left.to_be_bytes());
    ProgressObservation::decode(&data, &[3, 8])
}
fn context() -> Result<OperationContext> {
    let o = observation(1, 12345, 3)?;
    Ok(OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(2),
        anchor: StateAnchor {
            fortress_id: o.fortress_id(),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(0),
            state_hash: Digest32::ZERO,
        },
        budget: WorkBudget {
            max_entities: 4096,
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
struct Source {
    manifest: ProgressManifest,
    replies: VecDeque<Result<ProgressObservation>>,
    calls: Rc<Cell<u32>>,
    fenced: bool,
}
impl Source {
    fn new(replies: Vec<Result<ProgressObservation>>) -> Self {
        Self {
            manifest: ProgressManifest {
                generation: 7,
                df_version: "df".into(),
                dfhack_version: "dfhack".into(),
            },
            replies: replies.into(),
            calls: Rc::new(Cell::new(0)),
            fenced: false,
        }
    }
}
impl ProgressSource for Source {
    fn manifest(&self) -> &ProgressManifest {
        &self.manifest
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn read(&mut self, _: &[u32], _: Duration) -> Result<ProgressObservation> {
        self.calls.set(self.calls.get() + 1);
        if self.fenced {
            return Err(error(ErrorCode::AdapterUnavailable, "fenced"));
        }
        self.replies
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "no reply"))?
    }
}
#[test]
fn actual_native_fixture_is_complete_and_owned() -> Result<()> {
    let raw = fixture()?;
    let o = ProgressObservation::decode(&raw, &[3, 8])?;
    assert_eq!(o.canonical_bytes(), raw);
    assert_eq!(o.tick(), 12345);
    assert_eq!(o.queue_count(), 1);
    assert_eq!(o.rows()[0].phase(), "active");
    assert_eq!(o.rows()[1].phase(), "absent");
    let status = o.rows()[0]
        .order
        .as_ref()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture absent"))?;
    assert_eq!(status.remaining, 3);
    assert!(status.validated() && status.active());
    assert_eq!(status.recipe_name(), Some("wooden_bed"));
    Ok(())
}
#[test]
fn every_truncated_extent_and_trailing_bytes_are_refused() -> Result<()> {
    let raw = fixture()?;
    for n in 0..raw.len() {
        assert!(ProgressObservation::decode(&raw[..n], &[3, 8]).is_err());
    }
    let mut bad = raw;
    bad.push(0);
    assert!(ProgressObservation::decode(&bad, &[3, 8]).is_err());
    Ok(())
}
#[test]
fn selection_substitution_omission_and_bad_header_are_refused() -> Result<()> {
    let raw = fixture()?;
    for ids in [
        vec![],
        vec![3],
        vec![8, 3],
        vec![3, 3],
        vec![3, 9],
        vec![u32::MAX],
    ] {
        assert!(ProgressObservation::decode(&raw, &ids).is_err());
    }
    for (offset, value) in [(8, 0u64), (8, u64::MAX), (16, 0), (16, u64::MAX)] {
        let mut bad = raw.clone();
        bad[offset..offset + 8].copy_from_slice(&value.to_be_bytes());
        assert!(ProgressObservation::decode(&bad, &[3, 8]).is_err());
    }
    let mut bad = raw;
    bad[40..44].copy_from_slice(&0u32.to_be_bytes());
    assert!(ProgressObservation::decode(&bad, &[3, 8]).is_err());
    Ok(())
}
#[test]
fn recognized_templates_cannot_hide_counter_or_status_corruption() -> Result<()> {
    let raw = fixture()?;
    for (offset, value) in [
        (68, 6i32),
        (68, -1),
        (72, 0),
        (72, 101),
        (76, 4),
        (80, 1),
        (84, 0),
        (88, 0),
        (100, 1),
        (104, 1),
    ] {
        let mut bad = raw.clone();
        bad[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
        assert!(ProgressObservation::decode(&bad, &[3, 8]).is_err());
        bad[67] = 0; // unknown native configuration remains explicitly unrecognized
        let o = ProgressObservation::decode(&bad, &[3, 8])?;
        assert_eq!(o.rows()[0].phase(), "unrecognized_or_modified");
    }
    let mut bad = raw;
    bad[67] = 2;
    assert!(ProgressObservation::decode(&bad, &[3, 8]).is_err());
    Ok(())
}
#[test]
fn progression_and_increased_counters_are_distinct() -> Result<()> {
    let a = observation(1, 10, 5)?;
    let b = observation(2, 20, 2)?;
    let c = compare(Some(&a), &b)?;
    assert_eq!(c.elapsed_game_ticks, Some(10));
    assert_eq!(c.changes[0].kind, "remaining_counter_decreased");
    assert_eq!(c.changes[0].remaining_decrease, Some(3));
    let d = compare(Some(&b), &observation(3, 21, 4)?)?;
    assert_eq!(d.changes[0].kind, "remaining_counter_increased");
    assert_eq!(d.changes[0].remaining_increase, Some(2));
    Ok(())
}
#[test]
fn disappearance_is_unknown_not_completion_and_reappearance_is_unlinked() -> Result<()> {
    let a = observation(1, 10, 1)?;
    let mut bytes = a.canonical_bytes()[..58].to_vec();
    bytes[16..24].copy_from_slice(&2u64.to_be_bytes());
    bytes[40..44].copy_from_slice(&0u32.to_be_bytes());
    for id in [3u32, 8] {
        bytes.extend_from_slice(&id.to_be_bytes());
        bytes.push(0);
    }
    let absent = ProgressObservation::decode(&bytes, &[3, 8])?;
    let c = compare(Some(&a), &absent)?;
    assert_eq!(c.changes[0].kind, "disappeared_outcome_unknown");
    assert_eq!(c.changes[0].remaining_decrease, None);
    let back = compare(Some(&absent), &observation(3, 12, 5)?)?;
    assert_eq!(back.changes[0].kind, "appeared_identity_unlinked");
    Ok(())
}
#[test]
fn config_changes_disable_counter_comparison_and_zero_is_only_reported() -> Result<()> {
    let a = observation(1, 10, 3)?;
    let mut raw = observation(2, 11, 1)?.canonical_bytes().to_vec();
    raw[72..76].copy_from_slice(&4i32.to_be_bytes());
    let b = ProgressObservation::decode(&raw, &[3, 8])?;
    let c = compare(Some(&a), &b)?;
    assert_eq!(c.changes[0].kind, "configuration_changed");
    assert_eq!(c.changes[0].remaining_decrease, None);
    assert_eq!(
        observation(3, 12, 0)?.rows()[0].phase(),
        "reported_zero_remaining"
    );
    Ok(())
}
#[test]
fn epoch_clock_horizon_and_sequence_fences_never_manufacture_progress() -> Result<()> {
    let a = observation(2, 10, 3)?;
    assert!(compare(Some(&a), &a).is_err());
    assert!(compare(Some(&a), &observation(1, 9, 1)?).is_err());
    let backwards = compare(Some(&a), &observation(3, 9, 1)?)?;
    assert_eq!(backwards.status, "reset");
    assert!(backwards.changes.is_empty());
    assert_eq!(backwards.baseline, None);
    let mut raw = observation(3, 11, 1)?.canonical_bytes().to_vec();
    raw[8..16].copy_from_slice(&8u64.to_be_bytes());
    assert_eq!(
        compare(Some(&a), &ProgressObservation::decode(&raw, &[3, 8])?)?.status,
        "reset"
    );
    raw[8..16].copy_from_slice(&7u64.to_be_bytes());
    raw[36..40].copy_from_slice(&9u32.to_be_bytes());
    assert_eq!(
        compare(Some(&a), &ProgressObservation::decode(&raw, &[3, 8])?)?.status,
        "reset"
    );
    Ok(())
}
#[test]
fn failed_refresh_clears_old_selection_and_fences_source() -> Result<()> {
    let c = context()?;
    let source = Source::new(vec![
        observation(1, 10, 3),
        Err(error(ErrorCode::AdapterUnavailable, "lost")),
    ]);
    let calls = source.calls.clone();
    let mut s = ProgressSession::new(source, &c)?;
    s.refresh(&[3, 8], &c)?;
    assert!(s.current(&c)?.is_some());
    assert!(s.refresh(&[3, 8], &c).is_err());
    assert!(s.current(&c)?.is_none());
    assert_eq!(calls.get(), 2);
    assert!(s.source.fenced);
    Ok(())
}
#[test]
fn budgets_scope_and_cancellation_reject_before_native_work() -> Result<()> {
    let c = context()?;
    let source = Source::new(vec![observation(1, 10, 3)]);
    let calls = source.calls.clone();
    let mut s = ProgressSession::new(source, &c)?;
    let mut denied = c.clone();
    denied.budget.max_entities = 4095;
    assert!(s.refresh(&[3, 8], &denied).is_err());
    denied = c.clone();
    denied.budget.max_bytes = RPC_BYTE_RESERVE - 1;
    assert!(s.refresh(&[3, 8], &denied).is_err());
    denied = c.clone();
    denied.cancellation_requested = true;
    assert!(s.refresh(&[3, 8], &denied).is_err());
    denied = c.clone();
    denied.session_id = SessionId::new(2);
    assert!(s.refresh(&[3, 8], &denied).is_err());
    denied = c.clone();
    denied.grants.retain(|g| g.capability == Capability::Query);
    assert!(s.refresh(&[3, 8], &denied).is_err());
    assert_eq!(calls.get(), 0);
    Ok(())
}
#[test]
fn grant_expiry_uses_new_and_last_known_ticks_not_a_stale_caller_tick() -> Result<()> {
    let c = context()?;
    let source = Source::new(vec![observation(1, 10, 3), observation(2, 21, 2)]);
    let mut s = ProgressSession::new(source, &c)?;
    s.refresh(&[3, 8], &c)?;
    let mut expired = c.clone();
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(9));
    }
    assert!(s.current(&expired).is_err());
    for grant in &mut expired.grants {
        grant.expires_at_tick = Some(GameTick(20));
    }
    assert!(s.refresh(&[3, 8], &expired).is_err());
    assert!(s.current(&c)?.is_none());
    Ok(())
}
#[test]
fn unchanged_counters_and_same_tick_changes_remain_sampled_endpoints() -> Result<()> {
    let a = observation(1, 10, 3)?;
    let unchanged = compare(Some(&a), &observation(2, 10, 3)?)?;
    assert!(unchanged.changes.is_empty());
    assert_eq!(unchanged.status, "compared");
    let changed = compare(Some(&a), &observation(3, 10, 2)?)?;
    assert_eq!(changed.elapsed_game_ticks, Some(0));
    assert_eq!(changed.changes[0].remaining_decrease, Some(1));
    Ok(())
}
