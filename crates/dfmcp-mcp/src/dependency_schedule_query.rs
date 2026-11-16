//! Conditional duration-aware scheduling over one exact authorized projection.
//! The result is a planning model, never native completion or dispatch authority.

use std::collections::BTreeSet;
use std::time::Instant;

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};
use dfmcp_world::WorldSnapshot;
use dfmcp_world::graph_query::{
    DEPENDENCY_SCHEDULE_POLICY, DependencyScheduleQuery, GraphBudget,
    schedule_dependencies_with_check,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::core_query::{
    anchor_json, bounded_names, edge_kind, entity_id, fact_json, validate_input,
};

#[path = "spatial_query_source.rs"]
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
    Schedule {
        edge_kinds: Vec<String>,
        duration_field: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<Target>,
    },
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Target {
    entity_id: String,
    generation: u32,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn bounded(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

pub(crate) fn execute(
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

/// Only the laboratory owner may select an exact retained source. Authority
/// remains anchored to its CURRENT world, including after a tick-reset restore.
/// This entry point never selects or loads history and cannot widen its budget.
pub(crate) fn execute_laboratory_source(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    input: &Value,
    remaining_wall_millis: u64,
) -> Result<Value> {
    if remaining_wall_millis == 0 {
        return Err(bounded(
            "dependency schedule exhausted its original foreground deadline",
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
    let check = || {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
            return Err(bounded(
                "dependency schedule exceeded its foreground deadline",
            ));
        }
        Ok(())
    };
    check()?;
    if context.anchor.fortress_id != snapshot.fortress_id
        || (!retained_laboratory_source && context.anchor != snapshot.anchor())
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "dependency schedule context names another source",
        ));
    }
    validate_input(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid dependency schedule query envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("dependency scheduling requires dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| anchor != &anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "dependency schedule expected_anchor differs from the complete source anchor",
        ));
    }
    let Request::Schedule {
        edge_kinds,
        duration_field,
        targets,
    } = envelope.query;
    if edge_kinds.is_empty() || targets.len() > 64 {
        return Err(invalid(
            "schedule requires explicit dependent-to-prerequisite edge kinds and at most 64 targets",
        ));
    }
    let edge_kinds = bounded_names(edge_kinds, 32)?;
    bounded_names(vec![duration_field.clone()], 1)?;

    // Admit every source record and unselected value before canonical hashing or
    // graph allocation. The structural estimate and exact canonical-byte budget
    // are independent, as in the other bounded structured-query executors.
    let mut structural_context = context.clone();
    structural_context.budget.max_bytes = 16 * 1024 * 1024;
    let work = source::preflight(snapshot, &structural_context, started)?;
    if snapshot.canonical_bytes().len() as u64 > context.budget.max_bytes {
        return Err(bounded(
            "dependency schedule exceeds its canonical source byte allowance",
        ));
    }
    work.check()?;
    let mut target_ids = BTreeSet::new();
    let mut target_records = std::collections::BTreeMap::new();
    for target in targets {
        let id = entity_id(&target.entity_id)?;
        let record = snapshot.graph.entities.get(&id).ok_or_else(|| {
            invalid("schedule target is absent from this projection; world absence is not proven")
        })?;
        if target.generation != record.generation {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "schedule target generation changed; inspect its current handle",
            ));
        }
        if !target_ids.insert(id) {
            return Err(invalid("schedule targets must name distinct entities"));
        }
        target_records.insert(
            id,
            json!({"entity_id":id.to_string(),
            "generation":record.generation,"revision":record.revision}),
        );
    }
    let graph_budget = GraphBudget {
        max_vertices: (context.budget.max_entities as usize).min(100_000),
        max_edges: (context.budget.max_entities as usize).min(1_000_000),
        max_frontier: (context.budget.max_entities as usize).min(4_096),
        max_work: 4_000_000u64.saturating_sub(work.work as u64).min(2_000_000),
    };
    let query = DependencyScheduleQuery {
        edge_kinds: edge_kinds
            .iter()
            .map(|kind| edge_kind(kind))
            .collect::<Result<_>>()?,
        duration_field: duration_field.clone(),
        targets: target_ids.iter().copied().collect(),
    };
    let scope = Digest32::of_bytes(
        &serde_json::to_vec(&json!({
            "domain":"dfmcp-dependency-schedule-scope/1",
            "session":context.session_id.to_string(),
            "fortress":context.anchor.fortress_id.to_string(),
            "capability":"query", "scope":"whole_fortress"
        }))
        .map_err(|_| invalid("dependency schedule scope cannot be encoded"))?,
    );
    let analysis = schedule_dependencies_with_check(snapshot, scope, &query, graph_budget, || {
        work.check()?;
        check()
    })?;
    let mut entries = Vec::with_capacity(analysis.entries.len());
    for entry in &analysis.entries {
        check()?;
        let fact = snapshot
            .graph
            .entities
            .get(&entry.entity_id)
            .and_then(|entity| entity.fields.get(&duration_field))
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "scheduled duration lost its source fact",
                )
            })?;
        entries.push(json!({
            "entity_id":entry.entity_id.to_string(),
            "generation":entry.generation,"revision":entry.revision,
            "duration":entry.duration,"duration_fact":fact_json(fact,snapshot.tick),
            "earliest_start":entry.earliest_start,"earliest_finish":entry.earliest_finish,
            "latest_start":entry.latest_start,"latest_finish":entry.latest_finish,
            "total_slack":entry.total_slack
        }));
    }
    let witness = &analysis.witness;
    let payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"schedule",
        "anchor":anchor_json(snapshot.anchor()), "policy":DEPENDENCY_SCHEDULE_POLICY,
        "edge_kinds":edge_kinds, "duration_field":duration_field,
        "targets":target_records.into_values().collect::<Vec<_>>(),
        "model":{"epistemic_state":"predicted", "time_unit":"selected_field_units",
            "start_offset":0, "independent_capacity":"unlimited",
            "resource_contention_modeled":false, "native_completion_proven":false},
        "makespan":analysis.makespan, "entries":entries, "returned":entries.len(),
        "dependency_order":analysis.dependency_order.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "critical_chain":analysis.critical_chain.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "parallel_fronts":analysis.parallel_fronts.iter().map(|front| json!({
            "earliest_start":front.earliest_start,
            "entities":front.entities.iter().map(ToString::to_string).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "analysis_complete":true, "truncated":false, "continuation":null,
        "native_captures":0, "mutation_dispatched":false,
        "coverage":{"domain":"observed_projection",
            "selection":if target_ids.is_empty() {"all_vertices"} else {"complete_selected_prerequisite_closure"},
            "absence_proven":false,"mutation_authorized":false},
        "interpretation":"Conditional schedule in the selected duration field's units. Dependencies precede dependents; independent work has unlimited capacity. Start-time groups may contain zero-duration dependencies, so use dependency_order within a group. Missing observations and real worker/resource contention can change actual completion.",
        "witness":{"anchor":anchor_json(witness.source_anchor),
            "authorization_scope_digest":witness.authorization_scope_digest.to_string(),
            "projection_digest":witness.projection_digest.to_string(),
            "decision_digest":witness.decision_digest.to_string(),
            "scanned_vertices":witness.scanned_vertices,"scanned_edges":witness.scanned_edges,
            "examined_arcs":witness.examined_arcs,"work_units":witness.work_units,
            "source_structure_work_units":work.work}
    });
    let maximum = context
        .budget
        .max_bytes
        .min(output_bytes)
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    if serde_json::to_vec(&payload)
        .map_err(|_| invalid("dependency schedule result cannot be encoded"))?
        .len() as u64
        > maximum
    {
        return Err(bounded(
            "complete dependency schedule exceeds the output budget; select fewer target goals or increase the negotiated allowance",
        ));
    }
    check()?;
    Ok(payload)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_dependency_schedule_v1.json"
    ))
    .map_err(|_| invalid("embedded dependency schedule schema is invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("query schema variants absent"))?
        .push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "dependency_schedule_query_tests.rs"]
mod tests;
