//! Whole original-room goals run through finite excavation, production and
//! construction, including an ordinary authorized labor remedy and recovery.

use super::*;
use dfmcp_adapter::ActionReceipt;
use dfmcp_intent::Constraint;

#[path = "connected_room_material_tests.rs"]
mod materials;

fn workforce_source() -> Result<WorldSnapshot> {
    crate::lab_world::scenario_snapshot("starter_fortress_workforce", FortressId::new(985), false)
}

fn sealed_room(snapshot: &WorldSnapshot) -> Result<PreparedPlan> {
    let (intent, _) = request(false)?.compile(IntentId::new(985), snapshot, "")?;
    StaticPlanner::default().prepare_laboratory(snapshot, &intent, &context(snapshot))
}

fn begin(
    snapshot: WorldSnapshot,
    plan: &PreparedPlan,
) -> Result<(MemoryAdapter, Vec<ActionReceipt>)> {
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(plan, &prepared, &context(adapter.snapshot()))?;
    Ok((adapter, receipt.actions))
}

fn advance(adapter: &mut MemoryAdapter, actions: &mut [ActionReceipt], ticks: u64) -> Result<()> {
    adapter.advance_ticks(ticks)?;
    for action in actions {
        if !action.state.is_terminal() {
            *action = adapter.poll_action(action.action_id, &context(adapter.snapshot()))?;
        }
    }
    Ok(())
}

fn horizon(plan: &PreparedPlan) -> u64 {
    plan.steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|work| work.deadline_tick.0))
        .max()
        .unwrap_or(plan.anchor.tick.0)
}

fn finish(
    adapter: &mut MemoryAdapter,
    actions: &mut [ActionReceipt],
    plan: &PreparedPlan,
    cadence: u64,
) -> Result<()> {
    let deadline = horizon(plan);
    while adapter.snapshot().tick.0 < deadline
        && actions.iter().any(|action| !action.state.is_terminal())
    {
        let ticks = cadence.min(deadline - adapter.snapshot().tick.0);
        advance(adapter, actions, ticks)?;
    }
    assert!(
        actions
            .iter()
            .all(|action| action.state == CommitState::Verified),
        "{actions:?}"
    );
    for action in actions {
        assert!(adapter.action_work_state(action.action_id)?.is_quiescent());
    }
    assert_eq!(
        crate::lab_world::plan_goal_truth(adapter.snapshot(), plan)?,
        PredicateTruth::True
    );
    Ok(())
}

fn constructor_labor(adapter: &mut MemoryAdapter, enabled: bool, id: u128) -> Result<()> {
    let unit = dfmcp_core::EntityId::new(crate::lab_world::starter::FIRST_DWARF + 1);
    let goal = Predicate::FieldCompare {
        entity_id: unit,
        field: "labor.CONSTRUCTION".to_owned(),
        op: dfmcp_world::CompareOp::Eq,
        value: Value::Bool(enabled),
    };
    let intent = Intent {
        id: IntentId::new(id),
        anchor: adapter.snapshot().anchor(),
        summary: "change the explicitly selected constructor labor".to_owned(),
        terminal_condition: goal.clone(),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::SetLabor {
                units: vec![unit],
                labor: "CONSTRUCTION".to_owned(),
                enabled,
            },
            preconditions: vec![],
            postconditions: vec![goal],
            compensation: None,
            obligation: None,
            depends_on: vec![],
        }],
    };
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(adapter.snapshot()),
    )?;
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    assert!(
        receipt
            .actions
            .iter()
            .all(|action| action.state == CommitState::Verified)
    );
    Ok(())
}

fn construction_progress(snapshot: &WorldSnapshot) -> Result<BTreeMap<dfmcp_core::EntityId, u64>> {
    snapshot
        .graph
        .entities
        .values()
        .filter(|record| {
            record.kind == EntityKind::Building
                && record
                    .fields
                    .get(effects::CONSTRUCTION_STAGE_FIELD)
                    .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
                    .is_some_and(|value| {
                        matches!(value, Value::Text(stage) if stage == effects::STAGE_PLANNED || stage == effects::STAGE_UNDER_CONSTRUCTION)
                    })
        })
        .map(|record| {
            match record
                .fields
                .get("progress_ticks")
                .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
            {
                Some(Value::U64(progress)) => Ok((record.id, *progress)),
                _ => Err(invalid("construction fixture has unknown progress")),
            }
        })
        .collect()
}

#[test]
fn connected_room_forecast_and_execution_share_the_finite_labor_model() -> Result<()> {
    let snapshot = workforce_source()?;
    let plan = sealed_room(&snapshot)?;
    let original = MemoryAdapter::new(snapshot.clone());
    let forecast =
        super::super::super::plan_forecast::forecast_plan(&original, &plan, &context(&snapshot));
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    assert_eq!(
        forecast["predicted_workforce"]["model"],
        effects::LABORATORY_WORKFORCE_MODEL
    );
    assert_eq!(
        forecast["predicted_workforce"]["epistemic_state"],
        "predicted"
    );
    assert!(
        forecast["predicted_workforce"]
            .get("source_anchor")
            .is_none()
    );
    assert!(
        forecast["predicted_workforce"]
            .get("predicted_anchor")
            .is_some()
    );
    assert_eq!(original.snapshot(), &snapshot);
    let cadence = forecast["resolution_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("forecast cadence missing"))?;
    let (mut adapter, mut actions) = begin(snapshot, &plan)?;
    finish(&mut adapter, &mut actions, &plan, cadence)?;
    assert_eq!(
        forecast["predicted_completion_tick"],
        adapter.snapshot().tick.0
    );
    assert_eq!(wood(adapter.snapshot())?, 18);
    assert_eq!(
        crate::lab_world::briefing(adapter.snapshot())["workforce"]["active_activities"],
        0
    );
    Ok(())
}

#[test]
fn connected_room_waits_for_authorized_constructor_labor_and_resumes_original_work() -> Result<()> {
    let snapshot = workforce_source()?;
    let plan = sealed_room(&snapshot)?;
    let (mut adapter, mut actions) = begin(snapshot, &plan)?;
    constructor_labor(&mut adapter, false, 986)?;
    let mut found_blocker = false;
    for _ in 0..400 {
        advance(
            &mut adapter,
            &mut actions,
            effects::DEFAULT_POLL_INTERVAL_TICKS,
        )?;
        let report = effects::laboratory_workforce_report(adapter.snapshot())?
            .ok_or_else(|| invalid("current workforce is missing"))?;
        if report
            .blocked
            .iter()
            .any(|blocker| blocker.labor == "CONSTRUCTION")
        {
            found_blocker = true;
            break;
        }
        assert!(adapter.snapshot().tick.0 < horizon(&plan));
    }
    assert!(found_blocker);
    let before = construction_progress(adapter.snapshot())?;
    assert!(!before.is_empty());
    assert!(before.values().all(|ticks| *ticks == 0));
    advance(&mut adapter, &mut actions, 100)?;
    assert_eq!(construction_progress(adapter.snapshot())?, before);
    let briefing = crate::lab_world::briefing(adapter.snapshot());
    assert!(
        briefing["workforce"]["blocked"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row["labor"] == "CONSTRUCTION")),
        "{briefing}"
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::False
    );
    constructor_labor(&mut adapter, true, 987)?;
    advance(&mut adapter, &mut actions, 1)?;
    let earned = construction_progress(adapter.snapshot())?;
    assert_eq!(earned.values().copied().sum::<u64>(), 1);
    finish(
        &mut adapter,
        &mut actions,
        &plan,
        effects::DEFAULT_POLL_INTERVAL_TICKS,
    )?;
    assert_eq!(wood(adapter.snapshot())?, 18);
    Ok(())
}

#[test]
fn dispatched_room_construction_keeps_its_model_and_progress_after_recovery() -> Result<()> {
    let snapshot = workforce_source()?;
    let plan = sealed_room(&snapshot)?;
    let (mut original, mut actions) = begin(snapshot, &plan)?;
    let expected = plan
        .steps
        .iter()
        .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
        .count();
    for _ in 0..400 {
        advance(
            &mut original,
            &mut actions,
            effects::DEFAULT_POLL_INTERVAL_TICKS,
        )?;
        if construction_progress(original.snapshot())?.len() == expected {
            break;
        }
    }
    assert_eq!(construction_progress(original.snapshot())?.len(), expected);
    let encoded = original.snapshot().canonical_bytes();
    let decoded = WorldSnapshot::from_canonical_bytes(&encoded)?;
    let mut recovered = MemoryAdapter::recovered(decoded)?;
    assert_eq!(
        effects::laboratory_workforce_model(recovered.snapshot())?,
        Some(effects::LABORATORY_WORKFORCE_MODEL)
    );
    assert_ne!(recovered.snapshot().cursor, original.snapshot().cursor);
    assert_eq!(
        construction_progress(recovered.snapshot())?,
        construction_progress(original.snapshot())?
    );
    let remaining = effects::laboratory_workforce_report(recovered.snapshot())?
        .ok_or_else(|| invalid("recovered workforce is missing"))?
        .remaining_service_ticks;
    // Every room action has already dispatched. Recovery advances physical
    // work without adopting any old action handle or inventing a receipt.
    original.advance_ticks(remaining)?;
    recovered.advance_ticks(remaining)?;
    assert_eq!(recovered.snapshot().graph, original.snapshot().graph);
    assert_eq!(
        crate::lab_world::plan_goal_truth(recovered.snapshot(), &plan)?,
        PredicateTruth::True
    );
    assert_eq!(wood(recovered.snapshot())?, 18);
    Ok(())
}
