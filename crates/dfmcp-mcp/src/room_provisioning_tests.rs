use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, GameTick,
    OperationContext, RequestId, SessionId, WorkBudget,
};
use dfmcp_intent::PreparedPlan;
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Fact, FactPresence, FactSource, PredicateEvidence, PredicateTruth, tile_codes};

#[path = "room_residual_tests.rs"]
mod residual;

fn source() -> Result<WorldSnapshot> {
    crate::lab_world::scenario_snapshot("starter_fortress", FortressId::new(963), false)
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

fn bedrooms(rooms: u32) -> Result<ProvisionedRoomRequest> {
    ProvisionedRoomRequest::parse_current(&json!({
        "template":"provisioned_room",
        "layout":{"template":"bedroom_cluster","origin":[2,6,10],"rooms":rooms,"room_size":[3,3]},
    }).to_string())
}

fn dining(width: u8, height: u8) -> Result<ProvisionedRoomRequest> {
    ProvisionedRoomRequest::parse_current(
        &json!({
            "template":"provisioned_room",
            "layout":{"template":"dining_hall","origin":[2,6,10],"width":width,"height":height},
        })
        .to_string(),
    )
}

fn execute(source: WorldSnapshot, intent: &Intent) -> Result<(WorldSnapshot, PreparedPlan)> {
    let plan = StaticPlanner::default().prepare_laboratory(&source, intent, &context(&source))?;
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|work| work.deadline_tick.0))
        .max()
        .unwrap_or(source.tick.0);
    let mut adapter = MemoryAdapter::new(source);
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
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&intent.terminal_condition)?,
        PredicateTruth::True
    );
    Ok((adapter.snapshot().clone(), plan))
}

fn produce(source: WorldSnapshot, token: &str, count: u32) -> Result<WorldSnapshot> {
    let request = ProductionRequest::parse_current(
        &json!({
            "template":"production","quotas":[{"item":token,"minimum":count}],
        })
        .to_string(),
    )?;
    let compiled = request.compile(&source)?;
    let mut intent = Intent {
        id: IntentId::new(963),
        anchor: source.anchor(),
        summary: "original loose inventory".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    };
    compiled.apply_capacity_horizon(&mut intent)?;
    execute(source, &intent).map(|(snapshot, _)| snapshot)
}

fn field<'a>(snapshot: &'a WorldSnapshot, entity: EntityId, name: &str) -> Result<&'a Value> {
    snapshot
        .graph
        .entities
        .get(&entity)
        .and_then(|record| record.fields.get(name))
        .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
        .ok_or_else(|| invalid(format!("fixture field missing: {name}")))
}

fn material(snapshot: &WorldSnapshot) -> Result<u64> {
    let ledger =
        effects::stock_ledger(snapshot).ok_or_else(|| invalid("fixture ledger missing"))?;
    match field(snapshot, ledger, effects::STOCK_WOOD_FIELD)? {
        Value::U64(count) => Ok(*count),
        _ => Err(invalid("fixture wood count malformed")),
    }
}

fn fact(value: Value, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

#[test]
fn original_bedrooms_produce_excavate_place_and_prove_every_furnishing() -> Result<()> {
    let source = source()?;
    let before = source.clone();
    let (intent, analysis) = bedrooms(2)?.compile(IntentId::new(964), &source, "")?;
    assert_eq!(
        source, before,
        "planning does not reserve or consume inventory"
    );
    assert_eq!(analysis["inventory"][0]["new_items_required"], 2);
    assert_eq!(analysis["intermediate_stock_quotas_are_final_goals"], false);
    assert_eq!(
        intent
            .requested_actions
            .iter()
            .filter(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
            .count(),
        1
    );
    let placements: Vec<_> = intent
        .requested_actions
        .iter()
        .filter_map(|step| match step.action {
            Action::PlaceFurniture {
                kind,
                item,
                location,
            } => Some((kind, item, location)),
            _ => None,
        })
        .collect();
    assert_eq!(placements.len(), 2);
    assert_ne!(placements[0].1, placements[1].1);
    let adapter = MemoryAdapter::new(source.clone());
    let sealed =
        StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source))?;
    let forecast = super::super::plan_forecast::forecast_plan(&adapter, &sealed, &context(&source));
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_actions_complete"], true, "{forecast}");
    assert_eq!(forecast["predicted_goal_complete"], true, "{forecast}");
    assert_eq!(forecast["predicted_physical_quiescent"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    assert_eq!(
        adapter.snapshot(),
        &before,
        "forecast leaves its original adapter unchanged"
    );
    let (completed, plan) = execute(source, &intent)?;
    assert_eq!(plan, sealed);
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True
    );
    assert_eq!(
        forecast["resolution_ticks"],
        effects::DEFAULT_POLL_INTERVAL_TICKS
    );
    assert_eq!(forecast["predicted_completion_tick"], completed.tick.0);
    assert_eq!(material(&completed)?, 18);
    assert_eq!(observed_stock(&completed, FurnitureKind::Bed)?, 0);
    assert!(effects::loose_furniture_items(&completed, FurnitureKind::Bed)?.is_empty());
    for step in plan
        .steps
        .iter()
        .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
    {
        let building = effects::created_entity_id(&step.idempotency_key, 0);
        assert_eq!(
            field(&completed, building, effects::CONSTRUCTION_STAGE_FIELD)?,
            &Value::Text(effects::STAGE_COMPLETE.to_owned())
        );
        let mut incomplete = completed.clone();
        incomplete
            .graph
            .entities
            .get_mut(&building)
            .ok_or_else(|| invalid("placed building missing"))?
            .fields
            .insert(
                effects::CONSTRUCTION_STAGE_FIELD.to_owned(),
                fact(
                    Value::Text(effects::STAGE_PLANNED.to_owned()),
                    completed.tick,
                ),
            );
        incomplete.refresh_hash();
        assert_ne!(
            PredicateEvidence::laboratory(&incomplete)?.evaluate(&intent.terminal_condition)?,
            PredicateTruth::True,
            "excavation and the other room cannot prove a missing furnishing"
        );
    }
    Ok(())
}

#[test]
fn large_narrow_room_uses_work_scaled_excavation_and_finishes_its_furnishings() -> Result<()> {
    let source = source()?;
    let (intent, _) = dining(7, 15)?.compile(IntentId::new(966), &source, "")?;
    let sealed =
        StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source))?;
    let dig = sealed
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::DesignateDig { .. }))
        .ok_or_else(|| invalid("large room excavation missing"))?;
    let Action::DesignateDig { area, .. } = dig.action else {
        return Err(invalid("large room excavation changed action family"));
    };
    assert_eq!(area.tile_count(), Some(105));
    assert!(
        dfmcp_world::terrain::region_tiles(area)
            .all(|at| source.tile_code_at(at) == Some(tile_codes::SOLID_WALL))
    );
    let work_ticks = 105 * effects::DIG_TICKS_PER_TILE;
    assert!(work_ticks > 1_000, "the former fixed horizon is too short");
    let expected = effects::default_obligation(
        &dig.action,
        &dig.idempotency_key,
        source.fortress_id,
        source.tick,
    )?
    .ok_or_else(|| invalid("default dig obligation missing"))?;
    assert_eq!(dig.obligation.as_ref(), Some(&expected));
    assert!(expected.deadline_tick.0 > source.tick.0 + work_ticks);
    for placement in sealed
        .steps
        .iter()
        .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
    {
        assert!(placement.depends_on.contains(&dig.id));
        assert!(
            placement
                .obligation
                .as_ref()
                .is_some_and(|work| work.deadline_tick > expected.deadline_tick),
            "placement keeps its own completion horizon after excavation"
        );
    }
    let initial_tick = source.tick;
    let (completed, plan) = execute(source, &intent)?;
    assert_eq!(plan, sealed);
    assert!(completed.tick.0 >= initial_tick.0 + work_ticks);
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True
    );
    assert_eq!(observed_stock(&completed, FurnitureKind::Table)?, 0);
    assert_eq!(observed_stock(&completed, FurnitureKind::Chair)?, 0);
    Ok(())
}

#[test]
fn completed_room_goal_rejects_item_and_building_generation_replacement() -> Result<()> {
    let source = source()?;
    let (intent, _) = bedrooms(1)?.compile(IntentId::new(964), &source, "")?;
    let (completed, plan) = execute(source, &intent)?;
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True
    );
    let (item, building) = plan
        .steps
        .iter()
        .find_map(|step| match step.action {
            Action::PlaceFurniture { item, .. } => {
                Some((item, effects::created_entity_id(&step.idempotency_key, 0)))
            }
            _ => None,
        })
        .ok_or_else(|| invalid("missing original placement"))?;
    for replaced in [item, building] {
        let mut changed = completed.clone();
        changed
            .graph
            .entities
            .get_mut(&replaced)
            .ok_or_else(|| invalid("missing original furniture entity"))?
            .generation += 1;
        changed.refresh_hash();
        assert_eq!(
            PredicateEvidence::laboratory(&changed)?.evaluate(&plan.terminal_condition)?,
            PredicateTruth::True,
            "matching fields alone do not preserve entity generation"
        );
        assert!(
            !matches!(
                crate::lab_world::plan_goal_truth(&changed, &plan),
                Ok(PredicateTruth::True)
            ),
            "replacement generation must not inherit the original room proof"
        );
    }
    Ok(())
}

#[test]
fn dining_reuses_exact_table_and_produces_only_the_missing_chair() -> Result<()> {
    let source = produce(source()?, "TABLE", 1)?;
    let table = effects::loose_furniture_items(&source, FurnitureKind::Table)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("missing produced table"))?;
    let (intent, analysis) = dining(3, 3)?.compile(IntentId::new(964), &source, "")?;
    let jobs: Vec<_> = intent
        .requested_actions
        .iter()
        .filter_map(|step| match &step.action {
            Action::CreateWorkOrder {
                job_token, amount, ..
            } => Some((job_token.as_str(), *amount)),
            _ => None,
        })
        .collect();
    assert_eq!(jobs, vec![("CONSTRUCT_THRONE", 1)]);
    assert!(
        intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action,
        Action::PlaceFurniture { kind:FurnitureKind::Table,item,.. } if item==table))
    );
    assert_eq!(analysis["allocation"].as_array().map(Vec::len), Some(2));
    let (completed, _) = execute(source, &intent)?;
    assert_eq!(material(&completed)?, 18);
    assert_eq!(observed_stock(&completed, FurnitureKind::Table)?, 0);
    assert_eq!(observed_stock(&completed, FurnitureKind::Chair)?, 0);
    Ok(())
}

#[test]
fn already_excavated_room_still_requires_furniture_and_can_use_existing_inventory_without_logs()
-> Result<()> {
    let request = bedrooms(1)?;
    let mut source = produce(source()?, "BED", 2)?;
    let (origin, template) = request.layout.geometry();
    let layout = BlueprintPlanner.layout(origin, template)?;
    for part in layout.excavations() {
        for at in dfmcp_world::terrain::region_tiles(part.area) {
            source.set_tile_code(at, tile_codes::FLOOR)?;
        }
    }
    let ledger = effects::stock_ledger(&source).ok_or_else(|| invalid("missing ledger"))?;
    source
        .graph
        .entities
        .get_mut(&ledger)
        .ok_or_else(|| invalid("missing ledger"))?
        .fields
        .remove(effects::STOCK_WOOD_FIELD);
    source.refresh_hash();
    let (intent, _) = request.compile(IntentId::new(964), &source, "")?;
    assert!(
        !intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    assert_eq!(
        PredicateEvidence::laboratory(&source)?.evaluate(&intent.terminal_condition)?,
        PredicateTruth::False
    );
    let (completed, _) = execute(source, &intent)?;
    assert_eq!(observed_stock(&completed, FurnitureKind::Bed)?, 1);
    assert_eq!(
        effects::loose_furniture_items(&completed, FurnitureKind::Bed)?.len(),
        1
    );
    Ok(())
}

#[test]
fn legacy_aggregate_inventory_is_not_credited_as_an_allocatable_item() -> Result<()> {
    let mut source = produce(source()?, "BED", 1)?;
    let original = effects::loose_furniture_items(&source, FurnitureKind::Bed)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("missing legacy fixture item"))?;
    let legacy = source
        .graph
        .entities
        .get_mut(&original)
        .ok_or_else(|| invalid("missing item"))?;
    legacy.fields.remove("inventory_state");
    legacy.fields.remove("allocated_to");
    source.refresh_hash();
    let before = source.clone();
    let (intent, analysis) = bedrooms(1)?.compile(IntentId::new(964), &source, "")?;
    assert_eq!(source, before);
    assert_eq!(analysis["inventory"][0]["observed_aggregate"], 1);
    assert_eq!(analysis["inventory"][0]["eligible_items_selected"], 0);
    assert_eq!(analysis["inventory"][0]["procurement_stock_target"], 2);
    assert!(
        intent
            .requested_actions
            .iter()
            .filter_map(|step| match step.action {
                Action::PlaceFurniture { item, .. } => Some(item),
                _ => None,
            })
            .all(|item| item != original)
    );
    let (completed, _) = execute(source, &intent)?;
    assert_eq!(material(&completed)?, 18);
    assert_eq!(observed_stock(&completed, FurnitureKind::Bed)?, 1);
    assert_eq!(
        completed.graph.entities.get(&original),
        before.graph.entities.get(&original)
    );
    Ok(())
}

#[test]
fn cancelled_placement_releases_the_same_item_and_target_for_a_new_room_request() -> Result<()> {
    let mut source = produce(source()?, "BED", 1)?;
    let item = effects::loose_furniture_items(&source, FurnitureKind::Bed)?
        .into_iter()
        .next()
        .ok_or_else(|| invalid("missing loose bed"))?;
    let location = MapCoord::new(3, 7, 10);
    source.set_tile_code(location, tile_codes::FLOOR)?;
    let action = Action::PlaceFurniture {
        kind: FurnitureKind::Bed,
        item,
        location,
    };
    effects::apply_effect(&mut source, &action, "cancelled-room-placement")?;
    assert!(effects::cancel_effect(
        &mut source,
        &action,
        "cancelled-room-placement"
    )?);
    source.refresh_hash();
    let cancelled = effects::created_entity_id("cancelled-room-placement", 0);
    assert!(effects::occupied_building_footprint(&source, cancelled)?.is_none());
    let (intent, _) = bedrooms(1)?.compile(IntentId::new(964), &source, "")?;
    assert!(
        !intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    assert!(
        intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action,
        Action::PlaceFurniture { item: selected, .. } if selected == item))
    );
    let (completed, _) = execute(source, &intent)?;
    assert_eq!(material(&completed)?, 19);
    assert_eq!(observed_stock(&completed, FurnitureKind::Bed)?, 0);
    assert!(effects::occupied_building_footprint(&completed, cancelled)?.is_none());
    Ok(())
}

#[test]
fn complete_original_geometry_and_unused_setup_options_change_the_source_seal() -> Result<()> {
    let source = produce(source()?, "BED", 1)?;
    let plain = bedrooms(1)?;
    let mut options: Json = serde_json::from_str(&plain.canonical_json())
        .map_err(|_| invalid("bad canonical fixture"))?;
    options["prerequisites"] = json!({"assign_labor":true,"workshops":[
        {"building":"workshop:Carpenters","location":[1,0,10]},
    ]});
    let with_options = ProvisionedRoomRequest::parse_current(&options.to_string())?;
    let (left, _) = plain.compile(IntentId::new(964), &source, "")?;
    let (right, _) = with_options.compile(IntentId::new(964), &source, "")?;
    assert_eq!(
        left.requested_actions, right.requested_actions,
        "unused setup must not introduce work"
    );
    assert_ne!(
        left.summary, right.summary,
        "every original option participates in the seal"
    );
    let canonical = with_options.canonical_json();
    let restored = ProvisionedRoomRequest::parse(&canonical)?;
    assert_eq!(canonical, restored.canonical_json());
    let (replayed, _) = restored.compile(IntentId::new(964), &source, "")?;
    let first = StaticPlanner::default().prepare_laboratory(&source, &right, &context(&source))?;
    let replay =
        StaticPlanner::default().prepare_laboratory(&source, &replayed, &context(&source))?;
    assert_eq!(first, replay);
    options["layout"]["room_size"] = json!([3, 4]);
    let changed = ProvisionedRoomRequest::parse_current(&options.to_string())?;
    assert_ne!(changed.canonical_json(), canonical);
    assert_ne!(
        changed
            .compile(IntentId::new(964), &source, "")?
            .0
            .terminal_condition,
        right.terminal_condition
    );
    Ok(())
}

#[test]
fn unavailable_inventory_insufficient_logs_and_occupied_geometry_refuse_without_changes()
-> Result<()> {
    let request = bedrooms(1)?;
    for variant in 0..3 {
        let mut source = source()?;
        let ledger = effects::stock_ledger(&source).ok_or_else(|| invalid("missing ledger"))?;
        let fields = &mut source
            .graph
            .entities
            .get_mut(&ledger)
            .ok_or_else(|| invalid("missing ledger"))?
            .fields;
        match variant {
            0 => {
                fields.remove(effects::STOCK_BED_FIELD);
            }
            1 => {
                fields
                    .get_mut(effects::STOCK_BED_FIELD)
                    .ok_or_else(|| invalid("missing bed count"))?
                    .presence = Some(FactPresence::Unknown("uncaptured".to_owned()));
            }
            _ => {
                fields.insert(
                    effects::STOCK_WOOD_FIELD.to_owned(),
                    fact(Value::U64(0), source.tick),
                );
            }
        }
        source.refresh_hash();
        let before = source.clone();
        assert!(request.compile(IntentId::new(964), &source, "").is_err());
        assert_eq!(source, before);
    }
    let source = source()?;
    let (intent, _) = request.compile(IntentId::new(964), &source, "")?;
    let (occupied, _) = execute(source, &intent)?;
    assert!(request.compile(IntentId::new(965), &occupied, "").is_err());
    Ok(())
}

#[test]
fn bounded_room_grammar_keeps_the_fixed_compiler_and_distinct_interior_targets() -> Result<()> {
    let request = bedrooms(1)?;
    assert!(is_provisioned_room(&request.canonical_json()));
    let mut raw: Json = serde_json::from_str(&request.canonical_json())
        .map_err(|_| invalid("bad canonical fixture"))?;
    raw.as_object_mut()
        .ok_or_else(|| invalid("bad object fixture"))?
        .remove("planner");
    assert!(ProvisionedRoomRequest::parse(&raw.to_string()).is_err());
    assert!(ProvisionedRoomRequest::parse_current(&raw.to_string()).is_ok());
    raw["planner"] = json!("provisioned_room_v2");
    assert!(ProvisionedRoomRequest::parse_current(&raw.to_string()).is_err());
    raw["planner"] = json!(PLANNER);
    raw["ignored_authority"] = json!(true);
    assert!(ProvisionedRoomRequest::parse_current(&raw.to_string()).is_err());
    assert!(bedrooms(17).is_err());
    assert!(dining(1, 1).is_err());
    let source = source()?;
    let (intent, _) = dining(1, 2)?.compile(IntentId::new(964), &source, "")?;
    let locations: BTreeSet<_> = intent
        .requested_actions
        .iter()
        .filter_map(|step| match step.action {
            Action::PlaceFurniture { location, .. } => Some(location),
            _ => None,
        })
        .collect();
    assert_eq!(locations.len(), 2);
    assert!(
        locations
            .iter()
            .all(|at| at.x == 2 && (6..=7).contains(&at.y))
    );
    execute(source, &intent)?;
    Ok(())
}
