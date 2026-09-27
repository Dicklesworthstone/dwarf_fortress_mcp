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
    for (i, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
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
    if verified.is_none() {
        if let Some(hint) = result.get("pending_identity_hint").filter(|v| !v.is_null()) {
            active["unverified_operation_identity"] = hint.clone();
            active["pending_absence_proven"] = json!(false);
        }
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
    let phase = match op {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" if result.get("completion").is_some() => AgentPhase::Verify,
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
            if let Some(reference) = reference {
                if !references.contains(reference) {
                    references.push(reference.clone());
                }
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
            "max_output_tokens":c.budget.max_output_tokens,"max_game_ticks":0,"max_actions":1});
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
        if completion["inventory_verified"] == true && completion["assessments_complete"] == true {
            if let Some(domains) = turn["coverage"]["complete_domains"].as_array_mut() {
                domains.push(json!("original_plan_sampled_construction_condition"));
            }
        }
    }
    if let Some(summary) = result.get("source_summary").filter(|v| !v.is_null()) {
        turn["briefing"]["native_source_summary"] = summary.clone();
    }
    // No canonical world adapter exists for this protocol. A journal root or
    // native generation must never be invented as a canonical world anchor.
    turn["anchor"] = Value::Null;
    if let Some(b) = turn["briefing"].as_object_mut() {
        b.remove("admission");
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}
