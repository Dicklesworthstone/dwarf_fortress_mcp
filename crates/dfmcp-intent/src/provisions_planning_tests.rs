use super::*;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
    IntentId, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_world::{EntityRecord, Fact, FactSource, WorldGraph};

use crate::{Constraint, Intent, ObligationSpec, PlanPolicy, StaticPlanner};

const LEDGER: EntityId = EntityId::new(91);

fn put(snapshot: &mut WorldSnapshot, id: EntityId, kind: EntityKind, fields: Vec<(&str, Value)>) {
    snapshot.graph.entities.insert(
        id,
        EntityRecord {
            id,
            generation: 1,
            revision: 1,
            kind,
            label: "provisions planning fixture".to_owned(),
            fields: fields
                .into_iter()
                .map(|(field, value)| {
                    (
                        field.to_owned(),
                        Fact::known(
                            value,
                            snapshot.tick,
                            FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                            Digest32::ZERO,
                        ),
                    )
                })
                .collect(),
        },
    );
    snapshot.refresh_hash();
}

fn source(plants: u64, provisions: bool) -> WorldSnapshot {
    let mut snapshot = WorldSnapshot::new(
        FortressId::new(709),
        GameTick(1_000),
        ObservationCursor::ORIGIN,
        false,
        WorldGraph::default(),
    );
    let mut settings = vec![(
        effects::LABORATORY_WORKFORCE_MODEL_FIELD,
        Value::Text(effects::LABORATORY_WORKFORCE_MODEL.to_owned()),
    )];
    if provisions {
        settings.push((
            effects::LABORATORY_PROVISIONS_MODEL_FIELD,
            Value::Text(effects::LABORATORY_PROVISIONS_MODEL.to_owned()),
        ));
    }
    let settings_id = effects::fortress_settings_entity_id(snapshot.fortress_id);
    put(
        &mut snapshot,
        settings_id,
        EntityKind::Other(effects::FORTRESS_SETTINGS_KIND.to_owned()),
        settings,
    );
    put(
        &mut snapshot,
        LEDGER,
        EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        vec![
            (effects::STOCK_PLANT_FIELD, Value::U64(plants)),
            (effects::STOCK_DRINK_FIELD, Value::U64(40)),
            (effects::STOCK_FOOD_FIELD, Value::U64(60)),
            (effects::METABOLISM_TICKS_FIELD, Value::U64(0)),
        ],
    );
    snapshot
}

fn context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(709),
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

fn order(name: &str, job: &str, amount: u32) -> RequestedAction {
    RequestedAction {
        action: Action::CreateWorkOrder {
            name: name.to_owned(),
            job_token: job.to_owned(),
            amount,
            conditions: Vec::new(),
        },
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        obligation: None,
        compensation: None,
        depends_on: Vec::new(),
    }
}

fn intent(snapshot: &WorldSnapshot, actions: Vec<RequestedAction>) -> Intent {
    Intent {
        id: IntentId::new(709),
        anchor: snapshot.anchor(),
        summary: "produce from a shared finite ingredient supply".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: LEDGER,
            field: effects::STOCK_DRINK_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(45),
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: actions,
    }
}

#[test]
fn food_and_drink_cannot_each_spend_the_same_ingredient_supply() -> Result<()> {
    let planner = StaticPlanner::default();
    let actions = vec![
        order("brew", "BREW_DRINK", 2),
        order("cook", "PREPARE_MEAL", 2),
    ];
    let short = source(3, true);
    let unchanged = short.clone();
    let error = planner
        .prepare_laboratory(&short, &intent(&short, actions.clone()), &context(&short))
        .err()
        .ok_or_else(|| {
            DfmcpError::new(ErrorCode::InternalInvariantViolation, "short plan admitted")
        })?;
    assert_eq!(error.code, ErrorCode::PreconditionsFailed);
    assert!(error.message.contains("4 available PLANT"));
    assert_eq!(short, unchanged);

    let exact = source(4, true);
    let plan = planner.prepare_laboratory(&exact, &intent(&exact, actions), &context(&exact))?;
    assert_eq!(plan.steps.len(), 2);
    for step in &plan.steps {
        assert!(step.preconditions.contains(&Predicate::EntityKind {
            entity_id: LEDGER,
            kind: EntityKind::Other(effects::STOCK_LEDGER_KIND.to_owned()),
        }));
        assert!(step.preconditions.contains(&Predicate::FieldCompare {
            entity_id: LEDGER,
            field: effects::STOCK_PLANT_FIELD.to_owned(),
            op: CompareOp::Ge,
            value: Value::U64(2),
        }));
    }
    assert!(plan.digest_is_valid());
    Ok(())
}

#[test]
fn pending_food_demand_is_retained_until_original_work_is_cancelled() -> Result<()> {
    let mut snapshot = source(2, true);
    let existing = order("prior cook", "COOK_MEAL", 2).action;
    effects::apply_effect(&mut snapshot, &existing, "prior-provisions")?;
    snapshot.refresh_hash();
    let planner = StaticPlanner::default();
    assert!(
        planner
            .prepare_laboratory(
                &snapshot,
                &intent(&snapshot, vec![order("new brew", "BREW_DRINK", 1)]),
                &context(&snapshot),
            )
            .is_err()
    );
    effects::cancel_effect(&mut snapshot, &existing, "prior-provisions")?;
    snapshot.refresh_hash();
    let plan = planner.prepare_laboratory(
        &snapshot,
        &intent(&snapshot, vec![order("new brew", "BREW_DRINK", 1)]),
        &context(&snapshot),
    )?;
    assert_eq!(plan.steps.len(), 1);
    assert_eq!(
        effects::provisions_report(&snapshot)?.map(|report| report.available_plants),
        Some(2),
        "cancelling an unstarted batch must neither spend nor invent ingredients"
    );
    Ok(())
}

#[test]
fn mandatory_input_proof_preserves_an_explicit_deadline() -> Result<()> {
    let snapshot = source(2, true);
    let mut requested = order("brew", "BREW_DRINK", 2);
    requested.postconditions = vec![Predicate::True];
    requested.obligation = Some(ObligationSpec {
        terminal: Predicate::True,
        failure: None,
        deadline_tick: GameTick(1_750),
        poll_interval_ticks: 10,
        stable_for_observations: 2,
    });
    let intent = intent(&snapshot, vec![requested]);
    let planner = StaticPlanner::new(PlanPolicy::default());
    let plan = planner.prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let step = &plan.steps[0];
    let obligation = step.obligation.as_ref().ok_or_else(|| {
        DfmcpError::new(ErrorCode::InternalInvariantViolation, "obligation missing")
    })?;
    assert_eq!(obligation.deadline_tick, GameTick(1_750));
    assert_eq!(obligation.stable_for_observations, 2);
    assert_ne!(obligation.terminal, Predicate::True);
    for predicate in
        effects::provisions_completion_predicates(&snapshot, &step.action, &step.idempotency_key)?
    {
        assert!(step.postconditions.contains(&predicate.normalized()));
    }
    assert!(
        planner
            .prepare(&snapshot, &intent, &context(&snapshot))
            .is_err()
    );
    Ok(())
}

#[test]
fn no_model_keeps_legacy_order_proof_and_zero_input_planning() -> Result<()> {
    let snapshot = source(0, false);
    let plan = StaticPlanner::default().prepare_laboratory(
        &snapshot,
        &intent(&snapshot, vec![order("legacy brew", "BREW_DRINK", 3)]),
        &context(&snapshot),
    )?;
    let step = &plan.steps[0];
    let mut expected =
        effects::default_postconditions(&step.action, &step.idempotency_key, snapshot.fortress_id);
    expected = expected.iter().map(Predicate::normalized).collect();
    expected.sort_by_key(Predicate::canonical_bytes);
    expected.dedup();
    assert_eq!(step.postconditions, expected);
    assert!(!step.preconditions.iter().any(|predicate| matches!(
        predicate,
        Predicate::FieldCompare {field,..}
            if field == effects::LABORATORY_PROVISIONS_MODEL_FIELD
                || field == effects::STOCK_PLANT_FIELD
    )));
    Ok(())
}
