#![forbid(unsafe_code)]
//! Explicitly unadmitted excavation-run/1.18 through the frozen eleven-tool waist.
//! Native ownership, storage and effect semantics remain in the adapter. No
//! polling, designation, arbitrary command, checkpoint or production admission.
use dfmcp_adapter::excavation_run::{ExcavationCapture, ExcavationCell, ExcavationRegion,
    ExcavationRunPlan, ExcavationRunRecord, ExcavationRunSpec, ExcavationTrigger,
    FortressIdentity, RunPhase, RunSpec};
use dfmcp_adapter::excavation_run::coordinator::ExcavationEntry;
use dfmcp_adapter::excavation_run::session::{ExcavationCommand, ExcavationInventory,
    ExcavationMode, ExcavationOutcome, ExcavationSession, ExcavationSessionBackend,
    ExcavationSessionGuard, ExcavationTurn, MAX_SESSION_BYTES, RESPONSE_BYTES};
use dfmcp_adapter::excavation_run::session::native::PrivateExcavationBackend;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode,
    GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId,
    StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, OnceLock, TryLockError,
    atomic::{AtomicBool, AtomicU64, Ordering}};
use std::time::Instant;

mod presentation;
mod requests;
mod runtime;
use presentation::Cursors;
use requests::{CancelRequest, CommitRequest, Filter, Identity, PlanRequest, QueryRequest};

fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError { dfmcp_core::DfmcpError::new(code,message) }
fn invalid() -> dfmcp_core::DfmcpError { error(ErrorCode::InvalidRequest,"invalid bounded excavation request") }
fn denied() -> dfmcp_core::DfmcpError { error(ErrorCode::CapabilityDenied,"excavation development authority denied") }
fn exhausted() -> dfmcp_core::DfmcpError { error(ErrorCode::BudgetExceeded,"complete excavation operation exceeds budget") }
fn mode_name(mode: ExcavationMode) -> &'static str {
    match mode { ExcavationMode::Offline=>"offline",ExcavationMode::Recover=>"recover",ExcavationMode::Control=>"control" }
}
fn mode(raw: Option<&str>) -> Result<ExcavationMode> {
    match raw { None|Some("offline")=>Ok(ExcavationMode::Offline),Some("recover")=>Ok(ExcavationMode::Recover),
        Some("control")=>Ok(ExcavationMode::Control),_=>Err(invalid()) }
}
fn unbound(op: &str, cause: &dfmcp_core::DfmcpError) -> String {
    let mut value=presentation::failure(cause);
    value["effect_may_have_occurred"]=json!(matches!(op,"fortress.commit"|"fortress.cancel"));
    presentation::packet(op,value,None,None,None)
}

/// Operator-only configuration. Creating this value does not grant a client
/// clock authority. Native reads/starts still require the isolated environment.
pub struct Configuration { backend: PrivateExcavationBackend, initialize: AtomicBool }
impl Configuration {
    pub fn new(directory: PathBuf, fortress: FortressIdentity, region: ExcavationRegion,
        initialize: bool) -> Result<Self>
    {
        Ok(Self { backend: PrivateExcavationBackend::new(directory,fortress,region)?,
            initialize: AtomicBool::new(initialize) })
    }
}
static CONFIG: OnceLock<Configuration> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);
const FAMILY: u128 = 31u128 << 57;
static SESSION: Mutex<Option<State<PrivateExcavationBackend>>> = Mutex::new(None);
struct State<B> {
    id: SessionId,
    request: u128,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    session: ExcavationSession<B>,
    cursors: Cursors,
}
fn lock() -> Result<MutexGuard<'static,Option<State<PrivateExcavationBackend>>>> {
    match SESSION.try_lock() {
        Ok(value)=>Ok(value),Err(TryLockError::WouldBlock)=>Err(exhausted()),
        Err(TryLockError::Poisoned(_))=>Err(error(ErrorCode::CorruptLedger,"excavation process state poisoned; recover original journal")),
    }
}
fn context(id: SessionId, request: u128, fortress: &FortressIdentity, tick: u64,
    budget: WorkBudget, grants: Vec<CapabilityGrant>) -> OperationContext
{
    OperationContext { session_id:id,request_id:RequestId::new(request),
        anchor:StateAnchor{fortress_id:fortress.fortress_id(),cursor:ObservationCursor::ORIGIN,tick:GameTick(tick),state_hash:Digest32::ZERO},
        budget,grants,cancellation_requested:false }
}
fn grants(mode: ExcavationMode, fortress: &FortressIdentity, expiry: Option<u64>) -> Vec<CapabilityGrant> {
    let capabilities=if mode==ExcavationMode::Control {
        vec![Capability::Query,Capability::Observe,Capability::Plan,Capability::ControlClock]
    }else{vec![Capability::Query]};
    capabilities.into_iter().map(|capability|CapabilityGrant{
        capability,scope:CapabilityScope{fortress_id:Some(fortress.fortress_id()),..CapabilityScope::default()},
        max_risk:if matches!(capability,Capability::Query|Capability::Observe){RiskTier::ReadOnly}else{RiskTier::Guarded},
        expires_at_tick:expiry.map(GameTick),remaining_uses:None,
    }).collect()
}
fn dispatch<B: ExcavationSessionBackend>(state: &mut State<B>, op: &str, wall: Option<u64>,
    action: Result<ExcavationCommand>, query: Result<QueryRequest>, started: Instant,
    guard: &dyn ExcavationSessionGuard) -> String
{
    let next=match state.request.checked_add(1){Some(next)=>next,None=>return unbound(op,&exhausted())};
    state.request=next;
    let mut budget=state.budget;
    if let Some(wall)=wall { budget.max_wall_millis=budget.max_wall_millis.min(wall); }
    let c=context(state.id,next,state.session.inventory().binding().fortress(),state.session.high_tick(),budget,state.grants.clone());
    let (command,query,request_error)=match (action,query.and_then(|q|{q.validate()?;Ok(q)})) {
        (Ok(command),Ok(query))=>(command,query,None),
        (Err(cause),_)|(_,Err(cause))=>(ExcavationCommand::Inventory,QueryRequest::default(),Some(cause)),
    };
    let mut turn=state.session.execute(command,&c,started,guard);
    if turn.outcome.is_ok() { if let Some(cause)=request_error {turn.outcome=Err(cause);} }
    let mut candidate=state.cursors.clone();
    let rendered=match presentation::render(op,&turn,&c,state.session.mode(),&query,&mut candidate){
        Ok(value)=>value,
        Err(cause)=>{
            let failure=presentation::failure(&cause);
            turn.outcome=Err(cause);
            let value=presentation::packet(op,failure,
                Some(&c),Some(state.session.mode()),Some(&turn));
            if value.len() as u64>RESPONSE_BYTES {return unbound(op,&exhausted());}
            if let Err(cause)=guard.checkpoint(){return unbound(op,&cause);}
            return value;
        }
    };
    if let Err(cause)=guard.checkpoint(){return unbound(op,&cause);}
    if started.elapsed().as_millis()>=u128::from(budget.max_wall_millis){
        turn.historical_prior=turn.inventory.take();
        return presentation::packet(op,presentation::failure(&exhausted()),Some(&c),Some(state.session.mode()),Some(&turn));
    }
    state.cursors=candidate;
    rendered
}
async fn with_session(raw: String, op: &'static str, wall: Option<u64>, action: Result<ExcavationCommand>,
    query: Result<QueryRequest>) -> String
{
    runtime::owned(op,move|control|{
        let result=(||{
            control.checkpoint()?;
            if raw.len()!=32 {return Err(invalid());}
            let mut locked=lock()?;
            let state=locked.as_mut().filter(|s|s.id.to_string()==raw)
                .ok_or_else(||error(ErrorCode::SessionNotFound,"excavation session absent"))?;
            let output=dispatch(state,op,wall,action,query,control.started,&control);
            if state.session.is_released(){*locked=None;}
            Ok(output)
        })();
        match result{Ok(value)=>value,Err(cause)=>unbound(op,&cause)}
    }).await
}

#[tool(name="fortress.open_session",description="Open fixed offline (default), recover (query-only), or explicitly enabled excavation clock control. Paths, fortress and region are operator configuration. No automatic repair, native preparation or commit. Optional ceilings only narrow this bounded profile.")]
pub async fn fortress_open_session(mode: Option<String>, max_wall_millis: Option<u64>, max_bytes: Option<u64>,
    max_output_tokens: Option<u32>, max_game_ticks: Option<u64>, expires_at_tick: Option<u64>) -> String
{
    runtime::owned("fortress.open_session",move|control|{
        let result=(||{
            control.checkpoint()?;
            let mode=self::mode(mode.as_deref())?;
            let config=CONFIG.get().ok_or_else(denied)?;
            let mut locked=lock()?;
            if locked.is_some(){return Err(error(ErrorCode::Conflict,"release the existing excavation session first"));}
            let budget=WorkBudget{max_wall_millis:max_wall_millis.unwrap_or(15000),max_bytes:max_bytes.unwrap_or(MAX_SESSION_BYTES),
                max_output_tokens:max_output_tokens.unwrap_or(8192),max_game_ticks:max_game_ticks.unwrap_or(1200),max_entities:256,max_actions:1};
            budget.validate()?;
            if budget.max_wall_millis>60000||budget.max_bytes>MAX_SESSION_BYTES||budget.max_output_tokens>65536
                ||budget.max_game_ticks>1200||u64::from(budget.max_output_tokens)*4<RESPONSE_BYTES{return Err(exhausted());}
            let serial=NEXT.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<(1u64<<57)).then_some(n+1)).map_err(|_|exhausted())?;
            let id=SessionId::new((1u128<<127)|FAMILY|u128::from(serial));
            let grants=grants(mode,config.backend.fortress(),expires_at_tick);
            let c=context(id,1,config.backend.fortress(),0,budget,grants.clone());
            let initialize=mode==ExcavationMode::Control&&config.initialize.swap(false,Ordering::AcqRel);
            let session=ExcavationSession::open(config.backend.clone(),mode,initialize,&c,control.started,&control)?;
            let turn=ExcavationTurn{outcome:Ok(session.selected().cloned().map_or(ExcavationOutcome::Inventory,ExcavationOutcome::Observation)),
                inventory:Some(session.inventory().clone()),historical_prior:None,plan:None,uncertain_attempt:None,
                native_operation_attempted:initialize,released:false};
            let mut cursors=Cursors::default();
            let out=presentation::render("fortress.open_session",&turn,&c,mode,&QueryRequest::default(),&mut cursors)?;
            control.checkpoint()?;
            if control.started.elapsed().as_millis()>=u128::from(budget.max_wall_millis){return Err(exhausted());}
            *locked=Some(State{id,request:1,budget,grants,session,cursors});
            Ok(out)
        })();
        match result{Ok(value)=>value,Err(cause)=>unbound("fortress.open_session",&cause)}
    }).await
}
#[tool(name="fortress.observe",description="Acquire one coherent native excavation capture for the configured 1..8 by 1..8 region, retaining its exact witness. Control sessions only. No designation or clock advance; a refresh discards the old local review.")]
pub async fn fortress_observe(session_id: String, max_wall_millis: Option<u64>) -> String {
    with_session(session_id,"fortress.observe",max_wall_millis,Ok(ExcavationCommand::Observe),Ok(QueryRequest::default())).await
}
#[tool(name="fortress.plan",description="Create a local reviewed bounded run, not a native preparation. request is closed JSON: key, observation_witness, game_ticks, wall_millis; optional samples, stable_ticks, interval_ticks, max_gap_ticks. Native targets must already be visible, dry, unsatisfied and paused.")]
pub async fn fortress_plan(session_id: String, request: String) -> String {
    let command=requests::parse::<PlanRequest>(&request).and_then(PlanRequest::command);
    with_session(session_id,"fortress.plan",None,command,Ok(QueryRequest::default())).await
}
#[tool(name="fortress.commit",description="Confirm the exact local review once. request contains key, plan_digest, confirm=true. Re-observe, persist intent/preparation/dispatch, then one native commit. Duplicate requests return history; errors never restore dispatch permission.")]
pub async fn fortress_commit(session_id: String, request: String) -> String {
    let command=requests::parse::<CommitRequest>(&request).and_then(CommitRequest::command);
    with_session(session_id,"fortress.commit",None,command,Ok(QueryRequest::default())).await
}
#[tool(name="fortress.wait",description="One foreground native receipt query, or cached terminal evidence. request contains key and plan_digest. No polling, unpause or retry. SourceLost remains an unresolved obligation even though its native record is terminal.")]
pub async fn fortress_wait(session_id: String, request: String, max_wall_millis: Option<u64>) -> String {
    let command=requests::parse::<Identity>(&request).and_then(Identity::checked).map(|(key,digest)|ExcavationCommand::Wait{key,digest});
    with_session(session_id,"fortress.wait",max_wall_millis,command,Ok(QueryRequest::default())).await
}
#[tool(name="fortress.query",description="Verify local durable excavation inventory without native calls. request is closed JSON: kind=records with optional state (all/unresolved/terminal), limit 1..4, continuation; or kind=schema. Pending work remains visible independently of pagination.")]
pub async fn fortress_query(session_id: String, request: String) -> String {
    with_session(session_id,"fortress.query",None,Ok(ExcavationCommand::Inventory),requests::parse::<QueryRequest>(&request)).await
}
#[tool(name="fortress.cancel",description="Closed JSON scope: plan/effect require key and plan_digest; session accepts release_for_recovery. Plan cancellation is local. Effect cancellation requires Control and Clock authority. Explicit recovery release never claims native stop or quiescence.")]
pub async fn fortress_cancel(session_id: String, request: String) -> String {
    let command=requests::parse::<CancelRequest>(&request).and_then(CancelRequest::command);
    with_session(session_id,"fortress.cancel",None,command,Ok(QueryRequest::default())).await
}
#[tool(name="fortress.explain",description="Inspect exact retained plan, before capture, validated receipt and last native sample without a new native read. request contains key and plan_digest. Sampled floor and verified historical pause do not prove current pause or mining causality.")]
pub async fn fortress_explain(session_id: String, request: String) -> String {
    let command=requests::parse::<Identity>(&request).and_then(Identity::checked).map(|(key,digest)|ExcavationCommand::Explain{key,digest});
    with_session(session_id,"fortress.explain",None,command,Ok(QueryRequest::default())).await
}
#[tool(name="fortress.doctor",description="Verify this journal's retained excavation coordination inventory. Does not certify game health, global clock fencing, power-loss durability or production admission.")]
pub async fn fortress_doctor(session_id: String) -> String {
    with_session(session_id,"fortress.doctor",None,Ok(ExcavationCommand::Inventory),Ok(QueryRequest::default())).await
}
#[tool(name="fortress.checkpoint",description="Unavailable: excavation coordination journals are not verified game saves.")]
pub async fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id,"fortress.checkpoint",None,Err(denied()),Ok(QueryRequest::default())).await
}
#[tool(name="fortress.restore",description="Unavailable: receipt replay cannot restore a fortress or undo excavation.")]
pub async fn fortress_restore(session_id: String) -> String {
    with_session(session_id,"fortress.restore",None,Err(denied()),Ok(QueryRequest::default())).await
}
/// Launch only this isolated development profile. The production runner map is unchanged.
pub fn run_stdio(config: Configuration) -> Result<()> {
    runtime::environment()?;
    CONFIG.set(config).map_err(|_|error(ErrorCode::Conflict,"excavation server already configured"))?;
    let server=ServerBuilder::new("dfmcp-excavation-run-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted development excavation-run/1.18 only. Default offline inspection has no native connection. Recover queries original keys; Control requires explicit operator clock enablement. Inspect pending work, observe, locally review a finite run, confirm its exact digest once, then query and reconcile. This profile never designates terrain. Sampled floor, historical pause, current pause and mining causality are separate. Never re-prepare or invent a new key to evade uncertainty. No global controller fence, checkpoint, restore, arbitrary command or production admission.")
        .build();
    crate::run_modern_stdio(server);
    Ok(())
}
#[cfg(test)]
mod tests;
