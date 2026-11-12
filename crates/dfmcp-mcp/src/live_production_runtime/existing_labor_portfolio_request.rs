//! A bounded original portfolio whose selected workers already have the labor.
//! The separate identity domain preserves every existing request's bytes.
use super::existing_labor_request::ExistingLaborRequest;
use super::portfolio_request::{MAX_ORDERS, PortfolioOrder};
use super::request::recipe;
use super::stock_targets_binding::StockTargetsBinding;
use super::*;
use dfmcp_core::{IntentId, StepId};
use dfmcp_intent::effects::created_entity_id;
use dfmcp_intent::{
    Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{CompareOp, Predicate, Value as FactValue};
use orders::inventory_baseline::InventoryBaselines;
use orders::request::Objective;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExistingLaborPortfolioRequest {
    pub kind: String,
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
    pub orders: Vec<PortfolioOrder>,
    pub deadline_tick: u64,
    #[serde(default = "interval")]
    pub poll_interval_ticks: u64,
    #[serde(default = "samples")]
    pub stable_samples: u32,
}
fn interval() -> u64 {
    10
}
fn samples() -> u32 {
    1
}
impl ExistingLaborPortfolioRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 8192 {
            return Err(exhausted(
                "existing-labor portfolio request exceeds 8192 bytes",
            ));
        }
        let mut value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed existing-labor portfolio request"))?;
        value.orders.sort_by(|a, b| a.recipe.cmp(&b.recipe));
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn observation(&self) -> Result<Digest32> {
        workforce::digest(&self.expected_observation)
    }
    fn request(&self, order: &PortfolioOrder) -> ExistingLaborRequest {
        ExistingLaborRequest {
            kind: "existing_labor".into(),
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            recipe: order.recipe.clone(),
            amount: order.amount,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
            objective: order.objective.clone(),
        }
    }
    pub(crate) fn unit_ids(&self) -> Result<Vec<dfmcp_core::EntityId>> {
        self.orders
            .first()
            .ok_or_else(|| invalid("existing-labor portfolio is empty"))
            .and_then(|order| self.request(order).unit_ids())
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.kind != "existing_labor_portfolio"
            || self.orders.is_empty()
            || self.orders.len() > MAX_ORDERS
            || self
                .orders
                .windows(2)
                .any(|pair| pair[0].recipe >= pair[1].recipe)
        {
            return Err(invalid(
                "existing-labor portfolio requires one to four distinct recipes in canonical recipe order",
            ));
        }
        for order in &self.orders {
            self.request(order).validate()?;
        }
        Ok(())
    }
    pub(crate) fn compile(
        &self,
        evidence: &PredicateEvidence<'_>,
        requirement: &ExistingLaborRequirement,
        c: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.compile_with_inventory(evidence, requirement, &InventoryBaselines::default(), c)
    }
    pub(crate) fn compile_with_inventory(
        &self,
        evidence: &PredicateEvidence<'_>,
        requirement: &ExistingLaborRequirement,
        baselines: &InventoryBaselines,
        c: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.compile_with_workshop(evidence, requirement, baselines, c, None)
    }
    pub(crate) fn compile_with_workshop(
        &self,
        evidence: &PredicateEvidence<'_>,
        requirement: &ExistingLaborRequirement,
        baselines: &InventoryBaselines,
        c: &OperationContext,
        origin: Option<Digest32>,
    ) -> Result<PreparedPlan> {
        self.compile_with_stock_targets(evidence, requirement, baselines, c, origin, None)
    }
    pub(crate) fn compile_with_prerequisites(
        &self,
        evidence: &PredicateEvidence<'_>,
        requirement: &ExistingLaborRequirement,
        baselines: &InventoryBaselines,
        c: &OperationContext,
        origins: super::prerequisite_binding::Origins,
    ) -> Result<PreparedPlan> {
        self.compile_with_stock_targets(evidence, requirement, baselines, c, origins, None)
    }
    pub(super) fn compile_with_stock_targets(
        &self,
        evidence: &PredicateEvidence<'_>,
        requirement: &ExistingLaborRequirement,
        baselines: &InventoryBaselines,
        c: &OperationContext,
        origins: impl Into<super::prerequisite_binding::Origins>,
        stock_targets: Option<&StockTargetsBinding>,
    ) -> Result<PreparedPlan> {
        let origins = origins.into();
        self.validate()?;
        // Reuse the exact single-request citizen, prerequisite and timing
        // admission. This temporary plan never acquires custody or native state.
        let mut template = self.request(&self.orders[0]);
        template.objective = None;
        template.compile(evidence, requirement, c)?;

        let prerequisite = requirement.predicate();
        let predicate_bytes = prerequisite.canonical_bytes();
        let mut bytes = b"dfmcp-native-existing-labor-portfolio-request/1\0".to_vec();
        bytes.extend_from_slice(self.observation()?.as_bytes());
        bytes.extend_from_slice(&(predicate_bytes.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&predicate_bytes);
        bytes.extend_from_slice(&self.deadline_tick.to_be_bytes());
        bytes.extend_from_slice(&self.poll_interval_ticks.to_be_bytes());
        bytes.extend_from_slice(&self.stable_samples.to_be_bytes());
        bytes.extend_from_slice(&(self.orders.len() as u32).to_be_bytes());
        for order in &self.orders {
            let (native, _) = recipe(&order.recipe)?;
            bytes.extend_from_slice(&(native as u32).to_be_bytes());
            bytes.extend_from_slice(&order.amount.to_be_bytes());
            bytes.push(u8::from(order.objective.is_some()));
            if let Some(objective) = &order.objective {
                objective.append_identity(&mut bytes, baselines, c.anchor)?;
            }
        }
        if let Some(binding) = stock_targets {
            if binding.original_anchor != c.anchor {
                return Err(denied(
                    "stock-target portfolio belongs to another original anchor",
                ));
            }
            bytes = binding.identity(
                requirement,
                GameTick(self.deadline_tick),
                self.poll_interval_ticks,
                self.stable_samples,
            )?;
        }
        super::prerequisite_binding::append_identity(&mut bytes, origins)?;
        let digest = Digest32::of_bytes(&bytes);
        let id = IntentId::new(u128::from_be_bytes(
            digest.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("existing-labor portfolio intent identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("reserved existing-labor portfolio intent identity"));
        }
        let mut terminal = vec![prerequisite];
        if let Some(binding) = stock_targets {
            terminal.extend(binding.minima());
        }
        let mut actions = Vec::with_capacity(self.orders.len());
        for (index, requested) in self.orders.iter().enumerate() {
            let (native, job) = recipe(&requested.recipe)?;
            let action = Action::CreateWorkOrder {
                name: if stock_targets.is_some() {
                    format!("native inventory-targets/1 {} goal", native.as_str())
                } else {
                    format!("native existing-labor portfolio {} goal", native.as_str())
                },
                job_token: job.into(),
                amount: requested.amount,
                conditions: Vec::new(),
            };
            let key = derive_step_idempotency_key(id, c.anchor, StepId::new(index as u32), &action);
            let entity = created_entity_id(&key, 0);
            let equal = |field: &str, value| Predicate::FieldCompare {
                entity_id: entity,
                field: field.into(),
                op: CompareOp::Eq,
                value,
            };
            let goal = match &requested.objective {
                None => Predicate::All(vec![
                    equal("amount_total", FactValue::U64(u64::from(requested.amount))),
                    equal("amount_remaining", FactValue::U64(0)),
                    equal("validated", FactValue::Bool(true)),
                    equal("active", FactValue::Bool(false)),
                ]),
                Some(objective) => objective.predicate(baselines, c.anchor)?,
            };
            terminal.push(goal.clone());
            actions.push(RequestedAction {
                action,
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![goal.clone()],
                compensation: None,
                depends_on: Vec::new(),
                obligation: Some(ObligationSpec {
                    terminal: goal,
                    failure: None,
                    deadline_tick: GameTick(self.deadline_tick),
                    poll_interval_ticks: self.poll_interval_ticks,
                    stable_for_observations: self.stable_samples,
                }),
            });
        }
        let intent = Intent {
            id,
            anchor: c.anchor,
            summary: super::prerequisite_binding::summary(
                match stock_targets {
                    Some(binding) => binding.summary()?,
                    None => format!(
                        "Use existing {} on {} citizens for {} original finite furniture goals",
                        self.labor,
                        requirement.units().len(),
                        self.orders.len()
                    ),
                },
                origins,
            )?,
            terminal_condition: Predicate::All(terminal),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: actions,
        };
        let mut planner = StaticPlanner::default();
        if origins.manager.is_some() {
            planner.policy.max_string_bytes = 1024;
        }
        if stock_targets.is_some() {
            // The closed marker retains up to four complete source-bound
            // assessments. Native action strings keep their existing bounds;
            // legacy originals retain the default policy and identical bytes.
            planner.policy.max_string_bytes = 1024;
        }
        let mut plan = planner.prepare_with_evidence(evidence, &intent, c)?;
        for step in &mut plan.steps {
            step.compensation = None;
        }
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        let layout = live_production::ProductionLayout::from_plan(&plan)?;
        let prerequisite_matches = match (layout.existing_labor(), stock_targets) {
            (Some(actual), Some(binding)) => {
                actual.predicate() == requirement.predicate()
                    && Predicate::All(actual.inventory_minima().to_vec()).normalized()
                        == Predicate::All(binding.minima()).normalized()
            }
            (actual, None) => actual == Some(requirement),
            _ => false,
        };
        if !prerequisite_matches
            || layout.assignment_step().is_some()
            || layout.order_steps().len() != self.orders.len()
        {
            return Err(denied(
                "compiled existing-labor portfolio lost its original prerequisite",
            ));
        }
        if let Some(binding) = stock_targets {
            if super::stock_targets_binding::requirement(&plan)?.as_ref() != Some(binding) {
                return Err(denied(
                    "compiled stock-target portfolio lost its complete original policy",
                ));
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
#[path = "existing_labor_portfolio_request_tests.rs"]
mod tests;
