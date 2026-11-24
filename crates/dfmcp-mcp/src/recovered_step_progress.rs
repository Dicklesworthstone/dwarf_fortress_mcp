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

/// Preserve the real foreground owner's cadence rule. The monitor itself owns
/// its due-sample basis; last_observation_anchor is not a cadence origin.
pub(super) fn ceiling(now: GameTick, carried: &[CarriedStep]) -> Option<u64> {
    carried
        .iter()
        .filter(|step| !step.is_final())
        .filter_map(|step| step.work_step.as_ref()?.obligation.as_ref())
        .map(|spec| {
            let cadence = spec.poll_interval_ticks.max(1);
            let until_deadline = spec.deadline_tick.0.saturating_sub(now.0);
            if until_deadline == 0 {
                cadence
            } else {
                cadence.min(until_deadline)
            }
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
