//! Original RegionTerrain, cadence/stability and independently reauthorized history.
use super::fixtures::*;
use dfmcp_adapter::dig_designation::journal::DigMode;
use dfmcp_adapter::dig_designation::{DigObservation, DigPlan};
use dfmcp_adapter::live_spatial::citizens::LiveSpatialCitizenObservation;
use dfmcp_adapter::semantic_dig::{
    DigGoalProjection, DigGoalResult, DigGoalSource, DurableDigGoalMonitor, GoalHistoryStore,
    SemanticDigReview,
};
use dfmcp_core::{Capability, GameTick, ObservationCursor, OperationContext, Result};
use dfmcp_world::map_region::{Cell, Shape};
use dfmcp_world::{EvidencePolicy, PredicateTruth};

struct Source(LiveSpatialCitizenObservation);
impl DigGoalSource for Source {
    fn read_excavation(
        &mut self,
        _: &DigPlan,
        _: &OperationContext,
    ) -> Result<LiveSpatialCitizenObservation> {
        Ok(self.0.clone())
    }
}
fn granted(p: &DigGoalProjection, _: &OperationContext) -> Result<EvidencePolicy> {
    let mut policy = EvidencePolicy::at(p.anchor());
    policy.paused = true;
    policy.terrain_regions = p.terrain_regions().to_vec();
    Ok(policy)
}
fn designated(stable: u32) -> Result<(Fixture, SemanticDigReview)> {
    let mut f = Fixture::new()?;
    f.original = reseal(&f.original, |p| {
        if let Some(o) = &mut p.steps[0].obligation {
            o.stable_for_observations = stable;
            o.poll_interval_ticks = 1;
        }
    });
    let review = f.prepared()?;
    f.session.commit(
        review.seal(),
        true,
        &mut f.evidence,
        &f.context,
        &mut f.guard,
    )?;
    f.context
        .grants
        .retain(|g| matches!(g.capability, Capability::Query | Capability::Observe));
    Ok((f, review))
}
fn cursor(f: &Fixture, n: u64) -> ObservationCursor {
    ObservationCursor {
        epoch: f.original.anchor.cursor.epoch,
        sequence: f.original.anchor.cursor.sequence + n,
    }
}
fn sample(
    capture: &DigObservation,
    tick: u64,
    mut change: impl FnMut(&mut Cell),
) -> Result<LiveSpatialCitizenObservation> {
    goal_observation(capture, tick, |m| {
        let [x, y, z, w, h] = capture.region().coordinates();
        for yy in y..y + h {
            for xx in x..x + w {
                if let Some(i) = m.map.region.index([xx, yy, z]) {
                    change(&mut m.map.cells[i]);
                }
            }
        }
    })
}
fn floors(c: &DigObservation, t: u64) -> Result<LiveSpatialCitizenObservation> {
    sample(c, t, |cell| {
        if let Cell::Visible(tile) = cell {
            tile.shape = Shape::Floor;
            tile.dig_designation = 0;
        }
    })
}
fn durable(
    f: &mut Fixture,
    review: &SemanticDigReview,
    disk: Memory,
) -> Result<DurableDigGoalMonitor<Memory>> {
    let id = f.session.inventory(&f.context)?.journal_id;
    let history = GoalHistoryStore::open(disk, id, review, true, false, &f.context)?;
    f.session
        .begin_goal_monitor_durable(review.seal(), history, &f.context, granted)
}
fn poll(
    f: &mut Fixture,
    m: &mut DurableDigGoalMonitor<Memory>,
    n: u64,
    raw: LiveSpatialCitizenObservation,
) -> Result<DigGoalResult> {
    let cursor = cursor(f, n);
    f.session.poll_original_goal_durable(
        m,
        cursor,
        &f.context,
        move |_, _| Ok(Source(raw)),
        granted,
    )
}
#[test]
fn original_floor_goal_requires_current_evidence_and_original_stability() -> Result<()> {
    let (mut f, r) = designated(2)?;
    let mut m = durable(&mut f, &r, Memory::default())?;
    let pending = sample(&f.capture, f.capture.tick() + 1, |_| {})?;
    let result = poll(&mut f, &mut m, 1, pending)?;
    assert_eq!(result.progress().current_goal, PredicateTruth::False);
    assert!(!result.original_goal_proven());
    let raw = floors(&f.capture, f.capture.tick() + 2)?;
    let result = poll(&mut f, &mut m, 2, raw)?;
    assert!(result.progress().excavation_proven);
    assert!(!result.original_goal_proven());
    let raw = floors(&f.capture, f.capture.tick() + 3)?;
    let result = poll(&mut f, &mut m, 3, raw)?;
    assert!(result.semantic_completion_proven());
    assert_eq!(m.original_plan(), &f.original);
    assert_eq!(f.native.borrow().commits, 1);
    assert_eq!(f.native.borrow().queries, 0);
    Ok(())
}
#[test]
fn hidden_wet_designated_and_unsupported_targets_never_prove_original_floor() -> Result<()> {
    for kind in 0..4 {
        let (mut f, r) = designated(1)?;
        let mut m = durable(&mut f, &r, Memory::default())?;
        let raw = sample(&f.capture, f.capture.tick() + 1, |cell| {
            if kind == 0 {
                *cell = Cell::Hidden;
                return;
            }
            if let Cell::Visible(tile) = cell {
                tile.shape = Shape::Floor;
                match kind {
                    1 => tile.liquid_depth = 1,
                    2 => tile.dig_designation = 1,
                    _ => tile.shape = Shape::Other,
                }
            }
        })?;
        let result = poll(&mut f, &mut m, 1, raw)?;
        assert_eq!(result.progress().current_goal, PredicateTruth::Unknown);
        assert!(!result.original_goal_proven());
        assert!(!result.progress().excavation_proven);
    }
    Ok(())
}
#[test]
fn hashes_without_terrain_policy_grants_cannot_prove_completed_mining() -> Result<()> {
    let (mut f, r) = designated(1)?;
    let mut m = durable(&mut f, &r, Memory::default())?;
    let raw = floors(&f.capture, f.capture.tick() + 1)?;
    let c = cursor(&f, 1);
    let result = f.session.poll_original_goal_durable(
        &mut m,
        c,
        &f.context,
        move |_, _| Ok(Source(raw)),
        |p, _| Ok(EvidencePolicy::at(p.anchor())),
    )?;
    assert_eq!(result.progress().current_goal, PredicateTruth::Unknown);
    assert!(!result.original_goal_proven());
    Ok(())
}
#[test]
fn restart_keeps_original_deadline_but_breaks_unfinished_stability() -> Result<()> {
    let (mut f, r) = designated(2)?;
    let disk = Memory::default();
    let mut m = durable(&mut f, &r, disk.clone())?;
    let raw = floors(&f.capture, f.capture.tick() + 1)?;
    assert!(!poll(&mut f, &mut m, 1, raw)?.original_goal_proven());
    let high = m.high_tick();
    let deadline = f.original.steps[0]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("obligation"))?
        .deadline_tick;
    drop(m);
    f.context.anchor.tick = high;
    f.session = reopen(
        f.native_disk.clone(),
        f.associations.clone(),
        &f.binding,
        DigMode::Recover,
        false,
        &f.context,
    )?;
    let review = f.session.reattach(f.original.clone(), &f.context)?;
    let id = f.session.inventory(&f.context)?.journal_id;
    let history = GoalHistoryStore::open(disk, id, &review, false, false, &f.context)?;
    let mut m =
        f.session
            .begin_goal_monitor_durable(review.seal(), history, &f.context, granted)?;
    assert!(m.latest().is_none());
    assert_eq!(
        m.original_plan().steps[0]
            .obligation
            .as_ref()
            .ok_or_else(|| invalid("obligation"))?
            .deadline_tick,
        deadline
    );
    let raw = floors(&f.capture, f.capture.tick() + 2)?;
    assert!(!poll(&mut f, &mut m, 2, raw)?.original_goal_proven());
    let raw = floors(&f.capture, f.capture.tick() + 3)?;
    assert!(poll(&mut f, &mut m, 3, raw)?.original_goal_proven());
    assert_eq!(f.native.borrow().commits, 1);
    Ok(())
}
#[test]
fn denied_policy_consumes_clock_and_cursor_without_banking_a_stable_sample() -> Result<()> {
    let (mut f, r) = designated(2)?;
    let mut m = durable(&mut f, &r, Memory::default())?;
    let raw = floors(&f.capture, f.capture.tick() + 1)?;
    assert!(!poll(&mut f, &mut m, 1, raw)?.original_goal_proven());
    let raw = floors(&f.capture, f.capture.tick() + 2)?;
    let c = cursor(&f, 2);
    assert!(
        f.session
            .poll_original_goal_durable(
                &mut m,
                c,
                &f.context,
                move |_, _| Ok(Source(raw)),
                |_, _| Err(invalid("independent source policy denied"))
            )
            .is_err()
    );
    assert_eq!(m.high_tick(), GameTick(f.capture.tick() + 2));
    assert!(m.latest().is_none());
    let raw = floors(&f.capture, f.capture.tick() + 3)?;
    assert!(!poll(&mut f, &mut m, 3, raw)?.original_goal_proven());
    let raw = floors(&f.capture, f.capture.tick() + 4)?;
    assert!(poll(&mut f, &mut m, 4, raw)?.original_goal_proven());
    Ok(())
}
#[test]
fn changed_spatial_incarnation_fences_exact_original_goal_even_for_identical_floor() -> Result<()> {
    let (mut f, r) = designated(1)?;
    let mut m = durable(&mut f, &r, Memory::default())?;
    let raw = goal_observation(&f.capture, f.capture.tick() + 1, |map| {
        map.bridge_generation += 1;
        for cell in &mut map.map.cells {
            if let Cell::Visible(tile) = cell {
                tile.shape = Shape::Floor;
            }
        }
    })?;
    assert!(poll(&mut f, &mut m, 1, raw).is_err());
    assert!(m.is_fenced());
    assert!(m.latest().is_none());
    assert!(f.session.begin_goal_monitor(r.seal(), &f.context).is_err());
    Ok(())
}
#[test]
fn durable_history_tampering_and_missing_historical_authority_block_recovery() -> Result<()> {
    let (mut f, r) = designated(1)?;
    let disk = Memory::default();
    let mut m = durable(&mut f, &r, disk.clone())?;
    let raw = floors(&f.capture, f.capture.tick() + 1)?;
    assert!(poll(&mut f, &mut m, 1, raw)?.original_goal_proven());
    drop(m);
    let id = f.session.inventory(&f.context)?.journal_id;
    let history = GoalHistoryStore::open(disk.clone(), id, &r, false, false, &f.context)?;
    assert!(
        f.session
            .begin_goal_monitor_durable(r.seal(), history, &f.context, |_, _| Err(invalid(
                "historical authority unavailable"
            )))
            .is_err()
    );
    disk.corrupt();
    assert!(GoalHistoryStore::open(disk, id, &r, false, false, &f.context).is_err());
    Ok(())
}
#[test]
fn original_deadline_is_not_extended_by_late_completed_terrain() -> Result<()> {
    let (mut f, r) = designated(1)?;
    let deadline = f.original.steps[0]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("obligation"))?
        .deadline_tick;
    let mut m = durable(&mut f, &r, Memory::default())?;
    let raw = floors(&f.capture, deadline.get() + 1)?;
    let result = poll(&mut f, &mut m, 1, raw)?;
    assert_eq!(result.progress().current_goal, PredicateTruth::True);
    assert!(!result.original_goal_proven());
    assert!(matches!(
        result.progress().obligation,
        Some(dfmcp_intent::ObligationStatus::Failed { .. })
    ));
    Ok(())
}
