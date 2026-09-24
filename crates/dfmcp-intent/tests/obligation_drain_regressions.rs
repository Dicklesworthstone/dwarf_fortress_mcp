#![forbid(unsafe_code)]

//! df-action-coordinator-exec-ero.4: cancellation progress conserves registered work.

use dfmcp_core::{ActionId, ErrorCode, GameTick, Result};
use dfmcp_intent::{DrainProgressCertificate, ObligationRuntime, ObligationSpec, ObligationStatus};
use dfmcp_world::Predicate;

fn goal() -> ObligationSpec {
    ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(100),
        poll_interval_ticks: 1,
        stable_for_observations: 2,
    }
}

fn draining(steps: usize) -> Result<(ObligationRuntime, ActionId)> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, goal(), GameTick(0))?;
    runtime.request_cancel_with_steps(action, GameTick(10), steps)?;
    Ok((runtime, action))
}

fn progress(action: ActionId, tick: u64, done: usize, remaining: usize, quiet: bool)
    -> DrainProgressCertificate
{
    DrainProgressCertificate {
        action_id: action,
        drain_started_tick: GameTick(10),
        current_tick: GameTick(tick),
        steps_compensated: done,
        steps_remaining: remaining,
        is_quiescent: quiet,
    }
}

#[test]
fn multi_step_compensation_waits_for_both_all_work_and_quiescence() -> Result<()> {
    let (mut runtime, action) = draining(3)?;
    let first = progress(action, 11, 1, 2, false);
    runtime.record_drain_progress(&first)?;
    assert_eq!(runtime.get_drain_progress(action), Some(&first));
    assert!(runtime.finalize_cancel(action, GameTick(11), &first).is_err());
    let all_done = progress(action, 12, 3, 0, false);
    runtime.record_drain_progress(&all_done)?;
    assert!(runtime.finalize_cancel(action, GameTick(12), &all_done).is_err());
    let quiet = progress(action, 13, 3, 0, true);
    runtime.record_drain_progress(&quiet)?;
    runtime.finalize_cancel(action, GameTick(13), &quiet)?;
    assert!(matches!(
        runtime.get_status(action),
        Some(ObligationStatus::Cancelled { cancelled_at_tick: GameTick(13) })
    ));
    assert_eq!(runtime.get_drain_progress(action), Some(&quiet));
    Ok(())
}

#[test]
fn finalization_cannot_skip_progress_registration() -> Result<()> {
    let (mut runtime, action) = draining(3)?;
    let claimed = progress(action, 20, 3, 0, true);
    let before = runtime.get_drain_progress(action).cloned();
    let result = runtime.finalize_cancel(action, GameTick(20), &claimed);
    assert!(matches!(result, Err(error) if error.code == ErrorCode::CancellationIncomplete));
    assert_eq!(runtime.get_drain_progress(action).cloned(), before);
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Draining { .. })));
    Ok(())
}

#[test]
fn malformed_inventory_or_quiescence_is_rejected_atomically() -> Result<()> {
    let (mut runtime, action) = draining(3)?;
    let before = runtime.get_drain_progress(action).cloned();
    for bad in [
        progress(action, 11, 0, 0, true),
        progress(action, 11, 2, 2, false),
        progress(action, 11, 1, 2, true),
        progress(action, 11, usize::MAX, 1, false),
        progress(action, 11, 65_536, 1, false),
    ] {
        let result = runtime.record_drain_progress(&bad);
        assert!(matches!(result, Err(error) if error.code == ErrorCode::CancellationIncomplete));
        assert_eq!(runtime.get_drain_progress(action).cloned(), before);
    }
    Ok(())
}

#[test]
fn progress_counts_ticks_and_quiescence_cannot_regress() -> Result<()> {
    let (mut runtime, action) = draining(3)?;
    let first = progress(action, 20, 2, 1, false);
    runtime.record_drain_progress(&first)?;
    for bad in [progress(action, 21, 1, 2, false), progress(action, 19, 3, 0, true)] {
        assert!(runtime.record_drain_progress(&bad).is_err());
        assert_eq!(runtime.get_drain_progress(action), Some(&first));
    }
    let quiet = progress(action, 21, 3, 0, true);
    runtime.record_drain_progress(&quiet)?;
    assert!(runtime.record_drain_progress(&progress(action, 22, 3, 0, false)).is_err());
    assert_eq!(runtime.get_drain_progress(action), Some(&quiet));
    Ok(())
}

#[test]
fn certificate_identity_and_finalization_tick_must_match() -> Result<()> {
    let (mut runtime, action) = draining(1)?;
    let mut wrong_start = progress(action, 11, 1, 0, true);
    wrong_start.drain_started_tick = GameTick(9);
    assert!(runtime.record_drain_progress(&wrong_start).is_err());
    let unknown = runtime.record_drain_progress(&progress(ActionId::new(99), 11, 1, 0, true));
    assert!(matches!(unknown, Err(error) if error.code == ErrorCode::InvalidRequest));
    let quiet = progress(action, 11, 1, 0, true);
    runtime.record_drain_progress(&quiet)?;
    assert!(runtime.finalize_cancel(action, GameTick(12), &quiet).is_err());
    let wrong_action = progress(ActionId::new(2), 11, 1, 0, true);
    assert!(runtime.finalize_cancel(action, GameTick(11), &wrong_action).is_err());
    runtime.finalize_cancel(action, GameTick(11), &quiet)?;
    Ok(())
}

#[test]
fn exact_progress_and_terminal_replay_are_idempotent_not_replaceable() -> Result<()> {
    let (mut runtime, action) = draining(1)?;
    let quiet = progress(action, 11, 1, 0, true);
    runtime.record_drain_progress(&quiet)?;
    runtime.record_drain_progress(&quiet)?;
    runtime.finalize_cancel(action, GameTick(11), &quiet)?;
    let terminal = runtime.get_status(action).cloned();
    runtime.record_drain_progress(&quiet)?;
    runtime.finalize_cancel(action, GameTick(11), &quiet)?;
    let later = progress(action, 12, 1, 0, true);
    assert!(runtime.record_drain_progress(&later).is_err());
    assert!(runtime.finalize_cancel(action, GameTick(12), &later).is_err());
    assert_eq!(runtime.get_status(action).cloned(), terminal);
    assert_eq!(runtime.get_drain_progress(action), Some(&quiet));
    Ok(())
}

#[test]
fn repeated_requests_preserve_inventory_and_latest_progress() -> Result<()> {
    let (mut runtime, action) = draining(3)?;
    let first = progress(action, 20, 1, 2, false);
    runtime.record_drain_progress(&first)?;
    runtime.request_cancel(action, GameTick(21))?;
    runtime.request_cancel_with_steps(action, GameTick(21), 3)?;
    let wrong_count = runtime.request_cancel_with_steps(action, GameTick(21), 2);
    assert!(matches!(wrong_count, Err(error) if error.code == ErrorCode::Conflict));
    let backdated = runtime.request_cancel(action, GameTick(19));
    assert!(matches!(backdated, Err(error) if error.code == ErrorCode::StaleAnchor));
    assert_eq!(runtime.get_drain_progress(action), Some(&first));
    Ok(())
}

#[test]
fn zero_work_still_requires_a_report_of_quiescence() -> Result<()> {
    let (mut runtime, action) = draining(0)?;
    let quiet = progress(action, 11, 0, 0, true);
    assert!(runtime.finalize_cancel(action, GameTick(11), &quiet).is_err());
    runtime.record_drain_progress(&quiet)?;
    runtime.finalize_cancel(action, GameTick(11), &quiet)?;
    Ok(())
}

#[test]
fn excessive_inventory_is_refused_without_starting_cancellation() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, goal(), GameTick(0))?;
    let result = runtime.request_cancel_with_steps(action, GameTick(10), 65_537);
    assert!(matches!(result, Err(error) if error.code == ErrorCode::BudgetExceeded));
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Active { .. })));
    assert_eq!(runtime.get_drain_progress(action), None);
    runtime.request_cancel_with_steps(action, GameTick(10), 65_536)?;
    Ok(())
}

#[test]
fn progress_cannot_create_a_cancellation_for_an_active_obligation() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action = ActionId::new(1);
    runtime.register_obligation(action, goal(), GameTick(0))?;
    let result = runtime.record_drain_progress(&progress(action, 10, 0, 0, true));
    assert!(matches!(result, Err(error) if error.code == ErrorCode::Conflict));
    assert!(matches!(runtime.get_status(action), Some(ObligationStatus::Active { .. })));
    assert_eq!(runtime.get_drain_progress(action), None);
    Ok(())
}

#[test]
fn paused_game_allows_real_progress_at_the_same_tick() -> Result<()> {
    let (mut runtime, action) = draining(2)?;
    runtime.record_drain_progress(&progress(action, 10, 1, 1, false))?;
    let quiet = progress(action, 10, 2, 0, true);
    runtime.record_drain_progress(&quiet)?;
    runtime.finalize_cancel(action, GameTick(10), &quiet)?;
    Ok(())
}

#[test]
fn independent_drains_cannot_finalize_each_other() -> Result<()> {
    let (mut runtime, first) = draining(2)?;
    let second = ActionId::new(2);
    runtime.register_obligation(second, goal(), GameTick(0))?;
    runtime.request_cancel_with_steps(second, GameTick(10), 1)?;
    let first_progress = progress(first, 11, 1, 1, false);
    runtime.record_drain_progress(&first_progress)?;
    let second_done = progress(second, 12, 1, 0, true);
    runtime.record_drain_progress(&second_done)?;
    assert!(runtime.finalize_cancel(first, GameTick(12), &second_done).is_err());
    runtime.finalize_cancel(second, GameTick(12), &second_done)?;
    assert_eq!(runtime.get_drain_progress(first), Some(&first_progress));
    assert!(matches!(runtime.get_status(first), Some(ObligationStatus::Draining { .. })));
    Ok(())
}
