//! Construction diagnosis and explicit generation-bound watch proposals. This
//! handler borrows the existing coherent state; it performs no native operation,
//! watch registration, clock control, effect reconciliation or receipt import.
use super::{anchor_json, base, budget, identity, invalid, paginate};
use dfmcp_adapter::construction_progress::{self as progress, Report, Row, Status, Target};
use dfmcp_adapter::operations_analysis::{AnalysisHandle, OperationsStateView};
use dfmcp_core::{Capability, Digest32, MapCoord, OperationContext, Result, RiskTier};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    targets: Vec<TargetInput>,
    limit: Option<u32>,
    continuation: Option<String>,
    max_work: Option<u64>,
    monitor: Option<Monitor>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TargetInput {
    building_native_id: u32,
    expected_generation: Option<u32>,
    expected_type: Option<String>,
    item_native_id: Option<u32>,
}
impl From<TargetInput> for Target {
    fn from(v: TargetInput) -> Self {
        Self {
            building_native_id: v.building_native_id,
            expected_generation: v.expected_generation,
            expected_type: v.expected_type,
            item_native_id: v.item_native_id,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Monitor {
    key_prefix: String,
    deadline_tick: u64,
    poll_interval_ticks: Option<u64>,
    stable_observations: Option<u32>,
}
#[derive(Serialize)]
struct Monitoring {
    key_prefix: String,
    deadline_tick: u64,
    poll_interval_ticks: u64,
    stable_observations: u32,
}
impl Monitor {
    fn validate(self, context: &OperationContext) -> Result<Monitoring> {
        let cadence = self.poll_interval_ticks.unwrap_or(1);
        let stability = self.stable_observations.unwrap_or(2);
        let horizon = self.deadline_tick.checked_sub(context.anchor.tick.0);
        if self.key_prefix.is_empty() || self.key_prefix.len() > 32
            || !self.key_prefix.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
            || !(1..=1_000_000).contains(&cadence) || !(1..=64).contains(&stability)
        {
            return Err(invalid("invalid construction monitor key, cadence or stability"));
        }
        if horizon.is_none_or(|ticks| ticks == 0 || ticks > context.budget.max_game_ticks
            || u64::from(stability - 1) * cadence > ticks)
        {
            return Err(invalid("construction monitor cannot fit the future negotiated horizon"));
        }
        Ok(Monitoring {
            key_prefix: self.key_prefix, deadline_tick: self.deadline_tick,
            poll_interval_ticks: cadence, stable_observations: stability,
        })
    }
}
fn handle(v: AnalysisHandle) -> Value {
    json!({"entity_id":v.entity_id.to_string(),"generation":v.generation,"revision":v.revision})
}
fn position(p: MapCoord) -> [i32; 3] {
    [p.x, p.y, p.z]
}
fn selected(target: &Target) -> Value {
    json!({"building_native_id":target.building_native_id,
        "expected_generation":target.expected_generation,"expected_type":target.expected_type,
        "item_native_id":target.item_native_id})
}
fn literal(kind: &str, value: Value) -> Value {
    json!({"type":kind,"value":value})
}
fn field(root: AnalysisHandle, name: &str, value: Value) -> Value {
    json!({"op":"field","entity_id":root.entity_id.to_string(),"generation":root.generation,
        "field":name,"comparison":"eq","value":value})
}
fn row_field(name: &str, value: Value) -> Value {
    json!({"op":"field","field":name,"comparison":"eq","value":value})
}
fn related(root: AnalysisHandle, relation: &str, direction: &str) -> Value {
    json!({"op":"related","entity_id":root.entity_id.to_string(),"generation":root.generation,
        "relation":relation,"direction":direction})
}
fn count(kind: &str, predicate: Value, value: u64) -> Value {
    json!({"op":"entity_count","scope":"observed_projection","kind":kind,
        "predicate":predicate,"comparison":"eq","value":value})
}
fn all(args: Vec<Value>) -> Value {
    json!({"op":"all","args":args})
}
fn job_selection(root: AnalysisHandle, removal_only: bool) -> Value {
    let kind = row_field("type_key", literal("text", json!("DestroyBuilding")));
    let kind = if removal_only { kind } else {
        json!({"op":"any","args":[kind,
            row_field("type_key",literal("text",json!("ConstructBuilding")))]})
    };
    all(vec![related(root, "contained_in", "incoming"), kind])
}

/// Emit the existing watch language, not a second monitoring state machine.
/// Pin the observed maximum stage and generations; a new max/type does not
/// silently change the goal. Missing roots remain unknown in the shared engine.
fn proposal(row: &Row, c: &OperationContext, options: &Monitoring) -> Result<Value> {
    let unavailable = |why: &str| json!({"available":false,"reason":why,"watch_registered":false});
    if matches!(row.status, Status::Missing | Status::IdentityMismatch | Status::Unsupported) {
        return Ok(unavailable("building_identity_or_supported_stage_unestablished"));
    }
    let root = row.handle.ok_or_else(|| invalid("construction row identity missing"))?;
    let kind = row.type_key.as_deref().ok_or_else(|| invalid("construction kind missing"))?;
    let maximum = row.maximum_stage.ok_or_else(|| invalid("construction maximum stage missing"))?;
    let mut conditions = vec![
        field(root, "type_key", literal("text", json!(kind))),
        field(root, "build_stage", literal("i64", json!(maximum))),
        field(root, "max_build_stage", literal("i64", json!(maximum))),
        count("job", job_selection(root, false), 0),
    ];
    if let Some(item) = &row.item {
        let Some(item_handle) = item.handle else {
            return Ok(unavailable("exact_item_identity_unestablished"));
        };
        let item_kind = match kind {
            "Bed" => "BED", "Chair" => "CHAIR", "Table" => "TABLE",
            _ => return Err(invalid("unsupported construction kind")),
        };
        conditions.push(field(item_handle, "type_key", literal("text", json!(item_kind))));
        for (name, required) in [("in_building", true), ("in_job", false), ("removed", false),
            ("on_ground", false), ("in_inventory", false)] {
            conditions.push(field(item_handle, name, literal("bool", json!(required))));
        }
        // Exactly this item must point to this building. The direct item fields
        // above bind its generation even when the population query is empty.
        conditions.push(count("item", all(vec![
            row_field("native_item_id", literal("u64", json!(item.native_id))),
            related(root, "contained_in", "incoming"),
        ]), 1));
        // An item parent is a container; the expected building parent is not.
        conditions.push(count("item", related(item_handle, "contained_in", "outgoing"), 0));
        conditions.push(count("job", related(item_handle, "uses", "incoming"), 0));
    }
    let mut failure = count("job", job_selection(root, true), 0);
    failure["comparison"] = json!("gt");
    let key = format!("{}.{}", options.key_prefix, row.target.building_native_id);
    let request = json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(c.anchor),
        "query":{"kind":"watch","key":key,"label":format!("{} #{} construction condition",kind,row.target.building_native_id),
            "condition":all(conditions),"failure_condition":failure,
            "deadline_tick":options.deadline_tick,"poll_interval_ticks":options.poll_interval_ticks,
            "stable_observations":options.stable_observations}});
    let bytes = serde_json::to_vec(&json!({"policy":progress::POLICY,
        "session_id":c.session_id.to_string(),"request":request}))
        .map_err(|_| invalid("construction proposal encoding failed"))?;
    Ok(json!({"available":true,"proposal_digest":Digest32::of_bytes(&bytes).to_string(),
        "policy":progress::POLICY,"tool":"fortress.query","session_id":c.session_id.to_string(),
        "watch_request":request,"watch_registered":false,"placement_receipt_verified":false,
        "interpretation":"Submit this watch_request explicitly in this session. It pins entity generations and the observed maximum stage, and fails on an observed removal job. It does not prove original placement identity, footprint, usability, causality or native effect completion."}))
}
fn row_json(row: &Row, monitoring: Option<Value>) -> Value {
    let jobs: Vec<_> = row.jobs.examples.iter().map(|job| json!({"native_id":job.native_id,
        "handle":handle(job.handle),"type_key":job.type_key,"suspended":job.suspended,
        "worker_native_id":job.worker_native_id,"completion_timer":job.completion_timer,
        "attached_items":job.attached_items})).collect();
    let total = row.jobs.construction + row.jobs.removal + row.jobs.other;
    let mut result = json!({"selection":selected(&row.target),"building":row.handle.map(handle),
        "type_key":row.type_key,"min":row.min.map(position),"max":row.max.map(position),
        "status":row.status.as_str(),"build_stage":row.stage,"max_build_stage":row.maximum_stage,
        "stage_complete_at_observation":row.stage_complete,
        "condition_met_at_observation":row.status==Status::SatisfiedAtObservation,
        "jobs":{"construction":row.jobs.construction,"suspended_construction":row.jobs.suspended_construction,
            "construction_with_worker":row.jobs.construction_with_worker,"removal":row.jobs.removal,
            "other":row.jobs.other,"examples":jobs,"omitted":total as usize-jobs.len()},
        "item":row.item.as_ref().map(|item| json!({"native_id":item.native_id,
            "handle":item.handle.map(handle),"type_key":item.type_key,"flags":item.flags,
            "container_native_id":item.container_native_id,"holder_building_native_id":item.holder_building_native_id,
            "attached_jobs":item.attached_jobs,"attached_construction_jobs":item.attached_construction_jobs,
            "installed_condition_at_observation":item.installed_condition})),
        "cause_of_delay_proven":false,"native_effect_completed_proven":false});
    if let Some(monitoring) = monitoring { result["monitoring"] = monitoring; }
    result
}
fn summary(report: &Report) -> Value {
    let mut counts = BTreeMap::<&str, u64>::new();
    for row in &report.rows { *counts.entry(row.status.as_str()).or_default() += 1; }
    json!({"selected":report.rows.len(),"by_status":counts,
        "all_conditions_met_at_observation":report.rows.iter().all(|v| v.status==Status::SatisfiedAtObservation),
        "scope":"complete_requested_selection_not_just_this_page","work_units":report.work_used})
}

pub(super) fn execute<S: OperationsStateView>(
    state: &S, c: &OperationContext, source: Digest32, request: Request,
) -> Result<Value> {
    let started = Instant::now();
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let limit = request.limit.unwrap_or(4);
    if !(1..=32).contains(&limit) { return Err(invalid("construction page width must be 1..32")); }
    let monitoring = request.monitor.map(|v| v.validate(c)).transpose()?;
    let max_work = request.max_work.unwrap_or(progress::MAX_WORK);
    let targets: Vec<Target> = request.targets.into_iter().map(Target::from).collect();
    let report = progress::analyze(state, c, &targets, max_work)?;
    if report.source_digest != source { return Err(invalid("construction source differs from enclosing capture")); }
    let normalized: Vec<_> = report.rows.iter().map(|r| selected(&r.target)).collect();
    let id = identity(c, source, json!({"kind":"construction_progress","policy":progress::POLICY,
        "targets":normalized,"max_work":max_work,"monitor":monitoring}));
    let mut out = base(c, source, "construction_progress");
    out["policy"] = json!(progress::POLICY);
    out["summary"] = summary(&report);
    out["coverage"] = json!({"scope":"published_coherent_operations_projection",
        "complete_requested_selection_analyzed":true,"native_captures":0,
        "continuous_history_proven":false,"universal_world_absence_proven":false});
    out["placement_receipt_verified"] = json!(false);
    out["watch_registered"] = json!(false);
    out["mutation_dispatched"] = json!(false);
    out["native_effect_completed_proven"] = json!(false);
    out["interpretation"] = json!("Stage and observed-link conditions on the selected furniture, not a verified placement receipt, original footprint, current usability, delay cause, or permission to act. All jobs are counted; missing jobs alone cannot prove success. Counts describe this captured projection only.");
    let result = paginate(out, report.rows.len(), request.continuation.as_deref(), limit, id, c, |i| {
        if started.elapsed().as_millis() >= u128::from(c.budget.max_wall_millis) {
            return Err(budget("construction rendering deadline exhausted"));
        }
        let row = &report.rows[i];
        let proposal = monitoring.as_ref().map(|options| proposal(row, c, options)).transpose()?;
        Ok(row_json(row, proposal))
    })?;
    if started.elapsed().as_millis() >= u128::from(c.budget.max_wall_millis) {
        return Err(budget("construction query deadline exhausted"));
    }
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(result)
}

#[cfg(test)]
#[path = "spatial_construction_tests.rs"]
mod tests;
