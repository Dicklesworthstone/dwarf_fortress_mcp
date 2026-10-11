//! Original goals outlive their action receipts and their originating session.
//!
//! Current truth, immutable first achievement and restore abandonment are
//! independent facts. This book retains the exact sealed plan so recovery and
//! capacity admission inspect every original effect identity without granting
//! authority to dispatch it again.

use super::*;
use dfmcp_lab::durable::{DurableLabStore, DurableObjective, MAX_OBJECTIVES_PER_FORTRESS};
use serde_json::Value;

#[derive(Clone, Debug)]
pub(super) struct Objective {
    pub(super) plan_digest: Digest32,
    source: PlanSource,
    owner_session_id: SessionId,
    /// Session counters can repeat in another process. Numeric equality with
    /// recovered historical ownership does not identify the current caller.
    originating_process: bool,
    sealed_state_hash: Digest32,
    sealed_anchor: Option<StateAnchor>,
    plan: Option<PreparedPlan>,
    /// Live adapter handles can still own deferred dispatch. Recovery and
    /// restore never reconstruct that authority from the retained plan.
    original_handles_live: bool,
    first_satisfied_anchor: Option<StateAnchor>,
    historical_proof_verified: bool,
    restore_abandoned_anchor: Option<StateAnchor>,
    verification_error: Option<String>,
}

impl Objective {
    pub(super) fn committed(
        plan: &PreparedPlan,
        source: PlanSource,
        owner_session_id: SessionId,
    ) -> Self {
        Self {
            plan_digest: plan.digest,
            source,
            owner_session_id,
            originating_process: true,
            sealed_state_hash: plan.anchor.state_hash,
            sealed_anchor: Some(plan.anchor),
            plan: Some(plan.clone()),
            original_handles_live: true,
            first_satisfied_anchor: None,
            historical_proof_verified: false,
            restore_abandoned_anchor: None,
            verification_error: None,
        }
    }

    fn abandoned(&self, session: &LabSession) -> bool {
        self.restore_abandoned_anchor.is_some()
            || session.objective_restore.contains(&self.plan_digest)
    }

    fn current_truth(&self, session: &LabSession) -> Result<PredicateTruth> {
        let ctx = context_for(session, session.next_request_id);
        authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
        if self.abandoned(session) {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "restore abandoned pursuit of this original goal; its historical proof does not authorize a new pursuit",
            ));
        }
        if let Some(reason) = &self.verification_error {
            return Err(DfmcpError::new(ErrorCode::CorruptLedger, reason.clone()));
        }
        let plan = self.plan.as_ref().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CorruptLedger,
                "the original sealed goal is unavailable",
            )
        })?;
        PredicateEvidence::laboratory(session.adapter.snapshot())?
            .evaluate(&plan.terminal_condition)
    }

    fn projection(&self, session: &LabSession) -> (PredicateTruth, Value) {
        if let Err(error) = authorize_goal_observation(session) {
            return (PredicateTruth::Unknown, unavailable_goal_history(error));
        }
        let observed = self.current_truth(session);
        let truth = observed
            .as_ref()
            .copied()
            .unwrap_or(PredicateTruth::Unknown);
        let mut result = crate::observation_projection::laboratory_objective_evidence(observed);
        let abandoned = self.abandoned(session);
        let historical = self
            .first_satisfied_anchor
            .filter(|_| self.historical_proof_verified);
        let quiet = goal_work_quiescent(session, self).ok();
        result["plan_digest"] = json!(self.plan_digest.to_hex());
        result["summary"] = json!(source_summary(&self.source));
        result["original_source"] = source_json(&self.source);
        result["continuation"] = self.source.continuation_json();
        result["owner_session_id"] = json!(self.owner_session_id.to_string());
        result["owned_by_current_session"] =
            json!(self.originating_process && self.owner_session_id == session.session_id);
        result["sealed_anchor"] = self
            .sealed_anchor
            .as_ref()
            .map(anchor_json)
            .unwrap_or(Value::Null);
        result["terminal_condition"] = self
            .plan
            .as_ref()
            .map(|plan| crate::lab_world::predicate_json(&plan.terminal_condition))
            .unwrap_or(Value::Null);
        result["committed_tick"] = json!(self.sealed_anchor.map(|anchor| anchor.tick.0));
        result["observed_anchor"] = anchor_json(&session.adapter.snapshot().anchor());
        result["first_satisfied_anchor"] =
            historical.as_ref().map(anchor_json).unwrap_or(Value::Null);
        result["historical_achievement"] = json!(if historical.is_some() {
            "verified"
        } else if self.first_satisfied_anchor.is_some() {
            "unverifiable"
        } else {
            "not_recorded"
        });
        result["recorded_first_satisfied_anchor"] = self
            .first_satisfied_anchor
            .as_ref()
            .map(anchor_json)
            .unwrap_or(Value::Null);
        result["achieved_tick"] = json!(historical.map(|anchor| anchor.tick.0));
        result["restore_abandoned_anchor"] = self
            .restore_abandoned_anchor
            .as_ref()
            .map(anchor_json)
            .unwrap_or(Value::Null);
        result["abandonment_publication_pending"] =
            json!(session.objective_restore.contains(&self.plan_digest));
        result["physical_quiescent"] = json!(quiet);
        result["pursuit_active"] =
            json!(!abandoned && truth != PredicateTruth::True && quiet != Some(true));
        result["needs_replan"] =
            json!(!abandoned && truth == PredicateTruth::False && quiet == Some(true));
        result["replacement_work_dispatched"] = json!(false);
        result["blind_retry_allowed"] = json!(false);
        if !abandoned
            && truth == PredicateTruth::False
            && quiet == Some(true)
            && matches!(
                &self.source,
                PlanSource::Production { .. } | PlanSource::ProductionContinuation { .. }
            )
        {
            // An affordance carries no new authority. Reuse the exact intake
            // gate so active/unknown sibling pursuits are not offered duplicate
            // work; commit repeats the check even if this proposal was possible.
            match continuation_source(session, self.plan_digest, None, None) {
                Ok(_) => {
                    result["continuation_request"] = goal_continuation::request_json(
                        &session.session_id.to_string(),
                        self.plan_digest,
                    );
                }
                Err(error) => {
                    result["continuation_refused"] =
                        json!({"code": error.code.as_str(), "message": error.message});
                }
            }
        }
        if abandoned {
            result["status"] = json!("abandoned");
        } else if self.verification_error.is_some() {
            result["status"] = json!("indeterminate");
        } else if historical.is_some() && truth == PredicateTruth::False {
            result["status"] = json!("no_longer_holds");
        }
        (truth, result)
    }
}

fn authorize_goal_observation(session: &LabSession) -> Result<()> {
    authorize_entry(
        &context_for(session, session.next_request_id),
        Capability::Observe,
        RiskTier::ReadOnly,
    )
}

fn unavailable_goal_history(error: DfmcpError) -> Value {
    let mut result = crate::observation_projection::laboratory_objective_evidence(Err(error));
    result["status"] = json!("unavailable");
    result["coverage"] = json!("original goal history requires current Observe authority");
    result
}

fn source_summary(source: &PlanSource) -> &str {
    match source {
        PlanSource::Pause { summary, .. }
        | PlanSource::Actions { summary, .. }
        | PlanSource::Blueprint { summary, .. }
        | PlanSource::Production { summary, .. }
        | PlanSource::ProductionContinuation { summary, .. } => summary,
    }
}

fn source_json(source: &PlanSource) -> Value {
    match source {
        PlanSource::Pause { paused_target, .. } => {
            json!({"kind": "pause", "paused_target": paused_target})
        }
        PlanSource::Actions { raw, .. }
        | PlanSource::Blueprint { raw, .. }
        | PlanSource::Production { raw, .. }
        | PlanSource::ProductionContinuation { raw, .. } => json!({
            "kind": match source {
                PlanSource::Actions { .. } => "actions",
                PlanSource::Blueprint { .. } => "blueprint",
                PlanSource::ProductionContinuation { .. } => "production_continuation",
                _ => "production",
            },
            "request": serde_json::from_str::<Value>(raw).unwrap_or_else(|_| Value::String(raw.clone())),
        }),
    }
}

/// Reconstruct the complete intent for a new pursuit. This is a current
/// authority/evidence gate only: archived objective verification calls the pure
/// source compiler instead and never consults the current mutable goal book.
pub(super) fn continuation_source(
    session: &LabSession,
    parent: Digest32,
    summary: Option<String>,
    exclude_candidate: Option<Digest32>,
) -> Result<PlanSource> {
    let context = context_for(session, session.next_request_id);
    authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
    authorize_entry(&context, Capability::Plan, RiskTier::ReadOnly)?;
    if session.objectives.len() > MAX_OBJECTIVES_PER_FORTRESS
        || session.objectives.len() > context.budget.max_entities as usize
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "continuation lineage inspection exceeds its bounded goal domain",
        ));
    }
    let objective = session.objectives.iter().find(|goal| goal.plan_digest == parent)
        .ok_or_else(|| DfmcpError::new(ErrorCode::PreconditionsFailed,
            "the original production goal is not retained; no replacement request can be inferred from action receipts"))?;
    if objective.abandoned(session) {
        return Err(DfmcpError::new(
            ErrorCode::PreconditionsFailed,
            "restore abandoned pursuit of this original goal; explicitly request a new goal in the current epoch",
        ));
    }
    if let Some(reason) = &objective.verification_error {
        return Err(DfmcpError::new(ErrorCode::CorruptLedger, reason.clone()));
    }
    let original = objective.plan.as_ref().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::CorruptLedger,
            "the original sealed goal is unavailable",
        )
    })?;
    let continuation = match &objective.source {
        PlanSource::Production { raw, .. } => goal_continuation::ProductionContinuation::new(
            parent,
            parent,
            crate::lab_world::ProductionRequest::parse(raw)?,
        )?,
        PlanSource::ProductionContinuation { raw, .. } => {
            goal_continuation::ProductionContinuation::parse(raw)?.next(parent)?
        }
        _ => {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "continue_goal supports retained production objectives; legacy action, pause and room sources contain no inferred production quotas",
            ));
        }
    };
    let quiet = goal_work_quiescent(session, objective)?;
    goal_continuation::validate_goal_evidence(
        original,
        session.adapter.snapshot(),
        &context,
        quiet,
    )?;
    for other in &session.objectives {
        if other.plan_digest == parent || Some(other.plan_digest) == exclude_candidate {
            continue;
        }
        let root = match &other.source {
            PlanSource::Production { .. } => Some(other.plan_digest),
            PlanSource::ProductionContinuation { raw, .. } => {
                Some(goal_continuation::ProductionContinuation::parse(raw)?.root)
            }
            _ => None,
        };
        if root == Some(continuation.root) && !goal_work_quiescent(session, other)? {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                format!(
                    "original goal lineage still has unresolved work in plan {}; observe or explicitly drain that pursuit before continuing",
                    other.plan_digest.to_hex()
                ),
            ));
        }
    }
    Ok(PlanSource::ProductionContinuation {
        summary: summary.map_or_else(
            || source_summary(&objective.source).to_owned(),
            |summary| summary,
        ),
        raw: continuation.canonical_json(),
    })
}

/// A proposal can become unsafe without changing its own original action IDs.
/// Check the parent and every retained same-root pursuit again before commit.
/// The identical candidate may already have been durably admitted by a failed
/// commit; excluding only its exact digest preserves ordinary idempotent retry.
pub(super) fn validate_continuation(
    session: &LabSession,
    source: &PlanSource,
    exclude_candidate: Option<Digest32>,
) -> Result<()> {
    let Some(continuation) = source.continuation()? else {
        return Ok(());
    };
    let current = continuation_source(
        session,
        continuation.parent,
        Some(source_summary(source).to_owned()),
        exclude_candidate,
    )?;
    if !current
        .continuation()?
        .is_some_and(|expected| continuation.same_original_request(&expected))
    {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "continuation source no longer matches its retained original request and lineage",
        ));
    }
    Ok(())
}

pub(super) fn original_goal_observation(
    session: &LabSession,
    actual_plan_digest: &str,
) -> Result<(PredicateTruth, Value)> {
    let ctx = context_for(session, session.next_request_id);
    authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
    let digest = Digest32::from_hex(actual_plan_digest).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "original goal requires its sealed plan digest",
        )
    })?;
    session.objectives.iter().find(|objective| objective.plan_digest == digest)
        .map(|objective| objective.projection(session))
        .ok_or_else(|| DfmcpError::new(
            ErrorCode::PreconditionsFailed,
            "the original goal for this committed plan is not retained; action completion cannot establish it",
        ))
}

pub(super) fn objectives_json(session: &LabSession) -> Value {
    if let Err(error) = authorize_goal_observation(session) {
        // One coverage sentinel reveals neither original requests nor how
        // many private/shared historical goals the caller cannot observe.
        return json!([unavailable_goal_history(error)]);
    }
    json!(
        session
            .objectives
            .iter()
            .map(|objective| objective.projection(session).1)
            .collect::<Vec<_>>()
    )
}

/// Every original step must be quiet now. The absence of an unfinished commit
/// never proves this: retired and recovered plans still inspect their exact
/// registered physical identities. Live deferred steps cannot be forgotten.
fn goal_work_quiescent(session: &LabSession, objective: &Objective) -> Result<bool> {
    let ctx = context_for(session, session.next_request_id);
    authorize_entry(&ctx, Capability::Observe, RiskTier::ReadOnly)?;
    if objective.verification_error.is_some() {
        return Err(DfmcpError::new(
            ErrorCode::CorruptLedger,
            "original effect identities are unverifiable",
        ));
    }
    let plan = objective.plan.as_ref().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::CorruptLedger,
            "original effect identities are unavailable",
        )
    })?;
    for step in &plan.steps {
        if let Some(receipt) = session.adapter.step_receipt(plan.id, step.id) {
            if !action_fully_drained(&session.adapter, receipt.action_id) {
                return Ok(false);
            }
            continue;
        }
        if objective.original_handles_live {
            return Ok(false);
        }
        if session.carried.iter().any(|carried| {
            carried.plan_digest == plan.digest
                && carried.step == Some(step.id)
                && carried.is_final()
                && carried.work_state == EffectWorkState::NeverDispatched
        }) {
            continue;
        }
        if !dfmcp_intent::inspect_effect_work(
            session.adapter.snapshot(),
            &step.action,
            &step.idempotency_key,
            true,
        )?
        .is_quiescent()
        {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Select removable history before reservations, durable admission or effects.
/// Unmet, unknown and unverified history never yields capacity.
pub(super) fn objective_evictions(session: &LabSession, digest: Digest32) -> Result<Vec<Digest32>> {
    if session
        .objectives
        .iter()
        .any(|objective| objective.plan_digest == digest)
        || session.objectives.len() < MAX_OBJECTIVES_PER_FORTRESS
    {
        return Ok(Vec::new());
    }
    let required = session.objectives.len() + 1 - MAX_OBJECTIVES_PER_FORTRESS;
    let mut evicted = Vec::new();
    for objective in &session.objectives {
        if !objective.historical_proof_verified
            || objective.first_satisfied_anchor.is_none()
            || !matches!(objective.current_truth(session), Ok(PredicateTruth::True))
            || !goal_work_quiescent(session, objective).unwrap_or(false)
        {
            continue;
        }
        if session.durable_scenario.is_some()
            && with_durable_store(|store| {
                Ok(store
                    .commit(session.fortress_id, objective.plan_digest)
                    .is_some())
            })?
        {
            continue;
        }
        evicted.push(objective.plan_digest);
        if evicted.len() == required {
            return Ok(evicted);
        }
    }
    Err(DfmcpError::new(
        ErrorCode::BudgetExceeded,
        "the fortress original-goal book is full; unresolved goals or original work cannot be discarded to admit another effect",
    ))
}

pub(super) fn install_objective(
    session: &mut LabSession,
    plan: &PreparedPlan,
    source: PlanSource,
    evicted: &[Digest32],
) {
    session
        .objectives
        .retain(|objective| !evicted.contains(&objective.plan_digest));
    if !session
        .objectives
        .iter()
        .any(|objective| objective.plan_digest == plan.digest)
    {
        session
            .objectives
            .push(Objective::committed(plan, source, session.session_id));
    }
}

/// Gather claims before publishing a durable frontier. No historical anchor is
/// changed until its exact snapshot and goal update were saved together.
pub(super) fn newly_satisfied_objectives(session: &LabSession) -> Vec<Digest32> {
    session
        .objectives
        .iter()
        .filter(|objective| {
            objective.first_satisfied_anchor.is_none()
                && matches!(objective.current_truth(session), Ok(PredicateTruth::True))
        })
        .map(|objective| objective.plan_digest)
        .collect()
}

pub(super) fn record_objective_progress(
    session: &mut LabSession,
    anchor: StateAnchor,
    satisfied: &[Digest32],
    abandoned: &[Digest32],
) {
    for objective in &mut session.objectives {
        if abandoned.contains(&objective.plan_digest)
            && objective.restore_abandoned_anchor.is_none()
        {
            objective.restore_abandoned_anchor = Some(anchor);
        }
        if satisfied.contains(&objective.plan_digest)
            && objective.first_satisfied_anchor.is_none()
            && objective.restore_abandoned_anchor.is_none()
        {
            objective.first_satisfied_anchor = Some(anchor);
            objective.historical_proof_verified = true;
        }
    }
    for digest in abandoned {
        session.objective_restore.remove(digest);
    }
}

pub(super) fn stage_objective_abandonment(session: &mut LabSession) {
    for objective in &mut session.objectives {
        objective.original_handles_live = false;
    }
    session.objective_restore.extend(
        session
            .objectives
            .iter()
            .filter(|objective| objective.restore_abandoned_anchor.is_none())
            .map(|objective| objective.plan_digest),
    );
}

pub(super) fn objective_history_roots(session: &LabSession) -> impl Iterator<Item = Digest32> + '_ {
    session.objectives.iter().flat_map(|objective| {
        std::iter::once(objective.sealed_state_hash)
            .chain(
                objective
                    .first_satisfied_anchor
                    .map(|anchor| anchor.state_hash),
            )
            .chain(
                objective
                    .restore_abandoned_anchor
                    .map(|anchor| anchor.state_hash),
            )
    })
}

/// Recompile only against the exact archived seal. Recovered plans are proof
/// and identity material; no action handle or dispatch authority is restored.
pub(super) fn recover_objective(store: &DurableLabStore, retained: &DurableObjective) -> Objective {
    let source = PlanSource::from_durable(&retained.source);
    let mut objective = Objective {
        plan_digest: retained.plan_digest,
        source,
        owner_session_id: retained.owner_session_id,
        originating_process: false,
        sealed_state_hash: retained.sealed_state_hash,
        sealed_anchor: None,
        plan: None,
        original_handles_live: false,
        first_satisfied_anchor: retained.first_satisfied_anchor,
        historical_proof_verified: false,
        restore_abandoned_anchor: retained.restore_abandoned_anchor,
        verification_error: None,
    };
    let checked = (|| -> Result<()> {
        let sealed = store.load_snapshot(retained.sealed_state_hash)?;
        if sealed.fortress_id != retained.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "the original goal seal belongs to a different fortress",
            ));
        }
        objective.sealed_anchor = Some(sealed.anchor());
        // This pure planning grant checks canonical reconstruction. It grants
        // no current session the right to observe, commit or resume an action.
        let ctx = OperationContext {
            session_id: SessionId::new(u128::MAX),
            request_id: RequestId::new(retained.intent_id),
            anchor: sealed.anchor(),
            budget: MAX_LAB_BUDGET,
            grants: vec![CapabilityGrant {
                capability: Capability::Plan,
                scope: CapabilityScope {
                    fortress_id: Some(retained.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        };
        let intent = objective
            .source
            .intent(IntentId::new(retained.intent_id), &sealed)?;
        let plan = StaticPlanner::default().prepare_laboratory(&sealed, &intent, &ctx)?;
        if plan.digest != retained.plan_digest {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "the original goal source does not reproduce its sealed plan digest",
            ));
        }
        if let Some(anchor) = retained.first_satisfied_anchor {
            let proof = store.load_snapshot(anchor.state_hash)?;
            if proof.anchor() != anchor
                || anchor.fortress_id != retained.fortress_id
                || !PredicateEvidence::laboratory(&proof)?.establishes(&plan.terminal_condition)?
            {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "the retained first goal achievement does not prove its original predicate at that exact anchor",
                ));
            }
            objective.historical_proof_verified = true;
        }
        if let Some(anchor) = retained.restore_abandoned_anchor {
            let abandoned = store.load_snapshot(anchor.state_hash)?;
            if abandoned.anchor() != anchor || anchor.fortress_id != retained.fortress_id {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "the retained goal abandonment has no exact fortress anchor",
                ));
            }
        }
        objective.plan = Some(plan);
        Ok(())
    })();
    if let Err(error) = checked {
        objective.verification_error = Some(format!("{}: {}", error.code.as_str(), error.message));
    }
    objective
}
