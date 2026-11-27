use super::*;

fn unstaffed_workshop() -> Result<WorldSnapshot> {
    let mut snapshot = scenario_snapshot("starter_fortress_economy", FortressId::new(996), false)?;
    snapshot.graph.entities.remove(&starter::CARPENTERS);
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields.insert(
            "labor.CONSTRUCTION".to_owned(),
            lab_fact(Value::Bool(false)),
        );
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn input() -> Json {
    json!({"template":"production","quotas":[{"item":"BED","minimum":2}],
        "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1",
            "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}]}})
}

#[test]
fn workshop_production_enables_constructor_before_build_and_finishes() -> Result<()> {
    let snapshot = unstaffed_workshop()?;
    let original = snapshot.clone();
    let request = ProductionRequest::parse_current(&input().to_string())?;
    let compiled = request.compile(&snapshot)?;
    let actions = parse_steps(&compiled.actions)?;
    assert!(
        matches!(&actions[0].action, Action::SetLabor { labor, enabled:true, .. } if labor == "CONSTRUCTION")
    );
    let build = actions
        .iter()
        .position(|step| matches!(step.action, Action::Build { .. }))
        .ok_or_else(|| invalid("missing planned workshop"))?;
    assert!(actions[build].depends_on.contains(&0));
    let order = actions
        .iter()
        .find(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
        .ok_or_else(|| invalid("missing production"))?;
    assert!(order.depends_on.contains(&(build as u32)));
    assert_eq!(
        compiled.analysis["prerequisites"]["setup_workforce"]["actions_added"],
        1
    );
    assert_eq!(snapshot, original);
    let completed = execute(snapshot, &compiled)?;
    assert_eq!(stock(&completed, effects::STOCK_BED_FIELD)?, 2);
    assert_eq!(stock(&completed, effects::STOCK_WOOD_FIELD)?, 17);
    Ok(())
}

#[test]
fn existing_constructor_needs_no_mutation_and_no_opt_in_preserves_legacy_actions() -> Result<()> {
    let mut snapshot = unstaffed_workshop()?;
    write(
        &mut snapshot,
        EntityId::new(starter::FIRST_DWARF),
        "labor.CONSTRUCTION",
        Value::Bool(true),
    )?;
    let current = ProductionRequest::parse_current(&input().to_string())?.compile(&snapshot)?;
    assert_eq!(
        current.analysis["prerequisites"]["setup_workforce"]["actions_added"],
        0
    );
    let mut legacy = input();
    legacy["prerequisites"]
        .as_object_mut()
        .ok_or_else(|| invalid("missing setup"))?
        .remove("workforce");
    let saved = ProductionRequest::parse_current(&legacy.to_string())?;
    assert!(!saved.canonical_json().contains("workforce"));
    let old = saved.compile(&snapshot)?;
    assert_eq!(old.actions, current.actions);
    assert_eq!(
        ProductionRequest::parse(&saved.canonical_json())?
            .compile(&snapshot)?
            .actions,
        old.actions
    );
    Ok(())
}

#[test]
fn setup_policy_refuses_unknown_or_unauthorized_constructor_and_wrong_model() -> Result<()> {
    let snapshot = unstaffed_workshop()?;
    let mut without_permission = input();
    without_permission["prerequisites"]["assign_labor"] = json!(false);
    assert!(
        ProductionRequest::parse_current(&without_permission.to_string())?
            .compile(&snapshot)
            .is_err()
    );
    let request = ProductionRequest::parse_current(&input().to_string())?;
    let mut unknown = snapshot.clone();
    for unit in unknown
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields
            .get_mut("labor.CONSTRUCTION")
            .ok_or_else(|| invalid("missing labor"))?
            .presence = Some(FactPresence::Unknown("uncaptured".to_owned()));
    }
    unknown.refresh_hash();
    assert!(request.compile(&unknown).is_err());
    assert!(
        request.compile(&source()?).is_err(),
        "unmarked worlds cannot acquire setup semantics"
    );
    let mut invalid_policy = input();
    invalid_policy["prerequisites"]["workforce"] = json!("automatic");
    assert!(ProductionRequest::parse_current(&invalid_policy.to_string()).is_err());
    assert!(
        ProductionRequest::parse(&input().to_string()).is_err(),
        "archive needs exact compiler generation"
    );
    Ok(())
}

#[test]
fn canonical_setup_source_round_trips_and_guards_bind_only_its_original_program() -> Result<()> {
    let snapshot = unstaffed_workshop()?;
    let request = ProductionRequest::parse_current(&input().to_string())?;
    let restored = ProductionRequest::parse(&request.canonical_json())?;
    assert_eq!(request, restored);
    let compiled = restored.compile(&snapshot)?;
    let mut intent = Intent {
        id: IntentId::new(996),
        anchor: snapshot.anchor(),
        summary: "complete staffed production".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    let mut unrelated = intent.clone();
    unrelated.requested_actions[0].action = Action::Pause { paused: true };
    let before = unrelated.clone();
    assert!(compiled.apply_capacity_horizon(&mut unrelated).is_err());
    assert_eq!(unrelated, before);
    compiled.apply_capacity_horizon(&mut intent)?;
    let first = &intent.requested_actions[0];
    assert!(!first.preconditions.is_empty());
    assert!(matches!(
        first.compensation,
        Some(Action::SetLabor { enabled: false, .. })
    ));
    let mut restricted = context(&snapshot);
    restricted
        .grants
        .retain(|grant| grant.capability != Capability::ConfigureLabor);
    let plan = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &restricted)?;
    let mut adapter = MemoryAdapter::new(snapshot.clone());
    assert!(adapter.prepare(&plan, &restricted).is_err());
    assert_eq!(adapter.snapshot(), &snapshot);
    assert!(adapter.prepare(&plan, &context(&snapshot)).is_ok());
    Ok(())
}

#[test]
fn future_drink_reserve_includes_new_constructor_and_workshop_service() -> Result<()> {
    let mut snapshot =
        scenario_snapshot("starter_fortress_provisions", FortressId::new(9961), false)?;
    snapshot.graph.entities.remove(&starter::STILL);
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields.insert(
            "labor.CONSTRUCTION".to_owned(),
            lab_fact(Value::Bool(false)),
        );
    }
    snapshot.refresh_hash();
    let request = ProductionRequest::parse_current(
        &json!({"template":"production", "reserve_until_tick":2401,
            "quotas":[{"item":"DRINK","minimum":40}],
            "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1",
                "workshops":[{"building":"workshop:Still","location":[0,0,10]}]}})
        .to_string(),
    )?;
    let compiled = request.compile(&snapshot)?;
    assert_eq!(
        compiled.analysis["prerequisites"]["setup_workforce"]["actions_added"],
        1
    );
    let mut intent = Intent {
        id: IntentId::new(9961),
        anchor: snapshot.anchor(),
        summary: "fund construction and the original future drink reserve".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let deadline = plan
        .steps
        .iter()
        .filter_map(|step| {
            step.obligation
                .as_ref()
                .map(|obligation| obligation.deadline_tick.0)
        })
        .max()
        .ok_or_else(|| invalid("missing completion deadline"))?;
    assert_eq!(
        Some(deadline),
        compiled.analysis["consumption"]["work_completion_horizon_tick"].as_u64()
    );
    let mut completed = execute(snapshot, &compiled)?;
    let until = compiled.analysis["consumption"]["planning_horizon_tick"]
        .as_u64()
        .ok_or_else(|| invalid("missing reserve horizon"))?;
    let elapsed = until
        .checked_sub(completed.tick.0)
        .ok_or_else(|| invalid("work exceeded its reserved horizon"))?;
    completed.tick = GameTick(until);
    effects::advance_effects(&mut completed, elapsed)?;
    completed.refresh_hash();
    assert!(stock(&completed, effects::STOCK_DRINK_FIELD)? >= 40);
    assert_eq!(
        PredicateEvidence::laboratory(&completed)?.evaluate(&compiled.terminal)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn underground_site_mines_then_builds_then_makes_the_original_beds() -> Result<()> {
    let mut snapshot = unstaffed_workshop()?;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|row| row.kind == EntityKind::Unit)
    {
        unit.fields
            .insert("labor.MINING".into(), lab_fact(Value::Bool(false)));
    }
    snapshot.refresh_hash();
    let mut input = input();
    let site = &mut input["prerequisites"]["workshops"][0];
    site["location"] = json!([2, 6, 10]);
    site["min"] = json!([1, 5, 10]);
    site["max"] = json!([3, 7, 10]);
    assert!(
        ProductionRequest::parse_current(&input.to_string())?
            .compile(&snapshot)
            .is_err()
    );
    input["prerequisites"]["workshops"][0]["preparation"] = json!("mine_rock_v1");
    let request = ProductionRequest::parse_current(&input.to_string())?;
    let compiled = request.compile(&snapshot)?;
    let restored = ProductionRequest::parse(&request.canonical_json())?;
    assert_eq!(request, restored);
    assert_eq!(compiled.actions, restored.compile(&snapshot)?.actions);
    let actions = parse_steps(&compiled.actions)?;
    let find = |test: fn(&Action) -> bool| {
        actions
            .iter()
            .position(|step| test(&step.action))
            .ok_or_else(|| invalid("workshop project step missing"))
    };
    let miner = find(|a| matches!(a, Action::SetLabor{labor,..} if labor == "MINING"))?;
    let constructor = find(|a| matches!(a, Action::SetLabor{labor,..} if labor == "CONSTRUCTION"))?;
    let dig = find(|a| matches!(a, Action::DesignateDig { .. }))?;
    let build = find(|a| matches!(a, Action::Build { .. }))?;
    let order = find(|a| matches!(a, Action::CreateWorkOrder { .. }))?;
    assert_eq!(actions[dig].depends_on, vec![miner as u32]);
    assert!(actions[build].depends_on.contains(&(dig as u32)));
    assert!(actions[build].depends_on.contains(&(constructor as u32)));
    assert!(actions[order].depends_on.contains(&(build as u32)));
    assert_eq!(
        snapshot.tile_code_at(MapCoord::new(3, 7, 10)),
        Some(tile_codes::SOLID_WALL)
    );
    let finished = execute(snapshot, &compiled)?;
    assert_eq!(
        finished.tile_code_at(MapCoord::new(3, 7, 10)),
        Some(tile_codes::FLOOR)
    );
    assert_eq!(stock(&finished, effects::STOCK_BED_FIELD)?, 2);
    assert_eq!(stock(&finished, effects::STOCK_WOOD_FIELD)?, 17);
    Ok(())
}

#[test]
fn mine_rock_sites_skip_ready_floor_and_refuse_unknown_or_unsafe_terrain() -> Result<()> {
    let mut snapshot = unstaffed_workshop()?;
    let mut input = input();
    input["prerequisites"]["workshops"][0]["preparation"] = json!("mine_rock_v1");
    let request = ProductionRequest::parse_current(&input.to_string())?;
    let ready = request.compile(&snapshot)?;
    assert!(
        parse_steps(&ready.actions)?
            .iter()
            .all(|step| !matches!(step.action, Action::DesignateDig { .. }))
    );
    let site = MapCoord::new(0, 0, 10);
    for code in [tile_codes::OPEN_SPACE, tile_codes::STAIR, tile_codes::TREE] {
        snapshot.set_tile_code(site, code)?;
        snapshot.refresh_hash();
        assert!(request.compile(&snapshot).is_err());
    }
    for marker in [Json::Null, json!(true), json!("dig_anything")] {
        input["prerequisites"]["workshops"][0]["preparation"] = marker;
        assert!(ProductionRequest::parse_current(&input.to_string()).is_err());
    }
    Ok(())
}

#[test]
fn reserve_mining_must_lead_to_the_matching_workshop_and_output() -> Result<()> {
    let mut world = scenario_snapshot("starter_fortress_provisions", FortressId::new(997), false)?;
    world.graph.entities.remove(&starter::STILL);
    world.refresh_hash();
    let request = ProductionRequest::parse_current(&json!({
        "template":"production", "reserve_until_tick":2401, "quotas":[{"item":"DRINK","minimum":40}],
        "prerequisites":{"assign_labor":true,"workforce":"complete_project_v1",
            "workshops":[{"building":"workshop:Still","location":[2,6,10],"preparation":"mine_rock_v1"}]}
    }).to_string())?;
    let compiled = request.compile(&world)?;
    let mut original = Intent {
        id: IntentId::new(997),
        anchor: world.anchor(),
        summary: "maintain the original drink reserve".into(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut original)?;
    assert!(PredicateEvidence::laboratory(&world)?.establishes(&original.terminal_condition)?);
    let planner = StaticPlanner::default();
    let plan = planner.prepare_laboratory(&world, &original, &context(&world))?;
    assert!(plan.required_capabilities.contains(&Capability::Designate));
    let dig = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::DesignateDig { .. }))
        .ok_or_else(|| invalid("reserve lost its original workshop excavation"))?;
    let guard = Predicate::All(dig.preconditions.clone()).normalized();
    assert!(PredicateEvidence::laboratory(&world)?.establishes(&guard)?);
    let mut changed_terrain = world.clone();
    changed_terrain.set_tile_code(MapCoord::new(2, 6, 10), tile_codes::STAIR)?;
    changed_terrain.refresh_hash();
    assert!(!PredicateEvidence::laboratory(&changed_terrain)?.establishes(&guard)?);
    for mutation in 0..3 {
        let mut changed = original.clone();
        for step in &mut changed.requested_actions {
            match (&mut step.action, mutation) {
                (Action::CreateWorkOrder { .. }, 0) => step.depends_on.clear(),
                (Action::Build { kind, .. }, 1) => *kind = BuildingKind::Workshop("Kitchen".into()),
                (Action::DesignateDig { mode, .. }, 2) => *mode = DigMode::Channel,
                _ => {}
            }
        }
        assert!(
            planner
                .prepare_laboratory(&world, &changed, &context(&world))
                .is_err()
        );
    }
    Ok(())
}
