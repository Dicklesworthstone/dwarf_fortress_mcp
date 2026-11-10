//! Finite acquisition composes with complete room and access goals, including
//! cross-level routes, durable recovery and residual access-only repair.

use super::*;
use crate::server::connected_room_continuation::ConnectedRoomContinuation;
use dfmcp_adapter::GameAdapter;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, FortressId, OperationContext,
    RequestId, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{PreparedPlan, effects};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Fact, FactSource, Value, laboratory_fact_value};

fn source(stairs: bool) -> Result<WorldSnapshot> {
    let fortress = FortressId::new(98_601);
    let mut snapshot =
        crate::lab_world::scenario_snapshot("starter_fortress_resources", fortress, false)?;
    if stairs {
        snapshot.graph.chunks =
            crate::lab_world::scenario_snapshot("starter_fortress_multilevel", fortress, false)?
                .graph
                .chunks;
    }
    snapshot
        .graph
        .entities
        .remove(&crate::lab_world::starter::CARPENTERS);
    snapshot
        .graph
        .entities
        .get_mut(&crate::lab_world::starter::STOCK_LEDGER)
        .ok_or_else(|| invalid("fixture stock ledger missing"))?
        .fields
        .insert(
            effects::STOCK_WOOD_FIELD.to_owned(),
            Fact::known(
                Value::U64(0),
                snapshot.tick,
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                Digest32::ZERO,
            ),
        );
    snapshot.refresh_hash();
    Ok(snapshot)
}

fn request_value(stairs: bool) -> Json {
    json!({
        "template":"connected_room",
        "planner":if stairs {RESOURCE_STAIRS_PLANNER} else {RESOURCE_PLANNER},
        "room":{
            "template":"provisioned_room",
            "layout":{"template":"bedroom_cluster","origin":[2,6,if stairs {9} else {10}],"rooms":2,"room_size":[3,3]},
            "acquire_resources":{"assign_labor":false},
            "prerequisites":{
                "assign_labor":true,"workforce":"complete_project_v1",
                "workshops":[{"building":"workshop:Carpenters","location":[0,0,10]}],
            },
        },
        "access":{"entry":[0,2,10],"region":{"min":[0,0,if stairs {9} else {10}],"max":[12,12,10]}},
    })
}

fn request(stairs: bool) -> Result<ConnectedRoomRequest> {
    ConnectedRoomRequest::parse_current(&request_value(stairs).to_string())
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
            (Capability::Designate, RiskTier::Guarded),
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

fn execute(snapshot: WorldSnapshot, intent: &Intent) -> Result<(WorldSnapshot, PreparedPlan)> {
    let plan =
        StaticPlanner::default().prepare_laboratory(&snapshot, intent, &context(&snapshot))?;
    let horizon = plan
        .steps
        .iter()
        .filter_map(|step| step.obligation.as_ref().map(|work| work.deadline_tick.0))
        .max()
        .unwrap_or(snapshot.tick.0);
    let mut adapter = MemoryAdapter::new(snapshot);
    let prepared = adapter.prepare(&plan, &context(adapter.snapshot()))?;
    let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot()))?;
    let mut states: Vec<_> = receipt.actions.iter().map(|action| action.state).collect();
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
    }
    assert!(
        states.iter().all(|state| *state == CommitState::Verified),
        "{states:?}"
    );
    assert_eq!(
        crate::lab_world::plan_goal_truth(adapter.snapshot(), &plan)?,
        PredicateTruth::True
    );
    Ok((adapter.snapshot().clone(), plan))
}

fn wood(snapshot: &WorldSnapshot) -> Result<u64> {
    match snapshot
        .graph
        .entities
        .get(&crate::lab_world::starter::STOCK_LEDGER)
        .and_then(|record| record.fields.get(effects::STOCK_WOOD_FIELD))
        .and_then(|fact| laboratory_fact_value(fact, snapshot.tick))
    {
        Some(Value::U64(value)) => Ok(*value),
        _ => Err(invalid("fixture wood is unavailable")),
    }
}

fn route(analysis: &Json) -> Result<Vec<MapCoord>> {
    analysis["access"]["route"]
        .as_array()
        .ok_or_else(|| invalid("access route missing"))?
        .iter()
        .map(|value| {
            let at: [i32; 3] = serde_json::from_value(value.clone())
                .map_err(|_| invalid("invalid route point"))?;
            Ok(coordinate(at))
        })
        .collect()
}

#[test]
fn finite_acquisition_recovers_into_complete_cardinal_and_stair_rooms_then_repairs_access()
-> Result<()> {
    for stairs in [false, true] {
        let request = request(stairs)?;
        let snapshot = source(stairs)?;
        let original_world = snapshot.clone();
        let (harvest, analysis) = request.compile(IntentId::new(98_611), &snapshot, "")?;
        assert_eq!(snapshot, original_world);
        assert_eq!(
            classify_room_stage(&harvest),
            RoomCompileStage::ResourceAcquisition
        );
        assert_eq!(analysis["planner"], request.planner());
        assert_eq!(analysis["goal_scope"], "resource_acquisition_stage");
        assert_eq!(analysis["resource_workflow"]["stage"], "acquire_resources");
        assert_eq!(
            analysis["resource_workflow"]["original_request"]["planner"],
            request.planner()
        );
        assert_eq!(
            analysis["resource_workflow"]["original_request"]["access"]["entry"],
            json!([0, 2, 10])
        );
        assert_eq!(
            analysis["access"]["deferred_scope"],
            "separately_reviewed_room_and_access_pursuit"
        );
        let mut requested_wood = 0;
        for step in &harvest.requested_actions {
            match step.action {
                Action::HarvestResource { amount, .. } => requested_wood += amount,
                Action::SetLabor { enabled: true, .. } => {}
                _ => return Err(invalid("resource stage dispatched downstream room work")),
            }
        }
        assert_eq!(requested_wood, 3, "two beds and one finite workshop input");
        assert_eq!(
            request.semantic_goal_truth(&snapshot)?,
            PredicateTruth::False
        );
        let mut source_bytes = b"dfmcp-connected-room-source/2\0".to_vec();
        source_bytes.extend_from_slice(request.canonical_json().as_bytes());
        assert_eq!(
            analysis["source_binding"]["connected_room_source_digest"],
            Digest32::of_bytes(&source_bytes).to_hex()
        );

        let (harvested, harvest_plan) = execute(snapshot, &harvest)?;
        assert_eq!(wood(&harvested)?, 3);
        assert_eq!(
            request.semantic_goal_truth(&harvested)?,
            PredicateTruth::False
        );
        let original = ConnectedRoomContinuation::new(
            harvest_plan.digest,
            harvest_plan.digest,
            request.clone(),
        )?;
        let continuation = ConnectedRoomContinuation::parse(&original.canonical_json())?;
        let recovered = MemoryAdapter::recovered(harvested)?.snapshot().clone();
        let (room, room_analysis) = continuation.compile(
            IntentId::new(98_612),
            &recovered,
            "complete original connected room",
        )?;
        assert_eq!(classify_room_stage(&room), RoomCompileStage::FurnishRoom);
        assert!(
            !room
                .requested_actions
                .iter()
                .any(|step| matches!(step.action, Action::HarvestResource { .. }))
        );
        assert_eq!(room_analysis["planner"], request.residual_planner());
        assert_eq!(room_analysis["resource_workflow"]["stage"], "room");
        assert_eq!(room_analysis["new_placements"], 2);
        let (completed, room_plan) = execute(recovered, &room)?;
        assert_eq!(wood(&completed)?, 0);
        assert_eq!(
            continuation.semantic_goal_truth(&completed)?,
            PredicateTruth::True
        );
        assert_ne!(
            crate::lab_world::plan_goal_truth(&completed, &harvest_plan)?,
            PredicateTruth::True,
            "consumed acquisition stock does not overwrite the current full room truth"
        );
        let installations = room_plan
            .steps
            .iter()
            .filter_map(|step| match step.action {
                Action::PlaceFurniture { item, .. } => {
                    Some((item, effects::created_entity_id(&step.idempotency_key, 0)))
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(installations.len(), 2);

        let mut unknown = completed.clone();
        unknown
            .graph
            .entities
            .get_mut(&installations[0].1)
            .ok_or_else(|| invalid("completed furnishing building missing"))?
            .fields
            .remove("footprint_min");
        unknown.refresh_hash();
        assert!(!matches!(
            request.semantic_goal_truth(&unknown),
            Ok(PredicateTruth::True)
        ));
        assert!(
            continuation
                .compile(IntentId::new(98_613), &unknown, "unavailable occupancy")
                .is_err()
        );

        let path = route(&room_analysis)?;
        let (layout, _) = request.room.access_geometry()?;
        let broken = if stairs {
            path.windows(2)
                .find(|pair| pair[0].z != pair[1].z)
                .map(|pair| pair[1])
        } else {
            path.iter().copied().find(|at| {
                original_world.tile_code_at(*at) == Some(tile_codes::SOLID_WALL)
                    && !layout
                        .excavations()
                        .iter()
                        .any(|part| part.area.contains(*at))
            })
        }
        .ok_or_else(|| invalid("completed room route has no repairable access segment"))?;
        let mut interrupted = completed.clone();
        interrupted.set_tile_code(broken, tile_codes::SOLID_WALL)?;
        interrupted.refresh_hash();
        assert_eq!(
            request.semantic_goal_truth(&interrupted)?,
            PredicateTruth::False
        );
        let repair_source = continuation.next(room_plan.digest)?;
        let (repair, repaired_analysis) = repair_source.compile(
            IntentId::new(98_614),
            &interrupted,
            "restore original access",
        )?;
        assert_eq!(repaired_analysis["retained_placements"], 2);
        assert_eq!(repaired_analysis["new_placements"], 0);
        assert!(repair.requested_actions.iter().all(|step| matches!(
            step.action,
            Action::DesignateDig { .. } | Action::SetLabor { .. }
        )));
        let (repaired, _) = execute(interrupted, &repair)?;
        assert_eq!(wood(&repaired)?, 0);
        assert_eq!(
            repair_source.semantic_goal_truth(&repaired)?,
            PredicateTruth::True
        );
        for (item, building) in installations {
            assert_eq!(
                repaired.graph.entities.get(&item),
                completed.graph.entities.get(&item)
            );
            assert_eq!(
                repaired.graph.entities.get(&building),
                completed.graph.entities.get(&building)
            );
        }
    }
    Ok(())
}

#[test]
fn invalid_access_is_refused_before_the_first_resource_stage() -> Result<()> {
    for stairs in [false, true] {
        let request = request(stairs)?;
        let mut snapshot = source(stairs)?;
        snapshot.set_tile_code(request.entry, tile_codes::SOLID_WALL)?;
        snapshot.refresh_hash();
        let unchanged = snapshot.clone();
        assert!(
            request
                .compile(IntentId::new(98_621), &snapshot, "")
                .is_err()
        );
        assert!(
            request
                .compile_residual(IntentId::new(98_622), &snapshot, "")
                .is_err()
        );
        assert_eq!(snapshot, unchanged);
        assert_eq!(wood(&snapshot)?, 0);
        assert!(effects::harvest_report(&snapshot)?.is_some_and(|report| report.tasks.is_empty()));
    }
    Ok(())
}

#[test]
fn connected_resource_intake_requires_matching_generation_and_unique_nested_permission()
-> Result<()> {
    for stairs in [false, true] {
        let request = request(stairs)?;
        let raw = request.canonical_json();
        assert_eq!(ConnectedRoomRequest::parse(&raw)?, request);
        for marker in [PLANNER, STAIRS_PLANNER] {
            let mut value = request_value(stairs);
            value["planner"] = json!(marker);
            assert!(ConnectedRoomRequest::parse_current(&value.to_string()).is_err());
        }
        let duplicate = raw.replace(
            "\"acquire_resources\":{\"assign_labor\":false}",
            "\"acquire_resources\":{\"assign_labor\":true,\"assign_labor\":false}",
        );
        assert_ne!(raw, duplicate);
        assert!(ConnectedRoomRequest::parse_current(&duplicate).is_err());
        assert!(ConnectedRoomRequest::parse(&duplicate).is_err());
    }
    let mut cardinal = request_value(false);
    cardinal
        .as_object_mut()
        .ok_or_else(|| invalid("fixture request missing"))?
        .remove("planner");
    assert_eq!(
        ConnectedRoomRequest::parse_current(&cardinal.to_string())?.planner(),
        RESOURCE_PLANNER
    );
    assert!(ConnectedRoomRequest::parse(&cardinal.to_string()).is_err());
    Ok(())
}
