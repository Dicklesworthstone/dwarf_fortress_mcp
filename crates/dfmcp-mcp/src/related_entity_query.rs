//! Positive, witnessed relationship selection over one exact canonical source.
//! Missing relations never become negative evidence about the complete world.

#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Instant;

#[cfg(test)]
use dfmcp_core::EdgeId;
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
#[cfg(test)]
use dfmcp_world::{EdgeRecord, EntityRecord};
use dfmcp_world::{Predicate, WorldSnapshot};
use serde::Deserialize;
use serde_json::{Value, json};

use super::core_query;

#[path = "spatial_query_source.rs"]
#[allow(clippy::duplicate_mod)]
mod source;

#[path = "relation_selector.rs"]
#[allow(clippy::duplicate_mod)]
mod relation_selector;

use relation_selector::{Candidate, kinds};

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
    RelatedEntities {
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(rename = "where")]
        predicate: Option<Value>,
        relation: relation_selector::Request,
        #[serde(default)]
        fields: Vec<String>,
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
    kinds: BTreeSet<String>,
    predicate: Option<Predicate>,
    relation: relation_selector::Selection,
    fields: Vec<String>,
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
    fn predicate(bytes: &mut Vec<u8>, value: Option<&Predicate>) {
        match value {
            Some(value) => {
                bytes.push(1);
                let encoded = value.canonical_bytes();
                bytes.extend_from_slice(&(encoded.len() as u64).to_be_bytes());
                bytes.extend_from_slice(&encoded);
            }
            None => bytes.push(0),
        }
    }
    let mut bytes = b"dfmcp-related-entity-query-v1\0".to_vec();
    bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&snapshot.tick.0.to_be_bytes());
    bytes.extend_from_slice(snapshot.state_hash.as_bytes());
    bytes.extend_from_slice(&(selection.kinds.len() as u64).to_be_bytes());
    for name in &selection.kinds {
        string(&mut bytes, name);
    }
    selection.relation.append_identity(&mut bytes);
    predicate(&mut bytes, selection.predicate.as_ref());
    selection.relation.append_predicate_identity(&mut bytes);
    bytes.extend_from_slice(&(selection.fields.len() as u64).to_be_bytes());
    for field in &selection.fields {
        string(&mut bytes, field);
    }
    bytes.push(match selection.order {
        Order::IdAscending => 0,
        Order::IdDescending => 1,
    });
    Digest32::of_bytes(&bytes)
}

type Position = (EntityId, u32);

fn cursor(identity: Digest32, position: Position) -> String {
    let body = format!("rq1:{identity}:{}:{}", position.0, position.1);
    format!("{body}:{}", Digest32::of_bytes(body.as_bytes()))
}

fn continuation(raw: Option<&str>, identity: Digest32) -> Result<Option<Position>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 256 {
        return Err(exhausted("relation continuation exceeds 256 bytes"));
    }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len() != 5 || parts[0] != "rq1" {
        return Err(invalid("invalid relation continuation"));
    }
    let id = core_query::entity_id(parts[2])?;
    let generation = parts[3]
        .parse::<u32>()
        .map_err(|_| invalid("invalid relation cursor generation"))?;
    let position = (id, generation);
    if raw != cursor(identity, position) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "relation continuation changed or belongs to another source, session, or selection",
        ));
    }
    Ok(Some(position))
}

fn row(candidate: &Candidate<'_>, fields: &[String]) -> Value {
    let mut row = core_query::entity_json(candidate.entity, fields);
    row["relations"] = candidate.evidence.json();
    row
}

/// A bounded one-hop semijoin. Source and target predicates are evaluated once
/// per entity, then each edge is examined once. No graph walk runs per row.
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
            "relation query context does not name its source",
        ));
    }
    core_query::validate_input(input)?;
    let request: Envelope = serde_json::from_value(input.clone())
        .map_err(|error| invalid(format!("invalid relation query: {error}")))?;
    if request.schema != "dfmcp.query/1" {
        return Err(invalid("query schema must be dfmcp.query/1"));
    }
    if request
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| *anchor != core_query::anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "expected_anchor does not match the complete relation source",
        ));
    }
    let Query::RelatedEntities {
        kinds: source_kinds,
        predicate,
        relation,
        fields,
        order,
        limit,
        continuation: raw_cursor,
    } = request.query;
    let selection = Selection {
        kinds: kinds(source_kinds)?,
        predicate: predicate.map(core_query::decode_filter).transpose()?,
        relation: relation.normalize()?,
        fields: core_query::bounded_names(fields, 32)?,
        order: order.unwrap_or_default(),
    };
    let limit = limit.unwrap_or(4);
    if limit == 0 || limit > MAX_ROWS.min(context.budget.max_entities as usize) {
        return Err(invalid(
            "relation page limit exceeds the negotiated 1..128 row allowance",
        ));
    }
    let mut work = source::preflight(snapshot, context, started)?;
    let entity_work = selection
        .predicate
        .as_ref()
        .map_or(0, Predicate::complexity)
        .saturating_add(selection.relation.entity_work())
        .saturating_add(selection.kinds.len())
        .saturating_add(selection.fields.len())
        .saturating_add(4);
    work.charge(
        0,
        snapshot
            .graph
            .entities
            .len()
            .saturating_mul(entity_work)
            .saturating_add(
                snapshot
                    .graph
                    .edges
                    .len()
                    .saturating_mul(selection.relation.edge_work()),
            ),
    )?;
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "relation-query source hash is invalid",
        ));
    }
    let digest = identity(snapshot, context, &selection);
    let after = continuation(raw_cursor.as_deref(), digest)?;
    let relation_selector::Prepared {
        candidates,
        selected,
        filter_false,
        filter_unknown,
        selected_edges,
    } = relation_selector::prepare(
        snapshot,
        &selection.kinds,
        selection.predicate.as_ref(),
        &selection.relation,
        &work,
    )?;
    let matched = candidates
        .values()
        .filter(|candidate| candidate.evidence.matches > 0)
        .count();
    let unknown_relation = candidates
        .values()
        .filter(|candidate| candidate.evidence.unknown())
        .count();
    let missing_targets: usize = candidates
        .values()
        .map(|candidate| candidate.evidence.missing_targets)
        .sum();
    let unknown_targets: usize = candidates
        .values()
        .map(|candidate| candidate.evidence.unknown_targets)
        .sum();
    let no_observed_match = candidates
        .len()
        .saturating_sub(matched)
        .saturating_sub(unknown_relation);
    let mut after_found = after.is_none();
    let mut remaining = 0usize;
    let mut page = Vec::new();
    let ordered: Box<dyn Iterator<Item = &Candidate<'_>> + '_> = match selection.order {
        Order::IdAscending => Box::new(candidates.values()),
        Order::IdDescending => Box::new(candidates.values().rev()),
    };
    for candidate in ordered {
        work.check()?;
        if candidate.evidence.matches == 0 {
            continue;
        }
        if after == Some((candidate.entity.id, candidate.entity.generation)) {
            after_found = true;
        }
        if after.is_some_and(|(id, _)| match selection.order {
            Order::IdAscending => candidate.entity.id <= id,
            Order::IdDescending => candidate.entity.id >= id,
        }) {
            continue;
        }
        remaining += 1;
        if page.len() < limit {
            page.push(candidate);
        }
    }
    if !after_found || (after.is_some() && remaining == 0) {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "relation continuation does not name a nonfinal hit in this source",
        ));
    }
    let maximum = usize::try_from(
        context
            .budget
            .max_bytes
            .min(max_output_bytes)
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| exhausted("relation response byte allowance cannot be represented"))?;
    let output = core_query::PageBudget {
        maximum_bytes: maximum,
        fits,
    };
    let reaches_terminal = page.len() == remaining;
    let mut accepted = None;
    let mut payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"related_entities",
        "anchor":core_query::anchor_json(snapshot.anchor()), "query_digest":digest.to_string(),
        "ordering":match selection.order { Order::IdAscending => "entity_id_ascending", Order::IdDescending => "entity_id_descending" },
        "relation":selection.relation.json(),
        "matched":matched, "returned":0, "rows":[], "truncated":false, "continuation":null,
        "selection":{"kind_selected":selected, "excluded_filter_false":filter_false,
            "excluded_filter_unknown":filter_unknown, "unknown_relation":unknown_relation,
            "no_observed_match":no_observed_match, "missing_target_records":missing_targets,
            "unknown_target_filters":unknown_targets, "selected_edges":selected_edges},
        "coverage":{"domain":"positive_relations_in_canonical_projection",
            "selection_complete_within_projection":unknown_relation == 0 && filter_unknown == 0,
            "relation_domain_complete":false, "absence_proven":false,
            "eligible_as_mutation_evidence":false, "edge_source_authenticity":"not_established",
            "note":"Only witnessed positive relations select rows. No observed match is not complete-world absence; missing target records and unavailable target facts remain Unknown. Edge records and endpoint generations name this source, not historical causal attribution."},
        "epistemic_state":"inferred", "taint":"untrusted_source_text",
        "source_cost":{"bounded_bytes":work.bytes, "estimated_scan_work_units":work.work}
    });
    for candidate in page {
        if !source::render_fits(candidate.entity, &selection.fields, &[], maximum, &mut work)? {
            break;
        }
        let rows = payload["rows"]
            .as_array_mut()
            .ok_or_else(|| invalid("relation result rows are unavailable"))?;
        rows.push(row(candidate, &selection.fields));
        let count = rows.len();
        payload["returned"] = json!(count);
        payload["truncated"] = json!(count < remaining);
        payload["continuation"] = if count < remaining {
            json!(cursor(
                digest,
                (candidate.entity.id, candidate.entity.generation)
            ))
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
            "one complete relation row and its envelope cannot fit; narrow fields or increase the response budget",
        ))?;
    if !output.accepts(&payload)? {
        return Err(exhausted(
            "one complete relation row and its envelope cannot fit; narrow fields or increase the response budget",
        ));
    }
    work.check()?;
    Ok(payload)
}

#[cfg(test)]
#[path = "related_entity_query_tests.rs"]
mod tests;
