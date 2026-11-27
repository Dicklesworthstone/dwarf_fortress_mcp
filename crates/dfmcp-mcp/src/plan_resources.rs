//! Immutable borrows for the session resource consumer. No observation,
//! objective proof, dispatch, request identity or durable postlude runs here.

use super::{LabSession, PlanSource};
use dfmcp_core::{Digest32, PlanId, SessionId, StateAnchor, StepId};
use dfmcp_intent::{EffectWorkState, PlanStep, PreparedPlan, RecoveredObligation};

pub(crate) struct Entry<'a> {
    pub digest: Digest32,
    pub plan: Option<&'a PreparedPlan>,
    pub retention: &'static str,
    pub source_kind: &'static str,
}

/// Borrowed original-goal custody. These fields are retained evidence, not a
/// fresh evaluation of the goal or a permission to resume its old actions.
pub(crate) struct GoalEntry<'a> {
    pub digest: Digest32,
    pub plan: Option<&'a PreparedPlan>,
    pub source_kind: &'static str,
    pub summary: &'a str,
    pub request_json: Option<&'a str>,
    pub paused_target: Option<bool>,
    pub owner_session_id: SessionId,
    pub owned_by_current_session: bool,
    pub sealed_state_hash: Digest32,
    pub sealed_anchor: Option<StateAnchor>,
    pub first_satisfied_anchor: Option<StateAnchor>,
    pub historical_proof_verified: bool,
    pub restore_abandoned_anchor: Option<StateAnchor>,
    pub withdrawal: Option<&'a dfmcp_lab::durable::DurableObjectiveWithdrawal>,
    pub abandonment_publication_pending: bool,
    pub verification_error: Option<&'a str>,
}

pub(crate) fn goal_count(session: &LabSession) -> usize {
    session.objectives.len()
}

pub(crate) fn goal(session: &LabSession, digest: Digest32) -> Option<GoalEntry<'_>> {
    session
        .objectives
        .iter()
        .find(|objective| objective.plan_digest == digest)
        .map(|objective| objective.resource_goal(session))
}

pub(crate) fn action_history(
    session: &LabSession,
    digest: Digest32,
    admitted_seal: Option<dfmcp_intent::plan_archive::PlanArchiveSeal>,
) -> Option<dfmcp_core::Result<super::selected_plan_wait::ArchivedDispositions<'_>>> {
    session
        .objectives
        .iter()
        .find(|objective| objective.plan_digest == digest)?
        .resource_action_history(admitted_seal)
}

pub(crate) fn count(session: &LabSession) -> usize {
    session.objectives.len() + usize::from(session.pending.is_some())
}

pub(crate) fn entries(session: &LabSession) -> impl Iterator<Item = Entry<'_>> {
    session
        .pending
        .iter()
        .map(|pending| Entry {
            digest: pending.plan.digest,
            plan: Some(&pending.plan),
            retention: "pending_plan",
            source_kind: source_kind(&pending.source),
        })
        .chain(session.objectives.iter().map(|objective| Entry {
            digest: objective.plan_digest,
            plan: objective.resource_plan(),
            retention: if objective.is_withdrawn() {
                "withdrawn_objective"
            } else {
                "retained_objective"
            },
            source_kind: objective.resource_source_kind(),
        }))
}

/// Retained action receipts and recovered original steps are distinct sources.
/// Recovery does not manufacture an ActionId or restore effect authority.
pub(crate) enum WorkEntry<'a> {
    Live {
        plan_digest: Option<Digest32>,
        plan_id: Option<PlanId>,
        receipt: &'a dfmcp_adapter::ActionReceipt,
    },
    Recovered {
        plan_digest: Digest32,
        step: Option<StepId>,
        kind: &'static str,
        state: &'a str,
        recorded_state: &'a str,
        proof_anchor: Option<StateAnchor>,
        work_anchor: Option<StateAnchor>,
        work_state: &'a EffectWorkState,
        work_step: Option<&'a PlanStep>,
        monitor: Option<&'a RecoveredObligation>,
        observation_error: Option<&'a str>,
        failure_reason: Option<&'a str>,
        physical_drain: Option<&'a dfmcp_lab::RecoveredWorkDrainReceipt>,
    },
}

pub(crate) fn work_count(session: &LabSession) -> usize {
    session
        .adapter
        .retained_action_ids()
        .count()
        .saturating_add(session.carried.len())
}

pub(crate) fn work_world_source(
    session: &LabSession,
    context: &dfmcp_core::OperationContext,
) -> dfmcp_core::Result<serde_json::Value> {
    super::query_budget::admit_sources(&[session.adapter.snapshot()], context)
}

pub(crate) fn recovered_monitor_matches(
    monitor: &RecoveredObligation,
    plan: &PreparedPlan,
    step: &PlanStep,
) -> bool {
    let Some(mut spec) = step.obligation.clone() else {
        return false;
    };
    let mut predicates = step.postconditions.clone();
    predicates.push(spec.terminal.clone());
    spec.terminal = dfmcp_world::Predicate::All(predicates).normalized();
    monitor.matches_original(
        super::recovered_step_progress::proof_id(plan.digest, step.id),
        &spec,
        plan.anchor.tick,
    )
}

/// Immutable source enumeration only: no poll, monitor observation, goal
/// evaluation, durable postlude, or open-action refresh occurs here.
pub(crate) fn work_entries(session: &LabSession) -> impl Iterator<Item = WorkEntry<'_>> {
    session
        .adapter
        .retained_action_ids()
        .filter_map(|id| {
            let receipt = session.adapter.action_receipt(id)?;
            let commit = session.adapter.action_plan_receipt(id);
            Some(WorkEntry::Live {
                plan_digest: commit.map(|commit| commit.plan_digest),
                plan_id: commit.map(|commit| commit.plan_id),
                receipt,
            })
        })
        .chain(session.carried.iter().map(|row| WorkEntry::Recovered {
            plan_digest: row.plan_digest,
            step: row.step,
            kind: row.kind,
            state: &row.state,
            recorded_state: &row.recorded_state,
            proof_anchor: row.proof_anchor,
            work_anchor: row.work_anchor,
            work_state: &row.work_state,
            work_step: row.work_step.as_ref(),
            monitor: row.monitor.as_ref(),
            observation_error: row.observation_error.as_deref(),
            failure_reason: row.failure_reason.as_deref(),
            physical_drain: row.recovered_drain.as_ref(),
        }))
}

pub(super) fn source_kind(source: &PlanSource) -> &'static str {
    match source {
        PlanSource::Pause { .. } => "pause",
        PlanSource::Actions { .. } => "actions",
        PlanSource::Blueprint { .. } => "blueprint",
        PlanSource::Production { .. } => "production",
        PlanSource::ProductionContinuation { .. } => "production_continuation",
        PlanSource::RoomContinuation { .. } => "room_continuation",
        PlanSource::CivilianRelease { .. } => "civilian_release",
        PlanSource::ConnectedRoom { .. } => "connected_room",
        PlanSource::ConnectedRoomContinuation { .. } => "connected_room_continuation",
        PlanSource::ResourceAcquisitionContinuation { .. } => "resource_acquisition_continuation",
    }
}

#[cfg(test)]
#[path = "resources/goals/session_tests.rs"]
mod goal_resource_tests;

#[cfg(test)]
#[path = "resources/work/session_tests.rs"]
mod work_resource_tests;
