//! Exact bounded task selection over two disjoint resource universes. A task
//! receives ALL its worker and material demands, or consumes neither resource.
//! Mandatory material pools are supported by every candidate set, including the
//! empty set. This is a declared allocation model, not a reservation or schedule.
use std::time::{Duration, Instant};
use dfmcp_world::inventory_allocation::{self as flow, Allocation, AllocationError, Demand, Shortage, Supply};

type Result<T> = std::result::Result<T, AllocationError>;
pub const MAX_TASKS: usize = 8;
/// Material-only owner marker. These demands are never optional task inputs.
/// Each reserve pool requires distinct units, even when its selectors overlap.
pub const RESERVE_OWNER: usize = usize::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub key: String,
    pub priority: u32,
}
#[derive(Clone, Copy)]
pub struct Model<'a> {
    pub supplies: &'a [Supply],
    pub demands: &'a [Demand],
    /// One task index per demand in canonical order. Only the material model
    /// may use RESERVE_OWNER for mandatory, non-consumed reserve pools.
    pub owners: &'a [usize],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Domain { Workers, Materials }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rejection {
    pub task_mask: u16,
    pub domain: Domain,
    /// Demand indices refer to the ORIGINAL model, not a remapped subset.
    /// Material cuts may include mandatory reserves as well as task inputs.
    pub shortage: Shortage,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub task_mask: u16,
    pub priority: u64,
    pub workers: Allocation,
    /// Includes reserve support separately from task consumption. A shortage
    /// here means the hard reserves alone are infeasible: NO task set, including
    /// the empty one, is admissible. Partial reserve flow is diagnostic only.
    pub materials: Allocation,
    /// One checked cut for EVERY combination ranked ahead of the selected set.
    /// If reserves alone fail, materials.shortage excludes all sets directly;
    /// rejected is empty and there is no feasible optimum to claim.
    pub rejected: Vec<Rejection>,
    pub flow_calls: u32,
    pub work_units: u64,
}
struct Budget { used: u64, maximum: u64, started: Instant, wall: Duration, calls: u32 }
impl Budget {
    fn charge(&mut self, amount: u64) -> Result<()> {
        self.used = self.used.checked_add(amount).ok_or(AllocationError::ArithmeticOverflow)?;
        if self.used > self.maximum || self.started.elapsed() >= self.wall {
            return Err(AllocationError::BudgetExceeded);
        }
        Ok(())
    }
    fn available(&mut self) -> Result<u64> {
        self.charge(0)?;
        self.maximum.checked_sub(self.used).filter(|n| *n > 0).ok_or(AllocationError::BudgetExceeded)
    }
}
fn valid_key(key: &str) -> bool {
    !key.is_empty() && key.len() <= 64
        && key.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn validate_model(model: Model<'_>, tasks: usize, allow_reserves: bool) -> Result<()> {
    if model.demands.is_empty() || model.demands.len() > flow::MAX_DEMANDS
        || model.owners.len() != model.demands.len() || model.supplies.len() > flow::MAX_SUPPLIES {
        return Err(AllocationError::InvalidInput);
    }
    let mut present = 0u16;
    for &owner in model.owners {
        if owner == RESERVE_OWNER && allow_reserves { continue; }
        if owner >= tasks { return Err(AllocationError::InvalidInput); }
        present |= 1u16 << owner;
    }
    if present != (1u16 << tasks) - 1 { return Err(AllocationError::InvalidInput); }
    // Full-model flow checks IDs, ordering, keys, capacities, masks and overflow
    // BEFORE reserve infeasibility or subset selection can discard malformed input.
    Ok(())
}
fn full(model: Model<'_>, budget: &mut Budget) -> Result<Allocation> {
    let out = flow::allocate(model.supplies, model.demands, budget.available()?)?;
    budget.calls += 1;
    budget.charge(out.work_units)?;
    Ok(out)
}
fn empty(size: usize) -> Allocation {
    Allocation { assignments: Vec::new(), allocated_by_demand: vec![0; size], requested_units: 0,
        allocated_units: 0, cut_capacity: 0, shortage: None, work_units: 0 }
}
fn subset(model: Model<'_>, mask: u16, budget: &mut Budget) -> Result<Allocation> {
    budget.charge(model.demands.len() as u64)?;
    let indices: Vec<_> = model.owners.iter().enumerate()
        .filter_map(|(i, &owner)| (owner == RESERVE_OWNER || mask & (1u16 << owner) != 0).then_some(i)).collect();
    if indices.is_empty() { return Ok(empty(model.demands.len())); }
    let demands: Vec<_> = indices.iter().map(|&i| model.demands[i].clone()).collect();
    let mut supplies = Vec::new();
    for supply in model.supplies {
        budget.charge(1 + indices.len() as u64)?;
        let mut eligible = 0u32;
        for (new, &old) in indices.iter().enumerate() {
            if supply.eligible & (1u32 << old) != 0 { eligible |= 1u32 << new; }
        }
        if eligible != 0 { supplies.push(Supply { id: supply.id, units: supply.units, eligible }); }
    }
    let mut out = flow::allocate(&supplies, &demands, budget.available()?)?;
    budget.calls += 1;
    budget.charge(out.work_units + out.assignments.len() as u64 + indices.len() as u64)?;
    for assignment in &mut out.assignments { assignment.demand_index = indices[assignment.demand_index]; }
    let mut counts = vec![0; model.demands.len()];
    for (i, &old) in indices.iter().enumerate() { counts[old] = out.allocated_by_demand[i]; }
    out.allocated_by_demand = counts;
    if let Some(cut) = &mut out.shortage {
        for index in &mut cut.demand_indices { *index = indices[*index]; }
    }
    Ok(out)
}
fn rejection(mask: u16, domain: Domain, allocation: &Allocation) -> Result<Rejection> {
    let shortage = allocation.shortage.clone().ok_or(AllocationError::InvariantViolation)?;
    if shortage.deficit == 0 || allocation.allocated_units >= allocation.requested_units {
        return Err(AllocationError::InvariantViolation);
    }
    Ok(Rejection { task_mask: mask, domain, shortage })
}

/// Rank sets by priority sum DESC, task count DESC, selected key list ASC. Positive
/// priorities make adding a fully supported task strictly preferable. With unit
/// priorities this maximizes the number of COMPLETE tasks, not individual slots.
/// At most 255 nonempty sets are considered; exhaustion returns no partial plan.
/// Reserve support is solved jointly with each set, NOT greedily subtracted from
/// specific stacks first. Residual rerouting can preserve scarce task-only inputs.
pub fn select(tasks: &[Task], workers: Model<'_>, materials: Model<'_>, maximum_work: u64,
    wall: Duration) -> Result<Selection> {
    if tasks.is_empty() || tasks.len() > MAX_TASKS || maximum_work == 0
        || maximum_work > flow::MAX_WORK || wall.is_zero() {
        return Err(AllocationError::BudgetExceeded);
    }
    for (i, task) in tasks.iter().enumerate() {
        if !valid_key(&task.key) || !(1..=1_000_000).contains(&task.priority)
            || (i > 0 && tasks[i - 1].key >= task.key) {
            return Err(AllocationError::InvalidInput);
        }
    }
    validate_model(workers, tasks.len(), false)?;
    validate_model(materials, tasks.len(), true)?;
    if workers.supplies.iter().any(|s| s.units != 1) { return Err(AllocationError::InvalidInput); }
    let mut budget = Budget { used: 0, maximum: maximum_work, started: Instant::now(), wall, calls: 0 };
    let mut all_workers = Some(full(workers, &mut budget)?);
    let mut all_materials = Some(full(materials, &mut budget)?);
    let reserve_only = if materials.owners.contains(&RESERVE_OWNER) {
        subset(materials, 0, &mut budget)?
    } else { empty(materials.demands.len()) };
    if reserve_only.shortage.is_some() {
        // A subset of mandatory demands already lacks distinct supply. Every
        // larger model is infeasible, regardless of task priority or worker count.
        budget.charge(0)?;
        return Ok(Selection { task_mask: 0, priority: 0, workers: empty(workers.demands.len()),
            materials: reserve_only, rejected: Vec::new(), flow_calls: budget.calls, work_units: budget.used });
    }
    let all = (1u16 << tasks.len()) - 1;
    let mut ranked = Vec::with_capacity(usize::from(all));
    for mask in 1..=all {
        budget.charge(tasks.len() as u64 + 8)?;
        let indices: Vec<_> = (0..tasks.len()).filter(|i| mask & (1u16 << i) != 0).collect();
        let priority: u64 = indices.iter().map(|&i| u64::from(tasks[i].priority)).sum();
        ranked.push((mask, priority, indices));
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.2.len().cmp(&a.2.len())).then_with(|| a.2.cmp(&b.2)));
    let mut rejected = Vec::new();
    for (mask, priority, _) in ranked {
        budget.charge(1)?;
        let worker_allocation = if mask == all {
            all_workers.take().ok_or(AllocationError::InvariantViolation)?
        } else { subset(workers, mask, &mut budget)? };
        if worker_allocation.shortage.is_some() {
            rejected.push(rejection(mask, Domain::Workers, &worker_allocation)?);
            continue;
        }
        let material_allocation = if mask == all {
            all_materials.take().ok_or(AllocationError::InvariantViolation)?
        } else { subset(materials, mask, &mut budget)? };
        if material_allocation.shortage.is_some() {
            rejected.push(rejection(mask, Domain::Materials, &material_allocation)?);
            continue;
        }
        budget.charge(0)?;
        return Ok(Selection { task_mask: mask, priority, workers: worker_allocation, materials: material_allocation,
            rejected, flow_calls: budget.calls, work_units: budget.used });
    }
    budget.charge(0)?;
    Ok(Selection { task_mask: 0, priority: 0, workers: empty(workers.demands.len()), materials: reserve_only,
        rejected, flow_calls: budget.calls, work_units: budget.used })
}

#[cfg(test)]
#[path = "production_selection_tests.rs"]
mod tests;
