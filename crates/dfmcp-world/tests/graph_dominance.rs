#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{
    DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick,
    ObservationCursor, Result,
};
use dfmcp_world::graph_query::{
    DominanceQuery, GraphBudget, GraphDirection, analyze_dominators,
    analyze_dominators_with_check,
};
use dfmcp_world::{EdgeKind, EdgeRecord, EntityKind, EntityRecord, WorldGraph, WorldSnapshot};

fn snapshot(size: u64, pairs: &[(u64, u64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for id in 1..=size {
        graph.entities.insert(
            EntityId::new(id),
            EntityRecord {
                id: EntityId::new(id),
                generation: 1,
                revision: 1,
                kind: EntityKind::Building,
                label: format!("node {id}"),
                fields: BTreeMap::new(),
            },
        );
    }
    for (index, &(from, to)) in pairs.iter().enumerate() {
        let id = EdgeId::new(index as u128 + 1);
        graph.edges.insert(
            id,
            EdgeRecord {
                id,
                revision: 1,
                kind: EdgeKind::Requires,
                from: EntityId::new(from),
                to: EntityId::new(to),
                fields: BTreeMap::new(),
            },
        );
    }
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        true,
        graph,
    )
}

fn query(root: u64) -> DominanceQuery {
    DominanceQuery {
        root: EntityId::new(root),
        edge_kinds: vec![EdgeKind::Requires],
        direction: GraphDirection::Outgoing,
    }
}

fn scope() -> Digest32 {
    Digest32::of_bytes(b"authorized fixture")
}

#[test]
fn diamond_reconvergence_reports_unavoidable_nodes_not_one_chosen_path() -> Result<()> {
    // 7 -> 4 is an unreachable predecessor, not a bypass from root 1.
    let world = snapshot(7, &[(1, 2), (1, 3), (2, 4), (3, 4), (4, 5), (6, 7), (7, 6), (7, 4)]);
    let result = analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?;
    let parents: Vec<_> = result.vertices.iter().map(|v| {
        (v.entity_id.get(), v.immediate_dominator.map(EntityId::get), v.dominated_descendants)
    }).collect();
    assert_eq!(parents, vec![(1, None, 4), (2, Some(1), 0), (3, Some(1), 0), (4, Some(1), 1), (5, Some(4), 0)]);
    assert_eq!(result.unreachable, vec![EntityId::new(6), EntityId::new(7)]);
    assert_eq!(result.dominator_chain_to(EntityId::new(5))?, Some(vec![EntityId::new(1), EntityId::new(4), EntityId::new(5)]));
    assert_eq!(result.dominator_chain_to(EntityId::new(6))?, None);
    assert_eq!(result.dominator_chain_to(EntityId::new(99))?, None);
    assert_eq!(result.witness.source_anchor, world.anchor());
    Ok(())
}

#[test]
fn incoming_analysis_reverses_edges_without_rewriting_projection_identity() -> Result<()> {
    let world = snapshot(5, &[(1, 2), (2, 3), (3, 4), (1, 5)]);
    let mut request = query(4);
    let forward = analyze_dominators(&world, scope(), &request, GraphBudget::default())?;
    request.direction = GraphDirection::Incoming;
    let reverse = analyze_dominators(&world, scope(), &request, GraphBudget::default())?;
    assert_eq!(reverse.dominator_chain_to(EntityId::new(1))?, Some(vec![EntityId::new(4), EntityId::new(3), EntityId::new(2), EntityId::new(1)]));
    assert_eq!(reverse.unreachable, vec![EntityId::new(5)]);
    assert_eq!(forward.witness.projection_digest, reverse.witness.projection_digest);
    assert_ne!(forward.witness.decision_digest, reverse.witness.decision_digest);
    Ok(())
}

#[test]
fn explicit_edge_selection_changes_which_bypasses_exist() -> Result<()> {
    let mut world = snapshot(3, &[(1, 2), (2, 3), (1, 3)]);
    world.graph.edges.get_mut(&EdgeId::new(3)).unwrap().kind = EdgeKind::Uses;
    world.refresh_hash();
    let mut request = query(1);
    let selected = analyze_dominators(&world, scope(), &request, GraphBudget::default())?;
    assert_eq!(selected.vertices[2].immediate_dominator, Some(EntityId::new(2)));
    request.edge_kinds.push(EdgeKind::Uses);
    let mixed = analyze_dominators(&world, scope(), &request, GraphBudget::default())?;
    assert_eq!(mixed.vertices[2].immediate_dominator, Some(EntityId::new(1)));
    assert_ne!(selected.witness.projection_digest, mixed.witness.projection_digest);
    request.edge_kinds.reverse();
    request.edge_kinds.push(EdgeKind::Uses);
    assert_eq!(mixed, analyze_dominators(&world, scope(), &request, GraphBudget::default())?);
    Ok(())
}

#[test]
fn self_loops_parallel_edges_and_back_edges_do_not_invent_dominators() -> Result<()> {
    let world = snapshot(4, &[(1, 1), (1, 2), (1, 2), (1, 3), (2, 4), (3, 4), (4, 2), (4, 1), (4, 4)]);
    let result = analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?;
    assert_eq!(result.vertices[0].immediate_dominator, None);
    for record in result.vertices.iter().skip(1) {
        assert_eq!(record.immediate_dominator, Some(EntityId::new(1)));
        assert_eq!(record.dominated_descendants, 0);
    }
    Ok(())
}

#[test]
fn witness_replays_and_binds_scope_anchor_generation_and_edge_revision() -> Result<()> {
    let world = snapshot(3, &[(1, 2), (2, 3)]);
    let first = analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?;
    assert_eq!(first, analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?);
    let other_scope = analyze_dominators(&world, Digest32::of_bytes(b"other"), &query(1), GraphBudget::default())?;
    assert_eq!(first.vertices, other_scope.vertices);
    assert_ne!(first.witness.decision_digest, other_scope.witness.decision_digest);
    let mut changed = world.clone();
    changed.paused = false;
    changed.refresh_hash();
    let other_anchor = analyze_dominators(&changed, scope(), &query(1), GraphBudget::default())?;
    assert_eq!(first.witness.projection_digest, other_anchor.witness.projection_digest);
    assert_ne!(first.witness.decision_digest, other_anchor.witness.decision_digest);
    changed.graph.entities.get_mut(&EntityId::new(2)).unwrap().generation += 1;
    changed.graph.edges.get_mut(&EdgeId::new(1)).unwrap().revision += 1;
    changed.refresh_hash();
    let other_version = analyze_dominators(&changed, scope(), &query(1), GraphBudget::default())?;
    assert_ne!(first.witness.projection_digest, other_version.witness.projection_digest);
    assert_eq!(other_version.vertices[1].generation, 2);
    Ok(())
}

#[test]
fn exact_work_limit_succeeds_but_one_less_never_returns_partial_results() -> Result<()> {
    let world = snapshot(4, &[(1, 2), (2, 3), (3, 4), (4, 2)]);
    let result = analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?;
    let mut bounds = GraphBudget { max_work: result.witness.work_units, ..GraphBudget::default() };
    assert_eq!(result.vertices, analyze_dominators(&world, scope(), &query(1), bounds)?.vertices);
    bounds.max_work -= 1;
    assert!(matches!(analyze_dominators(&world, scope(), &query(1), bounds), Err(e) if e.code == ErrorCode::BudgetExceeded));
    for bounds in [
        GraphBudget { max_vertices: 3, ..GraphBudget::default() },
        GraphBudget { max_edges: 3, ..GraphBudget::default() },
        GraphBudget { max_frontier: 1, ..GraphBudget::default() },
        GraphBudget { max_work: 0, ..GraphBudget::default() },
    ] {
        assert!(matches!(analyze_dominators(&world, scope(), &query(1), bounds), Err(e) if e.code == ErrorCode::BudgetExceeded));
    }
    Ok(())
}

#[test]
fn invalid_roots_directions_selections_and_snapshots_fail_closed() {
    let world = snapshot(2, &[(1, 2)]);
    for request in [
        query(99),
        query(0),
        DominanceQuery { edge_kinds: Vec::new(), ..query(1) },
        DominanceQuery { direction: GraphDirection::Undirected, ..query(1) },
    ] {
        assert!(matches!(analyze_dominators(&world, scope(), &request, GraphBudget::default()), Err(e) if e.code == ErrorCode::InvalidRequest));
    }
    let mut broken = world.clone();
    broken.paused = false;
    assert!(matches!(analyze_dominators(&broken, scope(), &query(1), GraphBudget::default()), Err(e) if e.code == ErrorCode::InternalInvariantViolation));
    let dangling = snapshot(2, &[(1, 99)]);
    assert!(matches!(analyze_dominators(&dangling, scope(), &query(1), GraphBudget::default()), Err(e) if e.code == ErrorCode::InternalInvariantViolation));
}

#[test]
fn cooperative_failure_aborts_before_build_during_analysis_and_before_return() {
    let large = snapshot(200, &(1..200).map(|i| (i, i + 1)).collect::<Vec<_>>());
    for fail_at in [1, 2, 3, 4] {
        let mut calls = 0;
        let result = analyze_dominators_with_check(&large, scope(), &query(1), GraphBudget::default(), || {
            calls += 1;
            if calls == fail_at { Err(DfmcpError::new(ErrorCode::Conflict, "injected caller cancellation")) } else { Ok(()) }
        });
        assert!(matches!(result, Err(e) if e.code == ErrorCode::Conflict));
        assert_eq!(calls, fail_at);
    }
    let small = snapshot(1, &[]);
    let mut calls = 0;
    let result = analyze_dominators_with_check(&small, scope(), &query(1), GraphBudget::default(), || {
        calls += 1;
        if calls == 3 { Err(DfmcpError::new(ErrorCode::Conflict, "final cancellation")) } else { Ok(()) }
    });
    assert!(matches!(result, Err(e) if e.code == ErrorCode::Conflict));
    assert_eq!(calls, 3);
}

#[test]
fn long_chain_uses_no_recursion_and_linear_impact_accumulation() -> Result<()> {
    let size = 4_000;
    let world = snapshot(size, &(1..size).map(|i| (i, i + 1)).collect::<Vec<_>>());
    let result = analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?;
    assert_eq!(result.vertices.len(), size as usize);
    assert_eq!(result.vertices[0].dominated_descendants, size - 1);
    assert_eq!(result.fixed_point_passes, 2);
    assert!(result.witness.work_units < 30 * size);
    assert_eq!(result.dominator_chain_to(EntityId::new(size))?.unwrap().len(), size as usize);
    Ok(())
}

// Deliberately does not use DFS rank, parent intersection, or dominator equations.
fn reachable(size: u64, pairs: &[(u64, u64)], root: u64, removed: u64) -> BTreeSet<u64> {
    if root == removed { return BTreeSet::new(); }
    let mut seen = BTreeSet::from([root]);
    let mut queue = VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        for &(from, to) in pairs {
            if from == node && to != removed && to <= size && seen.insert(to) {
                queue.push_back(to);
            }
        }
    }
    seen
}

#[test]
fn all_directed_graphs_through_four_vertices_match_vertex_removal_oracle() -> Result<()> {
    for size in 1..=4 {
        let possible: Vec<_> = (1..=size).flat_map(|a| (1..=size).filter(move |&b| a != b).map(move |b| (a, b))).collect();
        for mask in 0..(1u64 << possible.len()) {
            let pairs: Vec<_> = possible.iter().enumerate().filter(|(bit, _)| mask & (1 << bit) != 0).map(|(_, &pair)| pair).collect();
            let world = snapshot(size, &pairs);
            for root in 1..=size {
                let result = analyze_dominators(&world, scope(), &query(root), GraphBudget::default())?;
                let baseline = reachable(size, &pairs, root, 0);
                assert_eq!(result.vertices.iter().map(|v| v.entity_id.get()).collect::<BTreeSet<_>>(), baseline);
                assert_eq!(result.unreachable.iter().map(|v| v.get()).collect::<BTreeSet<_>>(), (1..=size).filter(|v| !baseline.contains(v)).collect());
                for record in &result.vertices {
                    let node = record.entity_id.get();
                    let after_removal = reachable(size, &pairs, root, node);
                    let lost = baseline.difference(&after_removal).filter(|&&v| v != node).count();
                    assert_eq!(record.dominated_descendants, lost as u64, "mask={mask} root={root} node={node}");
                    let mandatory: BTreeSet<_> = baseline.iter().copied().filter(|&removed| !reachable(size, &pairs, root, removed).contains(&node)).collect();
                    let chain = result.dominator_chain_to(record.entity_id)?.unwrap();
                    assert_eq!(chain.iter().map(|v| v.get()).collect::<BTreeSet<_>>(), mandatory, "mask={mask} root={root} node={node}");
                }
            }
        }
    }
    Ok(())
}

#[test]
fn forged_parent_cycles_duplicates_and_missing_parents_are_rejected() -> Result<()> {
    let world = snapshot(3, &[(1, 2), (2, 3)]);
    let result = analyze_dominators(&world, scope(), &query(1), GraphBudget::default())?;
    let mut cycle = result.clone();
    cycle.vertices[1].immediate_dominator = Some(EntityId::new(3));
    assert!(cycle.dominator_chain_to(EntityId::new(3)).is_err());
    let mut duplicate = result.clone();
    duplicate.vertices.push(duplicate.vertices[1].clone());
    assert!(duplicate.dominator_chain_to(EntityId::new(3)).is_err());
    let mut missing = result;
    missing.vertices[2].immediate_dominator = Some(EntityId::new(99));
    assert!(missing.dominator_chain_to(EntityId::new(3)).is_err());
    Ok(())
}
