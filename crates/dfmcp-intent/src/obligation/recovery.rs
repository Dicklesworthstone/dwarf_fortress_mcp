//! Observation-only obligation recovery after a discontinuity.
//!
//! A durable world frontier is evidence of state, not a retained stability
//! streak. Recovery keeps the original absolute deadline and starts a fresh
//! cadence at that frontier. It never dispatches, grants authority, advances
//! time, or treats an archived snapshot as an additional positive sample.

use super::{ObligationRuntime, ObligationStatus};
use crate::ObligationSpec;
use dfmcp_core::{ActionId, DfmcpError, ErrorCode, GameTick, Result, StateAnchor};
use dfmcp_world::WorldSnapshot;

#[derive(Clone, Debug)]
pub struct RecoveredObligation {
    runtime: ObligationRuntime,
    action_id: ActionId,
    recovery_anchor: StateAnchor,
}

impl RecoveredObligation {
    /// Re-establish observation tracking without recovering action authority.
    /// `action_id` identifies only this proof monitor; it need not name any
    /// live adapter action. `registered_tick` is the original sealed basis,
    /// never a replacement deadline. The frontier's incomplete sample history
    /// is discarded conservatively, including any prior stability count.
    pub fn new(
        action_id: ActionId,
        spec: ObligationSpec,
        registered_tick: GameTick,
        frontier: &WorldSnapshot,
    ) -> Result<Self> {
        if !frontier.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::ChecksumMismatch,
                "recovered obligation requires a valid durable frontier",
            ));
        }
        if frontier.tick < registered_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "recovered frontier precedes the obligation's original registration",
            ));
        }
        let mut runtime = ObligationRuntime::new();
        runtime.register_obligation(action_id, spec, registered_tick)?;
        runtime
            .observation_anchors
            .insert(action_id, frontier.anchor());
        if let Some(obligation) = runtime.obligations.get_mut(&action_id) {
            obligation.last_evaluated_tick = Some(frontier.tick);
            obligation.status = ObligationStatus::Active {
                ticks_elapsed: frontier.tick.0.saturating_sub(registered_tick.0),
                consecutive_stable_observations: 0,
            };
        }
        // Negative evidence and expired deadlines apply immediately. Because
        // the frontier is the cadence floor it cannot count toward stability.
        runtime.step_tick(frontier)?;
        Ok(Self {
            runtime,
            action_id,
            recovery_anchor: frontier.anchor(),
        })
    }

    /// Consume one current authorized observation. The caller owns Observe
    /// authorization; this pure monitor cannot confer it. The normal runtime
    /// rejects forks, epoch changes and regressions before any transition.
    pub fn observe(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        self.runtime.step_tick(snapshot)
    }

    /// A failed authorized read is a gap in the unfinished stability proof.
    /// Rejected caller-supplied stale snapshots do not implicitly create a gap;
    /// the observing shell must signal actual interruption explicitly.
    pub fn observation_interrupted(&mut self) -> Result<()> {
        self.runtime.observation_interrupted(self.action_id)
    }

    #[must_use]
    pub fn status(&self) -> Option<&ObligationStatus> {
        self.runtime.get_status(self.action_id)
    }

    #[must_use]
    pub const fn recovery_anchor(&self) -> StateAnchor {
        self.recovery_anchor
    }

    #[must_use]
    pub fn last_observation_anchor(&self) -> Option<StateAnchor> {
        self.runtime.last_observation_anchor(self.action_id)
    }

    /// A second restart preserves terminal proof and discards unfinished
    /// stability. The absolute deadline and original registration survive.
    pub fn after_restart(&self, frontier: &WorldSnapshot) -> Result<Self> {
        if !frontier.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::ChecksumMismatch,
                "recovered obligation requires a valid durable frontier",
            ));
        }
        if let Some(previous) = self.last_observation_anchor() {
            let incoming = frontier.anchor();
            if incoming.fortress_id != previous.fortress_id
                || incoming.tick < previous.tick
                || incoming.cursor.epoch < previous.cursor.epoch
                || (incoming.cursor.epoch == previous.cursor.epoch
                    && (incoming.cursor.sequence < previous.cursor.sequence
                        || (incoming.cursor == previous.cursor && incoming != previous)))
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "restart frontier regressed or forked the last accepted observation",
                ));
            }
        }
        let obligation = self
            .runtime
            .obligations
            .get(&self.action_id)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "recovery monitor lost its obligation",
                )
            })?;
        if !matches!(obligation.status, ObligationStatus::Active { .. }) {
            return Ok(self.clone());
        }
        if frontier.fortress_id != self.recovery_anchor.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "recovery cannot move an obligation to another fortress",
            ));
        }
        Self::new(
            self.action_id,
            obligation.spec.clone(),
            obligation.registered_tick,
            frontier,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{FortressId, ObservationCursor};
    use dfmcp_world::{Predicate, WorldGraph};

    fn snapshot(tick: u64, epoch: u64, sequence: u64, paused: bool) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor { epoch, sequence },
            paused,
            WorldGraph::default(),
        )
    }

    fn spec(deadline: u64, cadence: u64, stable: u32) -> ObligationSpec {
        ObligationSpec {
            terminal: Predicate::Paused(false),
            failure: None,
            deadline_tick: GameTick(deadline),
            poll_interval_ticks: cadence,
            stable_for_observations: stable,
        }
    }

    #[test]
    fn frontier_and_repeated_reads_do_not_create_stability_samples() -> Result<()> {
        let frontier = snapshot(20, 2, 1, false);
        let mut monitor =
            RecoveredObligation::new(ActionId::new(1), spec(100, 10, 2), GameTick(0), &frontier)?;
        monitor.observe(&frontier)?;
        monitor.observe(&snapshot(29, 2, 2, false))?;
        assert!(matches!(
            monitor.status(),
            Some(ObligationStatus::Active {
                consecutive_stable_observations: 0,
                ..
            })
        ));
        monitor.observe(&snapshot(30, 2, 3, false))?;
        monitor.observe(&snapshot(30, 2, 4, false))?;
        assert!(matches!(
            monitor.status(),
            Some(ObligationStatus::Active {
                consecutive_stable_observations: 1,
                ..
            })
        ));
        monitor.observe(&snapshot(40, 2, 5, false))?;
        assert!(matches!(
            monitor.status(),
            Some(ObligationStatus::Fulfilled {
                fulfilled_at_tick: GameTick(40),
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn exact_deadline_can_supply_final_sample_but_late_first_proof_fails() -> Result<()> {
        let frontier = snapshot(10, 2, 1, true);
        let mut exact =
            RecoveredObligation::new(ActionId::new(2), spec(15, 100, 1), GameTick(0), &frontier)?;
        let mut late = exact.clone();
        exact.observe(&snapshot(15, 2, 2, false))?;
        late.observe(&snapshot(16, 2, 2, false))?;
        assert!(matches!(
            exact.status(),
            Some(ObligationStatus::Fulfilled {
                fulfilled_at_tick: GameTick(15),
                ..
            })
        ));
        assert!(matches!(
            late.status(),
            Some(ObligationStatus::Failed {
                failed_at_tick: GameTick(16),
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn already_expired_frontier_never_extends_the_deadline() -> Result<()> {
        let monitor = RecoveredObligation::new(
            ActionId::new(3),
            spec(15, 1, 1),
            GameTick(0),
            &snapshot(16, 2, 1, false),
        )?;
        assert!(matches!(
            monitor.status(),
            Some(ObligationStatus::Failed {
                failed_at_tick: GameTick(16),
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn failure_predicate_wins_over_matching_terminal_before_cadence() -> Result<()> {
        let mut obligation = spec(100, 20, 1);
        obligation.failure = Some(Predicate::Any(vec![
            Predicate::Paused(false),
            Predicate::EntityExists(dfmcp_core::EntityId::new(99)),
        ]));
        let mut monitor = RecoveredObligation::new(
            ActionId::new(4),
            obligation,
            GameTick(0),
            &snapshot(10, 2, 1, true),
        )?;
        monitor.observe(&snapshot(11, 2, 2, false))?;
        assert!(
            matches!(monitor.status(), Some(ObligationStatus::Failed { reason, .. }) if reason.contains("failure predicate"))
        );
        Ok(())
    }

    #[test]
    fn off_cadence_contradiction_breaks_the_stability_streak() -> Result<()> {
        let mut monitor = RecoveredObligation::new(
            ActionId::new(5),
            spec(100, 10, 2),
            GameTick(0),
            &snapshot(10, 2, 1, false),
        )?;
        monitor.observe(&snapshot(20, 2, 2, false))?;
        monitor.observe(&snapshot(21, 2, 3, true))?;
        monitor.observe(&snapshot(30, 2, 4, false))?;
        assert!(matches!(
            monitor.status(),
            Some(ObligationStatus::Active {
                consecutive_stable_observations: 1,
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn restart_discards_unproven_stability_but_retains_absolute_deadline() -> Result<()> {
        let mut monitor = RecoveredObligation::new(
            ActionId::new(6),
            spec(40, 10, 2),
            GameTick(0),
            &snapshot(10, 2, 1, false),
        )?;
        monitor.observe(&snapshot(20, 2, 2, false))?;
        let mut restarted = monitor.after_restart(&snapshot(25, 3, 1, false))?;
        restarted.observe(&snapshot(30, 3, 2, false))?;
        assert!(matches!(
            restarted.status(),
            Some(ObligationStatus::Active {
                consecutive_stable_observations: 0,
                ..
            })
        ));
        restarted.observe(&snapshot(35, 3, 3, false))?;
        restarted.observe(&snapshot(40, 3, 4, false))?;
        assert!(matches!(
            restarted.status(),
            Some(ObligationStatus::Fulfilled {
                fulfilled_at_tick: GameTick(40),
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn terminal_proof_is_immutable_after_later_contradictions_and_restart() -> Result<()> {
        let mut monitor = RecoveredObligation::new(
            ActionId::new(7),
            spec(30, 10, 1),
            GameTick(0),
            &snapshot(10, 2, 1, true),
        )?;
        monitor.observe(&snapshot(20, 2, 2, false))?;
        let terminal = monitor.status().cloned();
        let anchor = monitor.last_observation_anchor();
        monitor.observe(&snapshot(40, 2, 3, true))?;
        let restarted = monitor.after_restart(&snapshot(50, 3, 1, true))?;
        assert_eq!(monitor.status(), terminal.as_ref());
        assert_eq!(monitor.last_observation_anchor(), anchor);
        assert_eq!(restarted.status(), terminal.as_ref());
        assert_eq!(restarted.last_observation_anchor(), anchor);
        Ok(())
    }

    #[test]
    fn regressed_or_forked_observation_refuses_without_partial_progress() -> Result<()> {
        let mut monitor = RecoveredObligation::new(
            ActionId::new(8),
            spec(100, 10, 2),
            GameTick(0),
            &snapshot(10, 2, 1, true),
        )?;
        monitor.observe(&snapshot(20, 2, 2, false))?;
        let prior = monitor.status().cloned();
        assert!(monitor.observe(&snapshot(21, 3, 3, false)).is_err());
        assert!(monitor.observe(&snapshot(19, 2, 3, false)).is_err());
        assert!(monitor.observe(&snapshot(20, 2, 2, true)).is_err());
        assert!(monitor.after_restart(&snapshot(19, 3, 1, false)).is_err());
        assert!(monitor.after_restart(&snapshot(21, 1, 99, false)).is_err());
        assert_eq!(monitor.status(), prior.as_ref());
        Ok(())
    }

    #[test]
    fn interrupted_observation_resets_streak_without_extending_deadline() -> Result<()> {
        let mut monitor = RecoveredObligation::new(
            ActionId::new(9),
            spec(30, 10, 2),
            GameTick(0),
            &snapshot(10, 2, 1, true),
        )?;
        monitor.observe(&snapshot(20, 2, 2, false))?;
        monitor.observation_interrupted()?;
        monitor.observe(&snapshot(30, 2, 3, false))?;
        assert!(matches!(
            monitor.status(),
            Some(ObligationStatus::Failed {
                failed_at_tick: GameTick(30),
                ..
            })
        ));
        Ok(())
    }
}
