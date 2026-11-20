//! Least-cost routes over one already-authorized observed relation projection.
//!
//! Costs are nonnegative integer source facts in the named field's units. A
//! route describes observed relations; it is not game movement, capacity,
//! safety, complete-world absence, or authority to perform an effect.

use std::collections::{BTreeMap, BTreeSet};

use super::{
    DfmcpError, Digest32, EdgeId, EdgeKind, EntityId, ErrorCode, GraphBudget, GraphDirection,
    GraphWitness, Index, MAX_VERTICES, Result, WorldSnapshot, budget_error, invariant, put_bytes,
    put_str, put_u32, put_u64,
};
use crate::canonical::put_anchor;
use crate::evidence::laboratory_fact_value;
use crate::{Fact, FactSource, Value};

pub const WEIGHTED_ROUTE_POLICY: &str = "observed-weighted-routes-cost-hops-source-parent-edge/1";
const MAX_ROOTS: usize = 64;
const MAX_FIELD_BYTES: usize = 128;
const MAX_SOURCE_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeightedRouteQuery {
    /// At least one observed source. Duplicate IDs and source order do not
    /// change the core result; request parsers may require distinct handles.
    pub roots: Vec<EntityId>,
    /// Explicit edge semantics are required. Every selected edge needs a cost,
    /// including edges in unreachable components; unknown costs are not zero.
    pub edge_kinds: Vec<EdgeKind>,
    /// Outgoing follows stored edges; Incoming reverses them. Undirected refuses.
    pub direction: GraphDirection,
    pub cost_field: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeightedRouteRecord {
    pub entity_id: EntityId,
    pub generation: u32,
    pub revision: u64,
    pub root: EntityId,
    pub total_cost: u64,
    pub hops: u32,
    pub parent: Option<EntityId>,
    pub via_edge: Option<EdgeId>,
    pub via_edge_revision: Option<u64>,
    pub edge_cost: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeightedRoutePath {
    pub root: EntityId,
    pub total_cost: u64,
    pub vertices: Vec<EntityId>,
    /// Source-to-target traversal order; each tuple is (stable edge, revision,
    /// cost). Incoming traversal uses the opposite of the edge's stored direction.
    pub edges: Vec<(EdgeId, u64, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WeightedRoutes {
    pub roots: Vec<EntityId>,
    pub direction: GraphDirection,
    /// Reachable vertices in stable identity order, not settlement order.
    pub entries: Vec<WeightedRouteRecord>,
    /// Observed vertices outside every selected source's reachable set. This
    /// proves no path only in this projection, never in the complete game.
    pub unreachable: Vec<EntityId>,
    pub witness: GraphWitness,
}

impl WeightedRoutes {
    /// Reconstruct one canonical route without trusting mutable public records.
    /// None means the target was not reached, not that it is absent from the game.
    pub fn path_to(&self, target: EntityId) -> Result<Option<WeightedRoutePath>> {
        self.paths_to(&[target])?
            .pop()
            .ok_or_else(|| invariant("weighted path reconstruction omitted its requested target"))
    }

    /// Batch reconstruction validates the stable record order once and then
    /// binary-searches the immutable records, avoiding one index build per path.
    /// Results preserve the requested target order, including duplicates.
    pub fn paths_to(&self, targets: &[EntityId]) -> Result<Vec<Option<WeightedRoutePath>>> {
        self.paths_to_with_check(targets, || Ok(()))
    }

    /// O(V + P log V) work and O(P) returned storage for P total route vertices.
    /// At most 64 targets and 100,000 records are accepted. The caller must also
    /// budget P and rendered output before reconstruction. Its check runs before
    /// validation, every 128 inspected/path records and before publication.
    pub fn paths_to_with_check<F: FnMut() -> Result<()>>(
        &self,
        targets: &[EntityId],
        mut check: F,
    ) -> Result<Vec<Option<WeightedRoutePath>>> {
        check()?;
        if self.entries.len() > MAX_VERTICES
            || self.roots.len() > MAX_ROOTS
            || targets.len() > MAX_ROOTS
        {
            return Err(budget_error(
                "weighted path reconstruction exceeds its record bound",
            ));
        }
        let mut work = 0usize;
        let mut previous = None;
        for record in &self.entries {
            reconstruction_check(&mut work, &mut check)?;
            if record.entity_id == EntityId::NIL
                || previous.is_some_and(|id| record.entity_id <= id)
            {
                return Err(invariant(
                    "weighted route records must have unique ascending nonzero identities",
                ));
            }
            previous = Some(record.entity_id);
        }
        let mut paths = Vec::with_capacity(targets.len());
        for target in targets {
            reconstruction_check(&mut work, &mut check)?;
            paths.push(self.reconstruct_path(*target, &mut work, &mut check)?);
        }
        check()?;
        Ok(paths)
    }

    fn reconstruct_path<F: FnMut() -> Result<()>>(
        &self,
        target: EntityId,
        work: &mut usize,
        check: &mut F,
    ) -> Result<Option<WeightedRoutePath>> {
        let lookup = |id| {
            self.entries
                .binary_search_by_key(&id, |entry| entry.entity_id)
                .ok()
                .map(|index| &self.entries[index])
        };
        let Some(mut current) = lookup(target) else {
            return Ok(None);
        };
        let root = current.root;
        let total_cost = current.total_cost;
        let mut vertices = Vec::new();
        let mut edges = Vec::new();
        for _ in 0..self.entries.len() {
            reconstruction_check(work, check)?;
            vertices.push(current.entity_id);
            match (
                current.parent,
                current.via_edge,
                current.via_edge_revision,
                current.edge_cost,
            ) {
                (None, None, None, None)
                    if current.entity_id == root
                        && current.root == root
                        && current.hops == 0
                        && current.total_cost == 0
                        && self.roots.contains(&root) =>
                {
                    vertices.reverse();
                    edges.reverse();
                    return Ok(Some(WeightedRoutePath {
                        root,
                        total_cost,
                        vertices,
                        edges,
                    }));
                }
                (Some(parent), Some(edge), Some(revision), Some(cost)) => {
                    let parent = lookup(parent).ok_or_else(|| {
                        invariant("weighted route witness references an unvisited parent")
                    })?;
                    if parent.root != root
                        || parent.hops.checked_add(1) != Some(current.hops)
                        || parent.total_cost.checked_add(cost) != Some(current.total_cost)
                        || edge == EdgeId::NIL
                    {
                        return Err(invariant(
                            "weighted route witness has inconsistent cost, depth or source",
                        ));
                    }
                    edges.push((edge, revision, cost));
                    current = parent;
                }
                _ => {
                    return Err(invariant(
                        "weighted route witness has inconsistent parent fields",
                    ));
                }
            }
        }
        Err(invariant("weighted route witness contains a parent cycle"))
    }
}

fn reconstruction_check<F: FnMut() -> Result<()>>(work: &mut usize, check: &mut F) -> Result<()> {
    *work += 1;
    if (*work).is_multiple_of(128) {
        check()?;
    }
    Ok(())
}

// Field order is the closed tie policy: cost, hops, source, predecessor, edge.
// A chosen predecessor has strictly fewer hops even when its edge costs zero.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Label {
    cost: u64,
    hops: u32,
    root: usize,
    parent: Option<usize>,
    edge: Option<EdgeId>,
    edge_revision: Option<u64>,
    edge_cost: Option<u64>,
}

impl Label {
    fn queue_key(self, node: usize) -> (u64, u32, usize, usize) {
        (self.cost, self.hops, self.root, node)
    }
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

fn unavailable(id: EdgeId, field: &str) -> DfmcpError {
    DfmcpError::new(
        ErrorCode::PreconditionsFailed,
        format!(
            "route cost '{field}' on edge {id} requires a current, source-qualified nonnegative integer; no cost is inferred"
        ),
    )
}

// Source-class checking cannot establish compatibility or source admission.
// The caller owns the authentic, authorized observation supplied here.
fn cost(fact: &Fact, snapshot: &WorldSnapshot) -> Option<u64> {
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

pub fn shortest_routes(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &WeightedRouteQuery,
    budget: GraphBudget,
) -> Result<WeightedRoutes> {
    shortest_routes_with_check(snapshot, scope_digest, query, budget, || Ok(()))
}

/// Bounded multi-source Dijkstra with a single live ordered queue entry per
/// unsettled vertex. O((V + E) log(V + E)) time including ordered edge-cost
/// lookup (the priority queue itself uses O(log V) comparisons per operation);
/// O(V + E) auxiliary state. No path enumeration or depth recursion is used.
///
/// Every edge of the selected kinds must carry a consistently Known, nonfuture
/// u64 or nonnegative i64 cost from a registered laboratory source or a bounded
/// DFHack field with a nonzero source digest. Missing/unqualified weights,
/// overflow, invalid identity, cancellation and budget exhaustion return no
/// partial result. No meaning or unit is inferred from the field name.
///
/// Ties minimize total cost, then hop count, then source identity, then immediate
/// predecessor identity and edge identity. This is a canonical predecessor-tree
/// rule, not lexicographic comparison of complete paths. The queue settles equal
/// labels by vertex identity. Zero-cost cycles cannot create cyclic parents.
///
/// The caller check runs before/after shared index construction, every 128
/// analysis work units and before return. Shared hashing/index construction is
/// count-bounded but is not cooperatively interrupted by this callback.
pub fn shortest_routes_with_check<F: FnMut() -> Result<()>>(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &WeightedRouteQuery,
    budget: GraphBudget,
    mut check: F,
) -> Result<WeightedRoutes> {
    check()?;
    if query.roots.is_empty()
        || query.edge_kinds.is_empty()
        || query.direction == GraphDirection::Undirected
        || query.cost_field.trim().is_empty()
        || query.cost_field.len() > MAX_FIELD_BYTES
        || query.cost_field.chars().any(char::is_control)
    {
        return Err(invalid(
            "weighted routes require sources, explicit edge kinds, directed traversal and a 1..128-byte non-control cost field",
        ));
    }
    if query.roots.len() > MAX_ROOTS {
        return Err(budget_error("weighted routes exceed 64 source IDs"));
    }
    let index = Index::build(snapshot, &query.edge_kinds, budget)?;
    check()?;
    let next_check = index.work + 128;
    let mut run = Run {
        index,
        check,
        next_check,
    };
    let roots: Vec<_> = query
        .roots
        .iter()
        .copied()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let mut source_nodes = Vec::with_capacity(roots.len());
    for id in &roots {
        run.charge()?;
        source_nodes.push(run.index.ordinals.get(id).copied().ok_or_else(|| {
            invalid("weighted route source is not present in the authorized projection")
        })?);
    }

    let selected: BTreeSet<_> = query.edge_kinds.iter().collect();
    let mut weights = BTreeMap::new();
    let mut projection = Vec::new();
    put_str(&mut projection, "dfmcp-weighted-route-projection/1");
    put_bytes(&mut projection, run.index.projection_digest.as_bytes());
    put_anchor(&mut projection, snapshot.anchor());
    put_str(&mut projection, &query.cost_field);
    for (&id, edge) in &snapshot.graph.edges {
        run.charge()?;
        if !selected.contains(&edge.kind) {
            continue;
        }
        let fact = edge
            .fields
            .get(&query.cost_field)
            .ok_or_else(|| unavailable(id, &query.cost_field))?;
        let weight = cost(fact, snapshot).ok_or_else(|| unavailable(id, &query.cost_field))?;
        weights.insert(id, weight);
        projection.extend_from_slice(&id.get().to_be_bytes());
        put_u64(&mut projection, weight);
        put_bytes(&mut projection, &fact.canonical_bytes());
    }
    run.index.projection_digest = Digest32::of_bytes(&projection);
    if query.direction == GraphDirection::Incoming {
        std::mem::swap(&mut run.index.outgoing, &mut run.index.incoming);
    }

    let n = run.index.ids.len();
    let mut labels: Vec<Option<Label>> = vec![None; n];
    let mut settled = vec![false; n];
    let mut frontier = BTreeSet::new();
    for node in source_nodes {
        run.charge()?;
        let label = Label {
            cost: 0,
            hops: 0,
            root: node,
            parent: None,
            edge: None,
            edge_revision: None,
            edge_cost: None,
        };
        labels[node] = Some(label);
        frontier.insert(label.queue_key(node));
        run.index.frontier(frontier.len())?;
    }
    while let Some((_, _, _, node)) = frontier.pop_first() {
        run.charge()?;
        let current = labels[node].ok_or_else(|| invariant("weighted route queue has no label"))?;
        if settled[node] {
            return Err(invariant("weighted route settled a vertex more than once"));
        }
        settled[node] = true;
        for offset in 0..run.index.outgoing[node].len() {
            run.arc()?;
            let arc = run.index.outgoing[node][offset];
            let weight = weights
                .get(&arc.edge)
                .copied()
                .ok_or_else(|| invariant("weighted route edge has no admitted cost"))?;
            // Even a non-improving reachable alternative is checked. Refusing
            // an unrepresentable supplied model is safer than wrapping or
            // silently dropping it and claiming complete least-cost coverage.
            let total_cost = current
                .cost
                .checked_add(weight)
                .ok_or_else(|| budget_error("weighted route cost exceeds u64"))?;
            let hops = current
                .hops
                .checked_add(1)
                .ok_or_else(|| budget_error("weighted route depth exceeds u32"))?;
            let candidate = Label {
                cost: total_cost,
                hops,
                root: current.root,
                parent: Some(node),
                edge: Some(arc.edge),
                edge_revision: Some(arc.revision),
                edge_cost: Some(weight),
            };
            if labels[arc.target].is_some_and(|prior| prior <= candidate) {
                continue;
            }
            if settled[arc.target] {
                return Err(invariant("weighted route found a better settled label"));
            }
            if let Some(prior) = labels[arc.target] {
                frontier.remove(&prior.queue_key(arc.target));
            }
            labels[arc.target] = Some(candidate);
            frontier.insert(candidate.queue_key(arc.target));
            run.index.frontier(frontier.len())?;
        }
    }

    let mut decisions = Vec::new();
    put_str(&mut decisions, WEIGHTED_ROUTE_POLICY);
    decisions.push(u8::from(query.direction == GraphDirection::Incoming));
    for bound in [
        budget.max_vertices as u64,
        budget.max_edges as u64,
        budget.max_frontier as u64,
        budget.max_work,
    ] {
        put_u64(&mut decisions, bound);
    }
    put_u64(&mut decisions, roots.len() as u64);
    for id in &roots {
        run.charge()?;
        put_u64(&mut decisions, id.get());
    }
    put_u64(&mut decisions, n as u64);
    let mut entries = Vec::new();
    let mut unreachable = Vec::new();
    for (node, label) in labels.into_iter().enumerate() {
        run.charge()?;
        let id = run.index.ids[node];
        put_u64(&mut decisions, id.get());
        let Some(label) = label else {
            decisions.push(0);
            unreachable.push(id);
            continue;
        };
        decisions.push(1);
        put_u64(&mut decisions, label.cost);
        put_u32(&mut decisions, label.hops);
        let root = run.index.ids[label.root];
        put_u64(&mut decisions, root.get());
        let parent = label.parent.map(|parent| run.index.ids[parent]);
        match (parent, label.edge, label.edge_revision, label.edge_cost) {
            (Some(parent), Some(edge), Some(revision), Some(cost)) => {
                decisions.push(1);
                put_u64(&mut decisions, parent.get());
                decisions.extend_from_slice(&edge.get().to_be_bytes());
                put_u64(&mut decisions, revision);
                put_u64(&mut decisions, cost);
            }
            (None, None, None, None) => decisions.push(0),
            _ => {
                return Err(invariant(
                    "weighted route label has inconsistent parent fields",
                ));
            }
        }
        entries.push(WeightedRouteRecord {
            entity_id: id,
            generation: run.index.versions[node].0,
            revision: run.index.versions[node].1,
            root,
            total_cost: label.cost,
            hops: label.hops,
            parent,
            via_edge: label.edge,
            via_edge_revision: label.edge_revision,
            edge_cost: label.edge_cost,
        });
    }
    let witness = run.index.witness(snapshot, scope_digest, &decisions);
    (run.check)()?;
    Ok(WeightedRoutes {
        roots,
        direction: query.direction,
        entries,
        unreachable,
        witness,
    })
}
