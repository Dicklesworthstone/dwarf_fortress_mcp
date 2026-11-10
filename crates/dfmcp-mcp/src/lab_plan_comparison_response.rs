//! Pure, complete Agent Turn publication for candidate comparison and selection.
//! The owner installs a selected pending plan only after this packet is accepted.

use super::temporal_response::TemporalResponseProjection;
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, recommendation, uncertainty,
};
use dfmcp_core::{DfmcpError, ErrorCode, Result, SessionId};
use dfmcp_intent::PreparedPlan;
use serde_json::{Value, json};

fn invariant(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::InternalInvariantViolation, message)
}

/// Supply-dependent plans carry their complete proof once in the same response. The
/// catalog names that exact array rather than duplicating every predicate.
pub(super) fn pending_plan_entry(
    plan: &PreparedPlan,
    session_id: SessionId,
    share_step_proof: bool,
) -> Value {
    let digest = plan.digest.to_string();
    let mut pending = json!({
        "plan_digest": digest,
        "state": "prepared",
        "anchor": crate::observation_projection::anchor_json(&plan.anchor),
        "expires_at_tick": plan.expires_at_tick.0,
        "minimum_safe_next_step": {"tool": "fortress.commit", "arguments": {
            "session_id": session_id.to_string(), "plan_digest": digest,
        }},
    });
    if share_step_proof {
        pending["step_count"] = json!(plan.steps.len());
        pending["steps_reference"] = json!({
            "scope": "same_response",
            "json_pointer": "/steps",
            "plan_digest": digest,
            "complete": true,
        });
    } else {
        pending["steps"] = crate::lab_world::plan_steps_json(plan);
    }
    pending
}

pub(super) fn finish(
    projection: &TemporalResponseProjection,
    mut payload: Value,
    query_work: Value,
    selected_pending: Option<Value>,
) -> Result<String> {
    let acquisition = matches!(
        payload["kind"].as_str(),
        Some("resource_acquisition" | "resource_acquisition_continuation")
    );
    let resource_production = matches!(
        payload["kind"].as_str(),
        Some("resource_production" | "resource_production_continuation")
    );
    let selecting = payload["kind"] == "select_candidate" || acquisition || resource_production;
    if !payload.is_object()
        || payload["ok"] != true
        || payload["anchor"] != projection.anchor
        || payload["session_id"] != projection.session_id
        || (!selecting && payload["kind"] != "compare_candidates")
        || selecting != selected_pending.is_some()
    {
        return Err(invariant(
            "candidate publication differs from its owned source or operation",
        ));
    }
    projection.budget.validate()?;
    let maximum = usize::try_from(
        projection
            .budget
            .max_bytes
            .min(u64::from(projection.budget.max_output_tokens).saturating_mul(4)),
    )
    .map_err(|_| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "candidate response bound is not representable",
        )
    })?;
    let mut active_work = projection.active_work.clone();
    if !active_work.is_object() {
        return Err(invariant(
            "candidate publication requires current game work",
        ));
    }
    for name in ["query_baselines", "query_watches", "condition_watches"] {
        let records = query_work
            .get(name)
            .filter(|value| value.as_array().is_some_and(|rows| rows.len() <= 8))
            .ok_or_else(|| {
                invariant("candidate publication requires all bounded query catalogs")
            })?;
        active_work[name] = records.clone();
    }
    if let Some(pending) = selected_pending {
        if let Some(reference) = pending.get("steps_reference") {
            let steps = payload["steps"].as_array();
            if reference["scope"] != "same_response"
                || reference["json_pointer"] != "/steps"
                || reference["complete"] != true
                || reference["plan_digest"] != payload["plan_digest"]
                || pending["plan_digest"] != payload["plan_digest"]
                || steps
                    .is_none_or(|rows| pending["step_count"].as_u64() != Some(rows.len() as u64))
                || (!payload["resource_acquisition"].is_object()
                    && !payload["production"]["resource_workflow"].is_object())
            {
                return Err(invariant(
                    "resource pending catalog must reference its complete matching plan steps in this response",
                ));
            }
        }
        active_work["pending_plans"] = json!([pending]);
    }
    let recommendations = if selecting {
        let mut next = recommendation(
            if acquisition {
                "commit-resource-acquisition"
            } else if resource_production {
                "commit-production-stage"
            } else {
                "commit-selected-candidate"
            },
            "fortress.commit",
            if acquisition {
                "The original resource minima have a newly prepared finite acquisition plan."
            } else if resource_production {
                "The original production objective has a newly prepared stage based on observed supply."
            } else {
                "The selected source has a newly prepared ordinary plan."
            },
            "Pursue the selected original objective after current-state revalidation.",
            "Obtain observed effect and obligation evidence.",
            payload["max_risk"].as_str().unwrap_or("unknown"),
            "registered_action_policy",
            false,
            payload["next_step"]["arguments"].clone(),
        );
        next["invalidating_conditions"] = json!([
            "source anchor changes",
            "required authority changes",
            "plan expires",
        ]);
        vec![next]
    } else {
        Vec::new()
    };
    payload["request_id"] = json!(projection.request_id);
    payload["current_anchor"] = projection.anchor.clone();
    payload["game_tick"] = projection.anchor["game_tick"].clone();
    payload["paused"] = json!(projection.paused);
    payload["game_effect"] = json!("none");
    payload["publication"] = json!("complete_before_pending_plan_change");
    let admitted = json!({
        "max_wall_millis": projection.budget.max_wall_millis,
        "max_game_ticks": projection.budget.max_game_ticks,
        "max_entities": projection.budget.max_entities,
        "max_bytes": projection.budget.max_bytes,
        "max_output_tokens": projection.budget.max_output_tokens,
        "max_actions": projection.budget.max_actions,
        "maximum_response_bytes": maximum,
    });
    let output = AgentTurnBuilder::new(
        "fortress.plan",
        if selecting { AgentPhase::Propose } else { AgentPhase::Compare },
    )
        .session_id(projection.session_id.clone())
        .request_id(projection.request_id.clone())
        .turn_id(format!("lab-candidate-turn-{}", projection.request_id))
        .anchor(projection.anchor.clone())
        .continuity(ContinuityStatus::Partial, Some(projection.anchor.clone()), None, None)
        .profile(ObservationProfile::Tactical)
        .briefing(json!({
            "runtime": "laboratory_reference_world",
            "adapter": "memory",
            "paused": projection.paused,
            "objectives": projection.objectives,
            "comparison_storage": "stateless",
            "canonical_time_advanced": 0,
            "candidate_predictions_grant_authority": false,
            "recovered_work_observation": "retained_not_reobserved",
            "live_game_qualification": false,
        }))
        .active_work(active_work)
        .recommendations(recommendations)
        .uncertainty(vec![uncertainty(
            "candidate-forecast-model",
            "predicted",
            "Forecasts assume deterministic laboratory effects and no other agent actions; they dispatch only the candidate's deferred chain while physical world jobs advance.",
            "Other retained plans' deferred actions are not dispatched on these forks. Future outcomes and ordinary commit success remain unproven.",
            Some("fortress.observe"),
            json!({"session_id": projection.session_id}),
        )])
        .coverage(json!({
            "status": "partial",
            "complete_domains": ["laboratory.selected_plan_requests",
                "laboratory.pending_plan_catalog", "laboratory.session_query_baseline_catalog",
                "laboratory.session_query_watch_catalog", "laboratory.nonfinal_carried_obligation_catalog"],
            "partial_domains": [{"domain": "laboratory.future_outcomes",
                "reason": "conditional reference-world prediction; no observed effect evidence"},
                {"domain": "laboratory.current_game_work",
                "reason": "owned action receipts and retained recovery records are projected; recovery proofs are not refreshed and other sessions' deferred dispatch is not exhaustively projected"}],
            "omitted_domains": ["live_dwarf_fortress_state"],
            "absence_proof_scope": [],
            "continuation": null,
            "source_anchor": projection.anchor,
            "eligible_as_mutation_evidence": false,
        }))
        .budget(json!({
            "requested": null, "admitted": admitted,
            "consumed": {"canonical_source": payload["source_work"],
                "forecast_work": payload["forecast_work"], "canonical_game_ticks": 0},
            "remaining": null, "soft_stop_reason": null, "hard_stop_reason": null,
        }))
        .references(vec![json!({
            "kind": "laboratory_world",
            "state_hash": projection.anchor["state_hash"],
            "anchor": projection.anchor,
        })])
        .attach(payload);
    if output.len() > maximum {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete candidate result and current work exceed the response budget",
        ));
    }
    Ok(output)
}
