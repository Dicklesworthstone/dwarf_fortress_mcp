use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{BuildingKind, MaterialSelector, PreparedPlan, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{PredicateEvidence, PredicateTruth, Value, laboratory_fact_value};

#[path = "connected_room_workforce_tests.rs"]
mod workforce;

fn source() -> Result<WorldSnapshot> {
    crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(981), false)
}

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
            (Capability::Designate, RiskTier::Guarded),
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

fn room_value(dining: bool) -> Json {
    if dining {
        json!({"template":"provisioned_room","layout":{
            "template":"dining_hall","origin":[2,6,10],"width":7,"height":15,
        }})
    } else {
        json!({"template":"provisioned_room","layout":{
            "template":"bedroom_cluster","origin":[2,6,10],"rooms":2,"room_size":[3,3],
        }})
    }
}

fn request_value(dining: bool) -> Json {
    json!({"template":"connected_room","room":room_value(dining),"access":{
        "entry":[0,2,10], "region":{"min":[0,0,10],"max":[12,if dining {22} else {12},10]},
    }})
}

fn request(dining: bool) -> Result<ConnectedRoomRequest> {
    ConnectedRoomRequest::parse_current(&request_value(dining).to_string())
}

fn execute(snapshot: WorldSnapshot, intent: &Intent) -> Result<(WorldSnapshot, PreparedPlan)> {
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, intent, &context(&snapshot))?;
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|work| work.deadline_tick.0))
        .max()
        .unwrap_or(snapshot.tick.0);
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let mut states: Vec<_> = receipt.actions.iter().map(|action| action.state).collect();
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
    assert_eq!(
        crate::lab_world::plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::True
    );
    Ok((adapter.snapshot().clone(), plan))
}

fn wood(snapshot: &WorldSnapshot) -> Result<u64> {
    let ledger =
        effects::stock_ledger(snapshot).ok_or_else(|| invalid("fixture ledger missing"))?;
    match snapshot
        .graph
        .entities
        .get(&ledger)
        .and_then(|record| record.fields.get(effects::STOCK_WOOD_FIELD))
        .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
    {
        Some(Value::U64(count)) => Ok(*count),
        _ => Err(invalid("fixture wood missing")),
    }
}

fn route(analysis: &Json) -> Result<Vec<MapCoord>> {
    analysis["access"]["route"]
        .as_array()
        .ok_or_else(|| invalid("route missing"))?
        .iter()
        .map(|value| {
            let value: [i32; 3] =
                serde_json::from_value(value.clone()).map_err(|_| invalid("bad route point"))?;
            Ok(coordinate(value))
        })
        .collect()
}

#[test]
fn connected_bedrooms_forecast_and_execute_complete_access_and_item_goals() -> Result<()> {
    let request = request(false)?;
    let snapshot = source()?;
    let before = snapshot.clone();
    let (intent, analysis) = request.compile(IntentId::new(981), &snapshot, "")?;
    assert_eq!(snapshot, before);
    assert_eq!(analysis["access"]["new_corridor_tiles"], 7);
    assert_eq!(analysis["access"]["corridor_parts"], 1);
    assert_eq!(route(&analysis)?.first(), Some(&request.entry));
    assert_eq!(route(&analysis)?.last(), Some(&MapCoord::new(0, 10, 10)));
    let dig_steps: Vec<_> = intent
        .requested_actions
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            matches!(step.action, Action::DesignateDig { .. }).then_some(index as u32)
        })
        .collect();
    for step in &intent.requested_actions {
        if matches!(step.action, Action::PlaceFurniture { .. }) {
            assert!(
                dig_steps
                    .iter()
                    .all(|dependency| step.depends_on.contains(dependency))
            );
        }
    }
    let sealed =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let adapter = MemoryAdapter::new(snapshot.clone());
    let forecast =
        super::super::plan_forecast::forecast_plan(&adapter, &sealed, &context(&snapshot));
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    assert_eq!(forecast["predicted_goal_complete"], true, "{forecast}");
    assert_eq!(adapter.snapshot(), &before);
    let (completed, plan) = execute(snapshot, &intent)?;
    assert_eq!(plan, sealed);
    assert_eq!(forecast["predicted_completion_tick"], completed.tick.0);
    assert_eq!(wood(&completed)?, 18);
    let (layout, targets) = request.room.access_geometry()?;
    let planned: BTreeSet<_> = layout
        .excavations()
        .iter()
        .flat_map(|part| dfmcp_world::terrain::region_tiles(part.area))
        .collect();
    for at in protected_room_walls(&layout, &planned)? {
        assert_eq!(completed.tile_code_at(at), before.tile_code_at(at));
    }
    assert_eq!(
        dfmcp_world::laboratory_room_access_truth(
            &completed,
            request.area,
            request.entry,
            &targets
        )?,
        PredicateTruth::True
    );

    // Floor and furniture remain intact, but a new building blocks the sole
    // observed corridor. Access must invalidate the same original goal.
    let mut blocked = completed.clone();
    let at = MapCoord::new(0, 3, 10);
    effects::apply_effect(
        &mut blocked,
        &Action::Build {
            kind: BuildingKind::Furniture("Door".to_owned()),
            location: at,
            footprint: MapCuboid::new(at, at)?,
            material: MaterialSelector::default(),
        },
        "block-completed-connected-room",
    )?;
    blocked.refresh_hash();
    assert_eq!(blocked.tile_code_at(at), Some(tile_codes::FLOOR));
    assert_eq!(
        PredicateEvidence::laboratory(&blocked)?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::False
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(&blocked, &plan)?,
        PredicateTruth::False
    );
    assert_eq!(
        dfmcp_world::laboratory_room_access_truth(&blocked, request.area, request.entry, &targets)?,
        PredicateTruth::False
    );
    Ok(())
}

#[test]
fn connected_large_dining_residual_repairs_access_without_replacing_furnishings() -> Result<()> {
    let request = request(true)?;
    let snapshot = source()?;
    let (intent, _) = request.compile(IntentId::new(982), &snapshot, "")?;
    let sealed =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let hall = sealed
        .steps
        .iter()
        .find(|step| {
            matches!(step.action,
        Action::DesignateDig { area, .. } if area.tile_count() == Some(105))
        })
        .ok_or_else(|| invalid("large hall dig missing"))?;
    assert_eq!(
        hall.obligation
            .as_ref()
            .map(|work| work.deadline_tick.0 - snapshot.tick.0),
        Some(2200)
    );
    let (completed, original) = execute(snapshot, &intent)?;
    let original_items: Vec<_> = original
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
    let mut interrupted = completed.clone();
    for x in request.area.min.x..=request.area.max.x {
        interrupted.set_tile_code(MapCoord::new(x, 3, 10), tile_codes::SOLID_WALL)?;
    }
    interrupted.refresh_hash();
    let recovered = MemoryAdapter::recovered(interrupted)?.snapshot().clone();
    assert_eq!(
        crate::lab_world::plan_goal_truth(&recovered, &original)?,
        PredicateTruth::False
    );
    let (residual, analysis) =
        request.compile_residual(IntentId::new(983), &recovered, "restore access")?;
    assert_eq!(analysis["planner"], RESIDUAL_PLANNER);
    assert_eq!(analysis["retained_placements"], 2);
    assert_eq!(analysis["new_placements"], 0);
    assert_eq!(analysis["access"]["new_corridor_tiles"], 1);
    assert!(
        residual
            .requested_actions
            .iter()
            .all(|step| matches!(step.action, Action::DesignateDig { .. }))
    );
    let (repaired, _) = execute(recovered, &residual)?;
    assert_eq!(wood(&repaired)?, wood(&completed)?);
    for (item, building) in original_items {
        for record in [item, building] {
            let record = record.ok_or_else(|| invalid("original installation missing"))?;
            assert_eq!(repaired.graph.entities.get(&record.id), Some(&record));
        }
    }
    Ok(())
}

#[test]
fn connected_source_replay_is_closed_and_existing_room_compilers_are_unchanged() -> Result<()> {
    let request = request(false)?;
    let snapshot = source()?;
    let plain_before = request
        .room
        .compile(IntentId::new(984), &snapshot, "legacy request")?;
    let residual_before =
        request
            .room
            .compile_residual(IntentId::new(984), &snapshot, "legacy request")?;
    let canonical = request.canonical_json();
    let restored = ConnectedRoomRequest::parse(&canonical)?;
    assert_eq!(restored, request);
    assert_eq!(restored.canonical_json(), canonical);
    assert_eq!(
        request.compile(IntentId::new(984), &snapshot, "review")?,
        restored.compile(IntentId::new(984), &snapshot, "review")?
    );
    assert_eq!(
        request.compile_residual(IntentId::new(984), &snapshot, "review")?,
        restored.compile_residual(IntentId::new(984), &snapshot, "review")?
    );
    assert_eq!(
        plain_before,
        request
            .room
            .compile(IntentId::new(984), &snapshot, "legacy request")?
    );
    assert_eq!(
        residual_before,
        request
            .room
            .compile_residual(IntentId::new(984), &snapshot, "legacy request")?
    );
    let source: Json =
        serde_json::from_str(&canonical).map_err(|_| invalid("canonical source missing"))?;
    for mutation in 0..7 {
        let mut changed = source.clone();
        match mutation {
            0 => {
                changed
                    .as_object_mut()
                    .ok_or_else(|| invalid("source object missing"))?
                    .remove("planner");
            }
            1 => {
                changed["room"]
                    .as_object_mut()
                    .ok_or_else(|| invalid("room object missing"))?
                    .remove("planner");
            }
            2 => changed["planner"] = json!(RESIDUAL_PLANNER),
            3 => changed["room"]["planner"] = json!("provisioned_room_residual_v1"),
            4 => changed["access"]["allow_unknown"] = json!(true),
            5 => changed["access"]["region"]["max"] = json!([100, 100, 10]),
            _ => changed["access"]["region"]["max"] = json!([12, 12, 11]),
        }
        assert!(
            ConnectedRoomRequest::parse(&changed.to_string()).is_err(),
            "mutation {mutation}"
        );
    }
    let mut changed = source.clone();
    changed["access"]["region"]["max"] = json!([13, 12, 10]);
    let changed = ConnectedRoomRequest::parse(&changed.to_string())?;
    let (first, _) = request.compile(IntentId::new(984), &snapshot, "review")?;
    let (second, _) = changed.compile(IntentId::new(984), &snapshot, "review")?;
    assert_ne!(
        first.summary, second.summary,
        "unused routing choices still change the sealed source"
    );
    assert_ne!(first.terminal_condition, second.terminal_condition);
    let (residual, _) = request.compile_residual(IntentId::new(984), &snapshot, "review")?;
    assert_ne!(first.summary, residual.summary);
    assert!(is_connected_room(&canonical));
    assert!(!is_connected_room(&request.room.canonical_json()));
    Ok(())
}

#[test]
fn connected_routes_reuse_open_floor_and_avoid_actual_planned_workshops() -> Result<()> {
    let request = request(false)?;
    let mut snapshot = source()?;
    let (_, analysis) = request.compile(IntentId::new(985), &snapshot, "")?;
    for at in route(&analysis)? {
        snapshot.set_tile_code(at, tile_codes::FLOOR)?;
    }
    snapshot.refresh_hash();
    let (_, open) = request.compile(IntentId::new(985), &snapshot, "")?;
    assert_eq!(open["access"]["new_corridor_tiles"], 0);
    assert_eq!(open["access"]["corridor_parts"], 0);

    let carpenter = snapshot
        .graph
        .entities
        .values()
        .find(|record| record.label == "workshop:Carpenters")
        .map(|record| record.id)
        .ok_or_else(|| invalid("starter carpenter missing"))?;
    snapshot.graph.entities.remove(&carpenter);
    snapshot.refresh_hash();
    let mut input = request_value(false);
    input["access"]["region"]["min"] = json!([-1, 0, 10]);
    input["room"]["prerequisites"] = json!({"workshops":[{
        "building":"workshop:Carpenters","location":[0,4,10],
    }]});
    let setup = ConnectedRoomRequest::parse_current(&input.to_string())?;
    let (intent, analysis) = setup.compile(IntentId::new(986), &snapshot, "")?;
    assert_eq!(analysis["access"]["planned_workshops_avoided"], 1);
    assert!(!route(&analysis)?.contains(&MapCoord::new(0, 4, 10)));
    assert!(
        intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action, Action::Build { .. }))
    );
    let (completed, _) = execute(snapshot, &intent)?;
    assert_eq!(wood(&completed)?, 18);
    Ok(())
}

#[test]
fn connected_room_refuses_unobserved_blocked_and_unusable_access_without_mutation() -> Result<()> {
    let request = request(false)?;
    for mutation in 0..5 {
        let mut snapshot = source()?;
        match mutation {
            0 => {
                snapshot.set_tile_code(request.entry, tile_codes::SOLID_WALL)?;
            }
            1 => {
                for x in request.area.min.x..=request.area.max.x {
                    snapshot.set_tile_code(MapCoord::new(x, 3, 10), tile_codes::OPEN_SPACE)?;
                }
            }
            2 => {
                let at = request.entry;
                effects::apply_effect(
                    &mut snapshot,
                    &Action::Build {
                        kind: BuildingKind::Furniture("Door".to_owned()),
                        location: at,
                        footprint: MapCuboid::new(at, at)?,
                        material: MaterialSelector::default(),
                    },
                    "occupy-connected-room-entry",
                )?;
            }
            3 => {
                let chunk = snapshot
                    .graph
                    .chunks
                    .values_mut()
                    .next()
                    .ok_or_else(|| invalid("fixture terrain missing"))?;
                chunk.terrain_runs.insert(
                    0,
                    dfmcp_world::TerrainRun {
                        tile_code: tile_codes::FLOOR,
                        length: 0,
                    },
                );
            }
            _ => {
                let slot = dfmcp_world::terrain::tile_slot(request.entry);
                snapshot.graph.chunks.remove(&slot.chunk);
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            request.compile(IntentId::new(987), &snapshot, "").is_err(),
            "mutation {mutation}"
        );
        assert_eq!(snapshot, before);
    }
    let mut narrow = request_value(true);
    narrow["room"]["layout"]["width"] = json!(1);
    narrow["room"]["layout"]["height"] = json!(2);
    let narrow = ConnectedRoomRequest::parse_current(&narrow.to_string())?;
    assert!(
        narrow.compile(IntentId::new(987), &source()?, "").is_err(),
        "furniture consumes every perimeter approach"
    );
    Ok(())
}

#[test]
fn connected_room_protects_aliased_room_occupancy_and_routes_around_unrelated_aliases() -> Result<()>
{
    let request = request(false)?;
    let (_, targets) = request.room.access_geometry()?;
    for (overlaps_requested_work, setup, location) in [
        (true, false, targets[0]),
        (false, false, MapCoord::new(0, 3, 10)),
        (true, true, MapCoord::new(0, 3, 10)),
    ] {
        let mut snapshot = source()?;
        let request = if setup {
            let carpenter = snapshot
                .graph
                .entities
                .values()
                .find(|record| record.label == "workshop:Carpenters")
                .map(|record| record.id)
                .ok_or_else(|| invalid("starter carpenter missing"))?;
            snapshot.graph.entities.remove(&carpenter);
            let mut input = request_value(false);
            input["room"]["prerequisites"] = json!({"workshops":[{
                "building":"workshop:Carpenters", "location":position(location),
            }]});
            ConnectedRoomRequest::parse_current(&input.to_string())?
        } else {
            request.clone()
        };
        snapshot.set_tile_code(location, tile_codes::FLOOR)?;
        let key = "aliased-connected-room-obstacle";
        effects::apply_effect(
            &mut snapshot,
            &Action::Build {
                kind: BuildingKind::Furniture("Door".to_owned()),
                location,
                footprint: MapCuboid::new(location, location)?,
                material: MaterialSelector::default(),
            },
            key,
        )?;
        let building = effects::created_entity_id(key, 0);
        snapshot
            .graph
            .entities
            .get_mut(&building)
            .ok_or_else(|| invalid("aliased occupancy fixture missing"))?
            .kind = EntityKind::Other("building".to_owned());
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert_eq!(
            laboratory_building_footprints(&snapshot)?.get(&building),
            Some(&MapCuboid::new(location, location)?)
        );
        for residual in [false, true] {
            let compiled = request.compile_inner(IntentId::new(988), &snapshot, "", residual);
            if overlaps_requested_work {
                assert!(
                    compiled.is_err_and(|error| error.code == ErrorCode::PreconditionsFailed
                        && error.message.contains("aliased building")),
                    "neither room compiler may excavate or build over an aliased building"
                );
            } else {
                let (_, analysis) = compiled?;
                assert!(
                    !route(&analysis)?.contains(&location),
                    "unrelated aliases remain qualified routing obstacles"
                );
            }
            assert_eq!(snapshot, before);
        }
    }
    Ok(())
}

#[test]
fn minimum_mining_routes_match_exhaustive_simple_paths_and_exact_mask_partitions() -> Result<()> {
    let area = MapCuboid::new(MapCoord::new(0, 0, 0), MapCoord::new(2, 2, 0))?;
    let cells: Vec<_> = dfmcp_world::terrain::region_tiles(area).collect();
    let entry = cells[0];
    let endpoint = cells[8];
    for configuration in 0..3_u32.pow(7) {
        let mut encoded = configuration;
        let mut costs = BTreeMap::from([(entry, 0_u32), (endpoint, 0_u32)]);
        for at in &cells[1..8] {
            let value = encoded % 3;
            encoded /= 3;
            if value != 2 {
                costs.insert(*at, value);
            }
        }
        // Independent exhaustive simple-path oracle on nine cells. It does not
        // use Dijkstra distances, the production neighbor helper or its queue.
        let mut pending = vec![vec![entry]];
        type OraclePath = (u32, usize, Vec<(i32, i32, i32)>);
        let mut best: Option<OraclePath> = None;
        while let Some(path) = pending.pop() {
            let at = *path.last().ok_or_else(|| invalid("oracle path missing"))?;
            if at == endpoint {
                let cost = path
                    .iter()
                    .skip(1)
                    .map(|at| costs.get(at).copied().unwrap_or(0))
                    .sum();
                let candidate = (
                    cost,
                    path.len() - 1,
                    path.iter().map(|at| (at.z, at.y, at.x)).collect(),
                );
                if best.as_ref().is_none_or(|prior| candidate < *prior) {
                    best = Some(candidate);
                }
                continue;
            }
            for next in &cells {
                let distance = i64::from(next.x).abs_diff(i64::from(at.x))
                    + i64::from(next.y).abs_diff(i64::from(at.y));
                if distance == 1 && costs.contains_key(next) && !path.contains(next) {
                    let mut branch = path.clone();
                    branch.push(*next);
                    pending.push(branch);
                }
            }
        }
        let actual = minimum_path(area, entry, endpoint, &costs, &mut Work::default());
        match best {
            None => assert!(actual.is_err(), "configuration {configuration}"),
            Some((cost, length, path)) => {
                let actual = actual?;
                assert_eq!(
                    actual
                        .iter()
                        .map(|at| (at.z, at.y, at.x))
                        .collect::<Vec<_>>(),
                    path,
                    "configuration {configuration}"
                );
                assert_eq!(actual.len() - 1, length);
                assert_eq!(
                    actual
                        .iter()
                        .skip(1)
                        .map(|at| costs.get(at).copied().unwrap_or(0))
                        .sum::<u32>(),
                    cost
                );
            }
        }
    }
    for bits in 0..512_u32 {
        let mask: BTreeSet<_> = cells
            .iter()
            .enumerate()
            .filter_map(|(i, at)| (bits & (1 << i) != 0).then_some(*at))
            .collect();
        let parts = partition_mask(&mask)?;
        let mut expanded = BTreeSet::new();
        for part in parts {
            for at in dfmcp_world::terrain::region_tiles(part) {
                assert!(expanded.insert(at), "partition parts overlap");
            }
        }
        assert_eq!(
            expanded, mask,
            "partition {bits} must neither omit nor overdig cells"
        );
    }
    Ok(())
}
