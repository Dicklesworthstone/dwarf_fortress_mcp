#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::error::Error;

use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{
    Capability, CapabilityGrant, CapabilityScope, CommitState, ErrorCode, FortressId, GameTick,
    IntentId, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, StateAnchor,
    StepId, WorkBudget,
};
use dfmcp_intent::{Action, ObligationSpec, PlanStep, PreparedPlan, derive_step_idempotency_key};
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
    let grants = vec![
        CapabilityGrant {
            capability: Capability::Observe,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::ReadOnly,
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
    ];
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget::CONSERVATIVE_DEFAULT,
        grants,
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
        idempotency_key: derive_step_idempotency_key(
            IntentId::new(1),
            anchor,
            StepId::new(0),
            &Action::Pause {
                paused: paused_target,
            },
        ),
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

    let post_ctx = sample_context(commit_receipt.observed_anchor);

    // 1. Project action task: should map Verified commit state to Completed task status
    let task = project_action_task(&mut adapter, action_id, &post_ctx)?;
    assert_eq!(task.action_id, action_id);
    assert_eq!(task.status, McpTaskStatus::Completed);
    assert_eq!(task.commit_state, CommitState::Verified);
    assert!(task.summary.contains("verified") || task.summary.contains("postconditions"));

    // 2. Cannot cancel verified task
    let Err(cancel_err) = cancel_action_task(
        &mut adapter,
        action_id,
        CancelMode::CompensateReversible,
        &post_ctx,
    ) else {
        return Err("expected error canceling verified action".into());
    };
    assert_eq!(cancel_err.code, ErrorCode::Conflict);

    Ok(())
}

/// Cancelling a ready successor must never become the call that dispatches it.
/// The old read-before-cancel path paused the world and then refused cancellation
/// because that newly dispatched action had already verified.
#[test]
fn cancelling_eligible_deferred_task_keeps_its_effect_undispatched() -> Result<(), Box<dyn Error>> {
    let snapshot = sample_snapshot(true);
    let ctx = sample_context(snapshot.anchor());
    let mut adapter = MemoryAdapter::new(snapshot.clone());
    let mut parent = sample_plan(snapshot.anchor(), false).steps[0].clone();
    parent.obligation = Some(ObligationSpec {
        terminal: Predicate::Paused(false),
        failure: None,
        deadline_tick: GameTick(500),
        poll_interval_ticks: 1,
        stable_for_observations: 1,
    });
    parent.idempotency_key = derive_step_idempotency_key(
        IntentId::new(2),
        snapshot.anchor(),
        parent.id,
        &parent.action,
    );
    let mut child = sample_plan(snapshot.anchor(), true).steps[0].clone();
    child.id = StepId::new(1);
    child.preconditions.clear();
    child.depends_on = vec![parent.id];
    child.idempotency_key =
        derive_step_idempotency_key(IntentId::new(2), snapshot.anchor(), child.id, &child.action);
    let plan = PreparedPlan::builder(
        IntentId::new(2),
        snapshot.anchor(),
        "unpause, verify, then pause",
        Predicate::Paused(true),
    )
    .steps(vec![parent, child])
    .max_risk(RiskTier::Reversible)
    .required_capabilities(BTreeSet::from([Capability::ControlClock]))
    .requires_checkpoint(false)
    .expires_at_tick(GameTick(500))
    .build();
    let prepared = adapter.prepare(&plan, &ctx)?;
    let committed = adapter.commit(&plan, &prepared, &ctx)?;
    let parent_id = committed.actions[0].action_id;
    let child_id = committed.actions[1].action_id;
    assert_eq!(committed.actions[1].state, CommitState::Prepared);
    let ctx = sample_context(adapter.snapshot().anchor());
    assert_eq!(
        project_action_task(&mut adapter, parent_id, &ctx)?.status,
        McpTaskStatus::Completed
    );
    let before = adapter.snapshot().clone();
    assert!(!before.paused);
    let requested = cancel_action_task(&mut adapter, child_id, CancelMode::StopFutureSteps, &ctx)?;
    assert_eq!(requested.status, McpTaskStatus::Working);
    assert_eq!(requested.commit_state, CommitState::CancelRequested);
    assert!(requested.evidence_id.is_some());
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(
        adapter.finalize_cancel(child_id, &ctx)?.state,
        CommitState::Cancelled
    );
    assert_eq!(adapter.snapshot(), &before);
    assert_eq!(
        adapter
            .action_receipt(parent_id)
            .map(|receipt| receipt.state),
        Some(CommitState::Verified)
    );
    Ok(())
}
