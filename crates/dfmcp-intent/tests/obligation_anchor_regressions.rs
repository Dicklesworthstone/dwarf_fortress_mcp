#![forbid(unsafe_code)]

//! df-action-coordinator-exec-ero.4: do not combine evidence across observation lineages.

use dfmcp_core::{ActionId, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_intent::{ObligationRuntime, ObligationSpec, ObligationStatus};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

fn sample(fortress: u64, epoch: u64, sequence: u64, tick: u64, paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(fortress),
        GameTick(tick),
        ObservationCursor { epoch, sequence },
        paused,
        WorldGraph::default(),
    )
}

fn goal() -> ObligationSpec {
    ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(100),
        poll_interval_ticks: 10,
        stable_for_observations: 2,
    }
}

fn anchored() -> Result<(ObligationRuntime, ActionId)> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation_at(action, goal(), &sample(1, 0, 0, 0, false))?;
    Ok((runtime, action))
}

fn assert_streak(runtime: &ObligationRuntime, action: ActionId, expected: u32) {
    assert!(matches!(
        runtime.get_status(action),
        Some(ObligationStatus::Active { consecutive_stable_observations, .. })
            if *consecutive_stable_observations == expected
    ));
}

#[test]
fn creation_anchor_fences_the_first_observation() -> Result<()> {
    for (fortress, epoch) in [(2, 0), (1, 1)] {
        let (mut runtime, action) = anchored()?;
        let before = runtime.last_observation_anchor(action);
        let result = runtime.step_tick(&sample(fortress, epoch, 10, 10, false));
        assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
        assert_eq!(runtime.last_observation_anchor(action), before);
        assert_streak(&runtime, action, 0);
    }
    Ok(())
}

#[test]
fn legacy_registration_binds_on_first_accepted_sample() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, goal(), GameTick(0))?;
    assert_eq!(runtime.last_observation_anchor(action), None);
    let first = sample(1, 7, 10, 10, false);
    runtime.step_tick(&first)?;
    assert_eq!(runtime.last_observation_anchor(action), Some(first.anchor()));
    let result = runtime.step_tick(&sample(2, 7, 20, 20, false));
    assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
    assert_streak(&runtime, action, 1);
    Ok(())
}

#[test]
fn cursor_forks_and_sequence_regressions_cannot_supply_evidence() -> Result<()> {
    let (mut runtime, action) = anchored()?;
    let first = sample(1, 0, 10, 10, false);
    runtime.step_tick(&first)?;
    let before = runtime.get_status(action).cloned();
    for invalid in [
        sample(1, 0, 10, 10, true),
        sample(1, 0, 10, 20, false),
        sample(1, 0, 9, 20, false),
        sample(1, 0, 11, 9, false),
        sample(1, 1, 20, 20, false),
    ] {
        let result = runtime.step_tick(&invalid);
        assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
        assert_eq!(runtime.last_observation_anchor(action), Some(first.anchor()));
        assert_eq!(runtime.get_status(action).cloned(), before);
    }
    Ok(())
}

#[test]
fn off_cadence_reads_advance_the_observation_fence_not_the_poll_clock() -> Result<()> {
    let (mut runtime, action) = anchored()?;
    runtime.step_tick(&sample(1, 0, 10, 10, false))?;
    let off_cadence = sample(1, 0, 18, 18, false);
    runtime.step_tick(&off_cadence)?;
    assert_eq!(runtime.last_observation_anchor(action), Some(off_cadence.anchor()));
    let before = runtime.get_status(action).cloned();
    let result = runtime.step_tick(&sample(1, 0, 19, 17, true));
    assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
    assert_eq!(runtime.get_status(action).cloned(), before);
    runtime.step_tick(&sample(1, 0, 20, 20, false))?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Fulfilled { .. })));
    Ok(())
}

#[test]
fn cancellation_cannot_be_backdated_before_an_off_cadence_read() -> Result<()> {
    let (mut runtime, action) = anchored()?;
    runtime.step_tick(&sample(1, 0, 8, 8, false))?;
    let result = runtime.request_cancel(action, GameTick(7));
    assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
    assert_streak(&runtime, action, 0);
    runtime.request_cancel(action, GameTick(8))?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Draining { .. })));
    Ok(())
}

#[test]
fn one_incompatible_action_cannot_publish_another_actions_anchor() -> Result<()> {
    for (eligible, incompatible) in [(1, 2), (2, 1)] {
        let mut runtime = ObligationRuntime::new();
        let eligible = ActionId::new(eligible);
        let incompatible = ActionId::new(incompatible);
        let first = sample(1, 0, 0, 0, false);
        let other = sample(2, 0, 0, 0, false);
        runtime.register_obligation_at(eligible, goal(), &first)?;
        runtime.register_obligation_at(incompatible, goal(), &other)?;
        let result = runtime.step_tick(&sample(1, 0, 10, 10, false));
        assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
        assert_eq!(runtime.last_observation_anchor(eligible), Some(first.anchor()));
        assert_eq!(runtime.last_observation_anchor(incompatible), Some(other.anchor()));
        assert_streak(&runtime, eligible, 0);
        assert_streak(&runtime, incompatible, 0);
    }
    Ok(())
}

#[test]
fn duplicate_registration_cannot_rebind_existing_work() -> Result<()> {
    let (mut runtime, action) = anchored()?;
    let before = runtime.last_observation_anchor(action);
    let result = runtime.register_obligation_at(action, goal(), &sample(2, 0, 1, 1, false));
    assert!(matches!(result, Err(error) if error.code == ErrorCode::Conflict));
    assert_eq!(runtime.last_observation_anchor(action), before);
    assert_eq!(runtime.obligation_count(), 1);
    Ok(())
}

#[test]
fn invalid_creation_snapshot_does_not_reserve_an_identity() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    let mut invalid = sample(1, 0, 0, 0, false);
    invalid.state_hash = Digest32::ZERO;
    let result = runtime.register_obligation_at(action, goal(), &invalid);
    assert!(matches!(result, Err(error) if error.code == ErrorCode::ChecksumMismatch));
    assert_eq!(runtime.obligation_count(), 0);
    assert_eq!(runtime.last_observation_anchor(action), None);
    runtime.register_obligation_at(action, goal(), &sample(1, 0, 0, 0, false))?;
    Ok(())
}

#[test]
fn interrupted_read_resets_stability_without_recounting_old_evidence() -> Result<()> {
    let (mut runtime, action) = anchored()?;
    let first = sample(1, 0, 10, 10, false);
    runtime.step_tick(&first)?;
    assert_streak(&runtime, action, 1);
    runtime.observation_interrupted(action)?;
    runtime.observation_interrupted(action)?;
    runtime.step_tick(&first)?;
    assert_streak(&runtime, action, 0);
    assert_eq!(runtime.last_observation_anchor(action), Some(first.anchor()));
    runtime.step_tick(&sample(1, 0, 20, 20, false))?;
    assert_streak(&runtime, action, 1);
    runtime.step_tick(&sample(1, 0, 30, 30, false))?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Fulfilled { .. })));
    Ok(())
}

#[test]
fn interruption_does_not_extend_a_fixed_deadline_or_rewrite_terminal_evidence() -> Result<()> {
    let (mut runtime, action) = anchored()?;
    runtime.step_tick(&sample(1, 0, 90, 90, false))?;
    runtime.observation_interrupted(action)?;
    runtime.step_tick(&sample(1, 0, 100, 100, false))?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Failed { .. })));
    let before = runtime.get_status(action).cloned();
    let anchor = runtime.last_observation_anchor(action);
    runtime.observation_interrupted(action)?;
    runtime.step_tick(&sample(2, 1, 200, 200, false))?;
    assert_eq!(runtime.get_status(action).cloned(), before);
    assert_eq!(runtime.last_observation_anchor(action), anchor);
    let unknown = runtime.observation_interrupted(ActionId::new(99));
    assert!(matches!(unknown, Err(error) if error.code == ErrorCode::InvalidRequest));
    Ok(())
}
