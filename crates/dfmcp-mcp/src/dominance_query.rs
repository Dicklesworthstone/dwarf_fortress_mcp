//! Agent-facing, generation-fenced directed bottlenecks. No new observation,
//! retained state, effect, or native-protocol authority is created here.

use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
use dfmcp_world::graph_query::{
    DOMINANCE_POLICY, DominanceQuery, DominatorRecord, GraphBudget, GraphDirection,
    analyze_dominators_with_check,
};
use dfmcp_world::{EdgeKind, WorldSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::core_query::anchor_json;

#[path = "spatial_query_source.rs"]
#[allow(dead_code, clippy::duplicate_mod)] // Only the shared source admission is used here.
mod source;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Envelope {
    schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_anchor: Option<Value>,
    query: Request,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Dominators {
        root: String,
        root_generation: u32,
        edge_kinds: Vec<String>,
        direction: Option<Direction>,
        #[serde(
            default,
            deserialize_with = "present_target",
            skip_serializing_if = "Option::is_none"
        )]
        target: Option<String>,
        #[serde(
            default,
            deserialize_with = "present_target",
            skip_serializing_if = "Option::is_none"
        )]
        target_generation: Option<u32>,
    },
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    Outgoing,
    Incoming,
}

// An omitted endpoint is optional; an explicit null is not a valid handle.
// Keep this distinction when the laboratory redecodes the original JSON bytes.
fn present_target<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn failure(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn invalid(message: &str) -> DfmcpError {
    failure(ErrorCode::InvalidRequest, message)
}

fn bounded(message: &str) -> DfmcpError {
    failure(ErrorCode::BudgetExceeded, message)
}

// Bound even malformed JSON trees BEFORE cloning for typed deserialization.
// IDs, selectors and unknown fields receive stricter typed validation below.
fn validate_input(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(16);
        if depth > 8 || nodes > 256 {
            return Err(bounded("dominator query exceeds its input structure bound"));
        }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if values
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(nodes)
                    > 256
                {
                    return Err(bounded("dominator query exceeds its input node bound"));
                }
                pending.extend(values.iter().map(|value| (value, depth + 1)));
            }
            Value::Object(values) => {
                if values
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(nodes)
                    > 256
                {
                    return Err(bounded("dominator query exceeds its input node bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            Value::Number(number) => bytes = bytes.saturating_add(number.to_string().len()),
            _ => {}
        }
        if bytes > 16_384 {
            return Err(bounded("dominator query exceeds its input byte bound"));
        }
    }
    Ok(())
}

fn entity(raw: &str, generation: u32, snapshot: &WorldSnapshot) -> Result<EntityId> {
    if raw.is_empty()
        || raw.len() > 20
        || raw.starts_with('0')
        || !raw.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(invalid(
            "dominator IDs must be canonical positive decimal u64 strings",
        ));
    }
    let id = raw
        .parse::<u64>()
        .map(EntityId::new)
        .map_err(|_| invalid("dominator entity ID exceeds u64"))?;
    let record = snapshot.graph.entities.get(&id).ok_or_else(|| {
        invalid("dominator endpoint is not in the observed projection; absence is not proven")
    })?;
    if record.generation != generation {
        return Err(failure(
            ErrorCode::Conflict,
            "dominator endpoint generation changed; refresh its handle",
        ));
    }
    Ok(id)
}

fn edge_kind(raw: &str) -> Result<EdgeKind> {
    if raw.len() > 128 || raw.contains('\0') {
        return Err(invalid(
            "dominator edge selector violates its UTF-8 byte bound",
        ));
    }
    Ok(match raw {
        "located_at" => EdgeKind::LocatedAt,
        "contained_in" => EdgeKind::ContainedIn,
        "assigned_to" => EdgeKind::AssignedTo,
        "member_of" => EdgeKind::MemberOf,
        "performs" => EdgeKind::Performs,
        "requires" => EdgeKind::Requires,
        "produces" => EdgeKind::Produces,
        "uses" => EdgeKind::Uses,
        "supports" => EdgeKind::Supports,
        "threatens" => EdgeKind::Threatens,
        "ordered_by" => EdgeKind::OrderedBy,
        value if value.starts_with("custom:") && value.len() > 7 => {
            EdgeKind::Custom(value[7..].to_owned())
        }
        "parent_of" => EdgeKind::ParentOf,
        _ => return Err(invalid("unknown dominator edge kind")),
    })
}

fn record_json(record: &DominatorRecord) -> Value {
    json!({"entity_id":record.entity_id.to_string(),
        "generation":record.generation,"revision":record.revision,
        "immediate_dominator":record.immediate_dominator.map(|id| id.to_string()),
        "dominated_descendants":record.dominated_descendants})
}

pub(super) fn execute(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    execute_with_output_budget(snapshot, context, input, context.budget.max_bytes)
}

pub(super) fn execute_with_output_budget(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
    output_bytes: u64,
) -> Result<Value> {
    execute_at_source(snapshot, context, input, output_bytes, false)
}

/// History selection belongs to the laboratory owner. Keep authorization on
/// its actual current anchor, including when a restore moved the clock back.
pub(crate) fn execute_laboratory_source(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    input: &Value,
    remaining_wall_millis: u64,
) -> Result<Value> {
    if remaining_wall_millis == 0 {
        return Err(bounded(
            "dominator query exhausted its original foreground deadline",
        ));
    }
    let mut context = current.clone();
    context.budget.max_wall_millis = context.budget.max_wall_millis.min(remaining_wall_millis);
    execute_at_source(snapshot, &context, input, context.budget.max_bytes, true)
}

fn execute_at_source(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    input: &Value,
    output_bytes: u64,
    retained_laboratory_source: bool,
) -> Result<Value> {
    let started = Instant::now();
    // Whole-fortress Query only. Empty selectors must not widen a scoped grant.
    let check = || {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(bounded("dominator query exceeded its foreground deadline"));
        }
        Ok(())
    };
    check()?;
    if context.anchor.fortress_id != snapshot.fortress_id
        || (!retained_laboratory_source && context.anchor != snapshot.anchor())
    {
        return Err(failure(
            ErrorCode::StaleAnchor,
            "dominator context names another source",
        ));
    }
    validate_input(input)?;
    if ["target", "target_generation"]
        .iter()
        .any(|key| input["query"].get(*key).is_some_and(Value::is_null))
    {
        return Err(invalid(
            "omit both target fields for a full-tree query; explicit null is not an endpoint",
        ));
    }
    let parsed: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid dominators query envelope"))?;
    if parsed.schema != "dfmcp.query/1" {
        return Err(invalid("dominators require dfmcp.query/1"));
    }
    if parsed
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| anchor != &anchor_json(snapshot.anchor()))
    {
        return Err(failure(
            ErrorCode::StaleAnchor,
            "dominator expected_anchor differs from the complete source anchor",
        ));
    }
    let Request::Dominators {
        root,
        root_generation,
        mut edge_kinds,
        direction,
        target,
        target_generation,
    } = parsed.query;
    if edge_kinds.is_empty() || edge_kinds.len() > 32 {
        return Err(invalid("dominators require 1 to 32 explicit edge kinds"));
    }
    if target.is_some() != target_generation.is_some() {
        return Err(invalid(
            "target and target_generation must be supplied together",
        ));
    }
    let maximum = context
        .budget
        .max_bytes
        .min(output_bytes)
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    if maximum == 0 {
        return Err(bounded(
            "dominator query has no complete-response allowance",
        ));
    }
    // A target view limits presentation, never source validation or analysis.
    // Bound nesting and frontier allocation before canonical hashing.
    let mut structural_context = context.clone();
    structural_context.budget.max_bytes = 16 * 1024 * 1024;
    let mut work = source::preflight(snapshot, &structural_context, started)?;
    if snapshot.canonical_bytes().len() as u64 > context.budget.max_bytes {
        return Err(bounded(
            "dominator query exceeds its canonical source byte allowance",
        ));
    }
    work.check()?;
    let root = entity(&root, root_generation, snapshot)?;
    let target = match (target, target_generation) {
        (Some(id), Some(generation)) => Some(entity(&id, generation, snapshot)?),
        _ => None,
    };
    edge_kinds.sort();
    edge_kinds.dedup();
    let direction = match direction.unwrap_or(Direction::Outgoing) {
        Direction::Outgoing => GraphDirection::Outgoing,
        Direction::Incoming => GraphDirection::Incoming,
    };
    let request = DominanceQuery {
        root,
        direction,
        edge_kinds: edge_kinds
            .iter()
            .map(|kind| edge_kind(kind))
            .collect::<Result<_>>()?,
    };
    let graph_budget = GraphBudget {
        max_vertices: (context.budget.max_entities as usize).min(100_000),
        max_edges: (context.budget.max_entities as usize).min(1_000_000),
        max_frontier: (context.budget.max_entities as usize).min(4_096),
        max_work: 4_000_000u64.saturating_sub(work.work as u64).min(2_000_000),
    };
    let scope = Digest32::of_bytes(&serde_json::to_vec(&json!({
        "domain":"dfmcp-dominance-authorized-scope/1",
        "session":context.session_id.to_string(),"fortress":context.anchor.fortress_id.to_string(),
        "capability":"query","scope":"whole_fortress"
    })).map_err(|_| invalid("dominator scope cannot be encoded"))?);
    let analysis = analyze_dominators_with_check(snapshot, scope, &request, graph_budget, || {
        work.check()?;
        check()
    })?;
    let source_structure_work = work.work;
    let graph_work = usize::try_from(analysis.witness.work_units)
        .map_err(|_| bounded("dominator work count cannot be represented"))?;
    work.charge(0, graph_work)?;
    // Chain validation builds a bounded identity index over the complete tree.
    // Account for it even though a focused view emits only the selected chain.
    if target.is_some() {
        work.charge(0, analysis.vertices.len())?;
    }
    let chain = target
        .map(|id| analysis.dominator_chain_to(id))
        .transpose()?
        .flatten();
    let mut output_used = 0;
    let mut records = Vec::new();
    match &chain {
        Some(chain) => {
            for id in chain {
                let index = analysis
                    .vertices
                    .binary_search_by_key(id, |record| record.entity_id)
                    .map_err(|_| {
                        failure(
                            ErrorCode::InternalInvariantViolation,
                            "dominator chain lost its record",
                        )
                    })?;
                push_output(
                    &mut records,
                    record_json(&analysis.vertices[index]),
                    &mut output_used,
                    maximum,
                    &mut work,
                )?;
            }
        }
        None if target.is_none() => {
            for record in &analysis.vertices {
                push_output(
                    &mut records,
                    record_json(record),
                    &mut output_used,
                    maximum,
                    &mut work,
                )?;
            }
        }
        None => {}
    }
    let mut unreachable = Vec::new();
    if target.is_none() {
        for id in &analysis.unreachable {
            push_output(
                &mut unreachable,
                json!(id.to_string()),
                &mut output_used,
                maximum,
                &mut work,
            )?;
        }
    }
    let mut chain_json = Vec::new();
    if let Some(chain) = &chain {
        for id in chain {
            push_output(
                &mut chain_json,
                json!(id.to_string()),
                &mut output_used,
                maximum,
                &mut work,
            )?;
        }
    }
    let witness = &analysis.witness;
    let payload = json!({
        "schema":"dfmcp.query.result/1","kind":"dominators",
        "anchor":anchor_json(snapshot.anchor()),"root":root.to_string(),
        "root_generation":root_generation,
        "direction":if direction == GraphDirection::Incoming {"incoming"} else {"outgoing"},
        "edge_kinds":edge_kinds,"policy":DOMINANCE_POLICY,
        "view":if target.is_some() {"target_chain"} else {"dominator_tree"},
        "target":target.map(|id| id.to_string()),"target_generation":target_generation,
        "target_status":target.map(|_| if chain.is_some() {"reachable"} else {"unreachable_in_projection"}),
        "vertices":records,"returned":records.len(),
        "reachable_count":analysis.vertices.len(),"unreachable_count":analysis.unreachable.len(),
        "unreachable":if target.is_none() {Some(unreachable)} else {None},
        "dominator_chain":chain.as_ref().map(|_| chain_json),
        "analysis_complete":true,"truncated":false,"continuation":null,
        "native_captures":0,"mutation_dispatched":false,
        "coverage":{"domain":"observed_projection","absence_proven":false,
            "walkability_proven":false,"mutation_authorized":false},
        "interpretation":"Unavoidable vertices on selected directed relation paths, not a traversable route, resource feasibility, causal necessity, or whole-world absence. Impact counts exclude the removed vertex. The target view omits unrelated tree rows, not analysis work.",
        "budget":{"source_structure_work":source_structure_work,
            "analysis_work":graph_work,"total_work":work.work},
        "witness":{"anchor":anchor_json(witness.source_anchor),
            "authorization_scope_digest":witness.authorization_scope_digest.to_string(),
            "projection_digest":witness.projection_digest.to_string(),
            "decision_digest":witness.decision_digest.to_string(),
            "scanned_vertices":witness.scanned_vertices,"scanned_edges":witness.scanned_edges,
            "examined_arcs":witness.examined_arcs,"work_units":witness.work_units,
            "fixed_point_passes":analysis.fixed_point_passes,"intersection_steps":analysis.intersection_steps}
    });
    if serde_json::to_vec(&payload)
        .map_err(|_| invalid("dominator result cannot be encoded"))?
        .len() as u64
        > maximum
    {
        return Err(bounded(
            "complete dominator result exceeds the output budget; select a target or increase the negotiated budget",
        ));
    }
    work.check()?;
    check()?;
    Ok(payload)
}

fn push_output(
    output: &mut Vec<Value>,
    value: Value,
    used: &mut u64,
    maximum: u64,
    work: &mut source::Budget,
) -> Result<()> {
    work.charge(0, 1)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| invalid("dominator output child cannot be encoded"))?
        .len() as u64;
    let next = used
        .checked_add(bytes)
        .ok_or_else(|| bounded("dominator output length overflows"))?;
    if next > maximum {
        return Err(bounded(
            "dominator output materialization exceeds the complete-response allowance",
        ));
    }
    *used = next;
    output.push(value);
    Ok(())
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value =
        serde_json::from_str(include_str!("../../../schemas/mcp_dominators_v1.json"))
            .map_err(|_| invalid("embedded dominators schema is invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("query schema variants absent"))?
        .push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "dominance_query_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "dominance_query_history_tests.rs"]
mod history_tests;
