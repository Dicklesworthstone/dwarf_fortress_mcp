//! Closed native-observable finite furniture-order goal compiler.
use super::{exhausted, invalid};
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::{Digest32, GameTick, IntentId, OperationContext, Result, RiskTier, StepId};
use dfmcp_intent::effects::{MAX_DEFAULT_OBLIGATION_TICKS, created_entity_id};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
    derive_step_idempotency_key,
};
use dfmcp_world::{CompareOp, Predicate, PredicateEvidence, Value};
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub expected_observation: String,
    pub recipe: String,
    pub amount: u32,
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
impl Request {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 4096 {
            return Err(exhausted("semantic production request exceeds 4096 bytes"));
        }
        let request: Self = serde_json::from_str(raw)
            .map_err(|_| invalid("invalid closed semantic furniture-order request"))?;
        request.validate()?;
        Ok(request)
    }
    pub fn observation(&self) -> Result<Digest32> {
        super::digest(&self.expected_observation)
    }
    pub fn validate(&self) -> Result<()> {
        self.observation()?;
        recipe(&self.recipe)?;
        if !(1..=100).contains(&self.amount)
            || self.poll_interval_ticks == 0
            || self.poll_interval_ticks > MAX_DEFAULT_OBLIGATION_TICKS
            || !(1..=64).contains(&self.stable_samples)
        {
            return Err(invalid(
                "finite order requires amount 1..100 and bounded nonzero cadence/stability",
            ));
        }
        Ok(())
    }
    pub fn compile(
        &self,
        evidence: &PredicateEvidence<'_>,
        context: &OperationContext,
    ) -> Result<PreparedPlan> {
        self.validate()?;
        let span = self
            .deadline_tick
            .checked_sub(context.anchor.tick.get())
            .filter(|span| *span > 0 && *span <= MAX_DEFAULT_OBLIGATION_TICKS)
            .ok_or_else(|| {
                invalid("semantic order deadline must be future and at most one game year away")
            })?;
        if self.poll_interval_ticks > span
            || self
                .poll_interval_ticks
                .checked_mul(u64::from(self.stable_samples))
                .is_none_or(|n| n > span)
        {
            return Err(invalid(
                "original deadline cannot contain the requested cadence and stability",
            ));
        }
        let (native, job) = recipe(&self.recipe)?;
        let action = Action::CreateWorkOrder {
            name: format!("native {} goal", native.as_str()),
            job_token: job.into(),
            amount: self.amount,
            conditions: Vec::new(),
        };
        let mut identity = b"dfmcp-native-semantic-production-request/1\0".to_vec();
        identity.extend_from_slice(self.observation()?.as_bytes());
        identity.extend_from_slice(&(native as u32).to_be_bytes());
        identity.extend_from_slice(&self.amount.to_be_bytes());
        identity.extend_from_slice(&self.deadline_tick.to_be_bytes());
        identity.extend_from_slice(&self.poll_interval_ticks.to_be_bytes());
        identity.extend_from_slice(&self.stable_samples.to_be_bytes());
        let hash = Digest32::of_bytes(&identity);
        let id = IntentId::new(u128::from_be_bytes(
            hash.as_bytes()[..16]
                .try_into()
                .map_err(|_| invalid("invalid semantic intent identity"))?,
        ));
        if id == IntentId::NIL {
            return Err(invalid("semantic intent identity is reserved"));
        }
        let key = derive_step_idempotency_key(id, context.anchor, StepId::ZERO, &action);
        let entity = created_entity_id(&key, 0);
        let equal = |field: &str, value| Predicate::FieldCompare {
            entity_id: entity,
            field: field.into(),
            op: CompareOp::Eq,
            value,
        };
        let terminal = Predicate::All(vec![
            equal("amount_total", Value::U64(u64::from(self.amount))),
            equal("amount_remaining", Value::U64(0)),
            equal("validated", Value::Bool(true)),
            equal("active", Value::Bool(false)),
        ]);
        let intent = Intent {
            id,
            anchor: context.anchor,
            summary: format!(
                "Complete {} finite {} order units",
                self.amount,
                native.as_str()
            ),
            terminal_condition: terminal.clone(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action,
                preconditions: vec![Predicate::Paused(true)],
                // Explicit native-observable semantics avoid the laboratory's
                // name/job/status/condition fallback, which this source lacks.
                postconditions: vec![terminal.clone()],
                compensation: None,
                depends_on: Vec::new(),
                obligation: Some(ObligationSpec {
                    terminal,
                    failure: None,
                    deadline_tick: GameTick(self.deadline_tick),
                    poll_interval_ticks: self.poll_interval_ticks,
                    stable_for_observations: self.stable_samples,
                }),
            }],
        };
        StaticPlanner::default().prepare_with_evidence(evidence, &intent, context)
    }
}
fn recipe(raw: &str) -> Result<(WorkOrderRecipe, &'static str)> {
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
