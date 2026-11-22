//! Pending exact-item placements remain incomplete semantic evidence.

use super::*;

struct PendingFurniture {
    request: ProvisionedRoomRequest,
    adapter: MemoryAdapter,
    installation: effects::FurnitureInstallation,
}

/// Use the actual room plan to dispatch furniture after its terrain and item
/// dependencies finish. One extra, independently produced bed remains loose.
fn pending_furniture() -> Result<PendingFurniture> {
    let snapshot = produce(source()?, "BED", 2)?;
    let request = bedrooms(1)?;
    let (intent, _) = request.compile(IntentId::new(100_331), &snapshot, "pending room")?;
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context(&snapshot))?;
    let step = plan
        .steps
        .iter()
        .find(|step| matches!(step.action, Action::PlaceFurniture { .. }))
        .ok_or_else(|| invalid("fixture placement missing"))?;
    let Action::PlaceFurniture {
        kind,
        item,
        location,
    } = step.action
    else {
        return Err(invalid("fixture placement changed action family"));
    };
    let installation = effects::FurnitureInstallation {
        building: effects::created_entity_id(&step.idempotency_key, 0),
        kind,
        item,
        location,
    };
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|work| work.deadline_tick.0))
        .max()
        .ok_or_else(|| invalid("fixture room deadline missing"))?;
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let mut states: Vec<_> = receipt.actions.iter().map(|action| action.state).collect();
    loop {
        for (action, state) in receipt.actions.iter().zip(&mut states) {
            if !state.is_terminal() {
                *state = adapter
                    .poll_action(action.action_id, &context(adapter.snapshot()))?
                    .state;
            }
        }
        if adapter
            .snapshot()
            .graph
            .entities
            .contains_key(&installation.building)
        {
            assert_eq!(
                field(
                    adapter.snapshot(),
                    installation.building,
                    effects::CONSTRUCTION_STAGE_FIELD,
                )?,
                &Value::Text(effects::STAGE_PLANNED.to_owned())
            );
            return Ok(PendingFurniture {
                request,
                adapter,
                installation,
            });
        }
        if adapter.snapshot().tick.0 >= horizon {
            return Err(invalid("fixture placement never dispatched"));
        }
        adapter.advance_ticks(
            effects::DEFAULT_POLL_INTERVAL_TICKS.min(horizon - adapter.snapshot().tick.0),
        )?;
    }
}

fn write(snapshot: &mut WorldSnapshot, id: EntityId, name: &str, value: Value) -> Result<()> {
    snapshot
        .graph
        .entities
        .get_mut(&id)
        .ok_or_else(|| invalid("fixture entity missing"))?
        .fields
        .insert(name.to_owned(), fact(value, snapshot.tick));
    snapshot.refresh_hash();
    Ok(())
}

#[test]
fn planned_and_constructing_placements_are_false_until_exact_completion() -> Result<()> {
    let mut fixture = pending_furniture()?;
    for (elapsed, expected_stage) in [
        (0, effects::STAGE_PLANNED),
        (1, effects::STAGE_UNDER_CONSTRUCTION),
    ] {
        if elapsed != 0 {
            fixture.adapter.advance_ticks(elapsed)?;
        }
        let snapshot = fixture.adapter.snapshot();
        let before = snapshot.clone();
        assert_eq!(
            field(
                snapshot,
                fixture.installation.building,
                effects::CONSTRUCTION_STAGE_FIELD
            )?,
            &Value::Text(expected_stage.to_owned())
        );
        assert_eq!(
            fixture.request.semantic_goal_truth(snapshot)?,
            PredicateTruth::False
        );
        assert!(
            fixture
                .request
                .compile_residual(IntentId::new(100_332), snapshot, "")
                .is_err(),
            "a pending placement remains occupied and cannot become a replacement opportunity"
        );
        assert_eq!(
            snapshot, &before,
            "observation and refusal must leave source untouched"
        );
    }
    fixture.adapter.advance_ticks(effects::BUILD_TICKS - 1)?;
    let snapshot = fixture.adapter.snapshot();
    assert_eq!(
        effects::inspect_complete_furniture(snapshot, fixture.installation.building)?,
        fixture.installation
    );
    assert_eq!(
        fixture.request.semantic_goal_truth(snapshot)?,
        PredicateTruth::True
    );
    Ok(())
}

#[test]
fn pending_placement_custody_and_lifecycle_corruption_still_refuse() -> Result<()> {
    let mut fixture = pending_furniture()?;
    for elapsed in [0, 1] {
        if elapsed != 0 {
            fixture.adapter.advance_ticks(elapsed)?;
        }
        for mutation in 0..9 {
            let mut snapshot = fixture.adapter.snapshot().clone();
            match mutation {
                0 | 1 => {
                    let id = if mutation == 0 {
                        fixture.installation.building
                    } else {
                        fixture.installation.item
                    };
                    snapshot
                        .graph
                        .entities
                        .get_mut(&id)
                        .ok_or_else(|| invalid("fixture custody entity missing"))?
                        .generation += 1;
                }
                2 => write(
                    &mut snapshot,
                    fixture.installation.item,
                    effects::FURNITURE_ALLOCATED_TO_FIELD,
                    Value::Null,
                )?,
                3 => write(
                    &mut snapshot,
                    fixture.installation.item,
                    effects::FURNITURE_INVENTORY_STATE_FIELD,
                    Value::Text("installed".to_owned()),
                )?,
                4 => write(
                    &mut snapshot,
                    fixture.installation.building,
                    effects::FURNITURE_SOURCE_ITEM_GENERATION_FIELD,
                    Value::U64(2),
                )?,
                5 => write(
                    &mut snapshot,
                    fixture.installation.building,
                    "progress_ticks",
                    Value::U64(effects::BUILD_TICKS),
                )?,
                6 | 7 => {
                    let (id, name) = if mutation == 6 {
                        (
                            fixture.installation.item,
                            effects::FURNITURE_ALLOCATED_TO_FIELD,
                        )
                    } else {
                        (
                            fixture.installation.building,
                            effects::CONSTRUCTION_STAGE_FIELD,
                        )
                    };
                    snapshot
                        .graph
                        .entities
                        .get_mut(&id)
                        .and_then(|record| record.fields.get_mut(name))
                        .ok_or_else(|| invalid("fixture binding field missing"))?
                        .presence = Some(FactPresence::Unknown("uncaptured binding".to_owned()));
                }
                _ => write(
                    &mut snapshot,
                    fixture.installation.building,
                    effects::FURNITURE_SOURCE_ITEM_FIELD,
                    Value::Entity(EntityId::new(999_999)),
                )?,
            }
            snapshot.refresh_hash();
            let before = snapshot.clone();
            assert!(
                fixture.request.semantic_goal_truth(&snapshot).is_err(),
                "pending phase {elapsed}, corruption {mutation} must not be downgraded to incomplete"
            );
            assert_eq!(snapshot, before);
        }
    }
    Ok(())
}

#[test]
fn pending_and_complete_distinct_items_cannot_share_one_original_slot() -> Result<()> {
    let mut fixture = pending_furniture()?;
    fixture.adapter.advance_ticks(effects::BUILD_TICKS)?;
    let mut snapshot = fixture.adapter.snapshot().clone();
    assert_eq!(
        fixture.request.semantic_goal_truth(&snapshot)?,
        PredicateTruth::True
    );
    let spare = effects::loose_furniture_items(&snapshot, FurnitureKind::Bed)?
        .first()
        .copied()
        .ok_or_else(|| invalid("fixture spare bed missing"))?;
    let key = "pending-duplicate-room-slot";
    let duplicate = effects::created_entity_id(key, 0);
    let elsewhere = MapCoord::new(0, 2, 10);
    effects::apply_effect(
        &mut snapshot,
        &Action::PlaceFurniture {
            kind: FurnitureKind::Bed,
            item: spare,
            location: elsewhere,
        },
        key,
    )?;
    // Each item keeps exact independent provenance and reciprocal allocation;
    // only the room geometry is corrupt. Pending items must participate in its
    // occupancy census even though they contribute no completed furnishings.
    for name in ["position", "footprint_min", "footprint_max"] {
        write(
            &mut snapshot,
            duplicate,
            name,
            Value::Coord(fixture.installation.location),
        )?;
    }
    effects::validate_furniture_identity(
        &snapshot,
        duplicate,
        FurnitureKind::Bed,
        spare,
        fixture.installation.location,
    )?;
    effects::validate_furniture_binding(&snapshot, duplicate)?;
    assert_eq!(
        effects::inspect_complete_furniture(&snapshot, fixture.installation.building)?,
        fixture.installation
    );
    let before = snapshot.clone();
    let failure = fixture
        .request
        .semantic_goal_truth(&snapshot)
        .err()
        .ok_or_else(|| invalid("duplicate room slot was accepted"))?;
    assert_eq!(failure.code, ErrorCode::PreconditionsFailed);
    assert!(
        failure.message.contains("duplicate furniture occupancy"),
        "{failure:?}"
    );
    assert_eq!(snapshot, before);
    Ok(())
}
