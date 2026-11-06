//! Presentation and exact action binding for explicit laboratory setup staffing.

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use dfmcp_intent::workforce_planning::{ProjectSetupLabor, ProjectSetupStaffing};
use dfmcp_intent::{Action, Intent, RequestedAction};
use serde_json::{Value, json};

fn invalid(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

pub(crate) fn dependency(staffing: &ProjectSetupStaffing, labor: ProjectSetupLabor) -> Option<u32> {
    staffing
        .selections
        .iter()
        .find(|choice| choice.labor == labor)
        .and_then(|choice| choice.action_index)
}

pub(crate) fn report(staffing: &ProjectSetupStaffing) -> Value {
    json!({
        "policy": crate::lab_world::PROJECT_WORKFORCE_POLICY,
        "model": dfmcp_intent::effects::LABORATORY_WORKFORCE_MODEL,
        "source_state_hash": staffing.anchor.state_hash.to_hex(),
        "source_tick": staffing.anchor.tick.0,
        "actions_added": staffing.actions.len(),
        "selection_policy": "reuse qualified enabled labor, then prefer known nonmilitary workers and canonical identities; sequential stages may share a worker",
        "worker_reservations_created": false,
        "completion_guaranteed": false,
        "selections": staffing.selections.iter().map(|choice| json!({
            "labor": choice.labor.token(), "unit": choice.worker.get().to_string(),
            "generation": choice.generation, "previously_enabled": choice.previously_enabled,
            "evidence": if choice.previously_enabled { "observed" } else { "planned" },
            "action_index": choice.action_index,
        })).collect::<Vec<_>>(),
    })
}

pub(crate) fn action_json(requested: &RequestedAction) -> Result<Value> {
    let Action::SetLabor {
        units,
        labor,
        enabled: true,
    } = &requested.action
    else {
        return Err(invalid(
            "project setup escaped its closed labor-enable action family",
        ));
    };
    Ok(json!({"action": {
        "kind":"set_labor", "units":units.iter().map(|id| id.get().to_string()).collect::<Vec<_>>(),
        "labor":labor, "enabled":true,
    }, "depends_on":requested.depends_on}))
}

/// The public action grammar carries no arbitrary predicates. Install the
/// source-qualified guards only on the exact generated prefix, after the
/// production horizon has verified its complete parsed action program.
pub(crate) fn bind_actions(staffing: &ProjectSetupStaffing, intent: &mut Intent) -> Result<()> {
    if staffing.anchor != intent.anchor || intent.requested_actions.len() < staffing.actions.len() {
        return Err(invalid(
            "project staffing belongs to another source or action program",
        ));
    }
    for (current, expected) in intent.requested_actions.iter().zip(&staffing.actions) {
        if current.action != expected.action
            || current.depends_on != expected.depends_on
            || !current.preconditions.is_empty()
            || !current.postconditions.is_empty()
            || current.compensation.is_some()
            || current.obligation.is_some()
        {
            return Err(invalid(
                "project staffing cannot replace a different or already-bound action",
            ));
        }
    }
    for (current, expected) in intent.requested_actions.iter_mut().zip(&staffing.actions) {
        *current = expected.clone();
    }
    Ok(())
}

pub(crate) fn require_dependency(requested: &mut RequestedAction, dependency: Option<u32>) {
    if let Some(index) = dependency {
        requested.depends_on.push(index);
        requested.depends_on.sort_unstable();
        requested.depends_on.dedup();
    }
}

/// Procurement reports use indices in the complete room program too.
pub(crate) fn shift_production_report(report: &mut Value, offset: u32) -> Result<()> {
    for section in ["staffing", "workshops"] {
        if let Some(rows) = report
            .get_mut("prerequisites")
            .and_then(|setup| setup.get_mut(section))
            .and_then(Value::as_array_mut)
        {
            for row in rows {
                if let Some(index) = row["action_index"].as_u64() {
                    row["action_index"] = json!(
                        index
                            .checked_add(u64::from(offset))
                            .ok_or_else(|| invalid("project staffing report index overflow"))?
                    );
                }
            }
        }
    }
    // A room's complete shared-workforce schedule now includes excavation and
    // installation. Its procurement-only preview is not that final horizon.
    if let Some(object) = report.as_object_mut() {
        object.remove("consumption");
        object.insert("deadline_scope".to_owned(), json!("complete_room_workload"));
    }
    Ok(())
}
