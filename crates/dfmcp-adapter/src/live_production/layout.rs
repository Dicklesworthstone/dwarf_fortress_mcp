//! Bounded original layout shared by execution consumers and proof owners.
//! Ordinals select private storage slots; they never replace original StepIds.
use dfmcp_intent::Action;
use dfmcp_world::Predicate;
use std::collections::BTreeMap;

use super::*;

/// The complete closed labor/production shape, not current source authority or
/// proof that any original action or goal has succeeded. Legacy assignment
/// layouts retain their existing predicate semantics; the goal composer still
/// checks the entire original terminal before publishing complete-parent proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionLayout {
    archive: PlanArchiveSeal,
    assignment: Option<StepId>,
    orders: Vec<StepId>,
    existing_labor: Option<ExistingLaborRequirement>,
    inventory_minima: Vec<Predicate>,
}

impl ProductionLayout {
    pub const MAX_ORDERS: usize = 4;

    /// Validate the bounded original plan before deriving any layout. Existing
    /// workers require an exact sealed prerequisite; ordinary order-only plans
    /// cannot acquire this production-parent contract by omission.
    pub fn from_plan(plan: &PreparedPlan) -> Result<Self> {
        let archive = policy::validate(plan)?;
        Self::from_validated_plan(plan, archive)
    }

    pub(super) fn from_validated_plan(
        plan: &PreparedPlan,
        archive: PlanArchiveSeal,
    ) -> Result<Self> {
        if plan
            .steps
            .iter()
            .enumerate()
            .any(|(index, step)| step.id.get() as usize != index)
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "production layout requires the actual contiguous original step IDs",
            ));
        }
        if let Some(requirement) = ExistingLaborRequirement::from_plan(plan)? {
            return Ok(Self {
                archive,
                assignment: None,
                orders: plan.steps.iter().map(|step| step.id).collect(),
                inventory_minima: requirement.inventory_minima().to_vec(),
                existing_labor: Some(requirement),
            });
        }
        let first = plan.steps.first().ok_or_else(custody)?;
        if !(2..=Self::MAX_ORDERS + 1).contains(&plan.steps.len())
            || !matches!(first.action, Action::SetLabor { .. })
            || !first.depends_on.is_empty()
            || first.obligation.is_some()
            || plan.steps.iter().skip(1).any(|step| {
                !matches!(step.action, Action::CreateWorkOrder { .. })
                    || step.depends_on != [first.id]
            })
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "production layout requires one labor assignment with dependent orders or exact existing workers with independent orders",
            ));
        }
        Ok(Self {
            archive,
            assignment: Some(first.id),
            orders: plan.steps.iter().skip(1).map(|step| step.id).collect(),
            existing_labor: None,
            inventory_minima: assignment_inventory_minima(plan)?,
        })
    }

    pub const fn assignment_step(&self) -> Option<StepId> {
        self.assignment
    }
    pub const fn archive_seal(&self) -> PlanArchiveSeal {
        self.archive
    }
    pub fn order_steps(&self) -> &[StepId] {
        &self.orders
    }
    /// Zero-based storage ordinal, independent of the original semantic StepId.
    pub fn order_ordinal(&self, step: StepId) -> Option<usize> {
        self.orders.iter().position(|candidate| *candidate == step)
    }
    pub fn existing_labor(&self) -> Option<&ExistingLaborRequirement> {
        self.existing_labor.as_ref()
    }

    /// Complete-parent stock requirements independent of how the original
    /// workers were obtained. These contain no labor effect or source grant.
    /// Every entry is a closed furniture stack-unit minimum, in item-type order.
    pub fn inventory_minima(&self) -> &[Predicate] {
        &self.inventory_minima
    }
}

/// Preserve the original labor and every order requirement before accepting
/// additional stock minima. In particular, a target needing no order cannot
/// disappear from the complete parent when a labor assignment precedes it.
fn assignment_inventory_minima(plan: &PreparedPlan) -> Result<Vec<Predicate>> {
    let required = Predicate::All(
        plan.steps
            .iter()
            .flat_map(|step| step.postconditions.iter().cloned())
            .collect(),
    )
    .normalized();
    let mut parent = Vec::new();
    super::existing_labor::atoms(&plan.terminal_condition, &mut parent);
    let mut required_atoms = Vec::new();
    super::existing_labor::atoms(&required, &mut required_atoms);
    for predicate in required_atoms {
        let at = parent
            .iter()
            .position(|value| *value == predicate)
            .ok_or_else(|| {
                error(
                    ErrorCode::AdapterRejected,
                    "production parent omits an original step requirement",
                )
            })?;
        parent.remove(at);
    }
    let mut minima = BTreeMap::new();
    for predicate in parent {
        let item_type =
            super::existing_labor::inventory_minimum_type(predicate).ok_or_else(|| {
                error(
                    ErrorCode::AdapterRejected,
                    "production parent contains an unsupported stock requirement",
                )
            })?;
        if minima.len() >= ProductionLayout::MAX_ORDERS
            || minima
                .insert(item_type.to_owned(), predicate.clone())
                .is_some()
        {
            return Err(error(
                ErrorCode::AdapterRejected,
                "production parent stock requirements must name at most four distinct furniture types",
            ));
        }
    }
    let minima: Vec<Predicate> = minima.into_values().collect();
    let mut complete = vec![required];
    complete.extend(minima.iter().cloned());
    if plan.terminal_condition != Predicate::All(complete).normalized() {
        return Err(error(
            ErrorCode::AdapterRejected,
            "production parent differs from its complete original requirements",
        ));
    }
    Ok(minima)
}
