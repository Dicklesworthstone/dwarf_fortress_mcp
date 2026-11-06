//! Complete guarded room workflows starting without their prerequisite labor.

use super::*;
use dfmcp_core::{Digest32, EntityId};
use dfmcp_world::{Fact, FactPresence, FactSource};

fn unstaffed(single_worker: bool) -> Result<WorldSnapshot> {
    let mut snapshot = crate::lab_world::scenario_snapshot(
        "starter_fortress_economy",
        FortressId::new(997),
        false,
    )?;
    snapshot
        .graph
        .entities
        .remove(&crate::lab_world::starter::CARPENTERS);
    let tick = snapshot.tick;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        for labor in ["MINING", "CONSTRUCTION", "CARPENTRY"] {
            unit.fields.insert(
                format!("labor.{labor}"),
                qualified(Value::Bool(false), tick),
            );
        }
        if single_worker && unit.id != EntityId::new(crate::lab_world::starter::FIRST_DWARF) {
            unit.fields
                .insert("alive".to_owned(), qualified(Value::Bool(false), tick));
        }
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn qualified(value: Value, tick: dfmcp_core::GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn setup_request() -> Json {
    let mut value = request_value(false);
    value["room"]["prerequisites"] = json!({
        "assign_labor":true, "workforce":"complete_project_v1",
        "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}],
    });
    value
}

#[test]
fn complete_project_staffs_mining_construction_and_carpentry_and_finishes() -> Result<()> {
    let snapshot = unstaffed(false)?;
    let original = snapshot.clone();
    let request = ConnectedRoomRequest::parse_current(&setup_request().to_string())?;
    let (intent, analysis) = request.compile(IntentId::new(997), &snapshot, "")?;
    assert_eq!(snapshot, original);
    assert_eq!(analysis["setup_workforce"]["actions_added"], 2);
    let enabled: BTreeSet<_> = intent
        .requested_actions
        .iter()
        .filter_map(|step| match &step.action {
            Action::SetLabor {
                labor,
                enabled: true,
                ..
            } => Some(labor.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        enabled,
        BTreeSet::from(["MINING", "CONSTRUCTION", "CARPENTRY"])
    );
    for step in &intent.requested_actions {
        match step.action {
            Action::DesignateDig { .. } => assert!(step.depends_on.contains(&0)),
            Action::Build { .. } | Action::PlaceFurniture { .. } => {
                assert!(step.depends_on.contains(&1))
            }
            _ => {}
        }
    }
    for step in &intent.requested_actions[..2] {
        assert!(!step.preconditions.is_empty());
        assert!(matches!(
            step.compensation,
            Some(Action::SetLabor { enabled: false, .. })
        ));
    }
    let (completed, plan) = execute(snapshot, &intent)?;
    assert_eq!(wood(&completed)?, 17, "one workshop plus two exact beds");
    assert!(
        plan.required_capabilities
            .contains(&Capability::ConfigureLabor)
    );
    assert_eq!(analysis["allocation"][0]["supply"], "planned_production");
    Ok(())
}

#[test]
fn one_living_worker_completes_all_setup_and_room_stages_with_exact_item_keys() -> Result<()> {
    let snapshot = unstaffed(true)?;
    let request = ConnectedRoomRequest::parse_current(&setup_request().to_string())?;
    let (intent, analysis) = request.compile(IntentId::new(998), &snapshot, "")?;
    let expected = crate::lab_world::starter::FIRST_DWARF.to_string();
    for row in analysis["setup_workforce"]["selections"]
        .as_array()
        .ok_or_else(|| invalid("missing staffing"))?
    {
        assert_eq!(row["unit"], expected);
    }
    let (completed, _) = execute(snapshot, &intent)?;
    for row in analysis["allocation"]
        .as_array()
        .ok_or_else(|| invalid("missing allocation"))?
    {
        let id = row["item"]
            .as_str()
            .ok_or_else(|| invalid("missing item"))?
            .parse::<u64>()
            .map_err(|_| invalid("invalid item"))?;
        assert!(completed.graph.entities.contains_key(&EntityId::new(id)));
    }
    assert_eq!(wood(&completed)?, 17);
    Ok(())
}

#[test]
fn setup_requires_explicit_permission_and_qualified_capacity() -> Result<()> {
    let snapshot = unstaffed(false)?;
    let mut request = setup_request();
    request["room"]["prerequisites"]["assign_labor"] = json!(false);
    assert!(
        ConnectedRoomRequest::parse_current(&request.to_string())?
            .compile(IntentId::new(999), &snapshot, "")
            .is_err()
    );
    let request = ConnectedRoomRequest::parse_current(&setup_request().to_string())?;
    let mut unknown = snapshot.clone();
    for unit in unknown
        .graph
        .entities
        .values_mut()
        .filter(|unit| unit.kind == EntityKind::Unit)
    {
        unit.fields
            .get_mut("labor.CONSTRUCTION")
            .ok_or_else(|| invalid("missing construction"))?
            .presence = Some(FactPresence::Unknown("uncaptured".to_owned()));
    }
    unknown.refresh_hash();
    assert!(request.compile(IntentId::new(999), &unknown, "").is_err());
    let (intent, _) = request.compile(IntentId::new(999), &snapshot, "")?;
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
fn unmarked_room_sources_keep_their_existing_staffing_and_canonical_round_trip() -> Result<()> {
    let snapshot = unstaffed(false)?;
    let mut legacy = setup_request();
    legacy["room"]["prerequisites"]
        .as_object_mut()
        .ok_or_else(|| invalid("missing options"))?
        .remove("workforce");
    let request = ConnectedRoomRequest::parse_current(&legacy.to_string())?;
    let saved = request.canonical_json();
    assert!(!saved.contains("complete_project_v1"));
    let restored = ConnectedRoomRequest::parse(&saved)?;
    let (before, _) = request.compile(IntentId::new(1000), &snapshot, "")?;
    let (after, _) = restored.compile(IntentId::new(1000), &snapshot, "")?;
    assert_eq!(before, after);
    assert!(
        !before
            .requested_actions
            .iter()
            .any(|step| matches!(&step.action,
        Action::SetLabor { labor, .. } if labor == "MINING" || labor == "CONSTRUCTION"))
    );
    Ok(())
}

#[test]
fn setup_policy_survives_source_recovery_and_forecast_matches_execution() -> Result<()> {
    let snapshot = unstaffed(true)?;
    let request = ConnectedRoomRequest::parse_current(&setup_request().to_string())?;
    let retained = ConnectedRoomRequest::parse(&request.canonical_json())?;
    let decoded = WorldSnapshot::from_canonical_bytes(&snapshot.canonical_bytes())?;
    let (intent, _) = request.compile(IntentId::new(1001), &snapshot, "")?;
    assert_eq!(
        intent,
        retained.compile(IntentId::new(1001), &decoded, "")?.0
    );
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let forecast = crate::server::plan_forecast::forecast_plan(
        &MemoryAdapter::new(snapshot.clone()),
        &plan,
        &context(&snapshot),
    );
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    // Ready successors dispatch at observations. Compare the actual adapter
    // with the forecast's reported cadence, including its bounded coarse slices.
    let cadence = forecast["resolution_ticks"]
        .as_u64()
        .filter(|ticks| *ticks > 0)
        .ok_or_else(|| invalid("missing forecast cadence"))?;
    let (completed, _) = execute_with_cadence(snapshot, &intent, cadence)?;
    assert_eq!(forecast["predicted_completion_tick"], completed.tick.0);
    Ok(())
}

#[test]
fn already_excavated_room_needs_no_miner_but_preserves_every_terrain_goal() -> Result<()> {
    let mut snapshot = unstaffed(true)?;
    let worker = snapshot
        .graph
        .entities
        .get_mut(&EntityId::new(crate::lab_world::starter::FIRST_DWARF))
        .ok_or_else(|| invalid("fixture worker missing"))?;
    worker
        .fields
        .get_mut("labor.MINING")
        .ok_or_else(|| invalid("fixture mining fact missing"))?
        .presence = Some(FactPresence::Unknown("not observed".to_owned()));
    for labor in ["CONSTRUCTION", "CARPENTRY"] {
        worker.fields.insert(
            format!("labor.{labor}"),
            qualified(Value::Bool(true), snapshot.tick),
        );
    }
    let mut request_value = setup_request();
    request_value["room"]["prerequisites"]["assign_labor"] = json!(false);
    let request = ConnectedRoomRequest::parse_current(&request_value.to_string())?;
    for at in dfmcp_world::terrain::region_tiles(request.area) {
        snapshot.set_tile_code(at, tile_codes::FLOOR)?;
    }
    snapshot.refresh_hash();
    let original = snapshot.clone();
    let (intent, analysis) = request.compile(IntentId::new(1002), &snapshot, "")?;
    assert_eq!(snapshot, original);
    assert_eq!(analysis["setup_workforce"]["actions_added"], 0);
    let selections = analysis["setup_workforce"]["selections"]
        .as_array()
        .ok_or_else(|| invalid("missing setup selections"))?;
    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0]["labor"], "CONSTRUCTION");
    assert_eq!(analysis["access"]["new_corridor_tiles"], 0);
    assert!(
        !intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action, Action::SetLabor { .. }))
    );
    let digs: Vec<_> = intent
        .requested_actions
        .iter()
        .filter_map(|step| match step.action {
            Action::DesignateDig { area, mode } => Some((area, mode)),
            _ => None,
        })
        .collect();
    assert!(
        !digs.is_empty(),
        "original designation/terrain contract is retained"
    );
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    for (area, mode) in &digs {
        assert!(evidence.establishes(&Predicate::RegionTerrain {
            area: *area,
            tile_code: effects::dig_target_tile_code(*mode),
        })?);
    }
    let (completed, _) = execute(snapshot, &intent)?;
    assert_eq!(wood(&completed)?, 17);
    assert_eq!(
        completed
            .graph
            .entities
            .get(&EntityId::new(crate::lab_world::starter::FIRST_DWARF))
            .and_then(|unit| unit.fields.get("labor.MINING"))
            .and_then(|fact| laboratory_fact_value(fact, completed.tick)),
        None
    );

    // One real tile of remaining excavation restores the mining prerequisite;
    // the same unavailable labor cannot be treated as false or silently used.
    let mut remaining = original;
    remaining.set_tile_code(digs[0].0.min, tile_codes::SOLID_WALL)?;
    remaining.refresh_hash();
    assert!(
        request
            .compile(IntentId::new(1003), &remaining, "")
            .is_err()
    );
    Ok(())
}
