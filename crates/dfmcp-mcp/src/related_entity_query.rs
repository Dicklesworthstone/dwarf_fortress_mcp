//! Positive, witnessed relationship selection over one exact canonical source.
//! Missing relations never become negative evidence about the complete world.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EdgeId, EntityId, ErrorCode, OperationContext, Result,
    RiskTier,
};
use dfmcp_world::{
    EdgeRecord, EntityRecord, Predicate, PredicateTruth, WorldSnapshot, evaluate_truth_for,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::core_query;

#[path = "spatial_query_source.rs"]
#[allow(clippy::duplicate_mod)]
mod source;

const MAX_ROWS: usize = 128;
const MAX_WITNESSES: usize = 4;

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
        relation: RelationRequest,
        #[serde(default)]
        fields: Vec<String>,
        order: Option<Order>,
        limit: Option<usize>,
        continuation: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RelationRequest {
    edge_kinds: Vec<String>,
    direction: Option<Direction>,
    #[serde(default)]
    target_kinds: Vec<String>,
    #[serde(default)]
    target_ids: Vec<String>,
    #[serde(rename = "where")]
    predicate: Option<Value>,
}

#[derive(Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    #[default]
    Outgoing,
    Incoming,
    Undirected,
}

impl Direction {
    fn name(self) -> &'static str {
        match self {
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Undirected => "undirected",
        }
    }
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
    edge_kinds: BTreeSet<String>,
    direction: Direction,
    target_kinds: BTreeSet<String>,
    target_ids: BTreeSet<EntityId>,
    target_predicate: Option<Predicate>,
    fields: Vec<String>,
    order: Order,
}

struct Candidate<'a> {
    entity: &'a EntityRecord,
    matches: usize,
    unknown_targets: usize,
    missing_targets: usize,
    witnesses: Vec<(&'a EdgeRecord, &'a EntityRecord)>,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn kinds(names: Vec<String>) -> Result<BTreeSet<String>> {
    core_query::bounded_names(names, 32)?
        .into_iter()
        .map(|name| Ok(core_query::entity_kind(&name)?.as_str().to_owned()))
        .collect()
}

fn matches_kind(kinds: &BTreeSet<String>, entity: &EntityRecord) -> bool {
    kinds.is_empty() || kinds.contains(entity.kind.as_str())
}

fn truth(snapshot: &WorldSnapshot, id: EntityId, predicate: Option<&Predicate>) -> PredicateTruth {
    predicate.map_or(PredicateTruth::True, |predicate| {
        evaluate_truth_for(snapshot, id, predicate)
    })
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
    for names in [
        &selection.kinds,
        &selection.edge_kinds,
        &selection.target_kinds,
    ] {
        bytes.extend_from_slice(&(names.len() as u64).to_be_bytes());
        for name in names {
            string(&mut bytes, name);
        }
    }
    bytes.extend_from_slice(&(selection.target_ids.len() as u64).to_be_bytes());
    for id in &selection.target_ids {
        bytes.extend_from_slice(&id.get().to_be_bytes());
    }
    string(&mut bytes, selection.direction.name());
    predicate(&mut bytes, selection.predicate.as_ref());
    predicate(&mut bytes, selection.target_predicate.as_ref());
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

fn record_edge<'a>(
    subject: EntityId,
    target: EntityId,
    edge: &'a EdgeRecord,
    selection: &Selection,
    targets: &BTreeMap<EntityId, (&'a EntityRecord, PredicateTruth)>,
    candidates: &mut BTreeMap<EntityId, Candidate<'a>>,
) {
    let Some(candidate) = candidates.get_mut(&subject) else {
        return;
    };
    if !selection.target_ids.is_empty() && !selection.target_ids.contains(&target) {
        return;
    }
    match targets.get(&target) {
        Some((entity, PredicateTruth::True)) => {
            candidate.matches += 1;
            if candidate.witnesses.len() < MAX_WITNESSES {
                candidate.witnesses.push((edge, entity));
            }
        }
        Some((_, PredicateTruth::False)) => {}
        Some((_, PredicateTruth::Unknown)) => candidate.unknown_targets += 1,
        None => candidate.missing_targets += 1,
    }
}

fn row(candidate: &Candidate<'_>, fields: &[String]) -> Value {
    let mut row = core_query::entity_json(candidate.entity, fields);
    row["relations"] = json!({
        "matched_edges":candidate.matches,
        "witnesses_returned":candidate.witnesses.len(),
        "witnesses_omitted":candidate.matches.saturating_sub(candidate.witnesses.len()),
        "unknown_target_filters":candidate.unknown_targets,
        "missing_target_records":candidate.missing_targets,
        "witnesses":candidate.witnesses.iter().map(|(edge,target)| json!({
            "edge_id":edge.id.to_string(), "edge_revision":edge.revision,
            "edge_kind":edge.kind.as_str(),
            "from":edge.from.to_string(), "to":edge.to.to_string(),
            "target":{"entity_id":target.id.to_string(), "generation":target.generation,
                "revision":target.revision},
            "endpoint_identity_basis":"source_snapshot"
        })).collect::<Vec<_>>(),
        "eligible_as_mutation_evidence":false
    });
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
    let edge_kinds = core_query::bounded_names(relation.edge_kinds, 32)?;
    if edge_kinds.is_empty() {
        return Err(invalid(
            "relation selection requires at least one explicit edge kind",
        ));
    }
    let selection = Selection {
        kinds: kinds(source_kinds)?,
        predicate: predicate.map(core_query::decode_filter).transpose()?,
        edge_kinds: edge_kinds
            .into_iter()
            .map(|name| Ok(core_query::edge_kind(&name)?.as_str().to_owned()))
            .collect::<Result<_>>()?,
        direction: relation.direction.unwrap_or_default(),
        target_kinds: kinds(relation.target_kinds)?,
        target_ids: core_query::bounded_names(relation.target_ids, 64)?
            .into_iter()
            .map(|id| core_query::entity_id(&id))
            .collect::<Result<_>>()?,
        target_predicate: relation
            .predicate
            .map(core_query::decode_filter)
            .transpose()?,
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
        .saturating_add(
            selection
                .target_predicate
                .as_ref()
                .map_or(0, Predicate::complexity),
        )
        .saturating_add(selection.kinds.len())
        .saturating_add(selection.target_kinds.len())
        .saturating_add(selection.target_ids.len())
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
                    .saturating_mul(selection.edge_kinds.len().saturating_add(4)),
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
    let mut targets = BTreeMap::new();
    let mut candidates = BTreeMap::new();
    let (mut selected, mut filter_false, mut filter_unknown) = (0usize, 0usize, 0usize);
    for entity in snapshot.graph.entities.values() {
        work.check()?;
        let target = if matches_kind(&selection.target_kinds, entity)
            && (selection.target_ids.is_empty() || selection.target_ids.contains(&entity.id))
        {
            truth(snapshot, entity.id, selection.target_predicate.as_ref())
        } else {
            PredicateTruth::False
        };
        targets.insert(entity.id, (entity, target));
        if !matches_kind(&selection.kinds, entity) {
            continue;
        }
        selected += 1;
        match truth(snapshot, entity.id, selection.predicate.as_ref()) {
            PredicateTruth::False => {
                filter_false += 1;
                continue;
            }
            PredicateTruth::Unknown => {
                filter_unknown += 1;
                continue;
            }
            PredicateTruth::True => {}
        }
        candidates.insert(
            entity.id,
            Candidate {
                entity,
                matches: 0,
                unknown_targets: 0,
                missing_targets: 0,
                witnesses: Vec::new(),
            },
        );
    }
    let mut selected_edges = 0usize;
    for (id, edge) in &snapshot.graph.edges {
        work.check()?;
        if *id == EdgeId::NIL || *id != edge.id {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "relation-query edge identity is inconsistent",
            ));
        }
        if !selection.edge_kinds.contains(edge.kind.as_str()) {
            continue;
        }
        selected_edges += 1;
        match selection.direction {
            Direction::Outgoing => record_edge(
                edge.from,
                edge.to,
                edge,
                &selection,
                &targets,
                &mut candidates,
            ),
            Direction::Incoming => record_edge(
                edge.to,
                edge.from,
                edge,
                &selection,
                &targets,
                &mut candidates,
            ),
            Direction::Undirected => {
                record_edge(
                    edge.from,
                    edge.to,
                    edge,
                    &selection,
                    &targets,
                    &mut candidates,
                );
                if edge.from != edge.to {
                    record_edge(
                        edge.to,
                        edge.from,
                        edge,
                        &selection,
                        &targets,
                        &mut candidates,
                    );
                }
            }
        }
    }
    let matched = candidates
        .values()
        .filter(|candidate| candidate.matches > 0)
        .count();
    let unknown_relation = candidates
        .values()
        .filter(|candidate| {
            candidate.matches == 0
                && (candidate.unknown_targets > 0 || candidate.missing_targets > 0)
        })
        .count();
    let missing_targets: usize = candidates
        .values()
        .map(|candidate| candidate.missing_targets)
        .sum();
    let unknown_targets: usize = candidates
        .values()
        .map(|candidate| candidate.unknown_targets)
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
        if candidate.matches == 0 {
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
        "relation":{"direction":selection.direction.name(), "edge_kinds":selection.edge_kinds,
            "target_kinds":selection.target_kinds,
            "target_ids":selection.target_ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "witnesses_per_row_limit":MAX_WITNESSES},
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
