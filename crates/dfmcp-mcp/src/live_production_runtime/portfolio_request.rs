//! Explicit bounded production portfolio. Legacy request identities are unchanged.
use super::request::{Request, recipe};
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

pub(super) const MAX_ORDERS: usize = 4;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortfolioOrder {
    pub recipe: String,
    pub amount: u32,
    #[serde(default)]
    pub objective: Option<Objective>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct PortfolioRequest {
    pub kind: String,
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
    pub enabled: bool,
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

impl PortfolioRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 8192 {
            return Err(exhausted("production portfolio request exceeds 8192 bytes"));
        }
        let mut value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed production portfolio request"))?;
        value.orders.sort_by(|a, b| a.recipe.cmp(&b.recipe));
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn observation(&self) -> Result<Digest32> {
        workforce::digest(&self.expected_observation)
    }
    fn request(&self, order: &PortfolioOrder) -> Request {
        Request {
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            enabled: self.enabled,
            recipe: order.recipe.clone(),
            amount: order.amount,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
            objective: order.objective.clone(),
        }
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.kind != "portfolio"
            || self.orders.is_empty()
            || self.orders.len() > MAX_ORDERS
            || self
                .orders
                .windows(2)
                .any(|pair| pair[0].recipe >= pair[1].recipe)
        {
            return Err(invalid(
                "portfolio requires one to four distinct recipes in canonical recipe order",
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
        c: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.compile_with_inventory(evidence, &InventoryBaselines::default(), c)
    }
    pub(crate) fn compile_with_inventory(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        c: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.compile_with_workshop(evidence, baselines, c, None)
    }
    pub(crate) fn compile_with_workshop(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        c: &OperationContext,
        origin: Option<Digest32>,
    ) -> Result<PreparedPlan> {
        self.compile_with_stock_targets(evidence, baselines, c, origin, None)
    }
    pub(crate) fn compile_with_prerequisites(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        c: &OperationContext,
        origins: super::prerequisite_binding::Origins,
    ) -> Result<PreparedPlan> {
        self.compile_with_stock_targets(evidence, baselines, c, origins, None)
    }
    pub(super) fn compile_with_stock_targets(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        c: &OperationContext,
        origins: impl Into<super::prerequisite_binding::Origins>,
        stock_targets: Option<&StockTargetsBinding>,
    ) -> Result<PreparedPlan> {
        let origins = origins.into();
        self.validate()?;
        let first = self
            .orders
            .first()
            .ok_or_else(|| invalid("portfolio is empty"))?;
        // Reuse the unchanged single-order compiler's exact canonical citizen,
        // labor, original source and timing checks. This temporary value is never
        // persisted, prepared or dispatched; only its labor template is retained.
        let mut template_request = self.request(first);
        // The temporary template must not reject a useful whole portfolio just
        // because its alphabetically first inventory quota already holds.
        template_request.objective = None;
        let template = template_request.compile(evidence, c)?;
        let labor = template
            .steps
            .first()
            .ok_or_else(|| invalid("labor template absent"))?;
        let units = self.request(first).unit_ids()?;
        let mut bytes = b"dfmcp-native-production-portfolio-request/1\0".to_vec();
        bytes.extend_from_slice(self.observation()?.as_bytes());
        bytes.extend_from_slice(&(units.len() as u32).to_be_bytes());
        for unit in &units {
            bytes.extend_from_slice(&unit.get().to_be_bytes());
        }
        bytes.extend_from_slice(&(self.labor.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.labor.as_bytes());
        bytes.push(u8::from(self.enabled));
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
                    "assigned inventory targets belong to another original anchor",
                ));
            }
            bytes = binding.assignment_identity(
                &labor.action,
                &Predicate::All(labor.preconditions.clone()),
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
                .map_err(|_| invalid("portfolio intent identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("reserved portfolio intent identity"));
        }
        let mut terminal = labor.postconditions.clone();
        if let Some(binding) = stock_targets {
            terminal.extend(binding.minima());
        }
        let mut actions = vec![RequestedAction {
            action: labor.action.clone(),
            preconditions: labor.preconditions.clone(),
            postconditions: labor.postconditions.clone(),
            compensation: None,
            depends_on: Vec::new(),
            obligation: None,
        }];
        for (index, requested) in self.orders.iter().enumerate() {
            let (native, job) = recipe(&requested.recipe)?;
            let action = Action::CreateWorkOrder {
                name: if stock_targets.is_some() {
                    format!("native inventory-targets/2 {} goal", native.as_str())
                } else {
                    format!("native portfolio {} goal", native.as_str())
                },
                job_token: job.into(),
                amount: requested.amount,
                conditions: Vec::new(),
            };
            let key =
                derive_step_idempotency_key(id, c.anchor, StepId::new((index + 1) as u32), &action);
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
                depends_on: vec![0],
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
                        "Production portfolio: configure {} for {} citizens and satisfy {} original finite furniture goals",
                        self.labor,
                        units.len(),
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
            // Only the complete, closed vector marker needs the larger summary
            // allowance. Legacy plans retain their original policy and bytes.
            planner.policy.max_string_bytes = 1024;
        }
        if origins.input_supply.is_some() {
            planner.policy.max_string_bytes = 2048;
        }
        let mut plan = planner.prepare_with_evidence(evidence, &intent, c)?;
        for step in &mut plan.steps {
            step.compensation = None;
        }
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        plan.validate_structure()?;
        live_production::validate_plan(&plan)?;
        if let Some(binding) = stock_targets
            && super::stock_targets_binding::requirement(&plan)?.as_ref() != Some(binding)
        {
            return Err(denied(
                "compiled assigned inventory targets lost their complete original policy",
            ));
        }
        Ok(plan)
    }
}
