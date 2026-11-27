//! Physical progress must admit the complete keyed source before counting work.
//! Bead: df-predicate-evidence-authority-r7m.
//!
//! These use the public effect transition and real canonical records. In
//! particular, two map entries claiming one worker cannot supply two workers.

use std::collections::BTreeMap;

use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, EdgeId, EntityId,
    ErrorCode, FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId,
    Result, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::effects::{
    AMOUNT_REMAINING_FIELD, CONSTRUCTION_STAGE_FIELD, EffectAdvanceLimits, FORTRESS_SETTINGS_KIND,
    LABORATORY_WORKFORCE_MODEL, LABORATORY_WORKFORCE_MODEL_FIELD,
    LABORATORY_WORKFORCE_TASK_MODEL_FIELD, METABOLISM_TICKS_FIELD, STATUS_ACTIVE, STATUS_COMPLETE,
    STATUS_FIELD, STOCK_DRINK_FIELD, STOCK_FOOD_FIELD, STOCK_LEDGER_KIND,
    WORK_ORDER_CONDITIONS_FIELD, WORK_ORDER_NAME_FIELD, WORK_ORDER_TICKS_PER_UNIT,
    advance_effects_with_limits, fortress_settings_entity_id, laboratory_workforce_remaining_ticks,
    laboratory_workforce_report, work_order_conditions_value,
};
use dfmcp_intent::workforce_planning::LaboratoryWorkforcePlanning;
use dfmcp_intent::{Action, Constraint, Intent, PreparedPlan, RequestedAction, StaticPlanner};
use dfmcp_world::{
    EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Predicate, Value, WorldGraph,
    WorldSnapshot,
};

const WORKER: EntityId = EntityId::new(11);
const SECOND_WORKER: EntityId = EntityId::new(12);
const LEDGER: EntityId = EntityId::new(91);
const FIRST_ORDER: EntityId = EntityId::new(101);
const SECOND_ORDER: EntityId = EntityId::new(102);
const IDLE: EntityId = EntityId::new(900);

fn missing() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "missing source-identity fixture",
    )
}

fn record(id: EntityId, kind: EntityKind, fields: &[(&str, Value)]) -> EntityRecord {
    EntityRecord {
        id,
        generation: 1,
        revision: 1,
        label: kind.as_str().to_owned(),
        kind,
        fields: fields
            .iter()
            .map(|(name, value)| {
                (
                    (*name).to_owned(),
                    Fact::known(
                        value.clone(),
                        GameTick(1_000),
                        FactSource::Derived("dfmcp.reference-effects/1".to_owned()),
                        Digest32::ZERO,
                    ),
                )
            })
            .collect(),
    }
}

fn world() -> Result<WorldSnapshot> {
    let mut graph = WorldGraph::default();
    graph.entities.insert(
        WORKER,
        record(
            WORKER,
            EntityKind::Unit,
            &[
                ("alive", Value::Bool(true)),
                ("labor.BREW", Value::Bool(true)),
            ],
        ),
    );
    // Two legitimate workshops leave workers as the limiting resource.
    for id in [EntityId::new(51), EntityId::new(52)] {
        graph.entities.insert(
            id,
            record(
                id,
                EntityKind::Building,
                &[
                    ("building_kind", Value::Text("workshop:Still".to_owned())),
                    (
                        CONSTRUCTION_STAGE_FIELD,
                        Value::Text(STATUS_COMPLETE.to_owned()),
                    ),
                ],
            ),
        );
    }
    graph.entities.insert(
        LEDGER,
        record(
            LEDGER,
            EntityKind::Other(STOCK_LEDGER_KIND.to_owned()),
            &[
                (STOCK_DRINK_FIELD, Value::U64(0)),
                (STOCK_FOOD_FIELD, Value::U64(100)),
                (METABOLISM_TICKS_FIELD, Value::U64(0)),
            ],
        ),
    );
    for id in [FIRST_ORDER, SECOND_ORDER] {
        graph.entities.insert(
            id,
            record(
                id,
                EntityKind::WorkOrder,
                &[
                    (
                        WORK_ORDER_NAME_FIELD,
                        Value::Text(format!("brew-{}", id.get())),
                    ),
                    ("job_token", Value::Text("BREW_DRINK".to_owned())),
                    (
                        WORK_ORDER_CONDITIONS_FIELD,
                        work_order_conditions_value(&[])?,
                    ),
                    ("amount_total", Value::U64(1)),
                    (AMOUNT_REMAINING_FIELD, Value::U64(1)),
                    ("work_ticks", Value::U64(0)),
                    (STATUS_FIELD, Value::Text(STATUS_ACTIVE.to_owned())),
                ],
            ),
        );
    }
    graph.entities.insert(
        IDLE,
        record(IDLE, EntityKind::Other("unrelated_record".to_owned()), &[]),
    );
    Ok(WorldSnapshot::new(
        FortressId::new(9),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        graph,
    ))
}

fn shared_world() -> Result<WorldSnapshot> {
    let mut source = world()?;
    let settings = fortress_settings_entity_id(source.fortress_id);
    source.graph.entities.insert(
        settings,
        record(
            settings,
            EntityKind::Other(FORTRESS_SETTINGS_KIND.to_owned()),
            &[(
                LABORATORY_WORKFORCE_MODEL_FIELD,
                Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
            )],
        ),
    );
    for id in [FIRST_ORDER, SECOND_ORDER] {
        source
            .graph
            .entities
            .get_mut(&id)
            .ok_or_else(missing)?
            .fields
            .insert(
                LABORATORY_WORKFORCE_TASK_MODEL_FIELD.to_owned(),
                Fact::known(
                    Value::Text(LABORATORY_WORKFORCE_MODEL.to_owned()),
                    source.tick,
                    FactSource::Derived("dfmcp.reference-effects/1".to_owned()),
                    Digest32::ZERO,
                ),
            );
    }
    source.refresh_hash();
    Ok(source)
}

fn prepare_work(source: &WorldSnapshot) -> Result<PreparedPlan> {
    let intent = Intent {
        id: IntentId::new(71),
        anchor: source.anchor(),
        summary: "plan work against the exact current workforce".to_owned(),
        terminal_condition: Predicate::EntityExists(EntityId::new(987_654)),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::CreateWorkOrder {
                name: "additional-brew".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            obligation: None,
            compensation: None,
            depends_on: Vec::new(),
        }],
    };
    let context = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: source.anchor(),
        budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    };
    StaticPlanner::default().prepare_laboratory(source, &intent, &context)
}

fn assert_workforce_refused(
    source: &WorldSnapshot,
    case: &str,
    planning_error: ErrorCode,
) -> Result<()> {
    let before = source.clone();
    let report = laboratory_workforce_report(source);
    assert!(
        matches!(&report, Err(error) if error.code == ErrorCode::PreconditionsFailed),
        "{case}: invalid source produced workforce evidence: {report:?}",
    );
    let remaining = laboratory_workforce_remaining_ticks(source);
    assert!(
        matches!(&remaining, Err(error) if error.code == ErrorCode::PreconditionsFailed),
        "{case}: invalid source produced a remaining-work estimate: {remaining:?}",
    );
    let workload = LaboratoryWorkforcePlanning::capture(source);
    assert!(
        matches!(&workload, Err(error) if error.code == ErrorCode::PreconditionsFailed),
        "{case}: invalid source became a planning workload: {workload:?}",
    );
    // PredicateEvidence rejects mismatched lookup keys before the planner
    // reaches workforce inspection. Keep that existing error classification
    // exact; other malformed identities reach the shared source admission.
    let plan = prepare_work(source);
    assert!(
        matches!(&plan, Err(error) if error.code == planning_error),
        "{case}: invalid source became a sealed plan: {plan:?}",
    );
    assert_eq!(
        source, &before,
        "{case}: read-only inspection changed the source"
    );
    assert_refused_before_progress(source, case)
}

fn number(snapshot: &WorldSnapshot, id: EntityId, field: &str) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&id)
        .and_then(|record| record.fields.get(field))
        .and_then(Fact::known_value)
    {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(missing()),
    }
}

fn advance(source: &WorldSnapshot) -> Result<WorldSnapshot> {
    let mut shadow = source.clone();
    shadow.tick = source
        .tick
        .checked_add(WORK_ORDER_TICKS_PER_UNIT)
        .ok_or_else(missing)?;
    advance_effects_with_limits(
        &mut shadow,
        WORK_ORDER_TICKS_PER_UNIT,
        EffectAdvanceLimits::default(),
    )?;
    shadow.refresh_hash();
    Ok(shadow)
}

fn assert_refused_before_progress(source: &WorldSnapshot, case: &str) -> Result<()> {
    let mut shadow = source.clone();
    shadow.tick = source
        .tick
        .checked_add(WORK_ORDER_TICKS_PER_UNIT)
        .ok_or_else(missing)?;
    let before = shadow.clone();
    let result = advance_effects_with_limits(
        &mut shadow,
        WORK_ORDER_TICKS_PER_UNIT,
        EffectAdvanceLimits::default(),
    );
    assert!(
        matches!(&result, Err(error) if error.code == ErrorCode::PreconditionsFailed),
        "{case}: invalid source entered the timeline: {result:?}; drink stock={:?}",
        number(&shadow, LEDGER, STOCK_DRINK_FIELD)
    );
    // The whole census must precede work, not merely reject after a first
    // production event or consumption update inside the unpublished shadow.
    assert_eq!(
        shadow, before,
        "{case}: admission changed an unrelated physical effect"
    );
    Ok(())
}

#[test]
fn legitimate_single_and_distinct_workers_keep_their_actual_production_capacity() -> Result<()> {
    let one = world()?;
    let advanced = advance(&one)?;
    assert_eq!(number(&advanced, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_eq!(number(&advanced, FIRST_ORDER, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&advanced, SECOND_ORDER, AMOUNT_REMAINING_FIELD)?, 1);

    let mut two = one.clone();
    let mut worker = two.graph.entities.get(&WORKER).ok_or_else(missing)?.clone();
    worker.id = SECOND_WORKER;
    // Legitimate entity generations need not be exactly the initial one.
    worker.generation = 7;
    two.graph.entities.insert(SECOND_WORKER, worker);
    two.refresh_hash();
    let advanced = advance(&two)?;
    assert_eq!(number(&advanced, LEDGER, STOCK_DRINK_FIELD)?, 10);
    assert_eq!(number(&advanced, FIRST_ORDER, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&advanced, SECOND_ORDER, AMOUNT_REMAINING_FIELD)?, 0);
    assert_eq!(number(&one, LEDGER, STOCK_DRINK_FIELD)?, 0);
    Ok(())
}

#[test]
fn duplicate_lookup_entries_cannot_multiply_one_workers_production() -> Result<()> {
    let mut source = world()?;
    let worker = source
        .graph
        .entities
        .get(&WORKER)
        .ok_or_else(missing)?
        .clone();
    source.graph.entities.insert(SECOND_WORKER, worker);
    source.refresh_hash();
    assert_refused_before_progress(&source, "one worker under two lookup keys")
}

#[test]
fn unchanged_canonical_bytes_do_not_authorize_a_different_lookup_identity() -> Result<()> {
    let source = world()?;
    let mut aliased = source.clone();
    let idle = aliased.graph.entities.remove(&IDLE).ok_or_else(missing)?;
    aliased.graph.entities.insert(EntityId::new(901), idle);
    aliased.refresh_hash();
    assert_ne!(aliased.graph, source.graph);
    assert_eq!(aliased.canonical_bytes(), source.canonical_bytes());
    assert_eq!(aliased.anchor(), source.anchor());
    assert_refused_before_progress(&aliased, "same-anchor unrelated lookup alias")
}

#[test]
fn nil_and_zero_generation_records_refuse_before_unrelated_progress() -> Result<()> {
    for case in 0..4 {
        let mut source = world()?;
        let description = match case {
            0 => {
                source
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .ok_or_else(missing)?
                    .generation = 0;
                "zero-generation worker"
            }
            1 => {
                source
                    .graph
                    .entities
                    .get_mut(&IDLE)
                    .ok_or_else(missing)?
                    .generation = 0;
                "zero-generation unrelated entity"
            }
            2 => {
                let mut idle = source.graph.entities.remove(&IDLE).ok_or_else(missing)?;
                idle.id = EntityId::NIL;
                source.graph.entities.insert(EntityId::NIL, idle);
                "matching NIL lookup and entity identity"
            }
            _ => {
                source.graph.entities.get_mut(&IDLE).ok_or_else(missing)?.id = EntityId::NIL;
                "NIL entity behind a nonzero lookup key"
            }
        };
        source.refresh_hash();
        assert_refused_before_progress(&source, description)?;
    }
    Ok(())
}

#[test]
fn malformed_edge_identity_cannot_be_ignored_by_unrelated_physical_progress() -> Result<()> {
    let mut valid = world()?;
    let edge = EdgeRecord {
        id: EdgeId::new(701),
        revision: 1,
        kind: EdgeKind::AssignedTo,
        from: WORKER,
        to: EntityId::new(51),
        fields: BTreeMap::new(),
    };
    valid.graph.edges.insert(edge.id, edge.clone());
    valid.refresh_hash();
    assert_eq!(number(&advance(&valid)?, LEDGER, STOCK_DRINK_FIELD)?, 5);
    for case in 0..3 {
        let mut source = valid.clone();
        let description = match case {
            0 => {
                source.graph.edges.get_mut(&edge.id).ok_or_else(missing)?.id = EdgeId::new(702);
                "mismatched edge lookup identity"
            }
            1 => {
                let mut nil = source.graph.edges.remove(&edge.id).ok_or_else(missing)?;
                nil.id = EdgeId::NIL;
                source.graph.edges.insert(EdgeId::NIL, nil);
                "matching NIL lookup and edge identity"
            }
            _ => {
                source.graph.edges.insert(EdgeId::new(702), edge.clone());
                "same edge behind two lookup keys"
            }
        };
        source.refresh_hash();
        assert_refused_before_progress(&source, description)?;
    }
    Ok(())
}

#[test]
fn workforce_reports_planning_and_progress_agree_on_valid_later_generations() -> Result<()> {
    let mut source = shared_world()?;
    source
        .graph
        .entities
        .get_mut(&WORKER)
        .ok_or_else(missing)?
        .generation = 7;
    source
        .graph
        .entities
        .get_mut(&EntityId::new(51))
        .ok_or_else(missing)?
        .generation = 9;
    source.refresh_hash();
    let before = source.clone();
    let report = laboratory_workforce_report(&source)?.ok_or_else(missing)?;
    assert_eq!(report.eligible_workers, 1);
    assert_eq!(report.active_activities, 2);
    assert_eq!(report.assignments.len(), 1);
    assert_eq!(report.assignments[0].worker, WORKER);
    assert_eq!(report.assignments[0].work, FIRST_ORDER);
    assert_eq!(report.remaining_service_ticks, 2 * WORK_ORDER_TICKS_PER_UNIT);
    assert_eq!(
        laboratory_workforce_remaining_ticks(&source)?,
        Some(report.remaining_service_ticks),
    );
    let workload = LaboratoryWorkforcePlanning::capture(&source)?.ok_or_else(missing)?;
    assert_eq!(workload.queued_service_ticks, report.remaining_service_ticks);
    assert_eq!(workload.queued_activities, report.active_activities);
    assert!(prepare_work(&source)?.digest_is_valid());
    assert_eq!(number(&advance(&source)?, LEDGER, STOCK_DRINK_FIELD)?, 5);
    assert_eq!(source, before);
    Ok(())
}

#[test]
fn workforce_reports_and_plans_refuse_invalid_complete_entity_identity() -> Result<()> {
    for case in 0..7 {
        let mut source = shared_world()?;
        let description = match case {
            0 => {
                source
                    .graph
                    .entities
                    .get_mut(&WORKER)
                    .ok_or_else(missing)?
                    .generation = 0;
                "zero-generation worker cannot supply advisory capacity"
            }
            1 => {
                source
                    .graph
                    .entities
                    .get_mut(&EntityId::new(51))
                    .ok_or_else(missing)?
                    .generation = 0;
                "zero-generation workshop cannot supply advisory capacity"
            }
            2 => {
                source
                    .graph
                    .entities
                    .get_mut(&IDLE)
                    .ok_or_else(missing)?
                    .generation = 0;
                "unrelated zero-generation record is part of the complete source"
            }
            3 => {
                let mut worker = source.graph.entities.remove(&WORKER).ok_or_else(missing)?;
                worker.id = EntityId::NIL;
                source.graph.entities.insert(EntityId::NIL, worker);
                "matching NIL worker identity cannot supply advisory capacity"
            }
            4 => {
                source.graph.entities.get_mut(&IDLE).ok_or_else(missing)?.id = EntityId::NIL;
                "unrelated NIL record is part of the complete source"
            }
            5 => {
                let original = source.clone();
                let idle = source.graph.entities.remove(&IDLE).ok_or_else(missing)?;
                source.graph.entities.insert(EntityId::new(901), idle);
                assert_eq!(source.canonical_bytes(), original.canonical_bytes());
                "same-byte unrelated lookup alias cannot enter a planning census"
            }
            _ => {
                let worker = source.graph.entities.get(&WORKER).ok_or_else(missing)?.clone();
                source.graph.entities.insert(SECOND_WORKER, worker);
                "a duplicate worker cannot supply two advisory workers"
            }
        };
        source.refresh_hash();
        let planning_error = if case >= 4 {
            ErrorCode::InternalInvariantViolation
        } else {
            ErrorCode::PreconditionsFailed
        };
        assert_workforce_refused(&source, description, planning_error)?;
    }
    Ok(())
}

#[test]
fn workforce_reports_and_plans_refuse_same_hash_physical_kind_aliases() -> Result<()> {
    for id in [WORKER, EntityId::new(51), FIRST_ORDER] {
        let original = shared_world()?;
        let mut source = original.clone();
        let row = source.graph.entities.get_mut(&id).ok_or_else(missing)?;
        row.kind = EntityKind::Other(row.kind.as_str().to_owned());
        source.refresh_hash();
        assert_eq!(source.canonical_bytes(), original.canonical_bytes());
        assert_eq!(source.anchor(), original.anchor());
        assert_workforce_refused(
            &source,
            "same-anchor named-kind alias",
            ErrorCode::PreconditionsFailed,
        )?;
    }
    Ok(())
}

#[test]
fn workforce_reports_and_plans_refuse_invalid_complete_edge_identity() -> Result<()> {
    let mut valid = shared_world()?;
    let edge = EdgeRecord {
        id: EdgeId::new(701),
        revision: 1,
        kind: EdgeKind::AssignedTo,
        from: WORKER,
        to: EntityId::new(51),
        fields: BTreeMap::new(),
    };
    valid.graph.edges.insert(edge.id, edge.clone());
    valid.refresh_hash();
    assert_eq!(
        laboratory_workforce_report(&valid)?
            .ok_or_else(missing)?
            .assignments
            .len(),
        1,
    );
    assert!(prepare_work(&valid)?.digest_is_valid());
    for case in 0..3 {
        let mut source = valid.clone();
        let description = match case {
            0 => {
                source.graph.edges.get_mut(&edge.id).ok_or_else(missing)?.id = EdgeId::new(702);
                "mismatched edge lookup identity"
            }
            1 => {
                let mut nil = source.graph.edges.remove(&edge.id).ok_or_else(missing)?;
                nil.id = EdgeId::NIL;
                source.graph.edges.insert(EdgeId::NIL, nil);
                "matching NIL lookup and edge identity"
            }
            _ => {
                source.graph.edges.insert(EdgeId::new(702), edge.clone());
                "same edge behind two lookup keys"
            }
        };
        source.refresh_hash();
        let planning_error = if case == 1 {
            ErrorCode::PreconditionsFailed
        } else {
            ErrorCode::InternalInvariantViolation
        };
        assert_workforce_refused(&source, description, planning_error)?;
    }
    Ok(())
}
