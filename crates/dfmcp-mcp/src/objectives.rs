//! Original goals outlive their action receipts and their originating session.
//!
//! Current truth, immutable first achievement and restore abandonment are
//! independent facts. This book retains the exact sealed plan so recovery and
//! capacity admission inspect every original effect identity without granting
//! authority to dispatch it again.

use super::*;
use dfmcp_lab::durable::{DurableLabStore, DurableObjective, MAX_OBJECTIVES_PER_FORTRESS};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ResourceReserveStatus {
    WorkRequired,
    FundedFromCurrentStock,
    Expired,
}

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
    /// Exact original source custody for semantic production output checks.
    /// Reconstructed from the archived seal; never a new durable wire field.
    resource_output_ledger: Option<crate::lab_world::ProductionGoalLedger>,
    /// Current process-local continuity, separate from the immutable plan seal.
    /// Only live commit or exact durable recovery can establish this frontier.
    active_observation_anchor: Option<StateAnchor>,
    /// Live adapter handles can still own deferred dispatch. Recovery and
    /// restore never reconstruct that authority from the retained plan.
    original_handles_live: bool,
    /// Positive retired-step custody, verified against the original seal and
    /// exact archived frontier. A missing entity never populates this map.
    retired_never_dispatched: BTreeMap<u32, goal_custody::RetiredNondispatchProof>,
    first_satisfied_anchor: Option<StateAnchor>,
    historical_proof_verified: bool,
    restore_abandoned_anchor: Option<StateAnchor>,
    verification_error: Option<String>,
}

impl Objective {
    /// Resource inspection may export only the exact, verified retained plan.
    /// It never reconstructs missing custody or evaluates a current predicate.
    pub(super) fn resource_plan(&self) -> Option<&PreparedPlan> {
        self.verification_error
            .is_none()
            .then_some(self.plan.as_ref())
            .flatten()
    }

    pub(super) fn resource_source_kind(&self) -> &'static str {
        super::plan_resources::source_kind(&self.source)
    }

    pub(super) fn is_resource_acquisition(&self) -> bool {
        self.source.is_resource_acquisition()
    }

    pub(super) fn is_resource_aware_production(&self) -> bool {
        self.source.is_resource_aware_production()
    }

    pub(super) fn is_resource_aware_room(&self) -> bool {
        self.source.is_resource_aware_room()
    }

    fn current_resource_room_truth(&self, session: &LabSession) -> Result<PredicateTruth> {
        let context = context_for(session, session.next_request_id);
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        query_budget::admit_sources(&[session.adapter.snapshot()], &context)?;
        if !self.is_resource_aware_room()
            || self.abandoned(session)
            || self.verification_error.is_some()
            || self.plan.is_none()
        {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "the original resource-aware room has no verified active project custody",
            ));
        }
        self.source
            .resource_room_goal_truth(session.adapter.snapshot())
    }

    fn room_stage_scope(&self) -> &'static str {
        if self.plan.as_ref().is_some_and(|plan| {
            plan.steps
                .iter()
                .any(|step| matches!(&step.action, Action::HarvestResource { .. }))
        }) {
            "resource_acquisition_stage"
        } else {
            "reviewed_room_pursuit"
        }
    }

    fn resource_reserve_source(&self) -> Result<Option<goal_continuation::ProductionContinuation>> {
        let source = match &self.source {
            PlanSource::Production { raw, .. } => {
                let request = crate::lab_world::ProductionRequest::parse(raw)?;
                if request.planner() != Some(crate::lab_world::ProductionPlanner::ResourceReserveV1)
                {
                    return Ok(None);
                }
                goal_continuation::ProductionContinuation::new(
                    self.plan_digest,
                    self.plan_digest,
                    request,
                )?
            }
            PlanSource::ProductionContinuation { raw, .. } => {
                goal_continuation::ProductionContinuation::parse(raw)?
            }
            _ => return Ok(None),
        };
        Ok(source.is_resource_reserve().then_some(source))
    }

    fn resource_reserve_status(
        &self,
        session: &LabSession,
        source: &goal_continuation::ProductionContinuation,
    ) -> Result<ResourceReserveStatus> {
        // Preserve the original stock-ledger identity and current read grants.
        // This report never replaces the lineage, quiescence and plan checks at
        // continuation intake or grants authority to predicted future stock.
        self.current_resource_output_truth(session)?;
        let until = source.reserve_until_tick().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CorruptLedger,
                "the retained resource reserve has no original absolute horizon",
            )
        })?;
        if until <= session.adapter.snapshot().tick.0 {
            return Ok(ResourceReserveStatus::Expired);
        }
        Ok(
            if source.resource_reserve_work_required(session.adapter.snapshot())? {
                ResourceReserveStatus::WorkRequired
            } else {
                ResourceReserveStatus::FundedFromCurrentStock
            },
        )
    }

    fn current_resource_output_truth(&self, session: &LabSession) -> Result<PredicateTruth> {
        let context = context_for(session, session.next_request_id);
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        query_budget::admit_sources(&[session.adapter.snapshot()], &context)?;
        if self.abandoned(session) || self.verification_error.is_some() || self.plan.is_none() {
            return Err(DfmcpError::new(
                ErrorCode::PreconditionsFailed,
                "the original resource-aware production goal has no verified active custody",
            ));
        }
        let original_ledger = self.bound_resource_output_ledger()?;
        match &self.source {
            PlanSource::Production { raw, .. } => crate::lab_world::ProductionRequest::parse(raw)?
                .semantic_goal_truth_in_ledger(session.adapter.snapshot(), original_ledger),
            PlanSource::ProductionContinuation { raw, .. } => {
                goal_continuation::ProductionContinuation::parse(raw)?
                    .semantic_goal_truth_in_ledger(session.adapter.snapshot(), original_ledger)
            }
            _ => Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "current production output truth requires its complete original production request",
            )),
        }
    }

    fn bound_resource_output_ledger(&self) -> Result<crate::lab_world::ProductionGoalLedger> {
        let original = self.resource_output_ledger.ok_or_else(|| DfmcpError::new(
            ErrorCode::PreconditionsFailed,
            "the retained resource-aware production goal lacks its verified original stock ledger custody",
        ))?;
        original.validate_plan(self.plan.as_ref().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::CorruptLedger,
                "the original resource-aware production plan is unavailable",
            )
        })?)?;
        Ok(original)
    }

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
            resource_output_ledger: None,
            active_observation_anchor: Some(plan.anchor),
            original_handles_live: true,
            retired_never_dispatched: BTreeMap::new(),
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
        if self.is_resource_aware_production() {
            self.bound_resource_output_ledger()?
                .validate_current(session.adapter.snapshot())?;
        }
        crate::lab_world::plan_goal_truth(session.adapter.snapshot(), plan)
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
        let continuation_truth = if self.is_resource_aware_room() {
            let project = self.current_resource_room_truth(session);
            let truth = project.as_ref().copied().unwrap_or(PredicateTruth::Unknown);
            let mut projection =
                crate::observation_projection::laboratory_objective_evidence(project);
            projection["truth"] = projection["predicate_truth"].clone();
            projection["observed_anchor"] = anchor_json(&session.adapter.snapshot().anchor());
            projection["completion_scope"] = json!("complete_original_room_and_access");
            projection["historical_action_proof_rewritten"] = json!(false);
            result["semantic_room_goal"] = projection;
            result["goal_scope"] = json!(self.room_stage_scope());
            result["historical_achievement_scope"] = json!(self.room_stage_scope());
            truth
        } else if self.is_resource_aware_production() {
            let output = self.current_resource_output_truth(session);
            let truth = output.as_ref().copied().unwrap_or(PredicateTruth::Unknown);
            let mut projection =
                crate::observation_projection::laboratory_objective_evidence(output);
            projection["observed_anchor"] = anchor_json(&session.adapter.snapshot().anchor());
            projection["historical_action_proof_rewritten"] = json!(false);
            result["semantic_output_goal"] = projection;
            truth
        } else {
            truth
        };
        let reserve_source = self.resource_reserve_source().ok().flatten();
        let reserve_status = reserve_source
            .as_ref()
            .map(|source| self.resource_reserve_status(session, source));
        if let (Some(source), Some(status)) = (&reserve_source, &reserve_status) {
            let until = source.reserve_until_tick();
            let (name, epistemic, work_required) = match status {
                Ok(ResourceReserveStatus::WorkRequired) => {
                    ("work_required", "predicted", Some(true))
                }
                Ok(ResourceReserveStatus::FundedFromCurrentStock) => {
                    ("funded_from_current_stock", "predicted", Some(false))
                }
                Ok(ResourceReserveStatus::Expired) => ("expired", "observed", None),
                Err(_) => ("unknown", "unknown", None),
            };
            let mut reserve = json!({
                "planner": "resource_reserve_v1",
                "reserve_until_tick": until,
                "remaining_horizon_ticks": until.map(|tick| tick.saturating_sub(session.adapter.snapshot().tick.0)),
                "status": name,
                "epistemic": epistemic,
                "work_required": work_required,
                "new_request_required": matches!(status, Ok(ResourceReserveStatus::Expired)),
                "continuous_maintenance_proven": false,
                "historical_action_proof_rewritten": false,
            });
            if let Err(error) = status {
                reserve["error"] = json!({"code": error.code.as_str(), "message": error.message});
            }
            result["resource_reserve"] = reserve;
        }
        let continuation_needed = match &reserve_status {
            Some(Ok(ResourceReserveStatus::WorkRequired)) => true,
            Some(_) => false,
            None => continuation_truth == PredicateTruth::False,
        };
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
        result["retired_undispatched_steps"] = json!(
            self.retired_never_dispatched
                .iter()
                .map(|(step, proof)| json!({
                    "step": step,
                    "proof_anchor": anchor_json(&proof.anchor()),
                    "basis": "verified_retired_dispatch_custody",
                }))
                .collect::<Vec<_>>()
        );
        result["pursuit_active"] = json!(
            !abandoned
                && (truth != PredicateTruth::True
                    || reserve_source.is_some()
                    || (self.is_resource_aware_room()
                        && continuation_truth != PredicateTruth::True))
                && quiet != Some(true)
        );
        result["needs_replan"] = json!(!abandoned && continuation_needed && quiet == Some(true));
        result["replacement_work_dispatched"] = json!(false);
        result["blind_retry_allowed"] = json!(false);
        if !abandoned && continuation_needed && quiet == Some(true) && self.source.can_continue() {
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
        | PlanSource::ProductionContinuation { summary, .. }
        | PlanSource::RoomContinuation { summary, .. }
        | PlanSource::CivilianRelease { summary, .. }
        | PlanSource::ConnectedRoom { summary, .. }
        | PlanSource::ConnectedRoomContinuation { summary, .. }
        | PlanSource::ResourceAcquisitionContinuation { summary, .. } => summary,
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
        | PlanSource::ProductionContinuation { raw, .. }
        | PlanSource::RoomContinuation { raw, .. }
        | PlanSource::CivilianRelease { raw, .. }
        | PlanSource::ConnectedRoom { raw, .. }
        | PlanSource::ConnectedRoomContinuation { raw, .. }
        | PlanSource::ResourceAcquisitionContinuation { raw, .. } => json!({
            "kind": match source {
                PlanSource::Actions { .. } => "actions",
                PlanSource::Blueprint { .. } => "blueprint",
                PlanSource::ProductionContinuation { .. } => "production_continuation",
                PlanSource::RoomContinuation { .. } => "room_continuation",
                PlanSource::CivilianRelease { .. } => "civilian_release",
                PlanSource::ConnectedRoom { .. } => "connected_room",
                PlanSource::ConnectedRoomContinuation { .. } => "connected_room_continuation",
                PlanSource::ResourceAcquisitionContinuation { .. } => "resource_acquisition_continuation",
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
            "the original goal is not retained; no replacement request can be inferred from action receipts"))?;
    if objective.abandoned(session) {
        return Err(DfmcpError::new(
            ErrorCode::PreconditionsFailed,
            "restore abandoned pursuit of this original goal; explicitly request a new goal in the current epoch",
        ));
    }
    if objective.source.requires_complete_source_review() {
        // The public proposal, handoff discovery and commit-time revalidation
        // inspect the same complete finite resource and lineage domains.
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        query_budget::admit_sources(&[session.adapter.snapshot()], &context)?;
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
        PlanSource::Production { raw, .. } => {
            GoalContinuation::Production(goal_continuation::ProductionContinuation::new(
                parent,
                parent,
                crate::lab_world::ProductionRequest::parse(raw)?,
            )?)
        }
        PlanSource::ProductionContinuation { raw, .. } => GoalContinuation::Production(
            goal_continuation::ProductionContinuation::parse(raw)?.next(parent)?,
        ),
        PlanSource::Blueprint { raw, .. } if resource_acquisition::is_request(raw) => {
            GoalContinuation::ResourceAcquisition(
                resource_acquisition_continuation::ResourceAcquisitionContinuation::new(
                    parent,
                    parent,
                    resource_acquisition::ResourceAcquisitionRequest::parse(raw)?,
                )?,
            )
        }
        PlanSource::ResourceAcquisitionContinuation { raw, .. } => {
            GoalContinuation::ResourceAcquisition(
                resource_acquisition_continuation::ResourceAcquisitionContinuation::parse(raw)?
                    .next(parent)?,
            )
        }
        PlanSource::Blueprint { raw, .. } if room_provisioning::is_provisioned_room(raw) => {
            GoalContinuation::Room(room_continuation::RoomContinuation::new(
                parent,
                parent,
                room_provisioning::ProvisionedRoomRequest::parse(raw)?,
            )?)
        }
        PlanSource::RoomContinuation { raw, .. } => {
            GoalContinuation::Room(room_continuation::RoomContinuation::parse(raw)?.next(parent)?)
        }
        PlanSource::ConnectedRoom { raw, .. } => GoalContinuation::ConnectedRoom(
            connected_room_continuation::ConnectedRoomContinuation::new(
                parent,
                parent,
                connected_room::ConnectedRoomRequest::parse(raw)?,
            )?,
        ),
        PlanSource::ConnectedRoomContinuation { raw, .. } => GoalContinuation::ConnectedRoom(
            connected_room_continuation::ConnectedRoomContinuation::parse(raw)?.next(parent)?,
        ),
        _ => {
            return Err(DfmcpError::new(
                ErrorCode::InvalidIntent,
                "continue_goal supports retained production, resource acquisition and provisioned-room objectives; legacy action, pause and abstract room sources do not establish a resumable request",
            ));
        }
    };
    let quiet = goal_work_quiescent(session, objective)?;
    let active = objective.active_observation_anchor.ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::CorruptLedger,
            "the original goal has no verified active observation lineage",
        )
    })?;
    if continuation.is_resource_aware_room() {
        goal_continuation::validate_room_project_evidence(
            original,
            active,
            session.adapter.snapshot(),
            &context,
            quiet,
            continuation.resource_room_goal_truth(session.adapter.snapshot())?,
        )?;
    } else if let GoalContinuation::Production(source) = &continuation
        && source.is_resource_aware()
    {
        source.validate_resource_goal_evidence(
            original,
            objective.bound_resource_output_ledger()?,
            active,
            session.adapter.snapshot(),
            &context,
            quiet,
        )?;
    } else if active == original.anchor {
        goal_continuation::validate_goal_evidence(
            original,
            session.adapter.snapshot(),
            &context,
            quiet,
        )?;
    } else {
        goal_continuation::validate_goal_evidence_in_active_lineage(
            original,
            active,
            session.adapter.snapshot(),
            &context,
            quiet,
        )?;
    }
    let reserve_work_required = match &continuation {
        GoalContinuation::Production(source) if source.is_resource_reserve() => {
            source.resource_reserve_work_required(session.adapter.snapshot())?
        }
        _ => false,
    };
    for other in &session.objectives {
        if other.plan_digest == parent || Some(other.plan_digest) == exclude_candidate {
            continue;
        }
        let root = other.source.continuation_root(other.plan_digest)?;
        if root == Some(continuation.root()) {
            if !goal_work_quiescent(session, other)? {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    format!(
                        "original goal lineage still has unresolved work in plan {}; observe or explicitly drain that pursuit before continuing",
                        other.plan_digest.to_hex()
                    ),
                ));
            }
            if continuation.requires_semantic_sibling_check() {
                let sibling_truth = if continuation.is_resource_aware_room() {
                    other.current_resource_room_truth(session)?
                } else {
                    other.current_truth(session)?
                };
                match sibling_truth {
                    PredicateTruth::True if !reserve_work_required => {
                        return Err(DfmcpError::new(
                            ErrorCode::InvalidIntent,
                            format!(
                                "the retained semantic request is already satisfied by pursuit {}; historical exact goal proofs remain unchanged",
                                other.plan_digest.to_hex()
                            ),
                        ));
                    }
                    PredicateTruth::Unknown
                        if matches!(continuation, GoalContinuation::ResourceAcquisition(_))
                            || continuation.is_resource_aware_room() =>
                    {
                        return Err(DfmcpError::new(
                            ErrorCode::PreconditionsFailed,
                            "an earlier resource pursuit has unknown original-goal evidence; reconcile it before acquiring more material",
                        ));
                    }
                    PredicateTruth::True | PredicateTruth::False | PredicateTruth::Unknown => {}
                }
            }
        }
    }
    if let GoalContinuation::ResourceAcquisition(source) = &continuation {
        match source.stock_goal_truth(session.adapter.snapshot())? {
            PredicateTruth::True => {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidIntent,
                    "the complete original resource stock minima already hold; no new acquisition is needed and historical exact harvest proofs remain unchanged",
                ));
            }
            PredicateTruth::Unknown => {
                return Err(DfmcpError::new(
                    ErrorCode::PreconditionsFailed,
                    "the complete original resource stock minima are unknown; reconcile current qualified stock before continuing",
                ));
            }
            PredicateTruth::False => {}
        }
    }
    Ok(continuation.into_source(summary.map_or_else(
        || source_summary(&objective.source).to_owned(),
        |summary| summary,
    )))
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
        continuation.parent(),
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
        if let Some(proof) = objective.retired_never_dispatched.get(&step.id.get()) {
            // Contradicted custody requires reconciliation even if the
            // contradictory entity now looks like completed physical work.
            if !proof.covers(plan, step, session.adapter.snapshot()) {
                return Ok(false);
            }
            continue;
        }
        if let Some(carried) = session
            .carried
            .iter()
            .find(|carried| carried.plan_digest == plan.digest && carried.step == Some(step.id))
        {
            if carried.state == "indeterminate" {
                return Ok(false);
            }
            if carried.is_final() && carried.work_state == EffectWorkState::NeverDispatched {
                if !goal_custody::effect_identity_absent(step, session.adapter.snapshot()) {
                    return Ok(false);
                }
                continue;
            }
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
            || !goal_work_quiescent(session, objective).unwrap_or(false)
        {
            continue;
        }
        // The resource stage can finish before the room exists, and its WOOD
        // target can later become false when the completed room consumes it.
        // Neither observation determines completion of the retained project.
        let completion = if objective.is_resource_aware_room() {
            objective.current_resource_room_truth(session)
        } else {
            objective.current_truth(session)
        };
        if !matches!(completion, Ok(PredicateTruth::True)) {
            continue;
        }
        match objective.resource_reserve_source() {
            Ok(Some(source)) => match objective.resource_reserve_status(session, &source) {
                Ok(
                    ResourceReserveStatus::FundedFromCurrentStock | ResourceReserveStatus::Expired,
                ) => {}
                // A presently true quota can still have unfinished future
                // reserve work. Missing qualification never frees that intent.
                Ok(ResourceReserveStatus::WorkRequired) | Err(_) => continue,
            },
            Ok(None) => {}
            Err(_) => continue,
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

/// Establish retained semantic-output custody before leases, durable admission
/// or adapter dispatch. Only the exact immutable sealed version is eligible;
/// a post-dispatch current ledger can never stand in for this source.
pub(super) fn prepare_resource_output_ledger(
    session: &LabSession,
    plan: &PreparedPlan,
    source: &PlanSource,
) -> Result<Option<crate::lab_world::ProductionGoalLedger>> {
    if !source.is_resource_aware_production() {
        return Ok(None);
    }
    let sealed = if session.adapter.snapshot().anchor() == plan.anchor {
        Some(session.adapter.snapshot())
    } else {
        session.history.get(&plan.anchor.state_hash)
    }
    .filter(|snapshot| snapshot.anchor() == plan.anchor)
    .ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::StaleAnchor,
            "the exact sealed production source is unavailable for original stock ledger custody",
        )
    })?;
    let original = crate::lab_world::ProductionGoalLedger::capture(sealed)?;
    original.validate_plan(plan)?;
    Ok(Some(original))
}

pub(super) fn install_objective(
    session: &mut LabSession,
    plan: &PreparedPlan,
    source: PlanSource,
    evicted: &[Digest32],
    resource_output_ledger: Option<crate::lab_world::ProductionGoalLedger>,
) {
    session
        .objectives
        .retain(|objective| !evicted.contains(&objective.plan_digest));
    if !session
        .objectives
        .iter()
        .any(|objective| objective.plan_digest == plan.digest)
    {
        let mut objective = Objective::committed(plan, source, session.session_id);
        objective.resource_output_ledger = resource_output_ledger;
        // The public commit owner validates this before admitting any effect.
        // Internal or recovered callers that omit it remain visibly unknown.
        if objective.is_resource_aware_production()
            && let Err(error) = objective.bound_resource_output_ledger()
        {
            objective.verification_error = Some(error.message);
        }
        session.objectives.push(objective);
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
            objective.active_observation_anchor = None;
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
        objective.active_observation_anchor = None;
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
            .chain(
                objective
                    .retired_never_dispatched
                    .values()
                    .map(|proof| proof.anchor().state_hash),
            )
    })
}

/// Recompile only against the exact archived seal. Recovered plans are proof
/// and identity material; no action handle or dispatch authority is restored.
pub(super) fn recover_objective(
    store: &DurableLabStore,
    retained: &DurableObjective,
    recovery: &goal_continuation::RecoveredGoalLineage,
) -> Objective {
    let source = PlanSource::from_durable(&retained.source);
    let mut objective = Objective {
        plan_digest: retained.plan_digest,
        source,
        owner_session_id: retained.owner_session_id,
        originating_process: false,
        sealed_state_hash: retained.sealed_state_hash,
        sealed_anchor: None,
        plan: None,
        resource_output_ledger: None,
        active_observation_anchor: None,
        original_handles_live: false,
        retired_never_dispatched: BTreeMap::new(),
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
        if objective.is_resource_aware_production() {
            let original = crate::lab_world::ProductionGoalLedger::capture(&sealed)?;
            original.validate_plan(&plan)?;
            objective.resource_output_ledger = Some(original);
        }
        if !retained.retired_never_dispatched.is_empty()
            && store
                .commit(retained.fortress_id, retained.plan_digest)
                .is_some()
        {
            return Err(DfmcpError::new(
                ErrorCode::CorruptLedger,
                "a live original commit cannot import retired nondispatch custody",
            ));
        }
        let mut retired = BTreeMap::new();
        for (step, anchor) in &retained.retired_never_dispatched {
            let archived = store.load_snapshot(anchor.state_hash)?;
            let proof = goal_custody::RetiredNondispatchProof::verify(
                &plan, retained, *step, &archived, recovery,
            )?;
            retired.insert(*step, proof);
        }
        if let Some(anchor) = retained.first_satisfied_anchor {
            let proof = store.load_snapshot(anchor.state_hash)?;
            if objective.is_resource_aware_production() {
                objective.resource_output_ledger.ok_or_else(|| DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "the historical resource-aware goal lacks its original stock ledger custody",
                ))?.validate_current(&proof)?;
            }
            if proof.anchor() != anchor
                || anchor.fortress_id != retained.fortress_id
                || crate::lab_world::plan_goal_truth(&proof, &plan)? != PredicateTruth::True
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
        if retained.restore_abandoned_anchor.is_none() {
            let prior = recovery.previous_anchor();
            if prior.fortress_id != retained.fortress_id
                || plan.anchor.cursor.epoch > prior.cursor.epoch
                || plan.anchor.tick > prior.tick
                || (plan.anchor.cursor.epoch == prior.cursor.epoch
                    && plan.anchor.cursor.sequence > prior.cursor.sequence)
                || (plan.anchor.cursor == prior.cursor && plan.anchor != prior)
            {
                return Err(DfmcpError::new(
                    ErrorCode::CorruptLedger,
                    "the original goal does not precede its verified durable recovery head",
                ));
            }
            objective.active_observation_anchor = Some(recovery.active_anchor());
        }
        objective.plan = Some(plan);
        objective.retired_never_dispatched = retired;
        Ok(())
    })();
    if let Err(error) = checked {
        objective.verification_error = Some(format!("{}: {}", error.code.as_str(), error.message));
    }
    objective
}
