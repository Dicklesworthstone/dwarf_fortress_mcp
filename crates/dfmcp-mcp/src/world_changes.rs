//! What changed in the world between two observed laboratory anchors.
//!
//! The Agent Turn must answer "what changed" without transcript memory. The
//! laboratory keeps a bounded, immutable history of the canonical world
//! versions each session has seen; this module compares the agent's previous
//! anchor with the current one and reports entity creation, removal and field
//! changes (value before and after), terrain changes per level with their
//! bounding box and tile transitions, and clock and pause transitions. Every
//! change is `observed`: both sides are exact canonical versions. Output is
//! bounded and deterministic (entity id, then field name, then level order).

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_world::{ChunkCoord, EntityRecord, MapChunk, WorldSnapshot};
use serde_json::{Value, json};

use crate::lab_world::{tile_name, value_json};

/// Most entity changes listed in one turn.
pub(crate) const MAX_ENTITY_CHANGES: usize = 24;
/// Most changed fields listed per entity.
const MAX_FIELDS_PER_ENTITY: usize = 8;
const CHUNK_EDGE: i32 = dfmcp_world::terrain::TERRAIN_CHUNK_EDGE;

fn anchor_ref(snapshot: &WorldSnapshot) -> Value {
    json!({
        "epoch": snapshot.cursor.epoch,
        "sequence": snapshot.cursor.sequence,
        "game_tick": snapshot.tick.0,
        "state_hash": snapshot.state_hash.to_hex(),
    })
}

fn entity_subject(entity: &EntityRecord) -> Value {
    json!({
        "entity_id": entity.id.get().to_string(),
        "kind": entity.kind.as_str(),
        "label": entity.label,
    })
}

fn field_changes(before: &EntityRecord, after: &EntityRecord) -> (Vec<Value>, usize) {
    let names: BTreeSet<&String> = before.fields.keys().chain(after.fields.keys()).collect();
    let mut changed = Vec::new();
    let mut total = 0usize;
    for name in names {
        let old = before.fields.get(name);
        let new = after.fields.get(name);
        let same = match (old, new) {
            (Some(a), Some(b)) => a.value == b.value && a.presence == b.presence,
            (None, None) => true,
            _ => false,
        };
        if same {
            continue;
        }
        total += 1;
        if changed.len() < MAX_FIELDS_PER_ENTITY {
            changed.push(json!({
                "field": name,
                "before": old.map_or(Value::Null, |fact| value_json(&fact.value)),
                "after": new.map_or(Value::Null, |fact| value_json(&fact.value)),
            }));
        }
    }
    (changed, total)
}

fn expand(chunk: Option<&MapChunk>) -> Option<Vec<u32>> {
    let chunk = chunk?;
    let mut tiles = Vec::with_capacity(usize::from(chunk.width) * usize::from(chunk.height));
    for run in &chunk.terrain_runs {
        for _ in 0..run.length {
            tiles.push(run.tile_code);
        }
    }
    (tiles.len() == usize::from(chunk.width) * usize::from(chunk.height)
        && i32::from(chunk.width) == CHUNK_EDGE)
        .then_some(tiles)
}

#[derive(Default)]
struct LevelChange {
    tiles: u64,
    min: (i32, i32),
    max: (i32, i32),
    transitions: BTreeMap<String, u64>,
}

fn terrain_changes(base: &WorldSnapshot, target: &WorldSnapshot) -> Vec<Value> {
    let coords: BTreeSet<&ChunkCoord> = base
        .graph
        .chunks
        .keys()
        .chain(target.graph.chunks.keys())
        .collect();
    let mut levels: BTreeMap<i32, LevelChange> = BTreeMap::new();
    for coord in coords {
        let before = base.graph.chunks.get(coord);
        let after = target.graph.chunks.get(coord);
        if before == after {
            continue;
        }
        let (Some(old), Some(new)) = (expand(before), expand(after)) else {
            // A chunk that appeared, vanished or is not canonical: report the
            // level as changed without inventing tile detail.
            levels
                .entry(coord.z)
                .or_default()
                .transitions
                .insert("chunk_observed_or_lost".to_owned(), 1);
            continue;
        };
        for (offset, (a, b)) in old.iter().zip(new.iter()).enumerate() {
            if a == b {
                continue;
            }
            let local = i32::try_from(offset).unwrap_or(0);
            let x = coord.x * CHUNK_EDGE + local % CHUNK_EDGE;
            let y = coord.y * CHUNK_EDGE + local / CHUNK_EDGE;
            let level = levels.entry(coord.z).or_default();
            if level.tiles == 0 {
                level.min = (x, y);
                level.max = (x, y);
            }
            level.tiles += 1;
            level.min = (level.min.0.min(x), level.min.1.min(y));
            level.max = (level.max.0.max(x), level.max.1.max(y));
            *level
                .transitions
                .entry(format!("{}->{}", tile_name(*a), tile_name(*b)))
                .or_default() += 1;
        }
    }
    levels
        .into_iter()
        .map(|(z, level)| {
            json!({
                "kind": "terrain_changed",
                "subject": {"z": z},
                "tiles_changed": level.tiles,
                "bounding_box": (level.tiles > 0).then(|| json!({
                    "min": [level.min.0, level.min.1, z],
                    "max": [level.max.0, level.max.1, z],
                })),
                "transitions": level.transitions,
                "epistemic_state": "observed",
                "invalidates": [],
                "evidence": [],
            })
        })
        .collect()
}

/// Observed world changes from `base` to `target`, most structural first.
pub(crate) fn describe(base: &WorldSnapshot, target: &WorldSnapshot) -> Vec<Value> {
    if base.state_hash == target.state_hash || base.fortress_id != target.fortress_id {
        return Vec::new();
    }
    let basis = json!({"from": anchor_ref(base), "to": anchor_ref(target)});
    let mut out = Vec::new();
    if base.tick != target.tick {
        out.push(json!({
            "kind": "game_time_passed",
            "subject": {"from_tick": base.tick.0, "to_tick": target.tick.0},
            "epistemic_state": "observed",
            "basis": basis,
            "invalidates": [],
            "evidence": [],
        }));
    }
    if base.paused != target.paused {
        out.push(json!({
            "kind": if target.paused { "fortress_paused" } else { "fortress_unpaused" },
            "subject": {"paused": target.paused},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
        }));
    }
    let ids: BTreeSet<_> = base
        .graph
        .entities
        .keys()
        .chain(target.graph.entities.keys())
        .collect();
    let mut entity_changes = Vec::new();
    let mut omitted = 0usize;
    for id in ids {
        let change = match (base.graph.entities.get(id), target.graph.entities.get(id)) {
            (None, Some(created)) => Some(json!({
                "kind": "entity_created",
                "subject": entity_subject(created),
                "epistemic_state": "observed",
                "invalidates": [],
                "evidence": [],
            })),
            (Some(removed), None) => Some(json!({
                "kind": "entity_removed",
                "subject": entity_subject(removed),
                "epistemic_state": "observed",
                "invalidates": ["handles_naming_this_entity"],
                "evidence": [],
            })),
            (Some(before), Some(after)) if before != after => {
                let (fields, total) = field_changes(before, after);
                (total > 0 || before.generation != after.generation).then(|| {
                    json!({
                        "kind": "entity_changed",
                        "subject": entity_subject(after),
                        "fields": fields,
                        "fields_changed": total,
                        "generation_changed": before.generation != after.generation,
                        "epistemic_state": "observed",
                        "invalidates": [],
                        "evidence": [],
                    })
                })
            }
            _ => None,
        };
        if let Some(change) = change {
            if entity_changes.len() < MAX_ENTITY_CHANGES {
                entity_changes.push(change);
            } else {
                omitted += 1;
            }
        }
    }
    out.extend(entity_changes);
    if omitted > 0 {
        out.push(json!({
            "kind": "entity_changes_omitted",
            "subject": {"omitted": omitted},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
            "note": "more entities changed than one turn lists; query entities for the rest",
        }));
    }
    out.extend(terrain_changes(base, target));
    out
}
