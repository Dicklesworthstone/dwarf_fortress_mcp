#![forbid(unsafe_code)]

//! Semantic Rebase Conflict Resolution and Deterministic Certificate Generation.
//!
//! WP-WOR-04: Resolves optimistic concurrency conflicts when rebasing prepared plans
//! and change sets across world state epochs, emitting cryptographic certificates of conflict.

use dfmcp_core::{Digest32, PlanId, StateAnchor};

use crate::delta::WorldChange;
use crate::ledger::WitnessSet;
use crate::model::{WorldGraph, WorldSnapshot};

/// Classification of semantic rebase conflicts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    AnchorDivergence,
    PreconditionViolated { description: String },
    SpatialOverlap,
    EntityUnavailable,
    ResourceDepleted,
}

/// Cryptographic certificate proving why a rebase failed or was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConflictCertificate {
    pub plan_id: PlanId,
    pub base_anchor: StateAnchor,
    pub target_anchor: StateAnchor,
    pub conflict_kind: ConflictKind,
    pub diagnosis: String,
    pub certificate_digest: Digest32,
}

impl ConflictCertificate {
    #[must_use]
    pub fn new(
        plan_id: PlanId,
        base_anchor: StateAnchor,
        target_anchor: StateAnchor,
        conflict_kind: ConflictKind,
        diagnosis: String,
    ) -> Self {
        let mut bytes = Vec::new();
        crate::canonical::put_str(&mut bytes, "dfmcp-conflict-certificate-v1");
        bytes.extend_from_slice(&plan_id.get().to_be_bytes());
        crate::canonical::put_anchor(&mut bytes, base_anchor);
        crate::canonical::put_anchor(&mut bytes, target_anchor);
        encode_conflict_kind(&mut bytes, &conflict_kind);
        crate::canonical::put_str(&mut bytes, &diagnosis);
        let certificate_digest = Digest32::of_bytes(&bytes);

        Self {
            plan_id,
            base_anchor,
            target_anchor,
            conflict_kind,
            diagnosis,
            certificate_digest,
        }
    }
}

fn encode_conflict_kind(output: &mut Vec<u8>, kind: &ConflictKind) {
    match kind {
        ConflictKind::AnchorDivergence => output.push(0),
        ConflictKind::PreconditionViolated { description } => {
            output.push(1);
            crate::canonical::put_str(output, description);
        }
        ConflictKind::SpatialOverlap => output.push(2),
        ConflictKind::EntityUnavailable => output.push(3),
        ConflictKind::ResourceDepleted => output.push(4),
    }
}

/// Outcome of an attempted semantic rebase.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseOutcome {
    /// Rebase succeeded cleanly with re-anchored state and changes.
    Clean {
        rebased_anchor: StateAnchor,
        rebased_changes: Vec<WorldChange>,
    },
    /// Rebase failed due to state conflict.
    Conflicted(ConflictCertificate),
}

/// Deterministic semantic rebase engine for optimistic concurrency.
#[derive(Clone, Debug, Default)]
pub struct SemanticRebaseEngine;

impl SemanticRebaseEngine {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Attempt to rebase a set of world changes from a base snapshot onto a target snapshot.
    pub fn rebase_changes(
        &self,
        base_snapshot: &WorldSnapshot,
        target_snapshot: &WorldSnapshot,
        changes: &[WorldChange],
        plan_id: PlanId,
    ) -> RebaseOutcome {
        let base_anchor = base_snapshot.anchor();
        let target_anchor = target_snapshot.anchor();

        if base_anchor.fortress_id != target_anchor.fortress_id {
            return RebaseOutcome::Conflicted(ConflictCertificate::new(
                plan_id,
                base_anchor,
                target_anchor,
                ConflictKind::AnchorDivergence,
                format!(
                    "fortress id mismatch: base {} vs target {}",
                    base_anchor.fortress_id.get(),
                    target_anchor.fortress_id.get()
                ),
            ));
        }

        // Fast path: identical anchors require no structural rebase
        if base_anchor == target_anchor {
            return RebaseOutcome::Clean {
                rebased_anchor: target_anchor,
                rebased_changes: changes.to_vec(),
            };
        }

        let mut rebased = Vec::new();

        for change in changes {
            match change {
                WorldChange::UpsertEntity(record) => {
                    // Check if target entity exists and has advanced generation (ABA detection)
                    if let Some(target_record) = target_snapshot.graph.entities.get(&record.id)
                        && target_record.generation > record.generation
                    {
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::EntityUnavailable,
                            format!(
                                "ABA generation mismatch for entity {}: record gen {} < target gen {}",
                                record.id, record.generation, target_record.generation
                            ),
                        ));
                    }
                    let mut rebased_rec = record.clone();
                    // Rebase revision if target has a higher revision within same generation
                    if let Some(target_record) = target_snapshot.graph.entities.get(&record.id)
                        && target_record.generation == record.generation
                        && target_record.revision >= record.revision
                    {
                        rebased_rec.revision = target_record.revision.saturating_add(1);
                    }
                    rebased.push(WorldChange::UpsertEntity(rebased_rec));
                }
                WorldChange::RemoveEntity {
                    id,
                    expected_generation,
                    expected_revision,
                } => {
                    if let Some(target_record) = target_snapshot.graph.entities.get(id) {
                        if target_record.generation != *expected_generation {
                            return RebaseOutcome::Conflicted(ConflictCertificate::new(
                                plan_id,
                                base_anchor,
                                target_anchor,
                                ConflictKind::EntityUnavailable,
                                format!(
                                    "cannot remove entity {id}: expected generation {expected_generation} but found {}",
                                    target_record.generation
                                ),
                            ));
                        }
                    } else {
                        // Already absent in target - idempotent skip or conflict
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::EntityUnavailable,
                            format!("entity {id} was already removed in target snapshot"),
                        ));
                    }
                    rebased.push(WorldChange::RemoveEntity {
                        id: *id,
                        expected_generation: *expected_generation,
                        expected_revision: *expected_revision,
                    });
                }
                WorldChange::UpsertEdge(edge) => {
                    // Validate edge endpoints exist in target snapshot
                    if !target_snapshot.graph.entities.contains_key(&edge.from) {
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::PreconditionViolated {
                                description: format!("edge source entity {} missing", edge.from),
                            },
                            format!(
                                "edge source entity {} not found in target snapshot",
                                edge.from
                            ),
                        ));
                    }
                    if !target_snapshot.graph.entities.contains_key(&edge.to) {
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::PreconditionViolated {
                                description: format!("edge target entity {} missing", edge.to),
                            },
                            format!(
                                "edge target entity {} not found in target snapshot",
                                edge.to
                            ),
                        ));
                    }
                    let mut rebased_edge = edge.clone();
                    if let Some(target_edge) = target_snapshot.graph.edges.get(&edge.id)
                        && target_edge.revision >= edge.revision
                    {
                        rebased_edge.revision = target_edge.revision.saturating_add(1);
                    }
                    rebased.push(WorldChange::UpsertEdge(rebased_edge));
                }
                WorldChange::RemoveEdge {
                    id,
                    expected_revision,
                } => {
                    if !target_snapshot.graph.edges.contains_key(id) {
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::PreconditionViolated {
                                description: format!("edge {:?} missing", id),
                            },
                            format!("edge {:?} not found in target snapshot to remove", id),
                        ));
                    }
                    rebased.push(WorldChange::RemoveEdge {
                        id: *id,
                        expected_revision: *expected_revision,
                    });
                }
                WorldChange::UpsertMapChunk(chunk) => {
                    rebased.push(WorldChange::UpsertMapChunk(chunk.clone()));
                }
                WorldChange::RemoveMapChunk {
                    coord,
                    expected_revision,
                } => {
                    rebased.push(WorldChange::RemoveMapChunk {
                        coord: *coord,
                        expected_revision: *expected_revision,
                    });
                }
                WorldChange::AppendEvent(event) => {
                    rebased.push(WorldChange::AppendEvent(event.clone()));
                }
            }
        }

        RebaseOutcome::Clean {
            rebased_anchor: target_anchor,
            rebased_changes: rebased,
        }
    }

    /// Rebase changes with an associated witness set validation.
    pub fn rebase_with_witness(
        &self,
        base_snapshot: &WorldSnapshot,
        target_snapshot: &WorldSnapshot,
        witness: &WitnessSet,
        changes: &[WorldChange],
        plan_id: PlanId,
    ) -> RebaseOutcome {
        let base_anchor = base_snapshot.anchor();
        let target_anchor = target_snapshot.anchor();

        // 1. Verify positive entity witnesses against target snapshot
        for (w_id, w_gen, w_rev) in &witness.positive_entities {
            match target_snapshot.graph.entities.get(w_id) {
                None => {
                    return RebaseOutcome::Conflicted(ConflictCertificate::new(
                        plan_id,
                        base_anchor,
                        target_anchor,
                        ConflictKind::EntityUnavailable,
                        format!("witnessed positive entity {w_id} was removed in target snapshot"),
                    ));
                }
                Some(rec) => {
                    if rec.generation != *w_gen {
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::EntityUnavailable,
                            format!(
                                "witnessed positive entity {w_id} ABA generation mismatch: witnessed {w_gen} vs target {}",
                                rec.generation
                            ),
                        ));
                    }
                    if rec.revision != *w_rev {
                        return RebaseOutcome::Conflicted(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            target_anchor,
                            ConflictKind::PreconditionViolated {
                                description: format!("entity {w_id} revision divergence"),
                            },
                            format!(
                                "witnessed positive entity {w_id} revision changed: witnessed {w_rev} vs target {}",
                                rec.revision
                            ),
                        ));
                    }
                }
            }
        }

        // 2. Verify negative entity witnesses (phantom protection)
        for neg_id in &witness.negative_entities {
            if target_snapshot.graph.entities.contains_key(neg_id) {
                return RebaseOutcome::Conflicted(ConflictCertificate::new(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::PreconditionViolated {
                        description: format!("phantom entity {neg_id} appeared"),
                    },
                    format!("witnessed absent entity {neg_id} was created in target snapshot"),
                ));
            }
        }

        // 3. Rebase change set
        self.rebase_changes(base_snapshot, target_snapshot, changes, plan_id)
    }

    /// Perform a 3-way merge between base, ours, and theirs.
    #[allow(clippy::result_large_err)]
    pub fn three_way_merge(
        &self,
        base: &WorldSnapshot,
        ours: &WorldSnapshot,
        theirs: &WorldSnapshot,
        plan_id: PlanId,
    ) -> std::result::Result<WorldSnapshot, ConflictCertificate> {
        let base_anchor = base.anchor();
        let theirs_anchor = theirs.anchor();

        if base.fortress_id != ours.fortress_id || base.fortress_id != theirs.fortress_id {
            return Err(ConflictCertificate::new(
                plan_id,
                base_anchor,
                theirs_anchor,
                ConflictKind::AnchorDivergence,
                "three-way merge snapshots belong to differing fortresses".to_owned(),
            ));
        }

        let mut merged_entities = theirs.graph.entities.clone();

        // Merge our entity additions / updates
        for (id, our_entity) in &ours.graph.entities {
            match base.graph.entities.get(id) {
                None => {
                    // Added by us: check if theirs also added it
                    if let Some(their_entity) = theirs.graph.entities.get(id) {
                        if their_entity != our_entity {
                            return Err(ConflictCertificate::new(
                                plan_id,
                                base_anchor,
                                theirs_anchor,
                                ConflictKind::PreconditionViolated {
                                    description: format!(
                                        "divergent concurrent insertion of entity {id}"
                                    ),
                                },
                                format!("entity {id} inserted concurrently with different content"),
                            ));
                        }
                    } else {
                        merged_entities.insert(*id, our_entity.clone());
                    }
                }
                Some(base_entity) => {
                    if our_entity != base_entity {
                        // Modified by us: check if theirs also modified it
                        if let Some(their_entity) = theirs.graph.entities.get(id) {
                            if their_entity != base_entity && their_entity != our_entity {
                                return Err(ConflictCertificate::new(
                                    plan_id,
                                    base_anchor,
                                    theirs_anchor,
                                    ConflictKind::PreconditionViolated {
                                        description: format!(
                                            "concurrent modification of entity {id}"
                                        ),
                                    },
                                    format!("entity {id} modified concurrently on both branches"),
                                ));
                            }
                        } else {
                            // Deleted by theirs, modified by ours -> conflict
                            return Err(ConflictCertificate::new(
                                plan_id,
                                base_anchor,
                                theirs_anchor,
                                ConflictKind::EntityUnavailable,
                                format!("entity {id} modified by ours but deleted by theirs"),
                            ));
                        }
                        merged_entities.insert(*id, our_entity.clone());
                    }
                }
            }
        }

        // Merge our entity deletions
        for (id, base_entity) in &base.graph.entities {
            if !ours.graph.entities.contains_key(id) {
                // Deleted by us
                if let Some(their_entity) = theirs.graph.entities.get(id) {
                    if their_entity != base_entity {
                        return Err(ConflictCertificate::new(
                            plan_id,
                            base_anchor,
                            theirs_anchor,
                            ConflictKind::EntityUnavailable,
                            format!("entity {id} deleted by ours but modified by theirs"),
                        ));
                    }
                    merged_entities.remove(id);
                }
            }
        }

        let mut merged_edges = theirs.graph.edges.clone();
        for (id, our_edge) in &ours.graph.edges {
            if !theirs.graph.edges.contains_key(id) && !base.graph.edges.contains_key(id) {
                merged_edges.insert(*id, our_edge.clone());
            }
        }

        let mut merged_chunks = theirs.graph.chunks.clone();
        for (coord, our_chunk) in &ours.graph.chunks {
            if !theirs.graph.chunks.contains_key(coord) && !base.graph.chunks.contains_key(coord) {
                merged_chunks.insert(*coord, our_chunk.clone());
            }
        }

        let mut merged_events = theirs.graph.events.clone();
        for (id, our_event) in &ours.graph.events {
            if !theirs.graph.events.contains_key(id) && !base.graph.events.contains_key(id) {
                merged_events.insert(*id, our_event.clone());
            }
        }

        let merged_graph = WorldGraph {
            entities: merged_entities,
            edges: merged_edges,
            chunks: merged_chunks,
            events: merged_events,
        };

        let target_tick = ours.tick.max(theirs.tick);
        let target_cursor = dfmcp_core::ObservationCursor {
            epoch: theirs.cursor.epoch,
            sequence: theirs.cursor.sequence.saturating_add(1),
        };

        Ok(WorldSnapshot::new(
            base.fortress_id,
            target_tick,
            target_cursor,
            theirs.paused,
            merged_graph,
        ))
    }
}
