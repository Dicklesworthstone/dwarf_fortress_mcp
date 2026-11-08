//! One bounded stock target derived from an independently qualified census.
//!
//! The result seals its entire starting source, including unrelated rows. It is
//! neither a recurring manager condition nor a resource reservation. The caller
//! must separately exclude competing manager orders and revalidate the original
//! assessment before every effect boundary; this module never reads or mutates
//! a native source and never grants authority from observed field labels.

use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, StateAnchor};
use dfmcp_world::{EntityKind, EvidencePolicy, Value};

use crate::work_orders::{MAX_AMOUNT, WorkOrderRecipe};

use super::inventory_baseline::{
    INVENTORY_BASELINE_QUALIFY_BYTES, InventoryBaselineProjection, authorize,
};
use super::inventory_objective::ObservedInventoryIncreaseObjective;
use super::{Call, error};

const DOMAIN: &[u8] = b"dfmcp-native-stock-target-assessment-v1\0";

/// Immutable, source-qualified sizing for one finite furniture order.
///
/// Counts are observed stack units of the exact item type, regardless of
/// material, usability or origin. An increase records a net inventory goal,
/// not attribution of those goods to the proposed manager order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StockTargetAssessment {
    recipe: WorkOrderRecipe,
    target_stack_units: u64,
    max_order_amount: u32,
    baseline_stack_units: u64,
    deficit_stack_units: u64,
    original_anchor: StateAnchor,
    source_digest: Digest32,
    source_generation: u64,
    source_counters: [u64; 3],
    item_rows: u32,
    objective: Option<ObservedInventoryIncreaseObjective>,
    canonical_bytes: Vec<u8>,
    witness: Digest32,
}

impl StockTargetAssessment {
    pub const MAX_TARGET_STACK_UNITS: u64 = 1_000_000;
    pub const MAX_ORDER_AMOUNT: u32 = MAX_AMOUNT;

    pub(super) fn qualify(
        projection: &InventoryBaselineProjection,
        recipe: WorkOrderRecipe,
        target_stack_units: u64,
        max_order_amount: u32,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<Self> {
        let call = Call::new(context)?;
        call.admit(INVENTORY_BASELINE_QUALIFY_BYTES)?;
        authorize(projection.original_anchor(), &call.current()?)?;
        if projection.snapshot().graph.entities.len() > context.budget.max_entities as usize {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "stock target qualification exceeds the current entity-work bound",
            ));
        }
        if !(1..=Self::MAX_TARGET_STACK_UNITS).contains(&target_stack_units)
            || !(1..=Self::MAX_ORDER_AMOUNT).contains(&max_order_amount)
        {
            return Err(error(
                ErrorCode::InvalidRequest,
                "stock target requires 1..1000000 stack units and a finite order limit of 1..100",
            ));
        }
        let (item_type, producer_type) = recipe_types(recipe);
        let operations = projection.operations();
        let evidence = projection.evidence(policy)?;
        let baseline_stack_units = evidence
            .entity_quantity_total(
                &EntityKind::Item,
                "type_key",
                &Value::Text(item_type.to_owned()),
                Some("stack_size"),
            )?
            .ok_or_else(|| {
                error(
                    ErrorCode::CapabilityDenied,
                    "stock target requires a complete independently qualified starting count",
                )
            })?;
        let deficit_stack_units = target_stack_units.saturating_sub(baseline_stack_units);
        if deficit_stack_units > u64::from(max_order_amount) {
            return Err(error(
                ErrorCode::Conflict,
                "stock target deficit exceeds the original finite order limit; no partial order is proposed",
            ));
        }
        // These raw rows can only refuse a proposed order. Paused/repeating
        // jobs may resume or complete later, and a custom reaction has unknown
        // output. The profile has no manager-order attribution for these jobs.
        // A satisfied target creates no order and needs no overlap exclusion.
        if deficit_stack_units > 0
            && operations.jobs.jobs.iter().any(|job| {
                job.type_key == producer_type
                    || job.type_key == "CustomReaction"
                    || !job.reaction.is_empty()
            })
        {
            return Err(error(
                ErrorCode::Conflict,
                "stock target has an outstanding matching or unknown-output production job",
            ));
        }
        let original_anchor = projection.original_anchor();
        let source_digest = projection.source_digest();
        let source_generation = operations.jobs.bridge_generation;
        let source_counters = [
            u64::from(operations.jobs.next_job_id),
            u64::from(operations.next_building_id),
            u64::from(operations.next_item_id),
        ];
        let item_rows = u32::try_from(operations.items.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "stock target item row count exceeds u32",
            )
        })?;
        let objective = if deficit_stack_units == 0 {
            None
        } else {
            Some(ObservedInventoryIncreaseObjective::qualified(
                item_type.to_owned(),
                baseline_stack_units,
                deficit_stack_units,
                original_anchor,
                source_digest,
                source_generation,
                source_counters,
            )?)
        };
        let mut canonical_bytes = DOMAIN.to_vec();
        canonical_bytes.push(recipe as u8);
        canonical_bytes.extend_from_slice(&target_stack_units.to_be_bytes());
        canonical_bytes.extend_from_slice(&max_order_amount.to_be_bytes());
        canonical_bytes.extend_from_slice(&baseline_stack_units.to_be_bytes());
        canonical_bytes.extend_from_slice(&deficit_stack_units.to_be_bytes());
        canonical_bytes.extend_from_slice(&original_anchor.fortress_id.get().to_be_bytes());
        canonical_bytes.extend_from_slice(&original_anchor.cursor.epoch.to_be_bytes());
        canonical_bytes.extend_from_slice(&original_anchor.cursor.sequence.to_be_bytes());
        canonical_bytes.extend_from_slice(&original_anchor.tick.get().to_be_bytes());
        canonical_bytes.extend_from_slice(original_anchor.state_hash.as_bytes());
        canonical_bytes.extend_from_slice(source_digest.as_bytes());
        canonical_bytes.extend_from_slice(&source_generation.to_be_bytes());
        for counter in source_counters {
            canonical_bytes.extend_from_slice(&counter.to_be_bytes());
        }
        canonical_bytes.extend_from_slice(&item_rows.to_be_bytes());
        let witness = Digest32::of_bytes(&canonical_bytes);
        authorize(original_anchor, &call.current()?)?;
        Ok(Self {
            recipe,
            target_stack_units,
            max_order_amount,
            baseline_stack_units,
            deficit_stack_units,
            original_anchor,
            source_digest,
            source_generation,
            source_counters,
            item_rows,
            objective,
            canonical_bytes,
            witness,
        })
    }

    pub const fn recipe(&self) -> WorkOrderRecipe {
        self.recipe
    }

    pub fn item_type(&self) -> &'static str {
        recipe_types(self.recipe).0
    }

    pub const fn baseline_stack_units(&self) -> u64 {
        self.baseline_stack_units
    }

    pub const fn target_stack_units(&self) -> u64 {
        self.target_stack_units
    }

    pub const fn deficit_stack_units(&self) -> u64 {
        self.deficit_stack_units
    }

    pub const fn max_order_amount(&self) -> u32 {
        self.max_order_amount
    }

    pub fn amount(&self) -> Option<u32> {
        self.objective
            .as_ref()
            .map(|_| self.deficit_stack_units as u32)
    }

    pub fn objective(&self) -> Option<&ObservedInventoryIncreaseObjective> {
        self.objective.as_ref()
    }

    pub const fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }

    pub const fn source_digest(&self) -> Digest32 {
        self.source_digest
    }

    pub const fn source_generation(&self) -> u64 {
        self.source_generation
    }

    /// Native next job, building and item IDs, in that order.
    pub const fn source_counters(&self) -> [u64; 3] {
        self.source_counters
    }

    /// All observed Item rows, including types unrelated to this target.
    pub const fn item_rows(&self) -> u32 {
        self.item_rows
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }

    /// Requalify this exact original paused capture before an effect boundary.
    ///
    /// A changed tick, source row, allocator or canonical anchor refuses even
    /// when the matching count is unchanged. The independent policy must be
    /// reissued for `fresh`; old policy and a matching digest alone are not
    /// authority. This never renews a target, sizes a replacement order, or
    /// treats manager-order queue inspection as part of the Item census.
    pub fn revalidate(
        &self,
        fresh: &InventoryBaselineProjection,
        policy: EvidencePolicy,
        context: &OperationContext,
    ) -> Result<()> {
        let call = Call::new(context)?;
        call.admit(INVENTORY_BASELINE_QUALIFY_BYTES)?;
        authorize(self.original_anchor, &call.current()?)?;
        if fresh.original_anchor() != self.original_anchor
            || fresh.source_digest() != self.source_digest
            || fresh.operations().jobs.tick() != self.original_anchor.tick
            || !fresh.operations().jobs.paused
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "stock target source differs from its sealed original paused census",
            ));
        }
        let qualified = Self::qualify(
            fresh,
            self.recipe,
            self.target_stack_units,
            self.max_order_amount,
            policy,
            &call.current()?,
        )?;
        if qualified.canonical_bytes != self.canonical_bytes {
            return Err(error(
                ErrorCode::StaleAnchor,
                "stock target assessment differs from its immutable original sizing",
            ));
        }
        authorize(self.original_anchor, &call.current()?)
    }
}

fn recipe_types(recipe: WorkOrderRecipe) -> (&'static str, &'static str) {
    match recipe {
        WorkOrderRecipe::WoodenBed => ("BED", "ConstructBed"),
        WorkOrderRecipe::WoodenDoor => ("DOOR", "ConstructDoor"),
        WorkOrderRecipe::WoodenTable => ("TABLE", "ConstructTable"),
        WorkOrderRecipe::WoodenChair => ("CHAIR", "ConstructThrone"),
    }
}

#[cfg(test)]
#[path = "stock_target_tests.rs"]
mod tests;
