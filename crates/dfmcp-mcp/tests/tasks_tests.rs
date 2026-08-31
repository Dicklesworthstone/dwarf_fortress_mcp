#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::error::Error;

use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, Digest32, ErrorCode, FortressId,
    GameTick, IntentId, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId,
    StateAnchor, StepId, WorkBudget,
};
use dfmcp_intent::{Action, PlanStep, PreparedPlan};
use dfmcp_lab::MemoryAdapter;
use dfmcp_mcp::tasks::{McpTaskStatus, cancel_action_task, project_action_task};
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

fn sample_snapshot(paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(100),
        ObservationCursor::ORIGIN,
        paused,
        WorldGraph::default(),
    )
}

fn sample_context(anchor: StateAnchor) -> OperationContext {
    let grant = CapabilityGrant {
        capability: Capability::ControlClock,
        scope: CapabilityScope::default(),
        max_risk: RiskTier::Reversible,
        expires_at_tick: None,
        remaining_uses: None,
    };
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        grants: vec![grant],
        cancellation_requested: false,
    }
}

fn sample_plan(anchor: StateAnchor, paused_target: bool) -> PreparedPlan {
    let step = PlanStep {
        id: StepId::new(0),
        action: Action::Pause {
            paused: paused_target,
        },
        preconditions: vec![Predicate::Paused(!paused_target)],
        postconditions: vec![Predicate::Paused(paused_target)],
        compensation: Some(Action::Pause {
            paused: !paused_target,
        }),
        obligation: None,
        depends_on: Vec::new(),
        risk: RiskTier::Reversible,
        required_capability: Capability::ControlClock,
        idempotency_key: Digest32::of_bytes(b"step_pause_task").to_hex(),
    };

    let mut caps = BTreeSet::new();
    caps.insert(Capability::ControlClock);

    PreparedPlan::builder(
        IntentId::new(1),
        anchor,
        "Toggle pause task",
        Predicate::Paused(paused_target),
    )
    .steps(vec![step])
    .max_risk(RiskTier::Reversible)
    .required_capabilities(caps)
    .requires_checkpoint(false)
    .expires_at_tick(GameTick(500))
    .build()
}

/// TEST-017 & WP-13 Gate 3: MCP Tasks Binding backed by Obligation Engine
#[test]
fn test_tasks_projection_and_lifecycle_mapping() -> Result<(), Box<dyn Error>> {
    let snapshot = sample_snapshot(true);
    let ctx = sample_context(snapshot.anchor());
    let mut adapter = MemoryAdapter::new(snapshot.clone());
    let plan = sample_plan(snapshot.anchor(), false);

    let prep_receipt = adapter.prepare(&plan, &ctx)?;
    let commit_receipt = adapter.commit(&plan, &prep_receipt, &ctx)?;
    let action_id = commit_receipt.actions[0].action_id;

    // 1. Project action task: should map Verified commit state to Completed task status
    let task = project_action_task(&mut adapter, action_id, &ctx)?;
    assert_eq!(task.action_id, action_id);
    assert_eq!(task.status, McpTaskStatus::Completed);
    assert_eq!(task.commit_state, CommitState::Verified);
    assert!(task.summary.contains("verified") || task.summary.contains("postconditions"));

    // 2. Cannot cancel verified task
    let Err(cancel_err) = cancel_action_task(
        &mut adapter,
        action_id,
        CancelMode::CompensateReversible,
        &ctx,
    ) else {
        return Err("expected error canceling verified action".into());
    };
    assert_eq!(cancel_err.code, ErrorCode::Conflict);

    Ok(())
}
