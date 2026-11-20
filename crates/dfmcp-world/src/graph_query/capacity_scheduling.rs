//! Deterministic nonpreemptive scheduling with one shared pool of identical slots.
//!
//! This is a conditional list-scheduling heuristic over the selected dependency
//! closure, not a worker allocator, optimal schedule, or game-time guarantee.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BTreeSet};

use super::{
    DependencySchedule, DependencyScheduleQuery, DfmcpError, Digest32, EntityId, ErrorCode,
    GraphBudget, GraphWitness, Result, WorldSnapshot, budget_error, invariant, put_bytes, put_str,
    put_u64, schedule_dependencies_with_check,
};

pub const CAPACITY_SCHEDULE_POLICY: &str = "dependency-unit-capacity-longest-tail-stable-id/1";
pub const MAX_PARALLEL_SLOTS: u32 = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacityScheduledDependency {
    pub entity_id: EntityId,
    pub generation: u32,
    pub revision: u64,
    pub duration: u64,
    /// Model offsets, in the selected duration field's units.
    pub start: u64,
    pub finish: u64,
    /// Latest actual finish among this task's prerequisites, or zero for roots.
    pub ready_at: u64,
    /// Time spent ready but waiting for shared capacity: start - ready_at.
    pub queue_delay: u64,
    /// Zero-based model slot. Zero-duration tasks consume no slot.
    pub slot: Option<u32>,
    /// Longest selected path from this task through its dependents, including
    /// its own duration. Higher values are selected first among ready tasks.
    pub downstream_duration: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapacitySchedule {
    /// The original unlimited-capacity result, with unchanged legacy semantics.
    pub baseline: DependencySchedule,
    pub max_parallel: u32,
    /// Complete selected closure, in stable entity-ID order.
    pub entries: Vec<CapacityScheduledDependency>,
    /// Actual model dispatch order; preserves zero-duration prerequisites.
    pub dispatch_order: Vec<EntityId>,
    pub makespan: u64,
    /// Combined baseline and capacity-analysis work and source witnesses.
    pub witness: GraphWitness,
}

struct Work<F> {
    budget: GraphBudget,
    used: u64,
    examined_arcs: u64,
    next_check: u64,
    check: F,
}

impl<F: FnMut() -> Result<()>> Work<F> {
    fn charge(&mut self) -> Result<()> {
        self.used = self
            .used
            .checked_add(1)
            .ok_or_else(|| budget_error("capacity schedule work counter overflow"))?;
        if self.used > self.budget.max_work {
            return Err(budget_error(
                "capacity schedule exhausted its combined operation budget",
            ));
        }
        if self.used >= self.next_check {
            (self.check)()?;
            self.next_check = self.used + 128;
        }
        Ok(())
    }

    fn arc(&mut self) -> Result<()> {
        self.charge()?;
        self.examined_arcs += 1;
        Ok(())
    }

    fn frontier(&self, ready: usize, running: usize) -> Result<()> {
        if ready.saturating_add(running) > self.budget.max_frontier {
            return Err(budget_error(
                "capacity schedule exhausted its ready-and-running frontier bound",
            ));
        }
        Ok(())
    }
}

type Priority = (Reverse<u64>, usize);

struct Scheduler {
    dependents: Vec<Vec<usize>>,
    pending: Vec<usize>,
    ready_at: Vec<u64>,
    downstream: Vec<u64>,
    zero_ready: BTreeSet<Priority>,
    positive_ready: BTreeSet<Priority>,
    running: BTreeSet<(u64, u32, usize)>,
    slots: BTreeSet<u32>,
    finished: usize,
}

impl Scheduler {
    fn enqueue(&mut self, node: usize, durations: &[u64]) {
        let priority = (Reverse(self.downstream[node]), node);
        if durations[node] == 0 {
            self.zero_ready.insert(priority);
        } else {
            self.positive_ready.insert(priority);
        }
    }

    fn next(&mut self) -> Option<usize> {
        // Zero-duration work can run even when every slot is occupied. With a
        // free slot, both classes participate in the same longest-tail order.
        let zero = self.zero_ready.first().copied();
        let positive = if self.slots.is_empty() {
            None
        } else {
            self.positive_ready.first().copied()
        };
        match (zero, positive) {
            (Some(zero), Some(positive)) if positive < zero => {
                self.positive_ready.pop_first().map(|(_, node)| node)
            }
            (Some(_), _) => self.zero_ready.pop_first().map(|(_, node)| node),
            (None, Some(_)) => self.positive_ready.pop_first().map(|(_, node)| node),
            (None, None) => None,
        }
    }

    fn frontier<F: FnMut() -> Result<()>>(&self, work: &Work<F>) -> Result<()> {
        work.frontier(
            self.zero_ready.len() + self.positive_ready.len(),
            self.running.len(),
        )
    }

    fn complete<F: FnMut() -> Result<()>>(
        &mut self,
        node: usize,
        at: u64,
        durations: &[u64],
        work: &mut Work<F>,
    ) -> Result<()> {
        work.charge()?;
        self.finished += 1;
        for offset in 0..self.dependents[node].len() {
            work.arc()?;
            let dependent = self.dependents[node][offset];
            self.ready_at[dependent] = self.ready_at[dependent].max(at);
            self.pending[dependent] = self.pending[dependent]
                .checked_sub(1)
                .ok_or_else(|| invariant("capacity schedule prerequisite underflow"))?;
            if self.pending[dependent] == 0 {
                self.enqueue(dependent, durations);
            }
        }
        self.frontier(work)
    }
}

pub fn schedule_dependencies_with_capacity(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &DependencyScheduleQuery,
    budget: GraphBudget,
    max_parallel: u32,
) -> Result<CapacitySchedule> {
    schedule_dependencies_with_capacity_and_check(
        snapshot,
        scope_digest,
        query,
        budget,
        max_parallel,
        || Ok(()),
    )
}

/// Model each positive-duration task as requiring one identical slot for its
/// whole duration. Dispatch ready work by longest remaining downstream duration,
/// then stable entity identity; assign the smallest free slot. Process every
/// finish at an instant before selecting more work. Zero-duration tasks preserve
/// precedence but consume no capacity. No worker identity or eligibility is
/// inferred, no reservation is acquired, and no optimality is claimed.
///
/// Baseline source/provenance/cycle checks apply unchanged. One work budget covers
/// both analyses, and a callback checks cancellation/authority before, throughout
/// and after them. The additional pass uses O(V + E) bounded storage and
/// O((V + E) log V) ordered operations, without recursion or per-tick iteration.
/// Any overflow, cancellation or exhausted bound returns no partial schedule.
pub fn schedule_dependencies_with_capacity_and_check<F: FnMut() -> Result<()>>(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &DependencyScheduleQuery,
    budget: GraphBudget,
    max_parallel: u32,
    mut check: F,
) -> Result<CapacitySchedule> {
    check()?;
    if !(1..=MAX_PARALLEL_SLOTS).contains(&max_parallel) {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "dependency capacity requires 1..=64 identical parallel slots",
        ));
    }
    let baseline =
        schedule_dependencies_with_check(snapshot, scope_digest, query, budget, &mut check)?;
    let mut work = Work {
        budget,
        used: baseline.witness.work_units,
        examined_arcs: baseline.witness.examined_arcs,
        next_check: baseline.witness.work_units + 128,
        check,
    };
    let n = baseline.entries.len();
    let mut ordinals = BTreeMap::new();
    let mut durations = Vec::with_capacity(n);
    for (node, entry) in baseline.entries.iter().enumerate() {
        work.charge()?;
        ordinals.insert(entry.entity_id, node);
        durations.push(entry.duration);
    }
    let selected_kinds: BTreeSet<_> = query.edge_kinds.iter().cloned().collect();
    let mut prerequisites = vec![Vec::new(); n];
    let mut scheduler = Scheduler {
        dependents: vec![Vec::new(); n],
        pending: vec![0; n],
        ready_at: vec![0; n],
        downstream: vec![0; n],
        zero_ready: BTreeSet::new(),
        positive_ready: BTreeSet::new(),
        running: BTreeSet::new(),
        slots: (0..max_parallel).collect(),
        finished: 0,
    };
    // The baseline has validated the complete source and selected closure. This
    // second complete edge scan is still charged; a tiny closure cannot hide it.
    for edge in snapshot.graph.edges.values() {
        work.charge()?;
        if !selected_kinds.contains(&edge.kind) {
            continue;
        }
        let Some(&dependent) = ordinals.get(&edge.from) else {
            continue;
        };
        let prerequisite = ordinals.get(&edge.to).copied().ok_or_else(|| {
            invariant("capacity schedule baseline omitted a selected prerequisite")
        })?;
        work.arc()?;
        prerequisites[dependent].push(prerequisite);
        scheduler.dependents[prerequisite].push(dependent);
    }
    for (node, node_prerequisites) in prerequisites.iter_mut().enumerate() {
        work.charge()?;
        node_prerequisites.sort_unstable();
        node_prerequisites.dedup();
        scheduler.dependents[node].sort_unstable();
        scheduler.dependents[node].dedup();
        scheduler.pending[node] = node_prerequisites.len();
    }
    for id in baseline.dependency_order.iter().rev() {
        work.charge()?;
        let node = ordinals
            .get(id)
            .copied()
            .ok_or_else(|| invariant("capacity schedule baseline order names an absent vertex"))?;
        let mut tail = 0;
        for &dependent in &scheduler.dependents[node] {
            work.arc()?;
            tail = tail.max(scheduler.downstream[dependent]);
        }
        scheduler.downstream[node] = durations[node].checked_add(tail).ok_or_else(|| {
            budget_error("capacity schedule downstream duration exceeds u64 range")
        })?;
    }
    for (node, _) in durations.iter().enumerate() {
        work.charge()?;
        if scheduler.pending[node] == 0 {
            scheduler.enqueue(node, &durations);
            scheduler.frontier(&work)?;
        }
    }
    let mut entries = vec![None; n];
    let mut dispatch_order = Vec::with_capacity(n);
    let mut at = 0;
    while scheduler.finished < n {
        // A later simultaneous completion may unlock higher-priority work than
        // the first one; dispatch only after the complete finish set is applied.
        while let Some(&(finish, slot, node)) = scheduler.running.first() {
            if finish != at {
                break;
            }
            work.charge()?;
            scheduler.running.pop_first();
            scheduler.slots.insert(slot);
            scheduler.complete(node, at, &durations, &mut work)?;
        }
        while let Some(node) = scheduler.next() {
            work.charge()?;
            let duration = durations[node];
            let finish = at.checked_add(duration).ok_or_else(|| {
                budget_error("capacity schedule finish exceeds the u64 duration range")
            })?;
            let slot =
                if duration == 0 {
                    None
                } else {
                    Some(scheduler.slots.pop_first().ok_or_else(|| {
                        invariant("capacity schedule dispatched without a free slot")
                    })?)
                };
            let base = &baseline.entries[node];
            let ready_at = scheduler.ready_at[node];
            let queue_delay = at.checked_sub(ready_at).ok_or_else(|| {
                invariant("capacity schedule dispatched before prerequisites completed")
            })?;
            entries[node] = Some(CapacityScheduledDependency {
                entity_id: base.entity_id,
                generation: base.generation,
                revision: base.revision,
                duration,
                start: at,
                finish,
                ready_at,
                queue_delay,
                slot,
                downstream_duration: scheduler.downstream[node],
            });
            dispatch_order.push(base.entity_id);
            if let Some(slot) = slot {
                scheduler.running.insert((finish, slot, node));
                scheduler.frontier(&work)?;
            } else {
                scheduler.complete(node, at, &durations, &mut work)?;
            }
        }
        if scheduler.finished == n {
            break;
        }
        let next = scheduler.running.first().map(|&(finish, _, _)| finish);
        at = next
            .filter(|next| *next > at)
            .ok_or_else(|| invariant("capacity schedule stalled despite an acyclic baseline"))?;
    }
    let mut result_entries = Vec::with_capacity(n);
    for entry in entries {
        work.charge()?;
        result_entries.push(entry.ok_or_else(|| invariant("capacity schedule omitted a task"))?);
    }
    let mut projection = Vec::new();
    put_str(&mut projection, "dfmcp-dependency-capacity-projection/1");
    put_bytes(
        &mut projection,
        baseline.witness.projection_digest.as_bytes(),
    );
    put_u64(&mut projection, u64::from(max_parallel));
    let projection_digest = Digest32::of_bytes(&projection);
    let mut decisions = Vec::new();
    put_str(&mut decisions, CAPACITY_SCHEDULE_POLICY);
    put_bytes(&mut decisions, baseline.witness.decision_digest.as_bytes());
    put_bytes(&mut decisions, projection_digest.as_bytes());
    put_bytes(&mut decisions, scope_digest.as_bytes());
    put_u64(&mut decisions, at);
    put_u64(&mut decisions, dispatch_order.len() as u64);
    for id in &dispatch_order {
        work.charge()?;
        put_u64(&mut decisions, id.get());
    }
    put_u64(&mut decisions, result_entries.len() as u64);
    for entry in &result_entries {
        work.charge()?;
        put_u64(&mut decisions, entry.entity_id.get());
        for value in [
            entry.duration,
            entry.start,
            entry.finish,
            entry.ready_at,
            entry.queue_delay,
            entry.downstream_duration,
        ] {
            put_u64(&mut decisions, value);
        }
        match entry.slot {
            Some(slot) => {
                decisions.push(1);
                put_u64(&mut decisions, u64::from(slot));
            }
            None => decisions.push(0),
        }
    }
    put_u64(&mut decisions, work.used);
    put_u64(&mut decisions, work.examined_arcs);
    let witness = GraphWitness {
        source_anchor: baseline.witness.source_anchor,
        authorization_scope_digest: scope_digest,
        projection_digest,
        decision_digest: Digest32::of_bytes(&decisions),
        scanned_vertices: baseline.witness.scanned_vertices + n as u64,
        scanned_edges: baseline.witness.scanned_edges + snapshot.graph.edges.len() as u64,
        examined_arcs: work.examined_arcs,
        work_units: work.used,
    };
    (work.check)()?;
    Ok(CapacitySchedule {
        baseline,
        max_parallel,
        entries: result_entries,
        dispatch_order,
        makespan: at,
        witness,
    })
}
