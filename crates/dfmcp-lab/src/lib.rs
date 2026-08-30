#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use dfmcp_adapter::{
    ActionReceipt, AdapterHealth, AdapterIdentity, CancelMode, CancelReceipt, CheckpointReceipt,
    CommitReceipt, CompatibilityLevel, GameAdapter, HealthStatus, ObservationFrame,
    ObservationPayload, ObservationRequest, PrepareReceipt, QueryRequest, QueryResponse, QueryRow,
    RestoreReceipt,
};
use dfmcp_core::{
    ActionId, Capability, CheckpointId, CommitState, DfmcpError, Digest32, ErrorCode, Evidence,
    EvidenceId, EvidenceKind, GameTick, OperationContext, PlanId, Result, RiskTier, StateAnchor,
    StepId,
};
use dfmcp_intent::{Action, PlanStep, PreparedPlan};
use dfmcp_world::{WorldSnapshot, evaluate, execute_query};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabEvent {
    Observed(StateAnchor),
    Prepared(PlanId),
    Committed(PlanId),
    ActionPolled(ActionId, CommitState),
    CancelRequested(ActionId, CancelMode),
    CancelFinalized(ActionId, CommitState),
    Checkpointed(CheckpointId),
    Restored(CheckpointId),
    SnapshotInjected(StateAnchor),
    TickAdvanced(GameTick),
}

#[derive(Clone, Debug)]
struct LabAction {
    plan_id: PlanId,
    step: PlanStep,
    receipt: ActionReceipt,
    stable_observations: u32,
    cancel_mode: Option<CancelMode>,
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
    transcript: Vec<LabEvent>,
    nonce: u128,
}

impl MemoryAdapter {
    #[must_use]
    pub fn new(snapshot: WorldSnapshot) -> Self {
        let capabilities = [
            Capability::Observe,
            Capability::Query,
            Capability::Plan,
            Capability::ControlClock,
            Capability::Checkpoint,
            Capability::Restore,
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
            transcript: Vec::new(),
            nonce: 1,
        }
    }

    #[must_use]
    pub const fn snapshot(&self) -> &WorldSnapshot {
        &self.snapshot
    }

    #[must_use]
    pub fn transcript(&self) -> &[LabEvent] {
        &self.transcript
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
        self.snapshot = snapshot;
        self.transcript
            .push(LabEvent::SnapshotInjected(self.snapshot.anchor()));
        Ok(())
    }

    pub fn advance_ticks(&mut self, amount: u64) {
        self.snapshot.tick = self.snapshot.tick.saturating_add(amount);
        self.snapshot.cursor = self.snapshot.cursor.next();
        self.snapshot.refresh_hash();
        self.transcript
            .push(LabEvent::TickAdvanced(self.snapshot.tick));
    }

    fn next_nonce(&mut self) -> u128 {
        let value = self.nonce;
        self.nonce = self.nonce.saturating_add(1);
        value
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

    fn internal_checkpoint(&mut self, label: &str) -> CheckpointReceipt {
        let nonce = self.next_nonce();
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"dfmcp-lab-checkpoint-v1");
        bytes.extend_from_slice(self.snapshot.state_hash.as_bytes());
        bytes.extend_from_slice(&nonce.to_be_bytes());
        let digest = Digest32::of_bytes(&bytes);
        let checkpoint_id = CheckpointId::new(nonzero(digest.first_u128()));
        self.checkpoints
            .insert(checkpoint_id, self.snapshot.clone());
        let evidence = evidence(
            self.snapshot.anchor(),
            EvidenceKind::Checkpoint,
            &format!("laboratory checkpoint {label}"),
        );
        self.transcript.push(LabEvent::Checkpointed(checkpoint_id));
        CheckpointReceipt {
            checkpoint_id,
            label: label.to_owned(),
            anchor: self.snapshot.anchor(),
            content_digest: self.snapshot.state_hash,
            durable: true,
            evidence: vec![evidence],
        }
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
        let state = if self.dependencies_verified(plan_id, step) {
            apply_action(&mut self.snapshot, &step.action)?;
            if step
                .postconditions
                .iter()
                .all(|predicate| evaluate(&self.snapshot, predicate))
                && step.obligation.is_none()
            {
                CommitState::Verified
            } else {
                CommitState::AppliedAwaitingVerification
            }
        } else {
            CommitState::Prepared
        };
        let summary = match state {
            CommitState::Prepared => "waiting for dependency verification",
            CommitState::Verified => "semantic postconditions verified",
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
                stable_observations: 0,
                cancel_mode: None,
            },
        );
        Ok(receipt)
    }

    fn refresh_action(&mut self, action_id: ActionId) -> Result<ActionReceipt> {
        let (plan_id, step, prior_state, stable) = self
            .actions
            .get(&action_id)
            .map(|action| {
                (
                    action.plan_id,
                    action.step.clone(),
                    action.receipt.state,
                    action.stable_observations,
                )
            })
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unknown action {action_id}"),
                )
            })?;

        let dependencies_verified = self.dependencies_verified(plan_id, &step);
        if prior_state == CommitState::Prepared && dependencies_verified {
            apply_action(&mut self.snapshot, &step.action)?;
        }

        let mut state = prior_state;
        let mut stable_observations = stable;
        if matches!(
            state,
            CommitState::Prepared | CommitState::AppliedAwaitingVerification
        ) && dependencies_verified
        {
            if let Some(obligation) = &step.obligation {
                let failure_triggered = obligation
                    .failure
                    .as_ref()
                    .is_some_and(|predicate| evaluate(&self.snapshot, predicate));
                if failure_triggered || self.snapshot.tick > obligation.deadline_tick {
                    state = CommitState::Failed;
                } else if evaluate(&self.snapshot, &obligation.terminal)
                    && step
                        .postconditions
                        .iter()
                        .all(|predicate| evaluate(&self.snapshot, predicate))
                {
                    stable_observations = stable_observations.saturating_add(1);
                    if stable_observations >= obligation.stable_for_observations {
                        state = CommitState::Verified;
                    } else {
                        state = CommitState::AppliedAwaitingVerification;
                    }
                } else {
                    stable_observations = 0;
                    state = CommitState::AppliedAwaitingVerification;
                }
            } else if step
                .postconditions
                .iter()
                .all(|predicate| evaluate(&self.snapshot, predicate))
            {
                state = CommitState::Verified;
            } else {
                state = CommitState::AppliedAwaitingVerification;
            }
        }

        let message = if state == CommitState::Verified {
            "semantic postconditions verified"
        } else if state == CommitState::Failed {
            "obligation failed or exceeded its game-tick deadline"
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
            action.stable_observations = stable_observations;
        }
        self.transcript
            .push(LabEvent::ActionPolled(action_id, state));
        Ok(receipt)
    }
}

impl GameAdapter for MemoryAdapter {
    fn identity(&self) -> AdapterIdentity {
        self.identity.clone()
    }

    fn health(&mut self, context: &OperationContext) -> Result<AdapterHealth> {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
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
        let payload = match request.since {
            None => ObservationPayload::Snapshot(self.snapshot.clone()),
            Some(cursor) if cursor == self.snapshot.cursor => {
                ObservationPayload::Heartbeat(self.snapshot.anchor())
            }
            Some(cursor)
                if cursor.epoch == self.snapshot.cursor.epoch
                    && cursor.sequence < self.snapshot.cursor.sequence =>
            {
                ObservationPayload::Snapshot(self.snapshot.clone())
            }
            Some(_) => {
                return Err(DfmcpError::new(
                    ErrorCode::CursorGap,
                    "observation cursor is not resumable",
                )
                .retryable(true));
            }
        };
        self.transcript
            .push(LabEvent::Observed(self.snapshot.anchor()));
        Ok(ObservationFrame {
            payload,
            evidence: vec![evidence(
                self.snapshot.anchor(),
                EvidenceKind::Observation,
                "deterministic laboratory observation",
            )],
            warnings: if request
                .since
                .is_some_and(|cursor| cursor != self.snapshot.cursor)
            {
                vec!["delta history unavailable; returned a full snapshot".to_owned()]
            } else {
                Vec::new()
            },
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
        if request.continuation.is_some() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "laboratory adapter has no outstanding query continuation",
            ));
        }
        if request.max_output_tokens == 0
            || request.max_output_tokens > context.budget.max_output_tokens
        {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "query output-token budget is invalid",
            ));
        }
        let result = execute_query(&self.snapshot, &request.query, context.budget.max_entities)?;
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
            continuation: None,
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
        if self.snapshot.tick > plan.expires_at_tick {
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
        for step in &plan.steps {
            self.authorize_step(step, context)?;
            if !step
                .preconditions
                .iter()
                .all(|predicate| evaluate(&self.snapshot, predicate))
            {
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
        let nonce = self.next_nonce();
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
        self.transcript.push(LabEvent::Prepared(plan.id));
        Ok(receipt)
    }

    fn commit(
        &mut self,
        plan: &PreparedPlan,
        prepared: &PrepareReceipt,
        context: &OperationContext,
    ) -> Result<CommitReceipt> {
        self.check_anchor(context.anchor)?;
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
        if let Some(existing) = self.commits.get(&plan.id) {
            if existing.plan_digest == plan.digest {
                return Ok(existing.clone());
            }
            return Err(DfmcpError::new(
                ErrorCode::Conflict,
                "plan identifier was reused with a different digest",
            ));
        }
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
        if self.snapshot.tick > prepared.expires_at_tick
            || self.snapshot.tick > plan.expires_at_tick
        {
            return Err(DfmcpError::new(
                ErrorCode::InvalidPlan,
                "prepared plan expired before commit",
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
            if !step
                .preconditions
                .iter()
                .all(|predicate| evaluate(&self.snapshot, predicate))
            {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!("step {} failed commit-time revalidation", step.id),
                ));
            }
        }
        if plan.requires_checkpoint {
            context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
        }

        let checkpoint = if plan.requires_checkpoint {
            Some(self.internal_checkpoint(&format!("before-plan-{}", plan.id)))
        } else {
            None
        };
        let mut actions = Vec::with_capacity(plan.steps.len());
        for step in &plan.steps {
            let action_id = derived_action_id(plan.id, step.id);
            if let Some(existing) = self.actions.get(&action_id) {
                actions.push(existing.receipt.clone());
            } else {
                actions.push(self.dispatch_step(plan.id, step, action_id)?);
            }
        }
        self.transcript.push(LabEvent::Committed(plan.id));
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
    }

    fn poll_action(
        &mut self,
        action_id: ActionId,
        context: &OperationContext,
    ) -> Result<ActionReceipt> {
        self.check_anchor(context.anchor)?;
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        self.refresh_action(action_id)
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

        let state = if action.receipt.state.is_terminal() {
            action.receipt.state
        } else {
            CommitState::CancelRequested
        };
        if state == CommitState::CancelRequested
            && mode == CancelMode::EmergencyPauseAndDrain
            && !self.snapshot.paused
        {
            apply_action(&mut self.snapshot, &Action::Pause { paused: true })?;
        }
        if let Some(stored) = self.actions.get_mut(&action_id) {
            stored.cancel_mode = if state == CommitState::CancelRequested {
                Some(mode)
            } else {
                None
            };
        }
        let message = if state == CommitState::CancelRequested {
            "cancellation request recorded; drain remains pending"
        } else {
            "action was already terminal when cancellation was requested"
        };
        self.stored_action_receipt(action_id, state, EvidenceKind::AdapterReceipt, message)?;
        self.transcript
            .push(LabEvent::CancelRequested(action_id, mode));
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
        if state != CommitState::CancelRequested && !state.is_terminal() {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "cancellation must be requested before it can be finalized",
            ));
        }
        let mut compensation_action = None;
        if state == CommitState::CancelRequested {
            if action.cancel_mode == Some(CancelMode::CompensateReversible) {
                if let Some(compensation) = &action.step.compensation {
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
                    apply_action(&mut self.snapshot, compensation)?;
                    compensation_action = Some(derived_compensation_id(action_id));
                    state = CommitState::Compensated;
                } else {
                    state = CommitState::Cancelled;
                }
            } else {
                state = CommitState::Cancelled;
            }
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
        self.transcript
            .push(LabEvent::CancelFinalized(action_id, state));
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
    }

    fn checkpoint(&mut self, label: &str, context: &OperationContext) -> Result<CheckpointReceipt> {
        self.check_anchor(context.anchor)?;
        context.authorize(Capability::Checkpoint, RiskTier::Guarded, &[], None)?;
        if label.is_empty() || label.len() > 256 {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "checkpoint label is empty or too long",
            ));
        }
        Ok(self.internal_checkpoint(label))
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
        self.snapshot = checkpoint;
        self.snapshot.cursor = prior_anchor.cursor.reset_epoch();
        self.snapshot.refresh_hash();
        self.prepared.clear();
        self.plans.clear();
        self.actions.clear();
        self.action_by_step.clear();
        self.commits.clear();
        self.transcript.push(LabEvent::Restored(checkpoint_id));
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

fn action_is_supported(action: &Action) -> bool {
    matches!(action, Action::Pause { .. })
}

fn apply_action(snapshot: &mut WorldSnapshot, action: &Action) -> Result<()> {
    match action {
        Action::Pause { paused } => {
            if snapshot.paused != *paused {
                snapshot.paused = *paused;
                snapshot.cursor = snapshot.cursor.next();
                snapshot.refresh_hash();
            }
            Ok(())
        }
        _ => Err(DfmcpError::new(
            ErrorCode::AdapterRejected,
            "laboratory adapter received an unsupported semantic action",
        )),
    }
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
        Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode, FortressId, GameTick,
        IntentId, MapCoord, MapCuboid, ObservationCursor, OperationContext, RequestId, RiskTier,
        SessionId, WorkBudget,
    };
    use dfmcp_intent::{
        Action, Constraint, DigMode, Intent, ObligationSpec, RequestedAction, StaticPlanner,
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

    #[test]
    fn duplicate_commit_does_not_duplicate_effect() -> Result<(), DfmcpError> {
        let snapshot = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        );
        let intent = Intent {
            id: IntentId::new(1),
            anchor: snapshot.anchor(),
            summary: "unpause".to_owned(),
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
        let planner = StaticPlanner::default();
        let plan = planner.prepare(&snapshot, &intent, &context(&snapshot, 1))?;
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
    fn laboratory_identity_does_not_advertise_unimplemented_effects() {
        let adapter = MemoryAdapter::new(WorldSnapshot::new(
            FortressId::new(1),
            GameTick(1),
            ObservationCursor::ORIGIN,
            true,
            WorldGraph::default(),
        ));
        let identity = adapter.identity();
        assert!(identity.capabilities.contains(&Capability::ControlClock));
        assert!(!identity.capabilities.contains(&Capability::Designate));
        assert!(!identity.capabilities.contains(&Capability::Construct));
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
        let area = MapCuboid::new(MapCoord { x: 1, y: 1, z: 0 }, MapCoord { x: 2, y: 2, z: 0 })?;
        let intent = Intent {
            id: IntentId::new(9),
            anchor: snapshot.anchor(),
            summary: "designate a tiny test excavation".to_owned(),
            terminal_condition: Predicate::Paused(false),
            constraints: vec![Constraint::MaxRisk(RiskTier::Guarded)],
            requested_actions: vec![RequestedAction {
                action: Action::DesignateDig {
                    area,
                    mode: DigMode::Mine,
                },
                preconditions: vec![Predicate::Paused(true)],
                postconditions: vec![Predicate::True],
                compensation: None,
                obligation: Some(ObligationSpec {
                    terminal: Predicate::True,
                    failure: None,
                    deadline_tick: GameTick(20),
                    poll_interval_ticks: 1,
                    stable_for_observations: 1,
                }),
                depends_on: Vec::new(),
            }],
        };
        let plan = StaticPlanner::default().prepare(&snapshot, &intent, &context(&snapshot, 1))?;
        let mut adapter = MemoryAdapter::new(snapshot);
        let prepare_context = context(adapter.snapshot(), 2);
        let error = adapter
            .prepare(&plan, &prepare_context)
            .err()
            .ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::InternalInvariantViolation,
                    "unsupported plan was accepted",
                )
            })?;
        assert_eq!(error.code, ErrorCode::AdapterRejected);
        Ok(())
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
        let intent = Intent {
            id: IntentId::new(3),
            anchor: snapshot.anchor(),
            summary: "unpause".to_owned(),
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
        let plan = StaticPlanner::default().prepare(&snapshot, &intent, &context(&snapshot, 1))?;
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
        let intent = Intent {
            id: IntentId::new(4),
            anchor: snapshot.anchor(),
            summary: "unpause".to_owned(),
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
        let plan = StaticPlanner::default().prepare(&snapshot, &intent, &context(&snapshot, 2))?;
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
