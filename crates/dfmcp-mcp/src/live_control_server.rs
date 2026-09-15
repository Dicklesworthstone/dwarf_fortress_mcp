#![forbid(unsafe_code)]

//! Explicitly unadmitted pause-control runtime. This is the only live mutation
//! family: prepare/commit/reconcile for simulation pause state. Every commit is
//! durably recorded as started before bridge dispatch; ambiguous outcomes cannot
//! become retries merely because the Rust process restarts.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use dfmcp_adapter::control_effect_journal::{ControlEffectJournal, DurablePauseRecord,
    DurablePauseState, EffectTailRecovery, PrivateControlJournalFile, open_private_control_journal};
use dfmcp_adapter::live_control_rpc::{ControlRpcClient, PauseEffect};
use dfmcp_adapter::live_jobs_rpc::DeadlineStream;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode,
    OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

const FAMILY:u128=1u128<<57;
static NEXT:Mutex<u128>=Mutex::new(1);
static SLOTS:AtomicUsize=AtomicUsize::new(0);
static SESSIONS:LazyLock<Mutex<BTreeMap<SessionId,Arc<Mutex<ControlSession>>>>>=LazyLock::new(||Mutex::new(BTreeMap::new()));
fn err(code:ErrorCode,text:&str)->DfmcpError{DfmcpError::new(code,text)}
fn lock<T>(m:&Mutex<T>)->Result<MutexGuard<'_,T>>{m.lock().map_err(|_|err(ErrorCode::InternalInvariantViolation,"control mutex poisoned"))}

type Journal=ControlEffectJournal<PrivateControlJournalFile>;
struct Slot;
impl Slot{
    fn reserve()->Result<Self>{SLOTS.fetch_update(Ordering::AcqRel,Ordering::Acquire,|count|(count<1).then_some(count+1))
        .map_err(|_|err(ErrorCode::BudgetExceeded,"control/1.7 permits one retained mutation session"))?;Ok(Self)}
}
impl Drop for Slot{fn drop(&mut self){SLOTS.fetch_sub(1,Ordering::AcqRel);}}

struct ControlSession{
    id:SessionId,
    client:ControlRpcClient<DeadlineStream>,
    endpoint:SocketAddr,
    token:Vec<u8>,
    nonce:Vec<u8>,
    timeout:Duration,
    journal:Journal,
    request:u128,
    budget:WorkBudget,
    grants:Vec<CapabilityGrant>,
    _slot:Slot,
}
impl ControlSession{
    fn context(&mut self)->Result<OperationContext>{
        self.request=self.request.checked_add(1).ok_or_else(||err(ErrorCode::BudgetExceeded,"control request IDs exhausted"))?;
        Ok(OperationContext{session_id:self.id,request_id:RequestId::new(self.request),anchor:StateAnchor::default(),budget:self.budget,
            grants:self.grants.clone(),cancellation_requested:false})
    }
    fn reconnect(&mut self)->Result<()> {
        self.client=ControlRpcClient::connect(self.endpoint,self.token.clone(),self.nonce.clone(),self.timeout)?;Ok(())
    }
    fn ensure_connection(&mut self)->Result<()> {if self.client.poisoned(){self.reconnect()?;}Ok(())}
    fn query_with_reconnect(&mut self,key:&str,digest:Digest32)->Result<PauseEffect>{
        self.ensure_connection()?;
        match self.client.query_pause(key,digest){
            Ok(effect)=>Ok(effect),
            Err(first) if self.client.poisoned()=>{self.reconnect()?;self.client.query_pause(key,digest).map_err(|_|first)},
            Err(error)=>Err(error),
        }
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
        "DFMCP_CONTROL_JOURNAL is required; live pause effects refuse process-local-only coordination"))?;
    if path.is_empty(){return Err(err(ErrorCode::InvalidRequest,"DFMCP_CONTROL_JOURNAL must be nonempty UTF-8"));}
    let recovery=match std::env::var("DFMCP_CONTROL_JOURNAL_REPAIR"){
        Err(std::env::VarError::NotPresent)=>EffectTailRecovery::Refuse,
        Ok(value) if value=="1"=>EffectTailRecovery::TruncateIncomplete,
        _=>return Err(err(ErrorCode::InvalidRequest,"DFMCP_CONTROL_JOURNAL_REPAIR must be absent or exactly 1")),
    };
    Ok((PathBuf::from(path),recovery))
}
fn packet(operation:&str,value:Value)->String{json!({"agent_turn":{"operation":operation,"phase":"act",
    "briefing":{"runtime":"unadmitted_development","bridge_protocol":"1.7","runtime_admitted":false,
        "mutation_admissible":false,"development_mutation_enabled":true,"supported_effects":["pause"]},
    "coverage":{"status":"partial","complete_domains":["pause_effect_protocol","durable_pause_effect_coordination"],
        "omitted_domains":["dig","building","labor","burrow","stockpile","work_order","military","checkpoint"]}},"result":value}).to_string()}
fn failure(operation:&str,error:&DfmcpError)->String{let indeterminate=error.code==ErrorCode::EffectIndeterminate;
    packet(operation,json!({"ok":false,"error":{"code":error.code.as_str(),"message":error.message,
        "mutation_dispatched":if indeterminate{Value::Null}else{json!(false)},"reconciliation_required":indeterminate}}))}
fn with_session<F>(id:Option<String>,operation:&str,body:F)->String where F:FnOnce(&mut ControlSession,OperationContext)->Result<Value>{
    let handle=match resolve(id){Ok(value)=>value,Err(error)=>return failure(operation,&error)};
    let mut session=match lock(&handle){Ok(value)=>value,Err(error)=>return failure(operation,&error)};
    let context=match session.context(){Ok(value)=>value,Err(error)=>return failure(operation,&error)};
    match body(&mut session,context){Ok(value)=>packet(operation,value),Err(error)=>failure(operation,&error)}}
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
fn journal_json(journal:&Journal)->Value{json!({"journal_id":journal.id().to_string(),"head":journal.head().to_string(),
    "effects":journal.effect_count(),"transitions":journal.transition_count(),"retained_bytes":journal.retained_bytes(),
    "repaired_tail_bytes":journal.repaired_tail_bytes(),"fenced":journal.fenced(),"restart_recovery":true})}
fn same_identity(record:&DurablePauseRecord,digest:Digest32,paused:bool,tick:u64)->Result<()>{
    if record.plan_digest!=digest||record.desired_paused!=paused||record.expected_game_tick!=tick{
        return Err(err(ErrorCode::Conflict,"idempotency key already names different durable pause-effect content"));}Ok(())}
fn record_effect(journal:&mut Journal,key:&str,digest:Digest32,effect:&PauseEffect,context:&OperationContext)->Result<DurablePauseRecord>{
    journal.record_reconciliation(key,digest,effect.bridge_generation,effect.known,effect.applied,effect.paused,effect.observed_tick,receipt(effect)?,context)
}

#[tool(description="Open an explicitly unadmitted pause-control/1.7 development session. A private durable control journal is mandatory. Only pause/resume prepare/commit/reconcile are supported.")]
pub fn fortress_open_session(max_wall_millis:Option<u64>)->String{
    let result=(||->Result<String>{validate_environment()?;let (journal_path,recovery)=journal_configuration()?;let id=next_id()?;let slot=Slot::reserve()?;
        let millis=max_wall_millis.unwrap_or(5000);if !(1..=60_000).contains(&millis){return Err(err(ErrorCode::BudgetExceeded,"control wall-time must be 1..60000 milliseconds"));}
        let timeout=Duration::from_millis(millis);
        let endpoint=dfmcp_adapter::parse_loopback_endpoint(&std::env::var("DFMCP_CONTROL_ENDPOINT").unwrap_or_else(|_|"127.0.0.1:5000".to_owned()))?;
        let token=std::env::var("DFMCP_CONTROL_TOKEN").map_err(|_|err(ErrorCode::CapabilityDenied,"DFMCP_CONTROL_TOKEN required"))?.into_bytes();
        let nonce=id.get().to_be_bytes().to_vec();
        let client=ControlRpcClient::connect(endpoint,token.clone(),nonce.clone(),timeout)?;
        let grants=vec![CapabilityGrant{capability:Capability::ControlClock,scope:CapabilityScope::default(),max_risk:RiskTier::Reversible,expires_at_tick:None,remaining_uses:None}];
        let budget=WorkBudget{max_wall_millis:millis,max_actions:1,..WorkBudget::CONSERVATIVE_DEFAULT};budget.validate()?;
        let context=OperationContext{session_id:id,request_id:RequestId::new(1),anchor:StateAnchor::default(),budget,grants:grants.clone(),cancellation_requested:false};
        let journal=open_private_control_journal(&journal_path,&context,client.bridge_generation(),recovery)?;
        let summary=journal_json(&journal);
        let session=ControlSession{id,client,endpoint,token,nonce,timeout,journal,request:1,budget,grants,_slot:slot};
        lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));
        Ok(packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),"supported_actions":["pause"],
            "runtime_admitted":false,"durable_effect_journal":summary})))})();
    result.unwrap_or_else(|error|failure("fortress.open_session",&error))}

#[tool(description="Prepare one pause/resume effect. Requires stable idempotency_key, plan_digest, desired paused state and expected game tick. Prepare does not mutate; its receipt is synced to the required control journal before success is returned.")]
pub fn fortress_plan(session_id:Option<String>,idempotency_key:String,plan_digest:String,paused:bool,expected_game_tick:u64)->String{
    with_session(session_id,"fortress.plan",|session,context|{
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;let plan=digest(&plan_digest)?;
        if let Some(existing)=session.journal.lookup(&idempotency_key).cloned(){same_identity(&existing,plan,paused,expected_game_tick)?;
            return Ok(json!({"ok":true,"existing":true,"effect":record_json(&existing),"durable_effect_journal":journal_json(&session.journal)}));}
        session.ensure_connection()?;
        let effect=session.client.prepare_pause(&idempotency_key,plan,paused,expected_game_tick)?;
        if effect.known{return Err(err(ErrorCode::Conflict,"bridge already knows this idempotency key but the durable journal does not; choose a new key"));}
        let record=session.journal.record_prepared(idempotency_key,plan,paused,expected_game_tick,effect.bridge_generation,effect_token(&effect)?,&context)?;
        Ok(json!({"ok":true,"prepared":true,"effect":record_json(&record),"durable_effect_journal":journal_json(&session.journal)}))
    })}

#[tool(description="Commit a durably prepared pause/resume effect. The coordinator syncs commit_started before exactly one bridge dispatch and syncs reconciled evidence before acknowledging success. Any ambiguous result requires fortress.explain; same-effect retry is refused.")]
pub fn fortress_commit(session_id:Option<String>,idempotency_key:String,plan_digest:String,prepare_token_hex:String)->String{
    with_session(session_id,"fortress.commit",|session,context|{
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;let plan=digest(&plan_digest)?;let token=prepare_token(&prepare_token_hex)?;
        let current=session.journal.lookup(&idempotency_key).cloned().ok_or_else(||err(ErrorCode::InvalidRequest,"effect must be durably prepared before commit"))?;
        if current.plan_digest!=plan||current.prepare_token!=token{return Err(err(ErrorCode::Conflict,"commit does not match the durable prepared effect"));}
        if current.state.terminal(){return Ok(json!({"ok":current.effect_applied,"replayed_terminal":true,"effect":record_json(&current),"durable_effect_journal":journal_json(&session.journal)}));}
        if current.state.reconciliation_required(){return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,"durable journal contains an unresolved commit attempt; reconcile before any retry"));}
        if session.client.poisoned(){return Err(err(ErrorCode::AdapterUnavailable,"control source is fenced before dispatch; reconnect by reopening the session"));}
        let generation=session.client.bridge_generation();
        if !current.safe_to_dispatch(generation){return Err(DfmcpError::new(ErrorCode::EffectIndeterminate,"prepared effect belongs to another bridge generation; replan with a new idempotency key"));}
        // Durability boundary: if this fsync fails, no bridge mutation is attempted.
        session.journal.begin_commit(&idempotency_key,plan,generation,&context)?;
        match session.client.commit_pause(&idempotency_key,plan,&token){
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

#[tool(description="Reconcile one durable pause/resume idempotency key. Read-only reconciliation may reconnect once; it never dispatches an effect. Bridge-generation loss or unknown state remains indeterminate and is never reported safe to retry.")]
pub fn fortress_explain(session_id:Option<String>,idempotency_key:String,plan_digest:String)->String{
    with_session(session_id,"fortress.explain",|session,context|{
        context.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;let plan=digest(&plan_digest)?;
        let current=session.journal.lookup(&idempotency_key).cloned().ok_or_else(||err(ErrorCode::InvalidRequest,"effect is not present in the durable control journal"))?;
        if current.plan_digest!=plan{return Err(err(ErrorCode::Conflict,"idempotency key belongs to another plan digest"));}
        if current.state.terminal(){return Ok(json!({"ok":true,"effect":record_json(&current),"commit_permitted":false,
            "new_plan_required":!current.effect_applied,"durable_effect_journal":journal_json(&session.journal)}));}
        if current.state==DurablePauseState::Prepared{
            session.ensure_connection()?;let same_generation=session.client.bridge_generation()==current.bridge_generation;
            return Ok(json!({"ok":true,"effect":record_json(&current),"commit_permitted":same_generation,
                "new_plan_required":!same_generation,"safe_to_retry_same_effect":false,"durable_effect_journal":journal_json(&session.journal)}));
        }
        let effect=session.query_with_reconnect(&idempotency_key,plan)?;
        let record=record_effect(&mut session.journal,&idempotency_key,plan,&effect,&context)?;
        Ok(json!({"ok":true,"effect":record_json(&record),"commit_permitted":false,
            "new_plan_required":record.state==DurablePauseState::VerifiedNotApplied||record.state==DurablePauseState::Indeterminate,
            "safe_to_retry_same_effect":false,"durable_effect_journal":journal_json(&session.journal)}))
    })}

fn denied(id:Option<String>,operation:&str)->String{with_session(id,operation,|_,_|Err(err(ErrorCode::CapabilityDenied,"control/1.7 supports only durable pause prepare/commit/reconcile")))}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_observe(session_id:Option<String>)->String{denied(session_id,"fortress.observe")}
#[tool(description="Unavailable in control/1.7. Durable effect status is returned by fortress.plan/commit/explain/doctor.")] pub fn fortress_query(session_id:Option<String>)->String{denied(session_id,"fortress.query")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_wait(session_id:Option<String>)->String{denied(session_id,"fortress.wait")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_cancel(session_id:Option<String>)->String{denied(session_id,"fortress.cancel")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_checkpoint(session_id:Option<String>)->String{denied(session_id,"fortress.checkpoint")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_restore(session_id:Option<String>)->String{denied(session_id,"fortress.restore")}
#[tool(description="Report control/1.7 source and durable effect-journal health. This is diagnosis, never admission authority.")]
pub fn fortress_doctor(session_id:Option<String>)->String{with_session(session_id,"fortress.doctor",|session,_|Ok(json!({"ok":true,
    "source_fenced":session.client.poisoned(),"bridge_generation":session.client.bridge_generation(),"runtime_admitted":false,
    "supported_effects":["pause"],"durable_effect_journal":journal_json(&session.journal)})))}

pub fn run_stdio(){if let Err(error)=validate_environment(){eprintln!("{error}");std::process::exit(1);}let server=ServerBuilder::new("dfmcp-live-control-dev",env!("CARGO_PKG_VERSION"))
    .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
    .instructions("Explicitly unadmitted control/1.7. A private durable effect journal is mandatory. Only pause/resume prepare, exactly-once dispatch attempt, and read-only reconciliation are supported. Never retry an indeterminate effect; reconcile it or replan under a new idempotency key. No other live mutation family exists.").build();crate::run_modern_stdio(server);}
