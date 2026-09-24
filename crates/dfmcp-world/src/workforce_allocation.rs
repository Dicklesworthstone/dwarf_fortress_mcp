#![forbid(unsafe_code)]

//! Exact, bounded, lexicographic unit-worker allocation. No game eligibility or
//! mutation semantics live here. Inputs must already be filtered and canonical.
//! This file can be tested alone with `rustc --edition=2024 --test`.

use std::collections::BTreeSet;

pub const MAX_WORKERS: usize = 4_096;
pub const MAX_DEMANDS: usize = 16;
pub const MAX_SLOTS: u32 = 128;
pub const MAX_PRIORITY: u16 = 1_000;
pub const MAX_WORK: u64 = 10_000_000;
pub const POLICY: &str = "max-fill-priority-effective-nominal-distance/1";

/// Minimize lexicographically, *after* maximizing cardinality. No big-M weights
/// or floating point: one unit of an earlier objective dominates every later one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Cost(pub [i64; 4]);
impl Cost {
    fn add(self, other: Self) -> Result<Self> {
        let mut out = [0; 4];
        for (i, value) in out.iter_mut().enumerate() {
            *value = self.0[i]
                .checked_add(other.0[i])
                .ok_or(Error::ArithmeticOverflow)?;
        }
        Ok(Self(out))
    }
    fn sub(self, other: Self) -> Result<Self> {
        let mut out = [0; 4];
        for (i, value) in out.iter_mut().enumerate() {
            *value = self.0[i]
                .checked_sub(other.0[i])
                .ok_or(Error::ArithmeticOverflow)?;
        }
        Ok(Self(out))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Demand {
    pub key: String,
    pub workers: u32,
    /// Larger values prefer filling this demand when not all demands can be met.
    pub priority: u16,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub worker_id: u64,
    pub demand_index: usize,
    pub effective: i32,
    pub nominal: i32,
    pub steps: u32,
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Assignment {
    pub demand_index: usize,
    pub worker_id: u64,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortage {
    pub demand_indices: Vec<usize>,
    pub required_workers: u32,
    pub eligible_workers: u32,
    pub deficit: u32,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Witness {
    /// Canonical node order: source, demands by key, workers by ID, sink.
    pub potentials: Vec<Cost>,
    pub source_side: Vec<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub assignments: Vec<Assignment>,
    pub allocated_by_demand: Vec<u32>,
    pub requested_workers: u32,
    pub assigned_workers: u32,
    pub cut_capacity: u32,
    /// [-sum(priority), -sum(effective), -sum(nominal), sum(steps)].
    pub cost: Cost,
    pub shortage: Option<Shortage>,
    pub witness: Witness,
    pub work_units: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidInput,
    BudgetExceeded,
    ArithmeticOverflow,
    InvalidCertificate,
}
pub type Result<T> = std::result::Result<T, Error>;

struct Work {
    used: u64,
    maximum: u64,
}
impl Work {
    fn new(maximum: u64) -> Result<Self> {
        if maximum == 0 || maximum > MAX_WORK {
            return Err(Error::BudgetExceeded);
        }
        Ok(Self { used: 0, maximum })
    }
    fn charge(&mut self, n: u64) -> Result<()> {
        self.used = self.used.checked_add(n).ok_or(Error::ArithmeticOverflow)?;
        if self.used > self.maximum {
            return Err(Error::BudgetExceeded);
        }
        Ok(())
    }
}

struct Model {
    workers: Vec<u64>,
    requested: u32,
    sink: usize,
}
impl Model {
    fn worker_node(&self, demands: usize, id: u64) -> Result<usize> {
        self.workers
            .binary_search(&id)
            .map(|i| 1 + demands + i)
            .map_err(|_| Error::InvalidCertificate)
    }
}
fn validate(demands: &[Demand], candidates: &[Candidate], work: &mut Work) -> Result<Model> {
    if demands.is_empty()
        || demands.len() > MAX_DEMANDS
        || candidates.len() > MAX_DEMANDS * MAX_WORKERS
    {
        return Err(Error::BudgetExceeded);
    }
    let mut requested = 0u32;
    for (i, d) in demands.iter().enumerate() {
        work.charge(1)?;
        if d.key.is_empty()
            || d.key.len() > 64
            || d.workers == 0
            || d.workers > MAX_SLOTS
            || d.priority > MAX_PRIORITY
            || !d
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            || (i > 0 && demands[i - 1].key >= d.key)
        {
            return Err(Error::InvalidInput);
        }
        requested = requested
            .checked_add(d.workers)
            .ok_or(Error::ArithmeticOverflow)?;
    }
    if requested > MAX_SLOTS {
        return Err(Error::BudgetExceeded);
    }
    let mut workers = BTreeSet::new();
    for (i, c) in candidates.iter().enumerate() {
        work.charge(1)?;
        if c.worker_id == 0
            || c.demand_index >= demands.len()
            || c.effective < 0
            || (i > 0
                && (candidates[i - 1].demand_index, candidates[i - 1].worker_id)
                    >= (c.demand_index, c.worker_id))
        {
            return Err(Error::InvalidInput);
        }
        workers.insert(c.worker_id);
        if workers.len() > MAX_WORKERS {
            return Err(Error::BudgetExceeded);
        }
    }
    let sink = 1 + demands.len() + workers.len();
    Ok(Model {
        workers: workers.into_iter().collect(),
        requested,
        sink,
    })
}
fn cost(demands: &[Demand], c: &Candidate) -> Cost {
    Cost([
        -i64::from(demands[c.demand_index].priority),
        -i64::from(c.effective),
        -i64::from(c.nominal),
        i64::from(c.steps),
    ])
}
#[derive(Clone, Copy)]
struct Arc {
    to: usize,
    reverse: usize,
    remaining: u32,
    cost: Cost,
}
fn arc(graph: &mut [Vec<Arc>], from: usize, to: usize, capacity: u32, cost: Cost) -> Result<usize> {
    let index = graph[from].len();
    let reverse = graph[to].len();
    let negative = Cost::default().sub(cost)?;
    graph[from].push(Arc {
        to,
        reverse,
        remaining: capacity,
        cost,
    });
    graph[to].push(Arc {
        to: from,
        reverse: index,
        remaining: 0,
        cost: negative,
    });
    Ok(index)
}
fn deficiency(
    demands: &[Demand],
    candidates: &[Candidate],
    model: &Model,
    source_side: &[bool],
    assigned: u32,
    work: &mut Work,
) -> Result<Option<Shortage>> {
    if assigned == model.requested {
        return Ok(None);
    }
    let mut indices = Vec::new();
    let mut required = 0u32;
    for (d, demand) in demands.iter().enumerate() {
        work.charge(1)?;
        if source_side[1 + d] {
            indices.push(d);
            required += demand.workers;
        }
    }
    let mut neighbors = BTreeSet::new();
    for c in candidates {
        work.charge(1)?;
        if source_side[1 + c.demand_index] {
            neighbors.insert(c.worker_id);
        }
    }
    let eligible = u32::try_from(neighbors.len()).map_err(|_| Error::ArithmeticOverflow)?;
    let deficit = required
        .checked_sub(eligible)
        .ok_or(Error::InvalidCertificate)?;
    if deficit == 0 || deficit != model.requested - assigned {
        return Err(Error::InvalidCertificate);
    }
    Ok(Some(Shortage {
        demand_indices: indices,
        required_workers: required,
        eligible_workers: eligible,
        deficit,
    }))
}

/// Successive shortest augmenting paths with exact vector potentials. Fully
/// explores each residual search; equal labels keep the first canonical parent.
/// Runs the independent certificate verifier before returning any allocation.
pub fn allocate(demands: &[Demand], candidates: &[Candidate], max_work: u64) -> Result<Allocation> {
    let mut work = Work::new(max_work)?;
    let model = validate(demands, candidates, &mut work)?;
    let mut graph = vec![Vec::new(); model.sink + 1];
    let mut potentials = vec![Cost::default(); graph.len()];
    for (d, demand) in demands.iter().enumerate() {
        work.charge(1)?;
        arc(&mut graph, 0, 1 + d, demand.workers, Cost::default())?;
    }
    let mut links = Vec::with_capacity(candidates.len());
    for c in candidates {
        work.charge(1)?;
        let worker = model.worker_node(demands.len(), c.worker_id)?;
        let value = cost(demands, c);
        // Total requested slots is a proven finite upper bound on every flow,
        // not an arbitrary magic infinity. It also preserves the Hall cut.
        links.push(arc(
            &mut graph,
            1 + c.demand_index,
            worker,
            model.requested,
            value,
        )?);
        potentials[worker] = potentials[worker].min(value);
    }
    for i in 0..model.workers.len() {
        work.charge(1)?;
        let node = 1 + demands.len() + i;
        arc(&mut graph, node, model.sink, 1, Cost::default())?;
        potentials[model.sink] = potentials[model.sink].min(potentials[node]);
    }
    let mut assigned = 0u32;
    let source_side = loop {
        work.charge(graph.len() as u64)?;
        let mut distance = vec![None; graph.len()];
        let mut parent = vec![None; graph.len()];
        distance[0] = Some(Cost::default());
        let mut frontier = BTreeSet::from([(Cost::default(), 0usize)]);
        while let Some((label, from)) = frontier.pop_first() {
            work.charge(1)?;
            for (index, edge) in graph[from].iter().enumerate() {
                work.charge(1)?;
                if edge.remaining == 0 {
                    continue;
                }
                let reduced = edge.cost.add(potentials[from])?.sub(potentials[edge.to])?;
                if reduced < Cost::default() {
                    return Err(Error::InvalidCertificate);
                }
                let next = label.add(reduced)?;
                if distance[edge.to].is_none_or(|prior| next < prior) {
                    if let Some(prior) = distance[edge.to] {
                        frontier.remove(&(prior, edge.to));
                    }
                    distance[edge.to] = Some(next);
                    parent[edge.to] = Some((from, index));
                    frontier.insert((next, edge.to));
                }
            }
        }
        if distance[model.sink].is_none() {
            break distance.iter().map(Option::is_some).collect::<Vec<_>>();
        }
        // Raise unreachable potentials by the maximum finite distance. Leaving
        // them unchanged can invalidate edges from unreachable to reachable nodes.
        let maximum = distance
            .iter()
            .flatten()
            .copied()
            .max()
            .ok_or(Error::InvalidCertificate)?;
        for (node, p) in potentials.iter_mut().enumerate() {
            work.charge(1)?;
            *p = p.add(distance[node].unwrap_or(maximum))?;
        }
        let mut node = model.sink;
        let mut length = 0usize;
        while node != 0 {
            work.charge(1)?;
            length += 1;
            if length >= graph.len() {
                return Err(Error::InvalidCertificate);
            }
            let (from, index) = parent[node].ok_or(Error::InvalidCertificate)?;
            let edge = graph[from][index];
            graph[from][index].remaining = edge
                .remaining
                .checked_sub(1)
                .ok_or(Error::InvalidCertificate)?;
            graph[node][edge.reverse].remaining = graph[node][edge.reverse]
                .remaining
                .checked_add(1)
                .ok_or(Error::ArithmeticOverflow)?;
            node = from;
        }
        assigned += 1;
        if assigned > model.requested {
            return Err(Error::InvalidCertificate);
        }
    };
    let mut assignments = Vec::new();
    let mut allocated_by_demand = vec![0; demands.len()];
    let mut total_cost = Cost::default();
    for (c, &index) in candidates.iter().zip(&links) {
        work.charge(1)?;
        let remaining = graph[1 + c.demand_index][index].remaining;
        let flow = model
            .requested
            .checked_sub(remaining)
            .ok_or(Error::InvalidCertificate)?;
        if flow > 1 {
            return Err(Error::InvalidCertificate);
        }
        if flow == 1 {
            assignments.push(Assignment {
                demand_index: c.demand_index,
                worker_id: c.worker_id,
            });
            allocated_by_demand[c.demand_index] += 1;
            total_cost = total_cost.add(cost(demands, c))?;
        }
    }
    let shortage = deficiency(
        demands,
        candidates,
        &model,
        &source_side,
        assigned,
        &mut work,
    )?;
    let mut out = Allocation {
        assignments,
        allocated_by_demand,
        requested_workers: model.requested,
        assigned_workers: assigned,
        cut_capacity: assigned,
        cost: total_cost,
        shortage,
        witness: Witness {
            potentials,
            source_side,
        },
        work_units: 0,
    };
    verify_inner(demands, candidates, &out, &mut work)?;
    out.work_units = work.used;
    Ok(out)
}

/// Reconstructs residual capacities from the proposed assignments, not from the
/// solver's mutable residual network. Nonnegative reduced costs certify minimum
/// lexicographic cost; a residual-closed cut certifies maximum cardinality.
pub fn verify(
    demands: &[Demand],
    candidates: &[Candidate],
    result: &Allocation,
    max_work: u64,
) -> Result<u64> {
    let mut work = Work::new(max_work)?;
    verify_inner(demands, candidates, result, &mut work)?;
    Ok(work.used)
}
fn verify_inner(
    demands: &[Demand],
    candidates: &[Candidate],
    out: &Allocation,
    work: &mut Work,
) -> Result<()> {
    let model = validate(demands, candidates, work)?;
    let witness = &out.witness;
    if out.assignments.len() > model.requested as usize
        || out.requested_workers != model.requested
        || out.assigned_workers as usize != out.assignments.len()
        || out.cut_capacity != out.assigned_workers
        || out.allocated_by_demand.len() != demands.len()
        || witness.potentials.len() != model.sink + 1
        || witness.source_side.len() != model.sink + 1
        || !witness.source_side[0]
        || witness.source_side[model.sink]
    {
        return Err(Error::InvalidCertificate);
    }
    let mut counts = vec![0u32; demands.len()];
    let mut workers = BTreeSet::new();
    let mut selected = BTreeSet::new();
    let mut total = Cost::default();
    for (i, a) in out.assignments.iter().enumerate() {
        work.charge(1)?;
        if a.demand_index >= demands.len()
            || (i > 0 && out.assignments[i - 1] >= *a)
            || !workers.insert(a.worker_id)
        {
            return Err(Error::InvalidCertificate);
        }
        let key = (a.demand_index, a.worker_id);
        let index = candidates
            .binary_search_by_key(&key, |c| (c.demand_index, c.worker_id))
            .map_err(|_| Error::InvalidCertificate)?;
        selected.insert(key);
        counts[a.demand_index] += 1;
        total = total.add(cost(demands, &candidates[index]))?;
    }
    if counts != out.allocated_by_demand || total != out.cost {
        return Err(Error::InvalidCertificate);
    }
    let mut cut = 0u32;
    let mut check = |from: usize, to: usize, capacity: u32, flow: u32, value: Cost| -> Result<()> {
        work.charge(1)?;
        if flow > capacity {
            return Err(Error::InvalidCertificate);
        }
        if witness.source_side[from] && !witness.source_side[to] {
            cut = cut.checked_add(capacity).ok_or(Error::ArithmeticOverflow)?;
        }
        if flow < capacity {
            if value
                .add(witness.potentials[from])?
                .sub(witness.potentials[to])?
                < Cost::default()
                || (witness.source_side[from] && !witness.source_side[to])
            {
                return Err(Error::InvalidCertificate);
            }
        }
        if flow > 0 {
            if Cost::default()
                .sub(value)?
                .add(witness.potentials[to])?
                .sub(witness.potentials[from])?
                < Cost::default()
                || (witness.source_side[to] && !witness.source_side[from])
            {
                return Err(Error::InvalidCertificate);
            }
        }
        Ok(())
    };
    for (d, demand) in demands.iter().enumerate() {
        check(0, 1 + d, demand.workers, counts[d], Cost::default())?;
    }
    for c in candidates {
        let node = model.worker_node(demands.len(), c.worker_id)?;
        check(
            1 + c.demand_index,
            node,
            model.requested,
            u32::from(selected.contains(&(c.demand_index, c.worker_id))),
            cost(demands, c),
        )?;
    }
    for (i, id) in model.workers.iter().enumerate() {
        check(
            1 + demands.len() + i,
            model.sink,
            1,
            u32::from(workers.contains(id)),
            Cost::default(),
        )?;
    }
    if cut != out.cut_capacity
        || deficiency(
            demands,
            candidates,
            &model,
            &witness.source_side,
            out.assigned_workers,
            work,
        )? != out.shortage
    {
        return Err(Error::InvalidCertificate);
    }
    Ok(())
}

#[cfg(test)]
#[path = "workforce_allocation_tests.rs"]
mod tests;
