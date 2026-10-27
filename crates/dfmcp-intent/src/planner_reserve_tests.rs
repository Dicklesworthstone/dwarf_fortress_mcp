//! Typed reserve admission is limited to explicit laboratory production; it
//! changes neither the original terminal goal nor ordinary planner admission.

use super::*;
use dfmcp_core::{
    CapabilityGrant, CapabilityScope, Digest32, FortressId, IntentId, MapCoord, ObservationCursor,
    RequestId, SessionId, WorkBudget,
};
use dfmcp_world::{CompareOp, EntityKind, EntityRecord, Fact, FactSource, Value, WorldGraph};

const LEDGER: EntityId = EntityId::new(920);

fn source() -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(920),
        GameTick(1),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    snapshot.graph.entities.insert(
        LEDGER,
        EntityRecord {
            id: LEDGER,
            generation: 1,
            revision: 1,
            kind: EntityKind::Other(crate::effects::STOCK_LEDGER_KIND.to_owned()),
            label: "reserve admission stock".to_owned(),
            fields: [(
                crate::effects::STOCK_DRINK_FIELD.to_owned(),
                Fact::known(
                    Value::U64(40),
                    snapshot.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            )]
            .into_iter()
            .collect(),
        },
    );
    snapshot.refresh_hash();
    snapshot
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(920),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: vec![CapabilityGrant {
            capability: Capability::Plan,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
            expires_at_tick: None,
            remaining_uses: None,
        }],
        cancellation_requested: false,
    }
}

fn requested(action: Action) -> RequestedAction {
    RequestedAction {
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on: Vec::new(),
    }
}

fn reserve_intent(snapshot: &WorldSnapshot, until: u64) -> Intent {
    Intent {
        id: IntentId::new(920),
        anchor: snapshot.anchor(),
        summary: "produce explicit future drink reserves".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: LEDGER,
            field: crate::effects::STOCK_DRINK_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(40),
        },
        constraints: vec![Constraint::LaboratoryProductionReserveUntil(GameTick(
            until,
        ))],
        requested_actions: vec![requested(Action::CreateWorkOrder {
            name: "brew reserve".to_owned(),
            job_token: "BREW_DRINK".to_owned(),
            amount: 3,
            conditions: Vec::new(),
        })],
    }
}

#[test]
fn typed_reserve_admits_real_work_without_rewriting_original_goal_or_proof() -> Result<()> {
    let snapshot = source();
    let intent = reserve_intent(&snapshot, 2401);
    let planner = StaticPlanner::default();
    let context = context(&snapshot);
    let evidence = PredicateEvidence::laboratory(&snapshot)?;
    assert!(evidence.establishes(&intent.terminal_condition)?);
    let plan = planner.prepare_laboratory(&snapshot, &intent, &context)?;
    assert_eq!(plan.terminal_condition, intent.terminal_condition);
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(plan.steps[0].action, intent.requested_actions[0].action);
    assert!(!plan.steps[0].postconditions.is_empty());
    assert!(plan.steps[0].obligation.as_ref().is_some_and(|o| {
        o.deadline_tick.0 < 2401 && !evidence.establishes(&o.terminal).unwrap_or(true)
    }));
    assert!(
        plan.summary
            .ends_with("[laboratory production reserve until 2401]")
    );
    plan.validate_structure()?;
    let (archive, seal) = crate::plan_archive::encode(&plan, Default::default())?;
    assert_eq!(
        crate::plan_archive::decode(&archive, seal, Default::default())?,
        plan
    );

    let later =
        planner.prepare_laboratory(&snapshot, &reserve_intent(&snapshot, 2501), &context)?;
    assert_eq!(later.steps, plan.steps);
    assert_eq!(later.terminal_condition, plan.terminal_condition);
    assert_ne!(
        later.digest, plan.digest,
        "typed horizon must be independently sealed"
    );
    Ok(())
}

#[test]
fn reserve_marker_cannot_enable_generic_untrusted_or_summary_inferred_admission() -> Result<()> {
    let snapshot = source();
    let planner = StaticPlanner::default();
    let intent = reserve_intent(&snapshot, 2401);
    let context = context(&snapshot);
    assert!(
        planner
            .prepare(&snapshot, &intent, &context)
            .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
    );
    for evidence in [
        PredicateEvidence::untrusted(&snapshot)?,
        PredicateEvidence::laboratory(&snapshot)?,
    ] {
        assert!(
            planner
                .prepare_with_evidence(&evidence, &intent, &context)
                .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
        );
    }
    let mut ordinary = intent.clone();
    ordinary.constraints.clear();
    assert!(
        planner
            .prepare_laboratory(&snapshot, &ordinary, &context)
            .is_err_and(
                |e| e.code == ErrorCode::InvalidIntent && e.message.contains("already satisfied")
            )
    );
    ordinary
        .summary
        .push_str(" [laboratory production reserve until 2401]");
    assert!(
        planner
            .prepare_laboratory(&snapshot, &ordinary, &context)
            .is_err_and(
                |e| e.code == ErrorCode::InvalidIntent && e.message.contains("already satisfied")
            )
    );
    Ok(())
}

#[test]
fn reserve_horizon_rejects_duplicates_past_unbounded_and_overflowing_values() -> Result<()> {
    let snapshot = source();
    let planner = StaticPlanner::default();
    let context = context(&snapshot);
    for until in [
        0,
        1,
        2 + crate::effects::MAX_DEFAULT_OBLIGATION_TICKS,
        u64::MAX,
    ] {
        assert!(
            planner
                .prepare_laboratory(&snapshot, &reserve_intent(&snapshot, until), &context)
                .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
        );
    }
    for other in [2401, 2501] {
        let mut intent = reserve_intent(&snapshot, 2401);
        intent
            .constraints
            .push(Constraint::LaboratoryProductionReserveUntil(GameTick(
                other,
            )));
        assert!(
            planner
                .prepare_laboratory(&snapshot, &intent, &context)
                .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
        );
    }
    assert!(
        planner
            .prepare_laboratory(
                &snapshot,
                &reserve_intent(&snapshot, 1 + crate::effects::MAX_DEFAULT_OBLIGATION_TICKS),
                &context
            )
            .is_ok()
    );
    Ok(())
}

#[test]
fn reserve_admission_rejects_unregistered_actions_and_setup_without_production() -> Result<()> {
    let snapshot = source();
    let planner = StaticPlanner::default();
    let context = context(&snapshot);
    let position = MapCoord { x: 0, y: 0, z: 0 };
    for action in [
        Action::Pause { paused: true },
        Action::CreateWorkOrder {
            name: "unknown recipe".to_owned(),
            job_token: "UNREGISTERED".to_owned(),
            amount: 1,
            conditions: Vec::new(),
        },
        Action::SetLabor {
            units: vec![EntityId::new(1)],
            labor: "MINE".to_owned(),
            enabled: true,
        },
        Action::SetLabor {
            units: vec![EntityId::new(1)],
            labor: "BREW".to_owned(),
            enabled: false,
        },
        Action::Build {
            kind: BuildingKind::Workshop("Masons".to_owned()),
            location: position,
            footprint: MapCuboid {
                min: position,
                max: position,
            },
            material: Default::default(),
        },
    ] {
        let mut intent = reserve_intent(&snapshot, 2401);
        intent.requested_actions.push(requested(action));
        assert!(
            planner
                .prepare_laboratory(&snapshot, &intent, &context)
                .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
        );
    }
    let mut only_setup = reserve_intent(&snapshot, 2401);
    only_setup.requested_actions = vec![requested(Action::SetLabor {
        units: vec![EntityId::new(1)],
        labor: "BREW".to_owned(),
        enabled: true,
    })];
    assert!(
        planner
            .prepare_laboratory(&snapshot, &only_setup, &context)
            .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
    );
    Ok(())
}

#[test]
fn reserve_admission_preserves_plan_authority_risk_and_budget_bounds() -> Result<()> {
    let snapshot = source();
    let planner = StaticPlanner::default();
    let intent = reserve_intent(&snapshot, 2401);
    let mut denied = context(&snapshot);
    denied.grants.clear();
    assert!(
        planner
            .prepare_laboratory(&snapshot, &intent, &denied)
            .is_err_and(|e| e.code == ErrorCode::CapabilityDenied)
    );
    let mut bounded = context(&snapshot);
    bounded.budget.max_actions = 1;
    let mut two_orders = intent.clone();
    two_orders
        .requested_actions
        .push(requested(Action::CreateWorkOrder {
            name: "cook reserve".to_owned(),
            job_token: "PREPARE_MEAL".to_owned(),
            amount: 1,
            conditions: Vec::new(),
        }));
    assert!(
        planner
            .prepare_laboratory(&snapshot, &two_orders, &bounded)
            .is_err_and(|e| e.code == ErrorCode::BudgetExceeded)
    );
    let mut guarded = intent.clone();
    guarded
        .constraints
        .push(Constraint::MaxRisk(RiskTier::ReadOnly));
    assert!(
        planner
            .prepare_laboratory(&snapshot, &guarded, &context(&snapshot))
            .is_err()
    );
    let mut oversized = intent;
    oversized.summary = "r".repeat(planner.policy.max_string_bytes);
    assert!(
        planner
            .prepare_laboratory(&snapshot, &oversized, &context(&snapshot))
            .is_err_and(|e| e.code == ErrorCode::InvalidIntent)
    );
    Ok(())
}

#[test]
fn reserve_admission_accepts_only_qualified_established_production_stock_minima() -> Result<()> {
    let mut snapshot = source();
    let fields = [
        crate::effects::STOCK_DRINK_FIELD,
        crate::effects::STOCK_FOOD_FIELD,
        crate::effects::STOCK_BED_FIELD,
        crate::effects::STOCK_DOOR_FIELD,
        crate::effects::STOCK_TABLE_FIELD,
        crate::effects::STOCK_CHAIR_FIELD,
    ];
    let ledger = snapshot.graph.entities.get_mut(&LEDGER).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "reserve ledger fixture absent",
        )
    })?;
    for field in fields {
        ledger.fields.insert(
            field.to_owned(),
            Fact::known(
                Value::U64(40),
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    }
    let mut wrong_kind = ledger.clone();
    wrong_kind.id = EntityId::new(921);
    wrong_kind.kind = EntityKind::Unit;
    snapshot.graph.entities.insert(wrong_kind.id, wrong_kind);
    snapshot.refresh_hash();
    let planner = StaticPlanner::default();
    let field_goal = |entity_id, field: &str, op, value| Predicate::FieldCompare {
        entity_id,
        field: field.to_owned(),
        op,
        value,
    };
    let valid = reserve_intent(&snapshot, 2401).terminal_condition;
    for field in fields {
        let mut intent = reserve_intent(&snapshot, 2401);
        intent.terminal_condition = field_goal(LEDGER, field, CompareOp::Ge, Value::U64(40));
        assert!(
            planner
                .prepare_laboratory(&snapshot, &intent, &context(&snapshot))
                .is_ok()
        );
    }
    let mut conjunction = reserve_intent(&snapshot, 2401);
    conjunction.terminal_condition = Predicate::All(vec![
        valid.clone(),
        Predicate::All(vec![field_goal(
            LEDGER,
            crate::effects::STOCK_FOOD_FIELD,
            CompareOp::Ge,
            Value::U64(40),
        )]),
    ]);
    let plan = planner.prepare_laboratory(&snapshot, &conjunction, &context(&snapshot))?;
    assert_eq!(
        plan.terminal_condition,
        conjunction.terminal_condition.normalized()
    );

    for goal in [
        Predicate::Paused(false),
        Predicate::All(Vec::new()),
        Predicate::All(vec![valid.clone(), Predicate::All(Vec::new())]),
        Predicate::Any(vec![valid.clone()]),
        Predicate::Not(Box::new(Predicate::Paused(true))),
        Predicate::EntityKind {
            entity_id: LEDGER,
            kind: EntityKind::Other(crate::effects::STOCK_LEDGER_KIND.to_owned()),
        },
        field_goal(
            LEDGER,
            crate::effects::STOCK_PLANT_FIELD,
            CompareOp::Ge,
            Value::U64(0),
        ),
        field_goal(
            LEDGER,
            crate::effects::STOCK_DRINK_FIELD,
            CompareOp::Eq,
            Value::U64(40),
        ),
        field_goal(
            LEDGER,
            crate::effects::STOCK_DRINK_FIELD,
            CompareOp::Ge,
            Value::I64(40),
        ),
        field_goal(
            EntityId::new(921),
            crate::effects::STOCK_DRINK_FIELD,
            CompareOp::Ge,
            Value::U64(40),
        ),
        field_goal(
            EntityId::new(999),
            crate::effects::STOCK_DRINK_FIELD,
            CompareOp::Ge,
            Value::U64(0),
        ),
    ] {
        let mut intent = reserve_intent(&snapshot, 2401);
        intent.terminal_condition = goal;
        assert!(
            planner
                .prepare_laboratory(&snapshot, &intent, &context(&snapshot))
                .is_err_and(|error| error.code == ErrorCode::InvalidIntent),
            "accepted {:?}",
            intent.terminal_condition
        );
    }
    let mut unmet = reserve_intent(&snapshot, 2401);
    unmet.terminal_condition = field_goal(
        LEDGER,
        crate::effects::STOCK_DRINK_FIELD,
        CompareOp::Ge,
        Value::U64(41),
    );
    assert!(
        planner
            .prepare_laboratory(&snapshot, &unmet, &context(&snapshot))
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
    );
    unmet.constraints.clear();
    assert!(
        planner
            .prepare_laboratory(&snapshot, &unmet, &context(&snapshot))
            .is_ok(),
        "ordinary unmet production must retain its previous path"
    );

    let mut unqualified = snapshot.clone();
    let fact = unqualified
        .graph
        .entities
        .get_mut(&LEDGER)
        .and_then(|ledger| ledger.fields.get_mut(crate::effects::STOCK_DRINK_FIELD))
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "reserve stock fixture absent",
            )
        })?;
    fact.source = FactSource::Derived("unqualified-reserve-stock".to_owned());
    unqualified.refresh_hash();
    assert!(
        planner
            .prepare_laboratory(
                &unqualified,
                &reserve_intent(&unqualified, 2401),
                &context(&unqualified)
            )
            .is_err_and(|error| error.code == ErrorCode::InvalidIntent)
    );
    Ok(())
}
