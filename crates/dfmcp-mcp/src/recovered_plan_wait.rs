//! Exact action evidence for an observation-only recovered original wait.
//!
//! Capture every anchored step before recovery can retire its old commit.
//! Historical action proof and current physical work remain independent. An
//! archived goal predicate cannot replace a missing per-step receipt.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, Digest32, ErrorCode, Result, StateAnchor, StepId};
use dfmcp_intent::{EffectWorkState, ObligationStatus, PlanStep, PreparedPlan, effects};
use dfmcp_lab::durable::{DurableLabStore, DurableObjective};
use dfmcp_world::{Predicate, PredicateEvidence, PredicateTruth, WorldSnapshot};
use serde_json::{Value, json};

use super::super::{
    CarriedStep, LabSession, goal_continuation::RecoveredGoalLineage, goal_custody,
};
use super::{Actions, MAX_SOURCE_BYTES, MAX_STEPS, anchor_json};

fn unavailable(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
        .with_detail("reason", "action_evidence_unavailable")
}

fn corrupt(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::CorruptLedger, message)
        .with_detail("reason", "invalid_recovered_action_custody")
}

fn precedes(before: StateAnchor, after: StateAnchor) -> bool {
    before.fortress_id == after.fortress_id
        && before.tick <= after.tick
        && before.cursor.epoch <= after.cursor.epoch
        && (before.cursor.epoch < after.cursor.epoch
            || before.cursor.sequence <= after.cursor.sequence)
        && (before.cursor != after.cursor || before == after)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Dispatched,
    Verified,
    Failed,
    Cancelled,
    Compensated,
    NeverDispatched,
}

impl State {
    fn parse(value: &str) -> Result<Self> {
        match value {
            "dispatched" => Ok(Self::Dispatched),
            "verified" => Ok(Self::Verified),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "compensated" => Ok(Self::Compensated),
            "not_dispatched" => Ok(Self::NeverDispatched),
            _ => Err(unavailable(
                "the original step has no interpretable anchored action disposition",
            )),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Dispatched => "dispatched",
            Self::Verified => "verified",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Compensated => "compensated",
            Self::NeverDispatched => "not_dispatched",
        }
    }

    fn tag(self) -> u8 {
        match self {
            Self::Dispatched => 0,
            Self::Verified => 1,
            Self::Failed => 2,
            Self::Cancelled => 3,
            Self::Compensated => 4,
            Self::NeverDispatched => 5,
        }
    }

    fn count(self, actions: &mut Actions) {
        actions.verified += usize::from(self == Self::Verified);
        actions.terminal += usize::from(self != Self::Dispatched);
        actions.not_dispatched += usize::from(self == Self::NeverDispatched);
        actions.failed |= self == Self::Failed;
        actions.cancelled |= matches!(self, Self::Cancelled | Self::Compensated);
    }
}

#[derive(Clone, Debug)]
struct Receipt {
    step: StepId,
    state: State,
    anchor: StateAnchor,
}

#[derive(Clone, Debug)]
struct VerifiedCustody {
    plan_digest: Digest32,
    recovery_anchor: StateAnchor,
    archived_after_retirement: bool,
    receipts: Vec<Receipt>,
    digest: Digest32,
    canonical_bytes: usize,
    archived_sources: usize,
    archived_bytes: usize,
}

/// Only recovery of the exact original source can construct this custody.
/// Retain failures separately from the ordinary goal: retired legacy action
/// history may be unavailable while the original goal is still observable.
#[derive(Clone, Debug)]
pub(crate) struct RecoveredPlanCustody {
    verified: Result<VerifiedCustody>,
}

impl RecoveredPlanCustody {
    pub(crate) fn capture(
        store: &DurableLabStore,
        retained: &DurableObjective,
        plan: &PreparedPlan,
        recovery: &RecoveredGoalLineage,
    ) -> Self {
        Self {
            verified: Self::verify(store, retained, plan, recovery),
        }
    }

    fn verify(
        store: &DurableLabStore,
        retained: &DurableObjective,
        plan: &PreparedPlan,
        recovery: &RecoveredGoalLineage,
    ) -> Result<VerifiedCustody> {
        plan.validate_structure()?;
        if !plan.digest_is_valid()
            || !plan.id_is_valid()
            || plan.steps.is_empty()
            || plan.steps.len() > MAX_STEPS
            || plan.digest != retained.plan_digest
            || plan.intent_id.get() != retained.intent_id
            || plan.anchor.state_hash != retained.sealed_state_hash
            || plan.anchor.fortress_id != retained.fortress_id
            || !precedes(plan.anchor, recovery.previous_anchor())
        {
            return Err(corrupt(
                "recovered wait has no exact bounded original plan seal",
            ));
        }
        let (commit, archived_after_retirement) = match (
            store.commit(retained.fortress_id, retained.plan_digest),
            store.retired_commit(retained.fortress_id, retained.plan_digest),
        ) {
            (Some(commit), None) => (commit, false),
            (None, Some(commit)) => (commit, true),
            (Some(_), Some(_)) => {
                return Err(corrupt(
                    "the original action map is both active and retired",
                ));
            }
            (None, None) => {
                return Err(unavailable(
                    "the original action history is unavailable; retained goal truth cannot reconstruct its missing action receipts",
                ));
            }
        };
        if commit.fortress_id != retained.fortress_id
            || commit.plan_digest != retained.plan_digest
            || commit.intent_id != retained.intent_id
            || commit.sealed_state_hash != retained.sealed_state_hash
            || commit.source != retained.source
            || commit.steps.len() != plan.steps.len()
            || commit.step_anchors.len() != plan.steps.len()
        {
            return Err(unavailable(
                "the recovered original requires a complete exact anchored step receipt map",
            ));
        }
        let mut receipts = Vec::with_capacity(plan.steps.len());
        let mut sources = BTreeMap::<Digest32, StateAnchor>::new();
        for step in &plan.steps {
            let state = State::parse(
                commit
                    .steps
                    .get(&step.id.get())
                    .ok_or_else(|| unavailable("an original step receipt is missing"))?,
            )?;
            if archived_after_retirement && state == State::Dispatched {
                return Err(corrupt(
                    "retired original action custody contains unfinished dispatch",
                ));
            }
            let anchor = commit
                .step_anchors
                .get(&step.id.get())
                .copied()
                .ok_or_else(|| unavailable("an original step receipt lacks its atomic frontier"))?;
            if !precedes(plan.anchor, anchor) || !precedes(anchor, recovery.previous_anchor()) {
                return Err(corrupt(
                    "an original action receipt lies outside its recovery lineage",
                ));
            }
            if sources
                .insert(anchor.state_hash, anchor)
                .is_some_and(|prior| prior != anchor)
            {
                return Err(corrupt(
                    "one archived receipt object names conflicting anchors",
                ));
            }
            receipts.push(Receipt {
                step: step.id,
                state,
                anchor,
            });
        }
        let mut archived_bytes = 0usize;
        for (hash, anchor) in &sources {
            let archived = store.load_snapshot(*hash)?;
            archived_bytes = archived_bytes.saturating_add(archived.canonical_bytes().len());
            if archived_bytes > MAX_SOURCE_BYTES as usize {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "recovered action proof exceeds its complete archived-source bound",
                ));
            }
            if !archived.hash_is_valid() || archived.anchor() != *anchor {
                return Err(corrupt(
                    "an original action receipt lacks its exact archived world",
                ));
            }
            for (receipt, step) in receipts.iter().zip(&plan.steps) {
                if receipt.anchor != *anchor {
                    continue;
                }
                match receipt.state {
                    State::Verified => verify_terminal(step, &archived)?,
                    State::NeverDispatched => goal_custody::verify_nondispatch_frontier(
                        plan,
                        step.id.get(),
                        receipt.anchor,
                        &archived,
                        Some(recovery.previous_anchor()),
                    )?,
                    _ => {}
                }
            }
        }
        let mut bytes = b"dfmcp-recovered-plan-wait-custody/1\0".to_vec();
        bytes.extend_from_slice(plan.digest.as_bytes());
        bytes.push(u8::from(archived_after_retirement));
        bytes.extend_from_slice(&retained.owner_session_id.get().to_be_bytes());
        append_anchor(&mut bytes, plan.anchor);
        append_anchor(&mut bytes, recovery.previous_anchor());
        append_anchor(&mut bytes, recovery.active_anchor());
        bytes.extend_from_slice(&(receipts.len() as u32).to_be_bytes());
        for receipt in &receipts {
            bytes.extend_from_slice(&receipt.step.get().to_be_bytes());
            bytes.push(receipt.state.tag());
            append_anchor(&mut bytes, receipt.anchor);
        }
        Ok(VerifiedCustody {
            plan_digest: plan.digest,
            recovery_anchor: recovery.active_anchor(),
            archived_after_retirement,
            receipts,
            digest: Digest32::of_bytes(&bytes),
            canonical_bytes: bytes.len(),
            archived_sources: sources.len(),
            archived_bytes,
        })
    }

    fn checked(&self) -> Result<&VerifiedCustody> {
        self.verified.as_ref().map_err(Clone::clone)
    }

    pub(crate) fn validate(
        &self,
        plan: &PreparedPlan,
        active: StateAnchor,
        current: StateAnchor,
    ) -> Result<()> {
        let custody = self.checked()?;
        if custody.plan_digest != plan.digest
            || custody.recovery_anchor != active
            || active.cursor.epoch != current.cursor.epoch
            || !precedes(active, current)
            || custody.receipts.len() != plan.steps.len()
            || !plan.digest_is_valid()
            || !plan.id_is_valid()
        {
            return Err(corrupt(
                "recovered selected wait lost its exact original plan or active recovery lineage",
            ));
        }
        Ok(())
    }

    pub(crate) fn digest(&self) -> Result<Digest32> {
        Ok(self.checked()?.digest)
    }

    pub(crate) fn bytes(&self) -> Result<usize> {
        Ok(self.checked()?.canonical_bytes)
    }

    pub(crate) fn evidence_json(&self) -> Result<Value> {
        let custody = self.checked()?;
        Ok(json!({
            "schema":"dfmcp.recovered-plan-wait-custody/1",
            "digest":custody.digest.to_hex(),
            "recovery_anchor":anchor_json(&custody.recovery_anchor),
            "original_step_count":custody.receipts.len(),
            "verified_archived_sources":custody.archived_sources,
            "verified_archived_bytes":custody.archived_bytes,
            "historical_proof":"exact_archived_atomic_step_receipts",
            "action_record_origin":if custody.archived_after_retirement {
                "archived_retired_commit"
            } else { "retained_commit_at_recovery" },
            "current_proof":"original_carried_observation_monitors",
            "retired_action_history_inferred":false,
            "restores_dispatch_authority":false,
        }))
    }

    pub(super) fn inspect(&self, session: &LabSession, plan: &PreparedPlan) -> Result<Actions> {
        let custody = self.checked()?;
        let snapshot = session.adapter.snapshot();
        let mut carried = BTreeMap::new();
        for row in session
            .carried
            .iter()
            .filter(|row| row.plan_digest == plan.digest)
        {
            let step = row.step.ok_or_else(|| {
                unavailable("selected original has an unverifiable carried action")
            })?;
            if !plan.steps.iter().any(|original| original.id == step)
                || carried.insert(step, row).is_some()
            {
                return Err(corrupt(
                    "selected original has duplicate or foreign carried action rows",
                ));
            }
        }
        let mut actions = Actions {
            total: plan.steps.len(),
            ..Actions::default()
        };
        for (receipt, step) in custody.receipts.iter().zip(&plan.steps) {
            if receipt.step != step.id {
                return Err(corrupt("selected original action ordering changed"));
            }
            if let Some(row) = carried.get(&step.id) {
                inspect_carried(
                    row,
                    receipt,
                    step,
                    custody.recovery_anchor,
                    snapshot.anchor(),
                    &mut actions,
                )?;
            } else if receipt.state == State::Dispatched {
                return Err(unavailable(
                    "a dispatched original step lost its recovered observation monitor",
                ));
            } else {
                receipt.state.count(&mut actions);
            }
            // Old terminal proof never establishes present physical quiet.
            // Nondispatch is different: it requires positive archived custody
            // plus continued absence of its exact original creator identity.
            let work = if receipt.state == State::NeverDispatched {
                if !goal_custody::effect_identity_absent(step, snapshot) {
                    return Err(corrupt(
                        "current original effect identity contradicts archived nondispatch",
                    ));
                }
                EffectWorkState::NeverDispatched
            } else {
                dfmcp_intent::inspect_effect_work(
                    snapshot,
                    &step.action,
                    &step.idempotency_key,
                    true,
                )?
            };
            match work {
                EffectWorkState::Active { .. } => actions.active += 1,
                EffectWorkState::Unknown { .. } => actions.unknown += 1,
                EffectWorkState::NeverDispatched | EffectWorkState::Quiescent { .. } => {}
            }
        }
        Ok(actions)
    }
}

fn inspect_carried(
    row: &CarriedStep,
    receipt: &Receipt,
    step: &PlanStep,
    recovery: StateAnchor,
    current: StateAnchor,
    actions: &mut Actions,
) -> Result<()> {
    if row.work_step.as_ref() != Some(step)
        || row.kind != crate::lab_world::action_kind(&step.action)
        || row.proof != proof(step)
        || row.failure
            != step
                .obligation
                .as_ref()
                .and_then(|spec| spec.failure.clone())
        || row.deadline != step.obligation.as_ref().map(|spec| spec.deadline_tick)
        || row.recorded_state != receipt.state.name()
        || row.work_anchor.is_none_or(|anchor| {
            anchor.cursor.epoch != recovery.cursor.epoch
                || !precedes(recovery, anchor)
                || !precedes(anchor, current)
        })
    {
        return Err(corrupt(
            "carried action no longer binds its exact recovered original step",
        ));
    }
    if row.state == "indeterminate" || row.observation_error.is_some() {
        actions.indeterminate = true;
        return Ok(());
    }
    let state = State::parse(&row.state)?;
    if receipt.state != State::Dispatched {
        if state != receipt.state || row.proof_anchor != Some(receipt.anchor) {
            return Err(corrupt(
                "an immutable recovered terminal action receipt changed",
            ));
        }
    } else {
        if !matches!(state, State::Dispatched | State::Verified | State::Failed) {
            return Err(corrupt(
                "recovery changed an original dispatch into a different disposition",
            ));
        }
        if let Some(monitor) = &row.monitor {
            if monitor.recovery_anchor() != recovery {
                return Err(corrupt(
                    "carried proof monitor belongs to a different recovery",
                ));
            }
            let consistent = match (state, monitor.status()) {
                (State::Dispatched, Some(ObligationStatus::Active { .. })) => true,
                // An already expired recovery frontier may have failed before
                // the normal owner publishes its first current observation.
                (State::Dispatched, Some(ObligationStatus::Failed { .. })) => true,
                (State::Verified, Some(ObligationStatus::Fulfilled { .. }))
                | (State::Failed, Some(ObligationStatus::Failed { .. })) => {
                    row.proof_anchor == monitor.last_observation_anchor()
                }
                _ => false,
            };
            if !consistent {
                return Err(corrupt(
                    "carried action state disagrees with its original proof monitor",
                ));
            }
        } else if step.obligation.is_some() {
            return Err(unavailable(
                "an unfinished original temporal action lost its proof monitor",
            ));
        }
        if state != State::Dispatched
            && row.proof_anchor.is_none_or(|anchor| {
                anchor.cursor.epoch != recovery.cursor.epoch
                    || !precedes(recovery, anchor)
                    || !precedes(anchor, current)
            })
        {
            return Err(corrupt(
                "recovered terminal proof lacks its exact current observation frontier",
            ));
        }
    }
    state.count(actions);
    Ok(())
}

fn proof(step: &PlanStep) -> Predicate {
    let mut predicates = step.postconditions.clone();
    if let Some(spec) = &step.obligation {
        predicates.push(spec.terminal.clone());
    }
    Predicate::All(predicates).normalized()
}

fn verify_terminal(step: &PlanStep, snapshot: &WorldSnapshot) -> Result<()> {
    let evidence = PredicateEvidence::laboratory(snapshot)?;
    if let Some(failure) = step
        .obligation
        .as_ref()
        .and_then(|spec| spec.failure.as_ref())
        && evidence.evaluate(failure)? != PredicateTruth::False
    {
        return Err(corrupt(
            "the archived verified action does not establish its original failure predicate as false",
        ));
    }
    if step
        .obligation
        .as_ref()
        .is_some_and(|spec| snapshot.tick > spec.deadline_tick)
        || !evidence.establishes(&proof(step))?
    {
        return Err(corrupt(
            "the exact archived verified action does not prove its original terminal predicate",
        ));
    }
    effects::validate_terminal_action_identity(snapshot, step)
}

fn append_anchor(bytes: &mut Vec<u8>, anchor: StateAnchor) {
    bytes.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&anchor.tick.0.to_be_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
}

#[cfg(test)]
#[path = "selected_plan_recovery_wait_tests.rs"]
mod tests;
