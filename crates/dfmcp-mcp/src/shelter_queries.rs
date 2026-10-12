//! Named physical arrivals over the existing coherent native citizen owner.
//! The summary covers the complete original selection, not just returned rows.
use std::time::Instant;

use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_adapter::shelter_analysis::{self as shelter, CitizenTarget, ShelterSelection};
use dfmcp_core::{Capability, DfmcpError, Digest32, EntityId, ErrorCode, MapCoord, MapCuboid,
    OperationContext, Result, RiskTier};
use dfmcp_world::PredicateTruth;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Page, anchor_json, budget, invalid, page_identity, paginate, validate_shape};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema: String,
    expected_anchor: Option<Value>,
    query: Request,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target { entity_id: String, generation: u32 }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Area { min: [i32; 3], max: [i32; 3] }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    kind: String,
    citizens: Vec<Target>,
    shelters: Vec<Area>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_work: Option<u64>,
}
fn entity(raw: &str) -> Result<EntityId> {
    if raw.is_empty() || raw.len() > 20 || raw.starts_with('0')
        || !raw.bytes().all(|b| b.is_ascii_digit())
    { return Err(invalid("shelter entity IDs require canonical decimal strings")); }
    raw.parse::<u64>().map(EntityId::new).map_err(|_| invalid("shelter entity ID exceeds u64"))
}
fn coord(p: [i32; 3]) -> MapCoord { MapCoord::new(p[0], p[1], p[2]) }
fn coordinates(p: MapCoord) -> [i32; 3] { [p.x, p.y, p.z] }
fn truth(value: &PredicateTruth) -> &'static str {
    match value { PredicateTruth::True => "true", PredicateTruth::False => "false", PredicateTruth::Unknown => "unknown" }
}

pub(super) fn execute(state: &LiveSpatialCitizenState, context: &OperationContext, input: &Value) -> Result<Value> {
    let started = Instant::now();
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid shelter_status query fields"))?;
    if envelope.schema != "dfmcp.query/1" || envelope.query.kind != "shelter_status" {
        return Err(invalid("shelter_status requires dfmcp.query/1"));
    }
    if envelope.expected_anchor.as_ref().is_some_and(|a| a != &anchor_json(context.anchor))
        || state.snapshot().map(|s| s.anchor()) != Some(context.anchor)
    {
        return Err(DfmcpError::new(ErrorCode::StaleAnchor, "shelter query names another capture"));
    }
    let request = envelope.query;
    let limit = request.limit.unwrap_or(16);
    if !(1..=128).contains(&limit) { return Err(budget("shelter row limit must be 1..128")); }
    if request.citizens.len() > shelter::MAX_CITIZENS || request.shelters.len() > shelter::MAX_SHELTERS {
        return Err(budget("shelter selection exceeds fixed bounds"));
    }
    let citizens = request.citizens.iter().map(|target| Ok(CitizenTarget {
        entity_id: entity(&target.entity_id)?, generation: target.generation,
    })).collect::<Result<Vec<_>>>()?;
    let regions = request.shelters.iter().map(|area| MapCuboid::new(coord(area.min), coord(area.max)))
        .collect::<Result<Vec<_>>>()?;
    let selection = ShelterSelection::new(&citizens, &regions)?;
    let maximum = request.max_work.unwrap_or(shelter::MAX_WORK);
    let mut narrowed = context.clone();
    let elapsed = u64::try_from(started.elapsed().as_millis()).map_err(|_| budget("shelter wall overflow"))?;
    narrowed.budget.max_wall_millis = context.budget.max_wall_millis.checked_sub(elapsed)
        .filter(|left| *left > 0).ok_or_else(|| budget("shelter wall allowance exhausted"))?;
    let report = shelter::assess(state, &narrowed, &selection, maximum)?;
    let model = Digest32::of_bytes(json!({"domain":"dfmcp-shelter-status-model/1",
        "policy":shelter::POLICY,"selection":selection.digest().to_string(),
        "anchor":anchor_json(report.anchor),"source_digest":report.source_digest.to_string()})
        .to_string().as_bytes());
    let identity = page_identity(context, model, maximum);
    let out = json!({
        "schema":"dfmcp.query.result/1","kind":"shelter_status",
        "anchor":anchor_json(report.anchor),"source_digest":report.source_digest.to_string(),
        "bridge_generation":report.bridge_generation,"source_profile":"spatial/1.8",
        "model_digest":model.to_string(),"selection_digest":selection.digest().to_string(),
        "policy":shelter::POLICY,"selected_citizens":selection.citizens().len(),
        "shelters":selection.shelters().iter().map(|area| json!({"min":coordinates(area.min),"max":coordinates(area.max)})).collect::<Vec<_>>(),
        "arrival_truth":truth(&report.arrival_truth),
        "all_named_inside_at_capture":report.arrival_truth == PredicateTruth::True,
        "counts":{"inside":report.inside,"outside":report.outside,"not_alive":report.not_alive,"unresolved":report.unresolved},
        "assessment_complete":true,"complete_fortress_population_selected":false,
        "identity_scope":"current_spatial_session","restart_continuity_proven":false,
        "ordering":"original_entity_id_ascending","work_units":report.work_units,
        "unit_path_proven":false,"safety_proven":false,"burrow_membership_proven":false,
        "evacuation_execution_proven":false,"continuous_presence_proven":false,
        "mutation_dispatched":false,"native_captures":0,"control_authority_granted":false,
        "interpretation":"Named living citizen incarnations inside declared geometry at this capture only. Not safe shelter, native burrow membership, a route, stable presence, evacuation causality or permission to release restrictions."
    });
    let out = paginate(out, report.rows.len(), context,
        Page { limit, continuation: request.continuation.as_deref(), prefix: "ss1", identity },
        |index| {
            let row = &report.rows[index];
            Ok(json!({"citizen":{"entity_id":row.target.entity_id.to_string(),"generation":row.target.generation},
                "observed_generation":row.observed_generation,"observed_revision":row.observed_revision,
                "status":row.status.as_str(),"arrival_truth":truth(&row.status.truth()),
                "position":row.position.map(coordinates),"shelter_index":row.shelter_index}))
        })?;
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(budget("shelter response exhausted the original wall allowance"));
    }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(out)
}

#[cfg(test)]
#[path = "shelter_queries_tests.rs"]
mod tests;
