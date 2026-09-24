#![forbid(unsafe_code)]

//! df-action-coordinator-exec-ero.4: supplied evidence must not manufacture completion.

use dfmcp_core::{ActionId, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_intent::{ObligationRuntime, ObligationSpec, ObligationStatus};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

fn snapshot(tick: u64, sequence: u64, paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(tick),
        ObservationCursor { epoch: 0, sequence },
        paused,
        WorldGraph::default(),
    )
}

fn spec(deadline: u64, cadence: u64, samples: u32) -> ObligationSpec {
    ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(deadline),
        poll_interval_ticks: cadence,
        stable_for_observations: samples,
    }
}

fn assert_streak(runtime: &ObligationRuntime, action: ActionId, expected: u32) {
    assert!(matches!(
        runtime.get_status(action),
        Some(ObligationStatus::Active { consecutive_stable_observations, .. })
            if *consecutive_stable_observations == expected
    ));
}

#[test]
fn rejected_observation_does_not_partially_publish_in_either_action_order() -> Result<()> {
    for (eligible, future) in [(1, 2), (2, 1)] {
        let mut runtime = ObligationRuntime::new();
        let eligible = ActionId::new(eligible);
        let future = ActionId::new(future);
        runtime.register_obligation(eligible, spec(100, 1, 1), GameTick(0))?;
        runtime.register_obligation(future, spec(100, 1, 1), GameTick(20))?;
        let before_eligible = runtime.get_status(eligible).cloned();
        let before_future = runtime.get_status(future).cloned();
        let error = runtime.step_tick(&snapshot(10, 10, false));
        assert!(matches!(error, Err(error) if error.code == ErrorCode::StaleAnchor));
        assert_eq!(runtime.get_status(eligible).cloned(), before_eligible);
        assert_eq!(runtime.get_status(future).cloned(), before_future);
    }
    Ok(())
}

#[test]
fn failure_evidence_is_not_hidden_by_poll_cadence() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    let mut goal = spec(100, 10, 2);
    goal.failure = Some(Predicate::Paused(true));
    runtime.register_obligation(action, goal, GameTick(0))?;
    runtime.step_tick(&snapshot(1, 1, true))?;
    assert!(matches!(
        runtime.get_status(action),
        Some(ObligationStatus::Failed { failed_at_tick: GameTick(1), .. })
    ));
    Ok(())
}

#[test]
fn off_cadence_contradiction_resets_without_postponing_the_next_sample() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(100, 10, 2), GameTick(0))?;
    runtime.step_tick(&snapshot(10, 10, false))?;
    assert_streak(&runtime, action, 1);
    runtime.step_tick(&snapshot(11, 11, true))?;
    assert_streak(&runtime, action, 0);
    runtime.step_tick(&snapshot(20, 20, false))?;
    assert_streak(&runtime, action, 1);
    runtime.step_tick(&snapshot(30, 30, false))?;
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Fulfilled { .. })));
    Ok(())
}

#[test]
fn same_tick_changed_evidence_resets_but_cannot_add_a_sample() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(100, 10, 2), GameTick(0))?;
    runtime.step_tick(&snapshot(10, 10, false))?;
    runtime.step_tick(&snapshot(10, 11, true))?;
    assert_streak(&runtime, action, 0);
    runtime.step_tick(&snapshot(10, 12, false))?;
    assert_streak(&runtime, action, 0);
    runtime.step_tick(&snapshot(20, 20, false))?;
    assert_streak(&runtime, action, 1);
    Ok(())
}

#[test]
fn exact_replay_does_not_add_stability() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(100, 10, 2), GameTick(0))?;
    let evidence = snapshot(10, 10, false);
    runtime.step_tick(&evidence)?;
    let before = runtime.get_status(action).cloned();
    for _ in 0..16 {
        runtime.step_tick(&evidence)?;
        assert_eq!(runtime.get_status(action).cloned(), before);
    }
    Ok(())
}

#[test]
fn matching_deadline_with_insufficient_samples_is_immediately_terminal() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(15, 10, 3), GameTick(0))?;
    runtime.step_tick(&snapshot(10, 10, false))?;
    runtime.step_tick(&snapshot(15, 15, false))?;
    assert!(matches!(
        runtime.get_status(action),
        Some(ObligationStatus::Failed { failed_at_tick: GameTick(15), .. })
    ));
    Ok(())
}

#[test]
fn final_eligible_sample_at_deadline_can_complete_outside_poll_cadence() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(15, 10, 2), GameTick(0))?;
    runtime.step_tick(&snapshot(10, 10, false))?;
    let final_sample = snapshot(15, 15, false);
    runtime.step_tick(&final_sample)?;
    match runtime.get_status(action) {
        Some(ObligationStatus::Fulfilled { fulfilled_at_tick, evidence }) => {
            assert_eq!(*fulfilled_at_tick, GameTick(15));
            assert_eq!(evidence.len(), 1);
            assert_eq!(evidence[0].anchor, final_sample.anchor());
            assert_eq!(evidence[0].digest, final_sample.state_hash);
        }
        status => panic!("expected fulfilled obligation, got {status:?}"),
    }
    Ok(())
}

#[test]
fn cancellation_cannot_precede_last_evaluation() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(100, 10, 3), GameTick(0))?;
    runtime.step_tick(&snapshot(20, 20, false))?;
    let before = runtime.get_status(action).cloned();
    let result = runtime.request_cancel(action, GameTick(15));
    assert!(matches!(result, Err(error) if error.code == ErrorCode::StaleAnchor));
    assert_eq!(runtime.get_status(action).cloned(), before);
    Ok(())
}

#[test]
fn terminal_evidence_is_not_rewritten_by_later_contradictions() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(100, 1, 1), GameTick(0))?;
    runtime.step_tick(&snapshot(1, 1, false))?;
    let terminal = runtime.get_status(action).cloned();
    runtime.step_tick(&snapshot(2, 2, true))?;
    assert_eq!(runtime.get_status(action).cloned(), terminal);
    Ok(())
}

#[test]
fn invalid_hash_never_changes_obligations() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, spec(100, 1, 1), GameTick(0))?;
    let before = runtime.get_status(action).cloned();
    let mut invalid = snapshot(1, 1, false);
    invalid.state_hash = Digest32::ZERO;
    let result = runtime.step_tick(&invalid);
    assert!(matches!(result, Err(error) if error.code == ErrorCode::ChecksumMismatch));
    assert_eq!(runtime.get_status(action).cloned(), before);
    Ok(())
}
