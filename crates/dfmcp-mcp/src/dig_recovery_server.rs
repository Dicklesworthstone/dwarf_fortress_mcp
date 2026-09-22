#![forbid(unsafe_code)]
//! Isolated mining history and query-only recovery on the eleven-tool MCP waist.
//! No tool here can prepare, commit, cancel a native effect or create a journal.
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigCursor, DigMode, MAX_FRAME_BYTES, MAX_JOURNAL_BYTES};
use dfmcp_adapter::dig_designation::journal::session::{DigSession, DigSessionGuard, DigSessionView, SOURCE_RESERVATION_BYTES};
use dfmcp_adapter::dig_designation::rpc::{DigSource, DigRpcClient, DigTcpStream, RPC_BYTES};
use dfmcp_adapter::dig_designation::journal::private_file::{PrivateDigFile, open_private_dig};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, GameTick,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

mod presentation;
mod runtime;
mod goals;
use presentation::{OUTPUT_BYTES, digest, failure, inventory, mode_name, packet};
use runtime::{Config, RequestControl, QueryOnly};

const MAX_WORK_BYTES: u64 = 1024 * 1024 * 1024;
const VIEW_BYTES: u64 = MAX_JOURNAL_BYTES as u64 + 4096;
const LOCAL_BYTES: u64 = VIEW_BYTES + 32 * 1024;
const OPEN_BYTES: u64 = 3 * VIEW_BYTES;
const FAMILY: u128 = 16u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
type Native = QueryOnly<DigRpcClient<DigTcpStream>>;
struct Entry { state: State<PrivateDigFile, Native>, config: Config, goals: goals::Inventory }
static SESSION: Mutex<Option<Entry>> = Mutex::new(None);

fn error(code: ErrorCode, text: &str) -> dfmcp_core::DfmcpError { dfmcp_core::DfmcpError::new(code, text) }
fn exhausted() -> dfmcp_core::DfmcpError { error(ErrorCode::BudgetExceeded, "complete recovery work and response exceed the allowance") }
fn unbound(op: &str, e: &dfmcp_core::DfmcpError) -> String { packet(op, failure(e), None, None, None, None) }
fn key(raw: &str) -> Result<()> {
    if raw.is_empty() || raw.len() > 128 || !raw.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b,b'.'|b'_'|b'-')) {
        return Err(error(ErrorCode::InvalidRequest, "invalid mining key"));
    }
    Ok(())
}
fn identity(raw_key: String, raw_digest: String) -> Result<(String, Digest32)> {
    key(&raw_key)?; Ok((raw_key, digest(&raw_digest)?))
}
#[derive(Deserialize)]
#[serde(tag="mode", rename_all="snake_case", deny_unknown_fields)]
enum Query {
    Records { limit: Option<usize>, continuation: Option<String> },
    Tiles { idempotency_key: String, plan_digest: String, offset: usize, limit: Option<usize> },
    Schema,
}
impl Query {
    fn parse(raw: &str) -> Result<Self> {
        if raw.len() > 2048 { return Err(error(ErrorCode::InvalidRequest, "query exceeds 2 KiB")); }
        let query: Self = serde_json::from_str(raw).map_err(|_| error(ErrorCode::InvalidRequest, "query must match the closed mining recovery schema"))?;
        match &query {
            Self::Records { limit, continuation } => {
                if !(1..=8).contains(&limit.map_or(8, |n| n)) { return Err(exhausted()); }
                if let Some(token) = continuation { digest(token)?; }
            }
            Self::Tiles { idempotency_key, plan_digest, offset, limit } => {
                key(idempotency_key)?; digest(plan_digest)?;
                if *offset >= 300 || !(1..=16).contains(&limit.map_or(16, |n| n)) { return Err(exhausted()); }
            }
            Self::Schema => {},
        }
        Ok(query)
    }
}
enum Action { Query(Query), Explain(String,Digest32), Wait(String,Digest32), Inventory, Denied }
#[derive(Clone, Default)]
struct Cursors { sequence: u64, entries: VecDeque<(String, DigCursor)> }
impl Cursors {
    fn issue(&mut self, cursor: DigCursor, c: &OperationContext, head: Digest32) -> Result<String> {
        self.sequence = self.sequence.checked_add(1).ok_or_else(exhausted)?;
        let mut bytes = b"dfmcp-dig-recovery-cursor/1\0".to_vec();
        bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
        bytes.extend_from_slice(head.as_bytes()); bytes.extend_from_slice(&self.sequence.to_be_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if self.entries.len() == 64 { self.entries.pop_front(); }
        self.entries.push_back((token.clone(),cursor)); Ok(token)
    }
    fn resolve(&self, token: &str) -> Result<DigCursor> {
        digest(token)?;
        self.entries.iter().find(|(t,_)| t==token).map(|(_,c)| c.clone())
            .ok_or_else(|| error(ErrorCode::StaleAnchor, "recovery continuation expired; restart discovery"))
    }
}
struct State<S,N> {
    id: SessionId, request: u128, budget: WorkBudget,
    control: DigSession<S,N>, binding: DigBinding, cursors: Cursors,
}
impl<S: EffectJournalStorage,N: DigSource> State<S,N> {
    fn new(control: DigSession<S,N>, c: &OperationContext) -> Result<Self> {
        if control.mode()==DigMode::Control { return Err(error(ErrorCode::CapabilityDenied,"recovery server refuses Control journals")); }
        Ok(Self { id:c.session_id, request:c.request_id.get(), budget:c.budget,
            binding:control.binding().clone(),control,cursors:Cursors::default() })
    }
    fn context(&mut self, wall: Option<u64>) -> Result<OperationContext> {
        self.request = self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget=self.budget;
        if let Some(wall)=wall { budget.max_wall_millis=budget.max_wall_millis.min(wall); }
        Ok(context(self.id,RequestId::new(self.request),self.binding.fortress_id(),self.binding.scope(),
            self.control.high_tick(),budget))
    }
}
fn context(id: SessionId, request_id: RequestId, fortress: dfmcp_core::FortressId,
    scope: dfmcp_core::MapCuboid, tick:u64,budget:WorkBudget) -> OperationContext
{
    OperationContext {session_id:id,request_id,budget,cancellation_requested:false,
        anchor:StateAnchor {fortress_id:fortress,cursor:ObservationCursor::ORIGIN,tick:GameTick(tick),state_hash:Digest32::ZERO},
        grants:vec![CapabilityGrant {capability:Capability::Query,scope:CapabilityScope {
            fortress_id:Some(fortress),map_area:Some(scope),..CapabilityScope::default()},max_risk:RiskTier::ReadOnly,
            expires_at_tick:None,remaining_uses:None}]}
}
struct Work { deadline:Instant, bytes:u64 }
impl Work {
    fn new(c:&OperationContext, started:Instant) -> Result<Self> {
        c.budget.validate()?;
        if c.budget.max_wall_millis>60_000 || c.budget.max_bytes>MAX_WORK_BYTES
            || c.budget.max_output_tokens>65_536 || u64::from(c.budget.max_output_tokens)*4<OUTPUT_BYTES {
            return Err(exhausted());
        }
        let bytes=c.budget.max_bytes.checked_sub(OUTPUT_BYTES+2*VIEW_BYTES).ok_or_else(exhausted)?;
        let deadline=started.checked_add(Duration::from_millis(c.budget.max_wall_millis)).ok_or_else(exhausted)?;
        let work=Self{deadline,bytes}; work.current(c)?; Ok(work)
    }
    fn current(&self,c:&OperationContext)->Result<OperationContext> {
        let ms=self.deadline.checked_duration_since(Instant::now()).ok_or_else(exhausted)?.as_millis();
        if ms==0 {return Err(exhausted());}
        let mut out=c.clone();out.budget.max_wall_millis=u64::try_from(ms).map_err(|_| exhausted())?;
        out.budget.max_bytes=self.bytes;Ok(out)
    }
    fn take(&mut self,c:&OperationContext,n:u64)->Result<OperationContext> {
        let mut out=self.current(c)?;self.bytes=self.bytes.checked_sub(n).ok_or_else(exhausted)?;
        out.budget.max_bytes=n;Ok(out)
    }
    // Two view allowances are reserved before any action, never refunded to RPC.
    fn view(&self,c:&OperationContext)->Result<OperationContext> {
        let mut out=self.current(c)?;out.budget.max_bytes=VIEW_BYTES;Ok(out)
    }
}
struct Projection<'a> { view: &'a DigSessionView, cursors: &'a mut Cursors }
fn perform<S,N,F,G>(state:&mut State<S,N>,c:&OperationContext,work:&mut Work,projection:Projection<'_>,
    action:Action,factory:F,guard:&mut G)->Result<Value>
where S:EffectJournalStorage,N:DigSource,G:DigSessionGuard,
    F:FnOnce(&DigBinding,dfmcp_adapter::dig_designation::DigRegion,&OperationContext)->Result<N>
{
    let Projection { view, cursors } = projection;
    match action {
        Action::Inventory=>Ok(json!({"ok":true,"journal":inventory(view),"native_calls":0})),
        Action::Denied=>Err(error(ErrorCode::CapabilityDenied,"mining recovery cannot create or mutate game state")),
        Action::Explain(key,plan)=> {
            let r=state.control.get(&key,plan,&work.take(c,LOCAL_BYTES)?)?;
            Ok(json!({"ok":true,"record":presentation::record(&r),"native_calls":0}))
        }
        Action::Query(Query::Schema)=> {
            let schema:Value=serde_json::from_str(include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../schemas/dig_recovery_query.json")))
                .map_err(|_| error(ErrorCode::InternalInvariantViolation,"invalid embedded recovery schema"))?;
            Ok(json!({"ok":true,"schema":schema,"native_calls":0}))
        }
        Action::Query(Query::Records{limit,continuation})=> {
            let limit=limit.map_or(8,|n|n);
            let cursor=continuation.as_deref().map(|s| cursors.resolve(s)).transpose()?;
            let page=state.control.list(&work.take(c,LOCAL_BYTES)?,limit,cursor.as_ref())?;
            let next=page.continuation.map(|p|cursors.issue(p,c,page.head)).transpose()?;
            Ok(json!({"ok":true,"journal_id":page.journal_id.to_string(),"head":page.head.to_string(),
                "records":page.records.iter().map(presentation::summary).collect::<Vec<_>>(),
                "total_records":page.total_records,"unsettled_records":page.unsettled_records,
                "continuation":next,"complete_set_in_response":continuation.is_none()&&next.is_none(),"native_calls":0}))
        }
        Action::Query(Query::Tiles{idempotency_key,plan_digest,offset,limit})=> {
            let r=state.control.get(&idempotency_key,digest(&plan_digest)?,&work.take(c,LOCAL_BYTES)?)?;
            Ok(json!({"ok":true,"capture":presentation::tiles(&r,offset,limit.map_or(16,|n|n))?,"native_calls":0}))
        }
        Action::Wait(key,plan)=> {
            let r=state.control.get(&key,plan,&work.take(c,LOCAL_BYTES)?)?;
            if !r.needs_reconciliation() {
                return Ok(json!({"ok":true,"record":presentation::record(&r),"native_calls":0}));
            }
            let reserved=36*(view.byte_len+2*MAX_FRAME_BYTES) as u64+SOURCE_RESERVATION_BYTES+4*RPC_BYTES+65_536;
            let r=state.control.reconcile(&key,plan,&work.take(c,reserved)?,factory,guard)?;
            Ok(json!({"ok":true,"record":presentation::record(&r),"query_attempted":true,
                "commit_attempted":false,"historical_evidence_only":true}))
        }
    }
}
fn run_action<S,N,F,G>(state:&mut State<S,N>,c:OperationContext,started:Instant,op:&str,
    action:Result<Action>,factory:F,guard:&mut G)->String
where S:EffectJournalStorage,N:DigSource,G:DigSessionGuard,
    F:FnOnce(&DigBinding,dfmcp_adapter::dig_designation::DigRegion,&OperationContext)->Result<N>
{
    let mode=state.control.mode();let binding=state.binding.clone();
    let mut work=match Work::new(&c,started) {Ok(w)=>w,Err(e)=>return packet(op,failure(&e),Some(&c),Some(mode),None,None)};
    let before=match work.view(&c).and_then(|v|state.control.view(&v)) {
        Ok(v)=>v,Err(e)=>return packet(op,failure(&e),Some(&c),Some(mode),None,None),
    };
    let mut cursors=state.cursors.clone();
    let result=action.and_then(|a|perform(state,&c,&mut work,Projection { view:&before,cursors:&mut cursors },a,factory,guard));
    let after=work.view(&c).and_then(|v|state.control.view(&v));
    let publish_cursors=result.is_ok()&&after.is_ok();
    let (value,view)=match after {
        Ok(v)=>(match result{Ok(value)=>value,Err(e)=>failure(&e)},Some(v)),
        Err(e)=>{
            let mut failed=failure(&e);
            failed["last_verified_journal_at_request_start"]=inventory(&before);
            failed["pending_at_request_start"]=before.pending.as_ref().map_or(Value::Null,presentation::summary);
            failed["current_custody_verified"]=json!(false);
            (failed,None)
        },
    };
    let output=packet(op,value,Some(&c),Some(mode),Some(&binding),view.as_ref());
    if output.len() as u64>OUTPUT_BYTES {
        return packet(op,failure(&exhausted()),Some(&c),Some(mode),Some(&binding),view.as_ref());
    }
    // Cursor state is presentation only. Publish it only with a complete packet.
    if publish_cursors {state.cursors=cursors;}output
}
fn lock()->Result<MutexGuard<'static,Option<Entry>>> {
    match SESSION.try_lock() {Ok(v)=>Ok(v),Err(TryLockError::WouldBlock)=>Err(exhausted()),
        Err(TryLockError::Poisoned(_))=>Err(error(ErrorCode::InternalInvariantViolation,"mining recovery session poisoned"))}
}
fn session_id(raw:&str)->Result<SessionId> {
    if raw.len()!=32 || !raw.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {
        return Err(error(ErrorCode::InvalidRequest,"invalid mining recovery session ID"));
    }
    let value=u128::from_str_radix(raw,16).map_err(|_|error(ErrorCode::InvalidRequest,"invalid mining recovery ID"))?;
    let id=SessionId::new(value);
    if id.get()!=value || !id.is_process_scoped_live() || (value&((1u128<<62)-1))>>57!=16 {
        return Err(error(ErrorCode::InvalidRequest,"not a mining recovery session ID"));
    }
    Ok(id)
}
fn with_session(control:RequestControl,raw:String,op:&str,wall:Option<u64>,action:Result<Action>)->String {
    let result=(||->Result<String>{
        let id=session_id(&raw)?;let mut locked=lock()?;
        let entry=locked.as_mut().filter(|e|e.state.id==id).ok_or_else(||error(ErrorCode::SessionNotFound,"mining recovery session absent"))?;
        runtime::boundary(&control,&entry.config)?;
        let original=entry.state.context(wall)?;
        Work::new(&original,control.started)?; // Validate the original ceiling before goal reservations.
        let mut c=entry.goals.reserve(&original)?;
        let overall=Work::new(&c,control.started)?; // Refuse before goal/native I/O.
        let config=entry.config.clone();let binding=entry.state.binding.clone();
        let snapshots=entry.goals.load(&c,&binding,control.started,||runtime::boundary(&control,&config))?;
        entry.goals.narrow(&mut c);
        let prior_cursors=entry.state.cursors.clone();
        let mut guard=runtime::Guard::new(&control,&config,&binding);
        let output=run_action(&mut entry.state,c.clone(),control.started,op,action,
            |b,r,c|runtime::connect(&control,&config,b,r,c),&mut guard);
        // Native history pagination is published only when the final combined
        // native/goal handoff is complete, not at the earlier native-only render.
        let staged_cursors=std::mem::replace(&mut entry.state.cursors,prior_cursors);
        let output=snapshots.finish(&mut entry.goals,output,OUTPUT_BYTES as usize,||{
            runtime::boundary(&control,&config)?;overall.current(&c)?;Ok(())
        })?;
        let rendered:Value=serde_json::from_str(&output).map_err(|_|exhausted())?;
        if rendered["result"]["ok"]==true {entry.state.cursors=staged_cursors;}
        Ok(output)
    })();
    match result {Ok(s)=>s,Err(e)=>unbound(op,&e)}
}
fn open(control:RequestControl,wall:Option<u64>,bytes:Option<u64>,tokens:Option<u32>)->String {
    let result=(||->Result<String>{
        let config=runtime::configuration()?;runtime::boundary(&control,&config)?;
        let budget=WorkBudget {max_wall_millis:wall.map_or(10_000,|n|n),max_bytes:bytes.map_or(MAX_WORK_BYTES,|n|n),
            max_output_tokens:tokens.map_or(8192,|n|n),max_entities:1000,max_game_ticks:0,max_actions:1};
        let mut locked=lock()?;if locked.is_some(){return Err(error(ErrorCode::Conflict,"release the current recovery session first"));}
        let next=NEXT.fetch_update(Ordering::AcqRel,Ordering::Acquire,|v|(v<(1u64<<57)).then_some(v+1)).map_err(|_|exhausted())?;
        let id=SessionId::new((1u128<<127)|FAMILY|u128::from(next));
        let original=context(id,RequestId::new(1),config.fortress(),config.scope,0,budget);
        Work::new(&original,control.started)?; // Reservations cannot admit an oversized original request.
        let mut goals=goals::Inventory::new(config.goal_files.clone());
        let mut c=goals.reserve(&original)?;
        let mut work=Work::new(&c,control.started)?;
        // Validate the operator's exact scope/fortress through read-only custody
        // before any recovery-mode resynchronization of an existing journal.
        let inspected=open_private_dig(&config.path,&work.take(&c,OPEN_BYTES)?,DigMode::Offline,None)?;
        config.matches(inspected.binding())?;
        let journal=if config.mode()==DigMode::Recover {
            let expected=inspected.binding().clone();let id=inspected.id();let head=inspected.head();
            drop(inspected);runtime::boundary(&control,&config)?;
            let reopened=open_private_dig(&config.path,&work.take(&c,OPEN_BYTES)?,DigMode::Recover,Some(expected))?;
            if reopened.id()!=id||reopened.head()!=head{return Err(error(ErrorCode::Conflict,"mining history changed during recovery open"));}
            reopened
        }else{inspected};
        let session=DigSession::new(journal,&work.view(&c)?)?;
        let mut state=State::new(session,&c)?;
        state.budget=budget; // Goal reservations are per request, not cumulative narrowing.
        let snapshots=goals.load(&c,&state.binding,control.started,||runtime::boundary(&control,&config))?;
        goals.narrow(&mut c);
        let view=state.control.view(&work.view(&c)?)?;
        runtime::boundary(&control,&config)?;work.current(&c)?;
        let output=packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),
            "mode":mode_name(config.mode()),"capabilities":["query"],"journal":inventory(&view),
            "native_calls":0,"mutation_admissible":false}),Some(&c),Some(config.mode()),Some(&state.binding),Some(&view));
        if output.len() as u64>OUTPUT_BYTES{return Err(exhausted());}
        let output=snapshots.finish(&mut goals,output,OUTPUT_BYTES as usize,||{
            runtime::boundary(&control,&config)?;work.current(&c)?;Ok(())
        })?;
        *locked=Some(Entry{state,config,goals});Ok(output)
    })();
    match result{Ok(s)=>s,Err(e)=>unbound("fortress.open_session",&e)}
}
fn release(control:RequestControl,raw:String,force:bool)->String {
    let result=(||->Result<String>{
        control.checkpoint()?;
        let id=session_id(&raw)?;let mut locked=lock()?;
        let entry=locked.as_mut().filter(|e|e.state.id==id).ok_or_else(||error(ErrorCode::SessionNotFound,"mining recovery session absent"))?;
        let c=entry.state.context(None)?;
        let view=if force {None} else {
            runtime::boundary(&control,&entry.config)?;
            let work=Work::new(&c,control.started)?;
            let view=entry.state.control.view(&work.view(&c)?)?;
            if view.pending.is_some(){return Err(error(ErrorCode::EffectIndeterminate,"unsettled mining requires explicit release_for_recovery"));}
            Some(view)
        };
        let output=packet("fortress.cancel",json!({"ok":true,"scope":"session","closed":true,
            "release_for_recovery":force,"native_calls":0,"journal_changed":false,"effects_cancelled":false,
            "quiescence_of_game_effects_proven":false}),Some(&c),Some(entry.state.control.mode()),Some(&entry.state.binding),view.as_ref());
        if output.len() as u64>OUTPUT_BYTES{return Err(exhausted());}
        let output=entry.goals.release(output,OUTPUT_BYTES as usize)?;
        control.checkpoint()?;drop(locked.take());Ok(output)
    })();
    match result{Ok(s)=>s,Err(e)=>unbound("fortress.cancel",&e)}
}

#[tool(name="fortress.open_session",description="Open an existing operator-selected dig/1.16 journal for offline history or operator-enabled query-only recovery. No credentials or native connection are needed to open. Never creates a journal, grants Designate, or restores a commit permit.")]
pub async fn fortress_open_session(max_wall_millis:Option<u64>,max_bytes:Option<u64>,max_output_tokens:Option<u32>)->String {
    runtime::owned("fortress.open_session",move|c|open(c,max_wall_millis,max_bytes,max_output_tokens)).await
}
#[tool(name="fortress.observe",description="Orient to verified mining journal inventory, not live terrain. Includes any unsettled key even when it is not on the first records page. Optional operator-selected excavation journals are also replayed into active work. Zero native calls.")]
pub async fn fortress_observe(session_id:String)->String {
    runtime::owned("fortress.observe",move|c|with_session(c,session_id,"fortress.observe",None,Ok(Action::Inventory))).await
}
#[tool(name="fortress.query",description="Closed JSON query: records with limit 1..8 and continuation; tiles with exact idempotency_key, plan_digest, offset and limit 1..16; or schema. Historical evidence only. No native calls or renewed mutation authority.")]
pub async fn fortress_query(session_id:String,query:String)->String {
    runtime::owned("fortress.query",move|c|with_session(c,session_id,"fortress.query",None,Query::parse(&query).map(Action::Query))).await
}
#[tool(name="fortress.explain",description="Inspect an exact retained mining plan, scope, hidden-neighbor policy and native receipt. This is historical configuration evidence, never completed excavation or present terrain.")]
pub async fn fortress_explain(session_id:String,idempotency_key:String,plan_digest:String)->String {
    runtime::owned("fortress.explain",move|c|with_session(c,session_id,"fortress.explain",None,
        identity(idempotency_key,plan_digest).map(|(k,p)|Action::Explain(k,p)))).await
}
#[tool(name="fortress.wait",description="In recover mode, perform at most one QueryDesignation for the exact retained operation and persist validated evidence. Terminal and permanent-Unknown records return locally without credentials. Never prepares, commits, cancels or retries a designation. Offline uncertainty requires operator-enabled recovery reopen.")]
pub async fn fortress_wait(session_id:String,idempotency_key:String,plan_digest:String,max_wall_millis:Option<u64>)->String {
    runtime::owned("fortress.wait",move|c|with_session(c,session_id,"fortress.wait",max_wall_millis,
        identity(idempotency_key,plan_digest).map(|(k,p)|Action::Wait(k,p)))).await
}
#[tool(name="fortress.cancel",description="Release only this MCP session and its journal lock. Unsettled or fenced history needs explicit release_for_recovery=true. Does not cancel a native effect, undo designation, erase history or prove game quiescence.")]
pub async fn fortress_cancel(session_id:String,release_for_recovery:Option<bool>)->String {
    runtime::owned("fortress.cancel",move|c|release(c,session_id,release_for_recovery==Some(true))).await
}
#[tool(name="fortress.doctor",description="Verify mining journal custody and report complete local coordination inventory. No live game health, mining safety or compatibility admission is inferred.")]
pub async fn fortress_doctor(session_id:String)->String {
    runtime::owned("fortress.doctor",move|c|with_session(c,session_id,"fortress.doctor",None,Ok(Action::Inventory))).await
}
#[tool(name="fortress.plan",description="Unavailable in the query-only mining recovery profile. Retained plans cannot create new designation authority.")]
pub async fn fortress_plan(session_id:String)->String {
    runtime::owned("fortress.plan",move|c|with_session(c,session_id,"fortress.plan",None,Ok(Action::Denied))).await
}
#[tool(name="fortress.commit",description="Unavailable. This recovery profile cannot prepare, commit or replay a mining designation, even with a valid historical digest.")]
pub async fn fortress_commit(session_id:String)->String {
    runtime::owned("fortress.commit",move|c|with_session(c,session_id,"fortress.commit",None,Ok(Action::Denied))).await
}
#[tool(name="fortress.checkpoint",description="Unavailable. A mining journal is not a fortress save or game checkpoint.")]
pub async fn fortress_checkpoint(session_id:String)->String {
    runtime::owned("fortress.checkpoint",move|c|with_session(c,session_id,"fortress.checkpoint",None,Ok(Action::Denied))).await
}
#[tool(name="fortress.restore",description="Unavailable. Mining receipt recovery neither restores the game nor undoes a designation.")]
pub async fn fortress_restore(session_id:String)->String {
    runtime::owned("fortress.restore",move|c|with_session(c,session_id,"fortress.restore",None,Ok(Action::Denied))).await
}
pub fn run_stdio() {
    if let Err(e)=runtime::configuration(){eprintln!("{e}");std::process::exit(1);}
    let server=ServerBuilder::new("dfmcp-dig-recovery-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted query-only mining recovery. Discover the original journal and unsettled key first. Historical designation receipts do not prove current terrain, safety or excavation completion. Only an explicit wait in operator-enabled recover mode can query native evidence; it cannot dispatch or cancel mining. Missing native records never prove nonapplication. Session release preserves all obligations. Paths, scope, fortress, credentials and mode are operator-owned. No production admission or mutation authority exists. Optional excavation-goal journals are independently verified historical terrain evidence, never native-effect completion or permission to retry. Goal sampling remains in the standalone tracker.")
        .build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;
