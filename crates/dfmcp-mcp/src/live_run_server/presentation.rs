//! Authority-free projection of source-bound run coordination, not world state.
use std::collections::BTreeMap;
use dfmcp_adapter::bounded_run::{RunObservation, journal::{DurableRun,RunMode}};
use dfmcp_core::{DfmcpError,Digest32,ErrorCode,OperationContext,Result,SessionId};
use serde_json::{Value,json};
use crate::agent_turn::{AgentTurnBuilder,AgentPhase,ContinuityStatus,empty_active_work,uncertainty,recommendation};

pub(super) const BASE_RESERVE:u64=8192;
pub(super) const ROW_RESERVE:u64=4096;
pub(super) const MAX_PAGE:usize=8;
pub(super) fn error(code:ErrorCode,message:&str)->DfmcpError {DfmcpError::new(code,message)}
pub(super) fn digest(raw:&str)->Result<Digest32> {
    if raw.len()!=64 || !raw.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {
        return Err(error(ErrorCode::InvalidRequest,"digest must be 64 lowercase hexadecimal characters"));
    }
    let mut out=[0;32];
    for (index,byte) in out.iter_mut().enumerate() {
        *byte=u8::from_str_radix(&raw[index*2..index*2+2],16)
            .map_err(|_|error(ErrorCode::InvalidRequest,"invalid digest"))?;
    }
    Ok(Digest32::from_bytes(out))
}
pub(super) fn observation(value:&RunObservation)->Value {
    json!({"generation":value.generation(),"dispatch_sequence":value.sequence(),"tick":value.tick(),
        "loaded":value.loaded(),"paused":value.paused(),"witness":value.witness().to_string(),
        "eligible_at_capture":value.eligible(),"named_fortress_identity_established":false})
}
pub(super) fn reference(value:&DurableRun)->Value {
    json!({"idempotency_key":value.plan().key(),"plan_digest":value.plan().digest().to_string(),
        "state":value.state().as_str(),"reconciliation_required":value.unresolved()})
}
pub(super) fn record(value:&DurableRun)->Value {
    let native=value.native().map(|n|json!({"phase":n.phase().as_str(),"reason":n.reason().as_str(),
        "unpause_attempted":n.unpause_attempted(),"historical_pause_verified":n.pause_verified(),
        "observed_tick":n.observed_tick(),"observed_ticks_advanced":n.observed_ticks_advanced(),
        "observed_tick_overshoot":n.observed_tick_overshoot(),"receipt_digest":n.receipt().to_string()}));
    json!({"idempotency_key":value.plan().key(),"plan_digest":value.plan().digest().to_string(),
        "state":value.state().as_str(),"terminal_coordination":value.terminal(),
        "reconciliation_required":value.unresolved(),"game_ticks":value.plan().spec().game_ticks(),
        "run_wall_millis":value.plan().spec().wall_ms(),"before":observation(value.plan().before()),"native":native,
        "current_pause_unproved":true,"goal_completion_proven":false,"retry_unpause":false})
}
pub(super) fn failure(cause:&DfmcpError,operation:&str)->Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,
        "unpause_dispatch":if operation=="fortress.commit" {"not_inferred_from_error"}else{"none_by_this_tool"},
        "recovery":"inspect durable records; query or cancel, never replay an uncertain unpause"}})
}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub(super) enum Filter {All,Pending,Unresolved,Terminal}
impl Filter {
    pub(super) fn parse(raw:&str)->Result<Self> {
        match raw {"all"=>Ok(Self::All),"pending"=>Ok(Self::Pending),"unresolved"=>Ok(Self::Unresolved),
            "terminal"=>Ok(Self::Terminal),_=>Err(error(ErrorCode::InvalidRequest,"state must be all, pending, unresolved or terminal"))}
    }
    pub(super) fn matches(self,r:&DurableRun)->bool {
        match self {Self::All=>true,Self::Pending=>!r.terminal(),Self::Unresolved=>r.unresolved(),Self::Terminal=>r.terminal()}
    }
    pub(super) fn name(self)->&'static str {match self {Self::All=>"all",Self::Pending=>"pending",Self::Unresolved=>"unresolved",Self::Terminal=>"terminal"}}
}
#[derive(Clone,Debug,PartialEq,Eq)]
pub(super) struct PageKey {pub session:SessionId,pub journal:Digest32,pub head:Digest32,pub filter:Filter,pub limit:usize}
#[derive(Default)]
pub(super) struct Cursors {serial:u64,entries:BTreeMap<String,(PageKey,usize)>}
impl Cursors {
    pub(super) fn resolve(&self,token:&str,key:&PageKey)->Result<usize> {
        if token.len()!=64 {return Err(error(ErrorCode::InvalidRequest,"invalid run continuation"));}
        self.entries.get(token).filter(|(issued,_)|issued==key).map(|(_,offset)|*offset)
            .ok_or_else(||error(ErrorCode::StaleAnchor,"run continuation expired or session/head/filter/limit changed"))
    }
    pub(super) fn issue(&mut self,key:PageKey,offset:usize)->Result<String> {
        if let Some((token,_))=self.entries.iter().find(|(_,entry)|entry.0==key && entry.1==offset) {return Ok(token.clone());}
        self.serial=self.serial.checked_add(1).ok_or_else(||error(ErrorCode::BudgetExceeded,"run continuation IDs exhausted"))?;
        let mut bytes=b"dfmcp-run-page/1\0".to_vec();bytes.extend_from_slice(&key.session.get().to_be_bytes());
        bytes.extend_from_slice(key.journal.as_bytes());bytes.extend_from_slice(key.head.as_bytes());
        bytes.extend_from_slice(&self.serial.to_be_bytes());let token=Digest32::of_bytes(&bytes).to_string();
        if self.entries.len()>=64 {if let Some(old)=self.entries.keys().next().cloned() {self.entries.remove(&old);}}
        self.entries.insert(token.clone(),(key,offset));Ok(token)
    }
}
pub(super) struct View<'a> {
    pub context:Option<&'a OperationContext>,pub mode:Option<RunMode>,pub journal:Option<Value>,
    pub records:&'a [DurableRun],pub selected:Option<&'a RunObservation>,pub custody_verified:bool,
}
pub(super) fn packet(operation:&str,result:Value,view:View<'_>)->String {
    let phase=match operation {"fortress.open_session"=>AgentPhase::Bootstrap,"fortress.plan"=>AgentPhase::Propose,
        "fortress.commit"=>AgentPhase::Commit,"fortress.wait"|"fortress.cancel"=>AgentPhase::Reconcile,_=>AgentPhase::Inspect};
    let pending=view.records.iter().filter(|r|!r.terminal()).count();
    let unresolved=view.records.iter().filter(|r|r.unresolved()).count();
    let mut active=empty_active_work();
    active["actions"]=json!(view.records.iter().filter(|r|!r.terminal()||r.unresolved()).take(4).map(reference).collect::<Vec<_>>());
    active["pending_count"]=json!(pending);active["unresolved_count"]=json!(unresolved);
    let active_count=view.records.iter().filter(|r|!r.terminal()||r.unresolved()).count();
    active["omitted_count"]=json!(active_count.saturating_sub(4));
    active["scope"]=json!("this run journal only; historical coordination, not all game work");
    active["custody_verified"]=json!(view.custody_verified);
    active["count_evidence"]=json!(if view.custody_verified {"verified_retained_history"}else{"unverified_or_unbound; empty does not prove absence"});
    let briefing=json!({"runtime":"unadmitted_development","bridge_protocol":"1.13","runtime_admitted":false,
        "mode":view.mode.map(RunMode::as_str),"named_fortress_identity_established":false,
        "current_pause_unproved":true,"goal_completion_proven":false,
        "anchor_tick_semantics":"zero_sentinel_not_game_time",
        "journal":view.journal,"retained_selection":view.selected.map(observation)});
    let mut builder=AgentTurnBuilder::new(operation,phase).briefing(briefing.clone()).active_work(active)
        .continuity(ContinuityStatus::Indeterminate,None,None,Some("coordination is not continuous game history".into()))
        .coverage(json!({"status":"partial","complete_domains":if view.custody_verified {json!(["retained_run_coordination"])}else{json!([])},
            "partial_domains":[],"omitted_domains":["canonical_world","goal_completion","named_fortress_identity","current_pause"],"continuation":null}))
        .uncertainty(vec![uncertainty("run-clock-evidence","unknown",
            "Native stop limits are callback triggers, not exact-tick or hard real-time guarantees.",
            "Other controllers and native stalls are not excluded; historical pause does not prove current pause.",None,json!({}))]);
    if let Some(c)=view.context {
        let mut next=recommendation("run-journal-discovery","fortress.query",
            "Rediscover durable pending work before starting another run.","control_safety","high","read_only",
            "not_applicable",false,json!({"session_id":c.session_id.to_string(),"state":"unresolved","limit":2}));
        next["estimated_cost"]["bridge_bytes"]=json!(0);
        next["confidence"]["evidence"]=json!([{"journal_head":briefing["journal"]["head"]}]);
        next["prerequisites"]=json!(["current Query authority and verified journal custody"]);
        next["invalidating_conditions"]=json!(["session close or journal custody failure"]);
        builder=builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string())
            .anchor(json!({"fortress_id":c.anchor.fortress_id.to_string(),
                "cursor":{"epoch":c.anchor.cursor.epoch,"sequence":c.anchor.cursor.sequence},
                "tick":c.anchor.tick.get(),"state_hash":c.anchor.state_hash.to_string(),
                "domain":"source_bound_run_coordination_not_canonical_world"}))
            .budget(json!({"admitted":{"max_bytes":c.budget.max_bytes,"max_output_tokens":c.budget.max_output_tokens,
                "max_wall_millis":c.budget.max_wall_millis,"max_game_ticks":c.budget.max_game_ticks},
                "consumed":{},"remaining":null,"accounting":"conservative pre-reservation; token proxy is four UTF-8 bytes"}))
            .recommendations(vec![next]);
    }
    let mut turn=builder.build();
    // The shared builder can attach production provenance from an ambient
    // admitted process. This isolated profile must NEVER expose that provenance,
    // even in its admission-refusal error packet.
    turn["briefing"]=briefing;
    json!({"agent_turn":turn,"result":result}).to_string()
}
