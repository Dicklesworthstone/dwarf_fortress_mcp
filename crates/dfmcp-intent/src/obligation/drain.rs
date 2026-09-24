//! Quantitative cancellation bookkeeping; the effect owner supplies verified progress.

use super::{DrainProgressCertificate, ObligationRuntime, ObligationStatus};
use dfmcp_core::{ActionId, DfmcpError, ErrorCode, GameTick, Result};

const MAX_DRAIN_STEPS: usize = 65_536;

fn incomplete(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CancellationIncomplete, message)
}

fn total_steps(certificate: &DrainProgressCertificate) -> Result<usize> {
    certificate
        .steps_compensated
        .checked_add(certificate.steps_remaining)
        .filter(|total| *total <= MAX_DRAIN_STEPS)
        .ok_or_else(|| incomplete("drain step count overflowed or exceeded its bound"))
}

impl ObligationRuntime {
    /// Begin a zero-compensation drain, or repeat an existing cancellation without
    /// replacing its work budget. Quiescence still needs an explicit progress report.
    /// Use [`Self::request_cancel_with_steps`] when compensation work is required.
    pub fn request_cancel(&mut self, action_id: ActionId, current_tick: GameTick) -> Result<()> {
        let steps = match self.drain_progress.get(&action_id) {
            Some(progress) => total_steps(progress)?,
            None => 0,
        };
        self.request_cancel_with_steps(action_id, current_tick, steps)
    }

    /// Begin draining with an immutable, bounded number of compensation steps.
    /// Repeated requests cannot change the budget or backdate the last progress.
    /// This does not dispatch, authorize or prove any compensation game effect.
    pub fn request_cancel_with_steps(
        &mut self,
        action_id: ActionId,
        current_tick: GameTick,
        steps_to_compensate: usize,
    ) -> Result<()> {
        if steps_to_compensate > MAX_DRAIN_STEPS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "cancellation exceeds the bounded compensation-step inventory",
            ));
        }
        let obligation = self.obligations.get_mut(&action_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::InvalidRequest, "unknown obligation")
        })?;
        if current_tick < obligation.registered_tick
            || obligation
                .last_evaluated_tick
                .is_some_and(|tick| current_tick < tick)
            || self
                .observation_anchors
                .get(&action_id)
                .is_some_and(|anchor| current_tick < anchor.tick)
        {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "cancellation tick precedes obligation registration or observation",
            ));
        }
        if let Some(previous) = self.drain_progress.get(&action_id) {
            if total_steps(previous)? != steps_to_compensate {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "repeated cancellation cannot change its compensation inventory",
                ));
            }
            if current_tick < previous.current_tick {
                return Err(DfmcpError::new(
                    ErrorCode::StaleAnchor,
                    "cancellation request predates retained drain progress",
                ));
            }
        }
        match obligation.status {
            ObligationStatus::Active { .. } => {
                let initial = DrainProgressCertificate {
                    action_id,
                    drain_started_tick: current_tick,
                    current_tick,
                    steps_compensated: 0,
                    steps_remaining: steps_to_compensate,
                    is_quiescent: false,
                };
                self.drain_progress.insert(action_id, initial);
                obligation.status = ObligationStatus::Draining {
                    drain_started_tick: current_tick,
                };
                Ok(())
            }
            ObligationStatus::Draining { .. } | ObligationStatus::Cancelled { .. } => Ok(()),
            ObligationStatus::Fulfilled { .. } | ObligationStatus::Failed { .. } => {
                Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "cannot cancel an already fulfilled or failed obligation",
                ))
            }
            ObligationStatus::Pending => Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cannot cancel an obligation that has not become active",
            )),
        }
    }

    /// Accept a bounded, monotone progress report from the cancellation owner.
    /// Counts must conserve the registered work inventory, and no pending work
    /// may be described as quiescent. Same-tick progress is allowed while paused.
    ///
    /// Counts are bookkeeping, not authorization or independent effect evidence.
    /// The owner must verify compensation postconditions and drain in-flight work
    /// before asserting quiescence. This method performs no game or storage I/O.
    pub fn record_drain_progress(&mut self, certificate: &DrainProgressCertificate) -> Result<()> {
        let obligation = self.obligations.get(&certificate.action_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::InvalidRequest, "unknown obligation")
        })?;
        let previous = self.drain_progress.get(&certificate.action_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::Conflict, "obligation has no registered cancellation drain")
        })?;
        let drain_started_tick = match obligation.status {
            ObligationStatus::Draining { drain_started_tick } => drain_started_tick,
            ObligationStatus::Cancelled { .. } if previous == certificate => return Ok(()),
            _ => return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "progress can only advance an active cancellation drain",
            )),
        };
        if certificate.drain_started_tick != drain_started_tick
            || certificate.current_tick < drain_started_tick
            || certificate.current_tick < previous.current_tick
            || total_steps(certificate)? != total_steps(previous)?
            || certificate.steps_compensated < previous.steps_compensated
            || certificate.steps_remaining > previous.steps_remaining
            || (certificate.is_quiescent && certificate.steps_remaining != 0)
            || (previous.is_quiescent && !certificate.is_quiescent)
        {
            return Err(incomplete("drain progress changed its identity, inventory or monotone bounds"));
        }
        self.drain_progress.insert(certificate.action_id, certificate.clone());
        Ok(())
    }

    /// Inspect the latest registered progress, including immutable terminal history.
    #[must_use]
    pub fn get_drain_progress(&self, action_id: ActionId) -> Option<&DrainProgressCertificate> {
        self.drain_progress.get(&action_id)
    }

    /// Finalize only the exact latest registered certificate, at its reported tick.
    /// A caller cannot skip progress registration or replace it with a claimed zero.
    pub fn finalize_cancel(
        &mut self,
        action_id: ActionId,
        current_tick: GameTick,
        certificate: &DrainProgressCertificate,
    ) -> Result<()> {
        let obligation = self.obligations.get_mut(&action_id).ok_or_else(|| {
            DfmcpError::new(ErrorCode::InvalidRequest, "unknown obligation")
        })?;
        let identity_matches = certificate.action_id == action_id
            && certificate.current_tick == current_tick
            && self.drain_progress.get(&action_id) == Some(certificate);
        match obligation.status {
            ObligationStatus::Draining { drain_started_tick }
                if identity_matches
                    && certificate.drain_started_tick == drain_started_tick
                    && current_tick >= drain_started_tick
                    && certificate.is_quiescent
                    && certificate.steps_remaining == 0 =>
            {
                obligation.status = ObligationStatus::Cancelled {
                    cancelled_at_tick: current_tick,
                };
                Ok(())
            }
            ObligationStatus::Cancelled { cancelled_at_tick }
                if identity_matches && current_tick == cancelled_at_tick => Ok(()),
            ObligationStatus::Draining { .. } | ObligationStatus::Cancelled { .. } => {
                Err(incomplete("cancellation requires its exact recorded quiescent certificate"))
            }
            _ => Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cancellation can be finalized only from the draining state",
            )),
        }
    }
}
