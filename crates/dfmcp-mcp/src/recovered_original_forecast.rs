//! Immutable verified original steps in a discarded complete-project model.
//! These rows never become adapter action handles or observation runtimes.

use super::super::super::selected_plan_wait::RecoveredStepCustody;
use super::*;
use dfmcp_core::StateAnchor;
use dfmcp_intent::{PlanStep, PreparedPlan};

#[derive(Clone)]
pub(in super::super) struct RecoveredOriginal {
    original: Digest32,
    original_anchor: StateAnchor,
    recovery_anchor: StateAnchor,
    steps: Vec<(PlanStep, RecoveredStepCustody)>,
    evidence: Value,
}

impl RecoveredOriginal {
    pub(in super::super) fn new(
        plan: &PreparedPlan,
        census: Vec<RecoveredStepCustody>,
        evidence: Value,
        current: &WorldSnapshot,
    ) -> Result<Self> {
        let first = census.first().ok_or_else(|| {
            error(
                ErrorCode::Conflict,
                "recovered forecast requires complete original step custody",
            )
        })?;
        if !plan.digest_is_valid()
            || !plan.id_is_valid()
            || plan.steps.len() != census.len()
            || census.len() > 64
            || first.custody_digest == Digest32::ZERO
            || evidence["digest"] != first.custody_digest.to_hex()
            || census.iter().zip(&plan.steps).any(|(row, step)| {
                row.step != step.id
                    || row.state != "verified"
                    || row.proof_anchor.is_none()
                    || row.custody_digest != first.custody_digest
                    || row.recovery_anchor != first.recovery_anchor
                    || !row.work.is_quiescent()
            })
        {
            return Err(error(
                ErrorCode::Conflict,
                "recovered forecast requires the exact complete Verified, physically quiet original census",
            ));
        }
        let recovered = Self {
            original: plan.digest,
            original_anchor: plan.anchor,
            recovery_anchor: first.recovery_anchor,
            steps: plan.steps.iter().cloned().zip(census).collect(),
            evidence,
        };
        recovered.rows(current)?;
        Ok(recovered)
    }

    pub(super) fn original(&self) -> Digest32 {
        self.original
    }

    pub(super) fn len(&self) -> usize {
        self.steps.len()
    }

    /// Every modeled descendant must still prove quiet original physical work.
    /// Historical Verified proof is immutable and is never resampled or renewed.
    pub(super) fn rows(&self, snapshot: &WorldSnapshot) -> Result<Vec<Value>> {
        let anchor = snapshot.anchor();
        if anchor.fortress_id != self.recovery_anchor.fortress_id
            || anchor.cursor.epoch != self.recovery_anchor.cursor.epoch
            || anchor.tick < self.recovery_anchor.tick
            || anchor.cursor.sequence < self.recovery_anchor.cursor.sequence
            || (anchor.cursor == self.recovery_anchor.cursor && anchor != self.recovery_anchor)
        {
            return Err(error(
                ErrorCode::Conflict,
                "recovered forecast left its verified active recovery lineage",
            ));
        }
        let mut rows = Vec::with_capacity(self.steps.len());
        for (step, receipt) in &self.steps {
            let work = dfmcp_intent::inspect_effect_work(
                snapshot,
                &step.action,
                &step.idempotency_key,
                true,
            )?;
            if !work.is_quiescent() {
                return Err(error(
                    ErrorCode::Conflict,
                    "recovered original physical work is no longer provably quiet in the forecast",
                ));
            }
            rows.push(json!({
                "step":step.id.get(), "action_id":null,
                "predicted_state":"Verified",
                "action_state_origin":"verified_recovered_history",
                "recorded_state":receipt.recorded_state,
                "recorded_anchor":super::super::super::anchor_json(&receipt.recorded_anchor),
                "proof_anchor":receipt.proof_anchor.as_ref().map(super::super::super::anchor_json),
                "predicted_physical_work":work_json(&work),
                "obligation_deadline_tick":step.obligation.as_ref().map(|obligation|obligation.deadline_tick.0),
                "dispatch_handle_restored":false,
            }));
        }
        Ok(rows)
    }

    pub(super) fn annotate(&self, value: &mut Value) {
        value["selection_origin"] = json!("durable_recovery");
        value["owns_original_dispatch"] = json!(false);
        value["dispatch_handles_restored"] = json!(false);
        value["recovery_evidence"] = self.evidence.clone();
        value["original_source_anchor"] = super::super::super::anchor_json(&self.original_anchor);
    }
}
