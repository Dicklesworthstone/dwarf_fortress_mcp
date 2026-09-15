//! Explicitly unadmitted spatial/1.6 runtime. Operations and bounded terrain
//! arrive in one immutable capture; no independently timed snapshots are joined.
#[path="semantic_query.rs"] mod semantic_query;
#[path="query_response.rs"] mod query_response;
#[path="spatial_queries.rs"] mod spatial_queries;
#[path="spatial_history.rs"] mod history;

use std::collections::BTreeMap;
use std::sync::{Arc,LazyLock,Mutex,MutexGuard};
use std::sync::atomic::{AtomicUsize,Ordering};
use std::time::Duration;
use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_jobs_rpc::{DeadlineStream,operations::{paged::PagedOperationsLimits,map::spatial::{SpatialLimits,SpatialRpcClient}}};
use dfmcp_adapter::live_map::map_error;
use dfmcp_adapter::live_spatial::{LiveSpatialObservation,LiveSpatialState,MAX_SPATIAL_BYTES};
use dfmcp_world::map_region::Region;
use dfmcp_core::{Capability,CapabilityGrant,CapabilityScope,DfmcpError,ErrorCode,OperationContext,
    RequestId,Result,RiskTier,SessionId,StateAnchor,WorkBudget};
use crate::agent_turn::{AgentPhase,AgentTurnBuilder,ContinuityStatus,empty_active_work};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value,json};
use query_response::QueryResponseProjection;

const FAMILY:u128=1u128<<58;
static NEXT:Mutex<u128>=Mutex::new(1);
static SLOTS:AtomicUsize=AtomicUsize::new(0);
static SESSIONS:LazyLock<Mutex<BTreeMap<SessionId,Arc<Mutex<SpatialSession>>>>>=LazyLock::new(||Mutex::new(BTreeMap::new()));
fn error(code:ErrorCode,s:&str)->DfmcpError{DfmcpError::new(code,s)}
fn lock<T>(m:&Mutex<T>)->Result<MutexGuard<'_,T>>{m.lock().map_err(|_|error(ErrorCode::InternalInvariantViolation,"spatial mutex poisoned"))}
struct Slot;
impl Slot{fn reserve()->Result<Self>{SLOTS.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<2).then_some(n+1))
    .map_err(|_|error(ErrorCode::BudgetExceeded,"spatial runtime retains at most two sessions"))?;Ok(Self)}}
impl Drop for Slot{fn drop(&mut self){SLOTS.fetch_sub(1,Ordering::AcqRel);}}
trait SpatialSource:Send{
    fn read(&mut self,timeout:Duration)->Result<LiveSpatialObservation>;
    fn poisoned(&self)->bool;
    fn fence(&mut self);
    fn pages(&self)->u32;
}
impl SpatialSource for SpatialRpcClient<DeadlineStream>{
    fn read(&mut self,t:Duration)->Result<LiveSpatialObservation>{self.refresh(t)}
    fn poisoned(&self)->bool{SpatialRpcClient::poisoned(self)}
    fn fence(&mut self){SpatialRpcClient::fence(self);}
    fn pages(&self)->u32{self.last_page_count()}
}
struct SpatialSession{id:SessionId,source:Box<dyn SpatialSource>,state:LiveSpatialState,limits:SpatialLimits,
    journal:Option<history::Journal>,
    budget:WorkBudget,grants:Vec<CapabilityGrant>,request:u128,_slot:Slot}
impl SpatialSession{
    fn anchor(&self)->Result<StateAnchor>{self.state.snapshot().map(|s|s.anchor()).ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial snapshot absent"))}
    fn context(&mut self)->Result<OperationContext>{
        self.request=self.request.checked_add(1).ok_or_else(||error(ErrorCode::BudgetExceeded,"spatial request IDs exhausted"))?;
        Ok(OperationContext{session_id:self.id,request_id:RequestId::new(self.request),anchor:self.anchor()?,budget:self.budget,
            grants:self.grants.clone(),cancellation_requested:false})
    }
    fn refresh(&mut self,c:&OperationContext)->Result<JobPublication>{
        c.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
        if c.anchor!=self.anchor()?{return Err(error(ErrorCode::StaleAnchor,"spatial refresh anchor changed"));}
        if self.source.poisoned(){return Err(error(ErrorCode::AdapterUnavailable,"spatial source fenced; reopen session"));}
        if let Some(journal)=&self.journal {
            c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
            if journal.fenced() || journal.state().snapshot().map(|s|s.anchor())!=Some(c.anchor) {
                self.source.fence();
                return Err(error(ErrorCode::CorruptLedger,"spatial journal is fenced or disagrees with the published anchor"));
            }
        }
        let replay=history::replay_context(self,c);
        let result=self.source.read(Duration::from_millis(c.budget.max_wall_millis)).and_then(|v|{
            let op=v.operations();let limits=self.limits.operations;
            if op.jobs.jobs.len()>limits.jobs as usize||op.buildings.len()>limits.buildings as usize||op.items.len()>limits.items as usize
                ||v.terrain().map.region!=self.limits.region||v.encode_payload()?.len()>limits.payload_bytes{
                return Err(error(ErrorCode::BudgetExceeded,"spatial observation exceeds session acquisition bounds"));}
            match self.journal.as_mut() {
                Some(journal)=>{
                    let outcome=journal.append(v,&replay)?;
                    self.state=journal.state().clone();
                    Ok(outcome)
                }
                None=>self.state.publish(v),
            }
        });if result.is_err(){self.source.fence();}result
    }
}
fn next_id()->Result<SessionId>{let mut n=lock(&NEXT)?;if *n>=FAMILY{return Err(error(ErrorCode::BudgetExceeded,"spatial session IDs exhausted"));}
    let id=SessionId::new((1u128<<127)|FAMILY|*n);*n+=1;Ok(id)}
fn resolve(raw:Option<String>)->Result<Arc<Mutex<SpatialSession>>>{
    let text=raw.ok_or_else(||error(ErrorCode::InvalidRequest,"open a spatial session and supply session_id"))?;
    if text.len()!=32||!text.bytes().all(|b|b.is_ascii_hexdigit()){return Err(error(ErrorCode::InvalidRequest,"invalid spatial session handle"));}
    let raw=u128::from_str_radix(&text,16).map_err(|_|error(ErrorCode::InvalidRequest,"invalid spatial session hex"))?;
    let id=SessionId::new(raw);
    if id.get()!=raw||!id.is_process_scoped_live()||(raw&((1u128<<62)-1))>>58!=1{
        return Err(error(ErrorCode::InvalidRequest,"not an encoded spatial session"));}
    lock(&SESSIONS)?.get(&id).cloned().ok_or_else(||error(ErrorCode::SessionNotFound,"spatial session not found"))
}
fn allowed_environment(n:&str)->bool{!n.starts_with("DFMCP_")||matches!(n,"DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_6"|"DFMCP_SPATIAL_TOKEN"|"DFMCP_SPATIAL_ENDPOINT"|"DFMCP_SPATIAL_JOURNAL"|"DFMCP_SPATIAL_JOURNAL_REPAIR")}
fn validate_environment()->Result<()>{
    if std::env::var("DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_6").ok().as_deref()!=Some("1")
        ||std::env::vars_os().any(|(n,_)|!allowed_environment(&n.to_string_lossy()))||crate::admission::current_admission_provenance().is_some(){
        return Err(error(ErrorCode::CapabilityDenied,"spatial/1.6 requires its own opt-in and refuses other DFMCP/admission state"));}Ok(())
}
fn anchor_json(a:StateAnchor)->Value{json!({"fortress_id":a.fortress_id.to_string(),"epoch":a.cursor.epoch,
    "sequence":a.cursor.sequence,"game_tick":a.tick.0,"state_hash":a.state_hash.to_string()})}
fn briefing(s:&SpatialSession)->Value{
    let v=s.state.observation();
    json!({"runtime":"unadmitted_development","bridge_protocol":"1.6","observation_profile":"coherent-spatial",
        "runtime_admitted":false,"mutation_admissible":false,"read_only":true,"source_poisoned":s.source.poisoned(),
        "job_count":v.map(|v|v.operations().jobs.jobs.len()),"building_count":v.map(|v|v.operations().buildings.len()),
        "item_count":v.map(|v|v.operations().items.len()),"region":{"origin":s.limits.region.origin,"size":s.limits.region.size},
        "observation_history":history::summary(s.journal.as_ref()),
        "capture_tick":v.map(|v|v.terrain().tick().0),"last_transfer_pages":s.source.pages(),
        "freshness":"capture time, not transfer completion; simulation can advance during page transfer"})
}
fn coverage()->Value{json!({"status":"partial","complete_domains":["current_job_roster","building_roster","item_roster","job_item_attachments","requested_region.cell_presence"],
    "partial_domains":[{"domain":"fortress.spatial","reason":"one coherent capture; hidden terrain redacted, unallocated terrain unknown, route model deliberately restricted"}],
    "omitted_domains":["outside_region_terrain","unit_navigation_rules","citizens","native_material_requirements","continuous_game_history"],"continuation":null})}
fn packet(s:Option<&SpatialSession>,c:Option<&OperationContext>,operation:&str,mut v:Value)->Result<String>{
    let mut work=empty_active_work();if let Some(w)=v.as_object_mut().and_then(|m|m.remove("_condition_watch_work")){work["obligations"]=w;}
    let mut builder=AgentTurnBuilder::new(operation,AgentPhase::Inspect).active_work(work);let mut maximum=8192;
    if let(Some(s),Some(c))=(s,c){let a=s.anchor()?;maximum=s.budget.max_bytes.min(u64::from(s.budget.max_output_tokens)*4) as usize;
        v["session_id"]=json!(s.id.to_string());v["anchor"]=anchor_json(a);let reset=v["reset"]==true;
        builder=builder.session_id(s.id.to_string()).request_id(c.request_id.to_string()).anchor(anchor_json(a)).briefing(briefing(s)).coverage(coverage())
            .continuity(if s.source.poisoned(){ContinuityStatus::Stale}else if reset{ContinuityStatus::Reset}else if v["kind"]=="heartbeat"{ContinuityStatus::Heartbeat}else{ContinuityStatus::Continuous},
                Some(anchor_json(c.anchor)),None,reset.then(||"spatial_capture_epoch_reset".to_owned()));
    }else{builder=builder.briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"bridge_protocol":"1.6"}));}
    let out=builder.attach(v);if out.len()>maximum{return Err(error(ErrorCode::BudgetExceeded,"complete spatial packet exceeds output budget"));}Ok(out)
}
fn failure(s:Option<&SpatialSession>,c:Option<&OperationContext>,operation:&str,e:&DfmcpError)->String{
    let value=json!({"ok":false,"error":{"code":e.code.as_str(),"message":e.message,"operation":operation,"mutation_dispatched":false}});
    let result=match(s,c){(Some(s),Some(c))if c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None).is_ok()=>
        semantic_query::publish_with_active_work(c,value,|v|packet(Some(s),Some(c),operation,v)),_=>packet(None,None,operation,value)};
    result.unwrap_or_else(|_|AgentTurnBuilder::new(operation,AgentPhase::Inspect).attach(json!({"ok":false,
        "error":{"code":"budget_exceeded","message":"required response did not fit; retained work is unchanged"}})))
}
fn with_session<F>(id:Option<String>,operation:&str,cap:Capability,body:F)->String where F:FnOnce(&mut SpatialSession,OperationContext)->Result<String>{
    let handle=match resolve(id){Ok(v)=>v,Err(e)=>return failure(None,None,operation,&e)};
    let mut s=match lock(&handle){Ok(v)=>v,Err(e)=>return failure(None,None,operation,&e)};
    let c=match s.context(){Ok(v)=>v,Err(e)=>return failure(None,None,operation,&e)};
    if let Err(e)=c.authorize(cap,RiskTier::ReadOnly,&[],None){return failure(None,None,operation,&e);}
    match body(&mut s,c.clone()){Ok(v)=>v,Err(e)=>{let mut current=c;if let Ok(a)=s.anchor(){current.anchor=a;}failure(Some(&s),Some(&current),operation,&e)}}
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegionInput{origin:[u32;3],size:[u32;3]}
fn parse_region(input:&Value)->Result<Region>{
    // Bound the fixed six-coordinate input before cloning it for deserialization.
    let object=input.as_object().ok_or_else(||error(ErrorCode::InvalidRequest,"region must be an object"))?;
    if object.len()!=2||!["origin","size"].iter().all(|k|object.get(*k).and_then(Value::as_array)
        .is_some_and(|a|a.len()==3&&a.iter().all(|v|v.as_u64().is_some_and(|n|n<=32768)))){
        return Err(error(ErrorCode::InvalidRequest,"region requires origin and size triples"));}
    let r:RegionInput=serde_json::from_value(input.clone()).map_err(|_|error(ErrorCode::InvalidRequest,"invalid spatial region"))?;
    let r=Region{origin:r.origin,size:r.size};r.volume().map_err(map_error)?;Ok(r)
}
fn capabilities(input:Option<Vec<String>>)->Result<Vec<Capability>>{
    let input=input.unwrap_or_else(||vec!["observe".to_owned(),"query".to_owned(),"doctor".to_owned()]);
    if input.is_empty()||input.len()>3{return Err(error(ErrorCode::CapabilityDenied,"request one to three read capabilities"));}
    let mut out=Vec::new();for n in input{let c=match n.as_str(){"observe"=>Capability::Observe,"query"=>Capability::Query,"doctor"=>Capability::Doctor,
        _=>return Err(error(ErrorCode::CapabilityDenied,"spatial profile cannot grant that capability"))};
        if out.contains(&c){return Err(error(ErrorCode::InvalidRequest,"duplicate capability"));}out.push(c);}Ok(out)
}
#[tool(description="Open an isolated read-only spatial/1.6 session: complete jobs/buildings/inventory plus one terrain region are captured together, then paged immutably. Region is {origin:[x,y,z],size:[width,height,depth]}. No game effects.")]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(region:Value,max_items:Option<u32>,max_capture_bytes:Option<u64>,page_bytes:Option<u32>,
    max_output_tokens:Option<u32>,max_wall_millis:Option<u64>,requested_capabilities:Option<Vec<String>>)->String{
    let result=(||->Result<String>{validate_environment()?;let region=parse_region(&region)?;let caps=capabilities(requested_capabilities)?;
        let journal_configuration=history::configuration()?;
        if journal_configuration.is_some() && (!caps.contains(&Capability::Query)||!caps.contains(&Capability::Observe)) {
            return Err(error(ErrorCode::CapabilityDenied,"durable spatial sessions require Query and Observe authority"));
        }
        let capture=usize::try_from(max_capture_bytes.unwrap_or(MAX_SPATIAL_BYTES as u64)).map_err(|_|error(ErrorCode::BudgetExceeded,"capture size overflow"))?;
        let limits=SpatialLimits{operations:PagedOperationsLimits{items:max_items.unwrap_or(65536),payload_bytes:capture,
            page_bytes:page_bytes.unwrap_or(65536) as usize,..PagedOperationsLimits::default()},region};limits.validate()?;
        let budget=WorkBudget{max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:max_output_tokens.unwrap_or(8192),
            max_wall_millis:max_wall_millis.unwrap_or(5000),..WorkBudget::default()};budget.validate()?;
        if !(2048..=65536).contains(&budget.max_output_tokens)||!(1..=60000).contains(&budget.max_wall_millis){return Err(error(ErrorCode::BudgetExceeded,"spatial output or wall-time budget outside bounds"));}
        let slot=Slot::reserve()?;let id=next_id()?;
        let endpoint=dfmcp_adapter::parse_loopback_endpoint(&std::env::var("DFMCP_SPATIAL_ENDPOINT").unwrap_or_else(|_|"127.0.0.1:5000".to_owned()))?;
        let token=std::env::var("DFMCP_SPATIAL_TOKEN").map_err(|_|error(ErrorCode::CapabilityDenied,"DFMCP_SPATIAL_TOKEN must be configured"))?;
        let timeout=Duration::from_millis(budget.max_wall_millis);
        let mut source=SpatialRpcClient::connect(endpoint,token.into_bytes(),id.get().to_be_bytes().to_vec(),timeout,limits)?;
        let mut state=LiveSpatialState::default();state.publish(source.refresh(timeout)?)?;
        let fortress=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial bootstrap snapshot absent"))?.fortress_id;
        let grants=caps.iter().map(|c|CapabilityGrant{capability:*c,scope:CapabilityScope{fortress_id:Some(fortress),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect();
        let mut s=SpatialSession{id,source:Box::new(source),state,limits,journal:None,budget,grants,request:0,_slot:slot};let mut c=s.context()?;
        if let Some((path,recovery))=journal_configuration {
            history::attach(&mut s,&path,recovery,&c)?;
            c.anchor=s.anchor()?;
        }
        let out=packet(Some(&s),Some(&c),"fortress.open_session",json!({"ok":true,"granted_capabilities":caps.iter().map(|c|c.as_str()).collect::<Vec<_>>(),
            "schema_discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"mode":"schema"}}}))?;
        let mut registry=lock(&SESSIONS)?;if registry.contains_key(&id){return Err(error(ErrorCode::InternalInvariantViolation,"spatial session ID collision"));}
        registry.insert(id,Arc::new(Mutex::new(s)));Ok(out)})();
    match result{Ok(v)=>v,Err(e)=>failure(None,None,"fortress.open_session",&e)}
}
fn observe(id:Option<String>,operation:&str)->String{with_session(id,operation,Capability::Observe,|s,c|{
    let outcome=s.refresh(&c)?;let mut target=c.clone();target.anchor=s.anchor()?;target.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
    let v=json!({"ok":true,"kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},"reset":outcome==JobPublication::Reset,
        "native_captures":1,"transfer_pages":s.source.pages(),"game_clock_controlled":false});
    if target.authorize(Capability::Query,RiskTier::ReadOnly,&[],None).is_ok(){semantic_query::publish_with_active_work(&target,v,|v|packet(Some(s),Some(&c),operation,v))}
    else{packet(Some(s),Some(&c),operation,v)}
})}
#[tool(description="Capture coherent operations and the fixed terrain region once, without unpausing or modifying the game.")]
pub fn fortress_observe(session_id:Option<String>)->String{observe(session_id,"fortress.observe")}
#[tool(description="Acquire one spatial capture. Query await_watch evaluates a foreground condition after that same bounded refresh.")]
pub fn fortress_wait(session_id:Option<String>)->String{observe(session_id,"fortress.wait")}
fn view(s:&SpatialSession,c:&OperationContext)->Result<QueryResponseProjection>{
    Ok(QueryResponseProjection{session_id:s.id.to_string(),request_id:c.request_id.to_string(),anchor:anchor_json(s.anchor()?),briefing:briefing(s),
        attention:Vec::new(),affordances:Vec::new(),coverage:coverage(),
        uncertainty:vec![json!({"domain":"spatial_game_feasibility","epistemic_state":"unknown","reason":"candidate routes and declared stack models do not prove unit access, recipe satisfaction or safety"})],
        budget:json!({"admitted":{"max_bytes":s.budget.max_bytes,"max_output_tokens":s.budget.max_output_tokens}}),
        references:vec![json!({"kind":"coherent_spatial_capture","digest":s.state.source_digest()?.to_string()})],
        maximum_bytes:s.budget.max_bytes.min(u64::from(s.budget.max_output_tokens)*4) as usize})
}
fn finish(view:&QueryResponseProjection,v:Value)->Result<String>{
    let mut v:Value=serde_json::from_str(&view.finish(v)?).map_err(|_|error(ErrorCode::InternalInvariantViolation,"spatial query response invalid"))?;
    v["agent_turn"]["turn_id"]=json!(format!("spatial-turn-{}",view.request_id));let out=v.to_string();
    if out.len()>view.maximum_bytes{return Err(error(ErrorCode::BudgetExceeded,"spatial query response exceeds budget"));}Ok(out)
}
#[tool(description="Query one coherent operations/terrain capture. Modes: summary, jobs, buildings, items, tiles, history, schema. Historical_query replays an exact durable record without modifying live state. Structured map_route and spatial_inventory_plan use the same capture; ordinary filters, aggregates, baselines and watches are also available.")]
pub fn fortress_query(session_id:Option<String>,mode:Option<String>,query:Option<Value>)->String{
    with_session(session_id,"fortress.query",Capability::Query,|s,mut c|{
        if mode.is_some()&&query.is_some(){return Err(error(ErrorCode::InvalidRequest,"do not combine mode and query"));}
        let schema=mode.as_deref()==Some("schema");
        let mut input=match query{Some(v)=>v,None=>match mode.as_deref(){
            None|Some("summary"|"schema")=>json!({"schema":"dfmcp.query/1","query":{"kind":"aggregate","group_by":{"kind":"entity_kind"}}}),
            Some("history")=>json!({"schema":"dfmcp.query/1","query":{"kind":"history"}}),
            Some("items")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["item"],"fields":["type_key","stack_size","container","raw_position"],"limit":1}}),
            Some("jobs")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["job"],"fields":["type_key","suspended"],"limit":1}}),
            Some("buildings")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["building"],"fields":["type_key","build_stage"],"limit":1}}),
            Some("tiles")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["tile_feature"],"fields":["position","visibility","shape"],"limit":1}}),
            _=>return Err(error(ErrorCode::InvalidRequest,"unknown spatial query mode")),}};
        let kind=input.get("query").and_then(|v|v.get("kind")).and_then(Value::as_str).unwrap_or("");
        if !schema && history::handles(&input) {
            let result=history::execute(s,&c,&input);
            if matches!(&result,Err(e) if e.code==ErrorCode::CorruptLedger) {s.source.fence();}
            return result;
        }
        let local=matches!(kind,"watches"|"cancel_watch"|"release_watch"|"baselines"|"release_baseline");
        if s.source.poisoned()&&!local&&!schema{return Err(error(ErrorCode::AdapterUnavailable,"spatial source fenced; reopen or manage local records"));}
        let mut refresh=None;
        if !schema&&kind=="await_watch"{
            let snapshot=s.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial snapshot absent"))?;
            if semantic_query::prepare_await(snapshot,&c,&input)?{
                let basis=c.anchor;let outcome=s.refresh(&c)?;c.anchor=s.anchor()?;
                c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;c.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
                refresh=Some(json!({"basis":anchor_json(basis),"reset":outcome==JobPublication::Reset,"kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},
                    "native_captures":1,"transfer_pages":s.source.pages()}));
            }
            if let Some(obj)=input.as_object_mut(){obj.remove("expected_anchor");}input["query"]["kind"]=json!("poll_watch");
        }
        let view=view(s,&c)?;let mut narrowed=c.clone();narrowed.budget.max_bytes=view.result_byte_budget()? as u64;
        if schema{return semantic_query::publish_with_active_work(&c,json!({"query_schema":history::schema()?,"mode":"schema",
            "profile":"spatial/1.6","source_stale":s.source.poisoned(),"truncated":false,"continuation":null}),|v|finish(&view,v));}
        if spatial_queries::handles(&input){let rc=semantic_query::result_context(&narrowed)?;let v=spatial_queries::execute(&s.state,&rc,&input)?;
            return semantic_query::publish_with_active_work(&narrowed,v,|v|finish(&view,v));}
        let snapshot=s.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial snapshot absent"))?;
        semantic_query::execute_with_publisher(snapshot,&narrowed,&input,|mut v|{
            v["source_stale"]=json!(s.source.poisoned());if let Some(r)=refresh{v["observation_refresh"]=r;}finish(&view,v)
        })
    })
}
#[tool(description="Explain coherent spatial coverage and the difference between observed candidate routes and actual game feasibility.")]
pub fn fortress_explain(session_id:Option<String>)->String{with_session(session_id,"fortress.explain",Capability::Query,|s,c|
    semantic_query::publish_with_active_work(&c,json!({"ok":true,"coherence":"one native capture of operations plus requested terrain",
        "allocation":"declared interchangeable stack units whose outermost ground container has a candidate route from the requested origin",
        "unknown":["full unit navigation","native job requirement satisfaction","outside-region routes","safety","citizen state"],
        "history":"optional fixed-profile archive; history lists captures and historical_query reads an exact past record; no cross-profile import"}),|v|packet(Some(s),Some(&c),"fortress.explain",v)))}
#[tool(description="Report source health; does not reconnect or imply native qualification.")]
pub fn fortress_doctor(session_id:Option<String>)->String{with_session(session_id,"fortress.doctor",Capability::Doctor,|s,c|
    packet(Some(s),Some(&c),"fortress.doctor",json!({"ok":true,"status":if s.source.poisoned(){"source_fenced"}else{"read_only_unadmitted"}})))}
fn no_effect(id:Option<String>,operation:&str)->String{with_session(id,operation,Capability::Query,|_,_|Err(error(ErrorCode::CapabilityDenied,"spatial/1.6 has no live mutation, reservation, game checkpoint or restore path")))}
#[tool(description="Unavailable: spatial observations and allocations cannot authorize game effects.")]
pub fn fortress_plan(session_id:Option<String>)->String{no_effect(session_id,"fortress.plan")}
#[tool(description="Unavailable: spatial allocation results are not executable plans.")]
pub fn fortress_commit(session_id:Option<String>)->String{no_effect(session_id,"fortress.commit")}
#[tool(description="Unavailable for game effects. Local condition watches use query cancel_watch.")]
pub fn fortress_cancel(session_id:Option<String>)->String{no_effect(session_id,"fortress.cancel")}
#[tool(description="Unavailable: no game-save checkpoint is created.")]
pub fn fortress_checkpoint(session_id:Option<String>)->String{no_effect(session_id,"fortress.checkpoint")}
#[tool(description="Unavailable: no game or save state is restored.")]
pub fn fortress_restore(session_id:Option<String>)->String{no_effect(session_id,"fortress.restore")}

pub fn run_stdio(){
    if let Err(e)=validate_environment(){eprintln!("{e}");std::process::exit(1);}
    let server=ServerBuilder::new("dfmcp-live-spatial-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .request_timeout(60).instructions("Unadmitted read-only spatial/1.6. Open a fixed-region session first. Operations and terrain share one immutable native capture. Use schema for typed queries; spatial_inventory_plan allocates declared units within the observed route model, not native feasibility. map_route provides candidate paths only. Baselines and foreground watches use the same combined anchor. Capture time is not transfer completion. With an operator-configured spatial journal, history and historical_query inspect committed captures without changing current watches. No citizens, movement, reservations or live game effects.").build();
    crate::run_modern_stdio(server);
}
#[cfg(test)]
#[path="live_spatial_server_tests.rs"]
mod tests;
