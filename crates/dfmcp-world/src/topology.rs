#![forbid(unsafe_code)]

//! Directed Multigraph Topology Analysis and Entity Generation ABA Protection.
//!
//! WP-WLD-02: Enforces monotonic entity generation counters to eliminate ABA memory hazards
//! (INV-004) and provides cycle detection, topological sorting, and reachability traversal.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::{DfmcpError, EntityId, ErrorCode, Result};

use crate::model::{EdgeKind, WorldGraph};

/// Validates entity lookups against generation counters to prevent ABA hazards.
#[derive(Clone, Debug, Default)]
pub struct AbaEntityValidator {
    generations: BTreeMap<EntityId, u32>,
}

impl AbaEntityValidator {
    #[must_use]
    pub fn new() -> Self {
        Self {
            generations: BTreeMap::new(),
        }
    }

    /// Seed generation records from an existing world graph.
    pub fn seed_from_graph(&mut self, graph: &WorldGraph) {
        self.generations.clear();
        for (id, record) in &graph.entities {
            self.generations.insert(*id, record.generation);
        }
    }

    /// Validate that an entity reference matches current generation.
    pub fn validate_reference(&self, id: EntityId, expected_generation: u32) -> Result<()> {
        match self.generations.get(&id) {
            Some(&current_gen) => {
                if current_gen == expected_generation {
                    Ok(())
                } else {
                    Err(DfmcpError::new(
                        ErrorCode::Conflict,
                        format!(
                            "ABA hazard detected for entity {}: expected generation {}, active generation {}",
                            id.get(),
                            expected_generation,
                            current_gen
                        ),
                    ))
                }
            }
            None => Err(DfmcpError::new(
                ErrorCode::Conflict,
                format!("entity {} does not exist or has been deleted", id.get()),
            )),
        }
    }

    /// Increment generation upon entity retirement/recycling.
    pub fn retire_and_advance_generation(&mut self, id: EntityId) -> Result<u32> {
        if id == EntityId::NIL {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "cannot track the reserved nil entity identifier",
            ));
        }
        let entry = self.generations.entry(id).or_insert(0);
        *entry = entry.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "entity generation counter exhausted",
            )
        })?;
        Ok(*entry)
    }
}

/// Compute all entities reachable from a starting entity along directed edges.
pub fn find_reachability(
    graph: &WorldGraph,
    from: EntityId,
    filter_kind: Option<EdgeKind>,
) -> BTreeSet<EntityId> {
    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();

    if !graph.entities.contains_key(&from) {
        return visited;
    }

    queue.push_back(from);
    visited.insert(from);

    // Build adjacency index
    let mut adj: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();
    for edge in graph.edges.values() {
        if let Some(ref kind) = filter_kind
            && &edge.kind != kind
        {
            continue;
        }
        adj.entry(edge.from).or_default().push(edge.to);
    }

    while let Some(current) = queue.pop_front() {
        if let Some(neighbors) = adj.get(&current) {
            for &next in neighbors {
                if visited.insert(next) {
                    queue.push_back(next);
                }
            }
        }
    }

    visited
}

/// Detect whether the graph contains cycles.
pub fn detect_cycles(graph: &WorldGraph, filter_kind: Option<EdgeKind>) -> bool {
    let mut in_degree: BTreeMap<EntityId, usize> = BTreeMap::new();
    let mut adj: BTreeMap<EntityId, Vec<EntityId>> = BTreeMap::new();

    // Initialize in-degree for all entities in graph
    for &id in graph.entities.keys() {
        in_degree.insert(id, 0);
    }

    for edge in graph.edges.values() {
        if let Some(ref kind) = filter_kind
            && &edge.kind != kind
        {
            continue;
        }
        adj.entry(edge.from).or_default().push(edge.to);
        *in_degree.entry(edge.to).or_insert(0) += 1;
        in_degree.entry(edge.from).or_insert(0);
    }

    let mut queue: VecDeque<EntityId> = in_degree
        .iter()
        .filter_map(|(&id, &deg)| if deg == 0 { Some(id) } else { None })
        .collect();

    let mut visited_count = 0;
    while let Some(node) = queue.pop_front() {
        visited_count += 1;
        if let Some(neighbors) = adj.get(&node) {
            for &next in neighbors {
                if let Some(deg) = in_degree.get_mut(&next) {
                    *deg = deg.saturating_sub(1);
                    if *deg == 0 {
                        queue.push_back(next);
                    }
                }
            }
        }
    }

    visited_count < in_degree.len()
}

/// Get all transitive dependencies of an entity.
pub fn get_transitive_dependencies(
    graph: &WorldGraph,
    entity: EntityId,
    dependency_kind: EdgeKind,
) -> Vec<EntityId> {
    let mut reachable = find_reachability(graph, entity, Some(dependency_kind));
    reachable.remove(&entity); // Exclude the starting entity
    reachable.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EdgeKind, EdgeRecord, EntityKind, EntityRecord};
    use dfmcp_core::EdgeId;

    #[test]
    fn test_aba_entity_validator() -> Result<()> {
        let mut validator = AbaEntityValidator::new();
        let e1 = EntityId::new(1);

        assert!(validator.validate_reference(e1, 0).is_err());

        let gen1 = validator.retire_and_advance_generation(e1)?;
        assert_eq!(gen1, 1);
        assert!(validator.validate_reference(e1, 1).is_ok());
        assert!(validator.validate_reference(e1, 0).is_err());

        let gen2 = validator.retire_and_advance_generation(e1)?;
        assert_eq!(gen2, 2);
        assert!(validator.validate_reference(e1, 2).is_ok());
        assert!(validator.validate_reference(e1, 1).is_err());
        Ok(())
    }

    #[test]
    fn test_reachability_and_cycle_detection() {
        let mut graph = WorldGraph::default();

        let e1 = EntityId::new(1);
        let e2 = EntityId::new(2);
        let e3 = EntityId::new(3);

        graph.entities.insert(
            e1,
            EntityRecord {
                id: e1,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "E1".to_owned(),
                fields: BTreeMap::new(),
            },
        );
        graph.entities.insert(
            e2,
            EntityRecord {
                id: e2,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "E2".to_owned(),
                fields: BTreeMap::new(),
            },
        );
        graph.entities.insert(
            e3,
            EntityRecord {
                id: e3,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "E3".to_owned(),
                fields: BTreeMap::new(),
            },
        );

        // e1 -> e2 -> e3
        graph.edges.insert(
            EdgeId::new(1),
            EdgeRecord {
                id: EdgeId::new(1),
                revision: 1,
                kind: EdgeKind::ContainedIn,
                from: e1,
                to: e2,
                fields: BTreeMap::new(),
            },
        );
        graph.edges.insert(
            EdgeId::new(2),
            EdgeRecord {
                id: EdgeId::new(2),
                revision: 1,
                kind: EdgeKind::ContainedIn,
                from: e2,
                to: e3,
                fields: BTreeMap::new(),
            },
        );

        assert!(!detect_cycles(&graph, None));
        let reachable = find_reachability(&graph, e1, None);
        assert_eq!(reachable.len(), 3);
        assert!(reachable.contains(&e1));
        assert!(reachable.contains(&e2));
        assert!(reachable.contains(&e3));

        // Add e3 -> e1 to form cycle
        graph.edges.insert(
            EdgeId::new(3),
            EdgeRecord {
                id: EdgeId::new(3),
                revision: 1,
                kind: EdgeKind::ContainedIn,
                from: e3,
                to: e1,
                fields: BTreeMap::new(),
            },
        );

        assert!(detect_cycles(&graph, None));
    }
}
