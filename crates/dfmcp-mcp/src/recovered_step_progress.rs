//! Pure recovered proof progression shared by real waiting and private models.
//! The caller supplies current Observe authority and complete original custody.
//! No action handle is restored and no dispatch or observation is published here.

use super::CarriedStep;
use dfmcp_core::{ActionId, Digest32, GameTick, Result, StepId};
use dfmcp_intent::{Action, ObligationStatus, PlanStep};
use dfmcp_world::{Predicate, PredicateEvidence, WorldSnapshot};

pub(super) fn proof_id(plan: Digest32, step: StepId) -> ActionId {
    let mut identity = b"dfmcp-recovered-obligation-v1".to_vec();
    identity.extend_from_slice(plan.as_bytes());
    identity.extend_from_slice(&step.get().to_be_bytes());
    ActionId::new(Digest32::of_bytes(&identity).first_u128().max(1))
}

pub(super) fn monitor_matches(
    row: &CarriedStep,
    plan: Digest32,
    registered_tick: GameTick,
    step: &PlanStep,
) -> bool {
    let Some(monitor) = &row.monitor else {
        return false;
    };
    let Some(mut spec) = step.obligation.clone() else {
        return false;
    };
    let mut predicates = step.postconditions.clone();
    predicates.push(spec.terminal.clone());
    spec.terminal = Predicate::All(predicates).normalized();
    monitor.matches_original(proof_id(plan, step.id), &spec, registered_tick)
}

/// Preserve the retained monitor's eligible-sample floor across short waits
/// and off-cadence reads. Every owner observes the current frontier before its
/// first advance; the positive external ceiling must not create a zero-time
/// loop when that initial frontier is already due or expired.
pub(super) fn ceiling(now: GameTick, carried: &[CarriedStep]) -> Option<u64> {
    carried
        .iter()
        .filter(|step| !step.is_final())
        .filter_map(|step| {
            let spec = step.work_step.as_ref()?.obligation.as_ref()?;
            if let Some(due) = step
                .monitor
                .as_ref()
                .and_then(|monitor| monitor.next_observation_tick())
            {
                return Some(due.0.saturating_sub(now.0).max(1));
            }
            // Unresolved historical rows without an active monitor cannot
            // establish a positive sample schedule. Preserve their existing
            // bounded inspection interval without manufacturing a monitor.
            let cadence = spec.poll_interval_ticks.max(1);
            let until_deadline = spec.deadline_tick.0.saturating_sub(now.0);
            Some(if until_deadline == 0 {
                cadence
            } else {
                cadence.min(until_deadline)
            })
        })
        .min()
}

/// Return a complete private candidate. Failure leaves every caller-owned row
/// untouched, including stability and historical proof anchors.
pub(super) fn observe(snapshot: &WorldSnapshot, carried: &[CarriedStep]) -> Result<Vec<CarriedStep>> {
    let evidence = PredicateEvidence::laboratory(snapshot)?;
    let mut next = carried.to_vec();
    for step in &mut next {
        if step.is_final() {
            continue;
        }
        step.observation_error = None;
        if let Some(work) = &step.work_step {
            // An anchored failure may have preceded dispatch; the legacy
            // journal does not distinguish that case. Never infer absence
            // from a missing work entity: retain Unknown for reconciliation.
            step.work_state = dfmcp_intent::inspect_effect_work(
                snapshot,
                &work.action,
                &work.idempotency_key,
                true,
            )?;
            step.work_anchor = Some(snapshot.anchor());
            if step.state == "dispatched"
                && matches!(work.action, Action::HarvestResource { .. })
            {
                dfmcp_intent::effects::validate_harvest_identity(
                    snapshot,
                    dfmcp_intent::effects::created_entity_id(&work.idempotency_key, 0),
                    &work.action,
                )?;
            }
        }
        if step.state != "dispatched" {
            continue;
        }
        if let Some(monitor) = &mut step.monitor {
            monitor.observe_with_evidence(&evidence)?;
            match monitor.status() {
                Some(ObligationStatus::Fulfilled { .. }) => {
                    step.state = "verified".to_owned();
                    step.proof_anchor = monitor.last_observation_anchor();
                }
                Some(ObligationStatus::Failed { reason, .. }) => {
                    step.state = "failed".to_owned();
                    step.failure_reason = Some(reason.clone());
                    step.proof_anchor = monitor.last_observation_anchor();
                }
                _ => {}
            }
        } else if evidence.establishes(&step.proof)? {
            step.state = "verified".to_owned();
            step.proof_anchor = Some(snapshot.anchor());
        }
        if step.state == "verified"
            && let Some(work) = &step.work_step
        {
            // Recovery has the same proof obligations as live execution.
            // Matching field predicates cannot certify a replaced entity or
            // broken input/item custody. Refusal discards the shadow rows,
            // so neither the monitor nor durable state becomes Verified.
            dfmcp_intent::effects::validate_terminal_action_identity(snapshot, work)?;
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{Capability, FortressId, ObservationCursor, RiskTier};
    use dfmcp_intent::{EffectWorkState, ObligationSpec, RecoveredObligation};
    use dfmcp_world::WorldGraph;

    fn snapshot(tick: u64) -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(826_401),
            GameTick(tick),
            ObservationCursor { epoch: 2, sequence: tick },
            false,
            WorldGraph::default(),
        )
    }

    fn carried(frontier: &WorldSnapshot) -> Result<CarriedStep> {
        let original = Digest32::of_bytes(b"original recovered cadence fixture");
        let step = PlanStep {
            id: StepId::new(0),
            action: Action::Pause { paused: false },
            preconditions: Vec::new(),
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: Some(ObligationSpec {
                terminal: Predicate::Paused(false),
                failure: None,
                deadline_tick: GameTick(25),
                poll_interval_ticks: 10,
                stable_for_observations: 3,
            }),
            depends_on: Vec::new(),
            risk: RiskTier::Reversible,
            required_capability: Capability::ControlClock,
            idempotency_key: "recovered-cadence-fixture".to_owned(),
        };
        let monitor = RecoveredObligation::new_laboratory(
            proof_id(original, step.id),
            step.obligation.clone().ok_or_else(|| dfmcp_core::DfmcpError::new(
                dfmcp_core::ErrorCode::InvalidPlan, "fixture lost its original obligation",
            ))?,
            GameTick(0),
            frontier,
        )?;
        Ok(CarriedStep {
            plan_digest: original,
            step: Some(step.id),
            kind: "pause",
            proof: Predicate::Paused(false),
            failure: None,
            deadline: Some(GameTick(25)),
            monitor: Some(monitor),
            recorded_state: "dispatched".to_owned(),
            proof_anchor: None,
            observation_error: None,
            failure_reason: None,
            work_step: Some(step),
            work_state: EffectWorkState::Quiescent { entity_id: None },
            work_anchor: Some(frontier.anchor()),
            recovered_drain: None,
            state: "dispatched".to_owned(),
        })
    }

    #[test]
    fn recovered_owner_resumes_at_the_next_eligible_sample_after_a_short_wait() -> Result<()> {
        let original = vec![carried(&snapshot(0))?];
        let retained = format!("{original:?}");
        let mut observed = observe(&snapshot(7), &original)?;
        assert_eq!(ceiling(GameTick(7), &observed), Some(3));
        assert_eq!(observed[0].state, "dispatched");
        assert_eq!(format!("{original:?}"), retained);
        for tick in [10, 20, 25] {
            observed = observe(&snapshot(tick), &observed)?;
        }
        assert_eq!(observed[0].state, "verified");
        assert_eq!(observed[0].proof_anchor.map(|anchor| anchor.tick), Some(GameTick(25)));
        assert_eq!(observed[0].deadline, Some(GameTick(25)));
        assert_eq!(observed[0].work_step, original[0].work_step);
        assert_eq!(ceiling(GameTick(25), &observed), None);
        Ok(())
    }

    #[test]
    fn due_recovered_frontiers_keep_a_positive_ceiling_and_are_observed_before_progress() -> Result<()> {
        let original = vec![carried(&snapshot(0))?];
        assert_eq!(ceiling(GameTick(10), &original), Some(1));
        let observed = observe(&snapshot(10), &original)?;
        assert_eq!(ceiling(GameTick(10), &observed), Some(10));
        let expired = vec![carried(&snapshot(26))?];
        assert!(ceiling(GameTick(26), &expired).is_some_and(|ticks| ticks > 0));
        let observed = observe(&snapshot(26), &expired)?;
        assert_eq!(observed[0].state, "failed");
        assert_eq!(ceiling(GameTick(26), &observed), None);
        Ok(())
    }
}
