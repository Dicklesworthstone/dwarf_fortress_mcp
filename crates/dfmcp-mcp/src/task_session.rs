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

fn original_receipt(session: &LabSession, digest: &str) -> Result<Value> {
    let raw = session.commit_receipts.get(digest).ok_or_else(|| DfmcpError::new(
        ErrorCode::Conflict,
        "original task plan receipt is unavailable; a restore or session replacement invalidated it; do not retry the effect",
    ))?;
    serde_json::from_str(raw).map_err(|_| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "retained task plan receipt is invalid",
        )
    })
}

fn original_actions(session: &LabSession, digest: &str) -> Result<Vec<ActionId>> {
    let receipt = original_receipt(session, digest)?;
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

/// A witness rebase keeps the requested digest as an idempotent receipt alias.
/// The original objective belongs to the actual sealed plan in that receipt,
/// never to whichever unrelated plan the session prepared or committed later.
fn original_goal(session: &LabSession, requested_digest: &str) -> (PredicateTruth, Value) {
    let actual = original_receipt(session, requested_digest).and_then(|receipt| {
        receipt
            .get("plan_digest")
            .and_then(Value::as_str)
            .and_then(Digest32::from_hex)
            .map(|digest| digest.to_hex())
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::Conflict,
                    "the original task receipt has no exact sealed plan identity for goal verification",
                )
            })
    });
    let observed = actual
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|digest| original_goal_observation(session, digest));
    let (truth, mut payload) = match observed {
        Ok(observed) => observed,
        Err(error) => (
            PredicateTruth::Unknown,
            crate::observation_projection::laboratory_objective_evidence(Err(error)),
        ),
    };
    payload["requested_plan_digest"] = json!(requested_digest);
    payload["plan_digest"] = json!(actual.ok());
    payload["observed_anchor"] = anchor_json(&session.adapter.snapshot().anchor());
    payload["satisfied_at_current_anchor"] = json!(truth == PredicateTruth::True);
    payload["completion_inferred_from_action_states"] = json!(false);
    (truth, payload)
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

    fn action_proof_status(&self) -> McpTaskStatus {
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

    fn proof_status(&self, original_goal: PredicateTruth) -> McpTaskStatus {
        match self.action_proof_status() {
            McpTaskStatus::Completed if original_goal != PredicateTruth::True => {
                if self.remaining_actions == 0 {
                    // The finite original work is finished. An unmet or
                    // unavailable objective requires a new decision, not a
                    // manufactured completion or an automatic replacement.
                    McpTaskStatus::Failed
                } else {
                    McpTaskStatus::Working
                }
            }
            state => state,
        }
    }

    fn monitor_status(&self, original_goal: PredicateTruth) -> McpTaskStatus {
        if self.remaining_actions > 0 {
            McpTaskStatus::Working
        } else {
            self.proof_status(original_goal)
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
    let (goal_truth, goal) = original_goal(session, digest);
    let action_proof_status = progress.action_proof_status();
    let proof_status = progress.proof_status(goal_truth);
    let status = progress.monitor_status(goal_truth);
    let goal_unconfirmed_after_work = progress.remaining_actions == 0
        && action_proof_status == McpTaskStatus::Completed
        && goal_truth != PredicateTruth::True;
    let needs_replan = goal_unconfirmed_after_work && goal_truth == PredicateTruth::False;
    let cleanup_required =
        progress.remaining_work > 0 && action_proof_status != McpTaskStatus::Working;
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
        .briefing(json!({"objective_status": [goal.clone()]}))
        .active_work(work)
        .build();
    Ok(PlanTaskView {
        status,
        payload: json!({
            "ok": proof_status != McpTaskStatus::Failed,
            "schema": "dfmcp.lab-plan-task/1", "session_id": session.session_id.to_string(),
            "plan_digest": digest, "status": status.as_str(), "actions": actions,
            "proof_status": proof_status.as_str(),
            "action_proof_status": action_proof_status.as_str(),
            "original_goal": goal,
            "original_goal_satisfied": goal_truth == PredicateTruth::True,
            "goal_unconfirmed_after_work": goal_unconfirmed_after_work,
            "needs_replan": needs_replan,
            "replacement_work_dispatched": false,
            "cleanup_required": cleanup_required,
            "remaining_work": progress.remaining_work,
            "physical_quiescent": progress.remaining_work == 0,
            "drain_progress": progress.json(),
            "observed_anchor": anchor, "game_tick": session.adapter.snapshot().tick.0,
            "paused": session.adapter.snapshot().paused,
            "indeterminate": progress.indeterminate > 0 || goal_truth == PredicateTruth::Unknown,
            "recovery_class": if progress.indeterminate > 0 || progress.unknown_work > 0
                || goal_truth == PredicateTruth::Unknown {
                "reconciliation_required"
            } else if cleanup_required || needs_replan {
                "operator_action_required"
            } else { "never_unchanged" },
            "blind_retry_allowed": false,
            "agent_turn": packet,
            "scope": "laboratory_process_only",
            "next_step": if goal_unconfirmed_after_work {
                json!({
                    "tool": "fortress.observe", "arguments": {"session_id": session.session_id.to_string()},
                    "note": if needs_replan {
                        "the original work finished but the original goal is unmet; inspect current evidence and explicitly review a new plan; no replacement work was dispatched"
                    } else {
                        "the original work finished but its original goal cannot be established; reconcile the missing goal evidence before deciding on any new work"
                    },
                })
            } else if status == McpTaskStatus::Working
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
        if all_verified && all_drained && original_goal(session, digest).0 == PredicateTruth::True {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "cannot cancel a verified plan task whose original goal holds and physical work is already quiescent",
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
        let (goal_truth, goal) = original_goal(session, digest);
        let proof_status = progress.proof_status(goal_truth);
        let certificate = if finalize && progress.remaining_actions == 0 {
            let canonical = json!({"plan_digest": digest, "steps": steps, "anchor": anchor,
                "proof_status": proof_status.as_str(), "original_goal": goal,
                "action_proof_status": progress.action_proof_status().as_str(),
                "drain_progress": progress.json()});
            Some(
                json!({"digest": Digest32::of_bytes(canonical.to_string().as_bytes()).to_hex(),
                "statement": "every original action receipt is terminal and its registered physical work is quiescent at this anchor; terminal proof receipts are preserved",
                "proof_status": proof_status.as_str(), "anchor": anchor}),
            )
        } else {
            None
        };
        Ok(
            json!({"stage": if finalize { "finalized" } else { "cancel_requested" },
            "plan_digest": digest, "steps": steps, "drain_progress": progress.json(),
            "proof_status": proof_status.as_str(),
            "action_proof_status": progress.action_proof_status().as_str(),
            "original_goal": goal,
            "original_goal_satisfied": goal_truth == PredicateTruth::True,
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
        add_order_with_goal(session_id, early_proof, PredicateTruth::True)
    }

    fn add_order_with_goal(
        session_id: &str,
        early_proof: bool,
        goal: PredicateTruth,
    ) -> Result<(String, ActionId, EntityId)> {
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
            let original_goal = match goal {
                PredicateTruth::True => terminal.clone(),
                PredicateTruth::False => Predicate::FieldCompare {
                    entity_id,
                    field: "amount_total".to_owned(),
                    op: dfmcp_world::CompareOp::Ge,
                    value: dfmcp_world::Value::U64(101),
                },
                PredicateTruth::Unknown => Predicate::FieldCompare {
                    entity_id,
                    field: "unobserved_original_requirement".to_owned(),
                    op: dfmcp_world::CompareOp::Ge,
                    value: dfmcp_world::Value::U64(0),
                },
            };
            let intent = Intent {
                id: intent_id,
                anchor,
                summary: "retain physical work after a terminal proof".to_owned(),
                terminal_condition: original_goal,
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
                json!({"plan_digest": digest, "actions": [{"action_id": id.to_string()}]})
                    .to_string(),
            );
            session.objectives.push(Objective {
                plan_digest: digest.clone(),
                summary: plan.summary.clone(),
                terminal: plan.terminal_condition.clone(),
                committed_tick: anchor.tick.0,
                achieved_tick: None,
            });
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
    fn original_goal_truth_is_independent_of_verified_actions_and_physical_drain() -> TestResult {
        for (truth, label) in [
            (PredicateTruth::True, "true"),
            (PredicateTruth::False, "false"),
            (PredicateTruth::Unknown, "unknown"),
        ] {
            let session_id = new_session()?;
            let (digest, action, _) = add_order_with_goal(&session_id, true, truth)?;
            let receipt = prove(&session_id, action, true)?;
            assert_eq!(receipt.state, CommitState::Verified);
            let active = view(&session_id, &digest)?;
            assert_eq!(active.status, McpTaskStatus::Working);
            assert_eq!(active.payload["action_proof_status"], "completed");
            assert_eq!(active.payload["original_goal"]["predicate_truth"], label);
            assert_eq!(active.payload["physical_quiescent"], false);
            assert_eq!(active.payload["cleanup_required"], true);
            assert_eq!(active.payload["replacement_work_dispatched"], false);

            drain(&session_id, &digest, false)?;
            let finalized = drain(&session_id, &digest, true)?;
            assert_eq!(finalized["original_goal"]["predicate_truth"], label);
            assert_eq!(finalized["drain_progress"]["quiescent"], true);
            let final_view = view(&session_id, &digest)?;
            assert_eq!(
                final_view.status,
                if truth == PredicateTruth::True {
                    McpTaskStatus::Completed
                } else {
                    McpTaskStatus::Failed
                },
                "{}",
                final_view.payload
            );
            assert_eq!(final_view.payload["action_proof_status"], "completed");
            assert_eq!(final_view.payload["physical_quiescent"], true);
            assert_eq!(
                final_view.payload["needs_replan"],
                truth == PredicateTruth::False
            );
            assert_eq!(
                final_view.payload["goal_unconfirmed_after_work"],
                truth != PredicateTruth::True
            );
            assert_eq!(final_view.payload["blind_retry_allowed"], false);
            assert_eq!(
                final_view.payload["agent_turn"]["briefing"]["objective_status"][0],
                final_view.payload["original_goal"]
            );
            if truth == PredicateTruth::Unknown {
                assert_eq!(
                    final_view.payload["recovery_class"],
                    "reconciliation_required"
                );
                assert_eq!(
                    final_view.payload["original_goal"]["epistemic_state"],
                    "unknown"
                );
            }
            with_task_session(&session_id, |session| {
                assert_eq!(session.adapter.action_receipt(action), Some(&receipt));
                assert_eq!(session.commit_receipts.len(), 1);
                Ok(())
            })?;
        }
        Ok(())
    }

    #[test]
    fn original_goal_uses_rebased_receipt_identity_after_a_later_plan() -> TestResult {
        let session_id = new_session()?;
        let (actual, action, _) = add_order(&session_id, true)?;
        prove(&session_id, action, true)?;
        let requested = Digest32::of_bytes(b"requested digest before witness rebase").to_hex();
        with_task_session(&session_id, |session| {
            let receipt = session
                .commit_receipts
                .get(&actual)
                .cloned()
                .ok_or_else(session_error)?;
            session.commit_receipts.insert(requested.clone(), receipt);
            Ok(())
        })?;
        let (later, later_action, _) = add_order(&session_id, false)?;
        drain(&session_id, &requested, false)?;
        drain(&session_id, &requested, true)?;
        let original = view(&session_id, &requested)?;
        assert_eq!(original.status, McpTaskStatus::Completed);
        assert_eq!(original.payload["original_goal"]["plan_digest"], actual);
        assert_eq!(
            original.payload["original_goal"]["requested_plan_digest"],
            requested
        );
        assert_ne!(actual, later);
        assert_eq!(view(&session_id, &later)?.status, McpTaskStatus::Working);
        with_task_session(&session_id, |session| {
            assert!(matches!(
                session.adapter.action_work_state(later_action)?,
                EffectWorkState::Active { .. }
            ));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn missing_original_goal_is_unknown_after_all_original_work_is_quiet() -> TestResult {
        let session_id = new_session()?;
        let (digest, action, _) = add_order(&session_id, true)?;
        prove(&session_id, action, true)?;
        drain(&session_id, &digest, false)?;
        drain(&session_id, &digest, true)?;
        with_task_session(&session_id, |session| {
            session.objectives.clear();
            Ok(())
        })?;
        let failed = view(&session_id, &digest)?;
        assert_eq!(failed.status, McpTaskStatus::Failed);
        assert_eq!(failed.payload["physical_quiescent"], true);
        assert_eq!(
            failed.payload["original_goal"]["predicate_truth"],
            "unknown"
        );
        assert_eq!(failed.payload["needs_replan"], false);
        assert_eq!(failed.payload["recovery_class"], "reconciliation_required");
        assert_eq!(failed.payload["replacement_work_dispatched"], false);
        Ok(())
    }

    #[test]
    fn expired_observe_cannot_establish_task_goal_or_change_original_work() -> TestResult {
        let session_id = new_session()?;
        let (digest, action, _) = add_order(&session_id, true)?;
        let proof = prove(&session_id, action, true)?;
        let before = with_task_session(&session_id, |session| {
            let expired = GameTick(session.adapter.snapshot().tick.0.saturating_sub(1));
            for grant in &mut session.grants {
                if grant.capability == Capability::Observe {
                    grant.expires_at_tick = Some(expired);
                }
            }
            Ok(session.adapter.snapshot().clone())
        })?;
        let Err(error) = view(&session_id, &digest) else {
            return Err("expired Observe must refuse task goal verification".into());
        };
        assert_eq!(error.code, ErrorCode::CapabilityDenied);
        with_task_session(&session_id, |session| {
            assert_eq!(original_goal(session, &digest).0, PredicateTruth::Unknown);
            assert_eq!(session.adapter.snapshot(), &before);
            assert_eq!(session.adapter.action_receipt(action), Some(&proof));
            Ok(())
        })?;
        Ok(())
    }

    #[test]
    fn completed_production_tasks_fail_unmet_consumed_and_joint_original_quotas() -> TestResult {
        for (selector, before_ticks, work_ticks, quotas) in [
            ("7824201", 1099, 200, json!([{"item":"DRINK","minimum":60}])),
            (
                "7824202",
                1149,
                50,
                json!([{"item":"DRINK","minimum":40},{"item":"FOOD","minimum":65}]),
            ),
        ] {
            let opened: Value = serde_json::from_str(&open_session_in_scenario(
                Some(false),
                Some(selector.to_owned()),
                Some(vec![
                    ("observe".to_owned(), "read_only".to_owned()),
                    ("plan".to_owned(), "read_only".to_owned()),
                    ("control_clock".to_owned(), "reversible".to_owned()),
                    ("configure_production".to_owned(), "reversible".to_owned()),
                ]),
                None,
                Some(2000),
                None,
                None,
                Some(8192),
                Some(16),
                Some("starter_fortress".to_owned()),
                None,
                None,
            ))?;
            assert_eq!(opened["ok"], true, "{opened}");
            let session = opened["session_id"].as_str().ok_or("session missing")?;
            let advanced: Value = serde_json::from_str(&wait_with_ticks(
                Some(session.to_owned()),
                Some(before_ticks),
            ))?;
            assert_eq!(advanced["ok"], true, "{advanced}");
            let planned: Value = serde_json::from_str(&plan_request(
                Some(session.to_owned()),
                None,
                None,
                None,
                Some(json!({"template":"production","quotas":quotas}).to_string()),
                None,
            ))?;
            assert_eq!(planned["ok"], true, "{planned}");
            let digest = planned["plan_digest"].as_str().ok_or("plan missing")?;
            let committed: Value = serde_json::from_str(&fortress_commit(
                Some(session.to_owned()),
                digest.to_owned(),
            ))?;
            assert_eq!(committed["ok"], true, "{committed}");
            let settled: Value =
                serde_json::from_str(&wait_with_ticks(Some(session.to_owned()), Some(work_ticks)))?;
            assert_eq!(settled["ok"], true, "{settled}");
            let observed = view(session, digest)?;
            assert_eq!(
                observed.status,
                McpTaskStatus::Failed,
                "{}",
                observed.payload
            );
            assert_eq!(observed.payload["action_proof_status"], "completed");
            assert_eq!(observed.payload["physical_quiescent"], true);
            assert_eq!(
                observed.payload["original_goal"]["predicate_truth"],
                "false"
            );
            assert_eq!(observed.payload["needs_replan"], true);
            assert_eq!(observed.payload["replacement_work_dispatched"], false);
            assert_eq!(observed.payload["blind_retry_allowed"], false);
            assert_eq!(view(session, digest)?.payload, observed.payload);
        }
        Ok(())
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
