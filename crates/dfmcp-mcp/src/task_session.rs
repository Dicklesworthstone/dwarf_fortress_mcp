//! Engine-authorized views and staged drains for one original laboratory plan.
//! No MCP Task type crosses this seam and no inspection polls an action.
use super::*;
use crate::tasks::McpTaskStatus;
use serde_json::Value;

pub(crate) struct PlanTaskView {
    pub(crate) status: McpTaskStatus,
    pub(crate) payload: Value,
}

fn session_error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "laboratory task session poisoned",
    )
}

fn with_task_session<T>(
    session_id: &str,
    body: impl FnOnce(&mut LabSession) -> Result<T>,
) -> Result<T> {
    let session = resolve_session(Some(session_id.to_owned()))?;
    with_session(&session, || Err(session_error()), body)
}

pub(crate) fn read_authority(session_id: &str) -> Result<()> {
    with_task_session(session_id, |session| {
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)
    })
}

pub(crate) fn read_budget(session_id: &str) -> Result<WorkBudget> {
    with_task_session(session_id, |session| {
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        Ok(session.budget)
    })
}

pub(crate) fn has_original_receipt(session_id: &str, digest: &str) -> Result<bool> {
    with_task_session(session_id, |session| {
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        Ok(session.commit_receipts.contains_key(digest))
    })
}

/// Validate monitor authority without committing, consuming the pending plan,
/// polling, advancing time, or granting any new capability.
pub(crate) fn validate(session_id: &str, digest: &str) -> Result<()> {
    with_task_session(session_id, |session| {
        durability_gate(session)?;
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        if session.budget.max_output_tokens < 1_500 || session.budget.max_bytes < 6_000 {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory task monitors require at least 1500 output tokens and 6000 bytes for a complete bounded terminal summary; no plan was dispatched",
            ));
        }
        let pending = session
            .pending
            .as_ref()
            .filter(|pending| pending.digest == digest);
        if let Some(pending) = pending {
            if pending.plan.steps.len() > 64 {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "laboratory task monitors admit at most 64 original plan actions",
                ));
            }
            for step in &pending.plan.steps {
                let scope = step.action.scope();
                ctx.authorize(
                    step.required_capability,
                    step.risk,
                    &scope.entity_ids,
                    scope.map_area,
                )?;
            }
            Ok(())
        } else if session.commit_receipts.contains_key(digest) {
            if original_actions(session, digest)?.len() > 64 {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "laboratory task monitors admit at most 64 original plan actions",
                ));
            }
            for (capability, risk) in session.commit_authority.get(digest).into_iter().flatten() {
                authorize_entry(&ctx, *capability, *risk)?;
            }
            Ok(())
        } else {
            Err(DfmcpError::new(
                ErrorCode::Conflict,
                "task requires the exact pending or already committed plan digest",
            ))
        }
    })
}

fn original_actions(session: &LabSession, digest: &str) -> Result<Vec<ActionId>> {
    let raw = session.commit_receipts.get(digest).ok_or_else(|| DfmcpError::new(
        ErrorCode::Conflict,
        "original task plan receipt is unavailable; a restore or session replacement invalidated it; do not retry the effect",
    ))?;
    let receipt: Value = serde_json::from_str(raw).map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "retained task plan receipt is invalid",
        )
    })?;
    let actions = receipt
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "retained task plan has no action records",
            )
        })?;
    if actions.is_empty() || actions.len() > session.budget.max_actions as usize {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "retained task action count exceeds its negotiated bound",
        ));
    }
    actions
        .iter()
        .map(|action| {
            let id = action
                .get("action_id")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "retained task action identifier is missing",
                    )
                })?;
            u128::from_str_radix(id, 16)
                .map(ActionId::new)
                .map_err(|_| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "retained task action identifier is invalid",
                    )
                })
        })
        .collect()
}

fn view_locked(session: &LabSession, digest: &str) -> Result<PlanTaskView> {
    let ids = original_actions(session, digest)?;
    let mut actions = Vec::with_capacity(ids.len());
    let mut all_verified = true;
    let mut all_terminal = true;
    let mut failed = false;
    let mut indeterminate = false;
    for id in ids {
        let receipt = session.adapter.action_receipt(id).ok_or_else(|| DfmcpError::new(
            ErrorCode::Conflict, "original task action no longer exists; refresh after restore and do not retry its effect",
        ))?;
        all_verified &= receipt.state == CommitState::Verified;
        all_terminal &= receipt.state.is_terminal();
        failed |= receipt.state == CommitState::Failed;
        indeterminate |= receipt.state == CommitState::Indeterminate;
        let step = session.adapter.action_step(id);
        actions.push(json!({
            "action_id": id.to_string(), "step": receipt.step_id.get(),
            "state": format!("{:?}", receipt.state), "message": receipt.message,
            "receipt_digest": receipt.adapter_receipt_digest.to_hex(),
            "observed_anchor": anchor_json(&receipt.observed_anchor),
            "obligation": step.and_then(|step| step.obligation.as_ref()).map(|obligation| json!({
                "terminal": crate::lab_world::predicate_json(&obligation.terminal),
                "deadline_tick": obligation.deadline_tick.0,
            })),
            "evidence": receipt.evidence.iter().map(|evidence| json!({
                "evidence_id": evidence.id.to_string(), "digest": evidence.digest.to_hex(),
                "kind": format!("{:?}", evidence.kind), "summary": evidence.summary,
                "anchor": anchor_json(&evidence.anchor),
            })).collect::<Vec<_>>(),
        }));
    }
    let status = if all_verified {
        McpTaskStatus::Completed
    } else if indeterminate || (all_terminal && failed) {
        McpTaskStatus::Failed
    } else if all_terminal {
        McpTaskStatus::Cancelled
    } else {
        McpTaskStatus::Working
    };
    let anchor = anchor_json(&session.adapter.snapshot().anchor());
    let active = actions
        .iter()
        .filter(|action| {
            !matches!(
                action["state"].as_str(),
                Some("Verified" | "Cancelled" | "Compensated" | "Failed")
            )
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut work = crate::empty_active_work();
    work["actions"] = json!(active);
    let packet = crate::AgentTurnBuilder::new("fortress.commit", crate::AgentPhase::Verify)
        .session_id(session.session_id.to_string())
        .anchor(anchor.clone())
        .active_work(work)
        .build();
    Ok(PlanTaskView {
        status,
        payload: json!({
            "ok": status != McpTaskStatus::Failed,
            "schema": "dfmcp.lab-plan-task/1", "session_id": session.session_id.to_string(),
            "plan_digest": digest, "status": status.as_str(), "actions": actions,
            "observed_anchor": anchor, "game_tick": session.adapter.snapshot().tick.0,
            "paused": session.adapter.snapshot().paused,
            "indeterminate": indeterminate,
            "recovery_class": if indeterminate { "reconciliation_required" } else { "never_unchanged" },
            "blind_retry_allowed": false,
            "agent_turn": packet,
            "scope": "laboratory_process_only",
            "next_step": if status == McpTaskStatus::Working { json!({
                "tool": "fortress.wait", "arguments": {"session_id": session.session_id.to_string(), "max_game_ticks": 100},
                "note": "only an explicit foreground wait advances laboratory game time",
            }) } else { Value::Null },
        }),
    })
}

pub(crate) fn view(session_id: &str, digest: &str) -> Result<PlanTaskView> {
    with_task_session(session_id, |session| {
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        view_locked(session, digest)
    })
}

/// The Tasks store checks the engine before recording cancellation. A stale
/// task projection cannot cancel an already verified plan, and loss of the
/// original per-action authority cannot be hidden by transport cancellation.
pub(crate) fn cancellation_admission(session_id: &str, digest: &str) -> Result<bool> {
    with_task_session(session_id, |session| {
        durability_gate(session)?;
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        if !session.commit_receipts.contains_key(digest) {
            // The retained task has not dispatched its originating commit.
            return Ok(false);
        }
        let ids = original_actions(session, digest)?;
        let mut all_verified = true;
        for id in ids {
            let receipt = session.adapter.action_receipt(id).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "original task action is missing; cancellation cannot prove quiescence",
                )
            })?;
            all_verified &= receipt.state == CommitState::Verified;
            if !receipt.state.is_terminal() {
                let step = session.adapter.action_step(id).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "task action definition missing",
                    )
                })?;
                let scope = step.action.scope();
                ctx.authorize(
                    step.required_capability,
                    step.risk,
                    &scope.entity_ids,
                    scope.map_area,
                )?;
            }
        }
        if all_verified {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cannot cancel a verified or completed plan task",
            ));
        }
        Ok(true)
    })
}

/// Two separately observable phases. Both operate on the original plan, even
/// when the session has since committed another plan.
pub(crate) fn drain(session_id: &str, digest: &str, finalize: bool) -> Result<Value> {
    with_task_session(session_id, |session| {
        durability_gate(session)?;
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        let ids = original_actions(session, digest)?;
        let mut steps = Vec::with_capacity(ids.len());
        let mut remaining = 0usize;
        for id in ids.iter().rev().copied() {
            let before = session.adapter.action_receipt(id).cloned().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "original task action is missing during drain",
                )
            })?;
            if !before.state.is_terminal() {
                session.replay.mark_not_replayable(
                    "modern MCP Tasks cancellation drained original-plan actions outside the tool-call replay log",
                );
                let (_, ctx) = next_context(session)?;
                if finalize {
                    session.adapter.finalize_cancel(id, &ctx)?;
                } else {
                    session
                        .adapter
                        .request_cancel(id, CancelMode::StopFutureSteps, &ctx)?;
                }
            }
            let after = session
                .adapter
                .action_receipt(id)
                .cloned()
                .ok_or_else(|| session_error())?;
            if after.state.is_terminal() {
                release_action_leases(session, id);
            } else {
                remaining += 1;
            }
            steps.push(json!({"action_id": id.to_string(), "before": format!("{:?}", before.state), "after": format!("{:?}", after.state)}));
        }
        steps.reverse();
        session.open_actions.retain(|id| {
            session
                .adapter
                .action_receipt(*id)
                .is_some_and(|receipt| !receipt.state.is_terminal())
        });
        let anchor = anchor_json(&session.adapter.snapshot().anchor());
        let progress = json!({"actions_total": ids.len(), "remaining_nonterminal": remaining,
            "drained": ids.len().saturating_sub(remaining), "quiescent": remaining == 0});
        let certificate = if finalize && remaining == 0 {
            let canonical = json!({"plan_digest": digest, "steps": steps, "anchor": anchor});
            Some(
                json!({"digest": Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex(),
                "statement": "every original plan action is terminal; no task-owned work remains", "anchor": anchor}),
            )
        } else {
            None
        };
        Ok(
            json!({"stage": if finalize { "finalized" } else { "cancel_requested" },
            "plan_digest": digest, "steps": steps, "drain_progress": progress,
            "finalize_certificate": certificate, "observed_anchor": anchor}),
        )
    })
}
