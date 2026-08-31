#![forbid(unsafe_code)]

//! Integration tests for WP-PLN-02 JIT Production Logistics & Work Orders.

use dfmcp_core::Result;
use dfmcp_intent::Action;
use dfmcp_intent::logistics::{InventoryStockpile, ProductionLogisticsCompiler};

#[test]
fn test_weaponsmithing_multi_level_prerequisites() -> Result<()> {
    let compiler = ProductionLogisticsCompiler::default();
    let mut inventory = InventoryStockpile::new();

    // Have raw hematite and wood, need 5 iron swords
    inventory.set_stock("WEAPON_SWORD_SHORT_IRON", 0);
    inventory.set_stock("BAR_IRON", 0);
    inventory.set_stock("CHARCOAL", 0);
    inventory.set_stock("ORE_HEMATITE", 100);
    inventory.set_stock("WOOD", 100);

    let orders = compiler.compile_quota_work_orders("WEAPON_SWORD_SHORT_IRON", 5, &inventory)?;

    assert!(!orders.is_empty());
    // Should compile: MakeCharcoal, SmeltIronOre, ForgeIronShortSword in order
    let job_tokens: Vec<String> = orders
        .iter()
        .filter_map(|action| match action {
            Action::CreateWorkOrder { job_token, .. } => Some(job_token.clone()),
            _ => None,
        })
        .collect();

    assert!(job_tokens.contains(&"MakeCharcoal".to_owned()));
    assert!(job_tokens.contains(&"SmeltIronOre".to_owned()));
    assert!(job_tokens.contains(&"ForgeIronShortSword".to_owned()));

    Ok(())
}
