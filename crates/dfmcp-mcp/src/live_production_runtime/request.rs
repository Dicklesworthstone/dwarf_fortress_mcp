//! Closed combined labor and finite furniture-order request compiler.
use crate::semantic_work_order_runtime::inventory_baseline::InventoryBaselines;
use crate::semantic_work_order_runtime::request::Objective;
use crate::semantic_workforce_runtime::{digest, exhausted, invalid};
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::{
    Digest32, EntityId, GameTick, IntentId, OperationContext, Result, RiskTier, StepId,
};
use dfmcp_intent::effects::{MAX_DEFAULT_OBLIGATION_TICKS, created_entity_id};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
    derive_step_idempotency_key,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, Value};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
    pub enabled: bool,
    pub recipe: String,
    pub amount: u32,
    pub deadline_tick: u64,
    #[serde(default = "interval")]
    pub poll_interval_ticks: u64,
    #[serde(default = "samples")]
    pub stable_samples: u32,
    #[serde(default)]
    pub objective: Option<Objective>,
}
fn interval() -> u64 {
    10
}
fn samples() -> u32 {
    1
}

impl Request {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(exhausted("combined production request exceeds 4096 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed combined production request"))?;
        request.validate()?;
        Ok(request)
    }
    pub fn observation(&self) -> Result<Digest32> {
        digest(&self.expected_observation)
    }
    fn workforce(&self) -> crate::semantic_workforce_runtime::Request {
        crate::semantic_workforce_runtime::Request {
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            enabled: self.enabled,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
        }
    }
    pub fn unit_ids(&self) -> Result<Vec<EntityId>> {
        self.workforce().unit_ids()
    }
    pub fn validate(&self) -> Result<()> {
        self.workforce().validate()?;
        let (native, _) = recipe(&self.recipe)?;
        if !(1..=100).contains(&self.amount) {
            return Err(invalid("finite order amount must be between one and 100"));
        }
        if let Some(objective) = &self.objective {
            objective.validate_for(native)?;
        }
        Ok(())
    }
    pub fn compile(
        &self,
        evidence: &PredicateEvidence<'_>,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.compile_with_inventory(evidence, &InventoryBaselines::default(), context)
    }
    pub(crate) fn compile_with_inventory(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.compile_with_workshop(evidence, baselines, context, None)
    }
    pub(crate) fn compile_with_workshop(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        context: &OperationContext,
        origin: Option<Digest32>,
    ) -> Result<PreparedPlan> {
        self.compile_with_stock_target(evidence, baselines, context, origin, None)
    }
    pub(crate) fn compile_with_prerequisites(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        context: &OperationContext,
        origins: super::prerequisite_binding::Origins,
    ) -> Result<PreparedPlan> {
        self.compile_with_stock_target(evidence, baselines, context, origins, None)
    }
    pub(super) fn compile_with_stock_target(
        &self,
        evidence: &PredicateEvidence<'_>,
        baselines: &InventoryBaselines,
        context: &OperationContext,
        origins: impl Into<super::prerequisite_binding::Origins>,
        stock_target: Option<&super::stock_target_binding::StockTargetBinding>,
    ) -> Result<PreparedPlan> {
        let origins = origins.into();
        self.validate()?;
        if evidence.snapshot().anchor() != context.anchor {
            return Err(invalid(
                "combined compiler requires the exact observation anchor",
            ));
        }
        let span = self
            .deadline_tick
            .checked_sub(context.anchor.tick.get())
            .filter(|n| *n > 0 && *n <= MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(|| {
                invalid("goal deadline must be future and at most one game year away")
            })?;
        if self
            .poll_interval_ticks
            .checked_mul(u64::from(self.stable_samples))
            .is_none_or(|n| n > span)
        {
            return Err(invalid(
                "original deadline cannot contain the requested cadence and stability",
            ));
        }
        let units = self.unit_ids()?;
        let mut preconditions = vec![Predicate::Paused(true)];
        let mut labor_goals = Vec::with_capacity(units.len());
        for id in &units {
            let record = evidence
                .snapshot()
                .graph
                .entities
                .get(id)
                .filter(|r| r.kind == EntityKind::Unit)
                .ok_or_else(|| {
                    invalid("selected citizen is absent from the exact canonical roster")
                })?;
            let native = match record.fields.get("raw_unit_id").map(|field| &field.value) {
                Some(Value::I64(n)) if *n >= 0 => *n,
                _ => return Err(invalid("selected citizen has no observed native identity")),
            };
            preconditions.push(Predicate::EntityExists(*id));
            preconditions.push(Predicate::EntityKind {
                entity_id: *id,
                kind: EntityKind::Unit,
            });
            preconditions.push(Predicate::FieldCompare {
                entity_id: *id,
                field: "raw_unit_id".into(),
                op: CompareOp::Eq,
                value: Value::I64(native),
            });
            labor_goals.push(Predicate::FieldCompare {
                entity_id: *id,
                field: format!("labor.{}", self.labor),
                op: CompareOp::Eq,
                value: Value::Bool(self.enabled),
            });
        }
        let (native, job) = recipe(&self.recipe)?;
        let order = Action::CreateWorkOrder {
            name: stock_target.map_or_else(
                || format!("native {} goal", native.as_str()),
                |binding| binding.order_name(),
            ),
            job_token: job.into(),
            amount: self.amount,
            conditions: Vec::new(),
        };
        let mut bytes = if self.objective.is_some() {
            b"dfmcp-native-combined-production-request/2\0".to_vec()
        } else {
            b"dfmcp-native-combined-production-request/1\0".to_vec()
        };
        bytes.extend_from_slice(self.observation()?.as_bytes());
        bytes.extend_from_slice(&(units.len() as u32).to_be_bytes());
        for id in &units {
            bytes.extend_from_slice(&id.get().to_be_bytes());
        }
        bytes.extend_from_slice(&(self.labor.len() as u32).to_be_bytes());
        bytes.extend_from_slice(self.labor.as_bytes());
        bytes.push(u8::from(self.enabled));
        bytes.extend_from_slice(&(native as u32).to_be_bytes());
        bytes.extend_from_slice(&self.amount.to_be_bytes());
        bytes.extend_from_slice(&self.deadline_tick.to_be_bytes());
        bytes.extend_from_slice(&self.poll_interval_ticks.to_be_bytes());
        bytes.extend_from_slice(&self.stable_samples.to_be_bytes());
        if let Some(objective) = &self.objective {
            objective.append_identity(&mut bytes, baselines, context.anchor)?;
        }
        let assignment = Action::SetLabor {
            units: units.clone(),
            labor: self.labor.clone(),
            enabled: self.enabled,
        };
        super::stock_target_binding::append_assignment_identity(
            &mut bytes,
            stock_target,
            &assignment,
            &Predicate::All(preconditions.clone()),
            GameTick(self.deadline_tick),
            self.poll_interval_ticks,
            self.stable_samples,
        )?;
        super::prerequisite_binding::append_identity(&mut bytes, origins)?;
        let hash = Digest32::of_bytes(&bytes);
        let id = IntentId::new(u128::from_be_bytes(
            hash.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("intent identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("reserved intent identity"));
        }
        let order_key = derive_step_idempotency_key(id, context.anchor, StepId::new(1), &order);
        let entity = created_entity_id(&order_key, 0);
        let equal = |field: &str, value| Predicate::FieldCompare {
            entity_id: entity,
            field: field.into(),
            op: CompareOp::Eq,
            value,
        };
        let order_completion = Predicate::All(vec![
            equal("amount_total", Value::U64(u64::from(self.amount))),
            equal("amount_remaining", Value::U64(0)),
            equal("validated", Value::Bool(true)),
            equal("active", Value::Bool(false)),
        ]);
        let order_goal = match &self.objective {
            Some(objective) => objective.predicate(baselines, context.anchor)?,
            None => order_completion,
        };
        let labor_goal = Predicate::All(labor_goals);
        let summary = match &self.objective {
            Some(Objective::ObservedInventoryMinimum {
                item_type,
                minimum_stack_units,
            }) => format!(
                "Configure {} for {} citizens, insert {} finite {} order units and observe at least {minimum_stack_units} {item_type} stack units; production attribution unknown",
                self.labor,
                units.len(),
                self.amount,
                native.as_str()
            ),
            Some(Objective::ObservedInventoryIncrease {
                item_type,
                increase_stack_units,
            }) => format!(
                "Configure {} for {} citizens, insert {} finite {} order units and observe an increase of at least {increase_stack_units} {item_type} stack units from the sealed original census; production attribution unknown",
                self.labor,
                units.len(),
                self.amount,
                native.as_str()
            ),
            None => format!(
                "Configure {} for {} citizens and complete {} finite {} order units",
                self.labor,
                units.len(),
                self.amount,
                native.as_str()
            ),
        };
        let intent = Intent {
            id,
            anchor: context.anchor,
            summary: super::prerequisite_binding::summary(
                super::stock_target_binding::summary(summary, stock_target)?,
                origins,
            )?,
            terminal_condition: Predicate::All(vec![labor_goal.clone(), order_goal.clone()]),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![
                RequestedAction {
                    action: assignment,
                    preconditions: vec![Predicate::All(preconditions)],
                    postconditions: vec![labor_goal],
                    compensation: None,
                    depends_on: Vec::new(),
                    obligation: None,
                },
                RequestedAction {
                    action: order,
                    preconditions: vec![Predicate::Paused(true)],
                    postconditions: vec![order_goal.clone()],
                    compensation: None,
                    depends_on: vec![0],
                    obligation: Some(ObligationSpec {
                        terminal: order_goal,
                        failure: None,
                        deadline_tick: GameTick(self.deadline_tick),
                        poll_interval_ticks: self.poll_interval_ticks,
                        stable_for_observations: self.stable_samples,
                    }),
                },
            ],
        };
        let mut planner = StaticPlanner::default();
        if origins.manager.is_some() {
            planner.policy.max_string_bytes = 1024;
        }
        if origins.input_supply.is_some() {
            planner.policy.max_string_bytes = 2048;
        }
        let mut plan = planner.prepare_with_evidence(evidence, &intent, context)?;
        for step in &mut plan.steps {
            step.compensation = None;
        }
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        plan.validate_structure()?;
        dfmcp_adapter::live_production::validate_plan(&plan)?;
        if let Some(binding) = stock_target
            && super::stock_target_binding::requirement(&plan)?.as_ref() != Some(binding)
        {
            return Err(invalid(
                "compiled stock target differs from its exact assignment and census",
            ));
        }
        Ok(plan)
    }
}
pub(super) fn recipe(raw: &str) -> Result<(WorkOrderRecipe, &'static str)> {
    match raw {
        "wooden_bed" => Ok((WorkOrderRecipe::WoodenBed, "CONSTRUCT_BED")),
        "wooden_door" => Ok((WorkOrderRecipe::WoodenDoor, "CONSTRUCT_DOOR")),
        "wooden_table" => Ok((WorkOrderRecipe::WoodenTable, "CONSTRUCT_TABLE")),
        "wooden_chair" => Ok((WorkOrderRecipe::WoodenChair, "CONSTRUCT_CHAIR")),
        _ => Err(invalid(
            "recipe must be wooden_bed, wooden_door, wooden_table or wooden_chair",
        )),
    }
}
#[cfg(test)]
#[path = "request_tests.rs"]
mod tests;
