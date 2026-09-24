//! Presentation for production diagnosis and declared supply plans over sealed
//! coherent adapter states. Algorithms stay in the adapter/world crates.

use super::anchor_json;
use dfmcp_adapter::operations_analysis::{
    self as analysis, ANALYSIS_POLICY, AnalysisHandle, DiagnosisScope, JobDiagnosis,
    MAX_ANALYSIS_WORK, MaterialDemand, OperationsStateView, SUPPLY_POLICY,
};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, EntityId, ErrorCode, OperationContext, Result, RiskTier,
};
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_INPUT_BYTES: usize = 65_536;
const MAX_INPUT_NODES: usize = 4_096;
const MAX_PAGE: u32 = 128;

fn invalid(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, text)
}
fn invariant(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, text)
}
fn exhausted(text: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, text)
}

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
    ProductionDiagnosis {
        job: Option<Focus>,
        holder: Option<Focus>,
        include_clear_jobs: Option<bool>,
        limit: Option<u32>,
        continuation: Option<String>,
        max_work: Option<u64>,
    },
    InventoryPlan {
        quantity_unit: QuantityUnit,
        demands: Vec<DemandInput>,
        limit: Option<u32>,
        continuation: Option<String>,
        max_work: Option<u64>,
    },
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum QuantityUnit {
    StackUnits,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Focus {
    entity_id: String,
    generation: u32,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DemandInput {
    key: String,
    units: u64,
    item_types: Vec<String>,
    subtype: Option<i32>,
    material_type: Option<i32>,
    material_index: Option<i32>,
}
impl From<DemandInput> for MaterialDemand {
    fn from(d: DemandInput) -> Self {
        Self {
            key: d.key,
            units: d.units,
            item_types: d.item_types,
            subtype: d.subtype,
            material_type: d.material_type,
            material_index: d.material_index,
        }
    }
}
impl Focus {
    fn decode(self) -> Result<(EntityId, u32)> {
        if self.entity_id.is_empty()
            || self.entity_id.len() > 20
            || self.entity_id.starts_with('0')
            || !self.entity_id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(invalid(
                "focus IDs must be canonical positive decimal u64 strings",
            ));
        }
        let id = self
            .entity_id
            .parse::<u64>()
            .map_err(|_| invalid("focus ID exceeds u64"))?;
        Ok((EntityId::new(id), self.generation))
    }
}

fn validate_shape(input: &Value) -> Result<()> {
    let mut stack = vec![(input, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = stack.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(16);
        if depth > 12 || nodes > MAX_INPUT_NODES {
            return Err(exhausted("production query exceeds its shape bound"));
        }
        match value {
            Value::String(text) => bytes = bytes.saturating_add(text.len()),
            Value::Array(values) => {
                if nodes
                    .saturating_add(stack.len())
                    .saturating_add(values.len())
                    > MAX_INPUT_NODES
                {
                    return Err(exhausted("production query exceeds its node bound"));
                }
                stack.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(values) => {
                if nodes
                    .saturating_add(stack.len())
                    .saturating_add(values.len())
                    > MAX_INPUT_NODES
                {
                    return Err(exhausted("production query exceeds its node bound"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    stack.push((value, depth + 1));
                }
            }
            Value::Number(n) => bytes = bytes.saturating_add(n.to_string().len()),
            _ => {}
        }
        if bytes > MAX_INPUT_BYTES {
            return Err(exhausted("production query exceeds its input byte bound"));
        }
    }
    Ok(())
}

pub(super) fn handles(input: &Value) -> bool {
    matches!(
        input
            .get("query")
            .and_then(|q| q.get("kind"))
            .and_then(Value::as_str),
        Some("production_diagnosis" | "inventory_plan")
    )
}
fn put_text(out: &mut Vec<u8>, value: &str) {
    out.extend_from_slice(&(value.len() as u64).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
}
fn identity_prefix(context: &OperationContext, source: Digest32, kind: &str) -> Vec<u8> {
    let mut bytes = b"dfmcp-operations-analysis-query-v1\0".to_vec();
    put_text(&mut bytes, ANALYSIS_POLICY);
    put_text(&mut bytes, kind);
    bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
    for value in [
        context.anchor.fortress_id.get(),
        context.anchor.cursor.epoch,
        context.anchor.cursor.sequence,
        context.anchor.tick.0,
    ] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(context.anchor.state_hash.as_bytes());
    bytes.extend_from_slice(source.as_bytes());
    bytes
}
fn put_focus(out: &mut Vec<u8>, value: Option<(EntityId, u32)>) {
    out.push(u8::from(value.is_some()));
    if let Some((id, generation)) = value {
        out.extend_from_slice(&id.get().to_be_bytes());
        out.extend_from_slice(&generation.to_be_bytes());
    }
}
fn put_demand(out: &mut Vec<u8>, d: &MaterialDemand) {
    put_text(out, &d.key);
    out.extend_from_slice(&d.units.to_be_bytes());
    out.extend_from_slice(&(d.item_types.len() as u32).to_be_bytes());
    for kind in &d.item_types {
        put_text(out, kind);
    }
    for value in [d.subtype, d.material_type, d.material_index] {
        out.push(u8::from(value.is_some()));
        if let Some(value) = value {
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
}
fn token(offset: usize, identity: Digest32) -> String {
    let mut bytes = b"dfmcp-operations-analysis-cursor-v1\0".to_vec();
    bytes.extend_from_slice(identity.as_bytes());
    bytes.extend_from_slice(&(offset as u64).to_be_bytes());
    format!("op1:{offset}:{}", Digest32::of_bytes(&bytes))
}
fn page_offset(raw: Option<&str>) -> Result<usize> {
    let Some(raw) = raw else {
        return Ok(0);
    };
    if raw.len() > 128 {
        return Err(exhausted("production continuation exceeds 128 bytes"));
    }
    let parts = raw.split(':').collect::<Vec<_>>();
    if parts.len() != 3
        || parts[0] != "op1"
        || parts[1].is_empty()
        || parts[1].starts_with('0')
        || parts[1].len() > 6
        || !parts[1].bytes().all(|b| b.is_ascii_digit())
        || parts[2].len() != 64
        || !parts[2]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(invalid("invalid production continuation"));
    }
    parts[1]
        .parse::<usize>()
        .map_err(|_| invalid("production offset overflow"))
}
fn handle_json(handle: AnalysisHandle) -> Value {
    json!({"entity_id":handle.entity_id.to_string(),"generation":handle.generation,"revision":handle.revision})
}
fn diagnosis_json(row: &JobDiagnosis, context: &OperationContext) -> Value {
    json!({"job":handle_json(row.job),"native_job_id":row.native_job_id,"type_key":row.type_key,
        "assessment":"observed_conditions_not_proven_causes","blocker_proven":false,"job_ready_proven":false,
        "findings":row.findings,"suspended":row.suspended,"worker_assigned":row.worker_assigned,
        "holder":row.holder.map(handle_json),"holder_stage":row.holder_stage.map(|(stage,maximum)|json!({"stage":stage,"maximum":maximum})),
        "attachment_records":row.attachment_records,"distinct_attached_items":row.distinct_attached_items,
        "required_filter_count":row.required_filter_count,"filters_without_indexed_attachments":row.filters_without_indexed_attachments,
        "direct_item_flags":row.direct_item_flags,"container_item_flags":row.container_item_flags,
        "shared_attached_items":row.shared_attached_items,"zero_size_attached_items":row.zero_size_attached_items,
        "affected_item_count":row.affected_item_count,
        "affected_item_examples":row.affected_item_examples.iter().copied().map(handle_json).collect::<Vec<_>>(),
        "affected_item_examples_truncated":row.affected_item_examples.len() < row.affected_item_count as usize,
        "inspect_assignment":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(context.anchor),"query":{
            "kind":"inspect","entity_id":row.job.entity_id.to_string(),"generation":row.job.generation,
            "fields":["worker_assigned","worker_entity","worker_is_strict_citizen","position"]}},
        "inspect_relationships":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(context.anchor),"query":{"kind":"traverse",
            "roots":[row.job.entity_id.to_string()],"edge_kinds":["uses","contained_in"],"max_depth":4}}})
}
fn base_payload(context: &OperationContext, source: Digest32, kind: &str) -> Value {
    json!({"schema":"dfmcp.query.result/1","kind":kind,"anchor":anchor_json(context.anchor),
        "analysis_policy":ANALYSIS_POLICY,"source_digest":source.to_string(),
        "mutation_authority":false,"reservation_created":false,"commit_compatible":false,
        "coverage":{"domain":"coherent_observed_operations","game_feasibility":"unknown",
            "causal_blockers_proven":false,"intermediate_history_proven":false},
        "unknown":["full_native_job_requirements","path_access","labor_eligibility","successful_job_completion"]})
}

struct Page<'a> {
    raw: Option<&'a str>,
    offset: usize,
    limit: u32,
    identity: Digest32,
    maximum_bytes: usize,
}
fn page<F>(mut payload: Value, count: usize, spec: Page<'_>, row: F) -> Result<Value>
where
    F: Fn(usize) -> Result<Value>,
{
    if let Some(raw) = spec.raw {
        if raw != token(spec.offset, spec.identity) {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "production continuation belongs to another session, query, policy, or observation",
            ));
        }
        if spec.offset >= count {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "production continuation is past the result set",
            ));
        }
    }
    payload["analysis_digest"] = json!(spec.identity.to_string());
    payload["total_rows"] = json!(count);
    payload["returned"] = json!(0);
    payload["rows"] = json!([]);
    payload["truncated"] = json!(false);
    payload["continuation"] = Value::Null;
    let mut end = spec.offset;
    while end < count && end - spec.offset < spec.limit as usize {
        let mut candidate = payload.clone();
        candidate["rows"]
            .as_array_mut()
            .ok_or_else(|| invariant("production page lost its rows"))?
            .push(row(end)?);
        candidate["returned"] = json!(end + 1 - spec.offset);
        candidate["truncated"] = json!(end + 1 < count);
        candidate["continuation"] = if end + 1 < count {
            json!(token(end + 1, spec.identity))
        } else {
            Value::Null
        };
        if serde_json::to_vec(&candidate)
            .map_err(|_| invariant("production page cannot be encoded"))?
            .len()
            > spec.maximum_bytes
        {
            break;
        }
        payload = candidate;
        end += 1;
    }
    if end == spec.offset && spec.offset < count {
        return Err(exhausted(
            "one complete production row cannot fit; increase the negotiated output budget",
        ));
    }
    if serde_json::to_vec(&payload)
        .map_err(|_| invariant("production metadata cannot be encoded"))?
        .len()
        > spec.maximum_bytes
    {
        return Err(exhausted(
            "production summary cannot fit the remaining output budget",
        ));
    }
    Ok(payload)
}

pub(super) fn execute<S: OperationsStateView + ?Sized>(
    state: &S,
    context: &OperationContext,
    input: &Value,
) -> Result<Value> {
    let started = std::time::Instant::now();
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid production query envelope or fields"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("production query requires dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .as_ref()
        .is_some_and(|a| a != &anchor_json(context.anchor))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "production query expected_anchor is not current",
        ));
    }
    let (limit, continuation, maximum_work) = match &envelope.query {
        Query::ProductionDiagnosis {
            limit,
            continuation,
            max_work,
            ..
        }
        | Query::InventoryPlan {
            limit,
            continuation,
            max_work,
            ..
        } => (
            limit.unwrap_or(8),
            continuation.as_deref(),
            max_work.unwrap_or(MAX_ANALYSIS_WORK),
        ),
    };
    if limit == 0 || limit > MAX_PAGE {
        return Err(exhausted("production page limit must be 1..128"));
    }
    let offset = page_offset(continuation)?;
    let maximum_bytes = usize::try_from(
        context
            .budget
            .max_bytes
            .min(u64::from(context.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| exhausted("production output budget does not fit this platform"))?;
    let result = match envelope.query {
        Query::ProductionDiagnosis {
            job,
            holder,
            include_clear_jobs,
            continuation,
            ..
        } => {
            let scope = DiagnosisScope {
                job: job.map(Focus::decode).transpose()?,
                holder: holder.map(Focus::decode).transpose()?,
                include_clear_jobs: include_clear_jobs.unwrap_or(false),
            };
            let report = analysis::diagnose_production(state, context, scope, maximum_work)?;
            let mut bytes = identity_prefix(context, report.source_digest, "production_diagnosis");
            put_focus(&mut bytes, scope.job);
            put_focus(&mut bytes, scope.holder);
            bytes.push(u8::from(scope.include_clear_jobs));
            let identity = Digest32::of_bytes(&bytes);
            let mut payload = base_payload(context, report.source_digest, "production_diagnosis");
            payload["summary"] = json!({"jobs_considered":report.jobs_considered,"jobs_with_findings":report.jobs_with_findings,
                "finding_counts":report.finding_counts,"no_findings_does_not_prove_readiness":true});
            payload["interpretation"] = json!(
                "Unfinished holders may be normal for construction jobs; attachment indices, shared references and flags are observations, not material-shortage or causal-blocker proofs."
            );
            payload["ordering"] =
                json!("removed-rotten-forbidden-suspended-holder-stage-native-id/1");
            payload["work_units"] = json!(report.work_units);
            page(
                payload,
                report.rows.len(),
                Page {
                    raw: continuation.as_deref(),
                    offset,
                    limit,
                    identity,
                    maximum_bytes,
                },
                |index| Ok(diagnosis_json(&report.rows[index], context)),
            )
        }
        Query::InventoryPlan {
            quantity_unit: QuantityUnit::StackUnits,
            demands,
            continuation,
            ..
        } => {
            let requested: Vec<_> = demands.into_iter().map(MaterialDemand::from).collect();
            let report = analysis::plan_inventory(state, context, &requested, maximum_work)?;
            let mut bytes = identity_prefix(context, report.source_digest, "inventory_plan");
            put_text(&mut bytes, SUPPLY_POLICY);
            bytes.extend_from_slice(&(report.demands.len() as u32).to_be_bytes());
            for d in &report.demands {
                put_demand(&mut bytes, d);
            }
            let identity = Digest32::of_bytes(&bytes);
            let mut payload = base_payload(context, report.source_digest, "inventory_plan");
            payload["quantity_unit"] = json!("stack_units");
            payload["supply_policy"] = json!(SUPPLY_POLICY);
            payload["model_feasible"] = json!(report.allocation.shortage.is_none());
            payload["interpretation"] = json!(
                "Exact only for your declared interchangeable stack-unit requests and this conservative supply subset. No full DF requirement matching, access proof, reservation, or execution plan is created."
            );
            payload["summary"] = json!({"requested_units":report.allocation.requested_units,
                "allocated_units":report.allocation.allocated_units,"candidate_items":report.candidate_items,
                "candidate_stack_units":report.candidate_stack_units,"unmatched_items":report.unmatched_items,
                "excluded_items_by_primary_policy_reason":report.excluded_items});
            payload["demands"] =
                json!(report.demands.iter().enumerate().map(|(i,d)| json!({
                "key":d.key,"units":d.units,"item_types":d.item_types,"subtype":d.subtype,
                "material_type":d.material_type,"material_index":d.material_index,
                "allocated_units":report.allocation.allocated_by_demand[i],
                "unallocated_units":d.units-report.allocation.allocated_by_demand[i]
            })).collect::<Vec<_>>());
            payload["certificate"] = json!({"kind":"integral_flow_equal_min_cut",
            "flow_units":report.allocation.allocated_units,"cut_capacity":report.allocation.cut_capacity,
            "scope":"declared_demands_and_conservative_observed_supply_only",
            "shortage":report.allocation.shortage.as_ref().map(|cut|json!({
                "demand_keys":cut.demand_indices.iter().map(|&d|report.demands[d].key.as_str()).collect::<Vec<_>>(),
                "required_units":cut.required_units,"eligible_units":cut.eligible_units,"deficit":cut.deficit,
                "note":"joint shortage witness; per-demand maxima must not be added independently"
            }))});
            payload["work_units"] = json!(report.work_units);
            let snapshot = state
                .operations_snapshot()
                .ok_or_else(|| invariant("allocation source projection missing"))?;
            page(
                payload,
                report.allocation.assignments.len(),
                Page {
                    raw: continuation.as_deref(),
                    offset,
                    limit,
                    identity,
                    maximum_bytes,
                },
                |index| {
                    let assignment = &report.allocation.assignments[index];
                    let item = snapshot
                        .graph
                        .entities
                        .get(&EntityId::new(assignment.supply_id))
                        .ok_or_else(|| {
                            invariant("allocation item is missing from its source projection")
                        })?;
                    Ok(
                        json!({"demand_key":report.demands[assignment.demand_index].key,
                    "item":handle_json(AnalysisHandle {entity_id:item.id,generation:item.generation,revision:item.revision}),
                    "units":assignment.units,"source_digest":report.source_digest.to_string()}),
                    )
                },
            )
        }
    }?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(exhausted(
            "production query exhausted its cooperative wall-time budget",
        ));
    }
    Ok(result)
}

/// Compose into the selected runtime's existing query schema, without dropping
/// its workforce, route, watch or history definitions or inheriting another profile.
pub(super) fn extend_schema(mut base: Value) -> Result<Value> {
    let extension: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_operations_query_extensions_v1.json"
    ))
    .map_err(|_| invariant("invalid operations query schema extension"))?;
    let definitions = extension["definitions"]
        .as_object()
        .ok_or_else(|| invariant("operations schema definitions missing"))?;
    let target = base["$defs"]
        .as_object_mut()
        .ok_or_else(|| invariant("base query definitions missing"))?;
    for (name, definition) in definitions {
        if target.insert(name.clone(), definition.clone()).is_some() {
            return Err(invariant("operations schema definition collision"));
        }
    }
    let queries = extension["queries"]
        .as_array()
        .ok_or_else(|| invariant("operations schema variants missing"))?;
    base["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invariant("base query variants missing"))?
        .extend(queries.iter().cloned());
    Ok(base)
}

/// Retain the operations runtime's original schema identity and descriptions.
pub(super) fn query_schema() -> Result<Value> {
    let base: Value = serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))
        .map_err(|_| invariant("invalid base query schema"))?;
    let mut base = extend_schema(base)?;
    base["$id"] = json!("urn:dfmcp:operations-query:1");
    base["title"] = json!(
        "Operations query envelope with production diagnostics and conditional inventory planning"
    );
    base["description"] = json!(
        "Value of the operations/1.3 fortress.query query argument. Original structured queries plus observed production diagnostics and declared stack-unit allocation. Runtime enforces authority, canonical identities, UTF-8 bounds, work ceilings and full response budgets."
    );
    Ok(base)
}
