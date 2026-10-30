//! Closed whole-plan policy for native labor and finite work-order sequencing.
use dfmcp_core::{Digest32, EntityId, ErrorCode, Result, StepId};
use dfmcp_intent::plan_archive::{ArchiveLimits, PlanArchiveSeal};
use dfmcp_intent::{Action, PlanStep, PreparedPlan};
use dfmcp_world::{CompareOp, EntityKind, Predicate, Value};

use crate::live_routing::{LiveFamily, RoutedStep, route_step};
use crate::semantic_selection::SelectedSemanticStep;

pub const MAX_STEPS: usize = 16;
pub const MAX_PLAN_BYTES: usize = 64 * 1024;

pub const fn archive_limits() -> ArchiveLimits {
    ArchiveLimits {
        max_bytes: MAX_PLAN_BYTES,
        max_steps: MAX_STEPS,
        max_nodes: 4096,
        max_depth: 16,
    }
}
fn refused(message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(ErrorCode::AdapterRejected, message)
}
fn labor_precondition(predicate: &Predicate, units: &[EntityId]) -> bool {
    match predicate {
        Predicate::True | Predicate::Paused(true) => true,
        Predicate::EntityExists(id) => units.contains(id),
        Predicate::EntityKind {
            entity_id,
            kind: EntityKind::Unit,
        } => units.contains(entity_id),
        Predicate::FieldCompare {
            entity_id,
            field,
            op: CompareOp::Eq,
            ..
        } => {
            units.contains(entity_id) && matches!(field.as_str(), "raw_unit_id" | "native_unit_id")
        }
        Predicate::All(children) => children
            .iter()
            .all(|child| labor_precondition(child, units)),
        _ => false,
    }
}
fn order_precondition(predicate: &Predicate) -> bool {
    match predicate {
        Predicate::True | Predicate::Paused(true) => true,
        Predicate::All(children) => children.iter().all(order_precondition),
        _ => false,
    }
}
/// Only these complete postconditions follow from the strict historical
/// single-labor receipt. They establish no current fact or parent goal.
fn labor_postcondition(
    predicate: &Predicate,
    units: &[EntityId],
    labor: &str,
    enabled: bool,
) -> bool {
    match predicate {
        Predicate::FieldCompare {
            entity_id,
            field,
            op: CompareOp::Eq,
            value: Value::Bool(value),
        } => {
            units.contains(entity_id)
                && field.strip_prefix("labor.") == Some(labor)
                && *value == enabled
        }
        Predicate::All(children) => {
            !children.is_empty()
                && children
                    .iter()
                    .all(|child| labor_postcondition(child, units, labor, enabled))
        }
        _ => false,
    }
}

/// Validate every original step before either family can be contacted. The
/// archive retains all terminal predicates, obligations and dependencies exactly;
/// this executor never interprets an inserted order as their fulfillment.
pub fn validate(plan: &PreparedPlan) -> Result<PlanArchiveSeal> {
    let (_, seal) = dfmcp_intent::plan_archive::encode(plan, archive_limits())?;
    if plan.steps.is_empty() || plan.steps.len() > MAX_STEPS || plan.requires_checkpoint {
        return Err(refused(
            "combined production requires one to sixteen steps and cannot supply a mandatory checkpoint",
        ));
    }
    SelectedSemanticStep::validate(plan, StepId::ZERO)?;
    for step in &plan.steps {
        if step.compensation.is_some()
            || step.preconditions.len() > 64
            || step.postconditions.len() > 64
        {
            return Err(refused(
                "combined production cannot omit compensation or oversized predicate requirements",
            ));
        }
        let has_successor = plan
            .steps
            .iter()
            .any(|candidate| candidate.depends_on.contains(&step.id));
        match &step.action {
            Action::SetLabor {
                units,
                labor,
                enabled,
            } => {
                if units.is_empty()
                    || units.len() > 32
                    || labor.is_empty()
                    || labor.len() > 64
                    || step
                        .preconditions
                        .iter()
                        .any(|p| !labor_precondition(p, units))
                {
                    return Err(refused(
                        "labor preconditions must be protected by its exact selected native capture",
                    ));
                }
                if has_successor
                    && (step.obligation.is_some()
                        || step.postconditions.is_empty()
                        || step
                            .postconditions
                            .iter()
                            .any(|p| !labor_postcondition(p, units, labor, *enabled)))
                {
                    return Err(refused(
                        "labor predecessors require only exact selected labor boolean postconditions and no temporal obligation",
                    ));
                }
            }
            Action::CreateWorkOrder {
                name,
                job_token,
                conditions,
                ..
            } => {
                if has_successor
                    || !conditions.is_empty()
                    || name.len() > 256
                    || job_token.len() > 128
                    || step.preconditions.iter().any(|p| !order_precondition(p))
                {
                    return Err(refused(
                        "finite work orders must be leaves with no conditions and only protected paused-state preconditions",
                    ));
                }
            }
            _ => {
                return Err(refused(
                    "combined production supports only SetLabor and closed finite CreateWorkOrder steps",
                ));
            }
        }
        let routed = route(step)?;
        if !matches!(
            routed.family,
            LiveFamily::WorkforceV1_17 | LiveFamily::WorkOrdersV1_10
        ) {
            return Err(refused(
                "combined production step routes to an unsupported native family",
            ));
        }
    }
    Ok(seal)
}

pub(crate) fn route(step: &PlanStep) -> Result<RoutedStep> {
    route_step(step)
        .outcome
        .map_err(|cause| refused(&cause.reason))
}
pub(crate) fn selected(plan: &PreparedPlan, id: StepId) -> Result<&PlanStep> {
    plan.steps
        .iter()
        .find(|step| step.id == id)
        .ok_or_else(|| refused("selected step is absent from the complete original plan"))
}
pub(crate) fn verify_call(
    original: &PreparedPlan,
    plan: &PreparedPlan,
    step: &PlanStep,
    routed: &RoutedStep,
) -> Result<()> {
    if plan != original || selected(original, step.id)? != step || route(step)? != *routed {
        return Err(refused(
            "executor call changed the complete original plan, selected step or compiled typed route",
        ));
    }
    Ok(())
}
pub(crate) fn nonzero(value: Digest32) -> Result<Digest32> {
    if value == Digest32::ZERO {
        Err(refused("native production evidence identity is reserved"))
    } else {
        Ok(value)
    }
}
