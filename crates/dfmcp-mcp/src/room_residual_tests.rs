//! Residual room pursuits preserve completed work and the whole original target.

use super::*;
use dfmcp_adapter::CancelMode;

struct InterruptedRoom {
    snapshot: WorldSnapshot,
    original: PreparedPlan,
    retained: effects::FurnitureInstallation,
    released: (FurnitureKind, EntityId, MapCoord),
}

/// Cancel the last original placement after dispatch and finish the other work.
/// This leaves a completed installation and an actually released loose item.
fn interrupted(request: &ProvisionedRoomRequest) -> Result<InterruptedRoom> {
    let snapshot = source()?;
    let (intent, _) = request.compile(IntentId::new(970), &snapshot, "original room")?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let indices: Vec<_> = plan
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            matches!(step.action, Action::PlaceFurniture { .. }).then_some(index)
        })
        .collect();
    assert_eq!(indices.len(), 2);
    let retained_index = indices[0];
    let cancelled_index = indices[1];
    let released = match plan.steps[cancelled_index].action {
        Action::PlaceFurniture {
            kind,
            item,
            location,
        } => (kind, item, location),
        _ => return Err(invalid("fixture placement missing")),
    };
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| {
            step.obligation
                .as_ref()
                .map(|window| window.deadline_tick.0)
        })
        .max()
        .ok_or_else(|| invalid("fixture horizon missing"))?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let mut states: Vec<_> = receipt.actions.iter().map(|action| action.state).collect();
    let mut cancelled = false;
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
        if !cancelled && states[cancelled_index] == CommitState::AppliedAwaitingVerification {
            let action = receipt.actions[cancelled_index].action_id;
            adapter.request_cancel(
                action,
                CancelMode::StopFutureSteps,
                &context(adapter.snapshot()),
            )?;
            states[cancelled_index] = adapter
                .finalize_cancel(action, &context(adapter.snapshot()))?
                .state;
            assert_eq!(states[cancelled_index], CommitState::Cancelled);
            cancelled = true;
        }
    }
    assert!(cancelled, "original placement must actually dispatch");
    for (index, (state, action)) in states.iter().zip(&receipt.actions).enumerate() {
        assert_eq!(
            *state,
            if index == cancelled_index {
                CommitState::Cancelled
            } else {
                CommitState::Verified
            }
        );
        assert!(adapter.action_work_state(action.action_id)?.is_quiescent());
    }
    let retained = effects::inspect_complete_furniture(
        adapter.snapshot(),
        effects::created_entity_id(&plan.steps[retained_index].idempotency_key, 0),
    )?;
    assert!(effects::loose_furniture_items(adapter.snapshot(), released.0)?.contains(&released.1));
    assert_eq!(
        crate::lab_world::plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::False
    );
    Ok(InterruptedRoom {
        snapshot: adapter.snapshot().clone(),
        original: plan,
        retained,
        released,
    })
}

fn install_elsewhere(
    snapshot: WorldSnapshot,
    kind: FurnitureKind,
    item: EntityId,
) -> Result<WorldSnapshot> {
    let id = IntentId::new(971);
    let action = Action::PlaceFurniture {
        kind,
        item,
        location: MapCoord::new(0, 2, 10),
    };
    let key =
        derive_step_idempotency_key(id, snapshot.anchor(), StepId::new(0), &action.normalized());
    let postconditions = effects::default_postconditions(&action, &key, snapshot.fortress_id);
    let intent = Intent {
        id,
        anchor: snapshot.anchor(),
        summary: "use released furniture elsewhere".to_owned(),
        terminal_condition: Predicate::All(postconditions.clone()),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions,
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };
    execute(snapshot, &intent).map(|(snapshot, _)| snapshot)
}

#[test]
fn residual_bedrooms_reuse_complete_and_released_items_after_recovery() -> Result<()> {
    let request = bedrooms(2)?;
    let fixture = interrupted(&request)?;
    let recovered = MemoryAdapter::recovered(fixture.snapshot)?
        .snapshot()
        .clone();
    assert!(request.compile(IntentId::new(972), &recovered, "").is_err());
    let retained_building = recovered
        .graph
        .entities
        .get(&fixture.retained.building)
        .cloned();
    let retained_item = recovered
        .graph
        .entities
        .get(&fixture.retained.item)
        .cloned();
    let before = recovered.clone();
    let (intent, analysis) = request.compile_residual(IntentId::new(972), &recovered, "resume")?;
    assert_eq!(recovered, before);
    assert_eq!(analysis["planner"], RESIDUAL_PLANNER);
    assert_eq!(analysis["retained_placements"], 1);
    assert_eq!(analysis["new_placements"], 1);
    assert_eq!(analysis["proves_old_exact_action_goal"], false);
    assert!(
        !intent
            .requested_actions
            .iter()
            .any(|step| matches!(step.action, Action::CreateWorkOrder { .. }))
    );
    assert!(intent.requested_actions.iter().any(|step| matches!(
        step.action, Action::PlaceFurniture { item, .. } if item == fixture.released.1
    )));
    for step in &intent.requested_actions {
        if let Action::DesignateDig { area, .. } = step.action {
            assert!(!area.contains(fixture.retained.location));
        }
    }
    let (completed, plan) = execute(recovered, &intent)?;
    assert_eq!(material(&completed)?, material(&before)?);
    assert_eq!(
        completed
            .graph
            .entities
            .get(&fixture.retained.building)
            .cloned(),
        retained_building
    );
    assert_eq!(
        completed
            .graph
            .entities
            .get(&fixture.retained.item)
            .cloned(),
        retained_item
    );
    assert_eq!(observed_stock(&completed, FurnitureKind::Bed)?, 0);
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &fixture.original)?,
        PredicateTruth::False,
        "new placement does not rewrite cancelled original action outcome"
    );
    Ok(())
}

#[test]
fn residual_dining_manufactures_only_the_missing_kind_and_preserves_installed_supply() -> Result<()>
{
    let request = dining(3, 3)?;
    let fixture = interrupted(&request)?;
    assert_eq!(fixture.retained.kind, FurnitureKind::Table);
    assert_eq!(fixture.released.0, FurnitureKind::Chair);
    let mut snapshot = install_elsewhere(fixture.snapshot, fixture.released.0, fixture.released.1)?;
    let ledger =
        effects::stock_ledger(&snapshot).ok_or_else(|| invalid("fixture ledger missing"))?;
    let fields = &mut snapshot
        .graph
        .entities
        .get_mut(&ledger)
        .ok_or_else(|| invalid("fixture ledger missing"))?
        .fields;
    fields.remove(effects::STOCK_TABLE_FIELD);
    fields.insert(
        effects::STOCK_WOOD_FIELD.to_owned(),
        fact(Value::U64(1), snapshot.tick),
    );
    snapshot.refresh_hash();
    let retained = snapshot
        .graph
        .entities
        .get(&fixture.retained.building)
        .cloned();
    let (intent, analysis) = request.compile_residual(IntentId::new(973), &snapshot, "")?;
    let orders: Vec<_> = intent
        .requested_actions
        .iter()
        .filter_map(|step| match &step.action {
            Action::CreateWorkOrder {
                job_token, amount, ..
            } => Some((job_token.as_str(), *amount)),
            _ => None,
        })
        .collect();
    assert_eq!(orders.len(), 1);
    assert_eq!(effects::work_order_output_item(orders[0].0), Some("CHAIR"));
    assert_eq!(orders[0].1, 1);
    let table = analysis["inventory"]
        .as_array()
        .ok_or_else(|| invalid("inventory missing"))?
        .iter()
        .find(|row| row["item"] == "TABLE")
        .ok_or_else(|| invalid("table row missing"))?;
    assert_eq!(table["inventory_inspected"], false);
    assert_eq!(table["retained_installations"], 1);
    let (completed, plan) = execute(snapshot, &intent)?;
    assert_eq!(material(&completed)?, 0);
    assert_eq!(
        completed
            .graph
            .entities
            .get(&fixture.retained.building)
            .cloned(),
        retained
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn residual_excavation_preserves_installed_tiles_and_the_full_original_terrain_goal() -> Result<()>
{
    let request = dining(3, 3)?;
    let fixture = interrupted(&request)?;
    let mut snapshot = fixture.snapshot;
    let damaged = [MapCoord::new(2, 6, 10), fixture.released.2];
    for at in damaged {
        snapshot.set_tile_code(at, tile_codes::SOLID_WALL)?;
    }
    snapshot.refresh_hash();
    let (intent, _) = request.compile_residual(IntentId::new(974), &snapshot, "")?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let digs: Vec<_> = plan
        .steps
        .iter()
        .filter_map(|step| match step.action {
            Action::DesignateDig { area, .. } => Some((step.id, area)),
            _ => None,
        })
        .collect();
    assert_eq!(digs.len(), 2);
    assert!(
        digs.iter()
            .all(|(_, area)| !area.contains(fixture.retained.location))
    );
    assert!(
        damaged
            .iter()
            .all(|at| digs.iter().any(|(_, area)| area.contains(*at)))
    );
    let placement = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::PlaceFurniture { .. }))
        .ok_or_else(|| invalid("remaining placement missing"))?;
    assert!(digs.iter().all(|(id, _)| placement.depends_on.contains(id)));
    let (completed, _) = execute(snapshot, &intent)?;
    assert_eq!(
        crate::lab_world::plan_goal_truth(&completed, &plan)?,
        PredicateTruth::True
    );
    for at in [damaged[0], fixture.retained.location] {
        let mut changed = completed.clone();
        changed.set_tile_code(at, tile_codes::SOLID_WALL)?;
        changed.refresh_hash();
        assert_eq!(
            crate::lab_world::plan_goal_truth(&changed, &plan)?,
            PredicateTruth::False,
            "all original terrain remains required, including excluded installed tiles"
        );
    }
    Ok(())
}

#[test]
fn residual_refuses_ineligible_active_unrelated_and_duplicate_occupancy() -> Result<()> {
    let request = bedrooms(2)?;
    let fixture = interrupted(&request)?;
    for mutation in 0..7 {
        let mut snapshot = fixture.snapshot.clone();
        match mutation {
            0 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&fixture.retained.building)
                    .ok_or_else(|| invalid("building missing"))?
                    .generation += 1
            }
            1 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&fixture.retained.item)
                    .ok_or_else(|| invalid("item missing"))?
                    .generation += 1
            }
            2 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&fixture.retained.building)
                    .and_then(|record| record.fields.get_mut(effects::CONSTRUCTION_STAGE_FIELD))
                    .ok_or_else(|| invalid("stage missing"))?
                    .presence = Some(FactPresence::Unknown("lost observation".to_owned()))
            }
            3 => {
                snapshot
                    .graph
                    .entities
                    .get_mut(&fixture.retained.item)
                    .ok_or_else(|| invalid("item missing"))?
                    .fields
                    .insert(
                        effects::FURNITURE_ALLOCATED_TO_FIELD.to_owned(),
                        fact(Value::Null, snapshot.tick),
                    );
            }
            4 | 5 => {
                let action = Action::PlaceFurniture {
                    kind: fixture.released.0,
                    item: fixture.released.1,
                    location: if mutation == 4 {
                        fixture.released.2
                    } else {
                        MapCoord::new(2, 6, 10)
                    },
                };
                effects::apply_effect(&mut snapshot, &action, "unresolved-room-occupancy")?;
            }
            _ => {
                let mut duplicate = snapshot
                    .graph
                    .entities
                    .get(&fixture.retained.building)
                    .cloned()
                    .ok_or_else(|| invalid("building missing"))?;
                duplicate.id = EntityId::new(99_991);
                snapshot.graph.entities.insert(duplicate.id, duplicate);
            }
        }
        snapshot.refresh_hash();
        let before = snapshot.clone();
        assert!(
            request
                .compile_residual(IntentId::new(975), &snapshot, "")
                .is_err(),
            "mutation {mutation}"
        );
        assert_eq!(snapshot, before);
    }
    Ok(())
}

#[test]
fn residual_retained_identity_stays_mandatory_without_a_new_placement_action() -> Result<()> {
    let request = bedrooms(2)?;
    let fixture = interrupted(&request)?;
    let (intent, _) = request.compile_residual(IntentId::new(976), &fixture.snapshot, "")?;
    assert!(!intent.requested_actions.iter().any(|step| matches!(
        step.action, Action::PlaceFurniture { item, .. } if item == fixture.retained.item
    )));
    let (completed, plan) = execute(fixture.snapshot, &intent)?;
    for entity in [fixture.retained.building, fixture.retained.item] {
        let mut changed = completed.clone();
        changed
            .graph
            .entities
            .get_mut(&entity)
            .ok_or_else(|| invalid("retained identity missing"))?
            .generation += 1;
        changed.refresh_hash();
        assert_eq!(
            PredicateEvidence::laboratory(&changed)?.evaluate(&plan.terminal_condition)?,
            PredicateTruth::True
        );
        assert!(!matches!(
            crate::lab_world::plan_goal_truth(&changed, &plan),
            Ok(PredicateTruth::True)
        ));
    }
    Ok(())
}

#[test]
fn residual_replay_preserves_original_source_without_changing_normal_compilation() -> Result<()> {
    let snapshot = source()?;
    let request = bedrooms(2)?;
    let original_source = request.canonical_json();
    let (normal, original_analysis) =
        request.compile(IntentId::new(977), &snapshot, "same request")?;
    let (residual, residual_analysis) =
        request.compile_residual(IntentId::new(977), &snapshot, "same request")?;
    assert_eq!(normal.requested_actions, residual.requested_actions);
    assert_eq!(normal.terminal_condition, residual.terminal_condition);
    assert_ne!(
        normal.summary, residual.summary,
        "new compiler generation has a distinct seal"
    );
    assert_eq!(original_analysis["planner"], PLANNER);
    assert!(original_analysis.get("retained_placements").is_none());
    assert_eq!(residual_analysis["retained_placements"], 0);
    let restored = ProvisionedRoomRequest::parse(&original_source)?;
    assert_eq!(restored.canonical_json(), original_source);
    assert_eq!(
        restored.compile(IntentId::new(977), &snapshot, "same request")?,
        (normal, original_analysis)
    );
    assert_eq!(
        restored.compile_residual(IntentId::new(977), &snapshot, "same request")?,
        (residual, residual_analysis)
    );
    Ok(())
}
