//! A finite original order with an observed labor prerequisite and no assignment.
use super::request::{Request, recipe};
use super::*;
use dfmcp_adapter::live_production::ExistingLaborRequirement;
use dfmcp_core::{IntentId, StepId};
use dfmcp_intent::effects::{MAX_DEFAULT_OBLIGATION_TICKS, created_entity_id};
use dfmcp_intent::{
    Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner, derive_step_idempotency_key,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value as FactValue};
use orders::inventory_baseline::InventoryBaselines;
use orders::request::Objective;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExistingLaborRequest {
    pub kind: String,
    pub expected_observation: String,
    pub units: Vec<String>,
    pub labor: String,
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

impl ExistingLaborRequest {
    pub(crate) fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(exhausted("existing-labor request exceeds 4096 bytes"));
        }
        let value: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed existing-labor production request"))?;
        value.validate()?;
        Ok(value)
    }
    pub(crate) fn observation(&self) -> Result<Digest32> {
        workforce::digest(&self.expected_observation)
    }
    fn common(&self) -> Request {
        Request {
            expected_observation: self.expected_observation.clone(),
            units: self.units.clone(),
            labor: self.labor.clone(),
            enabled: true,
            recipe: self.recipe.clone(),
            amount: self.amount,
            deadline_tick: self.deadline_tick,
            poll_interval_ticks: self.poll_interval_ticks,
            stable_samples: self.stable_samples,
            objective: self.objective.clone(),
        }
    }
    pub(crate) fn unit_ids(&self) -> Result<Vec<dfmcp_core::EntityId>> {
        self.common().unit_ids()
    }
    pub(super) fn validate(&self) -> Result<()> {
        if self.kind != "existing_labor" {
            return Err(invalid(
                "existing-labor request requires kind=existing_labor",
            ));
        }
        self.common().validate()
    }
    /// The shell obtains this requirement from the exact retained native capture
    /// and independently qualifies it before any original custody is published.
    /// This compiler seals that prerequisite; it cannot invent a labor receipt.
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
        self.validate()?;
        let units = self.unit_ids()?;
        if evidence.snapshot().anchor() != c.anchor
            || requirement.units() != units
            || requirement.labor() != self.labor
        {
            return Err(denied(
                "existing labor differs from the exact original selection",
            ));
        }
        let span = self
            .deadline_tick
            .checked_sub(c.anchor.tick.get())
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
        for id in &units {
            if !evidence.establishes(&Predicate::EntityKind {
                entity_id: *id,
                kind: EntityKind::Unit,
            })? {
                return Err(denied(
                    "selected citizen kind is not established by the original source",
                ));
            }
        }
        let (native, job) = recipe(&self.recipe)?;
        let mut bytes = b"dfmcp-native-existing-labor-production-request/1\0".to_vec();
        bytes.extend_from_slice(self.observation()?.as_bytes());
        let prerequisite = requirement.predicate();
        let predicate_bytes = prerequisite.canonical_bytes();
        bytes.extend_from_slice(&(predicate_bytes.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&predicate_bytes);
        bytes.extend_from_slice(&(native as u32).to_be_bytes());
        bytes.extend_from_slice(&self.amount.to_be_bytes());
        bytes.extend_from_slice(&self.deadline_tick.to_be_bytes());
        bytes.extend_from_slice(&self.poll_interval_ticks.to_be_bytes());
        bytes.extend_from_slice(&self.stable_samples.to_be_bytes());
        bytes.push(u8::from(self.objective.is_some()));
        if let Some(objective) = &self.objective {
            objective.append_identity(&mut bytes, baselines, c.anchor)?;
        }
        super::workshop_binding::append_identity(&mut bytes, origin)?;
        let digest = Digest32::of_bytes(&bytes);
        let id = IntentId::new(u128::from_be_bytes(
            digest.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("existing-labor intent identity width"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("reserved existing-labor intent identity"));
        }
        let action = Action::CreateWorkOrder {
            name: format!("native existing-labor {} goal", native.as_str()),
            job_token: job.into(),
            amount: self.amount,
            conditions: Vec::new(),
        };
        let key = derive_step_idempotency_key(id, c.anchor, StepId::ZERO, &action);
        let entity = created_entity_id(&key, 0);
        let equal = |field: &str, value| Predicate::FieldCompare {
            entity_id: entity,
            field: field.into(),
            op: CompareOp::Eq,
            value,
        };
        let goal = match &self.objective {
            None => Predicate::All(vec![
                equal("amount_total", FactValue::U64(u64::from(self.amount))),
                equal("amount_remaining", FactValue::U64(0)),
                equal("validated", FactValue::Bool(true)),
                equal("active", FactValue::Bool(false)),
            ]),
            Some(objective) => objective.predicate(baselines, c.anchor)?,
        };
        let intent = Intent {
            id,
            anchor: c.anchor,
            summary: super::workshop_binding::summary(
                format!(
                    "Use existing {} on {} citizens for {} finite {} order units",
                    self.labor,
                    units.len(),
                    self.amount,
                    native.as_str()
                ),
                origin,
            )?,
            terminal_condition: Predicate::All(vec![prerequisite, goal.clone()]),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
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
            }],
        };
        let mut plan = StaticPlanner::default().prepare_with_evidence(evidence, &intent, c)?;
        for step in &mut plan.steps {
            step.compensation = None;
        }
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        plan.validate_structure()?;
        live_production::validate_plan(&plan)?;
        if ExistingLaborRequirement::from_plan(&plan)?.as_ref() != Some(requirement) {
            return Err(denied(
                "compiled existing-labor prerequisite did not retain its exact seal",
            ));
        }
        Ok(plan)
    }
}

#[cfg(test)]
#[path = "existing_labor_request_tests.rs"]
mod tests;
