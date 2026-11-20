//! Cost-aware multi-source routes over one exact authorized graph projection.
//! These are advisory graph paths, never native walkability or dispatch proof.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
use dfmcp_world::WorldSnapshot;
use dfmcp_world::graph_query::{
    GraphBudget, GraphDirection, WEIGHTED_ROUTE_POLICY, WeightedRouteQuery,
    shortest_routes_with_check,
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
    WeightedRoutes {
        roots: Vec<EntityRef>,
        edge_kinds: Vec<String>,
        cost_field: String,
        #[serde(default)]
        direction: Direction,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        targets: Vec<EntityRef>,
    },
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Direction {
    #[default]
    Outgoing,
    Incoming,
}

impl Direction {
    fn graph(self) -> GraphDirection {
        match self {
            Self::Outgoing => GraphDirection::Outgoing,
            Self::Incoming => GraphDirection::Incoming,
        }
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct EntityRef {
    entity_id: String,
    generation: u32,
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn bounded(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn resolve_refs(
    snapshot: &WorldSnapshot,
    selected: Vec<EntityRef>,
) -> Result<BTreeMap<EntityId, Value>> {
    let mut records = BTreeMap::new();
    for selected in selected {
        let id = entity_id(&selected.entity_id)?;
        let entity = snapshot.graph.entities.get(&id).ok_or_else(|| {
            invalid("route endpoint is absent from this projection; world absence is not proven")
        })?;
        if selected.generation != entity.generation {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "route endpoint generation changed; inspect its current handle",
            ));
        }
        if records
            .insert(
                id,
                json!({"entity_id":id.to_string(),"generation":entity.generation,
                    "revision":entity.revision}),
            )
            .is_some()
        {
            return Err(invalid("route endpoint lists must name distinct entities"));
        }
    }
    Ok(records)
}

/// Count rendered pieces before retaining them. The final full-object check
/// also charges framing and metadata; neither path may truncate a result.
fn charge_output(value: &Value, consumed: &mut u64, maximum: u64) -> Result<()> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| invalid("weighted route result cannot be encoded"))?
        .len() as u64;
    *consumed = consumed.saturating_add(bytes);
    if *consumed > maximum {
        return Err(bounded(
            "complete weighted routes exceed the output budget; select fewer targets or increase the negotiated allowance",
        ));
    }
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

/// The laboratory owner selects and retains the source. Historical source time
/// never renews current grants or the original foreground deadline.
pub(crate) fn execute_laboratory_source(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    input: &Value,
    remaining_wall_millis: u64,
) -> Result<Value> {
    if remaining_wall_millis == 0 {
        return Err(bounded(
            "weighted routes exhausted the original foreground deadline",
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
                "weighted routes exceeded their foreground deadline",
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
            "weighted route context names another source",
        ));
    }
    validate_input(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid weighted route query envelope"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("weighted routes require dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|anchor| anchor != &anchor_json(snapshot.anchor()))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "weighted routes expected_anchor differs from the complete source anchor",
        ));
    }
    let Request::WeightedRoutes {
        roots,
        edge_kinds,
        cost_field,
        direction,
        targets,
    } = envelope.query;
    if roots.is_empty() || roots.len() > 64 || targets.len() > 64 || edge_kinds.is_empty() {
        return Err(invalid(
            "weighted routes require 1..64 roots, explicit edge kinds and at most 64 targets",
        ));
    }
    let edge_kinds = bounded_names(edge_kinds, 32)?;
    bounded_names(vec![cost_field.clone()], 1)?;
    let kinds = edge_kinds
        .iter()
        .map(|kind| edge_kind(kind))
        .collect::<Result<_>>()?;

    let mut structural_context = context.clone();
    structural_context.budget.max_bytes = 16 * 1024 * 1024;
    let work = source::preflight(snapshot, &structural_context, started)?;
    if snapshot.canonical_bytes().len() as u64 > context.budget.max_bytes {
        return Err(bounded(
            "weighted routes exceed the canonical source byte allowance",
        ));
    }
    work.check()?;
    let roots = resolve_refs(snapshot, roots)?;
    let targets = resolve_refs(snapshot, targets)?;
    let query = WeightedRouteQuery {
        roots: roots.keys().copied().collect(),
        edge_kinds: kinds,
        direction: direction.graph(),
        cost_field: cost_field.clone(),
    };
    let graph_budget = GraphBudget {
        max_vertices: (context.budget.max_entities as usize).min(100_000),
        max_edges: (context.budget.max_entities as usize).min(1_000_000),
        max_frontier: (context.budget.max_entities as usize).min(4_096),
        max_work: 4_000_000u64.saturating_sub(work.work as u64).min(2_000_000),
    };
    let scope = Digest32::of_bytes(
        &serde_json::to_vec(&json!({
            "domain":"dfmcp-weighted-route-scope/1", "session":context.session_id.to_string(),
            "fortress":context.anchor.fortress_id.to_string(),
            "capability":"query", "scope":"whole_fortress"
        }))
        .map_err(|_| invalid("weighted route scope cannot be encoded"))?,
    );
    let analysis = shortest_routes_with_check(snapshot, scope, &query, graph_budget, || {
        work.check()?;
        check()
    })?;
    let maximum = context
        .budget
        .max_bytes
        .min(output_bytes)
        .min(u64::from(context.budget.max_output_tokens).saturating_mul(4));
    let mut output_used = 0;
    let mut render_work = (work.work as u64).saturating_add(analysis.witness.work_units);
    let mut entries = Vec::new();
    let mut target_path_vertices = 0u64;
    let selected: BTreeSet<_> = targets.keys().copied().collect();
    // Retrieving cost evidence searches both the source edge map and its
    // field map. Complete source work bounds every field-map cardinality.
    let edge_lookup_work = u64::from(usize::BITS - snapshot.graph.edges.len().leading_zeros());
    let field_lookup_work = u64::from(usize::BITS - work.work.leading_zeros());
    let edge_fact_lookup_work = edge_lookup_work + field_lookup_work;
    for entry in &analysis.entries {
        check()?;
        render_work = render_work.saturating_add(1);
        if render_work > 4_000_000 {
            return Err(bounded(
                "weighted route presentation exceeds its work allowance",
            ));
        }
        if !selected.is_empty() && !selected.contains(&entry.entity_id) {
            continue;
        }
        if entry.via_edge.is_some() {
            render_work = render_work.saturating_add(edge_fact_lookup_work);
            if render_work > 4_000_000 {
                return Err(bounded(
                    "weighted route cost evidence exceeds its work allowance",
                ));
            }
        }
        if !selected.is_empty() {
            target_path_vertices = target_path_vertices.saturating_add(u64::from(entry.hops) + 1);
        }
        let cost_fact = entry
            .via_edge
            .map(|id| {
                snapshot
                    .graph
                    .edges
                    .get(&id)
                    .and_then(|edge| edge.fields.get(&cost_field))
                    .map(|fact| fact_json(fact, snapshot.tick))
                    .ok_or_else(|| {
                        DfmcpError::new(
                            ErrorCode::InternalInvariantViolation,
                            "weighted route lost its source edge cost",
                        )
                    })
            })
            .transpose()?;
        let row = json!({"entity_id":entry.entity_id.to_string(),
            "generation":entry.generation,"revision":entry.revision,
            "root":entry.root.to_string(),"total_cost":entry.total_cost,"hops":entry.hops,
            "parent":entry.parent.map(|id|id.to_string()),
            "via_edge":entry.via_edge.map(|id|id.to_string()),
            "via_edge_revision":entry.via_edge_revision,"edge_cost":entry.edge_cost,
            "edge_cost_fact":cost_fact});
        charge_output(&row, &mut output_used, maximum)?;
        entries.push(row);
    }
    let mut unreachable = Vec::new();
    for id in &analysis.unreachable {
        check()?;
        render_work = render_work.saturating_add(1);
        if render_work > 4_000_000 {
            return Err(bounded(
                "weighted route presentation exceeds its work allowance",
            ));
        }
        if selected.is_empty() || selected.contains(id) {
            let value = json!(id.to_string());
            charge_output(&value, &mut output_used, maximum)?;
            unreachable.push(value);
        }
    }
    let mut paths = Vec::new();
    // Batch reconstruction validates one shared forest index. Admit all path
    // expansion before allocating its vertex and edge vectors, and retain one
    // deadline and one presentation allowance for the whole request.
    if !targets.is_empty() {
        let lookup_work = u64::from(usize::BITS - analysis.entries.len().leading_zeros());
        render_work = render_work
            .saturating_add(analysis.entries.len() as u64)
            .saturating_add(targets.len() as u64)
            .saturating_add(
                target_path_vertices.saturating_mul(2 + lookup_work + edge_fact_lookup_work),
            )
            .saturating_add((roots.len() as u64).saturating_mul(targets.len() as u64));
        if render_work > 4_000_000 || target_path_vertices > maximum / 3 {
            return Err(bounded(
                "weighted target paths exceed their shared work or output allowance",
            ));
        }
    }
    let target_ids: Vec<_> = targets.keys().copied().collect();
    let target_paths = if target_ids.is_empty() {
        Vec::new()
    } else {
        analysis.paths_to_with_check(&target_ids, || {
            work.check()?;
            check()
        })?
    };
    check()?;
    for ((_, target), path) in targets.iter().zip(target_paths) {
        check()?;
        let row = if let Some(path) = path {
            let mut edges = Vec::new();
            let mut path_output = output_used;
            for (edge_id, revision, cost) in &path.edges {
                check()?;
                let fact = snapshot
                    .graph
                    .edges
                    .get(edge_id)
                    .and_then(|edge| edge.fields.get(&cost_field))
                    .ok_or_else(|| {
                        DfmcpError::new(
                            ErrorCode::InternalInvariantViolation,
                            "weighted target path lost its source edge cost",
                        )
                    })?;
                let edge = json!({"edge_id":edge_id.to_string(),"revision":revision,
                    "cost":cost,"cost_fact":fact_json(fact,snapshot.tick)});
                charge_output(&edge, &mut path_output, maximum)?;
                edges.push(edge);
            }
            json!({"target":target,"status":"reachable", "root":path.root.to_string(),
                "total_cost":path.total_cost,
                "vertices":path.vertices.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "edges":edges})
        } else {
            json!({"target":target,"status":"unreachable_in_projection","root":null,
                "total_cost":null,"vertices":[],"edges":[]})
        };
        charge_output(&row, &mut output_used, maximum)?;
        paths.push(row);
    }
    let witness = &analysis.witness;
    let payload = json!({
        "schema":"dfmcp.query.result/1", "kind":"weighted_routes",
        "analysis_schema":"dfmcp.weighted-routes/1",
        "anchor":anchor_json(snapshot.anchor()), "policy":WEIGHTED_ROUTE_POLICY,
        "edge_kinds":edge_kinds, "cost_field":cost_field, "direction":direction,
        "roots":roots.into_values().collect::<Vec<_>>(),
        "targets":targets.into_values().collect::<Vec<_>>(),
        "model":{"epistemic_state":"inferred", "cost_unit":"selected_edge_field_units",
            "algorithm_result":"exact_within_projection", "native_walkability_proven":false,
            "resource_feasibility_proven":false},
        "entries":entries,"returned":entries.len(),"unreachable":unreachable,"paths":paths,
        "reachable_vertices":analysis.entries.len(),"unreachable_vertices":analysis.unreachable.len(),
        "analysis_complete":true,"truncated":false,"continuation":null,
        "native_captures":0,"mutation_dispatched":false,
        "coverage":{"domain":"observed_projection","selection":
            if selected.is_empty(){"complete_shortest_path_forest"}else{"explicit_target_paths"},
            "all_selected_edge_costs_qualified":true,"absence_proven":false,
            "mutation_authorized":false},
        "interpretation":"Minimum summed nonnegative cost in the selected edge field's units. Incoming paths follow recorded edges in reverse. Unreachable means no path in this exact projection; unobserved routes, native walkability, hazards and actual travel time are not established.",
        "witness":{"anchor":anchor_json(witness.source_anchor),
            "authorization_scope_digest":witness.authorization_scope_digest.to_string(),
            "projection_digest":witness.projection_digest.to_string(),
            "decision_digest":witness.decision_digest.to_string(),
            "scanned_vertices":witness.scanned_vertices,"scanned_edges":witness.scanned_edges,
            "examined_arcs":witness.examined_arcs,"work_units":witness.work_units,
            "source_structure_work_units":work.work,"total_charged_work_units":render_work}
    });
    let mut complete_output = 0;
    charge_output(&payload, &mut complete_output, maximum)?;
    check()?;
    Ok(payload)
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension: Value =
        serde_json::from_str(include_str!("../../../schemas/mcp_weighted_routes_v1.json"))
            .map_err(|_| invalid("embedded weighted routes schema is invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invalid("query schema variants absent"))?
        .push(extension);
    Ok(schema)
}

#[cfg(test)]
#[path = "weighted_route_query_tests.rs"]
mod tests;
