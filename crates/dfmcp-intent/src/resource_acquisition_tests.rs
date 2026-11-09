//! Original-source compilation and physical reference-economy regressions.

use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, FortressId, IntentId,
    ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{Fact, FactPresence, FactSource, PredicateTruth, WorldGraph};

use super::*;
use crate::{Constraint, Intent, PreparedPlan, StaticPlanner};

const LEDGER: EntityId = EntityId::new(91);
const WORKER: EntityId = EntityId::new(11);
const WOOD: EntityId = EntityId::new(81);
const PLANT: EntityId = EntityId::new(82);

fn fact(value: Value, tick: GameTick) -> Fact {
    Fact::known(
        value,
        tick,
        FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
        Digest32::ZERO,
    )
}

fn insert(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    kind: EntityKind,
    fields: Vec<(&str, Value)>,
) {
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: format!("resource fixture {id}"),
            fields: fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), fact(value, snapshot.tick)))
                .collect(),
        },
    );
    snapshot.refresh_hash();
}

fn add_source(snapshot: &mut WorldSnapshot, id: EntityId, resource: HarvestResource, amount: u64) {
    insert(
        snapshot,
        id,
        EntityKind::Plant,
        vec![
            (
                effects::HARVEST_SOURCE_MODEL_FIELD,
                Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            (
                effects::HARVEST_RESOURCE_FIELD,
                Value::Text(resource.as_str().to_owned()),
            ),
            (effects::HARVEST_INITIAL_FIELD, Value::U64(amount)),
            (effects::HARVEST_REMAINING_FIELD, Value::U64(amount)),
            (effects::HARVEST_YIELDED_FIELD, Value::U64(0)),
        ],
    );
}

fn world(source_units: u64) -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(8_271),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    let settings = effects::fortress_settings_entity_id(snapshot.fortress_id);
    insert(
        &mut snapshot,
        settings,
        EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
        vec![
            (
                effects::LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
            ),
            (
                effects::LABORATORY_HARVEST_MODEL_FIELD,
                Value::Text(effects::LABORATORY_HARVEST_MODEL.to_owned()),
            ),
            (
                effects::LABORATORY_PROVISIONS_MODEL_FIELD,
                Value::Text(effects::LABORATORY_PROVISIONS_MODEL.to_owned()),
            ),
        ],
    );
    insert(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_WOOD_FIELD, Value::U64(0)),
            (effects::STOCK_PLANT_FIELD, Value::U64(0)),
            (effects::STOCK_FOOD_FIELD, Value::U64(10_000)),
            (effects::STOCK_DRINK_FIELD, Value::U64(10_000)),
            (effects::STOCK_BED_FIELD, Value::U64(0)),
            (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    );
    insert(
        &mut snapshot,
        WORKER,
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.WOODCUTTING", Value::Bool(true)),
            ("labor.PLANT_GATHERING", Value::Bool(true)),
            ("labor.CARPENTRY", Value::Bool(true)),
            ("labor.BREW", Value::Bool(true)),
            (effects::SQUAD_FIELD, Value::Null),
        ],
    );
    for (id, workshop) in [(51, "Still"), (52, "Carpenters")] {
        insert(
            &mut snapshot,
            EntityId::new(id),
            EntityKind::Building,
            vec![
                ("building_kind", Value::Text(format!("workshop:{workshop}"))),
                (
                    effects::CONSTRUCTION_STAGE_FIELD,
                    Value::Text(effects::STAGE_COMPLETE.to_owned()),
                ),
            ],
        );
    }
    add_source(&mut snapshot, WOOD, HarvestResource::Wood, source_units);
    add_source(&mut snapshot, PLANT, HarvestResource::Plant, source_units);
    snapshot
}

fn set(snapshot: &mut WorldSnapshot, id: EntityId, field: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| refused("missing test entity"))?
        .fields
        .insert(field.to_owned(), fact(value, snapshot.tick));
    snapshot.refresh_hash();
    Ok(())
}

fn target(resource: HarvestResource, minimum: u64) -> ResourceStockTarget {
    ResourceStockTarget { resource, minimum }
}

fn stock(snapshot: &WorldSnapshot, resource: HarvestResource) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&LEDGER)
        .and_then(|ledger| value(ledger, stock_field(resource), snapshot.tick))
    {
        Some(Value::U64(quantity)) => Ok(*quantity),
        _ => Err(refused("missing test stock")),
    }
}

fn prepare(
    snapshot: &WorldSnapshot,
    acquisition: &ResourceAcquisitionPlan,
) -> Result<PreparedPlan> {
    let intent = Intent {
        id: IntentId::new(8_271),
        anchor: acquisition.source_anchor,
        summary: "acquire the complete original stock minima".to_owned(),
        terminal_condition: acquisition.terminal_condition.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: acquisition.requested_actions.clone(),
    };
    StaticPlanner::default().prepare_laboratory(
        snapshot,
        &intent,
        &OperationContext {
            session_id: SessionId::new(8_271),
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
        },
    )
}

fn apply(snapshot: &mut WorldSnapshot, action: &Action, key: &str) -> Result<()> {
    let mut candidate = snapshot.clone();
    effects::apply_effect(&mut candidate, action, key)?;
    candidate.refresh_hash();
    *snapshot = candidate;
    Ok(())
}

fn advance(snapshot: &mut WorldSnapshot, ticks: u64) -> Result<()> {
    let mut candidate = snapshot.clone();
    candidate.tick = candidate.tick.checked_add(ticks).ok_or_else(bounded)?;
    effects::advance_effects(&mut candidate, ticks)?;
    candidate.refresh_hash();
    *snapshot = candidate;
    Ok(())
}

#[test]
fn original_stock_minima_compile_real_deficits_and_conserve_completed_resources() -> Result<()> {
    let mut snapshot = world(40);
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(3),
    )?;
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(1),
    )?;
    let original = snapshot.clone();
    let targets = [
        target(HarvestResource::Wood, 6),
        target(HarvestResource::Plant, 4),
    ];
    let acquisition = ResourceAcquisitionPlan::compile(&snapshot, &targets, false)?;
    assert_eq!(
        snapshot, original,
        "planning must not reserve or credit stock"
    );
    assert_eq!(acquisition.ledger, LEDGER);
    assert_eq!(acquisition.ledger_generation, 1);
    assert_eq!(acquisition.requirements[0].observed, 3);
    assert_eq!(acquisition.requirements[0].deficit, 3);
    assert_eq!(acquisition.requirements[1].deficit, 3);
    assert_eq!(acquisition.requested_service_ticks, 300);
    let plan = prepare(&snapshot, &acquisition)?;
    assert!(plan.requires_checkpoint);
    assert_eq!(plan.terminal_condition, acquisition.terminal_condition);
    assert!(plan.steps.iter().all(|step| {
        step.obligation
            .as_ref()
            .is_some_and(|obligation| Some(obligation.deadline_tick) == acquisition.deadline_tick)
    }));
    for step in &plan.steps {
        apply(&mut snapshot, &step.action, &step.idempotency_key)?;
    }
    assert_eq!(stock(&snapshot, HarvestResource::Wood)?, 3);
    assert_eq!(stock(&snapshot, HarvestResource::Plant)?, 1);
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::False,
    );
    advance(&mut snapshot, acquisition.requested_service_ticks)?;
    assert_eq!(stock(&snapshot, HarvestResource::Wood)?, 6);
    assert_eq!(stock(&snapshot, HarvestResource::Plant)?, 4);
    for step in &plan.steps {
        let evidence = PredicateEvidence::laboratory(&snapshot)?;
        assert!(evidence.establishes(&Predicate::All(step.postconditions.clone()))?);
        effects::validate_harvest_completion(
            &snapshot,
            effects::created_entity_id(&step.idempotency_key, 0),
            &step.action,
        )?;
    }
    let report = effects::harvest_report(&snapshot)?.ok_or_else(bounded)?;
    for source in &report.sources {
        assert_eq!(source.initial, source.remaining + source.harvested);
        assert_eq!(source.harvested, 3);
        assert_eq!(source.active_demand, 0);
    }
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(&plan.terminal_condition)?);
    // Completed harvest is not proof that its original stock minimum remains
    // true after a later consumer removes output.
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(5),
    )?;
    assert_eq!(
        PredicateEvidence::laboratory(&snapshot)?.evaluate(&plan.terminal_condition)?,
        PredicateTruth::False,
    );
    Ok(())
}

#[test]
fn allocation_prefers_largest_useful_tranches_then_original_source_identity() -> Result<()> {
    let mut snapshot = world(40);
    add_source(&mut snapshot, EntityId::new(83), HarvestResource::Wood, 100);
    add_source(&mut snapshot, EntityId::new(84), HarvestResource::Wood, 80);
    let targets = [
        target(HarvestResource::Plant, 2),
        target(HarvestResource::Wood, 150),
    ];
    let plan = ResourceAcquisitionPlan::compile(&snapshot, &targets, false)?;
    assert_eq!(plan.allocations.len(), 3);
    assert_eq!(plan.allocations[0].source, EntityId::new(83));
    assert_eq!(plan.allocations[0].amount, 100);
    assert_eq!(plan.allocations[1].source, EntityId::new(84));
    assert_eq!(plan.allocations[1].amount, 50);
    let mut reversed = targets;
    reversed.reverse();
    assert_eq!(
        plan,
        ResourceAcquisitionPlan::compile(&snapshot, &reversed, false)?
    );
    let tied =
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 30)], false)?;
    assert_eq!(tied.allocations[0].source, WOOD);
    assert_eq!(tied.allocations[0].amount, 30);
    Ok(())
}

#[test]
fn source_supply_and_complete_action_count_refuse_without_partial_proposals() -> Result<()> {
    let snapshot = world(2_000);
    let full = ResourceAcquisitionPlan::compile(
        &snapshot,
        &[target(HarvestResource::Wood, 1_600)],
        false,
    )?;
    assert_eq!(
        full.requested_actions.len(),
        MAX_RESOURCE_ACQUISITION_ACTIONS
    );
    assert!(
        full.allocations
            .iter()
            .all(|allocation| allocation.amount == 100)
    );
    assert!(matches!(
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 1_601)], false),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    assert!(matches!(
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 2_001)], false),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
    ));
    let mut disabled = snapshot;
    set(
        &mut disabled,
        WORKER,
        "labor.WOODCUTTING",
        Value::Bool(false),
    )?;
    assert_eq!(
        ResourceAcquisitionPlan::compile(&disabled, &[target(HarvestResource::Wood, 1_500)], true)?
            .requested_actions
            .len(),
        16,
    );
    assert!(matches!(
        ResourceAcquisitionPlan::compile(&disabled, &[target(HarvestResource::Wood, 1_501)], true),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    Ok(())
}

#[test]
fn unknown_untrusted_future_and_malformed_source_fields_cannot_supply_acquisition() -> Result<()> {
    let original = world(40);
    for (id, field) in [
        (WOOD, effects::HARVEST_REMAINING_FIELD),
        (LEDGER, effects::STOCK_WOOD_FIELD),
        (WORKER, "alive"),
        (WORKER, "labor.WOODCUTTING"),
    ] {
        for variant in 0..5 {
            let mut snapshot = original.clone();
            let record = snapshot.graph.entities.get_mut(&id).ok_or_else(bounded)?;
            if variant == 4 {
                record.fields.remove(field);
            } else {
                let current = record.fields.get_mut(field).ok_or_else(bounded)?;
                match variant {
                    0 => current.presence = Some(FactPresence::Unknown("unobserved".to_owned())),
                    1 => current.source = FactSource::Derived("unregistered".to_owned()),
                    2 => current.source_digest = Digest32::of_bytes(b"not a reference producer"),
                    _ => current.observed_at = GameTick(snapshot.tick.0 + 1),
                }
            }
            snapshot.refresh_hash();
            assert!(
                ResourceAcquisitionPlan::compile(
                    &snapshot,
                    &[target(HarvestResource::Wood, 1)],
                    true,
                )
                .is_err(),
                "{id} {field} variant {variant} supplied unavailable truth",
            );
        }
    }
    Ok(())
}

#[test]
fn optional_staffing_enables_only_observed_false_labor_before_physical_work() -> Result<()> {
    let mut snapshot = world(40);
    set(
        &mut snapshot,
        WORKER,
        "labor.WOODCUTTING",
        Value::Bool(false),
    )?;
    set(
        &mut snapshot,
        WORKER,
        "labor.PLANT_GATHERING",
        Value::Bool(false),
    )?;
    let targets = [
        target(HarvestResource::Wood, 2),
        target(HarvestResource::Plant, 2),
    ];
    assert!(ResourceAcquisitionPlan::compile(&snapshot, &targets, false).is_err());
    let acquisition = ResourceAcquisitionPlan::compile(&snapshot, &targets, true)?;
    assert_eq!(acquisition.requested_actions.len(), 4);
    assert_eq!(acquisition.staffing[0].action_index, Some(0));
    assert_eq!(acquisition.staffing[1].action_index, Some(1));
    assert_eq!(acquisition.requested_actions[2].depends_on, vec![0]);
    assert_eq!(acquisition.requested_actions[3].depends_on, vec![1]);
    let prepared = prepare(&snapshot, &acquisition)?;
    for step in &prepared.steps[..2] {
        assert!(matches!(
            step.compensation,
            Some(Action::SetLabor { enabled: false, .. })
        ));
        apply(&mut snapshot, &step.action, &step.idempotency_key)?;
        assert!(
            PredicateEvidence::laboratory(&snapshot)?
                .establishes(&Predicate::All(step.postconditions.clone()))?
        );
    }
    for step in &prepared.steps[2..] {
        apply(&mut snapshot, &step.action, &step.idempotency_key)?;
    }
    advance(&mut snapshot, 200)?;
    assert_eq!(stock(&snapshot, HarvestResource::Wood)?, 2);
    assert_eq!(stock(&snapshot, HarvestResource::Plant)?, 2);
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(&prepared.terminal_condition)?);
    Ok(())
}

#[test]
fn staffing_prefers_existing_capacity_then_known_nonmilitary_and_canonical_id() -> Result<()> {
    let mut snapshot = world(40);
    set(
        &mut snapshot,
        WORKER,
        effects::SQUAD_FIELD,
        Value::Entity(EntityId::new(97)),
    )?;
    insert(
        &mut snapshot,
        EntityId::new(10),
        EntityKind::Unit,
        vec![
            ("alive", Value::Bool(true)),
            ("labor.WOODCUTTING", Value::Bool(false)),
            (effects::SQUAD_FIELD, Value::Null),
        ],
    );
    let targets = [target(HarvestResource::Wood, 1)];
    let initial = ResourceAcquisitionPlan::compile(&snapshot, &targets, true)?;
    assert_eq!(initial.staffing[0].worker, WORKER);
    assert!(initial.staffing[0].previously_enabled);
    set(
        &mut snapshot,
        WORKER,
        "labor.WOODCUTTING",
        Value::Bool(false),
    )?;
    let enabling = ResourceAcquisitionPlan::compile(&snapshot, &targets, true)?;
    assert_eq!(enabling.staffing[0].worker, EntityId::new(10));
    assert!(!enabling.staffing[0].previously_enabled);
    Ok(())
}

#[test]
fn active_harvest_is_never_double_ordered_but_unrelated_work_extends_the_horizon() -> Result<()> {
    let mut snapshot = world(40);
    apply(
        &mut snapshot,
        &Action::HarvestResource {
            source: WOOD,
            source_generation: 1,
            resource: HarvestResource::Wood,
            amount: 4,
        },
        "existing-original-wood",
    )?;
    assert!(matches!(
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 5)], false),
        Err(error) if error.code == ErrorCode::PreconditionsFailed
            && error.message.contains("harvest remains active")
    ));
    let plant =
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Plant, 2)], false)?;
    assert_eq!(plant.queued_activities, 1);
    assert_eq!(plant.queued_service_ticks, 200);
    assert_eq!(plant.requested_service_ticks, 100);
    let workforce = LaboratoryWorkforcePlanning::capture(&snapshot)?.ok_or_else(bounded)?;
    assert_eq!(
        plant.deadline_tick,
        Some(workforce.deadline_floor(&plant.requested_actions)?)
    );
    advance(&mut snapshot, 200)?;
    let next =
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 5)], false)?;
    assert_eq!(next.requirements[0].observed, 4);
    assert_eq!(next.requirements[0].deficit, 1);
    assert_eq!(next.allocations[0].amount, 1);
    Ok(())
}

#[test]
fn consuming_orders_refuse_target_acquisition_without_inventing_reserve_inputs() -> Result<()> {
    for (job, resource) in [
        ("CONSTRUCT_BED", HarvestResource::Wood),
        ("BREW_DRINK", HarvestResource::Plant),
    ] {
        let mut snapshot = world(40);
        apply(
            &mut snapshot,
            &Action::CreateWorkOrder {
                name: "existing finite consumer".to_owned(),
                job_token: job.to_owned(),
                amount: 2,
                conditions: Vec::new(),
            },
            "original-consumer",
        )?;
        let before = snapshot.clone();
        assert!(matches!(
            ResourceAcquisitionPlan::compile(&snapshot, &[target(resource, 3)], false),
            Err(error) if error.code == ErrorCode::PreconditionsFailed
                && error.message.contains("consuming production remains active")
        ));
        assert_eq!(snapshot, before);
        let other = match resource {
            HarvestResource::Wood => HarvestResource::Plant,
            HarvestResource::Plant => HarvestResource::Wood,
        };
        let independent = ResourceAcquisitionPlan::compile(&snapshot, &[target(other, 3)], false)?;
        assert_eq!(independent.queued_activities, 1);
        assert_eq!(independent.queued_service_ticks, 100);
        // A currently met companion target must survive the other harvest too.
        set(&mut snapshot, LEDGER, stock_field(resource), Value::U64(3))?;
        assert!(
            ResourceAcquisitionPlan::compile(
                &snapshot,
                &[target(resource, 3), target(other, 3)],
                false,
            )
            .is_err()
        );
        let no_op = ResourceAcquisitionPlan::compile(&snapshot, &[target(resource, 3)], true)?;
        assert!(no_op.requested_actions.is_empty());
    }
    Ok(())
}

#[test]
fn unavailable_workload_or_changed_original_identity_cannot_pass_as_a_valid_plan() -> Result<()> {
    let mut snapshot = world(40);
    apply(
        &mut snapshot,
        &Action::CreateWorkOrder {
            name: "unrelated original work".to_owned(),
            job_token: "BREW_DRINK".to_owned(),
            amount: 1,
            conditions: Vec::new(),
        },
        "unknown-queue",
    )?;
    let order = effects::created_entity_id("unknown-queue", 0);
    snapshot
        .graph
        .entities
        .get_mut(&order)
        .and_then(|record| record.fields.get_mut("work_ticks"))
        .ok_or_else(bounded)?
        .presence = Some(FactPresence::Unknown("unobserved service".to_owned()));
    snapshot.refresh_hash();
    assert!(
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 1)], false,)
            .is_err()
    );

    let source = world(40);
    let acquisition =
        ResourceAcquisitionPlan::compile(&source, &[target(HarvestResource::Wood, 1)], false)?;
    for identity in [WOOD, LEDGER, WORKER] {
        let mut changed = source.clone();
        changed
            .graph
            .entities
            .get_mut(&identity)
            .ok_or_else(bounded)?
            .generation += 1;
        changed.refresh_hash();
        assert!(matches!(
            prepare(&changed, &acquisition),
            Err(error) if error.code == ErrorCode::StaleAnchor
        ));
    }
    Ok(())
}

#[test]
fn satisfied_targets_are_true_no_ops_even_without_workers_and_large_stock_is_not_truncated()
-> Result<()> {
    let mut snapshot = world(0);
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_WOOD_FIELD,
        Value::U64(u64::MAX),
    )?;
    set(
        &mut snapshot,
        LEDGER,
        effects::STOCK_PLANT_FIELD,
        Value::U64(7),
    )?;
    snapshot.graph.entities.remove(&WORKER);
    snapshot.refresh_hash();
    let targets = [
        target(HarvestResource::Wood, u64::MAX),
        target(HarvestResource::Plant, 7),
    ];
    let plan = ResourceAcquisitionPlan::compile(&snapshot, &targets, true)?;
    assert!(plan.requested_actions.is_empty());
    assert!(plan.allocations.is_empty());
    assert!(plan.staffing.is_empty());
    assert!(plan.deadline_tick.is_none());
    assert_eq!(plan.requested_service_ticks, 0);
    assert!(PredicateEvidence::laboratory(&snapshot)?.establishes(&plan.terminal_condition)?);
    Ok(())
}

#[test]
fn request_duplicates_invalid_targets_and_overflowing_source_sums_refuse() -> Result<()> {
    let snapshot = world(40);
    for targets in [
        Vec::new(),
        vec![target(HarvestResource::Wood, 0)],
        vec![
            target(HarvestResource::Wood, 1),
            target(HarvestResource::Wood, 2),
        ],
        vec![
            target(HarvestResource::Wood, 1),
            target(HarvestResource::Plant, 2),
            target(HarvestResource::Wood, 3),
        ],
    ] {
        assert!(matches!(
            ResourceAcquisitionPlan::compile(&snapshot, &targets, false),
            Err(error) if error.code == ErrorCode::InvalidRequest
        ));
    }
    let mut overflow = world(u64::MAX);
    add_source(&mut overflow, EntityId::new(83), HarvestResource::Wood, 1);
    assert!(matches!(
        ResourceAcquisitionPlan::compile(&overflow, &[target(HarvestResource::Wood, 1)], false),
        Err(error) if error.code == ErrorCode::BudgetExceeded
    ));
    let mut bad_hash = snapshot;
    bad_hash
        .graph
        .entities
        .get_mut(&LEDGER)
        .ok_or_else(bounded)?
        .label = "changed".to_owned();
    assert!(matches!(
        ResourceAcquisitionPlan::compile(&bad_hash, &[target(HarvestResource::Wood, 1)], false),
        Err(error) if error.code == ErrorCode::ChecksumMismatch
    ));
    Ok(())
}

#[test]
fn source_generation_is_retained_and_completed_work_cannot_hide_a_replaced_ledger() -> Result<()> {
    let mut snapshot = world(40);
    // The harvest model retains any positive original ledger generation. The
    // independent provisions model deliberately admits only generation one;
    // keep this identity test in the harvest model it is exercising.
    snapshot
        .graph
        .entities
        .get_mut(&effects::fortress_settings_entity_id(snapshot.fortress_id))
        .ok_or_else(bounded)?
        .fields
        .remove(effects::LABORATORY_PROVISIONS_MODEL_FIELD);
    snapshot
        .graph
        .entities
        .get_mut(&WOOD)
        .ok_or_else(bounded)?
        .generation = 7;
    snapshot
        .graph
        .entities
        .get_mut(&LEDGER)
        .ok_or_else(bounded)?
        .generation = 3;
    snapshot.refresh_hash();
    let acquisition =
        ResourceAcquisitionPlan::compile(&snapshot, &[target(HarvestResource::Wood, 1)], false)?;
    assert_eq!(acquisition.ledger_generation, 3);
    assert_eq!(acquisition.allocations[0].source_generation, 7);
    let plan = prepare(&snapshot, &acquisition)?;
    let step = &plan.steps[0];
    apply(&mut snapshot, &step.action, &step.idempotency_key)?;
    advance(&mut snapshot, 50)?;
    effects::validate_harvest_completion(
        &snapshot,
        effects::created_entity_id(&step.idempotency_key, 0),
        &step.action,
    )?;
    snapshot
        .graph
        .entities
        .get_mut(&LEDGER)
        .ok_or_else(bounded)?
        .generation += 1;
    snapshot.refresh_hash();
    assert!(
        effects::validate_harvest_completion(
            &snapshot,
            effects::created_entity_id(&step.idempotency_key, 0),
            &step.action,
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn split_waits_and_shared_worker_service_preserve_the_same_resource_outcome() -> Result<()> {
    let source = world(40);
    let acquisition = ResourceAcquisitionPlan::compile(
        &source,
        &[
            target(HarvestResource::Wood, 4),
            target(HarvestResource::Plant, 4),
        ],
        false,
    )?;
    let plan = prepare(&source, &acquisition)?;
    let mut source = source;
    for step in &plan.steps {
        apply(&mut source, &step.action, &step.idempotency_key)?;
    }
    let mut whole = source.clone();
    let mut split = source;
    advance(&mut whole, 400)?;
    for ticks in [17, 83, 119, 181] {
        advance(&mut split, ticks)?;
    }
    assert_eq!(
        effects::harvest_report(&whole)?,
        effects::harvest_report(&split)?
    );
    for resource in [HarvestResource::Wood, HarvestResource::Plant] {
        assert_eq!(stock(&whole, resource)?, 4);
        assert_eq!(stock(&whole, resource)?, stock(&split, resource)?);
    }
    let totals = effects::harvest_report(&whole)?
        .ok_or_else(bounded)?
        .sources
        .into_iter()
        .map(|source| (source.resource, source.harvested))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(totals.get(&HarvestResource::Wood), Some(&4));
    assert_eq!(totals.get(&HarvestResource::Plant), Some(&4));
    Ok(())
}
