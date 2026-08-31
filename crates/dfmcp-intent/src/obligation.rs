#![forbid(unsafe_code)]

//! Long-Horizon Bounded Obligations and Failure Compensation Coordinator.
//!
//! WP-PLN-04: Manages temporal goal tracking with semantic terminal and failure
//! predicates, stability window verification (ADR-007), and quantitative cancellation drains.

use std::collections::BTreeMap;

use dfmcp_core::{ActionId, DfmcpError, ErrorCode, Evidence, EvidenceKind, GameTick, Result};
use dfmcp_world::WorldSnapshot;
use dfmcp_world::evaluate;

use crate::plan::ObligationSpec;

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
}

impl ObligationRuntime {
    #[must_use]
    pub fn new() -> Self {
        Self {
            obligations: BTreeMap::new(),
        }
    }

    /// Register a new bounded obligation.
    pub fn register_obligation(
        &mut self,
        action_id: ActionId,
        spec: ObligationSpec,
        current_tick: GameTick,
    ) -> Result<()> {
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

    /// Advance game tick and evaluate all active obligations against the new world snapshot.
    pub fn step_tick(&mut self, snapshot: &WorldSnapshot) -> Result<()> {
        for obligation in self.obligations.values_mut() {
            let mut next_status = None;

            if let ObligationStatus::Active {
                ticks_elapsed,
                consecutive_stable_observations,
            } = &obligation.status
            {
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
                if obligation
                    .last_evaluated_tick
                    .is_some_and(|last_tick| snapshot.tick == last_tick)
                {
                    continue;
                }
                let cadence_basis = obligation
                    .last_evaluated_tick
                    .map_or(obligation.registered_tick, |tick| tick);
                if snapshot.tick.0.saturating_sub(cadence_basis.0)
                    < obligation.spec.poll_interval_ticks
                {
                    continue;
                }
                obligation.last_evaluated_tick = Some(snapshot.tick);
                let new_elapsed = snapshot.tick.0.saturating_sub(obligation.registered_tick.0);

                // 1. Explicit failure predicates take precedence.
                if let Some(fail_pred) = &obligation.spec.failure
                    && evaluate(snapshot, fail_pred)
                {
                    next_status = Some(ObligationStatus::Failed {
                        failed_at_tick: snapshot.tick,
                        reason: "obligation failure predicate triggered".to_owned(),
                    });
                }

                // 2. Check terminal predicate and stability window. A terminal
                // observation at the deadline is eligible; a missed deadline is not.
                if next_status.is_none() {
                    let satisfied = evaluate(snapshot, &obligation.spec.terminal);
                    if satisfied {
                        let new_stable = consecutive_stable_observations.saturating_add(1);
                        if new_stable >= obligation.spec.stable_for_observations.max(1) {
                            let evidence = vec![Evidence {
                                id: dfmcp_core::EvidenceId::new(obligation.action_id.get()),
                                kind: EvidenceKind::Postcondition,
                                subject: None,
                                anchor: snapshot.anchor(),
                                digest: snapshot.state_hash,
                                summary: "obligation terminal predicate stability window satisfied"
                                    .to_owned(),
                            }];

                            next_status = Some(ObligationStatus::Fulfilled {
                                fulfilled_at_tick: snapshot.tick,
                                evidence,
                            });
                        } else {
                            next_status = Some(ObligationStatus::Active {
                                ticks_elapsed: new_elapsed,
                                consecutive_stable_observations: new_stable,
                            });
                        }
                    } else if snapshot.tick >= obligation.spec.deadline_tick {
                        next_status = Some(ObligationStatus::Failed {
                            failed_at_tick: snapshot.tick,
                            reason: format!(
                                "obligation deadline tick {} reached without fulfilling terminal predicate",
                                obligation.spec.deadline_tick.0
                            ),
                        });
                    } else {
                        // Reset stability counter if terminal predicate was not satisfied this tick
                        next_status = Some(ObligationStatus::Active {
                            ticks_elapsed: new_elapsed.max(*ticks_elapsed),
                            consecutive_stable_observations: 0,
                        });
                    }
                }
            }

            if let Some(status) = next_status {
                obligation.status = status;
            }
        }

        Ok(())
    }

    /// Request cancellation and begin draining.
    pub fn request_cancel(&mut self, action_id: ActionId, current_tick: GameTick) -> Result<()> {
        let obligation = self.obligations.get_mut(&action_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("obligation {:?} not found", action_id),
            )
        })?;

        if current_tick < obligation.registered_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "cancellation tick precedes obligation registration",
            ));
        }

        match obligation.status {
            ObligationStatus::Active { .. } => {
                obligation.status = ObligationStatus::Draining {
                    drain_started_tick: current_tick,
                };
                Ok(())
            }
            ObligationStatus::Fulfilled { .. } => Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "cannot cancel already fulfilled obligation",
            )),
            ObligationStatus::Failed { .. } => Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "cannot cancel already failed obligation",
            )),
            ObligationStatus::Draining { .. } | ObligationStatus::Cancelled { .. } => Ok(()),
            ObligationStatus::Pending => Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cannot cancel an obligation that has not become active",
            )),
        }
    }

    /// Finalize cancellation only after a quantitative certificate proves quiescence.
    pub fn finalize_cancel(
        &mut self,
        action_id: ActionId,
        current_tick: GameTick,
        certificate: &DrainProgressCertificate,
    ) -> Result<()> {
        let obligation = self.obligations.get_mut(&action_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("obligation {:?} not found", action_id),
            )
        })?;

        match obligation.status {
            ObligationStatus::Draining { drain_started_tick }
                if certificate.action_id == action_id
                    && certificate.drain_started_tick == drain_started_tick
                    && certificate.current_tick == current_tick
                    && certificate.is_quiescent
                    && certificate.steps_remaining == 0 =>
            {
                obligation.status = ObligationStatus::Cancelled {
                    cancelled_at_tick: current_tick,
                };
                Ok(())
            }
            ObligationStatus::Cancelled { .. } => Ok(()),
            ObligationStatus::Draining { .. } => Err(DfmcpError::new(
                ErrorCode::CancellationIncomplete,
                "cancellation drain certificate does not prove current quiescence",
            )),
            _ => Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cancellation can be finalized only from the draining state",
            )),
        }
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

        // Tick 11: simulation unpaused (stable count = 1)
        let snap1 = sample_snapshot(11, false);
        runtime.step_tick(&snap1)?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Active {
                consecutive_stable_observations: 1,
                ..
            })
        ));

        // Tick 12: simulation unpaused again (stable count = 2 -> fulfilled!)
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

        // Advance to tick 51 while still paused -> should fail
        let snap = sample_snapshot(51, true);
        runtime.step_tick(&snap)?;
        assert!(matches!(
            runtime.get_status(action_id),
            Some(ObligationStatus::Failed { .. })
        ));

        Ok(())
    }
}
