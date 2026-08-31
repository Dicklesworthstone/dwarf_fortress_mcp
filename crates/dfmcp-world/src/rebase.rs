#![forbid(unsafe_code)]

//! Conservative, deterministic structural rebase laboratory.
//!
//! This module merges only stable-key records whose base/ours/theirs relationship is
//! unambiguous. It never increments a revision or combines record fields speculatively.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{Digest32, PlanId, StateAnchor};

use crate::delta::{WorldChange, build_delta, diff_snapshots};
use crate::ledger::WitnessSet;
use crate::model::{WorldGraph, WorldSnapshot};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictKind {
    AnchorDivergence,
    PreconditionViolated { description: String },
    SpatialOverlap,
    EntityUnavailable,
    ResourceDepleted,
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RebaseOutcome {
    Clean {
        rebased_anchor: StateAnchor,
        rebased_changes: Vec<WorldChange>,
    },
    Conflicted(ConflictCertificate),
}

#[derive(Clone, Debug, Default)]
pub struct SemanticRebaseEngine;

impl SemanticRebaseEngine {
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    pub fn rebase_changes(
        &self,
        base: &WorldSnapshot,
        target: &WorldSnapshot,
        changes: &[WorldChange],
        plan_id: PlanId,
    ) -> RebaseOutcome {
        let base_anchor = base.anchor();
        let target_anchor = target.anchor();
        if !base.hash_is_valid()
            || !target.hash_is_valid()
            || base.fortress_id != target.fortress_id
            || base.cursor.epoch != target.cursor.epoch
            || target.cursor.sequence < base.cursor.sequence
            || target.tick < base.tick
            || !branch_is_valid(base, target)
        {
            return conflicted(
                plan_id,
                base_anchor,
                target_anchor,
                ConflictKind::AnchorDivergence,
                "rebase snapshots have invalid hashes, fortresses, or epochs",
            );
        }
        let Some(validation_cursor) = base.cursor.checked_next() else {
            return conflicted(
                plan_id,
                base_anchor,
                target_anchor,
                ConflictKind::AnchorDivergence,
                "base observation cursor is exhausted",
            );
        };
        if build_delta(base, validation_cursor, base.tick, changes.to_vec()).is_err() {
            return conflicted(
                plan_id,
                base_anchor,
                target_anchor,
                ConflictKind::PreconditionViolated {
                    description: "change set is invalid against its declared base".to_owned(),
                },
                "change set could not be applied to the base snapshot",
            );
        }

        let mut rebased = Vec::new();
        for change in changes {
            let decision = match change {
                WorldChange::UpsertEntity(incoming) => stable_upsert(
                    base.graph.entities.get(&incoming.id),
                    target.graph.entities.get(&incoming.id),
                    incoming,
                ),
                WorldChange::RemoveEntity {
                    id,
                    expected_generation,
                    expected_revision,
                } => stable_remove(
                    base.graph.entities.get(id),
                    target.graph.entities.get(id),
                    |record| {
                        record.generation == *expected_generation
                            && record.revision == *expected_revision
                    },
                ),
                WorldChange::UpsertEdge(incoming) => stable_upsert(
                    base.graph.edges.get(&incoming.id),
                    target.graph.edges.get(&incoming.id),
                    incoming,
                ),
                WorldChange::RemoveEdge {
                    id,
                    expected_revision,
                } => stable_remove(
                    base.graph.edges.get(id),
                    target.graph.edges.get(id),
                    |record| record.revision == *expected_revision,
                ),
                WorldChange::UpsertMapChunk(incoming) => stable_upsert(
                    base.graph.chunks.get(&incoming.coord),
                    target.graph.chunks.get(&incoming.coord),
                    incoming,
                ),
                WorldChange::RemoveMapChunk {
                    coord,
                    expected_revision,
                } => stable_remove(
                    base.graph.chunks.get(coord),
                    target.graph.chunks.get(coord),
                    |record| record.revision == *expected_revision,
                ),
                WorldChange::AppendEvent(incoming) => stable_upsert(
                    base.graph.events.get(&incoming.id),
                    target.graph.events.get(&incoming.id),
                    incoming,
                ),
            };
            match decision {
                StableDecision::Apply => rebased.push(change.clone()),
                StableDecision::AlreadyApplied => {}
                StableDecision::Conflict => {
                    let (kind, diagnosis) = match change {
                        WorldChange::UpsertEntity(incoming)
                            if target
                                .graph
                                .entities
                                .get(&incoming.id)
                                .is_some_and(|record| record.generation > incoming.generation) =>
                        {
                            (
                                ConflictKind::EntityUnavailable,
                                format!("ABA generation mismatch for entity {}", incoming.id),
                            )
                        }
                        _ => (
                            ConflictKind::PreconditionViolated {
                                description: "stable-key record changed concurrently".to_owned(),
                            },
                            "stable-key record changed concurrently; semantic replay is required"
                                .to_owned(),
                        ),
                    };
                    return conflicted(plan_id, base_anchor, target_anchor, kind, diagnosis);
                }
            }
        }
        if !rebased.is_empty() {
            let Some(rebased_cursor) = target.cursor.checked_next() else {
                return conflicted(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::AnchorDivergence,
                    "target observation cursor is exhausted",
                );
            };
            if build_delta(target, rebased_cursor, target.tick, rebased.clone()).is_err() {
                return conflicted(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::PreconditionViolated {
                        description: "rebased writes violate target invariants".to_owned(),
                    },
                    "rebased changes cannot be applied safely to the target snapshot",
                );
            }
        }
        RebaseOutcome::Clean {
            rebased_anchor: target_anchor,
            rebased_changes: rebased,
        }
    }

    pub fn rebase_with_witness(
        &self,
        base: &WorldSnapshot,
        target: &WorldSnapshot,
        witness: &WitnessSet,
        changes: &[WorldChange],
        plan_id: PlanId,
    ) -> RebaseOutcome {
        let base_anchor = base.anchor();
        let target_anchor = target.anchor();
        for (id, generation, revision) in &witness.positive_entities {
            let matches_witness = |snapshot: &WorldSnapshot| {
                snapshot.graph.entities.get(id).is_some_and(|record| {
                    record.generation == *generation && record.revision == *revision
                })
            };
            if !matches_witness(base) || !matches_witness(target) {
                return conflicted(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::EntityUnavailable,
                    format!("witnessed positive entity {id} changed or disappeared"),
                );
            }
        }
        for id in &witness.negative_entities {
            if base.graph.entities.contains_key(id) || target.graph.entities.contains_key(id) {
                return conflicted(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::PreconditionViolated {
                        description: format!("phantom entity {id} appeared"),
                    },
                    format!("witnessed absent entity {id} was created in target snapshot"),
                );
            }
        }
        for (coord, revision) in &witness.witnessed_chunks {
            let matches_witness = |snapshot: &WorldSnapshot| {
                snapshot
                    .graph
                    .chunks
                    .get(coord)
                    .is_some_and(|chunk| chunk.revision == *revision)
            };
            if !matches_witness(base) || !matches_witness(target) {
                return conflicted(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::SpatialOverlap,
                    format!("witnessed map chunk {coord:?} changed or disappeared"),
                );
            }
        }
        self.rebase_changes(base, target, changes, plan_id)
    }

    /// Record-granularity three-way merge with deterministic stable-key ordering.
    #[allow(clippy::result_large_err)]
    pub fn three_way_merge(
        &self,
        base: &WorldSnapshot,
        ours: &WorldSnapshot,
        theirs: &WorldSnapshot,
        plan_id: PlanId,
    ) -> std::result::Result<WorldSnapshot, ConflictCertificate> {
        let base_anchor = base.anchor();
        let target_anchor = theirs.anchor();
        if !base.hash_is_valid()
            || !ours.hash_is_valid()
            || !theirs.hash_is_valid()
            || base.fortress_id != ours.fortress_id
            || base.fortress_id != theirs.fortress_id
            || base.cursor.epoch != ours.cursor.epoch
            || base.cursor.epoch != theirs.cursor.epoch
            || ours.cursor.sequence < base.cursor.sequence
            || theirs.cursor.sequence < base.cursor.sequence
            || ours.tick < base.tick
            || theirs.tick < base.tick
            || !branch_is_valid(base, ours)
            || !branch_is_valid(base, theirs)
        {
            return Err(ConflictCertificate::new(
                plan_id,
                base_anchor,
                target_anchor,
                ConflictKind::AnchorDivergence,
                "three-way merge snapshots have invalid hashes, fortresses, or epochs".to_owned(),
            ));
        }

        let entities = merge_map(
            &base.graph.entities,
            &ours.graph.entities,
            &theirs.graph.entities,
        )
        .map_err(|key| merge_conflict(plan_id, base_anchor, target_anchor, "entity", &key))?;
        let edges = merge_map(&base.graph.edges, &ours.graph.edges, &theirs.graph.edges)
            .map_err(|key| merge_conflict(plan_id, base_anchor, target_anchor, "edge", &key))?;
        let chunks = merge_map(&base.graph.chunks, &ours.graph.chunks, &theirs.graph.chunks)
            .map_err(|key| merge_conflict(plan_id, base_anchor, target_anchor, "chunk", &key))?;
        let events = merge_map(&base.graph.events, &ours.graph.events, &theirs.graph.events)
            .map_err(|key| merge_conflict(plan_id, base_anchor, target_anchor, "event", &key))?;
        let paused = merge_scalar(base.paused, ours.paused, theirs.paused).ok_or_else(|| {
            merge_conflict(
                plan_id,
                base_anchor,
                target_anchor,
                "pause state",
                &"paused",
            )
        })?;
        let sequence = ours
            .cursor
            .sequence
            .max(theirs.cursor.sequence)
            .checked_add(1)
            .ok_or_else(|| {
                ConflictCertificate::new(
                    plan_id,
                    base_anchor,
                    target_anchor,
                    ConflictKind::AnchorDivergence,
                    "merged observation cursor would overflow".to_owned(),
                )
            })?;
        let merged = WorldSnapshot::new(
            base.fortress_id,
            ours.tick.max(theirs.tick),
            dfmcp_core::ObservationCursor {
                epoch: base.cursor.epoch,
                sequence,
            },
            paused,
            WorldGraph {
                entities,
                edges,
                chunks,
                events,
            },
        );
        if diff_snapshots(base, &merged).is_err() {
            return Err(ConflictCertificate::new(
                plan_id,
                base_anchor,
                target_anchor,
                ConflictKind::PreconditionViolated {
                    description: "merged graph violates canonical state invariants".to_owned(),
                },
                "independent record merges do not form a valid world snapshot".to_owned(),
            ));
        }
        Ok(merged)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StableDecision {
    Apply,
    AlreadyApplied,
    Conflict,
}

fn stable_upsert<T: PartialEq>(
    base: Option<&T>,
    target: Option<&T>,
    incoming: &T,
) -> StableDecision {
    if target == Some(incoming) {
        StableDecision::AlreadyApplied
    } else if target == base {
        StableDecision::Apply
    } else {
        StableDecision::Conflict
    }
}

fn stable_remove<T: PartialEq>(
    base: Option<&T>,
    target: Option<&T>,
    expected: impl FnOnce(&T) -> bool,
) -> StableDecision {
    let Some(base_record) = base else {
        return StableDecision::Conflict;
    };
    if !expected(base_record) {
        return StableDecision::Conflict;
    }
    match target {
        None => StableDecision::AlreadyApplied,
        Some(target_record) if target_record == base_record => StableDecision::Apply,
        Some(_) => StableDecision::Conflict,
    }
}

fn merge_map<K, V>(
    base: &BTreeMap<K, V>,
    ours: &BTreeMap<K, V>,
    theirs: &BTreeMap<K, V>,
) -> std::result::Result<BTreeMap<K, V>, K>
where
    K: Clone + Ord,
    V: Clone + PartialEq,
{
    let keys: BTreeSet<K> = base
        .keys()
        .chain(ours.keys())
        .chain(theirs.keys())
        .cloned()
        .collect();
    let mut merged = BTreeMap::new();
    for key in keys {
        let choice = if ours.get(&key) == base.get(&key) {
            theirs.get(&key)
        } else if theirs.get(&key) == base.get(&key) || ours.get(&key) == theirs.get(&key) {
            ours.get(&key)
        } else {
            return Err(key);
        };
        if let Some(value) = choice {
            merged.insert(key, value.clone());
        }
    }
    Ok(merged)
}

fn merge_scalar<T: Copy + PartialEq>(base: T, ours: T, theirs: T) -> Option<T> {
    if ours == base {
        Some(theirs)
    } else if theirs == base || ours == theirs {
        Some(ours)
    } else {
        None
    }
}

fn branch_is_valid(base: &WorldSnapshot, branch: &WorldSnapshot) -> bool {
    if base.anchor() == branch.anchor() {
        base == branch
    } else {
        diff_snapshots(base, branch).is_ok()
    }
}

fn merge_conflict(
    plan_id: PlanId,
    base_anchor: StateAnchor,
    target_anchor: StateAnchor,
    record_kind: &str,
    key: &impl std::fmt::Debug,
) -> ConflictCertificate {
    ConflictCertificate::new(
        plan_id,
        base_anchor,
        target_anchor,
        ConflictKind::PreconditionViolated {
            description: format!("concurrent {record_kind} edit"),
        },
        format!("concurrent {record_kind} edit at stable key {key:?}"),
    )
}

fn conflicted(
    plan_id: PlanId,
    base_anchor: StateAnchor,
    target_anchor: StateAnchor,
    kind: ConflictKind,
    diagnosis: impl Into<String>,
) -> RebaseOutcome {
    RebaseOutcome::Conflicted(ConflictCertificate::new(
        plan_id,
        base_anchor,
        target_anchor,
        kind,
        diagnosis.into(),
    ))
}
