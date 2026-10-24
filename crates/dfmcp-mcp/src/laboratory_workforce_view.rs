//! Bounded current scheduling decisions for the explicitly selected
//! laboratory workforce. These projections grant no execution authority.

use dfmcp_core::{EntityId, Result};
use dfmcp_intent::effects;
use dfmcp_world::WorldSnapshot;
use serde_json::{Value as Json, json};

/// A source-qualified scheduler report at exactly the supplied frontier.
/// The legacy model returns no field, preserving established response shapes.
pub(crate) fn workforce_json(snapshot: &WorldSnapshot) -> Result<Option<Json>> {
    let Some(report) = effects::laboratory_workforce_report(snapshot)? else {
        return Ok(None);
    };
    let assignments: Vec<Json> = report
        .assignments
        .iter()
        .take(super::MAX_BRIEFING_ITEMS)
        .map(assignment_json)
        .collect();
    let blocked: Vec<Json> = report
        .blocked
        .iter()
        .take(super::MAX_BRIEFING_ITEMS)
        .map(blocker_json)
        .collect();
    Ok(Some(json!({
        "model": report.model,
        "available": true,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "active_activities": report.active_activities,
        "eligible_workers": report.eligible_workers,
        "assignments": assignments,
        "assignments_omitted": report.assignments.len().saturating_sub(super::MAX_BRIEFING_ITEMS),
        "blocked": blocked,
        "blocked_omitted": report.blocked.len().saturating_sub(super::MAX_BRIEFING_ITEMS),
        "remaining_service_ticks": report.remaining_service_ticks,
        "work_units": report.work_units,
        "labor_requirements": {
            "excavation": "MINING",
            "construction_and_furniture_placement": "CONSTRUCTION",
            "brewing": "BREW",
            "cooking": "COOK",
            "wooden_furniture_production": "CARPENTRY",
        },
        "scheduling_rule": "one living eligible unit serves one activity at a time across all five labor families",
    })))
}

/// Per-action progress filters the complete admitted report before rendering;
/// a late canonical entity cannot disappear behind the briefing's list cap.
pub(crate) fn workforce_action_json(
    snapshot: &WorldSnapshot,
    entity_id: EntityId,
) -> Result<Option<Json>> {
    let Some(report) = effects::laboratory_workforce_report(snapshot)? else {
        return Ok(None);
    };
    let assignment = report
        .assignments
        .iter()
        .find(|assignment| assignment.work == entity_id);
    let blocker = report
        .blocked
        .iter()
        .find(|blocker| blocker.work == entity_id);
    Ok(Some(json!({
        "model": report.model,
        "epistemic_state": "certified_derived",
        "evidence_scope": "laboratory_reference_world",
        "source_anchor": crate::observation_projection::anchor_json(&snapshot.anchor()),
        "work_entity_id": entity_id.to_string(),
        "state": if assignment.is_some() {
            "assigned"
        } else if blocker.is_some() {
            "blocked"
        } else {
            "no_active_modeled_activity"
        },
        "assignment": assignment.map(assignment_json),
        "blocker": blocker.map(blocker_json),
    })))
}

fn assignment_json(assignment: &effects::LaboratoryWorkforceAssignment) -> Json {
    json!({
        "work_entity_id": assignment.work.to_string(),
        "worker_entity_id": assignment.worker.to_string(),
        "labor": assignment.labor,
    })
}

fn blocker_json(blocker: &effects::LaboratoryWorkforceBlocker) -> Json {
    json!({
        "work_entity_id": blocker.work.to_string(),
        "labor": blocker.labor,
        "reason": blocker.reason,
    })
}
