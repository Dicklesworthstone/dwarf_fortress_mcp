//! Retired original steps need positive dispatch custody, not inferred absence.
//!
//! The durable store retains only anchored `not_dispatched` records when the
//! owning commit retires. Recovery checks that custody against the exact sealed
//! plan and archived world before a goal may use it as quiescence evidence.

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, StateAnchor};
use dfmcp_intent::{Action, PlanStep, PreparedPlan, effects};
use dfmcp_lab::durable::{DurableObjective, MAX_STEPS_PER_COMMIT};
use dfmcp_world::WorldSnapshot;

use super::goal_continuation::RecoveredGoalLineage;

fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message).retryable(false)
}

/// Compare exact anchored history without treating an epoch reset as a fork.
fn precedes(before: StateAnchor, after: StateAnchor) -> bool {
    before.fortress_id == after.fortress_id
        && before.tick <= after.tick
        && before.cursor.epoch <= after.cursor.epoch
        && (before.cursor.epoch < after.cursor.epoch
            || before.cursor.sequence <= after.cursor.sequence)
        && (before.cursor != after.cursor || before == after)
}

/// Presence contradicts an undispatched creator's retained disposition.
/// Absence by itself is never sufficient to construct a proof.
pub(super) fn effect_identity_absent(step: &PlanStep, snapshot: &WorldSnapshot) -> bool {
    !matches!(
        step.action,
        Action::DesignateDig { .. }
            | Action::Build { .. }
            | Action::PlaceFurniture { .. }
            | Action::CreateWorkOrder { .. }
    ) || !snapshot
        .graph
        .entities
        .contains_key(&effects::created_entity_id(&step.idempotency_key, 0))
}

/// The first opening of a still-retained commit and later retired-goal recovery
/// must apply the same checks to an anchored nondispatch record. This checks
/// evidence only; the recovery owner separately removes all old dispatch handles.
pub(super) fn verify_nondispatch_frontier(
    plan: &PreparedPlan,
    step_id: u32,
    anchor: StateAnchor,
    archived: &WorldSnapshot,
    upper: Option<StateAnchor>,
) -> Result<()> {
    plan.validate_structure()?;
    if !plan.digest_is_valid() || !plan.id_is_valid() {
        return Err(corrupt(
            "nondispatch custody has an invalid original plan seal",
        ));
    }
    let step = plan
        .steps
        .iter()
        .find(|step| step.id.get() == step_id)
        .ok_or_else(|| corrupt("nondispatch custody names a step outside the original plan"))?;
    if !archived.hash_is_valid()
        || archived.anchor() != anchor
        || !precedes(plan.anchor, anchor)
        || !effect_identity_absent(step, archived)
    {
        return Err(corrupt(
            "nondispatch custody lacks its exact uncontradicted archived frontier",
        ));
    }
    if upper.is_some_and(|upper| !precedes(anchor, upper)) {
        return Err(corrupt(
            "nondispatch custody is newer than the verified recovery head",
        ));
    }
    Ok(())
}

/// Only verified recovery can construct this process-local interpretation of
/// durable retirement custody. It grants neither observation nor dispatch.
#[derive(Clone, Debug)]
pub(super) struct RetiredNondispatchProof {
    plan_digest: Digest32,
    step: u32,
    anchor: StateAnchor,
}

impl RetiredNondispatchProof {
    pub(super) fn verify(
        plan: &PreparedPlan,
        retained: &DurableObjective,
        step_id: u32,
        archived: &WorldSnapshot,
        recovery: &RecoveredGoalLineage,
    ) -> Result<Self> {
        if plan.digest != retained.plan_digest
            || plan.intent_id.get() != retained.intent_id
            || plan.anchor.state_hash != retained.sealed_state_hash
            || plan.anchor.fortress_id != retained.fortress_id
            || retained.retired_never_dispatched.len() > MAX_STEPS_PER_COMMIT
        {
            return Err(corrupt(
                "retired nondispatch custody does not bind the exact original sealed plan",
            ));
        }
        let anchor = retained
            .retired_never_dispatched
            .get(&step_id)
            .copied()
            .ok_or_else(|| corrupt("the original step has no retained nondispatch custody"))?;
        // A restore can rewind the world after abandoning a goal. Its historical
        // disposition remains readable, but can never reactivate that pursuit.
        verify_nondispatch_frontier(
            plan,
            step_id,
            anchor,
            archived,
            retained
                .restore_abandoned_anchor
                .is_none()
                .then_some(recovery.previous_anchor()),
        )?;
        Ok(Self {
            plan_digest: plan.digest,
            step: step_id,
            anchor,
        })
    }

    pub(super) const fn anchor(&self) -> StateAnchor {
        self.anchor
    }

    /// The owner additionally requires current Observe, retired original action
    /// handles and no restore abandonment before allowing goal continuation.
    pub(super) fn covers(
        &self,
        plan: &PreparedPlan,
        step: &PlanStep,
        current: &WorldSnapshot,
    ) -> bool {
        self.plan_digest == plan.digest
            && self.step == step.id.get()
            && current.hash_is_valid()
            && precedes(self.anchor, current.anchor())
            && effect_identity_absent(step, current)
    }
}

#[cfg(test)]
#[path = "goal_custody_tests.rs"]
mod tests;
