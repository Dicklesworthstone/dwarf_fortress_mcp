//! Lossless current-anchor sharing in a versioned, complete review packet.
//!
//! Only declared anchor fields participate. Source programs, predicates,
//! arbitrary evidence values and unequal historical anchors remain untouched.
//! This presentation never changes the canonical owner projection or its proof.

use dfmcp_core::{DfmcpError, ErrorCode, Result};
use serde_json::{Value, json};

const REFERENCE: &str = "/current_anchor";

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

fn complete_anchor(anchor: &Value) -> bool {
    anchor.as_object().is_some_and(|fields| fields.len() == 5)
        && anchor["fortress_id"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
        && ["epoch", "sequence", "game_tick"]
            .iter()
            .all(|field| anchor[*field].as_u64().is_some())
        && anchor["state_hash"].as_str().is_some_and(|hash| {
            hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
}

fn slot(
    value: &mut Value,
    field: &str,
    visit: &mut impl FnMut(&mut Value) -> Result<()>,
) -> Result<()> {
    if let Some(anchor) = value.get_mut(field) {
        visit(anchor)?;
    }
    Ok(())
}

/// Every eligible location is named explicitly. In particular, an anchor-shaped
/// value inside a source request or predicate remains literal semantic data.
fn visit_anchors(
    active_work: &mut Value,
    objectives: &mut Value,
    visit: &mut impl FnMut(&mut Value) -> Result<()>,
) -> Result<()> {
    if !active_work.is_object() {
        return Err(invariant(
            "anchor sharing requires an owned work projection",
        ));
    }
    for field in ["pending_plans", "actions"] {
        if let Some(rows) = active_work.get_mut(field) {
            let rows = rows
                .as_array_mut()
                .ok_or_else(|| invariant("anchor sharing requires complete work arrays"))?;
            for row in rows {
                if field == "pending_plans" {
                    slot(row, "anchor", visit)?;
                } else if let Some(work) = row.get_mut("work_state") {
                    slot(work, "observed_anchor", visit)?;
                    for domain in [
                        "workforce",
                        "construction_materials",
                        "provisions",
                        "harvesting",
                    ] {
                        if let Some(projection) = work.get_mut(domain) {
                            slot(projection, "source_anchor", visit)?;
                        }
                    }
                }
            }
        }
    }
    if let Some(work) = active_work.get_mut("untracked_work") {
        slot(work, "observed_anchor", visit)?;
    }
    let objectives = objectives
        .as_array_mut()
        .ok_or_else(|| invariant("anchor sharing requires complete objective rows"))?;
    for objective in objectives {
        for field in [
            "sealed_anchor",
            "observed_anchor",
            "first_satisfied_anchor",
            "recorded_first_satisfied_anchor",
            "restore_abandoned_anchor",
        ] {
            slot(objective, field, visit)?;
        }
        for field in ["semantic_room_goal", "semantic_output_goal"] {
            if let Some(goal) = objective.get_mut(field) {
                slot(goal, "observed_anchor", visit)?;
            }
        }
        if let Some(retired) = objective.get_mut("retired_undispatched_steps") {
            let retired = retired
                .as_array_mut()
                .ok_or_else(|| invariant("anchor sharing requires complete retired-step rows"))?;
            for step in retired {
                slot(step, "proof_anchor", visit)?;
            }
        }
    }
    Ok(())
}

/// Return the encoding declaration only after both complete views can be shared.
/// An error leaves both supplied views intact. The enclosing publisher must keep
/// the exact complete `anchor` at `/current_anchor` in the same response.
pub(crate) fn apply(
    active_work: &mut Value,
    objectives: &mut Value,
    anchor: &Value,
) -> Result<Value> {
    if !complete_anchor(anchor) {
        return Err(invariant(
            "shared review requires a complete canonical anchor",
        ));
    }
    let mut work = active_work.clone();
    let mut goals = objectives.clone();
    let mut references = 0_usize;
    visit_anchors(&mut work, &mut goals, &mut |value| {
        if value == anchor {
            *value = json!({"$ref": REFERENCE});
            references += 1;
        } else if value.get("$ref").is_some() {
            return Err(invariant(
                "owned anchor projection already contains a reference",
            ));
        }
        Ok(())
    })?;
    *active_work = work;
    *objectives = goals;
    Ok(json!({
        "schema": "dfmcp.current-anchor-references/1",
        "scope": "same_response",
        "json_pointer": REFERENCE,
        "complete": true,
        "reference_count": references,
    }))
}

#[cfg(test)]
#[path = "current_anchor_review_tests.rs"]
mod tests;
