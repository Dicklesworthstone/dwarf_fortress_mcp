//! Immutable borrows for the session resource consumer. No observation,
//! objective proof, dispatch, request identity or durable postlude runs here.

use super::{LabSession, PlanSource};
use dfmcp_core::Digest32;
use dfmcp_intent::PreparedPlan;

pub(crate) struct Entry<'a> {
    pub digest: Digest32,
    pub plan: Option<&'a PreparedPlan>,
    pub retention: &'static str,
    pub source_kind: &'static str,
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
            retention: "retained_objective",
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
