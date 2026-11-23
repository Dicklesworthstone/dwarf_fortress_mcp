//! Guidance for the actual Manager/1.23 child of the fixed resource project.
//! Appointment receipts and exported custody never establish current production.
use super::{RESPONSE_BYTES, recommendation};
use dfmcp_core::{Digest32, SessionId};
use serde_json::{Value, json};

pub(super) fn active(result: &Value) -> bool {
    result["stage"] == "manager" || result["child"]["source_profile"] == "manager/1.23"
}

pub(super) fn in_stage(result: &Value, reference: &Value) -> bool {
    let stage = result
        .get("stage")
        .unwrap_or(&reference["retained_stage"]["stage"]);
    active(result)
        || matches!(
            stage.as_str(),
            Some("manager_opening" | "manager" | "manager_transition")
        )
        || (stage == "cancelled"
            && matches!(
                reference["retained_stage"]["cancelled_from"].as_str(),
                Some("manager_opening" | "manager" | "manager_transition")
            ))
}

pub(super) fn verified(result: &Value) -> bool {
    result["ok"] == true
        && result["project_custody_currently_verified"] == true
        && result["child"]["ok"] == true
        && result["child"]["source_profile"] == "manager/1.23"
        && result["child"]["retained_state_currently_verified"] == true
        && result["mode"] == result["child"]["mode"]
}

fn digest(value: &Value) -> Option<&str> {
    let raw = value.as_str()?;
    Digest32::from_hex(raw).filter(|digest| *digest != Digest32::ZERO && digest.to_hex() == raw)?;
    Some(raw)
}

pub(super) fn source_fenced(child: &Value) -> bool {
    child["project_source_frontier"]["fenced"] == true
        || child["production_handoff_floor"]["fenced"] == true
        || child["custody"]["production_handoff_floor"]["fenced"] == true
}

pub(super) fn uncertain(child: &Value) -> bool {
    child["uncertain_effect"] == true
        || child["custody"]["uncertain_effect"] == true
        || child["original"]["unresolved"] == true
}

fn high_tick(child: &Value) -> Option<u64> {
    [
        "/consumed_tick_floor",
        "/custody/consumed_tick_floor",
        "/project_source_frontier/high_tick",
        "/production_handoff_floor/tick",
        "/custody/production_handoff_floor/tick",
        "/observation/tick",
        "/original/original_source/tick",
        "/original/after/tick",
    ]
    .into_iter()
    .filter_map(|pointer| child.pointer(pointer).and_then(Value::as_u64))
    .max()
}

fn deadline_current(result: &Value) -> bool {
    result["original"]["goal"]["deadline_tick"]
        .as_u64()
        .is_some_and(|deadline| high_tick(&result["child"]).is_some_and(|tick| tick < deadline))
}

/// This summary retains immutable identity from either a current child view or
/// a recovery reference. It deliberately contains no reusable review permission.
pub(super) fn original(child: &Value) -> Option<Value> {
    let current = &child["original"];
    let key = current["key"]
        .as_str()
        .or_else(|| child["original_key"].as_str())?;
    Some(json!({
        "key":key,
        "plan_digest":current.get("plan_digest").unwrap_or(&child["original_plan_digest"]),
        "deadline_tick":current.get("deadline_tick").unwrap_or(&child["project_deadline_tick"]),
        "state":current["state"],
        "native_phase":current.get("native_phase").unwrap_or(&child["native_phase"]),
        "dispatch_started":child.get("dispatch_started").unwrap_or(&child["custody"]["dispatch_started"]),
        "uncertain_effect":uncertain(child),
        "automatic_redispatch":false,
    }))
}

pub(super) fn review(result: &Value) -> Option<(&'static str, Value)> {
    let child = &result["child"];
    let original = &child["original"];
    if !verified(result)
        || result["mode"] != "control"
        || result["stage"] != "manager"
        || result["project_cancelled"] == true
        || result["session_closed"] == true
        || source_fenced(child)
        || uncertain(child)
        || !deadline_current(result)
        || original["deadline_tick"] != result["original"]["goal"]["deadline_tick"]
        || original["prepared_in_this_connection"] != true
        || original["state"] != "Prepared"
        || original["native_phase"] != 0
        || original["appointment_proved"] != false
    {
        return None;
    }
    Some(("goal", json!(digest(&original["review_seal"])?)))
}

pub(super) fn observed_plan(result: &Value) -> Option<Value> {
    let child = &result["child"];
    let observed = &child["observation"];
    let deadline = result["original"]["goal"]["deadline_tick"].as_u64()?;
    if !verified(result)
        || result["mode"] != "control"
        || result["stage"] != "manager"
        || result["project_cancelled"] == true
        || result["session_closed"] == true
        || !child["original"].is_null()
        || observed["eligible_at_anchor"] != true
        || observed["paused"] != true
        || source_fenced(child)
        || uncertain(child)
        || !deadline_current(result)
        || deadline > child["recommended_deadline_tick"].as_u64()?
    {
        return None;
    }
    Some(json!({
        "kind":"appoint_vacant_manager",
        "expected_observation":digest(&observed["witness"])?,
        "deadline_tick":deadline,
    }))
}

fn forward_current(result: &Value, reference: &Value) -> bool {
    let Some(deadline) = result["original"]["goal"]["deadline_tick"].as_u64() else {
        return false;
    };
    [
        reference["authority_tick_floor"].as_u64(),
        reference["parent_acknowledged"]["authority_tick_floor"].as_u64(),
        high_tick(&result["child"]),
    ]
    .into_iter()
    .flatten()
    .max()
    .is_some_and(|tick| tick < deadline)
}

fn handoff_ready(result: &Value, reference: &Value) -> bool {
    let child = &result["child"];
    let original = &child["original"];
    verified(result)
        && forward_current(result, reference)
        && !source_fenced(child)
        && !uncertain(child)
        && child["production_handoff_available"] == true
        && original["appointment_proved"] == true
        && original["production_manager_cache_proved"] == true
        && original["state"] == "Terminal"
        && original["native_phase"] == 2
        && original["prepared_in_this_connection"] == false
        && original["deadline_tick"] == result["original"]["goal"]["deadline_tick"]
}

pub(super) fn next_step(
    result: &Value,
    reference: &Value,
    session: SessionId,
) -> Option<Vec<Value>> {
    if !in_stage(result, reference) {
        return None;
    }
    if result["session_closed"] == true {
        return Some(Vec::new());
    }
    let stage = result
        .get("stage")
        .unwrap_or(&reference["retained_stage"]["stage"]);
    let child = &result["child"];
    let original = &child["original"];
    let cancelled = result["project_cancelled"] == true || stage == "cancelled";
    let mut arguments = json!({"session_id":session.to_string()});
    let mut confirmation = false;
    let (tool, reason) = if result["ok"] != true || reference["controller_fenced"] == true {
        (
            "fortress_query",
            "Inspect the exact parent and Manager custody before continuing. A failed publication restores no appointment permission or current production readiness.",
        )
    } else if result["mode"] == "offline" {
        (
            "fortress_query",
            "Inspect the retained original Manager identity and independent acknowledgement. Offline custody cannot observe a current Manager or enter another stage.",
        )
    } else if cancelled {
        if uncertain(child) {
            arguments["scope"] = json!("goal");
            (
                "fortress_wait",
                "Reconcile only the retained Manager key after the durable project stop. No new appointment or later project stage can begin.",
            )
        } else {
            (
                "fortress_explain",
                "Inspect the stopped project's original Manager custody. Cancellation does not remove an appointed Manager, undo history or complete the stock goal.",
            )
        }
    } else if stage == "manager_transition" {
        if result["mode"] == "control"
            && result["project_custody_currently_verified"] == true
            && reference["retained_stage"]["checkpoint"]["manager_handoff"].is_object()
            && forward_current(result, reference)
            && let Some(head) = digest(&reference["parent_acknowledged"]["head"])
        {
            arguments["request"] =
                json!(json!({"kind":"advance_from_manager","expected_head":head}).to_string());
            (
                "fortress_plan",
                "Continue the acknowledged Manager transfer into the original next resource stage. The later production owner independently imports and freshly qualifies this exact Manager.",
            )
        } else {
            (
                "fortress_query",
                "Inspect the exact acknowledged Manager handoff and unchanged project deadline. Recover mode cannot initialize a later child or restore appointment authority.",
            )
        }
    } else if stage == "manager_opening" {
        (
            "fortress_query",
            "Inspect the retained Manager opening marker and existing custody. An interrupted opening cannot be initialized again or replaced with another original.",
        )
    } else if child["opened"] == true && result["project_custody_currently_verified"] == true {
        (
            "fortress_observe",
            "Observe the original configured Manager candidate and vacancy before reviewing an appointment. Opening custody is not a commit permission.",
        )
    } else if !verified(result) {
        (
            "fortress_query",
            "Verify the held original Manager journal and acknowledgement before using any observation, review or retained appointment proof.",
        )
    } else if let Some((scope, seal)) = review(result) {
        confirmation = true;
        arguments["scope"] = json!(scope);
        arguments["review_seal"] = seal;
        arguments["confirmed"] = json!(true);
        (
            "fortress_commit",
            "Review the exact citizen/HF, vacant assignment, preserved flags, POSITION link, one history event and MANAGE_PRODUCTION cache change, then confirm this current original seal once.",
        )
    } else if result["mode"] == "control" && handoff_ready(result, reference) {
        if let Some(head) = digest(&reference["parent_acknowledged"]["head"]) {
            arguments["request"] =
                json!(json!({"kind":"advance_from_manager","expected_head":head}).to_string());
            (
                "fortress_plan",
                "Freshly verify the original Manager assignment and cache, retain its exact handoff, and enter the next configured resource stage. Historical appointment alone does not prove present production readiness.",
            )
        } else {
            (
                "fortress_query",
                "Retain the exact current parent acknowledgement before transferring the original Manager stage.",
            )
        }
    } else if source_fenced(child) {
        (
            "fortress_explain",
            "Inspect the original Manager source discontinuity and retained rejection floors. A fenced source cannot be repaired by a repeated read, replacement appointment or stage advance.",
        )
    } else if uncertain(child) {
        arguments["scope"] = json!("goal");
        (
            "fortress_wait",
            "Reconcile the original Manager key once. A missing or uncertain result cannot authorize another appointment or a later project stage.",
        )
    } else if original.is_object() {
        if original["state"] == "Prepared"
            && original["prepared_in_this_connection"] == false
            && result["mode"] == "control"
        {
            arguments["scope"] = json!("project");
            (
                "fortress_cancel",
                "Retire the original unattempted Manager preparation and stop this project. Its lost connection cannot be renewed from a retained seal.",
            )
        } else {
            (
                "fortress_explain",
                "Inspect the original Manager outcome, source fence or fixed-deadline blocker. Retained receipt and cache evidence cannot renew the project or replace its original appointment.",
            )
        }
    } else if let Some(request) = observed_plan(result) {
        arguments["request"] = json!(request.to_string());
        (
            "fortress_plan",
            "Prepare the observed eligible current Expedition Leader for the existing vacant MANAGER assignment under the project's unchanged absolute deadline.",
        )
    } else if child["observation"].is_object() || source_fenced(child) {
        (
            "fortress_explain",
            "Inspect the observed Manager eligibility, source continuity or fixed-deadline blocker before planning. The project cannot substitute another candidate or extend its original deadline.",
        )
    } else {
        (
            "fortress_observe",
            "Obtain the configured Manager candidate's bounded assignment, HF, history, getter and responsibility-cache observation before planning.",
        )
    };
    let mut next = recommendation(
        "continue_original_manager_project_stage",
        tool,
        reason,
        "high",
        "high",
        if confirmation {
            "irreversible"
        } else {
            "read_only"
        },
        if confirmation {
            "no automatic removal or rollback of appointment and history"
        } else {
            "not_applicable"
        },
        confirmation,
        arguments,
    );
    next["estimated_cost"] = json!({"game_ticks":0,
        "native_writer_calls_max":u8::from(confirmation),"output_bytes_max":RESPONSE_BYTES});
    next["grants_authority"] = json!(false);
    next["confidence"]["evidence"] = json!([{
        "parent_acknowledged":reference["parent_acknowledged"],
        "complete_child_custody_pointer":"/retained_reference/child_reference",
    }]);
    next["invalidating_conditions"] = json!([
        "original Manager source, generation, citizen/HF, vacancy, history or cache changes",
        "original project deadline, authority, lease or preparation expires",
        "the current prepared connection or complete review is lost",
        "parent or Manager journal and independent acknowledgement change",
    ]);
    Some(vec![next])
}

#[cfg(test)]
#[path = "manager_presentation_tests.rs"]
mod tests;
