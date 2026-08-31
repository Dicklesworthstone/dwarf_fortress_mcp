#![forbid(unsafe_code)]

//! JIT Manager Production Logistics and Workshop Work-Order Compiler.
//!
//! WP-PLN-02: Compiles multi-tier material dependency graphs into structured
//! DFHack manager work orders with automatic stock threshold conditions and deadlock prevention.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use dfmcp_core::Result;

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
        self.item_counts.get(token).copied().unwrap_or(0)
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
        let current_stock = inventory.get_stock(target_token);
        if current_stock >= target_amount {
            return Ok(Vec::new()); // Quota already met
        }

        let deficit = target_amount.saturating_sub(current_stock);
        let mut work_orders = Vec::new();
        let mut visited = BTreeSet::new();
        let mut queue = VecDeque::new();

        queue.push_back((target_token.to_owned(), deficit));

        while let Some((token, required_amount)) = queue.pop_front() {
            if !visited.insert(token.clone()) {
                continue;
            }

            if let Some(recipe) = self.recipes.get(&token) {
                let batches = required_amount.div_ceil(recipe.output_batch_size);
                let order_count = batches * recipe.output_batch_size;

                // Check prerequisites
                for (input_token, needed_per_batch) in &recipe.input_tokens {
                    let total_input_needed = needed_per_batch * batches;
                    let current_input_stock = inventory.get_stock(input_token);

                    if current_input_stock < total_input_needed {
                        let input_deficit = total_input_needed - current_input_stock;
                        queue.push_back((input_token.clone(), input_deficit));
                    }
                }

                // Create work order action with threshold conditions
                let conditions = vec![WorkOrderCondition::ItemCountBelow {
                    item_token: token.clone(),
                    threshold: target_amount,
                }];

                work_orders.push(Action::CreateWorkOrder {
                    name: format!("Auto-JIT: {}", recipe.job_token),
                    job_token: recipe.job_token.clone(),
                    amount: order_count,
                    conditions,
                });
            }
        }

        // Return prerequisite orders first (reverse topological order)
        work_orders.reverse();
        Ok(work_orders)
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
