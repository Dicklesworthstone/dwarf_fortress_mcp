#![forbid(unsafe_code)]

//! Integration tests for WP-DFH-04 Mutation Dispatcher and Two-Phase Effect Journal.

use dfmcp_adapter::dispatcher::MutationDispatcher;
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, ErrorCode, FortressId, GameTick,
    IntentId, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, Result,
    RiskTier, SessionId, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, DigMode, Intent, RequestedAction, StaticPlanner};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

fn sample_snapshot() -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        true,
        WorldGraph::default(),
    )
}

fn sample_context(snapshot: &WorldSnapshot) -> OperationContext {
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: vec![
            CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Irreversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::ControlClock,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::Designate,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Guarded,
                expires_at_tick: None,
                remaining_uses: None,
            },
            CapabilityGrant {
                capability: Capability::ConfigureLabor,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            },
        ],
        cancellation_requested: false,
    }
}

#[test]
fn test_dispatcher_all_actions_dispatch_and_journal() -> Result<()> {
    let mut snapshot = sample_snapshot();
    let ctx = sample_context(&snapshot);

    let dig_cuboid = MapCuboid::new(
        MapCoord { x: 0, y: 0, z: 100 },
        MapCoord { x: 5, y: 5, z: 100 },
    )?;

    let requested_actions = vec![
        RequestedAction {
            action: Action::Pause { paused: false },
            preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        },
        RequestedAction {
            action: Action::DesignateDig {
                area: dig_cuboid,
                mode: DigMode::Mine,
            },
            preconditions: Vec::new(),
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        },
        RequestedAction {
            action: Action::SetLabor {
                units: vec![dfmcp_core::EntityId::new(1)],
                labor: "MINE".to_owned(),
                enabled: true,
            },
            preconditions: Vec::new(),
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        },
    ];

    let intent = Intent {
        id: IntentId::new(1),
        anchor: snapshot.anchor(),
        summary: "multi-action dispatch test".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
        requested_actions,
    };

    let plan = StaticPlanner::default().prepare(&snapshot, &intent, &ctx)?;
    let mut dispatcher = MutationDispatcher::new();

    let prepare_receipt = dispatcher.prepare_mutation(&plan, &snapshot, &ctx)?;
    let commit_receipt =
        dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &ctx)?;

    assert_eq!(commit_receipt.actions.len(), 3);
    for action_receipt in &commit_receipt.actions {
        assert_eq!(action_receipt.state, CommitState::Verified);
    }

    assert_eq!(dispatcher.journal().len(), 1);
    assert!(!snapshot.paused);

    Ok(())
}

#[test]
fn test_indeterminate_state_blocks_blind_retry() -> Result<()> {
    let mut snapshot = sample_snapshot();
    let ctx = sample_context(&snapshot);

    let intent = Intent {
        id: IntentId::new(1),
        anchor: snapshot.anchor(),
        summary: "test action".to_owned(),
        terminal_condition: Predicate::Paused(false),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause { paused: false },
            preconditions: vec![Predicate::Paused(true)],
            postconditions: vec![Predicate::Paused(false)],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };

    let plan = StaticPlanner::default().prepare(&snapshot, &intent, &ctx)?;
    let mut dispatcher = MutationDispatcher::new();

    let prepare_receipt = dispatcher.prepare_mutation(&plan, &snapshot, &ctx)?;

    // Simulate connection drop: mark transaction Indeterminate in journal
    let idempotency_key = format!("dfmcp_tx_{}_{}", ctx.session_id.get(), plan.digest);
    dispatcher
        .journal_mut()
        .mark_indeterminate(&idempotency_key, "bridge socket timeout".to_owned())?;

    // Attempting commit must now fail with InternalInvariantViolation (reconciliation required)
    let result = dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &ctx);
    assert!(result.is_err());
    let err = match result {
        Err(e) => e,
        Ok(_) => {
            return Err(dfmcp_core::DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "expected error",
            ));
        }
    };
    assert_eq!(err.code, ErrorCode::InternalInvariantViolation);

    Ok(())
}
