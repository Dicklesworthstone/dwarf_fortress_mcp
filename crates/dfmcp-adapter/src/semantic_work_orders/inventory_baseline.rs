//! A source-qualified inventory baseline acquired before original planning.
//!
//! The operations projection is local to this read. Its graph is never substituted
//! for the canonical citizen anchor, and neither the projection nor its observed
//! source labels grant authority. The trusted shell independently qualifies the
//! complete Item census before a count becomes an immutable goal parameter.

use std::collections::BTreeSet;

use dfmcp_core::{
    Capability, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::{
    EntityKind, EvidenceCoverage, EvidencePolicy, EvidenceSource, FactSource,
    PredicateEvidence, Value, WorldSnapshot,
};

use crate::live_operations::{LiveOperationsObservation, LiveOperationsState};
use crate::work_orders::WorkOrderRecipe;

use super::goal_projection::{MAX_GOAL_EDGES, MAX_GOAL_ENTITIES};
use super::inventory_objective::ObservedInventoryIncreaseObjective;
use super::stock_target::StockTargetAssessment;
use super::{Call, error};

pub const INVENTORY_BASELINE_PROJECTION_BYTES: u64 = 128 * 1024 * 1024;
pub const INVENTORY_BASELINE_QUALIFY_BYTES: u64 = 16 * 1024 * 1024;
pub const INVENTORY_TYPE_SOURCE: &str = "operations/1.3.item.type_key";
pub const INVENTORY_QUANTITY_SOURCE: &str = "operations/1.3.item.getStackSize";

/// One complete bounded native census at the exact original canonical tick.
/// The raw capture and its native allocation frontier remain inspectable by
/// the independent policy issuer; no Created receipt or native order is needed.
#[derive(Clone, Debug)]
pub struct InventoryBaselineProjection {
    original_anchor: StateAnchor,
    operations: LiveOperationsObservation,
    snapshot: WorldSnapshot,
    source_digest: Digest32,
}

impl InventoryBaselineProjection {
    pub fn new(
        original_anchor: StateAnchor,
        operations: LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<Self> {
        let call = Call::new(context)?;
        call.admit(INVENTORY_BASELINE_PROJECTION_BYTES)?;
        authorize(original_anchor, &call.current()?)?;
        if context.budget.max_entities < MAX_GOAL_ENTITIES as u32
            || operations
                .jobs
                .jobs
                .len()
                .saturating_add(operations.items.len())
                .saturating_add(operations.buildings.len())
                .saturating_add(2)
                > MAX_GOAL_ENTITIES
            || operations.attachments.len() > MAX_GOAL_EDGES
        {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete inventory baseline exceeds the original-goal projection bound",
            ));
        }
        // Attachments are only one class of projected edge. Count every
        // relation before validation, hashing or graph allocation so the full
        // source cannot exceed its advertised projection allowance.
        let edges = operations
            .attachments
            .len()
            .saturating_add(
                operations
                    .jobs
                    .jobs
                    .iter()
                    .filter(|job| job.holder_native_id.is_some())
                    .count(),
            )
            .saturating_add(
                operations
                    .items
                    .iter()
                    .map(|item| {
                        usize::from(item.container_native_id.is_some())
                            + usize::from(item.holder_building_native_id.is_some())
                    })
                    .sum::<usize>(),
            );
        if edges > MAX_GOAL_EDGES {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete inventory baseline exceeds the total projection edge bound",
            ));
        }
        operations.validate()?;
        if !operations.jobs.paused
            || operations.jobs.tick() != original_anchor.tick
            || operations.jobs.fortress_id()? != original_anchor.fortress_id
            || operations.jobs.bridge_generation == u64::MAX
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "inventory baseline must share the original paused fortress tick",
            ));
        }
        let source_digest = operations.source_digest()?;
        let mut state = LiveOperationsState::default();
        state.publish(operations.clone())?;
        let snapshot = state.into_snapshot().ok_or_else(|| {
            error(ErrorCode::InternalInvariantViolation, "inventory baseline projection is absent")
        })?;
        authorize(original_anchor, &call.current()?)?;
        Ok(Self {
            original_anchor,
            operations,
            snapshot,
            source_digest,
        })
    }

    pub const fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }

    pub fn operations(&self) -> &LiveOperationsObservation {
        &self.operations
    }

    /// Source-local projection used only to qualify the complete count. Its
    /// cursor/hash are not a new shared canonical observation or plan anchor.
    pub fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }

    pub const fn source_digest(&self) -> Digest32 {
        self.source_digest
    }

    pub fn observed_sources(&self) -> BTreeSet<EvidenceSource> {
        self.snapshot
            .graph
            .entities
            .values()
            .filter(|entity| entity.kind == EntityKind::Item)
            .flat_map(|entity| entity.fields.values())
            .filter_map(|fact| match &fact.source {
                FactSource::DfhackField(field)
                    if matches!(field.as_str(), INVENTORY_TYPE_SOURCE | INVENTORY_QUANTITY_SOURCE) =>
                {
                    Some(EvidenceSource::Observed {
                        field: field.clone(),
                        source_digest: fact.source_digest,
                    })
                }
                _ => None,
            })
            .collect()
    }

    pub fn evidence(&self, policy: EvidencePolicy) -> Result<PredicateEvidence<'_>> {
        let available = self.observed_sources();
        if policy.all_entities != EvidenceCoverage::Unknown
            || policy.all_edges != EvidenceCoverage::Unknown
            || !policy.edge_kinds.is_empty()
            || !policy.terrain_regions.is_empty()
            || policy.entity_kinds.keys().any(|kind| *kind != EntityKind::Item)
            || policy.sources.iter().any(|source| !available.contains(source))
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "inventory baseline policy exceeds the complete Item type and stack census",
            ));
        }
        PredicateEvidence::scoped(&self.snapshot, policy)
    }

    /// Derive one finite furniture deficit from this independently qualified
    /// complete census. A satisfied target has no order or increase objective;
    /// an oversized deficit refuses instead of becoming a truncated order.
    pub fn stock_target(
        &self,
        recipe: WorkOrderRecipe,
        target_stack_units: u64,
        max_order_amount: u32,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<StockTargetAssessment> {
        StockTargetAssessment::qualify(
            self,
            recipe,
            target_stack_units,
            max_order_amount,
            policy,
            context,
        )
    }

    /// Seal an exact qualified starting quantity, never a caller-supplied count.
    /// A missing field or incomplete domain is a refused baseline, not zero.
    pub fn increase(
        &self,
        item_type: &str,
        increase_stack_units: u64,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<ObservedInventoryIncreaseObjective> {
        let call = Call::new(context)?;
        call.admit(INVENTORY_BASELINE_QUALIFY_BYTES)?;
        authorize(self.original_anchor, &call.current()?)?;
        if self.snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "inventory baseline qualification exceeds the current entity-work bound",
            ));
        }
        if !matches!(item_type, "BED" | "DOOR" | "TABLE" | "CHAIR")
            || !(1..=ObservedInventoryIncreaseObjective::MAX_INCREASE_STACK_UNITS)
                .contains(&increase_stack_units)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "inventory increase requires one furniture type and 1..1000000 stack units",
            ));
        }
        let evidence = self.evidence(policy)?;
        let baseline = evidence
            .entity_quantity_total(
                &EntityKind::Item,
                "type_key",
                &Value::Text(item_type.to_owned()),
                Some("stack_size"),
            )?
            .ok_or_else(|| {
                error(
                    ErrorCode::CapabilityDenied,
                    "inventory increase requires a complete independently qualified starting count",
                )
            })?;
        let objective = ObservedInventoryIncreaseObjective::qualified(
            item_type.to_owned(),
            baseline,
            increase_stack_units,
            self.original_anchor,
            self.source_digest,
            self.operations.jobs.bridge_generation,
            [
                u64::from(self.operations.jobs.next_job_id),
                u64::from(self.operations.next_building_id),
                u64::from(self.operations.next_item_id),
            ],
        )?;
        authorize(self.original_anchor, &call.current()?)?;
        Ok(objective)
    }
}

pub(super) fn authorize(original: StateAnchor, context: &OperationContext) -> Result<()> {
    if context.anchor != original {
        return Err(error(
            ErrorCode::StaleAnchor,
            "inventory baseline belongs to another original canonical anchor",
        ));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)
}

#[cfg(test)]
#[path = "inventory_baseline_tests.rs"]
mod tests;
