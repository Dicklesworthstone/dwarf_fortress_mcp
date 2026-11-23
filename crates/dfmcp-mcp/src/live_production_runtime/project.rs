//! The production child of one immutable Workshop-to-stock project.
//!
//! This seam selects existing stock-target execution. It cannot create a new
//! native family, replace an original, renew its deadline, or promote an order
//! receipt into proof of the complete labor and inventory goal.
use super::{
    ProductionClock, ProductionInventoryClock, ProjectProductionIntent, Runtime,
    StockTargetRequest, View, request, stock_target_binding, workshop_binding,
};
use crate::semantic_workforce_runtime::{denied, invalid};
use dfmcp_adapter::live_production::ProductionLayout;
use dfmcp_core::{Digest32, OperationContext, Result, StateAnchor};
use dfmcp_intent::{Action, PreparedPlan, effects::MAX_DEFAULT_OBLIGATION_TICKS};
use serde::{Deserialize, Serialize};

/// The parent owns the durable ACK; this borrowed callback cannot outlive the
/// single planning call or become a preparation/dispatch permit.
pub(super) struct ProjectPublication<'a> {
    pub(super) goal: &'a ProjectStockGoal,
    pub(super) acknowledge:
        &'a mut dyn FnMut(&ProjectProductionIntent, &OperationContext) -> Result<()>,
}

/// Complete caller-selected goal retained by the parent before either child
/// opens. Worker identities are chosen from a fresh production observation;
/// their native family and all goal and temporal terms remain fixed here.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectStockGoal {
    pub recipe: String,
    pub minimum_stack_units: u64,
    pub max_order_amount: u32,
    pub deadline_tick: u64,
    pub poll_interval_ticks: u64,
    pub stable_samples: u32,
    pub assign_labor: bool,
}

impl ProjectStockGoal {
    pub(crate) fn validate(&self) -> Result<()> {
        // The existing registry contains precisely the four wooden furniture
        // recipes admitted by the Carpenters Workshop production prerequisite.
        request::recipe(&self.recipe)?;
        if !(1..=1_000_000).contains(&self.minimum_stack_units)
            || !(1..=100).contains(&self.max_order_amount)
            || self.deadline_tick == 0
            || self.deadline_tick > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || !(1..=MAX_DEFAULT_OBLIGATION_TICKS).contains(&self.poll_interval_ticks)
            || !(1..=64).contains(&self.stable_samples)
            || self
                .poll_interval_ticks
                .checked_mul(u64::from(self.stable_samples))
                .is_none_or(|ticks| ticks > self.deadline_tick)
        {
            return Err(invalid("invalid bounded original project stock goal"));
        }
        Ok(())
    }

    pub(crate) fn validate_request(&self, request: &StockTargetRequest) -> Result<()> {
        self.validate()?;
        request.validate()?;
        // Workshop/1.20 qualifies the actual CARPENTER labor bit. A different
        // enabled labor must not become the selected production prerequisite.
        if request.labor != "CARPENTER"
            || request.recipe != self.recipe
            || request.minimum_stack_units != self.minimum_stack_units
            || request.max_order_amount != self.max_order_amount
            || request.deadline_tick != self.deadline_tick
            || request.poll_interval_ticks != self.poll_interval_ticks
            || request.stable_samples != self.stable_samples
            || request.assign_labor != self.assign_labor
        {
            return Err(denied(
                "production request changed the original project goal, deadline or Carpenter labor",
            ));
        }
        Ok(())
    }

    /// Reconstruct the existing stock-target identity, including its original
    /// census, actual step IDs and keys, before comparing the parent contract.
    /// This is also required when an existing child is opened from recovery.
    pub(crate) fn validate_original(
        &self,
        plan: &PreparedPlan,
        workshop_origin: Digest32,
    ) -> Result<()> {
        self.validate()?;
        let binding = stock_target_binding::requirement(plan)?
            .ok_or_else(|| denied("project production requires its original stock target"))?;
        let layout = ProductionLayout::from_plan(plan)?;
        let order = layout
            .order_steps()
            .first()
            .and_then(|id| plan.steps.iter().find(|step| step.id == *id))
            .ok_or_else(|| denied("project original finite order is absent"))?;
        let obligation = order
            .obligation
            .as_ref()
            .ok_or_else(|| denied("project original order obligation is absent"))?;
        let carpenter = if let Some(id) = layout.assignment_step() {
            plan.steps
                .iter()
                .find(|step| step.id == id)
                .is_some_and(|step| {
                    matches!(&step.action, Action::SetLabor { labor, enabled: true, .. }
                        if labor == "CARPENTER")
                })
        } else {
            layout
                .existing_labor()
                .is_some_and(|workers| workers.labor() == "CARPENTER")
        };
        if !carpenter
            || binding.recipe().as_str() != self.recipe
            || binding.objective().target_stack_units() != self.minimum_stack_units
            || binding.max_order_amount() != self.max_order_amount
            || binding.assign_labor() != self.assign_labor
            || obligation.deadline_tick.get() != self.deadline_tick
            || obligation.poll_interval_ticks != self.poll_interval_ticks
            || obligation.stable_for_observations != self.stable_samples
            || workshop_origin == Digest32::ZERO
            || workshop_binding::requirement(plan)? != Some(workshop_origin)
        {
            return Err(denied(
                "recovered production original differs from the immutable project goal or Workshop origin",
            ));
        }
        Ok(())
    }
}

/// A qualified complete stock census can close a no-work target without an
/// original order. It proves the count at this anchor only; there is no worker,
/// order-completion, causal-output, or stability proof to recover from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProjectTargetAlreadySatisfied {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    pub source_generation: u64,
    pub source_counters: [u64; 3],
    pub observed_stack_units: u64,
    pub target_stack_units: u64,
    pub witness: Digest32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProjectProductionStatus {
    pub original_plan_digest: Option<Digest32>,
    pub original_goal_proven: bool,
    pub current_goal_anchor: Option<StateAnchor>,
    pub clocks_settled: bool,
    pub already_satisfied: Option<ProjectTargetAlreadySatisfied>,
    pub no_work_stock_goal: serde_json::Value,
}

impl ProjectProductionStatus {
    pub(crate) fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "original_plan_digest": self.original_plan_digest.map(|digest| digest.to_hex()),
            "original_goal_proven": self.original_goal_proven,
            "current_goal_anchor": self.current_goal_anchor.map(super::anchor_json),
            "clocks_settled": self.clocks_settled,
            "no_work_stock_goal": self.no_work_stock_goal,
            "already_satisfied": self.already_satisfied.map(|census| serde_json::json!({
                "anchor": super::anchor_json(census.anchor),
                "source_digest": census.source_digest.to_hex(),
                "source_generation": census.source_generation,
                "source_counters": census.source_counters,
                "observed_stack_units": census.observed_stack_units,
                "target_stack_units": census.target_stack_units,
                "assessment_witness": census.witness.to_hex(),
                "no_order_required": true,
                "satisfied_at_anchor": true,
                "fresh_for_current_call": false,
                "current_inventory_proven": false,
                "worker_prerequisite_checked": false,
                "original_goal_proven": false,
                "stability_proven": false,
                "production_attribution": "unknown",
                "inventory_semantics": "all_observed_type_stack_units"
            })),
            "runtime_admitted": false
        })
    }
}

fn satisfied_census(
    goal: &ProjectStockGoal,
    no_work: &super::stock_target::StockTargetNoWork,
    anchor: StateAnchor,
) -> Result<ProjectTargetAlreadySatisfied> {
    goal.validate_request(&no_work.request)?;
    let assessment = &no_work.assessment;
    if assessment.amount().is_some()
        || assessment.original_anchor() != anchor
        || assessment.target_stack_units() != goal.minimum_stack_units
        || assessment.baseline_stack_units() < goal.minimum_stack_units
        || assessment.recipe().as_str() != goal.recipe
        || assessment.max_order_amount() != goal.max_order_amount
    {
        return Err(denied(
            "project no-work census differs from its original goal",
        ));
    }
    Ok(ProjectTargetAlreadySatisfied {
        anchor: assessment.original_anchor(),
        source_digest: assessment.source_digest(),
        source_generation: assessment.source_generation(),
        source_counters: assessment.source_counters(),
        observed_stack_units: assessment.baseline_stack_units(),
        target_stack_units: assessment.target_stack_units(),
        witness: assessment.witness(),
    })
}

impl Runtime {
    /// The resource-project parent may have consumed newer game time than its
    /// retained child checkpoint. Carry that rejection floor into opening,
    /// before any original or native custody is inspected or initialized.
    pub(crate) fn open_for_resource_project(
        config: super::Config,
        mode: super::Mode,
        session: dfmcp_core::SessionId,
        budget: dfmcp_core::WorkBudget,
        parent_tick_floor: u64,
    ) -> Result<Self> {
        if config.project_binding.is_none()
            || parent_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
        {
            return Err(denied(
                "invalid resource project production owner or authority floor",
            ));
        }
        Self::open_reserved(config, mode, session, budget, budget, parent_tick_floor)
    }

    /// A parent's newer rejection floor cannot become a canonical observation
    /// or preserve a stale approval. Retire process-local offers and proof but
    /// keep every original plan, receipt, deadline and journal byte unchanged.
    pub(crate) fn retain_project_authority_tick(&mut self, parent_tick_floor: u64) -> Result<()> {
        if self.config.project_binding.is_none() {
            return Err(denied(
                "production authority floor requires its original resource project",
            ));
        }
        if parent_tick_floor > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK {
            return Err(invalid(
                "invalid resource project production authority floor",
            ));
        }
        let current = self.parent_authority_tick_floor();
        self.retain_parent_authority_floor(parent_tick_floor);
        if parent_tick_floor > current {
            self.abandon_preparation();
            if self.original.is_none() {
                self.basis = None;
            }
        }
        Ok(())
    }

    fn project_workshop_origin(&self, goal: &ProjectStockGoal) -> Result<Digest32> {
        goal.validate()?;
        self.config.current()?;
        let binding = self.config.project_binding.ok_or_else(|| {
            denied("production child is not bound to its original native project")
        })?;
        let workshop = self
            .workshop
            .as_ref()
            .ok_or_else(|| denied("project production lost its completed Workshop donor"))?;
        let actual = workshop.witness()?;
        if actual != binding.workshop_origin
            || goal.deadline_tick != binding.deadline_tick
            || goal.deadline_tick > binding.authority_expires_tick
        {
            return Err(denied(
                "production child changed its project deadline or Workshop origin",
            ));
        }
        Ok(actual)
    }

    /// Retained restart custody, including the workforce identity that changes
    /// from initialize to reopen. This deliberately carries no current goal
    /// proof, review, or permit and is available after a failed child call.
    pub(crate) fn project_recovery_reference(&self) -> serde_json::Value {
        let clock = self
            .clock
            .as_ref()
            .map(ProductionClock::project_recovery_checkpoint);
        let inventory_clock = self
            .inventory_clock
            .as_ref()
            .map(ProductionInventoryClock::project_recovery_checkpoint);
        let floor = self
            .authority_tick_floor()
            .get()
            .max(clock.map_or(0, |pin| pin.authority_tick_floor))
            .max(inventory_clock.map_or(0, |pin| pin.authority_tick_floor));
        let mut recovery = self.recovery_json();
        if recovery.is_object() {
            // The compact parent reference must survive a failed operation
            // whose last published clock view predates its durable marker.
            recovery["clock_journal_id"] =
                serde_json::json!(clock.map(|pin| pin.journal_id.to_hex()));
            recovery["inventory_clock_journal_id"] =
                serde_json::json!(inventory_clock.map(|pin| pin.journal_id.to_hex()));
            let frontier_floor = recovery["frontier"]["authority_tick_floor"]
                .as_u64()
                .unwrap_or(0)
                .max(floor);
            recovery["frontier"]["authority_tick_floor"] = serde_json::json!(frontier_floor);
        }
        // Keep identities and monotone rejection floors only. Whole original
        // worker predicates, proof diagnostics and source censuses belong to
        // their existing child stores, not the parent's compact checkpoint.
        let checkpoint =
            |status: serde_json::Value| serde_json::json!({"checkpoint": status["checkpoint"]});
        serde_json::json!({
            "schema": "dfmcp.project-production-recovery/1",
            "recovery": recovery,
            "native_custody": {
                "workforce_journal_id": self.workforce_journal.to_hex(),
                "order_journal_id": self.order_journal.to_hex(),
            },
            "observation_tick_floor": floor,
            "clock": clock.map(|pin| pin.to_json()),
            "inventory_clock": inventory_clock.map(|pin| pin.to_json()),
            "workshop_prerequisite": self.workshop.as_ref()
                .map(|owner| checkpoint(owner.status())),
            "manager_prerequisite": self.manager.as_ref()
                .map(|owner| checkpoint(owner.status())),
            "input_supply": self.input_supply.as_ref()
                .map(|owner| checkpoint(owner.status())),
            "original_goal_proven": false,
            "preparation_restored": false,
        })
    }

    pub(crate) fn project_plan_stock_target(
        &mut self,
        goal: &ProjectStockGoal,
        request: StockTargetRequest,
        c: &OperationContext,
    ) -> Result<View> {
        goal.validate_request(&request)?;
        self.project_workshop_origin(goal)?;
        self.plan_stock_target(request, c)
    }

    pub(crate) fn project_plan_stock_target_recorded(
        &mut self,
        goal: &ProjectStockGoal,
        request: StockTargetRequest,
        c: &OperationContext,
        acknowledge: &mut dyn FnMut(&ProjectProductionIntent, &OperationContext) -> Result<()>,
    ) -> Result<View> {
        goal.validate_request(&request)?;
        self.project_workshop_origin(goal)?;
        self.plan_stock_target_recorded(request, c, Some(ProjectPublication { goal, acknowledge }))
    }

    /// All values below come from the completed compiler and retained source
    /// owners. The future archive, capsule and associations have not been
    /// created. The exact header identity is reused by their later publication.
    pub(super) fn project_production_intent(
        &self,
        goal: &ProjectStockGoal,
        plan: &PreparedPlan,
        seal: dfmcp_intent::plan_archive::PlanArchiveSeal,
        identity: Digest32,
        frontier: dfmcp_adapter::live_dispatch::durable::Recovery,
        origins: super::Origins,
        c: &OperationContext,
    ) -> Result<ProjectProductionIntent> {
        goal.validate_original(plan, self.project_workshop_origin(goal)?)?;
        if self.original.is_some() || self.frontier.is_some() || self.initialization_failed {
            return Err(denied(
                "project publication cannot replace an existing original",
            ));
        }
        let mut work = super::Work::new(c)?;
        let mut reference = self.project_recovery_reference();
        let floor = reference["observation_tick_floor"]
            .as_u64()
            .ok_or_else(|| invalid("project publication lost its authority floor"))?
            .max(frontier.authority_tick_floor);
        let mut recovery = serde_json::json!({
            "plan_digest": seal.plan_digest.to_hex(),
            "object_digest": seal.object_digest.to_hex(),
            "identity_digest": identity.to_hex(),
            "anchor": super::anchor_json(plan.anchor),
            "frontier": {
                "journal_id": frontier.journal_id.to_hex(),
                "head": frontier.head.to_hex(),
                "authority_tick_floor": floor,
            },
            "goal_history_present": false,
            "clock_journal_id": reference["clock"]["journal_id"],
            "inventory_clock_journal_id": reference["inventory_clock"]["journal_id"],
        });
        if let Some(owner) = &self.workshop {
            recovery["workshop_prerequisite_digest"] = serde_json::json!(
                owner
                    .publication_seal(plan, &work.take(super::MIB)?)?
                    .to_hex()
            );
            recovery["workshop_prerequisite_floor"] = owner.recovery_floor();
        }
        if let Some(owner) = &self.manager {
            recovery["manager_prerequisite_digest"] = serde_json::json!(
                owner
                    .publication_seal(plan, &work.take(super::MIB)?)?
                    .to_hex()
            );
            recovery["manager_prerequisite_floor"] = owner.recovery_floor();
        }
        let input_policy_witness = match (&self.input_supply, origins.input_supply) {
            (Some(owner), Some(binding)) => {
                let policy = reference["input_supply"]["checkpoint"]["origin_witness"]
                    .as_str()
                    .ok_or_else(|| denied("project input policy checkpoint is absent"))?
                    .to_owned();
                recovery["input_supply_digest"] = serde_json::json!(binding.witness().to_hex());
                recovery["input_supply_floor"] = owner.recovery_floor();
                reference["input_supply"]["checkpoint"]["origin_witness"] =
                    serde_json::json!(binding.witness().to_hex());
                Some(policy)
            }
            (None, None) => None,
            _ => return Err(denied("project publication changed its input source owner")),
        };
        reference["recovery"] = recovery;
        reference["observation_tick_floor"] = serde_json::json!(floor);
        self.config.current()?;
        work.current()?;
        ProjectProductionIntent::new(goal.clone(), reference, input_policy_witness)
    }

    /// Actual child opening has already authenticated every original byte and
    /// donor association and verified coordinator ancestry. A pending parent
    /// publication additionally requires the untouched initial frontier.
    pub(crate) fn validate_project_publication(
        &self,
        goal: &ProjectStockGoal,
        intent: &ProjectProductionIntent,
    ) -> Result<()> {
        intent.validate_goal(goal)?;
        let plan = self
            .original
            .as_ref()
            .ok_or_else(|| denied("project publication has no complete durable original"))?;
        goal.validate_original(plan, self.project_workshop_origin(goal)?)?;
        intent.validate_checkpoint(&self.project_recovery_reference())?;
        if self
            .config
            .project_recovery
            .is_some_and(|pin| pin.publication_pending)
        {
            let frontier = self
                .frontier
                .as_ref()
                .ok_or_else(|| denied("project publication lost its exact initial frontier"))?;
            intent.validate_initial_frontier(frontier.recovery())?;
            if self.goal_history_present
                || self.portfolio.is_some()
                || self.stock_targets.is_some()
                || self.clock.as_ref().is_some_and(ProductionClock::has_runs)
                || self
                    .inventory_clock
                    .as_ref()
                    .is_some_and(ProductionInventoryClock::has_runs)
            {
                return Err(denied(
                    "unacknowledged project publication cannot contain execution history",
                ));
            }
        }
        self.config.current()
    }

    /// Read typed retained proof after a successful ordinary child operation.
    /// All native reads and private custody checks remain in that operation;
    /// this accessor does not renew a sample or perform a second budgeted read.
    /// Failed publication and recovery already withdraw these current claims.
    pub(crate) fn project_status(
        &self,
        goal: &ProjectStockGoal,
    ) -> Result<ProjectProductionStatus> {
        let origin = self.project_workshop_origin(goal)?;
        if let Some(original) = &self.original {
            goal.validate_original(original, origin)?;
        }
        if self.portfolio.is_some() || self.stock_targets.is_some() {
            return Err(denied(
                "project production cannot adopt a portfolio original",
            ));
        }
        let clocks_settled = self.clock_review.is_none()
            && self.inventory_clock_review.is_none()
            && !self.clock.as_ref().is_some_and(|clock| {
                ProductionClock::pending(clock) || ProductionClock::may_be_running(clock)
            })
            && !self.inventory_clock.as_ref().is_some_and(|clock| {
                ProductionInventoryClock::pending(clock)
                    || ProductionInventoryClock::may_be_running(clock)
            })
            && !self.inactive_clock_may_be_running();
        let original_goal_proven = clocks_settled
            && !self.fenced
            && !self.initialization_failed
            && ((self.original.is_some() && self.parent_goal_proven())
                || self.no_work_goal_proven());
        let already_satisfied = if self.original.is_none()
            && clocks_settled
            && !self.clock_withholds_goal()
            && !self.fenced
            && !self.initialization_failed
        {
            self.stock_target_no_work
                .as_ref()
                .map(|no_work| {
                    satisfied_census(goal, no_work, no_work.assessment.original_anchor())
                })
                .transpose()?
        } else {
            None
        };
        self.config.current()?;
        Ok(ProjectProductionStatus {
            original_plan_digest: self.original.as_ref().map(|plan| plan.digest),
            original_goal_proven,
            current_goal_anchor: original_goal_proven.then_some(self.anchor),
            clocks_settled,
            already_satisfied,
            no_work_stock_goal: self.no_work_goal_json(),
        })
    }
}

#[cfg(test)]
#[path = "project_tests.rs"]
mod tests;
