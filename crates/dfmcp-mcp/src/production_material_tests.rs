//! Workshop setup and item production consume the same finite input budget.

use super::*;

fn economy(wood: u64) -> Result<WorldSnapshot> {
    let mut snapshot = scenario_snapshot("starter_fortress_economy", FortressId::new(991), false)?;
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(wood),
    )?;
    Ok(snapshot)
}

fn with_setup(quotas: Json, workshops: Json) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template":"production", "quotas":quotas,
            "prerequisites":{"assign_labor":false,"workshops":workshops},
        })
        .to_string(),
    )
}

#[test]
fn new_carpenters_and_pending_orders_share_one_exact_material_budget() -> Result<()> {
    let request = with_setup(
        json!([{"item":"BED","minimum":1},{"item":"CHAIR","minimum":1}]),
        json!([{"building":"workshop:Carpenters","location":[0,0,10]}]),
    )?;
    let mut snapshot = economy(3)?;
    snapshot.graph.entities.remove(&starter::CARPENTERS);
    prior_order(&mut snapshot, "CONSTRUCT_TABLE", 1)?;
    let original = snapshot.clone();
    let refusal = request
        .compile(&snapshot)
        .err()
        .ok_or_else(|| invalid("short plan admitted"))?;
    assert_eq!(refusal.code, ErrorCode::PreconditionsFailed);
    assert!(
        refusal
            .message
            .contains("shared laboratory material supply"),
        "{refusal}"
    );
    assert_eq!(
        snapshot, original,
        "infeasible setup must not allocate a log"
    );

    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(4),
    )?;
    let compiled = request.compile(&snapshot)?;
    let budget = &compiled.analysis["materials"]["WOOD"];
    assert_eq!(budget["observed_stock"], 4);
    assert_eq!(budget["reserved_for_existing_orders"], 1);
    assert_eq!(budget["reserved_for_new_workshops"], 1);
    assert_eq!(budget["available_for_new_plan"], 2);
    assert_eq!(
        parse_steps(&compiled.actions)?
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        1,
        "both furniture families must share the same new carpenter's shop"
    );
    let completed = execute(snapshot, &compiled)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 1);
    assert_eq!(stock(&completed, effects::STOCK_CHAIR_FIELD)?, 1);
    assert_eq!(stock(&completed, effects::STOCK_TABLE_FIELD)?, 1);
    let report = effects::construction_material_report(&completed)?
        .ok_or_else(|| invalid("completed material report missing"))?;
    assert_eq!(report.held_wood, 0);
    assert_eq!(report.consumed_wood, 1);
    assert_eq!(report.released_wood, 0);
    Ok(())
}

#[test]
fn existing_workshops_cost_nothing_even_when_a_new_site_is_offered() -> Result<()> {
    let request = with_setup(
        json!([{"item":"BED","minimum":2}]),
        json!([{"building":"workshop:Carpenters","location":[0,0,10]}]),
    )?;
    let snapshot = economy(2)?;
    let compiled = request.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["construction_materials"]["new_workshop_wood"],
        0
    );
    assert_eq!(
        compiled.analysis["materials"]["WOOD"]["available_for_new_plan"],
        2
    );
    assert!(
        parse_steps(&compiled.actions)?
            .iter()
            .all(|step| !matches!(step.action, Action::Build { .. }))
    );
    let completed = execute(snapshot, &compiled)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 2);
    assert_eq!(
        effects::construction_material_report(&completed)?
            .ok_or_else(|| invalid("reused material report missing"))?
            .consumed_wood,
        0
    );
    Ok(())
}

#[test]
fn consumption_aware_food_and_drink_setup_requires_both_workshop_inputs() -> Result<()> {
    let request = with_setup(
        json!([{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65}]),
        json!([
            {"building":"workshop:Still","location":[0,0,10]},
            {"building":"workshop:Kitchen","location":[1,0,10]},
        ]),
    )?;
    let mut snapshot = economy(2)?;
    snapshot.graph.entities.remove(&starter::STILL);
    snapshot.graph.entities.remove(&starter::KITCHEN);
    prior_order(&mut snapshot, "CONSTRUCT_TABLE", 1)?;
    assert!(request.compile(&snapshot).is_err());
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(3),
    )?;
    let compiled = request.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["construction_materials"]["new_workshop_wood"],
        2
    );
    assert_eq!(
        compiled.analysis["materials"]["WOOD"]["reserved_for_existing_orders"],
        1
    );
    assert_eq!(
        compiled.analysis["materials"]["WOOD"]["available_for_new_plan"],
        0
    );
    let completed = execute(snapshot, &compiled)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_TABLE_FIELD)?, 1);
    assert!(stock(&completed, effects::STOCK_DRINK_FIELD)? >= 45);
    assert!(stock(&completed, effects::STOCK_FOOD_FIELD)? >= 65);
    assert_eq!(
        effects::construction_material_report(&completed)?
            .ok_or_else(|| invalid("production material report missing"))?
            .consumed_wood,
        2
    );
    Ok(())
}

#[test]
fn missing_workshop_input_is_unknown_and_cannot_be_planned_as_free() -> Result<()> {
    let request = with_setup(
        json!([{"item":"DRINK","minimum":45}]),
        json!([{"building":"workshop:Still","location":[0,0,10]}]),
    )?;
    let mut snapshot = economy(1)?;
    snapshot.graph.entities.remove(&starter::STILL);
    snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("stock fixture missing"))?
        .fields
        .remove(effects::STOCK_WOOD_FIELD);
    snapshot.refresh_hash();
    assert!(request.compile(&snapshot).is_err());
    Ok(())
}
