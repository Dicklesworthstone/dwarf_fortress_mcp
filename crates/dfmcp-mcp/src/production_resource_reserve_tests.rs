//! Absolute reserves compose finite acquisition with observed production.
//! Predicted reserves retain the original output goal and bounded stage clocks.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId, OperationContext, RiskTier,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::FactSource;

fn source(plants: u64) -> Result<WorldSnapshot> {
    let mut snapshot =
        scenario_snapshot("starter_fortress_resources", FortressId::new(9988), false)?;
    laboratory_provisions_tests::write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(plants),
    )?;
    Ok(snapshot)
}

fn request(quotas: Json, until: u64, assign_labor: bool) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": quotas,
            "reserve_until_tick": until,
            "acquire_resources": {"assign_labor": assign_labor},
        })
        .to_string(),
    )
}

fn provisions(until: u64) -> Result<ProductionRequest> {
    request(
        json!([{"item": "DRINK", "minimum": 40}, {"item": "FOOD", "minimum": 60}]),
        until,
        false,
    )
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

fn bind(snapshot: &WorldSnapshot, compiled: &ProductionCompilation, id: u128) -> Result<Intent> {
    let mut intent = Intent {
        id: IntentId::new(id),
        anchor: snapshot.anchor(),
        summary: "fund the original explicit reserve through reviewed stages".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    Ok(intent)
}

fn seal(
    snapshot: &WorldSnapshot,
    compiled: &ProductionCompilation,
    id: u128,
) -> Result<PreparedPlan> {
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        &bind(snapshot, compiled, id)?,
        &context(snapshot),
    )
}

fn maximum_deadline(plan: &PreparedPlan) -> Result<u64> {
    plan.steps
        .iter()
        .filter_map(|step| {
            step.obligation
                .as_ref()
                .map(|obligation| obligation.deadline_tick.0)
        })
        .max()
        .ok_or_else(|| invalid("resource reserve fixture has no bounded obligation"))
}

fn execute(snapshot: WorldSnapshot, plan: &PreparedPlan) -> Result<MemoryAdapter> {
    let horizon = maximum_deadline(plan)?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(plan, &prepared, &context(adapter.snapshot()))?;
    let mut states = receipt
        .actions
        .iter()
        .map(|action| action.state)
        .collect::<Vec<_>>();
    while adapter.snapshot().tick.0 < horizon && states.iter().any(|state| !state.is_terminal()) {
        adapter.advance_ticks(
            effects::DEFAULT_POLL_INTERVAL_TICKS.min(horizon - adapter.snapshot().tick.0),
        )?;
        for (action, state) in receipt.actions.iter().zip(&mut states) {
            if !state.is_terminal() {
                *state = adapter
                    .poll_action(action.action_id, &context(adapter.snapshot()))?
                    .state;
            }
        }
    }
    assert!(
        states.iter().all(|state| *state == CommitState::Verified),
        "{states:?}"
    );
    for action in &receipt.actions {
        assert!(adapter.action_work_state(action.action_id)?.is_quiescent());
    }
    Ok(adapter)
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|ledger| production_fact(ledger, field, snapshot.tick))
    {
        Some(Value::U64(quantity)) => Ok(*quantity),
        _ => Err(invalid("resource reserve fixture stock unavailable")),
    }
}

fn requirement<'a>(compiled: &'a ProductionCompilation, item: &str) -> Result<&'a Json> {
    compiled.analysis["requirements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["item"] == item))
        .ok_or_else(|| invalid("resource reserve fixture requirement missing"))
}

fn resource_requirement<'a>(
    compiled: &'a ProductionCompilation,
    resource: &str,
) -> Result<&'a Json> {
    compiled.analysis["resource_workflow"]["requirements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["resource"] == resource))
        .ok_or_else(|| invalid("resource reserve fixture material requirement missing"))
}

#[test]
fn satisfied_output_goals_acquire_then_produce_the_explicit_future_reserve() -> Result<()> {
    let snapshot = source(0)?;
    let original = provisions(2401)?;
    let canonical = original.canonical_json();
    let before = snapshot.clone();
    assert_eq!(
        original.semantic_goal_truth(&snapshot)?,
        PredicateTruth::True
    );
    assert!(reserve_work_required(&original, &snapshot)?);
    let acquisition = original.compile(&snapshot)?;
    assert_eq!(snapshot, before);
    assert_eq!(
        requirement(&acquisition, "DRINK")?["planning_stock_target"],
        54
    );
    assert_eq!(
        requirement(&acquisition, "FOOD")?["planning_stock_target"],
        67
    );
    assert_eq!(requirement(&acquisition, "PLANT")?["consumed_units"], 5);
    assert_eq!(
        resource_requirement(&acquisition, "PLANT")?["observed_stock"],
        0
    );
    assert_eq!(resource_requirement(&acquisition, "PLANT")?["deficit"], 5);
    let workflow = &acquisition.analysis["resource_workflow"];
    assert_eq!(workflow["schema"], "dfmcp.production-resource-workflow/2");
    assert_eq!(workflow["planner"], "resource_reserve_v1");
    assert_eq!(workflow["stage"], "acquire_resources");
    assert_eq!(workflow["original_request"].to_string(), canonical);
    assert!(
        parse_steps(&acquisition.actions)?
            .iter()
            .all(|step| matches!(
                step.action,
                Action::HarvestResource {
                    resource: HarvestResource::Plant,
                    amount: 5,
                    ..
                }
            ))
    );
    let bound = bind(&snapshot, &acquisition, 99881)?;
    assert!(
        bound
            .constraints
            .contains(&Constraint::LaboratoryResourceProductionReserveUntil(
                GameTick(2401)
            ))
    );
    assert!(
        !bound.constraints.iter().any(|constraint| matches!(
            constraint,
            Constraint::LaboratoryProductionReserveUntil(_)
        ))
    );
    let plan = seal(&snapshot, &acquisition, 99881)?;
    assert!(maximum_deadline(&plan)? < 2401);
    assert_eq!(
        maximum_deadline(&plan)?,
        snapshot.tick.0
            + workflow["harvest_horizon_ticks"]
                .as_u64()
                .ok_or_else(|| invalid("stage horizon missing"))?
    );
    let harvested = execute(snapshot, &plan)?;
    assert_eq!(stock(harvested.snapshot(), effects::STOCK_PLANT_FIELD)?, 5);
    assert_eq!(stock(harvested.snapshot(), effects::STOCK_DRINK_FIELD)?, 40);
    assert_eq!(
        original.semantic_goal_truth(harvested.snapshot())?,
        PredicateTruth::True
    );
    assert!(reserve_work_required(&original, harvested.snapshot())?);

    let production = original.compile(harvested.snapshot())?;
    assert_eq!(
        production.analysis["resource_workflow"]["stage"],
        "production"
    );
    assert_eq!(production.terminal, acquisition.terminal);
    assert!(
        parse_steps(&production.actions)?
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let plan = seal(harvested.snapshot(), &production, 99882)?;
    let mut completed = execute(harvested.snapshot().clone(), &plan)?;
    assert_eq!(stock(completed.snapshot(), effects::STOCK_PLANT_FIELD)?, 0);
    assert!(!reserve_work_required(&original, completed.snapshot())?);
    assert!(
        original
            .compile(completed.snapshot())
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
    );
    assert!(completed.snapshot().tick.0 < 2401);
    completed.advance_ticks(2401 - completed.snapshot().tick.0)?;
    assert_eq!(stock(completed.snapshot(), effects::STOCK_DRINK_FIELD)?, 41);
    assert_eq!(stock(completed.snapshot(), effects::STOCK_FOOD_FIELD)?, 63);
    assert_eq!(
        original.semantic_goal_truth(completed.snapshot())?,
        PredicateTruth::True
    );
    assert_eq!(original.canonical_json(), canonical);
    assert_eq!(
        acquisition.analysis["consumption"]["requested_tick_is_action_deadline"],
        false
    );
    assert_eq!(
        acquisition.analysis["consumption"]["continuous_maintenance_proven"],
        false
    );
    Ok(())
}

#[test]
fn staged_work_horizon_can_exceed_reserve_tick_without_extending_stage_deadlines() -> Result<()> {
    let mut adapter = MemoryAdapter::new(source(20)?);
    adapter.advance_ticks(1100)?;
    let snapshot = adapter.snapshot();
    let original = request(
        json!([{"item": "DRINK", "minimum": 140}, {"item": "FOOD", "minimum": 65}]),
        1102,
        false,
    )?;
    let compiled = original.compile(snapshot)?;
    assert_eq!(
        original.semantic_goal_truth(snapshot)?,
        PredicateTruth::False
    );
    assert_eq!(
        resource_requirement(&compiled, "PLANT")?["required_stock"],
        30
    );
    assert_eq!(resource_requirement(&compiled, "PLANT")?["deficit"], 10);
    let report = &compiled.analysis["consumption"];
    assert_eq!(report["reserve_until_tick"], snapshot.tick.0 + 1);
    assert_eq!(report["planning_horizon_tick"], snapshot.tick.0 + 4260);
    assert_eq!(
        compiled.analysis["resource_workflow"]["harvest_horizon_ticks"],
        1120
    );
    assert_eq!(
        compiled.analysis["resource_workflow"]["production_horizon_ticks"],
        3140
    );
    let bound = bind(snapshot, &compiled, 99883)?;
    assert!(!bound.constraints.iter().any(|constraint| matches!(
        constraint,
        Constraint::LaboratoryProductionReserveUntil(_)
            | Constraint::LaboratoryResourceProductionReserveUntil(_)
    )));
    let plan = seal(snapshot, &compiled, 99883)?;
    assert_eq!(maximum_deadline(&plan)?, snapshot.tick.0 + 1120);
    assert!(maximum_deadline(&plan)? > 1102);
    Ok(())
}

#[test]
fn funded_reserve_dispatches_only_observed_inputs_and_preserves_old_no_work() -> Result<()> {
    let snapshot = source(5)?;
    let original = provisions(2401)?;
    let compiled = original.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["resource_workflow"]["stage"],
        "production"
    );
    assert!(compiled.acquisition.is_none());
    assert!(
        parse_steps(&compiled.actions)?
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let bound = bind(&snapshot, &compiled, 99884)?;
    assert!(
        bound
            .constraints
            .contains(&Constraint::LaboratoryProductionReserveUntil(GameTick(
                2401
            )))
    );
    assert!(!bound.constraints.iter().any(|constraint| matches!(
        constraint,
        Constraint::LaboratoryResourceProductionReserveUntil(_)
    )));
    seal(&snapshot, &compiled, 99884)?;
    let historical = ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": [{"item": "DRINK", "minimum": 40}, {"item": "FOOD", "minimum": 60}],
            "acquire_resources": {},
        })
        .to_string(),
    )?;
    assert_eq!(
        historical.planner(),
        Some(ProductionPlanner::ResourceAwareV1)
    );
    assert!(
        historical
            .compile(&snapshot)
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
    );
    let funded = request(json!([{"item": "DRINK", "minimum": 33}]), 1201, false)?;
    assert!(!reserve_work_required(&funded, &snapshot)?);
    assert!(
        funded
            .compile(&snapshot)
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
    );
    Ok(())
}

#[test]
fn absolute_horizon_replay_expires_instead_of_becoming_a_new_duration() -> Result<()> {
    let original = provisions(2401)?;
    assert_eq!(
        original.planner(),
        Some(ProductionPlanner::ResourceReserveV1)
    );
    let canonical = original.canonical_json();
    assert_eq!(ProductionRequest::parse(&canonical)?, original);
    let mut adapter = MemoryAdapter::new(source(0)?);
    assert_eq!(
        original.compile(adapter.snapshot())?.analysis["consumption"]["source_tick"],
        1
    );
    adapter.advance_ticks(2000)?;
    let replay = ProductionRequest::parse(&canonical)?.compile(adapter.snapshot())?;
    assert_eq!(replay.analysis["consumption"]["reserve_until_tick"], 2401);
    assert_eq!(replay.analysis["consumption"]["source_tick"], 2001);
    assert!(reserve_work_required(&original, adapter.snapshot())?);
    adapter.advance_ticks(400)?;
    assert!(
        original
            .compile(adapter.snapshot())
            .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
    );
    assert!(
        reserve_work_required(&original, adapter.snapshot())
            .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
    );
    assert_eq!(original.canonical_json(), canonical);
    Ok(())
}

#[test]
fn unknown_companion_stock_or_population_never_yields_reserve_eligibility() -> Result<()> {
    let original = provisions(2401)?;
    for (entity, field) in [
        (starter::STOCK_LEDGER, effects::STOCK_FOOD_FIELD),
        (starter::STOCK_LEDGER, effects::STOCK_DRINK_FIELD),
        (starter::STOCK_LEDGER, effects::METABOLISM_TICKS_FIELD),
        (EntityId::new(starter::FIRST_DWARF), "alive"),
    ] {
        let mut snapshot = source(0)?;
        snapshot
            .graph
            .entities
            .get_mut(&entity)
            .and_then(|record| record.fields.get_mut(field))
            .ok_or_else(|| invalid("resource reserve fixture fact missing"))?
            .source = FactSource::AgentAssertion("not current observed evidence".to_owned());
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            reserve_work_required(&original, &snapshot).is_err(),
            "{field}"
        );
        assert!(original.compile(&snapshot).is_err(), "{field}");
        assert_eq!(snapshot, before);
    }
    let mut snapshot = source(0)?;
    snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("resource reserve fixture ledger missing"))?
        .fields
        .remove(effects::STOCK_PLANT_FIELD);
    snapshot.refresh_hash();
    // Discovery establishes output demand, never availability of ingredients.
    assert!(reserve_work_required(&original, &snapshot)?);
    assert!(original.compile(&snapshot).is_err());
    Ok(())
}

#[test]
fn reserve_demand_never_chooses_between_duplicate_or_unqualified_ledgers() -> Result<()> {
    let original = provisions(2401)?;
    for duplicate in [false, true] {
        let mut snapshot = source(0)?;
        if duplicate {
            let mut ledger = snapshot
                .graph
                .entities
                .get(&starter::STOCK_LEDGER)
                .cloned()
                .ok_or_else(|| invalid("resource reserve fixture ledger missing"))?;
            ledger.id = EntityId::new(998_801);
            snapshot.graph.entities.insert(ledger.id, ledger);
        } else {
            snapshot
                .graph
                .entities
                .get_mut(&starter::STOCK_LEDGER)
                .ok_or_else(|| invalid("resource reserve fixture ledger missing"))?
                .generation = 0;
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(reserve_work_required(&original, &snapshot).is_err());
        assert!(original.compile(&snapshot).is_err());
        assert_eq!(snapshot, before);
    }
    Ok(())
}

#[test]
fn reserve_horizon_and_quantity_overflow_refuse_without_modifying_source() -> Result<()> {
    let snapshot = source(0)?;
    let before = snapshot.clone();
    for until in [0, snapshot.tick.0] {
        let original = provisions(until)?;
        assert!(
            original
                .compile(&snapshot)
                .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
        );
        assert!(
            reserve_work_required(&original, &snapshot)
                .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
        );
    }
    for until in [
        snapshot.tick.0 + effects::MAX_DEFAULT_OBLIGATION_TICKS + 1,
        u64::MAX,
    ] {
        let original = provisions(until)?;
        assert!(
            original
                .compile(&snapshot)
                .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
        );
        assert!(
            reserve_work_required(&original, &snapshot)
                .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
        );
    }
    let huge = request(json!([{"item": "DRINK", "minimum": u32::MAX}]), 2401, false)?;
    assert!(
        huge.compile(&snapshot)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    assert!(
        reserve_work_required(&huge, &snapshot)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(snapshot, before);
    let mut overflow = snapshot;
    laboratory_provisions_tests::write(
        &mut overflow,
        starter::STOCK_LEDGER,
        effects::METABOLISM_TICKS_FIELD,
        Value::U64(u64::MAX),
    )?;
    let before = overflow.clone();
    assert!(
        provisions(2401)?
            .compile(&overflow)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    assert!(
        reserve_work_required(&provisions(2401)?, &overflow)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(overflow, before);
    Ok(())
}

#[test]
fn finite_acquisition_and_current_production_prerequisites_remain_mandatory() -> Result<()> {
    assert!(provisions(50_001)?.compile(&source(0)?).is_err());
    let mut missing_workshop = source(0)?;
    missing_workshop.graph.entities.remove(&starter::STILL);
    missing_workshop.refresh_hash();
    assert!(provisions(2401)?.compile(&missing_workshop).is_err());
    let mut snapshot = source(0)?;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields.insert(
            format!(
                "{}{}",
                effects::LABOR_FIELD_PREFIX,
                HarvestResource::Plant.labor()
            ),
            lab_fact(Value::Bool(false)),
        );
    }
    snapshot.refresh_hash();
    assert!(provisions(2401)?.compile(&snapshot).is_err());
    let original = request(
        json!([{"item": "DRINK", "minimum": 40}, {"item": "FOOD", "minimum": 60}]),
        2401,
        true,
    )?;
    let compiled = original.compile(&snapshot)?;
    let acquisition = compiled
        .acquisition
        .as_ref()
        .ok_or_else(|| invalid("resource reserve fixture acquisition missing"))?;
    assert_eq!(acquisition.requested_actions.len(), 2);
    assert!(matches!(
        acquisition.requested_actions[0].action,
        Action::SetLabor { enabled: true, .. }
    ));
    assert!(!acquisition.requested_actions[0].preconditions.is_empty());
    assert!(acquisition.requested_actions[0].compensation.is_some());
    assert_eq!(acquisition.requested_actions[1].depends_on, vec![0]);
    let bound = bind(&snapshot, &compiled, 99885)?;
    assert_eq!(bound.requested_actions, acquisition.requested_actions);
    let plan = seal(&snapshot, &compiled, 99885)?;
    let harvested = execute(snapshot, &plan)?;
    assert_eq!(stock(harvested.snapshot(), effects::STOCK_PLANT_FIELD)?, 5);
    Ok(())
}

#[test]
fn workshop_and_recipe_inputs_are_acquired_together_before_any_downstream_effect() -> Result<()> {
    let mut snapshot = source(0)?;
    snapshot.graph.entities.remove(&starter::STILL);
    laboratory_provisions_tests::write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(0),
    )?;
    let original = ProductionRequest::parse_current(
        &json!({
            "template": "production",
            "quotas": [{"item": "DRINK", "minimum": 40}, {"item": "FOOD", "minimum": 60}],
            "reserve_until_tick": 4801,
            "acquire_resources": {"assign_labor": false},
            "prerequisites": {
                "assign_labor": false,
                "workshops": [{"building": "workshop:Still", "location": [0, 0, 10]}],
            },
        })
        .to_string(),
    )?;
    let acquisition = original.compile(&snapshot)?;
    assert_eq!(
        resource_requirement(&acquisition, "WOOD")?["required_stock"],
        1
    );
    assert_eq!(
        resource_requirement(&acquisition, "WOOD")?["workshop_units"],
        1
    );
    assert!(
        resource_requirement(&acquisition, "PLANT")?["required_stock"]
            .as_u64()
            .is_some_and(|units| units > 0)
    );
    assert!(
        parse_steps(&acquisition.actions)?
            .iter()
            .all(|step| matches!(step.action, Action::HarvestResource { .. }))
    );
    let plan = seal(&snapshot, &acquisition, 99886)?;
    let harvested = execute(snapshot, &plan)?;
    assert_eq!(stock(harvested.snapshot(), effects::STOCK_WOOD_FIELD)?, 1);
    assert!(
        !harvested
            .snapshot()
            .graph
            .entities
            .values()
            .any(|record| matches!(
                production_fact(record, "building_kind", harvested.snapshot().tick),
                Some(Value::Text(kind)) if kind == "workshop:Still"
            ))
    );
    let production = original.compile(harvested.snapshot())?;
    assert_eq!(
        production.analysis["resource_workflow"]["stage"],
        "production"
    );
    assert_eq!(
        parse_steps(&production.actions)?
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        1
    );
    let plan = seal(harvested.snapshot(), &production, 99887)?;
    let mut completed = execute(harvested.snapshot().clone(), &plan)?;
    assert_eq!(stock(completed.snapshot(), effects::STOCK_WOOD_FIELD)?, 0);
    assert!(completed.snapshot().tick.0 < 4801);
    completed.advance_ticks(4801 - completed.snapshot().tick.0)?;
    assert_eq!(
        original.semantic_goal_truth(completed.snapshot())?,
        PredicateTruth::True
    );
    Ok(())
}
