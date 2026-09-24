//! Bounded, authority-free mining history and recovery orientation.
use super::error;
use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work, empty_budget,
    recommendation, uncertainty,
};
use dfmcp_adapter::dig_designation::journal::session::DigSessionView;
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigMode, DigRecord, DigSummary};
use dfmcp_adapter::dig_designation::{DigBlocker, DigReason, DigTile};
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result};
use serde_json::{Value, json};

pub const OUTPUT_BYTES: u64 = 32 * 1024;
pub const MAX_TILE_PAGE: usize = 16;

pub fn digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "expected a lowercase SHA-256 digest",
        ));
    }
    let mut value = [0; 32];
    for (i, pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        let text = std::str::from_utf8(pair)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
        value[i] = u8::from_str_radix(text, 16)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid digest"))?;
    }
    Ok(Digest32::from_bytes(value))
}
pub fn mode_name(mode: DigMode) -> &'static str {
    match mode {
        DigMode::Offline => "offline",
        DigMode::Recover => "recover",
        DigMode::Control => "refused_control",
    }
}
pub fn summary(r: &DigSummary) -> Value {
    json!({
        "idempotency_key":r.key,"plan_digest":r.plan_digest.to_string(),"state":r.state.as_str(),
        "native_phase":r.native_phase.map(|p| p.as_str()),"receipt_digest":r.receipt.map(|d| d.to_string()),
        "terminal":r.state.terminal(),"dispatch_permitted":false,"retry_commit_permitted":false,
    })
}
pub fn inventory(v: &DigSessionView) -> Value {
    json!({
        "journal_id":v.journal_id.to_string(),"head":v.head.to_string(),"events":v.events,
        "byte_length":v.byte_len,"total_records":v.total_records,"unsettled_records":usize::from(v.pending.is_some()),
    })
}
fn blocker(value: &DigBlocker) -> &'static str {
    match value {
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
pub fn record(r: &DigRecord) -> Value {
    let p = r.plan();
    let c = p.before();
    let [x, y, z, width, height] = c.region().coordinates();
    let area = c.region().write_area();
    let native = r.effect().map(|e| json!({
        "phase":e.phase().as_str(),"reason":match e.reason() { DigReason::None => "none",
            DigReason::Stale => "stale", DigReason::CancelledBeforeDispatch => "cancelled_before_dispatch" },
        "designated_count":e.designated_count(),"after_witness":e.after_witness().map(|d| d.to_string()),
        "receipt_digest":e.receipt().map(|d| d.to_string()),
    }));
    json!({"idempotency_key":p.key(),"plan_digest":p.digest().to_string(),"state":r.state().as_str(),
        "terminal":r.state().terminal(),"permanent_unknown":r.permanent_unknown(),
        "native_query_can_help":r.needs_reconciliation(),"native":native,
        "review":{"region":{"x":x,"y":y,"z":z,"width":width,"height":height},
            "source_generation":c.generation(),"source_sequence":c.sequence(),"game_tick":c.tick(),
            "witness":c.witness().to_string(),"paused_at_capture":c.paused(),
            "allow_hidden_neighbors":p.allow_hidden_neighbors(),"target_count":c.region().target_count(),
            "halo_cells":c.region().halo_count(),"shared_block_write_area":{
                "min":[area.min.x,area.min.y,area.min.z],"max":[area.max.x,area.max.y,area.max.z]},
            "blockers_at_capture":c.blockers(p.allow_hidden_neighbors()).iter().map(blocker).collect::<Vec<_>>(),
            "tile_query":{"mode":"tiles","idempotency_key":p.key(),"plan_digest":p.digest().to_string(),"offset":0,"limit":16}},
        "historical_evidence_only":true,"current_terrain_proven":false,"excavation_completion_proven":false,
        "dispatch_permitted":false,"retry_commit_permitted":false})
}
pub fn tiles(r: &DigRecord, offset: usize, limit: usize) -> Result<Value> {
    let p = r.plan();
    let c = p.before();
    let total = c.region().halo_count();
    if !(1..=MAX_TILE_PAGE).contains(&limit) || offset >= total {
        return Err(error(
            ErrorCode::InvalidRequest,
            "tile page is outside the retained complete halo",
        ));
    }
    let end = (offset + limit).min(total);
    let rows = c.tiles().skip(offset).take(limit).map(|(position, tile)| {
        match tile {
            DigTile::Missing => json!({"coordinate":position,"presence":"missing"}),
            DigTile::Hidden => json!({"coordinate":position,"presence":"hidden"}),
            DigTile::Visible(t) => json!({"coordinate":position,"presence":"visible","tiletype":t.tiletype(),
                "designation_other":t.designation_other(),"occupancy":t.occupancy(),"priority":t.priority(),
                "cooldown":t.cooldown(),"block_other":t.block_other(),"temperatures":t.temperatures(),
                "dig":t.dig(),"hazards":t.hazards(),"flags":t.flags()}),
        }
    }).collect::<Vec<_>>();
    Ok(
        json!({"idempotency_key":p.key(),"plan_digest":p.digest().to_string(),"witness":c.witness().to_string(),
        "offset":offset,"total_tiles":total,"tiles":rows,"historical_evidence_only":true,
        "current_terrain_proven":false,"next_query":(end<total).then(|| json!({"mode":"tiles",
            "idempotency_key":p.key(),"plan_digest":p.digest().to_string(),"offset":end,"limit":limit}))}),
    )
}
pub fn failure(e: &dfmcp_core::DfmcpError) -> Value {
    let message = match e.code {
        ErrorCode::CapabilityDenied => {
            "Query-only recovery cannot mutate the game; current runtime and operator authority are required."
        }
        ErrorCode::BudgetExceeded => {
            "Complete recovery work and output exceed the admitted allowance."
        }
        ErrorCode::EffectIndeterminate => {
            "The original mining outcome remains unresolved. Never retry the designation."
        }
        ErrorCode::CorruptLedger => {
            "Journal custody or integrity failed. Preserve the original evidence; do not repair or reset it."
        }
        ErrorCode::StaleAnchor | ErrorCode::Conflict => {
            "The requested identity or continuation does not match the retained state."
        }
        _ => "Mining recovery failed its authority, custody, source, budget or evidence checks.",
    };
    json!({"ok":false,"error":{"code":e.code.as_str(),"message":message},
        "retry_commit_permitted":false,"effect_outcome_inferred":false,"game_mutation_dispatched_this_call":false,
        "recovery_class":if e.code==ErrorCode::EffectIndeterminate{"reconciliation_required"}else{"operator_action_required"},
        "recovery":"Preserve the original journal. Inspect it or release for recovery; never retry designation."})
}

pub fn packet(
    op: &str,
    result: Value,
    context: Option<&OperationContext>,
    mode: Option<DigMode>,
    binding: Option<&DigBinding>,
    view: Option<&DigSessionView>,
) -> String {
    let mut active = empty_active_work();
    active["scope"] = json!("this_dig_journal_only");
    active["inventory_verified"] = json!(view.is_some());
    active["pending_absence_proven"] = json!(view.is_some_and(|v| v.pending.is_none()));
    if let Some(v) = view {
        active["counts"] = inventory(v);
        if let Some(pending) = &v.pending {
            active["indeterminate_effects"] = json!([summary(pending)]);
            let query_can_help =
                pending.native_phase != Some(dfmcp_adapter::dig_designation::DigPhase::Unknown);
            active["required_recovery"] = json!({"tool":if query_can_help&&mode==Some(DigMode::Recover){"fortress.wait"}else{"fortress.explain"},
                "arguments":{"session_id":context.map(|c|c.session_id.to_string()),"idempotency_key":pending.key,
                    "plan_digest":pending.plan_digest.to_string()},"native_query_can_help":query_can_help});
        }
    }
    let phase = match op {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.wait" | "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut builder = AgentTurnBuilder::new(op, phase)
        .continuity(ContinuityStatus::Indeterminate, None, Some(json!({"world_history":"unestablished"})), None)
        .briefing(json!({"runtime":"unadmitted_mining_recovery","bridge_protocol":"1.16",
            "mode":mode.map(mode_name),"runtime_admitted":false,"mutation_admissible":false,
            "current_terrain_proven":false,"excavation_completion_proven":false}))
        .active_work(active)
        .coverage(json!({"status":"partial","complete_domains":if view.is_some(){json!(["this_journal_coordination_inventory"])}else{json!([])},
            "partial_domains":["selected_historical_mining_evidence"],"omitted_domains":["current_world","other_controllers","excavation_completion"]}))
        .uncertainty(vec![uncertainty("historical-mining-only","unknown",
            "Stored designation evidence is not current terrain, mining safety or excavation completion.",
            "Recover the original key; never create another key to bypass uncertainty.",None,Value::Null)]);
    if let Some(c) = context {
        let mut budget = empty_budget();
        budget["admitted"] = json!({"max_wall_millis":c.budget.max_wall_millis,"max_bytes":c.budget.max_bytes,
            "max_output_tokens":c.budget.max_output_tokens,"max_game_ticks":0,"max_actions":1});
        budget["reserved_output_bytes"] = json!(OUTPUT_BYTES);
        budget["accounting"] = json!("conservative_byte_reservations_not_measured_tokens");
        builder = builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string())
            .budget(budget)
            .recommendations(vec![recommendation("discover-mining","fortress.query","Inspect retained mining work before deciding how to recover.",
                "high","high","read_only","not_applicable",false,
                json!({"session_id":c.session_id.to_string(),"query":"{\"mode\":\"records\",\"limit\":8}"}))]);
    }
    if let (Some(b), Some(v)) = (binding, view) {
        builder = builder.anchor(json!({"fortress_id":b.fortress_id().to_string(),
            "epoch":b.manifest().generation,"sequence":v.events,"state_hash":v.head.to_string(),
            "scope":"coordination_root_not_world_state","game_tick":Value::Null}));
    }
    let mut turn = builder.build();
    if let Some(briefing) = turn["briefing"].as_object_mut() {
        briefing.remove("admission");
        if let (Some(b), Some(_)) = (binding, view) {
            briefing.insert(
                "expected_source".into(),
                json!({"endpoint":b.endpoint().to_string(),
                "df_version":b.manifest().df_version,"dfhack_version":b.manifest().dfhack_version,
                "generation":b.manifest().generation,"world_folder":b.folder(),"site_id":b.site()}),
            );
        }
    }
    json!({"result":result,"agent_turn":turn}).to_string()
}
