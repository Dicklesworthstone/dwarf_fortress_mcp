#![forbid(unsafe_code)]

//! In-memory dispatcher and two-phase effect-journal laboratory.
//!
//! This module does not execute on the DF game thread and does not talk to DFHack. It
//! executes the reference action semantics of `dfmcp_intent::effects` against a
//! `WorldSnapshot` to exercise prepare/commit/idempotency semantics, dependency
//! gating and obligation proof. `reconcile` proves pending obligations against
//! whatever later snapshot the caller observed; it never assumes progress.

use std::collections::{BTreeMap, BTreeSet};

use dfmcp_core::{
    ActionId, CommitState, DfmcpError, Digest32, ErrorCode, Evidence, EvidenceId, EvidenceKind,
    GameTick, OperationContext, PlanId, Result, StateAnchor,
};
use dfmcp_intent::execution::{DeferredStepDecision, deferred_step_decision_with_evidence};
use dfmcp_intent::{PlanStep, PreparedPlan, effects};
use dfmcp_world::{Predicate, PredicateEvidence, WorldSnapshot};

use crate::{ActionReceipt, CommitReceipt, PrepareReceipt};

const MAX_EFFECT_JOURNAL_RECORDS: usize = 65_536;
const MAX_EFFECT_ERROR_BYTES: usize = 4_096;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 512;

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
        validate_idempotency_key(&idempotency_key)?;
        plan.validate_structure()?;
        if tick >= plan.expires_at_tick {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "cannot journal a plan at or after its expiry tick",
            ));
        }
        if let Some(existing) = self.records.get(&idempotency_key) {
            if existing.plan_id != plan.id || existing.plan_digest != plan.digest {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    format!(
                        "idempotency key {idempotency_key} already belongs to different sealed plan content"
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

    /// Persist the transition from prepared to an effect attempt before any
    /// mutation is applied. A caller that proves no effect occurred may restore
    /// the prior journal image; an ambiguous failure must instead remain
    /// `Committing` or become `Indeterminate`.
    pub fn record_commit_attempt(&mut self, idempotency_key: &str) -> Result<()> {
        validate_idempotency_key(idempotency_key)?;
        let record = self.records.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                format!("no prepare record found in journal for idempotency key {idempotency_key}"),
            )
        })?;
        match record.state {
            CommitState::Prepared => {
                record.state = CommitState::Committing;
                record.error_message = None;
                Ok(())
            }
            CommitState::Committing => Ok(()),
            CommitState::Verified => Err(DfmcpError::new(
                ErrorCode::Conflict,
                "verified journal record cannot begin another effect attempt",
            )),
            _ => Err(DfmcpError::new(
                ErrorCode::EffectIndeterminate,
                "journal record is not safely dispatchable until reconciled",
            )),
        }
    }

    /// Record commit completion in the journal.
    pub fn record_commit(&mut self, idempotency_key: &str, receipt: CommitReceipt) -> Result<()> {
        validate_idempotency_key(idempotency_key)?;
        let record = self.records.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                format!("no prepare record found in journal for idempotency key {idempotency_key}"),
            )
        })?;

        if matches!(
            record.state,
            CommitState::Verified | CommitState::AppliedAwaitingVerification
        ) && record.state == aggregate_state(&receipt)
        {
            return if record.receipt.as_ref() == Some(&receipt) {
                Ok(())
            } else {
                Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "verified journal record was presented a different commit receipt",
                ))
            };
        }
        let unique_actions: BTreeSet<ActionId> = receipt
            .actions
            .iter()
            .map(|action| action.action_id)
            .collect();
        let unique_steps: BTreeSet<_> = receipt
            .actions
            .iter()
            .map(|action| action.step_id)
            .collect();
        let final_anchor_matches = receipt
            .actions
            .last()
            .is_some_and(|action| action.observed_anchor == receipt.observed_anchor);
        if record.state != CommitState::Committing
            || record.plan_id != receipt.plan_id
            || record.plan_digest != receipt.plan_digest
            || receipt.actions.is_empty()
            || receipt.actions.iter().any(|action| {
                !matches!(
                    action.state,
                    CommitState::Verified
                        | CommitState::AppliedAwaitingVerification
                        | CommitState::Prepared
                )
            })
            || unique_actions.len() != receipt.actions.len()
            || unique_steps.len() != receipt.actions.len()
            || !final_anchor_matches
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "commit receipt does not match the committing journal record",
            ));
        }
        record.state = aggregate_state(&receipt);
        record.receipt = Some(receipt);
        record.error_message = None;
        Ok(())
    }

    /// Replace the receipt of a dispatched transaction after its pending
    /// obligations were re-evaluated or deferred steps were dispatched.
    pub fn record_reconciliation(
        &mut self,
        idempotency_key: &str,
        receipt: CommitReceipt,
    ) -> Result<()> {
        validate_idempotency_key(idempotency_key)?;
        let record = self.records.get_mut(idempotency_key).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                format!("no journal record found for idempotency key {idempotency_key}"),
            )
        })?;
        let Some(prior) = record.receipt.as_ref() else {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "only a dispatched transaction can be reconciled",
            ));
        };
        if prior.plan_id != receipt.plan_id
            || prior.plan_digest != receipt.plan_digest
            || prior.actions.len() != receipt.actions.len()
            || prior
                .actions
                .iter()
                .zip(&receipt.actions)
                .any(|(old, new)| {
                    old.action_id != new.action_id
                        || old.step_id != new.step_id
                        || (old.state.is_terminal() && old != new)
                })
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "reconciled receipt does not extend the dispatched transaction",
            ));
        }
        record.state = aggregate_state(&receipt);
        record.receipt = Some(receipt);
        Ok(())
    }

    /// Mark an in-flight mutation as indeterminate due to bridge timeout or disconnect.
    pub fn mark_indeterminate(&mut self, idempotency_key: &str, error_msg: String) -> Result<()> {
        validate_idempotency_key(idempotency_key)?;
        if error_msg.is_empty() || error_msg.len() > MAX_EFFECT_ERROR_BYTES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "effect-journal error message must contain 1..=4096 bytes",
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

/// Transaction state derived from its actions: any failure fails the whole
/// transaction, all verified verifies it, otherwise work remains pending.
fn aggregate_state(receipt: &CommitReceipt) -> CommitState {
    if receipt
        .actions
        .iter()
        .any(|action| action.state == CommitState::Failed)
    {
        CommitState::Failed
    } else if receipt
        .actions
        .iter()
        .all(|action| action.state == CommitState::Verified)
    {
        CommitState::Verified
    } else {
        CommitState::AppliedAwaitingVerification
    }
}

fn validate_idempotency_key(idempotency_key: &str) -> Result<()> {
    if idempotency_key.is_empty()
        || idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
        || idempotency_key.chars().any(char::is_control)
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "idempotency key must contain 1..=512 non-control bytes",
        ));
    }
    Ok(())
}

/// Two-phase, in-memory dispatcher for the reference action semantics.
#[derive(Clone, Debug, Default)]
pub struct MutationDispatcher {
    journal: EffectJournal,
    /// Distinct observations on which each pending obligation held.
    stability: BTreeMap<ActionId, (u32, Option<StateAnchor>)>,
}

impl MutationDispatcher {
    #[must_use]
    pub fn new() -> Self {
        Self {
            journal: EffectJournal::new(),
            stability: BTreeMap::new(),
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
        let observation = PredicateEvidence::laboratory(snapshot)?;
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
        if snapshot.tick >= plan.expires_at_tick {
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
            if !predicates_established(&observation, &step.preconditions)? {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!(
                        "preconditions for step {} are not established true",
                        step.id.get()
                    ),
                ));
            }
        }

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
        for step in &plan.steps {
            let scope = step.action.scope();
            context.authorize(
                step.required_capability,
                step.risk,
                &scope.entity_ids,
                scope.map_area,
            )?;
        }

        let idempotency_key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        if let Some(existing) = self.journal.lookup(&idempotency_key) {
            if existing.plan_id != plan.id || existing.plan_digest != plan.digest {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "idempotency record does not match the supplied sealed plan",
                ));
            }
            if matches!(
                existing.state,
                CommitState::Verified
                    | CommitState::AppliedAwaitingVerification
                    | CommitState::Failed
            ) {
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

        if !snapshot.hash_is_valid() || context.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "commit snapshot or operation context anchor is invalid",
            ));
        }
        if snapshot.tick >= plan.expires_at_tick {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "prepared plan has expired before commit",
            ));
        }

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
        let prior_journal = self.journal.clone();
        let prior_stability = self.stability.clone();
        self.journal.record_commit_attempt(&idempotency_key)?;
        let result = (|| {
            let mut action_receipts: Vec<ActionReceipt> = Vec::with_capacity(plan.steps.len());
            for step in &plan.steps {
                let action_id = derived_action_id(plan.id, step.id);
                let receipt = if dependencies_verified(step, &action_receipts) {
                    self.dispatch_step(plan, step, action_id, snapshot)?
                } else {
                    action_receipt(
                        plan,
                        step,
                        action_id,
                        CommitState::Prepared,
                        snapshot,
                        "deferred until every dependency is verified".to_owned(),
                    )
                };
                action_receipts.push(receipt);
            }

            let commit_receipt = CommitReceipt {
                plan_id: plan.id,
                plan_digest: plan.digest,
                actions: action_receipts,
                checkpoint: None,
                observed_anchor: snapshot.anchor(),
                warnings: Vec::new(),
            };
            self.journal
                .record_commit(&idempotency_key, commit_receipt.clone())?;
            Ok(commit_receipt)
        })();
        if result.is_err() {
            *snapshot = prior_snapshot;
            self.journal = prior_journal;
            self.stability = prior_stability;
        }
        result
    }

    /// Re-evaluate a dispatched transaction against the current observation:
    /// prove or fail pending obligations, then dispatch deferred steps whose
    /// dependencies are now verified. Terminal actions never change. The
    /// snapshot is whatever the caller observed; this never simulates time.
    pub fn reconcile(
        &mut self,
        plan: &PreparedPlan,
        snapshot: &mut WorldSnapshot,
        context: &OperationContext,
    ) -> Result<CommitReceipt> {
        plan.validate_structure()?;
        let idempotency_key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        let prior = self
            .journal
            .lookup(&idempotency_key)
            .and_then(|record| record.receipt.clone())
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidPlan,
                    "plan has no dispatched transaction to reconcile",
                )
            })?;
        if prior.plan_id != plan.id
            || prior.plan_digest != plan.digest
            || prior.actions.len() != plan.steps.len()
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "journaled transaction does not match the supplied sealed plan",
            ));
        }
        if !snapshot.hash_is_valid() || context.anchor != snapshot.anchor() {
            return Err(DfmcpError::new(
                ErrorCode::StaleAnchor,
                "reconcile snapshot or operation context anchor is invalid",
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
        }

        let prior_snapshot = snapshot.clone();
        let prior_stability = self.stability.clone();
        let result = (|| {
            // Reuse one sealed observation while proving pending work. A newly
            // dispatched effect ends that borrow and obtains fresh evidence.
            let mut observation = PredicateEvidence::laboratory(snapshot)?;
            let mut receipts: Vec<ActionReceipt> = Vec::with_capacity(plan.steps.len());
            for (step, old) in plan.steps.iter().zip(&prior.actions) {
                let receipt = match old.state {
                    CommitState::AppliedAwaitingVerification => {
                        self.evaluate_obligation(plan, step, old.action_id, &observation)?
                    }
                    CommitState::Prepared => {
                        match deferred_step_decision_with_evidence(
                            step,
                            &observation,
                            |dependency| {
                                receipts
                                    .iter()
                                    .find(|receipt| receipt.step_id == dependency)
                                    .map(|receipt| receipt.state)
                            },
                        )? {
                            DeferredStepDecision::Ready => {
                                let receipt =
                                    self.dispatch_step(plan, step, old.action_id, snapshot)?;
                                observation = PredicateEvidence::laboratory(snapshot)?;
                                receipt
                            }
                            DeferredStepDecision::Waiting => old.clone(),
                            DeferredStepDecision::Failed(message) => action_receipt(
                                plan,
                                step,
                                old.action_id,
                                CommitState::Failed,
                                snapshot,
                                message,
                            ),
                        }
                    }
                    _ => old.clone(),
                };
                receipts.push(receipt);
            }
            let receipt = CommitReceipt {
                plan_id: plan.id,
                plan_digest: plan.digest,
                actions: receipts,
                checkpoint: None,
                observed_anchor: snapshot.anchor(),
                warnings: Vec::new(),
            };
            self.journal
                .record_reconciliation(&idempotency_key, receipt.clone())?;
            Ok(receipt)
        })();
        if result.is_err() {
            *snapshot = prior_snapshot;
            self.stability = prior_stability;
        }
        result
    }

    /// Apply one step's reference effect and classify its immediate outcome.
    fn dispatch_step(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        action_id: ActionId,
        snapshot: &mut WorldSnapshot,
    ) -> Result<ActionReceipt> {
        let observation = PredicateEvidence::laboratory(snapshot)?;
        if !predicates_established(&observation, &step.preconditions)? {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!(
                    "preconditions for step {} are no longer established true",
                    step.id.get()
                ),
            ));
        }
        if effects::apply_effect(snapshot, &step.action, &step.idempotency_key)? {
            let next_cursor = snapshot.cursor.checked_next().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::CursorGap,
                    "cannot publish a mutation because the observation cursor is exhausted",
                )
            })?;
            snapshot.cursor = next_cursor;
            snapshot.refresh_hash();
        }
        let observation = PredicateEvidence::laboratory(snapshot)?;
        if step.obligation.is_some() {
            return self.evaluate_obligation(plan, step, action_id, &observation);
        }
        if !predicates_established(&observation, &step.postconditions)? {
            return Err(DfmcpError::new(
                ErrorCode::AdapterRejected,
                format!(
                    "postconditions for step {} are not established true",
                    step.id.get()
                ),
            ));
        }
        Ok(action_receipt(
            plan,
            step,
            action_id,
            CommitState::Verified,
            snapshot,
            "semantic postconditions verified".to_owned(),
        ))
    }

    /// Prove, fail, or keep pending one dispatched temporal step.
    fn evaluate_obligation(
        &mut self,
        plan: &PreparedPlan,
        step: &PlanStep,
        action_id: ActionId,
        observation: &PredicateEvidence<'_>,
    ) -> Result<ActionReceipt> {
        let snapshot = observation.snapshot();
        let Some(obligation) = &step.obligation else {
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "pending step has no obligation to evaluate",
            ));
        };
        let failed = obligation
            .failure
            .as_ref()
            .map_or(Ok(false), |predicate| observation.establishes(predicate))?;
        let holds = observation.establishes(&obligation.terminal)?
            && predicates_established(observation, &step.postconditions)?;
        let (stable, last) = self.stability.get(&action_id).copied().unwrap_or((0, None));
        let anchor = snapshot.anchor();
        let stable = if failed || !holds {
            self.stability.insert(action_id, (0, None));
            0
        } else if last == Some(anchor) {
            stable
        } else {
            self.stability
                .insert(action_id, (stable.saturating_add(1), Some(anchor)));
            stable.saturating_add(1)
        };
        let (state, message) = if failed {
            (CommitState::Failed, "obligation failure predicate observed")
        } else if snapshot.tick > obligation.deadline_tick {
            (
                CommitState::Failed,
                "obligation deadline passed before stable completion was observed",
            )
        } else if holds && stable >= obligation.stable_for_observations {
            (
                CommitState::Verified,
                "obligation terminal proven on stable observations",
            )
        } else if snapshot.tick >= obligation.deadline_tick {
            (
                CommitState::Failed,
                "obligation reached its game-tick deadline without stable completion",
            )
        } else {
            (
                CommitState::AppliedAwaitingVerification,
                "effect dispatched; obligation proof pending later observation",
            )
        };
        if state.is_terminal() {
            self.stability.remove(&action_id);
        }
        Ok(action_receipt(
            plan,
            step,
            action_id,
            state,
            snapshot,
            message.to_owned(),
        ))
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

/// Require authoritative predicate evidence without treating unavailable facts as false.
fn predicates_established(
    observation: &PredicateEvidence<'_>,
    predicates: &[Predicate],
) -> Result<bool> {
    for predicate in predicates {
        if !observation.establishes(predicate)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn derived_action_id(plan_id: PlanId, step_id: dfmcp_core::StepId) -> ActionId {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-dispatch-action-v1");
    bytes.extend_from_slice(&plan_id.get().to_be_bytes());
    bytes.extend_from_slice(&step_id.get().to_be_bytes());
    let derived = Digest32::of_bytes(&bytes).first_u128();
    ActionId::new(if derived == 0 { 1 } else { derived })
}

fn dependencies_verified(step: &PlanStep, earlier: &[ActionReceipt]) -> bool {
    step.depends_on.iter().all(|dependency| {
        earlier
            .iter()
            .any(|receipt| receipt.step_id == *dependency && receipt.state == CommitState::Verified)
    })
}

fn action_receipt(
    plan: &PreparedPlan,
    step: &PlanStep,
    action_id: ActionId,
    state: CommitState,
    snapshot: &WorldSnapshot,
    message: String,
) -> ActionReceipt {
    let mut receipt_bytes = Vec::new();
    receipt_bytes.extend_from_slice(b"dfmcp-dispatch-action-receipt-v2");
    receipt_bytes.extend_from_slice(plan.digest.as_bytes());
    receipt_bytes.extend_from_slice(&action_id.get().to_be_bytes());
    receipt_bytes.extend_from_slice(&step.id.get().to_be_bytes());
    receipt_bytes.extend_from_slice(format!("{state:?}").as_bytes());
    receipt_bytes.extend_from_slice(snapshot.state_hash.as_bytes());
    let receipt_digest = Digest32::of_bytes(&receipt_bytes);
    ActionReceipt {
        action_id,
        step_id: step.id,
        state,
        observed_anchor: snapshot.anchor(),
        adapter_receipt_digest: receipt_digest,
        evidence: vec![Evidence {
            id: EvidenceId::new(action_id.get()),
            kind: EvidenceKind::Postcondition,
            subject: None,
            anchor: snapshot.anchor(),
            digest: snapshot.state_hash,
            summary: "in-memory postcondition evaluated against canonical snapshot".to_owned(),
        }],
        message,
    }
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
        .any(|step| matches!(step.action, dfmcp_intent::Action::Extension { .. }))
    {
        return Err(DfmcpError::new(
            ErrorCode::AdapterRejected,
            "in-memory dispatcher has no reference semantics for extension actions",
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
    use dfmcp_intent::{Action, Constraint, Intent, RequestedAction, StaticPlanner};
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

    fn unpause_plan(snapshot: &WorldSnapshot) -> Result<PreparedPlan> {
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
        StaticPlanner::default().prepare_laboratory(snapshot, &intent, &sample_context(snapshot))
    }


    fn authority_fact(value: bool, tick: GameTick) -> dfmcp_world::Fact {
        dfmcp_world::Fact::known(
            dfmcp_world::Value::Bool(value),
            tick,
            dfmcp_world::FactSource::Derived("dfmcp.lab-scenario/1".to_owned()),
            dfmcp_core::Digest32::ZERO,
        )
    }

    fn ineligible_facts(tick: GameTick) -> Vec<dfmcp_world::Fact> {
        use dfmcp_core::Digest32;
        use dfmcp_world::{Fact, FactSource, Value};
        let mut facts: Vec<_> = [
            FactSource::AgentAssertion("agent-memory".to_owned()),
            FactSource::Replay,
            FactSource::Derived("unregistered-model/1".to_owned()),
            FactSource::DfhackField("unit.ready".to_owned()),
        ]
        .into_iter()
        .map(|source| Fact::known(Value::Bool(true), tick, source, Digest32::ZERO))
        .collect();
        let mut wrong_digest = authority_fact(true, tick);
        wrong_digest.source_digest = Digest32::of_bytes(b"not-the-laboratory-source-seal");
        facts.push(wrong_digest);
        facts.push(authority_fact(true, GameTick(tick.0 + 1_000)));
        facts
    }

    fn authority_predicate(field: &str) -> Predicate {
        Predicate::FieldCompare {
            entity_id: dfmcp_core::EntityId::new(9),
            field: field.to_owned(),
            op: dfmcp_world::CompareOp::Eq,
            value: dfmcp_world::Value::Bool(true),
        }
    }

    fn add_authority_fields(snapshot: &mut WorldSnapshot, fact: dfmcp_world::Fact) {
        use dfmcp_core::EntityId;
        use dfmcp_world::{EntityKind, EntityRecord};
        snapshot.graph.entities.insert(
            EntityId::new(9),
            EntityRecord {
                id: EntityId::new(9),
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "evidence subject".to_owned(),
                fields: std::collections::BTreeMap::from([
                    ("ready".to_owned(), fact.clone()),
                    ("done".to_owned(), fact.clone()),
                    ("failed".to_owned(), fact),
                ]),
            },
        );
        snapshot.refresh_hash();
    }

    fn set_authority_field(
        snapshot: &mut WorldSnapshot,
        field: &str,
        fact: dfmcp_world::Fact,
    ) -> dfmcp_core::Result<()> {
        let entity = snapshot
            .graph
            .entities
            .get_mut(&dfmcp_core::EntityId::new(9))
            .ok_or_else(|| {
                DfmcpError::new(ErrorCode::InternalInvariantViolation, "missing evidence subject")
            })?;
        entity.fields.insert(field.to_owned(), fact);
        entity.revision += 1;
        snapshot.refresh_hash();
        Ok(())
    }

    fn reseal_authority_plan(plan: &mut dfmcp_intent::PreparedPlan) -> dfmcp_core::Result<()> {
        plan.digest = plan.compute_digest();
        plan.id = plan.expected_id();
        plan.validate_structure()
    }

    #[test]
    fn preparation_rejects_ineligible_fact_sources_without_journaling() -> Result<()> {
        for fact in ineligible_facts(GameTick(100)) {
            let mut snapshot = sample_snapshot();
            add_authority_fields(&mut snapshot, fact);
            assert!(dfmcp_world::evaluate(&snapshot, &authority_predicate("ready")));
            let mut plan = unpause_plan(&snapshot)?;
            plan.steps[0].preconditions = vec![authority_predicate("ready")];
            reseal_authority_plan(&mut plan)?;
            let mut dispatcher = MutationDispatcher::new();
            let result = dispatcher.prepare_mutation(&plan, &snapshot, &sample_context(&snapshot));
            assert!(matches!(result, Err(error) if error.code == ErrorCode::PreconditionsFailed));
            assert!(dispatcher.journal().is_empty());
        }
        let mut snapshot = sample_snapshot();
        add_authority_fields(&mut snapshot, authority_fact(true, GameTick(100)));
        let mut plan = unpause_plan(&snapshot)?;
        plan.steps[0].preconditions = vec![authority_predicate("ready")];
        reseal_authority_plan(&mut plan)?;
        let context = sample_context(&snapshot);
        let mut dispatcher = MutationDispatcher::new();
        let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
        let receipt = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context)?;
        assert_eq!(receipt.actions[0].state, CommitState::Verified);
        Ok(())
    }

    #[test]
    fn immediate_postconditions_cannot_be_proved_by_ineligible_facts() -> Result<()> {
        for fact in ineligible_facts(GameTick(100)) {
            let mut snapshot = sample_snapshot();
            add_authority_fields(&mut snapshot, fact);
            let mut plan = unpause_plan(&snapshot)?;
            plan.steps[0].postconditions = vec![authority_predicate("done")];
            reseal_authority_plan(&mut plan)?;
            let context = sample_context(&snapshot);
            let mut dispatcher = MutationDispatcher::new();
            let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
            let prior = snapshot.clone();
            let result = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context);
            assert!(matches!(result, Err(error) if error.code == ErrorCode::AdapterRejected));
            assert_eq!(snapshot, prior);
            let key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
            assert_eq!(dispatcher.journal().lookup(&key).map(|entry| entry.state), Some(CommitState::Prepared));
        }
        Ok(())
    }

    #[test]
    fn reconciliation_uses_authority_for_both_terminal_and_failure_predicates() -> Result<()> {
        use dfmcp_intent::ObligationSpec;
        for fact in ineligible_facts(GameTick(100)) {
            let mut snapshot = sample_snapshot();
            add_authority_fields(&mut snapshot, fact);
            let mut plan = unpause_plan(&snapshot)?;
            plan.steps[0].obligation = Some(ObligationSpec {
                terminal: authority_predicate("done"),
                failure: Some(authority_predicate("failed")),
                deadline_tick: GameTick(110),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            });
            reseal_authority_plan(&mut plan)?;
            let context = sample_context(&snapshot);
            let mut dispatcher = MutationDispatcher::new();
            let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
            let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context)?;
            assert_eq!(committed.actions[0].state, CommitState::AppliedAwaitingVerification);
            let context = sample_context(&snapshot);
            let pending = dispatcher.reconcile(&plan, &mut snapshot, &context)?;
            assert_eq!(pending.actions[0].state, CommitState::AppliedAwaitingVerification);

            let mut failed_dispatcher = dispatcher.clone();
            let mut failed_snapshot = snapshot.clone();
            failed_snapshot.tick = GameTick(101);
            failed_snapshot.cursor.sequence += 1;
            set_authority_field(&mut failed_snapshot, "failed", authority_fact(true, GameTick(101)))?;
            let context = sample_context(&failed_snapshot);
            let failed = failed_dispatcher.reconcile(&plan, &mut failed_snapshot, &context)?;
            assert_eq!(failed.actions[0].state, CommitState::Failed);

            snapshot.tick = GameTick(101);
            snapshot.cursor.sequence += 1;
            set_authority_field(&mut snapshot, "done", authority_fact(true, GameTick(101)))?;
            let context = sample_context(&snapshot);
            let verified = dispatcher.reconcile(&plan, &mut snapshot, &context)?;
            assert_eq!(verified.actions[0].state, CommitState::Verified);
        }
        Ok(())
    }

    #[test]
    fn deferred_reconciliation_refuses_agent_supplied_dispatch_evidence() -> Result<()> {
        use dfmcp_intent::ObligationSpec;
        use dfmcp_world::{Fact, FactSource, Value};
        let mut snapshot = sample_snapshot();
        add_authority_fields(&mut snapshot, authority_fact(true, GameTick(100)));
        set_authority_field(&mut snapshot, "done", authority_fact(false, GameTick(100)))?;
        let intent = Intent {
            id: IntentId::new(41),
            anchor: snapshot.anchor(),
            summary: "observe completion before pausing again".to_owned(),
            terminal_condition: Predicate::All(vec![
                authority_predicate("done"),
                Predicate::Paused(true),
            ]).normalized(),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![
                RequestedAction {
                    action: Action::Pause { paused: false },
                    preconditions: vec![Predicate::Paused(true)],
                    postconditions: vec![Predicate::Paused(false)],
                    compensation: None,
                    obligation: Some(ObligationSpec {
                        terminal: authority_predicate("done"),
                        failure: None,
                        deadline_tick: GameTick(110),
                        poll_interval_ticks: 1,
                        stable_for_observations: 1,
                    }),
                    depends_on: Vec::new(),
                },
                RequestedAction {
                    action: Action::Pause { paused: true },
                    preconditions: vec![authority_predicate("ready")],
                    postconditions: vec![Predicate::Paused(true)],
                    compensation: None,
                    obligation: None,
                    depends_on: vec![0],
                },
            ],
        };
        let context = sample_context(&snapshot);
        let plan = StaticPlanner::default().prepare_laboratory(&snapshot, &intent, &context)?;
        let mut dispatcher = MutationDispatcher::new();
        let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
        let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context)?;
        assert_eq!(committed.actions[1].state, CommitState::Prepared);
        snapshot.tick = GameTick(101);
        snapshot.cursor.sequence += 1;
        set_authority_field(&mut snapshot, "done", authority_fact(true, GameTick(101)))?;
        set_authority_field(&mut snapshot, "ready", Fact::known(
            Value::Bool(true), GameTick(101), FactSource::AgentAssertion("still ready".to_owned()), Digest32::ZERO
        ))?;
        let context = sample_context(&snapshot);
        let reconciled = dispatcher.reconcile(&plan, &mut snapshot, &context)?;
        assert_eq!(reconciled.actions[0].state, CommitState::Verified);
        assert_eq!(reconciled.actions[1].state, CommitState::Failed);
        assert!(reconciled.actions[1].message.contains("not dispatched"));
        assert!(!snapshot.paused);
        Ok(())
    }

    #[test]
    fn test_two_phase_prepare_and_commit() -> Result<()> {
        let mut snapshot = sample_snapshot();
        let context = sample_context(&snapshot);
        let plan = unpause_plan(&snapshot)?;
        let mut dispatcher = MutationDispatcher::new();

        let prepare_receipt = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
        assert_eq!(prepare_receipt.plan_id, plan.id);
        assert_eq!(prepare_receipt.plan_digest, plan.digest);

        let commit_receipt =
            dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &context)?;
        assert_eq!(commit_receipt.plan_id, plan.id);
        assert_eq!(commit_receipt.actions.len(), 1);
        assert_eq!(commit_receipt.actions[0].state, CommitState::Verified);
        assert!(!snapshot.paused);
        assert_eq!(snapshot.cursor.sequence, 1);
        let key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        assert_eq!(
            dispatcher.journal().lookup(&key).map(|record| record.state),
            Some(CommitState::Verified)
        );

        let replay_receipt =
            dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &context)?;
        assert_eq!(replay_receipt, commit_receipt);

        let mut denied_context = sample_context(&snapshot);
        denied_context.grants.clear();
        let denied =
            dispatcher.commit_mutation(&plan, &prepare_receipt, &mut snapshot, &denied_context);
        assert!(matches!(
            denied,
            Err(ref error) if error.code == ErrorCode::CapabilityDenied
        ));
        Ok(())
    }

    #[test]
    fn safe_commit_failure_restores_snapshot_and_prepared_journal_state() -> Result<()> {
        let mut snapshot = sample_snapshot();
        snapshot.cursor.sequence = u64::MAX;
        snapshot.refresh_hash();
        let context = sample_context(&snapshot);
        let plan = unpause_plan(&snapshot)?;
        let mut dispatcher = MutationDispatcher::new();
        let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
        let prior = snapshot.clone();
        let failure = dispatcher
            .commit_mutation(&plan, &prepared, &mut snapshot, &context)
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "cursor-exhausted commit unexpectedly succeeded",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::CursorGap);
        assert_eq!(snapshot, prior);
        let key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        assert_eq!(
            dispatcher.journal().lookup(&key).map(|record| record.state),
            Some(CommitState::Prepared)
        );
        Ok(())
    }

    #[test]
    fn journal_rejects_expired_plan_and_unbounded_key() -> Result<()> {
        let snapshot = sample_snapshot();
        let plan = unpause_plan(&snapshot)?;
        let mut journal = EffectJournal::new();
        let expired = journal.record_prepare("bounded".to_owned(), &plan, plan.expires_at_tick);
        assert!(matches!(expired, Err(ref error) if error.code == ErrorCode::InvalidPlan));
        let oversized = journal.record_prepare(
            "x".repeat(MAX_IDEMPOTENCY_KEY_BYTES.saturating_add(1)),
            &plan,
            snapshot.tick,
        );
        assert!(matches!(oversized, Err(ref error) if error.code == ErrorCode::InvalidRequest));
        Ok(())
    }

    #[test]
    fn test_stale_anchor_prepare_rejection() -> Result<()> {
        let snapshot = sample_snapshot();
        let context = sample_context(&snapshot);
        let mut mutated_anchor_snapshot = snapshot.clone();
        mutated_anchor_snapshot.tick = GameTick(200);
        let plan = unpause_plan(&snapshot)?;
        let mut dispatcher = MutationDispatcher::new();
        let result = dispatcher.prepare_mutation(&plan, &mutated_anchor_snapshot, &context);
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn dependent_steps_wait_for_observed_proof_then_dispatch_on_reconcile() -> Result<()> {
        use dfmcp_core::{EntityId, MapCoord, MapCuboid};
        use dfmcp_intent::{DigMode, PlanPolicy};
        use dfmcp_world::terrain::{region_tiles, uniform_chunk};
        use dfmcp_world::{ChunkCoord, EntityKind, EntityRecord, tile_codes};

        let miner = EntityId::new(5);
        let mut graph = WorldGraph::default();
        let rock = ChunkCoord { x: 0, y: 0, z: 3 };
        graph
            .chunks
            .insert(rock, uniform_chunk(rock, tile_codes::SOLID_WALL));
        graph.entities.insert(
            miner,
            EntityRecord {
                id: miner,
                generation: 1,
                revision: 1,
                kind: EntityKind::Unit,
                label: "miner".to_owned(),
                fields: BTreeMap::new(),
            },
        );
        let mut snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(100),
            ObservationCursor::ORIGIN,
            false,
            graph,
        );
        let mut context = sample_context(&snapshot);
        context.grants.push(CapabilityGrant {
            capability: Capability::ConfigureLabor,
            scope: CapabilityScope::default(),
            max_risk: RiskTier::Reversible,
            expires_at_tick: None,
            remaining_uses: None,
        });
        let area = MapCuboid::new(MapCoord::new(0, 0, 3), MapCoord::new(1, 0, 3))?;
        let request = |action, depends_on| RequestedAction {
            action,
            preconditions: Vec::new(),
            postconditions: Vec::new(),
            compensation: None,
            obligation: None,
            depends_on,
        };
        let intent = Intent {
            id: IntentId::new(3),
            anchor: snapshot.anchor(),
            summary: "dig, then stop mining".to_owned(),
            terminal_condition: Predicate::Paused(true),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![
                request(
                    Action::DesignateDig {
                        area,
                        mode: DigMode::Mine,
                    },
                    Vec::new(),
                ),
                request(
                    Action::SetLabor {
                        units: vec![miner],
                        labor: "MINE".to_owned(),
                        enabled: false,
                    },
                    vec![0],
                ),
            ],
        };
        let plan = StaticPlanner::new(PlanPolicy {
            require_checkpoint_at_or_above: RiskTier::Irreversible,
            ..PlanPolicy::default()
        })
        .prepare_laboratory(&snapshot, &intent, &context)?;
        let mut dispatcher = MutationDispatcher::new();
        let prepared = dispatcher.prepare_mutation(&plan, &snapshot, &context)?;
        let committed = dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context)?;
        assert_eq!(
            committed
                .actions
                .iter()
                .map(|a| a.state)
                .collect::<Vec<_>>(),
            vec![
                CommitState::AppliedAwaitingVerification,
                CommitState::Prepared
            ]
        );
        let labor_field = format!("{}MINE", effects::LABOR_FIELD_PREFIX);
        assert!(
            !snapshot.graph.entities[&miner]
                .fields
                .contains_key(&labor_field)
        );

        // Reconciling the same observation proves nothing new.
        let context = sample_context_with_labor(&snapshot, &context);
        let unchanged = dispatcher.reconcile(&plan, &mut snapshot, &context)?;
        assert_eq!(unchanged.actions[1].state, CommitState::Prepared);

        // A later observation shows the excavation done: reconcile proves it
        // and only then dispatches the dependent labor change.
        for coord in region_tiles(area) {
            snapshot.set_tile_code(coord, tile_codes::FLOOR)?;
        }
        snapshot.tick = GameTick(130);
        snapshot.cursor = snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| DfmcpError::new(ErrorCode::CursorGap, "cursor"))?;
        snapshot.refresh_hash();
        let context = sample_context_with_labor(&snapshot, &context);
        let reconciled = dispatcher.reconcile(&plan, &mut snapshot, &context)?;
        assert_eq!(
            reconciled
                .actions
                .iter()
                .map(|a| a.state)
                .collect::<Vec<_>>(),
            vec![CommitState::Verified, CommitState::Verified]
        );
        assert_eq!(
            snapshot.graph.entities[&miner].fields[&labor_field].value,
            dfmcp_world::Value::Bool(false)
        );
        let key = format!("dfmcp_tx_{}_{}", context.session_id.get(), plan.digest);
        assert_eq!(
            dispatcher.journal().lookup(&key).map(|record| record.state),
            Some(CommitState::Verified)
        );
        // Replaying the original commit returns the latest journaled receipt.
        assert_eq!(
            dispatcher.commit_mutation(&plan, &prepared, &mut snapshot, &context)?,
            reconciled
        );
        Ok(())
    }

    fn sample_context_with_labor(
        snapshot: &WorldSnapshot,
        prior: &OperationContext,
    ) -> OperationContext {
        OperationContext {
            anchor: snapshot.anchor(),
            ..prior.clone()
        }
    }
}
