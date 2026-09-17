//! Bounded, demand-driven planning over a declared single-output recipe model.
//!
//! All quotas are minimum *final* stocks, including stock that another recipe
//! consumes. These are model proposals, never native recipes or effect authority.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use super::{InventoryStockpile, ProductionLogisticsCompiler, ProductionRecipe};
use crate::{Action, BuildingKind, WorkOrderCondition};

const MAX_QUOTAS: usize = 64;
const MAX_INPUTS: usize = 64;
const MAX_TOKEN_BYTES: usize = 256;

/// One minimum final stock. Repeated tokens are normalized by taking the maximum.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionQuota {
    pub item_token: String,
    pub minimum_stock: u32,
}

/// Independent limits on the reachable model, not on the entire recipe catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProductionPlanningLimits {
    pub max_orders: usize,
    pub max_resources: usize,
    pub max_edges: usize,
    pub max_work: u64,
}

impl Default for ProductionPlanningLimits {
    fn default() -> Self {
        Self {
            max_orders: 250,
            max_resources: 1_024,
            max_edges: 8_192,
            max_work: 1_000_000,
        }
    }
}

/// One material balance, with initial stock credited exactly once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionRequirement {
    pub item_token: String,
    pub minimum_stock: u32,
    pub consumed_units: u32,
    pub stock_units: u32,
    pub planned_units: u32,
    pub missing_units: u32,
    pub surplus_units: u64,
}

/// A complete raw-resource deficit, not merely the first failed dependency.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionShortage {
    pub item_token: String,
    pub required_units: u32,
    pub stock_units: u32,
    pub missing_units: u32,
}

/// A hypothetical recipe step. Dependencies refer to earlier step indices.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionStep {
    pub output_token: String,
    pub job_token: String,
    pub workshop: BuildingKind,
    pub batches: u32,
    pub output_units: u32,
    pub input_units: Vec<(String, u32)>,
    pub depends_on: Vec<usize>,
    pub inventory_threshold: u32,
}

/// Immutable analysis of the caller's declared model. A feasible model does not
/// establish observed stock, a usable workshop, native eligibility or authority.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionPlan {
    requirements: Vec<ProductionRequirement>,
    shortages: Vec<ProductionShortage>,
    steps: Vec<ProductionStep>,
    work_used: u64,
    expansion_rounds: u32,
}

impl ProductionPlan {
    #[must_use]
    pub fn requirements(&self) -> &[ProductionRequirement] {
        &self.requirements
    }

    #[must_use]
    pub fn shortages(&self) -> &[ProductionShortage] {
        &self.shortages
    }

    #[must_use]
    pub fn steps(&self) -> &[ProductionStep] {
        &self.steps
    }

    #[must_use]
    pub fn model_feasible(&self) -> bool {
        self.shortages.is_empty()
    }

    #[must_use]
    pub const fn work_used(&self) -> u64 {
        self.work_used
    }

    #[must_use]
    pub const fn expansion_rounds(&self) -> u32 {
        self.expansion_rounds
    }

    /// Convert only a feasible model into action *proposals*. Ordinary plan
    /// sealing, authority, witness and effect validation still apply to them.
    pub fn into_work_orders(self) -> Result<Vec<Action>> {
        if let Some(shortage) = self.shortages.first() {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!(
                    "{} production resources are short; '{}' requires {} additional modeled units",
                    self.shortages.len(), shortage.item_token, shortage.missing_units,
                ),
            ));
        }
        Ok(self
            .steps
            .into_iter()
            .map(|step| Action::CreateWorkOrder {
                name: format!("Auto-JIT: {}", step.job_token),
                job_token: step.job_token,
                amount: step.batches,
                conditions: vec![WorkOrderCondition::ItemCountBelow {
                    item_token: step.output_token,
                    threshold: step.inventory_threshold,
                }],
            })
            .collect())
    }
}

struct Work {
    used: u64,
    limit: u64,
}

impl Work {
    fn charge(&mut self) -> Result<()> {
        self.used = self.used.checked_add(1).ok_or_else(work_exhausted)?;
        if self.used > self.limit {
            return Err(work_exhausted());
        }
        Ok(())
    }
}

fn work_exhausted() -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, "production planning work exhausted")
}

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn overflow(token: &str) -> DfmcpError {
    DfmcpError::new(
        ErrorCode::BudgetExceeded,
        format!("production quantity overflow for '{token}'"),
    )
}

fn validate_token(token: &str) -> Result<()> {
    if token.is_empty() || token.len() > MAX_TOKEN_BYTES || token.contains('\0') {
        return Err(invalid("production strings must contain 1..256 UTF-8 bytes without NUL"));
    }
    Ok(())
}

fn validate_limits(limits: ProductionPlanningLimits) -> Result<()> {
    if limits.max_orders == 0
        || limits.max_orders > 250
        || limits.max_resources == 0
        || limits.max_resources > 4_096
        || limits.max_edges == 0
        || limits.max_edges > 8_192
        || limits.max_work > 10_000_000
    {
        return Err(invalid("production planning limits exceed the fixed model bounds"));
    }
    Ok(())
}

fn normalized_recipe(recipe: &ProductionRecipe, work: &mut Work) -> Result<ProductionRecipe> {
    work.charge()?;
    validate_token(&recipe.output_token)?;
    validate_token(&recipe.job_token)?;
    // Account for the generated work-order name before allocating it.
    if recipe.job_token.len() > MAX_TOKEN_BYTES - "Auto-JIT: ".len() {
        return Err(invalid("production job token exceeds the work-order name budget"));
    }
    match &recipe.workshop {
        BuildingKind::Workshop(name)
        | BuildingKind::Furnace(name)
        | BuildingKind::Furniture(name)
        | BuildingKind::Construction(name)
        | BuildingKind::Trap(name)
        | BuildingKind::Custom(name) => validate_token(name)?,
        BuildingKind::FarmPlot | BuildingKind::Bridge | BuildingKind::Well => {}
    }
    if recipe.output_batch_size == 0 || recipe.input_tokens.len() > MAX_INPUTS {
        return Err(invalid("production recipe requires a positive yield and at most 64 inputs"));
    }
    let mut inputs = BTreeMap::<String, u32>::new();
    for (token, amount) in &recipe.input_tokens {
        work.charge()?;
        validate_token(token)?;
        if *amount == 0 {
            return Err(invalid("production input quantities must be positive"));
        }
        let total = inputs.entry(token.clone()).or_default();
        *total = total.checked_add(*amount).ok_or_else(|| overflow(token))?;
    }
    Ok(ProductionRecipe {
        output_token: recipe.output_token.clone(),
        output_batch_size: recipe.output_batch_size,
        input_tokens: inputs.into_iter().collect(),
        workshop: recipe.workshop.clone(),
        job_token: recipe.job_token.clone(),
    })
}

/// Consumer-before-supplier order. Only recipes that actually require production
/// contribute edges: already-stocked materials can discharge cyclic catalogs.
fn topological_order(
    nodes: &BTreeSet<String>,
    active: &BTreeMap<String, ProductionRecipe>,
    work: &mut Work,
) -> Result<Vec<String>> {
    let mut incoming = BTreeMap::<String, usize>::new();
    for token in nodes {
        work.charge()?;
        incoming.insert(token.clone(), 0);
    }
    for recipe in active.values() {
        for (input, _) in &recipe.input_tokens {
            work.charge()?;
            let count = incoming.get_mut(input).ok_or_else(|| {
                DfmcpError::new(ErrorCode::InternalInvariantViolation, "production graph lost an input")
            })?;
            *count += 1; // At most 250 active recipes and 64 distinct inputs each.
        }
    }
    let mut ready: BTreeSet<String> = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(token, _)| token.clone())
        .collect();
    let mut ordered = Vec::with_capacity(nodes.len());
    while let Some(token) = ready.pop_first() {
        work.charge()?;
        if let Some(recipe) = active.get(&token) {
            for (input, _) in &recipe.input_tokens {
                work.charge()?;
                let count = incoming.get_mut(input).ok_or_else(|| {
                    DfmcpError::new(ErrorCode::InternalInvariantViolation, "production graph lost an input")
                })?;
                *count = count.checked_sub(1).ok_or_else(|| {
                    DfmcpError::new(ErrorCode::InternalInvariantViolation, "production indegree underflow")
                })?;
                if *count == 0 {
                    ready.insert(input.clone());
                }
            }
        }
        ordered.push(token);
    }
    if ordered.len() != nodes.len() {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "cyclic active production dependency; cyclic bootstrap schedules are not modeled",
        ));
    }
    Ok(ordered)
}

impl ProductionLogisticsCompiler {
    /// Plan simultaneous minimum final stocks with complete material balances.
    ///
    /// Demand-driven graph expansion leaves unused recipes unexamined. Each
    /// round aggregates *all* consumers before rounding supplier batches once.
    /// At least one recipe is activated per unfinished round, so neither graph
    /// depth nor the number of dependency paths causes recursive execution.
    pub fn plan_quotas(
        &self,
        quotas: &[ProductionQuota],
        inventory: &InventoryStockpile,
        limits: ProductionPlanningLimits,
    ) -> Result<ProductionPlan> {
        validate_limits(limits)?;
        if quotas.is_empty() || quotas.len() > MAX_QUOTAS {
            return Err(invalid("production planning requires 1..64 quotas"));
        }
        let mut work = Work { used: 0, limit: limits.max_work };
        let mut goals = BTreeMap::<String, u32>::new();
        for quota in quotas {
            work.charge()?;
            validate_token(&quota.item_token)?;
            let minimum = goals.entry(quota.item_token.clone()).or_default();
            *minimum = (*minimum).max(quota.minimum_stock);
        }
        let mut nodes: BTreeSet<String> = goals.keys().cloned().collect();
        if nodes.len() > limits.max_resources {
            return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "production resource limit exceeded"));
        }
        let mut active = BTreeMap::<String, ProductionRecipe>::new();
        let mut edge_count = 0usize;
        let mut rounds = 0u32;
        loop {
            work.charge()?;
            rounds += 1; // At most max_orders + 1 rounds.
            let order = topological_order(&nodes, &active, &mut work)?;
            // Start afresh: demands from previous rounds must never be counted twice.
            let mut required = goals.clone();
            let mut batches = BTreeMap::<String, u32>::new();
            let mut pending = Vec::new();
            for token in &order {
                work.charge()?;
                let total = required.get(token).copied().map_or(0, |value| value);
                let shortage = total.saturating_sub(inventory.get_stock(token));
                if shortage == 0 {
                    continue;
                }
                let Some(recipe) = active.get(token) else {
                    if self.recipes.contains_key(token) {
                        pending.push(token.clone());
                    }
                    continue;
                };
                let count = shortage.div_ceil(recipe.output_batch_size);
                count.checked_mul(recipe.output_batch_size).ok_or_else(|| overflow(token))?;
                batches.insert(token.clone(), count);
                for (input, amount) in &recipe.input_tokens {
                    work.charge()?;
                    let additional = amount.checked_mul(count).ok_or_else(|| overflow(input))?;
                    let total = required.entry(input.clone()).or_default();
                    *total = total.checked_add(additional).ok_or_else(|| overflow(input))?;
                }
            }
            if pending.is_empty() {
                return ExpandedModel { order, required, batches, goals, active, rounds }
                    .finish(inventory, work);
            }
            for token in pending {
                work.charge()?;
                if active.len() >= limits.max_orders {
                    return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "production order limit exceeded"));
                }
                let recipe = self.recipes.get(&token).ok_or_else(|| {
                    DfmcpError::new(ErrorCode::InternalInvariantViolation, "production recipe disappeared")
                })?;
                let recipe = normalized_recipe(recipe, &mut work)?;
                edge_count += recipe.input_tokens.len();
                if edge_count > limits.max_edges {
                    return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "production edge limit exceeded"));
                }
                for (input, _) in &recipe.input_tokens {
                    work.charge()?;
                    if !nodes.contains(input) && nodes.len() >= limits.max_resources {
                        return Err(DfmcpError::new(ErrorCode::BudgetExceeded, "production resource limit exceeded"));
                    }
                    nodes.insert(input.clone());
                }
                active.insert(token, recipe);
            }
        }
    }

    /// Joint-quota counterpart of `compile_quota_work_orders`. Infeasible models
    /// return no actions; use `plan_quotas` to inspect all raw-resource shortages.
    pub fn compile_quotas_work_orders(
        &self,
        quotas: &[ProductionQuota],
        inventory: &InventoryStockpile,
        limits: ProductionPlanningLimits,
    ) -> Result<Vec<Action>> {
        self.plan_quotas(quotas, inventory, limits)?.into_work_orders()
    }
}

struct ExpandedModel {
    order: Vec<String>,
    required: BTreeMap<String, u32>,
    batches: BTreeMap<String, u32>,
    goals: BTreeMap<String, u32>,
    active: BTreeMap<String, ProductionRecipe>,
    rounds: u32,
}

impl ExpandedModel {
    fn finish(self, inventory: &InventoryStockpile, mut work: Work) -> Result<ProductionPlan> {
        let Self { order, required, batches, goals, active, rounds } = self;
        let mut requirements = Vec::with_capacity(required.len());
        let mut shortages = Vec::new();
        for (token, total) in &required {
            work.charge()?;
            let minimum = goals.get(token).copied().map_or(0, |value| value);
            let stock = inventory.get_stock(token);
            let planned = match (batches.get(token), active.get(token)) {
                (Some(count), Some(recipe)) => count
                    .checked_mul(recipe.output_batch_size)
                    .ok_or_else(|| overflow(token))?,
                _ => 0,
            };
            let available = u64::from(stock) + u64::from(planned);
            let missing = u32::try_from(u64::from(*total).saturating_sub(available))
                .map_err(|_| overflow(token))?;
            if missing > 0 {
                shortages.push(ProductionShortage {
                    item_token: token.clone(), required_units: *total, stock_units: stock,
                    missing_units: missing,
                });
            }
            requirements.push(ProductionRequirement {
                item_token: token.clone(), minimum_stock: minimum, consumed_units: *total - minimum,
                stock_units: stock, planned_units: planned, missing_units: missing,
                surplus_units: available.saturating_sub(u64::from(*total)),
            });
        }
        let mut steps = Vec::with_capacity(batches.len());
        let mut indices = BTreeMap::<String, usize>::new();
        for token in order.into_iter().rev() {
            work.charge()?;
            let Some(&count) = batches.get(&token) else { continue; };
            let recipe = active.get(&token).ok_or_else(|| {
                DfmcpError::new(ErrorCode::InternalInvariantViolation, "production recipe disappeared")
            })?;
            let mut depends_on = Vec::new();
            let mut inputs = Vec::with_capacity(recipe.input_tokens.len());
            for (input, amount) in &recipe.input_tokens {
                work.charge()?;
                inputs.push((input.clone(), amount.checked_mul(count).ok_or_else(|| overflow(input))?));
                if let Some(&index) = indices.get(input) { depends_on.push(index); }
            }
            depends_on.sort_unstable();
            let threshold = required.get(&token).copied().ok_or_else(|| {
                DfmcpError::new(ErrorCode::InternalInvariantViolation, "production requirement disappeared")
            })?;
            indices.insert(token.clone(), steps.len());
            steps.push(ProductionStep {
                output_token: token, job_token: recipe.job_token.clone(), workshop: recipe.workshop.clone(),
                batches: count, output_units: count.checked_mul(recipe.output_batch_size).ok_or_else(work_exhausted)?,
                input_units: inputs, depends_on, inventory_threshold: threshold,
            });
        }
        Ok(ProductionPlan { requirements, shortages, steps, work_used: work.used, expansion_rounds: rounds })
    }
}
