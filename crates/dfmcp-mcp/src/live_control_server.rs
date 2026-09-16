#![forbid(unsafe_code)]

//! Explicitly unadmitted pause-control runtime. Every commit is durably recorded
//! as started before bridge dispatch. Recovery-only sessions retain no connection
//! or mutation grant, and expose durable evidence without pretending it is live state.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dfmcp_adapter::control_effect_journal::{ControlEffectJournal, DurablePauseRecord,
    DurablePauseState, EffectJournalStorage, EffectTailRecovery, PrivateControlJournalFile,
    open_private_control_journal, open_private_control_recovery};
use dfmcp_adapter::live_control_rpc::{ControlDeadlineStream, ControlRpcClient, PauseEffect};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode,
    FortressId, GameTick, ObservationCursor, OperationContext, RequestId, Result, RiskTier,
    SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

#[path = "control_effect_queries.rs"]
mod effect_queries;

const FAMILY:u128=1u128<<57;
static NEXT:Mutex<u128>=Mutex::new(1);
static SLOTS:AtomicUsize=AtomicUsize::new(0);
static SESSIONS:LazyLock<Mutex<BTreeMap<SessionId,Arc<Mutex<ControlSession>>>>>=LazyLock::new(||Mutex::new(BTreeMap::new()));
fn err(code:ErrorCode,text:&str)->DfmcpError{DfmcpError::new(code,text)}
fn lock<T>(m:&Mutex<T>)->Result<MutexGuard<'_,T>>{m.lock().map_err(|_|err(ErrorCode::InternalInvariantViolation,"control mutex poisoned"))}
fn coordinator_anchor()->StateAnchor{StateAnchor{fortress_id:FortressId::NIL,cursor:ObservationCursor::ORIGIN,
    tick:GameTick(0),state_hash:Digest32::ZERO}}

type Journal=ControlEffectJournal<PrivateControlJournalFile>;
struct Slot;
impl Slot{
    fn reserve()->Result<Self>{SLOTS.fetch_update(Ordering::AcqRel,Ordering::Acquire,|count|(count<1).then_some(count+1))
        .map_err(|_|err(ErrorCode::BudgetExceeded,"control/1.7 permits one retained control or recovery session"))?;Ok(Self)}
}
impl Drop for Slot{fn drop(&mut self){SLOTS.fetch_sub(1,Ordering::AcqRel);}}

// Credentials and connection/reconnection logic exist only in live sessions.
struct ControlConnection {
    client:ControlRpcClient<ControlDeadlineStream>,
    endpoint:SocketAddr,
    token:Vec<u8>,
    nonce:Vec<u8>,
    timeout:Duration,
}
impl ControlConnection {
    fn reconnect(&mut self)->Result<()> {
        self.client=ControlRpcClient::connect(self.endpoint,self.token.clone(),self.nonce.clone(),self.timeout)?;Ok(())
    }
    fn arm(&mut self)->Result<()> {
        if self.client.poisoned(){self.reconnect()?;}
        self.client.reset_deadline(self.timeout)
    }
    fn query_with_reconnect(&mut self,key:&str,digest:Digest32)->Result<PauseEffect>{
        self.arm()?;
        match self.client.query_pause(key,digest){
            Ok(effect)=>Ok(effect),
            Err(first) if self.client.poisoned()=>{
                self.reconnect()?;self.client.reset_deadline(self.timeout)?;
                self.client.query_pause(key,digest).map_err(|_|first)
            }
            Err(error)=>Err(error),
        }
    }
}

struct ControlSession{
    id:SessionId,
    connection:Option<ControlConnection>,
    journal:Journal,
    request:u128,
    budget:WorkBudget,
    grants:Vec<CapabilityGrant>,
    _slot:Slot,
}
impl ControlSession{
    fn context(&mut self)->Result<OperationContext>{
        self.request=self.request.checked_add(1).ok_or_else(||err(ErrorCode::BudgetExceeded,"control request IDs exhausted"))?;
        Ok(OperationContext{session_id:self.id,request_id:RequestId::new(self.request),anchor:coordinator_anchor(),budget:self.budget,
            grants:self.grants.clone(),cancellation_requested:false})
    }
    fn live(&mut self)->Result<&mut ControlConnection> {
        if self.journal.read_only() {
            return Err(err(ErrorCode::CapabilityDenied,"recovery-only sessions cannot contact the bridge or dispatch effects"));
        }
        self.connection.as_mut().ok_or_else(||err(ErrorCode::CapabilityDenied,"no live control connection"))
    }
}
fn next_id()->Result<SessionId>{let mut n=lock(&NEXT)?;if *n>=FAMILY{return Err(err(ErrorCode::BudgetExceeded,"control session IDs exhausted"));}
    let id=SessionId::new((1u128<<127)|FAMILY|*n);*n+=1;Ok(id)}
fn resolve(raw:Option<String>)->Result<Arc<Mutex<ControlSession>>>{let raw=raw.ok_or_else(||err(ErrorCode::InvalidRequest,"open control session first"))?;
    if raw.len()!=32||!raw.bytes().all(|b|b.is_ascii_hexdigit()){return Err(err(ErrorCode::InvalidRequest,"invalid control session"));}
    let value=u128::from_str_radix(&raw,16).map_err(|_|err(ErrorCode::InvalidRequest,"invalid control session"))?;let id=SessionId::new(value);
    if id.get()!=value||!id.is_process_scoped_live()||(value&((1u128<<62)-1))>>57!=1{return Err(err(ErrorCode::InvalidRequest,"not a control session"));}
    lock(&SESSIONS)?.get(&id).cloned().ok_or_else(||err(ErrorCode::SessionNotFound,"control session not found"))}
fn validate_environment()->Result<()>{
    let allowed=["DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7","DFMCP_CONTROL_TOKEN","DFMCP_CONTROL_ENDPOINT",
        "DFMCP_CONTROL_JOURNAL","DFMCP_CONTROL_JOURNAL_REPAIR"];
    if std::env::var("DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7").ok().as_deref()!=Some("1")
        ||std::env::vars_os().any(|(key,_)|{let key=key.to_string_lossy();key.starts_with("DFMCP_")&&!allowed.contains(&key.as_ref())})
        ||crate::admission::current_admission_provenance().is_some(){return Err(err(ErrorCode::CapabilityDenied,
            "control/1.7 requires exact development opt-in and refuses admission/other DFMCP state"));}Ok(())}
fn journal_configuration()->Result<(PathBuf,EffectTailRecovery)>{
    let path=std::env::var("DFMCP_CONTROL_JOURNAL").map_err(|_|err(ErrorCode::CapabilityDenied,
        "DFMCP_CONTROL_JOURNAL is required; pause coordination and recovery require durable evidence"))?;
    if path.is_empty(){return Err(err(ErrorCode::InvalidRequest,"DFMCP_CONTROL_JOURNAL must be nonempty UTF-8"));}
    let recovery=match std::env::var("DFMCP_CONTROL_JOURNAL_REPAIR"){
        Err(std::env::VarError::NotPresent)=>EffectTailRecovery::Refuse,
        Ok(value) if value=="1"=>EffectTailRecovery::TruncateIncomplete,
        _=>return Err(err(ErrorCode::InvalidRequest,"DFMCP_CONTROL_JOURNAL_REPAIR must be absent or exactly 1")),
    };
    Ok((PathBuf::from(path),recovery))
}

fn packet(operation:&str,value:Value,recovery_only:bool)->String{json!({"agent_turn":{"operation":operation,
    "phase":if matches!(operation,"fortress.plan"|"fortress.commit"){"act"}else{"sense"},
    "briefing":{"runtime":"unadmitted_development","bridge_protocol":"1.7","runtime_admitted":false,
        "mutation_admissible":false,"development_mutation_enabled":!recovery_only,
        "supported_effects":if recovery_only{json!([])}else{json!(["pause"])}},
    "coverage":{"status":"partial","complete_domains":["durable_pause_effect_coordination"],
        "omitted_domains":["live_world_state","dig","building","labor","burrow","stockpile","work_order","military","checkpoint"]},
    "uncertainty":[{"code":"coordinator_evidence_not_live_state",
        "detail":"Durable effect evidence does not establish the current game state."}]},"result":value}).to_string()}
fn failure(operation:&str,error:&DfmcpError,recovery_only:bool)->String{let indeterminate=error.code==ErrorCode::EffectIndeterminate;
    packet(operation,json!({"ok":false,"error":{"code":error.code.as_str(),"message":error.message,
        "mutation_dispatched":if indeterminate{Value::Null}else{json!(false)},"reconciliation_required":indeterminate}}),recovery_only)}
fn with_session<F>(id:Option<String>,operation:&str,body:F)->String where F:FnOnce(&mut ControlSession,OperationContext)->Result<Value>{
    let handle=match resolve(id){Ok(value)=>value,Err(error)=>return failure(operation,&error,true)};
    let mut session=match lock(&handle){Ok(value)=>value,Err(error)=>return failure(operation,&error,true)};
    let recovery_only=session.journal.read_only();
    let context=match session.context(){Ok(value)=>value,Err(error)=>return failure(operation,&error,recovery_only)};
    // Recheck authority and custody even for idempotent/terminal lookups.
    if let Err(error)=session.journal.records(&context).map(|_|()) {
        return failure(operation,&error,recovery_only);
    }
    match body(&mut session,context){Ok(value)=>packet(operation,value,recovery_only),Err(error)=>failure(operation,&error,recovery_only)}}
fn digest(raw:&str)->Result<Digest32>{if raw.len()!=64||!raw.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)){
        return Err(err(ErrorCode::InvalidRequest,"plan_digest must be canonical lowercase SHA-256 hex"));}
    let mut out=[0u8;32];for i in 0..32{out[i]=u8::from_str_radix(&raw[i*2..i*2+2],16).map_err(|_|err(ErrorCode::InvalidRequest,"invalid plan digest"))?;}Ok(Digest32::from_bytes(out))}
fn prepare_token(raw:&str)->Result<[u8;16]>{if raw.len()!=32||!raw.bytes().all(|byte|byte.is_ascii_digit()||(b'a'..=b'f').contains(&byte)){
        return Err(err(ErrorCode::InvalidRequest,"prepare_token_hex must be canonical lowercase 16-byte hex"));}
    let mut out=[0u8;16];for i in 0..16{out[i]=u8::from_str_radix(&raw[i*2..i*2+2],16).map_err(|_|err(ErrorCode::InvalidRequest,"invalid prepare token"))?;}Ok(out)}
fn effect_token(effect:&PauseEffect)->Result<[u8;16]>{effect.prepare_token.as_slice().try_into().map_err(|_|err(ErrorCode::AdapterRejected,"control bridge returned an invalid prepare token length"))}
fn receipt(effect:&PauseEffect)->Result<Option<Digest32>>{
    if effect.receipt_digest.is_empty(){return Ok(None);}
    let bytes:[u8;32]=effect.receipt_digest.as_slice().try_into().map_err(|_|err(ErrorCode::AdapterRejected,"control bridge returned a non-SHA-256 receipt"))?;
    Ok(Some(Digest32::from_bytes(bytes)))
}
fn state_name(state:DurablePauseState)->&'static str{match state{DurablePauseState::Prepared=>"prepared",DurablePauseState::CommitStarted=>"commit_started",
    DurablePauseState::VerifiedApplied=>"verified_applied",DurablePauseState::VerifiedNotApplied=>"verified_not_applied",DurablePauseState::Indeterminate=>"indeterminate"}}
fn record_json(record:&DurablePauseRecord)->Value{json!({"idempotency_key":record.idempotency_key,"plan_digest":record.plan_digest.to_string(),
    "desired_paused":record.desired_paused,"expected_game_tick":record.expected_game_tick,"bridge_generation":record.bridge_generation,
    "prepare_token_hex":record.prepare_token.iter().map(|byte|format!("{byte:02x}")).collect::<String>(),"state":state_name(record.state),
    "effect_known":record.effect_known,"effect_applied":record.effect_applied,"observed_paused":record.observed_paused,
    "observed_game_tick":record.observed_game_tick,"receipt_digest":record.receipt_digest.map(|value|value.to_string()),
    "revision":record.revision,"transition_number":record.transition_number,"record_digest":record.record_digest.to_string(),
    "reconciliation_required":record.state.reconciliation_required(),"safe_to_retry_same_effect":false})}
fn journal_json<S:EffectJournalStorage>(journal:&ControlEffectJournal<S>)->Value{json!({"journal_id":journal.id().to_string(),"head":journal.head().to_string(),
    "effects":journal.effect_count(),"transitions":journal.transition_count(),"retained_bytes":journal.retained_bytes(),
    "repaired_tail_bytes":journal.repaired_tail_bytes(),"fenced":journal.fenced(),"restart_recovery":true,"read_only":journal.read_only()})}
fn same_identity(record:&DurablePauseRecord,digest:Digest32,paused:bool,tick:u64)->Result<()>{
    if record.plan_digest!=digest||record.desired_paused!=paused||record.expected_game_tick!=tick{
        return Err(err(ErrorCode::Conflict,"idempotency key already names different durable pause-effect content"));}Ok(())}
fn record_effect(journal:&mut Journal,key:&str,digest:Digest32,effect:&PauseEffect,context:&OperationContext)->Result<DurablePauseRecord>{
    journal.record_reconciliation(key,digest,effect.bridge_generation,effect.known,effect.applied,effect.paused,effect.observed_tick,receipt(effect)?,context)
}

fn configured_session(id:SessionId,path:&Path,recovery:EffectTailRecovery,budget:WorkBudget,
    recovery_only:bool,slot:Slot)->Result<ControlSession> {
    budget.validate()?;
    if recovery_only && recovery!=EffectTailRecovery::Refuse {
        return Err(err(ErrorCode::CapabilityDenied,"recovery-only sessions refuse journal repair; unset DFMCP_CONTROL_JOURNAL_REPAIR"));
    }
    let (capability,max_risk)=if recovery_only{(Capability::Query,RiskTier::ReadOnly)}else{(Capability::ControlClock,RiskTier::Reversible)};
    let grants=vec![CapabilityGrant{capability,scope:CapabilityScope::default(),max_risk,expires_at_tick:None,remaining_uses:None}];
    let context=OperationContext{session_id:id,request_id:RequestId::new(1),anchor:coordinator_anchor(),budget,grants:grants.clone(),cancellation_requested:false};
    // The recovery branch precedes all credential/endpoint reads and connection construction.
    let (connection,journal)=if recovery_only {
        (None,open_private_control_recovery(path,&context)?)
    }else{
        let timeout=Duration::from_millis(budget.max_wall_millis);
        let endpoint=dfmcp_adapter::parse_loopback_endpoint(&std::env::var("DFMCP_CONTROL_ENDPOINT").unwrap_or_else(|_|"127.0.0.1:5000".to_owned()))?;
        let token=std::env::var("DFMCP_CONTROL_TOKEN").map_err(|_|err(ErrorCode::CapabilityDenied,"DFMCP_CONTROL_TOKEN required"))?.into_bytes();
        let nonce=id.get().to_be_bytes().to_vec();
        let client=ControlRpcClient::connect(endpoint,token.clone(),nonce.clone(),timeout)?;
        let journal=open_private_control_journal(path,&context,client.bridge_generation(),recovery)?;
        (Some(ControlConnection{client,endpoint,token,nonce,timeout}),journal)
    };
    Ok(ControlSession{id,connection,journal,request:1,budget,grants,_slot:slot})
}

#[tool(description="Open an explicitly unadmitted pause-control/1.7 session. Set recovery_only=true to inspect an existing durable journal with Query authority, no bridge credentials/connection, no repair and no mutations. The default live mode retains pause/resume prepare/commit/reconcile.")]
pub fn fortress_open_session(max_wall_millis:Option<u64>,recovery_only:Option<bool>)->String{
    let recovery_only=recovery_only.unwrap_or(false);
    let result=(||->Result<String>{validate_environment()?;let (path,recovery)=journal_configuration()?;let id=next_id()?;let slot=Slot::reserve()?;
        let millis=max_wall_millis.unwrap_or(5000);if !(1..=60_000).contains(&millis){return Err(err(ErrorCode::BudgetExceeded,"control wall-time must be 1..60000 milliseconds"));}
        let budget=WorkBudget{max_wall_millis:millis,max_actions:1,..WorkBudget::CONSERVATIVE_DEFAULT};
        let session=configured_session(id,&path,recovery,budget,recovery_only,slot)?;
        let summary=journal_json(&session.journal);
        lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));
        Ok(packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),
            "supported_actions":if recovery_only{json!([])}else{json!(["pause"])},
            "recovery_only":recovery_only,"bridge_connection_present":!recovery_only,
            "current_freshness_proven":false,"runtime_admitted":false,"durable_effect_journal":summary,
            "effect_discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"state":"all","limit":8}}}),recovery_only))})();
    result.unwrap_or_else(|error|failure("fortress.open_session",&error,true))}

#[tool(description="List durable pause effects without contacting the bridge or changing the journal. State: all (default), nonterminal, reconciliation_required, prepared, commit_started, indeterminate, verified_applied, verified_not_applied. Limit 1..128. Continuations bind session, journal head and filter. Output budgets only narrow session limits; token accounting estimates UTF-8 bytes/4 including the Agent Turn.")]
pub fn fortress_query(session_id:Option<String>,state:Option<String>,limit:Option<u32>,continuation:Option<String>,
    max_bytes:Option<u64>,max_output_tokens:Option<u32>)->String {
    with_session(session_id,"fortress.query",|session,context|{
        effect_queries::effects(&mut session.journal,&context,effect_queries::EffectQuery{
            state:state.as_deref().unwrap_or("all"),limit:limit.unwrap_or(8),continuation:continuation.as_deref(),
            max_bytes,max_output_tokens})
    })
}

#[tool(description="Prepare one pause/resume effect. Requires stable idempotency_key, plan_digest, desired paused state and expected game tick. Prepare does not mutate; its receipt is synced before success. Recovery-only sessions refuse this tool.")]
pub fn fortress_plan(session_id:Option<String>,idempotency_key:String,plan_digest:String,paused:bool,expected_game_tick:u64)->String{
    with_session(session_id,"fortress.plan",|session,context|{
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
        session.live()?;
        let plan=digest(&plan_digest)?;
        if let Some(existing)=session.journal.lookup(&idempotency_key).cloned(){same_identity(&existing,plan,paused,expected_game_tick)?;
            return Ok(json!({"ok":true,"existing":true,"effect":record_json(&existing),"durable_effect_journal":journal_json(&session.journal)}));}
        let effect={let connection=session.live()?;connection.arm()?;
            connection.client.prepare_pause(&idempotency_key,plan,paused,expected_game_tick)?};
        if effect.known{return Err(err(ErrorCode::Conflict,"bridge already knows this idempotency key but the durable journal does not; choose a new key"));}
        let record=session.journal.record_prepared(idempotency_key,plan,paused,expected_game_tick,effect.bridge_generation,effect_token(&effect)?,&context)?;
        Ok(json!({"ok":true,"prepared":true,"effect":record_json(&record),"durable_effect_journal":journal_json(&session.journal)}))
    })}

#[tool(description="Commit a durably prepared pause/resume effect. Sync commit_started before exactly one bridge dispatch and reconciled evidence before acknowledging success. Any ambiguous result requires fortress.explain; same-effect retry and recovery-only commits are refused.")]
pub fn fortress_commit(session_id:Option<String>,idempotency_key:String,plan_digest:String,prepare_token_hex:String)->String{
    with_session(session_id,"fortress.commit",|session,context|{
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
        session.live()?;
        let plan=digest(&plan_digest)?;let token=prepare_token(&prepare_token_hex)?;
        let current=session.journal.lookup(&idempotency_key).cloned().ok_or_else(||err(ErrorCode::InvalidRequest,"effect must be durably prepared before commit"))?;
        if current.plan_digest!=plan||current.prepare_token!=token{return Err(err(ErrorCode::Conflict,"commit does not match the durable prepared effect"));}
        if current.state.terminal(){return Ok(json!({"ok":current.effect_applied,"replayed_terminal":true,"effect":record_json(&current),"durable_effect_journal":journal_json(&session.journal)}));}
        if current.state.reconciliation_required(){return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,"durable journal contains an unresolved commit attempt; reconcile before any retry"));}
        let generation={let connection=session.live()?;
            if connection.client.poisoned(){return Err(err(ErrorCode::AdapterUnavailable,"control source is fenced before dispatch; reconcile/reopen rather than retrying this commit"));}
            connection.client.reset_deadline(connection.timeout)?;
            connection.client.bridge_generation()};
        if !current.safe_to_dispatch(generation){return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,"prepared effect belongs to another bridge generation; replan with a new idempotency key"));}
        // Durability boundary: if this fsync fails, no bridge mutation is attempted.
        session.journal.begin_commit(&idempotency_key,plan,generation,&context)?;
        let response=match session.live(){Ok(connection)=>connection.client.commit_pause(&idempotency_key,plan,&token),Err(error)=>Err(error)};
        match response{
            Ok(effect)=>match record_effect(&mut session.journal,&idempotency_key,plan,&effect,&context){
                Ok(record)=>Ok(json!({"ok":record.effect_applied,"state":state_name(record.state),"effect":record_json(&record),
                    "durable_effect_journal":journal_json(&session.journal)})),
                Err(_)=>Err(DfmcpError::new(ErrorCode::EffectIndeterminate,
                    "pause result returned but terminal journal evidence could not be durably acknowledged; reconcile after reopening")),
            },
            Err(_)=>{
                let _=session.journal.mark_indeterminate(&idempotency_key,plan,&context);
                Err(DfmcpError::new(ErrorCode::EffectIndeterminate,"pause commit outcome is ambiguous; durable state requires reconciliation before any retry"))
            }
        }
    })}

#[tool(description="Explain one durable effect. Live sessions may query the bridge and durably reconcile, never dispatching a mutation. Recovery-only sessions return existing evidence without connecting or changing state; unresolved effects remain unresolved.")]
pub fn fortress_explain(session_id:Option<String>,idempotency_key:String,plan_digest:String)->String{
    with_session(session_id,"fortress.explain",|session,context|{
        let plan=digest(&plan_digest)?;
        let current=session.journal.lookup(&idempotency_key).cloned().ok_or_else(||err(ErrorCode::InvalidRequest,"effect is not present in the durable control journal"))?;
        if current.plan_digest!=plan{return Err(err(ErrorCode::Conflict,"idempotency key belongs to another plan digest"));}
        if session.journal.read_only(){
            context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
            return Ok(json!({"ok":true,"effect":record_json(&current),"recovery_only":true,
                "commit_permitted":false,"reconciliation_performed":false,"current_freshness_proven":false,
                "live_reconciliation_required":current.state.reconciliation_required(),
                "safe_to_retry_same_effect":false,"durable_effect_journal":journal_json(&session.journal)}));
        }
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;
        if current.state.terminal(){return Ok(json!({"ok":true,"effect":record_json(&current),"commit_permitted":false,
            "new_plan_required":!current.effect_applied,"durable_effect_journal":journal_json(&session.journal)}));}
        if current.state==DurablePauseState::Prepared{
            let same_generation={let connection=session.live()?;connection.arm()?;
                connection.client.bridge_generation()==current.bridge_generation};
            return Ok(json!({"ok":true,"effect":record_json(&current),"commit_permitted":same_generation,
                "new_plan_required":!same_generation,"safe_to_retry_same_effect":false,"durable_effect_journal":journal_json(&session.journal)}));
        }
        let effect=session.live()?.query_with_reconnect(&idempotency_key,plan)?;
        let record=record_effect(&mut session.journal,&idempotency_key,plan,&effect,&context)?;
        Ok(json!({"ok":true,"effect":record_json(&record),"commit_permitted":false,
            "new_plan_required":record.state==DurablePauseState::VerifiedNotApplied||record.state==DurablePauseState::Indeterminate,
            "safe_to_retry_same_effect":false,"durable_effect_journal":journal_json(&session.journal)}))
    })}

fn denied(id:Option<String>,operation:&str)->String{with_session(id,operation,|_,_|Err(err(ErrorCode::CapabilityDenied,"control/1.7 supports durable pause control and evidence discovery, not this operation")))}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_observe(session_id:Option<String>)->String{denied(session_id,"fortress.observe")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_wait(session_id:Option<String>)->String{denied(session_id,"fortress.wait")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_cancel(session_id:Option<String>)->String{denied(session_id,"fortress.cancel")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_checkpoint(session_id:Option<String>)->String{denied(session_id,"fortress.checkpoint")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_restore(session_id:Option<String>)->String{denied(session_id,"fortress.restore")}
#[tool(description="Report control/1.7 connection presence and custody-checked durable journal health without contacting the bridge. Recovery-only mode has no connection. Diagnosis is not admission authority or a current game observation.")]
pub fn fortress_doctor(session_id:Option<String>)->String{with_session(session_id,"fortress.doctor",|session,_|Ok(json!({"ok":true,
    "source_fenced":session.connection.as_ref().map(|c|c.client.poisoned()),
    "bridge_generation":session.connection.as_ref().map(|c|c.client.bridge_generation()),
    "bridge_connection_present":session.connection.is_some(),"recovery_only":session.journal.read_only(),
    "runtime_admitted":false,"current_freshness_proven":false,
    "supported_effects":if session.journal.read_only(){json!([])}else{json!(["pause"])},
    "durable_effect_journal":journal_json(&session.journal)})))}

pub fn run_stdio(){if let Err(error)=validate_environment(){eprintln!("{error}");std::process::exit(1);}let server=ServerBuilder::new("dfmcp-live-control-dev",env!("CARGO_PKG_VERSION"))
    .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
    .instructions("Explicitly unadmitted control/1.7. A private durable journal is mandatory. Open recovery_only=true to discover journaled effects without DFHack, credentials, repair or mutation authority. fortress.query lists bounded durable records, not current game facts. Live mode supports only pause/resume prepare, one dispatch attempt and reconciliation. Never retry an indeterminate effect. No other live mutation family exists.").build();crate::run_modern_stdio(server);}

#[cfg(all(test, unix))]
#[path = "live_control_recovery_tests.rs"]
mod recovery_tests;
