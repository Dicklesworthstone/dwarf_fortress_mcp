//! Conditional supply-chain plans using one coherent, conservative inventory.
//!
//! Observed stack quantities are evidence; recipes and interchangeability are
//! caller-declared assumptions. Neither layer grants authority to execute work.

use super::{
    AnalysisHandle, InventoryIndex, MaterialDemand, OperationsStateView, SUPPLY_POLICY, Work,
    exclusion, exhausted, handle, invalid, invariant, normalized_demands, source,
};
use crate::live_operations::item_entity_id;
use dfmcp_core::{Capability, Digest32, OperationContext, Result, RiskTier, StateAnchor};
use dfmcp_intent::{
    BuildingKind, InventoryStockpile, ProductionLogisticsCompiler, ProductionPlan,
    ProductionPlanningLimits, ProductionQuota, ProductionRecipe,
};
use std::collections::{BTreeMap, BTreeSet};

pub const CHAIN_POLICY: &str = "dfmcp.production-chain/1";
pub const MAX_CHAIN_RESOURCES: usize = 32;
pub const MAX_CHAIN_RECIPES: usize = 32;
const EXAMPLES: usize = 8;

/// One disjoint domain of modeled interchangeable stack units. Two resource
/// definitions may not match the same possible item, even if none is captured.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionResource {
    pub key: String,
    pub item_types: Vec<String>,
    pub subtype: Option<i32>,
    pub material_type: Option<i32>,
    pub material_index: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionResourceStock {
    pub resource: ProductionResource,
    pub stock_units: u32,
    pub eligible_items: u64,
    pub examples: Vec<AnalysisHandle>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionChainAnalysis {
    pub anchor: StateAnchor,
    pub source_digest: Digest32,
    /// Binds the normalized complete model, source identity and supply policy.
    pub model_digest: Digest32,
    pub resources: Vec<ProductionResourceStock>,
    pub quotas: Vec<ProductionQuota>,
    pub recipes: Vec<ProductionRecipe>,
    pub plan: ProductionPlan,
    pub excluded_items: BTreeMap<&'static str, u64>,
    pub unmatched_items: u64,
    pub work_units: u64,
}

fn valid_text(text: &str, maximum: usize) -> bool {
    !text.is_empty() && text.len() <= maximum && !text.contains('\0')
}

fn selectors_overlap(left: &MaterialDemand, right: &MaterialDemand) -> bool {
    left.item_types
        .iter()
        .any(|kind| right.item_types.binary_search(kind).is_ok())
        && [
            (left.subtype, right.subtype),
            (left.material_type, right.material_type),
            (left.material_index, right.material_index),
        ]
        .into_iter()
        .all(|(a, b)| match (a, b) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        })
}

fn selectors(input: &[ProductionResource], work: &mut Work) -> Result<Vec<MaterialDemand>> {
    if input.is_empty() || input.len() > MAX_CHAIN_RESOURCES {
        return Err(invalid(
            "production chains require 1..32 disjoint resource definitions",
        ));
    }
    // Bound borrowed inputs before cloning them for the existing selector validator.
    for resource in input {
        work.charge(1)?;
        if !valid_text(&resource.key, 64)
            || resource.item_types.is_empty()
            || resource.item_types.len() > 8
            || resource
                .item_types
                .iter()
                .any(|kind| !valid_text(kind, 128))
        {
            return Err(invalid(
                "production resource strings or selector count exceed their bounds",
            ));
        }
    }
    let requested: Vec<_> = input
        .iter()
        .map(|resource| MaterialDemand {
            key: resource.key.clone(),
            units: 1,
            item_types: resource.item_types.clone(),
            subtype: resource.subtype,
            material_type: resource.material_type,
            material_index: resource.material_index,
        })
        .collect();
    let normalized = normalized_demands(&requested)?;
    for (index, left) in normalized.iter().enumerate() {
        for right in &normalized[index + 1..] {
            work.charge(1)?;
            if selectors_overlap(left, right) {
                return Err(invalid(
                    "production resource selectors overlap; one physical stack cannot represent two model resources",
                ));
            }
        }
    }
    Ok(normalized)
}

struct Model {
    quotas: Vec<ProductionQuota>,
    recipes: Vec<ProductionRecipe>,
}

fn normalize_model(
    resources: &[MaterialDemand],
    quotas: &[ProductionQuota],
    recipes: &[ProductionRecipe],
    work: &mut Work,
) -> Result<Model> {
    if quotas.is_empty() || quotas.len() > 64 || recipes.len() > MAX_CHAIN_RECIPES {
        return Err(invalid(
            "production chain requires 1..64 quotas and at most 32 recipes",
        ));
    }
    let keys: BTreeSet<_> = resources
        .iter()
        .map(|resource| resource.key.as_str())
        .collect();
    let mut goals = BTreeMap::<String, u32>::new();
    for quota in quotas {
        work.charge(1)?;
        if !valid_text(&quota.item_token, 64) || !keys.contains(quota.item_token.as_str()) {
            return Err(invalid("production quota references an undefined resource"));
        }
        let minimum = goals.entry(quota.item_token.clone()).or_default();
        *minimum = (*minimum).max(quota.minimum_stock);
    }
    let mut normalized = BTreeMap::new();
    for recipe in recipes {
        work.charge(1)?;
        if !valid_text(&recipe.output_token, 64)
            || !keys.contains(recipe.output_token.as_str())
            || recipe.output_batch_size == 0
            || recipe.input_tokens.len() > MAX_CHAIN_RESOURCES
            || !valid_text(&recipe.job_token, 128)
        {
            return Err(invalid(
                "invalid production recipe output, yield, job token or input count",
            ));
        }
        match &recipe.workshop {
            BuildingKind::Workshop(name) | BuildingKind::Furnace(name) if valid_text(name, 128) => {
            }
            _ => {
                return Err(invalid(
                    "production recipe must declare a bounded workshop or furnace type",
                ));
            }
        }
        let mut inputs = BTreeMap::<String, u32>::new();
        for (key, amount) in &recipe.input_tokens {
            work.charge(1)?;
            if !valid_text(key, 64) || !keys.contains(key.as_str()) || *amount == 0 {
                return Err(invalid(
                    "production recipe input references an undefined resource or zero units",
                ));
            }
            let total = inputs.entry(key.clone()).or_default();
            *total = total
                .checked_add(*amount)
                .ok_or_else(|| exhausted("production input coefficient overflow"))?;
        }
        let value = ProductionRecipe {
            output_token: recipe.output_token.clone(),
            output_batch_size: recipe.output_batch_size,
            input_tokens: inputs.into_iter().collect(),
            workshop: recipe.workshop.clone(),
            job_token: recipe.job_token.clone(),
        };
        if normalized
            .insert(recipe.output_token.clone(), value)
            .is_some()
        {
            return Err(invalid(
                "production model permits exactly one recipe per output resource",
            ));
        }
    }
    Ok(Model {
        quotas: goals
            .into_iter()
            .map(|(item_token, minimum_stock)| ProductionQuota {
                item_token,
                minimum_stock,
            })
            .collect(),
        recipes: normalized.into_values().collect(),
    })
}

fn put_text(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn model_digest(
    anchor: StateAnchor,
    source: Digest32,
    resources: &[MaterialDemand],
    model: &Model,
    work: &mut Work,
) -> Result<Digest32> {
    let mut bytes = b"dfmcp-production-chain-model-v1\0".to_vec();
    put_text(&mut bytes, CHAIN_POLICY);
    put_text(&mut bytes, SUPPLY_POLICY);
    for value in [
        anchor.fortress_id.get(),
        anchor.cursor.epoch,
        anchor.cursor.sequence,
        anchor.tick.0,
    ] {
        bytes.extend_from_slice(&value.to_be_bytes());
    }
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
    bytes.extend_from_slice(source.as_bytes());
    bytes.extend_from_slice(&(resources.len() as u32).to_be_bytes());
    for resource in resources {
        work.charge(1)?;
        put_text(&mut bytes, &resource.key);
        bytes.extend_from_slice(&(resource.item_types.len() as u32).to_be_bytes());
        for kind in &resource.item_types {
            work.charge(1)?;
            put_text(&mut bytes, kind);
        }
        for value in [
            resource.subtype,
            resource.material_type,
            resource.material_index,
        ] {
            bytes.push(u8::from(value.is_some()));
            if let Some(value) = value {
                bytes.extend_from_slice(&value.to_be_bytes());
            }
        }
    }
    bytes.extend_from_slice(&(model.quotas.len() as u32).to_be_bytes());
    for quota in &model.quotas {
        work.charge(1)?;
        put_text(&mut bytes, &quota.item_token);
        bytes.extend_from_slice(&quota.minimum_stock.to_be_bytes());
    }
    bytes.extend_from_slice(&(model.recipes.len() as u32).to_be_bytes());
    for recipe in &model.recipes {
        work.charge(1)?;
        put_text(&mut bytes, &recipe.output_token);
        put_text(&mut bytes, &recipe.job_token);
        match &recipe.workshop {
            BuildingKind::Workshop(name) => {
                bytes.push(0);
                put_text(&mut bytes, name);
            }
            BuildingKind::Furnace(name) => {
                bytes.push(1);
                put_text(&mut bytes, name);
            }
            _ => return Err(invariant("normalized production workshop changed kind")),
        }
        bytes.extend_from_slice(&recipe.output_batch_size.to_be_bytes());
        bytes.extend_from_slice(&(recipe.input_tokens.len() as u32).to_be_bytes());
        for (token, amount) in &recipe.input_tokens {
            work.charge(1)?;
            put_text(&mut bytes, token);
            bytes.extend_from_slice(&amount.to_be_bytes());
        }
    }
    Ok(Digest32::of_bytes(&bytes))
}

/// Analyze a declared production model against a single complete captured item
/// roster. Current authority, exact anchor and whole-projection scan limits apply.
/// A model shortage concerns this conservative subset, not native job feasibility.
pub fn plan_production_chain<S: OperationsStateView + ?Sized>(
    state: &S,
    context: &OperationContext,
    resources: &[ProductionResource],
    quotas: &[ProductionQuota],
    recipes: &[ProductionRecipe],
    max_work: u64,
) -> Result<ProductionChainAnalysis> {
    let mut work = Work::new(max_work, context.budget.max_wall_millis)?;
    let (observation, snapshot, source_digest) = source(state, context)?;
    let selectors = selectors(resources, &mut work)?;
    let model = normalize_model(&selectors, quotas, recipes, &mut work)?;
    let model_digest = model_digest(context.anchor, source_digest, &selectors, &model, &mut work)?;
    let inventory = InventoryIndex::build(observation, &mut work)?;
    let mut resources: Vec<_> = selectors
        .iter()
        .map(|selector| ProductionResourceStock {
            resource: ProductionResource {
                key: selector.key.clone(),
                item_types: selector.item_types.clone(),
                subtype: selector.subtype,
                material_type: selector.material_type,
                material_index: selector.material_index,
            },
            stock_units: 0,
            eligible_items: 0,
            examples: Vec::new(),
        })
        .collect();
    let mut excluded_items = BTreeMap::new();
    let mut unmatched_items = 0u64;
    for (index, item) in observation.items.iter().enumerate() {
        work.charge(1)?;
        let reason = if item.stack_size == 0 {
            Some("zero_stack_size")
        } else {
            exclusion(inventory.inherited[index])
        };
        if let Some(reason) = reason {
            *excluded_items.entry(reason).or_default() += 1;
            continue;
        }
        let mut matched = false;
        for (selector, resource) in selectors.iter().zip(&mut resources) {
            work.charge(1)?;
            if !selector.matches(item) {
                continue;
            }
            if matched {
                return Err(invariant(
                    "validated disjoint production resources matched the same item",
                ));
            }
            matched = true;
            resource.stock_units = resource
                .stock_units
                .checked_add(item.stack_size)
                .ok_or_else(|| {
                    exhausted("captured production resource exceeds the u32 model domain")
                })?;
            resource.eligible_items += 1;
            if resource.examples.len() < EXAMPLES {
                resource
                    .examples
                    .push(handle(snapshot, item_entity_id(item.native_id))?);
            }
        }
        if !matched {
            unmatched_items += 1;
        }
    }
    let mut stock = InventoryStockpile::new();
    for resource in &resources {
        work.charge(1)?;
        stock.set_stock(resource.resource.key.clone(), resource.stock_units);
    }
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    for recipe in &model.recipes {
        work.charge(1)?;
        compiler.register_recipe(recipe.clone());
    }
    let plan = compiler.plan_quotas(
        &model.quotas,
        &stock,
        ProductionPlanningLimits {
            max_orders: MAX_CHAIN_RECIPES,
            max_resources: MAX_CHAIN_RESOURCES,
            max_edges: MAX_CHAIN_RESOURCES * MAX_CHAIN_RECIPES,
            max_work: work.maximum.saturating_sub(work.used),
        },
    )?;
    work.charge(plan.work_used())?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(ProductionChainAnalysis {
        anchor: snapshot.anchor(),
        source_digest,
        model_digest,
        resources,
        quotas: model.quotas,
        recipes: model.recipes,
        plan,
        excluded_items,
        unmatched_items,
        work_units: work.used,
    })
}
