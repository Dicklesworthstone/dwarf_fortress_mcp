//! Real production compilation, sealing and execution with retained order policy.

use super::*;

fn dependency_request(quotas: Json) -> Result<ProductionRequest> {
    ProductionRequest::parse_current(
        &json!({
            "template":"production", "orders":"dependency_preserving_v1", "quotas":quotas,
        })
        .to_string(),
    )
}

fn orders(compiled: &ProductionCompilation) -> Result<Vec<(String, Vec<WorkOrderCondition>)>> {
    parse_steps(&compiled.actions)?
        .into_iter()
        .filter_map(|step| match step.action {
            Action::CreateWorkOrder {
                name, conditions, ..
            } => Some(Ok((name, conditions))),
            _ => None,
        })
        .collect()
}

fn sealed(source: &WorldSnapshot, compiled: &ProductionCompilation) -> Result<PreparedPlan> {
    let mut intent = Intent {
        id: IntentId::new(954),
        anchor: source.anchor(),
        summary: "retained production policy".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    StaticPlanner::default().prepare_laboratory(source, &intent, &context(source))
}

fn shared_food_source() -> Result<WorldSnapshot> {
    let mut snapshot = source()?;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|entity| entity.kind == EntityKind::Unit)
    {
        for labor in ["labor.BREW", "labor.COOK"] {
            unit.fields.insert(
                labor.to_owned(),
                lab_fact(Value::Bool(unit.id.get() == starter::FIRST_DWARF)),
            );
        }
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn consumed_food(snapshot: WorldSnapshot) -> Result<WorldSnapshot> {
    let mut adapter = MemoryAdapter::new(snapshot);
    adapter.advance_ticks(2_400)?;
    Ok(adapter.snapshot().clone())
}

#[test]
fn unmarked_original_sources_keep_their_exact_names_and_canonical_bytes() -> Result<()> {
    let legacy = r#"{"quotas":[{"item":"BED","minimum":2}],"template":"production"}"#;
    let request = ProductionRequest::parse(legacy)?;
    assert_eq!(request.canonical_json(), legacy);
    let world = source()?;
    let original = request.compile(&world)?;
    assert_eq!(orders(&original)?[0].0, "bed for quota");
    assert!(original.analysis.get("order_compiler").is_none());
    assert_eq!(
        original.actions,
        ProductionRequest::parse(legacy)?.compile(&world)?.actions
    );
    let current = ProductionRequest::parse_current(legacy)?;
    assert_eq!(
        current.canonical_json(),
        r#"{"planner":"consumption_aware_v1","quotas":[{"item":"BED","minimum":2}],"template":"production"}"#
    );
    assert_eq!(orders(&current.compile(&world)?)?[0].0, "bed for quota");
    Ok(())
}

#[test]
fn exact_source_reopen_preserves_policy_names_conditions_and_seal() -> Result<()> {
    let request =
        dependency_request(json!([{"item":"BED","minimum":3},{"item":"CHAIR","minimum":2}]))?;
    let canonical = request.canonical_json();
    assert_eq!(
        serde_json::from_str::<Json>(&canonical).map_err(|_| invalid("source JSON"))?["orders"],
        PRODUCTION_ORDER_POLICY
    );
    let reopened = ProductionRequest::parse(&canonical)?;
    assert_eq!(request, reopened);
    assert_eq!(canonical, reopened.canonical_json());
    let world = source()?;
    let before = world.clone();
    let a = request.compile(&world)?;
    let b = reopened.compile(&world)?;
    assert_eq!(a.actions, b.actions);
    assert_eq!(a.analysis, b.analysis);
    assert_eq!(a.terminal, b.terminal);
    assert_eq!(sealed(&world, &a)?, sealed(&world, &b)?);
    assert_eq!(world, before);
    let names: BTreeSet<_> = orders(&a)?.into_iter().map(|(name, _)| name).collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().all(|name| name.len() <= MAX_NAME_BYTES));
    assert_eq!(
        a.analysis["order_compiler"]["policy"],
        PRODUCTION_ORDER_POLICY
    );
    assert_eq!(a.analysis["order_compiler"]["reserves_materials"], false);
    Ok(())
}

#[test]
fn source_frontier_and_unused_original_permissions_change_names() -> Result<()> {
    let base = source()?;
    let request = dependency_request(json!([{"item":"BED","minimum":2}]))?;
    let a = request.compile(&base)?;
    let mut later = base.clone();
    later.cursor = later
        .cursor
        .checked_next()
        .ok_or_else(|| invalid("cursor"))?;
    later.refresh_hash();
    let b = request.compile(&later)?;
    assert_eq!(a.terminal, b.terminal);
    assert_ne!(orders(&a)?[0].0, orders(&b)?[0].0);
    assert_ne!(
        a.analysis["order_compiler"]["source_namespace"],
        b.analysis["order_compiler"]["source_namespace"]
    );
    let with_unused_site = ProductionRequest::parse_current(&json!({
        "template":"production", "orders":PRODUCTION_ORDER_POLICY,
        "quotas":[{"item":"BED","minimum":2}],
        "prerequisites":{"assign_labor":false,"workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}]},
    }).to_string())?;
    let c = with_unused_site.compile(&base)?;
    assert!(
        parse_steps(&c.actions)?
            .iter()
            .all(|step| !matches!(step.action, Action::Build { .. }))
    );
    assert_eq!(a.terminal, c.terminal);
    assert_ne!(orders(&a)?[0].0, orders(&c)?[0].0);
    Ok(())
}

#[test]
fn dependencies_and_per_batch_gates_survive_the_complete_six_output_workflow() -> Result<()> {
    let world = scenario_snapshot("starter_fortress_provisions", FortressId::new(954), false)?;
    let request = dependency_request(json!([
        {"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65},
        {"item":"BED","minimum":3},{"item":"DOOR","minimum":1},
        {"item":"TABLE","minimum":1},{"item":"CHAIR","minimum":2},
    ]))?;
    let compiled = request.compile(&world)?;
    let steps = parse_steps(&compiled.actions)?;
    assert_eq!(steps.len(), 6);
    let mut named_dependencies = 0;
    for step in &steps {
        let Action::CreateWorkOrder {
            job_token,
            conditions,
            ..
        } = &step.action
        else {
            return Err(invalid("expected production order"));
        };
        let material = if matches!(job_token.as_str(), "BREW_DRINK" | "PREPARE_MEAL") {
            "PLANT"
        } else {
            "WOOD"
        };
        assert!(conditions.contains(&WorkOrderCondition::MaterialAvailable {
            material_token: material.to_owned(),
            minimum: 1
        }));
        for predecessor in &step.depends_on {
            let Action::CreateWorkOrder { name, .. } = &steps[*predecessor as usize].action else {
                return Err(invalid("expected predecessor order"));
            };
            assert!(conditions.contains(&WorkOrderCondition::CompletedOrder {
                order_name: name.clone()
            }));
            named_dependencies += 1;
        }
    }
    assert!(named_dependencies >= 3);
    let completed = execute(world, &compiled)?;
    for (field, amount) in [
        (effects::STOCK_BED_FIELD, 3),
        (effects::STOCK_DOOR_FIELD, 1),
        (effects::STOCK_TABLE_FIELD, 1),
        (effects::STOCK_CHAIR_FIELD, 2),
    ] {
        assert_eq!(stock(&completed, field)?, amount);
    }
    assert_eq!(
        stock(&completed, effects::STOCK_WOOD_FIELD)?,
        starter::OPENING_WOOD - 7
    );
    assert!(stock(&completed, effects::STOCK_DRINK_FIELD)? >= 45);
    assert!(stock(&completed, effects::STOCK_FOOD_FIELD)? >= 65);
    Ok(())
}

#[test]
fn setup_indices_and_completions_remain_correct_after_workshop_and_labor_insertion() -> Result<()> {
    let mut world = scenario_snapshot("starter_fortress_economy", FortressId::new(954), false)?;
    world.graph.entities.remove(&starter::CARPENTERS);
    for unit in world
        .graph
        .entities
        .values_mut()
        .filter(|entity| entity.kind == EntityKind::Unit)
    {
        unit.fields.insert(
            "labor.CONSTRUCTION".to_owned(),
            lab_fact(Value::Bool(false)),
        );
    }
    world.refresh_hash();
    let request = ProductionRequest::parse_current(
        &json!({
            "template":"production", "orders":PRODUCTION_ORDER_POLICY,
            "quotas":[{"item":"BED","minimum":2},{"item":"CHAIR","minimum":2}],
            "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1",
                "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}]},
        })
        .to_string(),
    )?;
    let compiled = request.compile(&world)?;
    let steps = parse_steps(&compiled.actions)?;
    assert!(
        steps
            .iter()
            .any(|step| matches!(step.action, Action::Build { .. }))
    );
    assert!(steps.iter().any(
        |step| matches!(step.action,Action::SetLabor{ref labor,..} if labor == "CONSTRUCTION")
    ));
    for (index, step) in steps.iter().enumerate() {
        assert!(
            step.depends_on
                .iter()
                .all(|predecessor| (*predecessor as usize) < index)
        );
    }
    let completed = execute(world, &compiled)?;
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 2);
    assert_eq!(stock(&completed, effects::STOCK_CHAIR_FIELD)?, 2);
    assert_eq!(
        stock(&completed, effects::STOCK_WOOD_FIELD)?,
        starter::OPENING_WOOD - 5
    );
    Ok(())
}

#[test]
fn retained_prior_orders_cannot_alias_new_reviewed_pursuit_names() -> Result<()> {
    let original =
        dependency_request(json!([{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65}]))?;
    let source = shared_food_source()?;
    let first = original.compile(&source)?;
    let old_names: BTreeSet<_> = orders(&first)?.into_iter().map(|(name, _)| name).collect();
    let completed = consumed_food(execute(source, &first)?)?;
    let reopened = ProductionRequest::parse(&original.canonical_json())?;
    let second = reopened.compile(&completed)?;
    assert_eq!(first.terminal, second.terminal);
    let new_orders = orders(&second)?;
    assert!(new_orders.iter().all(|(name, _)| !old_names.contains(name)));
    let new_names: BTreeSet<_> = new_orders.iter().map(|(name, _)| name.clone()).collect();
    for (_, conditions) in &new_orders {
        for condition in conditions {
            if let WorkOrderCondition::CompletedOrder { order_name } = condition {
                assert!(new_names.contains(order_name));
                assert!(!old_names.contains(order_name));
            }
        }
    }
    let final_world = execute(completed, &second)?;
    assert!(stock(&final_world, effects::STOCK_DRINK_FIELD)? >= 45);
    assert!(stock(&final_world, effects::STOCK_FOOD_FIELD)? >= 65);
    Ok(())
}

#[test]
fn continuation_and_verified_recovery_preserve_the_original_order_policy() -> Result<()> {
    use crate::goal_continuation::{ProductionContinuation, RecoveredGoalLineage};

    let original =
        dependency_request(json!([{"item":"DRINK","minimum":45},{"item":"FOOD","minimum":65}]))?;
    let source = shared_food_source()?;
    let compiled = original.compile(&source)?;
    let original_plan = sealed(&source, &compiled)?;
    let original_bytes = original.canonical_json();
    let completed = consumed_food(execute(source.clone(), &compiled)?)?;
    let recovered = MemoryAdapter::recovered(completed.clone())?;
    RecoveredGoalLineage::verify(
        &completed,
        completed.anchor(),
        recovered.snapshot().anchor(),
    )?;
    let continuation =
        ProductionContinuation::new(original_plan.digest, original_plan.digest, original.clone())?;
    let retained = continuation.canonical_json();
    let value: Json = serde_json::from_str(&retained).map_err(|_| invalid("continuation JSON"))?;
    assert_eq!(value["production"]["orders"], PRODUCTION_ORDER_POLICY);
    assert_eq!(value["production"].to_string(), original_bytes);
    let reopened = ProductionContinuation::parse(&retained)?;
    assert_eq!(reopened.canonical_json(), retained);
    let (intent, analysis) = reopened.compile(
        IntentId::new(955),
        recovered.snapshot(),
        "continue original food and drink",
    )?;
    let current = original.compile(recovered.snapshot())?;
    // Continuation attaches the compiled capacity obligations before returning
    // the intent. Reconstruct those through the ordinary compilation path too.
    let mut expected = intent.clone();
    expected.requested_actions = parse_steps(&current.actions)?;
    current.apply_capacity_horizon(&mut expected)?;
    assert_eq!(intent.requested_actions, expected.requested_actions);
    assert_eq!(intent.terminal_condition, original_plan.terminal_condition);
    assert_eq!(
        analysis["order_compiler"]["policy"],
        PRODUCTION_ORDER_POLICY
    );
    let child = StaticPlanner::default().prepare_laboratory(
        recovered.snapshot(),
        &intent,
        &context(recovered.snapshot()),
    )?;
    assert_ne!(child.digest, original_plan.digest);
    // Exact original replay reconstructs the original seal; reopening a world
    // creates a new namespace only for the explicitly reviewed new pursuit.
    assert_eq!(
        sealed(
            &source,
            &ProductionRequest::parse(&original_bytes)?.compile(&source)?
        )?,
        original_plan
    );
    let finished = execute(recovered.snapshot().clone(), &current)?;
    assert!(stock(&finished, effects::STOCK_DRINK_FIELD)? >= 45);
    assert!(stock(&finished, effects::STOCK_FOOD_FIELD)? >= 65);
    Ok(())
}

#[test]
fn unsupported_unsealed_and_material_unknown_requests_refuse_before_effects() -> Result<()> {
    for raw in [
        r#"{"template":"production","orders":"unknown","quotas":[{"item":"BED","minimum":1}]}"#,
        r#"{"template":"production","orders":{"WOOD":"PLANT"},"quotas":[{"item":"BED","minimum":1}]}"#,
    ] {
        assert!(ProductionRequest::parse_current(raw).is_err());
    }
    assert!(ProductionRequest::parse(r#"{"template":"production","orders":"dependency_preserving_v1","quotas":[{"item":"BED","minimum":1}]}"#).is_err());
    let mut source = source()?;
    let ledger = source
        .graph
        .entities
        .get_mut(&starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("ledger"))?;
    ledger
        .fields
        .get_mut(effects::STOCK_WOOD_FIELD)
        .ok_or_else(|| invalid("wood"))?
        .presence = Some(FactPresence::Unknown("unobserved input".to_owned()));
    source.refresh_hash();
    let before = source.clone();
    assert!(
        dependency_request(json!([{"item":"BED","minimum":1}]))?
            .compile(&source)
            .is_err()
    );
    assert_eq!(source, before);
    Ok(())
}
