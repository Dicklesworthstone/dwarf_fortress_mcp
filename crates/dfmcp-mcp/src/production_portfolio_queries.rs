//! Pure query presentation for joint, all-or-nothing task allocation. The parent
//! workforce module supplies its established whole-row pager and route witnesses.
use super::*;
use std::time::Instant;
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_adapter::workforce_analysis::portfolio::{self as joint, ProductionTask, ProductionPortfolio};
use joint::selection::Domain;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuantityUnit { StackUnits }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialInput {
    key: String, units: u64, item_types: Vec<String>, subtype: Option<i32>,
    material_type: Option<i32>, material_index: Option<i32>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskInput {
    key: String, priority: Option<u32>, workers: u32, skill_key: String,
    min_effective_skill: Option<i32>, preserve_social: Option<bool>, adults_only: Option<bool>,
    materials: Vec<MaterialInput>,
}
impl TaskInput {
    fn normalized(self) -> ProductionTask {
        ProductionTask { key: self.key, priority: self.priority.unwrap_or(1), workers: self.workers,
            skill_key: self.skill_key, min_effective_skill: self.min_effective_skill.unwrap_or(1),
            preserve_social: self.preserve_social.unwrap_or(true), adults_only: self.adults_only.unwrap_or(true),
            materials: self.materials.into_iter().map(|d| MaterialDemand { key: d.key, units: d.units,
                item_types: d.item_types, subtype: d.subtype, material_type: d.material_type, material_index: d.material_index }).collect() }
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    ProductionPortfolio { origin: [u32; 3], quantity_unit: QuantityUnit, tasks: Vec<TaskInput>,
        limit: Option<u32>, continuation: Option<String>, max_work: Option<u64> },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input { schema: String, expected_anchor: Option<Value>, query: Request }

pub(in super::super) fn handles(input: &Value) -> bool {
    input.get("query").and_then(|q| q.get("kind")).and_then(Value::as_str) == Some("production_portfolio")
}
fn material_json(d: &MaterialDemand) -> Value {
    json!({"key":d.key,"units":d.units,"item_types":d.item_types,"subtype":d.subtype,
        "material_type":d.material_type,"material_index":d.material_index})
}
fn request_json(report: &ProductionPortfolio) -> Value {
    json!({"origin":report.inventory.origin,"quantity_unit":"stack_units","tasks":report.tasks.iter().map(|t|
        json!({"key":t.key,"priority":t.priority,"workers":t.workers,"skill_key":t.skill_key,
            "min_effective_skill":t.min_effective_skill,"preserve_social":t.preserve_social,"adults_only":t.adults_only,
            "materials":t.materials.iter().map(material_json).collect::<Vec<_>>()})).collect::<Vec<_>>()})
}
fn task_keys(report: &ProductionPortfolio, mask: u16) -> Vec<&str> {
    report.tasks.iter().enumerate().filter_map(|(i, t)| (mask & (1u16 << i) != 0).then_some(t.key.as_str())).collect()
}
fn cut_json(report: &ProductionPortfolio, index: usize) -> Result<Value> {
    let rejected = report.selection.rejected.get(index).ok_or_else(|| invariant("production rejection row absent"))?;
    let cut = &rejected.shortage;
    let keys: Vec<_> = cut.demand_indices.iter().map(|&i| match rejected.domain {
        Domain::Workers => report.workforce.demands.get(i).map(|d| d.key.as_str()),
        Domain::Materials => report.inventory.demands.get(i).map(|d| d.key.as_str()),
    }.ok_or_else(|| invariant("production shortage references an absent demand"))).collect::<Result<_>>()?;
    Ok(json!({"row_kind":"rejected_combination","task_mask":rejected.task_mask,"task_keys":task_keys(report,rejected.task_mask),
        "domain":match rejected.domain {Domain::Workers=>"workers",Domain::Materials=>"materials"},
        "demand_keys":keys,"required_units":cut.required_units,"eligible_units":cut.eligible_units,"deficit":cut.deficit,
        "interpretation":"A Hall-deficient subset under the declared model; not an independently additive or global shortage."}))
}
fn row(state: &LiveSpatialCitizenState, report: &ProductionPortfolio, index: usize) -> Result<Value> {
    let w = &report.selection.workers.assignments;
    let m = &report.selection.materials.assignments;
    if index < w.len() {
        let assignment = &w[index];
        let candidate = report.workforce.candidates[assignment.demand_index].iter()
            .find(|c| c.entity_id == assignment.supply_id).ok_or_else(|| invariant("joint worker lost its candidate evidence"))?;
        let mut value = candidate_row(state, &report.workforce, assignment.demand_index, candidate)?;
        value["row_kind"] = json!("worker_assignment"); value["task_key"] = json!(report.tasks[assignment.demand_index].key);
        value["worker_slots"] = json!(assignment.units); return Ok(value);
    }
    let index = index - w.len();
    if index < m.len() {
        let assignment = &m[index];
        let owner = report.material_owners[assignment.demand_index];
        let demand = &report.inventory.demands[assignment.demand_index];
        let local_key = demand.key.split_once('.').map(|(_, key)| key).ok_or_else(|| invariant("joint material key absent"))?;
        let location = report.inventory.locations.get(&assignment.supply_id).ok_or_else(|| invariant("joint supply location lost"))?;
        return Ok(json!({"row_kind":"material_assignment","task_key":report.tasks[owner].key,"input_key":local_key,
            "demand_key":demand.key,"units":assignment.units,"item":{"entity_id":location.item_id.to_string(),"generation":location.generation},
            "ground_root":{"entity_id":location.outermost_item_id.to_string(),"generation":location.outermost_generation},
            "position":location.position,"candidate_steps":location.candidate_steps,
            "route_query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(report.workforce.anchor),
                "query":{"kind":"map_route","start":report.inventory.origin,"goal":location.position}}}));
    }
    cut_json(report, index - m.len())
}

pub(in super::super) fn execute(state: &LiveSpatialCitizenState, context: &OperationContext, input: &Value) -> Result<Value> {
    let started = Instant::now();
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let input: Input = serde_json::from_value(input.clone()).map_err(|_| invalid("invalid production portfolio request"))?;
    if input.schema != "dfmcp.query/1" { return Err(invalid("production portfolio requires dfmcp.query/1")); }
    if input.expected_anchor.as_ref().is_some_and(|a| a != &anchor_json(context.anchor))
        || state.snapshot().map(|s| s.anchor()) != Some(context.anchor) {
        return Err(DfmcpError::new(ErrorCode::StaleAnchor, "production portfolio names another coherent capture"));
    }
    let Request::ProductionPortfolio { origin, quantity_unit: QuantityUnit::StackUnits, tasks, limit, continuation, max_work } = input.query;
    let limit = limit.unwrap_or(8);
    if !(1..=128).contains(&limit) || continuation.as_ref().is_some_and(|c| c.len() > 128) {
        return Err(budget("invalid production portfolio page bounds"));
    }
    let maximum = max_work.unwrap_or(MAX_WORK);
    let requested: Vec<_> = tasks.into_iter().map(TaskInput::normalized).collect();
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(context.budget.max_wall_millis) { return Err(budget("portfolio parsing exhausted its deadline")); }
    let mut timed = context.clone(); timed.budget.max_wall_millis -= elapsed as u64;
    let report = joint::plan(state, &timed, origin, &requested, maximum)?;
    let model = Digest32::of_bytes(json!({"domain":"dfmcp-production-portfolio-model/1","policy":joint::PORTFOLIO_POLICY,
        "workforce_policy":workforce::WORKFORCE_POLICY,"supply_policy":dfmcp_adapter::spatial_inventory::SPATIAL_SUPPLY_POLICY,
        "anchor":anchor_json(context.anchor),"source_digest":report.workforce.source_digest.to_string(),
        "request":request_json(&report)}).to_string().as_bytes());
    let identity = page_identity(context, model, maximum);
    let cuts = (0..report.selection.rejected.len()).map(|i| cut_json(&report,i)).collect::<Result<Vec<_>>>()?;
    let proof_digest = Digest32::of_bytes(json!({"domain":"dfmcp-production-portfolio-exclusions/1",
        "model":model.to_string(),"selected_mask":report.selection.task_mask,"rejections":cuts}).to_string().as_bytes());
    let mut out = base(&report.workforce, "production_portfolio", model);
    out["origin"] = json!(origin); out["quantity_unit"] = json!("stack_units");
    out["portfolio_policy"] = json!(joint::PORTFOLIO_POLICY);
    out["selected_task_keys"] = json!(task_keys(&report,report.selection.task_mask));
    out["selected_task_mask"] = json!(report.selection.task_mask);
    out["selected_tasks"] = json!(report.selection.task_mask.count_ones());
    out["selected_priority"] = json!(report.selection.priority);
    out["all_tasks_supported"] = json!(report.selection.task_mask.count_ones() as usize == report.tasks.len());
    out["selected_set_model_feasible"] = json!(true);
    out["native_job_readiness_proven"] = json!(false); out["production_schedule_proven"] = json!(false);
    out["optimization"] = json!({"status":"exact_for_declared_model","objective":"priority_sum_desc_task_count_desc_selected_key_list_asc",
        "higher_ranked_sets_rejected":report.selection.rejected.len(),"exclusion_evidence_digest":proof_digest.to_string(),
        "proof_rows_included_in_pagination":true,"candidate_sets_bound":(1usize<<report.tasks.len())-1,
        "flow_calls":report.selection.flow_calls,"work_units":report.work_units});
    out["tasks"] = json!(report.tasks.iter().enumerate().map(|(i,t)| json!({"key":t.key,"priority":t.priority,
        "selected":report.selection.task_mask&(1u16<<i)!=0,"workers_requested":t.workers,
        "workers_assigned":report.selection.workers.allocated_by_demand[i],"worker_candidates":report.workforce.candidates[i].len(),
        "skill_key_observed":report.workforce.skill_key_observed[i],"worker_classification":report.workforce.classifications[i],
        "required_material_inputs":t.materials.len()})).collect::<Vec<_>>());
    out["supply_model"] = json!({"policy":dfmcp_adapter::spatial_inventory::SPATIAL_SUPPLY_POLICY,
        "candidate_stacks":report.inventory.supplies.len(),"item_classification":report.inventory.item_counts,
        "reachable_tiles":report.inventory.reachable_tiles,"touched_region_boundary":report.inventory.touched_region_boundary});
    out["assigned_workers"] = json!(report.selection.workers.allocated_units);
    out["assigned_stack_units"] = json!(report.selection.materials.allocated_units);
    out["interpretation"] = json!("Complete declared tasks share worker capacity one and conservative route-aware stack capacity at one origin. This is not inferred native requirements, a reservation, a timed schedule, or authorization to execute.");
    let count = report.selection.workers.assignments.len() + report.selection.materials.assignments.len() + report.selection.rejected.len();
    let result = paginate(out, count, context, Page { limit, continuation: continuation.as_deref(), prefix: "pp1", identity },
        |i| row(state,&report,i))?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(budget("production portfolio exhausted its shared analysis/render deadline"));
    }
    Ok(result)
}

pub(in super::super) fn extend_schema(mut base: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!("../../../schemas/mcp_production_portfolio_v1.json"))
        .map_err(|_| invariant("invalid production portfolio schema"))?;
    base["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(|| invariant("query schema variants absent"))?.push(extension);
    Ok(base)
}
