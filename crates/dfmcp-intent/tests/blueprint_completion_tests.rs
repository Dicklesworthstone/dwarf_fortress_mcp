#![forbid(unsafe_code)]

//! Complete ordinary furnishing goals and workload-sensitive sealed deadlines.
//! Owning bead: df-lab-shared-workforce-8n4.

use std::collections::BTreeMap;
use std::error::Error;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
    IntentId, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, RiskTier,
    SessionId, WorkBudget,
};
use dfmcp_intent::{
    Action, BlueprintPlanner, BlueprintTemplate, BuildingKind, Intent, MaterialSelector,
    PreparedPlan, StaticPlanner, effects, workforce_planning::LaboratoryWorkforcePlanning,
};
use dfmcp_world::{
    ChunkCoord, EntityKind, EntityRecord, Fact, FactSource, Predicate, PredicateEvidence,
    PredicateTruth, Value, WorldGraph, WorldSnapshot, tile_codes,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

fn source(shared: bool) -> TestResult<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    for z in 9..=11 {
        for y in -1..=1 {
            for x in -1..=1 {
                let coord = ChunkCoord { x, y, z };
                graph.chunks.insert(
                    coord,
                    dfmcp_world::terrain::uniform_chunk(coord, tile_codes::SOLID_WALL),
                );
            }
        }
    }
    let mut world = WorldSnapshot::new(
        FortressId::new(45_901),
        GameTick(100),
        ObservationCursor::ORIGIN,
        false,
        graph,
    );
    if shared {
        put(
            &mut world,
            EntityId::new(101),
            EntityKind::Unit,
            [
                ("alive", Value::Bool(true)),
                ("labor.MINING", Value::Bool(true)),
                ("labor.CONSTRUCTION", Value::Bool(true)),
                ("labor.BREW", Value::Bool(false)),
                ("labor.COOK", Value::Bool(false)),
                ("labor.CARPENTRY", Value::Bool(false)),
            ],
        );
        let settings = effects::fortress_settings_entity_id(world.fortress_id);
        put(
            &mut world,
            settings,
            EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
            [(
                effects::LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
            )],
        );
    }
    world.refresh_hash();
    Ok(world)
}

fn put<const N: usize>(
    world: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: [(&str, Value); N],
) {
    world.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: "blueprint fixture".to_owned(),
            fields: fields
                .into_iter()
                .map(|(name, value)| {
                    (
                        name.to_owned(),
                        Fact::known(
                            value,
                            world.tick,
                            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                            Digest32::ZERO,
                        ),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        },
    );
}

fn index(world: &WorldSnapshot) -> TestResult<dfmcp_world::ChunkSpatialIndex> {
    let mut index = dfmcp_world::ChunkSpatialIndex::new();
    for chunk in world.graph.chunks.values() {
        index.insert_or_update_chunk(chunk)?;
    }
    Ok(index)
}

fn intent(world: &WorldSnapshot, template: BlueprintTemplate) -> TestResult<Intent> {
    Ok(BlueprintPlanner.compile_furnished_blueprint_intent(
        IntentId::new(501),
        world.anchor(),
        MapCoord::new(2, 6, 10),
        template,
        &index(world)?,
    )?)
}

fn prepare(world: &WorldSnapshot, intent: &Intent) -> TestResult<PreparedPlan> {
    Ok(StaticPlanner::default().prepare_laboratory(
        world,
        intent,
        &OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: world.anchor(),
            budget: WorkBudget::default(),
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        },
    )?)
}

fn advance(world: &mut WorldSnapshot, ticks: u64) -> TestResult {
    world.tick = world.tick.checked_add(ticks).ok_or("tick overflow")?;
    effects::advance_effects(world, ticks)?;
    world.cursor = world.cursor.checked_next().ok_or("cursor overflow")?;
    world.refresh_hash();
    Ok(())
}

fn dispatch(world: &mut WorldSnapshot, action: &Action, key: &str) -> TestResult {
    effects::apply_effect(world, action, key)?;
    world.cursor = world.cursor.checked_next().ok_or("cursor overflow")?;
    world.refresh_hash();
    Ok(())
}

fn established(world: &WorldSnapshot, predicate: &Predicate) -> TestResult<bool> {
    Ok(PredicateEvidence::laboratory(world)?.establishes(predicate)?)
}

#[test]
fn furnished_goal_requires_every_exact_completed_furnishing_after_excavation() -> TestResult {
    let mut world = source(false)?;
    let intent = intent(
        &world,
        BlueprintTemplate::DiningHall {
            width: 3,
            height: 3,
        },
    )?;
    let plan = prepare(&world, &intent)?;
    let excavations: Vec<_> = plan
        .steps
        .iter()
        .filter(|step| matches!(step.action, Action::DesignateDig { .. }))
        .collect();
    let furnishings: Vec<_> = plan
        .steps
        .iter()
        .filter(|step| matches!(step.action, Action::Build { .. }))
        .collect();
    assert_eq!(furnishings.len(), 2);
    for step in &excavations {
        dispatch(&mut world, &step.action, &step.idempotency_key)?;
    }
    advance(&mut world, 100)?;
    for step in excavations {
        assert!(established(&world, &step.postconditions[0])?);
    }
    assert!(!established(&world, &plan.terminal_condition)?);

    // A submitted building and one completed furnishing are each insufficient.
    dispatch(
        &mut world,
        &furnishings[0].action,
        &furnishings[0].idempotency_key,
    )?;
    assert!(!established(&world, &plan.terminal_condition)?);
    advance(&mut world, effects::BUILD_TICKS)?;
    assert!(established(&world, &furnishings[0].postconditions[0])?);
    assert!(!established(&world, &plan.terminal_condition)?);
    dispatch(
        &mut world,
        &furnishings[1].action,
        &furnishings[1].idempotency_key,
    )?;
    advance(&mut world, effects::BUILD_TICKS)?;
    assert!(established(&world, &plan.terminal_condition)?);

    let complete = world.clone();
    let first_id = effects::created_entity_id(&furnishings[0].idempotency_key, 0);
    for (field, value) in [
        ("building_kind", Value::Text("furniture:Bed".to_owned())),
        ("position", Value::Coord(MapCoord::new(0, 0, 10))),
        ("footprint_min", Value::Coord(MapCoord::new(0, 0, 10))),
        ("footprint_max", Value::Coord(MapCoord::new(0, 0, 10))),
    ] {
        let mut changed = complete.clone();
        let fact = changed
            .graph
            .entities
            .get_mut(&first_id)
            .and_then(|entity| entity.fields.get_mut(field))
            .ok_or("furnishing field missing")?;
        fact.value = value;
        fact.presence = None;
        changed.refresh_hash();
        assert!(!established(&changed, &plan.terminal_condition)?, "{field}");
    }
    let mut wrong_kind = complete.clone();
    wrong_kind
        .graph
        .entities
        .get_mut(&first_id)
        .ok_or("furnishing missing")?
        .kind = EntityKind::Item;
    wrong_kind.refresh_hash();
    assert!(!established(&wrong_kind, &plan.terminal_condition)?);
    let mut absent = complete;
    absent.graph.entities.remove(&first_id);
    absent.refresh_hash();
    assert_eq!(
        PredicateEvidence::laboratory(&absent)?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::False,
    );
    Ok(())
}

#[test]
fn complete_blueprint_deadlines_cover_queued_and_serial_shared_worker_service() -> TestResult {
    let mut world = source(true)?;
    let queue_at = MapCoord::new(0, 0, 10);
    world.set_tile_code(queue_at, tile_codes::FLOOR)?;
    world.refresh_hash();
    dispatch(
        &mut world,
        &Action::Build {
            kind: BuildingKind::Workshop("Still".to_owned()),
            location: queue_at,
            footprint: MapCuboid::new(queue_at, queue_at)?,
            material: MaterialSelector::default(),
        },
        "already queued construction",
    )?;
    let original = world.clone();
    let intent = intent(
        &world,
        BlueprintTemplate::BedroomCluster {
            rooms_count: 12,
            room_size: (4, 4),
        },
    )?;
    assert!(
        intent
            .requested_actions
            .iter()
            .all(|request| request.obligation.is_none())
    );
    let workload = LaboratoryWorkforcePlanning::capture(&world)?.ok_or("missing workforce")?;
    assert_eq!(workload.queued_service_ticks, effects::BUILD_TICKS);
    let floor = workload.deadline_floor(&intent.requested_actions)?;
    let plan = prepare(&world, &intent)?;
    assert_eq!(world, original, "planning must not perform physical work");
    assert!(plan.steps.iter().all(|step| {
        step.obligation
            .as_ref()
            .is_some_and(|obligation| obligation.deadline_tick >= floor)
    }));
    let mut digging_service = 0;
    for step in &plan.steps {
        if let Action::DesignateDig { area, .. } = &step.action {
            digging_service +=
                area.tile_count().ok_or("invalid dig area")? * effects::DIG_TICKS_PER_TILE;
            dispatch(&mut world, &step.action, &step.idempotency_key)?;
        }
    }
    assert!(
        digging_service > 1_000,
        "this workload cannot fit the old fixed deadline"
    );
    advance(&mut world, 1_000)?;
    assert!(!established(&world, &plan.terminal_condition)?);
    assert!(plan.steps.iter().all(|step| {
        step.obligation
            .as_ref()
            .is_some_and(|obligation| obligation.deadline_tick > world.tick)
    }));
    advance(&mut world, digging_service + effects::BUILD_TICKS - 1_000)?;
    let mut buildings = 0;
    for step in &plan.steps {
        if matches!(step.action, Action::Build { .. }) {
            dispatch(&mut world, &step.action, &step.idempotency_key)?;
            buildings += 1;
        } else {
            assert!(established(&world, &step.postconditions[0])?);
        }
    }
    assert_eq!(buildings, 12);
    advance(&mut world, buildings * effects::BUILD_TICKS)?;
    assert!(world.tick < floor);
    assert!(established(&world, &plan.terminal_condition)?);
    Ok(())
}

#[test]
fn narrow_halls_keep_distinct_furniture_inside_the_excavation() -> TestResult {
    let world = source(false)?;
    for (width, height) in [(1, 2), (1, 3), (2, 1), (3, 1)] {
        let intent = intent(&world, BlueprintTemplate::DiningHall { width, height })?;
        let area = intent
            .requested_actions
            .iter()
            .find_map(|step| match step.action {
                Action::DesignateDig { area, .. } => Some(area),
                _ => None,
            })
            .ok_or("room excavation missing")?;
        let locations: Vec<_> = intent
            .requested_actions
            .iter()
            .filter_map(|step| match step.action {
                Action::Build { location, .. } => Some(location),
                _ => None,
            })
            .collect();
        assert_eq!(locations.len(), 2);
        assert_ne!(locations[0], locations[1]);
        assert!(locations.into_iter().all(|point| area.contains(point)));
    }
    assert!(
        intent(
            &world,
            BlueprintTemplate::DiningHall {
                width: 1,
                height: 1
            }
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn legacy_furnished_sources_keep_exact_excavation_goals_and_deadlines() -> TestResult {
    let world = source(false)?;
    let template = BlueprintTemplate::BedroomCluster {
        rooms_count: 4,
        room_size: (3, 3),
    };
    let spatial = index(&world)?;
    let excavation = BlueprintPlanner.compile_blueprint_intent(
        IntentId::new(501),
        world.anchor(),
        MapCoord::new(2, 6, 10),
        template.clone(),
        &spatial,
    )?;
    let old = BlueprintPlanner.compile_furnished_blueprint_intent_legacy(
        IntentId::new(501),
        world.anchor(),
        MapCoord::new(2, 6, 10),
        template.clone(),
        &spatial,
    )?;
    assert_eq!(old.terminal_condition, excavation.terminal_condition);
    assert_eq!(
        &old.requested_actions[..excavation.requested_actions.len()],
        &excavation.requested_actions
    );
    for step in &old.requested_actions[..excavation.requested_actions.len()] {
        assert_eq!(
            step.obligation
                .as_ref()
                .ok_or("legacy deadline missing")?
                .deadline_tick,
            GameTick(world.tick.0 + 1_000)
        );
    }
    let current = intent(&world, template)?;
    assert_ne!(old.terminal_condition, current.terminal_condition);
    assert_ne!(
        prepare(&world, &old)?.digest,
        prepare(&world, &current)?.digest
    );
    Ok(())
}

#[test]
fn equipped_hub_requires_each_distinct_workshop_in_its_own_completed_bay() -> TestResult {
    let mut world = source(false)?;
    let workshops = [
        "workshop:Still".to_owned(),
        "workshop:Carpenters".to_owned(),
        "workshop:Still".to_owned(),
    ];
    let intent = BlueprintPlanner.compile_workshop_hub_intent(
        IntentId::new(501),
        world.anchor(),
        MapCoord::new(2, 6, 10),
        &workshops,
        &index(&world)?,
    )?;
    let plan = prepare(&world, &intent)?;
    let (dig, builds) = plan.steps.split_at(8);
    assert_eq!(builds.len(), 3);
    for step in dig {
        dispatch(&mut world, &step.action, &step.idempotency_key)?;
    }
    advance(&mut world, 250)?;
    for step in dig {
        assert!(established(&world, &step.postconditions[0])?);
    }
    for (i, step) in builds.iter().enumerate() {
        assert!(!established(&world, &plan.terminal_condition)?);
        dispatch(&mut world, &step.action, &step.idempotency_key)?;
        advance(&mut world, effects::BUILD_TICKS)?;
        let id = effects::created_entity_id(&step.idempotency_key, 0);
        let record = world.graph.entities.get(&id).ok_or("missing workshop")?;
        assert_eq!(
            record.fields["building_kind"].value,
            Value::Text(workshops[i].clone())
        );
        assert_eq!(
            record.fields["footprint_min"].value,
            Value::Coord(MapCoord::new(3 + 6 * i as i32, 7, 10))
        );
        assert_eq!(
            record.fields["footprint_max"].value,
            Value::Coord(MapCoord::new(5 + 6 * i as i32, 9, 10))
        );
    }
    assert!(established(&world, &plan.terminal_condition)?);
    Ok(())
}
