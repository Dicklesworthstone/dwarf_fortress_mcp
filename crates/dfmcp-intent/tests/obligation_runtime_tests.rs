#![forbid(unsafe_code)]

//! Integration tests for WP-PLN-04 Bounded Obligations & Failure Compensation.

use dfmcp_core::{ActionId, GameTick, Result};
use dfmcp_intent::ObligationSpec;
use dfmcp_intent::obligation::{DrainProgressCertificate, ObligationRuntime, ObligationStatus};
use dfmcp_world::Predicate;

#[test]
fn test_cancellation_drain_transition() -> Result<()> {
    let mut runtime = ObligationRuntime::new();
    let action_id = ActionId::new(10);

    let spec = ObligationSpec {
        terminal: Predicate::True,
        failure: None,
        deadline_tick: GameTick(200),
        poll_interval_ticks: 1,
        stable_for_observations: 3,
    };

    runtime.register_obligation(action_id, spec, GameTick(100))?;

    // Request cancel
    runtime.request_cancel(action_id, GameTick(110))?;
    assert!(matches!(
        runtime.get_status(action_id),
        Some(ObligationStatus::Draining { .. })
    ));

    // Finalize cancel
    runtime.finalize_cancel(
        action_id,
        GameTick(115),
        &DrainProgressCertificate {
            action_id,
            drain_started_tick: GameTick(110),
            current_tick: GameTick(115),
            steps_compensated: 1,
            steps_remaining: 0,
            is_quiescent: true,
        },
    )?;
    assert!(matches!(
        runtime.get_status(action_id),
        Some(ObligationStatus::Cancelled { .. })
    ));

    Ok(())
}
