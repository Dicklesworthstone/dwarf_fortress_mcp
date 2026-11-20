#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor,
    Result,
};
use dfmcp_world::graph_query::{
    GraphBudget, GraphDirection, WeightedRouteQuery, WeightedRouteRecord, WeightedRoutes,
    shortest_routes, shortest_routes_with_check,
};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactPresence, FactSource, Value,
    WorldGraph, WorldSnapshot,
};

fn scope() -> Digest32 {
    Digest32::of_bytes(b"authorized-weighted-route-fixture")
}

fn query(roots: &[u64]) -> WeightedRouteQuery {
    WeightedRouteQuery {
        roots: roots.iter().copied().map(EntityId::new).collect(),
        edge_kinds: vec![EdgeKind::Requires],
        direction: GraphDirection::Outgoing,
        cost_field: "cost".to_owned(),
    }
}

fn world(size: u64, edges: &[(u64, u64, u64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for id in 1..=size {
        let id = EntityId::new(id);
        graph.entities.insert(
            id,
            EntityRecord {
                id,
                generation: 2,
                revision: 3,
                kind: EntityKind::Building,
                label: format!("node-{id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (offset, &(from, to, weight)) in edges.iter().enumerate() {
        let id = EdgeId::new(offset as u128 + 1);
        graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 4,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::from([(
                    "cost".to_owned(),
                    Fact::known(
                        Value::U64(weight),
                        GameTick(10),
                        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )]),
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

fn run(snapshot: &WorldSnapshot, roots: &[u64]) -> Result<WeightedRoutes> {
    shortest_routes(snapshot, scope(), &query(roots), GraphBudget::default())
}

fn missing() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "missing weighted route fixture",
    )
}

fn entry(routes: &WeightedRoutes, id: u64) -> Result<&WeightedRouteRecord> {
    routes
        .entries
        .iter()
        .find(|entry| entry.entity_id == EntityId::new(id))
        .ok_or_else(missing)
}

fn fact_mut(snapshot: &mut WorldSnapshot, id: u128) -> Result<&mut Fact> {
    snapshot
        .graph
        .edges
        .get_mut(&EdgeId::new(id))
        .and_then(|edge| edge.fields.get_mut("cost"))
        .ok_or_else(missing)
}

#[test]
fn weighted_routes_choose_lower_total_cost_over_fewer_hops() -> Result<()> {
    let source = world(5, &[(1, 4, 20), (1, 2, 2), (2, 3, 3), (3, 4, 1)]);
    let routes = run(&source, &[1])?;
    assert_eq!(routes.witness.source_anchor, source.anchor());
    assert_eq!(routes.witness.authorization_scope_digest, scope());
    assert_eq!(routes.unreachable, vec![EntityId::new(5)]);
    assert_eq!(entry(&routes, 4)?.total_cost, 6);
    assert_eq!(entry(&routes, 4)?.hops, 3);
    assert!(
        routes
            .entries
            .iter()
            .all(|entry| entry.generation == 2 && entry.revision == 3)
    );
    let path = routes.path_to(EntityId::new(4))?.ok_or_else(missing)?;
    assert_eq!(path.root, EntityId::new(1));
    assert_eq!(path.total_cost, 6);
    assert_eq!(
        path.vertices,
        (1..=4).map(EntityId::new).collect::<Vec<_>>()
    );
    assert_eq!(
        path.edges,
        vec![
            (EdgeId::new(2), 4, 2),
            (EdgeId::new(3), 4, 3),
            (EdgeId::new(4), 4, 1)
        ]
    );
    assert_eq!(routes.path_to(EntityId::new(5))?, None);
    assert_eq!(routes.path_to(EntityId::new(99))?, None);
    assert_eq!(
        routes.path_to(EntityId::new(1))?.ok_or_else(missing)?.edges,
        vec![]
    );
    Ok(())
}

#[test]
fn equal_cost_ties_use_hops_source_parent_then_edge_without_zero_cycles() -> Result<()> {
    let source = world(
        7,
        &[
            (1, 3, 0),
            (3, 1, 0),
            (1, 4, 0),
            (3, 4, 0),
            (4, 5, 2),
            (3, 5, 2),
            (3, 5, 2),
            (2, 5, 2),
            (1, 6, 2),
            (2, 6, 2),
            (6, 6, 0),
        ],
    );
    let routes = run(&source, &[2, 1])?;
    // Equal cost but one hop wins over the lower-ID source's two-hop route.
    assert_eq!(entry(&routes, 5)?.root, EntityId::new(2));
    assert_eq!(entry(&routes, 5)?.hops, 1);
    // Equal cost and hops choose the lower source.
    assert_eq!(entry(&routes, 6)?.root, EntityId::new(1));
    // A zero cycle cannot replace a source's zero-hop route.
    assert_eq!(entry(&routes, 1)?.parent, None);
    assert_eq!(entry(&routes, 4)?.parent, Some(EntityId::new(1)));
    let single = run(&source, &[1])?;
    assert_eq!(entry(&single, 5)?.parent, Some(EntityId::new(3)));
    assert_eq!(entry(&single, 5)?.via_edge, Some(EdgeId::new(6)));
    assert_eq!(
        single
            .path_to(EntityId::new(5))?
            .ok_or_else(missing)?
            .vertices,
        vec![EntityId::new(1), EntityId::new(3), EntityId::new(5)]
    );
    Ok(())
}

#[test]
fn incoming_routes_reverse_edges_and_bind_direction_without_rewriting_weights() -> Result<()> {
    let source = world(5, &[(1, 2, 3), (2, 3, 4), (3, 4, 5), (1, 5, 1)]);
    let mut request = query(&[4]);
    let outgoing = shortest_routes(&source, scope(), &request, GraphBudget::default())?;
    request.direction = GraphDirection::Incoming;
    let incoming = shortest_routes(&source, scope(), &request, GraphBudget::default())?;
    let path = incoming.path_to(EntityId::new(1))?.ok_or_else(missing)?;
    assert_eq!(path.total_cost, 12);
    assert_eq!(
        path.vertices,
        vec![
            EntityId::new(4),
            EntityId::new(3),
            EntityId::new(2),
            EntityId::new(1)
        ]
    );
    assert_eq!(incoming.unreachable, vec![EntityId::new(5)]);
    assert_eq!(
        outgoing.witness.projection_digest,
        incoming.witness.projection_digest
    );
    assert_ne!(
        outgoing.witness.decision_digest,
        incoming.witness.decision_digest
    );
    request.direction = GraphDirection::Undirected;
    assert!(
        matches!(shortest_routes(&source, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::InvalidRequest)
    );
    Ok(())
}

#[test]
fn source_and_selector_order_are_canonical_and_unselected_costs_are_irrelevant() -> Result<()> {
    let mut source = world(4, &[(1, 3, 2), (2, 3, 1), (3, 4, 3)]);
    let mut request = query(&[2, 1]);
    let baseline = shortest_routes(&source, scope(), &request, GraphBudget::default())?;
    request.roots = vec![EntityId::new(1), EntityId::new(2), EntityId::new(1)];
    request.edge_kinds.push(EdgeKind::Requires);
    assert_eq!(
        baseline,
        shortest_routes(&source, scope(), &request, GraphBudget::default())?
    );
    source
        .graph
        .edges
        .get_mut(&EdgeId::new(3))
        .ok_or_else(missing)?
        .kind = EdgeKind::Uses;
    source
        .graph
        .edges
        .get_mut(&EdgeId::new(3))
        .ok_or_else(missing)?
        .fields
        .clear();
    source.refresh_hash();
    let selected = shortest_routes(&source, scope(), &request, GraphBudget::default())?;
    assert_eq!(selected.unreachable, vec![EntityId::new(4)]);
    request.edge_kinds.push(EdgeKind::Uses);
    assert!(
        matches!(shortest_routes(&source, scope(), &request, GraphBudget::default()), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn entire_selected_projection_requires_known_costs_even_when_unreachable() -> Result<()> {
    let mut source = world(4, &[(1, 2, 1), (3, 4, 9)]);
    source
        .graph
        .edges
        .get_mut(&EdgeId::new(2))
        .ok_or_else(missing)?
        .fields
        .clear();
    source.refresh_hash();
    assert!(
        matches!(run(&source, &[1]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
    );
    Ok(())
}

#[test]
fn unavailable_future_contradictory_and_unqualified_costs_refuse_whole_results() -> Result<()> {
    let baseline = world(2, &[(1, 2, 1)]);
    let mut invalid = vec![Fact::with_presence(
        FactPresence::Absent,
        GameTick(10),
        FactSource::Derived("dfmcp.lab-scenario/1".into()),
        Digest32::ZERO,
    )];
    for presence in [
        FactPresence::Unknown("unknown".into()),
        FactPresence::Unsupported("unsupported".into()),
        FactPresence::Omitted("omitted".into()),
        FactPresence::Redacted("redacted".into()),
        FactPresence::Stale(baseline.anchor()),
    ] {
        invalid.push(Fact::with_presence(
            presence,
            GameTick(10),
            FactSource::Derived("dfmcp.lab-scenario/1".into()),
            Digest32::ZERO,
        ));
    }
    for value in [
        Value::I64(-1),
        Value::Fixed { units: 1, scale: 0 },
        Value::Text("1".into()),
        Value::Null,
        Value::Bool(true),
    ] {
        invalid.push(Fact::known(
            value,
            GameTick(10),
            FactSource::Derived("dfmcp.lab-scenario/1".into()),
            Digest32::ZERO,
        ));
    }
    for (observed_at, source, digest) in [
        (
            GameTick(11),
            FactSource::Derived("dfmcp.lab-scenario/1".into()),
            Digest32::ZERO,
        ),
        (
            GameTick(10),
            FactSource::Derived("dfmcp.lab-scenario/1".into()),
            scope(),
        ),
        (
            GameTick(10),
            FactSource::Derived("arbitrary".into()),
            Digest32::ZERO,
        ),
        (
            GameTick(10),
            FactSource::AgentAssertion("agent".into()),
            scope(),
        ),
        (GameTick(10), FactSource::Replay, scope()),
        (
            GameTick(10),
            FactSource::DfhackField("cost".into()),
            Digest32::ZERO,
        ),
        (GameTick(10), FactSource::DfhackField(" ".into()), scope()),
        (
            GameTick(10),
            FactSource::DfhackField("cost\n".into()),
            scope(),
        ),
        (
            GameTick(10),
            FactSource::DfhackField("x".repeat(257)),
            scope(),
        ),
        (
            GameTick(11),
            FactSource::DfhackField("cost".into()),
            scope(),
        ),
    ] {
        invalid.push(Fact::known(Value::U64(1), observed_at, source, digest));
    }
    let mut mismatch = Fact::known(
        Value::U64(1),
        GameTick(10),
        FactSource::Derived("dfmcp.lab-scenario/1".into()),
        Digest32::ZERO,
    );
    mismatch.presence = Some(FactPresence::Known(Value::U64(2)));
    invalid.push(mismatch);
    for fact in invalid {
        let mut source = baseline.clone();
        *fact_mut(&mut source, 1)? = fact;
        source.refresh_hash();
        assert!(
            matches!(run(&source, &[1]), Err(error) if error.code == ErrorCode::PreconditionsFailed)
        );
    }
    for (value, tick, fact_source, digest) in [
        (
            Value::I64(0),
            GameTick(10),
            FactSource::Derived("dfmcp.reference-effects/1".into()),
            Digest32::ZERO,
        ),
        (
            Value::I64(3),
            GameTick(9),
            FactSource::DfhackField("edge.cost".into()),
            scope(),
        ),
    ] {
        let mut source = baseline.clone();
        *fact_mut(&mut source, 1)? =
            Fact::with_presence(FactPresence::Known(value), tick, fact_source, digest);
        source.refresh_hash();
        assert!(run(&source, &[1]).is_ok());
    }
    Ok(())
}

#[test]
fn checked_cost_arithmetic_preserves_u64_max_and_refuses_overflow() -> Result<()> {
    let exact = run(&world(3, &[(1, 2, u64::MAX), (2, 3, 0)]), &[1])?;
    assert_eq!(entry(&exact, 3)?.total_cost, u64::MAX);
    assert_eq!(
        exact
            .path_to(EntityId::new(3))?
            .ok_or_else(missing)?
            .total_cost,
        u64::MAX
    );
    for source in [
        world(3, &[(1, 2, u64::MAX), (2, 3, 1)]),
        world(2, &[(1, 2, u64::MAX), (2, 1, 1)]),
    ] {
        assert!(
            matches!(run(&source, &[1]), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    Ok(())
}

#[test]
fn exact_work_input_and_frontier_bounds_refuse_without_partial_coverage() -> Result<()> {
    let source = world(
        4,
        &[(1, 2, 10), (1, 3, 1), (3, 2, 1), (2, 4, 1), (3, 4, 10)],
    );
    let request = query(&[1]);
    let baseline = shortest_routes(&source, scope(), &request, GraphBudget::default())?;
    let exact = GraphBudget {
        max_work: baseline.witness.work_units,
        ..GraphBudget::default()
    };
    assert!(shortest_routes(&source, scope(), &request, exact).is_ok());
    for limit in [
        GraphBudget {
            max_work: baseline.witness.work_units - 1,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_vertices: 3,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_edges: 4,
            ..GraphBudget::default()
        },
        GraphBudget {
            max_frontier: 1,
            ..GraphBudget::default()
        },
    ] {
        assert!(
            matches!(shortest_routes(&source, scope(), &request, limit), Err(error) if error.code == ErrorCode::BudgetExceeded)
        );
    }
    // Replacing an expensive queued label does not leave a stale queue entry.
    let bounded = shortest_routes(
        &source,
        scope(),
        &request,
        GraphBudget {
            max_frontier: 2,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(entry(&bounded, 4)?.total_cost, 3);
    Ok(())
}

#[test]
fn long_routes_are_iterative_and_cancellation_remains_observable() -> Result<()> {
    let source = world(
        4_000,
        &(1..4_000)
            .map(|id| (id, id + 1, id % 2))
            .collect::<Vec<_>>(),
    );
    let routes = run(&source, &[1])?;
    let path = routes.path_to(EntityId::new(4_000))?.ok_or_else(missing)?;
    assert_eq!(path.vertices.len(), 4_000);
    assert_eq!(path.total_cost, 2_000);
    for fail_at in [1, 2, 3, 20] {
        let mut calls = 0;
        let result = shortest_routes_with_check(
            &source,
            scope(),
            &query(&[1]),
            GraphBudget::default(),
            || {
                calls += 1;
                if calls == fail_at {
                    Err(DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "injected cancellation",
                    ))
                } else {
                    Ok(())
                }
            },
        );
        assert!(matches!(result, Err(error) if error.message == "injected cancellation"));
        assert_eq!(calls, fail_at);
    }
    Ok(())
}

#[test]
fn witness_binds_cost_provenance_source_direction_scope_and_budget() -> Result<()> {
    let source = world(3, &[(1, 2, 2), (2, 3, 1)]);
    let baseline = run(&source, &[1])?;
    let mut changed = source.clone();
    fact_mut(&mut changed, 1)?.observed_at = GameTick(9);
    changed.refresh_hash();
    let provenance = run(&changed, &[1])?;
    assert_eq!(baseline.entries, provenance.entries);
    assert_ne!(
        baseline.witness.projection_digest,
        provenance.witness.projection_digest
    );
    *fact_mut(&mut changed, 1)? = Fact::known(
        Value::U64(1),
        GameTick(10),
        FactSource::Derived("dfmcp.lab-scenario/1".into()),
        Digest32::ZERO,
    );
    changed.refresh_hash();
    assert_ne!(
        baseline.witness.projection_digest,
        run(&changed, &[1])?.witness.projection_digest
    );
    let other_scope = shortest_routes(
        &source,
        Digest32::ZERO,
        &query(&[1]),
        GraphBudget::default(),
    )?;
    assert_eq!(baseline.entries, other_scope.entries);
    assert_ne!(
        baseline.witness.decision_digest,
        other_scope.witness.decision_digest
    );
    let other_budget = shortest_routes(
        &source,
        scope(),
        &query(&[1]),
        GraphBudget {
            max_work: 900_000,
            ..GraphBudget::default()
        },
    )?;
    assert_eq!(baseline.entries, other_budget.entries);
    assert_ne!(
        baseline.witness.decision_digest,
        other_budget.witness.decision_digest
    );
    assert_ne!(
        baseline.witness.decision_digest,
        run(&source, &[2])?.witness.decision_digest
    );
    Ok(())
}

#[test]
fn malformed_identities_and_invalid_requests_fail_closed() -> Result<()> {
    let source = world(2, &[(1, 2, 1)]);
    let mut malformed = source.clone();
    malformed
        .graph
        .entities
        .get_mut(&EntityId::new(1))
        .ok_or_else(missing)?
        .id = EntityId::new(99);
    malformed.refresh_hash();
    assert!(run(&malformed, &[1]).is_err());
    malformed = source.clone();
    malformed
        .graph
        .edges
        .get_mut(&EdgeId::new(1))
        .ok_or_else(missing)?
        .to = EntityId::new(99);
    malformed.refresh_hash();
    assert!(run(&malformed, &[1]).is_err());
    malformed = source.clone();
    malformed.tick = GameTick(11);
    assert!(run(&malformed, &[1]).is_err());
    for roots in [vec![], vec![0], vec![99], vec![1; 65]] {
        assert!(run(&source, &roots).is_err());
    }
    for field in ["", " ", "cost\n"] {
        let mut request = query(&[1]);
        request.cost_field = field.to_owned();
        assert!(shortest_routes(&source, scope(), &request, GraphBudget::default()).is_err());
    }
    let mut request = query(&[1]);
    request.edge_kinds.clear();
    assert!(shortest_routes(&source, scope(), &request, GraphBudget::default()).is_err());
    Ok(())
}

#[test]
fn path_reconstruction_rejects_tampered_public_cost_parent_and_source_records() -> Result<()> {
    let routes = run(&world(3, &[(1, 2, 1), (2, 3, 2)]), &[1])?;
    for mode in 0..7 {
        let mut changed = routes.clone();
        match mode {
            0 => changed.entries[2].total_cost = 99,
            1 => changed.entries[2].parent = Some(EntityId::new(3)),
            2 => changed.entries[2].root = EntityId::new(2),
            3 => changed.entries[2].edge_cost = None,
            4 => changed.entries[0].total_cost = 1,
            5 => changed.roots.clear(),
            _ => changed.entries.push(changed.entries[1].clone()),
        }
        assert!(changed.path_to(EntityId::new(3)).is_err());
    }
    Ok(())
}

#[test]
fn batch_paths_share_validated_records_preserve_order_and_check_cancellation() -> Result<()> {
    let source = world(300, &(1..299).map(|id| (id, id + 1, 1)).collect::<Vec<_>>());
    let routes = run(&source, &[1])?;
    let targets = [
        EntityId::new(299),
        EntityId::new(300),
        EntityId::new(1),
        EntityId::new(299),
    ];
    let paths = routes.paths_to(&targets)?;
    for (target, path) in targets.iter().zip(paths) {
        assert_eq!(routes.path_to(*target)?, path);
    }
    assert!(routes.paths_to(&[EntityId::new(1); 65]).is_err());
    let mut calls = 0;
    let result = routes.paths_to_with_check(&targets, || {
        calls += 1;
        if calls == 4 {
            Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "path reconstruction cancellation",
            ))
        } else {
            Ok(())
        }
    });
    assert!(matches!(result, Err(error) if error.message == "path reconstruction cancellation"));
    assert_eq!(calls, 4);
    let mut reordered = routes.clone();
    reordered.entries.swap(0, 1);
    assert!(reordered.path_to(EntityId::new(299)).is_err());
    Ok(())
}

// Deliberately independent oracle: enumerate every simple directed path in a
// tiny graph, rather than using a priority queue, relaxation or core helpers.
// Nonnegative costs plus hop minimization imply an optimal path is simple.
type OracleLabel = (u64, u32, u64, Option<u64>, Option<u128>);

fn oracle(
    size: u64,
    edges: &[(u64, u64, u64)],
    roots: &[u64],
    incoming: bool,
) -> BTreeMap<u64, OracleLabel> {
    let mut best = BTreeMap::new();
    for &root in roots {
        let mut stack = vec![(root, 0_u64, 0_u32, BTreeSet::from([root]), None, None)];
        while let Some((node, cost, hops, seen, parent, edge)) = stack.pop() {
            let candidate = (cost, hops, root, parent, edge);
            if best.get(&node).is_none_or(|prior| candidate < *prior) {
                best.insert(node, candidate);
            }
            for (offset, &(mut from, mut to, weight)) in edges.iter().enumerate() {
                if incoming {
                    std::mem::swap(&mut from, &mut to);
                }
                if from != node || seen.contains(&to) {
                    continue;
                }
                let mut next_seen = seen.clone();
                next_seen.insert(to);
                assert!(next_seen.len() <= size as usize);
                stack.push((
                    to,
                    cost + weight,
                    hops + 1,
                    next_seen,
                    Some(node),
                    Some(offset as u128 + 1),
                ));
            }
        }
    }
    best
}

#[test]
fn every_three_vertex_weighted_digraph_matches_independent_simple_path_oracle() -> Result<()> {
    let pairs = [(1, 2), (1, 3), (2, 1), (2, 3), (3, 1), (3, 2)];
    for encoding in 0_u32..4096 {
        let mut remaining = encoding;
        let mut edges = Vec::new();
        for (from, to) in pairs {
            let choice = remaining % 4;
            remaining /= 4;
            if choice != 0 {
                edges.push((from, to, [0, 0, 1, 4][choice as usize]));
            }
        }
        let source = world(3, &edges);
        for roots in [&[1][..], &[1, 3][..]] {
            for incoming in [false, true] {
                let mut request = query(roots);
                if incoming {
                    request.direction = GraphDirection::Incoming;
                }
                let routes = shortest_routes(&source, scope(), &request, GraphBudget::default())?;
                let expected = oracle(3, &edges, roots, incoming);
                let actual: BTreeMap<_, OracleLabel> = routes
                    .entries
                    .iter()
                    .map(|entry| {
                        (
                            entry.entity_id.get(),
                            (
                                entry.total_cost,
                                entry.hops,
                                entry.root.get(),
                                entry.parent.map(EntityId::get),
                                entry.via_edge.map(EdgeId::get),
                            ),
                        )
                    })
                    .collect();
                assert_eq!(
                    actual, expected,
                    "weighted graph {encoding}, roots {roots:?}, incoming {incoming}"
                );
                assert_eq!(
                    routes.unreachable,
                    (1..=3)
                        .filter(|id| !expected.contains_key(id))
                        .map(EntityId::new)
                        .collect::<Vec<_>>()
                );
                for entry in &routes.entries {
                    let path = routes.path_to(entry.entity_id)?.ok_or_else(missing)?;
                    assert_eq!(
                        path.edges.iter().map(|edge| edge.2).sum::<u64>(),
                        entry.total_cost
                    );
                    assert_eq!(path.edges.len(), entry.hops as usize);
                }
            }
        }
    }
    Ok(())
}
