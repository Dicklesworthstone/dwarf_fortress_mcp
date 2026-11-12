//! Immutable identity for one bounded, census-derived native stock target.
//! This is combined-host dispatch policy, not canonical workshop feasibility,
//! stock usability, causal output, or an instruction to maintain stock forever.
use super::prerequisite_binding;
use super::request::recipe;
use crate::semantic_workforce_runtime::{digest, invalid};
use dfmcp_adapter::live_production::{ExistingLaborRequirement, ProductionLayout};
use dfmcp_adapter::semantic_work_orders::{
    ObservedInventoryIncreaseObjective, StockTargetAssessment,
};
use dfmcp_adapter::work_orders::WorkOrderRecipe;
use dfmcp_core::{Digest32, GameTick, IntentId, Result, StepId};
use dfmcp_intent::{Action, PreparedPlan, derive_step_idempotency_key};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value};

const RESERVED: &str = "Stock target";
const VERSION: &str = "dfmcp-stock-target";
const PREFIX: &str = "Stock target/1 ";
const ASSIGNMENT_PREFIX: &str = "Stock target/2 ";
const ASSIGNMENT_ACTION_TAG: &str = "stock-target/";
const IDENTITY_DOMAIN: &[u8] = b"dfmcp-native-combined-stock-target/1\0";
const ASSIGNMENT_IDENTITY_DOMAIN: &[u8] = b"dfmcp-native-assigned-stock-target/2\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StockTargetBinding {
    objective: ObservedInventoryIncreaseObjective,
    recipe: WorkOrderRecipe,
    max_order_amount: u32,
    witness: Digest32,
    assign_labor: bool,
}

impl StockTargetBinding {
    pub(super) fn from_assessment(assessment: &StockTargetAssessment) -> Result<Self> {
        let objective = assessment
            .objective()
            .ok_or_else(|| invalid("a satisfied stock target must not compile an original"))?
            .clone();
        let value = Self {
            objective,
            recipe: assessment.recipe(),
            max_order_amount: assessment.max_order_amount(),
            witness: assessment.witness(),
            assign_labor: false,
        };
        value.validate()?;
        if assessment.amount().map(u64::from) != Some(value.objective.increase_stack_units())
            || assessment.target_stack_units() != value.objective.target_stack_units()
            || assessment.original_anchor() != value.objective.baseline_anchor()
        {
            return Err(invalid(
                "stock target assessment differs from its qualified baseline",
            ));
        }
        Ok(value)
    }

    pub(super) fn from_assessment_with_assignment(
        assessment: &StockTargetAssessment,
    ) -> Result<Self> {
        let mut value = Self::from_assessment(assessment)?;
        value.assign_labor = true;
        Ok(value)
    }

    pub(super) const fn assign_labor(&self) -> bool {
        self.assign_labor
    }

    pub(super) fn order_name(&self) -> String {
        if self.assign_labor {
            format!("native stock-target/2 {} goal", self.recipe.as_str())
        } else {
            format!("native existing-labor {} goal", self.recipe.as_str())
        }
    }

    pub(super) fn objective(&self) -> &ObservedInventoryIncreaseObjective {
        &self.objective
    }
    pub(super) const fn recipe(&self) -> WorkOrderRecipe {
        self.recipe
    }
    pub(super) const fn max_order_amount(&self) -> u32 {
        self.max_order_amount
    }
    pub(super) const fn witness(&self) -> Digest32 {
        self.witness
    }

    fn validate(&self) -> Result<()> {
        let item_type = match self.recipe {
            WorkOrderRecipe::WoodenBed => "BED",
            WorkOrderRecipe::WoodenDoor => "DOOR",
            WorkOrderRecipe::WoodenTable => "TABLE",
            WorkOrderRecipe::WoodenChair => "CHAIR",
        };
        if !(1..=100).contains(&self.max_order_amount)
            || !(1..=1_000_000).contains(&self.objective.target_stack_units())
            || self.objective.increase_stack_units() == 0
            || self.objective.increase_stack_units() > u64::from(self.max_order_amount)
            || self.objective.item_type() != item_type
            || self.witness == Digest32::ZERO
        {
            return Err(invalid("invalid finite stock target identity or bound"));
        }
        self.objective.predicate().validate_shape()
    }

    fn identity(
        &self,
        workers: &ExistingLaborRequirement,
        deadline: GameTick,
        cadence: u64,
        stability: u32,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        if self.assign_labor {
            return Err(invalid(
                "assigned stock target cannot use existing-worker identity",
            ));
        }
        let prerequisite = workers.predicate().canonical_bytes();
        let predicate = self.objective.predicate().canonical_bytes();
        let mut bytes = IDENTITY_DOMAIN.to_vec();
        bytes.extend_from_slice(&(prerequisite.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&prerequisite);
        bytes.extend_from_slice(&(self.recipe as u32).to_be_bytes());
        bytes.extend_from_slice(&self.objective.target_stack_units().to_be_bytes());
        bytes.extend_from_slice(&self.max_order_amount.to_be_bytes());
        bytes.extend_from_slice(self.witness.as_bytes());
        bytes.extend_from_slice(&(predicate.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&predicate);
        bytes.extend_from_slice(&deadline.get().to_be_bytes());
        bytes.extend_from_slice(&cadence.to_be_bytes());
        bytes.extend_from_slice(&stability.to_be_bytes());
        Ok(bytes)
    }

    fn assignment_identity(
        &self,
        assignment: &Action,
        preconditions: &Predicate,
        deadline: GameTick,
        cadence: u64,
        stability: u32,
    ) -> Result<Vec<u8>> {
        self.validate()?;
        if !self.assign_labor {
            return Err(invalid(
                "existing-worker stock target cannot acquire assignment authority",
            ));
        }
        assignment_goal(assignment, preconditions)?;
        let action = assignment.canonical_bytes();
        let prerequisite = preconditions.normalized().canonical_bytes();
        let predicate = self.objective.predicate().canonical_bytes();
        let mut bytes = ASSIGNMENT_IDENTITY_DOMAIN.to_vec();
        bytes.extend_from_slice(&(action.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&action);
        bytes.extend_from_slice(&(prerequisite.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&prerequisite);
        bytes.extend_from_slice(&(self.recipe as u32).to_be_bytes());
        bytes.extend_from_slice(&self.objective.target_stack_units().to_be_bytes());
        bytes.extend_from_slice(&self.max_order_amount.to_be_bytes());
        bytes.extend_from_slice(self.witness.as_bytes());
        bytes.extend_from_slice(&(predicate.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&predicate);
        bytes.extend_from_slice(&deadline.get().to_be_bytes());
        bytes.extend_from_slice(&cadence.to_be_bytes());
        bytes.extend_from_slice(&stability.to_be_bytes());
        Ok(bytes)
    }
}

/// Validate the complete canonical citizen identity guard, not just whichever
/// subset a malformed labor postcondition happens to mention. The original
/// anchor and archive still supply source authority and custody separately.
pub(super) fn assignment_goal(assignment: &Action, preconditions: &Predicate) -> Result<Predicate> {
    let Action::SetLabor {
        units,
        labor,
        enabled: true,
    } = assignment
    else {
        return Err(invalid(
            "stock target requires one enabling labor assignment",
        ));
    };
    if units.is_empty()
        || units.len() > 32
        || units.iter().any(|unit| unit.get() == 0)
        || units.windows(2).any(|pair| pair[0] >= pair[1])
        || labor.is_empty()
        || labor.len() > 64
        || !labor
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(invalid(
            "invalid assigned stock-target citizen or labor selection",
        ));
    }
    preconditions.validate_shape()?;
    let normalized = preconditions.normalized();
    let Predicate::All(children) = &normalized else {
        return Err(invalid(
            "assigned stock target lacks complete identity preconditions",
        ));
    };
    let mut expected = vec![Predicate::Paused(true)];
    let mut goals = Vec::with_capacity(units.len());
    for unit in units {
        let native = children
            .iter()
            .find_map(|child| match child {
                Predicate::FieldCompare {
                    entity_id,
                    field,
                    op: CompareOp::Eq,
                    value: Value::I64(native),
                } if entity_id == unit && field == "raw_unit_id" && *native >= 0 => Some(*native),
                _ => None,
            })
            .ok_or_else(|| {
                invalid("assigned stock target lacks an original native citizen identity")
            })?;
        expected.push(Predicate::EntityExists(*unit));
        expected.push(Predicate::EntityKind {
            entity_id: *unit,
            kind: EntityKind::Unit,
        });
        expected.push(Predicate::FieldCompare {
            entity_id: *unit,
            field: "raw_unit_id".into(),
            op: CompareOp::Eq,
            value: Value::I64(native),
        });
        goals.push(Predicate::FieldCompare {
            entity_id: *unit,
            field: format!("labor.{labor}"),
            op: CompareOp::Eq,
            value: Value::Bool(true),
        });
    }
    if normalized != Predicate::All(expected).normalized() {
        return Err(invalid(
            "assigned stock target changed its complete identity preconditions",
        ));
    }
    Ok(Predicate::All(goals).normalized())
}

/// None preserves all legacy identity bytes. This runs before the optional
/// Workshop identity and before deriving any intent, step key or created ID.
pub(super) fn append_identity(
    bytes: &mut Vec<u8>,
    binding: Option<&StockTargetBinding>,
    workers: &ExistingLaborRequirement,
    deadline: GameTick,
    cadence: u64,
    stability: u32,
) -> Result<()> {
    if let Some(binding) = binding {
        *bytes = binding.identity(workers, deadline, cadence, stability)?;
    }
    Ok(())
}

/// Assignment is a separate identity and explicit action, never an upgrade of
/// a version-one existing-worker original. The typed action and its complete
/// canonical identity predicates are sealed before deriving either step key.
pub(super) fn append_assignment_identity(
    bytes: &mut Vec<u8>,
    binding: Option<&StockTargetBinding>,
    assignment: &Action,
    preconditions: &Predicate,
    deadline: GameTick,
    cadence: u64,
    stability: u32,
) -> Result<()> {
    if let Some(binding) = binding {
        *bytes =
            binding.assignment_identity(assignment, preconditions, deadline, cadence, stability)?;
    }
    Ok(())
}

pub(super) fn summary(summary: String, binding: Option<&StockTargetBinding>) -> Result<String> {
    match binding {
        None => Ok(summary),
        Some(binding) => {
            binding.validate()?;
            if summary.is_empty() || summary.contains(RESERVED) || summary.contains(VERSION) {
                return Err(invalid("invalid or repeated stock target marker"));
            }
            let prefix = if binding.assign_labor {
                ASSIGNMENT_PREFIX
            } else {
                PREFIX
            };
            Ok(format!(
                "{prefix}{}>={} max={} {}",
                binding.recipe.as_str(),
                binding.objective.target_stack_units(),
                binding.max_order_amount,
                binding.witness.to_hex(),
            ))
        }
    }
}

/// A marked original cannot silently become a manual inventory-growth request
/// during recovery. The exact typed objective and finite action must agree with
/// the marker, and their original intent/key must carry this identity domain.
pub(super) fn requirement(plan: &PreparedPlan) -> Result<Option<StockTargetBinding>> {
    plan.validate_structure()?;
    if !plan.digest_is_valid() || !plan.id_is_valid() {
        return Err(invalid("stock target requires the sealed original plan"));
    }
    let origins = prerequisite_binding::requirements(plan)?;
    let body = prerequisite_binding::body(plan)?;
    let Some(marker) = parse_summary(body)? else {
        if plan.steps.iter().any(|step| {
            matches!(&step.action, Action::CreateWorkOrder { name, .. }
                if name.contains(ASSIGNMENT_ACTION_TAG))
        }) {
            return Err(invalid(
                "assigned stock target lost its required original marker",
            ));
        }
        return Ok(None);
    };
    let layout = ProductionLayout::from_plan(plan)?;
    let step_id = if marker.assign_labor {
        if layout.assignment_step() != Some(StepId::ZERO)
            || layout.order_steps() != [StepId::new(1)]
            || layout.existing_labor().is_some()
        {
            return Err(invalid(
                "assigned stock target requires one assignment and one dependent original order",
            ));
        }
        StepId::new(1)
    } else {
        if layout.assignment_step().is_some() || layout.order_steps() != [StepId::ZERO] {
            return Err(invalid(
                "stock target requires exactly one original independent order",
            ));
        }
        StepId::ZERO
    };
    let step = plan
        .steps
        .get(step_id.get() as usize)
        .ok_or_else(|| invalid("stock target original order is absent"))?;
    let objective = ObservedInventoryIncreaseObjective::from_selected_plan(plan, step_id)
        .ok_or_else(|| invalid("stock target requires its typed original inventory increase"))?;
    let binding = StockTargetBinding {
        objective,
        recipe: marker.recipe,
        max_order_amount: marker.max_order_amount,
        witness: marker.witness,
        assign_labor: marker.assign_labor,
    };
    binding.validate()?;
    let (_, expected_job) = recipe(binding.recipe.as_str())?;
    let Action::CreateWorkOrder {
        name,
        job_token,
        amount,
        conditions,
    } = &step.action
    else {
        return Err(invalid("stock target original lacks a finite order"));
    };
    if binding.objective.target_stack_units() != marker.target
        || binding.objective.baseline_anchor() != plan.anchor
        || binding.objective.increase_stack_units() != u64::from(*amount)
        || *amount > binding.max_order_amount
        || job_token != expected_job
        || name != &binding.order_name()
        || !conditions.is_empty()
    {
        return Err(invalid(
            "stock target marker differs from the exact original action or goal",
        ));
    }
    let obligation = step
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("stock target original lacks its temporal obligation"))?;
    let mut identity = if binding.assign_labor {
        let assignment = &plan.steps[0];
        let preconditions = Predicate::All(assignment.preconditions.clone()).normalized();
        let labor_goal = assignment_goal(&assignment.action, &preconditions)?;
        if Predicate::All(step.preconditions.clone()).normalized() != Predicate::Paused(true)
            || Predicate::All(assignment.postconditions.clone()).normalized() != labor_goal
            || plan.terminal_condition.normalized()
                != Predicate::All(vec![labor_goal, binding.objective.predicate()]).normalized()
        {
            return Err(invalid(
                "assigned stock target changed its complete labor and inventory goal",
            ));
        }
        binding.assignment_identity(
            &assignment.action,
            &preconditions,
            obligation.deadline_tick,
            obligation.poll_interval_ticks,
            obligation.stable_for_observations,
        )?
    } else {
        let labor = layout
            .existing_labor()
            .ok_or_else(|| invalid("stock target requires original existing workers"))?;
        binding.identity(
            labor,
            obligation.deadline_tick,
            obligation.poll_interval_ticks,
            obligation.stable_for_observations,
        )?
    };
    prerequisite_binding::append_identity(&mut identity, origins)?;
    let hash = Digest32::of_bytes(&identity);
    let intent = IntentId::new(u128::from_be_bytes(
        hash.as_bytes()[..16]
            .try_into()
            .map_err(|_| invalid("stock target identity width"))?,
    ));
    if intent == IntentId::NIL
        || intent != plan.intent_id
        || plan.steps.iter().any(|selected| {
            selected.idempotency_key
                != derive_step_idempotency_key(intent, plan.anchor, selected.id, &selected.action)
        })
    {
        return Err(invalid(
            "stock target marker does not match its original identity domain",
        ));
    }
    Ok(Some(binding))
}

struct Marker {
    recipe: WorkOrderRecipe,
    target: u64,
    max_order_amount: u32,
    witness: Digest32,
    assign_labor: bool,
}

fn parse_summary(summary: &str) -> Result<Option<Marker>> {
    if !summary.contains(RESERVED) && !summary.contains(VERSION) {
        return Ok(None);
    }
    let (fields, assign_labor) = if let Some(fields) = summary.strip_prefix(PREFIX) {
        (fields, false)
    } else if let Some(fields) = summary.strip_prefix(ASSIGNMENT_PREFIX) {
        (fields, true)
    } else {
        return Err(invalid("unsupported or misplaced stock target marker"));
    };
    let values: Vec<_> = fields.split(' ').collect();
    if values.len() != 3 {
        return Err(invalid("stock target marker must retain all exact fields"));
    }
    let field = |index: usize, name: &str| -> Result<&str> {
        values[index]
            .strip_prefix(name)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| invalid("unsupported stock target marker field"))
    };
    let (recipe_raw, target_raw) = values[0]
        .split_once(">=")
        .ok_or_else(|| invalid("stock target marker lacks its recipe and target"))?;
    let (recipe, _) = recipe(recipe_raw)?;
    let target = target_raw
        .parse::<u64>()
        .ok()
        .filter(|value| (1..=1_000_000).contains(value) && value.to_string() == target_raw)
        .ok_or_else(|| invalid("stock target marker has an invalid target"))?;
    let max_raw = field(1, "max=")?;
    let max_order_amount = max_raw
        .parse::<u32>()
        .ok()
        .filter(|value| (1..=100).contains(value) && value.to_string() == max_raw)
        .ok_or_else(|| invalid("stock target marker has an invalid order bound"))?;
    Ok(Some(Marker {
        recipe,
        target,
        max_order_amount,
        witness: digest(values[2])?,
        assign_labor,
    }))
}

#[cfg(test)]
#[path = "stock_target_binding_tests.rs"]
mod tests;
