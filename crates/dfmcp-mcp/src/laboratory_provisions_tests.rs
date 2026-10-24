//! Finite food ingredients through the real quota compiler and laboratory.
//! These tests certify reference semantics, not native Dwarf Fortress recipes.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId,
    OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, PredicateTruth};

pub(crate) fn source(plants: u64) -> Result<WorldSnapshot> {
    let mut snapshot =
        scenario_snapshot("starter_fortress_provisions", FortressId::new(995), false)?;
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(plants),
    )?;
    Ok(snapshot)
}

pub(crate) fn request(quotas: Json) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(&json!({"template":"production", "quotas":quotas}).to_string())
}

pub(crate) fn write(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    field: &str,
    value: Value,
) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("provisions fixture entity missing"))?
        .fields
        .insert(field.to_owned(), lab_fact(value));
    snapshot.refresh_hash();
    Ok(())
}

pub(crate) fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::Construct, RiskTier::Guarded),
            (Capability::ConfigureLabor, RiskTier::Reversible),
            (Capability::ConfigureProduction, RiskTier::Reversible),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            max_risk,
            scope: CapabilityScope {
                fortress_id: Some(snapshot.fortress_id),
                ..CapabilityScope::default()
            },
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
    }
}

pub(crate) fn seal(
    snapshot: &WorldSnapshot,
    compiled: &ProductionCompilation,
) -> Result<PreparedPlan> {
    let mut intent = Intent {
        id: IntentId::new(995),
        anchor: snapshot.anchor(),
        summary: "meet the original food and drink stock with finite ingredients".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}

pub(crate) fn commit(adapter: &mut MemoryAdapter, plan: &PreparedPlan) -> Result<Vec<ActionId>> {
    let prepared = adapter.prepare(plan, &context(adapter.snapshot()))?;
    Ok(adapter
        .commit(plan, &prepared, &context(adapter.snapshot()))?
        .actions
        .into_iter()
        .map(|action| action.action_id)
        .collect())
}

pub(crate) fn finish(
    adapter: &mut MemoryAdapter,
    plan: &PreparedPlan,
    actions: &[ActionId],
) -> Result<()> {
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|o| o.deadline_tick.0))
        .max()
        .unwrap_or(adapter.snapshot().tick.0);
    loop {
        let mut complete = true;
        for action in actions {
            let polled = adapter.poll_action(*action, &context(adapter.snapshot()))?;
            complete &= polled.state == CommitState::Verified;
            complete &= adapter.action_work_state(*action)?.is_quiescent();
        }
        if complete {
            assert_eq!(
                plan_goal_truth(adapter.snapshot(), plan)?,
                PredicateTruth::True
            );
            return Ok(());
        }
        if adapter.snapshot().tick.0 >= horizon {
            return Err(invalid(
                "provisions fixture did not finish within its sealed horizon",
            ));
        }
        adapter.advance_ticks(
            effects::DEFAULT_POLL_INTERVAL_TICKS.min(horizon - adapter.snapshot().tick.0),
        )?;
    }
}

fn execute(snapshot: WorldSnapshot, compiled: &ProductionCompilation) -> Result<WorldSnapshot> {
    let plan = seal(&snapshot, compiled)?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let actions = commit(&mut adapter, &plan)?;
    finish(&mut adapter, &plan, &actions)?;
    Ok(adapter.snapshot().clone())
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|ledger| production_fact(ledger, field, snapshot.tick))
    {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(invalid("provisions fixture stock missing")),
    }
}

fn prior_order(
    snapshot: &mut WorldSnapshot,
    job: &str,
    amount: u32,
    key: &str,
) -> Result<EntityId> {
    effects::apply_effect(
        snapshot,
        &Action::CreateWorkOrder {
            name: "earlier physical provisions work".to_owned(),
            job_token: job.to_owned(),
            amount,
            conditions: Vec::new(),
        },
        key,
    )?;
    snapshot.refresh_hash();
    Ok(effects::created_entity_id(key, 0))
}

#[test]
fn provisions_scenario_extends_exact_economy_bytes_and_keeps_old_recipe_program() -> Result<()> {
    for fortress in [FortressId::new(1), FortressId::new(995)] {
        let previous = scenario_snapshot("starter_fortress_economy", fortress, false)?;
        let provisions = scenario_snapshot("starter_fortress_provisions", fortress, false)?;
        assert!(provisions_json(&previous)?.is_none());
        assert!(briefing(&previous).get("provisions").is_none());
        let view =
            provisions_json(&provisions)?.ok_or_else(|| invalid("provisions view missing"))?;
        assert_eq!(view["available_plants"], starter::OPENING_PLANTS);
        assert_eq!(view["consumed_plants"], 0);
        assert_eq!(view["remaining_demand_plants"], 0);
        assert_eq!(
            view["source_anchor"]["state_hash"],
            provisions.state_hash.to_hex()
        );
        assert_eq!(view["epistemic_state"], "certified_derived");
        let mut normalized = provisions;
        normalized
            .graph
            .entities
            .get_mut(&effects::fortress_settings_entity_id(fortress))
            .ok_or_else(|| invalid("provisions settings missing"))?
            .fields
            .remove(effects::LABORATORY_PROVISIONS_MODEL_FIELD);
        normalized
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .ok_or_else(|| invalid("provisions ledger missing"))?
            .fields
            .remove(effects::STOCK_PLANT_FIELD);
        normalized.refresh_hash();
        assert_eq!(normalized.canonical_bytes(), previous.canonical_bytes());
        assert_eq!(normalized.state_hash, previous.state_hash);
        let historical = ProductionRequest::parse(
            r#"{"template":"production","quotas":[{"item":"FOOD","minimum":65}]}"#,
        )?;
        assert_eq!(
            historical.compile(&previous)?.actions,
            r#"[{"action":{"amount":1,"conditions":[{"item_token":"FOOD","kind":"item_count_below","threshold":65}],"job_token":"PREPARE_MEAL","kind":"create_work_order","name":"food for quota"},"depends_on":[]}]"#
        );
        assert_eq!(
            historical.compile(&normalized)?.actions,
            historical.compile(&previous)?.actions
        );
    }
    Ok(())
}

#[test]
fn food_and_drink_share_one_exact_input_budget_and_conserve_every_completed_batch() -> Result<()> {
    let request = request(json!([{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65}]))?;
    let short = source(1)?;
    let before = short.clone();
    let error = request
        .compile(&short)
        .err()
        .ok_or_else(|| invalid("one-short provisions admitted"))?;
    assert_eq!(error.code, ErrorCode::PreconditionsFailed);
    assert!(error.message.contains("PLANT"), "{error}");
    assert_eq!(short, before);
    let snapshot = source(2)?;
    let compiled = request.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["materials"]["PLANT"]["available_for_new_plan"],
        2
    );
    for step in parse_steps(&compiled.actions)? {
        let Action::CreateWorkOrder { conditions, .. } = step.action else {
            return Err(invalid("unexpected provisions setup"));
        };
        assert!(conditions.contains(&WorkOrderCondition::MaterialAvailable {
            material_token: "PLANT".to_owned(),
            minimum: 1,
        }));
    }
    let completed = execute(snapshot, &compiled)?;
    let report =
        effects::provisions_report(&completed)?.ok_or_else(|| invalid("report missing"))?;
    assert_eq!(report.available_plants, 0);
    assert_eq!(report.consumed_plants, 2);
    assert_eq!(report.remaining_demand_plants, 0);
    assert_eq!(report.available_plants + report.consumed_plants, 2);
    assert_eq!(stock(&completed, effects::STOCK_DRINK_FIELD)?, 45);
    assert_eq!(stock(&completed, effects::STOCK_FOOD_FIELD)?, 65);
    assert!(
        report
            .orders
            .iter()
            .all(|order| order.consumed_plants == 1 && order.status == effects::STATUS_COMPLETE)
    );
    Ok(())
}

#[test]
fn consumption_reserve_fixed_point_budgets_its_extra_ingredients() -> Result<()> {
    let mut adapter = MemoryAdapter::new(source(20)?);
    adapter.advance_ticks(1150)?;
    let mut snapshot = adapter.snapshot().clone();
    let request = request(json!([{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65}]))?;
    let first = request.compile(&snapshot)?;
    let requirements = first.analysis["requirements"]
        .as_array()
        .ok_or_else(|| invalid("requirements missing"))?;
    let plant = requirements
        .iter()
        .find(|row| row["item"] == "PLANT")
        .ok_or_else(|| invalid("plant requirement missing"))?;
    assert_eq!(plant["consumed_units"], 4);
    assert_eq!(plant["consumption_allowance"], 0);
    assert_eq!(plant["predicted_consumption_through_horizon"], 0);
    assert_eq!(plant["role"], "raw_recipe_input");
    let drink = requirements
        .iter()
        .find(|row| row["item"] == "DRINK")
        .ok_or_else(|| invalid("drink requirement missing"))?;
    assert_eq!(drink["minimum_stock"], 45);
    assert_eq!(drink["consumption_allowance"], 7);
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(3),
    )?;
    assert!(
        request.compile(&snapshot).is_err(),
        "reserve ingredients may not be discarded to admit a short plan"
    );
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(4),
    )?;
    let compiled = request.compile(&snapshot)?;
    let completed = execute(snapshot, &compiled)?;
    let report =
        effects::provisions_report(&completed)?.ok_or_else(|| invalid("report missing"))?;
    assert_eq!(report.consumed_plants, 4);
    assert_eq!(report.available_plants, 0);
    assert!(stock(&completed, effects::STOCK_DRINK_FIELD)? >= 45);
    assert!(stock(&completed, effects::STOCK_FOOD_FIELD)? >= 65);
    Ok(())
}

#[test]
fn pending_other_output_consumes_input_budget_after_its_handle_is_gone() -> Result<()> {
    for (job, item, minimum) in [("BREW_DRINK", "FOOD", 65), ("COOK_MEAL", "DRINK", 45)] {
        let mut snapshot = source(2)?;
        let old = prior_order(&mut snapshot, job, 1, "retired-provisions-order")?;
        // No adapter handle exists for this retained physical order.
        let snapshot = WorldSnapshot::from_canonical_bytes(&snapshot.canonical_bytes())?;
        let request = request(json!([{"item":item,"minimum":minimum}]))?;
        let compiled = request.compile(&snapshot)?;
        assert_eq!(
            compiled.analysis["materials"]["PLANT"]["reserved_for_existing_orders"],
            1
        );
        assert_eq!(
            compiled.analysis["materials"]["PLANT"]["available_for_new_plan"],
            1
        );
        assert_eq!(
            compiled.analysis["capacity"]["plant_units_reserved_for_existing_orders"],
            1
        );
        assert_eq!(
            compiled.analysis["capacity"]["future_output_counted_as_stock"],
            false
        );
        let mut short = snapshot.clone();
        write(
            &mut short,
            starter::STOCK_LEDGER,
            effects::STOCK_PLANT_FIELD,
            Value::U64(1),
        )?;
        assert!(request.compile(&short).is_err());
        let completed = execute(snapshot, &compiled)?;
        let report =
            effects::provisions_report(&completed)?.ok_or_else(|| invalid("report missing"))?;
        assert_eq!(report.available_plants, 0);
        assert_eq!(report.consumed_plants, 2);
        assert_eq!(report.remaining_demand_plants, 0);
        assert!(
            report
                .orders
                .iter()
                .any(|order| order.order == old && order.status == effects::STATUS_COMPLETE)
        );
    }
    Ok(())
}

#[test]
fn workshop_and_furniture_wood_remain_joint_while_food_uses_plants() -> Result<()> {
    let request = ProductionRequest::parse_current(
        &json!({
            "template":"production", "quotas":[
                {"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65},
                {"item":"BED","minimum":1}],
            "prerequisites":{"assign_labor":false,"workshops":[
                {"building":"workshop:Still","location":[0,0,10]},
                {"building":"workshop:Kitchen","location":[1,0,10]}]},
        })
        .to_string(),
    )?;
    // The sealed setup horizon includes two construction jobs, so the output
    // reserve requires seven plant batches even though the initial deficits
    // alone need only one brewing and one cooking batch.
    let mut snapshot = source(7)?;
    snapshot.graph.entities.remove(&starter::STILL);
    snapshot.graph.entities.remove(&starter::KITCHEN);
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(2),
    )?;
    assert!(request.compile(&snapshot).is_err());
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(3),
    )?;
    let compiled = request.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["materials"]["WOOD"]["reserved_for_new_workshops"],
        2
    );
    assert_eq!(
        compiled.analysis["materials"]["WOOD"]["available_for_new_plan"],
        1
    );
    assert_eq!(
        compiled.analysis["materials"]["PLANT"]["available_for_new_plan"],
        7
    );
    let completed = execute(snapshot, &compiled)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_PLANT_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 1);
    Ok(())
}

#[test]
fn unqualified_model_or_inventory_is_unknown_in_briefing_and_refuses_planning() -> Result<()> {
    let request = request(json!([{"item":"DRINK","minimum":45}]))?;
    for field in [
        effects::LABORATORY_PROVISIONS_MODEL_FIELD,
        effects::STOCK_PLANT_FIELD,
    ] {
        for variant in 0..3 {
            let mut snapshot = source(2)?;
            let id = if field == effects::STOCK_PLANT_FIELD {
                starter::STOCK_LEDGER
            } else {
                effects::fortress_settings_entity_id(snapshot.fortress_id)
            };
            let fact = snapshot
                .graph
                .entities
                .get_mut(&id)
                .and_then(|record| record.fields.get_mut(field))
                .ok_or_else(|| invalid("fixture fact missing"))?;
            match variant {
                0 => fact.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                1 => fact.source = FactSource::AgentAssertion("unverified provisions".to_owned()),
                _ => fact.observed_at = GameTick(2),
            }
            snapshot.refresh_hash();
            assert!(provisions_json(&snapshot).is_err());
            assert!(request.compile(&snapshot).is_err());
            let view = briefing(&snapshot);
            assert_eq!(view["provisions"]["available"], false);
            assert_eq!(view["provisions"]["epistemic_state"], "unknown");
            assert!(view["provisions"].get("available_plants").is_none());
            assert!(view["provisions"].get("orders").is_none());
        }
    }
    Ok(())
}

#[test]
fn bounded_current_report_keeps_every_original_order_individually_addressable() -> Result<()> {
    let mut snapshot = source(20)?;
    for index in 0..MAX_BRIEFING_ITEMS + 3 {
        prior_order(
            &mut snapshot,
            "BREW_DRINK",
            1,
            &format!("provisions-projection-{index}"),
        )?;
    }
    let report = effects::provisions_report(&snapshot)?.ok_or_else(|| invalid("report missing"))?;
    let last = report
        .orders
        .last()
        .ok_or_else(|| invalid("order missing"))?;
    let view = provisions_json(&snapshot)?.ok_or_else(|| invalid("view missing"))?;
    assert_eq!(
        view["orders"].as_array().map(Vec::len),
        Some(MAX_BRIEFING_ITEMS)
    );
    assert_eq!(view["orders_omitted"], 3);
    assert_eq!(view["remaining_demand_plants"], 19);
    assert_eq!(
        view["available_plants"], 20,
        "pending demand does not allocate actual ingredients"
    );
    let individual = provisions_action_json(&snapshot, last.order)?
        .ok_or_else(|| invalid("order projection missing"))?;
    assert_eq!(
        individual["order"]["order_entity_id"],
        last.order.to_string()
    );
    assert_eq!(individual["order"]["status"], effects::STATUS_ACTIVE);
    assert!(provisions_action_json(&snapshot, starter::STILL)?.is_none());
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(0),
    )?;
    let blocked = provisions_action_json(&snapshot, last.order)?
        .ok_or_else(|| invalid("blocked projection missing"))?;
    assert!(
        blocked["order"]["input_blocker"]
            .as_str()
            .is_some_and(|reason| reason.contains("PLANT"))
    );
    assert_eq!(
        effects::work_order_blocker(&snapshot, "BREW_DRINK"),
        None,
        "ingredient shortage must not be relabeled as missing labor"
    );
    Ok(())
}

#[test]
fn original_goal_proof_rejects_replaced_consumption_identity_even_with_sufficient_stock()
-> Result<()> {
    let source = source(1)?;
    let compiled = request(json!([{"item":"DRINK","minimum":45}]))?.compile(&source)?;
    let plan = seal(&source, &compiled)?;
    let mut completed = execute(source, &compiled)?;
    assert_eq!(plan_goal_truth(&completed, &plan)?, PredicateTruth::True);
    let order = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    completed
        .graph
        .entities
        .get_mut(&order)
        .ok_or_else(|| invalid("completed original order missing"))?
        .generation += 1;
    completed.refresh_hash();
    assert_eq!(
        dfmcp_world::PredicateEvidence::laboratory(&completed)?
            .evaluate(&plan.terminal_condition)?,
        PredicateTruth::True
    );
    assert!(plan_goal_truth(&completed, &plan).is_err());
    Ok(())
}

#[test]
fn low_stock_alerts_explain_depleted_or_insufficient_inputs_without_false_remedies() -> Result<()> {
    for (plants, pending) in [(0, 0), (1, 1), (5, 0)] {
        let mut snapshot = source(plants)?;
        for field in [effects::STOCK_DRINK_FIELD, effects::STOCK_FOOD_FIELD] {
            write(&mut snapshot, starter::STOCK_LEDGER, field, Value::U64(0))?;
        }
        if pending > 0 {
            prior_order(&mut snapshot, "COOK_MEAL", pending, "pending-alert-input")?;
        }
        if plants == 0 {
            // Ingredient exhaustion takes priority even when labor is also
            // missing; a labor change alone cannot make this remedy usable.
            for unit in snapshot
                .graph
                .entities
                .values_mut()
                .filter(|record| record.kind == EntityKind::Unit)
            {
                for labor in ["labor.BREW", "labor.COOK"] {
                    unit.fields
                        .insert(labor.to_owned(), lab_fact(Value::Bool(false)));
                }
            }
            snapshot.refresh_hash();
        }
        let before = snapshot.clone();
        let alerts = world_alerts(&snapshot);
        assert_eq!(snapshot, before);
        for name in ["drink_supply", "food_supply"] {
            let alert = alerts
                .iter()
                .find(|alert| alert["alert"] == name)
                .ok_or_else(|| invalid("low-stock alert missing"))?;
            assert!(alert["remedy"].is_null(), "{alert}");
            assert!(
                alert["finding"]
                    .as_str()
                    .is_some_and(|finding| finding.contains("PLANT"))
            );
            assert!(
                alert["production_blocked_by"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("PLANT"))
            );
            let readiness = &alert["ingredient_readiness"];
            assert_eq!(readiness["status"], "blocked");
            assert_eq!(readiness["epistemic_state"], "certified_derived");
            assert_eq!(readiness["available_plants"], plants);
            assert_eq!(readiness["remaining_demand_plants"], pending);
            assert_eq!(readiness["minimum_new_batches"], 6);
            assert_eq!(
                readiness["source_anchor"]["state_hash"],
                snapshot.state_hash.to_hex()
            );
        }
    }
    // The explicit generation adds a readiness guard; old worlds retain their
    // existing low-stock remedy and never acquire an implicit ingredient rule.
    let mut legacy = scenario_snapshot("starter_fortress_economy", FortressId::new(995), false)?;
    write(
        &mut legacy,
        starter::STOCK_LEDGER,
        effects::STOCK_DRINK_FIELD,
        Value::U64(0),
    )?;
    let alerts = world_alerts(&legacy);
    let drink = alerts
        .iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or_else(|| invalid("legacy alert missing"))?;
    assert_eq!(drink["remedy"]["requires"], "configure_production");
    assert!(drink.get("ingredient_readiness").is_none());
    Ok(())
}

#[test]
fn low_stock_alerts_require_qualified_plant_evidence_instead_of_trusting_retained_counts()
-> Result<()> {
    for variant in 0..3 {
        let mut snapshot = source(20)?;
        for field in [effects::STOCK_DRINK_FIELD, effects::STOCK_FOOD_FIELD] {
            write(&mut snapshot, starter::STOCK_LEDGER, field, Value::U64(0))?;
        }
        let fact = snapshot
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .and_then(|record| record.fields.get_mut(effects::STOCK_PLANT_FIELD))
            .ok_or_else(|| invalid("plant fixture missing"))?;
        match variant {
            0 => fact.presence = Some(FactPresence::Unknown("not observed".to_owned())),
            1 => fact.source = FactSource::AgentAssertion("unverified inventory".to_owned()),
            _ => fact.observed_at = GameTick(2),
        }
        snapshot.refresh_hash();
        let alerts = world_alerts(&snapshot);
        for name in ["drink_supply", "food_supply"] {
            let alert = alerts
                .iter()
                .find(|alert| alert["alert"] == name)
                .ok_or_else(|| invalid("unknown-input alert missing"))?;
            assert!(alert["remedy"].is_null());
            assert!(
                alert["finding"]
                    .as_str()
                    .is_some_and(|finding| finding.contains("PLANT"))
            );
            assert_eq!(alert["ingredient_readiness"]["epistemic_state"], "unknown");
            assert!(
                alert["ingredient_readiness"]
                    .get("available_plants")
                    .is_none()
            );
            assert!(
                alert["ingredient_readiness"]
                    .get("remaining_demand_plants")
                    .is_none()
            );
        }
    }
    Ok(())
}

#[test]
fn retained_hunger_does_not_invent_inputs_for_an_already_met_stock_target() -> Result<()> {
    let mut snapshot = source(6)?;
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_FOOD_FIELD,
        Value::U64(0),
    )?;
    let mut adapter = MemoryAdapter::new(snapshot);
    adapter.advance_ticks(effects::FOOD_INTERVAL_TICKS)?;
    let compiled = request(json!([{"item":"FOOD","minimum":28}]))?.compile(adapter.snapshot())?;
    let plan = seal(adapter.snapshot(), &compiled)?;
    let actions = commit(&mut adapter, &plan)?;
    finish(&mut adapter, &plan, &actions)?;
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_PLANT_FIELD)?, 0);
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_FOOD_FIELD)?, 30);
    // The next meal has not happened: hunger remains recorded despite the
    // refill. Its already-met alert target needs zero additional ingredients.
    let alerts = world_alerts(adapter.snapshot());
    let food = alerts
        .iter()
        .find(|alert| alert["alert"] == "food_supply")
        .ok_or_else(|| invalid("retained hunger alert missing"))?;
    assert_eq!(food["deprived_units"], 7);
    assert!(food.get("ingredient_readiness").is_none());
    assert!(food["production_blocked_by"].is_null());
    let drink = alerts
        .iter()
        .find(|alert| alert["alert"] == "drink_supply")
        .ok_or_else(|| invalid("remaining drink shortage missing"))?;
    assert_eq!(drink["ingredient_readiness"]["status"], "blocked");
    assert!(drink["remedy"].is_null());
    Ok(())
}
