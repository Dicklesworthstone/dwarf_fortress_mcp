//! One whole-selection shelter condition in the existing watch engine.
//! Only the trusted observing shell supplies snapshots. Source labels and
//! digests are checked for consistency here, not issued as admission grants.
use super::*;
use dfmcp_adapter::live_spatial::citizens::{MAX_COHERENT_CITIZENS, citizen_entity_id};
use dfmcp_adapter::shelter_analysis::{
    ArrivalStatus, CitizenTarget, MAX_CITIZENS, MAX_SHELTERS, ShelterSelection,
};
use dfmcp_core::{GameTick, MapCoord, MapCuboid};
use dfmcp_world::{EntityKind, EntityRecord};

pub(super) const POLICY: &str = "dfmcp.shelter-set-condition/1";
pub(super) const MAX_TARGETS: usize = MAX_CITIZENS;
pub(super) const MAX_REGIONS: usize = MAX_SHELTERS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Target {
    entity_id: String,
    generation: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Area {
    min: [u32; 3],
    max: [u32; 3],
}

fn coord(value: [u32; 3]) -> Result<MapCoord> {
    if value.iter().any(|n| *n >= 32_768) {
        return Err(invalid("shelter coordinates must be 0..32767"));
    }
    Ok(MapCoord::new(value[0] as i32, value[1] as i32, value[2] as i32))
}

/// Use the same identity, geometry and volume contract as shelter_status.
/// Canonical ordering is required, so the retained definition has one encoding.
/// Do not silently deduplicate, shorten, or replace the original named goal.
pub(super) fn validate(citizens: &[Target], shelters: &[Area]) -> Result<ShelterSelection> {
    if citizens.is_empty() || citizens.len() > MAX_CITIZENS
        || shelters.is_empty() || shelters.len() > MAX_SHELTERS
    {
        return Err(bounded("shelter set requires 1..128 citizens and 1..16 regions"));
    }
    let targets = citizens.iter().map(|t| Ok(CitizenTarget {
        entity_id: positive_id(&t.entity_id)?, generation: t.generation,
    })).collect::<Result<Vec<_>>>()?;
    let regions = shelters.iter().map(|a| MapCuboid::new(coord(a.min)?, coord(a.max)?))
        .collect::<Result<Vec<_>>>()?;
    let selection = ShelterSelection::new(&targets, &regions)?;
    if selection.citizens() != targets.as_slice() || selection.shelters() != regions.as_slice() {
        return Err(invalid("shelter watch targets and regions must be in canonical order"));
    }
    Ok(selection)
}

fn observed<'a>(
    entity: &'a EntityRecord,
    field: &str,
    native: &str,
    source: Digest32,
    tick: GameTick,
) -> Option<&'a WorldValue> {
    let fact = entity.fields.get(field)?;
    if source == Digest32::ZERO || fact.source_digest != source || fact.observed_at != tick
        || !matches!(&fact.source, FactSource::DfhackField(name) if name == native)
    {
        return None;
    }
    fact.known_value()
}

fn source(snapshot: &WorldSnapshot) -> Option<(Digest32, MapCoord)> {
    let root = snapshot.graph.entities.get(&EntityId::new(1))?;
    if root.id != EntityId::new(1) || root.kind != EntityKind::Fortress
        || root.generation == 0 || root.revision == 0
    {
        return None;
    }
    let digest = root.fields.get("observation_profile")?.source_digest;
    if !matches!(observed(root, "observation_profile", "spatial/1.8.capture", digest, snapshot.tick),
        Some(WorldValue::Text(profile)) if profile == "spatial/1.8")
        || !matches!(observed(root, "strict_citizen_count", "spatial/1.8.citizens.count", digest, snapshot.tick),
            Some(WorldValue::U64(n)) if *n <= MAX_COHERENT_CITIZENS as u64)
    {
        return None;
    }
    let WorldValue::Coord(dimensions) = observed(root, "map_dimensions",
        "spatial/1.8.map.Maps.getTileSize", digest, snapshot.tick)? else { return None; };
    if [dimensions.x, dimensions.y, dimensions.z].iter().all(|n| (1..=32_768).contains(n)) {
        Some((digest, *dimensions))
    } else {
        None
    }
}

fn bit(entity: &EntityRecord, name: &str, source: Digest32, tick: GameTick) -> Option<bool> {
    match observed(entity, name, &format!("spatial/1.8.{name}"), source, tick) {
        Some(WorldValue::Bool(value)) => Some(*value),
        _ => None,
    }
}

struct Location {
    status: ArrivalStatus,
    position: Option<MapCoord>,
    shelter_index: Option<usize>,
    reason: Option<&'static str>,
}
impl Location {
    fn unknown(reason: &'static str) -> Self {
        Self { status: ArrivalStatus::PositionUnknown, position: None, shelter_index: None, reason: Some(reason) }
    }
    fn truth(&self) -> Truth {
        match self.status {
            ArrivalStatus::Inside => Truth::True,
            ArrivalStatus::Outside | ArrivalStatus::NotAlive => Truth::False,
            _ => Truth::Unknown,
        }
    }
}

fn location(
    entity: &EntityRecord,
    target: CitizenTarget,
    source: Digest32,
    dimensions: MapCoord,
    tick: GameTick,
    shelters: &[MapCuboid],
) -> Location {
    if entity.id != target.entity_id || entity.kind != EntityKind::Unit || entity.revision == 0 {
        return Location::unknown("citizen_projection_not_bound");
    }
    let native_id = target.entity_id.get() - citizen_entity_id(0).get();
    if observed(entity, "native_unit_id", "spatial/1.8.citizen.unit_id", source, tick)
        != Some(&WorldValue::U64(native_id))
        || bit(entity, "citizen", source, tick) != Some(true)
        || bit(entity, "resident", source, tick) != Some(false)
    {
        return Location::unknown("strict_citizen_source_not_bound");
    }
    match bit(entity, "alive", source, tick) {
        Some(false) => return Location { status: ArrivalStatus::NotAlive,
            position: None, shelter_index: None, reason: None },
        None => return Location::unknown("liveness_not_observed"),
        Some(true) => {}
    }
    if bit(entity, "active", source, tick) != Some(true)
        || bit(entity, "visible", source, tick) != Some(true)
    {
        return Location::unknown("active_visible_position_not_established");
    }
    let Some(WorldValue::Coord(p)) = observed(entity, "position",
        "spatial/1.8.citizen.position", source, tick) else {
        return Location::unknown("position_not_observed");
    };
    if p.x < 0 || p.y < 0 || p.z < 0
        || p.x >= dimensions.x || p.y >= dimensions.y || p.z >= dimensions.z
    {
        return Location::unknown("position_outside_map_dimensions");
    }
    let shelter_index = shelters.iter().position(|a| a.contains(*p));
    Location { status: if shelter_index.is_some() { ArrivalStatus::Inside } else { ArrivalStatus::Outside },
        position: Some(*p), shelter_index, reason: None }
}

pub(super) fn evaluate(
    probe: &mut Probe,
    snapshot: &WorldSnapshot,
    citizens: &[Target],
    shelters: &[Area],
    budget: &mut counts::EvaluationBudget,
) -> Result<Truth> {
    budget.check()?;
    let selection = validate(citizens, shelters)?;
    for _ in 0..8 + citizens.len() + shelters.len() { budget.charge()?; }
    let binding = source(snapshot);
    let geometry_bound = binding.is_some_and(|(_, d)| selection.shelters().iter().all(|a|
        a.max.x < d.x && a.max.y < d.y && a.max.z < d.z));
    let mut rows = Vec::with_capacity(citizens.len());
    let (mut inside, mut outside, mut not_alive, mut unresolved) = (0usize, 0usize, 0usize, 0usize);
    for target in selection.citizens() {
        // Reserve all fixed field checks and region comparisons, even for an
        // unavailable row. No target bypasses identity checks after a false row.
        for _ in 0..32 + shelters.len() { budget.charge()?; }
        let entity = snapshot.graph.entities.get(&target.entity_id);
        let found = match entity {
            None => Location { status: ArrivalStatus::NotObserved, position: None, shelter_index: None, reason: None },
            Some(e) if e.generation != target.generation => {
                probe.invalid_generation = true;
                Location { status: ArrivalStatus::IncarnationChanged, position: None, shelter_index: None, reason: None }
            }
            Some(e) => match binding {
                Some((source, dimensions)) if geometry_bound =>
                    location(e, *target, source, dimensions, snapshot.tick, selection.shelters()),
                Some(_) => Location::unknown("shelter_outside_observed_map_dimensions"),
                None => Location::unknown("coherent_spatial_citizen_source_unavailable"),
            },
        };
        match found.status {
            ArrivalStatus::Inside => inside += 1,
            ArrivalStatus::Outside => outside += 1,
            ArrivalStatus::NotAlive => not_alive += 1,
            _ => unresolved += 1,
        }
        rows.push(json!({"entity_id":target.entity_id.to_string(), "generation":target.generation,
            "observed_generation":entity.map(|e| e.generation), "revision":entity.map(|e| e.revision),
            "status":found.status.as_str(), "truth":found.truth().text(), "reason":found.reason,
            "position":found.position.map(|p| [p.x, p.y, p.z]), "shelter_index":found.shelter_index}));
    }
    let truth = if outside + not_alive > 0 { Truth::False }
        else if unresolved > 0 { Truth::Unknown } else { Truth::True };
    probe.facts.push(json!({"op":"shelter_set", "policy":POLICY, "truth":truth.text(),
        "selection_digest":selection.digest().to_string(), "source_digest":binding.map(|(s, _)| s.to_string()),
        "selected":citizens.len(), "inside":inside, "outside":outside, "not_alive":not_alive,
        "unresolved":unresolved, "records":rows, "records_complete":true,
        "geometry_within_observed_map":geometry_bound,
        "complete_fortress_population_selected":false, "continuous_presence_proven":false,
        "evacuation_effect_proven":false, "shelter_safety_proven":false,
        "release_authorized":false}));
    budget.check()?;
    Ok(truth)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!("../../../schemas/mcp_watch_shelter_set_v1.json"))
        .map_err(|_| invalid("embedded shelter-set schema is invalid"))?;
    schema["$defs"]["watch_condition"]["oneOf"].as_array_mut()
        .ok_or_else(|| invalid("watch condition variants absent"))?.push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "query_watch_shelter_tests.rs"]
mod tests;
