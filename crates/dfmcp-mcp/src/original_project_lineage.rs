//! Exact committed continuation membership for complete-project prediction.
//! A lineage describes retained custody; it never creates dispatch authority.

use super::super::project_forecast::{MAX_COMMITTED_PROJECT_STAGES, ProjectSource};
use super::*;

fn refused(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::Conflict, message)
}

fn expected_continuation(
    source: &ProjectSource,
    parent: Digest32,
    root: Digest32,
) -> Result<GoalContinuation> {
    match source {
        ProjectSource::Production(request) => {
            goal_continuation::ProductionContinuation::new(parent, root, request.clone())
                .map(GoalContinuation::Production)
        }
        ProjectSource::Room(request) => {
            room_continuation::RoomContinuation::new(parent, root, request.clone())
                .map(GoalContinuation::Room)
        }
        ProjectSource::ConnectedRoom(request) => {
            connected_room_continuation::ConnectedRoomContinuation::new(
                parent,
                root,
                request.clone(),
            )
            .map(GoalContinuation::ConnectedRoom)
        }
    }
}

/// Return descendants in parent-before-child order, independent of the order
/// of the goal book or the lexicographic order of their sealed digests.
pub(super) fn source(
    root: &Objective,
    session: &LabSession,
) -> Result<(ProjectSource, Vec<(Digest32, Digest32)>)> {
    let source = match &root.source {
        PlanSource::Production { raw, .. } => ProjectSource::parse_production(raw)?,
        PlanSource::Blueprint { raw, .. } if room_provisioning::is_provisioned_room(raw) => {
            ProjectSource::parse_room(raw)?
        }
        PlanSource::ConnectedRoom { raw, .. } => ProjectSource::parse_connected_room(raw)?,
        _ => {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "complete_project selects ordinary production, provisioned_room or connected_room roots; select the original root for a continued project",
            ));
        }
    };
    if session.objectives.len() > MAX_RETAINED_OBJECTIVES_PER_FORTRESS
        || session.objectives.len() > session.budget.max_entities as usize
    {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete-project lineage exceeds its retained objective bound",
        ));
    }
    let mut children = BTreeMap::new();
    let mut identities = BTreeSet::from([root.plan_digest]);
    let mut root_seen = false;
    for objective in &session.objectives {
        if objective.plan_digest == root.plan_digest {
            if root_seen {
                return Err(refused(
                    "complete-project root has duplicate original custody",
                ));
            }
            root_seen = true;
            continue;
        }
        let Some(continuation) = objective.source.continuation()? else {
            continue;
        };
        if continuation.root() != root.plan_digest {
            continue;
        }
        if !objective.originating_process
            || !objective.original_handles_live
            || objective.owner_session_id != session.session_id
            || objective.abandoned(session)
            || objective.is_withdrawn()
            || objective.verification_error.is_some()
        {
            return Err(refused(
                "complete-project continuation lacks current owned, unwithdrawn live custody",
            ));
        }
        let expected = expected_continuation(&source, continuation.parent(), root.plan_digest)?;
        if !continuation.same_original_request(&expected) {
            return Err(refused(
                "complete-project continuation changed the original typed request or source family",
            ));
        }
        if !identities.insert(objective.plan_digest)
            || children
                .insert(continuation.parent(), objective.plan_digest)
                .is_some()
        {
            return Err(refused(
                "complete-project forecast requires one unambiguous linear continuation chain",
            ));
        }
        if identities.len() > MAX_COMMITTED_PROJECT_STAGES {
            return Err(DfmcpError::new(
                ErrorCode::BudgetExceeded,
                "complete project exceeds eight already committed stages",
            ));
        }
    }
    let mut ordered = Vec::with_capacity(children.len());
    let mut parent = root.plan_digest;
    while let Some(child) = children.remove(&parent) {
        let previous = session
            .objectives
            .iter()
            .find(|objective| objective.plan_digest == parent)
            .and_then(Objective::resource_plan)
            .ok_or_else(|| refused("complete-project predecessor seal is unavailable"))?;
        let next = session
            .objectives
            .iter()
            .find(|objective| objective.plan_digest == child)
            .and_then(Objective::resource_plan)
            .ok_or_else(|| refused("complete-project continuation seal is unavailable"))?;
        if previous.anchor.fortress_id != next.anchor.fortress_id
            || previous.anchor.cursor.epoch != next.anchor.cursor.epoch
            || previous.anchor.cursor.sequence > next.anchor.cursor.sequence
            || previous.anchor.tick > next.anchor.tick
        {
            return Err(refused(
                "complete-project continuation predates or crosses its predecessor's source lineage",
            ));
        }
        ordered.push((child, parent));
        parent = child;
    }
    if !root_seen || !children.is_empty() {
        return Err(refused(
            "complete-project continuation chain has a missing predecessor or a cycle",
        ));
    }
    Ok((source, ordered))
}

#[cfg(test)]
#[path = "original_project_lineage_tests.rs"]
mod tests;
