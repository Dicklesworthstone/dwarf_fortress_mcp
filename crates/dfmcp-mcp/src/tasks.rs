#![forbid(unsafe_code)]

use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{
    ActionId, CommitState, DfmcpError, ErrorCode, EvidenceId, GameTick, OperationContext, PlanId,
    Result,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum McpTaskStatus {
    Working,
    InputRequired,
    Completed,
    Failed,
    Cancelled,
}

impl McpTaskStatus {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::InputRequired => "input_required",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpTaskProjection {
    pub task_id: String,
    pub action_id: ActionId,
    pub plan_id: Option<PlanId>,
    pub status: McpTaskStatus,
    pub commit_state: CommitState,
    pub summary: String,
    pub evidence_id: Option<EvidenceId>,
    pub terminal_predicate: Option<String>,
    pub created_tick: GameTick,
    pub updated_tick: GameTick,
}

impl McpTaskProjection {
    #[must_use]
    pub fn from_commit_state(
        action_id: ActionId,
        plan_id: Option<PlanId>,
        commit_state: CommitState,
        summary: impl Into<String>,
        evidence_id: Option<EvidenceId>,
        terminal_predicate: Option<String>,
        tick: GameTick,
    ) -> Self {
        let status = match commit_state {
            CommitState::Prepared
            | CommitState::Committing
            | CommitState::AppliedAwaitingVerification
            | CommitState::CompensationPending
            | CommitState::CancelRequested => McpTaskStatus::Working,
            CommitState::Verified => McpTaskStatus::Completed,
            CommitState::Cancelled | CommitState::Compensated => McpTaskStatus::Cancelled,
            CommitState::Failed | CommitState::Indeterminate => McpTaskStatus::Failed,
        };

        Self {
            task_id: format!("task_act_{}", action_id.get()),
            action_id,
            plan_id,
            status,
            commit_state,
            summary: summary.into(),
            evidence_id,
            terminal_predicate,
            created_tick: tick,
            updated_tick: tick,
        }
    }
}

pub fn project_action_task(
    adapter: &mut impl GameAdapter,
    action_id: ActionId,
    context: &OperationContext,
) -> Result<McpTaskProjection> {
    let receipt = adapter.poll_action(action_id, context)?;
    let evidence_id = receipt.evidence.first().map(|e| e.id);
    let summary = match receipt.evidence.first() {
        Some(item) => item.summary.clone(),
        None => receipt.message,
    };

    Ok(McpTaskProjection::from_commit_state(
        action_id,
        None,
        receipt.state,
        summary,
        evidence_id,
        None,
        receipt.observed_anchor.tick,
    ))
}

pub fn cancel_action_task(
    adapter: &mut impl GameAdapter,
    action_id: ActionId,
    mode: CancelMode,
    context: &OperationContext,
) -> Result<McpTaskProjection> {
    let receipt = adapter.poll_action(action_id, context)?;
    if receipt.state == CommitState::Verified {
        return Err(DfmcpError::new(
            ErrorCode::Conflict,
            "cannot cancel a verified or completed obligation task",
        ));
    }

    let cancel_receipt = adapter.request_cancel(action_id, mode, context)?;
    let evidence_id = cancel_receipt.evidence.first().map(|e| e.id);
    let summary = match cancel_receipt.evidence.first() {
        Some(item) => item.summary.clone(),
        None => cancel_receipt.message,
    };

    Ok(McpTaskProjection::from_commit_state(
        action_id,
        None,
        cancel_receipt.state,
        summary,
        evidence_id,
        None,
        cancel_receipt.observed_anchor.tick,
    ))
}
