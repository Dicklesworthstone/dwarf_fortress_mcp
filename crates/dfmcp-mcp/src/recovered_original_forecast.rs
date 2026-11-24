//! Exact recovered originals in a discarded complete-project model.
//! Historical receipts stay immutable; unfinished proof uses only cloned
//! authenticated carried monitors, never restored adapter action handles.

use super::super::super::{CarriedStep, recovered_step_progress, selected_plan_wait::RecoveredStepCustody};
use super::*;
use dfmcp_core::StateAnchor;
use dfmcp_intent::{ObligationStatus, PlanStep, PreparedPlan};

#[derive(Clone)]
pub(in super::super) struct RecoveredOriginal {
    original: Digest32,
    original_anchor: StateAnchor,
    recovery_anchor: StateAnchor,
    steps: Vec<(PlanStep, RecoveredStepCustody)>,
    evidence: Value,
}

pub(super) struct StepView {
    pub(super) state: CommitState,
    pub(super) work: EffectWorkState,
    pub(super) value: Value,
}

impl RecoveredOriginal {
    pub(in super::super) fn new(
        plan: &PreparedPlan,
        census: Vec<RecoveredStepCustody>,
        evidence: Value,
        current: &WorldSnapshot,
        carried: &[CarriedStep],
    ) -> Result<Self> {
        let first = census.first().ok_or_else(|| {
            error(ErrorCode::Conflict, "recovered forecast requires complete original step custody")
        })?;
        if !plan.digest_is_valid()
            || !plan.id_is_valid()
            || plan.steps.len() != census.len()
            || census.len() > 64
            || first.custody_digest == Digest32::ZERO
            || evidence["digest"] != first.custody_digest.to_hex()
            || census.iter().zip(&plan.steps).any(|(row, step)| {
                row.step != step.id
                    || !matches!(row.state, "verified" | "dispatched")
                    || row.custody_digest != first.custody_digest
                    || row.recovery_anchor != first.recovery_anchor
                    || (row.state == "verified"
                        && (row.proof_anchor.is_none() || !row.work.is_quiescent()))
                    || (row.state == "dispatched"
                        && (row.recorded_state != "dispatched"
                            || !matches!(row.work, EffectWorkState::Active { .. } | EffectWorkState::Quiescent { .. })))
            })
        {
            return Err(error(
                ErrorCode::Conflict,
                "recovered forecast requires complete Verified history or exact already-dispatched temporal work",
            ));
        }
        let recovered = Self {
            original: plan.digest,
            original_anchor: plan.anchor,
            recovery_anchor: first.recovery_anchor,
            steps: plan.steps.iter().cloned().zip(census).collect(),
            evidence,
        };
        for (step, receipt) in &recovered.steps {
            if receipt.state != "dispatched" {
                continue;
            }
            let row = recovered.carried_step(carried, step)?;
            if row.state != "dispatched"
                || row.recorded_state != receipt.recorded_state
                || row.proof_anchor != receipt.proof_anchor
                || row.monitor.as_ref().is_none_or(|monitor| {
                    !matches!(monitor.status(), Some(ObligationStatus::Active { .. }))
                })
                || dfmcp_intent::inspect_effect_work(current, &step.action, &step.idempotency_key, true)?
                    != receipt.work
            {
                return Err(error(ErrorCode::Conflict, "recovered forecast tip lost its admitted active monitor"));
            }
        }
        recovered.rows(current, carried)?;
        Ok(recovered)
    }

    pub(super) fn original(&self) -> Digest32 {
        self.original
    }

    pub(super) fn len(&self) -> usize {
        self.steps.len()
    }

    pub(super) fn historical_verified_steps(&self) -> usize {
        self.steps.iter().filter(|(_, receipt)| receipt.state == "verified").count()
    }

    pub(super) fn initially_complete(&self) -> bool {
        self.historical_verified_steps() == self.len()
    }

    /// A new live successor may follow only the accepted recovery frontier.
    /// An old sealed epoch alone cannot authorize crossing a reset.
    pub(super) fn validate_live_source(&self, anchor: StateAnchor) -> Result<()> {
        let recovery = self.recovery_anchor;
        if anchor.fortress_id != recovery.fortress_id
            || anchor.cursor.epoch != recovery.cursor.epoch
            || anchor.tick < recovery.tick
            || anchor.cursor.sequence < recovery.cursor.sequence
            || (anchor.cursor == recovery.cursor && anchor != recovery)
        {
            return Err(error(
                ErrorCode::Conflict,
                "live forecast continuation predates its accepted recovery frontier",
            ));
        }
        Ok(())
    }

    pub(super) fn shares_recovery(&self, other: &Self) -> bool {
        self.recovery_anchor == other.recovery_anchor
    }

    pub(super) fn validate_snapshot(&self, snapshot: &WorldSnapshot) -> Result<()> {
        self.validate_live_source(snapshot.anchor())
    }

    fn carried_step<'a>(&self, carried: &'a [CarriedStep], step: &PlanStep) -> Result<&'a CarriedStep> {
        let mut matches = carried.iter().filter(|row| {
            row.plan_digest == self.original && row.step == Some(step.id)
        });
        let row = matches.next().ok_or_else(|| {
            error(ErrorCode::Conflict, "recovered forecast lost its exact carried proof monitor")
        })?;
        if matches.next().is_some()
            || row.work_step.as_ref() != Some(step)
            || row.observation_error.is_some()
            || !recovered_step_progress::monitor_matches(row, self.original, self.original_anchor.tick, step)
            || row.monitor.as_ref().is_none_or(|monitor| monitor.recovery_anchor() != self.recovery_anchor)
        {
            return Err(error(ErrorCode::Conflict, "recovered forecast changed its original carried monitor identity"));
        }
        Ok(row)
    }

    /// Historical proof is never resampled. Private proof changes come only
    /// from the real observation transition running on the discarded world.
    pub(super) fn rows(&self, snapshot: &WorldSnapshot, carried: &[CarriedStep]) -> Result<Vec<StepView>> {
        self.validate_snapshot(snapshot)?;
        let mut rows = Vec::with_capacity(self.steps.len());
        for (step, receipt) in &self.steps {
            let work = dfmcp_intent::inspect_effect_work(
                snapshot, &step.action, &step.idempotency_key, true,
            )?;
            let mut value = json!({
                "step":step.id.get(), "action_id":null,
                "recorded_state":receipt.recorded_state,
                "recorded_anchor":super::super::super::anchor_json(&receipt.recorded_anchor),
                "proof_anchor":receipt.proof_anchor.as_ref().map(super::super::super::anchor_json),
                "predicted_physical_work":work_json(&work),
                "obligation_deadline_tick":step.obligation.as_ref().map(|obligation|obligation.deadline_tick.0),
                "dispatch_handle_restored":false,
            });
            let state = if receipt.state == "verified" {
                if !work.is_quiescent() {
                    return Err(error(
                        ErrorCode::Conflict,
                        "historical recovered work is no longer provably quiet in the forecast",
                    ));
                }
                value["action_state_origin"] = json!("verified_recovered_history");
                CommitState::Verified
            } else {
                let row = self.carried_step(carried, step)?;
                if !matches!(work, EffectWorkState::Active { .. } | EffectWorkState::Quiescent { .. }) {
                    return Err(error(ErrorCode::Conflict, "recovered modeled physical work became unknown"));
                }
                let state = match row.state.as_str() {
                    "dispatched" => CommitState::AppliedAwaitingVerification,
                    "verified" => CommitState::Verified,
                    "failed" => CommitState::Failed,
                    _ => return Err(error(ErrorCode::Conflict, "recovered modeled proof changed disposition")),
                };
                value["action_state_origin"] = json!("cloned_recovered_observation_monitor");
                value["predicted_proof_anchor"] = if state.is_terminal() {
                    row.proof_anchor.as_ref().map(super::super::super::anchor_json).unwrap_or(Value::Null)
                } else {
                    Value::Null
                };
                value["predicted_proof_is_canonical"] = json!(false);
                value["predicted_failure_reason"] = json!(row.failure_reason);
                value["predicted_stability"] = row.monitor.as_ref().and_then(|monitor| match monitor.status() {
                    Some(ObligationStatus::Active { consecutive_stable_observations, .. }) => Some(json!({
                        "consecutive_observations":consecutive_stable_observations,
                        "required_observations":step.obligation.as_ref().map(|spec|spec.stable_for_observations),
                        "recovery_anchor":super::super::super::anchor_json(&monitor.recovery_anchor()),
                    })),
                    _ => None,
                }).unwrap_or(Value::Null);
                state
            };
            value["predicted_state"] = json!(format!("{state:?}"));
            rows.push(StepView { state, work, value });
        }
        Ok(rows)
    }

    pub(super) fn annotate(&self, value: &mut Value) {
        value["selection_origin"] = json!("durable_recovery");
        value["owns_original_dispatch"] = json!(false);
        value["dispatch_handles_restored"] = json!(false);
        value["recovery_evidence"] = self.evidence.clone();
        value["original_source_anchor"] = super::super::super::anchor_json(&self.original_anchor);
        value["recovered_observation_prediction"] = json!(!self.initially_complete());
    }
}
