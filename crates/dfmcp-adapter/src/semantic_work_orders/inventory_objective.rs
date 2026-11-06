//! Recognize the closed inventory objective retained in an original sealed plan.
//! Recognition chooses a source-policy contract; it grants no source authority
//! and does not attribute observed inventory to the manager order.

use dfmcp_core::StepId;
use dfmcp_intent::{Action, PlanStep, PreparedPlan};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value};

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
        let Action::CreateWorkOrder { job_token, .. } = &step.action else {
            return None;
        };
        let expected_type = match job_token.as_str() {
            "CONSTRUCT_BED" | "wooden_bed" => "BED",
            "CONSTRUCT_DOOR" | "wooden_door" => "DOOR",
            "CONSTRUCT_TABLE" | "wooden_table" => "TABLE",
            "CONSTRUCT_CHAIR" | "CONSTRUCT_THRONE" | "wooden_chair" => "CHAIR",
            _ => return None,
        };
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
