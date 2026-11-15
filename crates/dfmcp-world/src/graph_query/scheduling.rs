//! Duration-aware scheduling over one authorized observed dependency projection.
//!
//! The prediction assumes independent capacity, fixed nonnegative durations and
//! only the selected dependent-to-prerequisite edges. It does not predict game
//! completion, reserve workers or resources, or authorize any effect.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::{
    DfmcpError, Digest32, EdgeKind, EntityId, ErrorCode, GraphBudget, GraphWitness, Index, Result,
    WorldSnapshot, budget_error, invariant, put_bytes, put_str, put_u64,
};
use crate::canonical::put_anchor;
use crate::evidence::laboratory_fact_value;
use crate::{Fact, FactSource, Value};

pub const DEPENDENCY_SCHEDULE_POLICY: &str = "observed-dependency-duration-schedule/1";
const MAX_TARGETS: usize = 64;
const MAX_DURATION_FIELD_BYTES: usize = 128;
const MAX_SOURCE_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyScheduleQuery {
    /// Explicit relation semantics: every selected edge is dependent -> prerequisite.
    pub edge_kinds: Vec<EdgeKind>,
    /// The same known nonnegative integer field on every scheduled vertex.
    /// Values use the field's declared units, not implicitly game ticks.
    pub duration_field: String,
    /// Empty schedules the entire authorized projection. Otherwise include these
    /// goals and their complete transitive prerequisite closure. Order and
    /// duplicate target IDs do not change the result.
    pub targets: Vec<EntityId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduledDependency {
    pub entity_id: EntityId,
    pub generation: u32,
    pub revision: u64,
    pub duration: u64,
    /// Relative offsets from model start; never absolute game clock values.
    pub earliest_start: u64,
    pub earliest_finish: u64,
    pub latest_start: u64,
    pub latest_finish: u64,
    /// Delay permitted without extending the complete selected model's makespan.
    pub total_slack: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScheduleFront {
    pub earliest_start: u64,
    /// Stable-ID order within this start-time group. Zero-duration dependencies
    /// can share a group: it is NOT an unordered dispatch batch. Use dependency_order.
    pub entities: Vec<EntityId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencySchedule {
    /// Canonical explicit targets; empty means the entire supplied projection.
    pub targets: Vec<EntityId>,
    /// Complete scheduled closure, in ascending stable entity identity.
    pub entries: Vec<ScheduledDependency>,
    /// Prerequisites first; ready-vertex ties use ascending stable entity identity.
    pub dependency_order: Vec<EntityId>,
    pub makespan: u64,
    /// One complete critical path, prerequisite first. Equal-finish terminal
    /// goals and equal-finish prerequisites choose the smallest stable ID.
    pub critical_chain: Vec<EntityId>,
    /// Earliest-start groups, ordered by offset then stable identity.
    pub parallel_fronts: Vec<ScheduleFront>,
    pub witness: GraphWitness,
}

struct Run<F> {
    index: Index,
    check: F,
    next_check: u64,
}

impl<F: FnMut() -> Result<()>> Run<F> {
    fn charge(&mut self) -> Result<()> {
        self.index.charge(1)?;
        if self.index.work >= self.next_check {
            (self.check)()?;
            self.next_check = self.index.work + 128;
        }
        Ok(())
    }

    fn arc(&mut self) -> Result<()> {
        self.charge()?;
        self.index.examined_arcs += 1;
        Ok(())
    }
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn unavailable(id: EntityId, field: &str) -> DfmcpError {
    DfmcpError::new(
        ErrorCode::PreconditionsFailed,
        format!(
            "dependency duration '{field}' on entity {id} requires a current, source-qualified nonnegative integer; no duration is inferred"
        ),
    )
}

/// This source-class check is NOT source admission. The caller must supply an
/// authentic, authorized projection. In particular a DFHack label/digest does
/// not establish compatibility or grant mutation-precondition eligibility.
fn duration(fact: &Fact, snapshot: &WorldSnapshot) -> Option<u64> {
    let value = if let Some(value) = laboratory_fact_value(fact, snapshot.tick) {
        value
    } else if fact.observed_at <= snapshot.tick
        && fact.source_digest != Digest32::ZERO
        && matches!(&fact.source, FactSource::DfhackField(name)
            if !name.trim().is_empty() && name.len() <= MAX_SOURCE_NAME_BYTES
                && !name.chars().any(char::is_control))
    {
        fact.known_value()?
    } else {
        return None;
    };
    match value {
        Value::U64(value) => Some(*value),
        Value::I64(value) => u64::try_from(*value).ok(),
        _ => None,
    }
}

pub fn schedule_dependencies(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &DependencyScheduleQuery,
    budget: GraphBudget,
) -> Result<DependencySchedule> {
    schedule_dependencies_with_check(snapshot, scope_digest, query, budget, || Ok(()))
}

/// Iterative critical-path scheduling with O(V + E) analysis work and auxiliary
/// space after bounded ordered-index construction (ordered ready/front groups
/// add O(V log V) comparisons). All vertices/edges in the supplied source are
/// admitted by the shared index, even when only a target closure is scheduled.
/// Cycles, missing/unavailable durations, overflow, exhausted bounds and caller
/// cancellation return no partial schedule. Parallel edges impose one semantic
/// precedence constraint and never add duration twice.
///
/// Source values are registered laboratory derivations or nonfuture, known
/// DFHack fields with a nonzero source digest. Replay, agent assertions and
/// unregistered derivations cannot supply durations. This checks provenance
/// shape, not bridge admission, field meaning, physical feasibility or freshness
/// beyond the selected snapshot. It confers no effect or predicate authority.
///
/// The callback runs before/after shared index construction, every 128 analysis
/// work units, and before return. Shared hash/index construction is count-bounded
/// but is not cooperatively interrupted internally by this callback.
pub fn schedule_dependencies_with_check<F: FnMut() -> Result<()>>(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &DependencyScheduleQuery,
    budget: GraphBudget,
    mut check: F,
) -> Result<DependencySchedule> {
    check()?;
    if query.edge_kinds.is_empty()
        || query.duration_field.trim().is_empty()
        || query.duration_field.len() > MAX_DURATION_FIELD_BYTES
        || query.duration_field.chars().any(char::is_control)
    {
        return Err(invalid(
            "dependency schedule requires explicit edge kinds and a 1..128-byte non-control duration field",
        ));
    }
    if query.targets.len() > MAX_TARGETS {
        return Err(budget_error("dependency schedule exceeds 64 target IDs"));
    }
    let index = Index::build(snapshot, &query.edge_kinds, budget)?;
    check()?;
    let next_check = index.work + 128;
    let mut run = Run {
        index,
        check,
        next_check,
    };
    let n = run.index.ids.len();
    let targets: Vec<_> = query
        .targets
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let selected = select_closure(&mut run, &targets)?;

    // Bind source anchor, duration field and exact facts as well as graph
    // structure. Numeric normalization must not erase provenance or presence.
    let mut projection = Vec::new();
    put_str(&mut projection, "dfmcp-dependency-schedule-projection/1");
    put_bytes(&mut projection, run.index.projection_digest.as_bytes());
    put_anchor(&mut projection, snapshot.anchor());
    put_str(&mut projection, &query.duration_field);
    put_u64(&mut projection, targets.len() as u64);
    for id in &targets {
        run.charge()?;
        put_u64(&mut projection, id.get());
    }
    let mut durations = vec![0; n];
    let mut scheduled = Vec::new();
    for node in 0..n {
        run.charge()?;
        if !selected[node] {
            continue;
        }
        let id = run.index.ids[node];
        let fact = snapshot
            .graph
            .entities
            .get(&id)
            .and_then(|entity| entity.fields.get(&query.duration_field))
            .ok_or_else(|| unavailable(id, &query.duration_field))?;
        durations[node] =
            duration(fact, snapshot).ok_or_else(|| unavailable(id, &query.duration_field))?;
        scheduled.push(node);
        put_u64(&mut projection, id.get());
        put_u64(&mut projection, durations[node]);
        put_bytes(&mut projection, &fact.canonical_bytes());
    }
    run.index.projection_digest = Digest32::of_bytes(&projection);

    let mut ready = BTreeSet::new();
    let mut pending = vec![0; n];
    for &node in &scheduled {
        run.charge()?;
        pending[node] = run.index.outgoing[node].len();
        if pending[node] == 0 {
            ready.insert(node);
            run.index.frontier(ready.len())?;
        }
    }
    let mut earliest_start = vec![0_u64; n];
    let mut earliest_finish = vec![0_u64; n];
    let mut predecessor = vec![None; n];
    let mut order = Vec::with_capacity(scheduled.len());
    while let Some(node) = ready.pop_first() {
        run.charge()?;
        earliest_finish[node] = earliest_start[node]
            .checked_add(durations[node])
            .ok_or_else(|| budget_error("dependency schedule exceeds the u64 duration range"))?;
        order.push(node);
        for offset in 0..run.index.incoming[node].len() {
            run.arc()?;
            let dependent = run.index.incoming[node][offset].target;
            if !selected[dependent] {
                continue;
            }
            if earliest_finish[node] > earliest_start[dependent]
                || (earliest_finish[node] == earliest_start[dependent]
                    && predecessor[dependent].is_none_or(|prior| node < prior))
            {
                earliest_start[dependent] = earliest_finish[node];
                predecessor[dependent] = Some(node);
            }
            pending[dependent] = pending[dependent]
                .checked_sub(1)
                .ok_or_else(|| invariant("schedule prerequisite count underflow"))?;
            if pending[dependent] == 0 {
                ready.insert(dependent);
                run.index.frontier(ready.len())?;
            }
        }
    }
    if order.len() != scheduled.len() {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "selected dependencies contain a cycle; inspect dependency analysis before scheduling",
        ));
    }

    let mut makespan = 0;
    for &node in &scheduled {
        run.charge()?;
        makespan = makespan.max(earliest_finish[node]);
    }
    let mut latest_start = vec![0_u64; n];
    let mut latest_finish = vec![makespan; n];
    let mut terminal = None;
    for &node in order.iter().rev() {
        run.charge()?;
        let mut has_dependent = false;
        let mut finish = makespan;
        for offset in 0..run.index.incoming[node].len() {
            run.arc()?;
            let dependent = run.index.incoming[node][offset].target;
            if selected[dependent] {
                has_dependent = true;
                finish = finish.min(latest_start[dependent]);
            }
        }
        latest_finish[node] = finish;
        latest_start[node] = finish
            .checked_sub(durations[node])
            .ok_or_else(|| invariant("schedule latest start precedes model origin"))?;
        if !has_dependent
            && earliest_finish[node] == makespan
            && terminal.is_none_or(|prior| node < prior)
        {
            terminal = Some(node);
        }
    }
    let mut critical_chain = Vec::new();
    let mut current = terminal;
    while let Some(node) = current {
        run.charge()?;
        if critical_chain.len() >= scheduled.len() {
            return Err(invariant(
                "schedule critical predecessor chain contains a cycle",
            ));
        }
        critical_chain.push(run.index.ids[node]);
        current = predecessor[node];
    }
    critical_chain.reverse();

    let mut decisions = Vec::new();
    put_str(&mut decisions, DEPENDENCY_SCHEDULE_POLICY);
    for bound in [
        budget.max_vertices as u64,
        budget.max_edges as u64,
        budget.max_frontier as u64,
        budget.max_work,
    ] {
        put_u64(&mut decisions, bound);
    }
    put_u64(&mut decisions, makespan);
    put_u64(&mut decisions, order.len() as u64);
    let mut dependency_order = Vec::with_capacity(order.len());
    for &node in &order {
        run.charge()?;
        dependency_order.push(run.index.ids[node]);
        put_u64(&mut decisions, run.index.ids[node].get());
    }
    let mut entries = Vec::with_capacity(scheduled.len());
    let mut fronts = BTreeMap::<u64, Vec<EntityId>>::new();
    put_u64(&mut decisions, scheduled.len() as u64);
    for &node in &scheduled {
        run.charge()?;
        let total_slack = latest_start[node]
            .checked_sub(earliest_start[node])
            .ok_or_else(|| invariant("schedule has negative total slack"))?;
        let id = run.index.ids[node];
        let entry = ScheduledDependency {
            entity_id: id,
            generation: run.index.versions[node].0,
            revision: run.index.versions[node].1,
            duration: durations[node],
            earliest_start: earliest_start[node],
            earliest_finish: earliest_finish[node],
            latest_start: latest_start[node],
            latest_finish: latest_finish[node],
            total_slack,
        };
        put_u64(&mut decisions, id.get());
        for value in [
            entry.duration,
            entry.earliest_start,
            entry.earliest_finish,
            entry.latest_start,
            entry.latest_finish,
            entry.total_slack,
        ] {
            put_u64(&mut decisions, value);
        }
        match predecessor[node] {
            Some(prior) => {
                decisions.push(1);
                put_u64(&mut decisions, run.index.ids[prior].get());
            }
            None => decisions.push(0),
        }
        let front = fronts.entry(entry.earliest_start).or_default();
        front.push(id);
        run.index.frontier(front.len())?;
        entries.push(entry);
    }
    put_u64(&mut decisions, critical_chain.len() as u64);
    for id in &critical_chain {
        run.charge()?;
        put_u64(&mut decisions, id.get());
    }
    let mut parallel_fronts = Vec::with_capacity(fronts.len());
    for (earliest_start, entities) in fronts {
        run.charge()?;
        parallel_fronts.push(ScheduleFront {
            earliest_start,
            entities,
        });
    }
    let witness = run.index.witness(snapshot, scope_digest, &decisions);
    (run.check)()?;
    Ok(DependencySchedule {
        targets,
        entries,
        dependency_order,
        makespan,
        critical_chain,
        parallel_fronts,
        witness,
    })
}

fn select_closure<F: FnMut() -> Result<()>>(
    run: &mut Run<F>,
    targets: &[EntityId],
) -> Result<Vec<bool>> {
    let n = run.index.ids.len();
    if targets.is_empty() {
        return Ok(vec![true; n]);
    }
    let mut selected = vec![false; n];
    let mut queue = VecDeque::new();
    for id in targets {
        run.charge()?;
        let node = run.index.ordinals.get(id).copied().ok_or_else(|| {
            invalid("schedule target is not present in the authorized projection")
        })?;
        selected[node] = true;
        queue.push_back(node);
        run.index.frontier(queue.len())?;
    }
    while let Some(node) = queue.pop_front() {
        run.charge()?;
        for offset in 0..run.index.outgoing[node].len() {
            run.arc()?;
            let prerequisite = run.index.outgoing[node][offset].target;
            if !selected[prerequisite] {
                selected[prerequisite] = true;
                queue.push_back(prerequisite);
                run.index.frontier(queue.len())?;
            }
        }
    }
    Ok(selected)
}
