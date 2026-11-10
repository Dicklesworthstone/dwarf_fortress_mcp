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
    let resource_room = matches!(
        payload["kind"].as_str(),
        Some("resource_room" | "resource_room_continuation")
    );
    let selecting = payload["kind"] == "select_candidate"
        || acquisition
        || resource_production
        || resource_room;
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
    let shared_predicates = matches!(
        payload["schema"].as_str(),
        Some("dfmcp.resource-production/2" | "dfmcp.resource-room/2")
    );
    if shared_predicates {
        if payload["version"] != 2 {
            return Err(invariant(
                "shared plan proof requires its versioned review envelope",
            ));
        }
        crate::sealed_plan_review::validate(&payload)?;
    } else if payload.get("predicate_encoding").is_some() || payload.get("predicates").is_some() {
        return Err(invariant(
            "shared plan proof requires a supported review envelope",
        ));
    }
    if payload.get("anchor_encoding").is_some() {
        return Err(invariant(
            "review anchor references must be produced from the owned current projection",
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
    if let Some(mut pending) = selected_pending {
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
                    && !payload["production"]["resource_workflow"].is_object()
                    && !payload["room"]["resource_workflow"].is_object())
            {
                return Err(invariant(
                    "resource pending catalog must reference its complete matching plan steps in this response",
                ));
            }
            if shared_predicates {
                pending["steps_reference"]["predicate_encoding_pointer"] =
                    json!("/predicate_encoding");
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
            } else if resource_room {
                "commit-room-stage"
            } else {
                "commit-selected-candidate"
            },
            "fortress.commit",
            if acquisition {
                "Finite plan for the original resource minima."
            } else if resource_production {
                "Stage uses observed supply for the original goal."
            } else if resource_room {
                "Room stage uses observed materials and geometry."
            } else {
                "Fresh ordinary plan for the selected source."
            },
            "Advance the revalidated original goal.",
            "Observe effect and obligation proof.",
            payload["max_risk"].as_str().unwrap_or("unknown"),
            "registered_action_policy",
            false,
            payload["next_step"]["arguments"].clone(),
        );
        next["invalidating_conditions"] =
            json!(["source anchor changes", "authority changes", "plan expires",]);
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
    let mut objectives = projection.objectives.clone();
    if shared_predicates {
        payload["anchor_encoding"] = crate::current_anchor_review::apply(
            &mut active_work,
            &mut objectives,
            &projection.anchor,
        )?;
    }
    let admitted = json!({
        "max_wall_millis": projection.budget.max_wall_millis,
        "max_game_ticks": projection.budget.max_game_ticks,
        "max_entities": projection.budget.max_entities,
        "max_bytes": projection.budget.max_bytes,
        "max_output_tokens": projection.budget.max_output_tokens,
        "max_actions": projection.budget.max_actions,
        "maximum_response_bytes": maximum,
    });
    let turn = AgentTurnBuilder::new(
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
            "objectives": objectives,
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
            "Assumes deterministic laboratory effects, no other agents, and only this candidate's deferred dispatch; physical jobs advance.",
            "Other plans' deferred steps are excluded. Outcomes and commit success are unproven.",
            Some("fortress.observe"),
            json!({"session_id": projection.session_id}),
        )])
        .coverage(json!({
            "status": "partial",
            "complete_domains": ["laboratory.selected_plan_requests",
                "laboratory.pending_plan_catalog", "laboratory.session_query_baseline_catalog",
                "laboratory.session_query_watch_catalog", "laboratory.nonfinal_carried_obligation_catalog"],
            "partial_domains": [{"domain": "laboratory.future_outcomes",
                "reason": "laboratory prediction; no observed effect proof"},
                {"domain": "laboratory.current_game_work",
                "reason": "owned receipts and recovery records; recovery proof unrefreshed; other sessions' deferred dispatch incomplete"}],
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
        .build();
    payload["agent_turn"] = turn;
    let output = payload.to_string();
    if output.len() <= maximum {
        return Ok(output);
    }
    // The original request and proof of every *previous* objective are forensic
    // detail, not part of this new plan's action program. Project only those
    // historical artifacts when a complete review exceeds the response budget;
    // never clip the candidate, its proof, active work, current goal truth,
    // uncertainty, or actionable continuation guidance.
    compact_objective_briefing(&mut payload)?;
    let output = payload.to_string();
    if output.len() > maximum {
        return Err(DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "complete candidate result and current work exceed the response budget",
        ));
    }
    Ok(output)
}

/// Keep every retained objective and every current control/evidence state.
/// Only the named archived artifacts move behind the retained-goal resource
/// consumer. Unknown fields are retained so new safety metadata cannot be
/// silently lost by an old presentation allowlist.
fn compact_objective_briefing(payload: &mut Value) -> Result<()> {
    const HISTORICAL_ARTIFACTS: [&str; 5] = [
        "original_source",
        "terminal_condition",
        "sealed_anchor",
        "first_satisfied_anchor",
        "recorded_first_satisfied_anchor",
    ];
    let anchor = payload["anchor"].clone();
    let session = payload["session_id"]
        .as_str()
        .ok_or_else(|| invariant("objective briefing requires its owned session"))?
        .to_owned();
    let rows = payload["agent_turn"]["briefing"]["objectives"]
        .as_array_mut()
        .ok_or_else(|| invariant("objective briefing requires its complete retained catalog"))?;
    let mut projected = false;
    let current_anchor_reference = json!({"$ref": "/current_anchor"});
    let mut removed_anchor_references = 0_u64;
    for row in rows.iter_mut() {
        let Some(object) = row.as_object_mut() else {
            return Err(invariant("objective briefing contains a malformed record"));
        };
        let Some(digest) = object
            .get("plan_digest")
            .and_then(Value::as_str)
            .filter(|digest| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            })
        else {
            // An unavailable/unidentified record has no safe drill-down. Keep
            // its complete uncertainty instead of pretending it is retrievable.
            continue;
        };
        let details = format!("df://session/{session}/goal-{digest}");
        let mut row_projected = false;
        for field in HISTORICAL_ARTIFACTS {
            if let Some(removed) = object.remove(field) {
                // Only these declared anchor slots participate in the upstream
                // current-anchor codec. Source/predicate values stay literal.
                if matches!(
                    field,
                    "sealed_anchor" | "first_satisfied_anchor" | "recorded_first_satisfied_anchor"
                ) && removed == current_anchor_reference
                {
                    removed_anchor_references =
                        removed_anchor_references.checked_add(1).ok_or_else(|| {
                            invariant("objective anchor reference count overflowed")
                        })?;
                }
                row_projected = true;
            }
        }
        // A current source may be shared only after exact equality. Historical
        // or unavailable anchors stay explicit and can never be relabeled as
        // current merely to make a response fit.
        if object.get("observed_anchor") == Some(&anchor) {
            object.remove("observed_anchor");
            object.insert("observed_anchor_reference".to_owned(), json!("/anchor"));
            row_projected = true;
        }
        for field in ["semantic_output_goal", "semantic_room_goal"] {
            if let Some(goal) = object.get_mut(field).and_then(Value::as_object_mut)
                && goal.get("observed_anchor") == Some(&anchor)
            {
                goal.remove("observed_anchor");
                goal.insert("observed_anchor_reference".to_owned(), json!("/anchor"));
                row_projected = true;
            }
        }
        if row_projected {
            object.insert("details".to_owned(), json!(details));
            projected = true;
        }
    }
    let count = rows.len();
    if removed_anchor_references > 0 {
        let remaining = payload["anchor_encoding"]["reference_count"]
            .as_u64()
            .and_then(|count| count.checked_sub(removed_anchor_references))
            .ok_or_else(|| invariant("objective projection lost its current-anchor reference count"))?;
        payload["anchor_encoding"]["reference_count"] = json!(remaining);
    }
    if projected {
        payload["agent_turn"]["briefing"]["objective_projection"] = json!({
            "schema": "dfmcp.objective-briefing/1",
            "records": count,
            "complete_record_membership": true,
            "complete_goal_evidence": false,
            "omitted_artifact_fields": HISTORICAL_ARTIFACTS,
            "shared_observed_anchor": {"scope": "same_response", "json_pointer": "/anchor",
                "reference_field": "observed_anchor_reference"},
            "details_field": "details",
            "details_scope": "retained original source and historical proof; current truth is not reevaluated",
        });
        payload["agent_turn"]["coverage"]["partial_domains"]
            .as_array_mut()
            .ok_or_else(|| invariant("objective briefing requires explicit partial coverage"))?
            .push(json!({
                "domain": "laboratory.original_goal_evidence",
                "reason": "all current goal statuses retained; archived source and proof anchors require each goal's detail resource",
            }));
    }
    Ok(())
}

#[cfg(test)]
#[path = "lab_plan_comparison_response_tests.rs"]
mod tests;
