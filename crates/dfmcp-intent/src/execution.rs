//! Pure decisions for steps whose effect has not been dispatched yet.
//!
//! A dependency is a proof gate, not a promise to dispatch eventually.
//! Failed prerequisites and expired obligations close undispatched work with
//! evidence; unresolved prerequisites never grant permission to perform it.

use dfmcp_core::{CommitState, StepId};
use dfmcp_world::{WorldSnapshot, evaluate};

use crate::PlanStep;

/// Semantic disposition of an undispatched step. `Ready` still requires
/// current effect authority, budgets and adapter checks at the write boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeferredStepDecision {
    Ready,
    Waiting,
    Failed(String),
}

/// Revalidate a deferred step against one current observation and the retained
/// states of its prerequisites. Callers validate the sealed plan's bounds before
/// reaching this pure core. Missing and indeterminate prerequisite evidence is
/// unresolved, never equivalent to verified or proven not applied.
#[must_use]
pub fn deferred_step_decision(
    step: &PlanStep,
    snapshot: &WorldSnapshot,
    mut dependency_state: impl FnMut(StepId) -> Option<CommitState>,
) -> DeferredStepDecision {
    let mut ready = true;
    for dependency in &step.depends_on {
        match dependency_state(*dependency) {
            Some(CommitState::Verified) => {}
            Some(state) if state.is_terminal() => {
                return DeferredStepDecision::Failed(format!(
                    "dependency step {} ended {state:?}; this step was not dispatched",
                    dependency.get()
                ));
            }
            _ => ready = false,
        }
    }
    if let Some(obligation) = &step.obligation {
        if obligation
            .failure
            .as_ref()
            .is_some_and(|predicate| evaluate(snapshot, predicate))
        {
            return DeferredStepDecision::Failed(
                "obligation failure predicate observed before dispatch; this step was not dispatched"
                    .to_owned(),
            );
        }
        if snapshot.tick >= obligation.deadline_tick {
            return DeferredStepDecision::Failed(format!(
                "obligation deadline tick {} reached before dispatch; this step was not dispatched",
                obligation.deadline_tick.0
            ));
        }
    }
    if !ready {
        return DeferredStepDecision::Waiting;
    }
    if !step
        .preconditions
        .iter()
        .all(|predicate| evaluate(snapshot, predicate))
    {
        return DeferredStepDecision::Failed(
            "preconditions are no longer established true; this step was not dispatched".to_owned(),
        );
    }
    DeferredStepDecision::Ready
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, ObligationSpec};
    use dfmcp_core::{Capability, FortressId, GameTick, ObservationCursor, RiskTier};
    use dfmcp_world::{Predicate, WorldGraph};

    fn step() -> PlanStep {
        PlanStep {
            id: StepId::new(1),
            action: Action::Pause { paused: false },
            preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: None,
            depends_on: vec![StepId::new(0)],
            risk: RiskTier::Reversible,
            required_capability: Capability::ControlClock,
            idempotency_key: "step-1".to_owned(),
        }
    }

    fn snapshot() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        )
    }

    #[test]
    fn only_verified_prerequisites_release_a_step_and_uncertainty_never_proves_failure() {
        let step = step();
        let snapshot = snapshot();
        for state in [
            CommitState::Prepared,
            CommitState::Committing,
            CommitState::AppliedAwaitingVerification,
            CommitState::CompensationPending,
            CommitState::CancelRequested,
            CommitState::Indeterminate,
        ] {
            assert_eq!(
                deferred_step_decision(&step, &snapshot, |_| Some(state)),
                DeferredStepDecision::Waiting
            );
        }
        assert_eq!(
            deferred_step_decision(&step, &snapshot, |_| None),
            DeferredStepDecision::Waiting
        );
        assert_eq!(
            deferred_step_decision(&step, &snapshot, |_| Some(CommitState::Verified)),
            DeferredStepDecision::Ready
        );
        for state in [
            CommitState::Failed,
            CommitState::Cancelled,
            CommitState::Compensated,
        ] {
            let result = deferred_step_decision(&step, &snapshot, |_| Some(state));
            assert!(
                matches!(result, DeferredStepDecision::Failed(ref message) if message.contains("not dispatched"))
            );
        }
    }

    #[test]
    fn deferred_failure_and_deadline_are_checked_even_with_unresolved_prerequisites() {
        let mut step = step();
        let snapshot = snapshot();
        step.obligation = Some(ObligationSpec {
            terminal: Predicate::Paused(false),
            failure: Some(Predicate::Paused(true)),
            deadline_tick: GameTick(200),
            poll_interval_ticks: 1,
            stable_for_observations: 1,
        });
        assert!(
            matches!(deferred_step_decision(&step, &snapshot, |_| None), DeferredStepDecision::Failed(message) if message.contains("failure predicate"))
        );
        step.obligation = Some(ObligationSpec {
            terminal: Predicate::Paused(false),
            failure: None,
            deadline_tick: GameTick(100),
            poll_interval_ticks: 1,
            stable_for_observations: 1,
        });
        assert!(
            matches!(deferred_step_decision(&step, &snapshot, |_| Some(CommitState::Indeterminate)), DeferredStepDecision::Failed(message) if message.contains("deadline"))
        );
    }
}
