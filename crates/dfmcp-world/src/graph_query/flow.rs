//! Finite directed capacity analysis over one authorized observed projection.
//!
//! The source supplies both capacities and their common unit. A flow/cut
//! certificate proves the supplied network model, never physical throughput,
//! game walkability, complete-world absence or mutation authority.

use std::collections::{BTreeSet, VecDeque};

use dfmcp_core::GameTick;

use super::{
    DfmcpError, Digest32, EdgeId, EdgeKind, EntityId, ErrorCode, GraphBudget, GraphWitness, Index,
    Result, WorldSnapshot, budget_error, invariant, put_bytes, put_str, put_u64,
};
use crate::canonical::put_anchor;
use crate::evidence::laboratory_fact_value;
use crate::{Fact, FactSource, Value};

pub const FLOW_POLICY: &str = "observed-directed-finite-capacity-dinic-stable-edge/1";
const MAX_FIELD_BYTES: usize = 128;
const MAX_UNIT_BYTES: usize = 128;
const MAX_SOURCE_NAME_BYTES: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowQuery {
    pub source: EntityId,
    pub sink: EntityId,
    /// Stored directions are retained; no symmetric or reverse capacity is
    /// inferred. Every edge of the selected kinds is part of this model.
    pub edge_kinds: Vec<EdgeKind>,
    pub capacity_field: String,
    /// A source-qualified nonblank Text fact on every selected edge. All unit
    /// labels must agree exactly; the caller does not provide or invent units.
    pub unit_field: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowEdge {
    pub edge_id: EdgeId,
    pub revision: u64,
    pub from: EntityId,
    pub to: EntityId,
    pub capacity: u64,
    pub flow: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlowAnalysis {
    pub source: EntityId,
    pub sink: EntityId,
    pub value: u64,
    /// None only when the projection contains no selected edges. Its flow is
    /// zero and no physical unit has been established.
    pub unit: Option<String>,
    /// Every selected edge, including zero-flow edges, in stable edge-ID order.
    pub edges: Vec<FlowEdge>,
    /// Exact residual source-side partition in ascending stable entity order.
    pub source_side: Vec<EntityId>,
    /// Every other observed vertex, including unrelated components.
    pub sink_side: Vec<EntityId>,
    /// All original edges from source_side to sink_side, including zero capacity.
    pub cut_edges: Vec<EdgeId>,
    /// Independently verified equal to value; no sentinel denotes infinity.
    pub cut_capacity: u64,
    pub witness: GraphWitness,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn unavailable(id: EdgeId, field: &str, expected: &str) -> DfmcpError {
    DfmcpError::new(
        ErrorCode::PreconditionsFailed,
        format!(
            "flow field '{field}' on edge {id} requires a current, source-qualified {expected}; no value is inferred"
        ),
    )
}

// This validates source shape and temporal eligibility. Compatibility and
// observation authenticity belong to the owner supplying this projection.
fn qualified_value(fact: &Fact, tick: GameTick) -> Option<&Value> {
    if let Some(value) = laboratory_fact_value(fact, tick) {
        Some(value)
    } else if fact.observed_at <= tick
        && fact.source_digest != Digest32::ZERO
        && matches!(&fact.source, FactSource::DfhackField(name)
            if !name.trim().is_empty() && name.len() <= MAX_SOURCE_NAME_BYTES
                && !name.chars().any(char::is_control))
    {
        fact.known_value()
    } else {
        None
    }
}

pub(super) fn capacity(fact: &Fact, tick: GameTick) -> Option<u64> {
    match qualified_value(fact, tick)? {
        Value::U64(value) => Some(*value),
        Value::I64(value) => u64::try_from(*value).ok(),
        _ => None,
    }
}

pub(super) fn unit(fact: &Fact, tick: GameTick) -> Option<&str> {
    match qualified_value(fact, tick)? {
        Value::Text(value)
            if !value.trim().is_empty()
                && value.len() <= MAX_UNIT_BYTES
                && !value.chars().any(char::is_control) =>
        {
            Some(value)
        }
        _ => None,
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

#[derive(Clone, Copy)]
struct ResidualArc {
    target: usize,
    edge: usize,
    reverse: bool,
}

struct Network {
    adjacent: Vec<Vec<ResidualArc>>,
    endpoints: Vec<(usize, usize)>,
    edges: Vec<FlowEdge>,
    unit: Option<String>,
}

impl Network {
    fn build<F: FnMut() -> Result<()>>(
        snapshot: &WorldSnapshot,
        query: &FlowQuery,
        run: &mut Run<F>,
    ) -> Result<Self> {
        let mut network = Self {
            adjacent: vec![Vec::new(); run.index.ids.len()],
            endpoints: Vec::new(),
            edges: Vec::new(),
            unit: None,
        };
        let selected: BTreeSet<_> = query.edge_kinds.iter().collect();
        let mut projection = Vec::new();
        put_str(&mut projection, "dfmcp-directed-capacity-projection/1");
        put_bytes(&mut projection, run.index.projection_digest.as_bytes());
        put_anchor(&mut projection, snapshot.anchor());
        put_str(&mut projection, &query.capacity_field);
        put_str(&mut projection, &query.unit_field);
        for (&id, edge) in &snapshot.graph.edges {
            run.charge()?;
            if !selected.contains(&edge.kind) {
                continue;
            }
            let missing_capacity =
                || unavailable(id, &query.capacity_field, "nonnegative integer capacity");
            let capacity_fact = edge
                .fields
                .get(&query.capacity_field)
                .ok_or_else(missing_capacity)?;
            let capacity = capacity(capacity_fact, snapshot.tick).ok_or_else(missing_capacity)?;
            let missing_unit = || unavailable(id, &query.unit_field, "nonblank bounded Text unit");
            let unit_fact = edge
                .fields
                .get(&query.unit_field)
                .ok_or_else(missing_unit)?;
            let unit = unit(unit_fact, snapshot.tick).ok_or_else(missing_unit)?;
            if network.unit.as_deref().is_some_and(|prior| prior != unit) {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "selected flow edges have different capacity units; no unit conversion is inferred",
                ));
            }
            if network.unit.is_none() {
                network.unit = Some(unit.to_owned());
            }
            let from = run.index.ordinals.get(&edge.from).copied().ok_or_else(|| {
                invariant("flow edge source disappeared from the admitted projection")
            })?;
            let to = run.index.ordinals.get(&edge.to).copied().ok_or_else(|| {
                invariant("flow edge target disappeared from the admitted projection")
            })?;
            let ordinal = network.edges.len();
            network.edges.push(FlowEdge {
                edge_id: id,
                revision: edge.revision,
                from: edge.from,
                to: edge.to,
                capacity,
                flow: 0,
            });
            network.endpoints.push((from, to));
            // Iteration is by stable original edge ID. Each adjacency therefore
            // uses that order too, with forward before reverse for a self-loop.
            // Antiparallel edges retain independent original identities.
            network.adjacent[from].push(ResidualArc {
                target: to,
                edge: ordinal,
                reverse: false,
            });
            network.adjacent[to].push(ResidualArc {
                target: from,
                edge: ordinal,
                reverse: true,
            });
            projection.extend_from_slice(&id.get().to_be_bytes());
            put_u64(&mut projection, capacity);
            put_bytes(&mut projection, &capacity_fact.canonical_bytes());
            put_bytes(&mut projection, &unit_fact.canonical_bytes());
        }
        put_u64(&mut projection, network.edges.len() as u64);
        run.index.projection_digest = Digest32::of_bytes(&projection);
        // The residual network now owns the adjacency needed by the solver.
        // Keep the shared index's identities, bounds and witness counters only.
        run.index.outgoing.clear();
        run.index.incoming.clear();
        Ok(network)
    }

    fn remaining(&self, arc: ResidualArc) -> Result<u64> {
        let edge = &self.edges[arc.edge];
        if arc.reverse {
            Ok(edge.flow)
        } else {
            edge.capacity
                .checked_sub(edge.flow)
                .ok_or_else(|| invariant("flow exceeds its original edge capacity"))
        }
    }

    /// Complete residual BFS. The final unsuccessful level graph also supplies
    /// the canonical reachable source-side of the minimum cut.
    fn levels<F: FnMut() -> Result<()>>(
        &self,
        source: usize,
        run: &mut Run<F>,
    ) -> Result<Vec<u32>> {
        let mut levels = vec![u32::MAX; self.adjacent.len()];
        levels[source] = 0;
        let mut queue = VecDeque::from([source]);
        run.index.frontier(queue.len())?;
        while let Some(node) = queue.pop_front() {
            run.charge()?;
            for &arc in &self.adjacent[node] {
                run.arc()?;
                if levels[arc.target] == u32::MAX && self.remaining(arc)? > 0 {
                    levels[arc.target] = levels[node]
                        .checked_add(1)
                        .ok_or_else(|| budget_error("flow level depth exceeds u32"))?;
                    queue.push_back(arc.target);
                    run.index.frontier(queue.len())?;
                }
            }
        }
        Ok(levels)
    }

    /// Iterative blocking-flow search with persistent current-arc pointers.
    /// A successful augmentation saturates at least one admissible edge; no
    /// one-unit loop or recursive traversal depends on the numeric capacity.
    fn blocking_flow<F: FnMut() -> Result<()>>(
        &mut self,
        source: usize,
        sink: usize,
        levels: &[u32],
        value: &mut u64,
        run: &mut Run<F>,
    ) -> Result<()> {
        let mut next = vec![0usize; self.adjacent.len()];
        let mut vertices = vec![source];
        let mut path = Vec::<ResidualArc>::new();
        run.index.frontier(vertices.len())?;
        while let Some(&node) = vertices.last() {
            run.charge()?;
            if node == sink {
                let mut amount = None;
                for &arc in &path {
                    run.arc()?;
                    let remaining = self.remaining(arc)?;
                    amount = Some(amount.map_or(remaining, |prior: u64| prior.min(remaining)));
                }
                let amount = amount
                    .filter(|amount| *amount > 0)
                    .ok_or_else(|| invariant("flow augmentation has no positive path capacity"))?;
                let new_value = value
                    .checked_add(amount)
                    .ok_or_else(|| budget_error("maximum flow exceeds u64"))?;
                for &arc in &path {
                    run.arc()?;
                    let edge = &mut self.edges[arc.edge];
                    edge.flow = if arc.reverse {
                        edge.flow
                            .checked_sub(amount)
                            .ok_or_else(|| invariant("reverse augmentation exceeds edge flow"))?
                    } else {
                        edge.flow
                            .checked_add(amount)
                            .filter(|flow| *flow <= edge.capacity)
                            .ok_or_else(|| invariant("augmentation exceeds edge capacity"))?
                    };
                }
                *value = new_value;
                vertices.truncate(1);
                path.clear();
                continue;
            }
            let mut advance = None;
            while next[node] < self.adjacent[node].len() {
                run.arc()?;
                let arc = self.adjacent[node][next[node]];
                if levels[node].checked_add(1) == Some(levels[arc.target])
                    && self.remaining(arc)? > 0
                {
                    advance = Some(arc);
                    break;
                }
                next[node] += 1;
            }
            if let Some(arc) = advance {
                path.push(arc);
                vertices.push(arc.target);
                run.index.frontier(vertices.len())?;
            } else {
                vertices.pop();
                path.pop();
                if let Some(&parent) = vertices.last() {
                    next[parent] += 1;
                }
            }
        }
        Ok(())
    }

    /// Separate primal/dual verification uses original edges and the partition,
    /// not the augmentation path or the solver's residual-capacity helper.
    /// Feasibility plus equal flow and cut values establishes optimality for
    /// the finite supplied network, even if the search itself were defective.
    fn certify<F: FnMut() -> Result<()>>(
        &self,
        source: usize,
        sink: usize,
        source_side: &[bool],
        value: u64,
        run: &mut Run<F>,
    ) -> Result<(Vec<EdgeId>, u64)> {
        if source_side.len() != self.adjacent.len()
            || !source_side[source]
            || source_side[sink]
            || self.edges.len() != self.endpoints.len()
        {
            return Err(invariant("flow certificate has an invalid partition"));
        }
        let mut incoming = vec![0u128; source_side.len()];
        let mut outgoing = vec![0u128; source_side.len()];
        let mut cut_edges = Vec::new();
        let mut cut_capacity = 0u64;
        for (edge, &(from, to)) in self.edges.iter().zip(&self.endpoints) {
            run.arc()?;
            if edge.flow > edge.capacity
                || (from == to && edge.flow != 0)
                || edge.from != run.index.ids[from]
                || edge.to != run.index.ids[to]
            {
                return Err(invariant(
                    "flow certificate violates edge identity or capacity",
                ));
            }
            incoming[to] = incoming[to]
                .checked_add(u128::from(edge.flow))
                .ok_or_else(|| budget_error("flow certificate incoming sum exceeds u128"))?;
            outgoing[from] = outgoing[from]
                .checked_add(u128::from(edge.flow))
                .ok_or_else(|| budget_error("flow certificate outgoing sum exceeds u128"))?;
            if source_side[from] && !source_side[to] {
                if edge.flow != edge.capacity {
                    return Err(invariant("flow cut contains an unsaturated forward edge"));
                }
                cut_capacity = cut_capacity
                    .checked_add(edge.capacity)
                    .ok_or_else(|| budget_error("minimum cut capacity exceeds u64"))?;
                cut_edges.push(edge.edge_id);
            } else if !source_side[from] && source_side[to] && edge.flow != 0 {
                return Err(invariant("flow cut contains positive reverse flow"));
            }
        }
        for node in 0..source_side.len() {
            run.charge()?;
            let (expected_in, expected_out) = if node == source {
                (
                    incoming[node],
                    incoming[node]
                        .checked_add(u128::from(value))
                        .ok_or_else(|| {
                            budget_error("flow certificate source balance exceeds u128")
                        })?,
                )
            } else if node == sink {
                (
                    outgoing[node]
                        .checked_add(u128::from(value))
                        .ok_or_else(|| {
                            budget_error("flow certificate sink balance exceeds u128")
                        })?,
                    outgoing[node],
                )
            } else {
                (incoming[node], incoming[node])
            };
            if incoming[node] != expected_in || outgoing[node] != expected_out {
                return Err(invariant(
                    "flow certificate violates conservation or terminal value",
                ));
            }
        }
        if cut_capacity != value {
            return Err(invariant("flow and minimum-cut certificate values differ"));
        }
        Ok((cut_edges, cut_capacity))
    }
}

pub fn maximum_flow(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &FlowQuery,
    budget: GraphBudget,
) -> Result<FlowAnalysis> {
    maximum_flow_with_check(snapshot, scope_digest, query, budget, || Ok(()))
}

/// Deterministic iterative Dinic maximum flow and minimum cut. O(V²E) worst-case
/// work, independent of capacity magnitudes, with O(V + E) auxiliary storage.
/// Every BFS queue and explicit DFS path obeys max_frontier. Source admission,
/// unit/capacity qualification, residual searches and independent primal/dual
/// verification share one operation budget. No recursion or partial result.
///
/// Capacities are finite u64 or nonnegative i64 facts. Unit facts are exact,
/// common nonblank Text labels of at most 128 UTF-8 bytes. Both require a
/// registered laboratory source or bounded DFHack field with nonzero digest,
/// consistent Known presence and a tick no later than the selected snapshot.
/// These checks establish source shape, not compatibility or native admission.
///
/// Stable original edge ID, then forward-before-reverse residual direction,
/// defines each adjacency order. Vertices and returned edges use ascending
/// stable identities. The final cut is source reachability in the exact final
/// residual network, not a lexicographically smallest cut among all optima.
///
/// The callback runs before/after shared index construction, at most 128 charged
/// subsequent work units apart, and before return. Shared hashing and initial
/// index construction remain count-bounded rather than internally interruptible.
pub fn maximum_flow_with_check<F: FnMut() -> Result<()>>(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &FlowQuery,
    budget: GraphBudget,
    mut check: F,
) -> Result<FlowAnalysis> {
    check()?;
    if query.source == query.sink
        || query.source == EntityId::NIL
        || query.sink == EntityId::NIL
        || query.edge_kinds.is_empty()
        || query.capacity_field == query.unit_field
        || [&query.capacity_field, &query.unit_field]
            .iter()
            .any(|field| {
                field.trim().is_empty()
                    || field.len() > MAX_FIELD_BYTES
                    || field.chars().any(char::is_control)
            })
    {
        return Err(invalid(
            "flow requires distinct source/sink IDs, explicit edge kinds and distinct nonblank capacity/unit fields of at most 128 UTF-8 bytes without control characters",
        ));
    }
    let index = Index::build(snapshot, &query.edge_kinds, budget)?;
    check()?;
    let source = index
        .ordinals
        .get(&query.source)
        .copied()
        .ok_or_else(|| invalid("flow source is not present in the authorized projection"))?;
    let sink = index
        .ordinals
        .get(&query.sink)
        .copied()
        .ok_or_else(|| invalid("flow sink is not present in the authorized projection"))?;
    let next_check = index.work + 128;
    let mut run = Run {
        index,
        check,
        next_check,
    };
    let mut network = Network::build(snapshot, query, &mut run)?;
    let mut value = 0u64;
    let partition = loop {
        run.charge()?;
        let levels = network.levels(source, &mut run)?;
        if levels[sink] == u32::MAX {
            break levels
                .iter()
                .map(|level| *level != u32::MAX)
                .collect::<Vec<_>>();
        }
        let before = value;
        network.blocking_flow(source, sink, &levels, &mut value, &mut run)?;
        if value == before {
            return Err(invariant(
                "reachable flow level graph made no blocking-flow progress",
            ));
        }
    };
    let (cut_edges, cut_capacity) = network.certify(source, sink, &partition, value, &mut run)?;
    let mut source_side = Vec::new();
    let mut sink_side = Vec::new();
    for (node, &included) in partition.iter().enumerate() {
        run.charge()?;
        if included {
            source_side.push(run.index.ids[node]);
        } else {
            sink_side.push(run.index.ids[node]);
        }
    }
    let mut decisions = Vec::new();
    put_str(&mut decisions, FLOW_POLICY);
    put_u64(&mut decisions, query.source.get());
    put_u64(&mut decisions, query.sink.get());
    for bound in [
        budget.max_vertices as u64,
        budget.max_edges as u64,
        budget.max_frontier as u64,
        budget.max_work,
    ] {
        put_u64(&mut decisions, bound);
    }
    put_u64(&mut decisions, value);
    put_u64(&mut decisions, cut_capacity);
    put_u64(&mut decisions, network.edges.len() as u64);
    for edge in &network.edges {
        run.charge()?;
        decisions.extend_from_slice(&edge.edge_id.get().to_be_bytes());
        put_u64(&mut decisions, edge.revision);
        put_u64(&mut decisions, edge.from.get());
        put_u64(&mut decisions, edge.to.get());
        put_u64(&mut decisions, edge.capacity);
        put_u64(&mut decisions, edge.flow);
    }
    for side in [&source_side, &sink_side] {
        put_u64(&mut decisions, side.len() as u64);
        for id in side {
            run.charge()?;
            put_u64(&mut decisions, id.get());
        }
    }
    put_u64(&mut decisions, cut_edges.len() as u64);
    for edge in &cut_edges {
        run.charge()?;
        decisions.extend_from_slice(&edge.get().to_be_bytes());
    }
    let witness = run.index.witness(snapshot, scope_digest, &decisions);
    (run.check)()?;
    Ok(FlowAnalysis {
        source: query.source,
        sink: query.sink,
        value,
        unit: network.unit,
        edges: network.edges,
        source_side,
        sink_side,
        cut_edges,
        cut_capacity,
        witness,
    })
}
