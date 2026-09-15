#![forbid(unsafe_code)]

//! Explicitly unadmitted pause-control runtime. This is the only live mutation
//! family: prepare/commit/reconcile for simulation pause state. No other effect
//! can be encoded or dispatched through this module.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::time::Duration;
use dfmcp_adapter::live_control_rpc::{ControlRpcClient, PauseEffect};
use dfmcp_adapter::live_jobs_rpc::DeadlineStream;
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, DfmcpError, Digest32, ErrorCode,
    OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

const FAMILY:u128=1u128<<58;
static NEXT:Mutex<u128>=Mutex::new(1);
static SESSIONS:LazyLock<Mutex<BTreeMap<SessionId,Arc<Mutex<ControlSession>>>>>=LazyLock::new(||Mutex::new(BTreeMap::new()));
fn err(code:ErrorCode,text:&str)->DfmcpError{DfmcpError::new(code,text)}
fn lock<T>(m:&Mutex<T>)->Result<MutexGuard<'_,T>>{m.lock().map_err(|_|err(ErrorCode::InternalInvariantViolation,"control mutex poisoned"))}
struct ControlSession{id:SessionId,client:ControlRpcClient<DeadlineStream>,request:u128,budget:WorkBudget,grants:Vec<CapabilityGrant>,last_anchor:Option<StateAnchor>}
impl ControlSession{
    fn context(&mut self)->Result<OperationContext>{self.request=self.request.checked_add(1).ok_or_else(||err(ErrorCode::BudgetExceeded,"control request IDs exhausted"))?;
        Ok(OperationContext{session_id:self.id,request_id:RequestId::new(self.request),anchor:self.last_anchor.unwrap_or_default(),budget:self.budget,
            grants:self.grants.clone(),cancellation_requested:false})}
}
fn next_id()->Result<SessionId>{let mut n=lock(&NEXT)?;if *n>=FAMILY{return Err(err(ErrorCode::BudgetExceeded,"control session IDs exhausted"));}
    let id=SessionId::new((1u128<<127)|FAMILY|*n);*n+=1;Ok(id)}
fn resolve(raw:Option<String>)->Result<Arc<Mutex<ControlSession>>>{let raw=raw.ok_or_else(||err(ErrorCode::InvalidRequest,"open control session first"))?;
    if raw.len()!=32||!raw.bytes().all(|b|b.is_ascii_hexdigit()){return Err(err(ErrorCode::InvalidRequest,"invalid control session"));}
    let v=u128::from_str_radix(&raw,16).map_err(|_|err(ErrorCode::InvalidRequest,"invalid control session"))?;let id=SessionId::new(v);
    if id.get()!=v||!id.is_process_scoped_live()||(v&((1u128<<62)-1))>>58!=1{return Err(err(ErrorCode::InvalidRequest,"not a control session"));}
    lock(&SESSIONS)?.get(&id).cloned().ok_or_else(||err(ErrorCode::SessionNotFound,"control session not found"))}
fn validate_environment()->Result<()>{
    let allowed=["DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7","DFMCP_CONTROL_TOKEN","DFMCP_CONTROL_ENDPOINT"];
    if std::env::var("DFMCP_ALLOW_UNADMITTED_CONTROL_V1_7").ok().as_deref()!=Some("1")
        ||std::env::vars_os().any(|(k,_)|{let k=k.to_string_lossy();k.starts_with("DFMCP_")&&!allowed.contains(&k.as_ref())})
        ||crate::admission::current_admission_provenance().is_some(){return Err(err(ErrorCode::CapabilityDenied,"control/1.7 requires exact development opt-in and refuses admission/other DFMCP state"));}Ok(())}
fn packet(operation:&str,value:Value)->String{json!({"agent_turn":{"operation":operation,"phase":"act","briefing":{"runtime":"unadmitted_development","bridge_protocol":"1.7","mutation_admissible":true,"supported_effects":["pause"]},"coverage":{"status":"partial","complete_domains":["pause_effect_protocol"],"omitted_domains":["dig","building","labor","burrow","stockpile","work_order","military","checkpoint"]}},"result":value}).to_string()}
fn failure(operation:&str,e:&DfmcpError)->String{packet(operation,json!({"ok":false,"error":{"code":e.code.as_str(),"message":e.message,"mutation_dispatched":false}}))}
fn with_session<F>(id:Option<String>,operation:&str,body:F)->String where F:FnOnce(&mut ControlSession,OperationContext)->Result<Value>{
    let h=match resolve(id){Ok(v)=>v,Err(e)=>return failure(operation,&e)};let mut s=match lock(&h){Ok(v)=>v,Err(e)=>return failure(operation,&e)};let c=match s.context(){Ok(v)=>v,Err(e)=>return failure(operation,&e)};
    match body(&mut s,c){Ok(v)=>packet(operation,v),Err(e)=>failure(operation,&e)}}
fn digest(raw:&str)->Result<Digest32>{if raw.len()!=64||!raw.bytes().all(|b|b.is_ascii_hexdigit()){return Err(err(ErrorCode::InvalidRequest,"plan_digest must be 64 hex chars"));}
    let mut out=[0u8;32];for i in 0..32{out[i]=u8::from_str_radix(&raw[i*2..i*2+2],16).map_err(|_|err(ErrorCode::InvalidRequest,"invalid plan digest"))?;}Ok(Digest32::from_bytes(out))}
fn effect_json(effect:PauseEffect)->Value{json!({"effect_known":effect.known,"effect_applied":effect.applied,"paused":effect.paused,"observed_game_tick":effect.observed_tick,
    "prepare_token_hex":effect.prepare_token.iter().map(|b|format!("{b:02x}")).collect::<String>(),"receipt_digest_hex":effect.receipt_digest.iter().map(|b|format!("{b:02x}")).collect::<String>()})}

#[tool(description="Open an explicitly unadmitted pause-control/1.7 development session. Only pause/resume prepare/commit/reconcile are supported.")]
pub fn fortress_open_session(max_wall_millis:Option<u64>)->String{
    let result=(||->Result<String>{validate_environment()?;let id=next_id()?;let timeout=Duration::from_millis(max_wall_millis.unwrap_or(5000));
        let endpoint=dfmcp_adapter::parse_loopback_endpoint(&std::env::var("DFMCP_CONTROL_ENDPOINT").unwrap_or_else(|_|"127.0.0.1:5000".to_owned()))?;
        let token=std::env::var("DFMCP_CONTROL_TOKEN").map_err(|_|err(ErrorCode::CapabilityDenied,"DFMCP_CONTROL_TOKEN required"))?;
        let client=ControlRpcClient::connect(endpoint,token.into_bytes(),id.get().to_be_bytes().to_vec(),timeout)?;
        let grants=vec![CapabilityGrant{capability:Capability::ControlClock,scope:CapabilityScope::default(),max_risk:RiskTier::Reversible,expires_at_tick:None,remaining_uses:None}];
        let budget=WorkBudget{max_wall_millis:max_wall_millis.unwrap_or(5000),max_actions:1,..WorkBudget::CONSERVATIVE_DEFAULT};
        let session=ControlSession{id,client,request:0,budget,grants,last_anchor:None};lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));
        Ok(packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),"supported_actions":["pause"],"runtime_admitted":false})))})();
    result.unwrap_or_else(|e|failure("fortress.open_session",&e))}

#[tool(description="Prepare one pause/resume effect. Requires stable idempotency_key, plan_digest, desired paused state and expected game tick. Does not mutate the game.")]
pub fn fortress_plan(session_id:Option<String>,idempotency_key:String,plan_digest:String,paused:bool,expected_game_tick:u64)->String{
    with_session(session_id,"fortress.plan",|s,c|{c.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;let d=digest(&plan_digest)?;
        let effect=s.client.prepare_pause(&idempotency_key,d,paused,expected_game_tick)?;Ok(json!({"ok":true,"prepared":true,"idempotency_key":idempotency_key,"plan_digest":plan_digest,"effect":effect_json(effect)}))})}

#[tool(description="Commit a previously prepared pause/resume effect. If transport fails after dispatch, treat the result as indeterminate and call fortress.explain with the same idempotency key and digest to reconcile before retrying.")]
pub fn fortress_commit(session_id:Option<String>,idempotency_key:String,plan_digest:String,prepare_token_hex:String)->String{
    with_session(session_id,"fortress.commit",|s,c|{c.authorize(Capability::ControlClock,RiskTier::Reversible,&[],None)?;let d=digest(&plan_digest)?;
        if prepare_token_hex.len()!=32||!prepare_token_hex.bytes().all(|b|b.is_ascii_hexdigit()){return Err(err(ErrorCode::InvalidRequest,"prepare_token_hex must be 32 hex chars"));}
        let mut token=Vec::with_capacity(16);for i in 0..16{token.push(u8::from_str_radix(&prepare_token_hex[i*2..i*2+2],16).map_err(|_|err(ErrorCode::InvalidRequest,"invalid prepare token"))?);}
        match s.client.commit_pause(&idempotency_key,d,&token){Ok(effect)=>Ok(json!({"ok":effect.applied,"state":if effect.applied{"verified"}else{"failed"},"effect":effect_json(effect)})),
            Err(e) if matches!(e.code,ErrorCode::AdapterUnavailable|ErrorCode::AdapterFailure)=>Err(DfmcpError::new(ErrorCode::EffectIndeterminate,"commit transport failed; reconcile before retry")),Err(e)=>Err(e)}})}

#[tool(description="Reconcile one pause/resume idempotency key after an ambiguous commit or inspect its known receipt. No new mutation is dispatched.")]
pub fn fortress_explain(session_id:Option<String>,idempotency_key:String,plan_digest:String)->String{
    with_session(session_id,"fortress.explain",|s,_|{let effect=s.client.query_pause(&idempotency_key,digest(&plan_digest)?)?;Ok(json!({"ok":true,"reconciliation":effect_json(effect),"safe_to_retry":!effect.known}))})}

fn denied(id:Option<String>,op:&str)->String{with_session(id,op,|_,_|Err(err(ErrorCode::CapabilityDenied,"control/1.7 supports only pause prepare/commit/reconcile")))}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_observe(session_id:Option<String>)->String{denied(session_id,"fortress.observe")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_query(session_id:Option<String>)->String{denied(session_id,"fortress.query")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_wait(session_id:Option<String>)->String{denied(session_id,"fortress.wait")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_cancel(session_id:Option<String>)->String{denied(session_id,"fortress.cancel")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_checkpoint(session_id:Option<String>)->String{denied(session_id,"fortress.checkpoint")}
#[tool(description="Unavailable in control/1.7.")] pub fn fortress_restore(session_id:Option<String>)->String{denied(session_id,"fortress.restore")}
#[tool(description="Report control/1.7 development status only.")] pub fn fortress_doctor(session_id:Option<String>)->String{with_session(session_id,"fortress.doctor",|s,_|Ok(json!({"ok":true,"source_fenced":s.client.poisoned(),"runtime_admitted":false,"supported_effects":["pause"]})))}

pub fn run_stdio(){if let Err(e)=validate_environment(){eprintln!("{e}");std::process::exit(1);}let server=ServerBuilder::new("dfmcp-live-control-dev",env!("CARGO_PKG_VERSION"))
    .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
    .instructions("Explicitly unadmitted control/1.7. Only pause/resume prepare, commit and reconcile are supported. Never retry an indeterminate commit without reconciliation.").build();crate::run_modern_stdio(server);}
