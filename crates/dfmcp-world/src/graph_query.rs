//! Bounded, deterministic cognition over an already-authorized world projection.
//!
//! These analyses describe observed edges, not game walkability, resource
//! feasibility, or complete-world absence. They carry no mutation authority.
//! Callers must remove unauthorized vertices and edges before constructing the
//! snapshot passed here. The supplied scope digest identifies that decision.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, Result, StateAnchor};

use crate::canonical::{put_bytes, put_str, put_u32, put_u64};
use crate::{EdgeKind, WorldSnapshot};

const MAX_VERTICES: usize = 100_000;
const MAX_EDGES: usize = 1_000_000;
const MAX_WORK: u64 = 10_000_000;
const MAX_DEPTH: u32 = 1_024;
const MAX_KINDS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphBudget {
    pub max_vertices: usize,
    pub max_edges: usize,
    pub max_frontier: usize,
    pub max_work: u64,
}

impl Default for GraphBudget {
    fn default() -> Self {
        Self {
            max_vertices: 10_000,
            max_edges: 100_000,
            max_frontier: 10_000,
            max_work: 1_000_000,
        }
    }
}

impl GraphBudget {
    fn validate(self) -> Result<()> {
        if self.max_vertices == 0
            || self.max_vertices > MAX_VERTICES
            || self.max_edges > MAX_EDGES
            || self.max_frontier == 0
            || self.max_frontier > MAX_VERTICES
            || self.max_work == 0
            || self.max_work > MAX_WORK
        {
            return Err(budget_error("graph budget is outside the implementation bounds"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphDirection {
    Outgoing,
    Incoming,
    Undirected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphTraversalQuery {
    pub roots: Vec<EntityId>,
    pub edge_kinds: Vec<EdgeKind>,
    pub direction: GraphDirection,
    pub max_depth: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphVisit {
    pub entity_id: EntityId,
    pub generation: u32,
    pub revision: u64,
    pub root: EntityId,
    pub depth: u32,
    pub parent: Option<EntityId>,
    pub via_edge: Option<EdgeId>,
    pub via_edge_revision: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphWitness {
    pub source_anchor: StateAnchor,
    pub authorization_scope_digest: Digest32,
    pub projection_digest: Digest32,
    pub decision_digest: Digest32,
    pub scanned_vertices: u64,
    pub scanned_edges: u64,
    pub examined_arcs: u64,
    pub work_units: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphTraversal {
    /// Breadth-first discovery order; roots and adjacency ties use stable IDs.
    pub visits: Vec<GraphVisit>,
    /// Depth-bound vertices with an observed neighbor outside the visited set.
    pub depth_frontier: Vec<EntityId>,
    pub witness: GraphWitness,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObservedGraphPath {
    pub vertices: Vec<EntityId>,
    pub edges: Vec<(EdgeId, u64)>,
}

impl GraphTraversal {
    /// Reconstruct a recorded shortest path. None means this traversal did not
    /// visit the target, not that a path cannot exist in the game world.
    pub fn observed_path_to(&self, target: EntityId) -> Result<Option<ObservedGraphPath>> {
        if self.visits.len() > MAX_VERTICES {
            return Err(budget_error("path reconstruction exceeds the vertex bound"));
        }
        let records: BTreeMap<_, _> = self
            .visits
            .iter()
            .map(|visit| (visit.entity_id, visit))
            .collect();
        if records.len() != self.visits.len() {
            return Err(invariant("path witness contains duplicate vertices"));
        }
        let Some(mut current) = records.get(&target).copied() else {
            return Ok(None);
        };
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        for _ in 0..self.visits.len() {
            vertices.push(current.entity_id);
            match (current.parent, current.via_edge, current.via_edge_revision) {
                (None, None, None) if current.depth == 0 && current.root == current.entity_id => {
                    vertices.reverse();
                    edges.reverse();
                    return Ok(Some(ObservedGraphPath { vertices, edges }));
                }
                (Some(parent), Some(edge), Some(revision)) => {
                    let parent = records.get(&parent).copied().ok_or_else(|| {
                        invariant("path witness references an unvisited parent")
                    })?;
                    if parent.depth.checked_add(1) != Some(current.depth)
                        || parent.root != current.root
                    {
                        return Err(invariant("path witness parent depth or source is inconsistent"));
                    }
                    edges.push((edge, revision));
                    current = parent;
                }
                _ => return Err(invariant("path witness has inconsistent parent/edge fields")),
            }
        }
        Err(invariant("path witness contains a parent cycle"))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyComponent {
    /// The minimum stable entity ID in this strongly connected component.
    pub id: EntityId,
    pub members: Vec<EntityId>,
    pub cyclic: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DependencyAnalysis {
    /// Condensation order: prerequisites before dependents, stable-ID ties.
    pub components: Vec<DependencyComponent>,
    /// Includes cycle members and every observed dependent of a cycle.
    pub blocked_by_cycles: Vec<EntityId>,
    /// Absent when cycles prevent a valid entity-level prerequisite order.
    pub dependency_order: Option<Vec<EntityId>>,
    /// Longest unweighted prerequisite chain, dependent first. Not a time estimate.
    pub critical_chain: Option<Vec<EntityId>>,
    pub witness: GraphWitness,
}

#[derive(Clone, Copy)]
struct Arc {
    target: usize,
    edge: EdgeId,
    revision: u64,
}

struct Index {
    ids: Vec<EntityId>,
    versions: Vec<(u32, u64)>,
    ordinals: BTreeMap<EntityId, usize>,
    outgoing: Vec<Vec<Arc>>,
    incoming: Vec<Vec<Arc>>,
    projection_digest: Digest32,
    budget: GraphBudget,
    work: u64,
    examined_arcs: u64,
    scanned_edges: u64,
}

fn budget_error(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

impl Index {
    fn build(snapshot: &WorldSnapshot, kinds: &[EdgeKind], budget: GraphBudget) -> Result<Self> {
        budget.validate()?;
        if snapshot.graph.entities.len() > budget.max_vertices
            || snapshot.graph.edges.len() > budget.max_edges
            || kinds.len() > MAX_KINDS
            || kinds.iter().any(|kind| kind.as_str().len() > 128)
        {
            return Err(budget_error("graph projection exceeds its input bounds"));
        }
        if !snapshot.hash_is_valid() {
            return Err(invariant("graph source snapshot has an invalid state hash"));
        }
        let mut index = Self {
            ids: Vec::new(),
            versions: Vec::new(),
            ordinals: BTreeMap::new(),
            outgoing: Vec::new(),
            incoming: Vec::new(),
            projection_digest: Digest32::ZERO,
            budget,
            work: 0,
            examined_arcs: 0,
            scanned_edges: 0,
        };
        let selected: BTreeSet<_> = kinds.iter().cloned().collect();
        let mut canonical = Vec::new();
        put_str(&mut canonical, "dfmcp-authorized-graph-projection-v1");
        put_u64(&mut canonical, selected.len() as u64);
        for kind in &selected {
            canonical.push(u8::from(matches!(kind, EdgeKind::Custom(_))));
            put_str(&mut canonical, kind.as_str());
        }
        put_u64(&mut canonical, snapshot.graph.entities.len() as u64);
        for (&id, entity) in &snapshot.graph.entities {
            index.charge(1)?;
            if id == EntityId::NIL || entity.id != id {
                return Err(invariant("graph vertex key is inconsistent with its stable identity"));
            }
            index.ordinals.insert(id, index.ids.len());
            index.ids.push(id);
            index.versions.push((entity.generation, entity.revision));
            index.outgoing.push(Vec::new());
            index.incoming.push(Vec::new());
            put_u64(&mut canonical, id.get());
            put_u32(&mut canonical, entity.generation);
            put_u64(&mut canonical, entity.revision);
        }
        for (&id, edge) in &snapshot.graph.edges {
            index.charge(1)?;
            index.scanned_edges += 1;
            if !selected.is_empty() && !selected.contains(&edge.kind) {
                continue;
            }
            if edge.kind.as_str().len() > 128 {
                return Err(budget_error("selected graph edge kind exceeds its byte bound"));
            }
            if id != edge.id || edge.id == EdgeId::NIL {
                return Err(invariant("graph edge key is inconsistent with its stable identity"));
            }
            let (Some(&from), Some(&to)) =
                (index.ordinals.get(&edge.from), index.ordinals.get(&edge.to))
            else {
                return Err(invariant("selected graph edge has an unobserved endpoint"));
            };
            index.outgoing[from].push(Arc { target: to, edge: id, revision: edge.revision });
            index.incoming[to].push(Arc { target: from, edge: id, revision: edge.revision });
            canonical.extend_from_slice(&id.get().to_be_bytes());
            put_u64(&mut canonical, edge.revision);
            canonical.push(u8::from(matches!(&edge.kind, EdgeKind::Custom(_))));
            put_str(&mut canonical, edge.kind.as_str());
            put_u64(&mut canonical, edge.from.get());
            put_u64(&mut canonical, edge.to.get());
        }
        for neighbors in index.outgoing.iter_mut().chain(index.incoming.iter_mut()) {
            neighbors.sort_by_key(|arc| (arc.target, arc.edge));
        }
        index.projection_digest = Digest32::of_bytes(&canonical);
        Ok(index)
    }

    fn charge(&mut self, units: u64) -> Result<()> {
        self.work = self.work.checked_add(units)
            .ok_or_else(|| budget_error("graph work counter overflow"))?;
        if self.work > self.budget.max_work {
            return Err(budget_error("graph traversal exhausted its operation budget"));
        }
        Ok(())
    }

    fn arc(&mut self) -> Result<()> {
        self.charge(1)?;
        self.examined_arcs += 1;
        Ok(())
    }

    fn frontier(&self, size: usize) -> Result<()> {
        if size > self.budget.max_frontier {
            return Err(budget_error("graph traversal exhausted its frontier budget"));
        }
        Ok(())
    }

    fn witness(&self, snapshot: &WorldSnapshot, scope: Digest32, decisions: &[u8]) -> GraphWitness {
        let mut bytes = Vec::new();
        put_str(&mut bytes, "dfmcp-graph-decision-v1");
        put_bytes(&mut bytes, self.projection_digest.as_bytes());
        put_bytes(&mut bytes, scope.as_bytes());
        put_bytes(&mut bytes, decisions);
        put_u64(&mut bytes, self.work);
        put_u64(&mut bytes, self.examined_arcs);
        GraphWitness {
            source_anchor: snapshot.anchor(),
            authorization_scope_digest: scope,
            projection_digest: self.projection_digest,
            decision_digest: Digest32::of_bytes(&bytes),
            scanned_vertices: self.ids.len() as u64,
            scanned_edges: self.scanned_edges,
            examined_arcs: self.examined_arcs,
            work_units: self.work,
        }
    }
}

/// Canonical multi-source, unweighted shortest-path traversal. Budget exhaustion
/// is an error, never a partial negative answer. A depth cutoff is explicit.
pub fn traverse_graph(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &GraphTraversalQuery,
    budget: GraphBudget,
) -> Result<GraphTraversal> {
    if query.roots.is_empty()
        || query.roots.len() > budget.max_vertices
        || query.max_depth > MAX_DEPTH
    {
        return Err(budget_error("graph roots or depth exceed the traversal bounds"));
    }
    let mut index = Index::build(snapshot, &query.edge_kinds, budget)?;
    let roots: BTreeSet<_> = query.roots.iter().copied().collect();
    let mut visits: Vec<GraphVisit> = Vec::new();
    let mut seen = vec![false; index.ids.len()];
    let mut queue = VecDeque::new();
    let mut boundary = Vec::new();
    for root in &roots {
        let ordinal = index.ordinals.get(root).copied().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                "graph root is not present in the authorized projection",
            )
        })?;
        index.charge(1)?;
        index.frontier(queue.len() + 1)?;
        seen[ordinal] = true;
        let (generation, revision) = index.versions[ordinal];
        visits.push(GraphVisit {
            entity_id: *root,
            generation,
            revision,
            root: *root,
            depth: 0,
            parent: None,
            via_edge: None,
            via_edge_revision: None,
        });
        queue.push_back((ordinal, visits.len() - 1));
    }
    while let Some((current, visit_index)) = queue.pop_front() {
        index.charge(1)?;
        let depth = visits[visit_index].depth;
        let root = visits[visit_index].root;
        if depth == query.max_depth {
            boundary.push(current);
            continue;
        }
        for arc in neighbors(&index, current, query.direction) {
            index.arc()?;
            if seen[arc.target] {
                continue;
            }
            index.frontier(queue.len() + 1)?;
            seen[arc.target] = true;
            let (generation, revision) = index.versions[arc.target];
            visits.push(GraphVisit {
                entity_id: index.ids[arc.target],
                generation,
                revision,
                root,
                depth: depth + 1,
                parent: Some(index.ids[current]),
                via_edge: Some(arc.edge),
                via_edge_revision: Some(arc.revision),
            });
            queue.push_back((arc.target, visits.len() - 1));
        }
    }
    let mut depth_frontier = Vec::new();
    for current in boundary {
        for arc in neighbors(&index, current, query.direction) {
            index.arc()?;
            if !seen[arc.target] {
                depth_frontier.push(index.ids[current]);
                break;
            }
        }
    }
    depth_frontier.sort();
    let mut decisions = Vec::new();
    put_str(&mut decisions, "multi-source-bfs-stable-id-v1");
    decisions.push(match query.direction {
        GraphDirection::Outgoing => 0,
        GraphDirection::Incoming => 1,
        GraphDirection::Undirected => 2,
    });
    put_u32(&mut decisions, query.max_depth);
    put_u64(&mut decisions, visits.len() as u64);
    for visit in &visits {
        put_u64(&mut decisions, visit.entity_id.get());
        put_u64(&mut decisions, visit.root.get());
        put_u32(&mut decisions, visit.depth);
        put_u64(&mut decisions, visit.parent.map_or(0, EntityId::get));
        decisions.extend_from_slice(&visit.via_edge.map_or(0, EdgeId::get).to_be_bytes());
    }
    put_u64(&mut decisions, depth_frontier.len() as u64);
    for id in &depth_frontier {
        put_u64(&mut decisions, id.get());
    }
    let witness = index.witness(snapshot, scope_digest, &decisions);
    Ok(GraphTraversal { visits, depth_frontier, witness })
}

fn neighbors(index: &Index, node: usize, direction: GraphDirection) -> Vec<Arc> {
    match direction {
        GraphDirection::Outgoing => index.outgoing[node].clone(),
        GraphDirection::Incoming => index.incoming[node].clone(),
        GraphDirection::Undirected => {
            let mut arcs = index.outgoing[node].clone();
            arcs.extend_from_slice(&index.incoming[node]);
            arcs.sort_by_key(|arc| (arc.target, arc.edge));
            arcs.dedup_by_key(|arc| (arc.target, arc.edge));
            arcs
        }
    }
}

/// Analyze edges interpreted as dependent -> prerequisite. SCCs identify cycles;
/// the condensation is ordered prerequisite-first, including disconnected nodes.
/// The optional critical chain uses one unit per edge, not game-time durations.
pub fn analyze_dependencies(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    edge_kinds: &[EdgeKind],
    budget: GraphBudget,
) -> Result<DependencyAnalysis> {
    let mut index = Index::build(snapshot, edge_kinds, budget)?;
    let n = index.ids.len();
    let mut seen = vec![false; n];
    let mut finished = Vec::new();
    for start in 0..n {
        if seen[start] {
            continue;
        }
        seen[start] = true;
        let mut stack = vec![(start, 0usize)];
        while let Some((node, next)) = stack.last_mut() {
            index.charge(1)?;
            if let Some(arc) = index.outgoing[*node].get(*next).copied() {
                *next += 1;
                index.arc()?;
                if !seen[arc.target] {
                    index.frontier(stack.len() + 1)?;
                    seen[arc.target] = true;
                    stack.push((arc.target, 0));
                }
            } else {
                finished.push(*node);
                stack.pop();
            }
        }
    }
    let mut assignment = vec![usize::MAX; n];
    let mut components = Vec::new();
    for start in finished.into_iter().rev() {
        if assignment[start] != usize::MAX {
            continue;
        }
        let component = components.len();
        let mut members = Vec::new();
        let mut queue = VecDeque::from([start]);
        assignment[start] = component;
        while let Some(node) = queue.pop_front() {
            index.charge(1)?;
            members.push(index.ids[node]);
            for arc in index.incoming[node].clone() {
                index.arc()?;
                if assignment[arc.target] == usize::MAX {
                    index.frontier(queue.len() + 1)?;
                    assignment[arc.target] = component;
                    queue.push_back(arc.target);
                }
            }
        }
        members.sort();
        let cyclic = members.len() > 1
            || index.outgoing[start].iter().any(|arc| arc.target == start);
        components.push(DependencyComponent { id: members[0], members, cyclic });
    }
    let count = components.len();
    let mut prerequisites = vec![BTreeSet::new(); count];
    let mut dependents = vec![BTreeSet::new(); count];
    for from in 0..n {
        for arc in index.outgoing[from].clone() {
            index.arc()?;
            let (source, target) = (assignment[from], assignment[arc.target]);
            if source != target {
                prerequisites[source].insert(target);
                dependents[target].insert(source);
            }
        }
    }
    let mut pending: Vec<_> = prerequisites.iter().map(BTreeSet::len).collect();
    let mut ready = BTreeSet::new();
    for (i, &degree) in pending.iter().enumerate() {
        if degree == 0 {
            ready.insert((components[i].id, i));
        }
    }
    index.frontier(ready.len())?;
    let mut order = Vec::new();
    let mut depths = vec![0usize; count];
    let mut blocked: Vec<_> = components.iter().map(|component| component.cyclic).collect();
    while let Some((_, component)) = ready.pop_first() {
        index.charge(1)?;
        order.push(component);
        let component_blocked = blocked[component];
        let next_depth = depths[component] + 1;
        for &dependent in &dependents[component] {
            index.arc()?;
            blocked[dependent] |= component_blocked;
            depths[dependent] = depths[dependent].max(next_depth);
            pending[dependent] = pending[dependent].checked_sub(1).ok_or_else(|| {
                invariant("dependency count underflow in condensation")
            })?;
            if pending[dependent] == 0 {
                ready.insert((components[dependent].id, dependent));
                index.frontier(ready.len())?;
            }
        }
    }
    if order.len() != count {
        return Err(invariant("SCC condensation unexpectedly contains a cycle"));
    }
    let acyclic = components.iter().all(|component| !component.cyclic);
    let dependency_order = acyclic.then(|| order.iter().map(|&i| components[i].id).collect());
    let critical_chain = if acyclic {
        let mut chain = Vec::new();
        index.charge(count as u64)?;
        let mut current = (0..count)
            .min_by_key(|&i| (std::cmp::Reverse(depths[i]), components[i].id));
        while let Some(node) = current {
            index.charge(1)?;
            chain.push(components[node].id);
            index.charge(prerequisites[node].len() as u64)?;
            current = prerequisites[node].iter().copied()
                .min_by_key(|&i| (std::cmp::Reverse(depths[i]), components[i].id));
        }
        Some(chain)
    } else {
        None
    };
    let blocked_by_cycles: Vec<_> = (0..n)
        .filter(|&i| blocked[assignment[i]])
        .map(|i| index.ids[i])
        .collect();
    let components: Vec<_> = order.into_iter().map(|i| components[i].clone()).collect();
    let mut decisions = Vec::new();
    put_str(&mut decisions, "scc-prerequisite-first-stable-id-v1");
    put_u64(&mut decisions, components.len() as u64);
    for component in &components {
        put_u64(&mut decisions, component.id.get());
        decisions.push(u8::from(component.cyclic));
        put_u64(&mut decisions, component.members.len() as u64);
        for id in &component.members {
            put_u64(&mut decisions, id.get());
        }
    }
    put_u64(&mut decisions, blocked_by_cycles.len() as u64);
    for id in &blocked_by_cycles {
        put_u64(&mut decisions, id.get());
    }
    match &critical_chain {
        Some(chain) => {
            decisions.push(1);
            put_u64(&mut decisions, chain.len() as u64);
            for id in chain {
                put_u64(&mut decisions, id.get());
            }
        }
        None => decisions.push(0),
    }
    let witness = index.witness(snapshot, scope_digest, &decisions);
    Ok(DependencyAnalysis {
        components, blocked_by_cycles, dependency_order, critical_chain, witness,
    })
}
