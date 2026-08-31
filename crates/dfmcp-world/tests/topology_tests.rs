#![forbid(unsafe_code)]

//! Integration tests for WP-WLD-02 Directed Multigraph Topology & ABA Protection.

use std::collections::BTreeMap;

use dfmcp_core::{EdgeId, EntityId};
use dfmcp_world::topology::{
    AbaEntityValidator, detect_cycles, find_reachability, get_transitive_dependencies,
};
use dfmcp_world::{EdgeKind, EdgeRecord, EntityKind, EntityRecord, WorldGraph};

fn sample_entity(id: EntityId, generation: u32) -> EntityRecord {
    EntityRecord {
        id,
        generation,
        revision: 1,
        kind: EntityKind::Unit,
        label: format!("unit_{}", id.get()),
        fields: BTreeMap::new(),
    }
}

#[test]
fn test_multigraph_reachability_traversal() {
    let mut graph = WorldGraph::default();

    for i in 1..=5 {
        let id = EntityId::new(i);
        graph.entities.insert(id, sample_entity(id, 1));
    }

    // 1 -> 2 -> 3, 1 -> 4
    graph.edges.insert(
        EdgeId::new(1),
        EdgeRecord {
            id: EdgeId::new(1),
            from: EntityId::new(1),
            to: EntityId::new(2),
            kind: EdgeKind::ContainedIn,
            revision: 1,
            fields: BTreeMap::new(),
        },
    );
    graph.edges.insert(
        EdgeId::new(2),
        EdgeRecord {
            id: EdgeId::new(2),
            from: EntityId::new(2),
            to: EntityId::new(3),
            kind: EdgeKind::ContainedIn,
            revision: 1,
            fields: BTreeMap::new(),
        },
    );
    graph.edges.insert(
        EdgeId::new(3),
        EdgeRecord {
            id: EdgeId::new(3),
            from: EntityId::new(1),
            to: EntityId::new(4),
            kind: EdgeKind::ContainedIn,
            revision: 1,
            fields: BTreeMap::new(),
        },
    );

    let reachable_from_1 = find_reachability(&graph, EntityId::new(1), None);
    assert_eq!(reachable_from_1.len(), 4);
    assert!(reachable_from_1.contains(&EntityId::new(1)));
    assert!(reachable_from_1.contains(&EntityId::new(2)));
    assert!(reachable_from_1.contains(&EntityId::new(3)));
    assert!(reachable_from_1.contains(&EntityId::new(4)));
    assert!(!reachable_from_1.contains(&EntityId::new(5)));
}

#[test]
fn test_topological_sorting_dependencies() {
    let mut graph = WorldGraph::default();
    let n1 = EntityId::new(1);
    let n2 = EntityId::new(2);
    let n3 = EntityId::new(3);

    graph.entities.insert(n1, sample_entity(n1, 1));
    graph.entities.insert(n2, sample_entity(n2, 1));
    graph.entities.insert(n3, sample_entity(n3, 1));

    // Dependency DAG: 1 -> 2 -> 3
    graph.edges.insert(
        EdgeId::new(1),
        EdgeRecord {
            id: EdgeId::new(1),
            from: n1,
            to: n2,
            kind: EdgeKind::Requires,
            revision: 1,
            fields: BTreeMap::new(),
        },
    );
    graph.edges.insert(
        EdgeId::new(2),
        EdgeRecord {
            id: EdgeId::new(2),
            from: n2,
            to: n3,
            kind: EdgeKind::Requires,
            revision: 1,
            fields: BTreeMap::new(),
        },
    );

    let deps = get_transitive_dependencies(&graph, n1, EdgeKind::Requires);
    assert_eq!(deps, vec![n2, n3]);
    assert!(!detect_cycles(&graph, Some(EdgeKind::Requires)));
}

#[test]
fn test_aba_entity_validator() {
    let mut validator = AbaEntityValidator::new();
    let mut graph = WorldGraph::default();
    let e1 = EntityId::new(1);
    graph.entities.insert(e1, sample_entity(e1, 1));

    validator.seed_from_graph(&graph);
    assert!(validator.validate_reference(e1, 1).is_ok());
    assert!(validator.validate_reference(e1, 2).is_err());
}
