#![forbid(unsafe_code)]

//! Deterministic Merkle state trees and entity inclusion proofs.

use std::collections::BTreeMap;

use dfmcp_core::{Digest32, EntityId};

use crate::model::WorldSnapshot;

const EMPTY_DOMAIN: &[u8] = b"dfmcp-merkle-empty-v1";
const LEAF_DOMAIN: &[u8] = b"dfmcp-merkle-leaf-v1";
const PAIR_DOMAIN: &[u8] = b"dfmcp-merkle-pair-v1";

/// Inclusion proof containing the ordered sibling path from a leaf to the
/// overall state root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MerkleInclusionProof {
    pub leaf_digest: Digest32,
    pub sibling_hashes: Vec<Digest32>,
    /// `true` when the corresponding sibling is on the left of the current
    /// node. The vector must have exactly the same length as `sibling_hashes`.
    pub sibling_is_left: Vec<bool>,
}

impl MerkleInclusionProof {
    /// Verify this inclusion proof against an expected Merkle root.
    #[must_use]
    pub fn verify_root(&self, expected_root: &Digest32) -> bool {
        if self.sibling_hashes.len() != self.sibling_is_left.len() {
            return false;
        }
        let computed = self.sibling_hashes.iter().zip(&self.sibling_is_left).fold(
            self.leaf_digest,
            |current, (sibling, sibling_is_left)| {
                if *sibling_is_left {
                    hash_pair(*sibling, current)
                } else {
                    hash_pair(current, *sibling)
                }
            },
        );
        computed == *expected_root
    }
}

/// Cryptographic Merkle tree derived from one world snapshot.
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
    /// Compute all subtree roots from canonical record encodings.
    #[must_use]
    pub fn from_snapshot(snapshot: &WorldSnapshot) -> Self {
        let entity_leaf_hashes: BTreeMap<EntityId, Digest32> = snapshot
            .graph
            .entities
            .iter()
            .map(|(id, entity)| (*id, hash_leaf(&entity.canonical_bytes())))
            .collect();
        let entities_root = merkle_root(entity_leaf_hashes.values().copied().collect());

        let edge_leaves = snapshot
            .graph
            .edges
            .values()
            .map(|edge| {
                let mut bytes = Vec::new();
                edge.encode(&mut bytes);
                hash_leaf(&bytes)
            })
            .collect();
        let edges_root = merkle_root(edge_leaves);

        let chunk_leaves = snapshot
            .graph
            .chunks
            .values()
            .map(|chunk| {
                let mut bytes = Vec::new();
                chunk.encode(&mut bytes);
                hash_leaf(&bytes)
            })
            .collect();
        let chunks_root = merkle_root(chunk_leaves);

        let event_leaves = snapshot
            .graph
            .events
            .values()
            .map(|event| {
                let mut bytes = Vec::new();
                event.encode(&mut bytes);
                hash_leaf(&bytes)
            })
            .collect();
        let events_root = merkle_root(event_leaves);

        let overall_root = merkle_root(vec![entities_root, edges_root, chunks_root, events_root]);

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
        let entity_ids: Vec<EntityId> = self.entity_leaf_hashes.keys().copied().collect();
        let index = entity_ids.binary_search(&id).ok()?;
        let leaves: Vec<Digest32> = self.entity_leaf_hashes.values().copied().collect();
        let leaf_digest = *leaves.get(index)?;
        let mut siblings = merkle_path(leaves, index)?;

        // Overall tree layout is [(entities, edges), (chunks, events)].
        siblings.push((self.edges_root, false));
        siblings.push((hash_pair(self.chunks_root, self.events_root), false));
        let (sibling_hashes, sibling_is_left): (Vec<_>, Vec<_>) = siblings.into_iter().unzip();

        Some(MerkleInclusionProof {
            leaf_digest,
            sibling_hashes,
            sibling_is_left,
        })
    }
}

fn hash_leaf(bytes: &[u8]) -> Digest32 {
    let mut encoded = Vec::with_capacity(LEAF_DOMAIN.len() + 8 + bytes.len());
    encoded.extend_from_slice(LEAF_DOMAIN);
    encoded.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    encoded.extend_from_slice(bytes);
    Digest32::of_bytes(&encoded)
}

fn hash_pair(left: Digest32, right: Digest32) -> Digest32 {
    let mut encoded = Vec::with_capacity(PAIR_DOMAIN.len() + 64);
    encoded.extend_from_slice(PAIR_DOMAIN);
    encoded.extend_from_slice(left.as_bytes());
    encoded.extend_from_slice(right.as_bytes());
    Digest32::of_bytes(&encoded)
}

fn merkle_root(mut level: Vec<Digest32>) -> Digest32 {
    if level.is_empty() {
        return Digest32::of_bytes(EMPTY_DOMAIN);
    }
    while level.len() > 1 {
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            match pair {
                [left, right] => next.push(hash_pair(*left, *right)),
                [only] => next.push(*only),
                _ => {}
            }
        }
        level = next;
    }
    match level.into_iter().next() {
        Some(root) => root,
        None => Digest32::of_bytes(EMPTY_DOMAIN),
    }
}

fn merkle_path(mut level: Vec<Digest32>, mut index: usize) -> Option<Vec<(Digest32, bool)>> {
    if index >= level.len() {
        return None;
    }
    let mut siblings = Vec::new();
    while level.len() > 1 {
        if index.is_multiple_of(2) {
            if let Some(sibling) = level.get(index + 1) {
                siblings.push((*sibling, false));
            }
        } else {
            let sibling = level.get(index - 1)?;
            siblings.push((*sibling, true));
        }
        let mut next = Vec::with_capacity(level.len().div_ceil(2));
        for pair in level.chunks(2) {
            match pair {
                [left, right] => next.push(hash_pair(*left, *right)),
                [only] => next.push(*only),
                _ => {}
            }
        }
        index /= 2;
        level = next;
    }
    Some(siblings)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{EntityKind, EntityRecord, WorldGraph};
    use dfmcp_core::{FortressId, GameTick, ObservationCursor};

    fn entity(id: u64) -> EntityRecord {
        EntityRecord {
            id: EntityId::new(id),
            kind: EntityKind::Unit,
            generation: 1,
            revision: 1,
            label: format!("Unit{id}"),
            fields: BTreeMap::new(),
        }
    }

    #[test]
    fn merkle_tree_is_deterministic_and_proofs_verify() {
        let mut graph = WorldGraph::default();
        for id in 1..=5 {
            graph.entities.insert(EntityId::new(id), entity(id));
        }

        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            graph.clone(),
        );
        let replay = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );

        let tree = MerkleStateTree::from_snapshot(&snapshot);
        let replay_tree = MerkleStateTree::from_snapshot(&replay);
        assert_eq!(tree, replay_tree);

        for id in 1..=5 {
            let proof = tree.generate_entity_proof(EntityId::new(id));
            assert!(proof.is_some());
            assert!(proof.is_some_and(|proof| proof.verify_root(&tree.overall_root)));
        }
        assert!(tree.generate_entity_proof(EntityId::new(99)).is_none());
    }

    #[test]
    fn modified_proof_is_rejected() {
        let mut graph = WorldGraph::default();
        graph.entities.insert(EntityId::new(1), entity(1));
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );
        let tree = MerkleStateTree::from_snapshot(&snapshot);
        let mut proof = match tree.generate_entity_proof(EntityId::new(1)) {
            Some(proof) => proof,
            None => return,
        };
        proof.leaf_digest = Digest32::of_bytes(b"tampered");
        assert!(!proof.verify_root(&tree.overall_root));
    }

    #[test]
    fn pair_hashing_commits_to_order() {
        let left = Digest32::of_bytes(b"left");
        let right = Digest32::of_bytes(b"right");
        assert_ne!(hash_pair(left, right), hash_pair(right, left));
    }

    #[test]
    fn proof_direction_is_authenticated() {
        let mut graph = WorldGraph::default();
        graph.entities.insert(EntityId::new(1), entity(1));
        graph.entities.insert(EntityId::new(2), entity(2));
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );
        let tree = MerkleStateTree::from_snapshot(&snapshot);
        let mut proof = match tree.generate_entity_proof(EntityId::new(1)) {
            Some(proof) => proof,
            None => return,
        };
        assert!(proof.verify_root(&tree.overall_root));
        if let Some(direction) = proof.sibling_is_left.first_mut() {
            *direction = !*direction;
        }
        assert!(!proof.verify_root(&tree.overall_root));
    }
}
