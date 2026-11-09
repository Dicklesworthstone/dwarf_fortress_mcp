//! Exact numeric ranking over one authorized immutable source.
//! A bounded heap retains only the requested page, not the entire population.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
use dfmcp_world::{
    EntityRecord, Predicate, PredicateTruth, Value as WorldValue, WorldSnapshot, evaluate_truth_for,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::core_query;

#[path = "spatial_query_source.rs"]
#[allow(clippy::duplicate_mod)]
mod source;

#[path = "relation_selector.rs"]
#[allow(clippy::duplicate_mod)]
mod relation_selector;

const MAX_ROWS: usize = 128;
const MAX_KEYS: usize = 3;

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
    RankedEntities {
        #[serde(default)]
        kinds: Vec<String>,
        #[serde(rename = "where")]
        predicate: Option<Value>,
        relation: Option<relation_selector::Request>,
        #[serde(default)]
        fields: Vec<String>,
        order_by: Vec<Sort>,
        limit: Option<usize>,
        continuation: Option<String>,
    },
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum NumericType {
    I64,
    U64,
    Fixed,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Ascending,
    Descending,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Sort {
    field: String,
    numeric_type: NumericType,
    direction: Direction,
    scale: Option<u32>,
    max_age_ticks: Option<u64>,
}

struct Selection {
    kinds: BTreeSet<String>,
    predicate: Option<Predicate>,
    relation: Option<relation_selector::Selection>,
    fields: Vec<String>,
    order_by: Vec<Sort>,
    rank_fields: Vec<String>,
}

// Reverse preserves MIN/MAX exactly; no negation, float conversion or mixed
// signedness can overflow or round two distinct values into a false tie.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Number {
    SignedAscending(i64),
    SignedDescending(Reverse<i64>),
    UnsignedAscending(u64),
    UnsignedDescending(Reverse<u64>),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Rank {
    values: Vec<Number>,
    id: EntityId,
    generation: u32,
}

struct Candidate<'a> {
    rank: Rank,
    entity: &'a EntityRecord,
}

impl PartialEq for Candidate<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.rank == other.rank
    }
}
impl Eq for Candidate<'_> {}
impl PartialOrd for Candidate<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Candidate<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank.cmp(&other.rank)
    }
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn gap() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::CursorGap,
        "rank continuation must name a nonfinal eligible row in this exact source",
    )
}

fn matches_kind(selection: &Selection, entity: &EntityRecord) -> bool {
    selection.kinds.is_empty() || selection.kinds.contains(entity.kind.as_str())
}

fn truth(snapshot: &WorldSnapshot, selection: &Selection, id: EntityId) -> PredicateTruth {
    selection
        .predicate
        .as_ref()
        .map_or(PredicateTruth::True, |predicate| {
            evaluate_truth_for(snapshot, id, predicate)
        })
}

fn rank(
    entity: &EntityRecord,
    order_by: &[Sort],
    tick: u64,
) -> std::result::Result<Rank, &'static str> {
    let mut values = Vec::with_capacity(order_by.len());
    for order in order_by {
        let fact = entity.fields.get(&order.field).ok_or("missing")?;
        let value = fact.known_value().ok_or("unavailable")?;
        let age = tick.checked_sub(fact.observed_at.0).ok_or("future")?;
        if order.max_age_ticks.is_some_and(|maximum| age > maximum) {
            return Err("too_old");
        }
        let value = match (order.numeric_type, value, order.direction) {
            (NumericType::I64, WorldValue::I64(value), Direction::Ascending) => {
                Number::SignedAscending(*value)
            }
            (NumericType::I64, WorldValue::I64(value), Direction::Descending) => {
                Number::SignedDescending(Reverse(*value))
            }
            (NumericType::U64, WorldValue::U64(value), Direction::Ascending) => {
                Number::UnsignedAscending(*value)
            }
            (NumericType::U64, WorldValue::U64(value), Direction::Descending) => {
                Number::UnsignedDescending(Reverse(*value))
            }
            (NumericType::Fixed, WorldValue::Fixed { units, scale }, direction) => {
                if Some(*scale) != order.scale {
                    return Err("scale_mismatch");
                }
                match direction {
                    Direction::Ascending => Number::SignedAscending(*units),
                    Direction::Descending => Number::SignedDescending(Reverse(*units)),
                }
            }
            _ => return Err("type_mismatch"),
        };
        values.push(value);
    }
    Ok(Rank {
        values,
        id: entity.id,
        generation: entity.generation,
    })
}

fn identity(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    selection: &Selection,
) -> Result<Digest32> {
    fn append(bytes: &mut Vec<u8>, value: &[u8]) {
        bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
        bytes.extend_from_slice(value);
    }
    let mut bytes = b"dfmcp-ranked-entity-query-v1\0".to_vec();
    bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.fortress_id.get().to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.epoch.to_be_bytes());
    bytes.extend_from_slice(&snapshot.cursor.sequence.to_be_bytes());
    bytes.extend_from_slice(&snapshot.tick.0.to_be_bytes());
    bytes.extend_from_slice(snapshot.state_hash.as_bytes());
    bytes.extend_from_slice(&(selection.kinds.len() as u64).to_be_bytes());
    for kind in &selection.kinds {
        append(&mut bytes, kind.as_bytes());
    }
    match &selection.predicate {
        None => bytes.push(0),
        Some(predicate) => {
            bytes.push(1);
            append(&mut bytes, &predicate.canonical_bytes());
        }
    }
    bytes.extend_from_slice(&(selection.fields.len() as u64).to_be_bytes());
    for field in &selection.fields {
        append(&mut bytes, field.as_bytes());
    }
    append(
        &mut bytes,
        &serde_json::to_vec(&selection.order_by)
            .map_err(|_| invalid("rank ordering cannot be encoded"))?,
    );
    if let Some(relation) = &selection.relation {
        // Leave all established unfiltered rk1 bytes unchanged. A relation is
        // a separately tagged selection, not a capability or retained source.
        append(&mut bytes, b"positive-relation-v1");
        relation.append_identity(&mut bytes);
        relation.append_predicate_identity(&mut bytes);
    }
    Ok(Digest32::of_bytes(&bytes))
}

type Position = (EntityId, u32);

fn cursor(identity: Digest32, position: Position) -> String {
    let body = format!("rk1:{identity}:{}:{}", position.0, position.1);
    format!("{body}:{}", Digest32::of_bytes(body.as_bytes()))
}

fn continuation(raw: Option<&str>, identity: Digest32) -> Result<Option<Position>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.len() > 256 {
        return Err(exhausted("rank continuation exceeds 256 bytes"));
    }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len() != 5 || parts[0] != "rk1" {
        return Err(invalid("invalid rank continuation"));
    }
    let position = (
        core_query::entity_id(parts[2])?,
        parts[3]
            .parse::<u32>()
            .map_err(|_| invalid("invalid rank cursor generation"))?,
    );
    if raw != cursor(identity, position) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "rank continuation changed or belongs to another source, session, or selection",
        ));
    }
    Ok(Some(position))
}

fn row(
    candidate: &Candidate<'_>,
    selection: &Selection,
    relation: Option<&relation_selector::Evidence<'_>>,
) -> Value {
    let mut value = core_query::entity_json(candidate.entity, &selection.fields);
    let facts: BTreeMap<_, _> = selection
        .rank_fields
        .iter()
        .filter_map(|name| {
            candidate
                .entity
                .fields
                .get(name)
                .map(|fact| (name, core_query::fact_json(fact)))
        })
        .collect();
    value["ranking"] = json!({
        "facts":facts, "all_keys_usable":true, "eligible_as_mutation_evidence":false
    });
    if let Some(relation) = relation {
        value["relations"] = relation.json();
    }
    value
}

/// Select one exact ranked page in O(N log K) ordering work and O(K) retained
/// candidates. Every page rescans its bound source, retaining no hidden index.
pub(super) fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    execute_with_output_budget(snapshot, context, input, context.budget.max_bytes)
}

/// The publisher reserves its own envelope bytes without shrinking the source
/// allowance. This private cap can only narrow output; authority and complete-
/// source admission still use the original request context.
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
            "rank query context does not name its source",
        ));
    }
    core_query::validate_input(input)?;
    let request: Envelope = serde_json::from_value(input.clone())
        .map_err(|error| invalid(format!("invalid ranked query: {error}")))?;
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
            "expected_anchor does not match the complete ranked source",
        ));
    }
    let Query::RankedEntities {
        kinds,
        predicate,
        relation,
        fields,
        order_by,
        limit,
        continuation: raw_cursor,
    } = request.query;
    if order_by.is_empty() || order_by.len() > MAX_KEYS {
        return Err(invalid(
            "rank ordering requires one to three numeric fields",
        ));
    }
    let rank_fields = core_query::bounded_names(
        order_by.iter().map(|order| order.field.clone()).collect(),
        MAX_KEYS,
    )?;
    if rank_fields.len() != order_by.len() {
        return Err(invalid("rank ordering fields must be distinct"));
    }
    for order in &order_by {
        if matches!(order.numeric_type, NumericType::Fixed) != order.scale.is_some() {
            return Err(invalid(
                "fixed rank keys require an exact scale; other numeric types cannot specify one",
            ));
        }
    }
    let selection = Selection {
        kinds: core_query::bounded_names(kinds, 32)?
            .into_iter()
            .map(|name| Ok(core_query::entity_kind(&name)?.as_str().to_owned()))
            .collect::<Result<_>>()?,
        predicate: predicate.map(core_query::decode_filter).transpose()?,
        relation: relation
            .map(relation_selector::Request::normalize)
            .transpose()?,
        fields: core_query::bounded_names(fields, 32)?,
        order_by,
        rank_fields,
    };
    let limit = limit.unwrap_or(4);
    if limit == 0 || limit > MAX_ROWS.min(context.budget.max_entities as usize) {
        return Err(invalid(
            "rank page limit exceeds the negotiated 1..128 row allowance",
        ));
    }
    let mut work = source::preflight(snapshot, context, started)?;
    let levels = (usize::BITS - limit.leading_zeros()) as usize;
    // Charge filters, field lookups, cursor comparison, heap replacement and
    // final page sorting before evaluation. Numeric comparisons are constant
    // work, independent of magnitude and fixed-point scale.
    let per_entity = selection
        .predicate
        .as_ref()
        .map_or(0, Predicate::complexity)
        .saturating_add(selection.kinds.len())
        .saturating_add(selection.fields.len())
        .saturating_add(selection.order_by.len())
        .saturating_add((selection.order_by.len() + 2).saturating_mul(3 * levels + 1))
        .saturating_add(4);
    let rank_work = snapshot
        .graph
        .entities
        .len()
        .saturating_add(1)
        .saturating_mul(per_entity);
    let relation_work = selection.relation.as_ref().map_or(0, |relation| {
        snapshot
            .graph
            .entities
            .len()
            .saturating_mul(relation.entity_work().saturating_add(4))
            .saturating_add(
                snapshot
                    .graph
                    .edges
                    .len()
                    .saturating_mul(relation.edge_work()),
            )
    });
    work.charge(0, rank_work.saturating_add(relation_work))?;
    if !snapshot.hash_is_valid() {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "rank-query source hash is invalid",
        ));
    }
    let digest = identity(snapshot, context, &selection)?;
    let after = continuation(raw_cursor.as_deref(), digest)?;
    let relations = selection
        .relation
        .as_ref()
        .map(|relation| {
            relation_selector::prepare(
                snapshot,
                &selection.kinds,
                selection.predicate.as_ref(),
                relation,
                &work,
            )
        })
        .transpose()?;
    let after_rank = after
        .map(|(id, generation)| {
            let entity = snapshot.graph.entities.get(&id).ok_or_else(gap)?;
            if entity.generation != generation
                || relations.as_ref().map_or_else(
                    || {
                        !matches_kind(&selection, entity)
                            || truth(snapshot, &selection, id) != PredicateTruth::True
                    },
                    |relations| {
                        !relations
                            .candidates
                            .get(&id)
                            .is_some_and(|candidate| candidate.evidence.matches > 0)
                    },
                )
            {
                return Err(gap());
            }
            rank(entity, &selection.order_by, snapshot.tick.0).map_err(|_| gap())
        })
        .transpose()?;
    let (mut selected, mut filter_false, mut filter_unknown) =
        relations
            .as_ref()
            .map_or((0usize, 0usize, 0usize), |relations| {
                (
                    relations.selected,
                    relations.filter_false,
                    relations.filter_unknown,
                )
            });
    let (mut matched, mut remaining, mut unknown_rank) = (0usize, 0usize, 0usize);
    let mut reasons = BTreeMap::<&str, usize>::new();
    let mut page = BinaryHeap::<Candidate<'_>>::with_capacity(limit);
    let mut peak_candidates = 0usize;
    for entity in snapshot.graph.entities.values() {
        work.check()?;
        if let Some(relations) = &relations {
            if !relations
                .candidates
                .get(&entity.id)
                .is_some_and(|candidate| candidate.evidence.matches > 0)
            {
                continue;
            }
        } else {
            if !matches_kind(&selection, entity) {
                continue;
            }
            selected += 1;
            match truth(snapshot, &selection, entity.id) {
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
        }
        let rank = match rank(entity, &selection.order_by, snapshot.tick.0) {
            Ok(rank) => rank,
            Err(reason) => {
                unknown_rank += 1;
                *reasons.entry(reason).or_default() += 1;
                continue;
            }
        };
        matched += 1;
        if after_rank.as_ref().is_some_and(|after| rank <= *after) {
            continue;
        }
        remaining += 1;
        if page.len() < limit {
            page.push(Candidate { rank, entity });
        } else if page.peek().is_some_and(|worst| rank < worst.rank) {
            page.pop();
            page.push(Candidate { rank, entity });
        }
        peak_candidates = peak_candidates.max(page.len());
    }
    if after.is_some() && remaining == 0 {
        return Err(gap());
    }
    let maximum = usize::try_from(
        context
            .budget
            .max_bytes
            .min(max_output_bytes)
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| exhausted("rank response byte allowance cannot be represented"))?;
    let coverage = json!({
        "domain":"known_typed_numeric_fields_in_canonical_projection",
        "selection_complete_within_projection":filter_unknown == 0 && unknown_rank == 0,
        "world_domain_complete":false, "absence_proven":false,
        "eligible_as_mutation_evidence":false,
        "note":"Ranking uses only available values of the exact declared types and scales. Unknown keys are excluded and counted, never zero-filled. Values retain their source and observation tick; ordering is not source authority or a causal priority claim."
    });
    let output = core_query::PageBudget {
        maximum_bytes: maximum,
        fits,
    };
    let reaches_terminal = page.len() == remaining;
    let mut accepted = None;
    let mut payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"ranked_entities",
        "anchor":core_query::anchor_json(snapshot.anchor()), "query_digest":digest.to_string(),
        "order_by":selection.order_by, "tie_breaker":"entity_id_ascending",
        "matched":matched, "returned":0, "rows":[], "truncated":false, "continuation":null,
        "selection":{"kind_selected":selected, "excluded_filter_false":filter_false,
            "excluded_filter_unknown":filter_unknown, "unknown_rank":unknown_rank,
            "unknown_rank_reasons":reasons,
            "reason_counting":"first_unusable_sort_key_per_entity"},
        "coverage":coverage, "epistemic_state":"inferred", "taint":"untrusted_source_text",
        "source_cost":{"bounded_bytes":work.bytes, "estimated_scan_work_units":work.work,
            "algorithm":"bounded_top_k", "maximum_retained_candidates":peak_candidates,
            "page_candidate_limit":limit}
    });
    if let (Some(relation), Some(relations)) = (&selection.relation, &relations) {
        let relation_matched = relations
            .candidates
            .values()
            .filter(|candidate| candidate.evidence.matches > 0)
            .count();
        let unknown_relation = relations
            .candidates
            .values()
            .filter(|candidate| candidate.evidence.unknown())
            .count();
        let missing_targets: usize = relations
            .candidates
            .values()
            .map(|candidate| candidate.evidence.missing_targets)
            .sum();
        let unknown_targets: usize = relations
            .candidates
            .values()
            .map(|candidate| candidate.evidence.unknown_targets)
            .sum();
        payload["relation"] = relation.json();
        payload["selection"]["relation_matched"] = json!(relation_matched);
        payload["selection"]["unknown_relation"] = json!(unknown_relation);
        payload["selection"]["no_observed_match"] = json!(
            relations
                .candidates
                .len()
                .saturating_sub(relation_matched)
                .saturating_sub(unknown_relation)
        );
        payload["selection"]["missing_target_records"] = json!(missing_targets);
        payload["selection"]["unknown_target_filters"] = json!(unknown_targets);
        payload["selection"]["selected_edges"] = json!(relations.selected_edges);
        payload["coverage"]["domain"] =
            json!("known_typed_numeric_fields_with_positive_relations_in_canonical_projection");
        payload["coverage"]["selection_complete_within_projection"] =
            json!(filter_unknown == 0 && unknown_relation == 0 && unknown_rank == 0);
        payload["coverage"]["relation_domain_complete"] = json!(false);
        payload["coverage"]["edge_source_authenticity"] = json!("not_established");
        payload["coverage"]["relation_note"] = json!(
            "Only witnessed positive relations enter ranking. No observed match is not complete-world absence; missing target records and unavailable target facts remain Unknown."
        );
        payload["source_cost"]["algorithm"] = json!("bounded_relation_semijoin_then_top_k");
        payload["source_cost"]["relation_source_candidates"] = json!(relations.candidates.len());
        payload["source_cost"]["relation_target_records"] = json!(snapshot.graph.entities.len());
    }
    for candidate in page.into_sorted_vec() {
        if !source::render_fits(
            candidate.entity,
            &selection.fields,
            &selection.rank_fields,
            maximum,
            &mut work,
        )? {
            break;
        }
        let rows = payload["rows"]
            .as_array_mut()
            .ok_or_else(|| invalid("rank result rows are unavailable"))?;
        let relation = relations
            .as_ref()
            .and_then(|relations| relations.candidates.get(&candidate.entity.id))
            .map(|candidate| &candidate.evidence);
        rows.push(row(&candidate, &selection, relation));
        let count = rows.len();
        payload["returned"] = json!(count);
        payload["truncated"] = json!(count < remaining);
        payload["continuation"] = if count < remaining {
            json!(cursor(
                digest,
                (candidate.rank.id, candidate.rank.generation)
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
            "one complete ranked row and its envelope cannot fit; narrow fields or increase the response budget",
        ))?;
    if !output.accepts(&payload)? {
        return Err(exhausted(
            "one complete ranked row and its envelope cannot fit; narrow fields or increase the response budget",
        ));
    }
    work.check()?;
    Ok(payload)
}

#[cfg(test)]
#[path = "ranked_entity_query_tests.rs"]
mod tests;
