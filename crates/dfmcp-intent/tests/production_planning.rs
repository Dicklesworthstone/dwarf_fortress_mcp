//! Joint-quota regressions exercise the public compiler used by the legacy API.
use std::collections::BTreeMap;

use dfmcp_core::{ErrorCode, Result};
use dfmcp_intent::{
    BuildingKind, InventoryStockpile, ProductionLogisticsCompiler, ProductionPlan,
    ProductionPlanningLimits, ProductionQuota, ProductionRecipe,
};

fn quota(token: &str, amount: u32) -> ProductionQuota {
    ProductionQuota { item_token: token.to_owned(), minimum_stock: amount }
}

fn recipe(token: &str, yield_units: u32, inputs: &[(&str, u32)]) -> ProductionRecipe {
    ProductionRecipe {
        output_token: token.to_owned(),
        output_batch_size: yield_units,
        input_tokens: inputs.iter().map(|(token, n)| ((*token).to_owned(), *n)).collect(),
        workshop: BuildingKind::Workshop("declared".to_owned()),
        job_token: format!("Make{token}"),
    }
}

fn plan(
    compiler: &ProductionLogisticsCompiler,
    quotas: &[ProductionQuota],
    stock: &InventoryStockpile,
) -> Result<ProductionPlan> {
    compiler.plan_quotas(quotas, stock, ProductionPlanningLimits::default())
}

#[test]
fn joint_goals_cannot_each_spend_the_same_initial_stock() -> Result<()> {
    let compiler = ProductionLogisticsCompiler::default();
    let mut stock = InventoryStockpile::new();
    stock.set_stock("WOOD", 3);
    let before = stock.item_counts.clone();
    assert!(compiler.compile_quota_work_orders("BARREL", 2, &stock).is_ok());
    assert!(compiler.compile_quota_work_orders("BIN", 2, &stock).is_ok());
    let report = plan(&compiler, &[quota("BARREL", 2), quota("BIN", 2)], &stock)?;
    assert!(!report.model_feasible());
    assert_eq!(report.shortages().len(), 1);
    assert_eq!(report.shortages()[0].item_token, "WOOD");
    assert_eq!(report.shortages()[0].required_units, 4);
    assert_eq!(report.shortages()[0].missing_units, 1);
    assert!(report.into_work_orders().is_err_and(|e| e.code == ErrorCode::PreconditionsFailed));
    assert_eq!(stock.item_counts, before);
    Ok(())
}

#[test]
fn output_quota_is_preserved_when_other_recipes_consume_that_output() -> Result<()> {
    let compiler = ProductionLogisticsCompiler::default();
    let mut stock = InventoryStockpile::new();
    stock.set_stock("WOOD", 2);
    stock.set_stock("CHARCOAL", 1);
    stock.set_stock("ORE_HEMATITE", 1);
    let report = plan(&compiler, &[quota("CHARCOAL", 2), quota("BAR_IRON", 1)], &stock)?;
    assert!(report.model_feasible());
    let balance = report.requirements().iter().find(|r| r.item_token == "CHARCOAL");
    assert!(balance.is_some_and(|r| r.minimum_stock == 2 && r.consumed_units == 1
        && r.stock_units == 1 && r.planned_units == 2 && r.surplus_units == 0));
    assert_eq!(report.steps()[0].output_token, "CHARCOAL");
    assert_eq!(report.steps()[1].depends_on, vec![0]);
    Ok(())
}

#[test]
fn diamond_consumers_share_one_rounded_supplier_batch() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("C", 1)]));
    compiler.register_recipe(recipe("B", 1, &[("C", 1)]));
    compiler.register_recipe(recipe("C", 3, &[("RAW", 2)]));
    let mut stock = InventoryStockpile::new();
    stock.set_stock("RAW", 2);
    let report = plan(&compiler, &[quota("A", 1), quota("B", 1)], &stock)?;
    assert!(report.model_feasible());
    assert_eq!(report.steps().len(), 3);
    assert_eq!(report.steps()[0].output_token, "C");
    assert_eq!(report.steps()[0].batches, 1);
    assert_eq!(report.steps()[0].input_units, vec![("RAW".to_owned(), 2)]);
    assert!(report.requirements().iter().any(|r| r.item_token == "C" && r.surplus_units == 1));
    for step in &report.steps()[1..] { assert_eq!(step.depends_on, vec![0]); }
    Ok(())
}

#[test]
fn complete_shortages_include_raw_goals_and_all_missing_leaves() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("WOOD", 2), ("ORE", 3)]));
    let report = plan(&compiler, &[quota("A", 2), quota("WATER", 1)], &InventoryStockpile::new())?;
    let deficits: Vec<_> = report.shortages().iter().map(|r| (r.item_token.as_str(), r.missing_units)).collect();
    assert_eq!(deficits, vec![("ORE", 6), ("WATER", 1), ("WOOD", 4)]);
    assert_eq!(report.steps().len(), 1); // Hypothetical, not dispatchable.
    Ok(())
}

#[test]
fn quota_order_and_duplicate_minima_do_not_change_the_normal_form() -> Result<()> {
    let compiler = ProductionLogisticsCompiler::default();
    let stock = InventoryStockpile::new();
    let first = plan(&compiler, &[quota("BIN", 2), quota("BARREL", 3)], &stock)?;
    let reordered = plan(&compiler, &[quota("BARREL", 3), quota("BIN", 2)], &stock)?;
    assert_eq!(first, reordered);
    let duplicate = plan(&compiler, &[quota("BIN", 1), quota("BARREL", 3), quota("BIN", 2)], &stock)?;
    assert_eq!(first.requirements(), duplicate.requirements());
    assert_eq!(first.steps(), duplicate.steps());
    assert_eq!(first.shortages(), duplicate.shortages());
    // Work accounts for each submitted quota, including duplicates.
    assert_eq!(duplicate.work_used(), first.work_used() + 1);
    Ok(())
}

#[test]
fn recipe_input_order_and_duplicate_inputs_are_normalized() -> Result<()> {
    let mut left = ProductionLogisticsCompiler::without_recipes();
    left.register_recipe(recipe("A", 1, &[("X", 1), ("Y", 2), ("X", 2)]));
    let mut right = ProductionLogisticsCompiler::without_recipes();
    right.register_recipe(recipe("A", 1, &[("Y", 2), ("X", 3)]));
    let stock = InventoryStockpile::new();
    let a = plan(&left, &[quota("A", 2)], &stock)?;
    let b = plan(&right, &[quota("A", 2)], &stock)?;
    assert_eq!(a.steps(), b.steps());
    assert_eq!(a.requirements(), b.requirements());
    assert_eq!(a.shortages(), b.shortages());
    Ok(())
}

#[test]
fn unused_invalid_or_cyclic_catalog_is_not_an_active_dependency() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("B", 1)]));
    compiler.register_recipe(recipe("B", 1, &[("A", 1)]));
    compiler.register_recipe(recipe("UNUSED", 0, &[("", 0)]));
    let mut stock = InventoryStockpile::new();
    stock.set_stock("B", 1);
    let report = plan(&compiler, &[quota("A", 1)], &stock)?;
    assert!(report.model_feasible());
    assert_eq!(report.steps().len(), 1);
    stock.set_stock("B", 0);
    assert!(plan(&compiler, &[quota("A", 1)], &stock).is_err_and(|e| e.code == ErrorCode::Conflict));
    stock.set_stock("A", 1);
    assert!(plan(&compiler, &[quota("A", 1)], &stock)?.steps().is_empty());
    Ok(())
}

#[test]
fn self_cycle_is_refused_without_recursing() {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 2, &[("A", 1)]));
    assert!(plan(&compiler, &[quota("A", 1)], &InventoryStockpile::new())
        .is_err_and(|e| e.code == ErrorCode::Conflict));
}

#[test]
fn deep_chain_is_iterative_and_order_limit_is_enforced() -> Result<()> {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    for n in 0..250 {
        compiler.register_recipe(recipe(&format!("N{n:03}"), 1, &[(&format!("N{:03}", n + 1), 1)]));
    }
    let mut stock = InventoryStockpile::new();
    stock.set_stock("N250", 1);
    let report = plan(&compiler, &[quota("N000", 1)], &stock)?;
    assert_eq!(report.steps().len(), 250);
    assert!(report.model_feasible());
    assert_eq!(report.expansion_rounds(), 251);
    assert!(report.work_used() <= 1_000_000);
    for (index, step) in report.steps().iter().enumerate() {
        assert!(step.depends_on.iter().all(|dependency| *dependency < index));
    }
    compiler.register_recipe(recipe("N250", 1, &[("N251", 1)]));
    stock.set_stock("N250", 0);
    assert!(plan(&compiler, &[quota("N000", 1)], &stock)
        .is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn work_resource_edge_and_order_budgets_fail_without_partial_plans() {
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 1, &[("X", 1), ("Y", 1)]));
    compiler.register_recipe(recipe("X", 1, &[("Z", 1)]));
    let defaults = ProductionPlanningLimits::default();
    for limits in [
        ProductionPlanningLimits { max_work: 0, ..defaults },
        ProductionPlanningLimits { max_resources: 2, ..defaults },
        ProductionPlanningLimits { max_edges: 1, ..defaults },
        ProductionPlanningLimits { max_orders: 1, ..defaults },
    ] {
        assert!(compiler.plan_quotas(&[quota("A", 1)], &InventoryStockpile::new(), limits)
            .is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    }
    assert!(compiler.plan_quotas(&[quota("A", 1)], &InventoryStockpile::new(),
        ProductionPlanningLimits { max_orders: 251, ..defaults })
        .is_err_and(|e| e.code == ErrorCode::InvalidRequest));
}

#[test]
fn reachable_invalid_recipes_and_shape_limits_are_rejected() {
    for invalid in [recipe("A", 0, &[]), recipe("A", 1, &[("X", 0)]), recipe("A", 1, &[("", 1)])] {
        let mut compiler = ProductionLogisticsCompiler::without_recipes();
        compiler.register_recipe(invalid);
        assert!(plan(&compiler, &[quota("A", 1)], &InventoryStockpile::new())
            .is_err_and(|e| e.code == ErrorCode::InvalidRequest));
    }
    let compiler = ProductionLogisticsCompiler::without_recipes();
    let stock = InventoryStockpile::new();
    for quotas in [vec![], vec![quota("X", 1); 65], vec![quota("", 1)],
        vec![quota(&"x".repeat(257), 1)], vec![quota("x\0y", 1)], vec![quota(&"é".repeat(129), 1)]] {
        assert!(plan(&compiler, &quotas, &stock).is_err_and(|e| e.code == ErrorCode::InvalidRequest));
    }
}

#[test]
fn checked_arithmetic_refuses_requirement_input_and_output_overflow() {
    let stock = InventoryStockpile::new();
    let mut compiler = ProductionLogisticsCompiler::without_recipes();
    compiler.register_recipe(recipe("A", 2, &[]));
    assert!(plan(&compiler, &[quota("A", u32::MAX)], &stock)
        .is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    compiler.register_recipe(recipe("A", 1, &[("X", u32::MAX)]));
    assert!(plan(&compiler, &[quota("A", 2)], &stock)
        .is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    compiler.register_recipe(recipe("A", 1, &[("X", 1)]));
    assert!(plan(&compiler, &[quota("A", 1), quota("X", u32::MAX)], &stock)
        .is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
    compiler.register_recipe(recipe("A", 1, &[("X", u32::MAX), ("X", 1)]));
    assert!(plan(&compiler, &[quota("A", 1)], &stock)
        .is_err_and(|e| e.code == ErrorCode::BudgetExceeded));
}

#[test]
fn all_small_acyclic_models_match_an_independent_batch_enumeration() -> Result<()> {
    // Three products, six possible forward edges including the RAW leaf.
    // The oracle enumerates batch vectors, rather than expanding requirements.
    let names = ["A", "B", "C", "RAW"];
    let edges = [(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)];
    for mask in 0..64u32 {
        for yield_mask in 0..8u32 {
            for stock_mask in 0..8u32 {
                let mut compiler = ProductionLogisticsCompiler::without_recipes();
                let mut stock = InventoryStockpile::new();
                let mut yields = [1u32; 3];
                let mut initial = [0u32; 4];
                initial[3] = 32;
                stock.set_stock("RAW", 32);
                for product in 0..3 {
                    yields[product] += (yield_mask >> product) & 1;
                    initial[product] = (stock_mask >> product) & 1;
                    stock.set_stock(names[product], initial[product]);
                    let inputs: Vec<_> = edges.iter().enumerate()
                        .filter(|(bit, (from, _))| *from == product && (mask & (1 << bit)) != 0)
                        .map(|(_, (_, to))| (names[*to], 1)).collect();
                    compiler.register_recipe(recipe(names[product], yields[product], &inputs));
                }
                let report = plan(&compiler, &[quota("A", 2), quota("B", 1), quota("C", 1)], &stock)?;
                assert!(report.model_feasible());
                let actual: BTreeMap<_, _> = report.steps().iter()
                    .map(|step| (step.output_token.as_str(), step.batches)).collect();
                let actual = names[..3].iter().map(|name| actual.get(name).copied().unwrap_or_default()).collect::<Vec<_>>();
                let mut best: Option<(u32, Vec<u32>)> = None;
                for a in 0..=2u32 {
                    for b in 0..=3u32 {
                        for c in 0..=6u32 {
                            let batches = vec![a, b, c];
                            let mut demand = [2u32, 1, 1, 0];
                            for (bit, (from, to)) in edges.iter().enumerate() {
                                if (mask & (1 << bit)) != 0 { demand[*to] += batches[*from]; }
                            }
                            if (0..3).all(|n| initial[n] + batches[n] * yields[n] >= demand[n])
                                && initial[3] >= demand[3] {
                                let candidate = (a + b + c, batches);
                                if best.as_ref().is_none_or(|prior| &candidate < prior) { best = Some(candidate); }
                            }
                        }
                    }
                }
                assert_eq!(best.map(|(_, batches)| batches), Some(actual), "mask={mask}, yields={yield_mask}, stock={stock_mask}");
                for balance in report.requirements() {
                    assert_eq!(u64::from(balance.stock_units) + u64::from(balance.planned_units),
                        u64::from(balance.minimum_stock) + u64::from(balance.consumed_units) + balance.surplus_units);
                    assert_eq!(balance.missing_units, 0);
                }
            }
        }
    }
    Ok(())
}
