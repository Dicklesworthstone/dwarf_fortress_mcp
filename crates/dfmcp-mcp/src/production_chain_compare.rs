//! Compare mutually exclusive recipe models at one source anchor. The frontier
//! uses component-wise deficits, never a sum of incompatible resource units.
use super::*;
use std::collections::BTreeSet;

const COMPARISON_POLICY: &str = "dfmcp.production-chain-deficit-frontier/1";
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidate {
    key: String,
    recipes: Vec<RecipeInput>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    schema: String,
    expected_anchor: Option<Value>,
    query: ComparisonQuery,
}
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ComparisonQuery {
    ProductionChainCompare {
        quantity_unit: Unit,
        resources: Vec<ResourceInput>,
        quotas: Vec<QuotaInput>,
        candidates: Vec<Candidate>,
        limit: Option<u32>,
        continuation: Option<String>,
        max_work: Option<u64>,
    },
}
struct Evaluated {
    key: String,
    report: ProductionChainAnalysis,
    deficits: Vec<u32>,
}

pub(super) fn handles(input: &Value) -> bool {
    input
        .get("query")
        .and_then(|v| v.get("kind"))
        .and_then(Value::as_str)
        == Some("production_chain_compare")
}
fn charge(used: &mut u64, count: u64, maximum: u64) -> Result<()> {
    *used = used
        .checked_add(count)
        .ok_or_else(|| budget("production comparison work overflow"))?;
    if *used > maximum {
        return Err(budget(
            "production comparison exhausted its shared work allowance",
        ));
    }
    Ok(())
}
/// Strict Pareto dominance. Equal vectors remain alternatives; resource keys
/// name distinct dimensions. No ordering across different domains is inferred.
fn dominates(a: &[u32], b: &[u32]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| x <= y)
        && a.iter().zip(b).any(|(x, y)| x < y)
}

pub(super) fn execute<S: OperationsStateView + ?Sized>(
    state: &S,
    c: &OperationContext,
    input: &Value,
) -> Result<Value> {
    let started = Instant::now();
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    validate_shape(input)?;
    let input: Input = serde_json::from_value(input.clone())
        .map_err(|_| invalid("invalid production comparison request"))?;
    if input.schema != "dfmcp.query/1" {
        return Err(invalid("production comparison requires dfmcp.query/1"));
    }
    if input
        .expected_anchor
        .is_some_and(|anchor| anchor != anchor_json(c.anchor))
    {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "production comparison expected anchor differs",
        ));
    }
    let ComparisonQuery::ProductionChainCompare {
        quantity_unit: Unit::StackUnits,
        resources,
        quotas,
        mut candidates,
        limit,
        continuation,
        max_work,
    } = input.query;
    let maximum = max_work.unwrap_or(MAX_ANALYSIS_WORK);
    let limit = limit.unwrap_or(8);
    if !(2..=8).contains(&candidates.len())
        || !(1..=128).contains(&limit)
        || maximum == 0
        || maximum > MAX_ANALYSIS_WORK
    {
        return Err(budget(
            "production comparison requires 2..8 alternatives and bounded page/shared work allowances",
        ));
    }
    let mut keys = BTreeSet::new();
    for candidate in &candidates {
        if candidate.key.is_empty()
            || candidate.key.len() > 64
            || candidate.key.contains('\0')
            || !keys.insert(candidate.key.clone())
            || candidate.recipes.len() > 32
        {
            return Err(invalid(
                "production alternatives require unique bounded keys and at most 32 recipes each",
            ));
        }
    }
    candidates.sort_by(|a, b| a.key.cmp(&b.key));
    let resources = resources
        .into_iter()
        .map(ProductionResource::from)
        .collect::<Vec<_>>();
    let quotas = quotas
        .into_iter()
        .map(ProductionQuota::from)
        .collect::<Vec<_>>();
    let mut used = 0u64;
    let mut evaluated: Vec<Evaluated> = Vec::with_capacity(candidates.len());
    for candidate in candidates {
        charge(&mut used, 1, maximum)?;
        let available = maximum
            .checked_sub(used)
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                budget("production comparison has no work left for another alternative")
            })?;
        let recipes = candidate
            .recipes
            .into_iter()
            .map(ProductionRecipe::from)
            .collect::<Vec<_>>();
        let report = chain::plan_production_chain(
            state,
            &remaining(c, started)?,
            &resources,
            &quotas,
            &recipes,
            available,
        )?;
        charge(&mut used, report.work_units, maximum)?;
        if let Some(first) = evaluated.first() {
            if report.anchor != first.report.anchor
                || report.source_digest != first.report.source_digest
                || report.resources != first.report.resources
                || report.quotas != first.report.quotas
            {
                return Err(invariant(
                    "production alternatives do not share one observed stock and quota universe",
                ));
            }
        }
        let mut deficits = Vec::with_capacity(report.resources.len());
        for resource in &report.resources {
            // Charge the bounded lookup, including unsuccessful comparisons.
            charge(&mut used, 1 + report.plan.shortages().len() as u64, maximum)?;
            deficits.push(
                report
                    .plan
                    .shortages()
                    .iter()
                    .find(|s| s.item_token == resource.resource.key)
                    .map_or(0, |s| s.missing_units),
            );
        }
        if report.plan.model_feasible() != deficits.iter().all(|d| *d == 0) {
            return Err(invariant(
                "production feasibility disagrees with complete deficit vector",
            ));
        }
        evaluated.push(Evaluated {
            key: candidate.key,
            report,
            deficits,
        });
    }
    let first = evaluated
        .first()
        .ok_or_else(|| invariant("production comparison lost its alternatives"))?;
    let mut rows = Vec::with_capacity(evaluated.len());
    let mut frontier = Vec::new();
    let mut feasible = Vec::new();
    for candidate in &evaluated {
        let mut dominators = Vec::new();
        for other in &evaluated {
            // Two vector passes plus identity comparison: shared with all plans.
            charge(&mut used, 1 + 2 * candidate.deficits.len() as u64, maximum)?;
            if other.key != candidate.key && dominates(&other.deficits, &candidate.deficits) {
                dominators.push(other.key.clone());
            }
        }
        if dominators.is_empty() {
            frontier.push(candidate.key.clone());
        }
        if candidate.report.plan.model_feasible() {
            feasible.push(candidate.key.clone());
        }
        let balances = candidate
            .report
            .resources
            .iter()
            .zip(&candidate.deficits)
            .map(|(stock, deficit)| {
                let requirement = candidate
                    .report
                    .plan
                    .requirements()
                    .iter()
                    .find(|r| r.item_token == stock.resource.key);
                json!({"resource":stock.resource.key,"missing_units":deficit,
                "consumed_units":requirement.map_or(0, |r| r.consumed_units),
                "planned_units":requirement.map_or(0, |r| r.planned_units)})
            })
            .collect::<Vec<_>>();
        charge(
            &mut used,
            (candidate.report.resources.len() * (1 + candidate.report.plan.requirements().len()))
                as u64,
            maximum,
        )?;
        rows.push(json!({"candidate":candidate.key,"model_digest":candidate.report.model_digest.to_string(),
            "model_feasible":candidate.report.plan.model_feasible(),"on_deficit_frontier":dominators.is_empty(),
            "dominated_by":dominators,"balances":balances,"modeled_steps":candidate.report.plan.steps().len(),
            "modeled_batches":candidate.report.plan.steps().iter().map(|s| u64::from(s.batches)).sum::<u64>(),
            "planner_work_units":candidate.report.work_units,"native_cost_proven":false}));
    }
    let identities = evaluated
        .iter()
        .map(|candidate| {
            json!({"key":candidate.key,
        "model_digest":candidate.report.model_digest.to_string()})
        })
        .collect::<Vec<_>>();
    let comparison_digest = Digest32::of_bytes(
        json!({"policy":COMPARISON_POLICY,"candidates":identities})
            .to_string()
            .as_bytes(),
    );
    let identity = page_identity(c, comparison_digest, Section::All, maximum);
    let stock = first
        .report
        .resources
        .iter()
        .map(|s| {
            json!({"resource":s.resource.key,
        "stock_units":s.stock_units,"eligible_items":s.eligible_items})
        })
        .collect::<Vec<_>>();
    let final_quotas = first
        .report
        .quotas
        .iter()
        .map(|q| {
            json!({"resource":q.item_token,
        "minimum_stock":q.minimum_stock})
        })
        .collect::<Vec<_>>();
    let out = json!({"schema":"dfmcp.query.result/1","kind":"production_chain_compare",
        "anchor":anchor_json(first.report.anchor),"source_digest":first.report.source_digest.to_string(),
        "comparison_digest":comparison_digest.to_string(),"comparison_policy":COMPARISON_POLICY,
        "chain_policy":chain::CHAIN_POLICY,"supply_policy":SUPPLY_POLICY,"quantity_unit":"stack_units",
        "summary":{"alternatives":evaluated.len(),"feasible_candidates":feasible,"deficit_frontier":frontier,
            "observed_stock":stock,"minimum_final_stock":final_quotas,
            "excluded_items":first.report.excluded_items,"unmatched_items":first.report.unmatched_items},
        "work_units":used,"all_declared_alternatives_evaluated":true,"native_captures":0,
        "plan_created":false,"reservation_created":false,"commit_compatible":false,"mutation_dispatched":false,
        "global_optimum_proven":false,"native_recipe_verified":false,"completion_proven":false,
        "interpretation":"Mutually exclusive declared recipe models share one observed conservative stock and the same final quotas. The deficit frontier removes only candidates with no smaller deficit in any resource and a larger deficit in at least one. Equal and incomparable vectors remain alternatives; batch counts are not native time or cost. Models are not combined or dispatched."});
    let out = paginate(out, &rows, c, limit, continuation.as_deref(), identity)?;
    c.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    remaining(c, started)?;
    Ok(out)
}
pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let query: Value = serde_json::from_str(include_str!(
        "../../../schemas/mcp_production_chain_compare_v1.json"
    ))
    .map_err(|_| invariant("embedded production comparison schema invalid"))?;
    schema["$defs"]["query"]["oneOf"]
        .as_array_mut()
        .ok_or_else(|| invariant("query schema variants absent"))?
        .push(query);
    Ok(schema)
}

#[cfg(test)]
#[path = "production_chain_compare_tests.rs"]
mod tests;
