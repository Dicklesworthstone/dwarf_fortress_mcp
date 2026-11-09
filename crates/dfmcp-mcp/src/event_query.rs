//! Bounded event windows over one authorized, immutable canonical projection.
//! A retained event is not proof of a complete history or of native causality.

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, EventId, OperationContext, Result,
    RiskTier,
};
use dfmcp_world::{Value as WorldValue, WorldEvent, WorldSnapshot};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_INPUT_NODES: usize = 4096;
const MAX_INPUT_BYTES: usize = 65_536;
const MAX_SCAN_EVENTS: usize = 100_000;
const MAX_SCAN_BYTES: usize = 16 * 1024 * 1024;
const MAX_WORK: usize = 4_000_000;
const MAX_ROWS: usize = 128;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    schema: String,
    expected_anchor: Option<Value>,
    query: Query,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    Events {
        from_tick: Option<u64>,
        through_tick: Option<u64>,
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(default)]
        subjects: Vec<String>,
        #[serde(default)]
        fields: Vec<String>,
        order: Option<Order>,
        limit: Option<usize>,
        continuation: Option<String>,
    },
}

#[derive(Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Order {
    #[default]
    TickAscending,
    TickDescending,
}

struct Selection {
    from_tick: u64,
    through_tick: u64,
    kinds: BTreeSet<String>,
    subjects: BTreeSet<EntityId>,
    fields: BTreeSet<String>,
    order: Order,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}

/// Check the JSON tree before cloning or recursive typed deserialization.
fn validate_input(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)];
    let (mut nodes, mut bytes) = (0usize, 0usize);
    while let Some((value, depth)) = pending.pop() {
        nodes = nodes.saturating_add(1);
        bytes = bytes.saturating_add(32);
        if nodes > MAX_INPUT_NODES || depth > 24 {
            return Err(exhausted("event query input shape exceeds its bound"));
        }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if values
                    .len()
                    .saturating_add(nodes)
                    .saturating_add(pending.len())
                    > MAX_INPUT_NODES
                {
                    return Err(exhausted("event query input node count exceeds its bound"));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values
                    .len()
                    .saturating_add(nodes)
                    .saturating_add(pending.len())
                    > MAX_INPUT_NODES
                {
                    return Err(exhausted("event query input node count exceeds its bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
        if bytes > MAX_INPUT_BYTES {
            return Err(exhausted("event query input bytes exceed their bound"));
        }
    }
    Ok(())
}

fn names(values: Vec<String>, maximum: usize) -> Result<BTreeSet<String>> {
    if values.len() > maximum
        || values
            .iter()
            .any(|value| value.is_empty() || value.len() > 128 || value.contains('\0'))
    {
        return Err(invalid("event selector exceeds its name or count bound"));
    }
    Ok(values.into_iter().collect())
}

fn subjects(values: Vec<String>) -> Result<BTreeSet<EntityId>> {
    if values.len() > 64 {
        return Err(invalid("event query accepts at most 64 subjects"));
    }
    values
        .into_iter()
        .map(|text| {
            let id = text
                .parse::<u64>()
                .map_err(|_| invalid("event subjects must be canonical nonzero decimal u64 IDs"))?;
            if id == 0 || text.len() > 20 || text != id.to_string() {
                return Err(invalid(
                    "event subjects must be canonical nonzero decimal u64 IDs",
                ));
            }
            Ok(EntityId::new(id))
        })
        .collect()
}

/// The versioned identity uses framed canonical fields, never JSON map order.
/// Width is deliberately excluded, so the caller may narrow the next page.
fn identity(snapshot: &WorldSnapshot, context: &OperationContext, spec: &Selection) -> Digest32 {
    fn strings(bytes: &mut Vec<u8>, values: &BTreeSet<String>) {
        bytes.extend_from_slice(&(values.len() as u64).to_be_bytes());
        for value in values {
            bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
    }
    let mut bytes = b"dfmcp-event-query-v1\0".to_vec();
    bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&snapshot.tick.0.to_be_bytes());
    bytes.extend_from_slice(snapshot.state_hash.as_bytes());
    bytes.extend_from_slice(&spec.from_tick.to_be_bytes());
    bytes.extend_from_slice(&spec.through_tick.to_be_bytes());
    strings(&mut bytes, &spec.kinds);
    bytes.extend_from_slice(&(spec.subjects.len() as u64).to_be_bytes());
    for subject in &spec.subjects {
        bytes.extend_from_slice(&subject.get().to_be_bytes());
    }
    strings(&mut bytes, &spec.fields);
    bytes.push(match spec.order {
        Order::TickAscending => 0,
        Order::TickDescending => 1,
    });
    Digest32::of_bytes(&bytes)
}

type Position = (u64, EventId);

fn cursor(identity: Digest32, position: Position) -> String {
    let body = format!("ev1:{identity}:{}:{}", position.0, position.1);
    format!("{body}:{}", Digest32::of_bytes(body.as_bytes()))
}

fn parse_cursor(raw: Option<&str>, identity: Digest32) -> Result<Option<Position>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 256 {
        return Err(exhausted("event continuation exceeds its byte bound"));
    }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len() != 5 || parts[0] != "ev1" || parts[3].len() != 32 {
        return Err(invalid("invalid event continuation"));
    }
    let tick = parts[2]
        .parse::<u64>()
        .map_err(|_| invalid("invalid event continuation tick"))?;
    let id = u128::from_str_radix(parts[3], 16)
        .map_err(|_| invalid("invalid event continuation identity"))?;
    if id == 0 {
        return Err(invalid("event ID zero is reserved"));
    }
    let position = (tick, EventId::new(id));
    if raw != cursor(identity, position) {
        return Err(stale(
            "event continuation changed or belongs to another selection, session, or complete anchor",
        ));
    }
    Ok(Some(position))
}

/// Bound the entire retained event domain before hashing or projecting it.
/// Excluded records still cost work; a narrow selector cannot hide an oversized source.
fn preflight_events(snapshot: &WorldSnapshot) -> Result<(usize, usize)> {
    let (mut bytes, mut work) = (0usize, 0usize);
    for (id, event) in &snapshot.graph.events {
        if *id != event.id || *id == EventId::NIL || event.tick > snapshot.tick {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "retained event identity or tick is inconsistent with its canonical snapshot",
            ));
        }
        bytes = bytes
            .saturating_add(64)
            .saturating_add(event.kind.as_str().len())
            .saturating_add(event.summary.len());
        work = work.saturating_add(1);
        // Frames are bounded before allocating a traversal frontier.
        if event.fields.len().saturating_add(work) > MAX_WORK {
            return Err(exhausted(
                "retained event fields exceed the scan work bound",
            ));
        }
        let mut pending = Vec::new();
        for (name, value) in &event.fields {
            bytes = bytes.saturating_add(name.len());
            pending.push((value, 0usize));
        }
        while let Some((value, depth)) = pending.pop() {
            work = work.saturating_add(1);
            bytes = bytes.saturating_add(32);
            if work > MAX_WORK || depth > 64 {
                return Err(exhausted(
                    "retained event values exceed their scan shape bound",
                ));
            }
            match value {
                WorldValue::Text(text) => bytes = bytes.saturating_add(text.len()),
                WorldValue::Bytes(value) => bytes = bytes.saturating_add(value.len()),
                WorldValue::List(values) => {
                    if values
                        .len()
                        .saturating_add(work)
                        .saturating_add(pending.len())
                        > MAX_WORK
                    {
                        return Err(exhausted(
                            "retained event values exceed the scan work bound",
                        ));
                    }
                    pending.extend(values.iter().map(|value| (value, depth + 1)));
                }
                WorldValue::Object(values) => {
                    if values
                        .len()
                        .saturating_add(work)
                        .saturating_add(pending.len())
                        > MAX_WORK
                    {
                        return Err(exhausted(
                            "retained event values exceed the scan work bound",
                        ));
                    }
                    for (key, value) in values {
                        bytes = bytes.saturating_add(key.len());
                        pending.push((value, depth + 1));
                    }
                }
                _ => {}
            }
            if bytes > MAX_SCAN_BYTES {
                return Err(exhausted(
                    "retained event values exceed the scan byte bound",
                ));
            }
        }
        if bytes > MAX_SCAN_BYTES || work > MAX_WORK {
            return Err(exhausted("retained events exceed the scan bound"));
        }
    }
    Ok((bytes, work))
}

/// Refuse certainly oversized values before materializing their JSON objects.
/// Fifteen bytes is the smallest typed value (`{"type":"null"}`); string and
/// binary payloads add a lower bound without assuming their escaping cost.
fn row_fits_minimum(event: &WorldEvent, fields: &BTreeSet<String>, maximum: usize) -> bool {
    let mut bytes = event
        .summary
        .len()
        .saturating_add(event.kind.as_str().len());
    let mut pending = Vec::new();
    for name in fields {
        bytes = bytes.saturating_add(name.len());
        if let Some(value) = event.fields.get(name) {
            pending.push(value);
        }
    }
    while let Some(value) = pending.pop() {
        bytes = bytes.saturating_add(15);
        match value {
            WorldValue::Text(value) => bytes = bytes.saturating_add(value.len()),
            WorldValue::Bytes(value) => {
                bytes = bytes.saturating_add(value.len().saturating_mul(2).saturating_sub(1));
            }
            WorldValue::List(values) => pending.extend(values),
            WorldValue::Object(values) => {
                for (name, value) in values {
                    bytes = bytes.saturating_add(name.len());
                    pending.push(value);
                }
            }
            _ => {}
        }
        if bytes > maximum {
            return false;
        }
    }
    bytes <= maximum
}

fn row(event: &WorldEvent, fields: &BTreeSet<String>) -> Value {
    let fields: BTreeMap<_, _> = fields
        .iter()
        .map(|name| {
            let projected = match event.fields.get(name) {
                Some(value) => json!({"presence":"known", "epistemic_state":"assumed",
                    "value":super::core_query::world_value(value)}),
                None => json!({"presence":"unknown", "epistemic_state":"unknown", "value":null,
                    "reason":"field is not present in the retained canonical record"}),
            };
            (name.clone(), projected)
        })
        .collect();
    json!({"event_id":event.id.to_string(), "tick":event.tick.0,
        "kind":event.kind.as_str(), "subject":event.subject.map(|id| id.to_string()),
        "summary":event.summary, "fields":fields, "epistemic_state":"assumed"})
}

fn page(
    base: &Value,
    rows: &[Value],
    source: &[&WorldEvent],
    start: usize,
    identity: Digest32,
) -> Value {
    let end = start + rows.len();
    let truncated = end < source.len();
    let continuation = if truncated {
        end.checked_sub(1)
            .and_then(|offset| source.get(offset))
            .map(|event| cursor(identity, (event.tick.0, event.id)))
    } else {
        None
    };
    let mut result = base.clone();
    result["rows"] = json!(rows);
    result["returned"] = json!(rows.len());
    result["truncated"] = json!(truncated);
    result["continuation"] = json!(continuation);
    result["coverage"]["page_complete"] = json!(start == 0 && end == source.len());
    result
}

/// Execute only against the enclosing session's published canonical source.
/// Current authority must be checked before a caller selects a historical source.
pub(super) fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    execute_with_page_budget(snapshot, context, input, context.budget.max_bytes, &|_| {
        Ok(true)
    })
}

/// Fit retained events with their complete caller-owned packet. The callback
/// checks size only; source admission, identity and authority remain unchanged.
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
        return Err(stale(
            "event query context does not name the source snapshot",
        ));
    }
    validate_input(input)?;
    let request: Request = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid closed event-window query"))?;
    if request.schema != "dfmcp.query/1" {
        return Err(invalid("event query schema must be dfmcp.query/1"));
    }
    let anchor = super::core_query::anchor_json(snapshot.anchor());
    if request
        .expected_anchor
        .as_ref()
        .is_some_and(|value| *value != anchor)
    {
        return Err(stale(
            "expected_anchor does not match the complete event source anchor",
        ));
    }
    let Query::Events {
        from_tick,
        through_tick,
        kinds,
        subjects: selected_subjects,
        fields,
        order,
        limit,
        continuation,
    } = request.query;
    let spec = Selection {
        from_tick: from_tick.unwrap_or(0),
        through_tick: through_tick.unwrap_or(snapshot.tick.0),
        kinds: names(kinds, 32)?,
        subjects: subjects(selected_subjects)?,
        fields: names(fields, 32)?,
        order: order.unwrap_or_default(),
    };
    if spec.from_tick > spec.through_tick || spec.through_tick > snapshot.tick.0 {
        return Err(invalid(
            "event window must be ordered and end at or before the source tick",
        ));
    }
    let limit = limit.unwrap_or(4);
    if limit == 0 || limit > MAX_ROWS {
        return Err(invalid("event page limit must be between 1 and 128"));
    }
    if snapshot.graph.events.len() > (context.budget.max_entities as usize).min(MAX_SCAN_EVENTS) {
        return Err(exhausted(
            "event query exceeds its retained-record scan budget",
        ));
    }
    let source_records = snapshot
        .graph
        .entities
        .len()
        .saturating_add(snapshot.graph.edges.len())
        .saturating_add(snapshot.graph.chunks.len())
        .saturating_add(snapshot.graph.events.len());
    if source_records > context.budget.max_entities as usize {
        return Err(exhausted(
            "event query exceeds its canonical source record budget",
        ));
    }
    let (scan_bytes, scan_work) = preflight_events(snapshot)?;
    let source_bytes = snapshot.canonical_bytes();
    // Live presentation narrows max_bytes to the remaining response allowance.
    // Its already-published source has its own adapter bounds; the lab wrapper
    // separately charges source bytes against the original session work budget.
    if source_bytes.len() > dfmcp_world::canonical_decode::MAX_CANONICAL_SNAPSHOT_BYTES {
        return Err(exhausted(
            "event query exceeds the canonical snapshot byte bound",
        ));
    }
    if Digest32::of_bytes(&source_bytes) != snapshot.state_hash {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "event query canonical snapshot hash is invalid",
        ));
    }
    let identity = identity(snapshot, context, &spec);
    let after = parse_cursor(continuation.as_deref(), identity)?;
    let maximum = usize::try_from(
        context
            .budget
            .max_bytes
            .min(max_output_bytes)
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| exhausted("event query byte bound cannot be represented"))?;
    let mut matching: Vec<_> = snapshot
        .graph
        .events
        .values()
        .filter(|event| {
            event.tick.0 >= spec.from_tick
                && event.tick.0 <= spec.through_tick
                && (spec.kinds.is_empty() || spec.kinds.contains(event.kind.as_str()))
                && (spec.subjects.is_empty()
                    || event.subject.is_some_and(|id| spec.subjects.contains(&id)))
        })
        .collect();
    matching.sort_by_key(|event| (event.tick, event.id));
    if spec.order == Order::TickDescending {
        matching.reverse();
    }
    let start = match after {
        None => 0,
        Some(position) => matching
            .iter()
            .position(|event| (event.tick.0, event.id) == position)
            .map(|offset| offset + 1)
            .filter(|offset| *offset < matching.len())
            .ok_or_else(|| stale("event continuation does not name a resumable selected record"))?,
    };
    let mut base = json!({"schema":"dfmcp.query.result/1", "kind":"events", "anchor":anchor,
        "query_digest":identity.to_string(), "matched":matching.len(),
        "order":if spec.order == Order::TickAscending {"tick_ascending"} else {"tick_descending"},
        "provenance":{"kind":"canonical_retained_events", "source_anchor":anchor,
            "record_identity":"event_id", "source_authenticity":"not_established",
            "field_provenance":"not_recorded_by_the_canonical_event_type", "taint":"untrusted_source_text"},
        "coverage":{"domain":"retained_event_projection", "from_tick":spec.from_tick,
            "through_tick":spec.through_tick, "kinds":spec.kinds,
            "subjects":spec.subjects.iter().map(ToString::to_string).collect::<Vec<_>>(), "fields":spec.fields,
            "retained_records":snapshot.graph.events.len(),
            "retained_min_tick":snapshot.graph.events.values().map(|event| event.tick.0).min(),
            "retained_max_tick":snapshot.graph.events.values().map(|event| event.tick.0).max(),
            "selection_complete_within_projection":true, "history_complete":false,
            "absence_proven":false, "eligible_as_mutation_evidence":false,
            "note":"retained extrema do not prove a continuous or complete history; missing events and source authenticity remain unknown"},
        "work":{"scanned_events":snapshot.graph.events.len(), "event_bytes":scan_bytes,
            "event_value_visits":scan_work, "source_records":source_records,
            "canonical_source_bytes":source_bytes.len()}});
    if let Some(history) = dfmcp_lab::event_history::coverage(snapshot)? {
        base["coverage"]["producer"] = json!({
            "model": dfmcp_lab::event_history::MODEL,
            "basis": "validated_canonical_model_structure",
            "source_authenticity": "not_established",
            "observation_boundary": "atomic_laboratory_publication",
            "intermediate_states_observed": false,
            "edge_and_terrain_details": "changed_record_counts_only",
            "retention_limit": dfmcp_lab::event_history::MAX_RETAINED_EVENTS,
            "entity_events_per_publication_limit": dfmcp_lab::event_history::MAX_ENTITY_EVENTS_PER_PUBLICATION,
            "changed_fields_per_entity_limit": dfmcp_lab::event_history::MAX_CHANGED_FIELDS,
            "captured_value_byte_limit": dfmcp_lab::event_history::MAX_CAPTURED_VALUE_BYTES,
            "published_events_on_branch": history.total_events,
            "evicted_events_on_branch": history.evicted_events,
            "omitted_entity_changes_on_branch": history.omitted_entity_changes,
            "omitted_fields_on_branch": history.omitted_fields,
            "omitted_values_on_branch": history.omitted_values,
            "oldest_retained_ordinal": history.oldest_ordinal,
            "newest_retained_ordinal": history.newest_ordinal,
            "note": "bounded endpoint observations; restore retains the checkpoint branch; counters do not prove complete history or confer mutation authority"
        });
    }
    let output = super::core_query::PageBudget {
        maximum_bytes: maximum,
        fits,
    };
    let reaches_terminal = matching.len().saturating_sub(start) <= limit;
    let mut rows = Vec::new();
    let mut accepted = None;
    for event in matching.iter().skip(start).take(limit) {
        if !row_fits_minimum(event, &spec.fields, maximum) {
            break;
        }
        rows.push(row(event, &spec.fields));
        let candidate = page(&base, &rows, &matching, start, identity);
        if output.accepts(&candidate)? {
            accepted = Some(candidate);
        } else if !output.terminal_may_fit(&candidate, reaches_terminal)? {
            break;
        }
        if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
            return Err(exhausted("event query exceeded its wall-time budget"));
        }
    }
    let result = accepted.or_else(|| {
        (start == matching.len()).then(|| page(&base, &[], &matching, start, identity))
    }).ok_or_else(|| exhausted(
        "one complete event row and its envelope exceed the output budget; select fewer fields or increase the response budget",
    ))?;
    if !output.accepts(&result)? {
        return Err(exhausted("event query metadata exceeds the output budget"));
    }
    // Publication repeats the immutable context's cancellation/authority gate
    // after rendering; historical callers already checked their current tick.
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(exhausted("event query exceeded its wall-time budget"));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "event_query_tests.rs"]
mod tests;
