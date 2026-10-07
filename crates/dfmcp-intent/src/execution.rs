//! Pure decisions for steps whose effect has not been dispatched yet.
//!
//! A dependency is a proof gate, not a promise to dispatch eventually.
//! Failed prerequisites and expired obligations close undispatched work with
//! evidence; unresolved prerequisites never grant permission to perform it.

use dfmcp_core::{CommitState, Result, StepId};
use dfmcp_world::{PredicateEvidence, WorldSnapshot};

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
    dependency_state: impl FnMut(StepId) -> Option<CommitState>,
) -> DeferredStepDecision {
    PredicateEvidence::untrusted(snapshot)
        .and_then(|evidence| deferred_step_decision_with_evidence(step, &evidence, dependency_state))
        .unwrap_or_else(|error| DeferredStepDecision::Failed(format!(
            "observation evidence is invalid; this step was not dispatched: {error}"
        )))
}

/// Decide with source and completeness rights supplied by the observing shell.
/// Unknown evidence is not a satisfied precondition, including under negation.
pub fn deferred_step_decision_with_evidence(
    step: &PlanStep,
    evidence: &PredicateEvidence<'_>,
    mut dependency_state: impl FnMut(StepId) -> Option<CommitState>,
) -> Result<DeferredStepDecision> {
    let snapshot = evidence.snapshot();
    let mut ready = true;
    for dependency in &step.depends_on {
        match dependency_state(*dependency) {
            Some(CommitState::Verified) => {}
            Some(state) if state.is_terminal() => {
                return Ok(DeferredStepDecision::Failed(format!(
                    "dependency step {} ended {state:?}; this step was not dispatched",
                    dependency.get()
                )));
            }
            _ => ready = false,
        }
    }
    if let Some(obligation) = &step.obligation {
        if obligation.failure.as_ref()
            .map(|predicate| evidence.establishes(predicate)).transpose()?.unwrap_or(false)
        {
            return Ok(DeferredStepDecision::Failed(
                "obligation failure predicate observed before dispatch; this step was not dispatched"
                    .to_owned(),
            ));
        }
        if snapshot.tick >= obligation.deadline_tick {
            return Ok(DeferredStepDecision::Failed(format!(
                "obligation deadline tick {} reached before dispatch; this step was not dispatched",
                obligation.deadline_tick.0
            )));
        }
    }
    if !ready {
        return Ok(DeferredStepDecision::Waiting);
    }
    for predicate in &step.preconditions {
        if !evidence.establishes(predicate)? {
            return Ok(DeferredStepDecision::Failed(
                "preconditions are no longer established true; this step was not dispatched".to_owned(),
            ));
        }
    }
    Ok(DeferredStepDecision::Ready)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Action, ObligationSpec};
    use dfmcp_core::{Capability, FortressId, GameTick, ObservationCursor, RiskTier};
    use dfmcp_world::{Predicate, WorldGraph};

    fn laboratory_decision(
        step: &PlanStep,
        snapshot: &WorldSnapshot,
        dependency_state: impl FnMut(StepId) -> Option<CommitState>,
    ) -> DeferredStepDecision {
        PredicateEvidence::laboratory(snapshot)
            .and_then(|evidence| deferred_step_decision_with_evidence(step, &evidence, dependency_state))
            .unwrap_or_else(|error| DeferredStepDecision::Failed(error.to_string()))
    }

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
                laboratory_decision(&step, &snapshot, |_| Some(state)),
                DeferredStepDecision::Waiting
            );
        }
        assert_eq!(
            laboratory_decision(&step, &snapshot, |_| None),
            DeferredStepDecision::Waiting
        );
        assert_eq!(
            laboratory_decision(&step, &snapshot, |_| Some(CommitState::Verified)),
            DeferredStepDecision::Ready
        );
        for state in [
            CommitState::Failed,
            CommitState::Cancelled,
            CommitState::Compensated,
        ] {
            let result = laboratory_decision(&step, &snapshot, |_| Some(state));
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
            matches!(laboratory_decision(&step, &snapshot, |_| None), DeferredStepDecision::Failed(message) if message.contains("failure predicate"))
        );
        step.obligation = Some(ObligationSpec {
            terminal: Predicate::Paused(false),
            failure: None,
            deadline_tick: GameTick(100),
            poll_interval_ticks: 1,
            stable_for_observations: 1,
        });
        assert!(
            matches!(laboratory_decision(&step, &snapshot, |_| Some(CommitState::Indeterminate)), DeferredStepDecision::Failed(message) if message.contains("deadline"))
        );
    }

    #[test]
    fn incomparable_fields_cannot_authorize_dispatch_or_terminal_obligation_evidence()
    -> dfmcp_core::Result<()> {
        use crate::{ObligationRuntime, ObligationStatus};
        use dfmcp_core::{ActionId, Digest32, EntityId};
        use dfmcp_world::{CompareOp, EntityKind, EntityRecord, Fact, FactSource, Value};
        use std::collections::BTreeMap;

        let mut snapshot = snapshot();
        let subject = EntityId::new(1);
        snapshot.graph.entities.insert(
            subject,
            EntityRecord {
                id: subject,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "worker".to_owned(),
                fields: BTreeMap::from([(
                    "ready".to_owned(),
                    Fact::known(
                        Value::U64(1),
                        snapshot.tick,
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )]),
            },
        );
        snapshot.refresh_hash();
        let predicate = Predicate::Not(Box::new(Predicate::FieldCompare {
            entity_id: subject,
            field: "ready".to_owned(),
            op: CompareOp::Lt,
            value: Value::Text("m".to_owned()),
        }));
        let mut step = step();
        step.preconditions = vec![predicate.clone()];
        assert!(matches!(
            laboratory_decision(&step, &snapshot, |_| Some(CommitState::Verified)),
            DeferredStepDecision::Failed(_),
        ));

        let mut runtime = ObligationRuntime::new();
        for (id, terminal, failure) in [
            (ActionId::new(1), predicate.clone(), None),
            (ActionId::new(2), Predicate::Paused(false), Some(predicate)),
        ] {
            runtime.register_obligation_at(
                id,
                ObligationSpec {
                    terminal,
                    failure,
                    deadline_tick: GameTick(200),
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                },
                &snapshot,
            )?;
        }
        snapshot.tick = GameTick(101);
        snapshot.cursor.sequence = 1;
        snapshot.refresh_hash();
        runtime.step_tick_laboratory(&snapshot)?;
        for id in [ActionId::new(1), ActionId::new(2)] {
            assert!(matches!(
                runtime.get_status(id),
                Some(ObligationStatus::Active {
                    consecutive_stable_observations: 0,
                    ..
                })
            ));
        }

        // A subsequent, genuinely comparable observation can establish either
        // outcome; uncertainty does not permanently disable the obligation.
        snapshot
            .graph
            .entities
            .get_mut(&subject)
            .unwrap()
            .fields
            .insert(
                "ready".to_owned(),
                Fact::known(
                    Value::Text("z".to_owned()),
                    GameTick(102),
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            );
        snapshot.tick = GameTick(102);
        snapshot.cursor.sequence = 2;
        snapshot.refresh_hash();
        assert_eq!(
            laboratory_decision(&step, &snapshot, |_| Some(CommitState::Verified)),
            DeferredStepDecision::Ready
        );
        runtime.step_tick_laboratory(&snapshot)?;
        assert!(matches!(
            runtime.get_status(ActionId::new(1)),
            Some(ObligationStatus::Fulfilled { .. })
        ));
        assert!(matches!(
            runtime.get_status(ActionId::new(2)),
            Some(ObligationStatus::Failed { .. })
        ));
        Ok(())
    }
}
