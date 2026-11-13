//! Immutable borrows for the session resource consumer. No observation,
//! objective proof, dispatch, request identity or durable postlude runs here.

use super::{LabSession, PlanSource};
use dfmcp_core::{Digest32, SessionId, StateAnchor};
use dfmcp_intent::PreparedPlan;

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
