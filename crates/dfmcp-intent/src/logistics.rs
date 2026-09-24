#![forbid(unsafe_code)]

//! JIT production proposals over a declared, single-output recipe model.
//!
//! Joint quotas share initial stock and supplier batches. This model does not
//! establish native recipe semantics, workshop readiness or mutation authority.

use std::collections::BTreeMap;

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::action::{Action, BuildingKind};

#[path = "logistics_planning.rs"]
mod planning;

pub use planning::{
    ProductionPlan, ProductionPlanningLimits, ProductionQuota, ProductionRequirement,
    ProductionShortage, ProductionStep,
};

/// Represents a single transformation in the caller's declared production model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionRecipe {
    pub output_token: String,
    pub output_batch_size: u32,
    pub input_tokens: Vec<(String, u32)>,
    pub workshop: BuildingKind,
    pub job_token: String,
}

/// Declared initial stock for logistics planning. Missing keys mean zero only
/// inside this model; callers must not coerce unknown observations into this map.
#[derive(Clone, Debug, Default)]
pub struct InventoryStockpile {
    pub item_counts: BTreeMap<String, u32>,
}

impl InventoryStockpile {
    #[must_use]
    pub fn new() -> Self {
        Self {
            item_counts: BTreeMap::new(),
        }
    }

    /// Set or update stock count for an item token.
    pub fn set_stock(&mut self, token: impl Into<String>, count: u32) {
        self.item_counts.insert(token.into(), count);
    }

    /// Get declared stock count for an item token (defaults to zero).
    #[must_use]
    pub fn get_stock(&self, token: &str) -> u32 {
        self.item_counts
            .get(token)
            .copied()
            .map_or(0, |count| count)
    }
}

/// Deterministic production proposal compiler. No game effects are dispatched.
#[derive(Clone, Debug)]
pub struct ProductionLogisticsCompiler {
    recipes: BTreeMap<String, ProductionRecipe>,
}

impl Default for ProductionLogisticsCompiler {
    fn default() -> Self {
        Self::with_standard_recipes()
    }
}

impl ProductionLogisticsCompiler {
    /// Start a closed caller-declared model without the illustrative catalog.
    #[must_use]
    pub fn without_recipes() -> Self {
        Self {
            recipes: BTreeMap::new(),
        }
    }

    /// Initialize the existing illustrative production catalog. These quantities
    /// are model assumptions, not certified native DF recipe semantics.
    #[must_use]
    pub fn with_standard_recipes() -> Self {
        let mut compiler = Self::without_recipes();

        compiler.register_recipe(ProductionRecipe {
            output_token: "DRINK".to_owned(),
            output_batch_size: 5,
            input_tokens: vec![("PLANT".to_owned(), 1), ("BARREL".to_owned(), 1)],
            workshop: BuildingKind::Workshop("Still".to_owned()),
            job_token: "BrewDrink".to_owned(),
        });
        compiler.register_recipe(ProductionRecipe {
            output_token: "BARREL".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("WOOD".to_owned(), 1)],
            workshop: BuildingKind::Workshop("Carpenters".to_owned()),
            job_token: "MakeWoodenBarrel".to_owned(),
        });
        compiler.register_recipe(ProductionRecipe {
            output_token: "BIN".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("WOOD".to_owned(), 1)],
            workshop: BuildingKind::Workshop("Carpenters".to_owned()),
            job_token: "MakeWoodenBin".to_owned(),
        });
        compiler.register_recipe(ProductionRecipe {
            output_token: "CHARCOAL".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("WOOD".to_owned(), 1)],
            workshop: BuildingKind::Furnace("WoodFurnace".to_owned()),
            job_token: "MakeCharcoal".to_owned(),
        });
        compiler.register_recipe(ProductionRecipe {
            output_token: "BAR_IRON".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("ORE_HEMATITE".to_owned(), 1), ("CHARCOAL".to_owned(), 1)],
            workshop: BuildingKind::Furnace("Smelter".to_owned()),
            job_token: "SmeltIronOre".to_owned(),
        });
        compiler.register_recipe(ProductionRecipe {
            output_token: "WEAPON_SWORD_SHORT_IRON".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("BAR_IRON".to_owned(), 2), ("CHARCOAL".to_owned(), 1)],
            workshop: BuildingKind::Furnace("MetalsmithsForge".to_owned()),
            job_token: "ForgeIronShortSword".to_owned(),
        });
        compiler
    }

    /// Register or replace a declared recipe. Reachable recipes are validated
    /// before planning; unrelated catalog entries do not consume planning work.
    pub fn register_recipe(&mut self, recipe: ProductionRecipe) {
        self.recipes.insert(recipe.output_token.clone(), recipe);
    }

    /// Compile one quota using the same bounded solver as joint-quota planning.
    /// Infeasible models return no actions. Use `plan_quotas` for full deficits.
    pub fn compile_quota_work_orders(
        &self,
        target_token: &str,
        target_amount: u32,
        inventory: &InventoryStockpile,
    ) -> Result<Vec<Action>> {
        // Check before copying a caller-supplied string into the quota.
        if target_token.is_empty() || target_token.len() > 256 || target_token.contains('\0') {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "production target must contain 1..256 UTF-8 bytes without NUL",
            ));
        }
        self.compile_quotas_work_orders(
            &[ProductionQuota {
                item_token: target_token.to_owned(),
                minimum_stock: target_amount,
            }],
            inventory,
            ProductionPlanningLimits::default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{DfmcpError, ErrorCode};

    #[test]
    fn test_brewing_supply_chain_derives_barrels() -> Result<()> {
        let compiler = ProductionLogisticsCompiler::default();
        let mut inventory = InventoryStockpile::new();
        inventory.set_stock("DRINK", 10);
        inventory.set_stock("PLANT", 50);
        inventory.set_stock("BARREL", 0);
        inventory.set_stock("WOOD", 100);
        let actions = compiler.compile_quota_work_orders("DRINK", 50, &inventory)?;
        assert_eq!(actions.len(), 2);
        match &actions[0] {
            Action::CreateWorkOrder {
                job_token, amount, ..
            } => {
                assert_eq!(job_token, "MakeWoodenBarrel");
                assert_eq!(*amount, 8);
            }
            _ => {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "unexpected action variant",
                ));
            }
        }
        match &actions[1] {
            Action::CreateWorkOrder { job_token, .. } => {
                assert_eq!(job_token, "BrewDrink");
            }
            _ => {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "unexpected action variant",
                ));
            }
        }
        Ok(())
    }

    #[test]
    fn test_satisfied_stock_yields_empty_work_orders() -> Result<()> {
        let compiler = ProductionLogisticsCompiler::default();
        let mut inventory = InventoryStockpile::new();
        inventory.set_stock("DRINK", 100);
        let actions = compiler.compile_quota_work_orders("DRINK", 50, &inventory)?;
        assert!(actions.is_empty());
        Ok(())
    }
}
