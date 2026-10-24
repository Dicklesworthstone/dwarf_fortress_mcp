//! Whole connected-room requests with finite workshop inputs, including an
//! interrupted setup recovered from the canonical world and explicitly replanned.

use super::*;
use dfmcp_adapter::CancelMode;
use dfmcp_core::Digest32;
use dfmcp_world::{Fact, FactSource};

fn economy(wood: u64) -> Result<WorldSnapshot> {
    let mut snapshot = crate::lab_world::scenario_snapshot(
        "starter_fortress_economy",
        FortressId::new(992),
        false,
    )?;
    snapshot
        .graph
        .entities
        .remove(&crate::lab_world::starter::CARPENTERS);
    snapshot
        .graph
        .entities
        .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("economy stock missing"))?
        .fields
        .insert(
            effects::STOCK_WOOD_FIELD.to_owned(),
            Fact::known(
                Value::U64(wood),
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn economy_room() -> Result<ConnectedRoomRequest> {
    let mut input = request_value(false);
    input["room"]["prerequisites"] = json!({
        "assign_labor":false,
        "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}],
    });
    ConnectedRoomRequest::parse_current(&input.to_string())
}

#[test]
fn connected_room_exact_budget_covers_setup_and_items_in_forecast_and_execution() -> Result<()> {
    let request = economy_room()?;
    let short = economy(2)?;
    let unchanged = short.clone();
    let error = request
        .compile(IntentId::new(992), &short, "")
        .err()
        .ok_or_else(|| invalid("one-log-short room was admitted"))?;
    assert_eq!(error.code, dfmcp_core::ErrorCode::PreconditionsFailed);
    assert_eq!(
        short, unchanged,
        "shortage must be refused before any dispatch"
    );

    let snapshot = economy(3)?;
    let (intent, analysis) = request.compile(IntentId::new(992), &snapshot, "")?;
    assert_eq!(
        analysis["production"]["construction_materials"]["new_workshop_wood"],
        1
    );
    assert_eq!(
        analysis["production"]["materials"]["WOOD"]["available_for_new_plan"],
        2
    );
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    assert_eq!(
        plan.steps
            .iter()
            .filter(|step| matches!(step.action, Action::Build { .. }))
            .count(),
        1
    );
    let original = MemoryAdapter::new(snapshot.clone());
    let forecast =
        crate::server::plan_forecast::forecast_plan(&original, &plan, &context(&snapshot));
    assert_eq!(forecast["available"], true, "{forecast}");
    assert_eq!(forecast["predicted_complete"], true, "{forecast}");
    assert_eq!(forecast["predicted_goal_complete"], true, "{forecast}");
    let materials = &forecast["predicted_construction_materials"];
    assert_eq!(
        materials["model"],
        effects::LABORATORY_CONSTRUCTION_MATERIAL_MODEL
    );
    assert_eq!(materials["available_wood"], 0);
    assert_eq!(materials["held_wood"], 0);
    assert_eq!(materials["consumed_wood"], 1);
    assert_eq!(materials["released_wood"], 0);
    assert_eq!(materials["epistemic_state"], "predicted");
    assert!(materials.get("source_anchor").is_none());
    assert!(materials.get("predicted_anchor").is_some());
    assert_eq!(
        original.snapshot(),
        &snapshot,
        "forecast cannot allocate canonical stock"
    );
    let cadence = forecast["resolution_ticks"]
        .as_u64()
        .ok_or_else(|| invalid("material forecast cadence missing"))?;
    let (mut adapter, mut actions) = begin(snapshot, &plan)?;
    let held = effects::construction_material_report(adapter.snapshot())?
        .ok_or_else(|| invalid("dispatched construction materials missing"))?;
    assert_eq!(held.available_wood, 2);
    assert_eq!(held.held_wood, 1);
    assert_eq!(held.consumed_wood, 0);
    finish(&mut adapter, &mut actions, &plan, cadence)?;
    assert_eq!(
        forecast["predicted_completion_tick"],
        adapter.snapshot().tick.0
    );
    assert_eq!(wood(adapter.snapshot())?, 0);
    let current = crate::lab_world::construction_material_json(adapter.snapshot())?
        .ok_or_else(|| invalid("completed construction materials missing"))?;
    assert_eq!(current["epistemic_state"], "certified_derived");
    for field in [
        "available_wood",
        "held_wood",
        "consumed_wood",
        "released_wood",
        "allocations",
    ] {
        assert_eq!(current[field], materials[field]);
    }
    assert_eq!(
        plan.steps
            .iter()
            .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
            .count(),
        2
    );
    let build = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::Build { .. }))
        .ok_or_else(|| invalid("completed setup missing"))?;
    for variant in 0..2 {
        let mut altered = adapter.snapshot().clone();
        let record = altered
            .graph
            .entities
            .get_mut(&effects::created_entity_id(&build.idempotency_key, 0))
            .ok_or_else(|| invalid("completed material allocation missing"))?;
        if variant == 0 {
            record.generation += 1;
        } else {
            record.fields.insert(
                effects::CONSTRUCTION_MATERIAL_STATE_FIELD.to_owned(),
                Fact::known(
                    Value::Text("held".to_owned()),
                    altered.tick,
                    FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    Digest32::ZERO,
                ),
            );
        }
        altered.refresh_hash();
        if variant == 0 {
            assert_eq!(
                PredicateEvidence::laboratory(&altered)?.evaluate(&plan.terminal_condition)?,
                PredicateTruth::True,
                "field predicates alone cannot certify entity generation"
            );
        }
        assert!(!matches!(
            crate::lab_world::plan_goal_truth(&altered, &plan),
            Ok(PredicateTruth::True)
        ));
    }
    Ok(())
}

#[test]
fn cancelled_workshop_refunds_once_and_recovered_original_room_reuses_the_site() -> Result<()> {
    let request = economy_room()?;
    let snapshot = economy(3)?;
    let (intent, _) = request.compile(IntentId::new(993), &snapshot, "original room")?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let build_index = plan
        .steps
        .iter()
        .position(|step| matches!(step.action, Action::Build { .. }))
        .ok_or_else(|| invalid("room construction missing"))?;
    let building = effects::created_entity_id(&plan.steps[build_index].idempotency_key, 0);
    let (mut adapter, mut actions) = begin(snapshot, &plan)?;
    advance(&mut adapter, &mut actions, effects::BUILD_TICKS / 4)?;
    let build_action = actions[build_index].action_id;
    assert_eq!(wood(adapter.snapshot())?, 2);
    assert_eq!(
        effects::construction_material_report(adapter.snapshot())?
            .ok_or_else(|| invalid("active materials missing"))?
            .held_wood,
        1
    );
    for action in actions.iter_mut().rev() {
        if !action.state.is_terminal() {
            adapter.request_cancel(
                action.action_id,
                CancelMode::StopFutureSteps,
                &context(adapter.snapshot()),
            )?;
            action.state = adapter
                .finalize_cancel(action.action_id, &context(adapter.snapshot()))?
                .state;
            assert_eq!(action.state, CommitState::Cancelled);
        }
        assert!(adapter.action_work_state(action.action_id)?.is_quiescent());
    }
    let cancelled = effects::construction_material_report(adapter.snapshot())?
        .ok_or_else(|| invalid("cancelled material receipt missing"))?;
    assert_eq!(cancelled.available_wood, 3);
    assert_eq!(cancelled.held_wood, 0);
    assert_eq!(cancelled.consumed_wood, 0);
    assert_eq!(cancelled.released_wood, 1);
    adapter.finalize_cancel(build_action, &context(adapter.snapshot()))?;
    assert_eq!(
        effects::construction_material_report(adapter.snapshot())?,
        Some(cancelled.clone())
    );

    let decoded = WorldSnapshot::from_canonical_bytes(&adapter.snapshot().canonical_bytes())?;
    let mut recovered = MemoryAdapter::recovered(decoded)?;
    assert_ne!(recovered.snapshot().cursor, adapter.snapshot().cursor);
    assert_eq!(
        effects::construction_material_report(recovered.snapshot())?,
        Some(cancelled)
    );
    assert!(
        recovered
            .poll_action(build_action, &context(recovered.snapshot()))
            .is_err()
    );
    let allocation =
        crate::lab_world::construction_material_action_json(recovered.snapshot(), building)?
            .ok_or_else(|| invalid("recovered allocation missing"))?;
    assert_eq!(allocation["allocation"]["state"], "released");
    let before_replan = recovered.snapshot().clone();
    let (residual, analysis) = request.compile_residual(
        IntentId::new(994),
        recovered.snapshot(),
        "finish the original room",
    )?;
    assert_eq!(recovered.snapshot(), &before_replan);
    assert_eq!(
        analysis["production"]["construction_materials"]["new_workshop_wood"],
        1
    );
    assert!(residual.requested_actions.iter().any(|step| matches!(
        step.action, Action::Build { location, .. } if location == MapCoord::new(0, 0, 10)
    )));
    let resumed = StaticPlanner::default().prepare_laboratory(
        recovered.snapshot(),
        &residual,
        &context(recovered.snapshot()),
    )?;
    let prepared = recovered.prepare(&resumed, &context(recovered.snapshot()))?;
    let mut resumed_actions = recovered
        .commit(&resumed, &prepared, &context(recovered.snapshot()))?
        .actions;
    finish(
        &mut recovered,
        &mut resumed_actions,
        &resumed,
        effects::DEFAULT_POLL_INTERVAL_TICKS,
    )?;
    let completed = effects::construction_material_report(recovered.snapshot())?
        .ok_or_else(|| invalid("resumed material report missing"))?;
    assert_eq!(completed.available_wood, 0);
    assert_eq!(completed.held_wood, 0);
    assert_eq!(completed.consumed_wood, 1);
    assert_eq!(completed.released_wood, 1);
    assert_eq!(completed.allocations.len(), 2);
    assert_eq!(
        resumed
            .steps
            .iter()
            .filter(|step| matches!(step.action, Action::PlaceFurniture { .. }))
            .count(),
        2
    );
    Ok(())
}
