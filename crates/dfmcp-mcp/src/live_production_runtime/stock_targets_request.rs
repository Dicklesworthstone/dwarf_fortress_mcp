//! A closed portfolio of finite inventory minima, sized from one qualified census.
use super::existing_labor_portfolio_request::ExistingLaborPortfolioRequest;
use super::existing_labor_request::ExistingLaborRequest;
use super::portfolio_request::PortfolioOrder;
use super::request::recipe;
use super::stock_targets_binding::StockTargetsBinding;
use crate::semantic_work_order_runtime::inventory_baseline::InventoryBaselines;
use crate::semantic_work_order_runtime::request::Objective;
use crate::semantic_workforce_runtime::{digest, exhausted, invalid};
use dfmcp_adapter::live_production::ExistingLaborRequirement;
use dfmcp_adapter::semantic_work_orders::StockTargetAssessment;
use dfmcp_core::{Digest32, EntityId, OperationContext, Result};
use dfmcp_intent::PreparedPlan;
use dfmcp_world::PredicateEvidence;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StockTarget {
    pub recipe: String,
    pub minimum_stack_units: u64,
    #[serde(default = "maximum")]
    pub max_order_amount: u32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct StockTargetsRequest {
    pub kind: String,
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
    pub targets: Vec<StockTarget>,
    pub deadline_tick: u64,
    #[serde(default = "interval")]
    pub poll_interval_ticks: u64,
    #[serde(default = "samples")]
    pub stable_samples: u32,
}

fn maximum() -> u32 { 100 }
fn interval() -> u64 { 10 }
fn samples() -> u32 { 1 }

impl StockTargetsRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 8192 {
            return Err(exhausted("stock-target portfolio request exceeds 8192 bytes"));
        }
        let mut value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed stock-target portfolio request"))?;
        value.targets.sort_by(|a, b| a.recipe.cmp(&b.recipe));
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn observation(&self) -> Result<Digest32> {
        digest(&self.expected_observation)
    }

    fn worker_request(&self, target: &StockTarget) -> ExistingLaborRequest {
        ExistingLaborRequest {
            kind: "existing_labor".into(),
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            recipe: target.recipe.clone(),
            amount: 1,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
            objective: None,
        }
    }

    pub(crate) fn unit_ids(&self) -> Result<Vec<EntityId>> {
        self.targets.first()
            .ok_or_else(|| invalid("stock-target portfolio is empty"))
            .and_then(|target| self.worker_request(target).unit_ids())
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.kind != "stock_targets"
            || self.targets.is_empty()
            || self.targets.len() > 4
            || self.targets.windows(2).any(|pair| pair[0].recipe >= pair[1].recipe)
        {
            return Err(invalid("stock_targets requires one to four distinct canonical recipes"));
        }
        for target in &self.targets {
            if !(1..=1_000_000).contains(&target.minimum_stack_units)
                || !(1..=100).contains(&target.max_order_amount)
            {
                return Err(invalid("stock-target portfolio minimum or finite order limit is invalid"));
            }
            self.worker_request(target).validate()?;
        }
        Ok(())
    }

    /// The complete target vector remains in the immutable original. Only
    /// positive deficits become finite order leaves; met minima remain parent
    /// requirements and are observed again with every current whole-goal proof.
    pub(crate) fn compile(
        &self,
        assessments: &[StockTargetAssessment],
        evidence: &PredicateEvidence<'_>,
        workers: &ExistingLaborRequirement,
        c: &OperationContext,
        origin: Option<Digest32>,
    ) -> Result<PreparedPlan> {
        self.validate()?;
        if assessments.len() != self.targets.len()
            || assessments.iter().zip(&self.targets).any(|(assessment, target)| {
                recipe(&target.recipe).map_or(true, |(native, _)| native != assessment.recipe())
                    || assessment.target_stack_units() != target.minimum_stack_units
                    || assessment.max_order_amount() != target.max_order_amount
                    || assessment.original_anchor() != c.anchor
            })
        {
            return Err(invalid("stock-target portfolio differs from its complete original census"));
        }
        let binding = StockTargetsBinding::from_assessments(assessments)?;
        let mut orders = Vec::new();
        let mut objectives = Vec::new();
        for assessment in assessments {
            if let Some(amount) = assessment.amount() {
                let objective = assessment.objective()
                    .ok_or_else(|| invalid("positive stock target lacks its qualified growth goal"))?
                    .clone();
                orders.push(PortfolioOrder {
                    recipe: assessment.recipe().as_str().into(),
                    amount,
                    objective: Some(Objective::ObservedInventoryIncrease {
                        item_type: objective.item_type().into(),
                        increase_stack_units: objective.increase_stack_units(),
                    }),
                });
                objectives.push(objective);
            }
        }
        if orders.is_empty() {
            return Err(invalid("an already-satisfied portfolio has no original effects"));
        }
        let request = ExistingLaborPortfolioRequest {
            kind: "existing_labor_portfolio".into(),
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            orders,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
        };
        request.compile_with_stock_targets(
            evidence, workers, &InventoryBaselines::from_qualified_many(objectives)?,
            c, origin, Some(&binding),
        )
    }
}

#[cfg(test)]
#[path = "stock_targets_request_tests.rs"]
mod tests;
