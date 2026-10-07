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

#[derive(Default)]
struct TaskProgress {
    total: usize,
    verified: usize,
    failed: usize,
    indeterminate: usize,
    remaining_nonterminal: usize,
    remaining_work: usize,
    unknown_work: usize,
    remaining_actions: usize,
}

impl TaskProgress {
    fn observe(&mut self, state: CommitState, work: &Value) {
        self.total += 1;
        self.verified += usize::from(state == CommitState::Verified);
        self.failed += usize::from(state == CommitState::Failed);
        self.indeterminate += usize::from(state == CommitState::Indeterminate);
        let terminal = state.is_terminal();
        let quiet = work["quiescent"].as_bool() == Some(true);
        self.remaining_nonterminal += usize::from(!terminal);
        self.remaining_work += usize::from(!quiet);
        self.unknown_work += usize::from(work["state"].as_str() == Some("unknown"));
        self.remaining_actions += usize::from(!terminal || !quiet);
    }

    fn proof_status(&self) -> McpTaskStatus {
        if self.failed > 0 || self.indeterminate > 0 {
            McpTaskStatus::Failed
        } else if self.verified == self.total {
            McpTaskStatus::Completed
        } else if self.remaining_nonterminal == 0 {
            McpTaskStatus::Cancelled
        } else {
            McpTaskStatus::Working
        }
    }

    fn monitor_status(&self) -> McpTaskStatus {
        if self.remaining_actions > 0 {
            McpTaskStatus::Working
        } else {
            self.proof_status()
        }
    }

    fn json(&self) -> Value {
        json!({
            "actions_total": self.total,
            "remaining_nonterminal": self.remaining_nonterminal,
            "remaining_work": self.remaining_work,
            "unknown_work": self.unknown_work,
            "remaining_actions": self.remaining_actions,
            "drained": self.total.saturating_sub(self.remaining_actions),
            "physical_quiescent": self.remaining_work == 0,
            "quiescent": self.remaining_actions == 0,
        })
    }
}

fn authorize_original_action(
    session: &LabSession,
    id: ActionId,
    context: &OperationContext,
) -> Result<()> {
    let step = session.adapter.action_step(id).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "task action definition missing",
        )
    })?;
    let scope = step.action.scope();
    context.authorize(
        step.required_capability,
        step.risk,
        &scope.entity_ids,
        scope.map_area,
    )
}

fn view_locked(session: &LabSession, digest: &str) -> Result<PlanTaskView> {
    let ids = original_actions(session, digest)?;
    let mut actions = Vec::with_capacity(ids.len());
    let mut progress = TaskProgress::default();
    for id in ids {
        let receipt = session.adapter.action_receipt(id).ok_or_else(|| DfmcpError::new(
            ErrorCode::Conflict, "original task action no longer exists; refresh after restore and do not retry its effect",
        ))?;
        let work = action_work_json(&session.adapter, id);
        progress.observe(receipt.state, &work);
        let fully_drained = receipt.state.is_terminal() && work["quiescent"] == true;
        let step = session.adapter.action_step(id);
        actions.push(json!({
            "action_id": id.to_string(), "step": receipt.step_id.get(),
            "state": format!("{:?}", receipt.state), "message": receipt.message,
            "work_state": work, "fully_drained": fully_drained,
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
    // The transport monitor owns cleanup as well as observation. A final goal
    // proof cannot close its original task while physical work can still run.
    // Keep the proof outcome explicit; no terminal action receipt is rewritten.
    let proof_status = progress.proof_status();
    let status = progress.monitor_status();
    let cleanup_required = progress.remaining_work > 0 && proof_status != McpTaskStatus::Working;
    let anchor = anchor_json(&session.adapter.snapshot().anchor());
    let active = actions
        .iter()
        .filter(|action| action["fully_drained"] != true)
        .cloned()
        .collect::<Vec<_>>();
    let mut work = crate::empty_active_work();
    work["actions"] = json!(active);
    if cleanup_required {
        work["cancellation_drains"] = json!([{
            "plan_digest": digest, "proof_status": proof_status.as_str(),
            "drain_progress": progress.json(),
            "target": "the task's original plan",
        }]);
    }
    work["indeterminate_effects"] = json!(
        actions
            .iter()
            .filter(|action| {
                action["state"] == "Indeterminate" || action["work_state"]["state"] == "unknown"
            })
            .cloned()
            .collect::<Vec<_>>()
    );
    let packet = crate::AgentTurnBuilder::new("fortress.commit", crate::AgentPhase::Verify)
        .session_id(session.session_id.to_string())
        .anchor(anchor.clone())
        .active_work(work)
        .build();
    Ok(PlanTaskView {
        status,
        payload: json!({
            "ok": proof_status != McpTaskStatus::Failed,
            "schema": "dfmcp.lab-plan-task/1", "session_id": session.session_id.to_string(),
            "plan_digest": digest, "status": status.as_str(), "actions": actions,
            "proof_status": proof_status.as_str(),
            "cleanup_required": cleanup_required,
            "remaining_work": progress.remaining_work,
            "physical_quiescent": progress.remaining_work == 0,
            "drain_progress": progress.json(),
            "observed_anchor": anchor, "game_tick": session.adapter.snapshot().tick.0,
            "paused": session.adapter.snapshot().paused,
            "indeterminate": progress.indeterminate > 0,
            "recovery_class": if progress.indeterminate > 0 || progress.unknown_work > 0 {
                "reconciliation_required"
            } else if cleanup_required {
                "operator_action_required"
            } else { "never_unchanged" },
            "blind_retry_allowed": false,
            "agent_turn": packet,
            "scope": "laboratory_process_only",
            "next_step": if status == McpTaskStatus::Working
                && (proof_status == McpTaskStatus::Failed || progress.unknown_work > 0) {
                json!({
                    "tool": "fortress.observe", "arguments": {"session_id": session.session_id.to_string()},
                    "note": "the goal proof is failed or physical work is unresolved; inspect current evidence and use tasks/cancel with this task's original handle for authorized cleanup",
                })
            } else if status == McpTaskStatus::Working { json!({
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
/// task projection cannot cancel an already verified and physically quiet
/// plan. Loss of action authority cannot be hidden by transport cancellation.
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
        let mut all_drained = true;
        for id in ids {
            let receipt = session.adapter.action_receipt(id).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "original task action is missing; cancellation cannot prove quiescence",
                )
            })?;
            all_verified &= receipt.state == CommitState::Verified;
            let drained = action_fully_drained(&session.adapter, id);
            all_drained &= drained;
            if !drained {
                authorize_original_action(session, id, &ctx)?;
            }
        }
        if all_verified && all_drained {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cannot cancel a verified plan task whose physical work is already quiescent",
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
        for id in ids.iter().rev().copied() {
            let before = session.adapter.action_receipt(id).cloned().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "original task action is missing during drain",
                )
            })?;
            let before_work = action_work_json(&session.adapter, id);
            let mut effect_drain = Value::Null;
            if !before.state.is_terminal() {
                let (_, ctx) = next_context(session)?;
                if finalize {
                    session.adapter.finalize_cancel(id, &ctx)?;
                } else {
                    session
                        .adapter
                        .request_cancel(id, CancelMode::StopFutureSteps, &ctx)?;
                }
                session.replay.mark_not_replayable(
                    "modern MCP Tasks cancellation drained original-plan actions outside the tool-call replay log",
                );
            } else if before_work["quiescent"] != true {
                // A failed deadline or early Verified proof does not stop the
                // original effect. Request observes and authorizes only; the
                // separately recorded finalization performs a stop-only drain.
                let ctx = context_for(session, session.next_request_id);
                authorize_original_action(session, id, &ctx)?;
                if finalize {
                    let (_, ctx) = next_context(session)?;
                    let receipt = session.adapter.drain_action_work(id, &ctx)?;
                    effect_drain = effect_drain_json(&receipt);
                    session.replay.mark_not_replayable(
                        "modern MCP Tasks cancellation drained original-plan actions outside the tool-call replay log",
                    );
                }
            }
            release_action_leases(session, id);
            steps.push(json!({
                "action_id": id.to_string(), "before": format!("{:?}", before.state),
                "before_work": before_work, "effect_drain": effect_drain,
                "proof_receipt_preserved": before.state.is_terminal(),
            }));
        }
        steps.reverse();
        // Inspect all original identities again at the final anchor, after
        // every stop. Earlier per-action observations cannot certify the
        // final state of the whole plan.
        let mut progress = TaskProgress::default();
        for (id, row) in ids.iter().copied().zip(steps.iter_mut()) {
            let after = session
                .adapter
                .action_receipt(id)
                .ok_or_else(session_error)?;
            let after_work = action_work_json(&session.adapter, id);
            progress.observe(after.state, &after_work);
            row["after"] = json!(format!("{:?}", after.state));
            row["receipt_digest"] = json!(after.adapter_receipt_digest.to_hex());
            row["receipt_anchor"] = anchor_json(&after.observed_anchor);
            row["evidence"] =
                json!(after.evidence.iter().map(|evidence| json!({
                "evidence_id": evidence.id.to_string(), "digest": evidence.digest.to_hex(),
                "kind": format!("{:?}", evidence.kind), "summary": evidence.summary,
                "anchor": anchor_json(&evidence.anchor),
            })).collect::<Vec<_>>());
            row["quiescent"] = json!(after.state.is_terminal() && after_work["quiescent"] == true);
            row["after_work"] = after_work;
        }
        retain_open_actions(session);
        let anchor = anchor_json(&session.adapter.snapshot().anchor());
        if finalize && progress.remaining_actions > 0 {
            return Err(DfmcpError::new(
                ErrorCode::CancellationIncomplete,
                "original task actions are not all terminal with proven physical quiescence",
            ));
        }
        let certificate = if finalize && progress.remaining_actions == 0 {
            let canonical = json!({"plan_digest": digest, "steps": steps, "anchor": anchor,
                "proof_status": progress.proof_status().as_str(), "drain_progress": progress.json()});
            Some(
                json!({"digest": Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex(),
                "statement": "every original action receipt is terminal and its registered physical work is quiescent at this anchor; terminal proof receipts are preserved",
                "proof_status": progress.proof_status().as_str(), "anchor": anchor}),
            )
        } else {
            None
        };
        Ok(
            json!({"stage": if finalize { "finalized" } else { "cancel_requested" },
            "plan_digest": digest, "steps": steps, "drain_progress": progress.json(),
            "proof_status": progress.proof_status().as_str(),
            "remaining_work": progress.remaining_work,
            "physical_quiescent": progress.remaining_work == 0,
            "active_work": {"actions": steps.iter().filter(|row| row["quiescent"] != true).collect::<Vec<_>>()},
            "finalize_certificate": certificate, "observed_anchor": anchor}),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_adapter::ActionReceipt;
    use dfmcp_core::StepId;
    use dfmcp_intent::{ObligationSpec, derive_step_idempotency_key, effects};

    type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

    fn new_session() -> TestResult<String> {
        let opened: Value = serde_json::from_str(&fortress_open_session(
            Some(false),
            Some("78241".to_owned()),
            Some(vec![
                ("observe".to_owned(), "read_only".to_owned()),
                ("plan".to_owned(), "read_only".to_owned()),
                ("configure_production".to_owned(), "reversible".to_owned()),
            ]),
            None,
            None,
            None,
            None,
            None,
            None,
        ))?;
        Ok(opened["session_id"]
            .as_str()
            .ok_or("task test could not open its laboratory session")?
            .to_owned())
    }

    fn add_order(session_id: &str, early_proof: bool) -> Result<(String, ActionId, EntityId)> {
        with_task_session(session_id, |session| {
            let anchor = session.adapter.snapshot().anchor();
            let action = Action::CreateWorkOrder {
                name: "task-owned bounded production".to_owned(),
                job_token: "MAKE_TEST".to_owned(),
                amount: 100,
                conditions: Vec::new(),
            };
            let intent_id = IntentId::new(771);
            let key = derive_step_idempotency_key(intent_id, anchor, StepId::new(0), &action);
            let entity_id = effects::created_entity_id(&key, 0);
            let postconditions = if early_proof {
                vec![Predicate::EntityExists(entity_id)]
            } else {
                effects::default_postconditions(&action, &key, session.fortress_id)
            };
            let terminal = Predicate::All(postconditions.clone()).normalized();
            let intent = Intent {
                id: intent_id,
                anchor,
                summary: "retain physical work after a terminal proof".to_owned(),
                terminal_condition: terminal.clone(),
                constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
                requested_actions: vec![RequestedAction {
                    action,
                    preconditions: Vec::new(),
                    postconditions,
                    compensation: None,
                    obligation: Some(ObligationSpec {
                        terminal,
                        failure: None,
                        deadline_tick: GameTick(anchor.tick.0 + 10),
                        poll_interval_ticks: 1,
                        stable_for_observations: 1,
                    }),
                    depends_on: Vec::new(),
                }],
            };
            let context = context_for(session, session.next_request_id);
            let plan = StaticPlanner::default().prepare_laboratory(
                session.adapter.snapshot(),
                &intent,
                &context,
            )?;
            let prepared = session.adapter.prepare(&plan, &context)?;
            let committed = session.adapter.commit(&plan, &prepared, &context)?;
            let id = committed
                .actions
                .first()
                .ok_or_else(session_error)?
                .action_id;
            let digest = plan.digest.to_hex();
            session.commit_receipts.insert(
                digest.clone(),
                json!({"actions": [{"action_id": id.to_string()}]}).to_string(),
            );
            session.last_action = Some(id);
            session.last_plan_actions = vec![id];
            session.open_actions.push(id);
            Ok((digest, id, entity_id))
        })
    }

    fn prove(session_id: &str, id: ActionId, early: bool) -> Result<ActionReceipt> {
        with_task_session(session_id, |session| {
            session.adapter.advance_ticks(if early { 1 } else { 10 })?;
            let context = context_for(session, session.next_request_id);
            session.adapter.poll_action(id, &context)
        })
    }

    #[test]
    fn terminal_goal_keeps_the_task_open_until_separate_physical_drain() -> TestResult {
        for early in [false, true] {
            let session_id = new_session()?;
            let (digest, id, entity_id) = add_order(&session_id, early)?;
            let proof = prove(&session_id, id, early)?;
            assert_eq!(
                proof.state,
                if early {
                    CommitState::Verified
                } else {
                    CommitState::Failed
                }
            );
            let proof_status = if early { "completed" } else { "failed" };
            let observed = view(&session_id, &digest)?;
            assert_eq!(observed.status, McpTaskStatus::Working);
            assert_eq!(observed.payload["proof_status"], proof_status);
            assert_eq!(observed.payload["remaining_work"], 1);
            assert_eq!(
                observed.payload["drain_progress"]["remaining_nonterminal"],
                0
            );
            assert_eq!(
                observed.payload["actions"][0]["work_state"]["state"],
                "active"
            );
            assert_eq!(
                observed.payload["agent_turn"]["active_work"]["actions"]
                    .as_array()
                    .map(Vec::len),
                Some(1)
            );
            let before = with_task_session(&session_id, |session| {
                Ok(session.adapter.snapshot().clone())
            })?;

            assert!(cancellation_admission(&session_id, &digest)?);
            let request = drain(&session_id, &digest, false)?;
            assert_eq!(request["proof_status"], proof_status);
            assert_eq!(request["drain_progress"]["remaining_work"], 1);
            assert_eq!(request["drain_progress"]["quiescent"], false);
            assert!(request["finalize_certificate"].is_null());
            assert!(request["steps"][0]["effect_drain"].is_null());
            with_task_session(&session_id, |session| {
                assert_eq!(session.adapter.snapshot(), &before);
                assert_eq!(session.adapter.action_receipt(id), Some(&proof));
                assert!(session.open_actions.contains(&id));
                Ok(())
            })?;

            let finalized = drain(&session_id, &digest, true)?;
            assert_eq!(finalized["proof_status"], proof_status);
            assert_eq!(finalized["drain_progress"]["remaining_work"], 0);
            assert_eq!(finalized["drain_progress"]["quiescent"], true);
            assert!(finalized["finalize_certificate"].is_object());
            assert_eq!(finalized["steps"][0]["effect_drain"]["stopped_work"], true);
            assert_eq!(
                finalized["steps"][0]["receipt_digest"],
                proof.adapter_receipt_digest.to_hex()
            );
            assert_eq!(
                request["steps"][0]["evidence"],
                finalized["steps"][0]["evidence"]
            );
            with_task_session(&session_id, |session| {
                assert_eq!(session.adapter.action_receipt(id), Some(&proof));
                assert!(!session.open_actions.contains(&id));
                let stopped = session
                    .adapter
                    .snapshot()
                    .graph
                    .entities
                    .get(&entity_id)
                    .cloned();
                session.adapter.advance_ticks(500)?;
                assert_eq!(
                    session.adapter.snapshot().graph.entities.get(&entity_id),
                    stopped.as_ref()
                );
                assert_eq!(session.adapter.action_receipt(id), Some(&proof));
                Ok(())
            })?;
            let finished = view(&session_id, &digest)?;
            assert_eq!(
                finished.status,
                if early {
                    McpTaskStatus::Completed
                } else {
                    McpTaskStatus::Failed
                }
            );
            assert_eq!(finished.payload["proof_status"], proof_status);
            assert_eq!(
                finished.payload["agent_turn"]["active_work"]["actions"],
                json!([])
            );
            if early {
                let Err(error) = cancellation_admission(&session_id, &digest) else {
                    return Err("a verified and quiet task must refuse cancellation".into());
                };
                assert_eq!(error.code, ErrorCode::Conflict);
            }
        }
        Ok(())
    }

    #[test]
    fn terminal_task_cleanup_keeps_a_later_plan_running() -> TestResult {
        let session_id = new_session()?;
        let (original_digest, original_id, _) = add_order(&session_id, false)?;
        let original_proof = prove(&session_id, original_id, false)?;
        let (later_digest, later_id, later_entity) = add_order(&session_id, false)?;
        assert_ne!(original_digest, later_digest);
        let later_before = with_task_session(&session_id, |session| {
            Ok(session
                .adapter
                .snapshot()
                .graph
                .entities
                .get(&later_entity)
                .cloned())
        })?;
        drain(&session_id, &original_digest, false)?;
        let finalized = drain(&session_id, &original_digest, true)?;
        assert_eq!(finalized["plan_digest"], original_digest);
        assert_eq!(finalized["drain_progress"]["actions_total"], 1);
        with_task_session(&session_id, |session| {
            assert_eq!(session.last_plan_actions, vec![later_id]);
            assert_eq!(
                session.adapter.action_receipt(original_id),
                Some(&original_proof)
            );
            assert_eq!(
                session.adapter.snapshot().graph.entities.get(&later_entity),
                later_before.as_ref()
            );
            assert!(matches!(
                session.adapter.action_work_state(later_id)?,
                EffectWorkState::Active { .. }
            ));
            assert!(session.open_actions.contains(&later_id));
            Ok(())
        })?;
        assert_eq!(
            view(&session_id, &later_digest)?.status,
            McpTaskStatus::Working
        );
        Ok(())
    }

    #[test]
    fn terminal_task_cleanup_reauthorizes_admission_request_and_finalization() -> TestResult {
        let session_id = new_session()?;
        let (digest, id, _) = add_order(&session_id, false)?;
        let proof = prove(&session_id, id, false)?;
        let (grants, before) = with_task_session(&session_id, |session| {
            let grants = session.grants.clone();
            session
                .grants
                .retain(|grant| grant.capability != Capability::ConfigureProduction);
            Ok((grants, session.adapter.snapshot().clone()))
        })?;
        for admitted in [
            cancellation_admission(&session_id, &digest).map(|_| ()),
            drain(&session_id, &digest, false).map(|_| ()),
        ] {
            let Err(error) = admitted else {
                return Err("terminal work cleanup must require current action authority".into());
            };
            assert_eq!(error.code, ErrorCode::CapabilityDenied);
        }
        with_task_session(&session_id, |session| {
            assert_eq!(session.adapter.snapshot(), &before);
            assert_eq!(session.adapter.action_receipt(id), Some(&proof));
            session.grants = grants.clone();
            Ok(())
        })?;
        assert!(cancellation_admission(&session_id, &digest)?);
        drain(&session_id, &digest, false)?;
        with_task_session(&session_id, |session| {
            session
                .grants
                .retain(|grant| grant.capability != Capability::ConfigureProduction);
            Ok(())
        })?;
        let Err(error) = drain(&session_id, &digest, true) else {
            return Err("finalization cannot reuse request-phase authority".into());
        };
        assert_eq!(error.code, ErrorCode::CapabilityDenied);
        with_task_session(&session_id, |session| {
            assert_eq!(session.adapter.snapshot(), &before);
            assert_eq!(session.adapter.action_receipt(id), Some(&proof));
            assert!(matches!(
                session.adapter.action_work_state(id)?,
                EffectWorkState::Active { .. }
            ));
            session.grants = grants;
            Ok(())
        })?;
        assert_eq!(
            drain(&session_id, &digest, true)?["drain_progress"]["quiescent"],
            true
        );
        Ok(())
    }
}
