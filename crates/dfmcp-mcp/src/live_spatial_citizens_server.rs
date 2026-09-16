//! Explicitly unadmitted spatial/1.8 runtime. Strict citizens, operations and
//! bounded terrain arrive in one immutable native capture.
#[path="semantic_query.rs"] mod semantic_query;
#[path="query_response.rs"] mod query_response;
#[path="spatial_queries.rs"] mod spatial_queries;
#[path="spatial_citizen_history.rs"] mod history;
#[path="spatial_watch_runtime.rs"] mod durable_watches;

use std::collections::BTreeMap;
use std::sync::{Arc,LazyLock,Mutex,MutexGuard};
use std::sync::atomic::{AtomicUsize,Ordering};
use std::time::Duration;
use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_jobs_rpc::{DeadlineStream,operations::{paged::PagedOperationsLimits,map::spatial::{SpatialLimits,citizens::{CitizenSpatialLimits,CitizenSpatialRpcClient}}}};
use dfmcp_adapter::live_map::map_error;
use dfmcp_adapter::live_spatial::{SpatialStateView,citizens::{LiveSpatialCitizenObservation,LiveSpatialCitizenState,MAX_SPATIAL_CITIZEN_BYTES}};
use dfmcp_world::map_region::Region;
use dfmcp_core::{Capability,CapabilityGrant,CapabilityScope,DfmcpError,ErrorCode,OperationContext,
    RequestId,Result,RiskTier,SessionId,StateAnchor,WorkBudget};
use crate::agent_turn::{AgentPhase,AgentTurnBuilder,ContinuityStatus,empty_active_work};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value,json};
use query_response::QueryResponseProjection;

const FAMILY:u128=1u128<<56;
static NEXT:Mutex<u128>=Mutex::new(1);
static SLOTS:AtomicUsize=AtomicUsize::new(0);
static SESSIONS:LazyLock<Mutex<BTreeMap<SessionId,Arc<Mutex<Session>>>>>=LazyLock::new(||Mutex::new(BTreeMap::new()));
fn error(code:ErrorCode,text:&str)->DfmcpError{DfmcpError::new(code,text)}
fn lock<T>(m:&Mutex<T>)->Result<MutexGuard<'_,T>>{m.lock().map_err(|_|error(ErrorCode::InternalInvariantViolation,"spatial/1.8 mutex poisoned"))}
struct Slot;
impl Slot{fn reserve()->Result<Self>{SLOTS.fetch_update(Ordering::AcqRel,Ordering::Acquire,|n|(n<2).then_some(n+1))
    .map_err(|_|error(ErrorCode::BudgetExceeded,"spatial/1.8 retains at most two sessions"))?;Ok(Self)}}
impl Drop for Slot{fn drop(&mut self){SLOTS.fetch_sub(1,Ordering::AcqRel);}}
trait Source:Send{fn read(&mut self,timeout:Duration)->Result<LiveSpatialCitizenObservation>;fn poisoned(&self)->bool;fn fence(&mut self);fn pages(&self)->u32;}
impl Source for CitizenSpatialRpcClient<DeadlineStream>{
    fn read(&mut self,t:Duration)->Result<LiveSpatialCitizenObservation>{self.refresh(t)}
    fn poisoned(&self)->bool{CitizenSpatialRpcClient::poisoned(self)}fn fence(&mut self){CitizenSpatialRpcClient::fence(self)}
    fn pages(&self)->u32{self.last_page_count()}
}
struct Session{id:SessionId,source:Box<dyn Source>,state:LiveSpatialCitizenState,limits:CitizenSpatialLimits,
    journal:Option<history::Journal>,budget:WorkBudget,grants:Vec<CapabilityGrant>,request:u128,
    _watch_journal:Option<semantic_query::WatchJournalGuard>,_slot:Slot}
impl Session{
    fn anchor(&self)->Result<StateAnchor>{self.state.snapshot().map(|s|s.anchor()).ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial/1.8 snapshot absent"))}
    fn context(&mut self)->Result<OperationContext>{self.request=self.request.checked_add(1).ok_or_else(||error(ErrorCode::BudgetExceeded,"spatial/1.8 request IDs exhausted"))?;
        Ok(OperationContext{session_id:self.id,request_id:RequestId::new(self.request),anchor:self.anchor()?,budget:self.budget,
            grants:self.grants.clone(),cancellation_requested:false})}
    fn refresh(&mut self,c:&OperationContext)->Result<JobPublication>{
        c.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;if c.anchor!=self.anchor()?{return Err(error(ErrorCode::StaleAnchor,"spatial/1.8 refresh anchor changed"));}
        if self.source.poisoned(){return Err(error(ErrorCode::AdapterUnavailable,"spatial/1.8 source fenced; reopen session"));}
        if let Some(journal)=&self.journal{
            c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
            if journal.fenced()||journal.state().snapshot().map(|s|s.anchor())!=Some(c.anchor){self.source.fence();
                return Err(error(ErrorCode::CorruptLedger,"spatial/1.8 journal is fenced or disagrees with published anchor"));}
        }
        let replay=history::replay_context(self,c);
        let result=self.source.read(Duration::from_millis(c.budget.max_wall_millis)).and_then(|value|{
            let op=value.spatial().operations();let limits=self.limits.spatial.operations;
            if op.jobs.jobs.len()>limits.jobs as usize||op.buildings.len()>limits.buildings as usize||op.items.len()>limits.items as usize
                ||value.citizens().len()>self.limits.citizens as usize||value.spatial().terrain().map.region!=self.limits.spatial.region
                ||value.encode_payload()?.len()>limits.payload_bytes{return Err(error(ErrorCode::BudgetExceeded,"spatial/1.8 observation exceeds negotiated bounds"));}
            match self.journal.as_mut(){
                Some(journal)=>{let outcome=journal.append(value,&replay)?;self.state=journal.state().clone();Ok(outcome)}
                None=>self.state.publish(value),
            }
        });if result.is_err(){self.source.fence();}result
    }
}
fn next_id()->Result<SessionId>{let mut n=lock(&NEXT)?;if *n>=FAMILY{return Err(error(ErrorCode::BudgetExceeded,"spatial/1.8 session IDs exhausted"));}
    let id=SessionId::new((1u128<<127)|FAMILY|*n);*n+=1;Ok(id)}
fn resolve(raw:Option<String>)->Result<Arc<Mutex<Session>>>{let text=raw.ok_or_else(||error(ErrorCode::InvalidRequest,"open a spatial/1.8 session first"))?;
    if text.len()!=32||!text.bytes().all(|b|b.is_ascii_hexdigit()){return Err(error(ErrorCode::InvalidRequest,"invalid spatial/1.8 session"));}
    let raw=u128::from_str_radix(&text,16).map_err(|_|error(ErrorCode::InvalidRequest,"invalid spatial/1.8 session"))?;let id=SessionId::new(raw);
    if id.get()!=raw||!id.is_process_scoped_live()||(raw&((1u128<<62)-1))>>56!=1{return Err(error(ErrorCode::InvalidRequest,"not a spatial/1.8 session"));}
    lock(&SESSIONS)?.get(&id).cloned().ok_or_else(||error(ErrorCode::SessionNotFound,"spatial/1.8 session not found"))}
fn allowed_environment(name:&str)->bool{!name.starts_with("DFMCP_")||matches!(name,
    "DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_8"|"DFMCP_SPATIAL_CITIZEN_TOKEN"|"DFMCP_SPATIAL_CITIZEN_ENDPOINT"|
    "DFMCP_SPATIAL_CITIZEN_JOURNAL"|"DFMCP_SPATIAL_CITIZEN_JOURNAL_REPAIR"|"DFMCP_SPATIAL_CITIZEN_WATCH_JOURNAL")}
fn validate_environment()->Result<()>{if std::env::var("DFMCP_ALLOW_UNADMITTED_SPATIAL_V1_8").ok().as_deref()!=Some("1")
    ||std::env::vars_os().any(|(n,_)|!allowed_environment(&n.to_string_lossy()))||crate::admission::current_admission_provenance().is_some(){
        return Err(error(ErrorCode::CapabilityDenied,"spatial/1.8 requires its own opt-in and refuses other DFMCP/admission state"));}Ok(())}
fn anchor_json(a:StateAnchor)->Value{json!({"fortress_id":a.fortress_id.to_string(),"epoch":a.cursor.epoch,"sequence":a.cursor.sequence,
    "game_tick":a.tick.0,"state_hash":a.state_hash.to_string()})}
fn briefing(s:&Session)->Value{let full=s.state.observation_full();let base=full.map(|v|v.spatial());
    json!({"runtime":"unadmitted_development","bridge_protocol":"1.8","observation_profile":"coherent-citizen-spatial",
        "runtime_admitted":false,"mutation_admissible":false,"read_only":true,"source_poisoned":s.source.poisoned(),
        "strict_citizen_count":full.map(|v|v.citizens().len()),"job_count":base.map(|v|v.operations().jobs.jobs.len()),
        "building_count":base.map(|v|v.operations().buildings.len()),"item_count":base.map(|v|v.operations().items.len()),
        "region":{"origin":s.limits.spatial.region.origin,"size":s.limits.spatial.region.size},
        "observation_history":history::summary(s.journal.as_ref()),"last_transfer_pages":s.source.pages(),
        "freshness":"capture time, not page-transfer completion"})}
fn coverage()->Value{json!({"status":"partial","complete_domains":["fortress.citizens.strict_roster","current_job_roster","building_roster",
    "item_roster","job_item_attachments","requested_region.cell_presence"],
    "partial_domains":[{"domain":"fortress.spatial","reason":"one coherent capture; hidden terrain redacted and route model deliberately restricted"}],
    "omitted_domains":["noncitizen_units","outside_region_terrain","full_unit_navigation_rules","citizen_skills_needs_health","native_material_requirements","continuous_game_history"],"continuation":null})}
fn packet(s:Option<&Session>,c:Option<&OperationContext>,operation:&str,mut v:Value)->Result<String>{
    let mut work=empty_active_work();if let Some(w)=v.as_object_mut().and_then(|m|m.remove("_condition_watch_work")){work["obligations"]=w;}
    let mut builder=AgentTurnBuilder::new(operation,AgentPhase::Inspect).active_work(work);let mut maximum=8192;
    if let(Some(s),Some(c))=(s,c){let a=s.anchor()?;maximum=s.budget.max_bytes.min(u64::from(s.budget.max_output_tokens)*4) as usize;v["session_id"]=json!(s.id.to_string());v["anchor"]=anchor_json(a);
        let reset=v["reset"]==true;
        let opening=operation=="fortress.open_session";
        let recovered=v.get("watch_recovery").and_then(|r|r.get("restored")).and_then(Value::as_u64).is_some_and(|n|n>0);
        let continuity=if s.source.poisoned(){ContinuityStatus::Stale}else if reset{ContinuityStatus::Reset}
            else if recovered{ContinuityStatus::Partial}else if opening{ContinuityStatus::Bootstrap}
            else if v["kind"]=="heartbeat"{ContinuityStatus::Heartbeat}else{ContinuityStatus::Continuous};
        builder=builder.session_id(s.id.to_string()).request_id(c.request_id.to_string()).anchor(anchor_json(a)).briefing(briefing(s)).coverage(coverage())
            .continuity(continuity,if opening{None}else{Some(anchor_json(c.anchor))},
                recovered.then(||json!({"reason":"watch_monitoring_gap_during_process_downtime","stability_reset":true})),
                reset.then(||"citizen_spatial_capture_epoch_reset".to_owned()));
    }else{builder=builder.briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"bridge_protocol":"1.8"}));}
    let out=builder.attach(v);if out.len()>maximum{return Err(error(ErrorCode::BudgetExceeded,"complete spatial/1.8 packet exceeds output budget"));}Ok(out)
}
fn failure(s:Option<&Session>,c:Option<&OperationContext>,operation:&str,e:&DfmcpError)->String{
    let value=json!({"ok":false,"error":{"code":e.code.as_str(),"message":e.message,"operation":operation,"mutation_dispatched":false}});
    let result=match(s,c){(Some(s),Some(c))if c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None).is_ok()=>
        semantic_query::publish_with_active_work(c,value.clone(),|v|packet(Some(s),Some(c),operation,v)),_=>packet(None,None,operation,value.clone())};
    // A fenced watch journal must not turn its own diagnosis into BudgetExceeded
    // merely because the usual active-work projection is now unavailable.
    result.unwrap_or_else(|_|AgentTurnBuilder::new(operation,AgentPhase::Inspect)
        .briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"bridge_protocol":"1.8","active_work_unavailable":true}))
        .attach(value))
}
fn with_session<F>(id:Option<String>,operation:&str,cap:Capability,body:F)->String where F:FnOnce(&mut Session,OperationContext)->Result<String>{
    let handle=match resolve(id){Ok(v)=>v,Err(e)=>return failure(None,None,operation,&e)};let mut s=match lock(&handle){Ok(v)=>v,Err(e)=>return failure(None,None,operation,&e)};
    let c=match s.context(){Ok(v)=>v,Err(e)=>return failure(None,None,operation,&e)};if let Err(e)=c.authorize(cap,RiskTier::ReadOnly,&[],None){return failure(None,None,operation,&e);}
    match body(&mut s,c.clone()){Ok(v)=>v,Err(e)=>{let mut current=c;if let Ok(a)=s.anchor(){current.anchor=a;}failure(Some(&s),Some(&current),operation,&e)}}
}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct RegionInput{origin:[u32;3],size:[u32;3]}
fn parse_region(input:&Value)->Result<Region>{let object=input.as_object().ok_or_else(||error(ErrorCode::InvalidRequest,"region must be an object"))?;
    if object.len()!=2||!["origin","size"].iter().all(|k|object.get(*k).and_then(Value::as_array).is_some_and(|a|a.len()==3&&a.iter().all(|v|v.as_u64().is_some_and(|n|n<=32768)))){
        return Err(error(ErrorCode::InvalidRequest,"region requires bounded origin and size triples"));}
    let r:RegionInput=serde_json::from_value(input.clone()).map_err(|_|error(ErrorCode::InvalidRequest,"invalid spatial/1.8 region"))?;
    let r=Region{origin:r.origin,size:r.size};r.volume().map_err(map_error)?;Ok(r)}
fn capabilities(input:Option<Vec<String>>)->Result<Vec<Capability>>{let input=input.unwrap_or_else(||vec!["observe".to_owned(),"query".to_owned(),"doctor".to_owned()]);
    if input.is_empty()||input.len()>3{return Err(error(ErrorCode::CapabilityDenied,"request one to three read capabilities"));}
    let mut out=Vec::new();for name in input{let c=match name.as_str(){"observe"=>Capability::Observe,"query"=>Capability::Query,"doctor"=>Capability::Doctor,
        _=>return Err(error(ErrorCode::CapabilityDenied,"spatial/1.8 cannot grant that capability"))};if out.contains(&c){return Err(error(ErrorCode::InvalidRequest,"duplicate capability"));}out.push(c);}Ok(out)}

#[tool(description="Open an unadmitted read-only spatial/1.8 session: strict citizens, jobs, buildings, inventory and one terrain region are captured together and paged immutably. Operator-configured paired observation/watch journals restore monitoring intent with fresh handles and reset stability after restart.")]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(region:Value,max_citizens:Option<u32>,max_items:Option<u32>,max_capture_bytes:Option<u64>,page_bytes:Option<u32>,
    max_output_tokens:Option<u32>,max_wall_millis:Option<u64>,requested_capabilities:Option<Vec<String>>)->String{
    let result=(||->Result<String>{validate_environment()?;let region=parse_region(&region)?;let caps=capabilities(requested_capabilities)?;
        let journal_configuration=history::configuration()?;
        let watch_path=durable_watches::configuration(journal_configuration.as_ref().map(|(path,_)|path.as_path()))?;
        if journal_configuration.is_some()&&(!caps.contains(&Capability::Query)||!caps.contains(&Capability::Observe)){
            return Err(error(ErrorCode::CapabilityDenied,"durable spatial/1.8 sessions require Query and Observe authority"));}
        let capture=usize::try_from(max_capture_bytes.unwrap_or(MAX_SPATIAL_CITIZEN_BYTES as u64)).map_err(|_|error(ErrorCode::BudgetExceeded,"capture size overflow"))?;
        let spatial=SpatialLimits{operations:PagedOperationsLimits{items:max_items.unwrap_or(65536),payload_bytes:capture,
            page_bytes:page_bytes.unwrap_or(65536) as usize,..PagedOperationsLimits::default()},region};
        let limits=CitizenSpatialLimits{spatial,citizens:max_citizens.unwrap_or(4096)};limits.validate()?;
        let budget=WorkBudget{max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:max_output_tokens.unwrap_or(8192),
            max_wall_millis:max_wall_millis.unwrap_or(5000),..WorkBudget::default()};budget.validate()?;
        if !(2048..=65536).contains(&budget.max_output_tokens)||!(1..=60000).contains(&budget.max_wall_millis){return Err(error(ErrorCode::BudgetExceeded,"spatial/1.8 output or wall budget outside bounds"));}
        let slot=Slot::reserve()?;let id=next_id()?;
        let endpoint=dfmcp_adapter::parse_loopback_endpoint(&std::env::var("DFMCP_SPATIAL_CITIZEN_ENDPOINT").unwrap_or_else(|_|"127.0.0.1:5000".to_owned()))?;
        let token=std::env::var("DFMCP_SPATIAL_CITIZEN_TOKEN").map_err(|_|error(ErrorCode::CapabilityDenied,"DFMCP_SPATIAL_CITIZEN_TOKEN required"))?;
        let timeout=Duration::from_millis(budget.max_wall_millis);let mut source=CitizenSpatialRpcClient::connect(endpoint,token.into_bytes(),id.get().to_be_bytes().to_vec(),timeout,limits)?;
        let mut state=LiveSpatialCitizenState::default();state.publish(source.refresh(timeout)?)?;
        let fortress=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial/1.8 bootstrap snapshot absent"))?.fortress_id;
        let grants=caps.iter().map(|c|CapabilityGrant{capability:*c,scope:CapabilityScope{fortress_id:Some(fortress),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect();
        let mut s=Session{id,source:Box::new(source),state,limits,journal:None,budget,grants,request:0,_watch_journal:None,_slot:slot};let mut c=s.context()?;
        if let Some((path,recovery))=journal_configuration{history::attach(&mut s,&path,recovery,&c)?;c.anchor=s.anchor()?;}
        let value=json!({"ok":true,"granted_capabilities":caps.iter().map(|c|c.as_str()).collect::<Vec<_>>(),
            "schema_discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"mode":"schema"}}});
        let out=durable_watches::finish_open(&mut s,&c,watch_path.as_deref(),value)?;
        lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(s)));Ok(out)})();match result{Ok(v)=>v,Err(e)=>failure(None,None,"fortress.open_session",&e)}}
fn observe(id:Option<String>,operation:&str)->String{with_session(id,operation,Capability::Observe,|s,c|{let outcome=s.refresh(&c)?;let mut target=c.clone();target.anchor=s.anchor()?;
    target.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;let v=json!({"ok":true,"kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},
        "reset":outcome==JobPublication::Reset,"native_captures":1,"transfer_pages":s.source.pages(),"game_clock_controlled":false});
    if target.authorize(Capability::Query,RiskTier::ReadOnly,&[],None).is_ok(){semantic_query::publish_with_active_work(&target,v,|v|packet(Some(s),Some(&c),operation,v))}else{packet(Some(s),Some(&c),operation,v)}})}
#[tool(description="Acquire one new coherent citizens/operations/terrain capture without modifying game time or state.")]pub fn fortress_observe(session_id:Option<String>)->String{observe(session_id,"fortress.observe")}
#[tool(description="Acquire one coherent capture. Use query await_watch for a sampled foreground condition.")]pub fn fortress_wait(session_id:Option<String>)->String{observe(session_id,"fortress.wait")}
fn view(s:&Session,c:&OperationContext)->Result<QueryResponseProjection>{Ok(QueryResponseProjection{session_id:s.id.to_string(),request_id:c.request_id.to_string(),
    anchor:anchor_json(s.anchor()?),briefing:briefing(s),attention:Vec::new(),affordances:Vec::new(),coverage:coverage(),
    uncertainty:vec![json!({"domain":"unit_navigation_and_labor","epistemic_state":"partial","reason":"citizen identity/position/status are observed; skills, needs and full movement rules are not"})],
    budget:json!({"admitted":{"max_bytes":s.budget.max_bytes,"max_output_tokens":s.budget.max_output_tokens}}),
    references:vec![json!({"kind":"coherent_citizen_spatial_capture","digest":s.state.source_digest()?.to_string()})],
    maximum_bytes:s.budget.max_bytes.min(u64::from(s.budget.max_output_tokens)*4) as usize})}
fn finish(view:&QueryResponseProjection,v:Value)->Result<String>{let mut v:Value=serde_json::from_str(&view.finish(v)?).map_err(|_|error(ErrorCode::InternalInvariantViolation,"spatial/1.8 response invalid"))?;
    v["agent_turn"]["turn_id"]=json!(format!("spatial-citizen-turn-{}",view.request_id));let out=v.to_string();if out.len()>view.maximum_bytes{return Err(error(ErrorCode::BudgetExceeded,"spatial/1.8 query exceeds budget"));}Ok(out)}
#[tool(description="Query one coherent citizen/operations/terrain capture. Modes: summary, citizens, jobs, buildings, items, tiles, history, schema. With paired operator journals, watch registration, samples, cancellation and release survive restart. Historical_query replays an exact record without changing live watches. Structured graph, watch, map_route and spatial_inventory_plan queries share the same anchor.")]
pub fn fortress_query(session_id:Option<String>,mode:Option<String>,query:Option<Value>)->String{with_session(session_id,"fortress.query",Capability::Query,|s,mut c|{
    if mode.is_some()&&query.is_some(){return Err(error(ErrorCode::InvalidRequest,"do not combine mode and query"));}let schema=mode.as_deref()==Some("schema");
    let mut input=match query{Some(v)=>v,None=>match mode.as_deref(){
        None|Some("summary"|"schema")=>json!({"schema":"dfmcp.query/1","query":{"kind":"aggregate","group_by":{"kind":"entity_kind"}}}),
        Some("history")=>json!({"schema":"dfmcp.query/1","query":{"kind":"history"}}),
        Some("citizens")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["unit"],"fields":["name","profession","position","alive","active"],"limit":1}}),
        Some("items")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["item"],"fields":["type_key","stack_size","raw_position"],"limit":1}}),
        Some("jobs")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["job"],"fields":["type_key","suspended","worker_entity","worker_is_strict_citizen"],"limit":1}}),
        Some("buildings")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["building"],"fields":["type_key","build_stage"],"limit":1}}),
        Some("tiles")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["tile_feature"],"fields":["position","visibility","shape"],"limit":1}}),
        _=>return Err(error(ErrorCode::InvalidRequest,"unknown spatial/1.8 query mode")),}};
    let kind=input.get("query").and_then(|v|v.get("kind")).and_then(Value::as_str).unwrap_or("");
    if !schema&&history::handles(&input){let result=history::execute(s,&c,&input);if matches!(&result,Err(e)if e.code==ErrorCode::CorruptLedger){s.source.fence();}return result;}
    let local=matches!(kind,"watches"|"cancel_watch"|"release_watch"|"baselines"|"release_baseline");
    if s.source.poisoned()&&!local&&!schema{return Err(error(ErrorCode::AdapterUnavailable,"spatial/1.8 source fenced; reopen or manage local records"));}
    let mut refresh=None;if !schema&&kind=="await_watch"{let snapshot=s.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"snapshot absent"))?;
        if semantic_query::prepare_await(snapshot,&c,&input)?{let basis=c.anchor;let outcome=s.refresh(&c)?;c.anchor=s.anchor()?;c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;c.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
            refresh=Some(json!({"basis":anchor_json(basis),"reset":outcome==JobPublication::Reset,"kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},"native_captures":1,"transfer_pages":s.source.pages()}));}
        if let Some(obj)=input.as_object_mut(){obj.remove("expected_anchor");}input["query"]["kind"]=json!("poll_watch");}
    let view=view(s,&c)?;let mut narrowed=c.clone();narrowed.budget.max_bytes=view.result_byte_budget()? as u64;
    if schema{return semantic_query::publish_with_active_work(&c,json!({"query_schema":history::schema()?,"mode":"schema","profile":"spatial/1.8","source_stale":s.source.poisoned(),"truncated":false,"continuation":null}),|v|finish(&view,v));}
    if spatial_queries::handles(&input){let rc=semantic_query::result_context(&narrowed)?;let v=spatial_queries::execute(&s.state,&rc,&input)?;return semantic_query::publish_with_active_work(&narrowed,v,|v|finish(&view,v));}
    let snapshot=s.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial/1.8 snapshot absent"))?;
    semantic_query::execute_with_publisher(snapshot,&narrowed,&input,|mut v|{v["source_stale"]=json!(s.source.poisoned());if let Some(r)=refresh{v["observation_refresh"]=r;}finish(&view,v)})})}
#[tool(description="Explain coherent citizen/job assignment, spatial coverage and optional exact-record history. No effect authority is inferred.")]
pub fn fortress_explain(session_id:Option<String>)->String{with_session(session_id,"fortress.explain",Capability::Query,|s,c|
    semantic_query::publish_with_active_work(&c,json!({"ok":true,"coherence":"strict citizens, operations and requested terrain are one native capture",
        "worker_join":"observed strict-citizen workers have generation-checked unit entities and performs edges to jobs",
        "location_join":"citizens and jobs inside the captured region have observed located_at edges to physical tile entities; this is not path feasibility",
        "history":"optional fixed-profile spatial/1.8 archive; history and historical_query never import another profile or mutate current watches",
        "unknown":["noncitizen unit details","skills","needs","labor eligibility","full unit pathfinding","outside-region routes"]}),|v|packet(Some(s),Some(&c),"fortress.explain",v)))}
#[tool(description="Report spatial/1.8 source health; no reconnect or qualification claim.")]pub fn fortress_doctor(session_id:Option<String>)->String{with_session(session_id,"fortress.doctor",Capability::Doctor,|s,c|
    packet(Some(s),Some(&c),"fortress.doctor",json!({"ok":true,"status":if s.source.poisoned(){"source_fenced"}else{"read_only_unadmitted"}})))}
fn no_effect(id:Option<String>,operation:&str)->String{with_session(id,operation,Capability::Query,|_,_|Err(error(ErrorCode::CapabilityDenied,"spatial/1.8 has no live mutation or reservation path")))}
#[tool(description="Unavailable: coherent observations do not authorize effects.")]pub fn fortress_plan(session_id:Option<String>)->String{no_effect(session_id,"fortress.plan")}
#[tool(description="Unavailable: no executable plan is created.")]pub fn fortress_commit(session_id:Option<String>)->String{no_effect(session_id,"fortress.commit")}
#[tool(description="Unavailable for game effects.")]pub fn fortress_cancel(session_id:Option<String>)->String{no_effect(session_id,"fortress.cancel")}
#[tool(description="Unavailable: no game-save checkpoint is created.")]pub fn fortress_checkpoint(session_id:Option<String>)->String{no_effect(session_id,"fortress.checkpoint")}
#[tool(description="Unavailable: no game or save state is restored.")]pub fn fortress_restore(session_id:Option<String>)->String{no_effect(session_id,"fortress.restore")}

pub fn run_stdio(){if let Err(e)=validate_environment(){eprintln!("{e}");std::process::exit(1);}let server=ServerBuilder::new("dfmcp-live-spatial-citizens-dev",env!("CARGO_PKG_VERSION"))
    .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
    .request_timeout(60).instructions("Unadmitted read-only spatial/1.8. Strict citizens, jobs, buildings, items and one terrain region share one immutable native capture. Use unit/job performs and located_at edges for observed assignment/location; route/allocation results remain model-only. Optional fixed-profile history replays exact captures. Paired operator-configured watch journals preserve monitoring definitions and outcomes; restart gives fresh handles, resets unfinished stability and never proves continuity during downtime. Rediscover with query watches, then await_watch for fresh evidence. No game effects.").build();crate::run_modern_stdio(server);}
