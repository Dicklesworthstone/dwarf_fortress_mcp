//! Exact-coordinate terrain predicates. Missing cells remain possible matches;
//! an incomplete projection cannot prove that a requested excavation is finished.
use super::super::{Comparison, Probe, Truth, bounded, compare, digest, invalid};
use super::{EvaluationBudget, Predicate, interval_truth, relationships};
use dfmcp_adapter::live_map::tile_entity_id;
use dfmcp_core::{Digest32, EntityId, GameTick, Result};
use dfmcp_world::{
    EntityKind, EntityRecord, FactPresence, FactSource, Value as WorldValue, WorldSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

const MAX_AREAS: usize = 64;
const MAX_TILES: u64 = 16_384;
const POLICY: &str = "dfmcp.requested-terrain-count/1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in super::super) struct Area {
    pub min: [u32; 3],
    pub max: [u32; 3],
}

/// A disjoint mask prevents a duplicated cuboid from counting one tile twice.
/// Bounds are enforced before iterating coordinates or allocating examples.
pub(in super::super) fn validate(areas: &[Area]) -> Result<u64> {
    if areas.is_empty() || areas.len() > MAX_AREAS {
        return Err(bounded("terrain mask requires 1..64 disjoint cuboids"));
    }
    let mut total = 0u64;
    for (index, area) in areas.iter().enumerate() {
        let mut volume = 1u64;
        for axis in 0..3 {
            if area.min[axis] > area.max[axis] || area.max[axis] >= 32_768 {
                return Err(invalid(
                    "terrain mask coordinates must be ordered and within 0..32767",
                ));
            }
            volume *= u64::from(area.max[axis] - area.min[axis] + 1);
        }
        total += volume; // At most 64 * 32768^3, well within u64.
        if total > MAX_TILES {
            return Err(bounded("terrain mask exceeds 16384 requested tiles"));
        }
        if areas[..index].iter().any(|other| {
            (0..3)
                .all(|axis| area.min[axis] <= other.max[axis] && other.min[axis] <= area.max[axis])
        }) {
            return Err(invalid("terrain mask cuboids must not overlap"));
        }
    }
    Ok(total)
}

struct Capture {
    prefix: &'static str,
    source: Digest32,
    region_min: [u32; 3],
    region_size: [u32; 3],
    dimensions: [u32; 3],
}

fn observed<'a>(
    entity: &'a EntityRecord,
    field: &str,
    suffix: &str,
    prefix: &str,
    source: Digest32,
    tick: GameTick,
) -> Option<&'a WorldValue> {
    let fact = entity.fields.get(field)?;
    if fact.source_digest != source || source == Digest32::ZERO || fact.observed_at != tick {
        return None;
    }
    if !matches!(&fact.source, FactSource::DfhackField(path) if path.strip_prefix(prefix) == Some(suffix))
    {
        return None;
    }
    match &fact.presence {
        None => Some(&fact.value),
        Some(FactPresence::Known(value)) if value == &fact.value => Some(&fact.value),
        _ => None,
    }
}

fn coordinates(value: &WorldValue) -> Option<[u32; 3]> {
    let WorldValue::Coord(p) = value else {
        return None;
    };
    Some([
        u32::try_from(p.x).ok()?,
        u32::try_from(p.y).ok()?,
        u32::try_from(p.z).ok()?,
    ])
}

fn capture(snapshot: &WorldSnapshot) -> Option<Capture> {
    let root = snapshot.graph.entities.get(&EntityId::new(1))?;
    if root.id != EntityId::new(1)
        || root.kind != EntityKind::Fortress
        || root.generation == 0
        || root.revision == 0
    {
        return None;
    }
    let origin = root.fields.get("region_origin")?;
    let FactSource::DfhackField(path) = &origin.source else {
        return None;
    };
    let prefix = match path.as_str() {
        "map/1.5.requested_region" => "map/1.5.",
        "spatial/1.6.map.requested_region" => "spatial/1.6.map.",
        "spatial/1.8.map.requested_region" => "spatial/1.8.map.",
        _ => return None,
    };
    let source = origin.source_digest;
    let value = |field, suffix| observed(root, field, suffix, prefix, source, snapshot.tick);
    let region_min = coordinates(value("region_origin", "requested_region")?)?;
    let region_size = coordinates(value("region_size", "requested_region")?)?;
    let dimensions = coordinates(value("map_dimensions", "Maps.getTileSize")?)?;
    dfmcp_world::map_region::Region {
        origin: region_min,
        size: region_size,
    }
    .volume()
    .ok()?;
    if (0..3).any(|i| {
        dimensions[i] == 0
            || dimensions[i] > 32_768
            || region_min[i] + region_size[i] > dimensions[i]
    }) {
        return None;
    }
    Some(Capture {
        prefix,
        source,
        region_min,
        region_size,
        dimensions,
    })
}

fn native_suffix(field: &str) -> Option<&'static str> {
    match field {
        "tiletype" => Some("tiletype"),
        "shape" => Some("shape"),
        "liquid_depth" => Some("liquid_depth"),
        "magma" => Some("magma"),
        "traffic" => Some("traffic"),
        "dig_designation" => Some("dig_designation"),
        "building_occupancy" => Some("building_occupancy"),
        "unit_occupancy" => Some("unit_occupancy"),
        "walkable_region" => Some("walkable_region"),
        "temperature_1_raw" => Some("temperature_1_raw"),
        "temperature_2_raw" => Some("temperature_2_raw"),
        "position" => Some("tile_position"),
        "visibility" => Some("tile_visibility"),
        _ => None,
    }
}

fn row_truth(
    predicate: &Predicate,
    entity: &EntityRecord,
    snapshot: &WorldSnapshot,
    capture: &Capture,
    budget: &mut EvaluationBudget,
) -> Result<Truth> {
    budget.charge()?;
    Ok(match predicate {
        Predicate::Always {} => Truth::True,
        // Terrain payloads do not certify entity relationships. Never use a
        // graph edge to bypass the stricter requested-tile evidence boundary.
        Predicate::Related { .. } => Truth::Unknown,
        Predicate::Field {
            field,
            comparison,
            value,
        } => native_suffix(field)
            .and_then(|suffix| {
                observed(
                    entity,
                    field,
                    suffix,
                    capture.prefix,
                    capture.source,
                    snapshot.tick,
                )
            })
            .map_or(Truth::Unknown, |actual| compare(actual, *comparison, value)),
        Predicate::Not { arg } => row_truth(arg, entity, snapshot, capture, budget)?.not(),
        Predicate::All { args } | Predicate::Any { args } => {
            let all = matches!(predicate, Predicate::All { .. });
            let mut decisive = false;
            let mut unknown = false;
            for arg in args {
                match row_truth(arg, entity, snapshot, capture, budget)? {
                    Truth::False if all => decisive = true,
                    Truth::True if !all => decisive = true,
                    Truth::Unknown => unknown = true,
                    _ => {}
                }
            }
            if decisive {
                Truth::from_bool(!all)
            } else if unknown {
                Truth::Unknown
            } else {
                Truth::from_bool(all)
            }
        }
    })
}

fn tile_truth(
    snapshot: &WorldSnapshot,
    capture: &Capture,
    p: [u32; 3],
    predicate: &Predicate,
    budget: &mut EvaluationBudget,
) -> Result<(Truth, Option<&'static str>, bool)> {
    if (0..3).any(|i| p[i] >= capture.dimensions[i]) {
        return Ok((Truth::Unknown, Some("outside_map"), false));
    }
    if (0..3).any(|i| {
        p[i] < capture.region_min[i] || p[i] - capture.region_min[i] >= capture.region_size[i]
    }) {
        return Ok((Truth::Unknown, Some("outside_capture"), false));
    }
    let id = tile_entity_id(p)?;
    let Some(entity) = snapshot.graph.entities.get(&id) else {
        return Ok((Truth::Unknown, Some("tile_not_observed"), false));
    };
    if entity.id != id
        || entity.kind != EntityKind::TileFeature
        || entity.generation == 0
        || entity.revision == 0
    {
        return Ok((Truth::Unknown, Some("tile_identity_unestablished"), false));
    }
    let value = |field, suffix| {
        observed(
            entity,
            field,
            suffix,
            capture.prefix,
            capture.source,
            snapshot.tick,
        )
    };
    if value("position", "tile_position").and_then(coordinates) != Some(p) {
        return Ok((Truth::Unknown, Some("tile_position_unestablished"), false));
    }
    match value("visibility", "tile_visibility") {
        Some(WorldValue::Text(text)) if text == "visible" => {}
        Some(WorldValue::Text(text)) if text == "hidden" => {
            return Ok((Truth::Unknown, Some("hidden"), false));
        }
        Some(WorldValue::Text(text)) if text == "unallocated" => {
            return Ok((Truth::Unknown, Some("unallocated"), false));
        }
        _ => return Ok((Truth::Unknown, Some("visibility_unestablished"), false)),
    }
    let truth = row_truth(predicate, entity, snapshot, capture, budget)?;
    Ok((
        truth,
        (truth == Truth::Unknown).then_some("predicate_unestablished"),
        true,
    ))
}

pub(in super::super) fn evaluate(
    probe: &mut Probe,
    snapshot: &WorldSnapshot,
    areas: &[Area],
    predicate: &Predicate,
    comparison: Comparison,
    value: u64,
    budget: &mut EvaluationBudget,
) -> Result<Truth> {
    let total = validate(areas)?;
    let relations = relationships::bind(predicate, snapshot, budget)?;
    let capture = capture(snapshot);
    let mut ordered = areas.to_vec();
    ordered.sort_by_key(|a| (a.min[2], a.min[1], a.min[0], a.max[2], a.max[1], a.max[0]));
    let mut matched = 0u64;
    let mut unknown = 0u64;
    let mut visible = 0u64;
    let mut reasons = BTreeMap::<&str, u64>::new();
    let mut examples = Vec::new();
    for area in &ordered {
        for z in area.min[2]..=area.max[2] {
            for y in area.min[1]..=area.max[1] {
                for x in area.min[0]..=area.max[0] {
                    budget.charge()?;
                    let p = [x, y, z];
                    let (truth, reason, is_visible) = match &capture {
                        Some(capture) => tile_truth(snapshot, capture, p, predicate, budget)?,
                        None => (
                            Truth::Unknown,
                            Some("capture_provenance_unestablished"),
                            false,
                        ),
                    };
                    visible += u64::from(is_visible);
                    matched += u64::from(truth == Truth::True);
                    unknown += u64::from(truth == Truth::Unknown);
                    if let Some(reason) = reason {
                        *reasons.entry(reason).or_default() += 1;
                        if examples.len() < 2 {
                            examples.push(json!({"position":p,"reason":reason}));
                        }
                    }
                }
            }
        }
    }
    budget.check()?;
    let upper = matched + unknown;
    let truth = if capture.is_some() {
        interval_truth(matched, upper, comparison, value)
    } else {
        Truth::Unknown
    };
    let truth = relations.guard(truth);
    let mut fact = json!({"op":"terrain_count","policy":POLICY,"scope":"requested_disjoint_terrain_mask",
        "mask_digest":digest(&json!(ordered))?.to_string(),
        "predicate_digest":digest(&json!(predicate))?.to_string(),"snapshot_hash":snapshot.state_hash.to_string(),
        "source_digest":capture.as_ref().map(|c|c.source.to_string()),
        "requested_tiles":total,"visible_tiles":visible,"all_positions_visible":visible==total,
        "matched_min":matched,"matched_max":upper,"known_nonmatches":total-upper,"unestablished":unknown,
        "unestablished_reasons":reasons,"unestablished_examples":examples,"examples_complete":unknown<=2,
        "comparison":comparison,"threshold":value,"truth":truth.text(),
        "native_job_completion_proven":false,"mutation_cause_proven":false,"safety_proven":false});
    relations.annotate(&mut fact);
    budget.check()?;
    probe.invalid_generation |= relations.invalid_generation();
    probe.facts.push(fact);
    Ok(truth)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let condition: Value =
        serde_json::from_str(include_str!("../../../schemas/mcp_watch_terrain_v1.json"))
            .map_err(|_| invalid("embedded terrain-condition schema is invalid"))?;
    schema["$defs"]["watch_condition"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("watch condition schema variants absent"))?
        .push(condition);
    Ok(schema)
}

#[cfg(test)]
#[path = "query_watch_terrain_tests.rs"]
mod tests;
