//! Real parser, scenario, planner and reference economy resource workflows.
use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, IntentId, OperationContext, RiskTier, StepId,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner, derive_step_idempotency_key};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, PredicateTruth};

fn source() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress_resources", FortressId::new(9981), false)
}
fn context(snapshot: &WorldSnapshot) -> OperationContext {
    let mut context = laboratory_provisions_tests::context(snapshot);
    context.grants.push(CapabilityGrant {
        capability: Capability::Designate,
        max_risk: RiskTier::Guarded,
        scope: CapabilityScope {
            fortress_id: Some(snapshot.fortress_id),
            ..CapabilityScope::default()
        },
        expires_at_tick: None,
        remaining_uses: None,
    });
    context
}
fn raw(resource: &str, amount: u32) -> String {
    json!([{"action":{
        "kind":"harvest_resource", "source":if resource == "WOOD" {"8001"} else {"8002"},
        "source_generation":1, "resource":resource, "amount":amount,
    }}])
    .to_string()
}
fn seal(snapshot: &WorldSnapshot, raw: &str) -> Result<PreparedPlan> {
    let id = IntentId::new(9981);
    let requested_actions = parse_steps(raw)?;
    let mut terminal = Vec::new();
    for (index, requested) in requested_actions.iter().enumerate() {
        let key = derive_step_idempotency_key(
            id,
            snapshot.anchor(),
            StepId::new(index as u32),
            &requested.action,
        );
        terminal.extend(effects::default_postconditions(
            &requested.action,
            &key,
            snapshot.fortress_id,
        ));
    }
    let intent = Intent {
        id,
        anchor: snapshot.anchor(),
        summary: "acquire finite original resources".to_owned(),
        terminal_condition: Predicate::All(terminal).normalized(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions,
    };
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}
fn execute(adapter: &mut MemoryAdapter, plan: &PreparedPlan) -> Result<()> {
    let prepared = adapter.prepare(plan, &context(adapter.snapshot()))?;
    let committed = adapter.commit(plan, &prepared, &context(adapter.snapshot()))?;
    let actions = committed
        .actions
        .iter()
        .map(|action| action.action_id)
        .collect::<Vec<_>>();
    laboratory_provisions_tests::finish(adapter, plan, &actions)
}
fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|ledger| production_fact(ledger, field, snapshot.tick))
    {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(invalid("harvest test stock is unavailable")),
    }
}

#[test]
fn resource_scenario_is_an_additive_generation_with_explicit_finite_sources() -> Result<()> {
    let previous = scenario_snapshot("starter_fortress_provisions", FortressId::new(9981), false)?;
    let snapshot = source()?;
    let view = harvest_json(&snapshot)?.ok_or_else(|| invalid("harvest view missing"))?;
    assert_eq!(view["sources_total"], 2);
    assert_eq!(view["sources"][0]["resource"], "WOOD");
    assert_eq!(view["sources"][0]["remaining_units"], 32);
    assert_eq!(view["sources"][1]["remaining_units"], 40);
    assert_eq!(
        view["source_anchor"]["state_hash"],
        snapshot.state_hash.to_hex()
    );
    assert_eq!(view["epistemic_state"], "certified_derived");
    assert_eq!(briefing(&snapshot)["harvesting"], view);
    assert!(harvest_json(&previous)?.is_none());
    let mut normalized = snapshot;
    normalized
        .graph
        .entities
        .remove(&laboratory_harvest::WOOD_SOURCE);
    normalized
        .graph
        .entities
        .remove(&laboratory_harvest::PLANT_SOURCE);
    normalized
        .graph
        .entities
        .get_mut(&effects::fortress_settings_entity_id(
            normalized.fortress_id,
        ))
        .ok_or_else(|| invalid("settings missing"))?
        .fields
        .remove(effects::LABORATORY_HARVEST_MODEL_FIELD);
    for unit in normalized
        .graph
        .entities
        .values_mut()
        .filter(|entity| entity.kind == EntityKind::Unit)
    {
        for resource in [HarvestResource::Wood, HarvestResource::Plant] {
            unit.fields.remove(&format!(
                "{}{}",
                effects::LABOR_FIELD_PREFIX,
                resource.labor()
            ));
        }
    }
    normalized.refresh_hash();
    assert_eq!(normalized.canonical_bytes(), previous.canonical_bytes());
    Ok(())
}

#[test]
fn harvest_request_is_closed_and_retains_the_exact_selected_source() -> Result<()> {
    let valid = raw("WOOD", 3);
    assert_eq!(
        parse_steps(&valid)?[0].action,
        Action::HarvestResource {
            source: EntityId::new(8001),
            source_generation: 1,
            resource: HarvestResource::Wood,
            amount: 3,
        }
    );
    for (field, value) in [
        ("amount", json!(0)),
        ("amount", json!(101)),
        ("amount", json!(1.5)),
        ("source_generation", json!(0)),
        ("source_generation", json!(u64::from(u32::MAX) + 1)),
        ("source", json!("08001")),
        ("source", json!(8001)),
        ("resource", json!("wood")),
        ("resource", json!("FOOD")),
        ("command", json!("unregistered")),
    ] {
        let mut request: Json =
            serde_json::from_str(&valid).map_err(|error| invalid(error.to_string()))?;
        request[0]["action"][field] = value;
        assert!(parse_steps(&request.to_string()).is_err(), "{request}");
    }
    let plan = seal(&source()?, &valid)?;
    let steps = plan_steps_json(&plan);
    let step = &steps[0];
    assert_eq!(step["kind"], "harvest_resource");
    assert_eq!(step["capability"], "designate");
    assert_eq!(step["risk"], "guarded");
    assert_eq!(step["source_entity_id"], "8001");
    assert_eq!(step["source_generation"], 1);
    assert_eq!(step["amount"], 3);
    assert_eq!(step["compensable"], false);
    assert!(step["creates_entity_id"].is_string());
    assert!(plan.requires_checkpoint);
    Ok(())
}

#[test]
fn harvested_inputs_enable_real_food_and_furniture_production_after_stock_is_observed() -> Result<()>
{
    let mut snapshot = source()?;
    for field in [effects::STOCK_WOOD_FIELD, effects::STOCK_PLANT_FIELD] {
        laboratory_provisions_tests::write(
            &mut snapshot,
            starter::STOCK_LEDGER,
            field,
            Value::U64(0),
        )?;
    }
    let production = ProductionRequest::parse_current(
        r#"{"template":"production","quotas":[{"item":"FOOD","minimum":65},{"item":"BED","minimum":1}]}"#,
    )?;
    assert!(production.compile(&snapshot).is_err());
    let harvesting = json!([
        {"action":{"kind":"harvest_resource","source":"8001","source_generation":1,"resource":"WOOD","amount":2}},
        {"action":{"kind":"harvest_resource","source":"8002","source_generation":1,"resource":"PLANT","amount":2}},
    ]).to_string();
    let plan = seal(&snapshot, &harvesting)?;
    assert!(
        production.compile(&snapshot).is_err(),
        "planning cannot credit future supply"
    );
    let mut adapter = MemoryAdapter::new(snapshot);
    execute(&mut adapter, &plan)?;
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_WOOD_FIELD)?, 2);
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_PLANT_FIELD)?, 2);
    let compiled = production.compile(adapter.snapshot())?;
    let production_plan = laboratory_provisions_tests::seal(adapter.snapshot(), &compiled)?;
    execute(&mut adapter, &production_plan)?;
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_BED_FIELD)?, 1);
    assert!(stock(adapter.snapshot(), effects::STOCK_FOOD_FIELD)? >= 65);
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_WOOD_FIELD)?, 1);
    let report = effects::harvest_report(adapter.snapshot())?
        .ok_or_else(|| invalid("harvest report missing"))?;
    assert_eq!(report.sources[0].remaining, 30);
    assert_eq!(report.sources[1].remaining, 38);
    assert!(
        report
            .tasks
            .iter()
            .all(|task| task.harvested == 2 && task.amount_remaining == 0)
    );
    let ingredients = effects::provisions_report(adapter.snapshot())?
        .ok_or_else(|| invalid("provisions report missing"))?;
    assert_eq!(
        ingredients.available_plants + ingredients.consumed_plants,
        2
    );
    Ok(())
}

#[test]
fn unknown_or_substituted_sources_never_become_a_positive_supply_projection() -> Result<()> {
    let original = source()?;
    for mutation in 0..4 {
        let mut snapshot = original.clone();
        let source = snapshot
            .graph
            .entities
            .get_mut(&laboratory_harvest::WOOD_SOURCE)
            .ok_or_else(|| invalid("wood source missing"))?;
        match mutation {
            0 => source.generation += 1,
            1 => {
                source.fields.remove(effects::HARVEST_REMAINING_FIELD);
            }
            2 => {
                source
                    .fields
                    .get_mut(effects::HARVEST_REMAINING_FIELD)
                    .ok_or_else(|| invalid("remaining count missing"))?
                    .presence = Some(FactPresence::Unknown("not observed".to_owned()));
            }
            _ => {
                source
                    .fields
                    .get_mut(effects::HARVEST_REMAINING_FIELD)
                    .ok_or_else(|| invalid("remaining count missing"))?
                    .source = FactSource::AgentAssertion("claimed supply".to_owned());
            }
        }
        snapshot.refresh_hash();
        assert!(seal(&snapshot, &raw("WOOD", 1)).is_err());
        if mutation != 0 {
            assert_eq!(
                briefing(&snapshot)["harvesting"]["epistemic_state"],
                "unknown"
            );
        }
    }
    assert!(seal(&original, &raw("WOOD", 33)).is_err());
    Ok(())
}

#[test]
fn original_harvest_goal_keeps_exact_source_and_ledger_generations() -> Result<()> {
    let snapshot = source()?;
    let plan = seal(&snapshot, &raw("WOOD", 1))?;
    let mut adapter = MemoryAdapter::new(snapshot);
    execute(&mut adapter, &plan)?;
    assert_eq!(
        plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::True
    );
    for id in [laboratory_harvest::WOOD_SOURCE, starter::STOCK_LEDGER] {
        let mut changed = adapter.snapshot().clone();
        changed
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(|| invalid("original harvest entity missing"))?
            .generation += 1;
        changed.refresh_hash();
        assert!(!matches!(
            plan_goal_truth(&changed, &plan),
            Ok(PredicateTruth::True)
        ));
    }
    Ok(())
}
