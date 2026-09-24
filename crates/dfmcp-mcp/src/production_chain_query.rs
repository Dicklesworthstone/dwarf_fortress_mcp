//! Agent-facing multi-stage production analysis. This module only presents the
//! existing observed-stock compiler; it never prepares or dispatches work orders.
use super::anchor_json;
use dfmcp_adapter::operations_analysis::production_chain::{
    self as chain, ProductionChainAnalysis, ProductionResource,
};
use dfmcp_adapter::operations_analysis::{
    AnalysisHandle, MAX_ANALYSIS_WORK, OperationsStateView, SUPPLY_POLICY,
};
use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier};
use dfmcp_intent::{BuildingKind, ProductionQuota, ProductionRecipe};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Instant;

#[path = "production_chain_compare.rs"]
mod comparisons;

const POLICY: &str = "dfmcp.production-chain-query/1";
fn invalid(s: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, s)
}
fn budget(s: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, s)
}
fn invariant(s: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, s)
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResourceInput {
    key: String,
    item_types: Vec<String>,
    subtype: Option<i32>,
    material_type: Option<i32>,
    material_index: Option<i32>,
}
impl From<ResourceInput> for ProductionResource {
    fn from(v: ResourceInput) -> Self {
        Self {
            key: v.key,
            item_types: v.item_types,
            subtype: v.subtype,
            material_type: v.material_type,
            material_index: v.material_index,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuotaInput {
    resource: String,
    minimum_stock: u32,
}
impl From<QuotaInput> for ProductionQuota {
    fn from(v: QuotaInput) -> Self {
        Self {
            item_token: v.resource,
            minimum_stock: v.minimum_stock,
        }
    }
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Ingredient {
    resource: String,
    units: u32,
}
#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Workshop {
    Workshop { type_key: String },
    Furnace { type_key: String },
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeInput {
    output: String,
    output_batch_size: u32,
    inputs: Vec<Ingredient>,
    job_token: String,
    workshop: Workshop,
}
impl From<RecipeInput> for ProductionRecipe {
    fn from(v: RecipeInput) -> Self {
        Self {
            output_token: v.output,
            output_batch_size: v.output_batch_size,
            input_tokens: v
                .inputs
                .into_iter()
                .map(|i| (i.resource, i.units))
                .collect(),
            job_token: v.job_token,
            workshop: match v.workshop {
                Workshop::Workshop { type_key } => BuildingKind::Workshop(type_key),
                Workshop::Furnace { type_key } => BuildingKind::Furnace(type_key),
            },
        }
    }
}
#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Section {
    All,
    Resources,
    Recipes,
    Steps,
    Shortages,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Unit {
    StackUnits,
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
    ProductionChain {
        quantity_unit: Unit,
        resources: Vec<ResourceInput>,
        quotas: Vec<QuotaInput>,
        recipes: Vec<RecipeInput>,
        section: Option<Section>,
        limit: Option<u32>,
        continuation: Option<String>,
        max_work: Option<u64>,
    },
}

pub(super) fn handles(input: &Value) -> bool {
    input
        .get("query")
        .and_then(|v| v.get("kind"))
        .and_then(Value::as_str)
        == Some("production_chain")
        || comparisons::handles(input)
}
fn validate_shape(input: &Value) -> Result<()> {
    let mut pending = vec![(input, 0usize)];
    let mut nodes = 0usize;
    let mut bytes = 0usize;
    while let Some((value, depth)) = pending.pop() {
        nodes += 1;
        bytes = bytes.saturating_add(32);
        if nodes > 4096 || depth > 16 {
            return Err(budget("production-chain input exceeds shape bounds"));
        }
        match value {
            Value::String(s) => bytes = bytes.saturating_add(s.len()),
            Value::Array(values) => {
                if nodes
                    .saturating_add(pending.len())
                    .saturating_add(values.len())
                    > 4096
                {
                    return Err(budget("production-chain input array is too wide"));
                }
                pending.extend(values.iter().map(|v| (v, depth + 1)));
            }
            Value::Object(values) => {
                if nodes
                    .saturating_add(pending.len())
                    .saturating_add(values.len())
                    > 4096
                {
                    return Err(budget("production-chain input object is too wide"));
                }
                for (key, value) in values {
                    bytes = bytes.saturating_add(key.len());
                    pending.push((value, depth + 1));
                }
            }
            _ => {}
        }
        if bytes > 131_072 {
            return Err(budget("production-chain input exceeds 128 KiB"));
        }
    }
    Ok(())
}
fn remaining(context: &OperationContext, start: Instant) -> Result<OperationContext> {
    let elapsed = start.elapsed().as_millis();
    if elapsed >= u128::from(context.budget.max_wall_millis) {
        return Err(budget(
            "production-chain request exhausted its wall-time allowance",
        ));
    }
    let mut c = context.clone();
    c.budget.max_wall_millis -= elapsed as u64;
    Ok(c)
}
fn handle(h: AnalysisHandle) -> Value {
    json!({"entity_id":h.entity_id.to_string(),"generation":h.generation,"revision":h.revision})
}
fn workshop(kind: &BuildingKind) -> Result<Value> {
    match kind {
        BuildingKind::Workshop(key) => Ok(json!({"kind":"workshop","type_key":key})),
        BuildingKind::Furnace(key) => Ok(json!({"kind":"furnace","type_key":key})),
        _ => Err(invariant("normalized chain recipe lost its workshop type")),
    }
}
fn ingredients(items: &[(String, u32)]) -> Vec<Value> {
    items
        .iter()
        .map(|(resource, units)| json!({"resource":resource,"units":units}))
        .collect()
}
fn rows(report: &ProductionChainAnalysis, section: Section) -> Result<Vec<Value>> {
    let mut rows = Vec::new();
    if matches!(section, Section::All | Section::Resources) {
        for stock in &report.resources {
            let key = &stock.resource.key;
            let balance = report
                .plan
                .requirements()
                .iter()
                .find(|r| &r.item_token == key);
            let quota = report.quotas.iter().find(|q| &q.item_token == key);
            rows.push(json!({"row_kind":"resource","key":key,
                "selector":{"item_types":stock.resource.item_types,"subtype":stock.resource.subtype,
                    "material_type":stock.resource.material_type,"material_index":stock.resource.material_index},
                "observed_stock":{"stack_units":stock.stock_units,"eligible_items":stock.eligible_items,
                    "examples":stock.examples.iter().copied().map(handle).collect::<Vec<_>>(),
                    "examples_complete":stock.examples.len() as u64 == stock.eligible_items},
                "minimum_final_stock":quota.map(|q| q.minimum_stock),
                "modeled_balance":balance.map(|r| json!({"minimum_stock":r.minimum_stock,
                    "consumed_units":r.consumed_units,"stock_units":r.stock_units,"planned_units":r.planned_units,
                    "missing_units":r.missing_units,"surplus_units":r.surplus_units}))}));
        }
    }
    if matches!(section, Section::All | Section::Recipes) {
        for r in &report.recipes {
            rows.push(json!({"row_kind":"recipe","output":r.output_token,"output_batch_size":r.output_batch_size,
                "inputs":ingredients(&r.input_tokens),"job_token":r.job_token,"workshop":workshop(&r.workshop)?,
                "native_recipe_verified":false}));
        }
    }
    if matches!(section, Section::All | Section::Steps) {
        for (index, step) in report.plan.steps().iter().enumerate() {
            let dependencies = step
                .depends_on
                .iter()
                .map(|&dependency| {
                    if dependency >= index {
                        return Err(invariant("production dependency is not an earlier step"));
                    }
                    let earlier = report
                        .plan
                        .steps()
                        .get(dependency)
                        .ok_or_else(|| invariant("production dependency absent"))?;
                    Ok(json!({"step_index":dependency,"output":earlier.output_token}))
                })
                .collect::<Result<Vec<_>>>()?;
            rows.push(json!({"row_kind":"step","step_index":index,"output":step.output_token,
                "batches":step.batches,"output_units":step.output_units,"input_units":ingredients(&step.input_units),
                "depends_on":dependencies,"job_token":step.job_token,"workshop":workshop(&step.workshop)?,
                "inventory_threshold":step.inventory_threshold,"work_order_created":false}));
        }
    }
    if matches!(section, Section::All | Section::Shortages) {
        for s in report.plan.shortages() {
            rows.push(json!({"row_kind":"shortage","resource":s.item_token,"required_units":s.required_units,
                "stock_units":s.stock_units,"missing_units":s.missing_units,"native_shortage_proven":false}));
        }
    }
    Ok(rows)
}
fn base(report: &ProductionChainAnalysis) -> Result<Value> {
    let mut goals_met = true;
    for quota in &report.quotas {
        let stock = report
            .resources
            .iter()
            .find(|r| r.resource.key == quota.item_token)
            .ok_or_else(|| invariant("production quota resource absent"))?;
        goals_met &= stock.stock_units >= quota.minimum_stock;
    }
    Ok(
        json!({"schema":"dfmcp.query.result/1","kind":"production_chain","anchor":anchor_json(report.anchor),
        "source_digest":report.source_digest.to_string(),"model_digest":report.model_digest.to_string(),
        "query_policy":POLICY,"chain_policy":chain::CHAIN_POLICY,"supply_policy":SUPPLY_POLICY,
        "quantity_unit":"stack_units","model_feasible":report.plan.model_feasible(),
        "observed_quotas_met":goals_met,"native_captures":0,"plan_created":false,
        "reservation_created":false,"commit_compatible":false,"mutation_dispatched":false,
        "native_recipe_verified":false,"job_readiness_proven":false,"completion_proven":false,
        "summary":{"resources":report.resources.len(),"declared_recipes":report.recipes.len(),
            "quotas":report.quotas.len(),"modeled_steps":report.plan.steps().len(),
            "shortage_resources":report.plan.shortages().len(),"expansion_rounds":report.plan.expansion_rounds(),
            "excluded_items":report.excluded_items,"unmatched_items":report.unmatched_items},
        "work_units":report.work_units,
        "coverage":{"stock":"coherent_conservative_observed_items","recipe_effects":"caller_declared",
            "workshop_capacity":"unknown","labor":"unknown","routes":"unknown","native_order_queue":"not_modeled"},
        "interpretation":"Initial stock is observed under the conservative supply policy. Production, consumption and final balances are conditional on declared single-output recipes and interchangeable stack units. A modeled deficit is not a native shortage; feasibility is not permission or readiness to execute."}),
    )
}
fn page_identity(c: &OperationContext, model: Digest32, section: Section, work: u64) -> Digest32 {
    Digest32::of_bytes(
        json!({"domain":POLICY,"session":c.session_id.to_string(),"anchor":anchor_json(c.anchor),
        "model":model.to_string(),"section":section,"max_work":work})
        .to_string()
        .as_bytes(),
    )
}
fn token(offset: usize, id: Digest32) -> String {
    let mut bytes = b"dfmcp-production-chain-page-v1\0".to_vec();
    bytes.extend_from_slice(id.as_bytes());
    bytes.extend_from_slice(&(offset as u64).to_be_bytes());
    format!("pc1:{offset}:{}", Digest32::of_bytes(&bytes))
}
fn offset(raw: Option<&str>, id: Digest32, count: usize) -> Result<usize> {
    let Some(raw) = raw else {
        return Ok(0);
    };
    if raw.len() > 128 {
        return Err(budget("production-chain cursor exceeds 128 bytes"));
    }
    let mut p = raw.split(':');
    let (Some("pc1"), Some(n), Some(_), None) = (p.next(), p.next(), p.next(), p.next()) else {
        return Err(invalid("invalid production-chain cursor"));
    };
    if n.is_empty() || n.starts_with('0') || n.len() > 6 || !n.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid("noncanonical production-chain offset"));
    }
    let n = n
        .parse::<usize>()
        .map_err(|_| invalid("production-chain offset overflow"))?;
    if raw != token(n, id) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "production-chain cursor names another session, capture, model, section or budget",
        ));
    }
    if n >= count {
        return Err(DfmcpError::new(
            ErrorCode::CursorGap,
            "production-chain cursor is beyond the result",
        ));
    }
    Ok(n)
}
fn paginate(
    mut out: Value,
    rows: &[Value],
    c: &OperationContext,
    limit: u32,
    continuation: Option<&str>,
    identity: Digest32,
) -> Result<Value> {
    if !(1..=128).contains(&limit) {
        return Err(budget("production-chain page limit must be 1..128"));
    }
    let start = offset(continuation, identity, rows.len())?;
    let maximum = c
        .budget
        .max_bytes
        .min(u64::from(c.budget.max_output_tokens) * 4);
    out["analysis_digest"] = json!(identity.to_string());
    out["total_rows"] = json!(rows.len());
    out["rows"] = json!([]);
    out["returned"] = json!(0);
    out["truncated"] = json!(false);
    out["continuation"] = Value::Null;
    let mut end = start;
    for row in rows
        .iter()
        .skip(start)
        .take(limit.min(c.budget.max_entities) as usize)
    {
        let mut next = out.clone();
        next["rows"]
            .as_array_mut()
            .ok_or_else(|| invariant("production-chain rows absent"))?
            .push(row.clone());
        next["returned"] = json!(end + 1 - start);
        next["truncated"] = json!(end + 1 < rows.len());
        next["continuation"] = if end + 1 < rows.len() {
            json!(token(end + 1, identity))
        } else {
            Value::Null
        };
        if next.to_string().len() as u64 > maximum {
            break;
        }
        out = next;
        end += 1;
    }
    if (end == start && start < rows.len()) || out.to_string().len() as u64 > maximum {
        return Err(budget(
            "complete production-chain summary and one row do not fit",
        ));
    }
    Ok(out)
}

pub(super) fn execute<S: OperationsStateView + ?Sized>(
    state: &S,
    c: &OperationContext,
    input: &Value,
) -> Result<Value> {
    if comparisons::handles(input) {
        return comparisons::execute(state, c, input);
    }
    let started = Instant::now();
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let envelope: Envelope = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid production-chain request"))?;
    if envelope.schema != "dfmcp.query/1" {
        return Err(invalid("production-chain requires dfmcp.query/1"));
    }
    if envelope
        .expected_anchor
        .is_some_and(|a| a != anchor_json(c.anchor))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "production-chain expected anchor differs",
        ));
    }
    let Query::ProductionChain {
        quantity_unit: Unit::StackUnits,
        resources,
        quotas,
        recipes,
        section,
        limit,
        continuation,
        max_work,
    } = envelope.query;
    let limit = limit.unwrap_or(8);
    let work = max_work.unwrap_or(MAX_ANALYSIS_WORK);
    if !(1..=128).contains(&limit) || work == 0 || work > MAX_ANALYSIS_WORK {
        return Err(budget(
            "production-chain page or work allowance exceeds bounds",
        ));
    }
    let resources = resources
        .into_iter()
        .map(ProductionResource::from)
        .collect::<Vec<_>>();
    let quotas = quotas
        .into_iter()
        .map(ProductionQuota::from)
        .collect::<Vec<_>>();
    let recipes = recipes
        .into_iter()
        .map(ProductionRecipe::from)
        .collect::<Vec<_>>();
    let report = chain::plan_production_chain(
        state,
        &remaining(c, started)?,
        &resources,
        &quotas,
        &recipes,
        work,
    )?;
    let section = section.unwrap_or(Section::All);
    let identity = page_identity(c, report.model_digest, section, work);
    let mut out = base(&report)?;
    out["section"] = json!(section);
    let out = paginate(
        out,
        &rows(&report, section)?,
        c,
        limit,
        continuation.as_deref(),
        identity,
    )?;
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    remaining(c, started)?;
    Ok(out)
}
pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let query: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_production_chain_v1.json"
    ))
    .map_err(|_| invariant("embedded production-chain schema invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invariant("query schema variants absent"))?
        .push(query);
    comparisons::extend_schema(schema)
}

#[cfg(test)]
#[path = "production_chain_query_tests.rs"]
mod tests;
