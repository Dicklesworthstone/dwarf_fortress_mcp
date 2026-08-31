#![forbid(unsafe_code)]

//! In-memory pause-only dispatcher and two-phase effect-journal laboratory.
//!
//! This module does not execute on the DF game thread and does not talk to DFHack. It
//! exists to exercise prepare/commit/idempotency semantics against a `WorldSnapshot`.

use std::collections::BTreeMap;

use dfmcp_core::{
    ActionId, CommitState, DfmcpError, Digest32, ErrorCode, Evidence, EvidenceId, EvidenceKind,
    GameTick, OperationContext, PlanId, Result, StateAnchor,
};
use dfmcp_intent::{Action, PreparedPlan};
use dfmcp_world::{WorldSnapshot, evaluate};

use crate::{ActionReceipt, CommitReceipt, PrepareReceipt};

const MAX_EFFECT_JOURNAL_RECORDS: usize = 65_536;
const MAX_EFFECT_ERROR_BYTES: usize = 4_096;

/// Record in the process-local effect-journal laboratory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectJournalRecord {
    pub idempotency_key: String,
    pub plan_id: PlanId,
    pub plan_digest: Digest32,
    pub state: CommitState,
    pub dispatch_tick: GameTick,
    pub receipt: Option<CommitReceipt>,
    pub error_message: Option<String>,
}

/// In-memory two-phase effect journal.
#[derive(Clone, Debug, Default)]
pub struct EffectJournal {
    records: BTreeMap<String, EffectJournalRecord>,
}

impl EffectJournal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
        }
    }

    /// Look up an existing transaction by its idempotency key.
    #[must_use]
    pub fn lookup(&self, idempotency_key: &str) -> Option<&EffectJournalRecord> {
        self.records.get(idempotency_key)
    }

    /// Record a new prepared transaction in the journal.
    pub fn record_prepare(
        &mut self,
        idempotency_key: String,
        plan: &PreparedPlan,
        tick: GameTick,
    ) -> Result<()> {
        if let Some(existing) = self.records.get(&idempotency_key) {
            if existing.plan_digest != plan.digest {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    format!(
                        "idempotency key {idempotency_key} already used for different plan digest"
                    ),
                ));
            }
            return Ok(());
        }
        if self.records.len() >= MAX_EFFECT_JOURNAL_RECORDS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "in-memory effect journal reached its explicit record bound",
            ));
        }

        self.records.insert(
            idempotency_key.clone(),
            EffectJournalRecord {
                idempotency_key,
                plan_id: plan.id,
                plan_digest: plan.digest,
                state: CommitState::Prepared,
                dispatch_tick: tick,
                receipt: None,
                error_message: None,
            },
        );
        Ok(())
    }

    /// Record commit completion in the journal.
    pub fn record_commit(&mut self, idempotency_key: &str, receipt: CommitReceipt) -> Result<()> {
        let record = self.records.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                format!("no prepare record found in journal for idempotency key {idempotency_key}"),
            )
        })?;

        if record.state != CommitState::Prepared
            || record.plan_id != receipt.plan_id
            || record.plan_digest != receipt.plan_digest
            || receipt.actions.is_empty()
            || receipt
                .actions
                .iter()
                .any(|action| action.state != CommitState::Verified)
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "commit receipt does not match the prepared journal record",
            ));
        }
        record.state = CommitState::Verified;
        record.receipt = Some(receipt);
        Ok(())
    }

    /// Mark an in-flight mutation as indeterminate due to bridge timeout or disconnect.
    pub fn mark_indeterminate(&mut self, idempotency_key: &str, error_msg: String) -> Result<()> {
        if error_msg.len() > MAX_EFFECT_ERROR_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "effect-journal error message exceeds its explicit byte bound",
            ));
        }
        let record = self.records.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                format!("no prepare record found in journal for idempotency key {idempotency_key}"),
            )
        })?;

        if record.state != CommitState::Prepared && record.state != CommitState::Committing {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "only an in-flight journal record can become indeterminate",
            ));
        }
        record.state = CommitState::Indeterminate;
        record.error_message = Some(error_msg);
        Ok(())
    }

    /// Number of entries in the effect journal.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether journal is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
}

/// Two-phase, in-memory pause-only dispatcher.
#[derive(Clone, Debug, Default)]
pub struct MutationDispatcher {
    journal: EffectJournal,
}

impl MutationDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            journal: EffectJournal::new(),
        }
    }

    /// Phase 1: Prepare mutation plan against current snapshot state.
    pub fn prepare_mutation(
        &mut self,
        plan: &PreparedPlan,
        snapshot: &WorldSnapshot,
        context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        plan.validate_structure()?;
        validate_dispatch_support(plan, context)?;
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "cannot prepare against a snapshot with an invalid state hash",
            ));
        }
        if plan.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "plan expected anchor does not match live snapshot anchor",
            ));
        }
        if context.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "operation context anchor does not match the current snapshot",
            ));
        }
        if snapshot.tick > plan.expires_at_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "prepared plan has expired",
            ));
        }
        for step in &plan.steps {
            let scope = step.action.scope();
            context.authorize(
                step.required_capability,
                step.risk,
                &scope.entity_ids,
                scope.map_area,
            )?;
            if !step
                .preconditions
                .iter()
                .all(|predicate| evaluate(snapshot, predicate))
            {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("preconditions failed for step {}", step.id.get()),
                ));
            }
        }

        // Generate idempotency key derived from plan digest and session
        let idempotency_key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        self.journal
            .record_prepare(idempotency_key, plan, snapshot.tick)?;

        let adapter_token = adapter_token(plan, snapshot, context);
        let adapter_token_digest = Digest32::of_bytes(&adapter_token);

        Ok(PrepareReceipt {
            plan_id: plan.id,
            plan_digest: plan.digest,
            revalidated_anchor: snapshot.anchor(),
            adapter_token,
            adapter_token_digest,
            expires_at_tick: plan.expires_at_tick,
            warnings: Vec::new(),
        })
    }

    /// Phase 2: Commit mutation plan, ensuring idempotency and receipt emission.
    pub fn commit_mutation(
        &mut self,
        plan: &PreparedPlan,
        prepare_receipt: &PrepareReceipt,
        snapshot: &mut WorldSnapshot,
        context: &OperationContext,
    ) -> Result<CommitReceipt> {
        plan.validate_structure()?;
        validate_dispatch_support(plan, context)?;
        // Reauthorization happens even for an idempotent receipt replay. A
        // caller who knows a digest but lacks the session's grants gains no
        // read or mutation authority from the journal.
        for step in &plan.steps {
            let scope = step.action.scope();
            context.authorize(
                step.required_capability,
                step.risk,
                &scope.entity_ids,
                scope.map_area,
            )?;
        }

        // Idempotency check comes before anchor validation: a verified
        // commit already mutated the snapshot, so the caller's context
        // anchor legitimately trails the post-commit snapshot anchor.
        // Returning the cached receipt is safe because the effect was
        // already applied and the caller proved authority above.
        let idempotency_key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        if let Some(existing) = self.journal.lookup(&idempotency_key) {
            if existing.plan_id != plan.id || existing.plan_digest != plan.digest {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "idempotency record does not match the supplied sealed plan",
                ));
            }
            if existing.state == CommitState::Verified {
                return existing.receipt.clone().ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "verified effect-journal record is missing its receipt",
                    )
                });
            }
            if existing.state != CommitState::Prepared {
                return Err(DfmcpError::new(
                    ErrorCode::EffectIndeterminate,
                    "previous commit attempt is not safely retryable until it is reconciled",
                ));
            }
        }

        // Anchor validation applies only to fresh commits: the snapshot
        // must be internally consistent and match the caller's anchor.
        if !snapshot.hash_is_valid() || context.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "commit snapshot or operation context anchor is invalid",
            ));
        }
        if snapshot.tick > plan.expires_at_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "prepared plan has expired before commit",
            ));
        }

        // Validate prepare receipt consistency
        let expected_token = adapter_token(plan, snapshot, context);
        if prepare_receipt.plan_id != plan.id
            || prepare_receipt.plan_digest != plan.digest
            || prepare_receipt.revalidated_anchor != snapshot.anchor()
            || prepare_receipt.expires_at_tick != plan.expires_at_tick
            || prepare_receipt.adapter_token != expected_token
            || prepare_receipt.adapter_token_digest
                != Digest32::of_bytes(&prepare_receipt.adapter_token)
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "prepare receipt is stale or does not match current fortress anchor",
            ));
        }

        let prior_snapshot = snapshot.clone();
        let mut action_receipts = Vec::new();

        // Dispatch each plan step into snapshot mutation
        for step in &plan.steps {
            if !step
                .preconditions
                .iter()
                .all(|predicate| evaluate(snapshot, predicate))
            {
                *snapshot = prior_snapshot;
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("preconditions changed before step {} commit", step.id.get()),
                ));
            }
            let action_id = derived_action_id(plan.id, step.id);
            let msg = match &step.action {
                Action::Pause { paused } => {
                    if snapshot.paused != *paused {
                        let Some(next_cursor) = snapshot.cursor.checked_next() else {
                            *snapshot = prior_snapshot;
                            return Err(DfmcpError::new(
                                ErrorCode::CursorGap,
                                "cannot publish pause mutation because the observation cursor is exhausted",
                            ));
                        };
                        snapshot.paused = *paused;
                        snapshot.cursor = next_cursor;
                        snapshot.refresh_hash();
                    }
                    format!("simulation pause state set to {}", *paused)
                }
                _ => {
                    *snapshot = prior_snapshot;
                    return Err(DfmcpError::new(
                        ErrorCode::AdapterRejected,
                        "in-memory dispatcher supports only the pause action",
                    ));
                }
            };

            if !step
                .postconditions
                .iter()
                .all(|predicate| evaluate(snapshot, predicate))
            {
                *snapshot = prior_snapshot;
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("postconditions failed for step {}", step.id.get()),
                ));
            }

            let mut receipt_bytes = Vec::new();
            receipt_bytes.extend_from_slice(b"dfmcp-dispatch-action-receipt-v1");
            receipt_bytes.extend_from_slice(plan.digest.as_bytes());
            receipt_bytes.extend_from_slice(&action_id.get().to_be_bytes());
            receipt_bytes.extend_from_slice(&step.id.get().to_be_bytes());
            receipt_bytes.extend_from_slice(snapshot.state_hash.as_bytes());
            let receipt_digest = Digest32::of_bytes(&receipt_bytes);

            action_receipts.push(ActionReceipt {
                action_id,
                step_id: step.id,
                state: CommitState::Verified,
                observed_anchor: snapshot.anchor(),
                adapter_receipt_digest: receipt_digest,
                evidence: vec![Evidence {
                    id: EvidenceId::new(action_id.get()),
                    kind: EvidenceKind::Postcondition,
                    subject: None,
                    anchor: snapshot.anchor(),
                    digest: snapshot.state_hash,
                    summary: "in-memory postcondition evaluated against canonical snapshot"
                        .to_owned(),
                }],
                message: msg,
            });
        }

        let commit_receipt = CommitReceipt {
            plan_id: plan.id,
            plan_digest: plan.digest,
            actions: action_receipts,
            checkpoint: None,
            observed_anchor: snapshot.anchor(),
            warnings: Vec::new(),
        };

        if let Err(error) = self
            .journal
            .record_commit(&idempotency_key, commit_receipt.clone())
        {
            *snapshot = prior_snapshot;
            return Err(error);
        }

        Ok(commit_receipt)
    }

    /// Reserved out-of-process prepare seam. It is deliberately unavailable
    /// until an authenticated bridge can revalidate semantic preconditions.
    pub fn prepare(
        &mut self,
        plan: &PreparedPlan,
        current_anchor: StateAnchor,
        context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        let _ = (plan, current_anchor, context);
        Err(DfmcpError::new(
            ErrorCode::CompatibilityUnknown,
            "out-of-process mutation prepare is unavailable without a live bridge adapter",
        ))
    }

    /// Reserved out-of-process commit seam. It never fabricates an effect receipt.
    pub fn commit(
        &mut self,
        plan: &PreparedPlan,
        prepare_receipt: &PrepareReceipt,
        current_anchor: StateAnchor,
        context: &OperationContext,
    ) -> Result<CommitReceipt> {
        let _ = (plan, prepare_receipt, current_anchor, context);
        Err(DfmcpError::new(
            ErrorCode::CompatibilityUnknown,
            "out-of-process mutation commit is unavailable without a live bridge adapter",
        ))
    }

    /// Access reference to internal effect journal.
    #[must_use]
    pub fn journal(&self) -> &EffectJournal {
        &self.journal
    }

    /// Access mutable reference to internal effect journal.
    pub fn journal_mut(&mut self) -> &mut EffectJournal {
        &mut self.journal
    }
}

fn derived_action_id(plan_id: PlanId, step_id: dfmcp_core::StepId) -> ActionId {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-dispatch-action-v1");
    bytes.extend_from_slice(&plan_id.get().to_be_bytes());
    bytes.extend_from_slice(&step_id.get().to_be_bytes());
    let derived = Digest32::of_bytes(&bytes).first_u128();
    ActionId::new(if derived == 0 { 1 } else { derived })
}

fn validate_dispatch_support(plan: &PreparedPlan, context: &OperationContext) -> Result<()> {
    if plan.steps.len() > context.budget.max_actions as usize {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "plan exceeds the in-memory dispatcher action budget",
        ));
    }
    if plan.requires_checkpoint {
        return Err(DfmcpError::new(
            ErrorCode::AdapterRejected,
            "in-memory dispatcher has no checkpoint implementation",
        ));
    }
    if plan
        .steps
        .iter()
        .any(|step| !matches!(step.action, Action::Pause { .. }) || step.obligation.is_some())
    {
        return Err(DfmcpError::new(
            ErrorCode::AdapterRejected,
            "in-memory dispatcher supports only immediate pause actions without obligations",
        ));
    }
    Ok(())
}

fn adapter_token(
    plan: &PreparedPlan,
    snapshot: &WorldSnapshot,
    context: &OperationContext,
) -> Vec<u8> {
    let mut token = Vec::new();
    token.extend_from_slice(b"dfmcp-memory-dispatch-token-v1");
    token.extend_from_slice(&context.session_id.get().to_be_bytes());
    token.extend_from_slice(&plan.id.get().to_be_bytes());
    token.extend_from_slice(plan.digest.as_bytes());
    token.extend_from_slice(snapshot.state_hash.as_bytes());
    token.extend_from_slice(&plan.expires_at_tick.0.to_be_bytes());
    token
}

#[cfg(test)]
mod tests {
    use super::*;
    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, FortressId, IntentId, ObservationCursor,
        RequestId, RiskTier, SessionId, WorkBudget,
    };
    use dfmcp_intent::{Constraint, Intent, RequestedAction, StaticPlanner};
    use dfmcp_world::{Predicate, WorldGraph};

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
            budget: WorkBudget::CONSERVATIVE_DEFAULT,
            grants: vec![
                CapabilityGrant {
                    capability: Capability::Plan,
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
                CapabilityGrant {
                    capability: Capability::Designate,
                    scope: CapabilityScope::default(),
                    max_risk: RiskTier::Guarded,
                    expires_at_tick: None,
                    remaining_uses: None,
                },
            ],
            cancellation_requested: false,
        }
    }

    #[test]
    fn test_two_phase_prepare_and_commit() -> Result<()> {
        let mut snapshot = sample_snapshot();
        let ctx = sample_context(&snapshot);

        let intent = Intent {
            id: IntentId::new(1),
            anchor: snapshot.anchor(),
            summary: "unpause simulation".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![RequestedAction {
                action: Action::Pause { paused: false },
                preconditions: vec![Predicate::Paused(true)],
                postconditions: Vec::new(),
                compensation: None,
                obligation: None,
                depends_on: Vec::new(),
            }],
        };

        let plan = StaticPlanner::default().prepare(&snapshot, &intent, &ctx)?;
        let mut dispatcher = MutationDispatcher::new();

        let prepare_receipt = dispatcher.prepare_mutation(&plan, &snapshot, &ctx)?;
        assert_eq!(prepare_receipt.plan_id, plan.id);
        assert_eq!(prepare_receipt.plan_digest, plan.digest);

        let commit_receipt =
            dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &ctx)?;
        assert_eq!(commit_receipt.plan_id, plan.id);
        assert_eq!(commit_receipt.actions.len(), 1);
        assert_eq!(commit_receipt.actions[0].state, CommitState::Verified);
        assert!(!snapshot.paused);
        assert_eq!(snapshot.cursor.sequence, 1);

        // Idempotency replay
        let replay_receipt =
            dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &ctx)?;
        assert_eq!(replay_receipt, commit_receipt);

        Ok(())
    }

    #[test]
    fn test_stale_anchor_prepare_rejection() -> Result<()> {
        let snapshot = sample_snapshot();
        let ctx = sample_context(&snapshot);

        let mut mutated_anchor_snapshot = snapshot.clone();
        mutated_anchor_snapshot.tick = GameTick(200);

        let intent = Intent {
            id: IntentId::new(1),
            anchor: snapshot.anchor(),
            summary: "unpause simulation".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![RequestedAction {
                action: Action::Pause { paused: false },
                preconditions: Vec::new(),
                postconditions: Vec::new(),
                compensation: None,
                obligation: None,
                depends_on: Vec::new(),
            }],
        };

        let plan = StaticPlanner::default().prepare(&snapshot, &intent, &ctx)?;
        let mut dispatcher = MutationDispatcher::new();

        // Prepare against mismatched anchor must fail
        let result = dispatcher.prepare_mutation(&plan, &mutated_anchor_snapshot, &ctx);
        assert!(result.is_err());

        Ok(())
    }
}
