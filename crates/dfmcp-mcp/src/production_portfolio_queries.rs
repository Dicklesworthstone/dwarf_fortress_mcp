//! Pure query presentation for joint, all-or-nothing task allocation. The parent
//! workforce module supplies its established whole-row pager and route witnesses.
use super::*;
use std::collections::BTreeMap;
use std::time::Instant;
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_adapter::workforce_analysis::portfolio::{self as joint, ProductionTask, ProductionPortfolio};
use joint::selection::{Domain, RESERVE_OWNER};

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuantityUnit { StackUnits }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialInput {
    key: String, units: u64, item_types: Vec<String>, subtype: Option<i32>,
    material_type: Option<i32>, material_index: Option<i32>,
}
impl MaterialInput {
    fn normalized(self) -> MaterialDemand {
        MaterialDemand { key:self.key,units:self.units,item_types:self.item_types,subtype:self.subtype,
            material_type:self.material_type,material_index:self.material_index }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskInput {
    key: String, priority: Option<u32>, workers: u32, skill_key: String,
    min_effective_skill: Option<i32>, preserve_social: Option<bool>, adults_only: Option<bool>,
    materials: Vec<MaterialInput>, origin: Option<[u32; 3]>,
}
impl TaskInput {
    fn normalized(self) -> ProductionTask {
        ProductionTask { key: self.key, priority: self.priority.unwrap_or(1), workers: self.workers,
            skill_key: self.skill_key, min_effective_skill: self.min_effective_skill.unwrap_or(1),
            preserve_social: self.preserve_social.unwrap_or(true), adults_only: self.adults_only.unwrap_or(true),
            materials: self.materials.into_iter().map(MaterialInput::normalized).collect() }
    }
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    ProductionPortfolio { origin: [u32; 3], quantity_unit: QuantityUnit, tasks: Vec<TaskInput>,
        reserves: Option<Vec<MaterialInput>>, limit: Option<u32>, continuation: Option<String>, max_work: Option<u64> },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input { schema: String, expected_anchor: Option<Value>, query: Request }

pub(in super::super) fn handles(input: &Value) -> bool {
    input.get("query").and_then(|q| q.get("kind")).and_then(Value::as_str) == Some("production_portfolio")
}
fn multisite(report: &ProductionPortfolio) -> bool { report.sites.multiple_origins(report.inventory.origin) }
fn material_json(d: &MaterialDemand) -> Value {
    json!({"key":d.key,"units":d.units,"item_types":d.item_types,"subtype":d.subtype,
        "material_type":d.material_type,"material_index":d.material_index})
}
fn request_json(report: &ProductionPortfolio) -> Value {
    let tasks:Vec<_>=report.tasks.iter().enumerate().map(|(i,t)| {
        let mut task=json!({"key":t.key,"priority":t.priority,"workers":t.workers,"skill_key":t.skill_key,
            "min_effective_skill":t.min_effective_skill,"preserve_social":t.preserve_social,"adults_only":t.adults_only,
            "materials":t.materials.iter().map(material_json).collect::<Vec<_>>()});
        let origin=report.sites.task_origins[i];
        if origin!=report.inventory.origin {task["origin"]=json!(origin);}
        task
    }).collect();
    let mut value=json!({"origin":report.inventory.origin,"quantity_unit":"stack_units","tasks":tasks});
    // Absent/null/explicit-default origins retain the common-origin model.
    // Absent/null/empty reserves likewise retain their original identity.
    if !report.reserves.is_empty() {value["reserves"]=json!(report.reserves.iter().map(material_json).collect::<Vec<_>>());}
    value
}
fn task_keys(report: &ProductionPortfolio, mask: u16) -> Vec<&str> {
    report.tasks.iter().enumerate().filter_map(|(i, t)| (mask & (1u16 << i) != 0).then_some(t.key.as_str())).collect()
}
fn reserve_key(report: &ProductionPortfolio,index:usize) -> Result<Option<&str>> {
    let owner=report.material_owners.get(index).ok_or_else(||invariant("material owner absent"))?;
    if *owner!=RESERVE_OWNER {return Ok(None);}
    let demand=report.inventory.demands.get(index).ok_or_else(||invariant("reserve demand absent"))?;
    Ok(Some(demand.key.strip_prefix("reserve.").ok_or_else(||invariant("reserve namespace absent"))?))
}
fn cut_json(report: &ProductionPortfolio, index: usize) -> Result<Value> {
    let rejected = report.selection.rejected.get(index).ok_or_else(|| invariant("production rejection row absent"))?;
    let cut = &rejected.shortage;
    let keys: Vec<_> = cut.demand_indices.iter().map(|&i| match rejected.domain {
        Domain::Workers => report.workforce.demands.get(i).map(|d| d.key.as_str()),
        Domain::Materials => report.inventory.demands.get(i).map(|d| d.key.as_str()),
    }.ok_or_else(|| invariant("production shortage references an absent demand"))).collect::<Result<_>>()?;
    let mut value=json!({"row_kind":"rejected_combination","task_mask":rejected.task_mask,"task_keys":task_keys(report,rejected.task_mask),
        "domain":match rejected.domain {Domain::Workers=>"workers",Domain::Materials=>"materials"},
        "demand_keys":keys,"required_units":cut.required_units,"eligible_units":cut.eligible_units,"deficit":cut.deficit,
        "interpretation":"A Hall-deficient subset under the declared model; not an independently additive or global shortage."});
    if multisite(report) {
        value["demand_origins"]=json!(cut.demand_indices.iter().map(|&i| match rejected.domain {
            Domain::Workers=>report.sites.task_origins.get(i),
            Domain::Materials=>report.sites.material_origins.get(i),
        }.copied().ok_or_else(||invariant("shortage site absent"))).collect::<Result<Vec<_>>>()?);
    }
    if !report.reserves.is_empty() {
        let mut reserves=Vec::new();
        if rejected.domain==Domain::Materials {
            for &i in &cut.demand_indices {if let Some(key)=reserve_key(report,i)? {reserves.push(key);}}
        }
        value["reserve_keys"]=json!(reserves);
    }
    Ok(value)
}
fn reserve_shortfall(report:&ProductionPortfolio) -> Result<Option<Value>> {
    let Some(cut)=&report.selection.materials.shortage else {return Ok(None);};
    if report.selection.task_mask!=0 || report.reserves.is_empty() {return Err(invariant("invalid reserve-only failure"));}
    let keys=cut.demand_indices.iter().map(|&i|reserve_key(report,i)?
        .ok_or_else(||invariant("reserve-only cut contains an optional task"))).collect::<Result<Vec<_>>>()?;
    Ok(Some(json!({"row_kind":"reserve_shortfall","domain":"materials","reserve_keys":keys,
        "required_units":cut.required_units,"eligible_units":cut.eligible_units,"deficit":cut.deficit,
        "all_task_sets_excluded":true,"partial_allocations_withheld":true,
        "interpretation":"Mandatory distinct reserve pools cannot all be supported by the eligible observed supply, even with no production. Not global stock absence."})))
}
fn row(state: &LiveSpatialCitizenState, report: &ProductionPortfolio, index: usize) -> Result<Value> {
    if let Some(shortfall)=reserve_shortfall(report)? {
        if index!=0 {return Err(invariant("reserve shortfall has exactly one evidence row"));}
        return Ok(shortfall);
    }
    let w = &report.selection.workers.assignments;
    let m = &report.selection.materials.assignments;
    if index < w.len() {
        let assignment = &w[index];
        let candidate = report.workforce.candidates[assignment.demand_index].iter()
            .find(|c| c.entity_id == assignment.supply_id).ok_or_else(|| invariant("joint worker lost its candidate evidence"))?;
        let mut value = candidate_row(state, &report.workforce, assignment.demand_index, candidate)?;
        value["row_kind"] = json!("worker_assignment"); value["task_key"] = json!(report.tasks[assignment.demand_index].key);
        if multisite(report) {value["origin"]=json!(report.sites.task_origins[assignment.demand_index]);}
        value["worker_slots"] = json!(assignment.units); return Ok(value);
    }
    let index = index - w.len();
    if index < m.len() {
        let assignment = &m[index];
        let owner = report.material_owners[assignment.demand_index];
        let demand = &report.inventory.demands[assignment.demand_index];
        let local_key = demand.key.split_once('.').map(|(_, key)| key).ok_or_else(|| invariant("joint material key absent"))?;
        let site = report.inventory_for(assignment.demand_index)?;
        let location = site.locations.get(&assignment.supply_id).ok_or_else(|| invariant("joint supply lacks evidence at its assigned site"))?;
        let protected=owner==RESERVE_OWNER;
        let task=if protected {None} else {Some(report.tasks.get(owner).ok_or_else(||invariant("material task owner invalid"))?.key.as_str())};
        let mut value=json!({"row_kind":if protected {"reserve_assignment"}else{"material_assignment"},"task_key":task,"input_key":local_key,
            "demand_key":demand.key,"units":assignment.units,"item":{"entity_id":location.item_id.to_string(),"generation":location.generation},
            "ground_root":{"entity_id":location.outermost_item_id.to_string(),"generation":location.outermost_generation},
            "position":location.position,"candidate_steps":location.candidate_steps,
            "route_query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(report.workforce.anchor),
                "query":{"kind":"map_route","start":site.origin,"goal":location.position}}});
        if multisite(report) {value["origin"]=json!(site.origin);}
        if protected {value["reserve_key"]=json!(local_key);value["consumed_by_selected_tasks"]=json!(false);
            value["reservation_created"]=json!(false);}
        return Ok(value);
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
    let Request::ProductionPortfolio { origin, quantity_unit: QuantityUnit::StackUnits, tasks, reserves, limit, continuation, max_work } = input.query;
    let limit = limit.unwrap_or(8);
    if !(1..=128).contains(&limit) || continuation.as_ref().is_some_and(|c| c.len() > 128) {
        return Err(budget("invalid production portfolio page bounds"));
    }
    let maximum = max_work.unwrap_or(MAX_WORK);
    let task_sites:BTreeMap<_,_>=tasks.iter().filter_map(|t|t.origin.map(|site|(t.key.clone(),site))).collect();
    let requested: Vec<_> = tasks.into_iter().map(TaskInput::normalized).collect();
    let reserves:Vec<_>=reserves.unwrap_or_default().into_iter().map(MaterialInput::normalized).collect();
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(context.budget.max_wall_millis) { return Err(budget("portfolio parsing exhausted its deadline")); }
    let mut timed = context.clone(); timed.budget.max_wall_millis -= elapsed as u64;
    let report = joint::plan_at_sites(state, &timed, origin, &requested, &reserves, &task_sites, maximum)?;
    let policy=if multisite(&report) {joint::MULTISITE_POLICY}
        else if report.reserves.is_empty() {joint::PORTFOLIO_POLICY}else{joint::RESERVE_POLICY};
    let shortfall=reserve_shortfall(&report)?;let feasible=shortfall.is_none();
    let model = Digest32::of_bytes(json!({"domain":"dfmcp-production-portfolio-model/1","policy":policy,
        "workforce_policy":workforce::WORKFORCE_POLICY,"supply_policy":dfmcp_adapter::spatial_inventory::SPATIAL_SUPPLY_POLICY,
        "anchor":anchor_json(context.anchor),"source_digest":report.workforce.source_digest.to_string(),
        "request":request_json(&report)}).to_string().as_bytes());
    let identity = page_identity(context, model, maximum);
    let cuts = (0..report.selection.rejected.len()).map(|i| cut_json(&report,i)).collect::<Result<Vec<_>>>()?;
    let mut proof=json!({"domain":"dfmcp-production-portfolio-exclusions/1",
        "model":model.to_string(),"selected_mask":report.selection.task_mask,"rejections":cuts});
    if let Some(shortfall)=&shortfall {proof["reserve_shortfall"]=shortfall.clone();}
    let proof_digest = Digest32::of_bytes(proof.to_string().as_bytes());
    let mut out = base(&report.workforce, "production_portfolio", model);
    out["origin"] = json!(origin); out["quantity_unit"] = json!("stack_units");
    out["portfolio_policy"] = json!(policy);
    out["selected_task_keys"] = json!(task_keys(&report,report.selection.task_mask));
    out["selected_task_mask"] = json!(report.selection.task_mask);
    out["selected_tasks"] = json!(report.selection.task_mask.count_ones());
    out["selected_priority"] = json!(report.selection.priority);
    out["all_tasks_supported"] = json!(feasible && report.selection.task_mask.count_ones() as usize == report.tasks.len());
    out["selected_set_model_feasible"] = json!(feasible);
    out["native_job_readiness_proven"] = json!(false); out["production_schedule_proven"] = json!(false);
    out["optimization"] = json!({"status":if feasible {"exact_for_declared_model"}else{"infeasible_hard_reserves"},
        "objective":"priority_sum_desc_task_count_desc_selected_key_list_asc",
        "higher_ranked_sets_rejected":report.selection.rejected.len(),"exclusion_evidence_digest":proof_digest.to_string(),
        "proof_rows_included_in_pagination":true,"candidate_sets_bound":(1usize<<report.tasks.len())-1,
        "flow_calls":report.selection.flow_calls,"work_units":report.work_units});
    out["tasks"] = json!(report.tasks.iter().enumerate().map(|(i,t)| {
        let mut task=json!({"key":t.key,"priority":t.priority,
            "selected":report.selection.task_mask&(1u16<<i)!=0,"workers_requested":t.workers,
            "workers_assigned":report.selection.workers.allocated_by_demand[i],"worker_candidates":report.workforce.candidates[i].len(),
            "skill_key_observed":report.workforce.skill_key_observed[i],"worker_classification":report.workforce.classifications[i],
            "required_material_inputs":t.materials.len()});
        if multisite(&report) {task["origin"]=json!(report.sites.task_origins[i]);}
        task
    }).collect::<Vec<_>>());
    out["supply_model"] = json!({"policy":dfmcp_adapter::spatial_inventory::SPATIAL_SUPPLY_POLICY,
        "candidate_stacks":report.sites.supplies.len(),"item_classification":report.inventory.item_counts,
        "reachable_tiles":report.inventory.reachable_tiles,"touched_region_boundary":report.inventory.touched_region_boundary});
    if multisite(&report) {
        out["supply_model"]["default_origin"]=json!(origin);
        out["supply_model"]["classification_and_reachability_scope"]=json!("default_origin_only; site_counts_are_not_additive");
        out["supply_model"]["shared_capacity_across_sites"]=json!(true);
        out["supply_model"]["sites"]=json!(std::iter::once(&report.inventory).chain(report.sites.additional_inventory.values()).map(|site| {
            let mask=report.sites.material_origins.iter().enumerate().fold(0u32,|mask,(i,p)|
                if *p==site.origin {mask|(1u32<<i)}else{mask});
            json!({"origin":site.origin,"candidate_stacks_for_local_demands":report.sites.supplies.iter().filter(|s|s.eligible&mask!=0).count(),
                "reachable_tiles":site.reachable_tiles,"touched_region_boundary":site.touched_region_boundary})
        }).collect::<Vec<_>>());
    }
    let mut protected=0u64;let mut consumed=0u64;let mut pools=Vec::new();
    for (i,demand) in report.inventory.demands.iter().enumerate() {
        let units=report.selection.materials.allocated_by_demand[i];
        if let Some(key)=reserve_key(&report,i)? {
            protected=protected.checked_add(units).ok_or_else(||budget("reserve total overflow"))?;
            let request=report.reserves.iter().find(|r|r.key==key).ok_or_else(||invariant("reserve request lost"))?;
            pools.push(json!({"request":material_json(request),"supported_units":units,
                "fully_supported":units==demand.units,"protected_in_model":feasible}));
        } else {consumed=consumed.checked_add(units).ok_or_else(||budget("task consumption overflow"))?;}
    }
    out["assigned_workers"] = json!(report.selection.workers.allocated_units);
    out["assigned_stack_units"] = json!(consumed);
    if !report.reserves.is_empty() {
        out["reserve_constraints"]=json!({"satisfied":feasible,"pools":pools,
            "scope":if multisite(&report) {"eligible_supply_at_default_origin"}else{"eligible_supply_at_common_origin"},
            "distinct_units_across_pools":true,"support_units":protected,"partial_support_is_diagnostic_only":!feasible,
            "protected_stack_units":if feasible {protected}else{0},"reservations_created":false});
    }
    out["interpretation"] = json!("Complete tasks share worker capacity one and finite conservative stack capacity across declared sites. Each assignment needs a candidate route at its own site. Reserves remain distinct unconsumed units accessible from the default origin. This is not inferred native requirements, a reservation, a carrying assignment, a timed schedule, or authorization to execute.");
    let count = if feasible {report.selection.workers.assignments.len() + report.selection.materials.assignments.len() + report.selection.rejected.len()}else{1};
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
