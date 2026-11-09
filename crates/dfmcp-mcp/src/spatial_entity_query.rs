//! Source-bound spatial entity selection, with no terrain or access inference.
use std::collections::BTreeMap;
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
use dfmcp_world::{
    EntityKind, EntityRecord, Predicate, PredicateTruth, WorldSnapshot, evaluate_truth_for,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[path = "spatial_query_source.rs"]
mod source;
#[path = "spatial_selector.rs"]
mod spatial_selector;

use spatial_selector::{Decision, Geometry, Region, Relation};

#[cfg(test)]
use dfmcp_world::Value as WorldValue;
#[cfg(test)]
use spatial_selector::coord;

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

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Order {
    #[default]
    IdAscending,
    IdDescending,
}

struct Selection {
    spatial: spatial_selector::Selection,
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
    selection.spatial.append_identity(&mut bytes);
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
    let id = parts[2]
        .parse::<u64>()
        .map_err(|_| invalid("invalid spatial cursor entity"))?;
    let generation = parts[3]
        .parse::<u32>()
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
    execute_with_output_budget(snapshot, context, input, context.budget.max_bytes)
}

/// Reserve presentation bytes without reducing the authorized source allowance.
/// The separate cap narrows only output; all source, authority and work checks
/// continue to use the original request context.
pub(super) fn execute_with_output_budget(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
    max_output_bytes: u64,
) -> Result<Value> {
    execute_with_page_budget(snapshot, context, input, max_output_bytes, &|_| Ok(true))
}

/// Select complete rows against a pure caller-owned packet-size check. The
/// check cannot publish, change authority, or shrink the source-work budget.
pub(super) fn execute_with_page_budget(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
    max_output_bytes: u64,
    fits: &dyn Fn(&Value) -> Result<bool>,
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
    if request
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| *anchor != super::core_query::anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "expected_anchor does not match the complete spatial source",
        ));
    }
    let Query::SpatialEntities {
        region,
        geometry,
        relation,
        kinds,
        fields,
        predicate,
        order,
        limit,
        continuation: raw_cursor,
    } = request.query;
    let selection = Selection {
        spatial: spatial_selector::Request {
            region,
            geometry,
            relation,
        }
        .normalize()?,
        kinds: super::core_query::bounded_names(kinds, 32)?
            .into_iter()
            .map(|name| Ok((name.clone(), super::core_query::entity_kind(&name)?)))
            .collect::<Result<_>>()?,
        fields: super::core_query::bounded_names(fields, 32)?,
        predicate: predicate
            .map(super::core_query::decode_filter)
            .transpose()?,
        order: order.unwrap_or_default(),
    };
    let limit = limit.unwrap_or(4);
    if limit == 0 || limit > MAX_ROWS.min(context.budget.max_entities as usize) {
        return Err(invalid(
            "spatial page limit exceeds the negotiated 1..128 row allowance",
        ));
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
            predicate_nodes
                .saturating_add(selection.fields.len())
                .saturating_add(selection.kinds.len())
                .saturating_add(selection.spatial.work_units_per_entity()),
        ),
    )?;
    let digest = identity(snapshot, context, &selection);
    let after = continuation(raw_cursor.as_deref(), digest)?;
    let mut after_found = after.is_none();
    let (
        mut selected,
        mut matched,
        mut outside,
        mut filter_false,
        mut filter_unknown,
        mut remaining,
    ) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    let mut unknown = BTreeMap::<&str, usize>::new();
    let mut page = Vec::new();
    let ordered: Box<dyn Iterator<Item = &EntityRecord> + '_> = match selection.order {
        Order::IdAscending => Box::new(snapshot.graph.entities.values()),
        Order::IdDescending => Box::new(snapshot.graph.entities.values().rev()),
    };
    for entity in ordered {
        work.check()?;
        if !selection.kinds.is_empty()
            && !selection
                .kinds
                .iter()
                .any(|(_, kind)| kind.as_str() == entity.kind.as_str())
        {
            continue;
        }
        selected += 1;
        let geometry = match selection
            .spatial
            .evaluate(entity, snapshot, &mut || work.check())?
        {
            Decision::Matched(geometry) => geometry,
            Decision::Unknown(reason) => {
                *unknown.entry(reason).or_default() += 1;
                continue;
            }
            Decision::Outside => {
                outside += 1;
                continue;
            }
        };
        match selection
            .predicate
            .as_ref()
            .map_or(PredicateTruth::True, |predicate| {
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
        context
            .budget
            .max_bytes
            .min(max_output_bytes)
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| exhausted("spatial response budget cannot be represented"))?;
    let unknown_count: usize = unknown.values().sum();
    let output = super::core_query::PageBudget {
        maximum_bytes: maximum,
        fits,
    };
    let reaches_terminal = page.len() == remaining;
    let mut accepted = None;
    let spatial = selection.spatial.json();
    let mut payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"spatial_entities",
        "anchor":super::core_query::anchor_json(snapshot.anchor()),
        "query_digest":digest.to_string(),
        "region":spatial["region"],
        "geometry":spatial["geometry"], "relation":spatial["relation"],
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
    let geometry_fields = selection.spatial.geometry_fields();
    for (entity, geometry) in page {
        if !source::render_fits(
            entity,
            &selection.fields,
            &geometry_fields,
            maximum,
            &mut work,
        )? {
            break;
        }
        let mut row = super::core_query::entity_json(entity, &selection.fields);
        row["geometry"] = selection.spatial.witness(entity, geometry);
        let rows = payload["rows"]
            .as_array_mut()
            .ok_or_else(|| invalid("spatial result rows are unavailable"))?;
        rows.push(row);
        let count = rows.len();
        payload["returned"] = json!(count);
        payload["truncated"] = json!(count < remaining);
        payload["continuation"] = if count < remaining {
            json!(cursor(digest, (entity.id, entity.generation)))
        } else {
            Value::Null
        };
        if output.accepts(&payload)? {
            accepted = Some(payload.clone());
        } else if !output.terminal_may_fit(&payload, reaches_terminal)? {
            break;
        }
        work.check()?;
    }
    let payload = accepted.or_else(|| (remaining == 0).then_some(payload))
        .ok_or_else(|| exhausted(
            "one complete spatial row and its envelope cannot fit; narrow fields or increase the response budget",
        ))?;
    if !output.accepts(&payload)? {
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
