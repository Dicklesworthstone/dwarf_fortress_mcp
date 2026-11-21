//! Demand-bounded minimum-cost flow over one authorized observed projection.
//!
//! Capacities, per-unit costs and both unit labels come from independently
//! qualified source facts. The result certifies this finite directed model,
//! never hauling feasibility, native progress or mutation authority.

use std::collections::BTreeSet;

use super::flow::{capacity, unit};
use super::{
    DfmcpError, Digest32, EdgeId, EdgeKind, EntityId, ErrorCode, GraphBudget, GraphWitness, Index,
    Result, WorldSnapshot, budget_error, invariant, put_bytes, put_str, put_u64,
};
use crate::canonical::put_anchor;

pub const MIN_COST_FLOW_POLICY: &str =
    "observed-directed-demand-min-cost-global-potentials-stable-edge/1";
const MAX_FIELD_BYTES: usize = 128;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinCostFlowQuery {
    pub source: EntityId,
    pub sink: EntityId,
    pub edge_kinds: Vec<EdgeKind>,
    pub capacity_field: String,
    pub unit_field: String,
    /// Nonnegative integer cost per unit of flow on the original directed edge.
    pub cost_field: String,
    /// Exact common source-recorded label for the per-unit cost dimension.
    pub cost_unit_field: String,
    /// Positive finite requested flow. Delivery never exceeds this amount.
    pub demand: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostFlowEdge {
    pub edge_id: EdgeId,
    pub revision: u64,
    pub from: EntityId,
    pub to: EntityId,
    pub capacity: u64,
    pub cost: u64,
    pub flow: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostFlowPotential {
    pub entity_id: EntityId,
    /// Every positive residual arc u -> v has c + p[u] - p[v] >= 0.
    pub potential: i128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostFlowCut {
    pub source_side: Vec<EntityId>,
    pub sink_side: Vec<EntityId>,
    pub cut_edges: Vec<EdgeId>,
    pub cut_capacity: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MinCostFlowAnalysis {
    pub source: EntityId,
    pub sink: EntityId,
    pub demand: u64,
    pub value: u64,
    pub unmet_demand: u64,
    pub total_cost: u128,
    /// None only when there are no selected edges.
    pub unit: Option<String>,
    /// None only when there are no selected edges.
    pub cost_unit: Option<String>,
    /// Every selected original edge in ascending stable edge-ID order.
    pub edges: Vec<CostFlowEdge>,
    /// A globally feasible dual potential for every observed vertex, sorted by ID.
    pub potentials: Vec<CostFlowPotential>,
    /// Present exactly when delivery is below demand; capacity equals value.
    /// A satisfied demand does not claim the network's maximum throughput.
    pub minimum_cut: Option<CostFlowCut>,
    pub witness: GraphWitness,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn unavailable(id: EdgeId, field: &str, expected: &str) -> DfmcpError {
    DfmcpError::new(
        ErrorCode::PreconditionsFailed,
        format!(
            "minimum-cost flow field '{field}' on edge {id} requires a current, source-qualified {expected}; no value is inferred"
        ),
    )
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

    /// Charge initialization before allocating an O(V) array. Checking each
    /// bounded chunk also prevents a long uninterrupted initialization budget.
    fn vertices(&mut self) -> Result<()> {
        for _ in 0..self.index.ids.len() {
            self.charge()?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct ResidualArc {
    target: usize,
    edge: usize,
    reverse: bool,
}

#[derive(Clone, Copy)]
struct Previous {
    from: usize,
    arc: ResidualArc,
}

struct ShortestPaths {
    distances: Vec<Option<i128>>,
    previous: Vec<Option<Previous>>,
    maximum_distance: i128,
}

struct Network {
    adjacent: Vec<Vec<ResidualArc>>,
    endpoints: Vec<(usize, usize)>,
    edges: Vec<CostFlowEdge>,
    unit: Option<String>,
    cost_unit: Option<String>,
}

struct CertificateInput<'a> {
    source: usize,
    sink: usize,
    value: u64,
    demand: u64,
    potentials: &'a [i128],
    partition: Option<&'a [bool]>,
}

impl Network {
    fn build<F: FnMut() -> Result<()>>(
        snapshot: &WorldSnapshot,
        query: &MinCostFlowQuery,
        run: &mut Run<F>,
    ) -> Result<Self> {
        run.vertices()?;
        let mut network = Self {
            adjacent: vec![Vec::new(); run.index.ids.len()],
            endpoints: Vec::new(),
            edges: Vec::new(),
            unit: None,
            cost_unit: None,
        };
        let selected: BTreeSet<_> = query.edge_kinds.iter().collect();
        let mut projection = Vec::new();
        put_str(&mut projection, "dfmcp-directed-cost-capacity-projection/1");
        put_bytes(&mut projection, run.index.projection_digest.as_bytes());
        put_anchor(&mut projection, snapshot.anchor());
        for field in [
            &query.capacity_field,
            &query.unit_field,
            &query.cost_field,
            &query.cost_unit_field,
        ] {
            put_str(&mut projection, field);
        }
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
            let capacity_unit = unit(unit_fact, snapshot.tick).ok_or_else(missing_unit)?;
            let missing_cost =
                || unavailable(id, &query.cost_field, "nonnegative integer per-unit cost");
            let cost_fact = edge
                .fields
                .get(&query.cost_field)
                .ok_or_else(missing_cost)?;
            let cost = super::flow::capacity(cost_fact, snapshot.tick).ok_or_else(missing_cost)?;
            let missing_cost_unit = || {
                unavailable(
                    id,
                    &query.cost_unit_field,
                    "nonblank bounded Text cost unit",
                )
            };
            let cost_unit_fact = edge
                .fields
                .get(&query.cost_unit_field)
                .ok_or_else(missing_cost_unit)?;
            let cost_unit = unit(cost_unit_fact, snapshot.tick).ok_or_else(missing_cost_unit)?;
            if network
                .unit
                .as_deref()
                .is_some_and(|prior| prior != capacity_unit)
                || network
                    .cost_unit
                    .as_deref()
                    .is_some_and(|prior| prior != cost_unit)
            {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "selected minimum-cost flow edges disagree on capacity or per-unit cost units; no conversion is inferred",
                ));
            }
            if network.unit.is_none() {
                network.unit = Some(capacity_unit.to_owned());
                network.cost_unit = Some(cost_unit.to_owned());
            }
            let from = run.index.ordinals.get(&edge.from).copied().ok_or_else(|| {
                invariant("minimum-cost flow source disappeared from the admitted projection")
            })?;
            let to = run.index.ordinals.get(&edge.to).copied().ok_or_else(|| {
                invariant("minimum-cost flow target disappeared from the admitted projection")
            })?;
            let ordinal = network.edges.len();
            network.edges.push(CostFlowEdge {
                edge_id: id,
                revision: edge.revision,
                from: edge.from,
                to: edge.to,
                capacity,
                cost,
                flow: 0,
            });
            network.endpoints.push((from, to));
            // Stable original edge ID, forward before reverse for a self-loop.
            // Parallel and antiparallel edges retain separate original identities.
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
            put_u64(&mut projection, cost);
            for fact in [capacity_fact, unit_fact, cost_fact, cost_unit_fact] {
                run.charge()?;
                put_bytes(&mut projection, &fact.canonical_bytes());
            }
        }
        put_u64(&mut projection, network.edges.len() as u64);
        run.index.projection_digest = Digest32::of_bytes(&projection);
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
                .ok_or_else(|| invariant("minimum-cost flow exceeds original capacity"))
        }
    }

    fn reduced_cost(&self, from: usize, arc: ResidualArc, potentials: &[i128]) -> Result<i128> {
        let cost = i128::from(self.edges[arc.edge].cost);
        let signed_cost = if arc.reverse { -cost } else { cost };
        signed_cost
            .checked_add(potentials[from])
            .and_then(|value| value.checked_sub(potentials[arc.target]))
            .ok_or_else(|| budget_error("minimum-cost flow reduced cost exceeds i128"))
    }

    /// Complete Dijkstra search, not an early exit at the sink. Unreachable
    /// vertices need the maximum finite distance to preserve global dual
    /// feasibility, including residual arcs entering the reached component.
    fn shortest_paths<F: FnMut() -> Result<()>>(
        &self,
        source: usize,
        potentials: &[i128],
        run: &mut Run<F>,
    ) -> Result<ShortestPaths> {
        for _ in 0..3 {
            run.vertices()?;
        }
        let mut distances = vec![None; self.adjacent.len()];
        let mut previous = vec![None; self.adjacent.len()];
        let mut settled = vec![false; self.adjacent.len()];
        let mut frontier = BTreeSet::from([(0i128, source)]);
        run.index.frontier(frontier.len())?;
        distances[source] = Some(0);
        let mut maximum_distance = 0;
        while let Some((distance, node)) = frontier.pop_first() {
            run.charge()?;
            if settled[node] || distances[node] != Some(distance) {
                return Err(invariant("minimum-cost flow frontier is inconsistent"));
            }
            settled[node] = true;
            maximum_distance = distance;
            for &arc in &self.adjacent[node] {
                run.arc()?;
                if self.remaining(arc)? == 0 {
                    continue;
                }
                let reduced = self.reduced_cost(node, arc, potentials)?;
                if reduced < 0 {
                    return Err(invariant(
                        "minimum-cost flow encountered an infeasible residual potential",
                    ));
                }
                let candidate = distance
                    .checked_add(reduced)
                    .ok_or_else(|| budget_error("minimum-cost flow distance exceeds i128"))?;
                if distances[arc.target].is_some_and(|prior| prior <= candidate) {
                    continue;
                }
                if settled[arc.target] {
                    return Err(invariant(
                        "minimum-cost flow improved an already settled vertex",
                    ));
                }
                if let Some(prior) = distances[arc.target]
                    && !frontier.remove(&(prior, arc.target))
                {
                    return Err(invariant("minimum-cost flow decrease-key lost its vertex"));
                }
                run.index.frontier(frontier.len() + 1)?;
                frontier.insert((candidate, arc.target));
                distances[arc.target] = Some(candidate);
                previous[arc.target] = Some(Previous { from: node, arc });
            }
        }
        Ok(ShortestPaths {
            distances,
            previous,
            maximum_distance,
        })
    }

    fn augment<F: FnMut() -> Result<()>>(
        &mut self,
        source: usize,
        sink: usize,
        remaining_demand: u64,
        previous: &[Option<Previous>],
        trace: &mut Digest32,
        run: &mut Run<F>,
    ) -> Result<u64> {
        let mut node = sink;
        let mut path = Vec::new();
        let mut amount = remaining_demand;
        while node != source {
            run.arc()?;
            if path.len() >= self.adjacent.len() {
                return Err(invariant("minimum-cost flow predecessor chain has a cycle"));
            }
            let step = previous[node]
                .ok_or_else(|| invariant("minimum-cost flow path lost its predecessor"))?;
            if step.arc.target != node {
                return Err(invariant("minimum-cost flow path has a different endpoint"));
            }
            amount = amount.min(self.remaining(step.arc)?);
            run.index.frontier(path.len() + 1)?;
            path.push(step);
            node = step.from;
        }
        if amount == 0 || path.is_empty() {
            return Err(invariant("minimum-cost flow augmentation made no progress"));
        }
        let mut decisions = Vec::new();
        put_str(&mut decisions, "dfmcp-min-cost-flow-augmentation/1");
        put_bytes(&mut decisions, trace.as_bytes());
        put_u64(&mut decisions, amount);
        put_u64(&mut decisions, path.len() as u64);
        for step in path.iter().rev() {
            run.arc()?;
            let edge = &mut self.edges[step.arc.edge];
            edge.flow = if step.arc.reverse {
                edge.flow
                    .checked_sub(amount)
                    .ok_or_else(|| invariant("minimum-cost reverse flow exceeds prior flow"))?
            } else {
                edge.flow
                    .checked_add(amount)
                    .filter(|flow| *flow <= edge.capacity)
                    .ok_or_else(|| invariant("minimum-cost augmentation exceeds capacity"))?
            };
            decisions.extend_from_slice(&edge.edge_id.get().to_be_bytes());
            decisions.push(u8::from(step.arc.reverse));
        }
        *trace = Digest32::of_bytes(&decisions);
        Ok(amount)
    }

    /// Independent primal/dual verification reads the original edge rows, not
    /// the solver's residual traversal, distances, or predecessor chains.
    fn certify<F: FnMut() -> Result<()>>(
        &self,
        input: CertificateInput<'_>,
        run: &mut Run<F>,
    ) -> Result<(u128, Option<CostFlowCut>)> {
        let CertificateInput {
            source,
            sink,
            value,
            demand,
            potentials,
            partition,
        } = input;
        let count = run.index.ids.len();
        if self.edges.len() != self.endpoints.len()
            || potentials.len() != count
            || value > demand
            || partition.is_some() != (value < demand)
        {
            return Err(invariant(
                "minimum-cost flow certificate dimensions disagree",
            ));
        }
        if let Some(side) = partition
            && (side.len() != count || !side[source] || side[sink])
        {
            return Err(invariant("minimum-cost flow cut has an invalid partition"));
        }
        run.vertices()?;
        run.vertices()?;
        let mut incoming = vec![0u128; count];
        let mut outgoing = vec![0u128; count];
        let mut total_cost = 0u128;
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
                    "minimum-cost flow certificate violates edge identity or capacity",
                ));
            }
            incoming[to] = incoming[to]
                .checked_add(u128::from(edge.flow))
                .ok_or_else(|| budget_error("minimum-cost incoming sum exceeds u128"))?;
            outgoing[from] = outgoing[from]
                .checked_add(u128::from(edge.flow))
                .ok_or_else(|| budget_error("minimum-cost outgoing sum exceeds u128"))?;
            let edge_cost = u128::from(edge.flow)
                .checked_mul(u128::from(edge.cost))
                .ok_or_else(|| budget_error("minimum-cost edge cost exceeds u128"))?;
            total_cost = total_cost
                .checked_add(edge_cost)
                .ok_or_else(|| budget_error("minimum-cost total cost exceeds u128"))?;
            let reduced = i128::from(edge.cost)
                .checked_add(potentials[from])
                .and_then(|value| value.checked_sub(potentials[to]))
                .ok_or_else(|| budget_error("minimum-cost certificate exceeds i128"))?;
            // Forward residual capacity needs nonnegative reduced cost; a
            // positive original flow supplies the reverse arc with its negation.
            // This checks ALL components, including vertices unreachable from s.
            if (edge.flow < edge.capacity && reduced < 0) || (edge.flow > 0 && reduced > 0) {
                return Err(invariant(
                    "minimum-cost certificate has a negative reduced-cost residual arc",
                ));
            }
            if let Some(side) = partition {
                if side[from] && !side[to] {
                    if edge.flow != edge.capacity {
                        return Err(invariant(
                            "minimum-cost cut has an unsaturated forward edge",
                        ));
                    }
                    cut_capacity = cut_capacity
                        .checked_add(edge.capacity)
                        .ok_or_else(|| budget_error("minimum-cost cut capacity exceeds u64"))?;
                    cut_edges.push(edge.edge_id);
                } else if !side[from] && side[to] && edge.flow != 0 {
                    return Err(invariant("minimum-cost cut contains positive reverse flow"));
                }
            }
        }
        for node in 0..count {
            run.charge()?;
            let expected_out = if node == source {
                incoming[node]
                    .checked_add(u128::from(value))
                    .ok_or_else(|| budget_error("minimum-cost source balance exceeds u128"))?
            } else if node == sink {
                incoming[node]
                    .checked_sub(u128::from(value))
                    .ok_or_else(|| invariant("minimum-cost sink received less than its value"))?
            } else {
                incoming[node]
            };
            if outgoing[node] != expected_out {
                return Err(invariant(
                    "minimum-cost flow violates conservation or terminal value",
                ));
            }
        }
        let cut = if let Some(side) = partition {
            if cut_capacity != value {
                return Err(invariant("minimum-cost flow and cut values differ"));
            }
            let mut source_side = Vec::new();
            let mut sink_side = Vec::new();
            for (node, &included) in side.iter().enumerate() {
                run.charge()?;
                if included {
                    source_side.push(run.index.ids[node]);
                } else {
                    sink_side.push(run.index.ids[node]);
                }
            }
            Some(CostFlowCut {
                source_side,
                sink_side,
                cut_edges,
                cut_capacity,
            })
        } else {
            None
        };
        Ok((total_cost, cut))
    }
}

pub fn minimum_cost_flow(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &MinCostFlowQuery,
    budget: GraphBudget,
) -> Result<MinCostFlowAnalysis> {
    minimum_cost_flow_with_check(snapshot, scope_digest, query, budget, || Ok(()))
}

/// Maximize delivered flow up to positive demand, then minimize its total cost.
/// Successive shortest residual paths use globally feasible integer potentials
/// and augment by the path bottleneck or remaining demand, never one-unit loops.
/// All original costs are nonnegative; internal reverse arcs negate their costs.
///
/// Dijkstra settles (distance, stable vertex ID) in ascending order and scans
/// original edge IDs forward-before-reverse. Equal-distance candidates retain
/// the first settled predecessor. One frontier entry per vertex avoids stale
/// heap growth. Paths and live frontiers share max_frontier, all phases share
/// max_work, and every result is independently checked for primal and dual
/// feasibility. This is O(A * (V + E log V)) for A bounded augmentations, with
/// O(V + E) auxiliary space; budget exhaustion returns no partial certificate.
///
/// A demand shortfall additionally carries an equal-capacity minimum cut.
/// Satisfied demand carries no maximum-throughput claim. Disconnected original
/// edges are still qualified, unit-checked and covered by the dual witness.
///
/// Callback cadence matches the other graph operators: before/after shared
/// index construction, every 128 subsequent charged work units, and at return.
/// Initial snapshot hashing/index construction remains count-bounded rather
/// than internally cooperatively interrupted.
pub fn minimum_cost_flow_with_check<F: FnMut() -> Result<()>>(
    snapshot: &WorldSnapshot,
    scope_digest: Digest32,
    query: &MinCostFlowQuery,
    budget: GraphBudget,
    mut check: F,
) -> Result<MinCostFlowAnalysis> {
    check()?;
    let fields = [
        &query.capacity_field,
        &query.unit_field,
        &query.cost_field,
        &query.cost_unit_field,
    ];
    if query.source == query.sink
        || query.source == EntityId::NIL
        || query.sink == EntityId::NIL
        || query.edge_kinds.is_empty()
        || query.demand == 0
        || fields.iter().any(|field| {
            field.trim().is_empty()
                || field.len() > MAX_FIELD_BYTES
                || field.chars().any(char::is_control)
        })
        || fields.iter().collect::<BTreeSet<_>>().len() != fields.len()
    {
        return Err(invalid(
            "minimum-cost flow requires distinct endpoints, positive demand, explicit edge kinds and four distinct bounded nonblank field names",
        ));
    }
    let index = Index::build(snapshot, &query.edge_kinds, budget)?;
    check()?;
    let source = index
        .ordinals
        .get(&query.source)
        .copied()
        .ok_or_else(|| invalid("minimum-cost source is not in the authorized projection"))?;
    let sink = index
        .ordinals
        .get(&query.sink)
        .copied()
        .ok_or_else(|| invalid("minimum-cost sink is not in the authorized projection"))?;
    let next_check = index.work + 128;
    let mut run = Run {
        index,
        check,
        next_check,
    };
    let mut network = Network::build(snapshot, query, &mut run)?;
    run.vertices()?;
    let mut potentials = vec![0i128; run.index.ids.len()];
    let mut value = 0u64;
    let mut trace = Digest32::of_bytes(MIN_COST_FLOW_POLICY.as_bytes());
    let mut partition = None;
    while value < query.demand {
        run.charge()?;
        let paths = network.shortest_paths(source, &potentials, &mut run)?;
        if paths.distances[sink].is_none() {
            let mut side = Vec::with_capacity(paths.distances.len());
            for distance in &paths.distances {
                run.charge()?;
                side.push(distance.is_some());
            }
            partition = Some(side);
            break;
        }
        for (node, distance) in paths.distances.iter().enumerate() {
            run.charge()?;
            // Reached-to-unreached positive residual arcs cannot exist after
            // full Dijkstra. Using max distance for every unreached node also
            // preserves feasibility of unreached-to-reached arcs.
            let adjustment = match distance {
                Some(distance) => *distance,
                None => paths.maximum_distance,
            };
            potentials[node] = potentials[node]
                .checked_add(adjustment)
                .ok_or_else(|| budget_error("minimum-cost potential exceeds i128"))?;
        }
        let amount = network.augment(
            source,
            sink,
            query.demand - value,
            &paths.previous,
            &mut trace,
            &mut run,
        )?;
        value = value
            .checked_add(amount)
            .filter(|value| *value <= query.demand)
            .ok_or_else(|| invariant("minimum-cost delivery exceeds requested demand"))?;
    }
    let (total_cost, minimum_cut) = network.certify(
        CertificateInput {
            source,
            sink,
            value,
            demand: query.demand,
            potentials: &potentials,
            partition: partition.as_deref(),
        },
        &mut run,
    )?;
    let mut decisions = Vec::new();
    put_str(&mut decisions, MIN_COST_FLOW_POLICY);
    put_bytes(&mut decisions, trace.as_bytes());
    put_u64(&mut decisions, query.source.get());
    put_u64(&mut decisions, query.sink.get());
    put_u64(&mut decisions, query.demand);
    for bound in [
        budget.max_vertices as u64,
        budget.max_edges as u64,
        budget.max_frontier as u64,
        budget.max_work,
    ] {
        put_u64(&mut decisions, bound);
    }
    put_u64(&mut decisions, value);
    decisions.extend_from_slice(&total_cost.to_be_bytes());
    put_u64(&mut decisions, network.edges.len() as u64);
    for edge in &network.edges {
        run.charge()?;
        decisions.extend_from_slice(&edge.edge_id.get().to_be_bytes());
        put_u64(&mut decisions, edge.revision);
        put_u64(&mut decisions, edge.from.get());
        put_u64(&mut decisions, edge.to.get());
        put_u64(&mut decisions, edge.capacity);
        put_u64(&mut decisions, edge.cost);
        put_u64(&mut decisions, edge.flow);
    }
    let mut output_potentials = Vec::with_capacity(potentials.len());
    put_u64(&mut decisions, potentials.len() as u64);
    for (node, potential) in potentials.into_iter().enumerate() {
        run.charge()?;
        let entity_id = run.index.ids[node];
        output_potentials.push(CostFlowPotential {
            entity_id,
            potential,
        });
        put_u64(&mut decisions, entity_id.get());
        decisions.extend_from_slice(&potential.to_be_bytes());
    }
    decisions.push(u8::from(minimum_cut.is_some()));
    if let Some(cut) = &minimum_cut {
        put_u64(&mut decisions, cut.cut_capacity);
        for side in [&cut.source_side, &cut.sink_side] {
            put_u64(&mut decisions, side.len() as u64);
            for id in side {
                run.charge()?;
                put_u64(&mut decisions, id.get());
            }
        }
        put_u64(&mut decisions, cut.cut_edges.len() as u64);
        for id in &cut.cut_edges {
            run.charge()?;
            decisions.extend_from_slice(&id.get().to_be_bytes());
        }
    }
    let witness = run.index.witness(snapshot, scope_digest, &decisions);
    (run.check)()?;
    Ok(MinCostFlowAnalysis {
        source: query.source,
        sink: query.sink,
        demand: query.demand,
        value,
        unmet_demand: query.demand - value,
        total_cost,
        unit: network.unit,
        cost_unit: network.cost_unit,
        edges: network.edges,
        potentials: output_potentials,
        minimum_cut,
        witness,
    })
}
