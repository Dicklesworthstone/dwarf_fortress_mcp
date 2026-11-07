//! Workshop evidence stays in its own namespace, without an operations join.
use super::runtime::RESPONSE_BYTES;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_core::{RequestId, SessionId, WorkBudget};
use serde_json::{Value, json};

fn spine(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
) -> AgentTurnBuilder {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut turn = AgentTurnBuilder::new(operation, phase).continuity(
        ContinuityStatus::Indeterminate,
        None,
        Some(json!({"world_history":"unestablished","canonical_world_anchor_available":false})),
        None,
    );
    if let Some(session) = session {
        turn = turn.session_id(session.to_string());
    }
    if let Some(request) = request {
        turn = turn.request_id(request.to_string());
    }
    turn
}
fn compact_reference(reference: &Value) -> Value {
    let last = &reference["last_complete_reference"];
    let latest = &reference["latest_unverified_reference"];
    let original = if latest["original"].is_object() {
        &latest["original"]
    } else if reference["original"].is_object() {
        &reference["original"]
    } else {
        &last["original"]
    };
    json!({"acknowledged":reference.get("acknowledged").or_else(||last.get("acknowledged")),
        "journal_pin_unverified":reference.get("journal_pin_unverified").or_else(||latest.get("journal_pin_unverified")),
        "original_key":original["key"],"original_plan_digest":original["plan_digest"],"review_seal":original["review_seal"],
        "native_receipt":original["native_receipt"],"currently_verified":false})
}
fn next_step(
    result: &Value,
    original: &Value,
    verified: bool,
    prepared: bool,
    session: SessionId,
) -> (&'static str, Value, &'static str) {
    let args = json!({"session_id":session.to_string()});
    if result["mode"] == "offline" || !verified {
        return (
            "fortress_query",
            args,
            "Verify the original local custody; this step grants no native authority.",
        );
    }
    if prepared {
        return (
            "fortress_commit",
            json!({"session_id":session.to_string(),"review_seal":original["review_seal"],"confirmed":true}),
            "Review the whole original plan before confirming this exact seal.",
        );
    }
    if original.is_object() {
        match original["native_phase"].as_str() {
            Some("refused") => {
                return (
                    "fortress_explain",
                    args,
                    "The original construction was refused. Inspect its terminal reason; never replace its key.",
                );
            }
            Some("cancelled") => {
                return (
                    "fortress_explain",
                    args,
                    "The original preparation was cancelled. Inspect retained history; no construction can progress.",
                );
            }
            Some("prepared") if original["state"] == "tracking" => {
                return (
                    "fortress_cancel",
                    json!({"session_id":session.to_string(),"scope":"goal"}),
                    "Retire the confirmed remaining preparation; recovery cannot restore commit permission.",
                );
            }
            _ => {}
        }
        if original["historical_goal"] == "complete" {
            return (
                "fortress_explain",
                args,
                "The original goal has historical completion evidence. Inspect it without phantom waiting or replacement keys.",
            );
        }
        if matches!(
            original["historical_goal"].as_str(),
            Some("original_building_absent" | "original_identity_contradicted")
        ) || original["high_generation"]
            .as_u64()
            .zip(original["source"]["generation"].as_u64())
            .is_some_and(|(high, initial)| high > initial)
        {
            return (
                "fortress_explain",
                args,
                "Original identity or source continuity is unestablished. Inspect the retained blocker; waiting cannot recreate continuity.",
            );
        }
        return (
            "fortress_wait",
            args,
            "Reconcile the original key or acquire one fresh original-building sample; never redispatch.",
        );
    }
    let observed = &result["observation"];
    if observed.is_null() {
        return (
            "fortress_observe",
            args,
            "Acquire the bounded original workshop/material/worker offer before planning.",
        );
    }
    if observed["eligible_at_anchor"] == true {
        if result["material_policy"] == "tree_supply" {
            let discovery = &result["tree_material"];
            let selected = discovery["offers"].as_array().and_then(|offers| {
                offers.iter().find(|offer| offer["item_id"] == observed["selection"]["item_id"]
                    && offer["host_selection_allowed"] == true)
            });
            if let Some(offer) = selected {
                let request = json!({"kind":"carpenters_from_tree_supply",
                    "expected_observation":discovery["witness"],"material":offer["material_handle"]}).to_string();
                return (
                    "fortress_plan",
                    json!({"session_id":session.to_string(),"request":request}),
                    "Select this observed Tree material offer, requalify it independently for the Carpenter footprint, and prepare the original workshop for review.",
                );
            }
            if discovery["offers"].as_array().is_some_and(|offers| {
                !offers.is_empty() && offers.iter().all(|offer| offer["host_selection_allowed"] == false)
            }) {
                return (
                    "fortress_explain",
                    args,
                    "The offered logs or fixed footprint exceed the host's scope or protected regions; inspect the configured spatial policy.",
                );
            }
            return (
                "fortress_observe",
                args,
                "Refresh the exact Tree material offers before selecting a workshop material.",
            );
        }
        let request=json!({"kind":"carpenters","expected_observation":observed["witness"],"material":observed["material_handle"]}).to_string();
        return (
            "fortress_plan",
            json!({"session_id":session.to_string(),"request":request}),
            "Prepare this exact observed Carpenter/log offer for review without constructing it.",
        );
    }
    if result["material_policy"] == "tree_supply"
        && let Some(offer) = result["tree_material"]["offers"].as_array().and_then(|offers| {
            offers.iter().find(|offer| {
                offer["host_selection_allowed"] == true
                    && offer["item_id"] != observed["selection"]["item_id"]
            })
        })
    {
        let request = json!({"kind":"carpenters_from_tree_supply",
            "expected_observation":result["tree_material"]["witness"],
            "material":offer["material_handle"]}).to_string();
        return (
            "fortress_plan",
            json!({"session_id":session.to_string(),"request":request}),
            "Independently qualify this alternative Tree material candidate; its workshop eligibility is still unestablished and construction requires a later reviewed commit.",
        );
    }
    (
        "fortress_explain",
        args,
        "Inspect the observed eligibility blockers before another observation or plan.",
    )
}
pub(super) fn packet(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    result: Value,
    reference: Value,
) -> String {
    let verified = result["ok"] == true && result["retained_state_currently_verified"] == true;
    let original = if verified {
        result["original"].clone()
    } else {
        Value::Null
    };
    let closed = result["session_closed"] == true;
    let uncertain = !closed && (!verified || original["unresolved"] == true);
    let prepared = verified && !closed && original["prepared_in_this_connection"] == true;
    let evidence = json!({"acknowledged":result["acknowledged"],"original_plan_digest":original["plan_digest"],"review_seal":original["review_seal"],"retained_reference":compact_reference(&reference)});
    let mut recommendations = vec![];
    if !closed && let Some(session) = session {
        let (next, arguments, reason) = next_step(&result, &original, verified, prepared, session);
        let mut next_step = recommendation(
            "continue_original_workshop",
            next,
            reason,
            "high",
            "high",
            if prepared { "guarded" } else { "read_only" },
            if prepared {
                "construction is not undone by this profile"
            } else {
                "not_applicable"
            },
            prepared,
            arguments,
        );
        next_step["confidence"]["evidence"] = json!([evidence.clone()]);
        next_step["estimated_cost"] = json!({"game_ticks":0,"native_writer_calls_max":u8::from(prepared),"wall_millis_max":budget.map(|b|b.max_wall_millis),"output_bytes_max":RESPONSE_BYTES});
        next_step["invalidating_conditions"] = json!([
            "source/footprint/material/worker change",
            "authority or lease expiry",
            "lost prepared connection",
            "changed custody/acknowledgement"
        ]);
        next_step["grants_authority"] = json!(false);
        recommendations.push(next_step);
    }
    let turn=spine(operation,session,request)
        .briefing(json!({"profile":"workshop/1.20","runtime_admitted":false,"compatibility_admitted":false,
            "server_artifact_qualified":false,"mutation_admissible":false,"development_commit_available":prepared,
            "canonical_world_continuity":"unestablished","canonical_world_anchor_available":false,"session_closed":closed,
            "native_source_reference":{"profile":original["source"]["profile"],"witness":original["source"]["witness"],"generation":original["source"]["generation"],"tick":original["source"]["tick"]},
            "original_building_inserted_at_anchor":original["placed_at_anchor"],
            "fresh_original_completion_proven":verified && original["fresh_completion_proven"]==true,
            "historical_goal":original["historical_goal"],"current_pause_proven":false,
            "automatic_redispatch":false,"clock_advanced":false,"manager_activation_available":false,
            "material_policy":result["material_policy"],"tree_material_discovery_available":verified && result["tree_material"].is_object(),
            "finite_bed_output_proven":false}))
        .active_work(json!({"pending_plans":if prepared {vec![evidence.clone()]} else {vec![]},
            "actions":if original.is_object() {vec![json!({"evidence":evidence,"state":original["state"],"native_phase":original["native_phase"],"unresolved":original["unresolved"]})]} else {vec![]},
            "obligations":if original.is_object() {vec![json!({"kind":"original_carpenter_workshop_complete","evidence":evidence,"fresh_completion_proven":original["fresh_completion_proven"]})]} else {vec![]},
            "cancellation_drains":[],"indeterminate_effects":if uncertain {vec![evidence.clone()]} else {vec![]},
            "publications":[],"confirmations":if prepared {vec![json!({"review_seal":original["review_seal"],"required":true})]} else {vec![]}}))
        .recommendations(recommendations)
        .uncertainty(vec![json!({"uncertainty_id":"development_evidence","epistemic_state":"unknown",
            "statement":"Operator source acceptance is unadmitted development authority. A native insertion is historical registration, never completion. Completion requires fresh original source/generation/material/subtype/geometry/stage evidence. Worker eligibility does not promise assignment."}),
            json!({"uncertainty_id":"production_prerequisites","epistemic_state":"unknown",
            "statement":"This slice has no manager, activation, game-clock or finite-bed production path. Original .14 remains open until its untouched-game and finite-output criteria are independently met."})])
        .coverage(json!({"status":"partial","complete_domains":if verified {vec!["this_original_workshop_custody"]} else {vec![]},
            "source":if verified && result["tree_material"].is_object() {"workshop/1.20 and independently pinned tree_harvest/1.21 material discovery"} else {"workshop/1.20 only"},"canonical_operations_join":false,"continuous_history_proven":false}))
        .budget(json!({"max_output_bytes":RESPONSE_BYTES,"max_game_ticks":0}))
        .references(vec![evidence]);
    let mut value = turn.build();
    if let Some(briefing) = value["briefing"].as_object_mut() {
        briefing.remove("admission");
    }
    let output=json!({"agent_turn":value,"result":result,"retained_reference":reference,"retained_reference_current":false}).to_string();
    let limit = budget.map_or(RESPONSE_BYTES, |b| {
        b.max_bytes
            .min(u64::from(b.max_output_tokens) * 4)
            .min(RESPONSE_BYTES)
    });
    if output.len() as u64 <= limit {
        return output;
    }
    // Full output was reserved before I/O. Never truncate an evidence object or
    // report an attempted construction as a clean read failure on overflow.
    let compact = compact_reference(&reference);
    let mut minimal=spine(operation,session,request)
        .briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"current_goal_available":false,"canonical_world_continuity":"unestablished","canonical_world_anchor_available":false}))
        .active_work(json!({"pending_plans":[],"actions":[],"obligations":[],"cancellation_drains":[],"indeterminate_effects":[compact.clone()],"publications":[],"confirmations":[]}))
        .uncertainty(vec![json!({"uncertainty_id":"output_exhausted","epistemic_state":"indeterminate","statement":"Output bound exhausted; preserve original custody and reconcile without redispatch."})])
        .coverage(json!({"status":"unavailable","omitted_domains":["full_original_evidence"],"canonical_world_anchor_available":false}))
        .references(vec![compact.clone()]).build();
    minimal["anchor"] = Value::Null;
    if let Some(b) = minimal["briefing"].as_object_mut() {
        b.remove("admission");
    }
    json!({"agent_turn":minimal,"retained_reference":compact,"retained_reference_current":false,
        "result":{"ok":false,"error":{"code":"budget_exceeded"},"current_goal_available":false,"automatic_redispatch":false}}).to_string()
}
