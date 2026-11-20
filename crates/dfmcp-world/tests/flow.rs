#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor,
    Result,
};
use dfmcp_world::graph_query::{
    FLOW_POLICY, FlowAnalysis, FlowEdge, FlowQuery, GraphBudget, maximum_flow,
    maximum_flow_with_check,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value,
    WorldGraph, WorldSnapshot,
};

fn scope() -> Digest32 {
    Digest32::of_bytes(b"authorized-finite-flow-fixture")
}

fn query(source: u64, sink: u64) -> FlowQuery {
    FlowQuery {
        source: EntityId::new(source),
        sink: EntityId::new(sink),
        edge_kinds: vec![EdgeKind::Requires],
        capacity_field: "capacity".to_owned(),
        unit_field: "capacity_unit".to_owned(),
    }
}

fn observed(value: Value, field: &str) -> Fact {
    Fact::known(
        value,
        GameTick(10),
        FactSource::DfhackField(field.to_owned()),
        Digest32::of_bytes(b"observed-network-fixture"),
    )
}

fn world(size: u64, edges: &[(u64, u64, u64)]) -> WorldSnapshot {
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
    for (offset, &(from, to, capacity)) in edges.iter().enumerate() {
        let id = EdgeId::new(offset as u128 + 1);
        graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 4,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::from([
                    (
                        "capacity".to_owned(),
                        observed(Value::U64(capacity), "network.capacity"),
                    ),
                    (
                        "capacity_unit".to_owned(),
                        observed(Value::Text("items/tick".to_owned()), "network.unit"),
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
        "missing flow fixture",
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

fn edge(result: &FlowAnalysis, id: u128) -> Result<&FlowEdge> {
    result
        .edges
        .iter()
        .find(|edge| edge.edge_id == EdgeId::new(id))
        .ok_or_else(missing)
}

fn run(world: &WorldSnapshot, source: u64, sink: u64) -> Result<FlowAnalysis> {
    maximum_flow(world, scope(), &query(source, sink), GraphBudget::default())
}

/// This verifier deliberately derives balances, residual reachability and the
/// cut independently of the production network or augmentation algorithm.
fn verify(world: &WorldSnapshot, query: &FlowQuery, result: &FlowAnalysis) -> Result<()> {
    assert_eq!(result.source, query.source);
    assert_eq!(result.sink, query.sink);
    assert_eq!(result.witness.source_anchor, world.anchor());
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
    assert!(result.source_side.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(result.sink_side.windows(2).all(|pair| pair[0] < pair[1]));
    let source_side: BTreeSet<_> = result.source_side.iter().copied().collect();
    let sink_side: BTreeSet<_> = result.sink_side.iter().copied().collect();
    assert!(source_side.contains(&query.source));
    assert!(sink_side.contains(&query.sink));
    assert!(source_side.is_disjoint(&sink_side));
    assert_eq!(
        source_side
            .union(&sink_side)
            .copied()
            .collect::<BTreeSet<_>>(),
        world.graph.entities.keys().copied().collect(),
    );
    assert_eq!(
        result.edges.len(),
        world
            .graph
            .edges
            .values()
            .filter(|edge| query.edge_kinds.contains(&edge.kind))
            .count(),
    );
    let mut incoming = BTreeMap::<EntityId, u128>::new();
    let mut outgoing = BTreeMap::<EntityId, u128>::new();
    let mut residual = BTreeMap::<EntityId, Vec<EntityId>>::new();
    let mut cut = Vec::new();
    let mut cut_capacity = 0u128;
    for flow in &result.edges {
        let original = world.graph.edges.get(&flow.edge_id).ok_or_else(missing)?;
        assert!(query.edge_kinds.contains(&original.kind));
        assert_eq!(
            (flow.from, flow.to, flow.revision),
            (original.from, original.to, original.revision)
        );
        let capacity = match original
            .fields
            .get(&query.capacity_field)
            .and_then(Fact::known_value)
        {
            Some(Value::U64(value)) => Some(*value),
            Some(Value::I64(value)) => u64::try_from(*value).ok(),
            _ => None,
        }
        .ok_or_else(missing)?;
        assert_eq!(flow.capacity, capacity);
        assert!(flow.flow <= capacity);
        *incoming.entry(flow.to).or_default() += u128::from(flow.flow);
        *outgoing.entry(flow.from).or_default() += u128::from(flow.flow);
        if flow.flow < flow.capacity {
            residual.entry(flow.from).or_default().push(flow.to);
        }
        if flow.flow > 0 {
            residual.entry(flow.to).or_default().push(flow.from);
        }
        if source_side.contains(&flow.from) && sink_side.contains(&flow.to) {
            cut.push(flow.edge_id);
            cut_capacity += u128::from(flow.capacity);
            assert_eq!(flow.flow, flow.capacity);
        }
        if sink_side.contains(&flow.from) && source_side.contains(&flow.to) {
            assert_eq!(flow.flow, 0);
        }
    }
    for id in world.graph.entities.keys() {
        let incoming = incoming.get(id).copied().unwrap_or_default();
        let outgoing = outgoing.get(id).copied().unwrap_or_default();
        if *id == query.source {
            assert_eq!(outgoing, incoming + u128::from(result.value));
        } else if *id == query.sink {
            assert_eq!(incoming, outgoing + u128::from(result.value));
        } else {
            assert_eq!(incoming, outgoing);
        }
    }
    let mut reachable = BTreeSet::from([query.source]);
    let mut queue = VecDeque::from([query.source]);
    while let Some(node) = queue.pop_front() {
        for &target in residual.get(&node).into_iter().flatten() {
            if reachable.insert(target) {
                queue.push_back(target);
            }
        }
    }
    assert_eq!(reachable, source_side);
    assert_eq!(result.cut_edges, cut);
    assert_eq!(u128::from(result.value), cut_capacity);
    assert_eq!(u128::from(result.cut_capacity), cut_capacity);
    Ok(())
}

/// Exhaustive cut enumeration uses the original edge list only. It does not
/// traverse a residual graph or call another implementation of max flow.
fn cut_oracle(size: u64, edges: &[(u64, u64, u64)], source: u64, sink: u64) -> u128 {
    (0..(1u64 << size))
        .filter(|mask| mask & (1 << (source - 1)) != 0 && mask & (1 << (sink - 1)) == 0)
        .map(|mask| {
            edges
                .iter()
                .filter(|(from, to, _)| {
                    mask & (1 << (from - 1)) != 0 && mask & (1 << (to - 1)) == 0
                })
                .map(|(_, _, capacity)| u128::from(*capacity))
                .sum::<u128>()
        })
        .min()
        .unwrap_or_default()
}

#[test]
fn flow_and_cut_prove_a_bottleneck_with_exact_source_identity() -> Result<()> {
    let world = world(
        6,
        &[
            (1, 2, 16),
            (1, 3, 13),
            (2, 3, 10),
            (3, 2, 4),
            (2, 4, 12),
            (4, 3, 9),
            (3, 5, 14),
            (5, 4, 7),
            (4, 6, 20),
            (5, 6, 4),
        ],
    );
    let result = run(&world, 1, 6)?;
    assert_eq!(result.value, 23);
    assert_eq!(result.unit.as_deref(), Some("items/tick"));
    assert_eq!(result.source_side, [1, 2, 3, 5].map(EntityId::new));
    assert_eq!(result.sink_side, [4, 6].map(EntityId::new));
    assert_eq!(result.cut_edges, [5, 8, 10].map(EdgeId::new));
    assert_eq!(result.witness.authorization_scope_digest, scope());
    assert_eq!(
        FLOW_POLICY,
        "observed-directed-finite-capacity-dinic-stable-edge/1"
    );
    verify(&world, &query(1, 6), &result)
}

#[test]
fn exhaustive_small_digraphs_match_independent_cut_enumeration() -> Result<()> {
    // All three-state graphs on 3 vertices: each directed edge is absent or has
    // capacity 1 or 2. Every distinct source/sink pair is checked (4,374 flows).
    let arcs = [(1, 2), (1, 3), (2, 1), (2, 3), (3, 1), (3, 2)];
    for encoded in 0..729 {
        let mut digits = encoded;
        let mut edges = Vec::new();
        for &(from, to) in &arcs {
            let capacity = digits % 3;
            digits /= 3;
            if capacity != 0 {
                edges.push((from, to, capacity));
            }
        }
        let world = world(3, &edges);
        for source in 1..=3 {
            for sink in 1..=3 {
                if source == sink {
                    continue;
                }
                let result = run(&world, source, sink)?;
                assert_eq!(
                    u128::from(result.value),
                    cut_oracle(3, &edges, source, sink),
                    "encoding {encoded}, {source}->{sink}"
                );
                verify(&world, &query(source, sink), &result)?;
            }
        }
    }
    // All 4,096 simple directed binary-capacity graphs on four vertices add
    // longer and branching paths, cycles and independent cut placements.
    let arcs: Vec<_> = (1..=4)
        .flat_map(|from| {
            (1..=4)
                .filter(move |&to| to != from)
                .map(move |to| (from, to))
        })
        .collect();
    for mask in 0..4096 {
        let edges: Vec<_> = arcs
            .iter()
            .enumerate()
            .filter(|(bit, _)| mask & (1 << bit) != 0)
            .map(|(_, &(from, to))| (from, to, 1))
            .collect();
        let world = world(4, &edges);
        let result = run(&world, 1, 4)?;
        assert_eq!(
            u128::from(result.value),
            cut_oracle(4, &edges, 1, 4),
            "four-vertex mask {mask}"
        );
        verify(&world, &query(1, 4), &result)?;
    }
    Ok(())
}

#[test]
fn residual_reverse_arcs_repair_an_earlier_stable_edge_choice() -> Result<()> {
    // First phase takes 1-2-4-6. Reaching value 2 requires later cancellation
    // of edge 2->4 through the residual path 1-3-4-2-5-6.
    let world = world(
        6,
        &[
            (1, 2, 1),
            (2, 4, 1),
            (4, 6, 1),
            (1, 3, 1),
            (3, 4, 1),
            (2, 5, 1),
            (5, 6, 1),
        ],
    );
    let result = run(&world, 1, 6)?;
    assert_eq!(result.value, 2);
    assert_eq!(edge(&result, 2)?.flow, 0);
    assert!(
        result
            .edges
            .iter()
            .filter(|edge| edge.edge_id != EdgeId::new(2))
            .all(|edge| edge.flow == 1)
    );
    verify(&world, &query(1, 6), &result)
}

#[test]
fn parallel_antiparallel_self_loop_and_zero_edges_keep_original_identity() -> Result<()> {
    let edges = [
        (1, 2, 2),
        (1, 2, 3),
        (2, 1, 7),
        (2, 3, 4),
        (1, 3, 0),
        (1, 1, u64::MAX),
        (2, 2, u64::MAX),
        (3, 3, u64::MAX),
        (3, 1, 9),
    ];
    let world = world(3, &edges);
    let result = run(&world, 1, 3)?;
    assert_eq!(result.value, 4);
    assert_eq!(edge(&result, 1)?.flow, 2);
    assert_eq!(edge(&result, 2)?.flow, 2);
    for id in [3, 5, 6, 7, 8, 9] {
        assert_eq!(edge(&result, id)?.flow, 0);
    }
    assert_eq!(result.cut_edges, [4, 5].map(EdgeId::new));
    verify(&world, &query(1, 3), &result)?;
    // Independent capacities for multigraph loops and antiparallel arcs.
    for encoded in 0..729 {
        let mut digits = encoded;
        let edges: Vec<_> = [(1, 2), (1, 2), (2, 1), (2, 3), (1, 1), (3, 3)]
            .into_iter()
            .map(|(from, to)| {
                let capacity = digits % 3;
                digits /= 3;
                (from, to, capacity)
            })
            .collect();
        let world = self::world(3, &edges);
        let result = run(&world, 1, 3)?;
        assert_eq!(u128::from(result.value), cut_oracle(3, &edges, 1, 3));
        verify(&world, &query(1, 3), &result)?;
    }
    Ok(())
}

#[test]
fn empty_zero_and_disconnected_projections_have_complete_partition_results() -> Result<()> {
    let empty = world(4, &[]);
    let result = run(&empty, 1, 4)?;
    assert_eq!(result.value, 0);
    assert_eq!(result.unit, None);
    assert!(result.edges.is_empty() && result.cut_edges.is_empty());
    assert_eq!(result.source_side, [EntityId::new(1)]);
    verify(&empty, &query(1, 4), &result)?;
    let zero = world(4, &[(1, 4, 0), (2, 3, 8)]);
    let result = run(&zero, 1, 4)?;
    assert_eq!(result.value, 0);
    assert_eq!(result.unit.as_deref(), Some("items/tick"));
    assert_eq!(result.cut_edges, [EdgeId::new(1)]);
    assert_eq!(result.sink_side, [2, 3, 4].map(EntityId::new));
    verify(&zero, &query(1, 4), &result)?;
    let mut selected_empty = zero.clone();
    for edge in selected_empty.graph.edges.values_mut() {
        edge.kind = EdgeKind::Custom("unselected".to_owned());
        edge.fields.clear();
    }
    selected_empty.refresh_hash();
    let result = run(&selected_empty, 1, 4)?;
    assert_eq!(result.unit, None);
    assert!(result.edges.is_empty());
    verify(&selected_empty, &query(1, 4), &result)
}

#[test]
fn selected_edges_require_both_facts_even_outside_source_reachability() -> Result<()> {
    let baseline = world(4, &[(1, 2, 2), (3, 4, 0)]);
    for field in ["capacity", "capacity_unit"] {
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
            matches!(run(&missing_field, 1, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
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
                matches!(run(&changed, 1, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
            );
        }
        let mut future = baseline.clone();
        fact_mut(&mut future, 2, field)?.observed_at = GameTick(11);
        future.refresh_hash();
        assert!(
            matches!(run(&future, 1, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    Ok(())
}

#[test]
fn capacity_and_unit_sources_must_be_qualified_independently() -> Result<()> {
    let baseline = world(2, &[(1, 2, 2)]);
    let bad_sources = [
        (
            FactSource::DfhackField("network.capacity".to_owned()),
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
    for field in ["capacity", "capacity_unit"] {
        for (source, digest) in &bad_sources {
            let mut changed = baseline.clone();
            let fact = fact_mut(&mut changed, 1, field)?;
            fact.source = source.clone();
            fact.source_digest = *digest;
            changed.refresh_hash();
            assert!(
                matches!(run(&changed, 1, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
            );
        }
        for name in ["dfmcp.lab-scenario/1", "dfmcp.reference-effects/1"] {
            let mut changed = baseline.clone();
            let fact = fact_mut(&mut changed, 1, field)?;
            fact.source = FactSource::Derived(name.to_owned());
            fact.source_digest = Digest32::ZERO;
            fact.presence = Some(FactPresence::Known(fact.value.clone()));
            changed.refresh_hash();
            assert_eq!(run(&changed, 1, 2)?.value, 2);
        }
    }
    Ok(())
}

#[test]
fn no_negative_or_default_capacity_and_no_mixed_or_unbounded_unit() -> Result<()> {
    let baseline = world(4, &[(1, 2, 2), (3, 4, 4)]);
    for value in [
        Value::I64(-1),
        Value::Text("infinity".to_owned()),
        Value::Null,
    ] {
        let mut changed = baseline.clone();
        fact_mut(&mut changed, 2, "capacity")?.value = value;
        changed.refresh_hash();
        assert!(
            matches!(run(&changed, 1, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    let mut signed = baseline.clone();
    fact_mut(&mut signed, 1, "capacity")?.value = Value::I64(2);
    signed.refresh_hash();
    assert_eq!(run(&signed, 1, 2)?.value, 2);
    for value in [
        Value::Text("different dimension".to_owned()),
        Value::Text("".to_owned()),
        Value::Text(" \t".to_owned()),
        Value::Text("items/tick\n".to_owned()),
        Value::Text("é".repeat(65)),
        Value::U64(1),
    ] {
        let mut changed = baseline.clone();
        fact_mut(&mut changed, 2, "capacity_unit")?.value = value;
        changed.refresh_hash();
        assert!(
            matches!(run(&changed, 1, 2), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    let mut boundary = baseline.clone();
    for id in [1, 2] {
        fact_mut(&mut boundary, id, "capacity_unit")?.value = Value::Text("é".repeat(64));
    }
    boundary.refresh_hash();
    assert_eq!(
        run(&boundary, 1, 2)?.unit.as_deref(),
        Some("é".repeat(64).as_str())
    );
    Ok(())
}

#[test]
fn finite_u64_max_is_a_capacity_and_overflow_refuses_the_whole_result() -> Result<()> {
    let exact = world(3, &[(1, 2, u64::MAX), (2, 3, u64::MAX)]);
    let result = run(&exact, 1, 3)?;
    assert_eq!(result.value, u64::MAX);
    verify(&exact, &query(1, 3), &result)?;
    let overflow = world(2, &[(1, 2, u64::MAX), (1, 2, 1)]);
    assert!(matches!(run(&overflow, 1, 2), Err(error) if error.code == ErrorCode::BudgetExceeded));
    // Huge unrelated capacities and outgoing sums are not rejected merely
    // because their sum exceeds u64; the maximum flow/cut here fits exactly.
    let bounded = world(
        4,
        &[
            (1, 2, u64::MAX),
            (1, 2, u64::MAX),
            (2, 4, 3),
            (2, 3, u64::MAX),
            (3, 2, u64::MAX),
        ],
    );
    let result = run(&bounded, 1, 4)?;
    assert_eq!(result.value, 3);
    verify(&bounded, &query(1, 4), &result)
}

#[test]
fn work_input_and_frontier_limits_refuse_without_partial_answers() -> Result<()> {
    let world = world(4, &[(1, 2, 2), (1, 3, 4), (2, 4, 1), (3, 4, 3)]);
    let request = query(1, 4);
    let baseline = run(&world, 1, 4)?;
    let exact = maximum_flow(
        &world,
        scope(),
        &request,
        GraphBudget {
            max_work: baseline.witness.work_units,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(exact.edges, baseline.edges);
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
            matches!(maximum_flow(&world, scope(), &request, budget), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    let mut unselected = world.clone();
    for edge in unselected.graph.edges.values_mut() {
        edge.kind = EdgeKind::Custom("ignored".to_owned());
    }
    unselected.refresh_hash();
    assert!(
        matches!(maximum_flow(&unselected, scope(), &request, GraphBudget { max_edges: 3, ..GraphBudget::default() }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn deep_network_is_iterative_and_checks_cancellation_through_final_publication() -> Result<()> {
    let world = world(
        4_000,
        &(1..4_000)
            .map(|id| (id, id + 1, u64::MAX))
            .collect::<Vec<_>>(),
    );
    let request = query(1, 4_000);
    let mut calls = 0;
    let result =
        maximum_flow_with_check(&world, scope(), &request, GraphBudget::default(), || {
            calls += 1;
            Ok(())
        })?;
    assert_eq!(result.value, u64::MAX);
    assert!(calls > 20);
    verify(&world, &request, &result)?;
    assert!(
        matches!(maximum_flow(&world, scope(), &request, GraphBudget { max_frontier: 3_999, ..GraphBudget::default() }), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    for fail_at in [1, 2, 3, 20, calls] {
        let mut checked = 0;
        let result =
            maximum_flow_with_check(&world, scope(), &request, GraphBudget::default(), || {
                checked += 1;
                if checked == fail_at {
                    Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "injected flow cancellation",
                    ))
                } else {
                    Ok(())
                }
            });
        assert!(matches!(result, Err(error) if error.message == "injected flow cancellation"));
        assert_eq!(checked, fail_at);
    }
    Ok(())
}

#[test]
fn query_identity_and_source_corruption_are_rejected() -> Result<()> {
    let baseline = world(2, &[(1, 2, 2)]);
    for (source, sink) in [(0, 2), (1, 0), (1, 1), (99, 2), (1, 99)] {
        assert!(
            matches!(run(&baseline, source, sink), Err(error) if error.code == ErrorCode::InvalidRequest)
        );
    }
    for field in [
        "".to_owned(),
        " ".to_owned(),
        "x\n".to_owned(),
        "é".repeat(65),
    ] {
        for capacity in [true, false] {
            let mut request = query(1, 2);
            if capacity {
                request.capacity_field = field.clone();
            } else {
                request.unit_field = field.clone();
            }
            assert!(
                matches!(maximum_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
            );
        }
    }
    let mut request = query(1, 2);
    request.unit_field = request.capacity_field.clone();
    assert!(
        matches!(maximum_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
    );
    request = query(1, 2);
    request.edge_kinds.clear();
    assert!(
        matches!(maximum_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
    );
    request.edge_kinds = vec![EdgeKind::Requires; 65];
    assert!(
        matches!(maximum_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    request.edge_kinds = vec![EdgeKind::Custom("x".repeat(129))];
    assert!(
        matches!(maximum_flow(&baseline, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::BudgetExceeded)
    );
    let mut invalid_hash = baseline.clone();
    invalid_hash.paused = false;
    assert!(
        matches!(run(&invalid_hash, 1, 2), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
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
            matches!(run(&changed, 1, 2), Err(error) if error.code == ErrorCode::InternalInvariantViolation)
        );
    }
    Ok(())
}

#[test]
fn witness_binds_provenance_units_scope_terminals_and_budget_deterministically() -> Result<()> {
    let baseline = world(3, &[(1, 2, 2), (2, 3, 2)]);
    let original = run(&baseline, 1, 3)?;
    assert_eq!(original, run(&baseline, 1, 3)?);
    let mut normalized = query(1, 3);
    normalized.edge_kinds.push(EdgeKind::Requires);
    assert_eq!(
        original,
        maximum_flow(&baseline, scope(), &normalized, GraphBudget::default())?
    );
    for field in ["capacity", "capacity_unit"] {
        let mut changed = baseline.clone();
        fact_mut(&mut changed, 1, field)?.observed_at = GameTick(9);
        changed.refresh_hash();
        let result = run(&changed, 1, 3)?;
        assert_eq!(original.edges, result.edges);
        assert_ne!(
            original.witness.projection_digest,
            result.witness.projection_digest
        );
        assert_ne!(
            original.witness.decision_digest,
            result.witness.decision_digest
        );
    }
    let mut changed = baseline.clone();
    for id in [1, 2] {
        fact_mut(&mut changed, id, "capacity_unit")?.value = Value::Text("jobs/turn".to_owned());
    }
    changed.refresh_hash();
    let result = run(&changed, 1, 3)?;
    assert_eq!(result.edges, original.edges);
    assert_ne!(result.unit, original.unit);
    assert_ne!(
        result.witness.projection_digest,
        original.witness.projection_digest
    );
    let other_scope = maximum_flow(
        &baseline,
        Digest32::ZERO,
        &query(1, 3),
        GraphBudget::default(),
    )?;
    assert_eq!(original.edges, other_scope.edges);
    assert_eq!(
        original.witness.projection_digest,
        other_scope.witness.projection_digest
    );
    assert_ne!(
        original.witness.decision_digest,
        other_scope.witness.decision_digest
    );
    let other_terminals = run(&baseline, 1, 2)?;
    assert_eq!(
        original.witness.projection_digest,
        other_terminals.witness.projection_digest
    );
    assert_ne!(
        original.witness.decision_digest,
        other_terminals.witness.decision_digest
    );
    let other_budget = maximum_flow(
        &baseline,
        scope(),
        &query(1, 3),
        GraphBudget {
            max_work: 999_999,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(original.edges, other_budget.edges);
    assert_ne!(
        original.witness.decision_digest,
        other_budget.witness.decision_digest
    );
    Ok(())
}

#[test]
fn full_width_edge_ids_and_selected_kind_tags_are_preserved() -> Result<()> {
    let mut world = world(2, &[(1, 2, 4), (1, 2, 6)]);
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
        .kind = EdgeKind::Custom("Requires".to_owned());
    world.refresh_hash();
    let selected = run(&world, 1, 2)?;
    assert_eq!(selected.value, 4);
    assert_eq!(
        selected
            .edges
            .iter()
            .map(|edge| edge.edge_id)
            .collect::<Vec<_>>(),
        [wide]
    );
    verify(&world, &query(1, 2), &selected)?;
    let mut request = query(1, 2);
    request
        .edge_kinds
        .push(EdgeKind::Custom("Requires".to_owned()));
    let both = maximum_flow(&world, scope(), &request, GraphBudget::default())?;
    assert_eq!(both.value, 10);
    assert_eq!(
        both.edges
            .iter()
            .map(|edge| edge.edge_id)
            .collect::<Vec<_>>(),
        [EdgeId::new(2), wide]
    );
    verify(&world, &request, &both)
}
