#![forbid(unsafe_code)]

//! JIT Manager Production Logistics and Workshop Work-Order Compiler.
//!
//! WP-PLN-02: Compiles multi-tier material dependency graphs into structured
//! DFHack manager work orders with automatic stock threshold conditions and deadlock prevention.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{DfmcpError, ErrorCode, Result};

use crate::action::{Action, BuildingKind, WorkOrderCondition};

/// Represents a single transformation recipe in the fortress manufacturing pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductionRecipe {
    pub output_token: String,
    pub output_batch_size: u32,
    pub input_tokens: Vec<(String, u32)>, // (input_item_token, quantity_needed)
    pub workshop: BuildingKind,
    pub job_token: String,
}

/// Fortress inventory snapshot for logistics planning.
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

    /// Get current stock count for an item token (defaults to 0).
    #[must_use]
    pub fn get_stock(&self, token: &str) -> u32 {
        self.item_counts.get(token).copied().map_or(0, |count| count)
    }
}

/// Production Logistics Compiler and Work-Order Generator.
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
    /// Initialize with standard Dwarf Fortress production recipes.
    #[must_use]
    pub fn with_standard_recipes() -> Self {
        let mut compiler = Self {
            recipes: BTreeMap::new(),
        };

        // Brewing: Plants -> Drink (Still)
        compiler.register_recipe(ProductionRecipe {
            output_token: "DRINK".to_owned(),
            output_batch_size: 5,
            input_tokens: vec![("PLANT".to_owned(), 1), ("BARREL".to_owned(), 1)],
            workshop: BuildingKind::Workshop("Still".to_owned()),
            job_token: "BrewDrink".to_owned(),
        });

        // Woodworking: Wood -> Barrel (Carpenter's Workshop)
        compiler.register_recipe(ProductionRecipe {
            output_token: "BARREL".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("WOOD".to_owned(), 1)],
            workshop: BuildingKind::Workshop("Carpenters".to_owned()),
            job_token: "MakeWoodenBarrel".to_owned(),
        });

        // Woodworking: Wood -> Bin (Carpenter's Workshop)
        compiler.register_recipe(ProductionRecipe {
            output_token: "BIN".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("WOOD".to_owned(), 1)],
            workshop: BuildingKind::Workshop("Carpenters".to_owned()),
            job_token: "MakeWoodenBin".to_owned(),
        });

        // Smelting: Wood -> Charcoal (Wood Furnace)
        compiler.register_recipe(ProductionRecipe {
            output_token: "CHARCOAL".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("WOOD".to_owned(), 1)],
            workshop: BuildingKind::Furnace("WoodFurnace".to_owned()),
            job_token: "MakeCharcoal".to_owned(),
        });

        // Smelting: Iron Ore + Charcoal -> Iron Bar (Smelter)
        compiler.register_recipe(ProductionRecipe {
            output_token: "BAR_IRON".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("ORE_HEMATITE".to_owned(), 1), ("CHARCOAL".to_owned(), 1)],
            workshop: BuildingKind::Furnace("Smelter".to_owned()),
            job_token: "SmeltIronOre".to_owned(),
        });

        // Weaponsmithing: Iron Bar + Charcoal -> Iron Short Sword (Forge)
        compiler.register_recipe(ProductionRecipe {
            output_token: "WEAPON_SWORD_SHORT_IRON".to_owned(),
            output_batch_size: 1,
            input_tokens: vec![("BAR_IRON".to_owned(), 2), ("CHARCOAL".to_owned(), 1)],
            workshop: BuildingKind::Furnace("MetalsmithsForge".to_owned()),
            job_token: "ForgeIronShortSword".to_owned(),
        });

        compiler
    }

    /// Register a custom production recipe.
    pub fn register_recipe(&mut self, recipe: ProductionRecipe) {
        self.recipes.insert(recipe.output_token.clone(), recipe);
    }

    /// Compile a target production quota, resolving upstream prerequisite supply chains.
    pub fn compile_quota_work_orders(
        &self,
        target_token: &str,
        target_amount: u32,
        inventory: &InventoryStockpile,
    ) -> Result<Vec<Action>> {
        if target_token.is_empty() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "production target token must be nonempty",
            ));
        }
        let current_stock = inventory.get_stock(target_token);
        if current_stock >= target_amount {
            return Ok(Vec::new()); // Quota already met
        }

        let mut requirements = BTreeMap::new();
        let mut batches = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        self.accumulate_requirement(
            target_token,
            target_amount,
            inventory,
            &mut requirements,
            &mut batches,
            &mut visiting,
        )?;

        let mut work_orders = Vec::new();
        let mut emitted = BTreeSet::new();
        let mut emit_visiting = BTreeSet::new();
        self.emit_orders(
            target_token,
            &requirements,
            &batches,
            &mut emitted,
            &mut emit_visiting,
            &mut work_orders,
        )?;
        Ok(work_orders)
    }

    fn accumulate_requirement(
        &self,
        token: &str,
        additional_required: u32,
        inventory: &InventoryStockpile,
        requirements: &mut BTreeMap<String, u32>,
        batches: &mut BTreeMap<String, u32>,
        visiting: &mut BTreeSet<String>,
    ) -> Result<()> {
        let prior_required = requirements.get(token).copied().map_or(0, |count| count);
        let total_required = prior_required
            .checked_add(additional_required)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    format!("production requirement overflow for '{token}'"),
                )
            })?;
        requirements.insert(token.to_owned(), total_required);

        let shortage = total_required.saturating_sub(inventory.get_stock(token));
        if shortage == 0 {
            return Ok(());
        }
        let recipe = self.recipes.get(token).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!(
                    "insufficient stock for raw or unknown production token '{token}' and no recipe is registered"
                ),
            )
        })?;
        if recipe.output_batch_size == 0 {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("recipe for '{token}' has a zero output batch size"),
            ));
        }

        let required_batches_u64 =
            u64::from(shortage).div_ceil(u64::from(recipe.output_batch_size));
        let required_batches = u32::try_from(required_batches_u64).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                format!("production batch count overflow for '{token}'"),
            )
        })?;
        let prior_batches = batches.get(token).copied().map_or(0, |count| count);
        if required_batches <= prior_batches {
            return Ok(());
        }
        let extra_batches = required_batches - prior_batches;
        batches.insert(token.to_owned(), required_batches);

        if !visiting.insert(token.to_owned()) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                format!("cyclic production dependency detected at '{token}'"),
            ));
        }
        let mut inputs = recipe.input_tokens.clone();
        inputs.sort_by(|left, right| left.0.cmp(&right.0));
        for (input_token, needed_per_batch) in inputs {
            let input_required = needed_per_batch.checked_mul(extra_batches).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    format!("production input requirement overflow for '{input_token}'"),
                )
            })?;
            self.accumulate_requirement(
                &input_token,
                input_required,
                inventory,
                requirements,
                batches,
                visiting,
            )?;
        }
        visiting.remove(token);
        Ok(())
    }

    fn emit_orders(
        &self,
        token: &str,
        requirements: &BTreeMap<String, u32>,
        batches: &BTreeMap<String, u32>,
        emitted: &mut BTreeSet<String>,
        visiting: &mut BTreeSet<String>,
        output: &mut Vec<Action>,
    ) -> Result<()> {
        let Some(&batch_count) = batches.get(token) else {
            return Ok(());
        };
        if emitted.contains(token) {
            return Ok(());
        }
        if !visiting.insert(token.to_owned()) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                format!("cyclic production dependency detected at '{token}'"),
            ));
        }
        let recipe = self.recipes.get(token).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                format!("planned production token '{token}' lost its recipe"),
            )
        })?;
        let mut inputs: Vec<&str> = recipe
            .input_tokens
            .iter()
            .map(|(input, _)| input.as_str())
            .collect();
        inputs.sort_unstable();
        inputs.dedup();
        for input in inputs {
            self.emit_orders(input, requirements, batches, emitted, visiting, output)?;
        }
        visiting.remove(token);

        let threshold = requirements.get(token).copied().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                format!("planned production token '{token}' has no requirement"),
            )
        })?;
        output.push(Action::CreateWorkOrder {
            name: format!("Auto-JIT: {}", recipe.job_token),
            job_token: recipe.job_token.clone(),
            amount: batch_count,
            conditions: vec![WorkOrderCondition::ItemCountBelow {
                item_token: token.to_owned(),
                threshold,
            }],
        });
        emitted.insert(token.to_owned());
        Ok(())
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
        inventory.set_stock("BARREL", 0); // Out of barrels!
        inventory.set_stock("WOOD", 100);

        // Target: 50 drinks (deficit 40 drinks = 8 batches = requires 8 barrels)
        let actions = compiler.compile_quota_work_orders("DRINK", 50, &inventory)?;

        // Should produce 2 work orders: first MakeWoodenBarrel, then BrewDrink
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
