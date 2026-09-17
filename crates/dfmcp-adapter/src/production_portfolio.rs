//! Task-level production analysis at one declared delivery/working origin.
//! All candidate evidence is taken from the same citizen/spatial observation.
use super::*;
use crate::operations_analysis::MaterialDemand;
use crate::spatial_inventory::{self as inventory, SpatialInventory};
use std::time::Duration;

#[path = "production_selection.rs"]
pub mod selection;

pub const PORTFOLIO_POLICY: &str = "joint-complete-tasks-common-origin/1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionTask {
    pub key: String,
    pub priority: u32,
    pub workers: u32,
    pub skill_key: String,
    pub min_effective_skill: i32,
    pub preserve_social: bool,
    pub adults_only: bool,
    /// Local keys are scoped to this task, not shared with other tasks. Every
    /// input is required; item_types within one input are interchangeable.
    pub materials: Vec<MaterialDemand>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionPortfolio {
    pub tasks: Vec<ProductionTask>,
    pub workforce: WorkforceAnalysis,
    pub inventory: SpatialInventory,
    pub material_owners: Vec<usize>,
    pub selection: selection::Selection,
    pub work_units: u64,
}
fn remaining_context(context: &OperationContext, work: &mut Work) -> Result<OperationContext> {
    work.charge(0)?;
    let elapsed = work.started.elapsed().as_millis();
    if elapsed >= u128::from(context.budget.max_wall_millis) {
        return Err(exhausted("joint production analysis exhausted its cooperative deadline"));
    }
    let mut narrowed = context.clone();
    narrowed.budget.max_wall_millis -= elapsed as u64;
    Ok(narrowed)
}
fn normalize_tasks(input: &[ProductionTask]) -> Result<Vec<ProductionTask>> {
    if input.is_empty() || input.len() > selection::MAX_TASKS {
        return Err(exhausted("joint production planning accepts one to eight tasks"));
    }
    // Validate raw cardinalities and string bounds before cloning or deduping.
    // A large repeated selector list must not evade the public model contract.
    let mut total_workers = 0u32;
    for task in input {
        if task.key.is_empty() || task.key.len() > 32
            || !task.key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || !(1..=1_000_000).contains(&task.priority)
            || !(1..=MAX_WORKER_SLOTS).contains(&task.workers)
            || task.skill_key.is_empty() || task.skill_key.len() > 96
            || task.skill_key.chars().any(char::is_control) || task.min_effective_skill < 0
            || task.materials.is_empty() || task.materials.len() > 4 {
            return Err(invalid("invalid production task key, priority, workers, skill or material-input count"));
        }
        total_workers = total_workers.checked_add(task.workers)
            .ok_or_else(|| exhausted("production worker count overflow"))?;
        for material in &task.materials {
            if material.key.is_empty() || material.key.len() > 48
                || !material.key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                || material.units == 0 || material.item_types.is_empty() || material.item_types.len() > 8
                || material.item_types.iter().any(|s| s.is_empty() || s.len() > 128 || s.contains('\0'))
                || material.subtype.is_some_and(|n| n < -1) || material.material_type.is_some_and(|n| n < -1)
                || material.material_index.is_some_and(|n| n < -1)
                || (material.material_index.is_some() && material.material_type.is_none()) {
                return Err(invalid("invalid task-local material demand"));
            }
        }
    }
    if total_workers > MAX_WORKER_SLOTS { return Err(exhausted("production request exceeds 128 worker slots")); }
    let mut tasks = input.to_vec();
    tasks.sort_by(|a, b| a.key.cmp(&b.key));
    if tasks.windows(2).any(|p| p[0].key == p[1].key) { return Err(invalid("duplicate production task key")); }
    for task in &mut tasks {
        for material in &mut task.materials {
            material.item_types.sort();
            material.item_types.dedup();
        }
        task.materials.sort_by(|a, b| a.key.cmp(&b.key));
        if task.materials.windows(2).any(|p| p[0].key == p[1].key) {
            return Err(invalid("duplicate material key within a production task"));
        }
    }
    Ok(tasks)
}

/// No plan handles, leases or effects are created. Both domains must fill every
/// demand of the SAME chosen set. Candidates discarded by a full-demand partial
/// allocation remain available to the subset solver through the complete model.
pub fn plan(state: &LiveSpatialCitizenState, context: &OperationContext, origin: [u32; 3],
    requested: &[ProductionTask], maximum_work: u64) -> Result<ProductionPortfolio> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mut work = Work::new(context, maximum_work)?;
    let tasks = normalize_tasks(requested)?;
    let worker_demands: Vec<_> = tasks.iter().map(|t| WorkforceDemand {
        key: t.key.clone(), workers: t.workers, target: origin, skill_key: t.skill_key.clone(),
        min_effective_skill: t.min_effective_skill, preserve_social: t.preserve_social, adults_only: t.adults_only,
    }).collect();
    let workforce = analyze_inner(state, context, &worker_demands, &mut work)?;
    let mut materials = Vec::new();
    let mut owners = BTreeMap::new();
    for (owner, task) in tasks.iter().enumerate() {
        for material in &task.materials {
            work.charge(1)?;
            let mut demand = material.clone();
            demand.key = format!("{owner:02}.{}", material.key);
            owners.insert(demand.key.clone(), owner);
            materials.push(demand);
        }
    }
    let inventory_context = remaining_context(context, &mut work)?;
    let inventory = inventory::plan(state, &inventory_context, origin, &materials, work.remaining())?;
    work.charge(inventory.work_units)?;
    if inventory.anchor != workforce.anchor || inventory.source_digest != workforce.source_digest {
        return Err(invariant("production domains do not name the same coherent capture"));
    }
    let material_owners = inventory.demands.iter().map(|d| owners.get(&d.key).copied()
        .ok_or_else(|| invariant("joint material owner was lost during normalization"))).collect::<Result<Vec<_>>>()?;
    let mut eligible = BTreeMap::<u64, u32>::new();
    for (demand, candidates) in workforce.candidates.iter().enumerate() {
        for candidate in candidates {
            work.charge(1)?;
            *eligible.entry(candidate.entity_id).or_default() |= 1u32 << demand;
        }
    }
    let workers: Vec<_> = eligible.into_iter().map(|(id, eligible)| flow::Supply { id, units: 1, eligible }).collect();
    let worker_model: Vec<_> = workforce.demands.iter().map(|d| flow::Demand { key: d.key.clone(), units: u64::from(d.workers) }).collect();
    let worker_owners: Vec<_> = (0..tasks.len()).collect();
    let material_model: Vec<_> = inventory.demands.iter().map(|d| flow::Demand { key: d.key.clone(), units: d.units }).collect();
    let scores: Vec<_> = tasks.iter().map(|t| selection::Task { key: t.key.clone(), priority: t.priority }).collect();
    let timed = remaining_context(context, &mut work)?;
    let selected = selection::select(&scores,
        selection::Model { supplies: &workers, demands: &worker_model, owners: &worker_owners },
        selection::Model { supplies: &inventory.supplies, demands: &material_model, owners: &material_owners },
        work.remaining(), Duration::from_millis(timed.budget.max_wall_millis)).map_err(|e| match e {
            flow::AllocationError::InvalidInput | flow::AllocationError::InvariantViolation => invariant("joint production model or certificate is inconsistent"),
            _ => exhausted("joint production planning exceeded work, time or arithmetic bounds; no partial optimum returned"),
        })?;
    work.charge(selected.work_units)?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(ProductionPortfolio { tasks, workforce, inventory, material_owners, selection: selected, work_units: work.used })
}
