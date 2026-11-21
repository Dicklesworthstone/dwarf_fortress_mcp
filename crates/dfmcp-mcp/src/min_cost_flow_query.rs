//! Minimum-cost finite demand over one authorized observed directed network.
//! Exact model allocations are advisory, never native dispatch or completion.

use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
use dfmcp_world::graph_query::{
    GraphBudget, MIN_COST_FLOW_POLICY, MinCostFlowQuery, minimum_cost_flow_with_check,
};
use dfmcp_world::{EdgeKind, WorldSnapshot};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::core_query::{
    anchor_json, bounded_names, edge_kind, entity_id, fact_json, validate_input,
};

// The shared file also supplies entity rendering to entity queries; this
// executor uses only its source-admission and work-budget helpers.
#[path = "spatial_query_source.rs"]
#[allow(dead_code)]
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
    MinCostFlow {
        edge_kinds: Vec<String>,
        capacity_field: String,
        unit_field: String,
        cost_field: String,
        cost_unit_field: String,
        demand: u64,
        source: Target,
        sink: Target,
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

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

// Admit each bounded child before retaining it. A small response allowance must
// not cause the complete source's much larger JSON tree to be materialized.
// Delimiters and fixed envelope fields are checked by the exact final encoding.
fn push_output(
    output: &mut Vec<Value>,
    value: Value,
    used: &mut u64,
    maximum: u64,
    work: &mut source::Budget,
) -> Result<()> {
    work.charge(0, 1)?;
    let bytes = serde_json::to_vec(&value)
        .map_err(|_| invalid("minimum-cost-flow output child cannot be encoded"))?
        .len() as u64;
    let next = used
        .checked_add(bytes)
        .ok_or_else(|| bounded("minimum-cost-flow output length overflows"))?;
    if next > maximum {
        return Err(bounded(
            "minimum-cost-flow output materialization exceeds the complete-response allowance",
        ));
    }
    *used = next;
    output.push(value);
    Ok(())
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

/// The laboratory owner selects history and retains the actual current context.
/// This cannot load history, renew grants, advance time or acquire work ownership.
pub(crate) fn execute_laboratory_source(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    input: &Value,
    remaining_wall_millis: u64,
) -> Result<Value> {
    if remaining_wall_millis == 0 {
        return Err(bounded(
            "minimum-cost-flow query exhausted its original foreground deadline",
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
                "minimum-cost-flow query exceeded its foreground deadline",
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
            "minimum-cost-flow query context names another source",
        ));
    }
    validate_input(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid minimum-cost-flow query envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("minimum-cost-flow queries require dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| anchor != &anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "minimum-cost-flow expected_anchor differs from the complete source anchor",
        ));
    }
    let Request::MinCostFlow {
        edge_kinds,
        capacity_field,
        unit_field,
        cost_field,
        cost_unit_field,
        demand,
        source: source_target,
        sink: sink_target,
    } = envelope.query;
    if edge_kinds.is_empty() {
        return Err(invalid(
            "minimum-cost-flow requires explicit directed edge kinds",
        ));
    }
    let edge_kinds = bounded_names(edge_kinds, 32)?;
    let selected_kinds = edge_kinds
        .iter()
        .map(|kind| edge_kind(kind))
        .collect::<Result<_>>()?;
    let fields = [
        capacity_field.clone(),
        unit_field.clone(),
        cost_field.clone(),
        cost_unit_field.clone(),
    ];
    bounded_names(fields.to_vec(), 4)?;
    if fields
        .iter()
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != 4
        || fields
            .iter()
            .any(|name| name.trim().is_empty() || name.chars().any(char::is_control))
    {
        return Err(invalid(
            "capacity, capacity unit, cost and cost unit fields must be distinct nonblank names without controls",
        ));
    }
    if demand == 0 {
        return Err(invalid(
            "minimum-cost-flow demand must be a positive finite u64",
        ));
    }
    let source_id = entity_id(&source_target.entity_id)?;
    let sink_id = entity_id(&sink_target.entity_id)?;
    if source_id == sink_id {
        return Err(invalid(
            "minimum-cost-flow source and sink must name distinct entities",
        ));
    }

    let maximum = context
        .budget
        .max_bytes
        .min(output_bytes)
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    if maximum == 0 {
        return Err(bounded(
            "minimum-cost-flow query has no complete-response allowance",
        ));
    }
    // Complete-source structural and canonical byte allowances remain distinct
    // from the response reservation. Unselected fields and edges still count.
    let mut structural_context = context.clone();
    structural_context.budget.max_bytes = 16 * 1024 * 1024;
    let mut work = source::preflight(snapshot, &structural_context, started)?;
    if snapshot.canonical_bytes().len() as u64 > context.budget.max_bytes {
        return Err(bounded(
            "minimum-cost-flow query exceeds its canonical source byte allowance",
        ));
    }
    work.check()?;
    for (id, generation) in [
        (source_id, source_target.generation),
        (sink_id, sink_target.generation),
    ] {
        let record = snapshot.graph.entities.get(&id).ok_or_else(|| {
            invalid("minimum-cost-flow endpoint is absent from this projection; world absence is not proven")
        })?;
        if record.generation != generation {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "minimum-cost-flow endpoint generation changed; inspect its current handle",
            ));
        }
    }
    let query = MinCostFlowQuery {
        source: source_id,
        sink: sink_id,
        edge_kinds: selected_kinds,
        capacity_field: capacity_field.clone(),
        unit_field: unit_field.clone(),
        cost_field: cost_field.clone(),
        cost_unit_field: cost_unit_field.clone(),
        demand,
    };
    let graph_budget = GraphBudget {
        max_vertices: (context.budget.max_entities as usize).min(100_000),
        max_edges: (context.budget.max_entities as usize).min(1_000_000),
        max_frontier: (context.budget.max_entities as usize).min(4_096),
        max_work: 4_000_000u64.saturating_sub(work.work as u64).min(2_000_000),
    };
    let scope = Digest32::of_bytes(
        &serde_json::to_vec(&json!({
            "domain":"dfmcp-minimum-cost-flow-scope/1", "session":context.session_id.to_string(),
            "fortress":context.anchor.fortress_id.to_string(),
            "capability":"query", "scope":"whole_fortress"
        }))
        .map_err(|_| invalid("minimum-cost-flow scope cannot be encoded"))?,
    );
    let analysis = minimum_cost_flow_with_check(snapshot, scope, &query, graph_budget, || {
        work.check()?;
        check()
    })?;
    let source_structure_work = work.work;
    let graph_work = usize::try_from(analysis.witness.work_units)
        .map_err(|_| bounded("minimum-cost-flow work count cannot be represented"))?;
    work.charge(0, graph_work)?;
    let mut output_used = 0u64;
    // Grow only as admitted serialized children are retained, rather than
    // reserving space for every original edge under a tiny output allowance.
    let mut edges = Vec::new();
    for edge in &analysis.edges {
        check()?;
        let record = snapshot
            .graph
            .edges
            .get(&edge.edge_id)
            .ok_or_else(|| invariant("minimum-cost-flow edge lost its source record"))?;
        let capacity_fact = record
            .fields
            .get(&capacity_field)
            .ok_or_else(|| invariant("minimum-cost-flow edge lost its capacity fact"))?;
        let unit_fact = record
            .fields
            .get(&unit_field)
            .ok_or_else(|| invariant("minimum-cost-flow edge lost its unit fact"))?;
        let cost_fact = record
            .fields
            .get(&cost_field)
            .ok_or_else(|| invariant("minimum-cost-flow edge lost its cost fact"))?;
        let cost_unit_fact = record
            .fields
            .get(&cost_unit_field)
            .ok_or_else(|| invariant("minimum-cost-flow edge lost its cost unit fact"))?;
        let kind = match &record.kind {
            EdgeKind::Custom(name) => format!("custom:{name}"),
            kind => kind.as_str().to_owned(),
        };
        push_output(
            &mut edges,
            json!({
                "edge_id":edge.edge_id.to_string(),"revision":edge.revision,"kind":kind,
                "from":handle(snapshot, edge.from)?,"to":handle(snapshot, edge.to)?,
                "capacity":edge.capacity,"cost":edge.cost,"flow":edge.flow,
                "capacity_fact":fact_json(capacity_fact,snapshot.tick),
                "unit_fact":fact_json(unit_fact,snapshot.tick),
                "cost_fact":fact_json(cost_fact,snapshot.tick),
                "cost_unit_fact":fact_json(cost_unit_fact,snapshot.tick),
                "in_minimum_cut":analysis.minimum_cut.as_ref()
                    .is_some_and(|cut| cut.cut_edges.binary_search(&edge.edge_id).is_ok())
            }),
            &mut output_used,
            maximum,
            &mut work,
        )?;
    }
    let minimum_cut = if let Some(cut) = &analysis.minimum_cut {
        let mut partition = |ids: &[EntityId]| -> Result<Vec<Value>> {
            let mut output = Vec::new();
            for id in ids {
                check()?;
                push_output(
                    &mut output,
                    handle(snapshot, *id)?,
                    &mut output_used,
                    maximum,
                    &mut work,
                )?;
            }
            Ok(output)
        };
        let source_side = partition(&cut.source_side)?;
        let sink_side = partition(&cut.sink_side)?;
        let mut cut_edges = Vec::new();
        for edge in &cut.cut_edges {
            check()?;
            push_output(
                &mut cut_edges,
                json!(edge.to_string()),
                &mut output_used,
                maximum,
                &mut work,
            )?;
        }
        json!({"capacity":cut.cut_capacity,"edge_ids":cut_edges,
            "source_side":source_side,"sink_side":sink_side})
    } else {
        Value::Null
    };
    let mut potentials = Vec::new();
    for potential in &analysis.potentials {
        check()?;
        push_output(
            &mut potentials,
            json!({"entity":handle(snapshot, potential.entity_id)?,
                "potential":potential.potential.to_string()}),
            &mut output_used,
            maximum,
            &mut work,
        )?;
    }
    work.charge(0, 3)?; // both terminal handles and the complete envelope
    let witness = &analysis.witness;
    let payload = json!({
        "schema":"dfmcp.query.result/1","kind":"min_cost_flow","policy":MIN_COST_FLOW_POLICY,
        "anchor":anchor_json(snapshot.anchor()), "edge_kinds":edge_kinds,
        "capacity_field":capacity_field,"unit_field":unit_field,
        "cost_field":cost_field,"cost_unit_field":cost_unit_field,
        "source":handle(snapshot, analysis.source)?,"sink":handle(snapshot, analysis.sink)?,
        "demand":analysis.demand,"value":analysis.value,"unmet_demand":analysis.unmet_demand,
        "demand_met":analysis.unmet_demand == 0,
        "unit":analysis.unit,"cost_unit":analysis.cost_unit,
        "total_cost":analysis.total_cost.to_string(),
        "edges":edges,"returned_edges":edges.len(),
        "potentials":potentials,"minimum_cut":minimum_cut,
        "model":{"epistemic_state":"inferred","exact_within_projection":true,
            "direction":"stored_directed_edges","capacities":"finite_source_facts",
            "unit_established":analysis.unit.is_some(),
            "cost_unit_established":analysis.cost_unit.is_some(),
            "objective":"maximize_delivery_up_to_demand_then_minimize_total_cost",
            "costs":"nonnegative_source_facts_per_flow_unit",
            "conservation_verified":true,"residual_dual_verified":true,
            "shortfall_cut_verified":analysis.minimum_cut.is_some(),
            "actual_throughput_proven":false,"native_completion_proven":false},
        "analysis_complete":true,"truncated":false,"continuation":null,
        "native_captures":0,"mutation_dispatched":false,
        "coverage":{"domain":"observed_projection",
            "selection":"all_edges_of_selected_kinds","absence_proven":false,
            "mutation_authorized":false,"native_capacity_semantics_established":false},
        "interpretation":"The model maximizes delivery up to the requested demand, then minimizes total cost at that delivered value. Source-recorded costs are per unit of flow. All positive residual arcs have nonnegative reduced costs under the returned potentials, certifying the fixed-value cost optimum. Unmet demand additionally has an equal-capacity minimum cut. Total cost and signed potentials use exact decimal strings. These are model certificates, not native rates, worker/material feasibility, dispatch authority or goal completion. Empty selected networks establish neither unit nor complete-world absence.",
        "witness":{"anchor":anchor_json(witness.source_anchor),
            "authorization_scope_digest":witness.authorization_scope_digest.to_string(),
            "projection_digest":witness.projection_digest.to_string(),
            "decision_digest":witness.decision_digest.to_string(),
            "scanned_vertices":witness.scanned_vertices,"scanned_edges":witness.scanned_edges,
            "examined_arcs":witness.examined_arcs,"work_units":witness.work_units,
            "source_structure_work_units":source_structure_work}
    });
    if serde_json::to_vec(&payload)
        .map_err(|_| invalid("minimum-cost-flow result cannot be encoded"))?
        .len() as u64
        > maximum
    {
        return Err(bounded(
            "complete minimum-cost-flow allocation and certificate exceed the output budget; narrow the source projection or increase the negotiated allowance",
        ));
    }
    check()?;
    Ok(payload)
}

fn handle(snapshot: &WorldSnapshot, id: EntityId) -> Result<Value> {
    let entity = snapshot
        .graph
        .entities
        .get(&id)
        .ok_or_else(|| invariant("minimum-cost-flow result lost an admitted entity"))?;
    Ok(
        json!({"entity_id":id.to_string(),"generation":entity.generation,"revision":entity.revision}),
    )
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value =
        serde_json::from_str(include_str!("../../../schemas/mcp_min_cost_flow_v1.json"))
            .map_err(|_| invalid("embedded minimum-cost-flow schema is invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("query schema variants absent"))?
        .push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "min_cost_flow_query_tests.rs"]
mod tests;
