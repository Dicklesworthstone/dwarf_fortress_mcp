//! Finite-input recommendations through observation, real planning and progress.

use super::*;
use crate::lab_world::{parse_steps, scenario_snapshot, starter, world_alerts};
use dfmcp_core::{Digest32, FortressId};
use dfmcp_intent::Action;
use dfmcp_world::{Fact, FactPresence, FactSource};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const PLANTS: EntityId = EntityId::new(8_002);
const GATHERER: EntityId = EntityId::new(starter::FIRST_DWARF + 4);

fn write(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> TestResult {
    let tick = snapshot.tick;
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or("fixture entity missing")?
        .fields
        .insert(
            field.to_owned(),
            Fact::known(
                value,
                tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    snapshot.refresh_hash();
    Ok(())
}

fn source(plants: u64) -> TestResult<WorldSnapshot> {
    let mut snapshot =
        scenario_snapshot("starter_fortress_resources", FortressId::new(98_780), false)?;
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(plants),
    )?;
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        Value::U64(0),
    )?;
    Ok(snapshot)
}

fn drink(snapshot: &WorldSnapshot) -> TestResult<Json> {
    world_alerts(snapshot)
        .into_iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or_else(|| "drink alert missing".into())
}

fn action(alert: &Json) -> TestResult<Action> {
    let raw = alert["remedy"]["arguments"]["actions"]
        .as_str()
        .ok_or("harvest source missing")?;
    let actions = parse_steps(raw)?;
    assert_eq!(actions.len(), 1);
    Ok(actions[0].action.clone())
}

fn source_units(snapshot: &mut WorldSnapshot, id: EntityId, units: u64) -> TestResult {
    write(
        snapshot,
        id,
        effects::HARVEST_INITIAL_FIELD,
        Value::U64(units),
    )?;
    write(
        snapshot,
        id,
        effects::HARVEST_REMAINING_FIELD,
        Value::U64(units),
    )
}

#[test]
fn harvest_advice_accounts_for_pending_input_debt_and_uses_canonical_source_generation()
-> TestResult {
    let mut snapshot = source(2)?;
    effects::apply_effect(
        &mut snapshot,
        &Action::CreateWorkOrder {
            name: "existing input debt".to_owned(),
            job_token: "BREW_DRINK".to_owned(),
            amount: 3,
            conditions: Vec::new(),
        },
        "existing-harvest-advice-input-debt",
    )?;
    source_units(&mut snapshot, PLANTS, 3)?;
    let original = snapshot.graph.entities[&PLANTS].clone();
    for (id, generation) in [(EntityId::new(7_901), 5), (EntityId::new(7_902), 1)] {
        let mut source = original.clone();
        source.id = id;
        source.generation = generation;
        snapshot.graph.entities.insert(id, source);
        source_units(&mut snapshot, id, 8)?;
    }
    snapshot.refresh_hash();
    let before = snapshot.clone();
    let alert = drink(&snapshot)?;
    assert_eq!(
        action(&alert)?,
        Action::HarvestResource {
            source: EntityId::new(7_901),
            source_generation: 5,
            resource: HarvestResource::Plant,
            amount: 7,
        }
    );
    assert_eq!(alert["ingredient_readiness"]["status"], "blocked");
    assert_eq!(alert["ingredient_readiness"]["available_plants"], 2);
    assert_eq!(alert["ingredient_readiness"]["remaining_demand_plants"], 3);
    assert_eq!(alert["ingredient_readiness"]["minimum_new_batches"], 6);
    let ready = &alert["ingredient_readiness"]["harvesting"];
    assert_eq!(ready["minimum_missing_plants"], 7);
    assert_eq!(ready["proposed_harvest_units"], 7);
    assert_eq!(ready["source_available_units"], 8);
    assert_eq!(ready["worker_entity_id"], GATHERER.to_string());
    assert_eq!(
        ready["source_anchor"]["state_hash"],
        snapshot.state_hash.to_hex()
    );
    assert_eq!(ready["credit_on_completion_only"], true);
    assert!(
        alert["remedy"]["then"]
            .as_str()
            .is_some_and(|next| next.contains("replan production"))
    );
    assert_eq!(
        snapshot, before,
        "advice cannot mutate input, source or original work"
    );
    Ok(())
}

#[test]
fn harvest_advice_is_bounded_by_available_source_and_registered_action_amount() -> TestResult {
    for (units, expected) in [(3, 3), (200, 100)] {
        let mut snapshot = source(0)?;
        source_units(&mut snapshot, PLANTS, units)?;
        let provisions = effects::provisions_report(&snapshot);
        let blocked =
            alert_input_blocker(&snapshot, &provisions, 150).ok_or("input blocker missing")?;
        let parsed = parse_steps(
            blocked.remedy["arguments"]["actions"]
                .as_str()
                .ok_or("remedy missing")?,
        )?;
        assert!(
            matches!(parsed[0].action, Action::HarvestResource { amount, .. } if amount == expected)
        );
        assert_eq!(
            blocked.readiness["harvesting"]["minimum_missing_plants"],
            150
        );
        assert_eq!(
            blocked.readiness["harvesting"]["proposed_harvest_units"],
            expected
        );
        assert_eq!(blocked.readiness["status"], "blocked");
    }
    Ok(())
}

#[test]
fn harvest_advice_never_credits_unfinished_work_or_duplicates_acquisition() -> TestResult {
    let mut snapshot = source(0)?;
    effects::apply_effect(
        &mut snapshot,
        &Action::HarvestResource {
            source: PLANTS,
            source_generation: 1,
            resource: HarvestResource::Plant,
            amount: 2,
        },
        "original-pending-harvest-advice",
    )?;
    snapshot.refresh_hash();
    let before = snapshot.clone();
    let alert = drink(&snapshot)?;
    assert!(alert["remedy"].is_null());
    assert_eq!(alert["ingredient_readiness"]["available_plants"], 0);
    assert_eq!(alert["ingredient_readiness"]["status"], "blocked");
    assert_eq!(
        alert["ingredient_readiness"]["harvesting"]["status"],
        "awaiting_harvest"
    );
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn simultaneous_supply_shortages_share_only_identical_harvest_advice() -> TestResult {
    for food in [0, 5] {
        let mut snapshot = source(0)?;
        write(
            &mut snapshot,
            starter::STOCK_LEDGER,
            effects::STOCK_FOOD_FIELD,
            Value::U64(food),
        )?;
        let before = snapshot.clone();
        let alerts = world_alerts(&snapshot);
        let drink = alerts
            .iter()
            .find(|alert| alert["alert"] == "drink_supply")
            .ok_or("drink alert missing")?;
        let food_alert = alerts
            .iter()
            .find(|alert| alert["alert"] == "food_supply")
            .ok_or("food alert missing")?;
        assert_eq!(drink["remedy"]["requires"], "designate");
        assert_eq!(drink["severity"], "critical");
        assert_eq!(
            food_alert["severity"],
            if food == 0 { "critical" } else { "high" }
        );
        if food == 0 {
            assert!(food_alert["remedy"].is_null());
            assert_eq!(
                food_alert["ingredient_readiness"]["harvesting"]["remedy_shared_with"],
                "drink_supply"
            );
        } else {
            assert_eq!(food_alert["remedy"]["requires"], "designate");
            assert_ne!(
                food_alert["remedy"]["arguments"],
                drink["remedy"]["arguments"]
            );
        }
        for alert in [drink, food_alert] {
            assert_eq!(alert["ingredient_readiness"]["status"], "blocked");
            assert_eq!(
                alert["ingredient_readiness"]["harvesting"]["status"],
                "ready"
            );
            assert_eq!(
                alert["ingredient_readiness"]["harvesting"]["source_entity_id"],
                PLANTS.to_string()
            );
            assert_eq!(
                alert["source_anchor"]["state_hash"],
                snapshot.state_hash.to_hex()
            );
        }
        assert_eq!(snapshot, before);
    }
    Ok(())
}

#[test]
fn harvest_advice_refuses_unknown_depleted_or_unstaffed_sources() -> TestResult {
    for case in 0..10 {
        let mut snapshot = source(0)?;
        match case {
            0 => source_units(&mut snapshot, PLANTS, 0)?,
            1 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&PLANTS)
                    .ok_or("plant missing")?
                    .fields
                    .get_mut(effects::HARVEST_REMAINING_FIELD)
                    .ok_or("source count missing")?
                    .presence = Some(FactPresence::Unknown(
                    "source count not observed".to_owned(),
                ));
            }
            2 => {
                let settings = effects::fortress_settings_entity_id(snapshot.fortress_id);
                snapshot
                    .graph
                    .entities
                    .get_mut(&settings)
                    .ok_or("settings missing")?
                    .fields
                    .get_mut(effects::LABORATORY_HARVEST_MODEL_FIELD)
                    .ok_or("model missing")?
                    .source = FactSource::AgentAssertion("claimed model".to_owned());
            }
            3 => write(
                &mut snapshot,
                GATHERER,
                "labor.PLANT_GATHERING",
                Value::Bool(false),
            )?,
            4 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&GATHERER)
                    .ok_or("gatherer missing")?
                    .fields
                    .get_mut("labor.PLANT_GATHERING")
                    .ok_or("labor missing")?
                    .source = FactSource::Replay;
            }
            5 => write(&mut snapshot, GATHERER, "alive", Value::Bool(false))?,
            6 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&PLANTS)
                    .ok_or("source missing")?
                    .generation = 0
            }
            7 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&PLANTS)
                    .ok_or("source missing")?
                    .kind = EntityKind::Other("plant".to_owned());
            }
            8 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&starter::STOCK_LEDGER)
                    .ok_or("ledger missing")?
                    .fields
                    .get_mut(effects::STOCK_PLANT_FIELD)
                    .ok_or("stock missing")?
                    .source = FactSource::Replay;
            }
            _ => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&GATHERER)
                    .ok_or("gatherer missing")?
                    .fields
                    .remove("labor.PLANT_GATHERING");
            }
        }
        snapshot.refresh_hash();
        let alert = drink(&snapshot)?;
        assert!(alert["remedy"].is_null(), "case {case}: {alert}");
        assert_eq!(
            alert["ingredient_readiness"]["status"],
            if case == 8 { "unknown" } else { "blocked" }
        );
        if [1, 2, 6, 7].contains(&case) {
            assert_eq!(
                alert["ingredient_readiness"]["harvesting"]["status"],
                "unknown"
            );
        }
    }
    for scenario in ["starter_fortress", "starter_fortress_provisions"] {
        let mut snapshot = scenario_snapshot(scenario, FortressId::new(98_781), false)?;
        write(
            &mut snapshot,
            starter::STOCK_LEDGER,
            effects::STOCK_DRINK_FIELD,
            Value::U64(0),
        )?;
        if scenario == "starter_fortress_provisions" {
            write(
                &mut snapshot,
                starter::STOCK_LEDGER,
                effects::STOCK_PLANT_FIELD,
                Value::U64(0),
            )?;
        }
        let alert = drink(&snapshot)?;
        assert!(alert["ingredient_readiness"].get("harvesting").is_none());
        if scenario == "starter_fortress" {
            assert_eq!(alert["remedy"]["requires"], "configure_production");
        } else {
            assert!(alert["remedy"].is_null());
        }
    }
    Ok(())
}
