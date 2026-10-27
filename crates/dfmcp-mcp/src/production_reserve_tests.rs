//! Explicit reserve predictions must fund actual work without changing the
//! original current-stock goal or giving its actions a later deadline.

use super::*;

fn reserve(quotas: Json, until: u64) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({"template":"production", "quotas":quotas, "reserve_until_tick":until}).to_string(),
    )
}

fn provisions(plants: u64) -> Result<WorldSnapshot> {
    let mut snapshot =
        scenario_snapshot("starter_fortress_provisions", FortressId::new(9560), false)?;
    write(
        &mut snapshot,
        starter::STOCK_LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(plants),
    )?;
    Ok(snapshot)
}

fn requirement<'a>(compiled: &'a ProductionCompilation, item: &str) -> Result<&'a Json> {
    compiled.analysis["requirements"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["item"] == item))
        .ok_or_else(|| invalid("reserve fixture requirement missing"))
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
        .ok_or_else(|| invalid("reserve fixture obligation missing"))
}

#[test]
fn explicit_reserve_produces_after_original_minima_are_already_true() -> Result<()> {
    let source = provisions(5)?;
    let original = reserve(
        json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]),
        2401,
    )?;
    let before = source.clone();
    let compiled = original.compile(&source)?;
    assert_eq!(source, before);
    assert_eq!(
        PredicateEvidence::laboratory(&source)?.evaluate(&compiled.terminal)?,
        PredicateTruth::True,
        "a currently satisfied goal must not suppress explicit future reserve work",
    );
    assert_eq!(
        requirement(&compiled, "DRINK")?["planning_stock_target"],
        54
    );
    assert_eq!(requirement(&compiled, "FOOD")?["planning_stock_target"], 67);
    assert_eq!(requirement(&compiled, "PLANT")?["consumed_units"], 5);
    let plan = seal(&source, &compiled, 9560)?;
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(
        maximum_deadline(&plan)?,
        compiled.analysis["consumption"]["work_completion_horizon_tick"]
            .as_u64()
            .ok_or_else(|| invalid("work horizon missing"))?,
    );
    assert!(maximum_deadline(&plan)? < 2401);
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_PLANT_FIELD)?, 0);
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_DRINK_FIELD)?, 55);
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_FOOD_FIELD)?, 70);
    assert_eq!(
        compiled.analysis["consumption"]["continuous_maintenance_proven"],
        false,
    );
    assert_eq!(
        compiled.analysis["consumption"]["requested_tick_is_action_deadline"],
        false,
    );
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}

#[test]
fn both_reserves_share_exact_finite_ingredients_and_qualified_workers() -> Result<()> {
    let original = reserve(
        json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}]),
        2401,
    )?;
    let short = provisions(4)?;
    let before = short.clone();
    let refusal = original
        .compile(&short)
        .err()
        .ok_or_else(|| invalid("one-short ingredient pool unexpectedly accepted"))?;
    assert_eq!(refusal.code, ErrorCode::PreconditionsFailed);
    assert!(refusal.message.contains("PLANT"));
    assert_eq!(short, before);

    let mut source = provisions(5)?;
    one_worker(&mut source);
    let compiled = original.compile(&source)?;
    let steps = parse_steps(&compiled.actions)?;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].depends_on, vec![0]);
    let plan = seal(&source, &compiled, 9561)?;
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}

#[test]
fn reserve_horizon_does_not_replace_later_work_and_setup_consumption() -> Result<()> {
    let mut source = provisions(20)?;
    source.graph.entities.remove(&starter::STILL);
    source.graph.entities.remove(&starter::KITCHEN);
    write(
        &mut source,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(2),
    )?;
    let raw = json!({
        "template":"production", "reserve_until_tick":101,
        "quotas":[{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65}],
        "prerequisites":{"assign_labor":false,"workshops":[
            {"building":"workshop:Still","location":[0,0,10]},
            {"building":"workshop:Kitchen","location":[2,0,10]}
        ]}
    });
    let original = ProductionRequest::parse_current(&raw.to_string())?;
    let compiled = original.compile(&source)?;
    let consumption = &compiled.analysis["consumption"];
    assert_eq!(consumption["reserve_until_tick"], 101);
    assert_eq!(
        consumption["planning_horizon_tick"],
        consumption["work_completion_horizon_tick"]
    );
    assert!(
        consumption["planning_horizon_tick"]
            .as_u64()
            .is_some_and(|tick| tick > 2401)
    );
    assert!(
        requirement(&compiled, "DRINK")?["consumption_allowance"]
            .as_u64()
            .is_some_and(|n| n >= 14)
    );
    assert!(
        requirement(&compiled, "FOOD")?["consumption_allowance"]
            .as_u64()
            .is_some_and(|n| n >= 7)
    );
    assert_eq!(
        compiled.analysis["construction_materials"]["new_workshop_wood"],
        2
    );
    let plan = seal(&source, &compiled, 9562)?;
    assert_eq!(
        maximum_deadline(&plan)?,
        consumption["planning_horizon_tick"]
            .as_u64()
            .ok_or_else(|| invalid("planning horizon missing"))?
    );
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_WOOD_FIELD)?, 0);
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}

#[test]
fn pending_original_orders_debit_reserve_ingredients_without_crediting_future_stock() -> Result<()>
{
    let original = reserve(json!([{"item":"DRINK","minimum":40}]), 1201)?;
    for (plants, accepted) in [(4, false), (5, true)] {
        let mut source = provisions(plants)?;
        effects::apply_effect(
            &mut source,
            &Action::CreateWorkOrder {
                name: "older meal input demand".to_owned(),
                job_token: "PREPARE_MEAL".to_owned(),
                amount: 3,
                conditions: Vec::new(),
            },
            "reserve-existing-meal",
        )?;
        source.refresh_hash();
        let before = source.clone();
        let outcome = original.compile(&source);
        assert_eq!(source, before);
        if accepted {
            let compiled = outcome?;
            assert_eq!(
                compiled.analysis["materials"]["PLANT"]["reserved_for_existing_orders"],
                3
            );
            assert_eq!(
                compiled.analysis["materials"]["PLANT"]["available_for_new_plan"],
                2
            );
            assert_eq!(requirement(&compiled, "DRINK")?["stock"], 40);
            assert_eq!(
                requirement(&compiled, "DRINK")?["planning_stock_target"],
                47
            );
        } else {
            assert!(outcome.is_err_and(|error| error.code == ErrorCode::PreconditionsFailed));
        }
    }
    Ok(())
}

#[test]
fn source_generation_is_closed_and_horizon_replay_never_extends_absolute_time() -> Result<()> {
    let raw = json!({"template":"production","quotas":[{"item":"DRINK","minimum":40}],"reserve_until_tick":2401});
    let request = ProductionRequest::parse_current(&raw.to_string())?;
    assert_eq!(
        request.planner(),
        Some(ProductionPlanner::ConsumptionReserveV1)
    );
    assert_eq!(request.reserve_until_tick(), Some(2401));
    let canonical = request.canonical_json();
    assert_eq!(ProductionRequest::parse(&canonical)?, request);
    assert!(ProductionRequest::parse(&raw.to_string()).is_err());
    for wrong in [
        json!("consumption_aware_v1"),
        json!("consumption_reserve_v2"),
    ] {
        let mut invalid_request = raw.clone();
        invalid_request["planner"] = wrong;
        assert!(ProductionRequest::parse_current(&invalid_request.to_string()).is_err());
    }
    let mut missing = raw.clone();
    missing
        .as_object_mut()
        .ok_or_else(|| invalid("request object missing"))?
        .remove("reserve_until_tick");
    missing["planner"] = json!("consumption_reserve_v1");
    assert!(ProductionRequest::parse_current(&missing.to_string()).is_err());
    let source = world_at(1001)?;
    let first = request.compile(&source)?;
    assert_eq!(first.analysis["consumption"]["reserve_horizon_ticks"], 1400);
    let later = world_at(2001)?;
    let replay = ProductionRequest::parse(&canonical)?.compile(&later)?;
    assert_eq!(replay.analysis["consumption"]["reserve_until_tick"], 2401);
    assert_eq!(replay.analysis["consumption"]["reserve_horizon_ticks"], 400);
    assert_eq!(request.canonical_json(), canonical);
    assert!(
        request
            .compile(&world_at(2401)?)
            .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
    );
    assert!(
        request
            .clone()
            .with_planner(ProductionPlanner::ConsumptionAwareV1)
            .compile(&source)
            .is_err()
    );
    Ok(())
}

#[test]
fn reserve_bounds_overflow_and_unavailable_consumption_refuse_without_effects() -> Result<()> {
    let source = world_at(1)?;
    let quotas = json!([{"item":"DRINK","minimum":45}]);
    let exact_year = reserve(
        quotas.clone(),
        source.tick.0 + effects::MAX_DEFAULT_OBLIGATION_TICKS,
    )?
    .compile(&source)?;
    assert_eq!(
        exact_year.analysis["consumption"]["planning_horizon_ticks"],
        effects::MAX_DEFAULT_OBLIGATION_TICKS,
    );
    for until in [0, 1] {
        assert!(
            reserve(quotas.clone(), until)?
                .compile(&source)
                .is_err_and(|error| error.code == ErrorCode::InvalidRequest)
        );
    }
    for until in [effects::MAX_DEFAULT_OBLIGATION_TICKS + 2, u64::MAX] {
        assert!(
            reserve(quotas.clone(), until)?
                .compile(&source)
                .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
        );
    }
    for invalid_tick in [json!(-1), json!(1.5), json!("2401"), json!({})] {
        assert!(
            ProductionRequest::parse_current(
                &json!({
                    "template":"production", "quotas":quotas, "reserve_until_tick":invalid_tick,
                })
                .to_string()
            )
            .is_err()
        );
    }
    let huge = reserve(json!([{"item":"DRINK","minimum":u32::MAX}]), 2401)?;
    assert!(
        huge.compile(&source)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    let mut overflow = source.clone();
    write(
        &mut overflow,
        starter::STOCK_LEDGER,
        effects::METABOLISM_TICKS_FIELD,
        Value::U64(u64::MAX),
    )?;
    assert!(
        reserve(quotas.clone(), 2401)?
            .compile(&overflow)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    let mut unknown = source.clone();
    unknown
        .graph
        .entities
        .get_mut(&EntityId::new(starter::FIRST_DWARF))
        .ok_or_else(|| invalid("worker missing"))?
        .fields
        .remove("alive");
    unknown.refresh_hash();
    let before = unknown.clone();
    assert!(
        reserve(quotas, 2401)?
            .compile(&unknown)
            .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed)
    );
    assert_eq!(unknown, before);
    Ok(())
}

#[test]
fn already_funded_reserves_do_not_invent_work_and_legacy_no_work_stays_unchanged() -> Result<()> {
    let source = world_at(1)?;
    for request in [
        request(json!([{"item":"DRINK","minimum":40}]))?,
        reserve(json!([{"item":"DRINK","minimum":30}]), 1201)?,
        reserve(json!([{"item":"DRINK","minimum":40}]), 100)?,
    ] {
        let canonical = request.canonical_json();
        assert!(
            request
                .compile(&source)
                .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
        );
        assert_eq!(request.canonical_json(), canonical);
    }
    Ok(())
}

#[test]
fn reserve_admission_binds_only_true_original_goals_and_refuses_atomically() -> Result<()> {
    let source = provisions(20)?;
    let unmet = reserve(json!([{"item":"DRINK","minimum":45}]), 2401)?.compile(&source)?;
    assert_eq!(
        PredicateEvidence::laboratory(&source)?.evaluate(&unmet.terminal)?,
        PredicateTruth::False
    );
    let mut ordinary = intent(&source, &unmet, 9570)?;
    unmet.apply_capacity_horizon(&mut ordinary)?;
    assert!(
        !ordinary.constraints.iter().any(|constraint| matches!(
            constraint,
            Constraint::LaboratoryProductionReserveUntil(_)
        ))
    );
    let prior_compatible =
        StaticPlanner::default().prepare_laboratory(&source, &ordinary, &context(&source))?;
    assert_eq!(
        prior_compatible.summary, ordinary.summary,
        "previously executable false-goal reserve plans retain the exact old summary/seal normalization"
    );
    assert_eq!(prior_compatible.terminal_condition, unmet.terminal);

    let satisfied = reserve(json!([{"item":"DRINK","minimum":40}]), 2401)?.compile(&source)?;
    let mut bound = intent(&source, &satisfied, 9571)?;
    satisfied.apply_capacity_horizon(&mut bound)?;
    assert!(
        bound
            .constraints
            .contains(&Constraint::LaboratoryProductionReserveUntil(GameTick(
                2401
            )))
    );
    for changed_goal in [false, true] {
        let mut unrelated = intent(&source, &satisfied, 9572)?;
        if changed_goal {
            unrelated.terminal_condition = Predicate::Paused(true);
        } else {
            unrelated
                .constraints
                .push(Constraint::LaboratoryProductionReserveUntil(GameTick(2501)));
        }
        let before = unrelated.clone();
        assert!(satisfied.apply_capacity_horizon(&mut unrelated).is_err());
        assert_eq!(unrelated, before);
    }
    Ok(())
}

#[test]
fn satisfied_original_reserve_executes_explicit_workshop_setup_before_production() -> Result<()> {
    let mut source = provisions(20)?;
    source.graph.entities.remove(&starter::STILL);
    source.graph.entities.remove(&starter::KITCHEN);
    write(
        &mut source,
        starter::STOCK_LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(2),
    )?;
    let raw = json!({"template":"production","reserve_until_tick":2401,
    "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":60}],
    "prerequisites":{"assign_labor":false,"workshops":[
        {"building":"workshop:Still","location":[0,0,10]},
        {"building":"workshop:Kitchen","location":[2,0,10]}
    ]}});
    let compiled = ProductionRequest::parse_current(&raw.to_string())?.compile(&source)?;
    assert_eq!(
        PredicateEvidence::laboratory(&source)?.evaluate(&compiled.terminal)?,
        PredicateTruth::True
    );
    let plan = seal(&source, &compiled, 9573)?;
    assert_eq!(
        plan.steps
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        2
    );
    let (adapter, states) = execute(source.clone(), &plan)?;
    assert!(states.iter().all(|state| *state == CommitState::Verified));
    assert_eq!(stock(adapter.snapshot(), effects::STOCK_WOOD_FIELD)?, 0);
    assert_through_horizon(adapter, &source, &compiled)?;
    Ok(())
}
