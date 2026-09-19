#![forbid(unsafe_code)]
//! Isolated work-orders/1.10 MCP development runtime; never a production runner.
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::work_order_control::{
    CreationState, JournalMode, MAX_RECORD_BYTES, PrivateWorkOrderFile, RPC_RESERVE_BYTES,
    WorkOrderSource, open_private_work_orders, session::WorkOrderSession,
};
use dfmcp_adapter::work_orders::{WorkOrderSpec, rpc::{WorkOrderRpcClient, WorkOrderTcpStream}};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, FortressId,
    GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

#[path = "work_order_presentation.rs"]
mod presentation;
use presentation::{BASE_RESERVE, RECORD_RESERVE, MAX_PAGE, Continuations, Filter, TurnView,
    digest, error, failure, mode_name, observation_json, packet, recipe, record_json, summary_json};

const FAMILY: u128 = 10u128 << 57;
const SEQUENCE_LIMIT: u64 = 1u64 << 57;
const MAX_BYTES: u64 = 68 * 1024 * 1024;
const BOOTSTRAP_RPC_BYTES: u64 = 6 * RPC_RESERVE_BYTES + 24;
static NEXT: AtomicU64 = AtomicU64::new(1);
type LiveState = State<PrivateWorkOrderFile, WorkOrderRpcClient<WorkOrderTcpStream>>;
static SESSION: Mutex<Option<LiveState>> = Mutex::new(None);

struct State<S, N> {
    id: SessionId, request: u128, anchor: StateAnchor, budget: WorkBudget,
    grants: Vec<CapabilityGrant>, mode: JournalMode, control: WorkOrderSession<S, N>, cursors: Continuations,
}
impl<S: EffectJournalStorage, N: WorkOrderSource> State<S, N> {
    fn context(&mut self, production_enabled: bool, cancelled: bool) -> Result<OperationContext> {
        self.request = self.request.checked_add(1)
            .ok_or_else(||error(ErrorCode::BudgetExceeded,"creation request identities exhausted"))?;
        Ok(OperationContext { session_id:self.id, request_id:RequestId::new(self.request),
            anchor:self.anchor, budget:self.budget,
            grants:self.grants.iter().filter(|g|g.capability != Capability::ConfigureProduction || production_enabled)
                .cloned().collect(), cancellation_requested:cancelled })
    }
    fn perform(&mut self, action: Action, context: &OperationContext) -> Result<Value> {
        match action {
            Action::Observe => {
                let observation = self.control.observe(context)?;
                self.anchor.tick = GameTick(observation.tick()); self.anchor.state_hash = observation.witness();
                Ok(json!({"ok":true,"observation":observation_json(&observation),"game_mutation_dispatched":false}))
            }
            Action::Plan { key, spec, witness } => {
                let record = self.control.plan(&key, spec, witness, context)?;
                Ok(json!({"ok":true,"effect":record_json(&record),"game_mutation_dispatched":false}))
            }
            Action::Commit { key, plan, witness } => {
                let record = self.control.commit(&key, plan, witness, context)?;
                Ok(json!({"ok":true,"operation_acknowledged":true,"effect":record_json(&record),
                    "creation_verified":record.state() == CreationState::Created,"production_goal_completion_proven":false}))
            }
            Action::Wait { key, plan } => {
                let record = self.control.reconcile(&key, plan, context)?;
                Ok(json!({"ok":true,"effect":record_json(&record),"game_mutation_dispatched":false}))
            }
            Action::Explain { key, plan } => {
                let record = self.control.record(&key, plan, context)?;
                Ok(json!({"ok":true,"effect":record_json(&record),"native_calls":0}))
            }
            Action::Cancel { key, plan } => {
                let record = self.control.cancel(&key, plan, context)?;
                Ok(json!({"ok":true,"effect":record_json(&record),"native_calls":0,"manager_order_deleted":false}))
            }
            Action::Query { filter, limit, continuation } => {
                let summary = self.control.summary(context)?;
                let after = continuation.as_deref().map(|token|
                    self.cursors.resolve(token,self.id,&summary,filter,limit)).transpose()?;
                let scan = 64usize.min((context.budget.max_bytes / MAX_RECORD_BYTES as u64) as usize)
                    .min(context.budget.max_entities as usize);
                if scan == 0 { return Err(error(ErrorCode::BudgetExceeded,"no complete creation record fits the work budget")); }
                let page = self.control.records_page(summary.head,after.as_deref(),scan,context)?;
                let mut rows = Vec::new(); let mut consumed = 0;
                for record in &page.records {
                    consumed += 1;
                    if filter.matches(record.state()) { rows.push(record_json(record)); }
                    if rows.len() == limit { break; }
                }
                let more = consumed < page.records.len() || page.next_after.is_some();
                let next = if more {
                    match page.records.get(consumed.saturating_sub(1)) {
                        Some(last) => Some(self.cursors.issue(self.id,&summary,last.plan().key().to_owned(),filter,limit)?),
                        None => return Err(error(ErrorCode::InternalInvariantViolation,"creation page did not advance")),
                    }
                } else { None };
                Ok(json!({"ok":true,"records":rows,"scanned_records":consumed,"state":filter.name(),
                    "matching_records_in_journal":filter.total(&summary),"continuation":next,
                    "complete_matching_set_in_this_response":rows.len() == filter.total(&summary),
                    "journal":summary_json(&summary)}))
            }
            Action::Doctor => Ok(json!({"ok":true,"journal":summary_json(&self.control.summary(context)?),
                "bridge_connection_present":self.control.has_source(context)?,"native_calls":0,
                "runtime_admitted":false,"current_freshness_proven":false})),
            Action::Unavailable => Err(error(ErrorCode::CapabilityDenied,
                "work-orders/1.10 does not implement checkpoint, restore or other mutation families")),
        }
    }
}
enum Action {
    Observe, Plan { key:String,spec:WorkOrderSpec,witness:Digest32 },
    Commit { key:String,plan:Digest32,witness:Digest32 }, Wait { key:String,plan:Digest32 },
    Explain { key:String,plan:Digest32 }, Cancel { key:String,plan:Digest32 },
    Query { filter:Filter,limit:usize,continuation:Option<String> }, Doctor, Unavailable,
}
#[derive(Default)]
struct Limits { wall:Option<u64>, bytes:Option<u64>, tokens:Option<u32> }
fn narrowed(mut context: OperationContext, limits: Limits, rows: usize) -> Result<(OperationContext,OperationContext)> {
    if let Some(v) = limits.wall { context.budget.max_wall_millis = v.min(context.budget.max_wall_millis); }
    if let Some(v) = limits.bytes { context.budget.max_bytes = v.min(context.budget.max_bytes); }
    if let Some(v) = limits.tokens { context.budget.max_output_tokens = v.min(context.budget.max_output_tokens); }
    context.budget.validate()?;
    if rows > MAX_PAGE { return Err(error(ErrorCode::InvalidRequest,"creation output page exceeds eight records")); }
    let reserve = BASE_RESERVE + RECORD_RESERVE * rows as u64;
    if context.budget.max_bytes <= reserve || u64::from(context.budget.max_output_tokens) * 4 < reserve {
        return Err(error(ErrorCode::BudgetExceeded,"complete creation response and recovery warnings do not fit; no work started"));
    }
    let mut work = context.clone(); work.budget.max_bytes -= reserve;
    Ok((context,work))
}
fn remaining(mut context: OperationContext, started: Instant, total: u64) -> Result<OperationContext> {
    let elapsed = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    context.budget.max_wall_millis = total.checked_sub(elapsed).filter(|n|*n > 0)
        .map(|n|n.min(context.budget.max_wall_millis))
        .ok_or_else(||error(ErrorCode::BudgetExceeded,"creation request deadline expired before native work"))?;
    Ok(context)
}
fn unbound(operation: &str, cause: &dfmcp_core::DfmcpError) -> String {
    packet(operation,failure(cause,operation),TurnView { context:None,mode:None,summary:None,selected:None })
}
fn render<S: EffectJournalStorage,N: WorkOrderSource>(state: &State<S,N>, context: &OperationContext,
    operation: &str, result: Value) -> String
{
    let mut current = context.clone(); current.anchor = state.anchor;
    let checked = state.control.summary(&current);
    let result = match &checked {
        Ok(_) => result,
        Err(cause) => json!({"ok":false,"retained_operation_result":result,
            "post_operation_error":failure(cause,operation),"journal_health_unknown":true}),
    };
    let summary = checked.ok(); let selected = state.control.selected(&current).ok().flatten();
    let out = packet(operation,result,TurnView { context:Some(context),mode:Some(state.mode),summary:summary.as_ref(),selected });
    let limit = context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens) * 4);
    if out.len() as u64 <= limit { return out; }
    let cause = error(if operation == "fortress.commit" { ErrorCode::EffectIndeterminate }
        else { ErrorCode::BudgetExceeded },"complete creation response exceeded reservation; inspect retained evidence before further control");
    packet(operation,failure(&cause,operation),TurnView { context:Some(context),mode:Some(state.mode),
        summary:summary.as_ref(),selected:None })
}
/// Shared actual handler path, parameterized for injected-source regression tests.
fn run_action<S: EffectJournalStorage,N: WorkOrderSource>(state: &mut State<S,N>, context: OperationContext,
    operation: &str, limits: Limits, rows: usize, action: Result<Action>) -> String
{
    let started = Instant::now();
    let (display,work) = match narrowed(context.clone(),limits,rows) {
        Ok(pair) => pair,
        Err(cause) => return render(state,&context,operation,failure(&cause,operation)),
    };
    let outcome = state.control.summary(&work)
        .and_then(|_|remaining(work,started,display.budget.max_wall_millis))
        .and_then(|c|action.and_then(|a|state.perform(a,&c)));
    render(state,&display,operation,outcome.unwrap_or_else(|e|failure(&e,operation)))
}
fn runtime_io() -> Result<()> {
    let cx = fastmcp_rust::asupersync::Cx::current()
        .ok_or_else(||error(ErrorCode::CapabilityDenied,"creation MCP I/O requires its owned runtime context"))?;
    cx.checkpoint().map_err(|_|error(ErrorCode::CancellationRequested,"creation request cancelled"))?;
    if cx.io().is_none() { return Err(error(ErrorCode::CapabilityDenied,"inherited context denies creation MCP I/O")); }
    Ok(())
}
fn session_lock() -> Result<MutexGuard<'static,Option<LiveState>>> {
    match SESSION.try_lock() {
        Ok(value) => Ok(value),
        Err(TryLockError::WouldBlock) => Err(error(ErrorCode::BudgetExceeded,"creation session is serving another bounded request")),
        Err(TryLockError::Poisoned(_)) => Err(error(ErrorCode::InternalInvariantViolation,"creation session poisoned; restart and recover journal")),
    }
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32 || !raw.bytes().all(|b|b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(error(ErrorCode::InvalidRequest,"invalid creation session ID"));
    }
    let value = u128::from_str_radix(raw,16).map_err(|_|error(ErrorCode::InvalidRequest,"invalid session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value || !id.is_process_scoped_live() || (value & ((1u128 << 62)-1)) >> 57 != 10 {
        return Err(error(ErrorCode::InvalidRequest,"not a work-orders/1.10 session ID"));
    }
    Ok(id)
}
const ALLOWED: [&str;6] = ["DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10","DFMCP_WORK_ORDERS_TOKEN",
    "DFMCP_WORK_ORDERS_ENDPOINT","DFMCP_WORK_ORDERS_JOURNAL","DFMCP_WORK_ORDERS_FORTRESS_ID",
    "DFMCP_WORK_ORDERS_ALLOW_PRODUCTION"];
fn environment_contract(opt_in: Option<&str>, production: Option<&str>, keys: &[String], admitted: bool) -> Result<()> {
    if opt_in != Some("1") || admitted || production.is_some_and(|v|v != "1")
        || keys.iter().any(|k|k.starts_with("DFMCP_") && !ALLOWED.contains(&k.as_str()))
    { return Err(error(ErrorCode::CapabilityDenied,"work-orders/1.10 requires exact development opt-in and refuses admission or other DFMCP state")); }
    Ok(())
}
fn production_enabled() -> bool { std::env::var("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION").ok().as_deref() == Some("1") }
fn validate_environment() -> Result<()> {
    let production = std::env::var("DFMCP_WORK_ORDERS_ALLOW_PRODUCTION");
    if matches!(production,Err(std::env::VarError::NotUnicode(_))) {
        return Err(error(ErrorCode::CapabilityDenied,"creation production opt-in must be absent or exactly 1"));
    }
    environment_contract(std::env::var("DFMCP_ALLOW_UNADMITTED_WORK_ORDERS_V1_10").ok().as_deref(),
        production.ok().as_deref(), &std::env::vars_os().map(|(k,_)|k.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        crate::admission::current_admission_provenance().is_some())
}
fn configured(name: &str, maximum: usize) -> Result<String> {
    let value = std::env::var(name).map_err(|_|error(ErrorCode::CapabilityDenied,"required operator creation configuration absent or not UTF-8"))?;
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(error(ErrorCode::InvalidRequest,"operator creation configuration exceeds its bound"));
    }
    Ok(value)
}
fn parse_mode(raw: &str) -> Result<JournalMode> {
    match raw { "offline" => Ok(JournalMode::Offline), "reconcile" => Ok(JournalMode::Reconcile),
        "control" => Ok(JournalMode::Control), _ => Err(error(ErrorCode::InvalidRequest,"mode must be offline, reconcile or control")) }
}
fn grants(mode: JournalMode, fortress: FortressId, enabled: bool) -> Result<Vec<CapabilityGrant>> {
    if mode == JournalMode::Control && !enabled {
        return Err(error(ErrorCode::CapabilityDenied,"control requires operator DFMCP_WORK_ORDERS_ALLOW_PRODUCTION=1"));
    }
    let mut result = vec![CapabilityGrant { capability:Capability::Query,
        scope:CapabilityScope { fortress_id:Some(fortress),..CapabilityScope::default() },
        max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None }];
    if mode == JournalMode::Control {
        result.push(CapabilityGrant { capability:Capability::ConfigureProduction,
            scope:CapabilityScope { fortress_id:Some(fortress),..CapabilityScope::default() },
            max_risk:RiskTier::Reversible,expires_at_tick:None,remaining_uses:None });
    }
    Ok(result)
}
fn with_session<F: FnOnce() -> Result<Action>>(raw: String, operation: &str, limits: Limits,
    rows: usize, action: F) -> String
{
    let result = (|| -> Result<String> {
        runtime_io()?; validate_environment()?;
        let id = session_id(&raw)?; let mut guard = session_lock()?;
        let state = guard.as_mut().filter(|s|s.id == id)
            .ok_or_else(||error(ErrorCode::SessionNotFound,"creation session absent or closed"))?;
        let cancelled = fastmcp_rust::asupersync::Cx::current().is_some_and(|cx|cx.checkpoint().is_err());
        let context = state.context(production_enabled(),cancelled)?;
        Ok(run_action(state,context,operation,limits,rows,action()))
    })();
    result.unwrap_or_else(|cause|unbound(operation,&cause))
}

#[tool(description="Open isolated unadmitted work-orders/1.10. Default offline opens an existing read-only journal without DFHack credentials. Reconcile opens existing writable custody with Query only; control additionally requires operator production enablement. Paths, fortress, endpoint and credentials are operator configuration, not tool arguments.")]
pub fn fortress_open_session(mode: Option<String>,max_wall_millis: Option<u64>,max_bytes: Option<u64>,max_output_tokens: Option<u32>) -> String {
    let result = (|| -> Result<String> {
        runtime_io()?; validate_environment()?; let started = Instant::now();
        let mode = parse_mode(mode.as_deref().unwrap_or("offline"))?;
        let mut guard = session_lock()?;
        if guard.is_some() { return Err(error(ErrorCode::Conflict,"close the retained creation session first")); }
        let raw = configured("DFMCP_WORK_ORDERS_FORTRESS_ID",20)?;
        let number = raw.parse::<u64>().map_err(|_|error(ErrorCode::InvalidRequest,"fortress ID must be canonical nonzero decimal"))?;
        if number == 0 || number.to_string() != raw { return Err(error(ErrorCode::InvalidRequest,"fortress ID must be canonical nonzero decimal")); }
        let fortress = FortressId::new(number); let grants = grants(mode,fortress,production_enabled())?;
        let path = PathBuf::from(configured("DFMCP_WORK_ORDERS_JOURNAL",4096)?);
        let budget = WorkBudget { max_wall_millis:max_wall_millis.unwrap_or(5000),max_bytes:max_bytes.unwrap_or(MAX_BYTES),
            max_output_tokens:max_output_tokens.unwrap_or(65_536),max_entities:4096,max_actions:1,max_game_ticks:0 };
        if budget.max_wall_millis > 60_000 || budget.max_bytes > MAX_BYTES || budget.max_output_tokens > 262_144 {
            return Err(error(ErrorCode::BudgetExceeded,"creation limits exceed 60000ms, 68MiB or 262144 output proxy units"));
        }
        let sequence = NEXT.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n < SEQUENCE_LIMIT).then_some(n+1))
            .map_err(|_|error(ErrorCode::BudgetExceeded,"creation session IDs exhausted"))?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        let anchor = StateAnchor { fortress_id:fortress,cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO };
        let context = OperationContext { session_id:id,request_id:RequestId::new(1),anchor,budget,
            grants:grants.clone(),cancellation_requested:false };
        let (display,mut work) = narrowed(context,Limits::default(),0)?;
        // Offline branches before token/endpoint reads and has no source object.
        let source = if mode == JournalMode::Offline { None } else {
            let endpoint = match std::env::var("DFMCP_WORK_ORDERS_ENDPOINT") {
                Ok(value) if value.len() <= 128 => value,
                Err(std::env::VarError::NotPresent) => "127.0.0.1:5000".to_owned(),
                _ => return Err(error(ErrorCode::InvalidRequest,"creation endpoint must be bounded UTF-8 numeric loopback")),
            };
            let endpoint = dfmcp_adapter::parse_loopback_endpoint(&endpoint)?;
            let token = configured("DFMCP_WORK_ORDERS_TOKEN",256)?.into_bytes();
            work.budget.max_bytes = work.budget.max_bytes.checked_sub(BOOTSTRAP_RPC_BYTES)
                .filter(|n|*n > 0).ok_or_else(||error(ErrorCode::BudgetExceeded,"creation handshake and journal exceed work budget"))?;
            work = remaining(work,started,budget.max_wall_millis)?;
            Some(WorkOrderRpcClient::connect(endpoint,token,id.get().to_be_bytes().to_vec(),Duration::from_millis(work.budget.max_wall_millis))?)
        };
        runtime_io()?; work = remaining(work,started,budget.max_wall_millis)?;
        if mode == JournalMode::Control && !production_enabled() {
            return Err(error(ErrorCode::CapabilityDenied,"creation production grant revoked during opening"));
        }
        let journal = open_private_work_orders(&path,&work,mode)?;
        let control = WorkOrderSession::new(journal,source,&work)?;
        let state = State { id,request:1,anchor,budget,grants,mode,control,cursors:Continuations::default() };
        let summary = state.control.summary(&work)?;
        let out = packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),"mode":mode_name(mode),
            "bridge_connection_present":mode != JournalMode::Offline,"journal":summary_json(&summary),
            "capabilities":state.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),
            "close":{"tool":"fortress.cancel","arguments":{"session_id":id.to_string(),"scope":"session"}}}),
            TurnView { context:Some(&display),mode:Some(mode),summary:Some(&summary),selected:None });
        if out.len() as u64 > display.budget.max_bytes.min(u64::from(display.budget.max_output_tokens)*4) {
            return Err(error(ErrorCode::BudgetExceeded,"creation opening response does not fit; session was not published"));
        }
        remaining(work,started,budget.max_wall_millis)?; runtime_io()?;
        *guard = Some(state); Ok(out)
    })();
    result.unwrap_or_else(|cause|unbound("fortress.open_session",&cause))
}
#[tool(description="Observe complete bounded native order queue membership, not order configuration or feasibility. Use its exact witness to seal finite creation intent. Failed refresh clears old selection; no mutation occurs.")]
pub fn fortress_observe(session_id:String) -> String {
    with_session(session_id,"fortress.observe",Limits::default(),1,||Ok(Action::Observe))
}
#[tool(description="Discover complete retained creation records without native calls. State: all, pending or reconciliation_required. Limit 1..8, default 2. Whole-record pages and continuations bind session, journal, exact head, filter and limit. Empty filtered pages may still require continuation.")]
pub fn fortress_query(session_id:String,state:Option<String>,limit:Option<u32>,continuation:Option<String>,
    max_bytes:Option<u64>,max_output_tokens:Option<u32>) -> String
{
    let limit = limit.unwrap_or(2) as usize;
    with_session(session_id,"fortress.query",Limits { bytes:max_bytes,tokens:max_output_tokens,wall:None },limit,|| {
        if limit == 0 || limit > MAX_PAGE { return Err(error(ErrorCode::InvalidRequest,"creation page limit must be 1..8")); }
        Ok(Action::Query { filter:Filter::parse(state.as_deref().unwrap_or("all"))?,limit,continuation })
    })
}
#[tool(description="Seal and durably prepare finite furniture creation: wooden_bed, wooden_door, wooden_table or wooden_chair, amount 1..100. Requires exact retained queue witness, stable key and ConfigureProduction. Server constructs plan digest; no order is inserted and replay does not renew TTL.")]
pub fn fortress_plan(session_id:String,idempotency_key:String,recipe_name:String,amount:u32,expected_witness:String) -> String {
    with_session(session_id,"fortress.plan",Limits::default(),1,||Ok(Action::Plan { key:idempotency_key,
        spec:WorkOrderSpec::new(recipe(&recipe_name)?,amount)?,witness:digest(&expected_witness)? }))
}
#[tool(description="Commit one exact durable creation plan once. Requires key, server-produced digest, exact current queue witness and ConfigureProduction. Dispatch intent is synced before insertion. Created proves only immediate order insertion/template readback, not approval or goods produced. Unknown attempts require reconciliation, never retry or a new key.")]
pub fn fortress_commit(session_id:String,idempotency_key:String,plan_digest:String,expected_witness:String) -> String {
    with_session(session_id,"fortress.commit",Limits::default(),1,||Ok(Action::Commit {
        key:idempotency_key,plan:digest(&plan_digest)?,witness:digest(&expected_witness)? }))
}
#[tool(description="Perform one bounded Query-only receipt reconciliation. No polling loop, reconnect or insertion retry. Prepared/terminal records return stored evidence. Missing native records never prove non-creation. Offline unresolved work requires explicit close and reopen in reconcile mode.")]
pub fn fortress_wait(session_id:String,idempotency_key:String,plan_digest:String,max_wall_millis:Option<u64>) -> String {
    with_session(session_id,"fortress.wait",Limits { wall:max_wall_millis,..Limits::default() },1,
        ||Ok(Action::Wait { key:idempotency_key,plan:digest(&plan_digest)? }))
}
#[tool(description="Inspect one exact retained creation plan and receipt without native calls, including offline. Historical evidence is not current order state or production completion.")]
pub fn fortress_explain(session_id:String,idempotency_key:String,plan_digest:String) -> String {
    with_session(session_id,"fortress.explain",Limits::default(),1,
        ||Ok(Action::Explain { key:idempotency_key,plan:digest(&plan_digest)? }))
}
fn close(raw:String) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(&raw)?; let mut guard = session_lock()?;
        let state = guard.as_mut().filter(|s|s.id == id)
            .ok_or_else(||error(ErrorCode::SessionNotFound,"creation session absent or already closed"))?;
        let context = state.context(false,false)?;
        let out = packet("fortress.cancel",json!({"ok":true,"scope":"session","closed":true,
            "effects_cancelled":false,"journal_changed":false,"native_calls":0}),
            TurnView { context:Some(&context),mode:Some(state.mode),summary:None,selected:None });
        // Drain owned custody even with cancelled/revoked authority or a fenced log.
        // The slot remains locked until the connection and journal have dropped.
        drop(guard.take()); Ok(out)
    })();
    result.unwrap_or_else(|cause|unbound("fortress.cancel",&cause))
}
#[tool(description="With scope=effect and exact key/digest, permanently retire a still-prepared local creation without deleting a manager order. Requires production authority. With scope=session and no effect identity, release custody even after fencing; no evidence or effects are erased or cancelled.")]
pub fn fortress_cancel(session_id:String,scope:String,idempotency_key:Option<String>,plan_digest:Option<String>) -> String {
    match (scope.as_str(),idempotency_key,plan_digest) {
        ("session",None,None) => close(session_id),
        ("effect",Some(key),Some(plan)) => with_session(session_id,"fortress.cancel",Limits::default(),1,
            ||Ok(Action::Cancel { key,plan:digest(&plan)? })),
        _ => with_session(session_id,"fortress.cancel",Limits::default(),0,
            ||Err(error(ErrorCode::InvalidRequest,"cancel requires scope=effect with key/digest, or scope=session without either"))),
    }
}
#[tool(description="Unavailable in isolated work-orders/1.10; no game checkpoint is implied by the coordination journal.")]
pub fn fortress_checkpoint(session_id:String) -> String {
    with_session(session_id,"fortress.checkpoint",Limits::default(),0,||Ok(Action::Unavailable))
}
#[tool(description="Unavailable in isolated work-orders/1.10; restarting or replaying the journal never restores game state.")]
pub fn fortress_restore(session_id:String) -> String {
    with_session(session_id,"fortress.restore",Limits::default(),0,||Ok(Action::Unavailable))
}
#[tool(description="Report custody-checked journal counts and connection presence without native calls. This is not a live health check, compatibility admission or proof of completed production.")]
pub fn fortress_doctor(session_id:String) -> String {
    with_session(session_id,"fortress.doctor",Limits::default(),0,||Ok(Action::Doctor))
}

pub fn run_stdio() {
    if let Err(cause) = validate_environment() { eprintln!("{cause}"); std::process::exit(1); }
    let server = ServerBuilder::new("dfmcp-live-work-orders-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted work-orders/1.10. Offline is the default. Discover pending creation evidence before new work. Operator-enabled control can create finite wooden furniture orders only while paused; observe the complete queue, seal intent and commit once through the durable journal. Native Created proves insertion, not approval, material feasibility or goods produced. Unresolved effects block all new creation; query the retained receipt, never retry insertion or change keys. Reconcile mode has Query only. No automatic reconnect, journal repair, order deletion, checkpoint or restore exists. Session close releases custody without changing effects.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path = "live_work_orders_server_tests.rs"]
mod tests;
