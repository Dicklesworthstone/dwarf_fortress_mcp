//! Original furniture quotas through the real compiler, planner and reference
//! adapter. These prove laboratory semantics, never native game compatibility.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, PredicateEvidence, PredicateTruth};

#[path = "production_material_tests.rs"]
mod materials;

fn context(snapshot: &WorldSnapshot) -> OperationContext {
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

fn source() -> Result<WorldSnapshot> {
    scenario_snapshot("starter_fortress", FortressId::new(954), false)
}

fn request(quotas: Json) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template":"production", "quotas":quotas,
        })
        .to_string(),
    )
}

fn furniture_request() -> Result<ProductionRequest> {
    request(json!([
        {"item":"BED","minimum":2},
        {"item":"CHAIR","minimum":2},
        {"item":"DOOR","minimum":1},
        {"item":"TABLE","minimum":1},
    ]))
}

fn write(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("fixture entity missing"))?
        .fields
        .insert(field.to_owned(), lab_fact(value));
    snapshot.refresh_hash();
    Ok(())
}

fn stock(snapshot: &WorldSnapshot, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&starter::STOCK_LEDGER)
        .and_then(|record| production_fact(record, field, snapshot.tick))
    {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(invalid("fixture stock missing")),
    }
}

fn execute(source: WorldSnapshot, compiled: &ProductionCompilation) -> Result<WorldSnapshot> {
    let mut intent = Intent {
        id: IntentId::new(954),
        anchor: source.anchor(),
        summary: "produce the complete original furniture stock".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    let plan = StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source))?;
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| {
            step.obligation
                .as_ref()
                .map(|obligation| obligation.deadline_tick.0)
        })
        .max()
        .unwrap_or(source.tick.0);
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let mut states: Vec<_> = receipt.actions.iter().map(|action| action.state).collect();
    while adapter.snapshot().tick.0 < horizon && states.iter().any(|state| !state.is_terminal()) {
        let ticks = effects::DEFAULT_POLL_INTERVAL_TICKS.min(horizon - adapter.snapshot().tick.0);
        adapter.advance_ticks(ticks)?;
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
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::True,
    );
    Ok(adapter.snapshot().clone())
}

#[test]
fn complete_furniture_quota_consumes_shared_logs_and_produces_exact_items() -> Result<()> {
    let source = source()?;
    let before = source.clone();
    let compiled = furniture_request()?.compile(&source)?;
    assert_eq!(source, before, "planning is read-only");
    let actions = parse_steps(&compiled.actions)?;
    assert_eq!(actions.len(), 4);
    for step in &actions {
        let Action::CreateWorkOrder { conditions, .. } = &step.action else {
            return Err(invalid("unexpected furniture setup"));
        };
        assert!(conditions.contains(&WorkOrderCondition::MaterialAvailable {
            material_token: "WOOD".to_owned(),
            minimum: 1,
        }));
    }
    let completed = execute(source, &compiled)?;
    assert_eq!(
        stock(&completed, effects::STOCK_WOOD_FIELD)?,
        starter::OPENING_WOOD - 6
    );
    for (field, amount) in [
        (effects::STOCK_BED_FIELD, 2),
        (effects::STOCK_CHAIR_FIELD, 2),
        (effects::STOCK_DOOR_FIELD, 1),
        (effects::STOCK_TABLE_FIELD, 1),
    ] {
        assert_eq!(stock(&completed, field)?, amount);
    }
    assert_eq!(
        completed
            .graph
            .entities
            .values()
            .filter(|record| { record.kind == EntityKind::Item })
            .count(),
        6
    );
    for row in compiled.analysis["requirements"]
        .as_array()
        .ok_or_else(|| invalid("missing report"))?
    {
        assert_eq!(row["consumption_allowance"], 0);
    }
    Ok(())
}

#[test]
fn mixed_economy_and_all_four_furniture_outputs_share_one_sealed_goal() -> Result<()> {
    let original = request(json!([
        {"item":"DRINK","minimum":45}, {"item":"FOOD","minimum":65},
        {"item":"BED","minimum":1}, {"item":"DOOR","minimum":1},
        {"item":"TABLE","minimum":1}, {"item":"CHAIR","minimum":1},
    ]))?;
    let source = source()?;
    let compiled = original.compile(&source)?;
    assert_eq!(parse_steps(&compiled.actions)?.len(), 6);
    let completed = execute(source, &compiled)?;
    assert_eq!(
        stock(&completed, effects::STOCK_WOOD_FIELD)?,
        starter::OPENING_WOOD - 4
    );
    assert!(stock(&completed, effects::STOCK_DRINK_FIELD)? >= 45);
    assert!(stock(&completed, effects::STOCK_FOOD_FIELD)? >= 65);
    Ok(())
}

#[test]
fn every_furniture_output_credits_the_same_finite_wood_pool_once() -> Result<()> {
    let mut source = source()?;
    write(
        &mut source,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(5),
    )?;
    let before = source.clone();
    let failure = match furniture_request()?.compile(&source) {
        Err(error) => error,
        Ok(_) => return Err(invalid("insufficient shared logs were accepted")),
    };
    assert_eq!(failure.code, ErrorCode::PreconditionsFailed);
    assert!(failure.message.contains("WOOD"));
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn unknown_or_unqualified_material_and_output_counts_are_never_zero_stock() -> Result<()> {
    for field in [effects::STOCK_WOOD_FIELD, effects::STOCK_BED_FIELD] {
        for variant in 0..4 {
            let mut source = source()?;
            let ledger = source
                .graph
                .entities
                .get_mut(&starter::STOCK_LEDGER)
                .ok_or_else(|| invalid("missing ledger"))?;
            match variant {
                0 => {
                    ledger.fields.remove(field);
                }
                1 => {
                    let fact = ledger
                        .fields
                        .get_mut(field)
                        .ok_or_else(|| invalid("missing stock"))?;
                    fact.presence = Some(FactPresence::Unknown("uncaptured".to_owned()));
                }
                2 => {
                    let fact = ledger
                        .fields
                        .get_mut(field)
                        .ok_or_else(|| invalid("missing stock"))?;
                    fact.source = FactSource::AgentAssertion("untrusted count".to_owned());
                }
                _ => {
                    let fact = ledger
                        .fields
                        .get_mut(field)
                        .ok_or_else(|| invalid("missing stock"))?;
                    fact.observed_at = GameTick(source.tick.0 + 1);
                }
            }
            source.refresh_hash();
            assert!(
                request(json!([{"item":"BED","minimum":1}]))?
                    .compile(&source)
                    .is_err(),
                "accepted {field} variant {variant}"
            );
        }
    }
    Ok(())
}

#[test]
fn shared_carpentry_setup_builds_once_and_enables_one_worker_once() -> Result<()> {
    let mut source = source()?;
    source.graph.entities.remove(&starter::CARPENTERS);
    for unit in source
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        unit.fields.remove("labor.CARPENTRY");
    }
    write(
        &mut source,
        EntityId::new(starter::FIRST_DWARF),
        "labor.CARPENTRY",
        Value::Bool(false),
    )?;
    let mut input: Json = serde_json::from_str(&furniture_request()?.canonical_json())
        .map_err(|_| invalid("invalid original request"))?;
    input["prerequisites"] = json!({
        "assign_labor":true,
        "workshops":[{"building":"workshop:Carpenters","location":[7,0,10]}],
    });
    let compiled = ProductionRequest::parse_current(&input.to_string())?.compile(&source)?;
    let actions = parse_steps(&compiled.actions)?;
    assert_eq!(actions.len(), 6);
    assert_eq!(
        actions
            .iter()
            .filter(|step| matches!(step.action, Action::SetLabor { .. }))
            .count(),
        1
    );
    assert_eq!(
        actions
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        1
    );
    execute(source, &compiled)?;
    Ok(())
}

#[test]
fn serial_furniture_outputs_keep_one_enabled_carpenter_without_extra_labor_changes() -> Result<()> {
    let mut source = source()?;
    let carpenter = EntityId::new(starter::FIRST_DWARF + 3);
    for unit in source
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        unit.fields.insert(
            "labor.CARPENTRY".to_owned(),
            lab_fact(Value::Bool(unit.id == carpenter)),
        );
    }
    source.refresh_hash();
    let mut input: Json = serde_json::from_str(&furniture_request()?.canonical_json())
        .map_err(|_| invalid("invalid furniture request"))?;
    input["prerequisites"] = json!({"assign_labor":true});
    let compiled = ProductionRequest::parse_current(&input.to_string())?.compile(&source)?;
    let actions = parse_steps(&compiled.actions)?;
    assert_eq!(actions.len(), 4);
    assert!(
        actions
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let staffing = compiled.analysis["prerequisites"]["staffing"]
        .as_array()
        .ok_or_else(|| invalid("missing furniture staffing"))?;
    assert_eq!(staffing.len(), 4);
    for row in staffing {
        assert_eq!(row["unit"], carpenter.get().to_string());
        assert_eq!(row["prior_labor"], "enabled");
        assert!(row["action_index"].is_null());
    }
    execute(source, &compiled)?;
    Ok(())
}

#[test]
fn six_outputs_use_three_capacity_families_and_preserve_the_only_carpenter() -> Result<()> {
    let mut source = source()?;
    let carpenter = EntityId::new(starter::FIRST_DWARF);
    let brewer = EntityId::new(starter::FIRST_DWARF + 1);
    let cook = EntityId::new(starter::FIRST_DWARF + 2);
    for unit in source
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        for (labor, specialist) in [("CARPENTRY", carpenter), ("BREW", brewer), ("COOK", cook)] {
            // The lowest-ID citizen can do all three jobs, but is the only
            // enabled carpenter. Joint selection must retain that scarce role.
            unit.fields.insert(
                format!("labor.{labor}"),
                lab_fact(Value::Bool(unit.id == carpenter || unit.id == specialist)),
            );
        }
    }
    source.refresh_hash();
    let input = json!({
        "template":"production",
        "quotas":[
            {"item":"BED","minimum":1}, {"item":"CHAIR","minimum":1},
            {"item":"DOOR","minimum":1}, {"item":"TABLE","minimum":1},
            {"item":"DRINK","minimum":45}, {"item":"FOOD","minimum":65},
        ],
        "prerequisites":{"assign_labor":true},
    });
    let compiled = ProductionRequest::parse_current(&input.to_string())?.compile(&source)?;
    let actions = parse_steps(&compiled.actions)?;
    assert_eq!(actions.len(), 6);
    assert!(
        actions
            .iter()
            .all(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    let staffing = compiled.analysis["prerequisites"]["staffing"]
        .as_array()
        .ok_or_else(|| invalid("missing mixed staffing"))?;
    assert_eq!(staffing.len(), 6);
    let mut selected = BTreeSet::new();
    for row in staffing {
        let expected = match row["labor"].as_str() {
            Some("CARPENTRY") => carpenter,
            Some("BREW") => brewer,
            Some("COOK") => cook,
            _ => return Err(invalid("unknown staffing family")),
        };
        assert_eq!(row["unit"], expected.get().to_string());
        assert!(row["action_index"].is_null());
        selected.insert(expected);
    }
    assert_eq!(selected.len(), 3);
    execute(source, &compiled)?;
    Ok(())
}

fn prior_order(source: &mut WorldSnapshot, job: &str, amount: u32) -> Result<EntityId> {
    let action = Action::CreateWorkOrder {
        name: "prior carpentry".to_owned(),
        job_token: job.to_owned(),
        amount,
        conditions: Vec::new(),
    };
    effects::apply_effect(source, &action, "prior-carpentry-order")?;
    source.refresh_hash();
    Ok(effects::created_entity_id("prior-carpentry-order", 0))
}

#[test]
fn older_other_output_work_debits_material_and_service_without_counting_future_output() -> Result<()>
{
    let mut source = source()?;
    prior_order(&mut source, "CONSTRUCT_TABLE", 4)?;
    let admitted = request(json!([{"item":"BED","minimum":16}]))?.compile(&source)?;
    assert_eq!(admitted.analysis["materials"]["WOOD"]["observed_stock"], 20);
    assert_eq!(
        admitted.analysis["materials"]["WOOD"]["reserved_for_existing_orders"],
        4
    );
    assert_eq!(
        admitted.analysis["materials"]["WOOD"]["available_for_new_plan"],
        16
    );
    assert_eq!(
        admitted.analysis["capacity"]["remaining_registered_service_ticks"],
        200
    );
    assert!(
        request(json!([{"item":"BED","minimum":17}]))?
            .compile(&source)
            .is_err()
    );
    let completed = execute(source, &admitted)?;
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 0);
    assert_eq!(stock(&completed, effects::STOCK_TABLE_FIELD)?, 4);
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 16);
    Ok(())
}

#[test]
fn unresolved_legacy_carpentry_and_duplicate_output_producers_refuse_new_quota_work() -> Result<()>
{
    let mut source = source()?;
    let id = prior_order(&mut source, "CONSTRUCT_BED", 1)?;
    assert!(
        request(json!([{"item":"BED","minimum":2}]))?
            .compile(&source)
            .is_err()
    );
    source
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("missing prior order"))?
        .fields
        .remove(effects::WORK_ORDER_PRODUCTION_MODEL_FIELD);
    source.refresh_hash();
    assert!(
        request(json!([{"item":"CHAIR","minimum":1}]))?
            .compile(&source)
            .is_err()
    );
    Ok(())
}

#[test]
fn historical_economy_source_keeps_its_program_with_unmarked_abstract_carpentry() -> Result<()> {
    let mut source = source()?;
    let original = ProductionRequest::parse(
        r#"{"template":"production","quotas":[{"item":"DRINK","minimum":50}]}"#,
    )?;
    let prior = original.compile(&source)?;
    let id = prior_order(&mut source, "CONSTRUCT_BED", 1)?;
    source
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("missing old abstract order"))?
        .fields
        .remove(effects::WORK_ORDER_PRODUCTION_MODEL_FIELD);
    source.refresh_hash();
    let recovered = original.compile(&source)?;
    assert_eq!(prior.actions, recovered.actions);
    assert_eq!(prior.terminal, recovered.terminal);
    assert_eq!(recovered.workload.reserved_wood_units(), 0);
    assert!(recovered.analysis.get("capacity").is_none());
    assert!(recovered.consumption_horizon.is_none());
    Ok(())
}

#[test]
fn furniture_without_a_compiler_marker_still_seals_shared_capacity_dependencies() -> Result<()> {
    let mut raw: Json = serde_json::from_str(&furniture_request()?.canonical_json())
        .map_err(|_| invalid("invalid original request"))?;
    raw.as_object_mut()
        .ok_or_else(|| invalid("expected original request object"))?
        .remove("planner");
    let request = ProductionRequest::parse(&raw.to_string())?;
    assert!(request.planner().is_none());
    let source = source()?;
    let compiled = request.compile(&source)?;
    let actions = parse_steps(&compiled.actions)?;
    assert_eq!(actions.len(), 4);
    for (index, step) in actions.iter().enumerate().skip(1) {
        assert!(step.depends_on.contains(&((index - 1) as u32)));
    }
    execute(source, &compiled)?;
    Ok(())
}

#[test]
fn normalized_quota_order_is_deterministic_and_unsupported_recipes_still_refuse() -> Result<()> {
    let left = furniture_request()?;
    let right = request(json!([
        {"item":"TABLE","minimum":1}, {"item":"BED","minimum":1},
        {"item":"DOOR","minimum":1}, {"item":"BED","minimum":2},
        {"item":"CHAIR","minimum":2},
    ]))?;
    let source = source()?;
    assert_eq!(left.canonical_json(), right.canonical_json());
    assert_eq!(
        left.compile(&source)?.actions,
        right.compile(&source)?.actions
    );
    assert!(request(json!([{"item":"BARREL","minimum":1}])).is_err());
    Ok(())
}
