//! Control reviews and historical evidence use the shared Agent Turn spine.
use dfmcp_adapter::dig_control_policy::{DigCheckpointPolicy, DigControlPolicy};
use dfmcp_adapter::dig_designation::{DigBlocker, DigObservation, DigReason, DigTile};
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigRecord, DigSummary};
use dfmcp_adapter::dig_designation::journal::session::DigSessionView;
use dfmcp_core::{Digest32, ErrorCode, MapCuboid, OperationContext, Result};
use serde_json::{Value,json};
use crate::agent_turn::{AgentPhase,AgentTurnBuilder,ContinuityStatus,empty_active_work,empty_budget,recommendation,uncertainty};
use super::error;

pub const OUTPUT_BYTES:u64=32768;
pub fn digest(raw:&str)->Result<Digest32> {
    if raw.len()!=64 || !raw.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {
        return Err(error(ErrorCode::InvalidRequest,"expected canonical lowercase SHA-256"));
    }
    let mut bytes=[0;32];
    for (i,pair) in raw.as_bytes().chunks_exact(2).enumerate() {
        let text=std::str::from_utf8(pair).map_err(|_|error(ErrorCode::InvalidRequest,"invalid digest"))?;
        bytes[i]=u8::from_str_radix(text,16).map_err(|_|error(ErrorCode::InvalidRequest,"invalid digest"))?;
    }
    Ok(Digest32::from_bytes(bytes))
}
pub fn area(v:MapCuboid)->Value {json!({"min":[v.min.x,v.min.y,v.min.z],"max":[v.max.x,v.max.y,v.max.z]})}
fn blocker(v:&DigBlocker)->&'static str {match v {
    DigBlocker::Unpaused=>"unpaused",DigBlocker::UnobservedTarget=>"unobserved_target",
    DigBlocker::NotNaturalWall=>"not_natural_wall",DigBlocker::ExistingDesignation=>"existing_designation",
    DigBlocker::OccupiedOrJob=>"occupied_or_job",DigBlocker::KnownHazard=>"known_hazard",
    DigBlocker::MissingContext=>"missing_context",DigBlocker::HiddenContext=>"hidden_context",
    DigBlocker::SequenceExhausted=>"sequence_exhausted",
}}
pub fn observation(c:&DigObservation)->Value {json!({
    "region":c.region().coordinates(),"witness":c.witness().to_string(),"generation":c.generation(),
    "sequence":c.sequence(),"game_tick":c.tick(),"paused_at_capture":c.paused(),
    "target_count":c.region().target_count(),"halo_cells":c.region().halo_count(),
    "blockers_without_hidden_acknowledgement":c.blockers(false).iter().map(blocker).collect::<Vec<_>>(),
    "shared_block_write_area":area(c.region().write_area()),"halo":area(c.region().halo()),
    "current_terrain_proven":false,"excavation_safety_proven":false,
    "tile_query":{"mode":"selection_tiles","witness":c.witness().to_string(),"offset":0,"limit":16}
})}
pub fn tile_page(c:&DigObservation,offset:usize,limit:usize)->Result<Value> {
    if !(1..=16).contains(&limit)||offset>=c.region().halo_count() {
        return Err(error(ErrorCode::InvalidRequest,"tile page is outside the complete retained halo"));
    }
    let end=(offset+limit).min(c.region().halo_count());
    let rows=c.tiles().skip(offset).take(limit).map(|(position,tile)|match tile {
        DigTile::Missing=>json!({"coordinate":position,"presence":"missing"}),
        DigTile::Hidden=>json!({"coordinate":position,"presence":"hidden"}),
        DigTile::Visible(t)=>json!({"coordinate":position,"presence":"visible","tiletype":t.tiletype(),
            "designation_other":t.designation_other(),"occupancy":t.occupancy(),"priority":t.priority(),
            "cooldown":t.cooldown(),"block_other":t.block_other(),"temperatures":t.temperatures(),
            "dig":t.dig(),"hazards":t.hazards(),"flags":t.flags()}),
    }).collect::<Vec<_>>();
    Ok(json!({"witness":c.witness().to_string(),"offset":offset,"total_tiles":c.region().halo_count(),
        "tiles":rows,"next_offset":(end<c.region().halo_count()).then_some(end),"historical_evidence_only":true}))
}
pub fn summary(v:&DigSummary)->Value {json!({"idempotency_key":v.key,"plan_digest":v.plan_digest.to_string(),
    "state":v.state.as_str(),"native_phase":v.native_phase.map(|p|p.as_str()),"terminal":v.state.terminal(),
    "receipt_digest":v.receipt.map(|v|v.to_string()),"retry_commit_permitted":false})}
pub fn inventory(v:&DigSessionView)->Value {json!({"journal_id":v.journal_id.to_string(),"head":v.head.to_string(),
    "events":v.events,"byte_length":v.byte_len,"total_records":v.total_records,"unsettled_records":usize::from(v.pending.is_some())})}
pub fn record(v:&DigRecord)->Value {
    let p=v.plan();
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
pub fn policy(v:&DigControlPolicy)->Value {json!({"digest":v.digest().to_string(),"journal_id":v.journal_id().to_string(),"lease_id":v.lease_id().to_string(),
    "lease_scope":"this_host_manager_and_configured_journal_only","global_controller_fence":false,
    "checkpoint_policy":match v.checkpoint_policy(){DigCheckpointPolicy::Required=>"required",
        DigCheckpointPolicy::DisposableFortress=>"disposable-fortress-no-checkpoint"},
    "checkpoint_verified":false,"protected_areas":v.protected_areas().iter().copied().map(area).collect::<Vec<_>>()})}
pub fn failure(e:&dfmcp_core::DfmcpError)->Value {json!({"ok":false,
    "error":{"code":e.code.as_str(),"message":match e.code {
        ErrorCode::CheckpointRequired=>"A verified game checkpoint is required and unavailable in this profile.",
        ErrorCode::LeaseDenied=>"The original exclusive spatial lease is absent, expired or does not cover the write scope.",
        ErrorCode::EffectIndeterminate=>"Recover the original journal operation; never retry its designation.",
        ErrorCode::CorruptLedger=>"Journal custody or integrity failed; preserve the original evidence.",
        ErrorCode::BudgetExceeded=>"Complete mining work and response exceed the admitted allowance.",
        _=>"Mining failed current authority, review, source, custody, runtime or policy checks."}},
    "retry_commit_permitted":false,"effect_outcome_inferred":false,
    "recovery":"Inspect the original key and journal; do not reprepare or create another key to bypass uncertainty."})}
pub fn packet(op:&str,result:Value,c:Option<&OperationContext>,binding:Option<&DigBinding>,
    view:Option<&DigSessionView>,control_policy:Option<&DigControlPolicy>,review:Option<Digest32>)->String {
    let mut active=empty_active_work();active["scope"]=json!("this_dig_journal_only");
    active["inventory_verified"]=json!(view.is_some());active["pending_absence_proven"]=json!(view.is_some_and(|v|v.pending.is_none()));
    if let Some(v)=view {
        active["counts"]=inventory(v);
        if let Some(p)=&v.pending {
            active["pending_plans"]=json!([summary(p)]);
            active["confirmation"]=json!({"review_seal":review.map(|v|v.to_string()),"permission_is_process_local":true,
                "dispatch_authorized_by_this_packet":false});
            if review.is_none(){active["indeterminate_effects"]=json!([summary(p)]);}
            active["recovery"]=json!({"tool":"fortress.explain","arguments":{"session_id":c.map(|c|c.session_id.to_string()),
                "idempotency_key":p.key,"plan_digest":p.plan_digest.to_string()}});
        }
    }
    let phase=match op {"fortress.open_session"=>AgentPhase::Bootstrap,"fortress.observe"=>AgentPhase::Orient,
        "fortress.plan"=>AgentPhase::Propose,"fortress.commit"=>AgentPhase::Commit,
        "fortress.wait"|"fortress.cancel"=>AgentPhase::Reconcile,_=>AgentPhase::Inspect};
    let mut builder=AgentTurnBuilder::new(op,phase)
        .continuity(ContinuityStatus::Indeterminate,None,Some(json!({"world_history":"unestablished"})),None)
        .briefing(json!({"runtime":"unadmitted_mining_control","bridge_protocol":"1.16","runtime_admitted":false,
            "mutation_admissible":false,"development_policy":control_policy.map(policy),
            "current_terrain_proven":false,"excavation_completion_proven":false}))
        .active_work(active)
        .coverage(json!({"status":"partial","complete_domains":if view.is_some(){json!(["this_journal_coordination_inventory"])}else{json!([])},
            "partial_domains":["bounded_mining_evidence"],"omitted_domains":["excavation_completion","structural_safety","other_controllers","game_checkpoint"]}))
        .uncertainty(vec![uncertainty("mining-not-excavation","unknown",
            "Designation proof is historical configuration, not excavation completion or structural safety.",
            "Preserve unresolved work and recover its exact key; never infer nonapplication from a lost reply.",None,Value::Null)]);
    if let Some(c)=c {
        let mut budget=empty_budget();budget["admitted"]=json!({"max_wall_millis":c.budget.max_wall_millis,
            "max_bytes":c.budget.max_bytes,"max_output_tokens":c.budget.max_output_tokens,"max_game_ticks":0,"max_actions":1});
        budget["reserved_output_bytes"]=json!(OUTPUT_BYTES);budget["accounting"]=json!("conservative_byte_reservations_not_measured_tokens");
        builder=builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string()).budget(budget)
            .recommendations(vec![recommendation("discover-mining","fortress.query","Inspect retained mining obligations before new work.",
                "high","high","read_only","not_applicable",false,json!({"session_id":c.session_id.to_string(),"query":"{\"mode\":\"records\",\"limit\":8}"}))]);
    }
    if let (Some(b),Some(v))=(binding,view) {
        builder=builder.anchor(json!({"fortress_id":b.fortress_id().to_string(),"epoch":b.manifest().generation,
            "sequence":v.events,"state_hash":v.head.to_string(),"game_tick":Value::Null,"scope":"coordination_root_not_world_state"}));
    }
    let mut turn=builder.build();if let Some(b)=turn["briefing"].as_object_mut(){b.remove("admission");}
    json!({"result":result,"agent_turn":turn}).to_string()
}
