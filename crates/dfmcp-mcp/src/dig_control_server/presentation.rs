//! Control reviews and historical evidence use the shared Agent Turn spine.
use super::error;
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work, empty_budget,
    recommendation, uncertainty,
};
use dfmcp_adapter::dig_control_policy::{DigCheckpointPolicy, DigControlPolicy};
use dfmcp_adapter::dig_designation::journal::session::DigSessionView;
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigRecord, DigSummary};
use dfmcp_adapter::dig_designation::{DigBlocker, DigObservation, DigReason, DigTile};
use dfmcp_core::{Capability, Digest32, ErrorCode, MapCuboid, OperationContext, Result, RiskTier};
use serde_json::{Value, json};

pub const OUTPUT_BYTES: u64 = 32768;
pub fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "expected canonical lowercase SHA-256",
        ));
    }
    let mut bytes = [0; 32];
    for (i, pair) in raw.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let text = std::str::from_utf8(pair)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
        bytes[i] = u8::from_str_radix(text, 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
    }
    Ok(Digest32::from_bytes(bytes))
}
pub fn area(v: MapCuboid) -> Value {
    json!({"min":[v.min.x,v.min.y,v.min.z],"max":[v.max.x,v.max.y,v.max.z]})
}
fn blocker(v: &DigBlocker) -> &'static str {
    match v {
        DigBlocker::Unpaused => "unpaused",
        DigBlocker::UnobservedTarget => "unobserved_target",
        DigBlocker::NotNaturalWall => "not_natural_wall",
        DigBlocker::ExistingDesignation => "existing_designation",
        DigBlocker::OccupiedOrJob => "occupied_or_job",
        DigBlocker::KnownHazard => "known_hazard",
        DigBlocker::MissingContext => "missing_context",
        DigBlocker::HiddenContext => "hidden_context",
        DigBlocker::SequenceExhausted => "sequence_exhausted",
    }
}
pub fn observation(c: &DigObservation) -> Value {
    json!({
        "region":c.region().coordinates(),"witness":c.witness().to_string(),"generation":c.generation(),
        "sequence":c.sequence(),"game_tick":c.tick(),"paused_at_capture":c.paused(),
        "target_count":c.region().target_count(),"halo_cells":c.region().halo_count(),
        "blockers_without_hidden_acknowledgement":c.blockers(false).iter().map(blocker).collect::<Vec<_>>(),
        "shared_block_write_area":area(c.region().write_area()),"halo":area(c.region().halo()),
        "current_terrain_proven":false,"excavation_safety_proven":false,
        "tile_query":{"mode":"selection_tiles","witness":c.witness().to_string(),"offset":0,"limit":16}
    })
}
pub fn tile_page(c: &DigObservation, offset: usize, limit: usize) -> Result<Value> {
    if !(1..=16).contains(&limit) || offset >= c.region().halo_count() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "tile page is outside the complete retained halo",
        ));
    }
    let end = (offset + limit).min(c.region().halo_count());
    let rows=c.tiles().skip(offset).take(limit).map(|(position,tile)|match tile {
        DigTile::Missing=>json!({"coordinate":position,"presence":"missing"}),
        DigTile::Hidden=>json!({"coordinate":position,"presence":"hidden"}),
        DigTile::Visible(t)=>json!({"coordinate":position,"presence":"visible","tiletype":t.tiletype(),
            "designation_other":t.designation_other(),"occupancy":t.occupancy(),"priority":t.priority(),
            "cooldown":t.cooldown(),"block_other":t.block_other(),"temperatures":t.temperatures(),
            "dig":t.dig(),"hazards":t.hazards(),"flags":t.flags()}),
    }).collect::<Vec<_>>();
    Ok(
        json!({"witness":c.witness().to_string(),"offset":offset,"total_tiles":c.region().halo_count(),
        "tiles":rows,"next_offset":(end<c.region().halo_count()).then_some(end),"historical_evidence_only":true}),
    )
}
pub fn summary(v: &DigSummary) -> Value {
    json!({"idempotency_key":v.key,"plan_digest":v.plan_digest.to_string(),
    "state":v.state.as_str(),"native_phase":v.native_phase.map(|p|p.as_str()),"terminal":v.state.terminal(),
    "receipt_digest":v.receipt.map(|v|v.to_string()),"retry_commit_permitted":false})
}
pub fn inventory(v: &DigSessionView) -> Value {
    json!({"journal_id":v.journal_id.to_string(),"head":v.head.to_string(),
    "events":v.events,"byte_length":v.byte_len,"total_records":v.total_records,"unsettled_records":usize::from(v.pending.is_some())})
}
pub fn record(v: &DigRecord) -> Value {
    let p = v.plan();
    let native=v.effect().map(|e|json!({"phase":e.phase().as_str(),"reason":match e.reason(){
        DigReason::None=>"none",DigReason::Stale=>"stale",DigReason::CancelledBeforeDispatch=>"cancelled_before_dispatch"},
        "designated_count":e.designated_count(),"after_witness":e.after_witness().map(|d|d.to_string()),
        "receipt_digest":e.receipt().map(|d|d.to_string())}));
    json!({"idempotency_key":p.key(),"plan_digest":p.digest().to_string(),"state":v.state().as_str(),
        "terminal":v.state().terminal(),"native_query_can_help":v.needs_reconciliation(),
        "permanent_unknown":v.permanent_unknown(),"allow_hidden_neighbors":p.allow_hidden_neighbors(),
        "review":observation(p.before()),"native":native,"historical_evidence_only":true,
        "current_terrain_proven":false,"excavation_completion_proven":false,"retry_commit_permitted":false,
        "tile_query":{"mode":"plan_tiles","idempotency_key":p.key(),"plan_digest":p.digest().to_string(),"offset":0,"limit":16}})
}
pub fn policy(v: &DigControlPolicy) -> Value {
    json!({"digest":v.digest().to_string(),"journal_id":v.journal_id().to_string(),"lease_id":v.lease_id().to_string(),
    "lease_scope":"this_host_manager_and_configured_journal_only","global_controller_fence":false,
    "checkpoint_policy":match v.checkpoint_policy(){DigCheckpointPolicy::Required=>"required",
        DigCheckpointPolicy::DisposableFortress=>"disposable-fortress-no-checkpoint"},
    "checkpoint_verified":false,"protected_areas":v.protected_areas().iter().copied().map(area).collect::<Vec<_>>()})
}
pub fn failure(e: &dfmcp_core::DfmcpError) -> Value {
    json!({"ok":false,
    "error":{"code":e.code.as_str(),"message":match e.code {
        ErrorCode::CheckpointRequired=>"A verified game checkpoint is required and unavailable in this profile.",
        ErrorCode::LeaseDenied=>"The original exclusive spatial lease is absent, expired or does not cover the write scope.",
        ErrorCode::EffectIndeterminate=>"Recover the original journal operation; never retry its designation.",
        ErrorCode::CorruptLedger=>"Journal custody or integrity failed; preserve the original evidence.",
        ErrorCode::BudgetExceeded=>"Complete mining work and response exceed the admitted allowance.",
        _=>"Mining failed current authority, review, source, custody, runtime or policy checks."}},
    "retry_commit_permitted":false,"effect_outcome_inferred":false,
    "recovery":"Inspect the original key and journal; do not reprepare or create another key to bypass uncertainty."})
}

fn opening_history_reference(mut reference: Value) -> Value {
    if reference.get("recovery").is_none() {
        reference = json!({"recovery":reference});
    }
    reference["history_verified"] = json!(false);
    reference["historical_evidence_only"] = json!(true);
    reference["sampled_excavation_condition_proven"] = json!(false);
    reference
}

/// Opening may retain an operator pin before custody or a complete reference
/// after custody. Preserve the same public recovery shape on either failure.
pub(super) fn opening_monitor_reference(reference: Value) -> Value {
    let mut reference = opening_history_reference(reference);
    reference["fresh_sample_acquired"] = json!(false);
    reference["original_designations_verified"] = json!(false);
    reference
}

pub(super) fn opening_clock_reference(reference: Value) -> Value {
    let mut reference = opening_history_reference(reference);
    reference["current_pause_proven"] = json!(false);
    reference["fresh_goal_sampling_permitted"] = json!(false);
    reference["retry_commit_permitted"] = json!(false);
    reference["dispatch_authorized"] = json!(false);
    reference
}

/// Keep bounded original identities if a final runtime check fails after work.
/// A prior packet is historical evidence, never a fresh inventory or review.
pub(super) fn retained_handoff(raw: &str) -> Option<Value> {
    if raw.len() as u64 > OUTPUT_BYTES {
        return None;
    }
    let packet: Value = serde_json::from_str(raw).ok()?;
    let active = &packet["agent_turn"]["active_work"];
    let result = &packet["result"];
    let record = ["effect", "plan", "record"].iter().find_map(|field| {
        let value = &result[*field];
        value["idempotency_key"].is_string().then(|| {
            json!({
                "idempotency_key":value["idempotency_key"],"plan_digest":value["plan_digest"],
                "state":value["state"],"native":value["native"]
            })
        })
    });
    Some(
        json!({"inventory_verified":false,"historical_evidence_only":true,
        "journal":active["counts"],"pending_plans":active["pending_plans"],
        "recovery":active["recovery"],"record":record,"operation":result["operation"],
        "blueprint_monitor":result.get("blueprint_monitor").map(|m|json!({
            "monitor_id":m["monitor_id"],"recovery":m["recovery"],"history_verified":false,
            "authority_tick_floor":m["authority_tick_floor"],
            "fresh_sample_acquired":false,"sampled_excavation_condition_proven":false})),
        "blueprint_clock":result.get("blueprint_clock").map(|clock|json!({
            "monitor_id":clock["monitor_id"],"recovery":clock["recovery"],"history_verified":false,
            "authority_tick_floor":clock["authority_tick_floor"],
            "pending":clock["pending"],"unresolved":clock["unresolved"],
            "current_pause_proven":false,"retry_commit_permitted":false})),
        "retry_commit_permitted":false,"dispatch_authorized":false}),
    )
}
pub fn packet(
    op: &str,
    mut result: Value,
    c: Option<&OperationContext>,
    binding: Option<&DigBinding>,
    view: Option<&DigSessionView>,
    control_policy: Option<&DigControlPolicy>,
    review: Option<Digest32>,
) -> String {
    let clock_recovery_only = result.get("clock_recovery_only") == Some(&Value::Bool(true));
    let clock_pending = result
        .get("blueprint_clock")
        .is_some_and(|clock| clock["pending"] == true || clock["unresolved"] == true);
    if (clock_pending || clock_recovery_only)
        && let Some(proof) = result.get_mut("blueprint_monitor")
    {
        proof["sampled_excavation_condition_proven"] = json!(false);
        proof["fresh_sample_acquired"] = json!(false);
        if clock_recovery_only {
            proof["original_designations_verified"] = json!(false);
        }
    }

    if clock_recovery_only && let Some(clock) = result.get_mut("blueprint_clock") {
        clock["fresh_goal_sampling_permitted"] = json!(false);
        clock["current_pause_proven"] = json!(false);
    }

    let mut active = empty_active_work();
    active["scope"] = json!("this_dig_journal_only");
    active["inventory_verified"] = json!(view.is_some());
    active["pending_absence_proven"] = json!(view.is_some_and(|v| v.pending.is_none()));
    if let Some(v) = view {
        active["counts"] = inventory(v);
        if let Some(p) = &v.pending {
            active["pending_plans"] = json!([summary(p)]);
            active["confirmation"] = json!({"review_seal":review.map(|v|v.to_string()),"permission_is_process_local":true,
                "dispatch_authorized_by_this_packet":false});
            if review.is_none() {
                active["indeterminate_effects"] = json!([summary(p)]);
            }
            active["recovery"] = json!({"tool":"fortress.explain","arguments":{"session_id":c.map(|c|c.session_id.to_string()),
                "idempotency_key":p.key,"plan_digest":p.plan_digest.to_string()}});
        }
    }
    let blueprint = result.get("blueprint").filter(|v| v.is_object());
    let monitor = result.get("blueprint_monitor").filter(|v| v.is_object());
    let clock = result.get("blueprint_clock").filter(|v| v.is_object());
    let monitor_only = result.get("monitor_only") == Some(&Value::Bool(true));
    let monitor_verified = monitor.is_some_and(|m| m["history_verified"] == true);
    let clock_verified = clock.is_some_and(|clock| clock["history_verified"] == true);
    let sampled_goal = monitor.is_some_and(|m| m["sampled_excavation_condition_proven"] == true);
    let blueprint_verified = blueprint.is_some_and(|p| p["inventory_verified"] == true);
    if let Some(parent) = blueprint {
        active["scope"] = json!("this_blueprint_and_original_dig_journal");
        active["blueprint"] = json!({
            "blueprint_id":parent["blueprint_id"],"journal_id":parent["journal_id"],
            "recovery":parent["recovery"],"layout_digest":parent["layout_digest"],
            "inventory_verified":blueprint_verified,"status":parent["status"],
            "total":parent["total"],"designated":parent["designated"],"next":parent["next"],
            "pending_step":parent["pending_step"],"stopped":parent["stopped"],
            "creation_pending":parent["creation_pending"],"advancement_fenced":parent["advancement_fenced"],
            "excavation_completion_proven":false,"reopening_restores_dispatch_permission":false
        });
    }
    if let Some(proof) = monitor {
        active["blueprint_monitor"] = json!({"monitor_id":proof["monitor_id"],"blueprint_id":proof["blueprint_id"],
            "recovery":proof["recovery"],"history_verified":monitor_verified,"status":proof["status"],
            "original_designations_verified":proof["original_designations_verified"],
            "fresh_sample_acquired":proof["fresh_sample_acquired"],"sampled_excavation_condition_proven":sampled_goal,
            "latest_sample_tick":proof["latest_sample_tick"],"first_satisfied_tick":proof["first_satisfied_tick"],
            "matching_samples":proof["matching_samples"],"matching_since_tick":proof["matching_since_tick"],
            "counts":proof["counts"],"sampling_fenced":proof["sampling_fenced"],
            "shared_process_identity_proven":false,"mining_causality_proven":false});
    }
    if monitor_only {
        active["scope"] = json!("this_blueprint_monitor_history_only");
        active["original_designations_verified"] = json!(false);
    }
    if let Some(clock) = clock {
        active["scope"] = json!("this_blueprint_mining_monitor_and_clock");
        active["blueprint_clock"] = json!({
            "monitor_id":clock["monitor_id"],"recovery":clock["recovery"],
            "history_verified":clock["history_verified"],"pending":clock["pending"],
            "unresolved":clock["unresolved"],"may_be_running":clock["may_be_running"],
            "authority_tick_floor":clock["authority_tick_floor"],"active_run":clock["active_run"],
            "current_pause_proven":false,"clock_receipt_proves_excavation":false,
            "reopening_restores_dispatch_permission":false});
        if clock_pending || clock["history_verified"] != true {
            active["pending_absence_proven"] = json!(false);
        }
    }
    if clock_recovery_only {
        active["scope"] = json!("this_original_monitor_and_clock_history_only");
        active["original_designations_verified"] = json!(false);
        active["clock_inventory_verified"] = json!(clock_verified);
        active["monitor_inventory_verified"] = json!(monitor_verified);
        active["clock_pending_absence_proven"] = json!(clock_verified && !clock_pending);
    }
    let phase = match op {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" | "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let complete_domains = if clock_recovery_only {
        let mut domains = vec![];
        if monitor_verified {
            domains.push("this_blueprint_monitor_history");
        }
        if clock_verified {
            domains.push("this_blueprint_clock_history");
        }
        json!(domains)
    } else if view.is_some() && blueprint_verified {
        json!([
            "this_journal_coordination_inventory",
            "this_blueprint_original_prefix"
        ])
    } else if view.is_some() {
        json!(["this_journal_coordination_inventory"])
    } else if monitor_only && monitor_verified {
        json!(["this_blueprint_monitor_history"])
    } else {
        json!([])
    };
    let mut builder=AgentTurnBuilder::new(op,phase)
        .continuity(ContinuityStatus::Indeterminate,None,Some(json!({"world_history":"unestablished"})),None)
        .briefing(json!({"runtime":if clock_recovery_only{"unadmitted_blueprint_clock_recovery"}else if monitor_only{"unadmitted_blueprint_monitor_recovery"}else{"unadmitted_mining_control"},
            "bridge_protocol":if clock_recovery_only{json!("1.13")}else if monitor_only{Value::Null}else{json!("1.16")},"runtime_admitted":false,
            "monitor_only":monitor_only,"clock_protocol":clock.map(|_|"1.13"),
            "clock_recovery_only":clock_recovery_only,
            "clock_pending":clock_pending,"current_pause_proven":false,
            "mutation_admissible":false,"development_policy":control_policy.map(policy),
            "current_terrain_proven":false,"excavation_completion_proven":false,
            "whole_blueprint_sampled_floor_condition_proven":sampled_goal,
            "monitor_history_verified":monitor_verified}))
        .active_work(active)
        .coverage(json!({"status":"partial","complete_domains":complete_domains,
            "partial_domains":["bounded_mining_evidence","sampled_blueprint_excavation"],
            "omitted_domains":["continuous_excavation_state","structural_safety","other_controllers","game_checkpoint"]}))
        .uncertainty(vec![uncertainty("mining-not-excavation","unknown",
            "Designation proof is historical configuration, not excavation completion or structural safety.",
            "Preserve unresolved work and recover its exact key; never infer nonapplication from a lost reply.",None,Value::Null)]);
    if let Some(c) = c {
        let mut budget = empty_budget();
        budget["admitted"] = json!({"max_wall_millis":c.budget.max_wall_millis,
            "max_bytes":c.budget.max_bytes,"max_output_tokens":c.budget.max_output_tokens,
            "max_entities":c.budget.max_entities,"max_game_ticks":c.budget.max_game_ticks,"max_actions":1});
        budget["reserved_output_bytes"] = json!(OUTPUT_BYTES);
        budget["accounting"] = json!("conservative_byte_reservations_not_measured_tokens");
        let mut next = vec![];
        if let Some(pending) = view.and_then(|v| v.pending.as_ref()) {
            if let Some(seal) = review {
                next.push(recommendation("confirm-original-mining","fortress.commit",
                    "Confirm this exact process-local review for the original pending region.",
                    "high","high","guarded","not_reversible",true,
                    json!({"session_id":c.session_id.to_string(),"idempotency_key":pending.key,
                        "plan_digest":pending.plan_digest.to_string(),"review_seal":seal.to_string()})));
            } else {
                next.push(recommendation("recover-original-mining","fortress.explain",
                    "Inspect the exact original pending operation before reconciliation; never retry its commit.",
                    "high","high","read_only","not_applicable",false,
                    json!({"session_id":c.session_id.to_string(),"idempotency_key":pending.key,
                        "plan_digest":pending.plan_digest.to_string()})));
            }
        } else if let Some(parent) =
            blueprint.filter(|p| p["inventory_verified"] == true && p["status"] == "ready")
        {
            let permitted = binding.is_some_and(|b| {
                c.authorize(Capability::Plan, RiskTier::Guarded, &[], Some(b.scope()))
                    .is_ok()
                    && c.authorize(
                        Capability::Designate,
                        RiskTier::Guarded,
                        &[],
                        Some(b.scope()),
                    )
                    .is_ok()
                    && c.authorize(
                        Capability::Observe,
                        RiskTier::ReadOnly,
                        &[],
                        Some(b.scope()),
                    )
                    .is_ok()
            });
            if permitted {
                let child = &parent["next"];
                let capture = result.get("observation").filter(|o| {
                    o["region"] == child["region"]
                        && o["witness"].is_string()
                        && o["blockers_without_hidden_acknowledgement"]
                            .as_array()
                            .is_some_and(Vec::is_empty)
                });
                if let Some(capture) = capture {
                    next.push(recommendation("review-next-blueprint-region","fortress.plan",
                        "Prepare a review for the next original region using its fresh selected observation.",
                        "high","high","guarded","not_applicable",false,
                        json!({"session_id":c.session_id.to_string(),"idempotency_key":child["idempotency_key"],
                            "observation_witness":capture["witness"],"allow_hidden_neighbors":false})));
                } else {
                    next.push(recommendation("observe-next-blueprint-region","fortress.observe",
                        "Read the next original region before preparing its separate review.",
                        "high","high","read_only","not_applicable",false,
                        json!({"session_id":c.session_id.to_string(),"region":child["region"].to_string()})));
                }
            }
        }
        if let Some(proof) = monitor.filter(|m| {
            m["history_verified"] == true
                && m["terminal"] == false
                && m["sampling_fenced"] == false
                && m["attempt_limit_reached"] == false
        }) {
            let can_observe = !monitor_only
                && !clock_recovery_only
                && !clock_pending
                && binding.is_some_and(|b| {
                    c.authorize(
                        Capability::Observe,
                        RiskTier::ReadOnly,
                        &[],
                        Some(b.scope()),
                    )
                    .is_ok()
                });
            if can_observe {
                next.push(recommendation("sample-original-blueprint","fortress.wait",
                    "Acquire one whole original blueprint capture and advance its fixed sampled stability goal.",
                    "high","high","read_only","not_applicable",false,
                    json!({"session_id":c.session_id.to_string(),"monitor_id":proof["monitor_id"]})));
            }
        }
        if let Some(clock) = clock.filter(|clock| clock["monitor_id"].is_string()) {
            let id = &clock["monitor_id"];
            let run = &clock["active_run"];
            let verified = clock["history_verified"] == true;
            let can_control = !monitor_only
                && c.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)
                    .is_ok();
            let can_plan = can_control
                && !clock_recovery_only
                && binding.is_some_and(|b| {
                    c.authorize(Capability::Plan, RiskTier::Guarded, &[], Some(b.scope()))
                        .is_ok()
                        && c.authorize(
                            Capability::Observe,
                            RiskTier::ReadOnly,
                            &[],
                            Some(b.scope()),
                        )
                        .is_ok()
                });
            if verified && run["review_seal"].is_string() && run["state"] == "prepared" && can_plan
            {
                next.push(recommendation("confirm-blueprint-clock","fortress.commit",
                    "Confirm this separately reviewed finite interval for the original blueprint.",
                    "high","high","guarded","not_reversible",true,
                    json!({"session_id":c.session_id.to_string(),"scope":"clock","monitor_id":id,"review_seal":run["review_seal"]})));
            } else if verified
                && clock["needs_reconciliation"] == true
                && clock["mode"] != "offline"
                && result["profile"] != "clock-offline"
                && result["mode"] != "clock-offline"
            {
                next.push(recommendation("reconcile-blueprint-clock","fortress.wait",
                    "Query the retained run once before another excavation sample; this cannot unpause.",
                    "high","high","read_only","not_applicable",false,
                    json!({"session_id":c.session_id.to_string(),"scope":"clock","monitor_id":id})));
            }
            if verified && clock_pending && run["terminal"] == false && can_control {
                next.push(recommendation("stop-blueprint-clock","fortress.cancel",
                    "Retire this interval or request its native safety pause while preserving the original run.",
                    "high","high","guarded","not_reversible",false,
                    json!({"session_id":c.session_id.to_string(),"scope":"clock","monitor_id":id})));
            }
            if (verified && clock["unresolved"] == true && run["terminal"] == true)
                || (clock_recovery_only && clock_pending && !can_control)
            {
                next.push(recommendation("release-unresolved-blueprint-clock","fortress.cancel",
                    "Preserve the unresolved run reference and release local custody without claiming a pause.",
                    "high","high","read_only","not_applicable",false,
                    json!({"session_id":c.session_id.to_string(),"scope":"session","release_for_recovery":true})));
            }
            next.push(recommendation("inspect-blueprint-clock","fortress.query",
                "Inspect the exact retained clock history and its pending effect before new work.",
                "high","high","read_only","not_applicable",false,
                json!({"session_id":c.session_id.to_string(),"query":json!({"mode":"blueprint_clock","monitor_id":id,"offset":0,"limit":8}).to_string()})));
        }
        let query = if let Some(proof) = monitor.filter(|m| m["monitor_id"].is_string()) {
            json!({"mode":"blueprint_monitor","monitor_id":proof["monitor_id"],"offset":0,"limit":16})
        } else if let Some(parent) = blueprint.filter(|p| p["blueprint_id"].is_string()) {
            json!({"mode":"blueprint","blueprint_id":parent["blueprint_id"],"offset":0,"limit":8})
        } else {
            json!({"mode":"records","limit":8})
        };
        next.push(recommendation(
            "discover-mining",
            "fortress.query",
            "Inspect retained mining obligations before new work.",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({"session_id":c.session_id.to_string(),"query":query.to_string()}),
        ));
        builder = builder
            .session_id(c.session_id.to_string())
            .request_id(c.request_id.to_string())
            .budget(budget)
            .recommendations(next);
    }
    if let (Some(b), Some(v)) = (binding, view) {
        builder=builder.anchor(json!({"fortress_id":b.fortress_id().to_string(),"epoch":b.manifest().generation,
            "sequence":v.events,"state_hash":v.head.to_string(),"game_tick":Value::Null,"scope":"coordination_root_not_world_state"}));
    }
    let mut turn = builder.build();
    if let Some(b) = turn["briefing"].as_object_mut() {
        b.remove("admission");
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}
