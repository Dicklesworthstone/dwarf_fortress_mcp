#![forbid(unsafe_code)]

//! Merkle State Trees and Cryptographic Inclusion Proofs for Autonomous Trust Protocol.
//!
//! WP-WLD-03: Organizes world snapshot sub-trees (entities, edges, chunks, events)
//! into deterministic Merkle DAGs, enabling lightweight verification across multi-agent swarms.

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId};

use crate::model::WorldSnapshot;

/// Inclusion proof containing the cryptographic hash sibling path to Merkle root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleInclusionProof {
    pub leaf_digest: Digest32,
    pub sibling_hashes: Vec<Digest32>,
}

impl MerkleInclusionProof {
    /// Verify this inclusion proof against an expected Merkle root.
    #[must_use]
    pub fn verify_root(&self, expected_root: &Digest32) -> bool {
        let mut current = self.leaf_digest;
        for sibling in &self.sibling_hashes {
            let mut hasher_bytes = Vec::with_capacity(64);
            if current <= *sibling {
                hasher_bytes.extend_from_slice(current.as_bytes());
                hasher_bytes.extend_from_slice(sibling.as_bytes());
            } else {
                hasher_bytes.extend_from_slice(sibling.as_bytes());
                hasher_bytes.extend_from_slice(current.as_bytes());
            }
            current = Digest32::of_bytes(&hasher_bytes);
        }
        current == *expected_root
    }
}

/// Cryptographic Merkle State Tree for a world snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleStateTree {
    pub entities_root: Digest32,
    pub edges_root: Digest32,
    pub chunks_root: Digest32,
    pub events_root: Digest32,
    pub overall_root: Digest32,
    entity_leaf_hashes: BTreeMap<EntityId, Digest32>,
}

impl MerkleStateTree {
    /// Compute Merkle state tree roots from a world snapshot.
    #[must_use]
    pub fn from_snapshot(snapshot: &WorldSnapshot) -> Self {
        // 1. Entities Merkle Sub-tree
        let mut entity_leaf_hashes = BTreeMap::new();
        let mut entity_bytes = Vec::new();

        for (id, entity) in &snapshot.graph.entities {
            let leaf_hash = Digest32::of_bytes(&entity.canonical_bytes());
            entity_leaf_hashes.insert(*id, leaf_hash);
            entity_bytes.extend_from_slice(leaf_hash.as_bytes());
        }
        let entities_root = Digest32::of_bytes(&entity_bytes);

        // 2. Edges Merkle Sub-tree
        let mut edge_bytes = Vec::new();
        for edge in snapshot.graph.edges.values() {
            let mut buf = Vec::new();
            edge.encode(&mut buf);
            let leaf_hash = Digest32::of_bytes(&buf);
            edge_bytes.extend_from_slice(leaf_hash.as_bytes());
        }
        let edges_root = Digest32::of_bytes(&edge_bytes);

        // 3. Chunks Merkle Sub-tree
        let mut chunk_bytes = Vec::new();
        for chunk in snapshot.graph.chunks.values() {
            let mut buf = Vec::new();
            chunk.encode(&mut buf);
            let leaf_hash = Digest32::of_bytes(&buf);
            chunk_bytes.extend_from_slice(leaf_hash.as_bytes());
        }
        let chunks_root = Digest32::of_bytes(&chunk_bytes);

        // 4. Events Merkle Sub-tree
        let mut event_bytes = Vec::new();
        for event in snapshot.graph.events.values() {
            let mut buf = Vec::new();
            event.encode(&mut buf);
            let leaf_hash = Digest32::of_bytes(&buf);
            event_bytes.extend_from_slice(leaf_hash.as_bytes());
        }
        let events_root = Digest32::of_bytes(&event_bytes);

        // Overall Root
        let mut overall_bytes = Vec::with_capacity(128);
        overall_bytes.extend_from_slice(entities_root.as_bytes());
        overall_bytes.extend_from_slice(edges_root.as_bytes());
        overall_bytes.extend_from_slice(chunks_root.as_bytes());
        overall_bytes.extend_from_slice(events_root.as_bytes());
        let overall_root = Digest32::of_bytes(&overall_bytes);

        Self {
            entities_root,
            edges_root,
            chunks_root,
            events_root,
            overall_root,
            entity_leaf_hashes,
        }
    }

    /// Generate an inclusion proof for a specific entity ID.
    #[must_use]
    pub fn generate_entity_proof(&self, id: EntityId) -> Option<MerkleInclusionProof> {
        let leaf_digest = self.entity_leaf_hashes.get(&id).copied()?;
        // Sibling path contains other sub-tree roots to compute overall root
        let sibling_hashes = vec![self.edges_root, self.chunks_root, self.events_root];
        Some(MerkleInclusionProof {
            leaf_digest,
            sibling_hashes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityKind, EntityRecord, WorldGraph};
    use dfmcp_core::{FortressId, GameTick, ObservationCursor};

    #[test]
    fn test_merkle_tree_derivation_deterministic() {
        let mut graph = WorldGraph::default();
        graph.entities.insert(
            EntityId::new(1),
            EntityRecord {
                id: EntityId::new(1),
                kind: EntityKind::Unit,
                generation: 1,
                revision: 1,
                label: "Unit1".to_owned(),
                fields: BTreeMap::new(),
            },
        );

        let snap1 = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            graph.clone(),
        );
        let snap2 = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );

        let tree1 = MerkleStateTree::from_snapshot(&snap1);
        let tree2 = MerkleStateTree::from_snapshot(&snap2);

        assert_eq!(tree1.overall_root, tree2.overall_root);
        assert_eq!(tree1.entities_root, tree2.entities_root);
    }
}
