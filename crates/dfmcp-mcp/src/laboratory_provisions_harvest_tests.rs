//! Finite-input recommendations through observation, real planning and progress.

use super::*;
use crate::lab_world::{ProductionRequest, parse_steps, scenario_snapshot, starter, world_alerts};
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

fn request(alert: &Json) -> TestResult<ProductionRequest> {
    let raw = alert["remedy"]["arguments"]["blueprint"]
        .as_str()
        .ok_or("retained production source missing")?;
    assert!(alert["remedy"]["arguments"]["actions"].is_null());
    Ok(ProductionRequest::parse_current(raw)?)
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
fn harvest_diagnosis_retains_pending_input_debt_without_proposing_competing_acquisition()
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
    assert!(alert["remedy"].is_null());
    assert_eq!(alert["ingredient_readiness"]["status"], "blocked");
    assert_eq!(alert["ingredient_readiness"]["available_plants"], 2);
    assert_eq!(alert["ingredient_readiness"]["remaining_demand_plants"], 3);
    assert_eq!(alert["ingredient_readiness"]["minimum_new_batches"], 6);
    let ready = &alert["ingredient_readiness"]["harvesting"];
    assert_eq!(ready["minimum_missing_plants"], 7);
    assert_eq!(ready["proposed_harvest_units"], 7);
    assert_eq!(ready["source_available_units"], 8);
    assert_eq!(ready["source_entity_id"], "7901");
    assert_eq!(ready["source_generation"], 5);
    assert_eq!(ready["worker_entity_id"], GATHERER.to_string());
    assert_eq!(
        ready["source_anchor"]["state_hash"],
        snapshot.state_hash.to_hex()
    );
    assert_eq!(ready["credit_on_completion_only"], true);
    assert_eq!(
        snapshot, before,
        "advice cannot mutate input, source or original work"
    );
    Ok(())
}

#[test]
fn harvest_diagnosis_is_bounded_by_available_source_and_registered_action_amount() -> TestResult {
    for (units, expected) in [(3, 3), (200, 100)] {
        let mut snapshot = source(0)?;
        source_units(&mut snapshot, PLANTS, units)?;
        let provisions = effects::provisions_report(&snapshot);
        let blocked =
            alert_input_blocker(&snapshot, &provisions, 150).ok_or("input blocker missing")?;
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
fn simultaneous_supply_shortages_share_one_full_output_goal_and_joint_input_budget() -> TestResult {
    for (food, plants) in [(0, 0), (5, 7)] {
        // Seven plants fund either immediate shortage separately, but not the
        // complete joint goal. The shared screen must still propose acquisition.
        let mut snapshot = source(plants)?;
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
        assert_eq!(drink["remedy"]["requires"], "configure_production");
        assert_eq!(drink["severity"], "critical");
        assert_eq!(
            food_alert["severity"],
            if food == 0 { "critical" } else { "high" }
        );
        assert!(food_alert["remedy"].is_null());
        assert_eq!(food_alert["remedy_shared_with"], "drink_supply");
        assert_eq!(drink["remedy"]["separate_stage_review_required"], true);
        let request = request(drink)?;
        let original: Json = serde_json::from_str(&request.canonical_json())?;
        assert_eq!(
            original["quotas"],
            json!([
                {"item":"DRINK","minimum":28}, {"item":"FOOD","minimum":28},
            ])
        );
        assert_eq!(original["acquire_resources"]["assign_labor"], false);
        let compiled = request.compile(&snapshot)?;
        assert_eq!(
            compiled.analysis["resource_workflow"]["stage"],
            "acquire_resources"
        );
        assert!(parse_steps(&compiled.actions)?.iter().all(|step| matches!(
            step.action,
            Action::HarvestResource {
                resource: HarvestResource::Plant,
                ..
            }
        )));
        for alert in [drink, food_alert] {
            assert_eq!(alert["ingredient_readiness"]["status"], "blocked");
            assert_eq!(
                alert["ingredient_readiness"]["minimum_new_batches"],
                if food == 0 { 12 } else { 11 }
            );
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
fn disabled_harvest_and_production_staffing_remain_permissions_on_the_original_output_source()
-> TestResult {
    let mut snapshot = source(0)?;
    write(
        &mut snapshot,
        GATHERER,
        "labor.PLANT_GATHERING",
        Value::Bool(false),
    )?;
    write(
        &mut snapshot,
        EntityId::new(starter::FIRST_DWARF + 2),
        "labor.BREW",
        Value::Bool(false),
    )?;
    let before = snapshot.clone();
    let alert = drink(&snapshot)?;
    let request = request(&alert)?;
    let original: Json = serde_json::from_str(&request.canonical_json())?;
    assert_eq!(original["quotas"], json!([{"item":"DRINK","minimum":28}]));
    assert_eq!(original["acquire_resources"]["assign_labor"], true);
    assert_eq!(original["prerequisites"]["assign_labor"], true);
    let compiled = request.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["resource_workflow"]["stage"],
        "acquire_resources"
    );
    let steps = parse_steps(&compiled.actions)?;
    assert!(steps.iter().any(|step| matches!(&step.action,
        Action::SetLabor { labor, enabled: true, .. } if labor == "PLANT_GATHERING")));
    assert!(steps.iter().any(|step| matches!(
        step.action,
        Action::HarvestResource {
            resource: HarvestResource::Plant,
            ..
        }
    )));
    assert!(!steps.iter().any(|step| matches!(&step.action,
        Action::SetLabor { labor, .. } if labor == "BREW")));
    assert!(
        steps
            .iter()
            .all(|step| !matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    assert_eq!(
        snapshot, before,
        "advice and compilation do not enable labor or credit stock"
    );
    Ok(())
}

#[test]
fn a_bounded_partial_input_tranche_cannot_replace_a_complete_supply_goal() -> TestResult {
    let mut snapshot = source(0)?;
    source_units(&mut snapshot, PLANTS, 3)?;
    let alert = drink(&snapshot)?;
    assert_eq!(
        alert["ingredient_readiness"]["harvesting"]["proposed_harvest_units"],
        3
    );
    assert_eq!(alert["ingredient_readiness"]["minimum_new_batches"], 6);
    assert!(
        alert["remedy"].is_null(),
        "partial acquisition cannot be advertised as output recovery"
    );
    Ok(())
}

#[test]
fn harvest_advice_refuses_unknown_depleted_or_unqualified_sources() -> TestResult {
    for case in [0, 1, 2, 4, 5, 6, 7, 8, 9] {
        let mut snapshot = source(0)?;
        if [4, 5, 9].contains(&case) {
            // The policy may legitimately enable any other known disabled
            // gatherer. Remove those alternatives so this is a decisive test
            // of unavailable evidence on the only remaining candidate.
            for unit in snapshot
                .graph
                .entities
                .values_mut()
                .filter(|entity| entity.kind == EntityKind::Unit && entity.id != GATHERER)
            {
                unit.fields.remove("labor.PLANT_GATHERING");
            }
        }
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
