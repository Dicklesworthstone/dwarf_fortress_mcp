#![forbid(unsafe_code)]

//! Long-Horizon Bounded Obligations and Failure Compensation Coordinator.
//!
//! WP-PLN-04: Manages temporal goal tracking with semantic terminal and failure
//! predicates, stability window verification (ADR-007), and quantitative cancellation drains.

use std::collections::BTreeMap;

use dfmcp_core::{
    ActionId, DfmcpError, ErrorCode, Evidence, EvidenceKind, GameTick, Result, StateAnchor,
};
use dfmcp_world::{Predicate, WorldSnapshot, evaluate};

use crate::plan::ObligationSpec;

mod drain;

const MAX_TRACKED_OBLIGATIONS: usize = 65_536;

/// Quantitative progress certificate emitted during cancellation drain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrainProgressCertificate {
    pub action_id: ActionId,
    pub drain_started_tick: GameTick,
    pub current_tick: GameTick,
    pub steps_compensated: usize,
    pub steps_remaining: usize,
    pub is_quiescent: bool,
}

/// Lifecycle status of a bounded long-running obligation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObligationStatus {
    Pending,
    Active {
        ticks_elapsed: u64,
        consecutive_stable_observations: u32,
    },
    Fulfilled {
        fulfilled_at_tick: GameTick,
        evidence: Vec<Evidence>,
    },
    Failed {
        failed_at_tick: GameTick,
        reason: String,
    },
    Draining {
        drain_started_tick: GameTick,
    },
    Cancelled {
        cancelled_at_tick: GameTick,
    },
}

/// Bounded Obligation instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedObligation {
    pub action_id: ActionId,
    pub spec: ObligationSpec,
    pub status: ObligationStatus,
    pub registered_tick: GameTick,
    pub last_evaluated_tick: Option<GameTick>,
}

/// Runtime coordinator managing long-horizon obligations across game ticks.
#[derive(Clone, Debug, Default)]
pub struct ObligationRuntime {
    obligations: BTreeMap<ActionId, BoundedObligation>,
    // Kept separately to preserve the public BoundedObligation record shape.
    // At most one anchor per registered action; terminal anchors are immutable.
    observation_anchors: BTreeMap<ActionId, StateAnchor>,
    drain_progress: BTreeMap<ActionId, DrainProgressCertificate>,
}

impl ObligationRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            obligations: BTreeMap::new(),
            observation_anchors: BTreeMap::new(),
            drain_progress: BTreeMap::new(),
        }
    }

    /// Register a bounded obligation using the legacy tick-only interface.
    ///
    /// Its source is bound by the first accepted observation, not at registration.
    /// Use [`Self::register_obligation_at`] when the creation snapshot is available.
    pub fn register_obligation(
        &mut self,
        action_id: ActionId,
        mut spec: ObligationSpec,
        current_tick: GameTick,
    ) -> Result<()> {
        if action_id == ActionId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "obligation action identifier zero is reserved",
            ));
        }
        spec.terminal.validate_shape()?;
        spec.terminal = spec.terminal.normalized();
        if matches!(spec.terminal, Predicate::True | Predicate::False) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "obligation terminal predicate must be nontrivial",
            ));
        }
        if let Some(failure) = spec.failure.as_mut() {
            failure.validate_shape()?;
            *failure = failure.normalized();
            if matches!(failure, Predicate::True | Predicate::False) {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "obligation failure predicate must be nontrivial",
                ));
            }
            if failure == &spec.terminal {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "obligation terminal and failure predicates must differ",
                ));
            }
        }
        if spec.deadline_tick <= current_tick
            || spec.poll_interval_ticks == 0
            || spec.stable_for_observations == 0
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "obligation requires a future deadline and nonzero polling/stability bounds",
            ));
        }
        if self.obligations.contains_key(&action_id) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "action already has a registered obligation",
            ));
        }
        if self.obligations.len() >= MAX_TRACKED_OBLIGATIONS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "obligation runtime reached its explicit tracked-action bound",
            ));
        }
        let obligation = BoundedObligation {
            action_id,
            spec,
            status: ObligationStatus::Active {
                ticks_elapsed: 0,
                consecutive_stable_observations: 0,
            },
            registered_tick: current_tick,
            last_evaluated_tick: None,
        };
        self.obligations.insert(action_id, obligation);
        Ok(())
    }

    /// Register against one complete creation snapshot without counting it as a
    /// stability sample. All later observations must remain in its fortress and
    /// observation epoch, with nonregressing ticks and sequences.
    pub fn register_obligation_at(
        &mut self,
        action_id: ActionId,
        spec: ObligationSpec,
        snapshot: &WorldSnapshot,
    ) -> Result<()> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::ChecksumMismatch,
                "obligation registration requires a valid world snapshot",
            ));
        }
        self.register_obligation(action_id, spec, snapshot.tick)?;
        self.observation_anchors.insert(action_id, snapshot.anchor());
        Ok(())
    }

    /// Forget only the unfinished stability streak after a failed or interrupted
    /// read. Identity, poll cadence, elapsed time and the fixed deadline survive.
    /// Historical terminal outcomes and cancellation drains are not rewritten.
    pub fn observation_interrupted(&mut self, action_id: ActionId) -> Result<()> {
        let obligation = self.obligations.get_mut(&action_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::InvalidRequest, "unknown obligation")
        })?;
        if let ObligationStatus::Active {
            consecutive_stable_observations,
            ..
        } = &mut obligation.status
        {
            *consecutive_stable_observations = 0;
        }
        Ok(())
    }

    /// Last accepted observation anchor, or the creation anchor for an unsampled
    /// anchored registration. A legacy registration returns None until sampled.
    #[must_use]
    pub fn last_observation_anchor(&self, action_id: ActionId) -> Option<StateAnchor> {
        self.observation_anchors.get(&action_id).copied()
    }

    /// Evaluate supplied evidence atomically across all active obligations.
    ///
    /// Poll cadence limits positive stability samples, not the failure evidence
    /// a caller has already supplied. An off-cadence contradiction resets the
    /// streak without moving the next eligible poll. Terminal records are immutable.
    pub fn step_tick(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::ChecksumMismatch,
                "obligations cannot be evaluated against an invalid world snapshot",
            ));
        }

        // Validate the entire batch before publishing any transition. Otherwise a
        // later action's stale tick could leave earlier actions falsely fulfilled
        // even though this call returned an error (df-action-coordinator-exec-ero.4).
        let incoming = snapshot.anchor();
        for obligation in self.obligations.values() {
            if !matches!(obligation.status, ObligationStatus::Active { .. }) {
                continue;
            }
            if snapshot.tick < obligation.registered_tick
                || obligation
                    .last_evaluated_tick
                    .is_some_and(|last_tick| snapshot.tick < last_tick)
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "obligation observation tick regressed",
                ));
            }
            if let Some(previous) = self.observation_anchors.get(&obligation.action_id)
                && (incoming.fortress_id != previous.fortress_id
                    || incoming.cursor.epoch != previous.cursor.epoch
                    || incoming.cursor.sequence < previous.cursor.sequence
                    || incoming.tick < previous.tick
                    || (incoming.cursor == previous.cursor && incoming != *previous))
            {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "obligation observation changed lineage, regressed, or forked a cursor",
                ));
            }
        }

        for obligation in self.obligations.values_mut() {
            let ObligationStatus::Active {
                consecutive_stable_observations,
                ..
            } = &obligation.status
            else {
                continue;
            };
            let previous_stable = *consecutive_stable_observations;
            // Track every accepted read, independently of the positive-sample
            // cadence. Otherwise an off-cadence read could be silently rewound.
            self.observation_anchors
                .insert(obligation.action_id, incoming);
            let new_elapsed = snapshot.tick.0.saturating_sub(obligation.registered_tick.0);

            // Failure evidence takes precedence even between scheduled polls or
            // when a second observation at the same game tick changes the facts.
            if obligation
                .spec
                .failure
                .as_ref()
                .is_some_and(|predicate| evaluate(snapshot, predicate))
            {
                obligation.status = ObligationStatus::Failed {
                    failed_at_tick: snapshot.tick,
                    reason: "obligation failure predicate triggered".to_owned(),
                };
                continue;
            }
            if snapshot.tick > obligation.spec.deadline_tick {
                obligation.status = ObligationStatus::Failed {
                    failed_at_tick: snapshot.tick,
                    reason: format!(
                        "obligation deadline tick {} was missed",
                        obligation.spec.deadline_tick.0
                    ),
                };
                continue;
            }

            let cadence_basis = obligation
                .last_evaluated_tick
                .unwrap_or(obligation.registered_tick);
            let distinct_tick = obligation.last_evaluated_tick != Some(snapshot.tick);
            let sample_due = distinct_tick
                && (snapshot.tick.0.saturating_sub(cadence_basis.0)
                    >= obligation.spec.poll_interval_ticks
                    || snapshot.tick == obligation.spec.deadline_tick);
            let satisfied = evaluate(snapshot, &obligation.spec.terminal);
            let next_stable = if !satisfied {
                0
            } else if sample_due {
                previous_stable.saturating_add(1)
            } else {
                previous_stable
            };
            if sample_due {
                obligation.last_evaluated_tick = Some(snapshot.tick);
            }

            obligation.status = if sample_due
                && satisfied
                && next_stable >= obligation.spec.stable_for_observations
            {
                ObligationStatus::Fulfilled {
                    fulfilled_at_tick: snapshot.tick,
                    evidence: vec![Evidence {
                        id: dfmcp_core::EvidenceId::new(obligation.action_id.get()),
                        kind: EvidenceKind::Postcondition,
                        subject: None,
                        anchor: snapshot.anchor(),
                        digest: snapshot.state_hash,
                        summary: "obligation terminal predicate stability window satisfied"
                            .to_owned(),
                    }],
                }
            } else if snapshot.tick >= obligation.spec.deadline_tick {
                // A matching final endpoint is insufficient when the required
                // stability count has not been reached. Do not wait for a later
                // tick to discover that no eligible sample remains.
                ObligationStatus::Failed {
                    failed_at_tick: snapshot.tick,
                    reason: format!(
                        "obligation deadline tick {} reached without fulfilling terminal predicate stability",
                        obligation.spec.deadline_tick.0
                    ),
                }
            } else {
                ObligationStatus::Active {
                    ticks_elapsed: new_elapsed,
                    consecutive_stable_observations: next_stable,
                }
            };
        }

        Ok(())
    }

    /// Look up status of an obligation.
    #[must_use]
    pub fn get_status(&self, action_id: ActionId) -> Option<&ObligationStatus> {
        self.obligations.get(&action_id).map(|o| &o.status)
    }

    /// Total number of tracked obligations.
    #[must_use]
    pub fn obligation_count(&self) -> usize {
        self.obligations.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{FortressId, ObservationCursor};
    use dfmcp_world::{Predicate, WorldGraph};

    fn sample_snapshot(tick: u64, paused: bool) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(tick),
            ObservationCursor {
                epoch: 0,
                sequence: tick,
            },
            paused,
            WorldGraph::default(),
        )
    }

    #[test]
    fn test_obligation_fulfillment_stability_window() -> Result<()> {
        let mut runtime = ObligationRuntime::new();
        let action_id = ActionId::new(1);

        let spec = ObligationSpec {
            terminal: Predicate::Paused(false),
            failure: None,
            deadline_tick: GameTick(100),
            poll_interval_ticks: 1,
            stable_for_observations: 2,
        };

        runtime.register_obligation(action_id, spec, GameTick(10))?;

        let snap1 = sample_snapshot(11, false);
        runtime.step_tick(&snap1)?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Active {
                consecutive_stable_observations: 1,
                ..
            })
        ));

        let snap2 = sample_snapshot(12, false);
        runtime.step_tick(&snap2)?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Fulfilled { .. })
        ));

        Ok(())
    }

    #[test]
    fn test_obligation_deadline_failure() -> Result<()> {
        let mut runtime = ObligationRuntime::new();
        let action_id = ActionId::new(2);

        let spec = ObligationSpec {
            terminal: Predicate::Paused(false),
            failure: None,
            deadline_tick: GameTick(50),
            poll_interval_ticks: 1,
            stable_for_observations: 1,
        };

        runtime.register_obligation(action_id, spec, GameTick(10))?;

        let snap = sample_snapshot(51, true);
        runtime.step_tick(&snap)?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Failed { .. })
        ));

        Ok(())
    }

    #[test]
    fn deadline_is_enforced_even_before_the_next_poll_cadence() -> Result<()> {
        let mut runtime = ObligationRuntime::new();
        let action_id = ActionId::new(3);
        runtime.register_obligation(
            action_id,
            ObligationSpec {
                terminal: Predicate::Paused(false),
                failure: None,
                deadline_tick: GameTick(15),
                poll_interval_ticks: 100,
                stable_for_observations: 1,
            },
            GameTick(10),
        )?;

        runtime.step_tick(&sample_snapshot(15, true))?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Failed {
                failed_at_tick: GameTick(15),
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn terminal_state_first_seen_after_deadline_does_not_fulfill() -> Result<()> {
        let mut runtime = ObligationRuntime::new();
        let action_id = ActionId::new(4);
        runtime.register_obligation(
            action_id,
            ObligationSpec {
                terminal: Predicate::Paused(false),
                failure: None,
                deadline_tick: GameTick(15),
                poll_interval_ticks: 100,
                stable_for_observations: 1,
            },
            GameTick(10),
        )?;

        runtime.step_tick(&sample_snapshot(16, false))?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Failed {
                failed_at_tick: GameTick(16),
                ..
            })
        ));
        Ok(())
    }

    #[test]
    fn registration_rejects_zero_identity_and_trivial_predicates() {
        let mut runtime = ObligationRuntime::new();
        let zero = runtime.register_obligation(
            ActionId::NIL,
            ObligationSpec {
                terminal: Predicate::Paused(false),
                failure: None,
                deadline_tick: GameTick(20),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            },
            GameTick(10),
        );
        assert!(matches!(zero, Err(ref error) if error.code == ErrorCode::InvalidRequest));

        let trivial = runtime.register_obligation(
            ActionId::new(5),
            ObligationSpec {
                terminal: Predicate::True,
                failure: None,
                deadline_tick: GameTick(20),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            },
            GameTick(10),
        );
        assert!(matches!(trivial, Err(ref error) if error.code == ErrorCode::InvalidRequest));
    }

    #[test]
    fn drain_certificate_cannot_precede_drain_start() -> Result<()> {
        let mut runtime = ObligationRuntime::new();
        let action_id = ActionId::new(6);
        runtime.register_obligation(
            action_id,
            ObligationSpec {
                terminal: Predicate::Paused(false),
                failure: None,
                deadline_tick: GameTick(30),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            },
            GameTick(10),
        )?;
        runtime.request_cancel(action_id, GameTick(20))?;
        let certificate = DrainProgressCertificate {
            action_id,
            drain_started_tick: GameTick(20),
            current_tick: GameTick(19),
            steps_compensated: 0,
            steps_remaining: 0,
            is_quiescent: true,
        };
        let result = runtime.finalize_cancel(action_id, GameTick(19), &certificate);
        assert!(
            matches!(result, Err(ref error) if error.code == ErrorCode::CancellationIncomplete)
        );
        Ok(())
    }
}
