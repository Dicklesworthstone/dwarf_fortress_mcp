use super::*;
use crate::{Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, IntentId, ObservationCursor, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{FactPresence, WorldGraph};

const LEDGER: EntityId = EntityId::new(91);
const WORKER: EntityId = EntityId::new(11);
const WOOD: EntityId = EntityId::new(81);
const PLANT: EntityId = EntityId::new(82);

fn put(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, Value)>,
) -> Result<()> {
    create_entity(
        snapshot,
        id,
        kind.clone(),
        kind.as_str().to_owned(),
        fields
            .into_iter()
            .map(|(field, value)| (field.to_owned(), value))
            .collect(),
    )?;
    snapshot.refresh_hash();
    Ok(())
}

fn world(quantity: u64) -> Result<WorldSnapshot> {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(761),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    let settings = fortress_settings_entity_id(snapshot.fortress_id);
    put(
        &mut snapshot,
        settings,
        EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
        vec![
            (
                LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            ),
            (
                LABORATORY_HARVEST_MODEL_FIELD,
                Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            (
                LABORATORY_PROVISIONS_MODEL_FIELD,
                Value::Text(LABORATORY_PROVISIONS_MODEL.to_owned()),
            ),
        ],
    )?;
    put(
        &mut snapshot,
        WORKER,
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.WOODCUTTING", Value::Bool(true)),
            ("labor.PLANT_GATHERING", Value::Bool(true)),
            ("labor.BREW", Value::Bool(true)),
            ("labor.CARPENTRY", Value::Bool(true)),
        ],
    )?;
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
        vec![
            (STOCK_WOOD_FIELD, Value::U64(0)),
            (STOCK_PLANT_FIELD, Value::U64(0)),
            (STOCK_DRINK_FIELD, Value::U64(100)),
            (STOCK_FOOD_FIELD, Value::U64(100)),
            (STOCK_BED_FIELD, Value::U64(0)),
            (METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    )?;
    for (id, resource) in [
        (WOOD, HarvestResource::Wood),
        (PLANT, HarvestResource::Plant),
    ] {
        put(
            &mut snapshot,
            id,
            EntityKind::Plant,
            vec![
                (
                    HARVEST_SOURCE_MODEL_FIELD,
                    Value::Text(LABORATORY_HARVEST_MODEL.to_owned()),
                ),
                (
                    HARVEST_RESOURCE_FIELD,
                    Value::Text(resource.as_str().to_owned()),
                ),
                (HARVEST_INITIAL_FIELD, Value::U64(quantity)),
                (HARVEST_REMAINING_FIELD, Value::U64(quantity)),
                (HARVEST_YIELDED_FIELD, Value::U64(0)),
            ],
        )?;
    }
    for (id, workshop) in [(51, "Still"), (52, "Carpenters")] {
        put(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Building,
            vec![
                ("building_kind", Value::Text(format!("workshop:{workshop}"))),
                (
                    CONSTRUCTION_STAGE_FIELD,
                    Value::Text(STAGE_COMPLETE.to_owned()),
                ),
            ],
        )?;
    }
    Ok(snapshot)
}

fn action(resource: HarvestResource, amount: u32) -> Action {
    Action::HarvestResource {
        source: if resource == HarvestResource::Wood {
            WOOD
        } else {
            PLANT
        },
        source_generation: 1,
        resource,
        amount,
    }
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    let mut next = snapshot.clone();
    apply_effect(&mut next, action, key)?;
    next.refresh_hash();
    *snapshot = next;
    Ok(())
}

fn advance_with(
    snapshot: &mut WorldSnapshot,
    ticks: u64,
    limits: EffectAdvanceLimits,
) -> Result<()> {
    let mut next = snapshot.clone();
    next.tick = snapshot.tick.checked_add(ticks).ok_or_else(bounded)?;
    advance_effects_with_limits(&mut next, ticks, limits)?;
    next.refresh_hash();
    *snapshot = next;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    advance_with(snapshot, ticks, EffectAdvanceLimits::default())
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    write_fields(snapshot, id, vec![(field.to_owned(), value)])?;
    snapshot.refresh_hash();
    Ok(())
}

fn count(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    field_u64(entity(snapshot, id)?, field, snapshot.tick)
}

fn report(snapshot: &WorldSnapshot) -> Result<HarvestReport> {
    harvest_report(snapshot)?.ok_or_else(|| precondition("missing harvest fixture model"))
}

fn physical(snapshot: &WorldSnapshot) -> WorldGraph {
    let mut graph = snapshot.graph.clone();
    for record in graph.entities.values_mut() {
        record.revision = 0;
        for fact in record.fields.values_mut() {
            fact.observed_at = GameTick(0);
        }
    }
    graph
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(761),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        cancellation_requested: false,
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            max_risk: RiskTier::ReadOnly,
            scope: CapabilityScope::default(),
            expires_at_tick: None,
            remaining_uses: None,
        }],
    }
}

fn requested(action: Action) -> RequestedAction {
    RequestedAction {
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        obligation: None,
        compensation: None,
        depends_on: Vec::new(),
    }
}

fn intent(snapshot: &WorldSnapshot, actions: Vec<RequestedAction>) -> Intent {
    Intent {
        id: IntentId::new(761),
        anchor: snapshot.anchor(),
        summary: "acquire finite original resources".to_owned(),
        terminal_condition: Predicate::Any(vec![
            Predicate::FieldCompare {
                entity_id: LEDGER,
                field: STOCK_WOOD_FIELD.to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(1),
            },
            Predicate::FieldCompare {
                entity_id: LEDGER,
                field: STOCK_PLANT_FIELD.to_owned(),
                op: CompareOp::Ge,
                value: Value::U64(1),
            },
        ]),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: actions,
    }
}

#[test]
fn each_harvested_unit_is_conserved_only_at_the_service_boundary() -> Result<()> {
    for resource in [HarvestResource::Wood, HarvestResource::Plant] {
        let mut snapshot = world(2)?;
        let harvest = action(resource, 2);
        let source = if resource == HarvestResource::Wood {
            WOOD
        } else {
            PLANT
        };
        let id = created_entity_id("harvest", 0);
        apply(&mut snapshot, &harvest, "harvest")?;
        assert_eq!(count(&snapshot, LEDGER, stock_field(resource))?, 0);
        advance(&mut snapshot, 49)?;
        assert_eq!(count(&snapshot, id, "work_ticks")?, 49);
        assert_eq!(count(&snapshot, source, HARVEST_REMAINING_FIELD)?, 2);
        assert!(validate_harvest_completion(&snapshot, id, &harvest).is_err());
        advance(&mut snapshot, 1)?;
        assert_eq!(count(&snapshot, source, HARVEST_REMAINING_FIELD)?, 1);
        assert_eq!(count(&snapshot, LEDGER, stock_field(resource))?, 1);
        assert_eq!(report(&snapshot)?.tasks[0].harvested, 1);
        advance(&mut snapshot, 50)?;
        validate_harvest_completion(&snapshot, id, &harvest)?;
        assert_eq!(count(&snapshot, source, HARVEST_REMAINING_FIELD)?, 0);
        assert_eq!(count(&snapshot, LEDGER, stock_field(resource))?, 2);
        assert_eq!(report(&snapshot)?.tasks[0].status, STATUS_COMPLETE);
        assert!(crate::inspect_effect_work(&snapshot, &harvest, "harvest", true)?.is_quiescent());
        let unchanged = snapshot.clone();
        assert!(apply(&mut snapshot, &harvest, "harvest").is_err());
        assert_eq!(snapshot, unchanged);
    }
    Ok(())
}

#[test]
fn harvests_share_workers_and_preserve_physical_partition_invariance() -> Result<()> {
    let mut snapshot = world(4)?;
    apply(&mut snapshot, &action(HarvestResource::Wood, 2), "wood")?;
    apply(&mut snapshot, &action(HarvestResource::Plant, 2), "plant")?;
    let schedule =
        laboratory_workforce_report(&snapshot)?.ok_or_else(|| precondition("no workforce"))?;
    assert_eq!(schedule.assignments.len(), 1);
    assert_eq!(schedule.active_activities, 2);
    assert_eq!(schedule.remaining_service_ticks, 200);
    let mut partitioned = snapshot.clone();
    advance(&mut snapshot, 200)?;
    for ticks in [1, 48, 2, 27, 73, 49] {
        advance(&mut partitioned, ticks)?;
    }
    assert_eq!(physical(&snapshot), physical(&partitioned));
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 2);
    assert_eq!(count(&snapshot, LEDGER, STOCK_PLANT_FIELD)?, 2);
    assert_eq!(
        laboratory_workforce_report(&snapshot)?
            .ok_or_else(|| precondition("no workforce"))?
            .active_activities,
        0
    );
    Ok(())
}

#[test]
fn cancellation_preserves_yield_and_releases_only_unfinished_demand() -> Result<()> {
    let mut snapshot = world(3)?;
    let first = action(HarvestResource::Wood, 3);
    apply(&mut snapshot, &first, "first")?;
    advance(&mut snapshot, 75)?;
    let id = created_entity_id("first", 0);
    assert_eq!(count(&snapshot, id, "work_ticks")?, 25);
    assert!(cancel_effect(&mut snapshot, &first, "first")?);
    snapshot.refresh_hash();
    assert!(!cancel_effect(&mut snapshot, &first, "first")?);
    assert_eq!(report(&snapshot)?.sources[0].active_demand, 0);
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 1);
    assert_eq!(count(&snapshot, WOOD, HARVEST_REMAINING_FIELD)?, 2);
    assert!(validate_harvest_completion(&snapshot, id, &first).is_err());
    assert!(cancel_effect(&mut snapshot, &Action::Pause { paused: true }, "first").is_err());
    advance(&mut snapshot, 100)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 1);
    apply(&mut snapshot, &action(HarvestResource::Wood, 2), "second")?;
    advance(&mut snapshot, 100)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_WOOD_FIELD)?, 3);
    assert_eq!(count(&snapshot, WOOD, HARVEST_REMAINING_FIELD)?, 0);
    assert!(apply(&mut snapshot, &action(HarvestResource::Wood, 1), "third").is_err());
    Ok(())
}

#[test]
fn unknown_or_changed_source_and_completed_history_never_establish_custody() -> Result<()> {
    let mut completed = world(1)?;
    let original = action(HarvestResource::Wood, 1);
    apply(&mut completed, &original, "original")?;
    advance(&mut completed, 50)?;
    let task = created_entity_id("original", 0);
    for mutation in 0..10 {
        let mut bad = completed.clone();
        match mutation {
            0 => {
                bad.graph
                    .entities
                    .get_mut(&WOOD)
                    .ok_or_else(bounded)?
                    .generation += 1
            }
            1 => {
                bad.graph
                    .entities
                    .get_mut(&LEDGER)
                    .ok_or_else(bounded)?
                    .generation += 1
            }
            2 => {
                bad.graph
                    .entities
                    .get_mut(&fortress_settings_entity_id(bad.fortress_id))
                    .ok_or_else(bounded)?
                    .fields
                    .remove(LABORATORY_HARVEST_MODEL_FIELD);
            }
            3 => {
                bad.graph
                    .entities
                    .get_mut(&WOOD)
                    .ok_or_else(bounded)?
                    .fields
                    .get_mut(HARVEST_REMAINING_FIELD)
                    .ok_or_else(bounded)?
                    .presence = Some(FactPresence::Unknown("missing source quantity".to_owned()));
            }
            4 => {
                set(
                    &mut bad,
                    WOOD,
                    HARVEST_RESOURCE_FIELD,
                    Value::Text("STONE".to_owned()),
                )?;
            }
            5 => {
                set(&mut bad, task, HARVEST_YIELDED_FIELD, Value::U64(0))?;
            }
            6 => {
                set(
                    &mut bad,
                    task,
                    HARVEST_SOURCE_GENERATION_FIELD,
                    Value::U64(2),
                )?;
            }
            7 => {
                bad.graph
                    .entities
                    .get_mut(&task)
                    .ok_or_else(bounded)?
                    .fields
                    .get_mut(HARVEST_YIELDED_FIELD)
                    .ok_or_else(bounded)?
                    .observed_at = GameTick(bad.tick.0 + 1);
            }
            8 => {
                set(
                    &mut bad,
                    WOOD,
                    HARVEST_SOURCE_GENERATION_FIELD,
                    Value::U64(1),
                )?;
            }
            _ => {
                bad.graph.entities.remove(&task);
            }
        }
        bad.refresh_hash();
        assert!(report(&bad).is_err(), "mutation {mutation}");
        assert!(validate_harvest_completion(&bad, task, &original).is_err());
        assert!(
            crate::inspect_effect_work(&bad, &original, "original", true)
                .map_or(true, |work| !work.is_quiescent())
        );
    }
    Ok(())
}

#[test]
fn planning_and_dispatch_count_joint_active_demand_without_crediting_future_inputs() -> Result<()> {
    let planner = StaticPlanner::default();
    let mut snapshot = world(3)?;
    let excessive = intent(
        &snapshot,
        vec![
            requested(action(HarvestResource::Plant, 2)),
            requested(action(HarvestResource::Plant, 2)),
        ],
    );
    assert!(
        planner
            .prepare_laboratory(&snapshot, &excessive, &context(&snapshot))
            .is_err()
    );
    apply(
        &mut snapshot,
        &action(HarvestResource::Plant, 2),
        "existing",
    )?;
    let next = intent(
        &snapshot,
        vec![requested(action(HarvestResource::Plant, 2))],
    );
    assert!(
        planner
            .prepare_laboratory(&snapshot, &next, &context(&snapshot))
            .is_err()
    );
    assert!(apply(&mut snapshot, &action(HarvestResource::Plant, 2), "excess").is_err());
    let next = intent(
        &snapshot,
        vec![requested(action(HarvestResource::Plant, 1))],
    );
    planner.prepare_laboratory(&snapshot, &next, &context(&snapshot))?;
    let mixed = intent(
        &snapshot,
        vec![
            requested(action(HarvestResource::Plant, 1)),
            requested(Action::CreateWorkOrder {
                name: "future ingredients".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            }),
        ],
    );
    assert!(
        planner
            .prepare_laboratory(&snapshot, &mixed, &context(&snapshot))
            .is_err()
    );
    Ok(())
}

#[test]
fn mandatory_harvest_proof_and_original_archive_survive_caller_overrides() -> Result<()> {
    let snapshot = world(2)?;
    let mut request = requested(action(HarvestResource::Wood, 2));
    request.postconditions = vec![Predicate::True];
    request.obligation = Some(ObligationSpec {
        terminal: Predicate::True,
        failure: None,
        deadline_tick: GameTick(1_111),
        poll_interval_ticks: 7,
        stable_for_observations: 2,
    });
    let plan = StaticPlanner::default().prepare_laboratory(
        &snapshot,
        &intent(&snapshot, vec![request]),
        &context(&snapshot),
    )?;
    let step = &plan.steps[0];
    assert!(plan.requires_checkpoint);
    assert_eq!(step.required_capability, Capability::Designate);
    assert_eq!(step.risk, RiskTier::Guarded);
    let obligation = step.obligation.as_ref().ok_or_else(bounded)?;
    assert_eq!(obligation.deadline_tick, GameTick(1_111));
    assert_eq!(obligation.poll_interval_ticks, 7);
    assert_eq!(obligation.stable_for_observations, 2);
    assert!(
        !dfmcp_world::PredicateEvidence::laboratory(&snapshot)?
            .establishes(&obligation.terminal)?
    );
    assert!(crate::workforce_planning::plan_uses_shared_workforce(&plan));
    assert!(step.postconditions.contains(&field_eq(
        created_entity_id(&step.idempotency_key, 0),
        HARVEST_LEDGER_GENERATION_FIELD,
        Value::U64(1)
    )));
    let limits = crate::plan_archive::ArchiveLimits::default();
    let (bytes, seal) = crate::plan_archive::encode(&plan, limits)?;
    assert_eq!(crate::plan_archive::decode(&bytes, seal, limits)?, plan);
    assert_eq!(step.action.canonical_bytes()[0], 11);
    assert_eq!(Action::Pause { paused: true }.canonical_bytes(), vec![0, 1]);
    Ok(())
}

#[test]
fn disabled_workers_do_not_bank_harvest_time_and_production_uses_observed_yield() -> Result<()> {
    let mut snapshot = world(1)?;
    set(
        &mut snapshot,
        WORKER,
        "labor.PLANT_GATHERING",
        Value::Bool(false),
    )?;
    apply(&mut snapshot, &action(HarvestResource::Plant, 1), "gather")?;
    advance(&mut snapshot, 500)?;
    let task = created_entity_id("gather", 0);
    assert_eq!(count(&snapshot, task, "work_ticks")?, 0);
    assert_eq!(count(&snapshot, LEDGER, STOCK_PLANT_FIELD)?, 0);
    apply(
        &mut snapshot,
        &Action::SetLabor {
            units: vec![WORKER],
            labor: "PLANT_GATHERING".to_owned(),
            enabled: true,
        },
        "enable",
    )?;
    advance(&mut snapshot, 49)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_PLANT_FIELD)?, 0);
    advance(&mut snapshot, 1)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_PLANT_FIELD)?, 1);
    let brew = Action::CreateWorkOrder {
        name: "actual supply".to_owned(),
        job_token: "BREW_DRINK".to_owned(),
        amount: 1,
        conditions: Vec::new(),
    };
    apply(&mut snapshot, &brew, "brew")?;
    advance(&mut snapshot, 50)?;
    assert_eq!(count(&snapshot, LEDGER, STOCK_PLANT_FIELD)?, 0);
    assert_eq!(count(&snapshot, LEDGER, STOCK_DRINK_FIELD)?, 105);
    assert_eq!(report(&snapshot)?.tasks[0].harvested, 1);
    Ok(())
}

#[test]
fn bounds_and_refusals_preserve_the_transaction_shadow() -> Result<()> {
    let mut snapshot = world(200)?;
    for amount in [0, MAX_HARVEST_AMOUNT + 1, u32::MAX] {
        let unchanged = snapshot.clone();
        assert!(apply(&mut snapshot, &action(HarvestResource::Wood, amount), "bad").is_err());
        assert_eq!(snapshot, unchanged);
    }
    for generation in [0, u64::from(u32::MAX) + 1, u64::MAX] {
        let request = Action::HarvestResource {
            source: WOOD,
            source_generation: generation,
            resource: HarvestResource::Wood,
            amount: 1,
        };
        assert!(
            StaticPlanner::default()
                .prepare_laboratory(
                    &snapshot,
                    &intent(&snapshot, vec![requested(request)]),
                    &context(&snapshot)
                )
                .is_err()
        );
    }
    apply(&mut snapshot, &action(HarvestResource::Wood, 1), "bounded")?;
    let unchanged = snapshot.clone();
    assert!(
        advance_with(
            &mut snapshot,
            50,
            EffectAdvanceLimits {
                max_work_units: 1,
                ..EffectAdvanceLimits::default()
            }
        )
        .is_err()
    );
    assert_eq!(snapshot, unchanged);
    set(
        &mut snapshot,
        LEDGER,
        STOCK_WOOD_FIELD,
        Value::U64(u64::MAX),
    )?;
    let unchanged = snapshot.clone();
    assert!(advance(&mut snapshot, 50).is_err());
    assert_eq!(snapshot, unchanged);
    Ok(())
}

#[test]
fn competing_harvest_completion_preserves_earned_partial_service_after_depletion() -> Result<()> {
    let mut source = world(2)?;
    let mut second = entity(&source, WORKER)?.clone();
    second.id = EntityId::new(12);
    source.graph.entities.insert(second.id, second);
    source.refresh_hash();
    apply(&mut source, &action(HarvestResource::Wood, 1), "first")?;
    apply(&mut source, &action(HarvestResource::Wood, 1), "second")?;
    // A separately published qualified ecological change can reduce the supply
    // after both original effects started. It does not rewrite either task.
    set(&mut source, WOOD, HARVEST_INITIAL_FIELD, Value::U64(1))?;
    set(&mut source, WOOD, HARVEST_REMAINING_FIELD, Value::U64(1))?;
    assert_eq!(report(&source)?.sources[0].active_demand, 2);
    let mut partitioned = source.clone();
    advance(&mut source, 50)?;
    advance(&mut partitioned, 49)?;
    advance(&mut partitioned, 1)?;
    assert_eq!(physical(&source), physical(&partitioned));
    assert_eq!(count(&source, LEDGER, STOCK_WOOD_FIELD)?, 1);
    let pending = report(&source)?
        .tasks
        .into_iter()
        .find(|task| task.status == STATUS_ACTIVE)
        .ok_or_else(bounded)?;
    assert_eq!(pending.work_ticks, 49);
    advance(&mut source, 500)?;
    assert_eq!(count(&source, pending.task, "work_ticks")?, 49);
    // Another qualified source publication can supply the missing unit; only
    // one new tick is needed, not a replacement effect or invented idle work.
    set(&mut source, WOOD, HARVEST_INITIAL_FIELD, Value::U64(2))?;
    set(&mut source, WOOD, HARVEST_REMAINING_FIELD, Value::U64(1))?;
    advance(&mut source, 1)?;
    assert_eq!(count(&source, LEDGER, STOCK_WOOD_FIELD)?, 2);
    assert!(
        report(&source)?
            .tasks
            .iter()
            .all(|task| task.status == STATUS_COMPLETE)
    );
    Ok(())
}
