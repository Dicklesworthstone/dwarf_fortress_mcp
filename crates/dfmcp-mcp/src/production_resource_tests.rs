//! Staged production exercises the real compiler, planner and reference
//! economy. Acquisition completion is deliberately distinct from output truth.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId, OperationContext, RiskTier,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;

fn source() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress_resources", FortressId::new(9984), false)
}

fn request(quotas: Json) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template":"production", "quotas":quotas,
            "acquire_resources":{"assign_labor":false},
        })
        .to_string(),
    )
}

fn beds(minimum: u32) -> Result<ProductionRequest> {
    request(json!([{"item":"BED","minimum":minimum}]))
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

fn intent(snapshot: &WorldSnapshot, compiled: &ProductionCompilation, id: u128) -> Result<Intent> {
    Ok(Intent {
        id: IntentId::new(id),
        anchor: snapshot.anchor(),
        summary: "reach the original production goal through reviewed observed stages".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    })
}

fn execute(
    snapshot: WorldSnapshot,
    compiled: &ProductionCompilation,
    id: u128,
) -> Result<WorldSnapshot> {
    let mut intent = intent(&snapshot, compiled, id)?;
    compiled.apply_capacity_horizon(&mut intent)?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|o| o.deadline_tick.0))
        .max()
        .ok_or_else(|| invalid("resource test has no bounded obligations"))?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
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
    Ok(adapter.snapshot().clone())
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|ledger| production_fact(ledger, field, snapshot.tick))
    {
        Some(Value::U64(quantity)) => Ok(*quantity),
        _ => Err(invalid("resource test stock unavailable")),
    }
}

#[test]
fn resource_policy_pins_closed_generation_and_preserves_historical_sources() -> Result<()> {
    let historical = r#"{"template":"production","quotas":[{"item":"BED","minimum":2}]}"#;
    let legacy = ProductionRequest::parse(historical)?;
    assert_eq!(legacy.planner(), None);
    assert!(!legacy.is_resource_aware());
    assert_eq!(
        legacy.canonical_json(),
        r#"{"quotas":[{"item":"BED","minimum":2}],"template":"production"}"#
    );
    let modern = ProductionRequest::parse_current(historical)?;
    assert_eq!(
        modern.planner(),
        Some(ProductionPlanner::ConsumptionAwareV1)
    );
    assert!(!modern.is_resource_aware());
    let staged = beds(22)?;
    assert!(staged.is_resource_aware());
    assert_eq!(staged.planner(), Some(ProductionPlanner::ResourceAwareV1));
    assert_eq!(ProductionRequest::parse(&staged.canonical_json())?, staged);
    assert_eq!(
        serde_json::from_str::<ProductionRequest>(&staged.canonical_json())
            .map_err(|e| invalid(e.to_string()))?,
        staged
    );
    for raw in [
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":null}"#,
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{"assign_labor":null}}"#,
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{"ignore_stock":true}}"#,
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{"assign_labor":false,"assign_labor":true}}"#,
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{},"planner":"consumption_aware_v1"}"#,
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{},"planner":"resource_aware_v1","reserve_until_tick":1000}"#,
        r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"planner":"resource_aware_v1"}"#,
    ] {
        assert!(ProductionRequest::parse_current(raw).is_err(), "{raw}");
    }
    let intake = r#"{"template":"production","quotas":[{"item":"BED","minimum":22}],"acquire_resources":{}}"#;
    assert!(ProductionRequest::parse(intake).is_err());
    let duplicate = staged.canonical_json().replacen(
        "\"template\":\"production\"",
        "\"template\":\"production\",\"template\":\"production\"",
        1,
    );
    assert!(serde_json::from_str::<ProductionRequest>(&duplicate).is_err());
    Ok(())
}

#[test]
fn source_bound_harvest_only_stage_then_observed_production_reaches_original_quota() -> Result<()> {
    let snapshot = source()?;
    let before = snapshot.canonical_bytes();
    let request = beds(22)?;
    let first = request.compile(&snapshot)?;
    assert_eq!(
        snapshot.canonical_bytes(),
        before,
        "diagnostic material demand is read-only"
    );
    assert_eq!(
        first.analysis["resource_workflow"]["stage"],
        "acquire_resources"
    );
    assert_eq!(
        first.analysis["resource_workflow"]["requirements"][0]["observed_stock"],
        20
    );
    assert_eq!(
        first.analysis["resource_workflow"]["requirements"][0]["required_stock"],
        22
    );
    assert_eq!(
        first.analysis["resource_workflow"]["requirements"][0]["deficit"],
        2
    );
    let actions = parse_steps(&first.actions)?;
    assert_eq!(actions.len(), 1);
    assert!(matches!(
        actions[0].action,
        Action::HarvestResource {
            resource: HarvestResource::Wood,
            amount: 2,
            ..
        }
    ));
    let harvested = execute(snapshot, &first, 99841)?;
    assert_eq!(stock(&harvested, effects::STOCK_WOOD_FIELD)?, 22);
    assert_eq!(stock(&harvested, effects::STOCK_BED_FIELD)?, 0);
    assert_eq!(
        request.semantic_goal_truth(&harvested)?,
        PredicateTruth::False
    );
    let second = request.compile(&harvested)?;
    assert_eq!(second.analysis["resource_workflow"]["stage"], "production");
    assert!(
        parse_steps(&second.actions)?
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let completed = execute(harvested, &second, 99842)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 22);
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    assert!(request.compile(&completed).is_err());
    Ok(())
}

#[test]
fn workshop_setup_wood_is_budgeted_once_without_dispatching_downstream_setup() -> Result<()> {
    let mut snapshot = source()?;
    snapshot.graph.entities.remove(&starter::CARPENTERS);
    laboratory_provisions_tests::write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(0),
    )?;
    let request = ProductionRequest::parse_current(&json!({
        "template":"production", "quotas":[{"item":"BED","minimum":1},{"item":"CHAIR","minimum":1}],
        "acquire_resources":{}, "prerequisites":{"assign_labor":false,
            "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}]},
    }).to_string())?;
    let first = request.compile(&snapshot)?;
    let row = &first.analysis["resource_workflow"]["requirements"][0];
    assert_eq!(row["production_units"], 2);
    assert_eq!(row["workshop_units"], 1);
    assert_eq!(row["required_stock"], 3);
    assert!(
        parse_steps(&first.actions)?
            .iter()
            .all(|step| matches!(step.action, Action::HarvestResource { amount: 3, .. }))
    );
    let harvested = execute(snapshot, &first, 99843)?;
    let second = request.compile(&harvested)?;
    assert_eq!(
        parse_steps(&second.actions)?
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        1
    );
    let completed = execute(harvested, &second, 99844)?;
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    Ok(())
}

#[test]
fn material_acquisition_never_bypasses_staffing_sites_or_finite_source_supply() -> Result<()> {
    let mut snapshot = source()?;
    snapshot.graph.entities.remove(&starter::CARPENTERS);
    snapshot.refresh_hash();
    assert!(beds(22)?.compile(&snapshot).is_err());
    let mut snapshot = source()?;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields
            .insert("labor.CARPENTRY".to_owned(), lab_fact(Value::Bool(false)));
    }
    snapshot.refresh_hash();
    assert!(beds(22)?.compile(&snapshot).is_err());
    assert!(
        beds(53)?.compile(&source()?).is_err(),
        "20 observed + 32 finite logs cannot fund 53 beds"
    );
    Ok(())
}

#[test]
fn existing_material_demand_is_counted_and_competing_consumers_refuse_harvest() -> Result<()> {
    let mut snapshot = source()?;
    effects::apply_effect(
        &mut snapshot,
        &Action::CreateWorkOrder {
            name: "earlier tables".to_owned(),
            job_token: "CONSTRUCT_TABLE".to_owned(),
            amount: 4,
            conditions: Vec::new(),
        },
        "resource-production-existing-tables",
    )?;
    snapshot.refresh_hash();
    let request = beds(18)?;
    let diagnostic = request.compile_candidate(&snapshot, &request.quotas, true, true)?;
    assert_eq!(diagnostic.resource_requirements[0].production_units, 18);
    assert_eq!(diagnostic.resource_requirements[0].existing_demand_units, 4);
    assert_eq!(diagnostic.resource_requirements[0].required_stock, 22);
    assert_eq!(diagnostic.resource_requirements[0].deficit, 2);
    assert!(
        request.compile(&snapshot).is_err(),
        "an active raw consumer requires observed settlement before harvest sizing"
    );
    let funded = beds(16)?.compile(&snapshot)?;
    assert_eq!(funded.analysis["resource_workflow"]["stage"], "production");
    Ok(())
}

#[test]
fn acquisition_labor_permission_retains_guards_and_compensation_when_bound() -> Result<()> {
    let mut snapshot = source()?;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields
            .insert("labor.WOODCUTTING".to_owned(), lab_fact(Value::Bool(false)));
    }
    snapshot.refresh_hash();
    assert!(beds(22)?.compile(&snapshot).is_err());
    let request = ProductionRequest::parse_current(
        &json!({
            "template":"production", "quotas":[{"item":"BED","minimum":22}],
            "acquire_resources":{"assign_labor":true},
        })
        .to_string(),
    )?;
    let compiled = request.compile(&snapshot)?;
    let core = compiled
        .acquisition
        .as_ref()
        .ok_or_else(|| invalid("missing acquisition"))?;
    assert_eq!(core.requested_actions.len(), 2);
    assert!(!core.requested_actions[0].preconditions.is_empty());
    assert!(core.requested_actions[0].compensation.is_some());
    let mut intent = intent(&snapshot, &compiled, 99845)?;
    compiled.apply_capacity_horizon(&mut intent)?;
    assert_eq!(intent.requested_actions, core.requested_actions);
    assert_eq!(intent.requested_actions[1].depends_on, vec![0]);
    StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    Ok(())
}

#[test]
fn source_or_program_or_goal_changes_cannot_receive_an_acquisition_binding() -> Result<()> {
    let snapshot = source()?;
    let compiled = beds(22)?.compile(&snapshot)?;
    for variant in 0..4 {
        let mut intent = intent(&snapshot, &compiled, 99846)?;
        match variant {
            0 => intent.anchor.tick = GameTick(snapshot.tick.0 + 1),
            1 => intent.terminal_condition = Predicate::All(Vec::new()),
            2 => intent.requested_actions[0].depends_on.push(0),
            _ => intent
                .constraints
                .push(Constraint::LaboratoryProductionReserveUntil(GameTick(1000))),
        }
        let unchanged = intent.clone();
        assert!(compiled.apply_capacity_horizon(&mut intent).is_err());
        assert_eq!(intent, unchanged);
    }
    Ok(())
}

#[test]
fn satisfied_original_outputs_do_not_acquire_unneeded_or_unknown_ingredients() -> Result<()> {
    let mut snapshot = source()?;
    laboratory_provisions_tests::write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_BED_FIELD,
        Value::U64(22),
    )?;
    let ledger = snapshot
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("ledger missing"))?;
    ledger.fields.remove(effects::STOCK_WOOD_FIELD);
    ledger.fields.remove(effects::STOCK_PLANT_FIELD);
    snapshot.refresh_hash();
    let request = beds(22)?;
    assert_eq!(
        request.semantic_goal_truth(&snapshot)?,
        PredicateTruth::True
    );
    assert_eq!(
        request
            .compile(&snapshot)
            .err()
            .ok_or_else(|| invalid("no-op scheduled work"))?
            .code,
        ErrorCode::InvalidIntent
    );
    Ok(())
}

#[test]
fn retained_output_truth_rejects_replacement_ledger_in_both_material_stages() -> Result<()> {
    let snapshot = source()?;
    for minimum in [2, 22] {
        let request = beds(minimum)?;
        let compilation = request.compile(&snapshot)?;
        let mut intent = intent(&snapshot, &compilation, 99850)?;
        compilation.apply_capacity_horizon(&mut intent)?;
        let original =
            StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
        let ledger = ProductionGoalLedger::capture(&snapshot)?;
        ledger.validate_plan(&original)?;
        assert_eq!(
            minimum == 22,
            original
                .steps
                .iter()
                .any(|step| matches!(step.action, Action::HarvestResource { .. }))
        );
        for replacement_id in [starter::STOCK_LEDGER, EntityId::new(990_001)] {
            let mut changed = snapshot.clone();
            let mut replacement = changed
                .graph
                .entities
                .remove(&starter::STOCK_LEDGER)
                .ok_or_else(|| invalid("original fixture ledger missing"))?;
            replacement.id = replacement_id;
            replacement.generation += 1;
            replacement.fields.insert(
                effects::STOCK_BED_FIELD.to_owned(),
                lab_fact(Value::U64(u64::from(minimum))),
            );
            changed.graph.entities.insert(replacement_id, replacement);
            changed.cursor = changed
                .cursor
                .checked_next()
                .ok_or_else(|| invalid("fixture cursor overflow"))?;
            changed.refresh_hash();
            assert_eq!(
                request.semantic_goal_truth(&changed)?,
                PredicateTruth::True,
                "a new explicit request can inspect the current unique ledger"
            );
            assert_eq!(
                request
                    .semantic_goal_truth_in_ledger(&changed, ledger)
                    .err()
                    .map(|error| error.code),
                Some(ErrorCode::PreconditionsFailed),
                "a retained original cannot silently adopt a replacement ledger"
            );
            let replacement_binding = ProductionGoalLedger::capture(&changed)?;
            assert!(
                replacement_binding.validate_plan(&original).is_err(),
                "post-source identity cannot replace original custody"
            );
        }
    }
    Ok(())
}

#[test]
fn unknown_or_asserted_material_and_output_never_become_zero_or_satisfaction() -> Result<()> {
    for field in [effects::STOCK_WOOD_FIELD, effects::STOCK_BED_FIELD] {
        let mut snapshot = source()?;
        snapshot
            .graph
            .entities
            .get_mut(&starter::STOCK_LEDGER)
            .and_then(|ledger| ledger.fields.get_mut(field))
            .ok_or_else(|| invalid("fixture fact missing"))?
            .source = FactSource::AgentAssertion("not an observation".to_owned());
        snapshot.refresh_hash();
        assert!(beds(22)?.compile(&snapshot).is_err());
        if field == effects::STOCK_BED_FIELD {
            assert_eq!(
                beds(22)?.semantic_goal_truth(&snapshot)?,
                PredicateTruth::Unknown
            );
        }
    }
    Ok(())
}

#[test]
fn funded_resource_generation_uses_exact_strict_production_service_horizon() -> Result<()> {
    let snapshot = source()?;
    for orders in [None, Some("dependency_preserving_v1")] {
        let mut raw = json!({"template":"production","quotas":[{"item":"BED","minimum":2},{"item":"CHAIR","minimum":1}],"acquire_resources":{}});
        if let Some(orders) = orders {
            raw["orders"] = json!(orders);
        }
        let request = ProductionRequest::parse_current(&raw.to_string())?;
        let compilation = request.compile(&snapshot)?;
        assert_eq!(
            compilation.analysis["resource_workflow"]["stage"],
            "production"
        );
        assert_eq!(
            compilation.analysis["resource_workflow"]["harvest_horizon_ticks"],
            0
        );
        assert_eq!(
            compilation.analysis["resource_workflow"]["production_horizon_ticks"],
            compilation.workload.horizon_ticks(&compilation.actions)?
        );
        execute(snapshot.clone(), &compilation, 99847)?;
    }
    Ok(())
}

#[test]
fn plant_demand_includes_consumption_through_harvest_and_full_production() -> Result<()> {
    let mut adapter = MemoryAdapter::new(source()?);
    adapter.advance_ticks(1100)?;
    let snapshot = adapter.snapshot();
    let request = request(json!([{"item":"DRINK","minimum":140},{"item":"FOOD","minimum":65}]))?;
    let compiled = request.compile(snapshot)?;
    let workflow = &compiled.analysis["resource_workflow"];
    assert_eq!(workflow["stage"], "acquire_resources");
    assert_eq!(workflow["requirements"][0]["resource"], "PLANT");
    // The fixed point includes both sealed service allowances, rather than
    // assuming immediate review at the earliest possible physical completion.
    // Twenty-six brewing and four cooking batches need thirty plants after
    // budgeting four drink and two meal boundaries through the combined span.
    assert_eq!(workflow["requirements"][0]["required_stock"], 30);
    assert_eq!(workflow["requirements"][0]["deficit"], 10);
    let harvest_ticks = workflow["harvest_horizon_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("harvest horizon missing"))?;
    let production_ticks = workflow["production_horizon_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("production horizon missing"))?;
    assert_eq!(harvest_ticks, 1120);
    assert_eq!(production_ticks, 3140);
    assert_eq!(
        compiled.analysis["consumption"]["planning_horizon_ticks"],
        harvest_ticks + production_ticks
    );
    assert_eq!(
        compiled.analysis["consumption"]["planner"],
        "resource_aware_v1"
    );
    assert_eq!(compiled.analysis["consumption"]["fixed_point_rounds"], 4);
    assert!(parse_steps(&compiled.actions)?.iter().all(|step| matches!(
        step.action,
        Action::HarvestResource {
            resource: HarvestResource::Plant,
            amount: 10,
            ..
        }
    )));
    let combined_deadline = snapshot.tick.0 + harvest_ticks + production_ticks;
    let harvest_deadline = snapshot.tick.0 + harvest_ticks;
    let harvested = execute(snapshot.clone(), &compiled, 99848)?;
    assert_eq!(stock(&harvested, effects::STOCK_PLANT_FIELD)?, 30);
    assert_eq!(
        request.semantic_goal_truth(&harvested)?,
        PredicateTruth::False
    );

    // Delay the foreground continuation until the original harvest allowance
    // expires. The complete reserve must still be funded by observed inputs.
    let mut adapter = MemoryAdapter::new(harvested);
    adapter.advance_ticks(harvest_deadline - adapter.snapshot().tick.0)?;
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_DRINK_FIELD)?, 33);
    let production = request.compile(adapter.snapshot())?;
    assert_eq!(
        production.analysis["resource_workflow"]["stage"],
        "production"
    );
    assert_eq!(
        production.analysis["resource_workflow"]["requirements"][0]["required_stock"],
        30
    );
    let completed = execute(adapter.snapshot().clone(), &production, 99849)?;
    assert_eq!(stock(&completed, effects::STOCK_PLANT_FIELD)?, 0);
    let mut adapter = MemoryAdapter::new(completed);
    adapter.advance_ticks(combined_deadline - adapter.snapshot().tick.0)?;
    assert_eq!(
        request.semantic_goal_truth(adapter.snapshot())?,
        PredicateTruth::True
    );
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_DRINK_FIELD)?, 142);
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_FOOD_FIELD)?, 66);
    Ok(())
}
