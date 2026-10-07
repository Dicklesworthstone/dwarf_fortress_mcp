#![forbid(unsafe_code)]

use std::collections::{BTreeMap, VecDeque};

use dfmcp_adapter::{
    ActionReceipt, AdapterHealth, AdapterIdentity, CancelMode, CancelReceipt, CheckpointReceipt,
    CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus, ObservationFrame,
    ObservationPayload, ObservationRequest, PrepareReceipt, QueryRequest, QueryResponse, QueryRow,
    RestoreReceipt,
};
use dfmcp_core::{
    ActionId, Capability, CheckpointId, CommitState, DfmcpError, Digest32, ErrorCode, Evidence,
    EvidenceId, EvidenceKind, FortressId, GameTick, ObservationCursor, OperationContext, PlanId,
    Result, RiskTier, StateAnchor, StepId,
};
use dfmcp_intent::execution::{DeferredStepDecision, deferred_step_decision_with_evidence};
use dfmcp_intent::{
    Action, EffectWorkState, ObligationRuntime, ObligationStatus, PlanStep, PreparedPlan, effects,
    inspect_effect_work,
};
pub mod durable;
pub mod faults;

pub use faults::{Boundary, CampaignReport, Fault, FaultPoint, FaultSchedule};

#[cfg(test)]
mod effect_drain_tests;

use dfmcp_world::{Predicate, PredicateEvidence, WorldGraph, WorldSnapshot, execute_bounded_query};

const MAX_LAB_PREPARED_PLANS: usize = 4_096;
const MAX_LAB_ACTIONS: usize = 16_384;
const MAX_LAB_CHECKPOINTS: usize = 1_024;
const MAX_LAB_COMMITS: usize = 4_096;
const MAX_LAB_TRANSCRIPT_EVENTS: usize = 65_536;
const CONSERVATIVE_BYTES_PER_OUTPUT_TOKEN: u64 = 4;

/// Managed simulated laboratory session hosting a `MemoryAdapter`.
#[derive(Clone, Debug)]
pub struct LabSession {
    adapter: MemoryAdapter,
}

impl LabSession {
    #[must_use]
    pub fn new(fortress_id: u64, paused: bool) -> Self {
        let snapshot = WorldSnapshot::new(
            FortressId::new(fortress_id),
            GameTick(0),
            ObservationCursor::ORIGIN,
            paused,
            WorldGraph::default(),
        );
        Self {
            adapter: MemoryAdapter::new(snapshot),
        }
    }

    #[must_use]
    pub const fn adapter(&self) -> &MemoryAdapter {
        &self.adapter
    }

    pub fn adapter_mut(&mut self) -> &mut MemoryAdapter {
        &mut self.adapter
    }

    #[must_use]
    pub fn current_snapshot(&self) -> &WorldSnapshot {
        self.adapter.snapshot()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabEvent {
    Observed(StateAnchor),
    Prepared(PlanId),
    Committed(PlanId),
    ActionPolled(ActionId, CommitState),
    CancelRequested(ActionId, CancelMode),
    CancelFinalized(ActionId, CommitState),
    /// Independently authorized cleanup of work whose goal proof is terminal.
    EffectDrained(ActionId, StateAnchor),
    Checkpointed(CheckpointId),
    Restored(CheckpointId),
    SnapshotInjected(StateAnchor),
    TickAdvanced(GameTick),
    /// The world was recovered from durable storage into a new epoch.
    Recovered(StateAnchor),
}

/// Evidence that an action's physical reference work was inspected and drained.
/// This does not replace or reinterpret its immutable semantic proof receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectDrainReceipt {
    pub action_id: ActionId,
    pub before: EffectWorkState,
    pub after: EffectWorkState,
    pub observed_anchor: StateAnchor,
    pub stopped_work: bool,
    pub evidence: Vec<Evidence>,
}

#[derive(Clone, Debug)]
struct LabAction {
    plan_id: PlanId,
    step: PlanStep,
    receipt: ActionReceipt,
    obligation_runtime: Option<ObligationRuntime>,
    cancel_mode: Option<CancelMode>,
    /// Whether the step's effect was ever applied. A step still waiting on
    /// its dependencies has nothing to compensate.
    dispatched: bool,
}

#[derive(Clone, Debug)]
pub struct MemoryAdapter {
    identity: AdapterIdentity,
    snapshot: WorldSnapshot,
    prepared: BTreeMap<PlanId, PrepareReceipt>,
    plans: BTreeMap<PlanId, PreparedPlan>,
    actions: BTreeMap<ActionId, LabAction>,
    action_by_step: BTreeMap<(PlanId, StepId), ActionId>,
    checkpoints: BTreeMap<CheckpointId, WorldSnapshot>,
    commits: BTreeMap<PlanId, CommitReceipt>,
    transcript: VecDeque<LabEvent>,
    transcript_truncated: bool,
    nonce: u128,
}

impl MemoryAdapter {
    #[must_use]
    pub fn new(snapshot: WorldSnapshot) -> Self {
        let capabilities = [
            Capability::Observe,
            Capability::Query,
            Capability::Plan,
            Capability::Designate,
            Capability::Construct,
            Capability::ConfigureLabor,
            Capability::ConfigureProduction,
            Capability::ConfigureLogistics,
            Capability::ConfigureMilitary,
            Capability::ControlClock,
            Capability::Checkpoint,
            Capability::Restore,
            Capability::Doctor,
        ]
        .into_iter()
        .collect();
        Self {
            identity: AdapterIdentity {
                name: "dfmcp-memory-lab".to_owned(),
                adapter_version: env!("CARGO_PKG_VERSION").to_owned(),
                bridge_protocol_version: "dfmcp-bridge-v1-lab".to_owned(),
                dwarf_fortress_version: "simulated".to_owned(),
                dfhack_version: "simulated".to_owned(),
                compatibility: CompatibilityLevel::Exact,
                capabilities,
                schema_digest: Digest32::of_bytes(b"dfmcp-memory-lab-schema-v1"),
            },
            snapshot,
            prepared: BTreeMap::new(),
            plans: BTreeMap::new(),
            actions: BTreeMap::new(),
            action_by_step: BTreeMap::new(),
            checkpoints: BTreeMap::new(),
            commits: BTreeMap::new(),
            transcript: VecDeque::new(),
            transcript_truncated: false,
            nonce: 1,
        }
    }

    #[must_use]
    pub const fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }

    /// An adapter over a world recovered from durable storage. The world
    /// enters a new observation epoch, so no cursor, plan or action handle
    /// from before the recovery can be mistaken for current.
    pub fn recovered(mut snapshot: WorldSnapshot) -> Result<Self> {
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "recovered snapshot failed its content-hash seal",
            ));
        }
        snapshot.cursor = snapshot.cursor.checked_reset_epoch().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CursorGap,
                "cannot recover because the observation epoch is exhausted",
            )
        })?;
        snapshot.refresh_hash();
        let mut adapter = Self::new(snapshot);
        adapter.record_event(LabEvent::Recovered(adapter.snapshot.anchor()));
        Ok(adapter)
    }

    /// The world state a checkpoint captured.
    #[must_use]
    pub fn checkpoint_snapshot(&self, checkpoint_id: CheckpointId) -> Option<&WorldSnapshot> {
        self.checkpoints.get(&checkpoint_id)
    }

    /// State hashes of every checkpoint this adapter can restore.
    pub fn checkpoint_state_hashes(&self) -> impl Iterator<Item = Digest32> + '_ {
        self.checkpoints
            .values()
            .map(|snapshot| snapshot.state_hash)
    }

    /// Make a durable checkpoint restorable through the ordinary restore path.
    pub fn adopt_checkpoint(
        &mut self,
        checkpoint_id: CheckpointId,
        snapshot: WorldSnapshot,
    ) -> Result<()> {
        if checkpoint_id == CheckpointId::NIL
            || snapshot.fortress_id != self.snapshot.fortress_id
            || !snapshot.hash_is_valid()
        {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "adopted checkpoint is unsealed or belongs to another fortress",
            ));
        }
        match self.checkpoints.get(&checkpoint_id) {
            Some(existing) if *existing == snapshot => Ok(()),
            Some(_) => Err(DfmcpError::new(
                ErrorCode::Conflict,
                "checkpoint identifier already names different content",
            )),
            None if self.checkpoints.len() >= MAX_LAB_CHECKPOINTS => Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory checkpoint store reached its explicit bound",
            )),
            None => {
                self.checkpoints.insert(checkpoint_id, snapshot);
                Ok(())
            }
        }
    }

    /// The last receipt recorded for an action, without polling (and so
    /// without dispatching deferred work or changing state).
    #[must_use]
    pub fn action_receipt(&self, action_id: ActionId) -> Option<&ActionReceipt> {
        self.actions.get(&action_id).map(|action| &action.receipt)
    }

    /// The immutable original commit that owns this action, without polling
    /// or reconstructing a plan from its current proof state.
    #[must_use]
    pub fn action_plan_receipt(&self, action_id: ActionId) -> Option<&CommitReceipt> {
        let action = self.actions.get(&action_id)?;
        self.commits.get(&action.plan_id)
    }

    /// The last receipt of a committed plan's step, if it was committed.
    #[must_use]
    pub fn step_receipt(&self, plan_id: PlanId, step_id: StepId) -> Option<&ActionReceipt> {
        self.action_by_step
            .get(&(plan_id, step_id))
            .and_then(|action_id| self.action_receipt(*action_id))
    }

    /// The sealed plan step an action executes.
    #[must_use]
    pub fn action_step(&self, action_id: ActionId) -> Option<&PlanStep> {
        self.actions.get(&action_id).map(|action| &action.step)
    }

    /// Inspect physical work without polling or dispatching a prepared action.
    /// A terminal goal proof does not, by itself, prove physical quiescence.
    pub fn action_work_state(&self, action_id: ActionId) -> Result<EffectWorkState> {
        let action = self.actions.get(&action_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown action {action_id}"),
            )
        })?;
        inspect_effect_work(
            &self.snapshot,
            &action.step.action,
            &action.step.idempotency_key,
            action.dispatched,
        )
    }

    /// Exact physical identities still covered by retained dispatch records,
    /// including records whose goal proof is already terminal. This iterator
    /// only reads bookkeeping and never polls or dispatches a prepared step.
    pub fn known_work_entity_ids(&self) -> impl Iterator<Item = dfmcp_core::EntityId> + '_ {
        self.actions.values().filter_map(|action| {
            (action.dispatched
                && matches!(
                    &action.step.action,
                    Action::DesignateDig { .. }
                        | Action::Build { .. }
                        | Action::CreateWorkOrder { .. }
                ))
            .then(|| effects::created_entity_id(&action.step.idempotency_key, 0))
        })
    }

    /// Stop remaining physical work of an already terminal action under a fresh
    /// scoped grant. Its Failed/Verified/etc. proof receipt remains byte-for-byte
    /// unchanged. Nonterminal work must use request_cancel/finalize_cancel.
    /// Unknown ownership or lifecycle evidence refuses before any mutation.
    pub fn drain_action_work(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<EffectDrainReceipt> {
        self.drain_action_work_in_mode(action_id, CancelMode::StopFutureSteps, context)
    }

    /// Terminal cleanup never compensates history. Emergency mode additionally
    /// pauses under current clock authority within the same atomic transaction.
    pub fn drain_action_work_in_mode(
        &mut self,
        action_id: ActionId,
        mode: CancelMode,
        context: &OperationContext,
    ) -> Result<EffectDrainReceipt> {
        self.check_anchor(context.anchor)?;
        let action = self.actions.get(&action_id).cloned().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown action {action_id}"),
            )
        })?;
        self.authorize_step(&action.step, context)?;
        if !action.receipt.state.is_terminal() {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "nonterminal work requires cancellation request and finalize",
            ));
        }
        let prior = self.clone();
        let result = (|| {
            let mut drain_context = context.clone();
            let emergency_pause =
                mode == CancelMode::EmergencyPauseAndDrain && !self.snapshot.paused;
            if mode == CancelMode::EmergencyPauseAndDrain {
                context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
            }
            if emergency_pause {
                drain_context.budget.max_actions = drain_context
                    .budget
                    .max_actions
                    .checked_sub(1)
                    .ok_or_else(|| {
                        DfmcpError::new(
                            ErrorCode::BudgetExceeded,
                            "emergency pause requires an available action budget",
                        )
                    })?;
                apply_action(&mut self.snapshot, &Action::Pause { paused: true }, "")?;
            }
            let (before, after, stopped_work) = drain_step_work(
                &mut self.snapshot,
                &action.step,
                action.dispatched,
                &drain_context,
            )?;
            let observed_anchor = self.snapshot.anchor();
            if stopped_work || emergency_pause {
                self.record_event(LabEvent::EffectDrained(action_id, observed_anchor));
            }
            let mut drain_evidence = vec![evidence(
                observed_anchor,
                EvidenceKind::Postcondition,
                &format!(
                    "physical work for action {action_id} is quiescent; terminal proof retained"
                ),
            )];
            if emergency_pause {
                drain_evidence.push(evidence(
                    observed_anchor,
                    EvidenceKind::Postcondition,
                    "emergency cleanup paused the fortress under current clock authority",
                ));
            }
            Ok(EffectDrainReceipt {
                action_id,
                before,
                after,
                observed_anchor,
                stopped_work,
                evidence: drain_evidence,
            })
        })();
        if result.is_err() {
            *self = prior;
        }
        result
    }

    #[must_use]
    pub fn transcript(&self) -> &VecDeque<LabEvent> {
        &self.transcript
    }

    #[must_use]
    pub const fn transcript_truncated(&self) -> bool {
        self.transcript_truncated
    }

    fn record_event(&mut self, event: LabEvent) {
        if self.transcript.len() >= MAX_LAB_TRANSCRIPT_EVENTS {
            self.transcript.pop_front();
            self.transcript_truncated = true;
        }
        self.transcript.push_back(event);
    }

    fn invalidate_active_work(&mut self) {
        self.prepared.clear();
        self.plans.clear();
        self.actions.clear();
        self.action_by_step.clear();
        self.commits.clear();
    }

    pub fn inject_snapshot(&mut self, snapshot: WorldSnapshot) -> Result<()> {
        if snapshot.fortress_id != self.snapshot.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "injected snapshot belongs to a different fortress",
            ));
        }
        if !snapshot.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "injected snapshot has an invalid state hash",
            ));
        }
        if snapshot == self.snapshot {
            return Ok(());
        }
        let current = self.snapshot.anchor();
        let incoming = snapshot.anchor();
        let cursor_regressed = incoming.cursor.epoch < current.cursor.epoch
            || (incoming.cursor.epoch == current.cursor.epoch
                && incoming.cursor.sequence <= current.cursor.sequence);
        let tick_regressed_without_epoch =
            incoming.cursor.epoch == current.cursor.epoch && incoming.tick < current.tick;
        if cursor_regressed || tick_regressed_without_epoch {
            return Err(DfmcpError::new(
                ErrorCode::CursorGap,
                "injected snapshot does not advance the canonical laboratory lineage",
            )
            .retryable(false));
        }
        self.snapshot = snapshot;
        self.invalidate_active_work();
        self.record_event(LabEvent::SnapshotInjected(self.snapshot.anchor()));
        Ok(())
    }

    pub fn advance_ticks(&mut self, amount: u64) -> Result<()> {
        if amount == 0 {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "laboratory tick advancement must be positive",
            ));
        }
        let next_tick = self.snapshot.tick.checked_add(amount).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory game tick exceeds the representable horizon",
            )
        })?;
        let next_cursor = self.snapshot.cursor.checked_next().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CursorGap,
                "laboratory observation cursor is exhausted",
            )
        })?;
        // Check the source before moving the clock. A pre-existing future
        // timestamp cannot become observation authority just because time passed.
        validate_tick_advance_source(&self.snapshot)?;
        // Work transitions on a shadow so a failed effect leaves no partial state.
        let mut next = self.snapshot.clone();
        next.tick = next_tick;
        next.cursor = next_cursor;
        // A paused fortress does no work; forced laboratory time while paused
        // advances the clock only.
        if !next.paused {
            effects::advance_effects(&mut next, amount)?;
        }
        next.refresh_hash();
        self.snapshot = next;
        self.record_event(LabEvent::TickAdvanced(self.snapshot.tick));
        Ok(())
    }

    fn next_nonce(&mut self) -> Result<u128> {
        let value = self.nonce;
        self.nonce = self.nonce.checked_add(1).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory nonce space is exhausted",
            )
        })?;
        Ok(value)
    }

    fn check_anchor(&self, anchor: StateAnchor) -> Result<()> {
        if anchor != self.snapshot.anchor() {
            return Err(
                DfmcpError::new(ErrorCode::StaleAnchor, "laboratory anchor is stale")
                    .retryable(true),
            );
        }
        Ok(())
    }

    fn authorize_step(&self, step: &PlanStep, context: &OperationContext) -> Result<()> {
        if !self
            .identity
            .capabilities
            .contains(&step.required_capability)
            || !action_is_supported(&step.action)
        {
            return Err(DfmcpError::new(
                ErrorCode::AdapterRejected,
                format!(
                    "laboratory adapter does not implement action capability {}",
                    step.required_capability.as_str()
                ),
            ));
        }
        let scope = step.action.scope();
        context.authorize(
            step.required_capability,
            step.risk,
            &scope.entity_ids,
            scope.map_area,
        )
    }

    fn stored_action_receipt(
        &mut self,
        action_id: ActionId,
        state: CommitState,
        kind: EvidenceKind,
        message: &str,
    ) -> Result<ActionReceipt> {
        let step_id = self
            .actions
            .get(&action_id)
            .map(|action| action.step.id)
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown action {action_id}"),
                )
            })?;
        let receipt = build_action_receipt(
            action_id,
            step_id,
            state,
            self.snapshot.anchor(),
            kind,
            message,
        );
        if let Some(action) = self.actions.get_mut(&action_id) {
            action.receipt = receipt.clone();
        }
        Ok(receipt)
    }

    fn internal_checkpoint(&mut self, label: &str) -> Result<CheckpointReceipt> {
        if self.checkpoints.len() >= MAX_LAB_CHECKPOINTS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory checkpoint store reached its explicit bound",
            ));
        }
        let nonce = self.next_nonce()?;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"dfmcp-lab-checkpoint-v1");
        bytes.extend_from_slice(self.snapshot.state_hash.as_bytes());
        bytes.extend_from_slice(&nonce.to_be_bytes());
        let digest = Digest32::of_bytes(&bytes);
        let checkpoint_id = CheckpointId::new(nonzero(digest.first_u128()));
        if self.checkpoints.contains_key(&checkpoint_id) {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "derived laboratory checkpoint identifier collided with existing content",
            ));
        }
        self.checkpoints
            .insert(checkpoint_id, self.snapshot.clone());
        let evidence = evidence(
            self.snapshot.anchor(),
            EvidenceKind::Checkpoint,
            &format!("laboratory checkpoint {label}"),
        );
        self.record_event(LabEvent::Checkpointed(checkpoint_id));
        Ok(CheckpointReceipt {
            checkpoint_id,
            label: label.to_owned(),
            anchor: self.snapshot.anchor(),
            content_digest: self.snapshot.state_hash,
            // This is a process-local snapshot clone. It is a useful recovery
            // laboratory, but it cannot survive process or machine loss.
            durable: false,
            evidence: vec![evidence],
        })
    }

    fn dependencies_verified(&self, plan_id: PlanId, step: &PlanStep) -> bool {
        step.depends_on.iter().all(|dependency| {
            self.action_by_step
                .get(&(plan_id, *dependency))
                .and_then(|action_id| self.actions.get(action_id))
                .is_some_and(|action| action.receipt.state == CommitState::Verified)
        })
    }

    fn dispatch_step(
        &mut self,
        plan_id: PlanId,
        step: &PlanStep,
        action_id: ActionId,
    ) -> Result<ActionReceipt> {
        if self.actions.contains_key(&action_id)
            || self.action_by_step.contains_key(&(plan_id, step.id))
        {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "derived laboratory action identity collided with existing state",
            ));
        }
        let mut obligation_runtime = None;
        let state = if self.dependencies_verified(plan_id, step) {
            let observation = PredicateEvidence::laboratory(&self.snapshot)?;
            if !predicates_established(&observation, &step.preconditions)? {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("step {} failed dispatch-time revalidation", step.id),
                ));
            }
            obligation_runtime = register_step_obligation(action_id, step, &self.snapshot)?;
            apply_action(&mut self.snapshot, &step.action, &step.idempotency_key)?;
            let observation = PredicateEvidence::laboratory(&self.snapshot)?;
            if let Some(runtime) = obligation_runtime.as_mut() {
                obligation_commit_state(runtime, action_id, &observation)?
            } else if predicates_established(&observation, &step.postconditions)? {
                CommitState::Verified
            } else {
                return Err(DfmcpError::new(
                    ErrorCode::AdapterRejected,
                    format!(
                        "immediate postconditions for step {} are not established true",
                        step.id
                    ),
                ));
            }
        } else {
            CommitState::Prepared
        };
        let summary = match state {
            CommitState::Prepared => "waiting for dependency verification",
            CommitState::Verified => "semantic postconditions verified",
            CommitState::Failed => "obligation failure observed after dispatch",
            _ => "action applied; semantic verification remains pending",
        };
        let receipt = build_action_receipt(
            action_id,
            step.id,
            state,
            self.snapshot.anchor(),
            EvidenceKind::AdapterReceipt,
            summary,
        );
        self.action_by_step.insert((plan_id, step.id), action_id);
        self.actions.insert(
            action_id,
            LabAction {
                plan_id,
                step: step.clone(),
                receipt: receipt.clone(),
                obligation_runtime,
                cancel_mode: None,
                dispatched: state != CommitState::Prepared,
            },
        );
        Ok(receipt)
    }

    fn refresh_action(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<ActionReceipt> {
        let (plan_id, step, prior_receipt, mut obligation_runtime) = self
            .actions
            .get(&action_id)
            .map(|action| {
                (
                    action.plan_id,
                    action.step.clone(),
                    action.receipt.clone(),
                    action.obligation_runtime.clone(),
                )
            })
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown action {action_id}"),
                )
            })?;
        let prior_state = prior_receipt.state;

        // Terminal proof belongs to the observation that established it.
        // Polling later must not rewrite its anchor, evidence or receipt. Nor
        // may an ordinary poll disguise a cancellation drain or uncertainty.
        if !matches!(
            prior_state,
            CommitState::Prepared | CommitState::AppliedAwaitingVerification
        ) {
            self.record_event(LabEvent::ActionPolled(action_id, prior_state));
            return Ok(prior_receipt);
        }

        let dependencies_verified = if prior_state == CommitState::Prepared {
            let observation = PredicateEvidence::laboratory(&self.snapshot)?;
            match deferred_step_decision_with_evidence(&step, &observation, |dependency| {
                self.step_receipt(plan_id, dependency)
                    .map(|receipt| receipt.state)
            })? {
                DeferredStepDecision::Ready => true,
                DeferredStepDecision::Waiting => false,
                DeferredStepDecision::Failed(message) => {
                    let receipt = self.stored_action_receipt(
                        action_id,
                        CommitState::Failed,
                        EvidenceKind::Postcondition,
                        &message,
                    )?;
                    self.record_event(LabEvent::ActionPolled(action_id, CommitState::Failed));
                    return Ok(receipt);
                }
            }
        } else {
            true
        };
        if prior_state == CommitState::Prepared && dependencies_verified {
            // Observing a running effect is read-only; starting its deferred
            // successor is a new effect boundary with current scoped authority.
            self.authorize_step(&step, context)?;
            if context.budget.max_actions == 0 {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "polling a deferred step requires an available action budget",
                ));
            }
            obligation_runtime = register_step_obligation(action_id, &step, &self.snapshot)?;
            apply_action(&mut self.snapshot, &step.action, &step.idempotency_key)?;
            if let Some(stored) = self.actions.get_mut(&action_id) {
                stored.dispatched = true;
            }
        }

        let mut state = prior_state;
        if matches!(
            state,
            CommitState::Prepared | CommitState::AppliedAwaitingVerification
        ) && dependencies_verified
        {
            let observation = PredicateEvidence::laboratory(&self.snapshot)?;
            if let Some(runtime) = obligation_runtime.as_mut() {
                state = obligation_commit_state(runtime, action_id, &observation)?;
            } else if step.obligation.is_some() {
                return Err(DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "dispatched temporal action has no registered proof monitor",
                ));
            } else if predicates_established(&observation, &step.postconditions)? {
                state = CommitState::Verified;
            } else {
                return Err(DfmcpError::new(
                    ErrorCode::AdapterRejected,
                    format!(
                        "immediate postconditions for step {} are not established true",
                        step.id
                    ),
                ));
            }
        }

        let message = if state == CommitState::Verified {
            "semantic postconditions verified"
        } else if state == CommitState::Failed {
            "obligation failed or reached its game-tick deadline without stable completion"
        } else if state == CommitState::Prepared {
            "waiting for dependency verification"
        } else {
            "verification pending"
        };
        let receipt = build_action_receipt(
            action_id,
            step.id,
            state,
            self.snapshot.anchor(),
            EvidenceKind::Postcondition,
            message,
        );
        if let Some(action) = self.actions.get_mut(&action_id) {
            action.receipt = receipt.clone();
            action.obligation_runtime = obligation_runtime;
        }
        self.record_event(LabEvent::ActionPolled(action_id, state));
        Ok(receipt)
    }
}

impl GameAdapter for MemoryAdapter {
    fn identity(&self) -> AdapterIdentity {
        self.identity.clone()
    }

    fn current_anchor(&self) -> Option<StateAnchor> {
        Some(self.snapshot.anchor())
    }

    fn health(&mut self, context: &OperationContext) -> Result<AdapterHealth> {
        context.authorize(Capability::Doctor, RiskTier::ReadOnly, &[], None)?;
        self.check_anchor(context.anchor)?;
        Ok(AdapterHealth {
            status: HealthStatus::Healthy,
            identity: self.identity(),
            fortress_loaded: true,
            paused: Some(self.snapshot.paused),
            current_anchor: Some(self.snapshot.anchor()),
            warnings: Vec::new(),
        })
    }

    fn observe(
        &mut self,
        request: &ObservationRequest,
        context: &OperationContext,
    ) -> Result<ObservationFrame> {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.check_anchor(context.anchor)?;
        if request.max_entities == 0
            || request.max_entities > context.budget.max_entities
            || request.max_bytes == 0
            || request.max_bytes > context.budget.max_bytes
            || request.max_output_tokens == 0
            || request.max_output_tokens > context.budget.max_output_tokens
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "observation request exceeds its operation budget",
            ));
        }
        if request.continuation.is_some() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "laboratory adapter has no outstanding observation continuation",
            ));
        }
        if request.since.is_none() {
            let entity_count = u32::try_from(self.snapshot.graph.entities.len()).map_err(|_| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "laboratory snapshot entity count cannot be represented",
                )
            })?;
            let snapshot_bytes =
                u64::try_from(self.snapshot.canonical_bytes().len()).map_err(|_| {
                    DfmcpError::new(
                        ErrorCode::BudgetExceeded,
                        "laboratory snapshot byte count cannot be represented",
                    )
                })?;
            let output_byte_bound = u64::from(request.max_output_tokens)
                .saturating_mul(CONSERVATIVE_BYTES_PER_OUTPUT_TOKEN);
            if entity_count > request.max_entities
                || snapshot_bytes > request.max_bytes
                || snapshot_bytes > output_byte_bound
            {
                return Err(DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "full laboratory snapshot exceeds the requested entity, byte, or conservative output-token bound",
                ));
            }
        }
        let payload = match request.since {
            None => ObservationPayload::Snapshot(self.snapshot.clone()),
            Some(cursor) if cursor == self.snapshot.cursor => {
                ObservationPayload::Heartbeat(self.snapshot.anchor())
            }
            Some(_) => {
                return Err(DfmcpError::new(
                    ErrorCode::CursorGap,
                    "laboratory adapter retains no delta history for the requested cursor",
                )
                .retryable(true));
            }
        };
        self.record_event(LabEvent::Observed(self.snapshot.anchor()));
        Ok(ObservationFrame {
            payload,
            evidence: vec![evidence(
                self.snapshot.anchor(),
                EvidenceKind::Observation,
                "deterministic laboratory observation",
            )],
            warnings: Vec::new(),
            truncated: false,
            continuation: None,
        })
    }

    fn query(
        &mut self,
        request: &QueryRequest,
        context: &OperationContext,
    ) -> Result<QueryResponse> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        self.check_anchor(context.anchor)?;
        self.check_anchor(request.anchor)?;
        if request.max_output_tokens == 0
            || request.max_output_tokens > context.budget.max_output_tokens
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query output-token budget is invalid",
            ));
        }
        if self.snapshot.graph.entities.len() > context.budget.max_entities as usize {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query would scan more entities than its operation budget permits",
            ));
        }
        let mut query = request.query.clone();
        if let (Some(outer), Some(inner)) = (&request.continuation, &query.continuation)
            && outer != inner
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "query continuation fields disagree",
            ));
        }
        query.continuation = request
            .continuation
            .clone()
            .or_else(|| query.continuation.clone());
        let byte_limit_u64 = context.budget.max_bytes.min(
            u64::from(request.max_output_tokens)
                .saturating_mul(CONSERVATIVE_BYTES_PER_OUTPUT_TOKEN),
        );
        let byte_limit = usize::try_from(byte_limit_u64).map_err(|_| {
            DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query byte budget cannot be represented on this platform",
            )
        })?;
        let result = execute_bounded_query(
            &self.snapshot,
            &query,
            context.budget.max_entities,
            Some(byte_limit),
        )?;
        let rows = result
            .entities
            .into_iter()
            .map(|entity| QueryRow {
                entity_id: entity.id,
                revision: entity.revision,
                fields: vec![
                    ("label".to_owned(), entity.label),
                    ("kind".to_owned(), entity.kind.as_str().to_owned()),
                ],
                score_micros: None,
                evidence: Vec::new(),
            })
            .collect();
        Ok(QueryResponse {
            anchor: self.snapshot.anchor(),
            rows,
            matched: result.matched,
            truncated: result.truncated,
            continuation: result.continuation,
            score_ledger: vec![
                "ordered deterministic world query; no relevance scoring".to_owned(),
            ],
        })
    }

    fn prepare(
        &mut self,
        plan: &PreparedPlan,
        context: &OperationContext,
    ) -> Result<PrepareReceipt> {
        self.check_anchor(context.anchor)?;
        plan.validate_structure()?;
        if plan.anchor != self.snapshot.anchor() {
            return Err(
                DfmcpError::new(ErrorCode::StaleAnchor, "plan anchor is stale").retryable(true),
            );
        }
        if self.snapshot.tick >= plan.expires_at_tick {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan expired before adapter preparation",
            ));
        }
        if plan.steps.len() > context.budget.max_actions as usize {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "plan exceeds the adapter operation action budget",
            ));
        }
        let observation = PredicateEvidence::laboratory(&self.snapshot)?;
        for step in &plan.steps {
            self.authorize_step(step, context)?;
            if !predicates_established(&observation, &step.preconditions)? {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("step {} failed preparation revalidation", step.id),
                ));
            }
        }
        if plan.requires_checkpoint {
            context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
        }
        if let Some(existing_plan) = self.plans.get(&plan.id) {
            if existing_plan != plan {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "plan identifier was reused for nonidentical plan content",
                ));
            }
            if let Some(existing_receipt) = self.prepared.get(&plan.id) {
                return Ok(existing_receipt.clone());
            }
            return Err(DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "stored plan is missing its prepare receipt",
            ));
        }
        if self.plans.len() >= MAX_LAB_PREPARED_PLANS
            || self.prepared.len() >= MAX_LAB_PREPARED_PLANS
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory prepared-plan store reached its explicit bound",
            ));
        }
        let nonce = self.next_nonce()?;
        let mut token = Vec::new();
        token.extend_from_slice(b"dfmcp-lab-prepare-v1");
        token.extend_from_slice(&plan.id.get().to_be_bytes());
        token.extend_from_slice(plan.digest.as_bytes());
        token.extend_from_slice(self.snapshot.state_hash.as_bytes());
        token.extend_from_slice(&nonce.to_be_bytes());
        let token_digest = Digest32::of_bytes(&token);
        let receipt = PrepareReceipt {
            plan_id: plan.id,
            plan_digest: plan.digest,
            revalidated_anchor: self.snapshot.anchor(),
            adapter_token: token,
            adapter_token_digest: token_digest,
            expires_at_tick: plan.expires_at_tick,
            warnings: Vec::new(),
        };
        self.prepared.insert(plan.id, receipt.clone());
        self.plans.insert(plan.id, plan.clone());
        self.record_event(LabEvent::Prepared(plan.id));
        Ok(receipt)
    }

    fn commit(
        &mut self,
        plan: &PreparedPlan,
        prepared: &PrepareReceipt,
        context: &OperationContext,
    ) -> Result<CommitReceipt> {
        plan.validate_structure()?;
        let stored_plan = self.plans.get(&plan.id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan was not prepared by this adapter",
            )
        })?;
        if stored_plan != plan {
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "commit plan does not exactly match the prepared plan",
            ));
        }
        if plan.steps.len() > context.budget.max_actions as usize {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "plan exceeds the commit action budget",
            ));
        }
        for step in &plan.steps {
            self.authorize_step(step, context)?;
        }
        if plan.requires_checkpoint {
            context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
        }

        // Authorization is deliberately rechecked before idempotent replay.
        // A stable idempotency key is not a bearer token for a prior caller's
        // authority, while an expired plan still retains its stable receipt.
        if let Some(existing) = self.commits.get(&plan.id) {
            if existing.plan_digest == plan.digest {
                return Ok(existing.clone());
            }
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "plan identifier was reused with a different digest",
            ));
        }
        self.check_anchor(context.anchor)?;
        let stored_receipt = self.prepared.get(&plan.id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidPlan,
                "plan is missing its prepare receipt",
            )
        })?;
        if stored_receipt != prepared
            || prepared.plan_id != plan.id
            || prepared.plan_digest != plan.digest
            || prepared.revalidated_anchor != self.snapshot.anchor()
            || Digest32::of_bytes(&prepared.adapter_token) != prepared.adapter_token_digest
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "prepare receipt is stale, forged, or inconsistent",
            ));
        }
        if self.snapshot.tick >= prepared.expires_at_tick
            || self.snapshot.tick >= plan.expires_at_tick
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "prepared plan expired before commit",
            ));
        }
        let observation = PredicateEvidence::laboratory(&self.snapshot)?;
        for step in &plan.steps {
            if !predicates_established(&observation, &step.preconditions)? {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("step {} failed commit-time revalidation", step.id),
                ));
            }
        }
        let new_action_count = self
            .actions
            .len()
            .checked_add(plan.steps.len())
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::BudgetExceeded,
                    "laboratory action count overflowed",
                )
            })?;
        if new_action_count > MAX_LAB_ACTIONS || self.commits.len() >= MAX_LAB_COMMITS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory action or commit store reached its explicit bound",
            ));
        }
        if plan.requires_checkpoint && self.checkpoints.len() >= MAX_LAB_CHECKPOINTS {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "laboratory checkpoint store reached its explicit bound",
            ));
        }

        // The memory laboratory uses a clone as its transaction shadow. Any
        // action, cursor, checkpoint, or journal failure restores every field.
        let prior_state = self.clone();
        let result = (|| {
            let checkpoint = if plan.requires_checkpoint {
                Some(self.internal_checkpoint(&format!("before-plan-{}", plan.id))?)
            } else {
                None
            };
            let mut actions = Vec::with_capacity(plan.steps.len());
            for step in &plan.steps {
                let action_id = derived_action_id(plan.id, step.id);
                actions.push(self.dispatch_step(plan.id, step, action_id)?);
            }
            self.record_event(LabEvent::Committed(plan.id));
            let receipt = CommitReceipt {
                plan_id: plan.id,
                plan_digest: plan.digest,
                actions,
                checkpoint,
                observed_anchor: self.snapshot.anchor(),
                warnings: Vec::new(),
            };
            self.commits.insert(plan.id, receipt.clone());
            Ok(receipt)
        })();
        if result.is_err() {
            *self = prior_state;
        }
        result
    }

    fn poll_action(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<ActionReceipt> {
        self.check_anchor(context.anchor)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if !self
            .actions
            .get(&action_id)
            .is_some_and(|action| action.receipt.state == CommitState::Prepared)
        {
            return self.refresh_action(action_id, context);
        }
        // A poll can dispatch deferred work. Its effect, proof monitor, receipt
        // and transcript must publish together, just as they do during commit.
        let prior = self.clone();
        let result = self.refresh_action(action_id, context);
        if result.is_err() {
            *self = prior;
        }
        result
    }

    fn request_cancel(
        &mut self,
        action_id: ActionId,
        mode: CancelMode,
        context: &OperationContext,
    ) -> Result<CancelReceipt> {
        self.check_anchor(context.anchor)?;
        let action = self.actions.get(&action_id).cloned().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("unknown action {action_id}"),
            )
        })?;
        let scope = action.step.action.scope();
        context.authorize(
            action.step.required_capability,
            action.step.risk,
            &scope.entity_ids,
            scope.map_area,
        )?;
        if mode == CancelMode::EmergencyPauseAndDrain {
            context.authorize(Capability::ControlClock, RiskTier::Reversible, &[], None)?;
        }
        if mode == CancelMode::CompensateReversible
            && let Some(compensation) = &action.step.compensation
        {
            if !action_is_supported(compensation)
                || !self
                    .identity
                    .capabilities
                    .contains(&compensation.capability())
            {
                return Err(DfmcpError::new(
                    ErrorCode::AdapterRejected,
                    "laboratory adapter cannot execute the compensation action",
                ));
            }
            let compensation_scope = compensation.scope();
            context.authorize(
                compensation.capability(),
                compensation.risk(),
                &compensation_scope.entity_ids,
                compensation_scope.map_area,
            )?;
        }

        match action.receipt.state {
            CommitState::CancelRequested => {
                if action.cancel_mode == Some(mode) {
                    return Ok(replayed_cancel_receipt(action_id, &action));
                }
                // A rejected or no-longer-authorized compensation must not
                // trap running work forever. The caller may narrow an existing
                // request to an explicitly authorized stop without compensation.
                if mode != CancelMode::StopFutureSteps {
                    return Err(DfmcpError::new(
                        ErrorCode::Conflict,
                        "cancellation was already requested with a different mode",
                    ));
                }
            }
            CommitState::Cancelled | CommitState::Compensated => {
                return Ok(replayed_cancel_receipt(action_id, &action));
            }
            state if state.is_terminal() => {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "cannot cancel an action that already reached a terminal state",
                ));
            }
            _ => {}
        }

        let state = CommitState::CancelRequested;
        if mode == CancelMode::EmergencyPauseAndDrain && !self.snapshot.paused {
            apply_action(&mut self.snapshot, &Action::Pause { paused: true }, "")?;
        }
        if let Some(stored) = self.actions.get_mut(&action_id) {
            stored.cancel_mode = Some(mode);
        }
        let message = "cancellation request recorded; drain remains pending";
        self.stored_action_receipt(action_id, state, EvidenceKind::AdapterReceipt, message)?;
        self.record_event(LabEvent::CancelRequested(action_id, mode));
        Ok(CancelReceipt {
            action_id,
            state,
            observed_anchor: self.snapshot.anchor(),
            compensation_action: None,
            evidence: vec![evidence(
                self.snapshot.anchor(),
                EvidenceKind::AdapterReceipt,
                message,
            )],
            message: message.to_owned(),
        })
    }

    fn finalize_cancel(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<CancelReceipt> {
        let prior = self.clone();
        let result = (|| {
            self.check_anchor(context.anchor)?;
            let action = self.actions.get(&action_id).cloned().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown action {action_id}"),
                )
            })?;
            let scope = action.step.action.scope();
            context.authorize(
                action.step.required_capability,
                action.step.risk,
                &scope.entity_ids,
                scope.map_area,
            )?;

            let mut state = action.receipt.state;
            if matches!(state, CommitState::Cancelled | CommitState::Compensated) {
                if !self.action_work_state(action_id)?.is_quiescent() {
                    return Err(DfmcpError::new(
                        ErrorCode::CancellationIncomplete,
                        "prior cancellation no longer establishes physical quiescence",
                    ));
                }
                return Ok(replayed_cancel_receipt(action_id, &action));
            }
            if state.is_terminal() {
                return Err(DfmcpError::new(
                    ErrorCode::Conflict,
                    "cannot finalize cancellation for an action that completed independently",
                ));
            }
            if state != CommitState::CancelRequested {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    "cancellation must be requested before it can be finalized",
                ));
            }
            let mut compensation_action = None;
            let (_, _, stopped_work) =
                drain_step_work(&mut self.snapshot, &action.step, action.dispatched, context)?;
            if action.cancel_mode == Some(CancelMode::CompensateReversible) && action.dispatched {
                if let Some(compensation) = &action.step.compensation {
                    if compensation.naturally_temporal() {
                        return Err(DfmcpError::new(
                            ErrorCode::CancellationIncomplete,
                            "temporal compensation requires a separate bounded plan and proof",
                        ));
                    }
                    if !action_is_supported(compensation)
                        || !self
                            .identity
                            .capabilities
                            .contains(&compensation.capability())
                    {
                        return Err(DfmcpError::new(
                            ErrorCode::AdapterRejected,
                            "laboratory adapter cannot execute the compensation action",
                        ));
                    }
                    let compensation_scope = compensation.scope();
                    context.authorize(
                        compensation.capability(),
                        compensation.risk(),
                        &compensation_scope.entity_ids,
                        compensation_scope.map_area,
                    )?;
                    if context.budget.max_actions < 1 + u32::from(stopped_work) {
                        return Err(DfmcpError::new(
                            ErrorCode::BudgetExceeded,
                            "cancellation stop and compensation exceed the available action budget",
                        ));
                    }
                    let compensation_key = format!("{}:compensation", action.step.idempotency_key);
                    apply_action(&mut self.snapshot, compensation, &compensation_key)?;
                    let observation = PredicateEvidence::laboratory(&self.snapshot)?;
                    let postconditions = effects::default_postconditions(
                        compensation,
                        &compensation_key,
                        self.snapshot.fortress_id,
                    );
                    if !predicates_established(&observation, &postconditions)? {
                        return Err(DfmcpError::new(
                            ErrorCode::CancellationIncomplete,
                            "compensation postconditions are not established true",
                        ));
                    }
                    compensation_action = Some(derived_compensation_id(action_id));
                    state = CommitState::Compensated;
                } else {
                    state = CommitState::Cancelled;
                }
            } else {
                state = CommitState::Cancelled;
            }
            let message = match state {
                CommitState::Compensated => "cancellation drained and compensation applied",
                CommitState::Cancelled => "cancellation drained without compensation",
                _ => "action was already terminal before cancellation finalization",
            };
            if let Some(stored) = self.actions.get_mut(&action_id) {
                stored.cancel_mode = None;
            }
            self.stored_action_receipt(action_id, state, EvidenceKind::Postcondition, message)?;
            self.record_event(LabEvent::CancelFinalized(action_id, state));
            Ok(CancelReceipt {
                action_id,
                state,
                observed_anchor: self.snapshot.anchor(),
                compensation_action,
                evidence: vec![evidence(
                    self.snapshot.anchor(),
                    EvidenceKind::Postcondition,
                    message,
                )],
                message: message.to_owned(),
            })
        })();
        if result.is_err() {
            *self = prior;
        }
        result
    }

    fn checkpoint(&mut self, label: &str, context: &OperationContext) -> Result<CheckpointReceipt> {
        self.check_anchor(context.anchor)?;
        context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
        if label.is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "checkpoint label is empty, too long, or contains control characters",
            ));
        }
        self.internal_checkpoint(label)
    }

    fn restore(
        &mut self,
        checkpoint_id: CheckpointId,
        context: &OperationContext,
    ) -> Result<RestoreReceipt> {
        self.check_anchor(context.anchor)?;
        context.authorize(Capability::Restore, RiskTier::Guarded, &[], None)?;
        let checkpoint = self
            .checkpoints
            .get(&checkpoint_id)
            .cloned()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown checkpoint {checkpoint_id}"),
                )
            })?;
        if !checkpoint.hash_is_valid() {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "checkpoint snapshot failed its content-hash seal",
            ));
        }
        let prior_anchor = self.snapshot.anchor();
        let content_digest = checkpoint.state_hash;
        let restored_cursor = prior_anchor.cursor.checked_reset_epoch().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CursorGap,
                "cannot restore because the observation epoch is exhausted",
            )
        })?;
        self.snapshot = checkpoint;
        self.snapshot.cursor = restored_cursor;
        self.snapshot.refresh_hash();
        self.invalidate_active_work();
        self.record_event(LabEvent::Restored(checkpoint_id));
        Ok(RestoreReceipt {
            checkpoint_id,
            prior_anchor,
            restored_anchor: self.snapshot.anchor(),
            content_digest,
            evidence: vec![evidence(
                self.snapshot.anchor(),
                EvidenceKind::Checkpoint,
                "laboratory checkpoint restored into a new observation epoch",
            )],
        })
    }
}

/// Advancing reference time may create new observations at the destination
/// tick, but cannot legitimize future-dated facts already present at the source.
fn validate_tick_advance_source(snapshot: &WorldSnapshot) -> Result<()> {
    let observation = PredicateEvidence::laboratory(snapshot)?;
    let source = observation.snapshot();
    let fields = source
        .graph
        .entities
        .values()
        .flat_map(|entity| entity.fields.values())
        .chain(
            source
                .graph
                .edges
                .values()
                .flat_map(|edge| edge.fields.values()),
        );
    for fact in fields {
        if fact.observed_at > source.tick
            && dfmcp_world::laboratory_fact_value(fact, fact.observed_at).is_some()
        {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "laboratory clock advancement cannot promote a future-dated known source fact",
            ));
        }
    }
    Ok(())
}

/// Register at the actual effect boundary. The creation observation sets the
/// cadence floor but cannot provide a positive stability sample.
fn register_step_obligation(
    action_id: ActionId,
    step: &PlanStep,
    snapshot: &WorldSnapshot,
) -> Result<Option<ObligationRuntime>> {
    let Some(mut spec) = step.obligation.clone() else {
        return Ok(None);
    };
    let mut terminal = vec![spec.terminal];
    terminal.extend(step.postconditions.iter().cloned());
    spec.terminal = Predicate::All(terminal).normalized();
    let mut runtime = ObligationRuntime::new();
    runtime.register_obligation_at(action_id, spec, snapshot)?;
    Ok(Some(runtime))
}

/// Use the shared obligation engine for sampling, continuity, and deadlines.
fn obligation_commit_state(
    runtime: &mut ObligationRuntime,
    action_id: ActionId,
    observation: &PredicateEvidence<'_>,
) -> Result<CommitState> {
    runtime.step_tick_with_evidence(observation)?;
    match runtime.get_status(action_id) {
        Some(ObligationStatus::Fulfilled { .. }) => Ok(CommitState::Verified),
        Some(ObligationStatus::Failed { .. }) => Ok(CommitState::Failed),
        Some(ObligationStatus::Pending | ObligationStatus::Active { .. }) => {
            Ok(CommitState::AppliedAwaitingVerification)
        }
        _ => Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "action proof monitor is missing or not in an observable lifecycle state",
        )),
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

fn action_is_supported(action: &Action) -> bool {
    !matches!(action, Action::Extension { .. })
}

/// Publish a changed snapshot as the next observation.
fn publish_change(snapshot: &mut WorldSnapshot) -> Result<()> {
    let next_cursor = snapshot.cursor.checked_next().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::CursorGap,
            "cannot publish a laboratory mutation because the observation cursor is exhausted",
        )
    })?;
    snapshot.cursor = next_cursor;
    snapshot.refresh_hash();
    Ok(())
}

fn stop_action_work(snapshot: &mut WorldSnapshot, step: &PlanStep) -> Result<()> {
    let mut next = snapshot.clone();
    if effects::cancel_effect(&mut next, &step.action, &step.idempotency_key)? {
        publish_change(&mut next)?;
        *snapshot = next;
    }
    Ok(())
}

/// The caller owns a transaction covering stop, any compensation, and evidence.
/// An undispatched step has nothing to stop, even if a colliding world entity
/// exists; a missing entity after dispatch cannot be mistaken for this case.
fn drain_step_work(
    snapshot: &mut WorldSnapshot,
    step: &PlanStep,
    dispatched: bool,
    context: &OperationContext,
) -> Result<(EffectWorkState, EffectWorkState, bool)> {
    let before = inspect_effect_work(snapshot, &step.action, &step.idempotency_key, dispatched)?;
    if let EffectWorkState::Unknown { reason, .. } = &before {
        return Err(DfmcpError::new(
            ErrorCode::CancellationIncomplete,
            reason.clone(),
        ));
    }
    let stopped_work = matches!(before, EffectWorkState::Active { .. });
    if stopped_work {
        if context.budget.max_actions == 0 {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "stopping physical work requires an available action budget",
            ));
        }
        stop_action_work(snapshot, step)?;
    }
    let after = inspect_effect_work(snapshot, &step.action, &step.idempotency_key, dispatched)?;
    if !after.is_quiescent() {
        return Err(DfmcpError::new(
            ErrorCode::CancellationIncomplete,
            "reference work is not proved quiescent after stopping it",
        ));
    }
    Ok((before, after, stopped_work))
}

fn replayed_cancel_receipt(action_id: ActionId, action: &LabAction) -> CancelReceipt {
    CancelReceipt {
        action_id,
        state: action.receipt.state,
        observed_anchor: action.receipt.observed_anchor,
        compensation_action: (action.receipt.state == CommitState::Compensated)
            .then(|| derived_compensation_id(action_id)),
        evidence: action.receipt.evidence.clone(),
        message: action.receipt.message.clone(),
    }
}

/// Apply one action's reference semantics. Effects run on a shadow, so an
/// effect that fails a precondition leaves canonical state untouched.
fn apply_action(
    snapshot: &mut WorldSnapshot,
    action: &Action,
    idempotency_key: &str,
) -> Result<()> {
    let mut next = snapshot.clone();
    if effects::apply_effect(&mut next, action, idempotency_key)? {
        publish_change(&mut next)?;
        *snapshot = next;
    }
    Ok(())
}

fn build_action_receipt(
    action_id: ActionId,
    step_id: StepId,
    state: CommitState,
    anchor: StateAnchor,
    evidence_kind: EvidenceKind,
    message: &str,
) -> ActionReceipt {
    ActionReceipt {
        action_id,
        step_id,
        state,
        observed_anchor: anchor,
        adapter_receipt_digest: action_receipt_digest(action_id, step_id, state, anchor),
        evidence: vec![evidence(anchor, evidence_kind, message)],
        message: message.to_owned(),
    }
}

fn derived_action_id(plan_id: PlanId, step_id: StepId) -> ActionId {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-lab-action-v1");
    bytes.extend_from_slice(&plan_id.get().to_be_bytes());
    bytes.extend_from_slice(&step_id.get().to_be_bytes());
    ActionId::new(nonzero(Digest32::of_bytes(&bytes).first_u128()))
}

fn derived_compensation_id(action_id: ActionId) -> ActionId {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-lab-compensation-v1");
    bytes.extend_from_slice(&action_id.get().to_be_bytes());
    ActionId::new(nonzero(Digest32::of_bytes(&bytes).first_u128()))
}

fn action_receipt_digest(
    action_id: ActionId,
    step_id: StepId,
    state: CommitState,
    anchor: StateAnchor,
) -> Digest32 {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-lab-action-receipt-v1");
    bytes.extend_from_slice(&action_id.get().to_be_bytes());
    bytes.extend_from_slice(&step_id.get().to_be_bytes());
    bytes.push(commit_state_code(state));
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
    Digest32::of_bytes(&bytes)
}

fn commit_state_code(state: CommitState) -> u8 {
    match state {
        CommitState::Prepared => 0,
        CommitState::Committing => 1,
        CommitState::AppliedAwaitingVerification => 2,
        CommitState::Verified => 3,
        CommitState::CompensationPending => 4,
        CommitState::Compensated => 5,
        CommitState::CancelRequested => 6,
        CommitState::Cancelled => 7,
        CommitState::Failed => 8,
        CommitState::Indeterminate => 9,
    }
}

fn evidence(anchor: StateAnchor, kind: EvidenceKind, summary: &str) -> Evidence {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"dfmcp-lab-evidence-v1");
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
    bytes.extend_from_slice(summary.as_bytes());
    let digest = Digest32::of_bytes(&bytes);
    Evidence {
        id: EvidenceId::new(nonzero(digest.first_u128())),
        kind,
        subject: None,
        anchor,
        digest,
        summary: summary.to_owned(),
    }
}

const fn nonzero(value: u128) -> u128 {
    if value == 0 { 1 } else { value }
}

#[cfg(test)]
mod tests {
    use dfmcp_adapter::{GameAdapter, ObservationRequest, Projection};
    use dfmcp_core::{
        Capability, CapabilityGrant, CapabilityScope, CommitState, DfmcpError, ErrorCode,
        FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId, RiskTier,
        SessionId, WorkBudget,
    };
    use dfmcp_intent::{
        Action, Constraint, Intent, ObligationSpec, RequestedAction, StaticPlanner,
    };
    use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};

    use super::MemoryAdapter;

    fn grants(fortress_id: FortressId) -> Vec<CapabilityGrant> {
        [
            (Capability::Observe, RiskTier::ReadOnly),
            (Capability::Plan, RiskTier::ReadOnly),
            (Capability::ControlClock, RiskTier::Reversible),
            (Capability::Checkpoint, RiskTier::Guarded),
            (Capability::Restore, RiskTier::Guarded),
            (Capability::Doctor, RiskTier::ReadOnly),
        ]
        .into_iter()
        .map(|(capability, max_risk)| CapabilityGrant {
            capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
    }

    fn context(snapshot: &WorldSnapshot, request: u128) -> OperationContext {
        OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(request),
            anchor: snapshot.anchor(),
            budget: WorkBudget::default(),
            grants: grants(snapshot.fortress_id),
            cancellation_requested: false,
        }
    }

    fn unpause_intent(
        snapshot: &WorldSnapshot,
        intent_id: u128,
        obligation: Option<ObligationSpec>,
    ) -> Intent {
        Intent {
            id: IntentId::new(intent_id),
            anchor: snapshot.anchor(),
            summary: "unpause".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
            requested_actions: vec![RequestedAction {
                action: Action::Pause { paused: false },
                preconditions: vec![Predicate::Paused(true)],
                postconditions: Vec::new(),
                compensation: None,
                obligation,
                depends_on: Vec::new(),
            }],
        }
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
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "missing evidence subject",
                )
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
    fn preparation_requires_exact_laboratory_fact_authority() -> Result<(), DfmcpError> {
        use dfmcp_world::FactSource;
        let snapshot = || {
            WorldSnapshot::new(
                FortressId::new(1),
                GameTick(1),
                ObservationCursor::ORIGIN,
                true,
                WorldGraph::default(),
            )
        };
        for fact in ineligible_facts(GameTick(1)) {
            let mut source = snapshot();
            add_authority_fields(&mut source, fact);
            assert!(dfmcp_world::evaluate(
                &source,
                &authority_predicate("ready")
            ));
            // An otherwise sealed plan can arrive from another planner. The
            // adapter must enforce evidence authority independently.
            let intent = unpause_intent(&source, 31, None);
            let mut plan = StaticPlanner::default().prepare_laboratory(
                &source,
                &intent,
                &context(&source, 1),
            )?;
            plan.steps[0].preconditions = vec![authority_predicate("ready")];
            reseal_authority_plan(&mut plan)?;
            let mut adapter = MemoryAdapter::new(source.clone());
            let result = adapter.prepare(&plan, &context(&source, 2));
            assert!(matches!(result, Err(error) if error.code == ErrorCode::PreconditionsFailed));
            assert_eq!(adapter.snapshot(), &source);
            assert!(adapter.prepared.is_empty());
        }
        for producer in ["dfmcp.lab-scenario/1", "dfmcp.reference-effects/1"] {
            let mut source = snapshot();
            let mut fact = authority_fact(true, source.tick);
            fact.source = FactSource::Derived(producer.to_owned());
            add_authority_fields(&mut source, fact);
            let mut intent = unpause_intent(&source, 32, None);
            intent.requested_actions[0].preconditions = vec![authority_predicate("ready")];
            let plan = StaticPlanner::default().prepare_laboratory(
                &source,
                &intent,
                &context(&source, 1),
            )?;
            let mut adapter = MemoryAdapter::new(source);
            let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
            let receipt = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
            assert_eq!(receipt.actions[0].state, CommitState::Verified);
        }
        Ok(())
    }

    #[test]
    fn polling_ignores_ineligible_terminal_and_failure_claims() -> Result<(), DfmcpError> {
        for fact in ineligible_facts(GameTick(1)) {
            let mut source = WorldSnapshot::new(
                FortressId::new(1),
                GameTick(1),
                ObservationCursor::ORIGIN,
                true,
                WorldGraph::default(),
            );
            add_authority_fields(&mut source, fact);
            let intent = unpause_intent(
                &source,
                33,
                Some(ObligationSpec {
                    terminal: authority_predicate("done"),
                    failure: Some(authority_predicate("failed")),
                    deadline_tick: GameTick(10),
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                }),
            );
            let plan = StaticPlanner::default().prepare_laboratory(
                &source,
                &intent,
                &context(&source, 1),
            )?;
            let mut adapter = MemoryAdapter::new(source);
            let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
            let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
            let action_id = committed.actions[0].action_id;
            let pending = adapter.poll_action(action_id, &context(adapter.snapshot(), 4))?;
            assert_eq!(pending.state, CommitState::AppliedAwaitingVerification);

            // A fresh observation resolves every previously ineligible model input
            // before advancing the reference clock.
            let tick = adapter.snapshot.tick;
            set_authority_field(&mut adapter.snapshot, "ready", authority_fact(true, tick))?;
            set_authority_field(&mut adapter.snapshot, "done", authority_fact(false, tick))?;
            set_authority_field(&mut adapter.snapshot, "failed", authority_fact(false, tick))?;
            adapter.snapshot.cursor = adapter
                .snapshot
                .cursor
                .checked_next()
                .ok_or_else(|| DfmcpError::new(ErrorCode::CursorGap, "test cursor overflow"))?;
            adapter.snapshot.refresh_hash();
            adapter.advance_ticks(1)?;
            let fact = authority_fact(true, adapter.snapshot.tick);
            set_authority_field(&mut adapter.snapshot, "done", fact)?;
            let verified = adapter.poll_action(action_id, &context(adapter.snapshot(), 5))?;
            assert_eq!(verified.state, CommitState::Verified);
        }
        Ok(())
    }

    #[test]
    fn polling_uses_authoritative_failure_evidence() -> Result<(), DfmcpError> {
        use dfmcp_world::{Fact, FactSource, Value};
        let mut source = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        add_authority_fields(
            &mut source,
            Fact::known(
                Value::Bool(true),
                GameTick(1),
                FactSource::AgentAssertion("unverified report".to_owned()),
                dfmcp_core::Digest32::ZERO,
            ),
        );
        let intent = unpause_intent(
            &source,
            34,
            Some(ObligationSpec {
                terminal: authority_predicate("done"),
                failure: Some(authority_predicate("failed")),
                deadline_tick: GameTick(10),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            }),
        );
        let plan =
            StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source, 1))?;
        let mut adapter = MemoryAdapter::new(source);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
        let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
        let action_id = committed.actions[0].action_id;
        assert_eq!(
            adapter
                .poll_action(action_id, &context(adapter.snapshot(), 4))?
                .state,
            CommitState::AppliedAwaitingVerification
        );
        adapter.advance_ticks(1)?;
        let fact = authority_fact(true, adapter.snapshot.tick);
        set_authority_field(&mut adapter.snapshot, "failed", fact)?;
        assert_eq!(
            adapter
                .poll_action(action_id, &context(adapter.snapshot(), 5))?
                .state,
            CommitState::Failed
        );
        Ok(())
    }

    #[test]
    fn deferred_effect_cannot_use_a_new_agent_assertion_as_its_precondition()
    -> Result<(), DfmcpError> {
        use dfmcp_world::{Fact, FactSource, Value};
        let mut source = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        add_authority_fields(&mut source, authority_fact(true, GameTick(1)));
        set_authority_field(&mut source, "done", authority_fact(false, GameTick(1)))?;
        let mut intent = unpause_intent(
            &source,
            35,
            Some(ObligationSpec {
                terminal: authority_predicate("done"),
                failure: None,
                deadline_tick: GameTick(10),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            }),
        );
        intent.requested_actions.push(RequestedAction {
            action: Action::Pause { paused: true },
            preconditions: vec![authority_predicate("ready")],
            postconditions: vec![Predicate::Paused(true)],
            compensation: None,
            obligation: None,
            depends_on: vec![0],
        });
        let plan =
            StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source, 1))?;
        let mut adapter = MemoryAdapter::new(source);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
        let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
        assert_eq!(committed.actions[1].state, CommitState::Prepared);
        adapter.advance_ticks(1)?;
        let tick = adapter.snapshot.tick;
        set_authority_field(&mut adapter.snapshot, "done", authority_fact(true, tick))?;
        set_authority_field(
            &mut adapter.snapshot,
            "ready",
            Fact::known(
                Value::Bool(true),
                tick,
                FactSource::AgentAssertion("still ready".to_owned()),
                dfmcp_core::Digest32::ZERO,
            ),
        )?;
        assert_eq!(
            adapter
                .poll_action(
                    committed.actions[0].action_id,
                    &context(adapter.snapshot(), 4)
                )?
                .state,
            CommitState::Verified
        );
        let refused = adapter.poll_action(
            committed.actions[1].action_id,
            &context(adapter.snapshot(), 5),
        )?;
        assert_eq!(refused.state, CommitState::Failed);
        assert!(!adapter.snapshot.paused);
        assert!(refused.message.contains("not dispatched"));
        Ok(())
    }

    fn cadence_adapter(
        interval: u64,
        stable: u32,
        deadline: u64,
    ) -> Result<(MemoryAdapter, dfmcp_core::ActionId), DfmcpError> {
        let mut source = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        add_authority_fields(&mut source, authority_fact(true, GameTick(1)));
        set_authority_field(&mut source, "failed", authority_fact(false, GameTick(1)))?;
        let intent = unpause_intent(
            &source,
            51,
            Some(ObligationSpec {
                terminal: authority_predicate("done"),
                failure: Some(authority_predicate("failed")),
                deadline_tick: GameTick(deadline),
                poll_interval_ticks: interval,
                stable_for_observations: stable,
            }),
        );
        let plan =
            StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source, 1))?;
        let mut adapter = MemoryAdapter::new(source);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
        let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
        assert_eq!(
            committed.actions[0].state,
            CommitState::AppliedAwaitingVerification
        );
        Ok((adapter, committed.actions[0].action_id))
    }

    #[test]
    fn temporal_proof_requires_scheduled_samples_at_distinct_game_ticks() -> Result<(), DfmcpError>
    {
        let (mut adapter, action_id) = cadence_adapter(5, 2, 30)?;
        let pending = |adapter: &mut MemoryAdapter| -> Result<(), DfmcpError> {
            let receipt = adapter.poll_action(action_id, &context(adapter.snapshot(), 9))?;
            assert_eq!(receipt.state, CommitState::AppliedAwaitingVerification);
            Ok(())
        };
        pending(&mut adapter)?; // Dispatch at tick 1 is the cadence floor.
        adapter.advance_ticks(4)?;
        pending(&mut adapter)?; // Tick 5 is still before the first due sample.
        adapter.advance_ticks(1)?;
        pending(&mut adapter)?; // Tick 6 supplies only sample 1.

        // A genuinely new canonical observation at the same game tick cannot
        // manufacture a second sample, even when its cursor/hash changed.
        adapter.snapshot.cursor = adapter
            .snapshot
            .cursor
            .checked_next()
            .ok_or_else(|| DfmcpError::new(ErrorCode::CursorGap, "test cursor overflow"))?;
        adapter.snapshot.refresh_hash();
        pending(&mut adapter)?;
        adapter.advance_ticks(4)?;
        pending(&mut adapter)?;
        adapter.advance_ticks(1)?;
        let terminal = adapter.poll_action(action_id, &context(adapter.snapshot(), 10))?;
        assert_eq!(terminal.state, CommitState::Verified);
        assert_eq!(terminal.observed_anchor.tick, GameTick(11));

        adapter.advance_ticks(1)?;
        let fact = authority_fact(false, adapter.snapshot.tick);
        set_authority_field(&mut adapter.snapshot, "done", fact)?;
        assert_eq!(
            adapter.poll_action(action_id, &context(adapter.snapshot(), 11))?,
            terminal
        );
        Ok(())
    }

    #[test]
    fn off_cadence_contradiction_resets_stability_without_moving_the_poll_floor()
    -> Result<(), DfmcpError> {
        use dfmcp_world::{Fact, FactSource, Value};
        for ineligible in [
            authority_fact(false, GameTick(7)),
            Fact::known(
                Value::Bool(true),
                GameTick(7),
                FactSource::AgentAssertion("predicted complete".to_owned()),
                dfmcp_core::Digest32::ZERO,
            ),
        ] {
            let (mut adapter, action_id) = cadence_adapter(5, 2, 30)?;
            adapter.advance_ticks(5)?;
            assert_eq!(
                adapter
                    .poll_action(action_id, &context(adapter.snapshot(), 10))?
                    .state,
                CommitState::AppliedAwaitingVerification
            );
            adapter.advance_ticks(1)?;
            set_authority_field(&mut adapter.snapshot, "done", ineligible)?;
            assert_eq!(
                adapter
                    .poll_action(action_id, &context(adapter.snapshot(), 11))?
                    .state,
                CommitState::AppliedAwaitingVerification
            );
            adapter.advance_ticks(1)?;
            let fact = authority_fact(true, adapter.snapshot.tick);
            set_authority_field(&mut adapter.snapshot, "done", fact)?;
            adapter.poll_action(action_id, &context(adapter.snapshot(), 12))?;
            adapter.advance_ticks(3)?;
            assert_eq!(
                adapter
                    .poll_action(action_id, &context(adapter.snapshot(), 13))?
                    .state,
                CommitState::AppliedAwaitingVerification
            ); // Tick 11 is the replacement first sample, not completion.
            adapter.advance_ticks(5)?;
            let terminal = adapter.poll_action(action_id, &context(adapter.snapshot(), 14))?;
            assert_eq!(terminal.state, CommitState::Verified);
            assert_eq!(terminal.observed_anchor.tick, GameTick(16));
        }
        Ok(())
    }

    #[test]
    fn temporal_deadline_and_failure_precedence_apply_between_scheduled_polls()
    -> Result<(), DfmcpError> {
        let (mut exact, action_id) = cadence_adapter(100, 1, 5)?;
        let mut late = exact.clone();
        exact.advance_ticks(4)?;
        assert_eq!(
            exact
                .poll_action(action_id, &context(exact.snapshot(), 10))?
                .state,
            CommitState::Verified
        ); // A sufficient final sample is eligible at the exact deadline.
        late.advance_ticks(5)?;
        assert_eq!(
            late.poll_action(action_id, &context(late.snapshot(), 11))?
                .state,
            CommitState::Failed
        );
        let (mut insufficient, action_id) = cadence_adapter(100, 2, 5)?;
        insufficient.advance_ticks(4)?;
        assert_eq!(
            insufficient
                .poll_action(action_id, &context(insufficient.snapshot(), 12))?
                .state,
            CommitState::Failed
        );
        for elapsed in [1, 4] {
            let (mut failed, action_id) = cadence_adapter(100, 1, 5)?;
            failed.advance_ticks(elapsed)?;
            let fact = authority_fact(true, failed.snapshot.tick);
            set_authority_field(&mut failed.snapshot, "failed", fact)?;
            assert_eq!(
                failed
                    .poll_action(action_id, &context(failed.snapshot(), 13))?
                    .state,
                CommitState::Failed
            ); // Failure beats a positive terminal even off cadence or at deadline.
        }
        Ok(())
    }

    fn time_guard_order(paused: bool) -> Result<(WorldSnapshot, dfmcp_core::EntityId), DfmcpError> {
        let mut source = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            paused,
            WorldGraph::default(),
        );
        let key = "clock-source-authority";
        super::apply_action(
            &mut source,
            &Action::CreateWorkOrder {
                name: "one unit of reference work".to_owned(),
                job_token: "MAKE_TEST".to_owned(),
                amount: 1,
                conditions: Vec::new(),
            },
            key,
        )?;
        Ok((source, dfmcp_intent::effects::created_entity_id(key, 0)))
    }

    #[test]
    fn advancing_time_cannot_promote_a_preexisting_future_dated_work_counter()
    -> Result<(), DfmcpError> {
        use dfmcp_intent::effects;
        use dfmcp_world::Value;
        for paused in [false, true] {
            let (mut source, order_id) = time_guard_order(paused)?;
            let order = source.graph.entities.get_mut(&order_id).ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "missing test work order",
                )
            })?;
            let counter = order
                .fields
                .get_mut(effects::AMOUNT_REMAINING_FIELD)
                .ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "missing test remaining count",
                    )
                })?;
            counter.observed_at = GameTick(2);
            source.refresh_hash();
            assert!(
                dfmcp_world::laboratory_fact_value(
                    &source.graph.entities[&order_id].fields[effects::AMOUNT_REMAINING_FIELD],
                    source.tick
                )
                .is_none()
            );

            if !paused {
                // The low-level timeline now repeats the source-time check.
                // Advancing a caller's shadow clock cannot legitimize this fact.
                let mut promoted = source.clone();
                promoted.tick = GameTick(51);
                let result = effects::advance_effects(&mut promoted, 50);
                assert!(
                    matches!(result, Err(error) if error.code == ErrorCode::PreconditionsFailed)
                );
                assert_eq!(promoted.graph, source.graph);
                assert_eq!(promoted.tick, GameTick(51));
            }
            let mut adapter = MemoryAdapter::new(source.clone());
            let result = adapter.advance_ticks(50);
            assert!(matches!(result, Err(error) if error.code == ErrorCode::PreconditionsFailed));
            assert_eq!(adapter.snapshot(), &source);
            assert!(adapter.transcript().is_empty());
        }

        // The original observation must be bound before the model receives its
        // unsealed next-tick shadow: advancing time cannot repair a forged hash
        // or a canonical serialization that concealed an aliased entity key.
        for aliased_entity in [false, true] {
            let (mut source, order_id) = time_guard_order(false)?;
            if aliased_entity {
                let record = source.graph.entities.remove(&order_id).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "missing test work order",
                    )
                })?;
                let alias = if order_id == dfmcp_core::EntityId::new(1) {
                    dfmcp_core::EntityId::new(2)
                } else {
                    dfmcp_core::EntityId::new(1)
                };
                source.graph.entities.insert(alias, record);
                source.refresh_hash();
                assert!(source.hash_is_valid());
            } else {
                source.paused = true;
                assert!(!source.hash_is_valid());
            }
            let mut adapter = MemoryAdapter::new(source.clone());
            assert!(adapter.advance_ticks(50).is_err());
            assert_eq!(adapter.snapshot(), &source);
            assert!(adapter.transcript().is_empty());
        }
        Ok(())
    }

    #[test]
    fn clock_guard_preserves_untrusted_metadata_and_new_owned_model_observations()
    -> Result<(), DfmcpError> {
        use dfmcp_intent::effects;
        use dfmcp_world::{Fact, FactSource, Value};
        let (mut source, order_id) = time_guard_order(false)?;
        let order = source.graph.entities.get_mut(&order_id).ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "missing test work order",
            )
        })?;
        for (field, producer) in [
            (
                "prediction",
                FactSource::AgentAssertion("agent model".to_owned()),
            ),
            (
                "imported_forecast",
                FactSource::Derived("unregistered-forecast/1".to_owned()),
            ),
        ] {
            order.fields.insert(
                field.to_owned(),
                Fact::known(
                    Value::Bool(true),
                    GameTick(2),
                    producer,
                    dfmcp_core::Digest32::ZERO,
                ),
            );
        }
        source.refresh_hash();
        let mut adapter = MemoryAdapter::new(source);
        adapter.advance_ticks(50)?;
        let current = adapter.snapshot();
        let counter = &current.graph.entities[&order_id].fields[effects::AMOUNT_REMAINING_FIELD];
        assert_eq!(current.tick, GameTick(51));
        assert_eq!(counter.value, Value::U64(0));
        assert_eq!(counter.observed_at, current.tick);
        assert!(dfmcp_world::laboratory_fact_value(counter, current.tick).is_some());
        for field in ["prediction", "imported_forecast"] {
            assert!(
                dfmcp_world::laboratory_fact_value(
                    &current.graph.entities[&order_id].fields[field],
                    current.tick
                )
                .is_none()
            );
        }
        Ok(())
    }

    #[test]
    fn deferred_poll_rolls_back_world_dispatch_flag_receipt_and_transcript_on_error()
    -> Result<(), DfmcpError> {
        use dfmcp_world::{CompareOp, Value};
        let mut source = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        add_authority_fields(&mut source, authority_fact(true, GameTick(1)));
        set_authority_field(&mut source, "done", authority_fact(false, GameTick(1)))?;
        let mut intent = unpause_intent(
            &source,
            61,
            Some(ObligationSpec {
                terminal: authority_predicate("done"),
                failure: None,
                deadline_tick: GameTick(10),
                poll_interval_ticks: 1,
                stable_for_observations: 1,
            }),
        );
        intent.requested_actions.push(RequestedAction {
            action: Action::Pause { paused: true },
            preconditions: vec![authority_predicate("ready")],
            postconditions: vec![Predicate::Paused(true)],
            compensation: None,
            obligation: None,
            depends_on: vec![0],
        });
        let plan =
            StaticPlanner::default().prepare_laboratory(&source, &intent, &context(&source, 1))?;
        let mut adapter = MemoryAdapter::new(source);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
        let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
        adapter.advance_ticks(1)?;
        let tick = adapter.snapshot.tick;
        set_authority_field(&mut adapter.snapshot, "done", authority_fact(true, tick))?;
        assert_eq!(
            adapter
                .poll_action(
                    committed.actions[0].action_id,
                    &context(adapter.snapshot(), 4)
                )?
                .state,
            CommitState::Verified
        );
        let deferred = committed.actions[1].action_id;
        for cursor_exhausted in [false, true] {
            let mut attempted = adapter.clone();
            if cursor_exhausted {
                attempted.snapshot.cursor.sequence = u64::MAX;
                attempted.snapshot.refresh_hash();
            } else {
                // Inject corruption at the stored-proof boundary. The valid
                // deferred pause effect runs before this malformed predicate
                // is evaluated, so an error must undo that effect and all flags.
                let action = attempted.actions.get_mut(&deferred).ok_or_else(|| {
                    DfmcpError::new(
                        ErrorCode::InternalInvariantViolation,
                        "missing deferred action",
                    )
                })?;
                action.step.postconditions = vec![Predicate::FieldCompare {
                    entity_id: dfmcp_core::EntityId::new(9),
                    field: "x".repeat(257),
                    op: CompareOp::Eq,
                    value: Value::Bool(true),
                }];
            }
            let before_snapshot = attempted.snapshot.clone();
            let before_receipt = attempted.action_receipt(deferred).cloned();
            let before_transcript = attempted.transcript().clone();
            let result = attempted.poll_action(deferred, &context(attempted.snapshot(), 5));
            assert!(result.is_err());
            if cursor_exhausted {
                assert!(matches!(result, Err(error) if error.code == ErrorCode::CursorGap));
            }
            assert_eq!(attempted.snapshot, before_snapshot);
            assert_eq!(attempted.action_receipt(deferred).cloned(), before_receipt);
            assert_eq!(attempted.transcript(), &before_transcript);
            assert!(!attempted.actions[&deferred].dispatched);
        }
        Ok(())
    }

    fn unproved_immediate_facts(tick: GameTick) -> [Option<dfmcp_world::Fact>; 3] {
        use dfmcp_world::{Fact, FactSource, Value};
        [
            None,
            Some(Fact::known(
                Value::Bool(true),
                tick,
                FactSource::AgentAssertion("expected immediate result".to_owned()),
                dfmcp_core::Digest32::ZERO,
            )),
            Some(authority_fact(false, tick)),
        ]
    }

    #[test]
    fn immediate_dispatch_requires_postcondition_proof_and_rolls_back_on_refusal()
    -> Result<(), DfmcpError> {
        for fact in unproved_immediate_facts(GameTick(1)) {
            let mut source = WorldSnapshot::new(
                FortressId::new(1),
                GameTick(1),
                ObservationCursor::ORIGIN,
                true,
                WorldGraph::default(),
            );
            add_authority_fields(&mut source, authority_fact(true, GameTick(1)));
            if let Some(fact) = fact {
                set_authority_field(&mut source, "immediate_done", fact)?;
            }
            let mut intent = unpause_intent(&source, 71, None);
            intent.requested_actions[0].postconditions = vec![
                Predicate::Paused(false),
                authority_predicate("immediate_done"),
            ];
            let plan = StaticPlanner::default().prepare_laboratory(
                &source,
                &intent,
                &context(&source, 1),
            )?;
            let mut adapter = MemoryAdapter::new(source);
            let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
            let before = adapter.clone();

            let result = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3));
            assert!(matches!(result, Err(error) if error.code == ErrorCode::AdapterRejected));
            assert_eq!(adapter.snapshot, before.snapshot);
            assert!(adapter.snapshot.paused);
            assert!(adapter.actions.is_empty());
            assert_eq!(adapter.action_by_step, before.action_by_step);
            assert_eq!(adapter.commits, before.commits);
            assert_eq!(adapter.prepared, before.prepared);
            assert_eq!(adapter.plans, before.plans);
            assert_eq!(adapter.transcript(), before.transcript());
            assert_eq!(adapter.nonce, before.nonce);
        }
        Ok(())
    }

    #[test]
    fn deferred_immediate_dispatch_requires_postcondition_proof_and_rolls_back_on_refusal()
    -> Result<(), DfmcpError> {
        for fact in unproved_immediate_facts(GameTick(1)) {
            let mut source = WorldSnapshot::new(
                FortressId::new(1),
                GameTick(1),
                ObservationCursor::ORIGIN,
                true,
                WorldGraph::default(),
            );
            add_authority_fields(&mut source, authority_fact(true, GameTick(1)));
            set_authority_field(&mut source, "done", authority_fact(false, GameTick(1)))?;
            if let Some(fact) = fact {
                set_authority_field(&mut source, "immediate_done", fact)?;
            }
            let mut intent = unpause_intent(
                &source,
                72,
                Some(ObligationSpec {
                    terminal: authority_predicate("done"),
                    failure: None,
                    deadline_tick: GameTick(10),
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                }),
            );
            intent.terminal_condition = Predicate::All(vec![
                Predicate::Paused(true),
                authority_predicate("done"),
                authority_predicate("immediate_done"),
            ])
            .normalized();
            intent.requested_actions.push(RequestedAction {
                action: Action::Pause { paused: true },
                preconditions: vec![authority_predicate("ready")],
                postconditions: vec![
                    Predicate::Paused(true),
                    authority_predicate("immediate_done"),
                ],
                compensation: None,
                obligation: None,
                depends_on: vec![0],
            });
            let plan = StaticPlanner::default().prepare_laboratory(
                &source,
                &intent,
                &context(&source, 1),
            )?;
            let mut adapter = MemoryAdapter::new(source);
            let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
            let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
            assert_eq!(
                committed.actions[0].state,
                CommitState::AppliedAwaitingVerification
            );
            assert_eq!(committed.actions[1].state, CommitState::Prepared);
            adapter.advance_ticks(1)?;
            let tick = adapter.snapshot.tick;
            set_authority_field(&mut adapter.snapshot, "done", authority_fact(true, tick))?;
            assert_eq!(
                adapter
                    .poll_action(
                        committed.actions[0].action_id,
                        &context(adapter.snapshot(), 4)
                    )?
                    .state,
                CommitState::Verified
            );
            let deferred = committed.actions[1].action_id;
            let before = adapter.clone();

            let result = adapter.poll_action(deferred, &context(adapter.snapshot(), 5));
            assert!(matches!(result, Err(error) if error.code == ErrorCode::AdapterRejected));
            assert_eq!(adapter.snapshot, before.snapshot);
            assert!(!adapter.snapshot.paused);
            assert_eq!(adapter.actions.len(), before.actions.len());
            for (action_id, prior) in &before.actions {
                let current = &adapter.actions[action_id];
                assert_eq!(current.receipt, prior.receipt);
                assert_eq!(current.dispatched, prior.dispatched);
                assert_eq!(
                    current.obligation_runtime.is_some(),
                    prior.obligation_runtime.is_some()
                );
            }
            assert!(!adapter.actions[&deferred].dispatched);
            assert!(adapter.actions[&deferred].obligation_runtime.is_none());
            assert_eq!(adapter.action_by_step, before.action_by_step);
            assert_eq!(adapter.commits, before.commits);
            assert_eq!(adapter.prepared, before.prepared);
            assert_eq!(adapter.plans, before.plans);
            assert_eq!(adapter.transcript(), before.transcript());
            assert_eq!(adapter.nonce, before.nonce);
        }
        Ok(())
    }

    #[test]
    fn duplicate_commit_does_not_duplicate_effect() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = unpause_intent(&snapshot, 1, None);
        let planner = StaticPlanner::default();
        let plan = planner.prepare_laboratory(&snapshot, &intent, &context(&snapshot, 1))?;
        let mut adapter = MemoryAdapter::new(snapshot);
        let prepare_context = context(adapter.snapshot(), 2);
        let prepared = adapter.prepare(&plan, &prepare_context)?;
        let commit_context = context(adapter.snapshot(), 3);
        let first = adapter.commit(&plan, &prepared, &commit_context)?;
        let cursor_after_first = adapter.snapshot().cursor;
        let retry_context = context(adapter.snapshot(), 4);
        let second = adapter.commit(&plan, &prepared, &retry_context)?;
        assert_eq!(first.actions[0].action_id, second.actions[0].action_id);
        assert_eq!(adapter.snapshot().cursor, cursor_after_first);
        Ok(())
    }

    #[test]
    fn same_cursor_observation_is_a_heartbeat() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let mut adapter = MemoryAdapter::new(snapshot);
        let request = ObservationRequest {
            since: Some(adapter.snapshot().cursor),
            projection: Projection::Summary,
            interest: Default::default(),
            max_entities: 1,
            max_bytes: 1_024,
            max_output_tokens: 128,
            continuation: None,
        };
        let observe_context = context(adapter.snapshot(), 1);
        let frame = adapter.observe(&request, &observe_context)?;
        assert!(matches!(
            frame.payload,
            dfmcp_adapter::ObservationPayload::Heartbeat(_)
        ));
        Ok(())
    }

    #[test]
    fn older_cursor_is_not_silently_bridged() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let mut adapter = MemoryAdapter::new(snapshot);
        let old_cursor = adapter.snapshot().cursor;
        adapter.advance_ticks(1)?;
        let request = ObservationRequest {
            since: Some(old_cursor),
            projection: Projection::Summary,
            interest: Default::default(),
            max_entities: 1,
            max_bytes: 1_024,
            max_output_tokens: 128,
            continuation: None,
        };
        let failure = adapter
            .observe(&request, &context(adapter.snapshot(), 1))
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "older laboratory cursor was silently bridged",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::CursorGap);
        Ok(())
    }

    #[test]
    fn laboratory_identity_does_not_advertise_unimplemented_effects() {
        let adapter = MemoryAdapter::new(WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        ));
        let identity = adapter.identity();
        for capability in [
            Capability::ControlClock,
            Capability::Designate,
            Capability::Construct,
            Capability::ConfigureLabor,
            Capability::ConfigureProduction,
            Capability::ConfigureLogistics,
            Capability::ConfigureMilitary,
        ] {
            assert!(identity.capabilities.contains(&capability));
        }
        // Extensions have no reference semantics in the laboratory.
        assert!(!identity.capabilities.contains(&Capability::Extension));
    }

    #[test]
    fn prepare_rejects_an_action_the_lab_cannot_execute() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = Intent {
            id: IntentId::new(9),
            anchor: snapshot.anchor(),
            summary: "run an extension the laboratory cannot model".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action: Action::Extension {
                    namespace: "lab".to_owned(),
                    name: "noop".to_owned(),
                    parameters: std::collections::BTreeMap::new(),
                },
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![Predicate::Paused(true)],
                compensation: None,
                obligation: None,
                depends_on: Vec::new(),
            }],
        };
        let plan = StaticPlanner::default().prepare_laboratory(
            &snapshot,
            &intent,
            &context(&snapshot, 1),
        )?;
        let mut adapter = MemoryAdapter::new(snapshot);
        let prepare_context = context(adapter.snapshot(), 2);
        let failure = adapter
            .prepare(&plan, &prepare_context)
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "unsupported plan was accepted",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::AdapterRejected);
        Ok(())
    }

    #[test]
    fn stable_observations_require_distinct_authoritative_anchors() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = unpause_intent(
            &snapshot,
            10,
            Some(ObligationSpec {
                terminal: Predicate::Paused(false),
                failure: None,
                deadline_tick: GameTick(10),
                poll_interval_ticks: 1,
                stable_for_observations: 2,
            }),
        );
        let plan = StaticPlanner::default().prepare_laboratory(
            &snapshot,
            &intent,
            &context(&snapshot, 1),
        )?;
        let mut adapter = MemoryAdapter::new(snapshot);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;
        let committed = adapter.commit(&plan, &prepared, &context(adapter.snapshot(), 3))?;
        let action_id = committed.actions[0].action_id;

        let first = adapter.poll_action(action_id, &context(adapter.snapshot(), 4))?;
        assert_eq!(first.state, CommitState::AppliedAwaitingVerification);
        let repeated = adapter.poll_action(action_id, &context(adapter.snapshot(), 5))?;
        assert_eq!(repeated.state, CommitState::AppliedAwaitingVerification);

        adapter.advance_ticks(1)?;
        let first_due = adapter.poll_action(action_id, &context(adapter.snapshot(), 6))?;
        assert_eq!(first_due.state, CommitState::AppliedAwaitingVerification);
        adapter.advance_ticks(1)?;
        let distinct = adapter.poll_action(action_id, &context(adapter.snapshot(), 7))?;
        assert_eq!(distinct.state, CommitState::Verified);
        Ok(())
    }

    #[test]
    fn snapshot_injection_invalidates_prepared_work() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = unpause_intent(&snapshot, 11, None);
        let plan = StaticPlanner::default().prepare_laboratory(
            &snapshot,
            &intent,
            &context(&snapshot, 1),
        )?;
        let mut adapter = MemoryAdapter::new(snapshot);
        let prepared = adapter.prepare(&plan, &context(adapter.snapshot(), 2))?;

        let mut injected = adapter.snapshot().clone();
        injected.tick = injected
            .tick
            .checked_add(1)
            .ok_or_else(|| DfmcpError::new(ErrorCode::BudgetExceeded, "test tick overflow"))?;
        injected.cursor = injected
            .cursor
            .checked_next()
            .ok_or_else(|| DfmcpError::new(ErrorCode::CursorGap, "test cursor overflow"))?;
        injected.refresh_hash();
        adapter.inject_snapshot(injected)?;

        let failure = adapter
            .commit(&plan, &prepared, &context(adapter.snapshot(), 3))
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "pre-injection prepared work remained valid",
                )
            })?;
        assert_eq!(failure.code, ErrorCode::InvalidPlan);
        Ok(())
    }

    #[test]
    fn zero_tick_advance_is_rejected() {
        let mut adapter = MemoryAdapter::new(WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        ));
        assert!(
            matches!(adapter.advance_ticks(0), Err(ref error) if error.code == ErrorCode::InvalidRequest)
        );
    }

    #[test]
    fn commit_rejects_plan_content_changed_after_prepare() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = unpause_intent(&snapshot, 3, None);
        let plan = StaticPlanner::default().prepare_laboratory(
            &snapshot,
            &intent,
            &context(&snapshot, 1),
        )?;
        let mut adapter = MemoryAdapter::new(snapshot);
        let prepare_context = context(adapter.snapshot(), 2);
        let prepared = adapter.prepare(&plan, &prepare_context)?;
        let mut mutated = plan.clone();
        mutated.summary.push_str(" after prepare");
        let commit_context = context(adapter.snapshot(), 3);
        let error = adapter
            .commit(&mutated, &prepared, &commit_context)
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "mutated plan was committed",
                )
            })?;
        assert_eq!(error.code, ErrorCode::InvalidPlan);
        assert!(adapter.snapshot().paused);
        Ok(())
    }

    #[test]
    fn restore_starts_a_new_epoch_and_invalidates_action_handles() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let mut adapter = MemoryAdapter::new(snapshot.clone());
        let checkpoint_context = context(adapter.snapshot(), 1);
        let checkpoint = adapter.checkpoint("before-unpause", &checkpoint_context)?;
        let intent = unpause_intent(&snapshot, 4, None);
        let plan = StaticPlanner::default().prepare_laboratory(
            &snapshot,
            &intent,
            &context(&snapshot, 2),
        )?;
        let prepare_context = context(adapter.snapshot(), 3);
        let prepared = adapter.prepare(&plan, &prepare_context)?;
        let commit_context = context(adapter.snapshot(), 4);
        let committed = adapter.commit(&plan, &prepared, &commit_context)?;
        let action_id = committed.actions[0].action_id;
        let prior_epoch = adapter.snapshot().cursor.epoch;
        let restore_context = context(adapter.snapshot(), 5);
        let restored = adapter.restore(checkpoint.checkpoint_id, &restore_context)?;
        assert_eq!(restored.content_digest, checkpoint.content_digest);
        assert_eq!(
            adapter.snapshot().cursor.epoch,
            prior_epoch.saturating_add(1)
        );
        assert!(adapter.snapshot().paused);
        let poll_context = context(adapter.snapshot(), 6);
        let error = adapter
            .poll_action(action_id, &poll_context)
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "pre-restore action handle remained valid",
                )
            })?;
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        Ok(())
    }
}
