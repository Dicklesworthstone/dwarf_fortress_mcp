#![forbid(unsafe_code)]
//! Explicitly unadmitted mining control. All native effects use DigSession and
//! its durable coordinator; recovery-only MCP remains a separate sealed profile.
use std::collections::VecDeque;
use std::sync::{Mutex, MutexGuard, TryLockError, atomic::{AtomicU64, Ordering}};
use std::time::{Duration, Instant};
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::dig_control_policy::{DigControlPolicy, DigReview, PolicyDigGuard};
use dfmcp_adapter::dig_designation::{DigObservation, DigPlan, DigRegion, MAX_NATIVE_TICK};
use dfmcp_adapter::dig_designation::journal::{DigBinding, DigCursor, DigMode, DigState, MAX_FRAME_BYTES, MAX_JOURNAL_BYTES};
use dfmcp_adapter::dig_designation::journal::session::{DigSession, DigSessionGuard, DigSessionView, SOURCE_RESERVATION_BYTES};
use dfmcp_adapter::dig_designation::journal::private_file::{PrivateDigFile, open_private_dig};
use dfmcp_adapter::dig_designation::rpc::{DigSource, DigRpcClient, DigTcpStream, RPC_BYTES};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, Digest32, ErrorCode, GameTick, LeaseManager,
    ObservationCursor, OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};

mod presentation;
mod runtime;
use presentation::{OUTPUT_BYTES, digest, failure, inventory, packet};
use runtime::{Config, RequestControl};
const MAX_WORK_BYTES:u64=1024*1024*1024;
const VIEW_BYTES:u64=MAX_JOURNAL_BYTES as u64+4096;
const LOCAL_BYTES:u64=VIEW_BYTES+32*1024;
const OPEN_BYTES:u64=4*VIEW_BYTES;
const FAMILY:u128=18u128<<57;
static NEXT:AtomicU64=AtomicU64::new(1);
type Native=DigRpcClient<DigTcpStream>;
struct Entry {state:State<PrivateDigFile,Native>,config:Config}
static SESSION:Mutex<Option<Entry>>=Mutex::new(None);
fn error(code:ErrorCode,text:&str)->dfmcp_core::DfmcpError {dfmcp_core::DfmcpError::new(code,text)}
fn exhausted()->dfmcp_core::DfmcpError {error(ErrorCode::BudgetExceeded,"complete mining work and response exceed the allowance")}
fn unbound(op:&str,e:&dfmcp_core::DfmcpError)->String {
    let mut result=failure(e);result["effect_may_have_occurred"]=json!(matches!(op,"fortress.commit"|"fortress.cancel"));
    packet(op,result,None,None,None,None,None)
}
fn key(raw:&str)->Result<()> {
    if raw.is_empty()||raw.len()>128||!raw.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'.'|b'_'|b'-')) {
        return Err(error(ErrorCode::InvalidRequest,"invalid mining key"));
    }
    Ok(())
}
#[derive(Clone,Default)]
struct Cursors {serial:u64,entries:VecDeque<(String,DigCursor)>}
impl Cursors {
    fn issue(&mut self,cursor:DigCursor,c:&OperationContext,head:Digest32)->Result<String> {
        self.serial=self.serial.checked_add(1).ok_or_else(exhausted)?;
        let mut raw=b"dfmcp-dig-control-cursor/1\0".to_vec();raw.extend_from_slice(&c.session_id.get().to_be_bytes());
        raw.extend_from_slice(head.as_bytes());raw.extend_from_slice(&self.serial.to_be_bytes());
        let token=Digest32::of_bytes(&raw).to_string();
        if self.entries.len()==64 {self.entries.pop_front();}
        self.entries.push_back((token.clone(),cursor));Ok(token)
    }
    fn resolve(&self,token:&str)->Result<DigCursor> {
        digest(token)?;
        self.entries.iter().find(|(t,_)|t==token).map(|(_,c)|c.clone())
            .ok_or_else(||error(ErrorCode::StaleAnchor,"mining continuation expired"))
    }
}
struct Review {key:String,plan:Digest32,witness:Digest32,hidden:bool,value:DigReview}
struct State<S,N> {
    id:SessionId,request:u128,budget:WorkBudget,grants:Vec<CapabilityGrant>,
    control:DigSession<S,N>,binding:DigBinding,policy:DigControlPolicy,leases:LeaseManager,
    review:Option<Review>,cursors:Cursors,
}
impl<S:EffectJournalStorage,N:DigSource> State<S,N> {
    fn new(mut control:DigSession<S,N>,c:&OperationContext,config:&Config)->Result<Self> {
        if control.mode()!=DigMode::Control {return Err(error(ErrorCode::CapabilityDenied,"control requires an explicitly opened Control journal"));}
        let view=control.view(c)?;let binding=control.binding().clone();config.matches(&binding)?;
        let mut leases=LeaseManager::new();
        let lease=leases.acquire_spatial_lease(c.session_id,binding.scope(),true,GameTick(control.high_tick()),1200)?;
        let policy=DigControlPolicy::new(binding.clone(),view.journal_id,c.session_id,lease,config.protected.clone(),config.checkpoint)?;
        Ok(Self{id:c.session_id,request:c.request_id.get(),budget:c.budget,grants:c.grants.clone(),control,binding,policy,leases,
            review:None,cursors:Cursors::default()})
    }
    fn context(&mut self,write_enabled:bool,wall:Option<u64>)->Result<OperationContext> {
        self.request=self.request.checked_add(1).ok_or_else(exhausted)?;
        let mut budget=self.budget;if let Some(wall)=wall {budget.max_wall_millis=budget.max_wall_millis.min(wall);}
        let mut c=context(self.id,RequestId::new(self.request),&self.binding,self.control.high_tick(),budget,false);
        c.grants=self.grants.iter().filter(|g|write_enabled||!matches!(g.capability,Capability::Plan|Capability::Designate)).cloned().collect();
        Ok(c)
    }
    fn abandon(&mut self) {self.review=None;self.control.abandon_preparation();}
    fn seal(&self)->Option<Digest32> {
        if self.control.has_preparation_connection() {self.review.as_ref().map(|r|r.value.seal())} else {None}
    }
}
fn grants(fortress:dfmcp_core::FortressId,scope:dfmcp_core::MapCuboid,write:bool)->Vec<CapabilityGrant> {
    [Capability::Query,Capability::Observe,Capability::Plan,Capability::Designate].into_iter()
        .filter(|v|write||matches!(v,Capability::Query|Capability::Observe)).map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope{fortress_id:Some(fortress),map_area:Some(scope),..CapabilityScope::default()},
            max_risk:if matches!(capability,Capability::Plan|Capability::Designate){RiskTier::Guarded}else{RiskTier::ReadOnly},
            expires_at_tick:None,remaining_uses:None}).collect()
}
fn context(id:SessionId,request_id:RequestId,binding:&DigBinding,tick:u64,budget:WorkBudget,write:bool)->OperationContext {
    OperationContext{session_id:id,request_id,budget,cancellation_requested:false,
        anchor:StateAnchor{fortress_id:binding.fortress_id(),cursor:ObservationCursor::ORIGIN,tick:GameTick(tick),state_hash:Digest32::ZERO},
        grants:grants(binding.fortress_id(),binding.scope(),write)}
}
struct Work {deadline:Instant,bytes:u64}
impl Work {
    fn new(c:&OperationContext,started:Instant)->Result<Self> {
        c.budget.validate()?;
        if c.budget.max_wall_millis>60_000||c.budget.max_bytes>MAX_WORK_BYTES||c.budget.max_output_tokens>65_536
            ||u64::from(c.budget.max_output_tokens)*4<OUTPUT_BYTES {return Err(exhausted());}
        let bytes=c.budget.max_bytes.checked_sub(OUTPUT_BYTES+2*VIEW_BYTES).ok_or_else(exhausted)?;
        let deadline=started.checked_add(Duration::from_millis(c.budget.max_wall_millis)).ok_or_else(exhausted)?;
        let out=Self{deadline,bytes};out.current(c)?;Ok(out)
    }
    fn current(&self,c:&OperationContext)->Result<OperationContext> {
        let left=self.deadline.checked_duration_since(Instant::now()).ok_or_else(exhausted)?.as_millis();
        if left==0 {return Err(exhausted());}
        let mut out=c.clone();out.budget.max_wall_millis=u64::try_from(left).map_err(|_|exhausted())?;
        out.budget.max_bytes=self.bytes;Ok(out)
    }
    fn take(&mut self,c:&OperationContext,bytes:u64)->Result<OperationContext> {
        let mut out=self.current(c)?;self.bytes=self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        out.budget.max_bytes=bytes;Ok(out)
    }
    fn view(&self,c:&OperationContext)->Result<OperationContext> {let mut out=self.current(c)?;out.budget.max_bytes=VIEW_BYTES;Ok(out)}
    fn operation(&mut self,c:&OperationContext,v:&DigSessionView)->Result<OperationContext> {
        self.take(c,44*(v.byte_len+2*MAX_FRAME_BYTES) as u64+SOURCE_RESERVATION_BYTES+6*RPC_BYTES+64*1024)
    }
}
#[derive(Deserialize)]
#[serde(tag="mode",rename_all="snake_case",deny_unknown_fields)]
enum Query {
    Records {limit:Option<usize>,continuation:Option<String>},
    SelectionTiles {witness:String,offset:usize,limit:Option<usize>},
    PlanTiles {idempotency_key:String,plan_digest:String,offset:usize,limit:Option<usize>},
    Schema,
}
impl Query {
    fn parse(raw:&str)->Result<Self> {
        if raw.len()>2048 {return Err(error(ErrorCode::InvalidRequest,"mining query exceeds 2 KiB"));}
        let shape:Value=serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"invalid closed mining query"))?;
        if shape.as_object().is_some_and(|m|m.values().any(Value::is_null)) {
            return Err(error(ErrorCode::InvalidRequest,"omit optional query fields instead of supplying null"));
        }
        // Parse the original again: converting from Value would hide duplicate fields.
        let value:Self=serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"invalid closed mining query"))?;
        match &value {
            Self::Records{limit,continuation}=>{
                if !(1..=8).contains(&limit.map_or(8,|n|n)){return Err(exhausted());}
                if let Some(v)=continuation {digest(v)?;}
            }
            Self::SelectionTiles{witness,offset,limit}=>{digest(witness)?;tile_bounds(*offset,*limit)?;}
            Self::PlanTiles{idempotency_key,plan_digest,offset,limit}=>{key(idempotency_key)?;digest(plan_digest)?;tile_bounds(*offset,*limit)?;}
            Self::Schema=>{}
        }
        Ok(value)
    }
}
fn tile_bounds(offset:usize,limit:Option<usize>)->Result<()> {
    if offset>=300||!(1..=16).contains(&limit.map_or(16,|n|n)){return Err(exhausted());}Ok(())
}
enum Action {
    Observe(DigRegion),Plan{key:String,witness:Digest32,hidden:bool},Commit{key:String,plan:Digest32,seal:Digest32},
    Wait(String,Digest32),Cancel(String,Digest32),Explain(String,Digest32),Query(Query),Inventory,Denied,
}
fn perform<S,N,G,F>(state:&mut State<S,N>,c:&OperationContext,work:&mut Work,before:&DigSessionView,
    cursors:&mut Cursors,action:Action,runtime:&mut G,connect:F)->Result<Value>
where S:EffectJournalStorage,N:DigSource,G:DigSessionGuard,
    F:FnOnce(&DigBinding,DigRegion,&OperationContext)->Result<N> {
    match action {
        Action::Observe(region)=>{
            state.abandon();
            let mut guard=PolicyDigGuard::new(&state.policy,&state.leases,runtime,None);
            let value=state.control.observe(region,&work.operation(c,before)?,connect,&mut guard)?;
            Ok(json!({"ok":true,"observation":presentation::observation(&value),"game_mutation_dispatched":false}))
        }
        Action::Plan{key,witness,hidden}=>{
            if let Some(review)=&state.review {
                if review.key!=key||review.witness!=witness||review.hidden!=hidden {return Err(error(ErrorCode::Conflict,"another exact mining review is pending"));}
                let value=state.control.get(&key,review.plan,&work.take(c,LOCAL_BYTES)?)?;
                state.policy.evaluate(value.plan(),c,&state.leases)?;
                return Ok(json!({"ok":true,"plan":presentation::record(&value),"review_seal":review.value.seal().to_string(),"replayed_locally":true}));
            }
            let capture=state.control.selected().filter(|v|v.witness()==witness)
                .ok_or_else(||error(ErrorCode::StaleAnchor,"planning needs this session's exact selected observation"))?;
            let plan=DigPlan::new(&key,hidden,capture.clone())?;
            state.policy.evaluate(&plan,c,&state.leases)?;
            let mut guard=PolicyDigGuard::new(&state.policy,&state.leases,runtime,None);
            let value=state.control.prepare(&key,hidden,witness,&work.operation(c,before)?,&mut guard)?;
            if value.state()==DigState::Prepared&&state.control.has_preparation_connection() {
                let review=state.policy.review(value.plan(),&work.current(c)?,&state.leases)?;
                state.review=Some(Review{key,plan:value.plan().digest(),witness,hidden,value:review});
            }
            Ok(json!({"ok":true,"plan":presentation::record(&value),"review_seal":state.seal().map(|s|s.to_string()),"game_mutation_dispatched":false}))
        }
        Action::Commit{key,plan,seal}=>{
            let old=state.control.get(&key,plan,&work.take(c,LOCAL_BYTES)?)?;
            if old.state().terminal(){return Ok(json!({"ok":true,"effect":presentation::record(&old),"native_calls":0,"historical_replay":true}));}
            if !state.review.as_ref().is_some_and(|r|r.key==key&&r.plan==plan&&r.value.seal()==seal) {
                return Err(error(ErrorCode::CapabilityDenied,"exact local review must be confirmed"));
            }
            state.policy.evaluate(old.plan(),&work.current(c)?,&state.leases)?;
            let review=state.review.take().ok_or_else(||error(ErrorCode::CapabilityDenied,"review unavailable"))?;
            let confirmed=review.value.confirm(seal)?;
            let mut guard=PolicyDigGuard::new(&state.policy,&state.leases,runtime,Some(&confirmed));
            let result=state.control.commit(&key,plan,&work.operation(c,before)?,&mut guard);
            state.abandon();
            Ok(json!({"ok":true,"effect":presentation::record(&result?),"excavation_completion_proven":false}))
        }
        Action::Wait(key,plan)=>{
            state.abandon();
            let mut guard=PolicyDigGuard::new(&state.policy,&state.leases,runtime,None);
            let result=state.control.reconcile(&key,plan,&work.operation(c,before)?,connect,&mut guard)?;
            Ok(json!({"ok":true,"effect":presentation::record(&result),"commit_retried":false}))
        }
        Action::Cancel(key,plan)=>{
            state.abandon();
            let mut guard=PolicyDigGuard::new(&state.policy,&state.leases,runtime,None);
            let result=state.control.cancel(&key,plan,&work.operation(c,before)?,connect,&mut guard)?;
            Ok(json!({"ok":true,"effect":presentation::record(&result),"designation_undone":false}))
        }
        Action::Explain(key,plan)=>{
            let value=state.control.get(&key,plan,&work.take(c,LOCAL_BYTES)?)?;
            Ok(json!({"ok":true,"record":presentation::record(&value),"native_calls":0}))
        }
        Action::Query(Query::Records{limit,continuation})=>{
            let cursor=continuation.as_deref().map(|s|cursors.resolve(s)).transpose()?;
            let page=state.control.list(&work.take(c,LOCAL_BYTES)?,limit.map_or(8,|n|n),cursor.as_ref())?;
            let next=page.continuation.map(|v|cursors.issue(v,c,page.head)).transpose()?;
            Ok(json!({"ok":true,"records":page.records.iter().map(presentation::summary).collect::<Vec<_>>(),
                "total_records":page.total_records,"unsettled_records":page.unsettled_records,"continuation":next,"native_calls":0}))
        }
        Action::Query(Query::SelectionTiles{witness,offset,limit})=>{
            let wanted=digest(&witness)?;
            let selected=state.control.selected().filter(|v|v.witness()==wanted)
                .ok_or_else(||error(ErrorCode::StaleAnchor,"selection witness no longer retained"))?;
            let limit=limit.map_or(16,|n|n);let mut page=presentation::tile_page(selected,offset,limit)?;
            page["next_query"]=page["next_offset"].as_u64().map_or(Value::Null,|offset|
                json!({"mode":"selection_tiles","witness":witness,"offset":offset,"limit":limit}));
            Ok(json!({"ok":true,"page":page,"native_calls":0}))
        }
        Action::Query(Query::PlanTiles{idempotency_key,plan_digest,offset,limit})=>{
            let value=state.control.get(&idempotency_key,digest(&plan_digest)?,&work.take(c,LOCAL_BYTES)?)?;
            let limit=limit.map_or(16,|n|n);let mut page=presentation::tile_page(value.plan().before(),offset,limit)?;
            page["next_query"]=page["next_offset"].as_u64().map_or(Value::Null,|offset|
                json!({"mode":"plan_tiles","idempotency_key":idempotency_key,"plan_digest":plan_digest,"offset":offset,"limit":limit}));
            Ok(json!({"ok":true,"page":page,"native_calls":0}))
        }
        Action::Query(Query::Schema)=>Ok(json!({"ok":true,"query_schema":serde_json::from_str::<Value>(include_str!("../../../schemas/dig_control_query.json"))
            .map_err(|_|error(ErrorCode::InternalInvariantViolation,"invalid embedded mining query schema"))?,"native_calls":0})),
        Action::Inventory=>Ok(json!({"ok":true,"journal":inventory(before),"native_calls":0,"live_health_checked":false})),
        Action::Denied=>Err(error(ErrorCode::CapabilityDenied,"mining journal is not a game checkpoint or restore")),
    }
}

fn run_action<S,N,G,F>(state:&mut State<S,N>,c:OperationContext,op:&str,action:Result<Action>,started:Instant,
    runtime:&mut G,connect:F)->String
where S:EffectJournalStorage,N:DigSource,G:DigSessionGuard,F:FnOnce(&DigBinding,DigRegion,&OperationContext)->Result<N> {
    if op=="fortress.observe" {state.abandon();}
    let mut work=match Work::new(&c,started) {Ok(w)=>w,Err(e)=>return packet(op,failure(&e),Some(&c),None,None,None,None)};
    let before=match work.view(&c).and_then(|c|state.control.view(&c)) {
        Ok(v)=>v,Err(e)=>{state.abandon();return packet(op,failure(&e),Some(&c),Some(&state.binding),None,Some(&state.policy),None);}
    };
    let mut staged=state.cursors.clone();
    let outcome=(||{
        let current=work.current(&c)?;
        perform(state,&current,&mut work,&before,&mut staged,action?,runtime,connect)
    })();
    if outcome.is_err()&&matches!(op,"fortress.observe"|"fortress.plan"|"fortress.commit"|"fortress.wait"|"fortress.cancel") {state.abandon();}
    let mut display=c.clone();display.anchor.tick=GameTick(state.control.high_tick());
    let after=work.view(&display).and_then(|c|state.control.view(&c));
    let (mut result,view)=match after {
        Ok(v)=>(match outcome {Ok(v)=>v,Err(e)=>failure(&e)},Some(v)),
        Err(e)=>{state.abandon();let mut failed=failure(&e);failed["post_operation_inventory_unverified"]=json!(true);
            failed["historical_pending_before_request"]=json!(before.pending.as_ref().map(presentation::summary));(failed,None)}
    };
    if result["ok"]!=true {result["effect_may_have_occurred"]=json!(matches!(op,"fortress.commit"|"fortress.cancel"));}
    let rendered=packet(op,result,Some(&display),Some(&state.binding),view.as_ref(),Some(&state.policy),state.seal());
    if rendered.len() as u64>OUTPUT_BYTES || work.current(&display).is_err() {
        state.abandon();return packet(op,failure(&error(ErrorCode::EffectIndeterminate,"response exceeded reservation; recover the journal")),
            Some(&display),Some(&state.binding),view.as_ref(),None,None);
    }
    if view.is_some(){state.cursors=staged;}
    rendered
}
fn lock()->Result<MutexGuard<'static,Option<Entry>>> {
    match SESSION.try_lock(){Ok(v)=>Ok(v),Err(TryLockError::WouldBlock)=>Err(error(ErrorCode::Conflict,"mining session is already serving a request")),
        Err(TryLockError::Poisoned(_))=>Err(error(ErrorCode::InternalInvariantViolation,"mining session poisoned; restart for recovery"))}
}
fn session_id(raw:&str)->Result<SessionId> {
    if raw.len()!=32||!raw.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)){return Err(error(ErrorCode::InvalidRequest,"invalid control session ID"));}
    let n=u128::from_str_radix(raw,16).map_err(|_|error(ErrorCode::InvalidRequest,"invalid control session ID"))?;
    let id=SessionId::new(n);
    if id.get()!=n||!id.is_process_scoped_live()||(n&((1u128<<62)-1))>>57!=18 {return Err(error(ErrorCode::InvalidRequest,"wrong mining session family"));}
    Ok(id)
}
async fn with_session(raw:String,op:&'static str,wall:Option<u64>,action:Result<Action>)->String {
    runtime::owned(op,move|control:RequestControl|{
        let result=(||{
            let id=session_id(&raw)?;let mut locked=lock()?;
            let entry=locked.as_mut().filter(|v|v.state.id==id).ok_or_else(||error(ErrorCode::SessionNotFound,"mining session absent"))?;
            if op=="fortress.observe"{entry.state.abandon();}
            runtime::boundary(&control,&entry.config,false)?;
            let c=entry.state.context(runtime::enabled()?,wall)?;
            let binding=entry.state.binding.clone();let config=entry.config.clone();
            let mut guard=runtime::Guard{control:&control,config:&config,binding:&binding};
            let rendered=run_action(&mut entry.state,c,op,action,control.started,&mut guard,|b,r,c|{
                config.matches(b)?;runtime::connect(&control,&config,r,c)
            });
            if let Err(e)=runtime::boundary(&control,&config,false) {entry.state.abandon();return Err(e);}
            Ok(rendered)
        })();
        match result{Ok(value)=>value,Err(e)=>unbound(op,&e)}
    }).await
}

#[tool(name="fortress.open_session",description="Open isolated mining control with a mandatory private Rust journal and host lease. region is a closed JSON array [x,y,z,width,height] used only to verify the configured fortress/source. Paths, protected areas, checkpoint policy and permissions are operator configuration. Default checkpoint policy forbids new designation. No native preparation or commit occurs during open.")]
pub async fn fortress_open_session(region:String,max_wall_millis:Option<u64>,max_bytes:Option<u64>,max_output_tokens:Option<u32>)->String {
    runtime::owned("fortress.open_session",move|control|{
        let result=(||{
            let config=runtime::configuration()?;runtime::boundary(&control,&config,false)?;
            let region=parse_region(&region)?;config.region(region)?;
            let budget=WorkBudget{max_wall_millis:max_wall_millis.map_or(10_000,|n|n),max_bytes:max_bytes.map_or(MAX_WORK_BYTES,|n|n),
                max_output_tokens:max_output_tokens.map_or(8192,|n|n),max_entities:300,max_actions:1,max_game_ticks:0};
            let mut locked=lock()?;if locked.is_some(){return Err(error(ErrorCode::Conflict,"release the current mining session first"));}
            let seq=NEXT.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<(1u64<<57)).then_some(n+1)).map_err(|_|exhausted())?;
            let id=SessionId::new((1u128<<127)|FAMILY|u128::from(seq));let write=runtime::enabled()?;
            let mut c=OperationContext{session_id:id,request_id:RequestId::new(1),budget,cancellation_requested:false,
                anchor:StateAnchor{fortress_id:config.fortress(),cursor:ObservationCursor::ORIGIN,tick:GameTick(0),state_hash:Digest32::ZERO},
                grants:grants(config.fortress(),config.scope,write)};
            let mut work=Work::new(&c,control.started)?;
            let mut source=runtime::connect(&control,&config,region,&work.take(&c,SOURCE_RESERVATION_BYTES)?)?;
            runtime::boundary(&control,&config,false)?;
            let before:DigObservation=source.observe(region,&work.take(&c,RPC_BYTES)?)?;
            if before.region()!=region||before.fortress_id()!=config.fortress()||before.folder()!=config.folder||before.site()!=config.site
                ||before.tick()>MAX_NATIVE_TICK {return Err(error(ErrorCode::StaleAnchor,"bootstrap source differs from configured fortress"));}
            c.anchor.tick=GameTick(before.tick());
            let binding=DigBinding::new(config.endpoint,source.manifest().clone(),&before,config.scope)?;
            drop(source);runtime::boundary(&control,&config,false)?;
            let journal=open_private_dig(&config.path,&work.take(&c,OPEN_BYTES)?,DigMode::Control,Some(binding))?;
            let session=DigSession::<_,Native>::new(journal,&work.view(&c)?)?;
            let mut state=State::new(session,&work.take(&c,LOCAL_BYTES)?,&config)?;
            // Keep the negotiated request ceilings, not a nested replay allowance.
            state.budget=budget;state.grants=c.grants.clone();
            let view=state.control.view(&work.view(&c)?)?;
            let rendered=packet("fortress.open_session",json!({"ok":true,"session_id":id.to_string(),"journal":inventory(&view),
                "capabilities":c.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>(),"planning_observation_retained":false,
                "native_designation_dispatched":false}),Some(&c),Some(&state.binding),Some(&view),Some(&state.policy),None);
            if rendered.len() as u64>OUTPUT_BYTES {return Err(exhausted());}
            work.current(&c)?;runtime::boundary(&control,&config,false)?;
            *locked=Some(Entry{state,config});Ok(rendered)
        })();
        match result{Ok(v)=>v,Err(e)=>unbound("fortress.open_session",&e)}
    }).await
}
fn parse_region(raw:&str)->Result<DigRegion> {
    if raw.len()>128{return Err(error(ErrorCode::InvalidRequest,"region exceeds its bound"));}
    let [x,y,z,w,h]:[u32;5]=serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"region must be [x,y,z,width,height]"))?;
    DigRegion::new(x,y,z,w,h)
}
#[tool(name="fortress.observe",description="Retain one exact bounded mining observation and its native connection. region is [x,y,z,width,height], width/height 1..8. A new observation abandons old local commit permission, never the durable obligation. No game mutation.")]
pub async fn fortress_observe(session_id:String,region:String)->String {
    with_session(session_id,"fortress.observe",None,parse_region(&region).map(Action::Observe)).await
}
#[tool(name="fortress.plan",description="Prepare ordinary mining from this session's exact observation witness and explicit hidden-neighbor policy. Requires current operator designation enablement, whole-block lease and allowed checkpoint/protected-region policy. Review the returned native plan digest and policy-bound review seal. No designation is dispatched yet.")]
pub async fn fortress_plan(session_id:String,idempotency_key:String,observation_witness:String,allow_hidden_neighbors:bool)->String {
    let action=key(&idempotency_key).and_then(|_|digest(&observation_witness)).map(|witness|Action::Plan{key:idempotency_key,witness,hidden:allow_hidden_neighbors});
    with_session(session_id,"fortress.plan",None,action).await
}
#[tool(name="fortress.commit",description="Confirm the exact reviewed plan digest and review seal for one native commit attempt on the original preparation connection. Current policy, leases, capabilities, source and complete terrain witness are rechecked. Lost replies require exact-record recovery, never retry. Success proves designation configuration, not completed excavation.")]
pub async fn fortress_commit(session_id:String,idempotency_key:String,plan_digest:String,review_seal:String)->String {
    let action=key(&idempotency_key).and_then(|_|Ok((digest(&plan_digest)?,digest(&review_seal)?)))
        .map(|(plan,seal)|Action::Commit{key:idempotency_key,plan,seal});
    with_session(session_id,"fortress.commit",None,action).await
}
#[tool(name="fortress.query",description="Bounded local records, selection_tiles, plan_tiles or schema query. query is closed JSON. Records page 1..8, terrain page 1..16. Continuations bind exact session/journal/head/page width; terrain is historical evidence. No native calls.")]
pub async fn fortress_query(session_id:String,query:String)->String {with_session(session_id,"fortress.query",None,Query::parse(&query).map(Action::Query)).await}
fn effect_action(key_value:String,raw:String,make:impl FnOnce(String,Digest32)->Action)->Result<Action> {
    key(&key_value)?;Ok(make(key_value,digest(&raw)?))
}
#[tool(name="fortress.wait",description="Query the original native mining outcome once, then sync validated evidence. No commit replay, timer loop or game-clock advance. Terminal and permanent Unknown return locally. Querying a preparation revokes local commit permission.")]
pub async fn fortress_wait(session_id:String,idempotency_key:String,plan_digest:String,max_wall_millis:Option<u64>)->String {
    with_session(session_id,"fortress.wait",max_wall_millis,effect_action(idempotency_key,plan_digest,Action::Wait)).await
}
#[tool(name="fortress.explain",description="Inspect the exact retained mining plan, witness and native receipt without a new game observation or mutation.")]
pub async fn fortress_explain(session_id:String,idempotency_key:String,plan_digest:String)->String {
    with_session(session_id,"fortress.explain",None,effect_action(idempotency_key,plan_digest,Action::Explain)).await
}
async fn close(raw:String,release:bool)->String {
    runtime::owned("fortress.cancel",move|control|{
        let result=(||{
            control.checkpoint()?;let id=session_id(&raw)?;let mut locked=lock()?;
            let entry=locked.as_mut().filter(|e|e.state.id==id).ok_or_else(||error(ErrorCode::SessionNotFound,"mining session absent"))?;
            let c=entry.state.context(false,None)?;
            let view=if release{None}else{
                runtime::boundary(&control,&entry.config,false)?;
                let work=Work::new(&c,control.started)?;Some(entry.state.control.view(&work.view(&c)?)?)
            };
            if !release&&view.as_ref().is_some_and(|v|v.pending.is_some()){return Err(error(ErrorCode::EffectIndeterminate,"unsettled work requires explicit recovery release"));}
            let output=packet("fortress.cancel",json!({"ok":true,"scope":"session","closed":true,"release_for_recovery":release,
                "effects_cancelled":false,"history_erased":false,"native_quiescence_proven":false}),Some(&c),Some(&entry.state.binding),view.as_ref(),None,None);
            if output.len() as u64>OUTPUT_BYTES{return Err(exhausted());}
            drop(locked.take());Ok(output)
        })();match result{Ok(v)=>v,Err(e)=>unbound("fortress.cancel",&e)}
    }).await
}
#[tool(name="fortress.cancel",description="scope=effect retires the exact native preparation under current operator authorization; it cannot undo designation. scope=session releases only settled custody unless release_for_recovery=true. Releasing a session drops local permission but preserves every journal obligation.")]
pub async fn fortress_cancel(session_id:String,scope:String,idempotency_key:Option<String>,plan_digest:Option<String>,release_for_recovery:Option<bool>)->String {
    let release=release_for_recovery.map_or(false,|v|v);
    match (scope.as_str(),idempotency_key,plan_digest) {
        ("session",None,None)=>close(session_id,release).await,
        ("effect",Some(key),Some(plan)) if !release=>with_session(session_id,"fortress.cancel",None,effect_action(key,plan,Action::Cancel)).await,
        _=>unbound("fortress.cancel",&error(ErrorCode::InvalidRequest,"cancel scope and identity disagree")),
    }
}
#[tool(name="fortress.checkpoint",description="Unavailable: the mining journal is not a verified game save. Required-checkpoint policy refuses new designation.")]
pub async fn fortress_checkpoint(session_id:String)->String {with_session(session_id,"fortress.checkpoint",None,Ok(Action::Denied)).await}
#[tool(name="fortress.restore",description="Unavailable: journal replay does not restore terrain or undo mining.")]
pub async fn fortress_restore(session_id:String)->String {with_session(session_id,"fortress.restore",None,Ok(Action::Denied)).await}
#[tool(name="fortress.doctor",description="Verify local mining coordination inventory; this is not live-game health, structural safety or production admission.")]
pub async fn fortress_doctor(session_id:String)->String {with_session(session_id,"fortress.doctor",None,Ok(Action::Inventory)).await}
pub fn run_stdio() {
    if let Err(e)=runtime::configuration(){eprintln!("{e}");std::process::exit(1);}
    let server=ServerBuilder::new("dfmcp-dig-control-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted development mining control only. Default policy requires an unavailable game checkpoint and refuses designation. Only explicitly operator-selected disposable-fortress policy permits uncheckpointed development work. Observe an exact region, review a prepared plan and its policy seal, commit once on the original connection, and recover by the original key. Historical designation is not excavation completion or safety. Whole shared blocks require the host lease and must avoid protected areas. Other controllers and UI are not globally fenced. No arbitrary commands, clock advancement, checkpoint or restore.")
        .build();crate::run_modern_stdio(server);
}
#[cfg(test)]
mod tests;
