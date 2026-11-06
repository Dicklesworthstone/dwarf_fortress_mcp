//! Complete cross-level room pursuits and retained-source compatibility.

use super::*;
use dfmcp_intent::DigMode;

fn multilevel_source() -> Result<WorldSnapshot> {
    crate::lab_world::scenario_snapshot("starter_fortress_multilevel", FortressId::new(981), false)
}

fn multilevel_value() -> Json {
    let mut value = request_value(false);
    value["planner"] = json!(STAIRS_PLANNER);
    value["room"]["layout"]["origin"] = json!([2, 6, 9]);
    value["room"]["prerequisites"] = json!({
        "assign_labor":true, "workforce":"complete_project_v1",
    });
    value["access"]["region"]["min"] = json!([0, 0, 9]);
    value
}

fn multilevel() -> Result<ConnectedRoomRequest> {
    ConnectedRoomRequest::parse_current(&multilevel_value().to_string())
}

#[test]
fn paired_stair_room_forecast_and_execution_furnish_the_lower_level() -> Result<()> {
    let snapshot = multilevel_source()?;
    let original = snapshot.clone();
    let request = multilevel()?;
    let (intent, analysis) = request.compile(IntentId::new(98_401), &snapshot, "lower bedrooms")?;
    assert_eq!(snapshot, original);
    assert_eq!(analysis["planner"], STAIRS_PLANNER);
    assert_eq!(analysis["access"]["new_stair_tiles"], 2);
    assert_eq!(analysis["access"]["vertical_edges"], 1);
    let path = route(&analysis)?;
    assert_eq!(path.first(), Some(&request.entry));
    assert_eq!(path.last().map(|at| at.z), Some(9));
    let stair_steps: Vec<_> = intent
        .requested_actions
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            matches!(
                step.action,
                Action::DesignateDig {
                    mode: DigMode::UpDownStair,
                    ..
                }
            )
            .then_some(index as u32)
        })
        .collect();
    assert_eq!(stair_steps.len(), 2);
    for step in &intent.requested_actions {
        if matches!(step.action, Action::PlaceFurniture { .. }) {
            assert!(
                stair_steps
                    .iter()
                    .all(|index| step.depends_on.contains(index))
            );
        }
    }
    let sealed =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let adapter = MemoryAdapter::new(snapshot.clone());
    let forecast =
        super::super::super::plan_forecast::forecast_plan(&adapter, &sealed, &context(&snapshot));
    assert_eq!(forecast["predicted_goal_complete"], true, "{forecast}");
    assert_eq!(adapter.snapshot(), &original);
    let cadence = forecast["resolution_ticks"]
        .as_u64()
        .filter(|ticks| *ticks > 0)
        .ok_or_else(|| invalid("missing forecast cadence"))?;
    let (completed, plan) = execute_with_cadence(snapshot, &intent, cadence)?;
    assert_eq!(sealed, plan);
    assert_eq!(forecast["predicted_completion_tick"], completed.tick.0);
    let (_, targets) = request.room.access_geometry()?;
    assert_eq!(
        dfmcp_world::laboratory_room_access_paired_stairs_truth(
            &completed,
            request.area,
            request.entry,
            &targets,
        )?,
        PredicateTruth::True
    );
    for pair in path.windows(2).filter(|pair| pair[0].z != pair[1].z) {
        for at in pair {
            assert_eq!(completed.tile_code_at(*at), Some(tile_codes::STAIR));
        }
    }
    assert_eq!(wood(&completed)?, wood(&original)? - 2);
    Ok(())
}

#[test]
fn existing_paired_stairs_are_reused_and_room_floor_masks_stay_floor() -> Result<()> {
    let request = multilevel()?;
    let mut snapshot = multilevel_source()?;
    let (_, initial) = request.compile(IntentId::new(98_402), &snapshot, "")?;
    let path = route(&initial)?;
    for pair in path.windows(2).filter(|pair| pair[0].z != pair[1].z) {
        for at in pair {
            snapshot.set_tile_code(*at, tile_codes::STAIR)?;
        }
    }
    snapshot.refresh_hash();
    let (intent, analysis) = request.compile(IntentId::new(98_403), &snapshot, "reuse shaft")?;
    assert_eq!(analysis["access"]["new_stair_tiles"], 0);
    assert!(!intent.requested_actions.iter().any(|step| matches!(
        step.action,
        Action::DesignateDig {
            mode: DigMode::UpDownStair,
            ..
        }
    )));
    let (completed, _) = execute(snapshot, &intent)?;
    let (layout, _) = request.room.access_geometry()?;
    for at in layout
        .excavations()
        .iter()
        .flat_map(|part| dfmcp_world::terrain::region_tiles(part.area))
    {
        assert_eq!(completed.tile_code_at(at), Some(tile_codes::FLOOR));
    }
    Ok(())
}

#[test]
fn broken_stair_repair_retains_furniture_and_does_not_spend_more_wood() -> Result<()> {
    let request = multilevel()?;
    let snapshot = multilevel_source()?;
    let (intent, analysis) = request.compile(IntentId::new(98_404), &snapshot, "")?;
    let (completed, original) = execute(snapshot, &intent)?;
    let installations: Vec<_> = original
        .steps
        .iter()
        .filter_map(|step| match step.action {
            Action::PlaceFurniture { item, .. } => Some((
                completed.graph.entities.get(&item).cloned(),
                completed
                    .graph
                    .entities
                    .get(&effects::created_entity_id(&step.idempotency_key, 0))
                    .cloned(),
            )),
            _ => None,
        })
        .collect();
    let shaft = route(&analysis)?
        .windows(2)
        .find(|pair| pair[0].z != pair[1].z)
        .map(|pair| pair[1])
        .ok_or_else(|| invalid("test plan omitted a shaft"))?;
    let mut interrupted = completed.clone();
    interrupted.set_tile_code(shaft, tile_codes::SOLID_WALL)?;
    interrupted.refresh_hash();
    let recovered = MemoryAdapter::recovered(interrupted)?.snapshot().clone();
    assert_eq!(
        crate::lab_world::plan_goal_truth(&recovered, &original)?,
        PredicateTruth::False
    );
    let (repair, analysis) =
        request.compile_residual(IntentId::new(98_405), &recovered, "repair shaft")?;
    assert_eq!(analysis["planner"], STAIRS_RESIDUAL_PLANNER);
    assert_eq!(analysis["retained_placements"], 2);
    assert_eq!(analysis["new_placements"], 0);
    assert_eq!(analysis["access"]["new_stair_tiles"], 1);
    assert_eq!(analysis["access"]["new_corridor_tiles"], 0);
    let mut stair_repairs = 0;
    for step in &repair.requested_actions {
        match step.action {
            Action::DesignateDig {
                area,
                mode: DigMode::UpDownStair,
            } => {
                stair_repairs += 1;
                assert_eq!(area, MapCuboid::new(shaft, shaft)?);
            }
            Action::DesignateDig {
                area,
                mode: DigMode::Mine,
            } => {
                // The retained room compiler keeps doorway/corridor
                // designations that contain no installed furnishing. They
                // require no physical work here and cannot hide a new route.
                assert!(
                    dfmcp_world::terrain::region_tiles(area)
                        .all(|at| recovered.tile_code_at(at) == Some(tile_codes::FLOOR))
                );
            }
            _ => return Err(invalid("shaft repair introduced non-excavation work")),
        }
    }
    assert_eq!(stair_repairs, 1);
    let (repaired, _) = execute(recovered, &repair)?;
    assert_eq!(wood(&repaired)?, wood(&completed)?);
    assert_eq!(
        crate::lab_world::plan_goal_truth(&repaired, &original)?,
        PredicateTruth::True
    );
    for (item, building) in installations {
        for record in [item, building] {
            let record = record.ok_or_else(|| invalid("original installation missing"))?;
            assert_eq!(repaired.graph.entities.get(&record.id), Some(&record));
        }
    }
    Ok(())
}

#[test]
fn paired_stair_source_requires_explicit_generation_and_complete_hazard_coverage() -> Result<()> {
    let mut value = multilevel_value();
    value
        .as_object_mut()
        .ok_or_else(|| invalid("test request missing"))?
        .remove("planner");
    assert!(ConnectedRoomRequest::parse_current(&value.to_string()).is_err());
    value["planner"] = json!(PLANNER);
    assert!(ConnectedRoomRequest::parse_current(&value.to_string()).is_err());
    let request = multilevel()?;
    let retained = request.canonical_json();
    assert_eq!(ConnectedRoomRequest::parse(&retained)?, request);
    assert_eq!(
        ConnectedRoomRequest::parse(&retained)?.canonical_json(),
        retained
    );
    assert!(
        request
            .compile(IntentId::new(98_406), &source()?, "")
            .is_err()
    );
    let mut snapshot = multilevel_source()?;
    snapshot.set_tile_code(request.entry, tile_codes::SOLID_WALL)?;
    snapshot.refresh_hash();
    assert!(
        request
            .compile(IntentId::new(98_407), &snapshot, "")
            .is_err()
    );
    Ok(())
}

#[test]
fn paired_stair_continuation_binds_original_generation_and_flat_lineage() -> Result<()> {
    use super::super::super::connected_room_continuation::ConnectedRoomContinuation;
    let root = Digest32::of_bytes(b"paired stair original");
    let source = ConnectedRoomContinuation::new(root, root, multilevel()?)?;
    let raw = source.canonical_json();
    let mut value: Json = serde_json::from_str(&raw).map_err(|error| invalid(error.to_string()))?;
    assert_eq!(value["planner"], STAIRS_RESIDUAL_PLANNER);
    assert_eq!(value["connected_room"]["planner"], STAIRS_PLANNER);
    assert_eq!(ConnectedRoomContinuation::parse(&raw)?, source);
    value["planner"] = json!(RESIDUAL_PLANNER);
    assert!(ConnectedRoomContinuation::parse(&value.to_string()).is_err());
    let next = source.next(Digest32::of_bytes(b"paired stair parent"))?;
    assert_eq!(next.root, root);
    assert_eq!(next.canonical_json().len(), raw.len());
    assert_eq!(next.lineage_json()["access_requirement_preserved"], true);
    Ok(())
}

#[test]
fn multilevel_scenario_preserves_every_existing_provisions_fact() -> Result<()> {
    let old = crate::lab_world::scenario_snapshot(
        "starter_fortress_provisions",
        FortressId::new(981),
        false,
    )?;
    let mut new = multilevel_source()?;
    new.graph
        .chunks
        .retain(|coord, _| (9..=11).contains(&coord.z));
    new.refresh_hash();
    assert_eq!(new, old);
    Ok(())
}

#[test]
fn occupancy_projection_clips_each_building_to_its_actual_levels() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(0, 0, 9), MapCoord::new(2, 2, 11))?;
    let footprint = MapCuboid::new(MapCoord::new(0, 0, 10), MapCoord::new(1, 1, 10))?;
    let mut occupied = BTreeSet::new();
    project_occupancy(area, footprint, &mut occupied, &mut Work::default())?;
    assert_eq!(occupied.len(), 4);
    assert!(occupied.iter().all(|at| at.z == 10));
    Ok(())
}

#[test]
fn vertical_route_edges_require_both_eligible_endpoints() -> Result<()> {
    let lower = MapCoord::new(i32::MAX, 0, i32::MIN);
    let upper = MapCoord::new(i32::MAX, 0, i32::MIN + 1);
    let area = MapCuboid::new(lower, upper)?;
    for eligible in [
        BTreeSet::new(),
        BTreeSet::from([lower]),
        BTreeSet::from([upper]),
    ] {
        assert!(super::super::stairs::adjacent(area, lower, &eligible).is_empty());
    }
    let both = BTreeSet::from([lower, upper]);
    assert_eq!(
        super::super::stairs::adjacent(area, lower, &both),
        vec![upper]
    );
    assert_eq!(
        super::super::stairs::adjacent(area, upper, &both),
        vec![lower]
    );
    Ok(())
}
