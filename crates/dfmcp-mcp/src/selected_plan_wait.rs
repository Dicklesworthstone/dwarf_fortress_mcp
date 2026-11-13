//! Stop a foreground wait at the outcome of one exact original commit.
//!
//! The world clock and normal progression remain shared. Selecting a stop
//! condition neither changes other work nor adopts a continuation's actions.
//! Current project truth, sealed stage truth, action proof, physical work and
//! an original future reserve frontier are separate observations.

use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, CommitState, DfmcpError, Digest32, ErrorCode, Result, RiskTier, StateAnchor,
};
use dfmcp_intent::{EffectWorkState, plan_archive};
use dfmcp_world::PredicateTruth;
use serde_json::{Value, json};

use super::{LabSession, Objective, anchor_json, authorize_entry, read_context};

const MAX_SOURCE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STEPS: usize = 64;

#[path = "recovered_plan_wait.rs"]
mod recovered;
pub(super) use recovered::RecoveredPlanCustody;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

pub(super) fn parse_digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "selected wait requires exactly 64 lowercase hexadecimal plan-digest digits",
        ));
    }
    Digest32::from_hex(raw).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "invalid selected original plan digest",
        )
    })
}

#[derive(Clone, Copy, Debug, Default)]
struct Actions {
    total: usize,
    verified: usize,
    terminal: usize,
    not_dispatched: usize,
    active: usize,
    unknown: usize,
    failed: bool,
    cancelled: bool,
    cancelling: bool,
    indeterminate: bool,
}

impl Actions {
    fn verified(self) -> bool {
        self.total > 0 && self.total == self.verified
    }

    fn terminal(self) -> bool {
        self.total > 0 && self.total == self.terminal
    }

    fn quiet(self) -> Option<bool> {
        (self.unknown == 0).then_some(self.active == 0)
    }

    fn interrupted(self) -> Option<&'static str> {
        if self.indeterminate {
            Some("indeterminate")
        } else if self.unknown > 0 {
            Some("work_unknown")
        } else if self.failed {
            Some("failed")
        } else if self.cancelled {
            Some("cancelled")
        } else if self.cancelling {
            Some("cancellation_pending")
        } else {
            None
        }
    }
}

#[derive(Clone, Copy)]
struct ReserveObservation {
    anchor: StateAnchor,
    truth: PredicateTruth,
}

#[derive(Clone)]
struct RecoveredSelection {
    custody_digest: Digest32,
    evidence: Value,
}

fn exact_original(session: &LabSession, digest: Digest32) -> Result<&Objective> {
    let mut originals = session
        .objectives
        .iter()
        .filter(|objective| objective.plan_digest == digest);
    let original = originals
        .next()
        .ok_or_else(|| refused("selected wait has no retained original goal"))?;
    if originals.next().is_some() {
        return Err(refused(
            "selected wait has ambiguous retained original goal custody",
        ));
    }
    Ok(original)
}

/// One request's selector and observation state, held under the session owner.
/// Historical reserve evidence is never inferred from a later current stock.
/// Exact observations survive durable recovery only with verified archived
/// original custody. Restore abandonment never imports an active endpoint.
pub(super) struct SelectedPlanWait {
    selected: super::plan_cancellation::Selection,
    recovered: Option<RecoveredSelection>,
    original_owner_session_id: String,
    started: Instant,
    retained_plan_bytes: usize,
    archive_digest: Digest32,
    source_inspections: u64,
    inspected_records: u64,
    inspected_bytes: u64,
    maximum_source_records: u64,
    maximum_source_bytes: u64,
    reserve_until: Option<u64>,
    can_continue: bool,
    reserve_observation: Option<ReserveObservation>,
    reserve_reused: bool,
    reserve_durable: bool,
    remaining_reserve_work: Option<bool>,
    reserve_error: Option<Value>,
    goal_truth: PredicateTruth,
    stage_truth: PredicateTruth,
    goal_error: Option<Value>,
    goal_anchor: Option<StateAnchor>,
    actions: Actions,
    stop: Option<&'static str>,
}

/// An original goal's current evidence for a passive Task monitor. Pending
/// future evidence is distinct from unavailable or negative terminal proof.
pub(super) struct TaskGoalObservation {
    pub(super) truth: PredicateTruth,
    pub(super) pending: bool,
    pub(super) projection: Value,
}

impl SelectedPlanWait {
    pub(super) fn new(
        session: &mut LabSession,
        digest: Digest32,
        started: Instant,
    ) -> Result<Self> {
        let mut selected = Self::from_custody(session, digest, started)?;
        super::objectives::ensure_reserve_snapshot(session, selected.selected.actual)?;
        selected.inspect(session)?;
        Ok(selected)
    }

    /// Consume only already-retained endpoint evidence. Task reads, wakeups and
    /// cancellation checks cannot capture a reserve endpoint, publish history,
    /// poll an action, dispatch a successor or advance the world clock.
    pub(super) fn observe_retained(
        session: &LabSession,
        digest: Digest32,
    ) -> Result<TaskGoalObservation> {
        let mut selected = Self::from_custody(session, digest, Instant::now())?;
        selected.inspect_current(session, false)?;
        let now = session.adapter.snapshot().tick.0;
        let pending = selected.reserve_until.is_some_and(|until| now <= until)
            && selected.reserve_observation.is_none()
            && selected.stop.is_none();
        let truth = if pending || selected.reserve_error.is_some() {
            PredicateTruth::Unknown
        } else if let Some(until) = selected.reserve_until {
            match selected.reserve_observation.map(|sample| sample.truth) {
                Some(PredicateTruth::False) => PredicateTruth::False,
                Some(PredicateTruth::Unknown) => PredicateTruth::Unknown,
                None if now >= until => PredicateTruth::Unknown,
                None if selected.stop == Some("needs_continuation") => PredicateTruth::False,
                _ => selected.goal_truth,
            }
        } else {
            selected.goal_truth
        };
        let mut projection = selected.projection(session, true);
        projection["observation_only"] = json!(true);
        projection["reserve_capture_required"] =
            json!(selected.reserve_until == Some(now) && selected.reserve_observation.is_none());
        Ok(TaskGoalObservation {
            truth,
            pending,
            projection,
        })
    }

    fn from_custody(session: &LabSession, digest: Digest32, started: Instant) -> Result<Self> {
        let context = read_context(session);
        context.budget.validate()?;
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        // A present live receipt is authoritative for selection. Malformed or
        // foreign live custody cannot be bypassed by trying recovery instead.
        let (selected, recovered) = if session.commit_receipts.contains_key(&digest.to_hex()) {
            (super::plan_cancellation::select(session, digest)?, None)
        } else {
            let original = exact_original(session, digest)?;
            let custody = original.selected_wait_recovery(session)?;
            let plan = original
                .resource_plan()
                .ok_or_else(|| refused("selected recovered original plan is unavailable"))?;
            (
                super::plan_cancellation::Selection {
                    requested: digest,
                    actual: digest,
                    plan_id: plan.id.to_string(),
                    actions: Vec::new(),
                },
                Some(RecoveredSelection {
                    custody_digest: custody.digest()?,
                    evidence: custody.evidence_json()?,
                }),
            )
        };
        let original = exact_original(session, selected.actual)?;
        let entry = original.resource_goal(session);
        if (recovered.is_none() && !entry.owned_by_current_session)
            || original.is_withdrawn()
            || entry.restore_abandoned_anchor.is_some()
            || entry.abandonment_publication_pending
            || entry.verification_error.is_some()
        {
            return Err(refused(
                "selected wait requires verified original goal custody whose pursuit was neither abandoned nor withdrawn",
            ));
        }
        let plan = entry
            .plan
            .ok_or_else(|| refused("selected original plan is unavailable"))?;
        if plan.digest != selected.actual
            || plan.id.to_string() != selected.plan_id
            || plan.anchor.fortress_id != context.anchor.fortress_id
            || (recovered.is_none()
                && (plan.steps.len() != selected.actions.len()
                    || plan.anchor.cursor.epoch != context.anchor.cursor.epoch))
        {
            return Err(refused(
                "selected original plan, epoch and action custody disagree",
            ));
        }
        if plan.steps.is_empty()
            || plan.steps.len() > MAX_STEPS
            || plan.steps.len() > context.budget.max_actions as usize
        {
            return Err(exhausted(
                "selected original action census exceeds the request budget",
            ));
        }
        let original_owner_session_id = entry.owner_session_id.to_string();
        let (archive, seal) = plan_archive::encode_with_check(
            plan,
            plan_archive::ArchiveLimits {
                max_bytes: (context.budget.max_bytes.min(MAX_SOURCE_BYTES) as usize)
                    .min(plan_archive::MAX_ARCHIVE_BYTES),
                max_steps: MAX_STEPS,
                ..plan_archive::ArchiveLimits::default()
            },
            || check_time(session, started),
        )?;
        // The optional continuation oracle can inspect every retained sibling
        // seal. Charge the complete retained plan domain once under this same
        // request clock; immutable plans do not grow during foreground time.
        let mut retained_plan_bytes = archive.len();
        let maximum_bytes = context.budget.max_bytes.min(MAX_SOURCE_BYTES) as usize;
        for other in &session.objectives {
            if other.plan_digest == selected.actual {
                continue;
            }
            if let Some(plan) = other.resource_plan() {
                let (archive, _) = plan_archive::encode_with_check(
                    plan,
                    plan_archive::ArchiveLimits {
                        max_bytes: maximum_bytes
                            .saturating_sub(retained_plan_bytes)
                            .min(plan_archive::MAX_ARCHIVE_BYTES),
                        max_steps: MAX_STEPS,
                        ..plan_archive::ArchiveLimits::default()
                    },
                    || check_time(session, started),
                )?;
                retained_plan_bytes = retained_plan_bytes.saturating_add(archive.len());
            }
        }
        let reserve_until = original.selected_wait_reserve_horizon()?;
        let can_continue = original.selected_wait_can_continue();
        let reserve_observation = original
            .selected_wait_reserve_observation
            .map(|(anchor, truth)| ReserveObservation { anchor, truth });
        let reserve_durable = original.selected_wait_reserve_durable;
        if reserve_observation.is_some_and(|sample| {
            reserve_until != Some(sample.anchor.tick.0)
                || sample.anchor.fortress_id != context.anchor.fortress_id
                || sample.anchor.cursor.epoch > context.anchor.cursor.epoch
                || (sample.anchor.cursor.epoch != context.anchor.cursor.epoch
                    && (recovered.is_none() || !reserve_durable))
                || (sample.anchor.cursor.epoch == context.anchor.cursor.epoch
                    && sample.anchor.cursor.sequence > context.anchor.cursor.sequence)
                || (sample.anchor.cursor == context.anchor.cursor
                    && sample.anchor != context.anchor)
                || sample.anchor.tick > context.anchor.tick
        }) {
            return Err(refused(
                "retained exact reserve observation disagrees with original custody",
            ));
        }
        Ok(Self {
            selected,
            recovered,
            original_owner_session_id,
            started,
            retained_plan_bytes,
            archive_digest: seal.object_digest,
            source_inspections: 0,
            inspected_records: 0,
            inspected_bytes: 0,
            maximum_source_records: 0,
            maximum_source_bytes: 0,
            reserve_until,
            can_continue,
            reserve_observation,
            reserve_reused: reserve_observation.is_some(),
            reserve_durable,
            remaining_reserve_work: None,
            reserve_error: None,
            goal_truth: PredicateTruth::Unknown,
            stage_truth: PredicateTruth::Unknown,
            goal_error: None,
            goal_anchor: None,
            actions: Actions::default(),
            stop: None,
        })
    }

    fn original<'a>(&self, session: &'a LabSession) -> Result<&'a Objective> {
        exact_original(session, self.selected.actual)
    }

    fn recovery_custody<'a>(&self, session: &'a LabSession) -> Result<&'a RecoveredPlanCustody> {
        let selected = self
            .recovered
            .as_ref()
            .ok_or_else(|| refused("selected wait has no recovered action custody"))?;
        let custody = self.original(session)?.selected_wait_recovery(session)?;
        if custody.digest()? != selected.custody_digest {
            return Err(refused(
                "selected recovered original action custody changed",
            ));
        }
        Ok(custody)
    }

    pub(super) fn check(&self, session: &LabSession) -> Result<()> {
        let context = read_context(session);
        authorize_entry(&context, Capability::Observe, RiskTier::ReadOnly)?;
        authorize_entry(&context, Capability::Query, RiskTier::ReadOnly)?;
        check_time(session, self.started)
    }

    /// Admission covers all source rows, including rows omitted from the small
    /// selected result. The fixed progress slice/poll ceilings and one deadline
    /// bound repeated inspections; source size is checked at every frontier.
    fn admit_source(&mut self, session: &LabSession) -> Result<()> {
        self.check(session)?;
        let mut context = read_context(session);
        let spent = self.started.elapsed().as_nanos().div_ceil(1_000_000);
        if spent >= u128::from(context.budget.max_wall_millis) {
            return Err(exhausted(
                "selected wait has no source-inspection wall budget",
            ));
        }
        context.budget.max_wall_millis -= spent as u64;
        let mut snapshots = vec![session.adapter.snapshot()];
        if let Some(sample) = self.reserve_observation
            && sample.anchor != session.adapter.snapshot().anchor()
        {
            let exact = session
                .history
                .get(&sample.anchor.state_hash)
                .filter(|snapshot| snapshot.anchor() == sample.anchor)
                .ok_or_else(|| {
                    refused("the selected original reserve snapshot is no longer retained")
                })?;
            snapshots.push(exact);
        }
        let source = super::query_budget::admit_sources(&snapshots, &context)?;
        let mut records = source["records"].as_u64().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "source row count is missing",
            )
        })?;
        let mut bytes = source["canonical_bytes"].as_u64().ok_or_else(|| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                "source byte count is missing",
            )
        })?;
        records = records.saturating_add(session.objectives.len() as u64);
        // The normal continuation oracle can inspect sibling goals. Admit their
        // complete retained source and step domain before offering that request.
        for objective in &session.objectives {
            let entry = objective.resource_goal(session);
            records = records.saturating_add(entry.plan.map_or(0, |plan| plan.steps.len()) as u64);
            bytes = bytes
                .saturating_add(entry.summary.len() as u64)
                .saturating_add(entry.request_json.map_or(0, str::len) as u64)
                .saturating_add(entry.verification_error.map_or(0, str::len) as u64);
            if let Some((anchor, truth)) = objective.selected_wait_reserve_observation {
                records = records.saturating_add(1);
                bytes = bytes.saturating_add(
                    json!({"anchor":anchor_json(&anchor), "truth":truth_name(truth)})
                        .to_string()
                        .len() as u64,
                );
            }
        }
        records = records.saturating_add(session.carried.len() as u64);
        bytes = bytes.saturating_add(self.retained_plan_bytes as u64);
        if self.recovered.is_some() {
            let custody = self.recovery_custody(session)?;
            let plan = self
                .original(session)?
                .resource_plan()
                .ok_or_else(|| refused("selected recovered original plan disappeared"))?;
            records = records.saturating_add(plan.steps.len() as u64);
            bytes = bytes.saturating_add(custody.bytes()? as u64);
            // The original proof monitor and continuation oracle can inspect
            // carried work from siblings. Count every retained row, not only
            // the selected entries projected in the compact outcome.
            for row in &session.carried {
                bytes = bytes.saturating_add(row.to_json().to_string().len() as u64);
            }
        } else {
            bytes = bytes.saturating_add(
                session
                    .commit_receipts
                    .get(&self.selected.requested.to_hex())
                    .ok_or_else(|| refused("selected original commit custody disappeared"))?
                    .len() as u64,
            );
        }
        if records > u64::from(context.budget.max_entities)
            || bytes > context.budget.max_bytes.min(MAX_SOURCE_BYTES)
        {
            return Err(exhausted(
                "selected wait exceeds its complete world and original-source budget",
            ));
        }
        self.source_inspections += 1;
        self.inspected_records = self.inspected_records.saturating_add(records);
        self.inspected_bytes = self.inspected_bytes.saturating_add(bytes);
        self.maximum_source_records = self.maximum_source_records.max(records);
        self.maximum_source_bytes = self.maximum_source_bytes.max(bytes);
        self.check(session)
    }

    fn inspect_actions(&self, session: &LabSession) -> Result<Actions> {
        if self.recovered.is_some() {
            let original = self.original(session)?;
            let plan = original
                .resource_plan()
                .ok_or_else(|| refused("selected recovered original plan disappeared"))?;
            return self.recovery_custody(session)?.inspect(session, plan);
        }
        let mut actions = Actions {
            total: self.selected.actions.len(),
            ..Actions::default()
        };
        for id in &self.selected.actions {
            self.check(session)?;
            let receipt = session
                .adapter
                .action_receipt(*id)
                .ok_or_else(|| refused("selected original action receipt disappeared"))?;
            actions.verified += usize::from(receipt.state == CommitState::Verified);
            actions.terminal += usize::from(receipt.state.is_terminal());
            actions.failed |= receipt.state == CommitState::Failed;
            actions.cancelled |= matches!(
                receipt.state,
                CommitState::Cancelled | CommitState::Compensated
            );
            actions.cancelling |= matches!(
                receipt.state,
                CommitState::CancelRequested | CommitState::CompensationPending
            );
            actions.indeterminate |= matches!(
                receipt.state,
                CommitState::Indeterminate | CommitState::Committing
            );
            match session.adapter.action_work_state(*id)? {
                EffectWorkState::Active { .. } => actions.active += 1,
                EffectWorkState::Unknown { .. } => actions.unknown += 1,
                EffectWorkState::NeverDispatched | EffectWorkState::Quiescent { .. } => {}
            }
        }
        Ok(actions)
    }

    /// Sample the selected outcome only after the owner published the current
    /// complete progression slice. No polling, dispatch or clock change occurs.
    fn inspect_current(&mut self, session: &LabSession, capture_endpoint: bool) -> Result<()> {
        self.admit_source(session)?;
        self.actions = self.inspect_actions(session)?;
        let original = self.original(session)?;
        let (goal, goal_error) = observed_truth(original.selected_wait_truth(session))?;
        let (stage, stage_error) = observed_truth(original.selected_wait_stage_truth(session))?;
        let reserve = original.selected_wait_reserve(session);
        self.goal_truth = goal;
        self.stage_truth = stage;
        self.goal_error = goal_error.or(stage_error);
        self.goal_anchor = Some(session.adapter.snapshot().anchor());
        match reserve {
            Ok(value) => {
                if value.map(|value| value.0) != self.reserve_until {
                    return Err(refused("selected original reserve horizon changed"));
                }
                self.remaining_reserve_work = value.and_then(|value| value.1);
                self.reserve_error = None;
            }
            Err(error) if fatal_evaluation(&error) => return Err(error),
            Err(error) => {
                self.remaining_reserve_work = None;
                self.reserve_error = Some(error_json(error));
            }
        }
        if capture_endpoint && self.reserve_until == Some(session.adapter.snapshot().tick.0) {
            // The first exact observed endpoint is immutable across bounded
            // calls by this owner. Later stock cannot replace False/Unknown
            // with a more convenient sample.
            if self.reserve_observation.is_none() {
                self.reserve_observation = Some(ReserveObservation {
                    anchor: session.adapter.snapshot().anchor(),
                    truth: self.goal_truth,
                });
            }
        }
        self.stop = stop_reason(
            self.actions,
            self.goal_truth,
            self.reserve_until,
            self.reserve_observation
                .map(|observation| observation.truth),
            self.remaining_reserve_work,
            self.reserve_error.is_some(),
            session.adapter.snapshot().tick.0,
        );
        if self.stop == Some("goal_unmet") && self.can_continue {
            self.stop = Some("needs_continuation");
        }
        // An explicit zero-tick selected wait owns endpoint capture. A passive
        // monitor at that exact tick must leave time to perform that observation;
        // it cannot turn an unrecorded endpoint into either proof or failure.
        if !capture_endpoint
            && self.reserve_until == Some(session.adapter.snapshot().tick.0)
            && self.reserve_observation.is_none()
            && self.actions.interrupted().is_none()
        {
            self.stop = None;
        }
        self.check(session)
    }

    pub(super) fn inspect(&mut self, session: &mut LabSession) -> Result<()> {
        self.inspect_current(session, true)?;
        if let Some(sample) = self.reserve_observation {
            self.reserve_durable = super::objectives::retain_reserve_observation(
                session,
                self.selected.actual,
                sample.anchor,
                sample.truth,
            )?;
        }
        self.check(session)
    }

    pub(super) fn stopped(&self) -> bool {
        self.stop.is_some()
    }

    /// Keep an original reserve endpoint inside a coarse caller duration and
    /// the ordinary cadence/deadline ceiling. Never extend a sealed obligation.
    pub(super) fn ceiling(&self, now: u64, other: Option<u64>) -> Option<u64> {
        let reserve = self
            .reserve_until
            .filter(|until| *until > now)
            .map(|until| until - now);
        match (other, reserve) {
            (Some(left), Some(right)) => Some(left.min(right)),
            (left, right) => left.or(right),
        }
    }

    pub(super) fn finish(&mut self, paused: bool) {
        if self.stop.is_none() {
            self.stop = Some(if paused { "paused" } else { "tick_limit" });
        }
    }

    pub(super) fn projection(&self, session: &LabSession, inspection_complete: bool) -> Value {
        let reserve = self.reserve_until.map(|until| {
            let observed = self.reserve_observation;
            json!({
                "reserve_until_tick":until,
                "observed_anchor":observed.as_ref().map(|sample| anchor_json(&sample.anchor)),
                "truth":observed.map(|sample| truth_name(sample.truth)),
                "epistemic":if observed.is_some_and(|sample| sample.truth != PredicateTruth::Unknown) {
                    "certified_derived"
                } else { "unknown" },
                "available":observed.is_some(),
                "reused_from_prior_wait":self.reserve_reused,
                "retention":if self.reserve_durable {
                    "durable_original_exact_snapshot"
                } else { "owning_session_exact_snapshot" },
                "durable":self.reserve_durable,
                "survives_restart":self.reserve_durable,
                "survives_restore":false,
                "remaining_work_required":self.remaining_reserve_work,
                "remaining_work_epistemic":"predicted",
                "error":self.reserve_error,
                "continuous_maintenance_proven":false,
                "changes_action_deadlines":false,
            })
        });
        let mut value = json!({
            "schema":"dfmcp.selected-plan-wait/1",
            "requested_plan_digest":self.selected.requested.to_hex(),
            "plan_digest":self.selected.actual.to_hex(),
            "plan_id":self.selected.plan_id,
            "plan_object_digest":self.archive_digest.to_hex(),
            "selection_scope":if self.recovered.is_some() {
                "exact_recovered_original"
            } else { "exact_original_commit" },
            "selection_origin":if self.recovered.is_some() {
                "durable_recovery"
            } else { "current_session_commit" },
            "original_owner_session_id":self.original_owner_session_id,
            "owns_original_dispatch":self.recovered.is_none(),
            "dispatch_handles_restored":false,
            "recovery_evidence":self.recovered.as_ref().map(|recovered| &recovered.evidence),
            "descendant_plans_included":false,
            "other_session_work_progresses":true,
            "complete":inspection_complete && self.stop == Some("complete"),
            "inspection_complete":inspection_complete,
            "stop_reason":self.stop,
            "goal_truth":truth_name(self.goal_truth),
            "stage_goal_truth":truth_name(self.stage_truth),
            "goal_error":self.goal_error,
            "goal_observed_anchor":self.goal_anchor.as_ref().map(anchor_json),
            "observed_anchor":anchor_json(&session.adapter.snapshot().anchor()),
            "actions_verified":self.actions.verified(),
            "actions_terminal":self.actions.terminal(),
            "physical_quiescent":self.actions.quiet(),
            "action_count":self.actions.total,
            "verified_actions":self.actions.verified,
            "terminal_actions":self.actions.terminal,
            "not_dispatched_actions":self.actions.not_dispatched,
            "active_work_actions":self.actions.active,
            "unknown_work_actions":self.actions.unknown,
            "reserve_frontier":reserve,
            "automatic_continuation":false,
            "source_work":{
                "inspections":self.source_inspections,
                "inspected_records":self.inspected_records,
                "inspected_canonical_and_retained_bytes":self.inspected_bytes,
                "maximum_source_records":self.maximum_source_records,
                "maximum_source_bytes":self.maximum_source_bytes,
                "size_limit_scope":"each_complete_observed_source",
                "repetition_bounds":"one_foreground_deadline_and_progress_slice_poll_limits",
            },
        });
        if inspection_complete && matches!(self.stop, Some("needs_continuation" | "goal_unmet")) {
            match super::objectives::continuation_source(session, self.selected.actual, None, None)
            {
                Ok(_) => {
                    let next = super::goal_continuation::request_json(
                        &session.session_id.to_string(),
                        self.selected.actual,
                    );
                    value["continuation_request"] = next.clone();
                    value["next_step"] = next;
                }
                Err(error) => value["continuation_refused"] = error_json(error),
            }
        }
        value
    }
}

fn check_time(session: &LabSession, started: Instant) -> Result<()> {
    if started.elapsed() >= Duration::from_millis(session.budget.max_wall_millis) {
        Err(exhausted(
            "selected wait exhausted its complete foreground wall-time budget",
        ))
    } else {
        Ok(())
    }
}

fn error_json(error: DfmcpError) -> Value {
    json!({"code":error.code.as_str(),"message":error.message})
}

fn fatal_evaluation(error: &DfmcpError) -> bool {
    matches!(
        error.code,
        ErrorCode::BudgetExceeded
            | ErrorCode::CapabilityDenied
            | ErrorCode::CancellationRequested
            | ErrorCode::InternalInvariantViolation
    )
}

pub(super) fn observed_truth(
    result: Result<PredicateTruth>,
) -> Result<(PredicateTruth, Option<Value>)> {
    match result {
        Ok(truth) => Ok((truth, None)),
        Err(error) if fatal_evaluation(&error) => Err(error),
        Err(error) => Ok((PredicateTruth::Unknown, Some(error_json(error)))),
    }
}

fn truth_name(truth: PredicateTruth) -> &'static str {
    match truth {
        PredicateTruth::True => "true",
        PredicateTruth::False => "false",
        PredicateTruth::Unknown => "unknown",
    }
}

fn stop_reason(
    actions: Actions,
    goal: PredicateTruth,
    until: Option<u64>,
    endpoint: Option<PredicateTruth>,
    reserve_work: Option<bool>,
    reserve_unknown: bool,
    now: u64,
) -> Option<&'static str> {
    if let Some(stop) = actions.interrupted() {
        return Some(stop);
    }
    if let Some(until) = until {
        if now > until && endpoint.is_none() {
            return Some("reserve_unobserved");
        }
        if endpoint == Some(PredicateTruth::False) {
            return Some("reserve_unmet");
        }
        if endpoint == Some(PredicateTruth::Unknown) {
            return Some("reserve_unknown");
        }
    }
    if !actions.terminal() || actions.quiet() != Some(true) {
        return None;
    }
    if goal == PredicateTruth::Unknown {
        return Some("goal_unknown");
    }
    if !actions.verified() {
        return Some("goal_unmet");
    }
    if let Some(until) = until {
        if reserve_unknown {
            return Some("reserve_unknown");
        }
        if now < until {
            return if reserve_work == Some(true) || goal == PredicateTruth::False {
                Some("needs_continuation")
            } else {
                None
            };
        }
        if endpoint != Some(PredicateTruth::True) {
            return Some("reserve_unknown");
        }
    }
    if goal == PredicateTruth::True {
        Some("complete")
    } else {
        Some("goal_unmet")
    }
}

#[cfg(test)]
#[path = "selected_plan_wait_tests.rs"]
mod tests;
