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
            // The only dwarf also brews: brewing needs a worker with the labor.
            fields: BTreeMap::from([(
                "labor.BREW".to_owned(),
                dfmcp_world::Fact::known(
                    Value::Bool(true),
                    GameTick(1),
                    dfmcp_world::FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
                    dfmcp_core::Digest32::ZERO,
                ),
            )]),
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

/// A completed still, so brewing orders can progress without building one.
fn with_still(mut snapshot: WorldSnapshot) -> WorldSnapshot {
    let fact = |value: Value| {
        dfmcp_world::Fact::known(
            value,
            GameTick(1),
            dfmcp_world::FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
            dfmcp_core::Digest32::ZERO,
        )
    };
    let still = EntityId::new(201);
    snapshot.graph.entities.insert(
        still,
        EntityRecord {
            id: still,
            generation: 1,
            revision: 1,
            kind: EntityKind::Building,
            label: "workshop:Still".to_owned(),
            fields: BTreeMap::from([
                (
                    "building_kind".to_owned(),
                    fact(Value::Text("workshop:Still".to_owned())),
                ),
                (
                    CONSTRUCTION_STAGE_FIELD.to_owned(),
                    fact(Value::Text(STAGE_COMPLETE.to_owned())),
                ),
            ]),
        },
    );
    snapshot.refresh_hash();
    snapshot
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
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter, 1),
    )?;
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
    let unpause_plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &unpause,
        &context(&adapter, 4),
    )?;
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
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter, 1),
    )?;
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
    let mut adapter = MemoryAdapter::new(with_still(world(false)));
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
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter, 1),
    )?;
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
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter, 1),
    )?;
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
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter, 1),
    )?;
    let before = adapter.snapshot().clone();
    let failure = commit(&mut adapter, &plan, 2)
        .err()
        .ok_or_else(|| DfmcpError::new(ErrorCode::InternalInvariantViolation, "accepted"))?;
    assert_eq!(failure.code, ErrorCode::PreconditionsFailed);
    assert_eq!(adapter.snapshot(), &before);
    Ok(())
}

// Bind these execution fixtures to the exact modeled completion of their
// original actions; the planner rejects tautological intent goals.
fn bind_action_completion_goal(intent: &mut Intent) -> Result<()> {
    let mut predicates = Vec::new();
    for (index, requested) in intent.requested_actions.iter().enumerate() {
        let index = u32::try_from(index).map_err(|_| {
            DfmcpError::new(ErrorCode::BudgetExceeded, "fixture step index overflow")
        })?;
        let action = requested.action.normalized();
        let key = dfmcp_intent::derive_step_idempotency_key(
            intent.id,
            intent.anchor,
            dfmcp_core::StepId::new(index),
            &action,
        );
        predicates.extend(effects::default_postconditions(
            &action,
            &key,
            intent.anchor.fortress_id,
        ));
    }
    intent.terminal_condition = Predicate::All(predicates);
    Ok(())
}

#[test]
fn conditional_orders_receive_only_time_after_their_prerequisite_completes() -> Result<()> {
    let mut seed = MemoryAdapter::new(world(false));
    let mut intent = Intent {
        id: IntentId::new(901),
        anchor: seed.snapshot().anchor(),
        summary: "make a part, then assemble it".to_owned(),
        terminal_condition: Predicate::True,
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![
            request(
                Action::CreateWorkOrder {
                    name: "first part".to_owned(),
                    job_token: "MAKE_PART".to_owned(),
                    amount: 1,
                    conditions: Vec::new(),
                },
                Vec::new(),
            ),
            request(
                Action::CreateWorkOrder {
                    name: "assembly".to_owned(),
                    job_token: "ASSEMBLE_PART".to_owned(),
                    amount: 1,
                    conditions: vec![dfmcp_intent::WorkOrderCondition::CompletedOrder {
                        order_name: "first part".to_owned(),
                    }],
                },
                Vec::new(),
            ),
        ],
    };
    bind_action_completion_goal(&mut intent)?;
    let plan = StaticPlanner::default().prepare_laboratory(
        seed.snapshot(),
        &intent,
        &context(&seed, 1),
    )?;
    let actions = commit(&mut seed, &plan, 2)?;
    let first = effects::created_entity_id(&plan.steps[0].idempotency_key, 0);
    let second = effects::created_entity_id(&plan.steps[1].idempotency_key, 0);
    let mut at_boundary = seed.clone();
    at_boundary.advance_ticks(50)?;
    assert_eq!(
        field(&at_boundary, first, AMOUNT_REMAINING_FIELD),
        Some(Value::U64(0))
    );
    assert_eq!(
        field(&at_boundary, second, AMOUNT_REMAINING_FIELD),
        Some(Value::U64(1))
    );
    assert_eq!(
        field(&at_boundary, second, "work_ticks"),
        Some(Value::U64(0)),
        "prerequisite completion cannot grant time from before its boundary"
    );

    for partition in [
        vec![100],
        vec![25, 25, 25, 25],
        vec![1; 100],
        vec![49, 1, 49, 1],
        vec![33, 33, 34],
    ] {
        let mut adapter = seed.clone();
        for ticks in &partition {
            adapter.advance_ticks(*ticks)?;
        }
        for (id, action) in [first, second].into_iter().zip(actions.iter().copied()) {
            assert_eq!(
                field(&adapter, id, AMOUNT_REMAINING_FIELD),
                Some(Value::U64(0)),
                "{partition:?}"
            );
            assert_eq!(
                field(&adapter, id, "work_ticks"),
                Some(Value::U64(0)),
                "{partition:?}"
            );
            assert_eq!(
                poll(&mut adapter, action, 3)?,
                CommitState::Verified,
                "{partition:?}"
            );
        }
    }
    Ok(())
}

#[test]
fn internal_time_boundaries_do_not_manufacture_obligation_samples() -> Result<()> {
    let mut seed = MemoryAdapter::new(world(false));
    let mut order = request(
        Action::CreateWorkOrder {
            name: "two observed samples".to_owned(),
            job_token: "MAKE_PART".to_owned(),
            amount: 1,
            conditions: Vec::new(),
        },
        Vec::new(),
    );
    order.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(1_000),
        poll_interval_ticks: 1,
        stable_for_observations: 2,
    });
    let mut intent = Intent {
        id: IntentId::new(902),
        anchor: seed.snapshot().anchor(),
        summary: "physical completion still requires observed proof".to_owned(),
        terminal_condition: Predicate::True,
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![order],
    };
    bind_action_completion_goal(&mut intent)?;
    intent.requested_actions[0]
        .obligation
        .as_mut()
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "fixture obligation missing",
            )
        })?
        .terminal = intent.terminal_condition.clone();
    let plan = StaticPlanner::default().prepare_laboratory(
        seed.snapshot(),
        &intent,
        &context(&seed, 1),
    )?;
    let action = commit(&mut seed, &plan, 2)?[0];
    for partition in [vec![100], vec![1; 100]] {
        let mut adapter = seed.clone();
        for ticks in partition {
            adapter.advance_ticks(ticks)?;
        }
        assert!(adapter.action_work_state(action)?.is_quiescent());
        assert_eq!(
            poll(&mut adapter, action, 3)?,
            CommitState::AppliedAwaitingVerification
        );
        assert_eq!(
            poll(&mut adapter, action, 4)?,
            CommitState::AppliedAwaitingVerification
        );
        adapter.advance_ticks(1)?;
        assert_eq!(poll(&mut adapter, action, 5)?, CommitState::Verified);
    }
    Ok(())
}

#[test]
fn refused_time_horizon_preserves_world_receipts_and_transcript() -> Result<()> {
    let mut adapter = MemoryAdapter::new(world(false));
    let mut intent = Intent {
        id: IntentId::new(903),
        anchor: adapter.snapshot().anchor(),
        summary: "bounded temporal execution".to_owned(),
        terminal_condition: Predicate::True,
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![request(
            Action::CreateWorkOrder {
                name: "bounded order".to_owned(),
                job_token: "MAKE_PART".to_owned(),
                amount: 2,
                conditions: Vec::new(),
            },
            Vec::new(),
        )],
    };
    bind_action_completion_goal(&mut intent)?;
    let plan = StaticPlanner::default().prepare_laboratory(
        adapter.snapshot(),
        &intent,
        &context(&adapter, 1),
    )?;
    let action = commit(&mut adapter, &plan, 2)?[0];
    let snapshot = adapter.snapshot().clone();
    let receipt = adapter.action_receipt(action).cloned();
    let transcript = adapter.transcript().clone();
    let result = adapter.advance_ticks(u64::MAX - snapshot.tick.0);
    assert!(result.is_err_and(|error| error.code == ErrorCode::BudgetExceeded));
    assert_eq!(adapter.snapshot(), &snapshot);
    assert_eq!(adapter.action_receipt(action), receipt.as_ref());
    assert_eq!(adapter.transcript(), &transcript);
    Ok(())
}
