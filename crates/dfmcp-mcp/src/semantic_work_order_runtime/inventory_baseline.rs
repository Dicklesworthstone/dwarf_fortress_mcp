//! One independently qualified pre-insertion inventory census for the original
//! request. Its resolved values are sealed in typed predicates, never accepted
//! from client JSON or recomputed when an original plan already exists.
use super::request::Objective;
use super::{MIB, Shell, Work, denied, invalid};
use dfmcp_adapter::semantic_work_orders::{
    INVENTORY_BASELINE_PROJECTION_BYTES, INVENTORY_BASELINE_QUALIFY_BYTES,
    InventoryBaselineProjection, ObservedInventoryIncreaseObjective, OperationsGoalSource,
};
use dfmcp_core::{Capability, GameTick, OperationContext, Result, RiskTier, StateAnchor};
use dfmcp_intent::PreparedPlan;
use std::collections::BTreeMap;

/// One source connection/read, one complete projection, and up to four
/// independently checked recipe quantities. This is logical work, not an eager
/// allocation. The native source retains its unchanged complete-roster limits.
pub(crate) const BASELINE_BYTES: u64 = 33 * MIB
    + INVENTORY_BASELINE_PROJECTION_BYTES
    + 4 * INVENTORY_BASELINE_QUALIFY_BYTES;

#[derive(Clone, Debug, Default)]
pub(crate) struct InventoryBaselines {
    values: BTreeMap<String, ObservedInventoryIncreaseObjective>,
}

impl InventoryBaselines {
    /// Accept an adapter-qualified objective without acquiring another census.
    /// The value's constructor is private to the adapter; request JSON cannot
    /// provide a baseline, source witness or allocation frontier.
    pub(crate) fn from_qualified(value: ObservedInventoryIncreaseObjective) -> Self {
        Self {
            values: BTreeMap::from([(value.item_type().to_owned(), value)]),
        }
    }

    /// Recovery and repetition use only the exact retained original. No source
    /// is consulted and no baseline is silently renewed.
    pub(crate) fn from_original(plan: &PreparedPlan) -> Result<Self> {
        let mut result = Self::default();
        for step in &plan.steps {
            if let Some(value) =
                ObservedInventoryIncreaseObjective::from_selected_plan(plan, step.id)
            {
                if result.values.insert(value.item_type().to_owned(), value).is_some() {
                    return Err(invalid("original repeats an inventory-increase recipe"));
                }
            }
        }
        Ok(result)
    }

    pub(crate) fn get(
        &self,
        item_type: &str,
        increase: u64,
        original: StateAnchor,
    ) -> Result<&ObservedInventoryIncreaseObjective> {
        self.values
            .get(item_type)
            .filter(|value| {
                value.item_type() == item_type
                    && value.increase_stack_units() == increase
                    && value.baseline_anchor() == original
            })
            .ok_or_else(|| {
                denied("inventory increase requires its exact server-qualified original baseline")
            })
    }
}

pub(crate) fn needed(objectives: &[&Objective]) -> bool {
    objectives.iter().any(|objective| {
        matches!(objective, Objective::ObservedInventoryIncrease { .. })
    })
}

/// Consumed native time constrains authority without inventing a new canonical
/// state hash. Reacquisition must not revive a selectively expired Observe grant.
pub(crate) fn authorize_observe(c: &OperationContext, floor: GameTick) -> Result<()> {
    let mut authority = c.clone();
    authority.anchor.tick = authority.anchor.tick.max(floor);
    authority.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    authority.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
}

pub(crate) fn acquire<S: Shell>(
    shell: &S,
    objectives: &[&Objective],
    consumed_tick: &mut GameTick,
    c: &OperationContext,
) -> Result<InventoryBaselines> {
    if objectives.len() > 4 {
        return Err(invalid("inventory baseline supports at most four original objectives"));
    }
    if !needed(objectives) {
        return Ok(InventoryBaselines::default());
    }
    // An earlier failed read may have consumed a newer native tick. Re-observe
    // that actual canonical state before contacting the source again.
    if c.anchor.tick < *consumed_tick {
        return Err(denied("reobserve the canonical source after a newer inventory capture"));
    }
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    c.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    c.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    shell.check_environment()?;
    let mut work = Work::new(c)?;
    let mut source = shell.operations(&work.reserve(16 * MIB)?)?;
    let operations = source.read_operations(&work.reserve(16 * MIB)?)?;
    // Retain a consumed native time even when source pins, pause or exact-tick
    // qualification below refuses. Never relabel the canonical source anchor.
    *consumed_tick = (*consumed_tick).max(operations.jobs.tick());
    let mut authority = work.current()?;
    authority.anchor.tick = authority.anchor.tick.max(*consumed_tick);
    authority.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    authority.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    authority.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    shell.check_environment()?;
    let projection = InventoryBaselineProjection::new(
        c.anchor,
        operations,
        &work.reserve(INVENTORY_BASELINE_PROJECTION_BYTES)?,
    )?;
    let policy = shell.acceptance().baseline(&projection, &work.reserve(MIB)?)?;
    let mut result = InventoryBaselines::default();
    for objective in objectives {
        if let Objective::ObservedInventoryIncrease { item_type, increase_stack_units } = objective {
            let value = projection.increase(
                item_type,
                *increase_stack_units,
                policy.clone(),
                &work.reserve(INVENTORY_BASELINE_QUALIFY_BYTES)?,
            )?;
            if result.values.insert(item_type.clone(), value).is_some() {
                return Err(invalid("inventory baseline repeats an original recipe"));
            }
        }
    }
    shell.check_environment()?;
    work.current()?;
    Ok(result)
}
