//! Source-qualified conservative input supply for one original furniture plan.
//!
//! This closed host policy requires one observed eligible WOOD stack unit per
//! requested furniture unit, plus an optional retained reserve. It is not DF's
//! native ingredient language, a reservation, path/access evidence, or a promise
//! that production will succeed. The complete original demand is retained after
//! order insertion: a Created receipt does not consume the order's input demand.
//!
//! A trusted embedding constructs source pins independently of captured fields.
//! Qualification borrows one complete Operations/1.3 capture, including the same
//! census used by stock sizing, and reuses the conservative inventory allocator
//! without allocating another canonical graph. Read grants never become mutation
//! permission. Callers retain every consumed native frontier across failed reads
//! and requalify at each protected effect boundary; a prior assessment is history.

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, StateAnchor};
use dfmcp_world::inventory_allocation::Allocation;

use crate::live_operations::LiveOperationsObservation;
use crate::operations_analysis::{
    MAX_ANALYSIS_WORK, MaterialDemand, SUPPLY_POLICY, plan_raw_inventory,
};
use crate::work_orders::{WorkOrderRecipe, WorkOrderSpec};

use super::inventory_baseline::authorize;
use super::{Call, error};

pub const FURNITURE_INPUT_POLICY: &str = "dfmcp.furniture-wood-input-requirement/1";
/// Complete raw validation, ancestry, hashing and joint allocation; no second
/// 128 MiB world projection is constructed or charged by this assessment.
pub const INPUT_SUPPLY_QUALIFY_BYTES: u64 = 16 * 1024 * 1024;
pub const INPUT_SUPPLY_MAX_ENTITIES: usize = 4096;
pub const INPUT_SUPPLY_MAX_EDGES: usize = 8192;
pub const INPUT_SUPPLY_MAX_RESERVE_STACK_UNITS: u64 = 1_000_000;

const REQUIREMENT_DOMAIN: &[u8] = b"dfmcp-furniture-input-requirements-v1\0";
const SOURCE_DOMAIN: &[u8] = b"dfmcp-furniture-input-source-v1\0";
const ASSESSMENT_DOMAIN: &[u8] = b"dfmcp-furniture-input-assessment-v1\0";

fn invalid(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::InvalidRequest, message)
}

fn bounded_text(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.contains('\0')
}

fn append_text(bytes: &mut Vec<u8>, text: &str) {
    bytes.extend_from_slice(&(text.len() as u64).to_be_bytes());
    bytes.extend_from_slice(text.as_bytes());
}

fn append_anchor(bytes: &mut Vec<u8>, anchor: StateAnchor) {
    bytes.extend_from_slice(&anchor.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&anchor.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&anchor.tick.get().to_be_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
}

/// Immutable demand for every original finite order, in closed recipe order.
/// Input order does not change the requirement; duplicate recipes are refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FurnitureInputRequirements {
    orders: Vec<WorkOrderSpec>,
    reserve_stack_units: u64,
    order_stack_units: u64,
    required_stack_units: u64,
    canonical_bytes: Vec<u8>,
    witness: Digest32,
}

impl FurnitureInputRequirements {
    pub fn new(orders: &[WorkOrderSpec], reserve_stack_units: u64) -> Result<Self> {
        if orders.is_empty()
            || orders.len() > 4
            || reserve_stack_units > INPUT_SUPPLY_MAX_RESERVE_STACK_UNITS
        {
            return Err(invalid(
                "furniture inputs require one to four distinct finite recipes and at most 1000000 reserve units",
            ));
        }
        let mut orders = orders.to_vec();
        orders.sort_by_key(|order| order.recipe() as u8);
        if orders
            .windows(2)
            .any(|pair| pair[0].recipe() == pair[1].recipe())
        {
            return Err(invalid("furniture input recipes must be distinct"));
        }
        let mut order_stack_units = 0u64;
        for order in &orders {
            // WorkOrderSpec is private and validated; repeat its closed bounds
            // so this requirement cannot silently widen with a future catalog.
            match order.recipe() {
                WorkOrderRecipe::WoodenBed
                | WorkOrderRecipe::WoodenDoor
                | WorkOrderRecipe::WoodenTable
                | WorkOrderRecipe::WoodenChair => {}
            }
            WorkOrderSpec::new(order.recipe(), order.amount())?;
            order_stack_units = order_stack_units
                .checked_add(u64::from(order.amount()))
                .ok_or_else(|| invalid("furniture input demand overflow"))?;
        }
        let required_stack_units = order_stack_units
            .checked_add(reserve_stack_units)
            .ok_or_else(|| invalid("furniture input reserve overflow"))?;
        let mut canonical_bytes = REQUIREMENT_DOMAIN.to_vec();
        append_text(&mut canonical_bytes, FURNITURE_INPUT_POLICY);
        append_text(&mut canonical_bytes, SUPPLY_POLICY);
        canonical_bytes.extend_from_slice(&(orders.len() as u32).to_be_bytes());
        for order in &orders {
            canonical_bytes.push(order.recipe() as u8);
            canonical_bytes.extend_from_slice(&order.amount().to_be_bytes());
        }
        canonical_bytes.extend_from_slice(&reserve_stack_units.to_be_bytes());
        canonical_bytes.extend_from_slice(&required_stack_units.to_be_bytes());
        let witness = Digest32::of_bytes(&canonical_bytes);
        Ok(Self {
            orders,
            reserve_stack_units,
            order_stack_units,
            required_stack_units,
            canonical_bytes,
            witness,
        })
    }

    pub fn orders(&self) -> &[WorkOrderSpec] {
        &self.orders
    }

    pub const fn reserve_stack_units(&self) -> u64 {
        self.reserve_stack_units
    }

    pub const fn order_stack_units(&self) -> u64 {
        self.order_stack_units
    }

    pub const fn required_stack_units(&self) -> u64 {
        self.required_stack_units
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }

    fn demands(&self) -> Vec<MaterialDemand> {
        let mut demands: Vec<_> = self
            .orders
            .iter()
            .map(|order| MaterialDemand {
                key: order.recipe().as_str().to_owned(),
                units: u64::from(order.amount()),
                item_types: vec!["WOOD".to_owned()],
                subtype: None,
                material_type: None,
                material_index: None,
            })
            .collect();
        if self.reserve_stack_units > 0 {
            demands.push(MaterialDemand {
                key: "retained_wood_reserve".to_owned(),
                units: self.reserve_stack_units,
                item_types: vec!["WOOD".to_owned()],
                subtype: None,
                material_type: None,
                material_index: None,
            });
        }
        demands
    }
}

/// Exact independent Operations/1.3 pins supplied by the trusted owner. Neither
/// captured field labels nor a client-provided query result constructs this
/// policy inside the combined host. This type issues read qualification only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSupplySource {
    world_folder: String,
    site_id: u32,
    df_version: String,
    dfhack_version: String,
    generation: u64,
    witness: Digest32,
}

impl InputSupplySource {
    pub fn new(
        world_folder: String,
        site_id: u32,
        df_version: String,
        dfhack_version: String,
        generation: u64,
    ) -> Result<Self> {
        if !bounded_text(&world_folder, 512)
            || site_id > i32::MAX as u32
            || !bounded_text(&df_version, 128)
            || !bounded_text(&dfhack_version, 128)
            || generation == 0
            || generation == u64::MAX
        {
            return Err(invalid(
                "invalid independently pinned furniture input source",
            ));
        }
        let mut bytes = SOURCE_DOMAIN.to_vec();
        append_text(&mut bytes, "operations/1.3");
        append_text(&mut bytes, &world_folder);
        bytes.extend_from_slice(&site_id.to_be_bytes());
        append_text(&mut bytes, &df_version);
        append_text(&mut bytes, &dfhack_version);
        bytes.extend_from_slice(&generation.to_be_bytes());
        Ok(Self {
            world_folder,
            site_id,
            df_version,
            dfhack_version,
            generation,
            witness: Digest32::of_bytes(&bytes),
        })
    }

    /// A constant-size identity comparison, not validation or readiness. Native
    /// readers may use it to retain decoded allocator/tick floors before later
    /// tick, authority, budget or input-availability checks reject a capture.
    pub fn matches_capture_identity(&self, operations: &LiveOperationsObservation) -> bool {
        let source = &operations.jobs;
        source.world_folder == self.world_folder
            && source.site_id == self.site_id as i32
            && source.df_version == self.df_version
            && source.dfhack_version == self.dfhack_version
            && source.bridge_generation == self.generation
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }
}

/// Immutable historical assessment. `ready` means that this exact capture's
/// conservative subset satisfies the complete host requirement. It grants no
/// native reservation or effect permission, and is not current after the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSupplyAssessment {
    requirements: FurnitureInputRequirements,
    original_anchor: StateAnchor,
    source_witness: Digest32,
    source_digest: Digest32,
    source_generation: u64,
    source_counters: [u64; 3],
    item_rows: u32,
    eligible_items: u64,
    eligible_stack_units: u64,
    demands: Vec<MaterialDemand>,
    allocation: Allocation,
    excluded_items: BTreeMap<&'static str, u64>,
    unmatched_items: u64,
    work_units: u64,
    canonical_bytes: Vec<u8>,
    witness: Digest32,
}

impl InputSupplyAssessment {
    pub fn qualify(
        requirements: &FurnitureInputRequirements,
        source: &InputSupplySource,
        original_anchor: StateAnchor,
        operations: &LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<Self> {
        let call = Call::new(context)?;
        call.admit(INPUT_SUPPLY_QUALIFY_BYTES)?;
        authorize(original_anchor, &call.current()?)?;
        source_bounds(operations, context)?;
        if !source.matches_capture_identity(operations) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture input capture differs from its independently pinned source",
            ));
        }
        if !operations.jobs.paused
            || operations.jobs.tick() != original_anchor.tick
            || operations.jobs.fortress_id()? != original_anchor.fortress_id
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "furniture input supply requires the exact original paused fortress tick",
            ));
        }
        let analysis = plan_raw_inventory(
            operations,
            &call.current()?,
            &requirements.demands(),
            MAX_ANALYSIS_WORK,
        )?;
        let source_digest = operations.source_digest()?;
        let source_generation = operations.jobs.bridge_generation;
        let source_counters = [
            u64::from(operations.jobs.next_job_id),
            u64::from(operations.next_building_id),
            u64::from(operations.next_item_id),
        ];
        let item_rows = u32::try_from(operations.items.len())
            .map_err(|_| error(ErrorCode::BudgetExceeded, "input item count exceeds u32"))?;
        if analysis.allocation.requested_units != requirements.required_stack_units {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "joint input allocation lost an original order or retained reserve",
            ));
        }
        let mut canonical_bytes = ASSESSMENT_DOMAIN.to_vec();
        canonical_bytes.extend_from_slice(requirements.witness.as_bytes());
        canonical_bytes.extend_from_slice(source.witness.as_bytes());
        append_anchor(&mut canonical_bytes, original_anchor);
        canonical_bytes.extend_from_slice(source_digest.as_bytes());
        canonical_bytes.extend_from_slice(&source_generation.to_be_bytes());
        for counter in source_counters {
            canonical_bytes.extend_from_slice(&counter.to_be_bytes());
        }
        canonical_bytes.extend_from_slice(&item_rows.to_be_bytes());
        canonical_bytes.extend_from_slice(&analysis.candidate_items.to_be_bytes());
        canonical_bytes.extend_from_slice(&analysis.candidate_stack_units.to_be_bytes());
        canonical_bytes.extend_from_slice(&analysis.allocation.requested_units.to_be_bytes());
        canonical_bytes.extend_from_slice(&analysis.allocation.allocated_units.to_be_bytes());
        // The full raw source and both versioned policies commit to every input
        // of the deterministic allocator. Its assignment vector is not copied
        // into this small original/assessment commitment.
        let witness = Digest32::of_bytes(&canonical_bytes);
        authorize(original_anchor, &call.current()?)?;
        Ok(Self {
            requirements: requirements.clone(),
            original_anchor,
            source_witness: source.witness,
            source_digest,
            source_generation,
            source_counters,
            item_rows,
            eligible_items: analysis.candidate_items,
            eligible_stack_units: analysis.candidate_stack_units,
            demands: analysis.demands,
            allocation: analysis.allocation,
            excluded_items: analysis.excluded_items,
            unmatched_items: analysis.unmatched_items,
            work_units: analysis.work_units,
            canonical_bytes,
            witness,
        })
    }

    /// Refresh this exact original requirement without demanding frozen roster
    /// bytes. The source pin, original anchor and original allocator seed cannot
    /// change. The owner must additionally retain higher floors consumed by other
    /// reads, including failed ones; this value does not acknowledge those reads.
    pub fn revalidate(
        &self,
        source: &InputSupplySource,
        operations: &LiveOperationsObservation,
        context: &OperationContext,
    ) -> Result<Self> {
        if source.witness != self.source_witness {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "original furniture input source cannot be substituted",
            ));
        }
        let fresh = Self::qualify(
            &self.requirements,
            source,
            self.original_anchor,
            operations,
            context,
        )?;
        if fresh
            .source_counters
            .iter()
            .zip(self.source_counters)
            .any(|(new, original)| *new < original)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "furniture input allocator regressed below its original source seed",
            ));
        }
        Ok(fresh)
    }

    pub fn requirements(&self) -> &FurnitureInputRequirements {
        &self.requirements
    }

    pub const fn ready(&self) -> bool {
        self.allocation.allocated_units == self.requirements.required_stack_units
    }

    pub const fn required_stack_units(&self) -> u64 {
        self.requirements.required_stack_units
    }

    pub const fn eligible_stack_units(&self) -> u64 {
        self.eligible_stack_units
    }

    pub const fn shortfall_stack_units(&self) -> u64 {
        self.requirements
            .required_stack_units
            .saturating_sub(self.allocation.allocated_units)
    }

    pub const fn eligible_items(&self) -> u64 {
        self.eligible_items
    }

    pub fn demands(&self) -> &[MaterialDemand] {
        &self.demands
    }

    pub const fn allocation(&self) -> &Allocation {
        &self.allocation
    }

    pub fn excluded_items(&self) -> &BTreeMap<&'static str, u64> {
        &self.excluded_items
    }

    pub const fn unmatched_items(&self) -> u64 {
        self.unmatched_items
    }

    pub const fn work_units(&self) -> u64 {
        self.work_units
    }

    pub const fn original_anchor(&self) -> StateAnchor {
        self.original_anchor
    }

    pub const fn source_witness(&self) -> Digest32 {
        self.source_witness
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

    pub const fn item_rows(&self) -> u32 {
        self.item_rows
    }

    pub fn canonical_bytes(&self) -> &[u8] {
        &self.canonical_bytes
    }

    pub const fn witness(&self) -> Digest32 {
        self.witness
    }
}

fn source_bounds(operations: &LiveOperationsObservation, context: &OperationContext) -> Result<()> {
    let entities = operations
        .jobs
        .jobs
        .len()
        .saturating_add(operations.buildings.len())
        .saturating_add(operations.items.len())
        .saturating_add(2);
    if entities > INPUT_SUPPLY_MAX_ENTITIES
        || entities > context.budget.max_entities as usize
        || operations.attachments.len() > INPUT_SUPPLY_MAX_EDGES
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete furniture input census exceeds its entity or attachment bound",
        ));
    }
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
    if edges > INPUT_SUPPLY_MAX_EDGES {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "complete furniture input census exceeds its relationship bound",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "input_supply_tests.rs"]
mod tests;
