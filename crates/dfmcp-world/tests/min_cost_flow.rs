#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor,
    Result,
};
use dfmcp_world::graph_query::{
    GraphBudget, MinCostFlowAnalysis, MinCostFlowQuery, minimum_cost_flow,
    minimum_cost_flow_with_check,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value,
    WorldGraph, WorldSnapshot,
};

type InputEdge = (u64, u64, u64, u64);

fn scope() -> Digest32 {
    Digest32::of_bytes(b"authorized-minimum-cost-flow-fixture")
}

fn query(source: u64, sink: u64, demand: u64) -> MinCostFlowQuery {
    MinCostFlowQuery {
        source: EntityId::new(source),
        sink: EntityId::new(sink),
        edge_kinds: vec![EdgeKind::Uses],
        capacity_field: "capacity".to_owned(),
        unit_field: "capacity_unit".to_owned(),
        cost_field: "cost".to_owned(),
        cost_unit_field: "cost_unit".to_owned(),
        demand,
    }
}

fn observed(value: Value, field: &str) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField(field.to_owned()),
        Digest32::of_bytes(b"observed-costed-network-fixture"),
    )
}

fn world(size: u64, edges: &[InputEdge]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for raw in 1..=size {
        let id = EntityId::new(raw);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 2,
                revision: 3,
                kind: EntityKind::Building,
                label: format!("node-{raw}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (offset, &(from, to, capacity, cost)) in edges.iter().enumerate() {
        let id = EdgeId::new(offset as u128 + 1);
        graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 4,
                kind: EdgeKind::Uses,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::from([
                    (
                        "capacity".to_owned(),
                        observed(Value::U64(capacity), "network.capacity"),
                    ),
                    (
                        "capacity_unit".to_owned(),
                        observed(Value::Text("items".to_owned()), "network.capacity_unit"),
                    ),
                    (
                        "cost".to_owned(),
                        observed(Value::U64(cost), "network.cost"),
                    ),
                    (
                        "cost_unit".to_owned(),
                        observed(
                            Value::Text("labor_ticks/item".to_owned()),
                            "network.cost_unit",
                        ),
                    ),
                ]),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(10),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn missing() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "missing minimum-cost-flow fixture",
    )
}

fn fact_mut<'a>(world: &'a mut WorldSnapshot, edge: u128, field: &str) -> Result<&'a mut Fact> {
    world
        .graph
        .edges
        .get_mut(&EdgeId::new(edge))
        .and_then(|edge| edge.fields.get_mut(field))
        .ok_or_else(missing)
}

fn run(world: &WorldSnapshot, source: u64, sink: u64, demand: u64) -> Result<MinCostFlowAnalysis> {
    minimum_cost_flow(
        world,
        scope(),
        &query(source, sink, demand),
        GraphBudget::default(),
    )
}

fn flow(result: &MinCostFlowAnalysis, id: u128) -> Result<u64> {
    result
        .edges
        .iter()
        .find(|edge| edge.edge_id == EdgeId::new(id))
        .map(|edge| edge.flow)
        .ok_or_else(missing)
}

fn integer(fact: &Fact) -> Result<u64> {
    match fact.known_value() {
        Some(Value::U64(value)) => Ok(*value),
        Some(Value::I64(value)) => u64::try_from(*value).map_err(|_| missing()),
        _ => Err(missing()),
    }
}

/// Independent primal/dual verification uses the original records and returned
/// certificate, never production adjacency, shortest paths or augmentation.
fn verify(
    world: &WorldSnapshot,
    request: &MinCostFlowQuery,
    result: &MinCostFlowAnalysis,
) -> Result<()> {
    assert_eq!((result.source, result.sink), (request.source, request.sink));
    assert_eq!(result.demand, request.demand);
    assert!(result.value <= request.demand);
    assert_eq!(result.unmet_demand, request.demand - result.value);
    assert_eq!(result.witness.source_anchor, world.anchor());
    assert_eq!(result.witness.authorization_scope_digest, scope());
    assert_eq!(
        result.witness.scanned_vertices as usize,
        world.graph.entities.len()
    );
    assert_eq!(
        result.witness.scanned_edges as usize,
        world.graph.edges.len()
    );
    assert!(
        result
            .edges
            .windows(2)
            .all(|pair| pair[0].edge_id < pair[1].edge_id)
    );
    assert!(
        result
            .potentials
            .windows(2)
            .all(|pair| pair[0].entity_id < pair[1].entity_id)
    );
    let potentials: BTreeMap<_, _> = result
        .potentials
        .iter()
        .map(|value| (value.entity_id, value.potential))
        .collect();
    assert_eq!(potentials.len(), world.graph.entities.len());
    assert_eq!(
        potentials.keys().copied().collect::<Vec<_>>(),
        world.graph.entities.keys().copied().collect::<Vec<_>>(),
    );
    assert_eq!(
        result.edges.len(),
        world
            .graph
            .edges
            .values()
            .filter(|edge| request.edge_kinds.contains(&edge.kind))
            .count(),
    );
    let mut balance = BTreeMap::<EntityId, i128>::new();
    let mut residual = BTreeMap::<EntityId, Vec<EntityId>>::new();
    let mut total_cost = 0u128;
    for edge in &result.edges {
        let original = world.graph.edges.get(&edge.edge_id).ok_or_else(missing)?;
        assert!(request.edge_kinds.contains(&original.kind));
        assert_eq!(
            (edge.from, edge.to, edge.revision),
            (original.from, original.to, original.revision)
        );
        assert_eq!(
            edge.capacity,
            integer(
                original
                    .fields
                    .get(&request.capacity_field)
                    .ok_or_else(missing)?
            )?
        );
        assert_eq!(
            edge.cost,
            integer(
                original
                    .fields
                    .get(&request.cost_field)
                    .ok_or_else(missing)?
            )?
        );
        assert!(edge.flow <= edge.capacity);
        *balance.entry(edge.from).or_default() -= i128::from(edge.flow);
        *balance.entry(edge.to).or_default() += i128::from(edge.flow);
        total_cost = total_cost
            .checked_add(u128::from(edge.flow) * u128::from(edge.cost))
            .ok_or_else(missing)?;
        let from = potentials.get(&edge.from).copied().ok_or_else(missing)?;
        let to = potentials.get(&edge.to).copied().ok_or_else(missing)?;
        if edge.flow < edge.capacity {
            let reduced = i128::from(edge.cost) + from - to;
            assert!(
                reduced >= 0,
                "negative forward reduced cost on {}: {reduced}",
                edge.edge_id
            );
            residual.entry(edge.from).or_default().push(edge.to);
        }
        if edge.flow > 0 {
            let reduced = -i128::from(edge.cost) + to - from;
            assert!(
                reduced >= 0,
                "negative reverse reduced cost on {}: {reduced}",
                edge.edge_id
            );
            residual.entry(edge.to).or_default().push(edge.from);
        }
    }
    assert_eq!(total_cost, result.total_cost);
    for id in world.graph.entities.keys() {
        let actual = balance.get(id).copied().unwrap_or_default();
        let expected = if *id == request.source {
            -i128::from(result.value)
        } else if *id == request.sink {
            i128::from(result.value)
        } else {
            0
        };
        assert_eq!(actual, expected, "flow balance at {id}");
    }
    if result.value == request.demand {
        assert!(result.minimum_cut.is_none());
    } else {
        let cut = result.minimum_cut.as_ref().ok_or_else(missing)?;
        let source_side: BTreeSet<_> = cut.source_side.iter().copied().collect();
        let sink_side: BTreeSet<_> = cut.sink_side.iter().copied().collect();
        assert!(cut.source_side.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(cut.sink_side.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(source_side.contains(&request.source));
        assert!(sink_side.contains(&request.sink));
        assert!(source_side.is_disjoint(&sink_side));
        assert_eq!(
            source_side.union(&sink_side).copied().collect::<Vec<_>>(),
            world.graph.entities.keys().copied().collect::<Vec<_>>()
        );
        let mut reachable = BTreeSet::from([request.source]);
        let mut queue = VecDeque::from([request.source]);
        while let Some(node) = queue.pop_front() {
            for &target in residual.get(&node).into_iter().flatten() {
                if reachable.insert(target) {
                    queue.push_back(target);
                }
            }
        }
        assert_eq!(reachable, source_side);
        let crossing: Vec<_> = result
            .edges
            .iter()
            .filter(|edge| source_side.contains(&edge.from) && sink_side.contains(&edge.to))
            .collect();
        assert_eq!(
            cut.cut_edges,
            crossing.iter().map(|edge| edge.edge_id).collect::<Vec<_>>()
        );
        let capacity: u128 = crossing.iter().map(|edge| u128::from(edge.capacity)).sum();
        assert_eq!(capacity, u128::from(result.value));
        assert_eq!(u128::from(cut.cut_capacity), capacity);
        assert!(crossing.iter().all(|edge| edge.flow == edge.capacity));
        assert!(
            result
                .edges
                .iter()
                .filter(|edge| sink_side.contains(&edge.from) && source_side.contains(&edge.to))
                .all(|edge| edge.flow == 0)
        );
    }
    Ok(())
}

/// Enumerate integer flows directly. This oracle does not use paths, potentials,
/// residual networks, max-flow or a second implementation of shortest paths.
fn assignment_oracle(
    size: usize,
    edges: &[InputEdge],
    source: usize,
    sink: usize,
    demand: u64,
) -> (u64, u128) {
    let assignments: u64 = edges.iter().map(|edge| edge.2 + 1).product();
    let mut best = (0, 0);
    for encoded in 0..assignments {
        let mut digits = encoded;
        let mut balance = vec![0i128; size + 1];
        let mut cost = 0u128;
        for &(from, to, capacity, unit_cost) in edges {
            let flow = digits % (capacity + 1);
            digits /= capacity + 1;
            balance[from as usize] -= i128::from(flow);
            balance[to as usize] += i128::from(flow);
            cost += u128::from(flow) * u128::from(unit_cost);
        }
        let value = -balance[source];
        if value < 0
            || value > i128::from(demand)
            || balance[sink] != value
            || (1..=size).any(|id| id != source && id != sink && balance[id] != 0)
        {
            continue;
        }
        let value = value as u64;
        if value > best.0 || (value == best.0 && cost < best.1) {
            best = (value, cost);
        }
    }
    best
}

#[test]
fn cheaper_routes_fill_before_expensive_capacity_and_keep_source_evidence() -> Result<()> {
    let world = world(
        4,
        &[
            (1, 2, 2, 1),
            (2, 4, 2, 1),
            (1, 3, 3, 2),
            (3, 4, 3, 3),
            (1, 4, 3, 20),
        ],
    );
    let result = run(&world, 1, 4, 4)?;
    assert_eq!(
        (result.value, result.unmet_demand, result.total_cost),
        (4, 0, 14)
    );
    assert_eq!(result.unit.as_deref(), Some("items"));
    assert_eq!(result.cost_unit.as_deref(), Some("labor_ticks/item"));
    assert_eq!(
        result
            .edges
            .iter()
            .map(|edge| edge.flow)
            .collect::<Vec<_>>(),
        [2, 2, 2, 2, 0]
    );
    verify(&world, &query(1, 4, 4), &result)
}

#[test]
fn exhaustive_direct_flow_assignments_prove_cost_and_delivery_on_small_digraphs() -> Result<()> {
    // 729 directed three-vertex graphs: each arc is absent, free capacity one,
    // or unit-cost capacity two. Three demands cover feasible and short models.
    let arcs = [(1, 2), (1, 3), (2, 1), (2, 3), (3, 1), (3, 2)];
    for encoded in 0..729 {
        let mut digits = encoded;
        let mut edges = Vec::new();
        for &(from, to) in &arcs {
            let state = digits % 3;
            digits /= 3;
            match state {
                1 => edges.push((from, to, 1, 0)),
                2 => edges.push((from, to, 2, 1)),
                _ => {}
            }
        }
        let world = world(3, &edges);
        for demand in [1, 2, 4] {
            let result = run(&world, 1, 3, demand)?;
            assert_eq!(
                (result.value, result.total_cost),
                assignment_oracle(3, &edges, 1, 3, demand),
                "encoding {encoded}, demand {demand}"
            );
            verify(&world, &query(1, 3, demand), &result)?;
        }
    }
    // All 256 capacity masks on a four-vertex network add branching, cycles,
    // self-loops and source-returning arcs, with independently varying costs.
    let arcs = [
        (1, 2),
        (1, 3),
        (2, 3),
        (3, 2),
        (2, 4),
        (3, 4),
        (4, 1),
        (2, 2),
    ];
    for mask in 0u64..256 {
        let edges: Vec<_> = arcs
            .iter()
            .enumerate()
            .map(|(offset, &(from, to))| {
                (from, to, (mask >> offset) & 1, (mask + offset as u64) % 4)
            })
            .collect();
        let world = world(4, &edges);
        for demand in [1, 2, 4] {
            let result = run(&world, 1, 4, demand)?;
            assert_eq!(
                (result.value, result.total_cost),
                assignment_oracle(4, &edges, 1, 4, demand),
                "mask {mask}, demand {demand}"
            );
            verify(&world, &query(1, 4, demand), &result)?;
        }
    }
    Ok(())
}

#[test]
fn reverse_residual_edges_repair_the_unique_cheapest_first_assignment() -> Result<()> {
    let world = world(
        6,
        &[
            (1, 2, 1, 0),
            (1, 3, 1, 0),
            (2, 4, 1, 0),
            (2, 5, 1, 2),
            (3, 4, 1, 1),
            (3, 5, 1, 100),
            (4, 6, 1, 0),
            (5, 6, 1, 0),
        ],
    );
    let first = run(&world, 1, 6, 1)?;
    assert_eq!(first.total_cost, 0);
    assert_eq!(flow(&first, 3)?, 1);
    let result = run(&world, 1, 6, 2)?;
    assert_eq!((result.value, result.total_cost), (2, 3));
    assert_eq!(flow(&result, 3)?, 0);
    assert_eq!(flow(&result, 4)?, 1);
    assert_eq!(flow(&result, 5)?, 1);
    assert_eq!(flow(&result, 6)?, 0);
    verify(&world, &query(1, 6, 2), &result)
}

#[test]
fn primal_and_dual_certificate_covers_edges_from_unreachable_components() -> Result<()> {
    // Updating only Dijkstra-reachable potentials makes the 3->2 reduced cost
    // negative after reaching vertex 2 at cost ten. The certificate is global.
    let world = world(
        5,
        &[(1, 2, 1, 10), (3, 2, 1, 0), (4, 3, 2, 0), (5, 4, 1, 0)],
    );
    for demand in [1, 2] {
        let result = run(&world, 1, 2, demand)?;
        assert_eq!((result.value, result.total_cost), (1, 10));
        verify(&world, &query(1, 2, demand), &result)?;
    }
    Ok(())
}

#[test]
fn parallel_antiparallel_zero_cycles_and_self_loops_keep_exact_edges() -> Result<()> {
    let world = world(
        3,
        &[
            (1, 2, 1, 2),
            (1, 2, 2, 1),
            (2, 1, 9, 0),
            (2, 3, 2, 3),
            (1, 3, 0, 0),
            (1, 1, u64::MAX, 0),
            (2, 2, u64::MAX, 1),
            (3, 3, u64::MAX, 0),
            (3, 1, 9, 0),
        ],
    );
    let result = run(&world, 1, 3, 2)?;
    assert_eq!((result.value, result.total_cost), (2, 8));
    assert_eq!(flow(&result, 2)?, 2);
    assert_eq!(flow(&result, 4)?, 2);
    for id in [1, 3, 5, 6, 7, 8, 9] {
        assert_eq!(flow(&result, id)?, 0);
    }
    verify(&world, &query(1, 3, 2), &result)?;
    // 729 multigraphs include distinct parallel capacities and free cycles.
    for encoded in 0u64..729 {
        let mut digits = encoded;
        let edges: Vec<_> = [(1, 2), (1, 2), (2, 1), (2, 3), (1, 1), (3, 3)]
            .into_iter()
            .enumerate()
            .map(|(offset, (from, to))| {
                let capacity = digits % 3;
                digits /= 3;
                (from, to, capacity, (offset as u64 + encoded) % 3)
            })
            .collect();
        let world = self::world(3, &edges);
        let result = run(&world, 1, 3, 3)?;
        assert_eq!(
            (result.value, result.total_cost),
            assignment_oracle(3, &edges, 1, 3, 3)
        );
        verify(&world, &query(1, 3, 3), &result)?;
    }
    Ok(())
}

#[test]
fn unmet_demand_retains_the_cheapest_maximum_and_independent_bottleneck() -> Result<()> {
    let world = world(
        6,
        &[
            (1, 2, 2, 1),
            (2, 4, 2, 1),
            (1, 3, 3, 4),
            (3, 4, 1, 1),
            (5, 6, 9, 0),
            (6, 5, 9, 0),
        ],
    );
    let result = run(&world, 1, 4, 7)?;
    assert_eq!(
        (result.value, result.unmet_demand, result.total_cost),
        (3, 4, 9)
    );
    assert_eq!(
        result.minimum_cut.as_ref().ok_or_else(missing)?.cut_edges,
        [1, 4].map(EdgeId::new)
    );
    verify(&world, &query(1, 4, 7), &result)
}

#[test]
fn empty_zero_and_unselected_networks_preserve_unknown_unit_and_cut_semantics() -> Result<()> {
    let empty = world(3, &[]);
    let result = run(&empty, 1, 3, 2)?;
    assert_eq!(
        (result.value, result.total_cost, result.unmet_demand),
        (0, 0, 2)
    );
    assert!(result.unit.is_none() && result.cost_unit.is_none());
    assert!(result.edges.is_empty());
    verify(&empty, &query(1, 3, 2), &result)?;
    let zero = world(4, &[(1, 2, 0, 7), (3, 4, 9, 3)]);
    let result = run(&zero, 1, 2, 1)?;
    assert_eq!((result.value, result.total_cost), (0, 0));
    assert_eq!(result.unit.as_deref(), Some("items"));
    assert_eq!(result.cost_unit.as_deref(), Some("labor_ticks/item"));
    verify(&zero, &query(1, 2, 1), &result)?;
    let mut unselected = zero;
    for edge in unselected.graph.edges.values_mut() {
        edge.kind = EdgeKind::Requires;
        edge.fields.clear();
    }
    unselected.refresh_hash();
    let result = run(&unselected, 1, 2, 1)?;
    assert!(result.unit.is_none() && result.cost_unit.is_none());
    assert!(result.edges.is_empty());
    verify(&unselected, &query(1, 2, 1), &result)
}

#[test]
fn all_four_facts_are_required_and_time_qualified_even_on_disconnected_zero_edges() -> Result<()> {
    let baseline = world(4, &[(1, 2, 2, 1), (3, 4, 0, 0)]);
    for field in ["capacity", "capacity_unit", "cost", "cost_unit"] {
        let mut missing_field = baseline.clone();
        missing_field
            .graph
            .edges
            .get_mut(&EdgeId::new(2))
            .ok_or_else(missing)?
            .fields
            .remove(field);
        missing_field.refresh_hash();
        assert!(
            matches!(run(&missing_field, 1, 2, 1), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
        for presence in [
            FactPresence::Absent,
            FactPresence::Unknown("unknown".to_owned()),
            FactPresence::Unsupported("unsupported".to_owned()),
            FactPresence::Omitted("omitted".to_owned()),
            FactPresence::Redacted("redacted".to_owned()),
            FactPresence::Stale(baseline.anchor()),
            FactPresence::Known(Value::U64(99)),
        ] {
            let mut changed = baseline.clone();
            fact_mut(&mut changed, 2, field)?.presence = Some(presence);
            changed.refresh_hash();
            assert!(
                matches!(run(&changed, 1, 2, 1), Err(error) if error.code == ErrorCode::PreconditionsFailed)
            );
        }
        let mut future = baseline.clone();
        fact_mut(&mut future, 2, field)?.observed_at = GameTick(11);
        future.refresh_hash();
        assert!(
            matches!(run(&future, 1, 2, 1), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    Ok(())
}

#[test]
fn each_capacity_cost_and_unit_source_must_be_independently_qualified() -> Result<()> {
    let baseline = world(2, &[(1, 2, 2, 3)]);
    let bad_sources = [
        (
            FactSource::DfhackField("network.cost".to_owned()),
            Digest32::ZERO,
        ),
        (FactSource::DfhackField(" ".to_owned()), scope()),
        (FactSource::DfhackField("x\n".to_owned()), scope()),
        (FactSource::DfhackField("x".repeat(257)), scope()),
        (FactSource::Derived("unregistered".to_owned()), scope()),
        (
            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
            scope(),
        ),
        (FactSource::AgentAssertion("network".to_owned()), scope()),
        (FactSource::Replay, scope()),
    ];
    for field in ["capacity", "capacity_unit", "cost", "cost_unit"] {
        for (source, digest) in &bad_sources {
            let mut changed = baseline.clone();
            let fact = fact_mut(&mut changed, 1, field)?;
            fact.source = source.clone();
            fact.source_digest = *digest;
            changed.refresh_hash();
            assert!(
                matches!(run(&changed, 1, 2, 1), Err(error) if error.code == ErrorCode::PreconditionsFailed)
            );
        }
        for name in ["dfmcp.lab-scenario/1", "dfmcp.reference-effects/1"] {
            let mut changed = baseline.clone();
            let fact = fact_mut(&mut changed, 1, field)?;
            fact.source = FactSource::Derived(name.to_owned());
            fact.source_digest = Digest32::ZERO;
            fact.presence = Some(FactPresence::Known(fact.value.clone()));
            changed.refresh_hash();
            assert_eq!(run(&changed, 1, 2, 1)?.total_cost, 3);
        }
    }
    Ok(())
}

#[test]
fn costs_and_capacities_are_nonnegative_and_unit_domains_never_mix() -> Result<()> {
    let baseline = world(4, &[(1, 2, 2, 3), (3, 4, 1, 1)]);
    for field in ["capacity", "cost"] {
        for value in [
            Value::I64(-1),
            Value::Text("infinity".to_owned()),
            Value::Null,
            Value::Bool(true),
        ] {
            let mut changed = baseline.clone();
            fact_mut(&mut changed, 2, field)?.value = value;
            changed.refresh_hash();
            assert!(
                matches!(run(&changed, 1, 2, 1), Err(error) if error.code == ErrorCode::PreconditionsFailed)
            );
        }
        let mut signed = baseline.clone();
        fact_mut(&mut signed, 2, field)?.value = Value::I64(1);
        signed.refresh_hash();
        assert_eq!(run(&signed, 1, 2, 1)?.total_cost, 3);
    }
    for field in ["capacity_unit", "cost_unit"] {
        for value in [
            Value::Text("different".to_owned()),
            Value::Text(String::new()),
            Value::Text(" \t".to_owned()),
            Value::Text("unit\n".to_owned()),
            Value::Text("é".repeat(65)),
            Value::U64(1),
        ] {
            let mut changed = baseline.clone();
            fact_mut(&mut changed, 2, field)?.value = value;
            changed.refresh_hash();
            assert!(
                matches!(run(&changed, 1, 2, 1), Err(error) if error.code == ErrorCode::PreconditionsFailed)
            );
        }
        let mut boundary = baseline.clone();
        for id in [1, 2] {
            fact_mut(&mut boundary, id, field)?.value = Value::Text("é".repeat(64));
        }
        boundary.refresh_hash();
        assert_eq!(run(&boundary, 1, 2, 1)?.total_cost, 3);
    }
    Ok(())
}

#[test]
fn full_u64_values_are_finite_and_checked_total_cost_overflow_refuses() -> Result<()> {
    let exact = world(2, &[(1, 2, u64::MAX, u64::MAX)]);
    let result = run(&exact, 1, 2, u64::MAX)?;
    assert_eq!(result.value, u64::MAX);
    assert_eq!(
        result.total_cost,
        u128::from(u64::MAX) * u128::from(u64::MAX)
    );
    verify(&exact, &query(1, 2, u64::MAX), &result)?;
    let overflow = world(3, &[(1, 2, u64::MAX, u64::MAX), (2, 3, u64::MAX, u64::MAX)]);
    assert!(
        matches!(run(&overflow, 1, 3, u64::MAX), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    // Unused high-cost capacities must not be multiplied or summed as flow.
    let bounded = world(
        4,
        &[
            (1, 2, u64::MAX, u64::MAX),
            (2, 4, 1, 0),
            (1, 3, u64::MAX, u64::MAX),
            (3, 4, 1, 0),
        ],
    );
    let result = run(&bounded, 1, 4, 2)?;
    assert_eq!(result.total_cost, 2 * u128::from(u64::MAX));
    verify(&bounded, &query(1, 4, 2), &result)
}

#[test]
fn input_work_and_frontier_bounds_refuse_complete_results() -> Result<()> {
    let world = world(4, &[(1, 2, 2, 0), (1, 3, 4, 1), (2, 4, 1, 1), (3, 4, 3, 0)]);
    let request = query(1, 4, 4);
    let baseline = run(&world, 1, 4, 4)?;
    let exact = minimum_cost_flow(
        &world,
        scope(),
        &request,
        GraphBudget {
            max_work: baseline.witness.work_units,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(exact.edges, baseline.edges);
    assert_eq!(exact.total_cost, baseline.total_cost);
    assert_eq!(exact.witness.work_units, baseline.witness.work_units);
    for budget in [
        GraphBudget {
            max_work: baseline.witness.work_units - 1,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_vertices: 3,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_edges: 3,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_frontier: 1,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_work: 0,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_work: 10_000_001,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_frontier: 0,
            ..GraphBudget::default()
        },
    ] {
        assert!(
            matches!(minimum_cost_flow(&world, scope(), &request, budget), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    let mut unselected = world;
    for edge in unselected.graph.edges.values_mut() {
        edge.kind = EdgeKind::Requires;
    }
    unselected.refresh_hash();
    assert!(
        matches!(minimum_cost_flow(&unselected, scope(), &request, GraphBudget { max_edges: 3, ..GraphBudget::default() }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn deep_flow_is_iterative_and_cancellation_survives_every_stage() -> Result<()> {
    let world = world(
        2_000,
        &(1..2_000)
            .map(|id| (id, id + 1, u64::MAX, 1))
            .collect::<Vec<_>>(),
    );
    let request = query(1, 2_000, u64::MAX);
    let mut calls = 0;
    let result =
        minimum_cost_flow_with_check(&world, scope(), &request, GraphBudget::default(), || {
            calls += 1;
            Ok(())
        })?;
    assert_eq!(result.value, u64::MAX);
    assert_eq!(result.total_cost, 1_999 * u128::from(u64::MAX));
    assert!(calls > 20);
    verify(&world, &request, &result)?;
    for fail_at in [1, 2, 3, 20, calls] {
        let mut checked = 0;
        let result =
            minimum_cost_flow_with_check(&world, scope(), &request, GraphBudget::default(), || {
                checked += 1;
                if checked == fail_at {
                    Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "injected minimum-cost-flow cancellation",
                    ))
                } else {
                    Ok(())
                }
            });
        assert!(
            matches!(result, Err(error) if error.message == "injected minimum-cost-flow cancellation")
        );
        assert_eq!(checked, fail_at);
    }
    Ok(())
}

#[test]
fn demand_terminals_fields_and_corrupt_source_identity_are_rejected() -> Result<()> {
    let baseline = world(2, &[(1, 2, 2, 1)]);
    for (source, sink, demand) in [
        (0, 2, 1),
        (1, 0, 1),
        (1, 1, 1),
        (99, 2, 1),
        (1, 99, 1),
        (1, 2, 0),
    ] {
        assert!(
            matches!(run(&baseline, source, sink, demand), Err(error) if error.code == ErrorCode::InvalidRequest)
        );
    }
    for field in [
        String::new(),
        " ".to_owned(),
        "x\n".to_owned(),
        "é".repeat(65),
    ] {
        for position in 0..4 {
            let mut request = query(1, 2, 1);
            match position {
                0 => request.capacity_field = field.clone(),
                1 => request.unit_field = field.clone(),
                2 => request.cost_field = field.clone(),
                _ => request.cost_unit_field = field.clone(),
            }
            assert!(
                matches!(minimum_cost_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
            );
        }
    }
    let names = ["capacity", "capacity_unit", "cost", "cost_unit"];
    for (left, name) in names.iter().enumerate() {
        for right in left + 1..4 {
            let mut request = query(1, 2, 1);
            match right {
                1 => request.unit_field = (*name).to_owned(),
                2 => request.cost_field = (*name).to_owned(),
                _ => request.cost_unit_field = (*name).to_owned(),
            }
            assert!(
                matches!(minimum_cost_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
            );
        }
    }
    let mut request = query(1, 2, 1);
    request.edge_kinds.clear();
    assert!(
        matches!(minimum_cost_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
    );
    request.edge_kinds = vec![EdgeKind::Uses; 65];
    assert!(
        matches!(minimum_cost_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let mut invalid_hash = baseline.clone();
    invalid_hash.paused = false;
    assert!(
        matches!(run(&invalid_hash, 1, 2, 1), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
    );
    for corrupt in 0..4 {
        let mut changed = baseline.clone();
        match corrupt {
            0 => {
                changed
                    .graph
                    .entities
                    .get_mut(&EntityId::new(1))
                    .ok_or_else(missing)?
                    .id = EntityId::new(77)
            }
            1 => {
                changed
                    .graph
                    .edges
                    .get_mut(&EdgeId::new(1))
                    .ok_or_else(missing)?
                    .id = EdgeId::new(77)
            }
            2 => {
                changed
                    .graph
                    .edges
                    .get_mut(&EdgeId::new(1))
                    .ok_or_else(missing)?
                    .from = EntityId::new(77)
            }
            _ => {
                changed
                    .graph
                    .edges
                    .get_mut(&EdgeId::new(1))
                    .ok_or_else(missing)?
                    .to = EntityId::new(77)
            }
        }
        changed.refresh_hash();
        assert!(
            matches!(run(&changed, 1, 2, 1), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
        );
    }
    Ok(())
}

#[test]
fn deterministic_witness_binds_units_provenance_generation_scope_demand_and_budget() -> Result<()> {
    let baseline = world(3, &[(1, 2, 3, 1), (2, 3, 3, 1)]);
    let original = run(&baseline, 1, 3, 2)?;
    assert_eq!(original, run(&baseline, 1, 3, 2)?);
    let mut normalized = query(1, 3, 2);
    normalized.edge_kinds.push(EdgeKind::Uses);
    assert_eq!(
        original,
        minimum_cost_flow(&baseline, scope(), &normalized, GraphBudget::default())?
    );
    for field in ["capacity", "capacity_unit", "cost", "cost_unit"] {
        let mut changed = baseline.clone();
        fact_mut(&mut changed, 1, field)?.observed_at = GameTick(9);
        changed.refresh_hash();
        let result = run(&changed, 1, 3, 2)?;
        assert_eq!(result.edges, original.edges);
        assert_ne!(
            result.witness.projection_digest,
            original.witness.projection_digest
        );
        assert_ne!(
            result.witness.decision_digest,
            original.witness.decision_digest
        );
    }
    for field in ["capacity_unit", "cost_unit"] {
        let mut changed = baseline.clone();
        for id in [1, 2] {
            fact_mut(&mut changed, id, field)?.value = Value::Text("other-source-unit".to_owned());
        }
        changed.refresh_hash();
        let result = run(&changed, 1, 3, 2)?;
        assert_eq!(result.edges, original.edges);
        assert_ne!(
            result.witness.projection_digest,
            original.witness.projection_digest
        );
        assert_ne!(
            result.witness.decision_digest,
            original.witness.decision_digest
        );
    }
    let mut replaced = baseline.clone();
    replaced
        .graph
        .entities
        .get_mut(&EntityId::new(2))
        .ok_or_else(missing)?
        .generation += 1;
    replaced.refresh_hash();
    assert_ne!(
        run(&replaced, 1, 3, 2)?.witness.projection_digest,
        original.witness.projection_digest
    );
    let other_scope = minimum_cost_flow(
        &baseline,
        Digest32::ZERO,
        &query(1, 3, 2),
        GraphBudget::default(),
    )?;
    assert_eq!(other_scope.edges, original.edges);
    assert_eq!(
        other_scope.witness.projection_digest,
        original.witness.projection_digest
    );
    assert_ne!(
        other_scope.witness.decision_digest,
        original.witness.decision_digest
    );
    let other_demand = run(&baseline, 1, 3, 3)?;
    assert_eq!(
        other_demand.witness.projection_digest,
        original.witness.projection_digest
    );
    assert_ne!(
        other_demand.witness.decision_digest,
        original.witness.decision_digest
    );
    let other_budget = minimum_cost_flow(
        &baseline,
        scope(),
        &query(1, 3, 2),
        GraphBudget {
            max_work: 999_999,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(other_budget.edges, original.edges);
    assert_ne!(
        other_budget.witness.decision_digest,
        original.witness.decision_digest
    );
    Ok(())
}

#[test]
fn full_width_edge_identity_and_custom_kind_tags_do_not_alias() -> Result<()> {
    let mut world = world(2, &[(1, 2, 2, 1), (1, 2, 3, 7)]);
    let original = world
        .graph
        .edges
        .remove(&EdgeId::new(1))
        .ok_or_else(missing)?;
    let wide = EdgeId::new(u128::MAX - 1);
    world.graph.edges.insert(
        wide,
        EdgeRecord {
            id: wide,
            ..original
        },
    );
    world
        .graph
        .edges
        .get_mut(&EdgeId::new(2))
        .ok_or_else(missing)?
        .kind = EdgeKind::Custom("uses".to_owned());
    world.refresh_hash();
    let selected = run(&world, 1, 2, 3)?;
    assert_eq!((selected.value, selected.total_cost), (2, 2));
    assert_eq!(
        selected
            .edges
            .iter()
            .map(|edge| edge.edge_id)
            .collect::<Vec<_>>(),
        [wide]
    );
    verify(&world, &query(1, 2, 3), &selected)?;
    let mut both = query(1, 2, 3);
    both.edge_kinds.push(EdgeKind::Custom("uses".to_owned()));
    let result = minimum_cost_flow(&world, scope(), &both, GraphBudget::default())?;
    assert_eq!((result.value, result.total_cost), (3, 9));
    verify(&world, &both, &result)
}
