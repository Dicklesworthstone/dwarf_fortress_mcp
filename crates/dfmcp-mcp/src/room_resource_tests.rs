//! Original room goals across separately reviewed finite-material stages.

use super::*;
use dfmcp_adapter::CancelMode;
use dfmcp_intent::HarvestResource;

fn resource_source(wood: u64) -> Result<WorldSnapshot> {
    let mut snapshot = crate::lab_world::scenario_snapshot(
        "starter_fortress_resources",
        FortressId::new(10031),
        false,
    )?;
    let ledger = effects::stock_ledger(&snapshot)
        .ok_or_else(|| invalid("resource room fixture ledger missing"))?;
    write(
        &mut snapshot,
        ledger,
        effects::STOCK_WOOD_FIELD,
        Value::U64(wood),
    )?;
    Ok(snapshot)
}

fn write(snapshot: &mut WorldSnapshot, entity: EntityId, name: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&entity)
        .ok_or_else(|| invalid("resource room fixture entity missing"))?
        .fields
        .insert(name.to_owned(), fact(value, snapshot.tick));
    snapshot.refresh_hash();
    Ok(())
}

fn resource_room_value(rooms: u32) -> Json {
    json!({
        "template":"provisioned_room", "acquire_resources":{"assign_labor":false},
        "layout":{"template":"bedroom_cluster", "origin":[2,6,10],
            "rooms":rooms, "room_size":[3,3]},
    })
}

fn resource_room(rooms: u32) -> Result<ProvisionedRoomRequest> {
    ProvisionedRoomRequest::parse_current(&resource_room_value(rooms).to_string())
}

fn harvest_units(intent: &Intent) -> u32 {
    intent
        .requested_actions
        .iter()
        .filter_map(|step| match step.action {
            Action::HarvestResource {
                resource: HarvestResource::Wood,
                amount,
                ..
            } => Some(amount),
            _ => None,
        })
        .sum()
}

#[test]
fn resource_room_generation_is_explicit_strict_and_preserves_historical_source_bytes() -> Result<()>
{
    let old = bedrooms(2)?;
    assert!(!old.is_resource_aware());
    assert_eq!(old.planner(), PLANNER);
    assert_eq!(
        old.canonical_json(),
        r#"{"layout":{"origin":[2,6,10],"room_size":[3,3],"rooms":2,"template":"bedroom_cluster"},"planner":"provisioned_room_v1","template":"provisioned_room"}"#
    );
    let new = resource_room(2)?;
    assert!(new.is_resource_aware());
    assert_eq!(new.planner(), RESOURCE_PLANNER);
    assert_eq!(new.residual_planner(), RESOURCE_RESIDUAL_PLANNER);
    assert_eq!(ProvisionedRoomRequest::parse(&new.canonical_json())?, new);
    assert_eq!(
        serde_json::from_str::<ProvisionedRoomRequest>(&new.canonical_json())
            .map_err(|error| invalid(error.to_string()))?,
        new
    );
    assert!(ProvisionedRoomRequest::parse(&resource_room_value(2).to_string()).is_err());
    for (key, value) in [
        ("acquire_resources", Json::Null),
        ("acquire_resources", json!({"assign_labor":null})),
        ("acquire_resources", json!({"automatic_commit":true})),
        ("planner", json!(PLANNER)),
        ("planner", json!("provisioned_room_resources_v2")),
    ] {
        let mut source = resource_room_value(2);
        source[key] = value;
        assert!(
            ProvisionedRoomRequest::parse_current(&source.to_string()).is_err(),
            "{source}"
        );
    }
    let mut missing_policy: Json =
        serde_json::from_str(&new.canonical_json()).map_err(|error| invalid(error.to_string()))?;
    missing_policy
        .as_object_mut()
        .ok_or_else(|| invalid("fixture object missing"))?
        .remove("acquire_resources");
    assert!(ProvisionedRoomRequest::parse_current(&missing_policy.to_string()).is_err());
    let duplicate = new.canonical_json().replacen(
        "\"assign_labor\":false",
        "\"assign_labor\":false,\"assign_labor\":true",
        1,
    );
    assert!(ProvisionedRoomRequest::parse_current(&duplicate).is_err());
    assert!(serde_json::from_str::<ProvisionedRoomRequest>(&duplicate).is_err());
    Ok(())
}

#[test]
fn zero_wood_room_harvests_then_manufactures_excavates_and_furnishes_on_new_observation()
-> Result<()> {
    let source = resource_source(0)?;
    let original = source.clone();
    assert!(
        bedrooms(2)?
            .compile(IntentId::new(100310), &source, "")
            .is_err()
    );
    let request = resource_room(2)?;
    let (first, first_analysis) = request.compile(IntentId::new(100311), &source, "")?;
    assert_eq!(
        source, original,
        "planning cannot reserve or fabricate materials"
    );
    assert_eq!(
        classify_room_stage(&first),
        RoomCompileStage::ResourceAcquisition
    );
    assert_eq!(harvest_units(&first), 2);
    assert!(
        first
            .requested_actions
            .iter()
            .all(|step| matches!(step.action, Action::HarvestResource { .. }))
    );
    assert_eq!(first_analysis["goal_scope"], "resource_acquisition_stage");
    assert_eq!(
        first_analysis["resource_workflow"]["schema"],
        "dfmcp.resource-room/1"
    );
    assert_eq!(
        first_analysis["resource_workflow"]["stage"],
        "acquire_resources"
    );
    assert_eq!(
        first_analysis["resource_workflow"]["original_request"]["layout"]["rooms"],
        2
    );
    assert_eq!(
        first_analysis["allocation"],
        json!([]),
        "no predicted item identities before observed supply"
    );
    assert_eq!(request.semantic_goal_truth(&source)?, PredicateTruth::False);
    let (harvested, first_plan) = execute(source, &first)?;
    assert_eq!(material(&harvested)?, 2);
    assert_eq!(
        crate::lab_world::plan_goal_truth(&harvested, &first_plan)?,
        PredicateTruth::True
    );
    assert_eq!(
        request.semantic_goal_truth(&harvested)?,
        PredicateTruth::False,
        "material proof does not prove the original furnished room"
    );
    let (second, second_analysis) =
        request.compile_residual(IntentId::new(100312), &harvested, "")?;
    assert_eq!(classify_room_stage(&second), RoomCompileStage::FurnishRoom);
    assert_eq!(harvest_units(&second), 0);
    assert_eq!(second_analysis["planner"], RESOURCE_RESIDUAL_PLANNER);
    assert_eq!(second_analysis["goal_scope"], "reviewed_room_pursuit");
    assert_eq!(second_analysis["resource_workflow"]["stage"], "room");
    assert_eq!(
        second_analysis["resource_workflow"]["original_request"],
        first_analysis["resource_workflow"]["original_request"]
    );
    let (completed, _) = execute(harvested, &second)?;
    assert_eq!(material(&completed)?, 0);
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    assert_eq!(
        PredicateEvidence::laboratory(&completed)?.evaluate(&first.terminal_condition)?,
        PredicateTruth::False,
        "consumed intermediate material never becomes the semantic room goal"
    );
    Ok(())
}

#[test]
fn room_harvest_counts_shared_workshop_and_furniture_wood_once() -> Result<()> {
    let mut source = resource_source(0)?;
    source
        .graph
        .entities
        .remove(&crate::lab_world::starter::CARPENTERS);
    source.refresh_hash();
    let mut value = resource_room_value(2);
    value["prerequisites"] = json!({"assign_labor":false, "workforce":"complete_project_v1",
        "workshops":[{"building":"workshop:Carpenters", "location":[0,0,10]}]});
    let request = ProvisionedRoomRequest::parse_current(&value.to_string())?;
    let (first, analysis) = request.compile(IntentId::new(100313), &source, "")?;
    assert_eq!(harvest_units(&first), 3);
    assert_eq!(
        analysis["resource_workflow"]["requirements"][0]["workshop_units"],
        1
    );
    assert_eq!(
        analysis["resource_workflow"]["requirements"][0]["production_units"],
        2
    );
    assert_eq!(analysis["downstream_preflight"]["workshops"], true);
    assert!(
        first
            .requested_actions
            .iter()
            .all(|step| matches!(step.action, Action::HarvestResource { .. }))
    );
    let (harvested, _) = execute(source, &first)?;
    let (second, _) = request.compile_residual(IntentId::new(100314), &harvested, "")?;
    assert_eq!(
        second
            .requested_actions
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        1
    );
    let (completed, _) = execute(harvested, &second)?;
    assert_eq!(material(&completed)?, 0);
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn observed_exact_loose_furniture_needs_no_harvest_or_production() -> Result<()> {
    let mut source = produce(resource_source(20)?, "BED", 1)?;
    let ledger = effects::stock_ledger(&source).ok_or_else(|| invalid("fixture ledger missing"))?;
    write(
        &mut source,
        ledger,
        effects::STOCK_WOOD_FIELD,
        Value::U64(0),
    )?;
    let request = resource_room(1)?;
    let (intent, analysis) = request.compile(IntentId::new(100315), &source, "")?;
    assert_eq!(classify_room_stage(&intent), RoomCompileStage::FurnishRoom);
    assert!(intent.requested_actions.iter().all(|step| !matches!(
        step.action,
        Action::HarvestResource { .. } | Action::CreateWorkOrder { .. } | Action::Build { .. }
    )));
    assert_eq!(analysis["resource_workflow"]["stage"], "room");
    assert_eq!(analysis["resource_workflow"]["requirements"], json!([]));
    let (completed, _) = execute(source, &intent)?;
    assert_eq!(material(&completed)?, 0);
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn impossible_room_geometry_workshops_and_staffing_refuse_before_harvesting() -> Result<()> {
    let request = resource_room(2)?;
    let mut source = resource_source(0)?;
    source.set_tile_code(MapCoord::new(2, 6, 10), tile_codes::MAGMA_WALL)?;
    source.refresh_hash();
    assert!(request.compile(IntentId::new(100316), &source, "").is_err());
    let mut source = resource_source(0)?;
    source
        .graph
        .entities
        .remove(&crate::lab_world::starter::CARPENTERS);
    source.refresh_hash();
    assert!(request.compile(IntentId::new(100316), &source, "").is_err());
    let mut value = resource_room_value(2);
    value["prerequisites"] = json!({"assign_labor":false,
        "workshops":[{"building":"workshop:Carpenters", "location":[2,6,10]}]});
    let overlap = ProvisionedRoomRequest::parse_current(&value.to_string())?;
    assert!(overlap.compile(IntentId::new(100316), &source, "").is_err());
    let mut source = resource_source(0)?;
    for unit in source
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        unit.fields.insert(
            "labor.CONSTRUCTION".to_owned(),
            fact(Value::Bool(false), source.tick),
        );
    }
    source.refresh_hash();
    assert!(
        request.compile(IntentId::new(100316), &source, "").is_err(),
        "harvest permission cannot invent downstream construction staffing"
    );
    let mut source = resource_source(0)?;
    write(
        &mut source,
        EntityId::new(8001),
        effects::HARVEST_REMAINING_FIELD,
        Value::U64(0),
    )?;
    write(
        &mut source,
        EntityId::new(8001),
        effects::HARVEST_YIELDED_FIELD,
        Value::U64(32),
    )?;
    assert!(request.compile(IntentId::new(100316), &source, "").is_err());
    Ok(())
}

#[test]
fn harvesting_and_deferred_room_labor_keep_independent_explicit_permissions() -> Result<()> {
    let mut source = resource_source(0)?;
    for unit in source
        .graph
        .entities
        .values_mut()
        .filter(|record| record.kind == EntityKind::Unit)
    {
        for labor in ["WOODCUTTING", "MINING", "CONSTRUCTION", "CARPENTRY"] {
            unit.fields.insert(
                format!("labor.{labor}"),
                fact(Value::Bool(false), source.tick),
            );
        }
    }
    source.refresh_hash();
    let mut value = resource_room_value(2);
    value["acquire_resources"]["assign_labor"] = json!(true);
    assert!(
        ProvisionedRoomRequest::parse_current(&value.to_string())?
            .compile(IntentId::new(100317), &source, "")
            .is_err()
    );
    value["prerequisites"] = json!({"assign_labor":true, "workforce":"complete_project_v1"});
    let request = ProvisionedRoomRequest::parse_current(&value.to_string())?;
    let (first, analysis) = request.compile(IntentId::new(100317), &source, "")?;
    let labor: Vec<_> = first
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
    assert_eq!(labor, vec!["WOODCUTTING"]);
    assert!(!first.requested_actions[0].preconditions.is_empty());
    assert!(first.requested_actions[0].compensation.is_some());
    assert_eq!(analysis["deferred_setup_workforce"]["actions_added"], 2);
    let (harvested, _) = execute(source, &first)?;
    let (second, _) = request.compile_residual(IntentId::new(100318), &harvested, "")?;
    let labor: BTreeSet<_> = second
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
        labor,
        BTreeSet::from(["MINING", "CONSTRUCTION", "CARPENTRY"])
    );
    let (completed, _) = execute(harvested, &second)?;
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn partial_harvest_cancellation_reacquires_only_observed_remaining_room_demand() -> Result<()> {
    let source = resource_source(0)?;
    let request = resource_room(3)?;
    let (first, _) = request.compile(IntentId::new(100319), &source, "")?;
    let plan = StaticPlanner::default().prepare_laboratory(&source, &first, &context(&source))?;
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(receipt.actions.len(), 1);
    let action = receipt.actions[0].action_id;
    adapter.advance_ticks(effects::HARVEST_TICKS_PER_UNIT)?;
    assert_eq!(material(adapter.snapshot())?, 1);
    adapter.request_cancel(
        action,
        CancelMode::StopFutureSteps,
        &context(adapter.snapshot()),
    )?;
    adapter.finalize_cancel(action, &context(adapter.snapshot()))?;
    assert!(adapter.action_work_state(action)?.is_quiescent());
    assert_eq!(
        request.semantic_goal_truth(adapter.snapshot())?,
        PredicateTruth::False
    );
    let recovered = MemoryAdapter::recovered(adapter.snapshot().clone())?;
    let (remaining, _) =
        request.compile_residual(IntentId::new(100320), recovered.snapshot(), "")?;
    assert_eq!(harvest_units(&remaining), 2);
    let (harvested, _) = execute(recovered.snapshot().clone(), &remaining)?;
    let (room, _) = request.compile_residual(IntentId::new(100321), &harvested, "")?;
    let (completed, _) = execute(harvested, &room)?;
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    assert_eq!(material(&completed)?, 0);
    assert_ne!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True,
        "new success cannot rewrite the cancelled original stage's exact proof"
    );
    Ok(())
}

#[test]
fn semantic_room_goal_requires_every_original_terrain_region_and_reciprocal_item_custody()
-> Result<()> {
    let source = resource_source(20)?;
    let request = resource_room(2)?;
    let (intent, _) = request.compile(IntentId::new(100322), &source, "")?;
    let (completed, plan) = execute(source, &intent)?;
    assert_eq!(
        request.semantic_goal_truth(&completed)?,
        PredicateTruth::True
    );
    let mut damaged = completed.clone();
    damaged.set_tile_code(MapCoord::new(2, 6, 10), tile_codes::SOLID_WALL)?;
    damaged.refresh_hash();
    assert_eq!(
        request.semantic_goal_truth(&damaged)?,
        PredicateTruth::False
    );
    let step = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::PlaceFurniture { .. }))
        .ok_or_else(|| invalid("fixture furniture missing"))?;
    let building = effects::created_entity_id(&step.idempotency_key, 0);
    let mut aliased = completed.clone();
    aliased
        .graph
        .entities
        .get_mut(&building)
        .ok_or_else(|| invalid("fixture building missing"))?
        .kind = EntityKind::Other("building".to_owned());
    aliased.refresh_hash();
    assert!(
        request.semantic_goal_truth(&aliased).is_err(),
        "canonical aliases cannot hide occupied room slots"
    );
    let mut corrupt = completed.clone();
    write(
        &mut corrupt,
        building,
        "source_item",
        Value::Entity(EntityId::new(999999)),
    )?;
    assert!(request.semantic_goal_truth(&corrupt).is_err());
    let mut unknown = completed;
    unknown
        .graph
        .entities
        .get_mut(&building)
        .ok_or_else(|| invalid("fixture building missing"))?
        .fields
        .get_mut("construction_stage")
        .ok_or_else(|| invalid("fixture lifecycle missing"))?
        .presence = Some(FactPresence::Unknown("uncaptured".to_owned()));
    unknown.refresh_hash();
    assert!(request.semantic_goal_truth(&unknown).is_err());
    Ok(())
}
