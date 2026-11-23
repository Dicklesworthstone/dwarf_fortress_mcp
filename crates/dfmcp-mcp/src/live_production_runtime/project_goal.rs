//! Immutable scalar or portfolio stock goals for the native resource project.
//! This pure boundary preserves legacy scalar JSON and validates every target,
//! temporal term and reconstructed original before any child effect is possible.
use super::{StockTargetRequest, request, stock_target_binding, workshop_binding};
use crate::semantic_workforce_runtime::{denied, invalid};
use dfmcp_adapter::live_production::ProductionLayout;
use dfmcp_core::{Digest32, Result};
use dfmcp_intent::{Action, PreparedPlan, effects::MAX_DEFAULT_OBLIGATION_TICKS};
use serde::{Deserialize, Serialize};

/// Complete caller-selected goal retained before any child opens. The scalar
/// variant preserves the exact JSON shape of existing project journals. A
/// portfolio owns its complete canonical target vector, including met minima.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub(crate) enum ProjectStockGoal {
    Single(ProjectSingleStockGoal),
    Portfolio(ProjectStockTargetsGoal),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectSingleStockGoal {
    pub recipe: String,
    pub minimum_stack_units: u64,
    pub max_order_amount: u32,
    pub deadline_tick: u64,
    pub poll_interval_ticks: u64,
    pub stable_samples: u32,
    pub assign_labor: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectStockTargetsGoal {
    pub targets: Vec<super::stock_targets_request::StockTarget>,
    pub deadline_tick: u64,
    pub poll_interval_ticks: u64,
    pub stable_samples: u32,
    pub assign_labor: bool,
}

impl ProjectStockGoal {
    pub(crate) fn is_portfolio(&self) -> bool {
        matches!(self, Self::Portfolio(_))
    }

    pub(crate) fn deadline_tick(&self) -> u64 {
        match self {
            Self::Single(goal) => goal.deadline_tick,
            Self::Portfolio(goal) => goal.deadline_tick,
        }
    }

    pub(crate) fn poll_interval_ticks(&self) -> u64 {
        match self {
            Self::Single(goal) => goal.poll_interval_ticks,
            Self::Portfolio(goal) => goal.poll_interval_ticks,
        }
    }

    pub(crate) fn stable_samples(&self) -> u32 {
        match self {
            Self::Single(goal) => goal.stable_samples,
            Self::Portfolio(goal) => goal.stable_samples,
        }
    }

    pub(crate) fn assign_labor(&self) -> bool {
        match self {
            Self::Single(goal) => goal.assign_labor,
            Self::Portfolio(goal) => goal.assign_labor,
        }
    }

    /// Normalize only fresh operator input, before fixing the parent original.
    /// Retained journal and intent bytes must instead validate unchanged.
    pub(crate) fn canonicalize(&mut self) -> Result<()> {
        if let Self::Portfolio(goal) = self {
            goal.targets.sort_by(|a, b| a.recipe.cmp(&b.recipe));
        }
        self.validate()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        let target = |recipe: &str, minimum: u64, maximum: u32| -> Result<()> {
            request::recipe(recipe)?;
            if !(1..=1_000_000).contains(&minimum) || !(1..=100).contains(&maximum) {
                return Err(invalid("invalid bounded original project stock target"));
            }
            Ok(())
        };
        match self {
            Self::Single(goal) => {
                target(
                    &goal.recipe,
                    goal.minimum_stack_units,
                    goal.max_order_amount,
                )?;
            }
            Self::Portfolio(goal) => {
                if goal.targets.is_empty()
                    || goal.targets.len() > 4
                    || goal
                        .targets
                        .windows(2)
                        .any(|pair| pair[0].recipe >= pair[1].recipe)
                {
                    return Err(invalid(
                        "project portfolio requires one to four distinct canonical recipes",
                    ));
                }
                for row in &goal.targets {
                    target(&row.recipe, row.minimum_stack_units, row.max_order_amount)?;
                }
            }
        }
        if self.deadline_tick() == 0
            || self.deadline_tick() > dfmcp_adapter::bounded_run::MAX_NATIVE_TICK
            || !(1..=MAX_DEFAULT_OBLIGATION_TICKS).contains(&self.poll_interval_ticks())
            || !(1..=64).contains(&self.stable_samples())
            || self
                .poll_interval_ticks()
                .checked_mul(u64::from(self.stable_samples()))
                .is_none_or(|ticks| ticks > self.deadline_tick())
        {
            return Err(invalid("invalid bounded original project stock goal"));
        }
        Ok(())
    }

    fn validate_terms(
        &self,
        labor: &str,
        deadline: u64,
        cadence: u64,
        stability: u32,
        assignment: bool,
    ) -> Result<()> {
        if labor != "CARPENTER"
            || deadline != self.deadline_tick()
            || cadence != self.poll_interval_ticks()
            || stability != self.stable_samples()
            || assignment != self.assign_labor()
        {
            return Err(denied(
                "production request changed the original project timing, assignment or Carpenter labor",
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_request(&self, request: &StockTargetRequest) -> Result<()> {
        self.validate()?;
        request.validate()?;
        let Self::Single(goal) = self else {
            return Err(denied(
                "a project portfolio requires its complete stock_targets request",
            ));
        };
        if request.recipe != goal.recipe
            || request.minimum_stack_units != goal.minimum_stack_units
            || request.max_order_amount != goal.max_order_amount
        {
            return Err(denied(
                "production request changed the original project stock target",
            ));
        }
        self.validate_terms(
            &request.labor,
            request.deadline_tick,
            request.poll_interval_ticks,
            request.stable_samples,
            request.assign_labor,
        )
    }

    pub(crate) fn validate_targets_request(
        &self,
        request: &super::StockTargetsRequest,
    ) -> Result<()> {
        self.validate()?;
        request.validate()?;
        let Self::Portfolio(goal) = self else {
            return Err(denied(
                "a single-target project cannot become a stock portfolio",
            ));
        };
        if request.targets != goal.targets {
            return Err(denied(
                "production request changed the complete original project portfolio",
            ));
        }
        self.validate_terms(
            &request.labor,
            request.deadline_tick,
            request.poll_interval_ticks,
            request.stable_samples,
            request.assign_labor,
        )
    }

    /// Reconstruct every original target and native key before comparing the
    /// root contract. Met portfolio minima remain required even without an order.
    pub(crate) fn validate_original(
        &self,
        plan: &PreparedPlan,
        workshop_origin: Digest32,
    ) -> Result<()> {
        self.validate()?;
        let layout = ProductionLayout::from_plan(plan)?;
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
            || workshop_origin == Digest32::ZERO
            || workshop_binding::requirement(plan)? != Some(workshop_origin)
        {
            return Err(denied(
                "recovered production original changed its Carpenter labor or Workshop origin",
            ));
        }
        for id in layout.order_steps() {
            let obligation = plan
                .steps
                .iter()
                .find(|step| step.id == *id)
                .and_then(|step| step.obligation.as_ref())
                .ok_or_else(|| denied("project original order obligation is absent"))?;
            if obligation.deadline_tick.get() != self.deadline_tick()
                || obligation.poll_interval_ticks != self.poll_interval_ticks()
                || obligation.stable_for_observations != self.stable_samples()
            {
                return Err(denied(
                    "recovered production original changed its immutable temporal goal",
                ));
            }
        }
        match self {
            Self::Single(goal) => {
                let binding = stock_target_binding::requirement(plan)?.ok_or_else(|| {
                    denied("project production requires its original stock target")
                })?;
                if layout.order_steps().len() != 1
                    || super::stock_targets_binding::requirement(plan)?.is_some()
                    || binding.recipe().as_str() != goal.recipe
                    || binding.objective().target_stack_units() != goal.minimum_stack_units
                    || binding.max_order_amount() != goal.max_order_amount
                    || binding.assign_labor() != goal.assign_labor
                {
                    return Err(denied(
                        "recovered production original differs from the immutable project target",
                    ));
                }
            }
            Self::Portfolio(goal) => {
                let binding =
                    super::stock_targets_binding::requirement(plan)?.ok_or_else(|| {
                        denied("project production requires its complete original portfolio")
                    })?;
                if stock_target_binding::requirement(plan)?.is_some()
                    || binding.assign_labor() != goal.assign_labor
                    || binding.rows.len() != goal.targets.len()
                    || binding.rows.iter().zip(&goal.targets).any(|(row, target)| {
                        row.recipe.as_str() != target.recipe
                            || row.target != target.minimum_stack_units
                            || row.maximum != target.max_order_amount
                    })
                {
                    return Err(denied(
                        "recovered production original changed an original portfolio minimum",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "project_goal_tests.rs"]
mod tests;
