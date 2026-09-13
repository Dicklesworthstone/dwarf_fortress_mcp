use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Digest32, EdgeId, EntityId, ErrorCode, FortressId, GameTick, ObservationCursor, Result};
use dfmcp_world::graph_query::{
    GraphBudget, GraphDirection, GraphTraversalQuery, analyze_dependencies, traverse_graph,
};
use dfmcp_world::{EdgeKind, EdgeRecord, EntityKind, EntityRecord, WorldGraph, WorldSnapshot};

fn world(count: u64, edges: &[(u64, u64)]) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    for id in 1..=count {
        graph.entities.insert(EntityId::new(id), EntityRecord {
            id: EntityId::new(id), generation: 1, revision: 1,
            kind: EntityKind::Job, label: format!("job-{id}"), fields: BTreeMap::new(),
        });
    }
    for (offset, &(from, to)) in edges.iter().enumerate() {
        let id = EdgeId::new(offset as u128 + 1);
        graph.edges.insert(id, EdgeRecord {
            id, revision: 1, kind: EdgeKind::Requires, from: EntityId::new(from),
            to: EntityId::new(to), fields: BTreeMap::new(),
        });
    }
    WorldSnapshot::new(FortressId::new(1), GameTick(10), ObservationCursor::ORIGIN, true, graph)
}

fn scope() -> Digest32 { Digest32::of_bytes(b"authorized-fixture-projection") }
fn ids(values: &[EntityId]) -> Vec<u64> { values.iter().map(|id| id.get()).collect() }
fn query(roots: &[u64], direction: GraphDirection, max_depth: u32) -> GraphTraversalQuery {
    GraphTraversalQuery {
        roots: roots.iter().map(|&id| EntityId::new(id)).collect(),
        edge_kinds: vec![EdgeKind::Requires], direction, max_depth,
    }
}

#[test]
fn diamond_ties_have_one_canonical_shortest_path() -> Result<()> {
    let snapshot = world(4, &[(1, 3), (3, 4), (1, 2), (2, 4)]);
    let result = traverse_graph(&snapshot, scope(), &query(&[1], GraphDirection::Outgoing, 10), GraphBudget::default())?;
    let path = result.observed_path_to(EntityId::new(4))?.ok_or_else(|| {
        dfmcp_core::DfmcpError::new(ErrorCode::InternalInvariantViolation, "missing fixture path")
    })?;
    assert_eq!(ids(&path.vertices), vec![1, 2, 4]);
    assert_eq!(path.edges, vec![(EdgeId::new(3), 1), (EdgeId::new(4), 1)]);
    assert!(result.depth_frontier.is_empty());
    assert_eq!(result.witness.source_anchor, snapshot.anchor());
    assert!(result.witness.work_units <= GraphBudget::default().max_work);
    Ok(())
}

#[test]
fn source_order_duplicates_and_parallel_edges_are_deterministic() -> Result<()> {
    let snapshot = world(4, &[(1, 3), (1, 3), (2, 3), (3, 4)]);
    let left = traverse_graph(&snapshot, scope(), &query(&[2, 1, 2], GraphDirection::Outgoing, 10), GraphBudget::default())?;
    let right = traverse_graph(&snapshot, scope(), &query(&[1, 2], GraphDirection::Outgoing, 10), GraphBudget::default())?;
    assert_eq!(left, right);
    assert_eq!(left.visits[2].root, EntityId::new(1));
    assert_eq!(left.visits[2].via_edge, Some(EdgeId::new(1)));
    Ok(())
}

#[test]
fn depth_cutoff_is_explicit_and_does_not_claim_unreachability() -> Result<()> {
    let snapshot = world(4, &[(1, 2), (2, 3), (3, 4)]);
    let result = traverse_graph(&snapshot, scope(), &query(&[1], GraphDirection::Outgoing, 2), GraphBudget::default())?;
    assert_eq!(ids(&result.depth_frontier), vec![3]);
    assert!(result.observed_path_to(EntityId::new(4))?.is_none());
    let root_only = traverse_graph(&snapshot, scope(), &query(&[1], GraphDirection::Outgoing, 0), GraphBudget::default())?;
    assert_eq!(root_only.visits.len(), 1);
    assert_eq!(ids(&root_only.depth_frontier), vec![1]);
    let cycle = world(3, &[(1, 2), (2, 3), (3, 1)]);
    let covered = traverse_graph(&cycle, scope(), &query(&[1], GraphDirection::Outgoing, 2), GraphBudget::default())?;
    assert!(covered.depth_frontier.is_empty());
    Ok(())
}

#[test]
fn reverse_and_undirected_traversal_preserve_edge_witnesses() -> Result<()> {
    let snapshot = world(3, &[(1, 2), (2, 3)]);
    let incoming = traverse_graph(&snapshot, scope(), &query(&[3], GraphDirection::Incoming, 5), GraphBudget::default())?;
    assert_eq!(incoming.visits.iter().map(|v| v.entity_id.get()).collect::<Vec<_>>(), vec![3, 2, 1]);
    assert_eq!(incoming.visits[1].via_edge, Some(EdgeId::new(2)));
    let both = traverse_graph(&snapshot, scope(), &query(&[2], GraphDirection::Undirected, 5), GraphBudget::default())?;
    assert_eq!(both.visits.iter().map(|v| v.entity_id.get()).collect::<Vec<_>>(), vec![2, 1, 3]);
    Ok(())
}

#[test]
fn dependency_order_and_critical_chain_put_requirements_first() -> Result<()> {
    let snapshot = world(4, &[(1, 2), (1, 3), (2, 4), (3, 4)]);
    let result = analyze_dependencies(&snapshot, scope(), &[EdgeKind::Requires], GraphBudget::default())?;
    assert_eq!(result.dependency_order.as_deref().map(ids), Some(vec![4, 2, 3, 1]));
    assert_eq!(result.critical_chain.as_deref().map(ids), Some(vec![1, 2, 4]));
    assert!(result.blocked_by_cycles.is_empty());
    assert!(result.components.iter().all(|component| !component.cyclic));
    Ok(())
}

#[test]
fn cycles_and_all_their_dependents_are_explained_without_a_fake_schedule() -> Result<()> {
    let snapshot = world(5, &[(1, 2), (2, 1), (3, 1), (4, 3)]);
    let result = analyze_dependencies(&snapshot, scope(), &[EdgeKind::Requires], GraphBudget::default())?;
    assert_eq!(ids(&result.components[0].members), vec![1, 2]);
    assert_eq!(result.components[0].id, EntityId::new(1));
    assert!(result.components[0].cyclic);
    assert_eq!(ids(&result.blocked_by_cycles), vec![1, 2, 3, 4]);
    assert!(result.dependency_order.is_none());
    assert!(result.critical_chain.is_none());
    let self_loop = analyze_dependencies(&world(1, &[(1, 1)]), scope(), &[], GraphBudget::default())?;
    assert!(self_loop.components[0].cyclic);
    assert_eq!(ids(&self_loop.blocked_by_cycles), vec![1]);
    Ok(())
}

#[test]
fn empty_and_disconnected_graphs_have_a_real_total_order() -> Result<()> {
    let empty = analyze_dependencies(&world(0, &[]), scope(), &[], GraphBudget::default())?;
    assert_eq!(empty.dependency_order, Some(Vec::new()));
    assert_eq!(empty.critical_chain, Some(Vec::new()));
    let disconnected = analyze_dependencies(&world(3, &[]), scope(), &[], GraphBudget::default())?;
    assert_eq!(disconnected.dependency_order.as_deref().map(ids), Some(vec![1, 2, 3]));
    Ok(())
}

#[test]
fn bad_inputs_and_exhausted_budgets_never_return_partial_success() {
    let snapshot = world(4, &[(1, 2), (1, 3), (1, 4)]);
    for budget in [
        GraphBudget { max_vertices: 3, ..GraphBudget::default() },
        GraphBudget { max_edges: 2, ..GraphBudget::default() },
        GraphBudget { max_frontier: 1, ..GraphBudget::default() },
        GraphBudget { max_work: 1, ..GraphBudget::default() },
    ] {
        assert!(matches!(traverse_graph(&snapshot, scope(), &query(&[1], GraphDirection::Outgoing, 10), budget),
            Err(error) if error.code == ErrorCode::BudgetExceeded));
    }
    assert!(traverse_graph(&snapshot, scope(), &query(&[99], GraphDirection::Outgoing, 1), GraphBudget::default()).is_err());
    let mut invalid = snapshot.clone();
    invalid.paused = false;
    assert!(matches!(analyze_dependencies(&invalid, scope(), &[], GraphBudget::default()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
    let dangling = world(2, &[(1, 99)]);
    assert!(matches!(analyze_dependencies(&dangling, scope(), &[], GraphBudget::default()),
        Err(error) if error.code == ErrorCode::InternalInvariantViolation));
}

#[test]
fn kind_and_scope_identity_do_not_alias() -> Result<()> {
    let snapshot = world(3, &[(1, 2), (2, 3)]);
    let selected = analyze_dependencies(&snapshot, scope(), &[EdgeKind::Requires], GraphBudget::default())?;
    let other = analyze_dependencies(&snapshot, scope(), &[EdgeKind::Custom("requires".to_owned())], GraphBudget::default())?;
    assert_ne!(selected.witness.projection_digest, other.witness.projection_digest);
    assert_eq!(other.dependency_order.as_deref().map(ids), Some(vec![1, 2, 3]));
    let rescope = analyze_dependencies(&snapshot, Digest32::of_bytes(b"other-authorized-scope"), &[EdgeKind::Requires], GraphBudget::default())?;
    assert_eq!(selected.components, rescope.components);
    assert_ne!(selected.witness.decision_digest, rescope.witness.decision_digest);
    Ok(())
}

#[test]
fn corrupted_path_records_fail_instead_of_looping() -> Result<()> {
    let snapshot = world(2, &[(1, 2)]);
    let mut result = traverse_graph(&snapshot, scope(), &query(&[1], GraphDirection::Outgoing, 5), GraphBudget::default())?;
    result.visits[1].parent = Some(EntityId::new(2));
    assert!(result.observed_path_to(EntityId::new(2)).is_err());
    Ok(())
}

#[test]
fn exhaustive_three_vertex_graphs_match_independent_closure_and_distance_oracles() -> Result<()> {
    for mask in 0u16..512 {
        let mut edges = Vec::new();
        let mut distance = [[99usize; 3]; 3];
        for (i, row) in distance.iter_mut().enumerate() {
            row[i] = 0;
        }
        for (i, row) in distance.iter_mut().enumerate() {
            for (j, value) in row.iter_mut().enumerate() {
                if mask & (1 << (i * 3 + j)) != 0 {
                    edges.push((i as u64 + 1, j as u64 + 1));
                    *value = (*value).min(1);
                }
            }
        }
        for k in 0..3 {
            for i in 0..3 {
                for j in 0..3 {
                    distance[i][j] = distance[i][j].min(distance[i][k] + distance[k][j]);
                }
            }
        }
        let snapshot = world(3, &edges);
        let analysis = analyze_dependencies(&snapshot, scope(), &[], GraphBudget::default())?;
        let mut component_of = [EntityId::NIL; 3];
        let cyclic: BTreeSet<_> = (0..3).filter(|&i| {
            mask & (1 << (i * 3 + i)) != 0
                || (0..3).any(|j| i != j && distance[i][j] < 99 && distance[j][i] < 99)
        }).collect();
        assert_eq!(analysis.dependency_order.is_some(), cyclic.is_empty(), "mask={mask}");
        for component in &analysis.components {
            for id in &component.members {
                component_of[id.get() as usize - 1] = component.id;
                assert_eq!(component.cyclic, cyclic.contains(&(id.get() as usize - 1)), "mask={mask}");
            }
        }
        for i in 0..3 {
            for j in 0..3 {
                assert_eq!(component_of[i] == component_of[j], distance[i][j] < 99 && distance[j][i] < 99, "mask={mask}");
            }
        }
        let expected_blocked: Vec<_> = (0..3).filter(|&i| cyclic.iter().any(|&c| distance[i][c] < 99))
            .map(|i| i as u64 + 1).collect();
        assert_eq!(ids(&analysis.blocked_by_cycles), expected_blocked, "mask={mask}");
        if let Some(order) = analysis.dependency_order {
            let rank: BTreeMap<_, _> = order.iter().enumerate().map(|(i, id)| (id.get(), i)).collect();
            for (from, to) in &edges {
                assert!(rank[to] < rank[from], "mask={mask}");
            }
        }
        let traversal = traverse_graph(&snapshot, scope(), &query(&[1], GraphDirection::Outgoing, 3), GraphBudget::default())?;
        for vertex in 0..3 {
            let found = traversal.visits.iter().find(|visit| visit.entity_id.get() == vertex as u64 + 1);
            assert_eq!(found.map(|visit| visit.depth as usize), (distance[0][vertex] < 99).then_some(distance[0][vertex]), "mask={mask}");
        }
        assert!(traversal.depth_frontier.is_empty());
    }
    Ok(())
}
