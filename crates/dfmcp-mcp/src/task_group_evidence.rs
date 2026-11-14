//! Fitted results keep every selected original's decisive outcome. Complete
//! immutable evidence remains available through the ordinary Task pages.
use serde_json::{Value, json};

use super::{TaskWork, compact_selected_outcome};

pub(super) fn compact_set_outcome(payload: &Value) -> Value {
    let selected = &payload["selected_plan_set_wait"];
    if !selected.is_object() {
        return Value::Null;
    }
    let mut compact = json!({});
    for field in [
        "schema",
        "plan_policy",
        "requested_plan_digests",
        "plan_digests",
        "selected",
        "complete_plans",
        "any_complete",
        "observation_only",
        "identity_scope",
        "admitted_anchor",
        "group_complete",
        "all_complete",
        "inspection_complete",
        "stop_reason",
        "attention",
        "interrupted_plan_digests",
        "unfinished_plan_digests",
        "triggered_plan_digests",
        "evaluated_anchor",
        "observed_anchor",
        "completion_scope",
        "earlier_completion_is_latched",
        "simultaneous_current_goal_truth_proven",
        "reserve_capture_required",
        "endpoint_publication",
    ] {
        if let Some(value) = selected.get(field) {
            compact[field] = value.clone();
        }
    }
    compact["plans"] = json!(
        selected["plans"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|plan| {
                let mut row = compact_selected_outcome(&json!({"selected_plan_wait":plan}));
                for field in [
                    "plan_id",
                    "plan_object_digest",
                    "observed_anchor",
                    "actions_verified",
                    "actions_terminal",
                    "physical_quiescent",
                    "terminal_actions",
                    "active_work_actions",
                    "unknown_work_actions",
                    "original_goal_pending",
                    "complete_original_goal_truth",
                    "reserve_capture_required",
                ] {
                    if let Some(value) = plan.get(field) {
                        row[field] = value.clone();
                    }
                }
                row["evidence_coverage"] =
                    json!("decisive_outcome_with_complete_evidence_retained");
                row
            })
            .collect::<Vec<_>>()
    );
    compact["automatic_continuation"] = json!(false);
    compact["automatic_cancellation"] = json!(false);
    compact["dispatch_handles_restored"] = json!(false);
    compact
}

pub(super) fn unresolved_set_outcome(binding: &Value) -> Value {
    let mut outcome = binding.clone();
    outcome["inspection_complete"] = json!(false);
    outcome["group_complete"] = json!(false);
    outcome["complete_plans"] = Value::Null;
    outcome["all_complete"] = json!(false);
    outcome["any_complete"] = json!(false);
    outcome["stop_reason"] = json!("inspection_unavailable");
    outcome["observation_only"] = json!(true);
    outcome["earlier_completion_is_latched"] = json!(false);
    outcome["observed_anchor"] = Value::Null;
    if let Some(plans) = outcome["plans"].as_array_mut() {
        for plan in plans {
            plan["inspection_complete"] = json!(false);
            plan["complete"] = json!(false);
            plan["stop_reason"] = json!("inspection_unavailable");
            plan["goal_truth"] = json!("unknown");
            plan["stage_goal_truth"] = json!("unknown");
            plan["complete_original_goal_truth"] = json!("unknown");
            plan["physical_quiescent"] = Value::Null;
        }
    }
    outcome
}

pub(super) fn compact_set_terminal(
    work: &TaskWork,
    task_id: &str,
    payload: &Value,
    failure: Option<&str>,
    max_output_tokens: u32,
    max_bytes: u64,
) -> Value {
    let selected = compact_set_outcome(payload);
    let mut active = crate::empty_active_work();
    active["obligations"] = json!(
        selected["plans"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|plan| plan["complete"] != true)
            .map(|plan| json!({
                "kind":"observed_original_project",
                "plan_digest":plan["plan_digest"],
                "requested_plan_digest":plan["requested_plan_digest"],
                "active_work_actions":plan["active_work_actions"],
                "unknown_work_actions":plan["unknown_work_actions"],
                "physical_quiescent":plan["physical_quiescent"],
                "observer_owns_work":false,
            }))
            .collect::<Vec<_>>()
    );
    let anchor = payload
        .get("observed_anchor")
        .cloned()
        .unwrap_or_else(|| payload["agent_turn"]["anchor"].clone());
    let turn = crate::AgentTurnBuilder::new("fortress.wait", crate::AgentPhase::Verify)
        .session_id(work.session_id().to_owned())
        .anchor(anchor.clone())
        .active_work(active)
        .build();
    let full = payload.to_string();
    let mut compact = json!({
        "ok":failure.is_none(),"schema":"dfmcp.lab-original-set-task-summary/1",
        "session_id":work.session_id(),
        "monitor_kind":work.monitor_kind(),"cancellation_scope":"monitor_only",
        "owns_original_dispatch":false,"effects_dispatched_by_task":false,
        "effect_cancellation_allowed":false,"dispatch_handles_restored":false,
        "replacement_work_dispatched":false,"automatic_continuation":false,
        "status":if failure.is_some() { "failed" } else { "completed" },
        "selected_plan_set_wait":selected,
        "observed_anchor":anchor,"blind_retry_allowed":false,
        "agent_turn":turn,"scope":"laboratory_process_only",
        "evidence":{
            "coverage":"all_selected_outcomes_with_complete_evidence_retained",
            "digest":dfmcp_core::Digest32::of_bytes(full.as_bytes()).to_hex(),
            "bytes":full.len(),
            "details":format!("df://session/{}/task-{task_id}", work.session_id()),
            "next":format!("df://session/{}/task-{task_id}~evidence-0", work.session_id()),
        },
        "output_budget":{
            "max_output_tokens":max_output_tokens,"max_bytes":max_bytes,"clipped":true,
            "omitted":"full per-original goal, custody and source diagnostics remain in digest-checked evidence pages",
        },
    });
    work.attach_selection(&mut compact);
    for field in [
        "actual_plan_digests",
        "group_complete",
        "all_originals_complete",
        "proof_status",
        "action_proof_status",
        "action_count",
        "original_goal_satisfied",
        "original_goal_pending",
        "goal_unconfirmed_after_work",
        "needs_replan",
        "indeterminate",
        "remaining_work",
        "physical_quiescent",
        "cleanup_required",
        "drain_progress",
        "physical_work",
        "recovery_class",
        "next_step",
        "error",
        "inspection_complete",
    ] {
        if let Some(value) = payload.get(field) {
            compact[field] = value.clone();
        }
    }
    compact
}
