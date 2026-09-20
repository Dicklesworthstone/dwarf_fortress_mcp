#![forbid(unsafe_code)]
//! Explicitly unadmitted, foreground-only work-order progress. No mutation edge.
use std::net::SocketAddr;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use dfmcp_adapter::work_order_progress::{self as progress, ProgressComparison, ProgressObservation, ProgressSession};
use dfmcp_adapter::work_order_progress::rpc::{ProgressRpcClient, ProgressTcpStream};
use dfmcp_core::{AgentPhase, Capability, CapabilityGrant, CapabilityScope, ContinuityStatus,
    DfmcpError, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor, OperationContext,
    RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

const MAX_BYTES: u64 = 2 * 1024 * 1024;
const BASE_RESERVE: u64 = 16 * 1024;
const ROW_RESERVE: u64 = 4 * 1024;
const FAMILY: u128 = 12u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<RuntimeSession>> = Mutex::new(None);
type Reader = ProgressSession<ProgressRpcClient<ProgressTcpStream>>;
struct RuntimeSession {
    id: SessionId, request: u128, anchor: StateAnchor, budget: WorkBudget,
    grants: Vec<CapabilityGrant>, ids: Vec<u32>, reader: Reader,
}
fn error(code: ErrorCode, text: &str) -> DfmcpError { DfmcpError::new(code, text) }
fn runtime_io() -> Result<()> {
    let cx = fastmcp_rust::asupersync::Cx::current().ok_or_else(|| error(ErrorCode::CapabilityDenied, "progress requires its owned runtime context"))?;
    cx.checkpoint().map_err(|_| error(ErrorCode::CancellationRequested, "progress request cancelled"))?;
    if cx.io().is_none() { return Err(error(ErrorCode::CapabilityDenied, "inherited runtime denies progress I/O")); }
    Ok(())
}
const ENVIRONMENT: [&str; 4] = ["DFMCP_ALLOW_UNADMITTED_WORK_ORDER_PROGRESS_V1_12",
    "DFMCP_WORK_ORDER_PROGRESS_TOKEN", "DFMCP_WORK_ORDER_PROGRESS_ENDPOINT", "DFMCP_WORK_ORDER_PROGRESS_FORTRESS_ID"];
fn environment_contract(opt_in: Option<&str>, keys: &[String], admitted: bool) -> Result<()> {
    if opt_in != Some("1") || admitted || keys.iter().any(|k| k.starts_with("DFMCP_") && !ENVIRONMENT.contains(&k.as_str())) {
        return Err(error(ErrorCode::CapabilityDenied, "progress/1.12 requires exact development opt-in and refuses production/admission or other DFMCP environment state"));
    }
    Ok(())
}
fn validate_environment() -> Result<()> {
    let keys = std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()).collect::<Vec<_>>();
    environment_contract(std::env::var(ENVIRONMENT[0]).ok().as_deref(), &keys, crate::admission::current_admission_provenance().is_some())
}
fn configured(name: &str, maximum: usize) -> Result<String> {
    let value = std::env::var(name).map_err(|_| error(ErrorCode::CapabilityDenied, "required progress operator configuration is absent or not UTF-8"))?;
    if value.is_empty() || value.len() > maximum || value.contains('\0') { return Err(error(ErrorCode::InvalidRequest, "invalid progress operator configuration bound")); }
    Ok(value)
}
fn slot() -> Result<MutexGuard<'static, Option<RuntimeSession>>> {
    match SESSION.try_lock() {
        Ok(g) => Ok(g), Err(TryLockError::WouldBlock) => Err(error(ErrorCode::BudgetExceeded, "progress session is busy; no additional capture started")),
        Err(TryLockError::Poisoned(_)) => Err(error(ErrorCode::InternalInvariantViolation, "progress session lock poisoned; restart this read-only process")),
    }
}
fn parse_session(raw: &str) -> Result<SessionId> {
    if raw.len() != 32 || !raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err(error(ErrorCode::InvalidRequest, "invalid progress session ID")); }
    let value = u128::from_str_radix(raw, 16).map_err(|_| error(ErrorCode::InvalidRequest, "invalid session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value || !id.is_process_scoped_live() || (value & ((1u128 << 62)-1)) >> 57 != 12 {
        return Err(error(ErrorCode::InvalidRequest, "not a progress/1.12 session ID"));
    }
    Ok(id)
}
fn parse_digest(raw: &str) -> Result<Digest32> {
    if raw.len() != 64 || !raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) { return Err(error(ErrorCode::InvalidRequest, "witness must be lowercase SHA-256 hex")); }
    let mut out = [0;32];
    for (i, byte) in out.iter_mut().enumerate() { *byte = u8::from_str_radix(&raw[i*2..i*2+2],16).map_err(|_| error(ErrorCode::InvalidRequest,"invalid witness"))?; }
    Ok(Digest32::from_bytes(out))
}
fn normalized_ids(mut ids: Vec<u32>) -> Result<Vec<u32>> {
    if ids.len() > progress::MAX_TARGETS { return Err(error(ErrorCode::InvalidRequest, "progress selects at most 32 IDs")); }
    ids.sort_unstable(); progress::validate_targets(&ids)?; Ok(ids)
}
fn reserve(mut context: OperationContext, rows: usize) -> Result<OperationContext> {
    context.budget.validate()?;
    if rows == 0 || rows > progress::MAX_TARGETS { return Err(error(ErrorCode::InvalidRequest, "invalid progress selection size")); }
    let reserve = BASE_RESERVE + ROW_RESERVE * rows as u64;
    if context.budget.max_bytes <= reserve || u64::from(context.budget.max_output_tokens) * 4 < reserve {
        return Err(error(ErrorCode::BudgetExceeded, "complete progress output does not fit; no capture was started"));
    }
    context.budget.max_bytes -= reserve; Ok(context)
}
fn remaining(mut context: OperationContext, start: Instant, allowance: u64) -> Result<OperationContext> {
    let elapsed = u64::try_from(start.elapsed().as_millis()).unwrap_or(u64::MAX);
    context.budget.max_wall_millis = allowance.checked_sub(elapsed).filter(|n| *n > 0)
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "progress request deadline expired"))?;
    Ok(context)
}
impl RuntimeSession {
    fn context(&mut self) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(|| error(ErrorCode::BudgetExceeded,"progress request IDs exhausted"))?;
        Ok(OperationContext { session_id: self.id, request_id: RequestId::new(self.request), anchor: self.anchor,
            budget: self.budget, grants: self.grants.clone(), cancellation_requested:
            fastmcp_rust::asupersync::Cx::current().is_some_and(|cx| cx.checkpoint().is_err()) })
    }
}
fn observation_json(o: &ProgressObservation) -> Value {
    let rows = o.rows().iter().map(|row| {
        let details = row.order.as_ref().map(|s| json!({"job_type":s.job_type,"job_type_key":s.type_key,"reaction":s.reaction,
            "recognized_recipe":s.recipe_name(),"reported_remaining":s.remaining,"reported_total":s.total,
            "validated":s.validated(),"active":s.active(),"raw_status_bits":s.status_bits,
            "frequency":s.frequency,"workshop_id":s.workshop_id,"max_workshops":s.max_workshops,
            "next_check_year":s.next_check_year,"next_check_tick":s.next_check_tick,
            "item_condition_count":s.item_conditions,"order_condition_count":s.order_conditions}));
        json!({"native_order_id":row.native_order_id,"present":row.order.is_some(),"phase":row.phase(),"details":details,
            "production_completion_proven":false,"historical_creation_identity_proven":false})
    }).collect::<Vec<_>>();
    json!({"witness":o.witness().to_string(),"bridge_generation":o.generation(),"capture_sequence":o.sequence(),
        "game_tick":o.tick(),"fortress_id":o.fortress_id().to_string(),"world_folder":o.world_folder(),"site_id":o.site(),
        "paused":o.paused(),"next_order_id":o.next_order_id(),"queue_count":o.queue_count(),"rows":rows,
        "selected_presence_complete":true,"continuous_history_proven":false})
}
fn comparison_json(c: &ProgressComparison) -> Value {
    json!({"status":c.status,"reset_reason":c.reset_reason,"baseline_witness":c.baseline.map(|d|d.to_string()),
        "elapsed_game_ticks":c.elapsed_game_ticks,"client_acknowledged_baseline":false,"continuous_history_proven":false,
        "changes":c.changes.iter().map(|v|json!({"native_order_id":v.native_order_id,"kind":v.kind,
            "before_phase":v.before_phase,"after_phase":v.after_phase,"remaining_counter_decrease":v.remaining_decrease,
            "remaining_counter_increase":v.remaining_increase,"goods_produced_proven":false})).collect::<Vec<_>>()})
}
fn failure(cause: &DfmcpError) -> Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,
        "game_mutation_dispatched":false,"recovery":"Inspect cached evidence or explicitly close/reopen after source failure; no automatic reconnect."}})
}
fn packet(operation: &str, result: Value, context: Option<&OperationContext>, capture: Option<&ProgressObservation>, comparison: Option<&ProgressComparison>) -> String {
    let phase = if operation == "fortress.open_session" { AgentPhase::Bootstrap } else { AgentPhase::Inspect };
    let continuity = if capture.is_none() { ContinuityStatus::Indeterminate }
        else if comparison.is_some_and(|c|c.status == "reset") { ContinuityStatus::Reset }
        else if comparison.is_some_and(|c|c.status == "bootstrap") { ContinuityStatus::Bootstrap } else { ContinuityStatus::Partial };
    let mut builder = crate::AgentTurnBuilder::new(operation, phase)
        .continuity(continuity, None, None, comparison.and_then(|c|c.reset_reason).map(str::to_owned))
        .briefing(json!({"runtime":"unadmitted_development","bridge_protocol":"1.12","runtime_admitted":false,"read_only":true,
            "current_freshness_proven":false,"production_completion_proven":false}))
        .active_work(json!({"pending_plans":[],"actions":[],"obligations":[],"cancellation_drains":[],
            "indeterminate_effects":[],"publications":[],"confirmations":[],
            "scope":"This read-only session only; other journals, effects and monitors were not examined."}))
        .coverage(json!({"status":"partial","complete_domains":if capture.is_some(){json!(["presence_for_selected_order_ids"])}else{json!([])},
            "partial_domains":["selected_order_progress_fields"],"omitted_domains":["continuous_history","full_order_configuration",
            "material_availability","causal_blockers","created_receipt_identity","goods_produced"],"continuation":null}))
        .uncertainty(vec![json!({"code":"counters_are_not_completion","detail":"Absent orders, zero remaining and counter decreases do not independently prove completed goods or resolve a creation receipt."})]);
    if let Some(c) = context { builder = builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string()); }
    if let Some(o) = capture {
        builder = builder.anchor(json!({"kind":"selected_native_order_progress","fortress_id":o.fortress_id().to_string(),
            "bridge_generation":o.generation(),"capture_sequence":o.sequence(),"game_tick":o.tick(),"witness":o.witness().to_string(),
            "canonical_world_anchor":false}));
    }
    let mut turn = builder.build();
    // The common builder may attach ambient admission provenance. This isolated
    // profile never inherits it, including in an environment-refusal packet.
    if let Some(briefing) = turn.get_mut("briefing").and_then(Value::as_object_mut) { briefing.remove("admission"); }
    json!({"result":result,"agent_turn":turn}).to_string()
}
fn unbound(operation: &str, cause: &DfmcpError) -> String { packet(operation, failure(cause), None, None, None) }
fn render(operation: &str, result: Value, s: &RuntimeSession, c: &OperationContext) -> String {
    let mut current = c.clone(); current.anchor = s.anchor;
    let o = s.reader.current(&current).ok().flatten(); let comparison = s.reader.comparison(&current).ok().flatten();
    let out = packet(operation, result, Some(c), o, comparison);
    if out.len() as u64 <= c.budget.max_bytes.min(u64::from(c.budget.max_output_tokens)*4) { out }
    else { packet(operation, failure(&error(ErrorCode::BudgetExceeded,"progress output exceeded its reservation; no game effect occurred")),Some(c),None,None) }
}
fn selected_result(s: &RuntimeSession, c: &OperationContext, fresh: bool) -> Result<Value> {
    let capture = s.reader.current(c)?.ok_or_else(|| error(ErrorCode::StaleAnchor,"no valid capture is retained; explicitly close/reopen a fenced source"))?;
    Ok(json!({"ok":true,"source_read_this_call":fresh,"observation":observation_json(capture),
        "comparison":s.reader.comparison(c)?.map(comparison_json),
        "next_step":{"tool":"fortress.wait","session_id":s.id.to_string(),"expected_witness":capture.witness().to_string()}}))
}
fn with_session<F>(id: String, operation: &str, rows: Option<usize>, max_wall: Option<u64>, body: F) -> String
where F: FnOnce(&mut RuntimeSession, &OperationContext) -> Result<Value> {
    let started = Instant::now();
    let result = (|| {
        runtime_io()?; validate_environment()?; let id = parse_session(&id)?; let mut guard = slot()?;
        let s = guard.as_mut().filter(|s|s.id==id).ok_or_else(||error(ErrorCode::SessionNotFound,"progress session absent or closed"))?;
        let mut display = s.context()?;
        if let Some(wall) = max_wall { display.budget.max_wall_millis = wall.min(display.budget.max_wall_millis); }
        let work = reserve(display.clone(), rows.unwrap_or(s.ids.len())).and_then(|c|remaining(c,started,display.budget.max_wall_millis));
        let outcome = work.and_then(|work| body(s,&work)).and_then(|value| {
            if started.elapsed() >= Duration::from_millis(display.budget.max_wall_millis) {
                Err(error(ErrorCode::BudgetExceeded, "progress request exceeded its acknowledgement deadline; inspect retained evidence"))
            } else { Ok(value) }
        });
        let result = outcome.unwrap_or_else(|cause|failure(&cause));
        Ok::<_,DfmcpError>(render(operation,result,s,&display))
    })();
    result.unwrap_or_else(|cause|unbound(operation,&cause))
}

#[tool(description="Open an explicitly unadmitted read-only work-order-progress/1.12 session for 1..32 native order IDs. Operator supplies fortress, numeric loopback endpoint and separate token. A complete queue scan establishes selected presence; counters never prove goods produced. One capture is acquired before publishing the session.")]
pub fn fortress_open_session(native_order_ids: Vec<u32>, max_wall_millis: Option<u64>, max_bytes: Option<u64>, max_output_tokens: Option<u32>) -> String {
    let started = Instant::now();
    let result = (|| {
        runtime_io()?; validate_environment()?; let ids = normalized_ids(native_order_ids)?; let mut guard = slot()?;
        if guard.is_some() { return Err(error(ErrorCode::Conflict,"close the existing progress session first")); }
        let raw_fortress = configured(ENVIRONMENT[3],20)?;
        let value = raw_fortress.parse::<u64>().map_err(|_|error(ErrorCode::InvalidRequest,"fortress ID must be canonical nonzero decimal"))?;
        if value==0 || value.to_string()!=raw_fortress { return Err(error(ErrorCode::InvalidRequest,"fortress ID must be canonical nonzero decimal")); }
        let fortress = FortressId::new(value);
        let budget = WorkBudget { max_wall_millis:max_wall_millis.unwrap_or(5000),max_game_ticks:0,max_entities:4096,
            max_bytes:max_bytes.unwrap_or(MAX_BYTES),max_output_tokens:max_output_tokens.unwrap_or(65_536),max_actions:1 };
        budget.validate()?;
        if budget.max_wall_millis>60_000 || budget.max_bytes>MAX_BYTES || budget.max_output_tokens>131_072 { return Err(error(ErrorCode::BudgetExceeded,"progress session limits exceed 60000ms, 2MiB or 131072 output proxy units")); }
        let seq = NEXT.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<(1u64<<57)).then_some(n+1))
            .map_err(|_|error(ErrorCode::BudgetExceeded,"progress session identities exhausted"))?;
        let id = SessionId::new((1u128<<127)|FAMILY|u128::from(seq));
        let grants = [Capability::Query,Capability::Observe].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope{fortress_id:Some(fortress),..CapabilityScope::default()},max_risk:RiskTier::ReadOnly,
            expires_at_tick:None,remaining_uses:None }).collect::<Vec<_>>();
        let anchor = StateAnchor { fortress_id:fortress,cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO };
        let context = OperationContext {session_id:id,request_id:RequestId::new(1),anchor,budget,grants:grants.clone(),cancellation_requested:false};
        let mut work = reserve(context.clone(),ids.len())?;
        work.budget.max_bytes = work.budget.max_bytes.checked_sub(progress::BOOTSTRAP_BYTE_RESERVE)
            .filter(|left|*left>=progress::RPC_BYTE_RESERVE).ok_or_else(||error(ErrorCode::BudgetExceeded,"progress bootstrap and complete read do not fit"))?;
        let endpoint = match std::env::var(ENVIRONMENT[2]) {
            Ok(v) if v.len()<=128 => v,
            Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
            _ => return Err(error(ErrorCode::InvalidRequest,"progress endpoint must be bounded UTF-8 numeric loopback")),
        }.parse::<SocketAddr>().map_err(|_|error(ErrorCode::InvalidRequest,"progress endpoint must be numeric loopback"))?;
        let token = configured(ENVIRONMENT[1],256)?.into_bytes();
        work = remaining(work,started,budget.max_wall_millis)?;
        let source = ProgressRpcClient::connect(endpoint,token,id.get().to_be_bytes().to_vec(),Duration::from_millis(work.budget.max_wall_millis))?;
        work = remaining(work,started,budget.max_wall_millis)?; runtime_io()?;
        let mut reader = ProgressSession::new(source,&work)?; reader.refresh(&ids,&work)?;
        let o = reader.current(&work)?.ok_or_else(||error(ErrorCode::InternalInvariantViolation,"bootstrap capture missing"))?;
        let anchor = StateAnchor{tick:GameTick(o.tick()),state_hash:o.witness(),..anchor};
        let session = RuntimeSession{id,request:1,anchor,budget,grants,ids,reader};
        let result = json!({"ok":true,"session_id":id.to_string(),"capabilities":["query","observe"],"read_only":true,
            "capture":selected_result(&session,&context,true)?,"close":{"tool":"fortress.cancel","session_id":id.to_string()}});
        let out = packet("fortress.open_session",result,Some(&context),
            session.reader.current(&context)?,session.reader.comparison(&context)?);
        if out.len() as u64>budget.max_bytes.min(u64::from(budget.max_output_tokens)*4) || started.elapsed()>=Duration::from_millis(budget.max_wall_millis) {
            return Err(error(ErrorCode::BudgetExceeded,"progress bootstrap response did not fit; session was not published"));
        }
        *guard=Some(session); Ok::<_,DfmcpError>(out)
    })(); result.unwrap_or_else(|cause|unbound("fortress.open_session",&cause))
}

#[tool(description="Acquire one complete selected order-progress capture; optional IDs replace the selection. Comparisons reset on changed selection. Failed native refresh clears the old capture. This never changes a manager order or game clock.")]
pub fn fortress_observe(session_id: String, native_order_ids: Option<Vec<u32>>) -> String {
    let ids = match native_order_ids.map(normalized_ids).transpose() { Ok(v)=>v,Err(c)=>return unbound("fortress.observe",&c) };
    with_session(session_id,"fortress.observe",ids.as_ref().map(Vec::len),None,|s,c| {
        let ids=ids.unwrap_or_else(||s.ids.clone()); s.reader.refresh(&ids,c)?;
        let o=s.reader.current(c)?.ok_or_else(||error(ErrorCode::InternalInvariantViolation,"capture missing"))?;
        s.anchor.tick=GameTick(o.tick());s.anchor.state_hash=o.witness();s.ids=ids;selected_result(s,c,true)
    })
}
#[tool(description="Inspect the retained complete progress capture without a native call. The response explicitly does not prove current freshness. Optional expected witness rejects a stale client baseline.")]
pub fn fortress_query(session_id: String, expected_witness: Option<String>) -> String {
    with_session(session_id,"fortress.query",None,None,|s,c| {
        if let Some(w)=expected_witness { check_witness(s,c,&w)?; } selected_result(s,c,false)
    })
}
fn check_witness(s:&RuntimeSession,c:&OperationContext,raw:&str)->Result<()> {
    let expected=parse_digest(raw)?;
    if s.reader.current(c)?.is_none_or(|o|o.witness()!=expected) { return Err(error(ErrorCode::StaleAnchor,"progress baseline changed; query the current capture first")); }
    Ok(())
}
#[tool(description="Perform one bounded foreground progress refresh from an exact retained witness. No sleep, polling loop, background task or game-clock advance. Counter decreases and disappearance are not proof of completed goods; reset evidence never becomes a cross-incarnation comparison.")]
pub fn fortress_wait(session_id: String, expected_witness: String, max_wall_millis: Option<u64>) -> String {
    with_session(session_id,"fortress.wait",None,max_wall_millis,|s,c| {
        check_witness(s,c,&expected_witness)?;s.reader.refresh(&s.ids,c)?;
        let o=s.reader.current(c)?.ok_or_else(||error(ErrorCode::InternalInvariantViolation,"capture missing"))?;
        s.anchor.tick=GameTick(o.tick());s.anchor.state_hash=o.witness();selected_result(s,c,true)
    })
}
#[tool(description="Explain retained validation/activity and remaining-work counters without native calls. Inactive does not establish why work is blocked; absence and zero remaining do not certify goods produced.")]
pub fn fortress_explain(session_id: String)->String {
    with_session(session_id,"fortress.explain",None,None,|s,c|selected_result(s,c,false))
}
#[tool(description="Release this read-only progress session and its native connection. This also works after source failure or environment revocation; it never cancels or deletes manager orders.")]
pub fn fortress_cancel(session_id: String)->String {
    let result=(|| {
        let id=parse_session(&session_id)?;let mut guard=slot()?;
        if guard.as_ref().is_none_or(|s|s.id!=id) { return Err(error(ErrorCode::SessionNotFound,"progress session absent or already closed")); }
        drop(guard.take());
        Ok::<_,DfmcpError>(packet("fortress.cancel",json!({"ok":true,"closed":true,"orders_cancelled":false,"game_mutation_dispatched":false}),None,None,None))
    })();result.unwrap_or_else(|cause|unbound("fortress.cancel",&cause))
}
fn denied(session_id:String,operation:&str)->String {
    with_session(session_id,operation,None,None,|_,_|Err(error(ErrorCode::CapabilityDenied,"progress/1.12 is strictly read-only; use the separately authorized creation runtime for work-order intent")))
}
#[tool(description="Unavailable: progress/1.12 never prepares effects.")]
pub fn fortress_plan(session_id:String)->String {denied(session_id,"fortress.plan")}
#[tool(description="Unavailable: progress/1.12 has no mutation RPC.")]
pub fn fortress_commit(session_id:String)->String {denied(session_id,"fortress.commit")}
#[tool(description="Unavailable: progress observations are not game checkpoints.")]
pub fn fortress_checkpoint(session_id:String)->String {denied(session_id,"fortress.checkpoint")}
#[tool(description="Unavailable: progress/1.12 cannot restore game state.")]
pub fn fortress_restore(session_id:String)->String {denied(session_id,"fortress.restore")}
#[tool(description="Inspect local progress-session state without probing the native bridge. This is not compatibility admission, connection-health proof or a game-state refresh.")]
pub fn fortress_doctor(session_id:String)->String {
    with_session(session_id,"fortress.doctor",None,None,|s,c|Ok(json!({"ok":true,"runtime_admitted":false,"native_calls":0,
        "selected_order_ids":s.ids,"capture_available":s.reader.current(c)?.is_some(),"read_only":true})))
}
pub fn run_stdio() {
    if let Err(cause)=validate_environment() {eprintln!("{cause}");std::process::exit(1);}
    let server=ServerBuilder::new("dfmcp-live-work-order-progress-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted read-only progress/1.12. Select native manager-order IDs, inspect approval/activity/counters, and wait for one new bounded capture using its exact witness. No background polling or clock control. Disappearance, zero remaining or decreasing counters do not prove completed goods or resolve historical creation effects. Comparisons are between sampled endpoints, not continuous history. Unknown configuration is explicit. After source failure close and reopen explicitly. Creation and its durable journal remain a separate authorized profile.")
        .build();crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path="live_work_order_progress_server_tests.rs"]
mod tests;
