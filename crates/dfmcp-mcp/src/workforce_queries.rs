//! Whole-row workforce queries over one coherent spatial/1.8 capture.
//! Planning is read-only analysis; it never creates an executable action.
use std::collections::BTreeSet;
use dfmcp_adapter::live_spatial::{SpatialStateView, citizens::LiveSpatialCitizenState};
use dfmcp_adapter::workforce_analysis::{self as workforce, WorkforceAnalysis, WorkforceCandidate, WorkforceDemand};
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};
use dfmcp_world::inventory_allocation::MAX_WORK;
use dfmcp_world::map_region::MAX_ROUTE_WORK;
use serde::Deserialize;
use serde_json::{Value, json};
use super::anchor_json;

#[path = "production_portfolio_queries.rs"]
pub(super) mod portfolio;

fn invalid(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InvalidRequest, text) }
fn budget(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::BudgetExceeded, text) }
fn invariant(text: &str) -> DfmcpError { DfmcpError::new(ErrorCode::InternalInvariantViolation, text) }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope { schema: String, expected_anchor: Option<Value>, query: Query }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DemandInput {
    key: String, workers: u32, target: [u32; 3], skill_key: String,
    min_effective_skill: Option<i32>, preserve_social: Option<bool>, adults_only: Option<bool>,
}
impl DemandInput {
    fn normalized(self) -> WorkforceDemand {
        WorkforceDemand { key: self.key, workers: self.workers, target: self.target, skill_key: self.skill_key,
            min_effective_skill: self.min_effective_skill.unwrap_or(1),
            preserve_social: self.preserve_social.unwrap_or(true), adults_only: self.adults_only.unwrap_or(true) }
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Query {
    WorkforceCandidates { target: [u32; 3], skill_key: String, min_effective_skill: Option<i32>,
        preserve_social: Option<bool>, adults_only: Option<bool>, limit: Option<u32>,
        continuation: Option<String>, max_work: Option<u64> },
    WorkforcePlan { demands: Vec<DemandInput>, limit: Option<u32>, continuation: Option<String>, max_work: Option<u64> },
}

pub(super) fn handles(input: &Value) -> bool {
    matches!(input.get("query").and_then(|q| q.get("kind")).and_then(Value::as_str),
        Some("workforce_candidates" | "workforce_plan"))
}
fn validate_shape(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)]; let mut nodes = 0usize; let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1; bytes = bytes.saturating_add(32);
        if nodes > 4096 || depth > 16 { return Err(budget("workforce query exceeds shape bound")); }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if values.len().saturating_add(pending.len()).saturating_add(nodes) > 4096 { return Err(budget("workforce array is too wide")); }
                pending.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(values) => {
                if values.len().saturating_add(pending.len()).saturating_add(nodes) > 4096 { return Err(budget("workforce object is too wide")); }
                for (key, value) in values { bytes = bytes.saturating_add(key.len()); pending.push((value, depth + 1)); }
            }
            _ => {}
        }
        if bytes > 65_536 { return Err(budget("workforce query exceeds aggregate input bound")); }
    }
    Ok(())
}
fn demand_json(d: &WorkforceDemand) -> Value {
    json!({"key":d.key,"workers":d.workers,"target":d.target,"skill_key":d.skill_key,
        "min_effective_skill":d.min_effective_skill,"preserve_social":d.preserve_social,"adults_only":d.adults_only})
}
fn model_digest(a: &WorkforceAnalysis, kind: &str) -> Digest32 {
    Digest32::of_bytes(json!({"domain":"dfmcp-workforce-model/1","kind":kind,"policy":workforce::WORKFORCE_POLICY,
        "anchor":anchor_json(a.anchor),"source_digest":a.source_digest.to_string(),
        "demands":a.demands.iter().map(demand_json).collect::<Vec<_>>()}).to_string().as_bytes())
}
fn page_identity(context: &OperationContext, model: Digest32, maximum_work: u64) -> Digest32 {
    Digest32::of_bytes(json!({"domain":"dfmcp-workforce-page/2","session":context.session_id.to_string(),
        "anchor":anchor_json(context.anchor),"model":model.to_string(),"max_work":maximum_work}).to_string().as_bytes())
}
fn token(prefix: &str, offset: usize, identity: Digest32) -> String {
    let value = json!({"domain":"dfmcp-workforce-continuation/2","prefix":prefix,"offset":offset,"identity":identity.to_string()});
    format!("{prefix}:{offset}:{}", Digest32::of_bytes(value.to_string().as_bytes()))
}
fn page_offset(raw: Option<&str>, prefix: &str, identity: Digest32, count: usize) -> Result<usize> {
    let Some(raw) = raw else { return Ok(0); };
    if raw.len() > 128 { return Err(budget("workforce continuation exceeds 128 bytes")); }
    let parts: Vec<_> = raw.split(':').collect();
    if parts.len() != 3 || parts[0] != prefix || parts[1].is_empty() || parts[1].starts_with('0')
        || !parts[1].bytes().all(|b| b.is_ascii_digit()) { return Err(invalid("invalid workforce continuation")); }
    let offset = parts[1].parse::<usize>().map_err(|_| invalid("workforce continuation offset overflow"))?;
    if raw != token(prefix, offset, identity) { return Err(DfmcpError::new(ErrorCode::StaleAnchor, "workforce continuation names another session, capture or model")); }
    if offset >= count { return Err(DfmcpError::new(ErrorCode::CursorGap, "workforce continuation is past the result set")); }
    Ok(offset)
}
struct Page<'a> { limit: u32, continuation: Option<&'a str>, prefix: &'a str, identity: Digest32 }
fn paginate(mut out: Value, count: usize, context: &OperationContext, page: Page<'_>,
    row: impl Fn(usize) -> Result<Value>) -> Result<Value> {
    if !(1..=128).contains(&page.limit) { return Err(budget("workforce page limit must be 1..128")); }
    let limit = page.limit.min(context.budget.max_entities) as usize;
    let maximum = usize::try_from(context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4))
        .map_err(|_| budget("workforce response allowance overflow"))?;
    let start = page_offset(page.continuation, page.prefix, page.identity, count)?;
    out["rows"] = json!([]); out["returned"] = json!(0); out["total_rows"] = json!(count);
    out["truncated"] = json!(false); out["continuation"] = Value::Null;
    if out.to_string().len() > maximum { return Err(budget("complete workforce summary does not fit the response allowance")); }
    let mut end = start;
    while end < count && end - start < limit {
        let mut candidate = out.clone();
        candidate["rows"].as_array_mut().ok_or_else(|| invariant("workforce rows absent"))?.push(row(end)?);
        candidate["returned"] = json!(end + 1 - start); candidate["truncated"] = json!(end + 1 < count);
        candidate["continuation"] = if end + 1 < count { json!(token(page.prefix, end + 1, page.identity)) } else { Value::Null };
        if candidate.to_string().len() > maximum { break; }
        out = candidate; end += 1;
    }
    if start < count && end == start { return Err(budget("one complete workforce row and summary cannot fit")); }
    Ok(out)
}
fn base(a: &WorkforceAnalysis, kind: &str, model: Digest32) -> Value {
    json!({"schema":"dfmcp.query.result/1","kind":kind,"anchor":anchor_json(a.anchor),
        "source_digest":a.source_digest.to_string(),"model_digest":model.to_string(),"candidate_policy":workforce::WORKFORCE_POLICY,
        "unit_path_proven":false,"labor_eligibility_proven":false,"job_readiness_proven":false,"safety_proven":false,
        "reservations_created":false,"mutation_dispatched":false,"native_captures":0,"skill_registry_complete":false,
        "interpretation":"Read-only allocation under declared skill minima, observed job availability and bounded terrain approaches. Not native job eligibility, unit pathfinding, a work schedule or permission to assign labor."})
}
fn candidate_row(state: &LiveSpatialCitizenState, a: &WorkforceAnalysis, demand: usize, candidate: &WorkforceCandidate) -> Result<Value> {
    let observation = state.observation_full().ok_or_else(|| invariant("workforce source absent"))?;
    let citizen = observation.citizens().get(candidate.citizen_index).ok_or_else(|| invariant("workforce citizen index lost"))?;
    let target = a.demands[demand].target;
    Ok(json!({"citizen":{"entity_id":candidate.entity_id.to_string(),"generation":candidate.generation,"revision":candidate.revision},
        "name":citizen.name,"profession":citizen.profession,"position":[citizen.position.x,citizen.position.y,citizen.position.z],
        "stress_category":citizen.stress_category,"job_available_preserve_social":citizen.job_available_preserve_social,
        "job_available_interrupt_social":citizen.job_available_interrupt_social,
        "skill":{"key":a.demands[demand].skill_key,"nominal":candidate.nominal,"effective":candidate.effective,"experience":candidate.experience},
        "candidate_steps":candidate.steps,"approach_tile":candidate.approach,"unit_endpoint_step_modeled":candidate.endpoint_step,
        "route_query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(a.anchor),
            "query":{"kind":"map_route","start":target,"goal":candidate.approach}}}))
}

pub(super) fn execute(state: &LiveSpatialCitizenState, context: &OperationContext, input: &Value) -> Result<Value> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone()).map_err(|_| invalid("invalid workforce query fields"))?;
    if envelope.schema != "dfmcp.query/1" { return Err(invalid("workforce query requires dfmcp.query/1")); }
    if envelope.expected_anchor.as_ref().is_some_and(|a| a != &anchor_json(context.anchor))
        || state.snapshot().map(|s| s.anchor()) != Some(context.anchor) {
        return Err(DfmcpError::new(ErrorCode::StaleAnchor, "workforce query anchor differs"));
    }
    match envelope.query {
        Query::WorkforceCandidates { target, skill_key, min_effective_skill, preserve_social, adults_only, limit, continuation, max_work } => {
            let maximum = max_work.unwrap_or(MAX_ROUTE_WORK);
            if maximum == 0 || maximum > MAX_ROUTE_WORK { return Err(budget("candidate max_work exceeds route allowance")); }
            let demand = WorkforceDemand { key: "candidate".into(), workers: 1, target, skill_key,
                min_effective_skill: min_effective_skill.unwrap_or(0), preserve_social: preserve_social.unwrap_or(true),
                adults_only: adults_only.unwrap_or(true) };
            let a = workforce::analyze(state, context, &[demand], maximum)?;
            let model = model_digest(&a, "workforce_candidates"); let identity = page_identity(context, model, maximum);
            let mut out = base(&a, "workforce_candidates", model);
            out["request"] = demand_json(&a.demands[0]); out["classification"] = json!(a.classifications[0]);
            out["skill_key_observed_in_capture"] = json!(a.skill_key_observed[0]);
            out["ordering"] = json!("effective_desc_nominal_desc_steps_asc_entity_id_asc");
            out["reachable_tiles"] = json!(a.reachable_tiles[0]); out["touched_region_boundary"] = json!(a.touched_region_boundary[0]);
            out["work_units"] = json!(a.work_units);
            paginate(out, a.candidates[0].len(), context, Page { limit: limit.unwrap_or(16), continuation: continuation.as_deref(), prefix: "wc2", identity },
                |i| candidate_row(state, &a, 0, &a.candidates[0][i]))
        }
        Query::WorkforcePlan { demands, limit, continuation, max_work } => {
            let maximum = max_work.unwrap_or(MAX_WORK);
            let requested: Vec<_> = demands.into_iter().map(DemandInput::normalized).collect();
            let planned = workforce::plan(state, context, &requested, maximum)?;
            let a = &planned.analysis; let allocation = &planned.allocation;
            let model = model_digest(a, "workforce_plan"); let identity = page_identity(context, model, maximum);
            let mut out = base(a, "workforce_plan", model);
            out["model_feasible"] = json!(allocation.allocated_units == allocation.requested_units);
            out["requested_workers"] = json!(allocation.requested_units); out["assigned_workers"] = json!(allocation.allocated_units);
            out["worker_capacity"] = json!(1); out["cut_capacity"] = json!(allocation.cut_capacity); out["work_units"] = json!(planned.work_units);
            out["optimization"] = json!("maximum_filled_slots_only; deterministic eligibility-mask/demand-key/citizen-id ties; no global skill, distance or priority optimum");
            out["demands"] = json!(a.demands.iter().enumerate().map(|(i,d)| json!({"request":demand_json(d),
                "assigned_workers":allocation.allocated_by_demand[i],"unfilled_workers":u64::from(d.workers)-allocation.allocated_by_demand[i],
                "candidate_count":a.candidates[i].len(),"classification":a.classifications[i],
                "skill_key_observed_in_capture":a.skill_key_observed[i],"reachable_tiles":a.reachable_tiles[i],
                "touched_region_boundary":a.touched_region_boundary[i]})).collect::<Vec<_>>());
            out["shortage"] = match &allocation.shortage {
                None => Value::Null,
                Some(shortage) => {
                    let neighbors: BTreeSet<_> = shortage.demand_indices.iter()
                        .flat_map(|&i| a.candidates[i].iter().map(|c| c.entity_id)).collect();
                    if neighbors.len() as u64 != shortage.eligible_units { return Err(invariant("workforce shortage neighbors disagree")); }
                    json!({"kind":"observed_model_hall_deficiency","demand_keys":shortage.demand_indices.iter().map(|&i| &a.demands[i].key).collect::<Vec<_>>(),
                        "required_workers":shortage.required_units,"distinct_candidate_workers":shortage.eligible_units,"deficit":shortage.deficit,
                        "candidate_entity_ids":neighbors.iter().map(u64::to_string).collect::<Vec<_>>(),
                        "global_workforce_shortage_proven":false,"independently_additive":false})
                }
            };
            paginate(out, allocation.assignments.len(), context, Page { limit: limit.unwrap_or(16), continuation: continuation.as_deref(), prefix: "wp1", identity }, |i| {
                let assignment = &allocation.assignments[i];
                let candidate = a.candidates[assignment.demand_index].iter().find(|c| c.entity_id == assignment.supply_id)
                    .ok_or_else(|| invariant("assigned citizen lacks candidate evidence"))?;
                let mut row = candidate_row(state, a, assignment.demand_index, candidate)?;
                row["demand_key"] = json!(a.demands[assignment.demand_index].key); row["worker_slots"] = json!(1); Ok(row)
            })
        }
    }
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    for source in [include_str!("../../../schemas/mcp_workforce_candidates_v1.json"), include_str!("../../../schemas/mcp_workforce_plan_v1.json")] {
        let variant: Value = serde_json::from_str(source).map_err(|_| invariant("embedded workforce schema invalid"))?;
        schema["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(|| invariant("query variants absent"))?.push(variant);
    }
    Ok(schema)
}

#[cfg(test)]
#[path = "workforce_queries_tests.rs"]
mod tests;
