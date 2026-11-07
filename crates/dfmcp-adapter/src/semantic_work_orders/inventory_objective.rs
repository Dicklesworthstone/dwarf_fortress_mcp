//! Recognize the closed objectives retained in an original sealed work-order plan.
//! Recognition chooses a source-policy contract; it grants no source authority
//! and does not attribute observed inventory to the manager order.

use dfmcp_core::{Digest32, EntityId, ErrorCode, Result, StateAnchor, StepId};
use dfmcp_intent::effects::created_entity_id;
use dfmcp_intent::{Action, PlanStep, PreparedPlan};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value};

use crate::live_operations::LiveOperationsObservation;

use super::error;

/// Completion of the exact receipt-linked finite order, while its native row
/// remains present. This is not an inventory or order-attributed output goal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetainedOrderObjective {
    pub order_entity_id: EntityId,
    pub amount_total: u32,
}

impl RetainedOrderObjective {
    /// Recover only the original closed counter/status conjunction. A broader
    /// or weakened predicate cannot select this source-policy contract, and the
    /// complete parent and actual selected idempotency key remain unchanged.
    pub fn from_selected_plan(plan: &PreparedPlan, step_id: StepId) -> Option<Self> {
        super::selected_original(plan, step_id).ok()?;
        let step = super::selected_step(plan, step_id).ok()?;
        let Action::CreateWorkOrder { amount, .. } = &step.action else {
            return None;
        };
        let order_entity_id = created_entity_id(&step.idempotency_key, 0);
        let equal = |field: &str, value| Predicate::FieldCompare {
            entity_id: order_entity_id,
            field: field.into(),
            op: CompareOp::Eq,
            value,
        };
        let expected = Predicate::All(vec![
            equal("amount_total", Value::U64(u64::from(*amount))),
            equal("amount_remaining", Value::U64(0)),
            equal("validated", Value::Bool(true)),
            equal("active", Value::Bool(false)),
        ])
        .normalized();
        let obligation = step.obligation.as_ref()?;
        if obligation.failure.is_some()
            || obligation.terminal.normalized() != expected
            || Predicate::All(step.postconditions.clone()).normalized() != expected
        {
            return None;
        }
        Some(Self {
            order_entity_id,
            amount_total: *amount,
        })
    }
}

/// An explicitly selected minimum of observed stack units of one furniture type.
/// Material, usability, ownership and production attribution are not predicates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedInventoryObjective {
    pub item_type: String,
    pub min_stack_units: u64,
}

impl ObservedInventoryObjective {
    /// The closed request's scalar bound, independent of the finite order amount.
    pub const MAX_STACK_UNITS: u64 = 1_000_000;

    /// Recover the objective from the exact original plan rather than from a
    /// later request or an observation. Legacy goals and any broader predicate
    /// shape return `None`, so they cannot widen an existing evidence policy.
    pub fn from_plan(plan: &PreparedPlan) -> Option<Self> {
        super::original(plan).ok()?;
        let step = plan.steps.first()?;
        Self::from_step(step, &plan.terminal_condition)
    }

    /// Recognize a selected production step without slicing or resealing its
    /// complete parent. This proves only the step's retained inventory objective;
    /// the parent terminal and other steps still require their own evidence.
    pub fn from_selected_plan(plan: &PreparedPlan, step_id: StepId) -> Option<Self> {
        super::selected_original(plan, step_id).ok()?;
        let step = super::selected_step(plan, step_id).ok()?;
        Self::from_step(step, &step.obligation.as_ref()?.terminal)
    }

    fn from_step(step: &PlanStep, terminal: &Predicate) -> Option<Self> {
        let expected_type = output_type(step)?;
        let Predicate::EntityQuantity {
            kind: EntityKind::Item,
            filter_field,
            filter_value: Value::Text(item_type),
            quantity_field: Some(quantity_field),
            op: CompareOp::Ge,
            value,
        } = terminal
        else {
            return None;
        };
        if filter_field != "type_key"
            || item_type != expected_type
            || quantity_field != "stack_size"
            || !(1..=Self::MAX_STACK_UNITS).contains(value)
        {
            return None;
        }
        let obligation = step.obligation.as_ref()?;
        if obligation.failure.is_some()
            || obligation.terminal.normalized() != *terminal
            || Predicate::All(step.postconditions.clone()).normalized() != *terminal
        {
            return None;
        }
        Some(Self {
            item_type: item_type.clone(),
            min_stack_units: *value,
        })
    }
}

/// A positive net increase over one independently qualified original census.
/// The baseline is immutable goal data, not a claim that any particular order
/// made the current items. Reopening does not recount or move the baseline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedInventoryIncreaseObjective {
    item_type: String,
    baseline_stack_units: u64,
    increase_stack_units: u64,
    target_stack_units: u64,
    baseline_anchor: StateAnchor,
    baseline_source_digest: Digest32,
    source_generation: u64,
    baseline_source_counters: [u64; 3],
}

impl ObservedInventoryIncreaseObjective {
    pub const MAX_INCREASE_STACK_UNITS: u64 = 1_000_000;

    pub(super) fn qualified(
        item_type: String,
        baseline_stack_units: u64,
        increase_stack_units: u64,
        baseline_anchor: StateAnchor,
        baseline_source_digest: Digest32,
        source_generation: u64,
        baseline_source_counters: [u64; 3],
    ) -> Result<Self> {
        let target_stack_units = baseline_stack_units
            .checked_add(increase_stack_units)
            .ok_or_else(|| {
                error(ErrorCode::InvalidRequest, "inventory increase target overflows stack units")
            })?;
        if !matches!(item_type.as_str(), "BED" | "DOOR" | "TABLE" | "CHAIR")
            || !(1..=Self::MAX_INCREASE_STACK_UNITS).contains(&increase_stack_units)
            || source_generation == 0
            || source_generation == u64::MAX
            || baseline_source_counters.iter().any(|value| *value > i32::MAX as u64)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "invalid original inventory increase or native source frontier",
            ));
        }
        let value = Self {
            item_type,
            baseline_stack_units,
            increase_stack_units,
            target_stack_units,
            baseline_anchor,
            baseline_source_digest,
            source_generation,
            baseline_source_counters,
        };
        value.predicate().validate_shape()?;
        Ok(value)
    }

    pub fn item_type(&self) -> &str {
        &self.item_type
    }
    pub const fn baseline_stack_units(&self) -> u64 {
        self.baseline_stack_units
    }
    pub const fn increase_stack_units(&self) -> u64 {
        self.increase_stack_units
    }
    pub const fn target_stack_units(&self) -> u64 {
        self.target_stack_units
    }
    pub const fn baseline_anchor(&self) -> StateAnchor {
        self.baseline_anchor
    }
    pub const fn baseline_source_digest(&self) -> Digest32 {
        self.baseline_source_digest
    }
    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }
    /// Operations/1.3 order: next job, next building, next item identity.
    pub const fn baseline_source_counters(&self) -> [u64; 3] {
        self.baseline_source_counters
    }

    pub fn predicate(&self) -> Predicate {
        Predicate::EntityQuantityIncrease {
            kind: EntityKind::Item,
            filter_field: "type_key".into(),
            filter_value: Value::Text(self.item_type.clone()),
            quantity_field: Some("stack_size".into()),
            baseline: self.baseline_stack_units,
            increase: self.increase_stack_units,
            baseline_anchor: self.baseline_anchor,
            baseline_source_digest: self.baseline_source_digest,
            source_generation: self.source_generation,
            baseline_source_counters: self.baseline_source_counters,
        }
    }

    pub fn from_plan(plan: &PreparedPlan) -> Option<Self> {
        super::original(plan).ok()?;
        Self::from_step(plan.steps.first()?, &plan.terminal_condition, plan.anchor)
    }

    pub fn from_selected_plan(plan: &PreparedPlan, step_id: StepId) -> Option<Self> {
        super::selected_original(plan, step_id).ok()?;
        let step = super::selected_step(plan, step_id).ok()?;
        Self::from_step(step, &step.obligation.as_ref()?.terminal, plan.anchor)
    }

    fn from_step(step: &PlanStep, terminal: &Predicate, anchor: StateAnchor) -> Option<Self> {
        let expected_type = output_type(step)?;
        let Predicate::EntityQuantityIncrease {
            kind: EntityKind::Item,
            filter_field,
            filter_value: Value::Text(item_type),
            quantity_field: Some(quantity_field),
            baseline,
            increase,
            baseline_anchor,
            baseline_source_digest,
            source_generation,
            baseline_source_counters,
        } = terminal
        else {
            return None;
        };
        if item_type != expected_type
            || filter_field != "type_key"
            || quantity_field != "stack_size"
            || *baseline_anchor != anchor
        {
            return None;
        }
        let obligation = step.obligation.as_ref()?;
        if obligation.failure.is_some()
            || obligation.terminal.normalized() != *terminal
            || Predicate::All(step.postconditions.clone()).normalized() != *terminal
        {
            return None;
        }
        Self::qualified(
            item_type.clone(),
            *baseline,
            *increase,
            *baseline_anchor,
            *baseline_source_digest,
            *source_generation,
            *baseline_source_counters,
        )
        .ok()
    }

    /// Recheck the sealed source frontier even before the first goal sample.
    /// Both live polling and durable replay use this guard; later source-policy
    /// issuance must still independently qualify software, scope and fields.
    pub fn validate_operations(&self, capture: &LiveOperationsObservation) -> Result<()> {
        capture.validate()?;
        let current = [
            u64::from(capture.jobs.next_job_id),
            u64::from(capture.next_building_id),
            u64::from(capture.next_item_id),
        ];
        if capture.jobs.fortress_id()? != self.baseline_anchor.fortress_id
            || capture.jobs.bridge_generation != self.source_generation
            || !capture.jobs.paused
            || capture.jobs.tick() < self.baseline_anchor.tick
            || current.iter().zip(self.baseline_source_counters).any(|(now, before)| *now < before)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "inventory source changed or regressed below the sealed original baseline",
            ));
        }
        Ok(())
    }
}

/// Recognized original inventory goals. Minimum requests keep their old exact
/// shape and source policy; an increase retains its own typed baseline identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ObservedInventoryGoal {
    Minimum(ObservedInventoryObjective),
    Increase(ObservedInventoryIncreaseObjective),
}

impl ObservedInventoryGoal {
    pub fn from_plan(plan: &PreparedPlan) -> Option<Self> {
        super::original(plan).ok()?;
        Self::from_step(plan.steps.first()?, &plan.terminal_condition, plan.anchor)
    }

    pub fn from_selected_plan(plan: &PreparedPlan, step_id: StepId) -> Option<Self> {
        super::selected_original(plan, step_id).ok()?;
        let step = super::selected_step(plan, step_id).ok()?;
        Self::from_step(step, &step.obligation.as_ref()?.terminal, plan.anchor)
    }

    fn from_step(step: &PlanStep, terminal: &Predicate, anchor: StateAnchor) -> Option<Self> {
        if let Some(minimum) = ObservedInventoryObjective::from_step(step, terminal) {
            return Some(Self::Minimum(minimum));
        }
        ObservedInventoryIncreaseObjective::from_step(step, terminal, anchor).map(Self::Increase)
    }

    pub fn item_type(&self) -> &str {
        match self {
            Self::Minimum(value) => &value.item_type,
            Self::Increase(value) => value.item_type(),
        }
    }

    pub fn target_stack_units(&self) -> u64 {
        match self {
            Self::Minimum(value) => value.min_stack_units,
            Self::Increase(value) => value.target_stack_units(),
        }
    }

    pub fn predicate(&self) -> Predicate {
        match self {
            Self::Minimum(value) => Predicate::EntityQuantity {
                kind: EntityKind::Item,
                filter_field: "type_key".into(),
                filter_value: Value::Text(value.item_type.clone()),
                quantity_field: Some("stack_size".into()),
                op: CompareOp::Ge,
                value: value.min_stack_units,
            },
            Self::Increase(value) => value.predicate(),
        }
    }
}

fn output_type(step: &PlanStep) -> Option<&'static str> {
    let Action::CreateWorkOrder { job_token, .. } = &step.action else {
        return None;
    };
    match job_token.as_str() {
        "CONSTRUCT_BED" | "wooden_bed" => Some("BED"),
        "CONSTRUCT_DOOR" | "wooden_door" => Some("DOOR"),
        "CONSTRUCT_TABLE" | "wooden_table" => Some("TABLE"),
        "CONSTRUCT_CHAIR" | "CONSTRUCT_THRONE" | "wooden_chair" => Some("CHAIR"),
        _ => None,
    }
}
