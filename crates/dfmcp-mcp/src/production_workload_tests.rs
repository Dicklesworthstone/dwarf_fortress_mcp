//! Shared-fortress production queue regressions using the actual compiler,
//! planner, finite-capacity reference timeline and MemoryAdapter.

use super::*;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, IntentId, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Constraint, Intent, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{FactPresence, PredicateEvidence, PredicateTruth};

const OLD_ORDER: EntityId = EntityId::new(91);

fn world() -> Result<WorldSnapshot> {
    let mut snapshot = scenario_snapshot("starter_fortress", FortressId::new(902), false)?;
    for unit in snapshot
        .graph
        .entities
        .values_mut()
        .filter(|e| e.kind == EntityKind::Unit)
    {
        for labor in ["BREW", "COOK"] {
            unit.fields.insert(
                format!("labor.{labor}"),
                lab_fact(Value::Bool(unit.id == EntityId::new(starter::FIRST_DWARF))),
            );
        }
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn order(
    snapshot: &mut WorldSnapshot,
    id: EntityId,
    job: &str,
    remaining: u64,
    partial: u64,
) -> Result<()> {
    snapshot.graph.entities.insert(
        id,
        record(
            id,
            EntityKind::WorkOrder,
            "earlier physical work",
            vec![
                (
                    effects::STATUS_FIELD,
                    Value::Text(effects::STATUS_ACTIVE.to_owned()),
                ),
                ("job_token", Value::Text(job.to_owned())),
                (
                    effects::WORK_ORDER_NAME_FIELD,
                    Value::Text(format!("existing {}", id.get())),
                ),
                (
                    effects::WORK_ORDER_CONDITIONS_FIELD,
                    effects::work_order_conditions_value(&[])?,
                ),
                ("amount_total", Value::U64(remaining)),
                (effects::AMOUNT_REMAINING_FIELD, Value::U64(remaining)),
                ("work_ticks", Value::U64(partial)),
            ],
        ),
    );
    snapshot.refresh_hash();
    Ok(())
}

fn field(snapshot: &mut WorldSnapshot, id: EntityId, key: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("fixture entity missing"))?
        .fields
        .insert(key.to_owned(), lab_fact(value));
    snapshot.refresh_hash();
    Ok(())
}

fn request(item: &str, minimum: u32) -> Result<ProductionRequest> {
    ProductionRequest::parse(
        &json!({
            "template":"production", "quotas":[{"item":item,"minimum":minimum}],
        })
        .to_string(),
    )
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

fn intent(snapshot: &WorldSnapshot, compiled: &ProductionCompilation) -> Result<Intent> {
    Ok(Intent {
        id: IntentId::new(902),
        anchor: snapshot.anchor(),
        summary: "meet the original reserve after existing physical production".to_owned(),
        terminal_condition: compiled.terminal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: parse_steps(&compiled.actions)?,
    })
}

fn seal(snapshot: &WorldSnapshot, compiled: &ProductionCompilation) -> Result<PreparedPlan> {
    let mut intent = intent(snapshot, compiled)?;
    compiled.apply_capacity_horizon(&mut intent)?;
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}

#[test]
fn no_backlog_preserves_exact_source_actions_intent_and_seal() -> Result<()> {
    let mut source = world()?;
    // Unrelated epistemic holes must not become a new workload refusal.
    source.graph.entities.insert(
        EntityId::new(92),
        record(
            EntityId::new(92),
            EntityKind::Other("unrelated".to_owned()),
            "unrelated",
            vec![],
        ),
    );
    source.refresh_hash();
    let original = request("FOOD", 65)?;
    let source_json = original.canonical_json();
    let compiled = original.compile(&source)?;
    assert!(compiled.analysis.get("capacity").is_none());
    assert_eq!(
        compiled.actions,
        r#"[{"action":{"amount":1,"conditions":[{"item_token":"FOOD","kind":"item_count_below","threshold":65}],"job_token":"PREPARE_MEAL","kind":"create_work_order","name":"food for quota"},"depends_on":[]}]"#
    );
    let before = intent(&source, &compiled)?;
    let mut after = before.clone();
    compiled.apply_capacity_horizon(&mut after)?;
    assert_eq!(after, before);
    let previous =
        StaticPlanner::default().prepare_laboratory(&source, &before, &context(&source))?;
    assert_eq!(seal(&source, &compiled)?, previous);
    assert_eq!(
        ProductionRequest::parse(&source_json)?.canonical_json(),
        source_json
    );
    Ok(())
}

#[test]
fn queued_short_order_survives_actual_existing_work_in_both_role_directions() -> Result<()> {
    for (prior_job, item, minimum) in [("BREW_DRINK", "FOOD", 65), ("COOK_MEAL", "DRINK", 45)] {
        let mut source = world()?;
        order(&mut source, OLD_ORDER, prior_job, 20, 0)?;
        let before = source.anchor();
        let compiled = request(item, minimum)?.compile(&source)?;
        assert_eq!(source.anchor(), before);
        assert_eq!(
            compiled.analysis["capacity"]["remaining_registered_service_ticks"],
            1000
        );
        assert_eq!(
            compiled.analysis["capacity"]["queued_service_allowance_ticks"],
            2000
        );
        assert_eq!(
            compiled.analysis["capacity"]["future_output_counted_as_stock"],
            false
        );
        let plan = seal(&source, &compiled)?;
        let deadline = plan.steps[0]
            .obligation
            .as_ref()
            .ok_or_else(|| invalid("missing obligation"))?
            .deadline_tick;
        assert_eq!(deadline.0, source.tick.0 + 2200);
        let isolated = StaticPlanner::default().prepare_laboratory(
            &source,
            &intent(&source, &compiled)?,
            &context(&source),
        )?;
        assert_ne!(isolated.digest, plan.digest);
        let mut old = MemoryAdapter::new(source.clone());
        let prepared = old.prepare(&isolated, &context(old.snapshot()))?;
        let receipt = old.commit(&isolated, &prepared, &context(old.snapshot()))?;
        old.advance_ticks(250)?;
        assert_eq!(
            old.poll_action(receipt.actions[0].action_id, &context(old.snapshot()))?
                .state,
            CommitState::Failed
        );

        let mut adapter = MemoryAdapter::new(source);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
        let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
        let action = receipt.actions[0].action_id;
        adapter.advance_ticks(250)?;
        assert_eq!(
            adapter
                .poll_action(action, &context(adapter.snapshot()))?
                .state,
            CommitState::AppliedAwaitingVerification
        );
        adapter.advance_ticks(800)?;
        assert_eq!(
            adapter
                .poll_action(action, &context(adapter.snapshot()))?
                .state,
            CommitState::Verified
        );
        assert_eq!(
            PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
            PredicateTruth::True
        );
    }
    Ok(())
}

#[test]
fn earned_partial_service_and_replay_use_current_work_without_changing_original_quota() -> Result<()>
{
    let mut source = world()?;
    order(&mut source, OLD_ORDER, "BREW_DRINK", 20, 49)?;
    let original = request("FOOD", 65)?;
    let canonical = original.canonical_json();
    let first = original.compile(&source)?;
    let first_plan = seal(&source, &first)?;
    assert_eq!(
        first.analysis["capacity"]["remaining_registered_service_ticks"],
        951
    );
    assert_eq!(
        first.analysis["capacity"]["queued_service_allowance_ticks"],
        1902
    );
    assert_eq!(
        first.analysis["capacity"]["existing_orders"][0]["entity_id"],
        "91"
    );
    let mut adapter = MemoryAdapter::new(source);
    adapter.advance_ticks(101)?;
    let replayed = ProductionRequest::parse(&canonical)?.compile(adapter.snapshot())?;
    assert_eq!(replayed.terminal, first.terminal);
    assert_eq!(replayed.actions, first.actions);
    assert_eq!(
        replayed.analysis["capacity"]["remaining_registered_service_ticks"],
        850
    );
    assert_ne!(
        seal(adapter.snapshot(), &replayed)?.digest,
        first_plan.digest
    );
    assert_eq!(original.canonical_json(), canonical);
    Ok(())
}

#[test]
fn active_same_output_orders_are_refused_even_when_currently_condition_blocked() -> Result<()> {
    for (job, item, minimum) in [
        ("BREW_DRINK", "DRINK", 45),
        ("COOK_MEAL", "FOOD", 65),
        ("PREPARE_MEAL", "FOOD", 65),
    ] {
        let mut source = world()?;
        order(&mut source, OLD_ORDER, job, 10, 7)?;
        field(
            &mut source,
            OLD_ORDER,
            effects::WORK_ORDER_CONDITIONS_FIELD,
            effects::work_order_conditions_value(&[WorkOrderCondition::ItemCountBelow {
                item_token: item.to_owned(),
                threshold: 1,
            }])?,
        )?;
        let before = source.clone();
        let error = request(item, minimum)?
            .compile(&source)
            .err()
            .ok_or_else(|| invalid("overlap unexpectedly accepted"))?;
        assert_eq!(error.code, ErrorCode::PreconditionsFailed);
        for expected in [
            "order 91",
            "status=active",
            "amount_remaining=10",
            "work_ticks=7",
            "cancel",
            "not been counted as current stock",
        ] {
            assert!(error.message.contains(expected), "{}", error.message);
        }
        assert_eq!(source, before);
    }
    Ok(())
}

#[test]
fn observed_terminal_orders_and_unregistered_job_capacity_do_not_inflate_horizons() -> Result<()> {
    for status in [effects::STATUS_COMPLETE, effects::STATUS_CANCELLED] {
        let mut source = world()?;
        order(&mut source, OLD_ORDER, "PREPARE_MEAL", 10, 0)?;
        field(
            &mut source,
            OLD_ORDER,
            effects::STATUS_FIELD,
            Value::Text(status.to_owned()),
        )?;
        let compiled = request("FOOD", 65)?.compile(&source)?;
        assert!(compiled.analysis.get("capacity").is_none());
    }
    let mut source = world()?;
    order(&mut source, OLD_ORDER, "UNMODELED_JOB", 1000, 0)?;
    source
        .graph
        .entities
        .get_mut(&OLD_ORDER)
        .ok_or_else(|| invalid("fixture missing"))?
        .fields
        .remove("work_ticks");
    source.refresh_hash();
    assert!(
        request("FOOD", 65)?
            .compile(&source)?
            .analysis
            .get("capacity")
            .is_none()
    );
    Ok(())
}

#[test]
fn unknown_untrusted_and_future_workload_selectors_never_disappear_from_the_queue() -> Result<()> {
    let mut original = world()?;
    order(&mut original, OLD_ORDER, "BREW_DRINK", 2, 0)?;
    for key in [
        effects::STATUS_FIELD,
        "job_token",
        effects::AMOUNT_REMAINING_FIELD,
        "work_ticks",
    ] {
        for variant in 0..5 {
            let mut source = original.clone();
            let record = source
                .graph
                .entities
                .get_mut(&OLD_ORDER)
                .ok_or_else(|| invalid("fixture missing"))?;
            if variant == 0 {
                record.fields.remove(key);
            } else {
                let fact = record
                    .fields
                    .get_mut(key)
                    .ok_or_else(|| invalid("fixture field missing"))?;
                match variant {
                    1 => fact.presence = Some(FactPresence::Unknown("not observed".to_owned())),
                    2 => fact.source = FactSource::AgentAssertion("not evidence".to_owned()),
                    3 => fact.observed_at = GameTick(2),
                    _ => fact.source = FactSource::Replay,
                }
            }
            source.refresh_hash();
            assert!(
                request("FOOD", 65)?
                    .compile(&source)
                    .is_err_and(|error| error.code == ErrorCode::PreconditionsFailed),
                "{key}/{variant}"
            );
        }
    }
    Ok(())
}

#[test]
fn malformed_progress_and_oversized_service_refuse_before_horizon_publication() -> Result<()> {
    for (remaining, partial, code) in [
        (0, 0, ErrorCode::PreconditionsFailed),
        (1, 50, ErrorCode::PreconditionsFailed),
        (u64::MAX, 0, ErrorCode::BudgetExceeded),
        (5000, 0, ErrorCode::BudgetExceeded),
    ] {
        let mut source = world()?;
        order(&mut source, OLD_ORDER, "BREW_DRINK", remaining, partial)?;
        assert!(
            request("FOOD", 65)?
                .compile(&source)
                .is_err_and(|error| error.code == code)
        );
    }
    let mut source = world()?;
    order(&mut source, OLD_ORDER, "BREW_DRINK", 4031, 0)?;
    let compiled = request("FOOD", 65)?.compile(&source)?;
    let mut requested = intent(&source, &compiled)?;
    let before = requested.clone();
    assert!(
        compiled
            .apply_capacity_horizon(&mut requested)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    assert_eq!(
        requested, before,
        "no partially adjusted program after refusal"
    );
    Ok(())
}

#[test]
fn workload_and_presentation_are_bounded_before_growing_order_state() -> Result<()> {
    let mut source = world()?;
    for ordinal in 0..128 {
        order(
            &mut source,
            EntityId::new(10_000 + ordinal),
            "BREW_DRINK",
            1,
            0,
        )?;
    }
    let compiled = request("FOOD", 65)?.compile(&source)?;
    assert_eq!(
        compiled.analysis["capacity"]["observed_existing_order_count"],
        128
    );
    assert_eq!(
        compiled.analysis["capacity"]["existing_orders"]
            .as_array()
            .map(Vec::len),
        Some(16)
    );
    assert_eq!(
        compiled.analysis["capacity"]["existing_orders_omitted"],
        112
    );
    order(&mut source, EntityId::new(20_000), "BREW_DRINK", 1, 0)?;
    assert!(
        request("FOOD", 65)?
            .compile(&source)
            .is_err_and(|error| error.code == ErrorCode::BudgetExceeded)
    );
    Ok(())
}

#[test]
fn capacity_allowance_is_bound_to_the_exact_original_source_and_program() -> Result<()> {
    let mut source = world()?;
    order(&mut source, OLD_ORDER, "BREW_DRINK", 10, 0)?;
    let compiled = request("FOOD", 65)?.compile(&source)?;
    for variant in 0..2 {
        let mut requested = intent(&source, &compiled)?;
        if variant == 0 {
            requested.anchor.tick = GameTick(2);
        } else if let Action::CreateWorkOrder { amount, .. } =
            &mut requested.requested_actions[0].action
        {
            *amount = 2;
        }
        let before = requested.clone();
        assert!(
            compiled
                .apply_capacity_horizon(&mut requested)
                .is_err_and(|error| error.code == ErrorCode::StaleAnchor)
        );
        assert_eq!(requested, before);
    }
    Ok(())
}

#[test]
fn capacity_horizon_preserves_setup_dependencies_and_exact_created_identity() -> Result<()> {
    let mut source = world()?;
    source.graph.entities.remove(&starter::KITCHEN);
    order(&mut source, OLD_ORDER, "BREW_DRINK", 30, 0)?;
    let original = ProductionRequest::parse(
        &json!({
            "template":"production", "quotas":[{"item":"FOOD","minimum":65}],
            "prerequisites":{"workshops":[{"building":"workshop:Kitchen","location":[0,0,10]}]},
        })
        .to_string(),
    )?;
    let compiled = original.compile(&source)?;
    let plan = seal(&source, &compiled)?;
    assert_eq!(plan.steps.len(), 2);
    assert_eq!(plan.steps[1].depends_on, vec![dfmcp_core::StepId::new(0)]);
    let build_deadline = plan.steps[0]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("missing build obligation"))?
        .deadline_tick;
    let production_deadline = plan.steps[1]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("missing order obligation"))?
        .deadline_tick;
    assert_eq!(build_deadline.0, source.tick.0 + effects::BUILD_TICKS * 4);
    assert_eq!(production_deadline.0, source.tick.0 + 5200);
    for step in &plan.steps {
        assert_eq!(
            Predicate::All(step.postconditions.clone()).normalized(),
            Predicate::All(effects::default_postconditions(
                &step.action,
                &step.idempotency_key,
                source.fortress_id
            ))
            .normalized()
        );
        assert_eq!(
            step.obligation
                .as_ref()
                .ok_or_else(|| invalid("missing obligation"))?
                .terminal,
            Predicate::All(step.postconditions.clone()).normalized()
        );
    }
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert_eq!(receipt.actions[1].state, CommitState::Prepared);
    adapter.advance_ticks(500)?;
    assert_eq!(
        adapter
            .poll_action(receipt.actions[0].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::AppliedAwaitingVerification
    );
    adapter.advance_ticks(1050)?;
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    Ok(())
}

#[test]
fn backlog_planning_retains_initially_satisfied_quota_and_current_effect_authority() -> Result<()> {
    let mut source = world()?;
    order(&mut source, OLD_ORDER, "BREW_DRINK", 10, 0)?;
    let original = ProductionRequest::parse(&json!({
        "template":"production", "quotas":[{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}],
    }).to_string())?;
    let compiled = original.compile(&source)?;
    assert!(matches!(&compiled.terminal, Predicate::All(clauses) if clauses.len() == 2));
    let plan = seal(&source, &compiled)?;
    let mut adapter = MemoryAdapter::new(source);
    let mut observer = context(adapter.snapshot());
    observer
        .grants
        .retain(|grant| grant.capability != Capability::ConfigureProduction);
    assert!(
        adapter
            .prepare(&plan, &observer)
            .is_err_and(|error| error.code == ErrorCode::CapabilityDenied)
    );
    assert_eq!(
        adapter
            .snapshot()
            .graph
            .entities
            .values()
            .filter(|entity| entity.kind == EntityKind::WorkOrder)
            .count(),
        1
    );
    Ok(())
}

#[test]
fn late_setup_observation_cannot_spend_a_condition_blocked_orders_queue_allowance() -> Result<()> {
    let mut source = world()?;
    source.graph.entities.remove(&starter::KITCHEN);
    order(&mut source, OLD_ORDER, "BREW_DRINK", 20, 0)?;
    // Brewing starts only when the first food meal drops food below sixty.
    // That happens after the caller finally observes the completed kitchen.
    field(
        &mut source,
        OLD_ORDER,
        effects::WORK_ORDER_CONDITIONS_FIELD,
        effects::work_order_conditions_value(&[WorkOrderCondition::ItemCountBelow {
            item_token: "FOOD".to_owned(),
            threshold: 60,
        }])?,
    )?;
    field(
        &mut source,
        starter::STOCK_LEDGER,
        effects::METABOLISM_TICKS_FIELD,
        Value::U64(450),
    )?;
    let compiled = ProductionRequest::parse(
        &json!({
            "template":"production", "quotas":[{"item":"FOOD","minimum":110}],
            "prerequisites":{"workshops":[{"building":"workshop:Kitchen","location":[0,0,10]}]},
        })
        .to_string(),
    )?
    .compile(&source)?;
    let plan = seal(&source, &compiled)?;
    let mut adapter = MemoryAdapter::new(source);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    adapter.advance_ticks(1900)?;
    assert_eq!(
        adapter
            .poll_action(receipt.actions[0].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::AppliedAwaitingVerification
    );
    adapter.advance_ticks(1500)?;
    assert_eq!(
        adapter
            .poll_action(receipt.actions[1].action_id, &context(adapter.snapshot()))?
            .state,
        CommitState::Verified
    );
    let deadline = plan.steps[1]
        .obligation
        .as_ref()
        .ok_or_else(|| invalid("missing production horizon"))?
        .deadline_tick;
    assert_eq!(deadline.0, 5101);
    assert!(
        adapter.snapshot().tick.0 > 3101,
        "max(setup horizon, source+allowance) would have expired"
    );
    // Food consumed during the wait remains separate from work-order proof.
    assert_eq!(
        PredicateEvidence::laboratory(adapter.snapshot())?.evaluate(&compiled.terminal)?,
        PredicateTruth::False
    );
    Ok(())
}
