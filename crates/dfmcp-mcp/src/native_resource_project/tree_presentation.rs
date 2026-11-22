//! Guidance for the actual Tree child of the immutable stock project.
//! Published source offers select requests; only the child owners grant effects.
use super::{RESPONSE_BYTES, recommendation};
use dfmcp_core::{Digest32, SessionId};
use serde_json::{Value, json};

pub(super) fn active(result: &Value) -> bool {
    result["stage"] == "tree" || result["child"]["source_profile"] == "tree-harvest/1.21"
}

fn verified(result: &Value) -> bool {
    result["ok"] == true
        && result["child"]["source_profile"] == "tree-harvest/1.21"
        && result["child"]["retained_state_currently_verified"] == true
        && result["mode"] == result["child"]["mode"]
}

fn digest(value: &Value) -> Option<&str> {
    let raw = value.as_str()?;
    Digest32::from_hex(raw).filter(|digest| *digest != Digest32::ZERO && digest.to_hex() == raw)?;
    Some(raw)
}

fn unsettled(clock: &Value) -> bool {
    clock["pending"] == true
        || clock["unresolved"] == true
        || matches!(
            clock["active_run"]["state"].as_str(),
            Some(
                "intent_recorded"
                    | "prepared"
                    | "dispatch_started"
                    | "tracking"
                    | "cancel_requested"
            )
        )
}

/// A recovery seal is data. Confirmation requires the actual current owner,
/// current preparation and a separate clock scope when the review is a run.
pub(super) fn review(result: &Value) -> Option<(&'static str, Value)> {
    if !verified(result)
        || result["mode"] != "control"
        || result["project_cancelled"] == true
        || result["stage"] == "cancelled"
        || result["child"]["project_source_frontier"]["fenced"] == true
        || result["session_closed"] == true
    {
        return None;
    }
    let child = &result["child"];
    let clock = &child["clock"];
    if clock.is_object() {
        if clock["verified"] != true {
            return None;
        }
        if clock["commit_available"] == true
            && clock["pending"] == true
            && clock["unresolved"] == false
            && clock["advancement_fenced"] != true
            && clock["active_run"]["state"] == "prepared"
        {
            return Some(("clock", json!(digest(&clock["active_run"]["review_seal"])?)));
        }
        if unsettled(clock) {
            return None;
        }
    }
    let original = &child["original"];
    if original["prepared_in_this_connection"] == true
        && original["sample_publication_pending"] == false
        && original["continuity_lost"] == false
    {
        return Some(("goal", json!(digest(&original["review_seal"])?)));
    }
    None
}

pub(super) fn observed_plan(result: &Value) -> Option<Value> {
    if !verified(result) || result["mode"] != "control" {
        return None;
    }
    let child = &result["child"];
    let observation = &child["observation"];
    let deadline = result["original"]["goal"]["deadline_tick"].as_u64()?;
    let tick = observation["tick"].as_u64()?;
    if !child["original"].is_null()
        || observation["eligible_at_anchor"] != true
        || observation["visible_domain_complete"] != true
        || child["project_source_frontier"]["fenced"] == true
        || deadline <= tick
        || deadline > child["recommended_deadline_tick"].as_u64()?
    {
        return None;
    }
    Some(json!({
        "kind":"fell_one_visible_tree",
        "expected_observation":digest(&observation["witness"])?,
        "deadline_tick":deadline,
    }))
}

fn ready_to_handoff(result: &Value, reference: &Value) -> bool {
    let child = &result["child"];
    let original = &child["original"];
    let clock = &child["clock"];
    let Some(deadline) = result["original"]["goal"]["deadline_tick"].as_u64() else {
        return false;
    };
    let high_tick = [
        &reference["authority_tick_floor"],
        &reference["parent_acknowledged"]["authority_tick_floor"],
        &child["custody"]["consumed_tick_floor"],
        &child["project_source_frontier"]["high_tick"],
        &original["historical_supply_observed_tick"],
        &original["latest_progress"]["tick"],
        &clock["high_tick"],
        &clock["consumed_tick_floor"],
    ]
    .into_iter()
    .filter_map(Value::as_u64)
    .max();
    verified(result)
        && high_tick.is_some_and(|tick| tick < deadline)
        && child["project_source_frontier"]["fenced"] != true
        && original["historical_supply_observed_tick"].is_u64()
        && original["designation_acknowledged"] == true
        && original["native_phase"] == 2
        && original["deadline_tick"] == result["original"]["goal"]["deadline_tick"]
        && original["unresolved"] == false
        && original["sample_publication_pending"] == false
        && original["continuity_lost"] == false
        && original["prepared_in_this_connection"] == false
        && (clock.is_null()
            || (clock["verified"] == true
                && clock["pending"] == false
                && clock["unresolved"] == false
                && clock["advancement_fenced"] != true
                && clock["commit_available"] != true
                && !unsettled(clock)))
}

fn run_request(result: &Value) -> Option<Value> {
    let child = &result["child"];
    let clock = &child["clock"];
    let original = &child["original"];
    let deadline = result["original"]["goal"]["deadline_tick"].as_u64()?;
    let ticks = clock["recommended_game_ticks"].as_u64()?;
    if !verified(result)
        || result["mode"] != "control"
        || clock["verified"] != true
        || clock["new_run_available"] != true
        || clock["clock_ready"] != true
        || clock["advancement_fenced"] == true
        || child["project_source_frontier"]["fenced"] == true
        || unsettled(clock)
        || original["native_phase"] != 2
        || !original["historical_supply_observed_tick"].is_null()
        || original["continuity_lost"] != false
        || original["sample_publication_pending"] != false
        || original["unresolved"] != false
        || original["deadline_tick"].as_u64()? != deadline
        || clock["deadline_tick"].as_u64()? != deadline
        || !(1..=1200).contains(&ticks)
        || ticks > child["max_game_ticks"].as_u64()?
        || ticks > clock["remaining_horizon_ticks"].as_u64()?
        || clock["remaining_tree_samples"].as_u64()? < 3
        || result["budget"]["max_game_ticks"]
            .as_u64()
            .is_some_and(|maximum| ticks > maximum)
    {
        return None;
    }
    Some(
        json!({"kind":"run_tree", "target_id":digest(&clock["target_id"])?,
        "game_ticks":ticks,"wall_millis":1000}),
    )
}

fn next(
    tool: &str,
    reason: &str,
    arguments: Value,
    reference: &Value,
    confirmation: bool,
    writer_calls: u8,
    ticks: u64,
) -> Value {
    let designation = confirmation && arguments["scope"] == "goal";
    let mut value = recommendation(
        "continue_original_tree_project_stage",
        tool,
        reason,
        "high",
        "high",
        if designation {
            "irreversible"
        } else if confirmation || writer_calls > 0 || tool == "fortress_cancel" {
            "guarded"
        } else {
            "read_only"
        },
        if designation {
            "deferred felling may be irreversible; no automatic undo"
        } else if confirmation {
            "ordinary world evolution; no automatic undo"
        } else if writer_calls > 0 {
            "stops future clock exposure without undoing elapsed time"
        } else {
            "not_applicable"
        },
        confirmation,
        arguments,
    );
    value["estimated_cost"] = json!({"game_ticks":ticks,
        "native_writer_calls_max":writer_calls,"output_bytes_max":RESPONSE_BYTES});
    value["grants_authority"] = json!(false);
    value["confidence"]["evidence"] = json!([{
        "parent_acknowledged":reference["parent_acknowledged"],
        "complete_child_custody_pointer":"/retained_reference/child_reference",
    }]);
    value["invalidating_conditions"] = json!([
        "original Tree source, root, body, worker or tool offer changes",
        "original deadline, authority or lease expires",
        "the prepared connection or current review is lost",
        "parent, Tree or clock custody changes",
    ]);
    value
}

/// Return None outside the Tree stages so existing Workshop/production
/// guidance stays with those actual owners.
pub(super) fn next_step(
    result: &Value,
    reference: &Value,
    session: SessionId,
) -> Option<Vec<Value>> {
    let stage = result
        .get("stage")
        .unwrap_or(&reference["retained_stage"]["stage"]);
    let from = &reference["retained_stage"]["cancelled_from"];
    if !active(result)
        && !matches!(
            stage.as_str(),
            Some("tree_opening" | "tree" | "tree_transition")
        )
        && !(stage == "cancelled"
            && matches!(
                from.as_str(),
                Some("tree_opening" | "tree" | "tree_transition")
            ))
    {
        return None;
    }
    if result["session_closed"] == true {
        return Some(Vec::new());
    }
    let mut arguments = json!({"session_id":session.to_string()});
    let child = &result["child"];
    let original = &child["original"];
    let clock = &child["clock"];
    let cancelled = result["project_cancelled"] == true || stage == "cancelled";
    let mut confirmation = false;
    let mut writer_calls = 0;
    let mut ticks = 0;
    let mut safety_stop = false;
    let (tool, reason) = if result["ok"] != true || reference["controller_fenced"] == true {
        (
            "fortress_query",
            "Inspect the retained parent, Tree and clock custody before continuing; a failed publication restores no review or current supply proof.",
        )
    } else if result["mode"] == "offline" {
        (
            "fortress_query",
            "Inspect the exact local Tree and clock histories. Offline mode cannot acquire a new supply sample or enter another stage.",
        )
    } else if stage == "tree_transition" && !cancelled {
        if result["mode"] == "control"
            && reference["retained_stage"]["checkpoint"]["tree_handoff"].is_object()
            && let Some(head) = digest(&reference["parent_acknowledged"]["head"])
        {
            arguments["request"] =
                json!(json!({"kind":"advance_to_workshop", "expected_head":head}).to_string());
            (
                "fortress_plan",
                "Continue the acknowledged Tree handoff into the original Workshop stage. The importer will independently qualify current material before planning construction.",
            )
        } else {
            (
                "fortress_query",
                "Inspect the exact acknowledged Tree transition and its retained handoff before reopening Workshop custody.",
            )
        }
    } else if stage == "tree"
        && !cancelled
        && result["project_custody_currently_verified"] == true
        && child["opened"] == true
        && reference["child_reference"]["project_source_frontier"].is_object()
        && reference["child_reference"]["original"].is_null()
    {
        (
            "fortress_observe",
            "Observe the newly opened original Tree child before choosing its source-bound designation. Opening custody supplies no native observation or review.",
        )
    } else if !verified(result) || (clock.is_object() && clock["verified"] != true) {
        (
            "fortress_query",
            "Verify the held Tree and clock custody before using an observation, review or historical completion.",
        )
    } else if let Some((scope, seal)) = review(result) {
        confirmation = true;
        writer_calls = 1;
        ticks = if scope == "clock" {
            clock["active_run"]["game_ticks"].as_u64().unwrap_or(0)
        } else {
            0
        };
        arguments["scope"] = json!(scope);
        arguments["review_seal"] = seal;
        arguments["confirmed"] = json!(true);
        (
            "fortress_commit",
            "Review the complete current Tree or clock proposal and confirm this exact scoped review. Tree progress remains separate from the fixed stock goal.",
        )
    } else if unsettled(clock) {
        match clock["active_run"]["state"].as_str() {
            Some("dispatch_started" | "tracking" | "cancel_requested") => {
                arguments["scope"] = json!("clock");
                safety_stop = result["mode"] == "control" && clock["cancel_available"] == true;
                (
                    "fortress_wait",
                    "Reconcile the exact original Tree clock once. A settled interval still needs the child's separate supply sample; never repeat its unpause attempt.",
                )
            }
            Some("intent_recorded" | "prepared")
                if result["mode"] == "control" && clock["cancel_available"] == true =>
            {
                arguments["scope"] = json!("clock");
                (
                    "fortress_cancel",
                    "Retire the original unattempted clock interval under current clock authority. A lost or withdrawn review cannot be restored.",
                )
            }
            _ => (
                "fortress_explain",
                "Inspect the retained clock blocker. A terminal unresolved outcome or an unattempted interval without current retirement authority cannot be repaired by repeated waits or a replacement run.",
            ),
        }
    } else if cancelled {
        if original["unresolved"] == true {
            arguments["scope"] = json!("goal");
            (
                "fortress_wait",
                "Reconcile the existing Tree key after the durable project stop. No designation review or later project stage can be started.",
            )
        } else {
            (
                "fortress_explain",
                "Inspect the stopped project's retained Tree evidence. Stopping future stages does not reverse a designation or prove the stock goal.",
            )
        }
    } else if result["mode"] == "control" && ready_to_handoff(result, reference) {
        if let Some(head) = digest(&reference["parent_acknowledged"]["head"]) {
            arguments["request"] =
                json!(json!({"kind":"advance_to_workshop", "expected_head":head}).to_string());
            (
                "fortress_plan",
                "Freshly qualify the completed original Tree handoff and enter Workshop construction. Historical supply does not certify current material, causal wood provenance or the final stock goal.",
            )
        } else {
            (
                "fortress_query",
                "Inspect and retain the exact current parent acknowledgement before requesting the Tree-to-Workshop transition.",
            )
        }
    } else if let Some(request) = run_request(result) {
        arguments["request"] = json!(request.to_string());
        (
            "fortress_plan",
            "Prepare one separately reviewed finite interval for the unfinished original Tree goal, preserving the fixed project deadline and enough remaining Tree sample capacity.",
        )
    } else if original.is_object() {
        if original["historical_supply_observed_tick"].is_u64()
            || original["continuity_lost"] == true
            || original["sample_publication_pending"] == true
            || matches!(original["native_phase"].as_u64(), Some(3 | 4))
            || clock["advancement_fenced"] == true
            || clock["remaining_horizon_ticks"] == 0
            || clock["remaining_tree_samples"]
                .as_u64()
                .is_some_and(|samples| samples < 3)
        {
            (
                "fortress_explain",
                "Inspect the original Tree's terminal, continuity or remaining-capacity evidence. Do not replace its key, renew its deadline or infer that more waiting will restore a blocked source.",
            )
        } else {
            arguments["scope"] = json!("goal");
            (
                "fortress_wait",
                "Reconcile the original Tree key or obtain one foreground root, felling-job and eligible-ground-wood sample. A designation receipt alone cannot unlock Workshop construction.",
            )
        }
    } else if let Some(request) = observed_plan(result) {
        arguments["request"] = json!(request.to_string());
        (
            "fortress_plan",
            "Prepare the published eligible Tree observation under the fixed stock project's original deadline. Planning changes no native designation and does not choose a new root.",
        )
    } else if child["observation"].is_object() {
        (
            "fortress_explain",
            "Inspect the observed Tree eligibility or fixed-deadline blockers. The child cannot shorten or renew the project's original deadline to fit this offer.",
        )
    } else {
        (
            "fortress_observe",
            "Obtain the configured Tree's current bounded root, visible body, worker and tool-offer evidence before planning its original designation.",
        )
    };
    let mut suggestions = vec![next(
        tool,
        reason,
        arguments,
        reference,
        confirmation,
        writer_calls,
        ticks,
    )];
    if safety_stop {
        suggestions.push(next(
            "fortress_cancel",
            "Request the independently authorized safety pause for this retained Tree clock. Keep the original Tree and clock references; stopping time does not undo elapsed exposure or complete the stock goal.",
            json!({"session_id":session.to_string(),"scope":"clock"}),
            reference, false, 1, 0,
        ));
    }
    Some(suggestions)
}

#[cfg(test)]
#[path = "tree_presentation_tests.rs"]
mod tests;
