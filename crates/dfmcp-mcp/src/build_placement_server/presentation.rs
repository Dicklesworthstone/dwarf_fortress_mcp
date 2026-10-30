//! Historical native evidence projected through the common Agent Turn builder.
use super::{error, policy::Policy};
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work,
    empty_budget, recommendation, uncertainty,
};
use dfmcp_adapter::build_placement::journal::{BuildEntry, BuildInventory};
use dfmcp_adapter::build_placement::{
    BuildBinding, BuildCapture, BuildNativeSummary, BuildSelection,
};
use dfmcp_core::{Digest32, ErrorCode, MapCuboid, OperationContext, Result};
use serde_json::{Value, json};

pub(super) const OUTPUT_BYTES: u64 = 65536;
pub(super) fn digest(raw: &str) -> Result<Digest32> {
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
    let mut out = [0; 32];
    for (i, pair) in raw.as_bytes().as_chunks::<2>().0.iter().enumerate() {
        let s = std::str::from_utf8(pair)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
        out[i] = u8::from_str_radix(s, 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
    }
    Ok(Digest32::from_bytes(out))
}
fn hex(bytes: &[u8]) -> String {
    const CHARS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(CHARS[usize::from(b >> 4)] as char);
        out.push(CHARS[usize::from(b & 15)] as char);
    }
    out
}
pub(super) fn area(v: MapCuboid) -> Value {
    json!([v.min.x, v.min.y, v.min.z, v.max.x, v.max.y, v.max.z])
}
pub(super) fn selection(v: BuildSelection) -> Value {
    let [x, y, z] = v.target();
    json!([v.kind().as_str(), v.item_id(), x, y, z])
}
pub(super) fn native_reference(c: &BuildCapture) -> Value {
    json!({"kind":"build_native_capture","protocol":"1.19","generation":c.generation(),"sequence":c.sequence(),
        "game_tick":c.tick(),"world_folder":c.fortress().folder(),"site_id":c.fortress().site(),
        "capture_sha256":c.witness().to_string(),"canonical_world_anchor":false})
}
pub(super) fn observation(c: &BuildCapture) -> Value {
    json!({"selection":selection(c.selection()),"observation_witness":c.witness().to_string(),"eligible":c.eligible(),
        "eligibility_scope":"captured_item_and_target_only",
        "blockers":c.blockers().iter().map(|b|b.as_str()).collect::<Vec<_>>(),"paused_at_capture":c.paused(),
        "dimensions":c.dimensions(),"native_reference":native_reference(c),"selected_item_position":c.item_position(),
        "canonical_capture_hex":hex(c.canonical_bytes()),"historical_evidence_only":true,
        "building_completion_proven":false,"query":{"mode":"selection","witness":c.witness().to_string()}})
}
pub(super) fn source_summary(s: BuildNativeSummary) -> Value {
    let mut blockers = Vec::new();
    if s.unresolved() {
        blockers.push("native_unresolved_effect");
    }
    if s.retained_records() == 256 {
        blockers.push("native_retention_full");
    }
    json!({"unresolved":s.unresolved(),"retained_records":s.retained_records(),
        "prepare_available":s.prepare_available(),"preparation_blockers":blockers,"historical_last_native_reply":true,
        "journal_local_pending_absence_implies_native_absence":false,
        "authorizes_new_preparation":false})
}
pub(super) fn summary(e: &BuildEntry) -> Value {
    json!({"idempotency_key":e.plan().key(),"plan_digest":e.plan().digest().to_string(),
        "state":if e.unresolved() && !e.needs_reconciliation(){"indeterminate"}else{e.state().as_str()},
        "coordinator_state":e.state().as_str(),"unresolved":e.unresolved(),"dispatch_started":e.dispatch_started(),
        "cancel_requested":e.cancel_requested(),"native_phase":e.native().map(|v|v.phase().as_str()),
        "receipt_digest":e.native().map(|v|v.receipt().to_string()),"retry_commit_permitted":false})
}
pub(super) fn record(e: &BuildEntry) -> Value {
    json!({"summary":summary(e),"review":observation(e.plan().before()),"native":e.native().map(|n|json!({
        "phase":n.phase().as_str(),"reason":n.reason().as_str(),"attempted":n.attempted(),"resolved":n.resolved(),
        "receipt_digest":n.receipt().to_string(),"canonical_record_hex":hex(n.canonical_bytes()),
        "after_reference":n.after().map(native_reference),
        "insertion":n.insertion().map(|v|json!({"building_id":v.building_id(),"job_id":v.job_id(),"item_id":v.item_id(),
            "kind":v.kind().as_str(),"position":v.position(),"material":v.material(),"material_index":v.material_index(),
            "stage":v.stage(),"max_stage":v.max_stage(),"linked":v.linked(),"construct_job":v.construct_job(),
            "exact_item_link":v.exact_item_link(),"suspended":v.suspended(),"historical_evidence_only":true}))})),
        "historical_evidence_only":true,"placed_means":"exact stage-zero building and construction job registration",
        "building_completion_proven":false,"current_building_usable_proven":false,"retry_commit_permitted":false})
}
pub(super) fn inventory(v: &BuildInventory) -> Value {
    json!({"journal_id":v.journal_id.to_string(),"head":v.head.to_string(),"frames":v.frames,"byte_length":v.byte_len,
        "total_records":v.entries().len(),"unresolved_records":v.entries().iter().filter(|e|e.unresolved()).count()})
}
pub(super) fn policy(p: &Policy) -> Value {
    json!({"digest":p.digest.to_string(),"journal_id":p.journal.to_string(),"lease_id":p.lease.to_string(),
        "checkpoint_policy":p.config.checkpoint.as_str(),"checkpoint_verified":false,"configured_scope":area(p.config.scope),
        "protected_regions":p.config.protected.iter().copied().map(area).collect::<Vec<_>>(),
        "host_lease_ticks":1200,"global_controller_fence":false})
}
pub(super) fn failure(e: &dfmcp_core::DfmcpError) -> Value {
    json!({"ok":false,"error":{"code":e.code.as_str(),"message":match e.code{
        ErrorCode::CheckpointRequired=>"A verified game checkpoint is required and unavailable in this development profile.",
        ErrorCode::LeaseDenied=>"The original host lease is absent, expired, or does not cover the selected item and target.",
        ErrorCode::EffectIndeterminate=>"The original furniture effect remains unresolved. Recover its exact key; never retry commit.",
        ErrorCode::CorruptLedger=>"Journal custody or integrity failed. Preserve the original evidence for recovery.",
        ErrorCode::BudgetExceeded=>"Complete furniture work and response exceed the request allowance.",
        ErrorCode::InvalidRequest=>"The furniture request does not match its closed schema and bounds.",
        _=>"Furniture work failed current authority, source, review, custody, runtime, or policy checks."}},
        "retry_commit_permitted":false,"effect_outcome_inferred":false,
        "recovery_class":if matches!(e.code,ErrorCode::EffectIndeterminate|ErrorCode::CancellationIncomplete){"reconciliation_required"}else{"never_unchanged"}})
}
#[allow(clippy::too_many_arguments)]
pub(super) fn packet(
    op: &str,
    result: Value,
    c: Option<&OperationContext>,
    binding: Option<&BuildBinding>,
    verified: Option<&BuildInventory>,
    historical: Option<&BuildInventory>,
    policy_value: Option<&Policy>,
    review: Option<Digest32>,
    selected: Option<&BuildCapture>,
) -> String {
    let mut active = empty_active_work();
    active["scope"] = json!("this_build_journal_only");
    active["inventory_verified"] = json!(verified.is_some());
    active["pending_absence_proven"] = json!(verified.is_some_and(|v| v.pending().is_none()));
    let retained = verified.or(historical);
    if let Some(v) = retained {
        active["counts"] = inventory(v);
        active["counts_currently_verified"] = json!(verified.is_some());
        let pending = v
            .entries()
            .iter()
            .filter(|e| e.unresolved())
            .map(summary)
            .collect::<Vec<_>>();
        active["pending_plans"] = json!(pending);
        active["historical_identity_only"] = json!(verified.is_none());
        if let Some(p) = v.pending() {
            if review.is_none() || p.dispatch_started() {
                active["indeterminate_effects"] = json!([summary(p)]);
            }
            active["confirmation"] = json!({"review_seal":review.map(|s|s.to_string()),"permission_is_process_local":true,
                "dispatch_authorized_by_this_packet":false});
            active["recovery"] = json!({"tool":"fortress.explain","arguments":{"session_id":c.map(|c|c.session_id.to_string()),
                "idempotency_key":p.plan().key(),"plan_digest":p.plan().digest().to_string()}});
        }
    }
    if verified.is_none()
        && let Some(hint) = result.get("pending_identity_hint").filter(|v| !v.is_null())
    {
        active["unverified_operation_identity"] = hint.clone();
        active["pending_absence_proven"] = json!(false);
    }
    if let Some(batch) = result.get("batch") {
        active["scope"] = json!("this_furniture_batch_and_original_build_journal");
        active["original_build_journal_inventory_verified"] = json!(verified.is_some());
        active["inventory_verified"] =
            json!(verified.is_some() && batch["inventory_verified"] == true);
        if batch["inventory_verified"] != true {
            active["pending_absence_proven"] = json!(false);
        }
        // Keep the complete plan and every original step exactly once in the
        // result. The common handoff spine names their location and identity.
        active["furniture_batch"] = json!({"inventory_path":"result.batch",
            "batch_id":batch["batch_id"],"plan_digest":batch["plan_digest"],
            "inventory_verified":batch["inventory_verified"],"status":batch["status"],
            "stopped":batch["stopped"],"advancement_fenced":batch["advancement_fenced"],
            "placed":batch["placed"],"total":batch["total"],"pending_step":batch["pending_step"],
            "next":batch["next"],"construction_completion_proven":false});
    }
    if let Some(completion) = result.get("completion") {
        active["scope"] = json!("original_furniture_batch_placement_and_completion");
        active["inventory_verified"] =
            json!(active["inventory_verified"] == true && completion["inventory_verified"] == true);
        if completion["inventory_verified"] != true || completion["terminal"] != true {
            active["pending_absence_proven"] = json!(false);
        }
        active["construction_completion"] = json!({
            "inventory_path":"result.completion","monitor_id":completion["monitor_id"],
            "goal_digest":completion["goal_digest"],"origin_digest":completion["origin_digest"],
            "phase":completion["phase"],"reason":completion["reason"],
            "origin_verified":completion["origin_verified"],"inventory_verified":completion["inventory_verified"],
            "monitor_inventory_verified":completion["monitor_inventory_verified"],
            "monitor_terminal":completion["monitor_terminal"],
            "read_outcome_unknown":completion["read_outcome_unknown"],
            "sampled_condition_satisfied":completion["sampled_condition_satisfied"],
            "observations":completion["observations"],"streak":completion["streak"],
            "deadline":completion["timing"]["deadline"],"last_tick":completion["last_tick"]
        });
        if completion["monitor_terminal"] != true {
            active["obligations"] = json!([active["construction_completion"].clone()]);
        }
    }
    if let Some(clock) = result.get("construction_clock") {
        active["scope"] = json!("original_furniture_batch_completion_and_clock");
        active["construction_clock"] = json!({
            "inventory_path":"result.construction_clock","monitor_id":clock["monitor_id"],
            "recovery":clock["recovery"],"history_verified":clock["history_verified"],
            "pending":clock["pending"],"unresolved":clock["unresolved"],
            "active_run":clock["active_run"],"current_pause_proven":false,
            "fresh_goal_sampling_permitted":clock["fresh_goal_sampling_permitted"]
        });
        if clock["history_verified"] != true
            || clock["pending"] == true
            || clock["unresolved"] == true
        {
            active["pending_absence_proven"] = json!(false);
            active["inventory_verified"] = json!(false);
        }
    }
    if let Some(clock) = result.get("room_clock") {
        active["room_clock"] = json!({"inventory_path":"result.room_clock","monitor_id":clock["monitor_id"],
            "recovery":clock["recovery"],"history_verified":clock["history_verified"],
            "authority_tick_floor":clock["authority_tick_floor"],"pending":clock["pending"],
            "unresolved":clock["unresolved"],"active_run":clock["active_run"],
            "bootstrap_sample_required":clock["bootstrap_sample_required"],
            "initial_plan_required":clock["initial_plan_required"],"current_pause_proven":false});
        if clock["history_verified"] != true
            || clock["pending"] == true
            || clock["unresolved"] == true
        {
            active["pending_absence_proven"] = json!(false);
        }
    }
    if let Some(room) = result.get("room_completion") {
        active["scope"] = json!("original_furniture_batch_and_joint_room_goal");
        active["room_completion"] = json!({"inventory_path":"result.room_completion",
            "monitor_id":room["monitor_id"],"recovery":room["recovery"],
            "room_intent_digest":room["room_intent_digest"],"origin_digest":room["origin_digest"],
            "inventory_verified":room["inventory_verified"],"origin_verified":room["origin_verified"],
            "monitor_inventory_verified":room["monitor_inventory_verified"],
            "phase":room["phase"],"reason":room["reason"],"terminal":room["monitor_terminal"],
            "sampled_condition_satisfied":room["sampled_condition_satisfied"],
            "read_outcome_unknown":room["read_outcome_unknown"],
            "authority_tick_floor":room["authority_tick_floor"],"deadline":room["timing"]["deadline"],
            "observations":room["observations"],"streak":room["streak"],
            "sampling_permitted":room["sampling_permitted"],"local_cancel_permitted":room["local_cancel_permitted"]});
        if room["inventory_verified"] != true {
            active["inventory_verified"] = json!(false);
        }
        if room["monitor_terminal"] != true {
            active["pending_absence_proven"] = json!(false);
            if !active["obligations"].is_array() {
                active["obligations"] = json!([]);
            }
            let obligation = active["room_completion"].clone();
            if let Some(obligations) = active["obligations"].as_array_mut() {
                obligations.push(obligation);
            }
        }
    }
    let phase = match op {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe"
            if result.get("completion").is_some() || result.get("room_completion").is_some() =>
        {
            AgentPhase::Verify
        }
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" | "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut references = Vec::new();
    if let Some(s) = selected.or_else(|| {
        retained
            .and_then(|v| v.pending())
            .map(|p| p.plan().before())
    }) {
        references.push(native_reference(s));
    }
    for name in ["plan", "effect", "record"] {
        if let Some(value) = result.get(name) {
            let reference = value
                .get("native")
                .and_then(|v| v.get("after_reference"))
                .filter(|v| !v.is_null())
                .or_else(|| value.get("review").and_then(|v| v.get("native_reference")));
            if let Some(reference) = reference
                && !references.contains(reference)
            {
                references.push(reference.clone());
            }
        }
    }
    if let Some(v) = retained {
        references.push(json!({"kind":"build_coordination_root","journal_id":v.journal_id.to_string(),
        "head":v.head.to_string(),"frames":v.frames,"currently_verified":verified.is_some(),"canonical_world_anchor":false}));
    }
    if let Some(batch) = result.get("batch") {
        references.push(
            json!({"kind":"furniture_batch_definition","batch_id":batch["batch_id"],
            "plan_digest":batch["plan_digest"],"journal_id":batch["journal_id"],
            "currently_verified":batch["inventory_verified"],"canonical_world_anchor":false}),
        );
    }
    if let Some(completion) = result.get("completion") {
        references.push(json!({"kind":"original_plan_sampled_construction_condition",
            "monitor_id":completion["monitor_id"],"goal_digest":completion["goal_digest"],
            "origin_digest":completion["origin_digest"],"capture_sha256":completion["capture_sha256"],
            "game_tick":completion["last_tick"],"source":completion["source"],
            "currently_verified":completion["inventory_verified"],"canonical_world_anchor":false}));
    }
    if let Some(clock) = result.get("construction_clock") {
        references.push(json!({"kind":"original_construction_clock_journal",
            "monitor_id":clock["monitor_id"],"recovery":clock["recovery"],
            "currently_verified":clock["history_verified"],"canonical_world_anchor":false}));
    }
    if let Some(clock) = result.get("room_clock") {
        references.push(
            json!({"kind":"original_joint_room_clock_journal","monitor_id":clock["monitor_id"],
            "recovery":clock["recovery"],"authority_tick_floor":clock["authority_tick_floor"],
            "currently_verified":clock["history_verified"],"canonical_world_anchor":false}),
        );
    }
    if let Some(room) = result.get("room_completion") {
        references.push(json!({"kind":"original_room_joint_sampled_condition",
            "monitor_id":room["monitor_id"],"room_intent_digest":room["room_intent_digest"],
            "origin_digest":room["origin_digest"],"capture_sha256":room["capture_sha256"],
            "game_tick":room["last_tick"],"authority_tick_floor":room["authority_tick_floor"],
            "recovery":room["recovery"],"source":room["source"],
            "currently_verified":room["inventory_verified"],"canonical_world_anchor":false}));
    }
    let mut builder=AgentTurnBuilder::new(op,phase).profile(if matches!(op,"fortress.plan"|"fortress.observe"){ObservationProfile::Tactical}else{ObservationProfile::Forensic})
        .continuity(ContinuityStatus::Indeterminate,None,Some(json!({"world_history":"unestablished","canonical_world_anchor_available":false})),None)
        .briefing(json!({"runtime":"unadmitted_build_placement_development","bridge_protocol":"1.19","runtime_admitted":false,
            "production_mutation_admissible":false,"development_policy":policy_value.map(policy),
            "source":binding.map(|b|json!({"world_folder":b.fortress().folder(),"site_id":b.fortress().site(),"generation":b.generation()})),
            "building_completion_proven":false,"current_building_usable_proven":false}))
        .active_work(active).references(references)
        .coverage(json!({"status":"partial","complete_domains":if verified.is_some(){json!(["this_journal_coordination_inventory"])}else{json!([])},
            "partial_domains":["exact_item_furniture_registration_evidence"],"omitted_domains":["building_completion","fortress_history","other_controllers","game_checkpoint"]}))
        .uncertainty(vec![uncertainty("furniture-registration-not-completion","unknown",
            "A placed receipt proves historical stage-zero registration of one building and its construction job; completion and current usability are unestablished.",
            "Inspect the original effect evidence and preserve unresolved keys. Lost native retention never proves nonapplication.",None,Value::Null)]);
    if let Some(c) = c {
        let mut budget = empty_budget();
        budget["admitted"] = json!({"max_wall_millis":c.budget.max_wall_millis,"max_bytes":c.budget.max_bytes,
            "max_output_tokens":c.budget.max_output_tokens,"max_game_ticks":c.budget.max_game_ticks,"max_actions":1});
        budget["reserved_output_bytes"] = json!(OUTPUT_BYTES);
        budget["accounting"] = json!("conservative_byte_reservations_not_measured_tokens");
        builder=builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string()).budget(budget)
            .recommendations(vec![recommendation("discover-furniture","fortress.query","Inspect retained furniture obligations before new effects.",
                "high","high","read_only","not_applicable",false,json!({"session_id":c.session_id.to_string(),"query":"{\"mode\":\"records\",\"limit\":8}"}))]);
    }
    let mut turn = builder.build();
    if result.get("batch").is_some() {
        turn["recommendations"] = json!([recommendation(
            "inspect-original-furniture-batch",
            "fortress.query",
            "Inspect the complete original plan and its retained placement prefix before advancing or recovering work.",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({"session_id":c.map(|c|c.session_id.to_string()),"query":"{\"mode\":\"batch\"}"})
        )]);
    }
    if let Some(completion) = result.get("completion") {
        turn["briefing"]["receipt_linked_sampled_completion"] =
            completion["sampled_condition_satisfied"].clone();
        turn["briefing"]["completion_phase"] = completion["phase"].clone();
        turn["uncertainty"] = json!([uncertainty(
            "sampled-completion-scope",
            "unknown",
            "Completion is a historical condition over the whole original plan at sampled game ticks. Current usability, continuous stability and causal attribution remain unproved.",
            "Inspect the fixed goal, all target assessments and original custody before relying on retained evidence.",
            None,
            Value::Null
        )]);
        let live = completion["inventory_verified"] == true && completion["terminal"] != true;
        turn["recommendations"] = json!([recommendation(
            "inspect-original-plan-completion",
            if live {
                "fortress.observe"
            } else {
                "fortress.query"
            },
            if live {
                "Acquire one bounded receipt-linked sample for the fixed original plan."
            } else {
                "Inspect retained completion evidence and original custody."
            },
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            if live {
                json!({"session_id":c.map(|c|c.session_id.to_string()),"selection":"completion"})
            } else {
                json!({"session_id":c.map(|c|c.session_id.to_string()),"query":"{\"mode\":\"completion\"}"})
            }
        )]);
        if completion["inventory_verified"] == true
            && completion["assessments_complete"] == true
            && let Some(domains) = turn["coverage"]["complete_domains"].as_array_mut()
        {
            domains.push(json!("original_plan_sampled_construction_condition"));
        }
    }
    if let Some(room) = result.get("room_completion") {
        turn["briefing"]["joint_original_room_sampled_condition"] =
            room["sampled_condition_satisfied"].clone();
        turn["briefing"]["room_completion_phase"] = room["phase"].clone();
        let uncertainty = uncertainty(
            "joint-room-sampled-scope",
            "unknown",
            "The original terrain masks and all original furnishings share one bracketed sampled condition. The retained result does not prove current usability, continuous stability, room assignment or an atomic snapshot across native profiles.",
            "Inspect the fixed original room, every furnishing assessment and bounded terrain deficits.",
            None,
            Value::Null,
        );
        if let Some(values) = turn["uncertainty"].as_array_mut() {
            values.push(uncertainty);
        }
        turn["recommendations"] = json!([room_recommendation(room, c)]);
        if room["inventory_verified"] == true
            && room["assessments_complete"] == true
            && let Some(domains) = turn["coverage"]["complete_domains"].as_array_mut()
        {
            domains.push(json!("original_room_joint_sampled_condition"));
        }
        if room["local_cancel_permitted"] == true {
            turn["active_work"]["room_completion"]["cancellation"] = json!({"tool":"fortress.cancel",
                "arguments":{"session_id":c.map(|c|c.session_id.to_string()),"scope":"room_completion"},
                "effect":"local goal cancellation only; placement effects and clock obligations remain"});
        }
    }
    if let Some(summary) = result.get("source_summary").filter(|v| !v.is_null()) {
        turn["briefing"]["native_source_summary"] = summary.clone();
    }
    if let Some(clock) = result.get("construction_clock") {
        let id = clock["monitor_id"].clone();
        let session = c.map(|c| c.session_id.to_string());
        let seal = &clock["active_run"]["review_seal"];
        let verified = clock["history_verified"] == true;
        let pending = clock["pending"] == true || clock["unresolved"] == true;
        let recovery = result["clock_recovery_only"] == true;
        let closed = result["closed"] == true;
        let can_reconcile = !recovery || result["clock_reconciliation_permitted"] == true;
        let can_cancel = !recovery || result["clock_safety_cancel_permitted"] == true;
        turn["briefing"]["construction_clock"] = json!({
            "history_verified":verified,"pending":pending,
            "current_pause_proven":false,"goal_sample_published":false,
            "separate_review_required":!recovery,"clock_recovery_only":recovery,
            "source_has_named_fortress_identity":false});
        let inspect = json!({"session_id":session,"query":json!({
            "mode":"construction_clock","monitor_id":id,
            "head":clock["recovery"]["head"],"limit":8}).to_string()});
        let next = if !recovery && verified && seal.is_string() {
            recommendation(
                "review-construction-clock",
                "fortress.commit",
                "Review the complete original construction goal and finite interval, then confirm this separate clock seal.",
                "high",
                "high",
                "guarded",
                "bounded_run_not_rollback",
                true,
                json!({"session_id":session,"scope":"clock","monitor_id":id,"review_seal":seal}),
            )
        } else if verified && pending && can_reconcile {
            recommendation(
                "reconcile-construction-clock",
                "fortress.wait",
                if recovery {
                    "Recover one outcome of the original interval; independent recovery cannot start runs or sample construction."
                } else {
                    "Reconcile the retained original interval before another run or completion sample."
                },
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({"session_id":session,"scope":"clock","monitor_id":id}),
            )
        } else if !verified || recovery {
            recommendation(
                "inspect-construction-clock",
                "fortress.query",
                if recovery {
                    "Inspect the exact original clock history; this recovery profile provides historical evidence only."
                } else {
                    "Verify the exact retained clock history before using any completion claim."
                },
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                inspect,
            )
        } else {
            recommendation(
                "sample-complete-furniture-goal",
                "fortress.observe",
                "Acquire one separate receipt-linked sample of every original furnishing; the clock receipt proves no construction goal.",
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({"session_id":session,"selection":"completion"}),
            )
        };
        turn["recommendations"] = if recovery && closed {
            json!([])
        } else {
            json!([next])
        };
        if pending && can_cancel && !(recovery && closed) {
            turn["active_work"]["construction_clock"]["cancellation"] = json!({
                "tool":"fortress.cancel","arguments":{"session_id":session,
                    "scope":"clock","monitor_id":id},
                "requires":"current separately retained clock authority"});
        }
    }
    if let (Some(room), Some(clock)) = (
        result.get("room_completion"),
        result.get("construction_clock"),
    ) && clock["history_verified"] == true
        && clock["pending"] != true
        && clock["unresolved"] != true
    {
        turn["recommendations"] = json!([room_recommendation(room, c)]);
    }
    if let Some(clock) = result.get("room_clock") {
        let id = clock["monitor_id"].clone();
        let session = c.map(|c| c.session_id.to_string());
        let verified = clock["history_verified"] == true;
        let pending = clock["pending"] == true || clock["unresolved"] == true;
        let offline = clock["mode"] == "offline";
        let seal = &clock["active_run"]["review_seal"];
        let ordinary_blocks = result.get("construction_clock").is_some_and(|other| {
            other["history_verified"] != true
                || other["pending"] == true
                || other["unresolved"] == true
        });
        turn["briefing"]["room_clock"] = json!({"history_verified":verified,"pending":pending,
            "current_pause_proven":false,"goal_sample_published":false,"separate_review_required":true,
            "source_has_named_fortress_identity":false,"active_dig_work_proven":false,
            "known_terrain_deficits":clock["readiness"]["terrain"]["deficit_count"]});
        let inspect = json!({"session_id":session,"query":json!({"mode":"room_clock","monitor_id":id,
            "head":clock["recovery"]["head"],"limit":8}).to_string()});
        if !ordinary_blocks {
            let next = if verified && seal.is_string() && !offline {
                recommendation(
                    "review-original-room-clock",
                    "fortress.commit",
                    "Review the original room terrain deficits, all furnishings and finite interval, then confirm this separate room-clock seal.",
                    "high",
                    "high",
                    "guarded",
                    "bounded_run_not_rollback",
                    true,
                    json!({"session_id":session,"scope":"room_clock","monitor_id":id,"review_seal":seal}),
                )
            } else if verified && pending && !offline {
                recommendation(
                    "reconcile-original-room-clock",
                    "fortress.wait",
                    "Reconcile the original room interval before another run or joint sample.",
                    "high",
                    "high",
                    "read_only",
                    "not_applicable",
                    false,
                    json!({"session_id":session,"scope":"room_clock","monitor_id":id}),
                )
            } else if clock["initial_plan_required"] == true {
                let ticks = c
                    .map_or(0, |c| c.budget.max_game_ticks)
                    .min(
                        result["room_completion"]["timing"]["max_gap"]
                            .as_u64()
                            .map_or(0, |v| v),
                    )
                    .min(
                        result["room_completion"]["timing"]["deadline"]
                            .as_u64()
                            .map_or(0, |deadline| {
                                deadline.saturating_sub(c.map_or(0, |c| c.anchor.tick.get()))
                            }),
                    );
                let allowed = ticks > 0
                    && c.is_some_and(|c| {
                        c.authorize(
                            dfmcp_core::Capability::Plan,
                            dfmcp_core::RiskTier::Guarded,
                            &[],
                            None,
                        )
                        .is_ok()
                            && c.authorize(
                                dfmcp_core::Capability::ControlClock,
                                dfmcp_core::RiskTier::Guarded,
                                &[],
                                None,
                            )
                            .is_ok()
                    });
                if allowed {
                    recommendation(
                        "prepare-original-room-clock",
                        "fortress.plan",
                        "Initialize the absent room clock under current planning authority and review one finite interval for the exact joint goal.",
                        "high",
                        "high",
                        "guarded",
                        "bounded_run_not_rollback",
                        false,
                        json!({"session_id":session,"scope":"room_clock","clock":json!({"monitor_id":id,
                            "game_ticks":1,"wall_millis":c.map_or(1,|c|c.budget.max_wall_millis.min(5000))}).to_string()}),
                    )
                } else {
                    room_recommendation(&result["room_completion"], c)
                }
            } else if clock["bootstrap_sample_required"] == true {
                room_recommendation(&result["room_completion"], c)
            } else if verified && !pending && !offline {
                room_recommendation(&result["room_completion"], c)
            } else {
                recommendation(
                    "inspect-original-room-clock",
                    "fortress.query",
                    "Inspect exact retained room-clock history; historical readiness and pause receipts establish no current room condition.",
                    "high",
                    "high",
                    "read_only",
                    "not_applicable",
                    false,
                    inspect,
                )
            };
            turn["recommendations"] = json!([next]);
        }
        if pending && clock["mode"] == "control" {
            turn["active_work"]["room_clock"]["cancellation"] = json!({"tool":"fortress.cancel",
                "arguments":{"session_id":session,"scope":"room_clock","monitor_id":id},
                "requires":"current separately retained room clock authority"});
        }
    }
    if result["closed"] == true {
        turn["recommendations"] = json!([]);
        for name in ["room_completion", "construction_clock", "room_clock"] {
            if let Some(domain) = turn["active_work"]
                .get_mut(name)
                .and_then(Value::as_object_mut)
            {
                domain.remove("cancellation");
            }
        }
    }
    // No canonical world adapter exists for this protocol. A journal root or
    // native generation must never be invented as a canonical world anchor.
    turn["anchor"] = Value::Null;
    if let Some(b) = turn["briefing"].as_object_mut() {
        b.remove("admission");
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}

fn room_recommendation(room: &Value, c: Option<&OperationContext>) -> Value {
    let live = room["sampling_permitted"] == true;
    recommendation(
        "inspect-original-joint-room",
        if live {
            "fortress.observe"
        } else {
            "fortress.query"
        },
        if live {
            "Acquire one bounded joint sample of the original room terrain and every furnishing."
        } else {
            "Inspect the retained original joint room goal and exact recovery identity."
        },
        "high",
        "high",
        "read_only",
        "not_applicable",
        false,
        if live {
            json!({"session_id":c.map(|c|c.session_id.to_string()),"selection":"room_completion"})
        } else {
            json!({"session_id":c.map(|c|c.session_id.to_string()),"query":"{\"mode\":\"room_completion\"}"})
        },
    )
}
