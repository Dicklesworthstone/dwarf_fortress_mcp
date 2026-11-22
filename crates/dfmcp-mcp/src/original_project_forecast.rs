//! Hypothetical successors of selected original projects on one private world.
//! These plans and action identities never enter a session or a durable owner.

use super::super::super::project_forecast::ProjectSource;
use super::*;
use crate::lab_world::ProductionGoalLedger;
use dfmcp_intent::PreparedPlan;

pub(crate) const MAX_FUTURE_STAGES: usize = 3;

#[derive(Clone)]
struct Stage {
    plan: PreparedPlan,
    actions: Vec<ActionId>,
}

#[derive(Clone)]
pub(crate) struct Project {
    pub(crate) original: Digest32,
    source: ProjectSource,
    ledger: ProductionGoalLedger,
    stages: Vec<Stage>,
}

impl Project {
    pub(crate) fn new(
        original: Digest32,
        source: ProjectSource,
        ledger: ProductionGoalLedger,
    ) -> Self {
        Self {
            original,
            source,
            ledger,
            stages: Vec::new(),
        }
    }

    pub(crate) fn goal_truth(&self, snapshot: &WorldSnapshot) -> Result<PredicateTruth> {
        self.source.goal_truth(snapshot, self.ledger)
    }

    /// Retain original action rows independently from all hypothetical stages.
    /// A prior stage's stock predicate may become false when a later stage
    /// consumes its output; the immutable verified receipt is not rewritten.
    pub(super) fn outcome(
        &self,
        fork: &MemoryAdapter,
        original: &Original,
        reserve: Option<PredicateTruth>,
        goal: PredicateTruth,
        original_stage: PredicateTruth,
    ) -> Result<Outcome> {
        let initial = observe(fork, original, reserve, goal, original_stage)?;
        let mut actions = original.actions.clone();
        for stage in &self.stages {
            actions.extend_from_slice(&stage.actions);
        }
        let combined = Original {
            requested: original.requested,
            actual: original.actual,
            actions,
            reserve_until: original.reserve_until,
            reserve_observation: original.reserve_observation,
            can_continue: true,
        };
        let mut outcome = observe(fork, &combined, reserve, goal, PredicateTruth::True)?;
        let needed = outcome.value["predicted_outcome"] == "needs_continuation"
            || (outcome.value["predicted_outcome"] == "reserve_pending"
                && self.source.remaining_reserve_required(fork.snapshot())?);
        outcome.continuation = needed;
        if needed {
            outcome.attention = false;
            outcome.complete = false;
            outcome.value["predicted_complete"] = json!(false);
            outcome.value["predicted_outcome"] = json!("needs_continuation");
        }
        outcome.value["predicted_project_actions_complete"] =
            outcome.value["predicted_actions_complete"].clone();
        outcome.value["predicted_original_physical_quiescent"] =
            initial.value["predicted_physical_quiescent"].clone();
        outcome.value["predicted_actions_complete"] =
            initial.value["predicted_actions_complete"].clone();
        outcome.value["predicted_stage_truth"] = json!(truth_name(original_stage));
        outcome.value["steps"] = initial.value["steps"].clone();
        outcome.value["project_source_digest"] = json!(self.source.source_digest().to_hex());
        outcome.value["future_stages"] = json!(self.stage_views(fork)?);
        outcome.value["future_stage_plans_are_executable"] = json!(false);
        outcome.value["requires_review_between_stages"] = json!(true);
        Ok(outcome)
    }

    fn stage_views(&self, fork: &MemoryAdapter) -> Result<Vec<Value>> {
        let mut rows = Vec::with_capacity(self.stages.len());
        for (index, stage) in self.stages.iter().enumerate() {
            let descriptor = Original {
                requested: self.original,
                actual: self.original,
                actions: stage.actions.clone(),
                reserve_until: None,
                reserve_observation: None,
                can_continue: false,
            };
            let status = observe(
                fork,
                &descriptor,
                None,
                PredicateTruth::True,
                PredicateTruth::True,
            )?;
            let mut steps = status.value["steps"].clone();
            if let Some(steps) = steps.as_array_mut() {
                for step in steps {
                    if let Some(fields) = step.as_object_mut()
                        && let Some(id) = fields.remove("action_id")
                    {
                        fields.insert("model_action_id".to_owned(), id);
                    }
                }
            }
            rows.push(json!({
                "index":index+2, "stage":self.source.stage_name(&stage.plan),
                "model_plan_digest":stage.plan.digest.to_hex(),
                "executable_review":false, "model_actions_are_live_handles":false,
                "from_tick":stage.plan.anchor.tick.0,
                "predicted_actions_complete":status.value["predicted_actions_complete"],
                "predicted_physical_quiescent":status.value["predicted_physical_quiescent"],
                "predicted_stage_complete":status.complete,
                "action_count":stage.plan.steps.len(),
                "max_risk":stage.plan.max_risk.as_str(),
                "requires_checkpoint":stage.plan.requires_checkpoint,
                "required_capabilities":stage.plan.required_capabilities.iter()
                    .map(|capability|capability.as_str()).collect::<Vec<_>>(),
                "obligation_deadlines":stage.plan.steps.iter().filter_map(|step|
                    step.obligation.as_ref().map(|obligation|json!({
                        "step":step.id.get(), "deadline_tick":obligation.deadline_tick.0,
                        "poll_interval_ticks":obligation.poll_interval_ticks,
                        "stable_observations":obligation.stable_for_observations,
                    }))).collect::<Vec<_>>(),
                "steps":steps,
            }));
        }
        Ok(rows)
    }

    pub(super) fn prepare_stage(
        &self,
        fork: &MemoryAdapter,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        // Check current modeled authority before even the stage-count refusal.
        for capability in [Capability::Observe, Capability::Query, Capability::Plan] {
            context.authorize(capability, RiskTier::ReadOnly, &[], None)?;
        }
        self.ledger.validate_current(fork.snapshot())?;
        if self.stages.len() >= MAX_FUTURE_STAGES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "joint complete project exceeds three hypothetical successor stages",
            ));
        }
        self.source
            .prepare_joint_stage(
                self.original,
                self.stages.len() + 2,
                fork.snapshot(),
                context,
            )
            .map_err(|error| error.with_detail("forecast_original", self.original.to_hex()))
    }

    pub(super) fn committed(&mut self, plan: PreparedPlan, actions: Vec<ActionId>) {
        self.stages.push(Stage { plan, actions });
    }
}
