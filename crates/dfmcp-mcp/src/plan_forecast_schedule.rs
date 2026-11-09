//! Observation frontiers for a discarded laboratory forecast.
//!
//! A shared coarse stride must not skip another step's sealed deadline. Near
//! that deadline, reserve enough cadence-spaced observations for its original
//! stability requirement. This schedules reads; only the adapter can count a
//! positive sample, release a dependency, or establish an action outcome.

use dfmcp_core::CommitState;
use dfmcp_intent::ObligationSpec;

pub(super) struct ObservationSchedule {
    deadline: u64,
    cadence: u64,
    stable_samples: u32,
    next_eligible: Option<u64>,
}

impl ObservationSchedule {
    pub(super) fn new(spec: &ObligationSpec, state: CommitState, tick: u64) -> Self {
        Self {
            deadline: spec.deadline_tick.0,
            cadence: spec.poll_interval_ticks,
            stable_samples: spec.stable_for_observations,
            next_eligible: if state == CommitState::AppliedAwaitingVerification {
                tick.checked_add(spec.poll_interval_ticks)
            } else {
                None
            },
        }
    }

    /// Next mandatory observation in addition to the ordinary coarse stride.
    /// Deferred work has no sampling clock until the adapter dispatches it, but
    /// retains its original absolute deadline throughout the dependency wait.
    pub(super) fn next_frontier(&self, now: u64) -> Option<u64> {
        if now >= self.deadline {
            return None;
        }
        let mut next = self.deadline;
        if self.stable_samples > 1
            && let Some(eligible) = self.next_eligible
        {
            let tail = self
                .cadence
                .saturating_mul(u64::from(self.stable_samples - 1));
            let first = self.deadline.saturating_sub(tail).max(eligible);
            if first > now {
                next = next.min(first);
            }
        }
        Some(next)
    }

    /// Mirror only the runtime's eligible-read clock, never its truth or
    /// stability streak. An off-cadence read cannot postpone the next sample.
    pub(super) fn observed(&mut self, before: CommitState, after: CommitState, tick: u64) {
        if after == CommitState::AppliedAwaitingVerification {
            if before == CommitState::Prepared
                || self.next_eligible.is_some_and(|eligible| tick >= eligible)
            {
                self.next_eligible = tick.checked_add(self.cadence);
            }
        } else if after.is_terminal() {
            self.next_eligible = None;
        }
    }
}
