//! One finite stock-target request, lowered from an independently qualified
//! original census into the existing-worker production owner.
use super::existing_labor_request::ExistingLaborRequest;
use super::request::recipe;
use super::stock_target_binding::StockTargetBinding;
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
pub(crate) struct StockTargetRequest {
    pub kind: String,
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
    pub recipe: String,
    pub minimum_stack_units: u64,
    #[serde(default = "maximum")]
    pub max_order_amount: u32,
    pub deadline_tick: u64,
    #[serde(default = "interval")]
    pub poll_interval_ticks: u64,
    #[serde(default = "samples")]
    pub stable_samples: u32,
}

fn maximum() -> u32 {
    100
}
fn interval() -> u64 {
    10
}
fn samples() -> u32 {
    1
}

impl StockTargetRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(exhausted("stock-target request exceeds 4096 bytes"));
        }
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed stock-target production request"))?;
        value.validate()?;
        Ok(value)
    }

    pub(crate) fn observation(&self) -> Result<Digest32> {
        digest(&self.expected_observation)
    }

    pub(crate) fn unit_ids(&self) -> Result<Vec<EntityId>> {
        self.existing(1, None).unit_ids()
    }

    pub(crate) fn validate(&self) -> Result<()> {
        if self.kind != "stock_target" {
            return Err(invalid("stock-target request requires kind=stock_target"));
        }
        if !(1..=1_000_000).contains(&self.minimum_stack_units) {
            return Err(invalid(
                "stock target must be between one and 1000000 stack units",
            ));
        }
        if !(1..=100).contains(&self.max_order_amount) {
            return Err(invalid(
                "stock-target maximum order amount must be between one and 100",
            ));
        }
        self.existing(1, None).validate()
    }

    fn existing(&self, amount: u32, objective: Option<Objective>) -> ExistingLaborRequest {
        ExistingLaborRequest {
            kind: "existing_labor".into(),
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            recipe: self.recipe.clone(),
            amount,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
            objective,
        }
    }

    /// A met target is a no-work observation handled before this compiler. A
    /// positive assessment seals exactly its bounded deficit; no truncation,
    /// caller baseline, replacement census or labor assignment is introduced.
    pub(crate) fn compile(
        &self,
        assessment: &StockTargetAssessment,
        evidence: &PredicateEvidence<'_>,
        requirement: &ExistingLaborRequirement,
        c: &OperationContext,
        origin: Option<Digest32>,
    ) -> Result<PreparedPlan> {
        self.validate()?;
        let (requested_recipe, _) = recipe(&self.recipe)?;
        if assessment.recipe() != requested_recipe
            || assessment.target_stack_units() != self.minimum_stack_units
            || assessment.max_order_amount() != self.max_order_amount
            || assessment.original_anchor() != c.anchor
        {
            return Err(invalid(
                "stock target differs from its exact original assessment",
            ));
        }
        let binding = StockTargetBinding::from_assessment(assessment)?;
        let objective = binding.objective().clone();
        let amount = assessment
            .amount()
            .ok_or_else(|| invalid("a satisfied stock target has no original order amount"))?;
        let existing = self.existing(
            amount,
            Some(Objective::ObservedInventoryIncrease {
                item_type: objective.item_type().to_owned(),
                increase_stack_units: objective.increase_stack_units(),
            }),
        );
        let baselines = InventoryBaselines::from_qualified(objective);
        existing.compile_with_stock_target(
            evidence,
            requirement,
            &baselines,
            c,
            origin,
            Some(&binding),
        )
    }
}

#[cfg(test)]
#[path = "stock_target_request_tests.rs"]
pub(super) mod tests;
