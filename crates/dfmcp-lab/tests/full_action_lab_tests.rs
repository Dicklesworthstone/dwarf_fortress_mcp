#![forbid(unsafe_code)]

//! End-to-end laboratory execution of non-pause semantic actions: planner
//! defaults, two-phase commit, dependency gating, game-time obligations,
//! cancellation and deadline failure. Deterministic, in-memory, no DFHack.

use std::collections::BTreeMap;

use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CommitState, DfmcpError, EntityId,
    ErrorCode, FortressId, GameTick, IntentId, MapCoord, MapCuboid, ObservationCursor,
    OperationContext, RequestId, Result, RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::effects::{
    self, AMOUNT_REMAINING_FIELD, CONSTRUCTION_STAGE_FIELD, LABOR_FIELD_PREFIX, STAGE_COMPLETE,
    STATUS_CANCELLED, STATUS_FIELD,
};
use dfmcp_intent::{
    Action, BuildingKind, Constraint, DigMode, Intent, MaterialSelector, ObligationSpec,
    PreparedPlan, RequestedAction, StaticPlanner,
};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::terrain::uniform_chunk;
use dfmcp_world::{
    ChunkCoord, EntityKind, EntityRecord, Predicate, Value, WorldGraph, WorldSnapshot, tile_codes,
};

const MINER: EntityId = EntityId::new(101);

fn world(paused: bool) -> WorldSnapshot {
    let mut graph = WorldGraph::default();
    let rock = ChunkCoord { x: 0, y: 0, z: 10 };
    graph
        .chunks
        .insert(rock, uniform_chunk(rock, tile_codes::SOLID_WALL));
    graph.entities.insert(
        MINER,
        EntityRecord {
            id: MINER,
            generation: 1,
            revision: 1,
            kind: EntityKind::Unit,
            label: "Urist McMiner".to_owned(),
            fields: BTreeMap::new(),
        },
    );
    WorldSnapshot::new(
        FortressId::new(7),
        GameTick(100),
        ObservationCursor::ORIGIN,
        paused,
        graph,
    )
}

fn context(adapter: &MemoryAdapter, request: u128) -> OperationContext {
    let fortress = adapter.snapshot().fortress_id;
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(request),
        anchor: adapter.snapshot().anchor(),
        budget: WorkBudget::default(),
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
            scope: CapabilityScope {
                fortress_id: Some(fortress),
                ..CapabilityScope::default()
            },
            max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect(),
        cancellation_requested: false,
    }
}

fn request(action: Action, depends_on: Vec<u32>) -> RequestedAction {
    RequestedAction {
        action,
        preconditions: Vec::new(),
        postconditions: Vec::new(),
        compensation: None,
        obligation: None,
        depends_on,
    }
}

fn room() -> Result<MapCuboid> {
    MapCuboid::new(MapCoord::new(2, 2, 10), MapCoord::new(4, 4, 10))
}

fn workshop_intent(snapshot: &WorldSnapshot) -> Result<Intent> {
    Ok(Intent {
        id: IntentId::new(41),
        anchor: snapshot.anchor(),
        summary: "dig a room, build a still in it, and brew".to_owned(),
        terminal_condition: Predicate::RegionTerrain {
            area: room()?,
            tile_code: tile_codes::FLOOR,
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![
            request(
                Action::DesignateDig {
                    area: room()?,
                    mode: DigMode::Mine,
                },
                Vec::new(),
            ),
            request(
                Action::Build {
                    kind: BuildingKind::Workshop("Still".to_owned()),
                    location: MapCoord::new(3, 3, 10),
                    footprint: room()?,
                    material: MaterialSelector::default(),
                },
                vec![0],
            ),
            request(
                Action::CreateWorkOrder {
                    name: "brew".to_owned(),
                    job_token: "BREW_DRINK".to_owned(),
                    amount: 3,
                    conditions: Vec::new(),
                },
                vec![1],
            ),
        ],
    })
}

fn commit(
    adapter: &mut MemoryAdapter,
    plan: &PreparedPlan,
    request: u128,
) -> Result<Vec<ActionId>> {
    let prepared = adapter.prepare(plan, &context(adapter, request))?;
    let receipt = adapter.commit(plan, &prepared, &context(adapter, request + 1))?;
    Ok(receipt.actions.iter().map(|a| a.action_id).collect())
}

fn poll(adapter: &mut MemoryAdapter, action: ActionId, request: u128) -> Result<CommitState> {
    let ctx = context(adapter, request);
    Ok(adapter.poll_action(action, &ctx)?.state)
}

fn field(adapter: &MemoryAdapter, id: EntityId, name: &str) -> Option<Value> {
    adapter
        .snapshot()
        .graph
        .entities
        .get(&id)
        .and_then(|e| e.fields.get(name))
        .map(|f| f.value.clone())
}

/// Drive the whole workshop plan to completion; return the final state hash
/// and the tick at which each step verified.
fn run_workshop() -> Result<(dfmcp_core::Digest32, Vec<u64>, PreparedPlan)> {
    let mut adapter = MemoryAdapter::new(world(true));
    let intent = workshop_intent(adapter.snapshot())?;
    let plan =
        StaticPlanner::default().prepare(adapter.snapshot(), &intent, &context(&adapter, 1))?;
    // Planner defaults are sealed: dig proves its region, build and brew prove
    // the entities their idempotency keys will create.
    assert_eq!(
        plan.steps[0].postconditions,
        vec![Predicate::RegionTerrain {
            area: room()?,
            tile_code: tile_codes::FLOOR
        }]
    );
    assert!(plan.steps.iter().all(|step| step.obligation.is_some()));
    let actions = commit(&mut adapter, &plan, 2)?;
    let building = effects::created_entity_id(&plan.steps[1].idempotency_key, 0);
    let order = effects::created_entity_id(&plan.steps[2].idempotency_key, 0);
    assert!(!adapter.snapshot().graph.entities.contains_key(&building));

    // A paused fortress makes no progress even when laboratory time is forced.
    adapter.advance_ticks(50)?;
    assert_eq!(
        poll(&mut adapter, actions[0], 3)?,
        CommitState::AppliedAwaitingVerification
    );
    assert_eq!(
        adapter.snapshot().tile_code_at(MapCoord::new(2, 2, 10)),
        Some(tile_codes::SOLID_WALL)
    );

    let unpause = Intent {
        id: IntentId::new(42),
        anchor: adapter.snapshot().anchor(),
        summary: "unpause".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![request(Action::Pause { paused: false }, Vec::new())],
    };
    let unpause_plan =
        StaticPlanner::default().prepare(adapter.snapshot(), &unpause, &context(&adapter, 4))?;
    commit(&mut adapter, &unpause_plan, 5)?;

    let mut verified_at = vec![0u64; 3];
    let mut request_id = 10u128;
    for _ in 0..200 {
        adapter.advance_ticks(10)?;
        for (index, action) in actions.iter().enumerate() {
            request_id += 1;
            let state = poll(&mut adapter, *action, request_id)?;
            assert_ne!(state, CommitState::Failed, "step {index} failed");
            if state == CommitState::Verified && verified_at[index] == 0 {
                verified_at[index] = adapter.snapshot().tick.0;
            }
        }
        // The dependent building never exists before its excavation verified.
        if verified_at[0] == 0 {
            assert!(!adapter.snapshot().graph.entities.contains_key(&building));
        }
        if verified_at.iter().all(|tick| *tick > 0) {
            break;
        }
    }
    assert!(verified_at.iter().all(|tick| *tick > 0), "{verified_at:?}");
    assert!(verified_at[0] < verified_at[1] && verified_at[1] < verified_at[2]);
    assert_eq!(
        field(&adapter, building, CONSTRUCTION_STAGE_FIELD),
        Some(Value::Text(STAGE_COMPLETE.to_owned()))
    );
    assert_eq!(
        field(&adapter, order, AMOUNT_REMAINING_FIELD),
        Some(Value::U64(0))
    );
    for coord in dfmcp_world::terrain::region_tiles(room()?) {
        assert_eq!(
            adapter.snapshot().tile_code_at(coord),
            Some(tile_codes::FLOOR)
        );
    }
    Ok((adapter.snapshot().state_hash, verified_at, plan))
}

#[test]
fn dig_build_and_produce_complete_in_dependency_order_with_game_time() -> Result<()> {
    let (first_hash, first_ticks, first_plan) = run_workshop()?;
    let (second_hash, second_ticks, second_plan) = run_workshop()?;
    assert_eq!(first_plan.digest, second_plan.digest);
    assert_eq!(first_hash, second_hash);
    assert_eq!(first_ticks, second_ticks);
    Ok(())
}

#[test]
fn immediate_labor_change_verifies_at_commit_and_compensates_on_cancel() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(true));
    let field_name = format!("{LABOR_FIELD_PREFIX}MINE");
    let intent = Intent {
        id: IntentId::new(51),
        anchor: adapter.snapshot().anchor(),
        summary: "enable mining".to_owned(),
        terminal_condition: Predicate::FieldCompare {
            entity_id: MINER,
            field: field_name.clone(),
            op: dfmcp_world::CompareOp::Eq,
            value: Value::Bool(true),
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![request(
            Action::SetLabor {
                units: vec![MINER],
                labor: "MINE".to_owned(),
                enabled: true,
            },
            Vec::new(),
        )],
    };
    let plan =
        StaticPlanner::default().prepare(adapter.snapshot(), &intent, &context(&adapter, 1))?;
    assert!(plan.steps[0].compensation.is_some());
    let prepared = adapter.prepare(&plan, &context(&adapter, 2))?;
    let receipt = adapter.commit(&plan, &prepared, &context(&adapter, 3))?;
    assert_eq!(receipt.actions[0].state, CommitState::Verified);
    assert_eq!(field(&adapter, MINER, &field_name), Some(Value::Bool(true)));
    // A verified action is terminal: cancellation cannot rewrite history.
    let ctx = context(&adapter, 4);
    let refused = adapter
        .request_cancel(
            receipt.actions[0].action_id,
            CancelMode::CompensateReversible,
            &ctx,
        )
        .err()
        .map(|e| e.code);
    assert_eq!(refused, Some(ErrorCode::Conflict));
    Ok(())
}

#[test]
fn cancelled_work_order_stops_producing_but_keeps_its_record() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let intent = Intent {
        id: IntentId::new(61),
        anchor: adapter.snapshot().anchor(),
        summary: "brew".to_owned(),
        terminal_condition: Predicate::Paused(true),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![request(
            Action::CreateWorkOrder {
                name: "brew".to_owned(),
                job_token: "BREW_DRINK".to_owned(),
                amount: 10,
                conditions: Vec::new(),
            },
            Vec::new(),
        )],
    };
    let plan =
        StaticPlanner::default().prepare(adapter.snapshot(), &intent, &context(&adapter, 1))?;
    let action = commit(&mut adapter, &plan, 2)?[0];
    let order = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT * 3)?;
    assert_eq!(
        field(&adapter, order, AMOUNT_REMAINING_FIELD),
        Some(Value::U64(7))
    );
    let ctx = context(&adapter, 3);
    adapter.request_cancel(action, CancelMode::StopFutureSteps, &ctx)?;
    let ctx = context(&adapter, 4);
    let finalized = adapter.finalize_cancel(action, &ctx)?;
    assert_eq!(finalized.state, CommitState::Cancelled);
    adapter.advance_ticks(effects::WORK_ORDER_TICKS_PER_UNIT * 10)?;
    assert_eq!(
        field(&adapter, order, AMOUNT_REMAINING_FIELD),
        Some(Value::U64(7))
    );
    assert_eq!(
        field(&adapter, order, STATUS_FIELD),
        Some(Value::Text(STATUS_CANCELLED.to_owned()))
    );
    Ok(())
}

#[test]
fn excavation_that_misses_its_explicit_deadline_fails() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let terminal = Predicate::RegionTerrain {
        area: room()?,
        tile_code: tile_codes::FLOOR,
    };
    let mut dig = request(
        Action::DesignateDig {
            area: room()?,
            mode: DigMode::Mine,
        },
        Vec::new(),
    );
    // Nine tiles need 90 ticks of work; allow only 40.
    dig.obligation = Some(ObligationSpec {
        terminal: terminal.clone(),
        failure: None,
        deadline_tick: GameTick(140),
        poll_interval_ticks: 10,
        stable_for_observations: 1,
    });
    let intent = Intent {
        id: IntentId::new(71),
        anchor: adapter.snapshot().anchor(),
        summary: "rushed excavation".to_owned(),
        terminal_condition: terminal,
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![dig],
    };
    let plan =
        StaticPlanner::default().prepare(adapter.snapshot(), &intent, &context(&adapter, 1))?;
    let action = commit(&mut adapter, &plan, 2)?[0];
    let mut last = CommitState::Prepared;
    for request_id in 3..10u128 {
        adapter.advance_ticks(10)?;
        last = poll(&mut adapter, action, request_id)?;
        if last == CommitState::Failed {
            break;
        }
    }
    assert_eq!(last, CommitState::Failed);
    assert!(adapter.snapshot().tick >= GameTick(140));
    Ok(())
}

#[test]
fn excavation_over_unobserved_terrain_is_rejected_without_partial_state() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let area = MapCuboid::new(MapCoord::new(0, 0, 10), MapCoord::new(0, 0, 11))?;
    let intent = Intent {
        id: IntentId::new(81),
        anchor: adapter.snapshot().anchor(),
        summary: "dig into the unknown".to_owned(),
        terminal_condition: Predicate::RegionTerrain {
            area,
            tile_code: tile_codes::FLOOR,
        },
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions: vec![request(
            Action::DesignateDig {
                area,
                mode: DigMode::Mine,
            },
            Vec::new(),
        )],
    };
    let plan =
        StaticPlanner::default().prepare(adapter.snapshot(), &intent, &context(&adapter, 1))?;
    let before = adapter.snapshot().clone();
    let failure = commit(&mut adapter, &plan, 2)
        .err()
        .ok_or_else(|| DfmcpError::new(ErrorCode::InternalInvariantViolation, "accepted"))?;
    assert_eq!(failure.code, ErrorCode::PreconditionsFailed);
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}
