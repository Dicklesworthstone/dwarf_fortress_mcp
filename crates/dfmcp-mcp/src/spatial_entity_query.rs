//! Source-bound spatial entity selection, with no terrain or access inference.
use std::collections::BTreeMap;
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, MapCoord, MapCuboid,
    OperationContext, Result, RiskTier,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Predicate, PredicateTruth, Value as WorldValue, WorldSnapshot,
    evaluate_truth_for,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[path = "spatial_query_source.rs"]
mod source;

const MAX_ROWS: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: Query,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    SpatialEntities {
        region: Region,
        geometry: Geometry,
        relation: Option<Relation>,
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(default)]
        fields: Vec<String>,
        #[serde(rename = "where")]
        predicate: Option<Value>,
        order: Option<Order>,
        limit: Option<usize>,
        continuation: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Region {
    min: [i32; 3],
    max: [i32; 3],
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Geometry {
    Point { field: String },
    Cuboid { min_field: String, max_field: String },
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Relation {
    #[default]
    Intersects,
    Within,
    Contains,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Order {
    #[default]
    IdAscending,
    IdDescending,
}

struct Selection {
    region: MapCuboid,
    geometry: Geometry,
    relation: Relation,
    kinds: Vec<(String, EntityKind)>,
    fields: Vec<String>,
    predicate: Option<Predicate>,
    order: Order,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn coord([x, y, z]: [i32; 3]) -> MapCoord {
    MapCoord::new(x, y, z)
}

fn coordinates(value: MapCoord) -> [i32; 3] {
    [value.x, value.y, value.z]
}

impl Geometry {
    fn fields(&self) -> Vec<String> {
        match self {
            Self::Point { field } => vec![field.clone()],
            Self::Cuboid { min_field, max_field } => vec![min_field.clone(), max_field.clone()],
        }
    }

    fn json(&self) -> Value {
        match self {
            Self::Point { field } => json!({"kind":"point","field":field}),
            Self::Cuboid { min_field, max_field } => {
                json!({"kind":"cuboid","min_field":min_field,"max_field":max_field})
            }
        }
    }

    fn read(
        &self,
        entity: &EntityRecord,
        snapshot: &WorldSnapshot,
    ) -> std::result::Result<MapCuboid, &'static str> {
        let read = |name: &str| {
            let fact = entity.fields.get(name).ok_or("missing_field")?;
            let value = fact.known_value().ok_or("unavailable_or_contradictory_field")?;
            if fact.observed_at > snapshot.tick {
                return Err("future_geometry");
            }
            match value {
                WorldValue::Coord(value) => Ok(*value),
                _ => Err("non_coordinate_field"),
            }
        };
        let (min, max) = match self {
            Self::Point { field } => {
                let point = read(field)?;
                (point, point)
            }
            Self::Cuboid { min_field, max_field } => (read(min_field)?, read(max_field)?),
        };
        MapCuboid::new(min, max).map_err(|_| "inverted_cuboid")
    }
}

impl Relation {
    fn name(self) -> &'static str {
        match self {
            Self::Intersects => "intersects",
            Self::Within => "within",
            Self::Contains => "contains",
        }
    }

    fn matches(self, geometry: MapCuboid, region: MapCuboid) -> bool {
        match self {
            Self::Within => region.contains_cuboid(geometry),
            Self::Contains => geometry.contains_cuboid(region),
            Self::Intersects => {
                geometry.min.x <= region.max.x && geometry.max.x >= region.min.x
                    && geometry.min.y <= region.max.y && geometry.max.y >= region.min.y
                    && geometry.min.z <= region.max.z && geometry.max.z >= region.min.z
            }
        }
    }
}

fn identity(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    selection: &Selection,
) -> Digest32 {
    fn string(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    let mut bytes = b"dfmcp-spatial-entity-query-v1\0".to_vec();
    bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&snapshot.tick.0.to_be_bytes());
    bytes.extend_from_slice(snapshot.state_hash.as_bytes());
    for point in [selection.region.min, selection.region.max] {
        for dimension in coordinates(point) {
            bytes.extend_from_slice(&dimension.to_be_bytes());
        }
    }
    bytes.push(match &selection.geometry {
        Geometry::Point { .. } => 0,
        Geometry::Cuboid { .. } => 1,
    });
    for field in selection.geometry.fields() {
        string(&mut bytes, &field);
    }
    string(&mut bytes, selection.relation.name());
    bytes.extend_from_slice(&(selection.kinds.len() as u64).to_be_bytes());
    for (name, _) in &selection.kinds {
        string(&mut bytes, name);
    }
    bytes.extend_from_slice(&(selection.fields.len() as u64).to_be_bytes());
    for name in &selection.fields {
        string(&mut bytes, name);
    }
    match &selection.predicate {
        Some(predicate) => {
            bytes.push(1);
            let encoded = predicate.canonical_bytes();
            bytes.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
            bytes.extend_from_slice(&encoded);
        }
        None => bytes.push(0),
    }
    bytes.push(match selection.order {
        Order::IdAscending => 0,
        Order::IdDescending => 1,
    });
    Digest32::of_bytes(&bytes)
}

type Position = (EntityId, u32);

fn cursor(identity: Digest32, position: Position) -> String {
    let body = format!("sq1:{identity}:{}:{}", position.0, position.1);
    format!("{body}:{}", Digest32::of_bytes(body.as_bytes()))
}

fn continuation(raw: Option<&str>, identity: Digest32) -> Result<Option<Position>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 256 {
        return Err(exhausted("spatial continuation exceeds 256 bytes"));
    }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len() != 5 || parts[0] != "sq1" {
        return Err(invalid("invalid spatial continuation"));
    }
    let id = parts[2].parse::<u64>().map_err(|_| invalid("invalid spatial cursor entity"))?;
    let generation = parts[3].parse::<u32>()
        .map_err(|_| invalid("invalid spatial cursor generation"))?;
    if id == 0 {
        return Err(invalid("spatial cursor entity zero is reserved"));
    }
    let position = (EntityId::new(id), generation);
    if raw != cursor(identity, position) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "spatial continuation changed or belongs to another source, session, or selection",
        ));
    }
    Ok(Some(position))
}

/// Execute against one published source; callers own source retention and the
/// final Agent Turn budget. No selected geometry grants effect authority.
pub(super) fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    let started = Instant::now();
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if context.anchor != snapshot.anchor() {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "spatial query context does not name its source",
        ));
    }
    super::core_query::validate_input(input)?;
    let request: Envelope = serde_json::from_value(input.clone())
        .map_err(|error| invalid(format!("invalid spatial query: {error}")))?;
    if request.schema != "dfmcp.query/1" {
        return Err(invalid("query schema must be dfmcp.query/1"));
    }
    if request.expected_anchor.as_ref().is_some_and(|anchor| {
        *anchor != super::core_query::anchor_json(snapshot.anchor())
    }) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "expected_anchor does not match the complete spatial source",
        ));
    }
    let Query::SpatialEntities {
        region, geometry, relation, kinds, fields, predicate, order, limit,
        continuation: raw_cursor,
    } = request.query;
    super::core_query::bounded_names(geometry.fields(), 2)?;
    if matches!(&geometry, Geometry::Cuboid { min_field, max_field } if min_field == max_field) {
        return Err(invalid("cuboid geometry requires two distinct coordinate fields"));
    }
    let selection = Selection {
        region: MapCuboid::new(coord(region.min), coord(region.max))?,
        geometry,
        relation: relation.unwrap_or_default(),
        kinds: super::core_query::bounded_names(kinds, 32)?.into_iter()
            .map(|name| Ok((name.clone(), super::core_query::entity_kind(&name)?)))
            .collect::<Result<_>>()?,
        fields: super::core_query::bounded_names(fields, 32)?,
        predicate: predicate.map(super::core_query::decode_filter).transpose()?,
        order: order.unwrap_or_default(),
    };
    let limit = limit.unwrap_or(4);
    if limit == 0 || limit > MAX_ROWS.min(context.budget.max_entities as usize) {
        return Err(invalid("spatial page limit exceeds the negotiated 1..128 row allowance"));
    }
    let mut work = source::preflight(snapshot, context, started)?;
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "spatial-query source hash is invalid",
        ));
    }
    let mut predicate_nodes = 0usize;
    let mut pending = selection.predicate.iter().collect::<Vec<_>>();
    while let Some(predicate) = pending.pop() {
        predicate_nodes += 1;
        match predicate {
            Predicate::All(children) | Predicate::Any(children) => pending.extend(children),
            Predicate::Not(child) => pending.push(child),
            _ => {}
        }
    }
    work.charge(
        0,
        snapshot.graph.entities.len().saturating_mul(
            predicate_nodes.saturating_add(selection.fields.len())
                .saturating_add(selection.kinds.len()).saturating_add(3),
        ),
    )?;
    let digest = identity(snapshot, context, &selection);
    let after = continuation(raw_cursor.as_deref(), digest)?;
    let mut after_found = after.is_none();
    let (mut selected, mut matched, mut outside, mut filter_false, mut filter_unknown, mut remaining) =
        (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut unknown = BTreeMap::<&str, usize>::new();
    let mut page = Vec::new();
    let ordered: Box<dyn Iterator<Item = &EntityRecord> + '_> = match selection.order {
        Order::IdAscending => Box::new(snapshot.graph.entities.values()),
        Order::IdDescending => Box::new(snapshot.graph.entities.values().rev()),
    };
    for entity in ordered {
        work.check()?;
        if !selection.kinds.is_empty()
            && !selection.kinds.iter().any(|(_, kind)| kind.as_str() == entity.kind.as_str())
        {
            continue;
        }
        selected += 1;
        let geometry = match selection.geometry.read(entity, snapshot) {
            Ok(geometry) => geometry,
            Err(reason) => {
                *unknown.entry(reason).or_default() += 1;
                continue;
            }
        };
        if !selection.relation.matches(geometry, selection.region) {
            outside += 1;
            continue;
        }
        match selection.predicate.as_ref().map_or(PredicateTruth::True, |predicate| {
            evaluate_truth_for(snapshot, entity.id, predicate)
        }) {
            PredicateTruth::True => {}
            PredicateTruth::False => {
                filter_false += 1;
                continue;
            }
            PredicateTruth::Unknown => {
                filter_unknown += 1;
                continue;
            }
        }
        matched += 1;
        if after == Some((entity.id, entity.generation)) {
            after_found = true;
        }
        if after.is_some_and(|(id, _)| match selection.order {
            Order::IdAscending => entity.id <= id,
            Order::IdDescending => entity.id >= id,
        }) {
            continue;
        }
        remaining += 1;
        if page.len() < limit {
            page.push((entity, geometry));
        }
    }
    if !after_found || (after.is_some() && remaining == 0) {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "spatial continuation does not name a nonfinal hit in this source",
        ));
    }
    let maximum = usize::try_from(
        context.budget.max_bytes.min(
            u64::from(context.budget.max_output_tokens).saturating_mul(4),
        ),
    ).map_err(|_| exhausted("spatial response budget cannot be represented"))?;
    let unknown_count: usize = unknown.values().sum();
    let mut payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"spatial_entities",
        "anchor":super::core_query::anchor_json(snapshot.anchor()),
        "query_digest":digest.to_string(),
        "region":{"min":coordinates(selection.region.min),"max":coordinates(selection.region.max)},
        "geometry":selection.geometry.json(), "relation":selection.relation.name(),
        "ordering":match selection.order {
            Order::IdAscending => "entity_id_ascending",
            Order::IdDescending => "entity_id_descending"
        },
        "matched":matched, "returned":0, "rows":[], "truncated":false, "continuation":null,
        "selection":{
            "kind_selected":selected, "outside_relation":outside, "unknown_geometry":unknown_count,
            "unknown_geometry_reasons":unknown, "excluded_filter_false":filter_false,
            "excluded_filter_unknown":filter_unknown
        },
        "coverage":{
            "domain":"named_known_geometry_in_canonical_projection",
            "selection_complete_within_projection":unknown_count == 0 && filter_unknown == 0,
            "absence_proven":false, "eligible_as_mutation_evidence":false,
            "terrain_evaluated":false, "accessibility_evaluated":false,
            "note":"Inclusive coordinate comparisons over explicit fields; unknown geometry is not outside the region. Sources and freshness remain those of the named facts; this is not a complete occupancy or safe-placement proof."
        },
        "source_cost":{"bounded_bytes":work.bytes,"estimated_scan_work_units":work.work}
    });
    let geometry_fields = selection.geometry.fields();
    for (entity, geometry) in page {
        if !source::render_fits(entity, &selection.fields, &geometry_fields, maximum, &mut work)? {
            break;
        }
        let mut row = super::core_query::entity_json(entity, &selection.fields);
        row["geometry"] = json!({
            "min":coordinates(geometry.min), "max":coordinates(geometry.max),
            "facts":super::core_query::entity_json(entity, &geometry_fields)["fields"],
            "eligible_as_mutation_evidence":false
        });
        let mut candidate = payload.clone();
        let rows = candidate["rows"].as_array_mut()
            .ok_or_else(|| invalid("spatial result rows are unavailable"))?;
        rows.push(row);
        let count = rows.len();
        candidate["returned"] = json!(count);
        candidate["truncated"] = json!(count < remaining);
        candidate["continuation"] = if count < remaining {
            json!(cursor(digest, (entity.id, entity.generation)))
        } else {
            Value::Null
        };
        if serde_json::to_vec(&candidate)
            .map_err(|_| invalid("spatial result cannot be encoded"))?.len() > maximum
        {
            break;
        }
        payload = candidate;
        work.check()?;
    }
    if (remaining > 0 && payload["returned"] == 0) || payload.to_string().len() > maximum {
        return Err(exhausted(
            "one complete spatial row and its envelope cannot fit; narrow fields or increase the response budget",
        ));
    }
    work.check()?;
    Ok(payload)
}

#[cfg(test)]
#[path = "spatial_entity_query_tests.rs"]
mod tests;
