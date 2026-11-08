//! Workshop evidence and separately reviewed clock custody remain distinct.
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
pub(super) fn clock_reference(clock: &Value) -> Value {
    if !clock.is_object()
        || (clock["enabled"] == false
            && !clock["recovery"].is_object()
            && !clock["active_run"].is_object())
    {
        return Value::Null;
    }
    let active = &clock["active_run"];
    json!({"enabled":clock["enabled"],"target_id":clock["target_id"],
        "deadline_tick":clock["deadline_tick"],"recovery":clock["recovery"],
        "pending":clock["pending"],"unresolved":clock["unresolved"],
        "advancement_fenced":clock["advancement_fenced"],
        "active_run":if active.is_object() {json!({
            "key":active["key"],"plan_digest":active["plan_digest"],
            "review_seal":active["review_seal"],"state":active["state"],
            "native_phase":active["native_phase"],"game_ticks":active["game_ticks"],
            "wall_millis":active["wall_millis"],"start_tick":active["start_tick"],
            "observed_tick":active["observed_tick"],"stopped_with_pause":active["stopped_with_pause"],
            "native_unpause_attempted":active["native_unpause_attempted"]
        })} else {Value::Null},
        "verified":false,"commit_available":false,"cancel_available":false,"new_run_available":false,
        "currently_verified":false,"current_pause_proven":false,
        "goal_completion_proven":false,"causal_provenance_known":false})
}
fn retained_clock(reference: &Value) -> &Value {
    let latest = &reference["latest_unverified_reference"]["clock"];
    let direct = &reference["clock"];
    if latest.is_object() {
        latest
    } else if direct.is_object() {
        direct
    } else {
        &reference["last_complete_reference"]["clock"]
    }
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
        "native_receipt":original["native_receipt"],
        "clock":clock_reference(retained_clock(reference)),"currently_verified":false})
}
fn clock_commit_available(result: &Value, verified: bool) -> bool {
    let clock = &result["clock"];
    verified
        && result["mode"] == "control"
        && clock["enabled"] == true
        && clock["verified"] == true
        && clock["commit_available"] == true
        && clock["pending"] == true
        && clock["unresolved"] == false
        && clock["advancement_fenced"] != true
        && clock["active_run"]["state"] == "prepared"
        && clock["active_run"]["review_seal"]
            .as_str()
            .is_some_and(|seal| !seal.is_empty())
}
fn next_step(
    result: &Value,
    original: &Value,
    verified: bool,
    prepared: bool,
    session: SessionId,
    budget: Option<WorkBudget>,
) -> (&'static str, Value, &'static str) {
    let args = json!({"session_id":session.to_string()});
    if result["mode"] == "offline" || !verified {
        return (
            "fortress_query",
            args,
            "Verify the original local custody; this step grants no native authority.",
        );
    }
    let clock = &result["clock"];
    if clock["pending"] == true || clock["unresolved"] == true {
        if clock_commit_available(result, verified) {
            return (
                "fortress_commit",
                json!({"session_id":session.to_string(),
                    "review_seal":clock["active_run"]["review_seal"],"confirmed":true}),
                "Review this exact bounded run separately from construction, then confirm its own seal. The run cannot prove workshop completion.",
            );
        }
        if clock["verified"] == true {
            let state = clock["active_run"]["state"].as_str();
            if state == Some("terminal") {
                return (
                    "fortress_explain",
                    args,
                    "The retained run has a terminal unresolved outcome. Inspect its original source-loss or unknown-result evidence; another wait cannot restore continuity or prove a pause.",
                );
            }
            if matches!(state, Some("intent_recorded" | "prepared"))
                && result["mode"] == "control"
                && clock["cancel_available"] == true
            {
                return (
                    "fortress_cancel",
                    json!({"session_id":session.to_string(),"scope":"clock"}),
                    "Retire the retained unattempted run under current clock authority. Its earlier review cannot be restored, and local retirement does not unpause the game.",
                );
            }
            if matches!(state, Some("intent_recorded" | "prepared")) {
                return (
                    "fortress_explain",
                    args,
                    "The original run was never dispatched, but no current review or clock-retirement authority is available. Recovery cannot restore dispatch permission; inspect and preserve its original custody instead of repeating waits.",
                );
            }
            return (
                "fortress_wait",
                json!({"session_id":session.to_string(),"scope":"clock"}),
                "Reconcile the retained bounded run without another unpause. A settled run still needs fresh original-workshop evidence.",
            );
        }
        return (
            "fortress_query",
            args,
            "Verify the retained run custody and recovery reference before any further clock or construction action.",
        );
    }
    if clock["advancement_fenced"] == true {
        return (
            "fortress_explain",
            args,
            "Further clock advancement is permanently fenced in this owner after a failed source or guard check. Inspect the retained blocker; repeated workshop samples cannot restore control permission.",
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
        let ticks = clock["recommended_game_ticks"].as_u64();
        let remaining = clock["deadline_tick"]
            .as_u64()
            .zip(original["sample_tick"].as_u64())
            .and_then(|(deadline, sampled)| deadline.checked_sub(sampled));
        if result["mode"] == "control"
            && clock["enabled"] == true
            && clock["verified"] == true
            && clock["clock_ready"] == true
            && clock["new_run_available"] == true
            && clock["pending"] == false
            && clock["unresolved"] == false
            && clock["advancement_fenced"] != true
            && clock["target_id"]
                .as_str()
                .is_some_and(|target| !target.is_empty())
            && let (Some(ticks), Some(remaining), Some(budget)) = (ticks, remaining, budget)
            && ticks > 0
            && ticks <= 1200.min(budget.max_game_ticks).min(remaining)
        {
            let request = json!({"kind":"run_workshop","target_id":clock["target_id"],
                "game_ticks":ticks,"wall_millis":budget.max_wall_millis.min(5000)})
            .to_string();
            return (
                "fortress_plan",
                json!({"session_id":session.to_string(),"request":request}),
                "Prepare a separately reviewed bounded interval for the original workshop. Fresh workshop evidence must independently establish construction progress.",
            );
        }
        return (
            "fortress_wait",
            json!({"session_id":session.to_string(),"scope":"goal"}),
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
                offers.iter().find(|offer| {
                    offer["item_id"] == observed["selection"]["item_id"]
                        && offer["host_selection_allowed"] == true
                })
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
                !offers.is_empty()
                    && offers
                        .iter()
                        .all(|offer| offer["host_selection_allowed"] == false)
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
        && let Some(offer) = result["tree_material"]["offers"]
            .as_array()
            .and_then(|offers| {
                offers.iter().find(|offer| {
                    offer["host_selection_allowed"] == true
                        && offer["item_id"] != observed["selection"]["item_id"]
                })
            })
    {
        let request = json!({"kind":"carpenters_from_tree_supply",
            "expected_observation":result["tree_material"]["witness"],
            "material":offer["material_handle"]})
        .to_string();
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
    let mut compact = compact_reference(&reference);
    let clock = if result["clock"].is_object() {
        clock_reference(&result["clock"])
    } else {
        compact["clock"].clone()
    };
    // A successful view is itself complete evidence even if a caller has no
    // earlier reference. The bounded fallback must retain its run identity too.
    compact["clock"] = clock.clone();
    let clock_verified = verified && !closed && result["clock"]["verified"] == true;
    let clock_unsettled = clock["pending"] == true || clock["unresolved"] == true;
    let clock_prepared = !closed && clock_commit_available(&result, verified);
    let clock_active = clock["active_run"].is_object();
    let clock_uncertain =
        clock_active && (clock["unresolved"] == true || (!clock_verified && clock_unsettled));
    let prepared =
        verified && !closed && !clock_unsettled && original["prepared_in_this_connection"] == true;
    let fresh_completion =
        verified && !closed && !clock_unsettled && original["fresh_completion_proven"] == true;
    let evidence = json!({"acknowledged":result["acknowledged"],
        "original_plan_digest":original["plan_digest"],"review_seal":original["review_seal"],
        "clock":clock,"retained_reference":compact});
    let clock_evidence = json!({"scope":"clock","target_id":clock["target_id"],
        "recovery":clock["recovery"],"active_run":clock["active_run"],
        "custody_currently_verified":clock_verified,"pending":clock["pending"],
        "unresolved":clock["unresolved"],"goal_completion_proven":false});
    let mut recommendations = vec![];
    if !closed && let Some(session) = session {
        let (next, arguments, reason) =
            next_step(&result, &original, verified, prepared, session, budget);
        let clock_commit = next == "fortress_commit" && clock_prepared;
        let clock_cancel = next == "fortress_cancel" && arguments["scope"] == "clock";
        let clock_plan = next == "fortress_plan"
            && arguments["request"]
                .as_str()
                .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
                .is_some_and(|request| request["kind"] == "run_workshop");
        let commits = next == "fortress_commit";
        let mut next_step = recommendation(
            if clock_commit || clock_plan || arguments["scope"] == "clock" {
                "continue_original_workshop_clock"
            } else {
                "continue_original_workshop"
            },
            next,
            reason,
            "high",
            "high",
            if commits || clock_plan || clock_cancel {
                "guarded"
            } else {
                "read_only"
            },
            if clock_commit {
                "advancing simulation can change the fortress; a stop does not undo it"
            } else if commits {
                "construction is not undone by this profile"
            } else {
                "not_applicable"
            },
            commits,
            arguments,
        );
        next_step["confidence"]["evidence"] = json!([evidence.clone()]);
        next_step["estimated_cost"] = json!({
            "game_ticks":if clock_commit {clock["active_run"]["game_ticks"].clone()} else {json!(0)},
            "native_writer_calls_max":u8::from(commits),
            "native_clock_unpause_calls_max":u8::from(clock_commit),
            "native_run_wall_millis":if clock_commit {clock["active_run"]["wall_millis"].clone()} else {Value::Null},
            "wall_millis_max":budget.map(|b|b.max_wall_millis),"output_bytes_max":RESPONSE_BYTES});
        next_step["invalidating_conditions"] = json!([
            "source/footprint/material/worker change",
            "authority or lease expiry",
            "lost prepared connection",
            "changed construction custody/acknowledgement or clock journal",
            "original workshop continuity or deadline lost",
            "unresolved clock run or absent current clock permission"
        ]);
        next_step["grants_authority"] = json!(false);
        recommendations.push(next_step);
    }
    let mut pending_plans = vec![];
    let mut actions = vec![];
    let mut obligations = vec![];
    let mut indeterminate_effects = vec![];
    let mut confirmations = vec![];
    if original.is_object() {
        actions.push(json!({"scope":"goal","evidence":evidence,
            "state":original["state"],"native_phase":original["native_phase"],
            "unresolved":original["unresolved"]}));
        obligations.push(json!({"kind":"original_carpenter_workshop_complete",
            "evidence":evidence,"fresh_completion_proven":fresh_completion}));
    }
    if prepared {
        pending_plans.push(evidence.clone());
        confirmations
            .push(json!({"scope":"goal","review_seal":original["review_seal"],"required":true}));
    }
    if uncertain {
        indeterminate_effects.push(evidence.clone());
    }
    if clock_active {
        actions.push(clock_evidence.clone());
        if clock_unsettled {
            obligations.push(json!({"kind":"bounded_run_stop","evidence":clock_evidence,
                "current_pause_proven":false,"workshop_completion_proven":false}));
        }
    }
    if clock_prepared {
        pending_plans.push(clock_evidence.clone());
        confirmations.push(json!({"scope":"clock","target_id":clock["target_id"],
            "review_seal":clock["active_run"]["review_seal"],"required":true}));
    }
    if clock_uncertain {
        indeterminate_effects.push(clock_evidence.clone());
    }
    let mut complete_domains = vec![];
    if verified {
        complete_domains.push("this_original_workshop_custody");
    }
    if clock_verified {
        complete_domains.push("this_bound_workshop_clock_custody");
    }
    let clock_advanced = if clock.is_object() {
        if verified && !closed {
            result.get("clock_advanced").cloned().unwrap_or(Value::Null)
        } else {
            Value::Null
        }
    } else {
        json!(false)
    };
    let turn=spine(operation,session,request)
        .briefing(json!({"profile":"workshop/1.20","runtime_admitted":false,"compatibility_admitted":false,
            "server_artifact_qualified":false,"mutation_admissible":false,"development_commit_available":prepared || clock_prepared,
            "canonical_world_continuity":"unestablished","canonical_world_anchor_available":false,"session_closed":closed,
            "native_source_reference":{"profile":original["source"]["profile"],"witness":original["source"]["witness"],"generation":original["source"]["generation"],"tick":original["source"]["tick"]},
            "original_building_inserted_at_anchor":original["placed_at_anchor"],
            "fresh_original_completion_proven":fresh_completion,
            "historical_goal":original["historical_goal"],"current_pause_proven":false,
            "automatic_redispatch":false,"clock_advanced":clock_advanced,
            "clock_advance_evidence_scope":"historical_run_receipt_only",
            "clock_control_attempted":result["clock_control_attempted"],
            "native_unpause_attempted":result["native_unpause_attempted"],
            "clock_control_enabled":verified && result["clock"]["enabled"]==true,
            "clock_custody_currently_verified":clock_verified,
            "clock_pending":clock["pending"],"clock_unresolved":clock["unresolved"],
            "clock_advancement_fenced":clock["advancement_fenced"],
            "clock_recovery":clock["recovery"],"clock_receipt_proves_workshop_completion":false,
            "manager_activation_available":false,
            "material_policy":result["material_policy"],"tree_material_discovery_available":verified && result["tree_material"].is_object(),
            "finite_bed_output_proven":false}))
        .active_work(json!({"pending_plans":pending_plans,"actions":actions,"obligations":obligations,
            "cancellation_drains":[],"indeterminate_effects":indeterminate_effects,
            "publications":[],"confirmations":confirmations}))
        .recommendations(recommendations)
        .uncertainty(vec![json!({"uncertainty_id":"development_evidence","epistemic_state":"unknown",
            "statement":"Operator source acceptance is unadmitted development authority. A native insertion is historical registration, never completion. Completion requires fresh original source/generation/material/subtype/geometry/stage evidence. Worker eligibility does not promise assignment."}),
            json!({"uncertainty_id":"production_prerequisites","epistemic_state":"unknown",
            "statement":"This slice has no manager, activation or finite-bed production path. Original .14 remains open until its untouched-game and finite-output criteria are independently met."}),
            json!({"uncertainty_id":"bounded_clock_evidence","epistemic_state":"unknown",
            "statement":"Optional run/1.13 control has separate review, authority and durable custody. A stopped receipt is historical pause evidence, not current pause, construction completion or causal attribution. Tick and wall limits are callback stop triggers; other controllers are not fenced and native run/1.13 does not identify a fortress."})])
        .coverage(json!({"status":"partial","complete_domains":complete_domains,
            "source":if verified && result["tree_material"].is_object() {"workshop/1.20 and independently pinned tree_harvest/1.21 material discovery"} else {"workshop/1.20 only"},
            "separate_clock_custody":clock.is_object(),"clock_source":"run/1.13 independently pinned when configured",
            "canonical_operations_join":false,"continuous_history_proven":false}))
        .budget(json!({"max_output_bytes":RESPONSE_BYTES,"max_game_ticks":budget.map(|b|b.max_game_ticks)}))
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
    let mut minimal=spine(operation,session,request)
        .briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"current_goal_available":false,
            "fresh_original_completion_proven":false,"current_pause_proven":false,
            "clock_custody_currently_verified":false,"clock_receipt_proves_workshop_completion":false,
            "clock_recovery":compact["clock"]["recovery"],"clock_pending":compact["clock"]["pending"],
            "clock_unresolved":compact["clock"]["unresolved"],"clock_advanced":null,
            "clock_advancement_fenced":compact["clock"]["advancement_fenced"],
            "canonical_world_continuity":"unestablished","canonical_world_anchor_available":false}))
        .active_work(json!({"pending_plans":[],"actions":[],"obligations":[],"cancellation_drains":[],"indeterminate_effects":[compact.clone()],"publications":[],"confirmations":[]}))
        .uncertainty(vec![json!({"uncertainty_id":"output_exhausted","epistemic_state":"indeterminate","statement":"Output bound exhausted; preserve original construction acknowledgement and independent run custody. Reconcile the original keys without redispatch; neither current pause nor workshop completion is established."})])
        .coverage(json!({"status":"unavailable","omitted_domains":["full_original_evidence"],"canonical_world_anchor_available":false}))
        .references(vec![compact.clone()]).build();
    minimal["anchor"] = Value::Null;
    if let Some(b) = minimal["briefing"].as_object_mut() {
        b.remove("admission");
    }
    json!({"agent_turn":minimal,"retained_reference":compact,"retained_reference_current":false,
        "result":{"ok":false,"error":{"code":"budget_exceeded"},"current_goal_available":false,
            "clock":compact["clock"],"automatic_redispatch":false}})
    .to_string()
}
