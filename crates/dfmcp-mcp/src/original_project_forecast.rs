//! Hypothetical successors of selected original projects on one private world.
//! These plans and action identities never enter a session or a durable owner.

use super::super::super::project_forecast::{MAX_COMMITTED_PROJECT_STAGES, ProjectSource};
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
struct RetainedStage {
    plan: PreparedPlan,
    parent: Digest32,
    original: Original,
    reserve_sample: Option<PredicateTruth>,
    recovered: Option<RecoveredOriginal>,
}

#[derive(Clone)]
pub(crate) struct Project {
    pub(crate) original: Digest32,
    source: ProjectSource,
    ledger: ProductionGoalLedger,
    recovered: Option<RecoveredOriginal>,
    retained: Vec<RetainedStage>,
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
            recovered: None,
            retained: Vec::new(),
            stages: Vec::new(),
        }
    }

    pub(crate) fn goal_truth(&self, snapshot: &WorldSnapshot) -> Result<PredicateTruth> {
        self.source.goal_truth(snapshot, self.ledger)
    }

    pub(in super::super) fn with_recovery(mut self, recovered: RecoveredOriginal) -> Result<Self> {
        if recovered.original() != self.original || !self.retained.is_empty() {
            return Err(error(
                ErrorCode::InvalidPlan,
                "recovered project identity changed",
            ));
        }
        self.recovered = Some(recovered);
        Ok(self)
    }

    pub(super) fn recovered_steps(&self) -> usize {
        self.recovered.as_ref().map_or(0, RecoveredOriginal::len)
            + self.recovered_retained_steps()
    }

    pub(super) fn recovered_retained_steps(&self) -> usize {
        self.retained
            .iter()
            .filter_map(|stage| stage.recovered.as_ref())
            .map(RecoveredOriginal::len)
            .sum()
    }

    /// The caller has admitted the exact live source, lineage and complete
    /// action census. Keep its receipts and runtime; never recommit this plan.
    pub(in super::super) fn retain_stage(
        &mut self,
        plan: PreparedPlan,
        parent: Digest32,
        original: Original,
        sealed: &WorldSnapshot,
    ) -> Result<()> {
        for recovered in self.recovered.iter().chain(
            self.retained
                .iter()
                .filter_map(|stage| stage.recovered.as_ref()),
        ) {
            recovered.validate_live_source(plan.anchor)?;
        }
        self.validate_stage(&plan, parent, &original)?;
        if original.actions.is_empty() || original.actions.len() != plan.steps.len() {
            return Err(error(
                ErrorCode::InvalidPlan,
                "retained live stage requires its complete original action handles",
            ));
        }
        self.ledger.validate_current(sealed)?;
        ProductionGoalLedger::capture(sealed)?.validate_plan(&plan)?;
        let reserve_sample = original.reserve_observation;
        self.retained.push(RetainedStage {
            plan,
            parent,
            original,
            reserve_sample,
            recovered: None,
        });
        Ok(())
    }

    /// The enclosing owner has verified the exact archived source and receipt
    /// census. Keep historical proof separate from every live/model ActionId.
    pub(in super::super) fn retain_recovered_stage(
        &mut self,
        plan: PreparedPlan,
        parent: Digest32,
        original: Original,
        recovered: RecoveredOriginal,
        current: &WorldSnapshot,
    ) -> Result<()> {
        self.validate_stage(&plan, parent, &original)?;
        if !original.actions.is_empty()
            || recovered.original() != plan.digest
            || recovered.len() != plan.steps.len()
            || self
                .recovered
                .as_ref()
                .is_none_or(|root| !root.shares_recovery(&recovered))
            || self.retained.iter().any(|stage| {
                stage
                    .recovered
                    .as_ref()
                    .is_none_or(|prior| !prior.shares_recovery(&recovered))
            })
        {
            return Err(error(
                ErrorCode::Conflict,
                "recovered continuation requires complete historical custody in the same recovered prefix",
            ));
        }
        self.ledger.validate_current(current)?;
        recovered.rows(current)?;
        let reserve_sample = original.reserve_observation;
        self.retained.push(RetainedStage {
            plan,
            parent,
            original,
            reserve_sample,
            recovered: Some(recovered),
        });
        Ok(())
    }

    fn validate_stage(
        &self,
        plan: &PreparedPlan,
        parent: Digest32,
        original: &Original,
    ) -> Result<()> {
        if self.retained.len() + 1 >= MAX_COMMITTED_PROJECT_STAGES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete project exceeds eight already committed stages",
            ));
        }
        let preceding = self
            .retained
            .last()
            .map_or(self.original, |stage| stage.original.actual);
        if parent != preceding
            || plan.digest != original.actual
            || original.actual == self.original
            || !plan.digest_is_valid()
            || !plan.id_is_valid()
            || plan.steps.is_empty()
            || original.reserve_until != self.source.reserve_until_tick()
            || self
                .retained
                .iter()
                .any(|stage| stage.original.actual == original.actual)
        {
            return Err(error(
                ErrorCode::InvalidPlan,
                "retained model stage differs from the exact original project chain",
            ));
        }
        Ok(())
    }

    pub(super) fn retained_actions(&self) -> impl Iterator<Item = ActionId> + '_ {
        self.retained
            .iter()
            .flat_map(|stage| stage.original.actions.iter().copied())
    }

    pub(super) fn retained_stage_count(&self) -> usize {
        self.retained.len()
    }

    /// Each already committed pursuit keeps its own first exact endpoint.
    /// A descendant cannot lend historical evidence to its root or siblings.
    pub(super) fn sample_retained_reserves(&mut self, tick: u64, goal: PredicateTruth) {
        for stage in &mut self.retained {
            if stage.reserve_sample.is_none() && stage.original.reserve_until == Some(tick) {
                stage.reserve_sample = Some(goal);
            }
        }
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
        if self.recovered.is_some() && !original.actions.is_empty() {
            return Err(error(
                ErrorCode::InvalidPlan,
                "recovered project cannot contain live original handles",
            ));
        }
        let initial = observe_with_recovery(
            fork,
            original,
            reserve,
            goal,
            original_stage,
            self.recovered.as_ref(),
        )?;
        let mut actions = original.actions.clone();
        actions.extend(self.retained_actions());
        let committed_verified = (self.recovered_steps() > 0 || !actions.is_empty())
            && actions.iter().all(|id| {
                fork.action_receipt(*id)
                    .is_some_and(|receipt| receipt.state == CommitState::Verified)
            });
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
        let mut outcome = observe_with_recovery(
            fork,
            &combined,
            reserve,
            goal,
            PredicateTruth::True,
            self.recovered.iter().chain(
                self.retained
                    .iter()
                    .filter_map(|stage| stage.recovered.as_ref()),
            ),
        )?;
        let retained = self.retained_stage_views(fork, goal)?;
        // Action failures take precedence as in ordinary original inspection.
        // Otherwise every admitted pursuit's endpoint remains independently
        // required, including a missing sample at an already-passed horizon.
        if matches!(
            outcome.value["predicted_outcome"].as_str(),
            Some(
                "complete"
                    | "pending"
                    | "reserve_pending"
                    | "needs_continuation"
                    | "goal_unknown"
                    | "goal_unmet"
            )
        ) && let Some(problem) = retained.iter().find_map(|stage| {
            stage["predicted_outcome"].as_str().filter(|outcome| {
                matches!(
                    *outcome,
                    "reserve_unobserved" | "reserve_unmet" | "reserve_unknown"
                )
            })
        }) {
            outcome.complete = false;
            outcome.attention = true;
            outcome.value["predicted_complete"] = json!(false);
            outcome.value["predicted_outcome"] = json!(problem);
        }
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
        outcome.value["predicted_committed_actions_complete"] = json!(committed_verified);
        outcome.value["predicted_original_physical_quiescent"] =
            initial.value["predicted_physical_quiescent"].clone();
        outcome.value["predicted_actions_complete"] =
            initial.value["predicted_actions_complete"].clone();
        outcome.value["predicted_stage_truth"] = json!(truth_name(original_stage));
        outcome.value["steps"] = initial.value["steps"].clone();
        outcome.value["project_source_digest"] = json!(self.source.source_digest().to_hex());
        outcome.value["retained_stages"] = json!(retained);
        outcome.value["future_stages"] = json!(self.stage_views(fork)?);
        outcome.value["future_stage_plans_are_executable"] = json!(false);
        outcome.value["requires_review_between_stages"] = json!(true);
        Ok(outcome)
    }

    fn retained_stage_views(
        &self,
        fork: &MemoryAdapter,
        goal: PredicateTruth,
    ) -> Result<Vec<Value>> {
        let mut rows = Vec::with_capacity(self.retained.len());
        for (index, stage) in self.retained.iter().enumerate() {
            let truth = crate::lab_world::plan_goal_truth(fork.snapshot(), &stage.plan)?;
            let mut status = observe_with_recovery(
                fork,
                &stage.original,
                stage.reserve_sample,
                goal,
                truth,
                stage.recovered.as_ref(),
            )?;
            status.value["index"] = json!(index + 2);
            status.value["stage"] = json!(self.source.stage_name(&stage.plan));
            status.value["root_plan_digest"] = json!(self.original.to_hex());
            status.value["parent_plan_digest"] = json!(stage.parent.to_hex());
            status.value["source_anchor"] = super::super::super::anchor_json(&stage.plan.anchor);
            status.value["custody"] = json!(if stage.recovered.is_some() {
                "recovered_verified_original"
            } else {
                "already_committed_original"
            });
            status.value["recommitted_by_forecast"] = json!(false);
            status.value["obligation_deadlines"] = json!(
                stage
                    .plan
                    .steps
                    .iter()
                    .filter_map(|step| step.obligation.as_ref().map(|obligation| json!({
                        "step":step.id.get(), "deadline_tick":obligation.deadline_tick.0,
                        "poll_interval_ticks":obligation.poll_interval_ticks,
                        "stable_observations":obligation.stable_for_observations,
                    })))
                    .collect::<Vec<_>>()
            );
            rows.push(status.value);
        }
        Ok(rows)
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
                "index":self.retained.len()+index+2, "stage":self.source.stage_name(&stage.plan),
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
                self.retained.len() + self.stages.len() + 2,
                fork.snapshot(),
                context,
            )
            .map_err(|error| error.with_detail("forecast_original", self.original.to_hex()))
    }

    pub(super) fn committed(&mut self, plan: PreparedPlan, actions: Vec<ActionId>) {
        self.stages.push(Stage { plan, actions });
    }
}
