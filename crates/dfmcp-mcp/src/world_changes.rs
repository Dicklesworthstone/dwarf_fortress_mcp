//! What changed in the world between two observed laboratory anchors.
//!
//! The Agent Turn must answer "what changed" without transcript memory. The
//! laboratory keeps a bounded, immutable history of the canonical world
//! versions each session has seen; this module compares the agent's previous
//! anchor with the current one and reports entity, relation, retained-event,
//! terrain, clock and pause changes. These are differences between exact
//! canonical records, not proof of event authenticity or intervening history.
//! Output has independent domain bounds and canonical identity/field ordering.
//! A different fortress or observation epoch yields a reset, never an ordinary
//! delta that could disguise restore as game progress.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_world::{
    ChunkCoord, EdgeRecord, EntityRecord, Fact, MapChunk, WorldEvent, WorldSnapshot,
};
use serde_json::{Value, json};

use crate::lab_world::tile_name;
use crate::observation_projection::{
    anchor_json, fact_value, presence_json, value_json, value_presence,
};

/// Most entity changes listed in one turn.
pub(crate) const MAX_ENTITY_CHANGES: usize = 24;
/// Each domain has its own allowance: entity churn cannot hide an assignment
/// change or consume every retained-event slot.
const MAX_RELATION_CHANGES: usize = 24;
const MAX_EVENT_CHANGES: usize = 24;
const MAX_TERRAIN_CHANGES: usize = 24;
/// Most changed fields listed per entity, relation or retained event.
const MAX_FIELDS_PER_ENTITY: usize = 8;
const CHUNK_EDGE: i32 = dfmcp_world::terrain::TERRAIN_CHUNK_EDGE;

fn anchor_ref(snapshot: &WorldSnapshot) -> Value {
    anchor_json(&snapshot.anchor())
}

fn entity_subject(entity: &EntityRecord) -> Value {
    json!({
        "entity_id": entity.id.get().to_string(),
        "kind": entity.kind.as_str(),
        "label": entity.label,
        "generation": entity.generation,
        "revision": entity.revision,
    })
}

fn change_presence(fact: Option<&Fact>) -> Value {
    match fact {
        Some(fact) => {
            let mut presence = presence_json(fact);
            presence["recorded"] = json!(true);
            presence
        }
        None => json!({
            "state": "unknown",
            "recorded": false,
            "reason": "field is not present in the retained record",
        }),
    }
}

fn field_changes(before: &EntityRecord, after: &EntityRecord) -> (Vec<Value>, usize) {
    fact_field_changes(&before.fields, &after.fields)
}

fn fact_field_changes(
    before: &BTreeMap<String, Fact>,
    after: &BTreeMap<String, Fact>,
) -> (Vec<Value>, usize) {
    let names: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut changed = Vec::new();
    let mut total = 0usize;
    for name in names {
        let old = before.get(name);
        let new = after.get(name);
        if old == new {
            continue;
        }
        total += 1;
        if changed.len() < MAX_FIELDS_PER_ENTITY {
            changed.push(json!({
                "field": name,
                "before": old.map_or(Value::Null, fact_value),
                "after": new.map_or(Value::Null, fact_value),
                "before_presence": change_presence(old),
                "after_presence": change_presence(new),
                "provenance_changed": matches!((old, new), (Some(a), Some(b))
                    if a.observed_at != b.observed_at || a.source != b.source
                        || a.source_digest != b.source_digest),
            }));
        }
    }
    (changed, total)
}

fn relation_subject(edge: &EdgeRecord, snapshot: &WorldSnapshot) -> Value {
    json!({
        "edge_id": edge.id.get().to_string(),
        "kind": edge.kind.as_str(),
        "revision": edge.revision,
        "from": edge.from.get().to_string(),
        "to": edge.to.get().to_string(),
        // These generations belong to the endpoint records at this side's
        // anchor. A dangling relation supplies no generation; zero is not a
        // substitute and the opposite snapshot cannot fill the gap.
        "from_generation": snapshot.graph.entities.get(&edge.from).map(|e| e.generation),
        "to_generation": snapshot.graph.entities.get(&edge.to).map(|e| e.generation),
    })
}

fn omitted_change(kind: &str, omitted: usize, note: &str) -> Value {
    json!({
        "kind": kind,
        "subject": {"omitted": omitted},
        "epistemic_state": "observed",
        "invalidates": [],
        "evidence": [],
        "note": note,
    })
}

fn relation_changes(base: &WorldSnapshot, target: &WorldSnapshot) -> Vec<Value> {
    let ids: BTreeSet<_> = base
        .graph
        .edges
        .keys()
        .chain(target.graph.edges.keys())
        .collect();
    let empty = BTreeMap::new();
    let mut out = Vec::new();
    let mut omitted = 0;
    for id in ids {
        let before = base.graph.edges.get(id);
        let after = target.graph.edges.get(id);
        let old = before.map(|edge| relation_subject(edge, base));
        let new = after.map(|edge| relation_subject(edge, target));
        // An endpoint's ABA generation change matters even if the edge bytes
        // did not change. Canonical kind spelling, rather than enum variant,
        // determines the reported relationship.
        if old == new && before.map(|e| &e.fields) == after.map(|e| &e.fields) {
            continue;
        }
        if out.len() == MAX_RELATION_CHANGES {
            omitted += 1;
            continue;
        }
        let (fields, total) = fact_field_changes(
            before.map_or(&empty, |edge| &edge.fields),
            after.map_or(&empty, |edge| &edge.fields),
        );
        out.push(json!({
            "kind": match (before, after) {
                (None, Some(_)) => "relation_created",
                (Some(_), None) => "relation_removed",
                _ => "relation_changed",
            },
            "subject": new.as_ref().or(old.as_ref()),
            "before": old,
            "after": new,
            "fields_omitted": total.saturating_sub(fields.len()),
            "fields": fields,
            "fields_changed": total,
            "epistemic_state": "observed",
            "invalidates": ["witnesses_naming_this_relation_or_its_adjacency"],
            "evidence": [],
        }));
    }
    if omitted > 0 {
        out.push(omitted_change(
            "relation_changes_omitted",
            omitted,
            "more relations changed than one turn lists; inspect relations at both retained anchors",
        ));
    }
    out
}

fn event_subject(event: &WorldEvent) -> Value {
    json!({
        "event_id": event.id.get().to_string(),
        "kind": event.kind.as_str(),
        "tick": event.tick.0,
        "entity_id": event.subject.map(|id| id.get().to_string()),
        "summary": event.summary,
        // WorldEvent records no subject generation. Looking it up in either
        // snapshot would invent a binding to an entity at the event's tick.
        "subject_generation": null,
    })
}

fn event_value_presence(value: Option<&dfmcp_world::Value>) -> Value {
    match value {
        Some(value) => {
            let mut presence = value_presence(value);
            presence["recorded"] = json!(true);
            presence
        }
        None => change_presence(None),
    }
}

fn event_field_changes(
    before: Option<&WorldEvent>,
    after: Option<&WorldEvent>,
) -> (Vec<Value>, usize) {
    let names: BTreeSet<_> = before
        .into_iter()
        .flat_map(|event| event.fields.keys())
        .chain(after.into_iter().flat_map(|event| event.fields.keys()))
        .collect();
    let mut out = Vec::new();
    let mut total = 0;
    for name in names {
        let old = before.and_then(|event| event.fields.get(name));
        let new = after.and_then(|event| event.fields.get(name));
        if old == new {
            continue;
        }
        total += 1;
        if out.len() < MAX_FIELDS_PER_ENTITY {
            out.push(json!({
                "field": name,
                "before": old.map_or(Value::Null, value_json),
                "after": new.map_or(Value::Null, value_json),
                "before_presence": event_value_presence(old),
                "after_presence": event_value_presence(new),
            }));
        }
    }
    (out, total)
}

fn event_changes(base: &WorldSnapshot, target: &WorldSnapshot) -> Vec<Value> {
    let ids: BTreeSet<_> = base
        .graph
        .events
        .keys()
        .chain(target.graph.events.keys())
        .collect();
    let mut out = Vec::new();
    let mut omitted = 0;
    for id in ids {
        let before = base.graph.events.get(id);
        let after = target.graph.events.get(id);
        let old = before.map(event_subject);
        let new = after.map(event_subject);
        if old == new && before.map(|e| &e.fields) == after.map(|e| &e.fields) {
            continue;
        }
        if out.len() == MAX_EVENT_CHANGES {
            omitted += 1;
            continue;
        }
        let (fields, total) = event_field_changes(before, after);
        out.push(json!({
            "kind": match (before, after) {
                (None, Some(_)) => "event_observed",
                (Some(_), None) => "event_no_longer_retained",
                _ => "retained_event_changed",
            },
            "subject": new.as_ref().or(old.as_ref()),
            "before": old,
            "after": new,
            "fields_omitted": total.saturating_sub(fields.len()),
            "fields": fields,
            "fields_changed": total,
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
            "coverage": {
                "domain": "retained_event_projection",
                "history_complete": false,
                "occurrence_since_basis_proven": false,
                "eligible_as_mutation_evidence": false,
            },
            "provenance": {
                "source_authenticity": "not_established",
                "field_provenance": "not_recorded_by_the_canonical_event_type",
                "taint": "untrusted_source_text",
            },
            "note": "retained record membership is observed; arrival need not mean a new occurrence, and removal never means the event was undone",
        }));
    }
    if omitted > 0 {
        out.push(omitted_change(
            "event_changes_omitted",
            omitted,
            "more retained events changed than one turn lists; query events at both retained anchors; complete fortress history is not established",
        ));
    }
    out
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
    let omitted = levels.len().saturating_sub(MAX_TERRAIN_CHANGES);
    let mut out: Vec<_> = levels
        .into_iter()
        .take(MAX_TERRAIN_CHANGES)
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
        .collect();
    if omitted > 0 {
        out.push(omitted_change(
            "terrain_changes_omitted",
            omitted,
            "more terrain levels changed than one turn lists; query terrain at both retained anchors",
        ));
    }
    out
}

/// Observed world changes from `base` to `target`, most structural first.
pub(crate) fn describe(base: &WorldSnapshot, target: &WorldSnapshot) -> Vec<Value> {
    let basis = json!({"from": anchor_ref(base), "to": anchor_ref(target)});
    if base.fortress_id != target.fortress_id || base.cursor.epoch != target.cursor.epoch {
        return vec![json!({
            "kind": "world_observation_reset",
            "subject": {"reason": if base.fortress_id != target.fortress_id {
                "fortress_lineage_changed"
            } else { "observation_epoch_changed" }},
            "basis": basis,
            "epistemic_state": "observed",
            "invalidates": ["all_prior_world_handles_and_continuations"],
            "evidence": [],
            "note": "these anchors do not share an observation epoch and fortress; establish a new baseline instead of interpreting ordinary world changes",
        })];
    }
    if target.cursor.sequence < base.cursor.sequence || target.tick < base.tick {
        return vec![json!({
            "kind": "world_history_gap",
            "subject": {"reason": "target_precedes_basis_within_epoch"},
            "basis": basis,
            "epistemic_state": "unknown",
            "invalidates": [],
            "evidence": [],
            "note": "the target does not follow the requested basis; no forward world delta is established",
        })];
    }
    if base.state_hash == target.state_hash {
        return Vec::new();
    }
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
                let old = entity_subject(before);
                let new = entity_subject(after);
                (total > 0 || old != new).then(|| {
                    json!({
                        "kind": "entity_changed",
                        "subject": new,
                        "before": old,
                        "after": new,
                        "fields_omitted": total.saturating_sub(fields.len()),
                        "fields": fields,
                        "fields_changed": total,
                        "generation_changed": before.generation != after.generation,
                        "epistemic_state": "observed",
                        "invalidates": if before.generation != after.generation {
                            vec!["handles_naming_this_entity"]
                        } else { Vec::<&str>::new() },
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
        out.push(omitted_change(
            "entity_changes_omitted",
            omitted,
            "more entities changed than one turn lists; query entities at both retained anchors",
        ));
    }
    out.extend(relation_changes(base, target));
    out.extend(terrain_changes(base, target));
    out.extend(event_changes(base, target));
    for change in &mut out {
        change["basis"] = basis.clone();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        Digest32, EdgeId, EntityId, EventId, FortressId, GameTick, ObservationCursor, StateAnchor,
    };
    use dfmcp_world::{
        EdgeKind, EntityKind, FactPresence, FactSource, Value as WorldValue, WorldEventKind,
        WorldGraph,
    };

    fn anchor() -> StateAnchor {
        StateAnchor {
            fortress_id: FortressId::new(7),
            cursor: ObservationCursor::ORIGIN,
            tick: GameTick(20),
            state_hash: Digest32::ZERO,
        }
    }

    fn record(fact: Option<Fact>) -> EntityRecord {
        EntityRecord {
            id: EntityId::new(1),
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Urist".to_owned(),
            fields: fact
                .into_iter()
                .map(|fact| ("subject".to_owned(), fact))
                .collect(),
        }
    }

    fn fact(presence: FactPresence) -> Fact {
        Fact::with_presence(presence, GameTick(20), FactSource::Replay, Digest32::ZERO)
    }


    fn snapshot() -> WorldSnapshot {
        WorldSnapshot::new(
            FortressId::new(7),
            GameTick(20),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph {
                entities: BTreeMap::from([(EntityId::new(1), record(None))]),
                ..WorldGraph::default()
            },
        )
    }

    fn successor(base: &WorldSnapshot) -> WorldSnapshot {
        let mut next = base.clone();
        next.cursor.sequence += 1;
        next.refresh_hash();
        next
    }

    fn relation(id: u128) -> EdgeRecord {
        EdgeRecord {
            id: EdgeId::new(id),
            revision: 1,
            kind: EdgeKind::AssignedTo,
            from: EntityId::new(1),
            to: EntityId::new(2),
            fields: BTreeMap::new(),
        }
    }

    fn event(id: u128) -> WorldEvent {
        WorldEvent {
            id: EventId::new(id),
            tick: GameTick(10),
            kind: WorldEventKind::Announcement,
            subject: Some(EntityId::new(1)),
            summary: "A retained notice".to_owned(),
            fields: BTreeMap::new(),
        }
    }

    #[test]
    fn entity_metadata_only_changes_have_exact_before_and_after_identity() {
        let base = snapshot();
        for metadata in ["label", "kind", "revision", "generation"] {
            let mut target = successor(&base);
            let mut entity = record(None);
            match metadata {
                "label" => entity.label = "Urist renamed".to_owned(),
                "kind" => entity.kind = EntityKind::Item,
                "revision" => entity.revision = 2,
                _ => entity.generation = 2,
            }
            target.graph.entities.insert(entity.id, entity);
            target.refresh_hash();
            let changes = describe(&base, &target);
            assert_eq!(changes.len(), 1, "{metadata}: {changes:?}");
            let change = &changes[0];
            assert_eq!(change["kind"], "entity_changed");
            assert_eq!(change["fields_changed"], 0);
            assert_ne!(change["before"][metadata], change["after"][metadata]);
            assert_eq!(change["basis"]["from"], anchor_ref(&base));
            assert_eq!(change["basis"]["to"], anchor_ref(&target));
            assert_eq!(change["generation_changed"], metadata == "generation");
            assert_eq!(
                change["invalidates"].as_array().map(Vec::len),
                Some(usize::from(metadata == "generation"))
            );
        }
    }

    #[test]
    fn relation_only_creation_change_and_removal_are_not_invisible() {
        let base = snapshot();
        let mut created = successor(&base);
        let edge = relation(u128::MAX);
        created.graph.edges.insert(edge.id, edge.clone());
        created.refresh_hash();
        let changes = describe(&base, &created);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["kind"], "relation_created");
        assert_eq!(changes[0]["subject"]["edge_id"], u128::MAX.to_string());
        assert_eq!(changes[0]["after"]["from_generation"], 1);
        assert!(changes[0]["after"]["to_generation"].is_null());
        assert!(changes[0]["before"].is_null());

        let mut updated = successor(&created);
        let mut replacement = edge.clone();
        replacement.kind = EdgeKind::MemberOf;
        replacement.from = EntityId::new(2);
        replacement.to = EntityId::new(1);
        replacement.revision = 2;
        replacement.fields.insert(
            "assigned".to_owned(),
            fact(FactPresence::Known(WorldValue::Bool(true))),
        );
        updated.graph.edges.insert(edge.id, replacement);
        updated.refresh_hash();
        let changes = describe(&created, &updated);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["kind"], "relation_changed");
        assert_eq!(changes[0]["before"]["kind"], "assigned_to");
        assert_eq!(changes[0]["after"]["kind"], "member_of");
        assert_eq!(changes[0]["after"]["to_generation"], 1);
        assert!(changes[0]["after"]["from_generation"].is_null());
        assert_eq!(changes[0]["fields"][0]["after"], true);
        assert_eq!(changes[0]["fields"][0]["before_presence"]["recorded"], false);

        let mut removed = successor(&updated);
        removed.graph.edges.clear();
        removed.refresh_hash();
        let changes = describe(&updated, &removed);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["kind"], "relation_removed");
        assert_eq!(changes[0]["before"]["revision"], 2);
        assert!(changes[0]["after"].is_null());
        assert_eq!(changes[0]["fields"][0]["after_presence"]["recorded"], false);
    }

    #[test]
    fn relation_endpoint_generation_changes_are_bound_to_each_snapshot() {
        let mut base = snapshot();
        base.graph.edges.insert(EdgeId::new(1), relation(1));
        base.refresh_hash();
        let mut target = successor(&base);
        let mut reused = record(None);
        reused.generation = 7;
        target.graph.entities.insert(reused.id, reused);
        target.refresh_hash();
        let changes = describe(&base, &target);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[1]["kind"], "relation_changed");
        assert_eq!(changes[1]["before"]["from_generation"], 1);
        assert_eq!(changes[1]["after"]["from_generation"], 7);
        assert!(changes[1]["before"]["to_generation"].is_null());
        assert!(changes[1]["after"]["to_generation"].is_null());
    }

    #[test]
    fn relation_fields_use_the_same_unavailable_value_mask_as_entities() {
        for presence in [
            FactPresence::Absent,
            FactPresence::Unknown("not observed".to_owned()),
            FactPresence::Unsupported("not supported".to_owned()),
            FactPresence::Omitted("outside projection".to_owned()),
            FactPresence::Redacted("policy/1".to_owned()),
            FactPresence::Stale(anchor()),
            FactPresence::Known(WorldValue::Bool(true)),
        ] {
            let base = snapshot();
            let mut target = successor(&base);
            let mut concealed = fact(presence);
            concealed.value = WorldValue::Text("relation-secret".to_owned());
            let expected_presence = presence_json(&concealed);
            let mut edge = relation(1);
            edge.fields.insert("secret".to_owned(), concealed);
            target.graph.edges.insert(edge.id, edge);
            target.refresh_hash();
            let changes = describe(&base, &target);
            assert_eq!(
                changes[0]["fields"][0]["after_presence"]["state"],
                expected_presence["state"]
            );
            assert!(changes[0]["fields"][0]["after"].is_null());
            assert!(!json!(changes).to_string().contains("relation-secret"));
        }
    }

    #[test]
    fn retained_events_distinguish_arrival_revision_and_retention_loss() {
        let mut base = snapshot();
        base.graph.events.insert(EventId::new(1), event(1));
        base.graph.events.insert(EventId::new(2), event(2));
        base.refresh_hash();
        let mut target = successor(&base);
        target.graph.events.remove(&EventId::new(2));
        let mut revised = event(1);
        revised.summary = "A corrected notice".to_owned();
        revised.fields.insert(
            "payload".to_owned(),
            WorldValue::List(vec![WorldValue::Bytes(vec![4, 5])]),
        );
        target.graph.events.insert(revised.id, revised);
        target.graph.events.insert(EventId::new(3), event(3));
        target.refresh_hash();
        let changes = describe(&base, &target);
        assert_eq!(
            changes.iter().map(|row| row["kind"].as_str()).collect::<Vec<_>>(),
            [
                Some("retained_event_changed"),
                Some("event_no_longer_retained"),
                Some("event_observed")
            ]
        );
        assert_eq!(changes[0]["before"]["summary"], "A retained notice");
        assert_eq!(changes[0]["after"]["summary"], "A corrected notice");
        assert_eq!(changes[0]["fields"][0]["after"], json!([{"bytes":2}]));
        assert_eq!(changes[0]["fields"][0]["after_presence"]["state"], "omitted");
        assert!(changes[1]["after"].is_null());
        assert!(changes[2]["before"].is_null());
        assert_eq!(changes[2]["subject"]["tick"], 10);
        for change in changes {
            assert!(change["subject"]["subject_generation"].is_null());
            assert_eq!(change["coverage"]["history_complete"], false);
            assert_eq!(change["coverage"]["occurrence_since_basis_proven"], false);
            assert_eq!(change["coverage"]["eligible_as_mutation_evidence"], false);
            assert_eq!(change["provenance"]["taint"], "untrusted_source_text");
        }
    }

    #[test]
    fn each_domain_and_each_field_list_has_an_independent_explicit_bound() {
        let mut base = snapshot();
        base.graph.entities.clear();
        base.refresh_hash();
        let mut target = successor(&base);
        for id in 1..=28 {
            let mut entity = record(None);
            entity.id = EntityId::new(id);
            target.graph.entities.insert(entity.id, entity);
        }
        for id in 1..=26 {
            let mut edge = relation(id);
            for field in 0..10 {
                edge.fields.insert(
                    format!("f{field:02}"),
                    fact(FactPresence::Known(WorldValue::Bool(true))),
                );
            }
            target.graph.edges.insert(edge.id, edge);
            let coord = ChunkCoord { x: 0, y: 0, z: id as i32 };
            target.graph.chunks.insert(
                coord,
                dfmcp_world::terrain::uniform_chunk(coord, dfmcp_world::tile_codes::FLOOR),
            );
        }
        for id in 1..=27 {
            let mut event = event(id);
            for field in 0..11 {
                event.fields.insert(format!("f{field:02}"), WorldValue::U64(field));
            }
            target.graph.events.insert(event.id, event);
        }
        target.refresh_hash();
        let changes = describe(&base, &target);
        for (kind, expected, omitted) in [
            ("entity", MAX_ENTITY_CHANGES, 4),
            ("relation", MAX_RELATION_CHANGES, 2),
            ("event", MAX_EVENT_CHANGES, 3),
            ("terrain", MAX_TERRAIN_CHANGES, 2),
        ] {
            let marker = format!("{kind}_changes_omitted");
            let rows: Vec<_> = changes
                .iter()
                .filter(|row| row["kind"].as_str().is_some_and(|name| name.starts_with(kind)))
                .collect();
            assert_eq!(rows.len(), expected + 1, "{kind}");
            assert_eq!(rows[expected]["kind"], marker);
            assert_eq!(rows[expected]["subject"]["omitted"], omitted);
        }
        for row in &changes {
            if row["kind"] == "relation_created" || row["kind"] == "event_observed" {
                assert_eq!(row["fields"].as_array().map(Vec::len), Some(MAX_FIELDS_PER_ENTITY));
                assert_eq!(row["fields"][0]["field"], "f00");
                assert_eq!(row["fields"][7]["field"], "f07");
                assert_eq!(
                    row["fields_omitted"],
                    if row["kind"] == "relation_created" { 2 } else { 3 }
                );
            }
            assert_eq!(row["basis"]["from"], anchor_ref(&base));
            assert_eq!(row["basis"]["to"], anchor_ref(&target));
        }
        assert_eq!(changes, describe(&base, &target));
    }

    #[test]
    fn resets_and_reversed_cursors_never_masquerade_as_forward_world_changes() {
        let base = snapshot();
        for reason in ["fortress_lineage_changed", "observation_epoch_changed"] {
            let mut target = successor(&base);
            target.graph.entities.clear();
            target.tick = GameTick(0);
            if reason == "fortress_lineage_changed" {
                target.fortress_id = FortressId::new(8);
            } else {
                target.cursor.epoch += 1;
                target.cursor.sequence = 0;
            }
            target.refresh_hash();
            let changes = describe(&base, &target);
            assert_eq!(changes.len(), 1);
            assert_eq!(changes[0]["kind"], "world_observation_reset");
            assert_eq!(changes[0]["subject"]["reason"], reason);
        }
        let mut newer = successor(&base);
        newer.tick = GameTick(30);
        newer.refresh_hash();
        let changes = describe(&newer, &base);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0]["kind"], "world_history_gap");
        assert_eq!(changes[0]["epistemic_state"], "unknown");
        assert!(describe(&base, &base).is_empty());
    }

    #[test]
    fn canonical_kind_aliases_do_not_create_false_changes() {
        let mut base = snapshot();
        base.graph.edges.insert(EdgeId::new(1), relation(1));
        base.graph.events.insert(EventId::new(1), event(1));
        base.refresh_hash();
        let mut target = successor(&base);
        let mut entity = record(None);
        entity.kind = EntityKind::Other("unit".to_owned());
        target.graph.entities.insert(entity.id, entity);
        let mut edge = relation(1);
        edge.kind = EdgeKind::Custom("assigned_to".to_owned());
        target.graph.edges.insert(edge.id, edge);
        let mut event = event(1);
        event.kind = WorldEventKind::Other("announcement".to_owned());
        target.graph.events.insert(event.id, event);
        target.refresh_hash();
        assert!(describe(&base, &target).is_empty());
    }

    #[test]
    fn unavailable_change_values_are_withheld_with_their_presence_intact() {
        for presence in [
            FactPresence::Absent,
            FactPresence::Unknown("not observed".to_owned()),
            FactPresence::Unsupported("not supported".to_owned()),
            FactPresence::Omitted("outside projection".to_owned()),
            FactPresence::Redacted("policy/1".to_owned()),
            FactPresence::Stale(anchor()),
            FactPresence::Known(WorldValue::Bool(true)),
        ] {
            let mut before = fact(presence.clone());
            before.value = WorldValue::Text("withheld-before".to_owned());
            let mut after = fact(presence);
            after.value = WorldValue::Text("withheld-after".to_owned());
            let before_presence = presence_json(&before);
            let after_presence = presence_json(&after);
            let (changes, total) = field_changes(&record(Some(before)), &record(Some(after)));
            assert_eq!(total, 1);
            assert!(changes[0]["before"].is_null());
            assert!(changes[0]["after"].is_null());
            assert_eq!(
                changes[0]["before_presence"]["state"],
                before_presence["state"]
            );
            assert_eq!(
                changes[0]["after_presence"]["state"],
                after_presence["state"]
            );
            assert_eq!(changes[0]["before_presence"]["recorded"], true);
            assert!(!json!(changes).to_string().contains("withheld-"));
        }
    }

    #[test]
    fn unrecorded_fields_are_distinct_from_explicit_absence_and_known_null() {
        for (before, after, old_state, new_state, old_recorded, new_recorded) in [
            (
                None,
                Some(fact(FactPresence::Absent)),
                "unknown",
                "absent",
                false,
                true,
            ),
            (
                Some(fact(FactPresence::Absent)),
                None,
                "absent",
                "unknown",
                true,
                false,
            ),
            (
                Some(fact(FactPresence::Known(WorldValue::Null))),
                Some(fact(FactPresence::Absent)),
                "known",
                "absent",
                true,
                true,
            ),
        ] {
            let (changes, total) = field_changes(&record(before), &record(after));
            assert_eq!(total, 1);
            assert!(changes[0]["before"].is_null());
            assert!(changes[0]["after"].is_null());
            assert_eq!(changes[0]["before_presence"]["state"], old_state);
            assert_eq!(changes[0]["after_presence"]["state"], new_state);
            assert_eq!(changes[0]["before_presence"]["recorded"], old_recorded);
            assert_eq!(changes[0]["after_presence"]["recorded"], new_recorded);
        }
    }

    #[test]
    fn known_structured_values_keep_null_members_in_change_evidence() {
        let before = record(Some(fact(FactPresence::Known(WorldValue::Object(
            BTreeMap::from([("slot".to_owned(), WorldValue::Null)]),
        )))));
        let after = record(Some(fact(FactPresence::Known(WorldValue::Object(
            BTreeMap::from([("slot".to_owned(), WorldValue::U64(7))]),
        )))));
        let (changes, total) = field_changes(&before, &after);
        assert_eq!(total, 1);
        assert_eq!(changes[0]["before"], json!({"slot": null}));
        assert_eq!(changes[0]["after"], json!({"slot": 7}));
        assert_eq!(changes[0]["before_presence"]["state"], "known");
    }
}
