//! Closed native-observable finite furniture-order goal compiler.
use super::{exhausted, invalid};
use super::inventory_baseline::InventoryBaselines;
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::{Digest32, GameTick, IntentId, OperationContext, Result, RiskTier, StateAnchor, StepId};
use dfmcp_intent::effects::{MAX_DEFAULT_OBLIGATION_TICKS, created_entity_id};
use dfmcp_intent::{
    Action, Constraint, Intent, ObligationSpec, PreparedPlan, RequestedAction, StaticPlanner,
    derive_step_idempotency_key,
};
use dfmcp_world::{CompareOp, EntityKind, Predicate, PredicateEvidence, Value};
use serde::Deserialize;

/// A separately requested observation goal. It never asserts which order
/// produced an item, its material, or whether it is available for use.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Objective {
    ObservedInventoryMinimum {
        item_type: String,
        minimum_stack_units: u64,
    },
    ObservedInventoryIncrease {
        item_type: String,
        increase_stack_units: u64,
    },
}

impl Objective {
    pub(crate) fn validate_for(&self, recipe: WorkOrderRecipe) -> Result<()> {
        let (item_type, units) = match self {
            Self::ObservedInventoryMinimum { item_type, minimum_stack_units } => (item_type, minimum_stack_units),
            Self::ObservedInventoryIncrease { item_type, increase_stack_units } => (item_type, increase_stack_units),
        };
        if item_type != output_type(recipe) || !(1..=1_000_000).contains(units) {
            return Err(invalid(
                "inventory objective requires the recipe's exact item type and 1..1000000 stack units",
            ));
        }
        Ok(())
    }

    pub(crate) fn predicate(&self, baselines: &InventoryBaselines, anchor: StateAnchor) -> Result<Predicate> {
        Ok(match self {
            Self::ObservedInventoryMinimum { item_type, minimum_stack_units } => Predicate::EntityQuantity {
                kind: EntityKind::Item,
                filter_field: "type_key".into(),
                filter_value: Value::Text(item_type.clone()),
                quantity_field: Some("stack_size".into()),
                op: CompareOp::Ge,
                value: *minimum_stack_units,
            },
            Self::ObservedInventoryIncrease { item_type, increase_stack_units } =>
                baselines.get(item_type, *increase_stack_units, anchor)?.predicate(),
        })
    }

    pub(crate) fn append_identity(&self, bytes: &mut Vec<u8>, baselines: &InventoryBaselines, anchor: StateAnchor) -> Result<()> {
        match self {
            Self::ObservedInventoryMinimum { item_type, minimum_stack_units } => {
                // Preserve every original minimum request and step key byte.
                bytes.extend_from_slice(&(item_type.len() as u64).to_be_bytes());
                bytes.extend_from_slice(item_type.as_bytes());
                bytes.extend_from_slice(&minimum_stack_units.to_be_bytes());
            }
            Self::ObservedInventoryIncrease { .. } => {
                // A reserved impossible legacy type length separates the new
                // mode. The complete predicate binds the independently derived
                // baseline, native source lineage and original anchor to keys.
                bytes.extend_from_slice(&u64::MAX.to_be_bytes());
                bytes.extend_from_slice(b"dfmcp-observed-inventory-increase/1\0");
                let predicate = self.predicate(baselines, anchor)?.canonical_bytes();
                bytes.extend_from_slice(&(predicate.len() as u32).to_be_bytes());
                bytes.extend_from_slice(&predicate);
            }
        }
        Ok(())
    }
}

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
        let (native, _) = recipe(&self.recipe)?;
        if !(1..=100).contains(&self.amount)
            || self.poll_interval_ticks == 0
            || self.poll_interval_ticks > MAX_DEFAULT_OBLIGATION_TICKS
            || !(1..=64).contains(&self.stable_samples)
        {
            return Err(invalid(
                "finite order requires amount 1..100 and bounded nonzero cadence/stability",
            ));
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
        // Default requests retain their exact identity, original plan bytes and
        // history policy. The optional objective is a distinct sealed intent.
        let mut identity = if self.objective.is_some() {
            b"dfmcp-native-semantic-production-request/2\0".to_vec()
        } else {
            b"dfmcp-native-semantic-production-request/1\0".to_vec()
        };
        identity.extend_from_slice(self.observation()?.as_bytes());
        identity.extend_from_slice(&(native as u32).to_be_bytes());
        identity.extend_from_slice(&self.amount.to_be_bytes());
        identity.extend_from_slice(&self.deadline_tick.to_be_bytes());
        identity.extend_from_slice(&self.poll_interval_ticks.to_be_bytes());
        identity.extend_from_slice(&self.stable_samples.to_be_bytes());
        if let Some(objective) = &self.objective {
            objective.append_identity(&mut identity, baselines, context.anchor)?;
        }
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
        let order_terminal = Predicate::All(vec![
            equal("amount_total", Value::U64(u64::from(self.amount))),
            equal("amount_remaining", Value::U64(0)),
            equal("validated", Value::Bool(true)),
            equal("active", Value::Bool(false)),
        ]);
        let (terminal, summary) = match &self.objective {
            Some(
                objective @ Objective::ObservedInventoryMinimum {
                    item_type,
                    minimum_stack_units,
                },
            ) => (
                objective.predicate(baselines, context.anchor)?,
                format!(
                    "Observe at least {minimum_stack_units} {item_type} stack units after inserting {} finite {} order units; production attribution unknown",
                    self.amount,
                    native.as_str()
                ),
            ),
            Some(objective @ Objective::ObservedInventoryIncrease { item_type, increase_stack_units }) => (
                objective.predicate(baselines, context.anchor)?,
                format!(
                    "Observe an increase of at least {increase_stack_units} {item_type} stack units from the sealed original census after inserting {} finite {} order units; production attribution unknown",
                    self.amount, native.as_str()
                ),
            ),
            None => (
                order_terminal,
                format!(
                    "Complete {} finite {} order units",
                    self.amount,
                    native.as_str()
                ),
            ),
        };
        let intent = Intent {
            id,
            anchor: context.anchor,
            summary,
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
fn output_type(recipe: WorkOrderRecipe) -> &'static str {
    match recipe {
        WorkOrderRecipe::WoodenBed => "BED",
        WorkOrderRecipe::WoodenDoor => "DOOR",
        WorkOrderRecipe::WoodenTable => "TABLE",
        WorkOrderRecipe::WoodenChair => "CHAIR",
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
