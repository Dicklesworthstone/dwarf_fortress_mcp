//! Shared explicitly unadmitted operations handlers. Fixed bootstrap entries
//! choose 1.3 single-frame or 1.4 immutable-page acquisition, never agent queries.

#[path = "semantic_query.rs"]
mod semantic_query;
#[path = "query_response.rs"]
mod query_response;
#[path = "operations_production.rs"]
mod production;
#[path = "operations_history.rs"]
mod history;
#[path = "live_operations_paged_server.rs"]
pub mod paged;

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use dfmcp_adapter::live_jobs::JobPublication;
use dfmcp_adapter::live_jobs_rpc::{DeadlineStream, operations::{OperationsLimits, OperationsRpcClient}};
use dfmcp_adapter::live_operations::{LiveOperationsObservation, LiveOperationsState, OperationsProfile, MAX_OPERATIONS_BYTES};
use dfmcp_core::{Capability, CapabilityGrant, CapabilityScope, DfmcpError, ErrorCode,
    OperationContext, RequestId, Result, RiskTier, SessionId, StateAnchor, WorkBudget};
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, empty_active_work};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};
use query_response::QueryResponseProjection;

const MAX_SESSIONS: usize = 8;
const FAMILY: u128 = 2u128 << 60;
static NEXT: LazyLock<Mutex<u128>> = LazyLock::new(|| Mutex::new(1));
static SLOTS: AtomicUsize = AtomicUsize::new(0);
static SESSIONS: LazyLock<Mutex<BTreeMap<SessionId, Arc<Mutex<OperationsSession>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn error(code: ErrorCode, text: &str) -> DfmcpError { DfmcpError::new(code, text) }
fn lock<T>(value: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    value.lock().map_err(|_| error(ErrorCode::InternalInvariantViolation, "operations mutex is poisoned"))
}
struct SessionSlot;
impl SessionSlot {
    fn reserve() -> Result<Self> {
        SLOTS.fetch_update(Ordering::AcqRel, Ordering::Acquire,
            |value| (value < MAX_SESSIONS).then_some(value + 1))
            .map_err(|_| error(ErrorCode::BudgetExceeded, "operations session capacity reached"))?;
        Ok(Self)
    }
}
impl Drop for SessionSlot { fn drop(&mut self) { SLOTS.fetch_sub(1, Ordering::AcqRel); } }

trait OperationsSource: Send {
    fn read(&mut self, timeout: Duration) -> Result<LiveOperationsObservation>;
    fn poisoned(&self) -> bool;
    fn fence(&mut self);
}
impl OperationsSource for OperationsRpcClient<DeadlineStream> {
    fn read(&mut self, timeout: Duration) -> Result<LiveOperationsObservation> { self.refresh(timeout) }
    fn poisoned(&self) -> bool { OperationsRpcClient::poisoned(self) }
    fn fence(&mut self) { OperationsRpcClient::fence(self); }
}
struct OperationsSession {
    id: SessionId,
    source: Box<dyn OperationsSource>,
    state: LiveOperationsState,
    journal: Option<dfmcp_adapter::operations_journal::OperationsJournal<dfmcp_adapter::operations_journal::PrivateJournalFile>>,
    limits: OperationsLimits,
    budget: WorkBudget,
    grants: Vec<CapabilityGrant>,
    request: u128,
    _slot: SessionSlot,
}
impl OperationsSession {
    fn anchor(&self) -> Result<StateAnchor> {
        self.state.snapshot().map(|snapshot| snapshot.anchor())
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "operations snapshot missing"))
    }
    fn context(&mut self) -> Result<OperationContext> {
        self.request = self.request.checked_add(1)
            .ok_or_else(|| error(ErrorCode::BudgetExceeded, "operations request IDs exhausted"))?;
        Ok(OperationContext { session_id: self.id, request_id: RequestId::new(self.request),
            anchor: self.anchor()?, budget: self.budget, grants: self.grants.clone(), cancellation_requested: false })
    }
    fn refresh(&mut self, context: &OperationContext) -> Result<JobPublication> {
        context.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
        if context.anchor != self.anchor()? { return Err(error(ErrorCode::StaleAnchor, "operations refresh is stale")); }
        if self.source.poisoned() { return Err(error(ErrorCode::AdapterUnavailable, "operations source fenced; reopen session")); }
        if let Some(journal) = &self.journal {
            context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            if self.state.profile() != OperationsProfile::V1_3 || journal.fenced()
                || journal.state().snapshot().map(|s|s.anchor()) != Some(context.anchor) {
                return Err(error(ErrorCode::CorruptLedger, "journal is fenced or disagrees with the published session profile or anchor"));
            }
        }
        let result = self.source.read(Duration::from_millis(context.budget.max_wall_millis))
            .and_then(|value| {
                if value.jobs.jobs.len() > self.limits.jobs as usize
                    || value.buildings.len() > self.limits.buildings as usize || value.items.len() > self.limits.items as usize
                    || value.encode_profile(self.state.profile())?.len() > self.limits.payload_bytes {
                    return Err(error(ErrorCode::BudgetExceeded, "operations observation exceeds session limits"));
                }
                match self.journal.as_mut() {
                    Some(journal) => {
                        let outcome = journal.append(value, context)?;
                        self.state = journal.state().clone();
                        Ok(outcome)
                    }
                    None => self.state.publish(value),
                }
            });
        if result.is_err() { self.source.fence(); }
        result
    }
}
fn next_id() -> Result<SessionId> {
    let mut next = lock(&NEXT)?;
    if *next >= 1u128 << 60 { return Err(error(ErrorCode::BudgetExceeded, "operations session IDs exhausted")); }
    let id = SessionId::new((1u128 << 127) | FAMILY | *next); *next += 1; Ok(id)
}
fn resolve(raw: Option<String>) -> Result<Arc<Mutex<OperationsSession>>> {
    let text = raw.ok_or_else(|| error(ErrorCode::InvalidRequest, "open an operations session and supply session_id"))?;
    if text.len() != 32 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(error(ErrorCode::InvalidRequest, "invalid operations session handle"));
    }
    let raw = u128::from_str_radix(&text, 16).map_err(|_| error(ErrorCode::InvalidRequest, "invalid session hex"))?;
    let id = SessionId::new(raw);
    if id.get() != raw || !id.is_process_scoped_live() || !matches!((raw & ((1u128 << 62) - 1)) >> 60, 2 | 3) {
        return Err(error(ErrorCode::InvalidRequest, "not an encoded operations runtime handle"));
    }
    lock(&SESSIONS)?.get(&id).cloned().ok_or_else(|| error(ErrorCode::SessionNotFound, "operations session not found"))
}
fn allowed_environment(name: &str) -> bool {
    !name.starts_with("DFMCP_") || matches!(name, "DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_3"
        | "DFMCP_OPERATIONS_TOKEN" | "DFMCP_OPERATIONS_ENDPOINT"
        | "DFMCP_OPERATIONS_JOURNAL" | "DFMCP_OPERATIONS_JOURNAL_REPAIR")
}
fn validate_environment() -> Result<()> {
    if std::env::var("DFMCP_ALLOW_UNADMITTED_OPERATIONS_V1_3").ok().as_deref() != Some("1")
        || std::env::vars_os().any(|(name, _)| !allowed_environment(&name.to_string_lossy()))
        || crate::admission::current_admission_provenance().is_some() {
        return Err(error(ErrorCode::CapabilityDenied,
            "operations/1.3 requires its exact development opt-in and refuses other DFMCP or production admission state"));
    }
    Ok(())
}
fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id":anchor.fortress_id.to_string(), "epoch":anchor.cursor.epoch,
        "sequence":anchor.cursor.sequence, "game_tick":anchor.tick.0, "state_hash":anchor.state_hash.to_string()})
}
fn coverage() -> Value {
    json!({"status":"partial", "complete_domains":["fortress.current_job_roster",
        "fortress.buildings.all_membership", "fortress.items.all_membership", "fortress.job_item_attachments"],
        "partial_domains":[{"domain":"fortress.operations", "reason":"raw fields and observed relations only; eligibility, accessibility and production sufficiency are not established"}],
        "omitted_domains":["fortress.citizens", "fortress.announcements", "fortress.map",
            "fortress.labor_eligibility", "fortress.material_requirements", "fortress.history"], "continuation":null})
}
fn briefing(session: &OperationsSession) -> Value {
    let value = session.state.observation();
    json!({"runtime":"unadmitted_development", "bridge_protocol":session.state.profile().protocol(), "observation_profile":"operations",
        "read_only":true, "live":true, "runtime_admitted":false, "compatibility_admitted":false,
        "mutation_admissible":false, "source_poisoned":session.source.poisoned(),
        "paused":value.map(|v|v.jobs.paused), "job_count":value.map(|v|v.jobs.jobs.len()),
        "building_count":value.map(|v|v.buildings.len()), "item_count":value.map(|v|v.items.len()),
        "attached_item_relations":value.map(|v|v.attachments.len()),
        "native_snapshot_paging":session.state.profile()==OperationsProfile::PagedV1_4,
        "snapshot_tick_is_capture_tick":true,
        "observation_history":history::summary(session.journal.as_ref()),
        "note":"inventory membership and job attachment do not prove usable supply or fulfillment"})
}
fn packet_limit(budget: WorkBudget) -> usize {
    budget.max_bytes.min(u64::from(budget.max_output_tokens) * 4) as usize
}
fn packet(session: Option<&OperationsSession>, context: Option<&OperationContext>, operation: &str,
    mut payload: Value) -> Result<String> {
    let mut work = empty_active_work();
    if let Some(value) = payload.as_object_mut().and_then(|map|map.remove("_condition_watch_work")) {
        work["obligations"] = value;
    }
    let mut builder = AgentTurnBuilder::new(operation, if operation == "fortress.observe" {
        AgentPhase::Orient
    } else { AgentPhase::Inspect }).profile(ObservationProfile::Briefing).active_work(work);
    let mut limit = 8192;
    if let (Some(session), Some(context)) = (session, context) {
        let anchor = session.anchor()?; limit = packet_limit(session.budget);
        let reset = payload.get("reset").and_then(Value::as_bool) == Some(true);
        let continuity = if session.source.poisoned() { ContinuityStatus::Stale }
            else if reset { ContinuityStatus::Reset }
            else if payload.get("kind").and_then(Value::as_str) == Some("heartbeat") { ContinuityStatus::Heartbeat }
            else { ContinuityStatus::Continuous };
        payload["session_id"] = json!(session.id.to_string()); payload["anchor"] = anchor_json(anchor);
        builder = builder.session_id(session.id.to_string()).request_id(context.request_id.to_string())
            .anchor(anchor_json(anchor)).briefing(briefing(session)).coverage(coverage())
            .budget(json!({"admitted":{"max_bytes":session.budget.max_bytes,"max_output_tokens":session.budget.max_output_tokens}}))
            .continuity(continuity, Some(anchor_json(context.anchor)), None,
                reset.then(||"operations_clock_generation_or_identity_horizon_reset".to_owned()));
    } else {
        builder = builder.briefing(json!({"runtime":"unadmitted_development",
            "runtime_admitted":false,"mutation_admissible":false,"fortress_loaded":false}))
            .coverage(json!({"status":"unknown","complete_domains":[],"partial_domains":[],
                "omitted_domains":["fortress.operations"],"continuation":null}));
    }
    let encoded = builder.attach(payload);
    if encoded.len() > limit { return Err(error(ErrorCode::BudgetExceeded,"complete operations packet exceeds output budget")); }
    Ok(encoded)
}
fn failure_packet(session: Option<&OperationsSession>, context: Option<&OperationContext>, operation: &str, failure: &DfmcpError) -> String {
    let payload = json!({"ok":false,"error":{"operation":operation,"code":failure.code.as_str(),
        "message":failure.message,"retryable":false,"mutation_dispatched":false}});
    // Only already-authorized failures may disclose the cached projection.
    let result = match (session, context) {
        (Some(session), Some(context)) if context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None).is_ok() => {
            semantic_query::publish_with_active_work(context,payload,
                |value|packet(Some(session),Some(context),operation,value))
        }
        _ => packet(None,None,operation,payload),
    };
    match result {
        Ok(encoded) => encoded,
        Err(_) => AgentTurnBuilder::new(operation,AgentPhase::Inspect)
            .briefing(json!({"runtime_admitted":false,"mutation_admissible":false}))
            .attach(json!({"ok":false,"error":{"code":"budget_exceeded",
                "message":"required operations response could not fit; cached work remains retained"}})),
    }
}
fn with_session<F>(id: Option<String>, operation: &str, capability: Capability, body: F) -> String
where F: FnOnce(&mut OperationsSession, OperationContext) -> Result<String> {
    let handle = match resolve(id) { Ok(v)=>v, Err(e)=>return failure_packet(None,None,operation,&e) };
    let mut session = match lock(&handle) { Ok(v)=>v, Err(e)=>return failure_packet(None,None,operation,&e) };
    let context = match session.context() { Ok(v)=>v, Err(e)=>return failure_packet(None,None,operation,&e) };
    if let Err(e)=context.authorize(capability,RiskTier::ReadOnly,&[],None) {
        return failure_packet(None,None,operation,&e);
    }
    match body(&mut session,context.clone()) {
        Ok(encoded)=>encoded,
        Err(e)=>{
            let mut current=context;
            if let Ok(anchor)=session.anchor(){current.anchor=anchor;}
            failure_packet(Some(&session),Some(&current),operation,&e)
        }
    }
}
fn capabilities(input: Option<Vec<String>>) -> Result<Vec<Capability>> {
    let input=input.unwrap_or_else(||vec!["observe".to_owned(),"query".to_owned(),"doctor".to_owned()]);
    if input.is_empty() || input.len()>3 {return Err(error(ErrorCode::CapabilityDenied,"request 1..3 read capabilities"));}
    let mut result=Vec::new();
    for name in input {
        let value=match name.as_str(){"observe"=>Capability::Observe,"query"=>Capability::Query,"doctor"=>Capability::Doctor,
            _=>return Err(error(ErrorCode::CapabilityDenied,"operations cannot grant that capability"))};
        if result.contains(&value){return Err(error(ErrorCode::InvalidRequest,"duplicate capability"));} result.push(value);
    }
    Ok(result)
}

#[tool(description = "Open an authenticated, explicitly unadmitted operations/1.3 session covering jobs, buildings, inventory and attachments in one native read. Token and loopback endpoint are process configuration, not tool arguments.")]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(max_jobs:Option<u32>,max_buildings:Option<u32>,max_items:Option<u32>,
    max_bytes:Option<u64>,max_output_tokens:Option<u32>,max_wall_millis:Option<u64>,requested_capabilities:Option<Vec<String>>) -> String {
    let result=(|| -> Result<String> {
        validate_environment()?;
        let capabilities=capabilities(requested_capabilities)?;
        let journal_configuration=history::configuration()?;
        if journal_configuration.is_some() && (!capabilities.contains(&Capability::Query) || !capabilities.contains(&Capability::Observe)) {
            return Err(error(ErrorCode::CapabilityDenied,"durable observation sessions require Query and Observe authority"));
        }
        let bytes=max_bytes.unwrap_or(MAX_OPERATIONS_BYTES as u64);
        let limits=OperationsLimits {jobs:max_jobs.unwrap_or(1024),buildings:max_buildings.unwrap_or(1024),
            items:max_items.unwrap_or(8192),payload_bytes:usize::try_from(bytes)
                .map_err(|_|error(ErrorCode::BudgetExceeded,"operations bytes do not fit this platform"))?};
        limits.validate()?;
        let budget=WorkBudget {max_wall_millis:max_wall_millis.unwrap_or(5000),max_game_ticks:1_000_000,
            max_entities:limits.entity_limit(),max_bytes:bytes,max_output_tokens:max_output_tokens.unwrap_or(8192),max_actions:1};
        budget.validate()?;
        if bytes<8192 || !(2048..=65536).contains(&budget.max_output_tokens) || !(1..=60_000).contains(&budget.max_wall_millis) {
            return Err(error(ErrorCode::BudgetExceeded,"operations response or wall-time budgets exceed supported bounds"));
        }
        let slot=SessionSlot::reserve()?;let id=next_id()?;
        let endpoint=std::env::var("DFMCP_OPERATIONS_ENDPOINT").unwrap_or_else(|_|"127.0.0.1:5000".to_owned());
        let endpoint=dfmcp_adapter::parse_loopback_endpoint(&endpoint)?;
        let token=std::env::var("DFMCP_OPERATIONS_TOKEN").map_err(|_|error(ErrorCode::CapabilityDenied,"DFMCP_OPERATIONS_TOKEN is required"))?;
        let mut source=OperationsRpcClient::connect(endpoint,token.into_bytes(),id.get().to_be_bytes().to_vec(),
            Duration::from_millis(budget.max_wall_millis),limits)?;
        let mut state=LiveOperationsState::default();
        state.publish(source.refresh(Duration::from_millis(budget.max_wall_millis))?)?;
        let fortress=state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"bootstrap snapshot missing"))?.fortress_id;
        let grants=capabilities.iter().map(|capability|CapabilityGrant {capability:*capability,
            scope:CapabilityScope {fortress_id:Some(fortress),..CapabilityScope::default()},max_risk:RiskTier::ReadOnly,
            expires_at_tick:None,remaining_uses:None}).collect();
        let mut session=OperationsSession {id,source:Box::new(source),state,journal:None,limits,budget,grants,request:0,_slot:slot};
        let mut context=session.context()?;
        if let Some((path,repair))=journal_configuration {
            history::attach(&mut session,&path,repair,&context)?;
            context.anchor=session.anchor()?;
        }
        let encoded=packet(Some(&session),Some(&context),"fortress.open_session",json!({"ok":true,
            "granted_capabilities":capabilities.iter().map(|c|c.as_str()).collect::<Vec<_>>(),
            "limits":{"jobs":limits.jobs,"buildings":limits.buildings,"items":limits.items,"payload_bytes":limits.payload_bytes},
            "schema_discovery":{"tool":"fortress.query","arguments":{"session_id":id.to_string(),"mode":"schema"}}}))?;
        let mut registry=lock(&SESSIONS)?;
        if registry.contains_key(&id){return Err(error(ErrorCode::InternalInvariantViolation,"operations session ID collision"));}
        registry.insert(id,Arc::new(Mutex::new(session)));Ok(encoded)
    })();
    match result {Ok(encoded)=>encoded,Err(e)=>failure_packet(None,None,"fortress.open_session",&e)}
}
fn observe(id:Option<String>,operation:&str)->String {
    with_session(id,operation,Capability::Observe,|session,context| {
        let outcome=session.refresh(&context)?;
        let payload=json!({"ok":true,"kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},
            "reset":outcome==JobPublication::Reset,"native_observations":1,"game_clock_controlled":false});
        let mut target=context.clone();target.anchor=session.anchor()?;
        target.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
        if target.authorize(Capability::Query,RiskTier::ReadOnly,&[],None).is_ok() {
            semantic_query::publish_with_active_work(&target,payload,|value|packet(Some(session),Some(&context),operation,value))
        } else {packet(Some(session),Some(&context),operation,payload)}
    })
}
#[tool(description = "Read one coherent jobs/buildings/items observation without unpausing. An oversized or inconsistent domain prevents combined publication.")]
pub fn fortress_observe(session_id:Option<String>)->String{observe(session_id,"fortress.observe")}
#[tool(description = "Refresh one operations observation without controlling game time. Use query await_watch for bounded condition evaluation.")]
pub fn fortress_wait(session_id:Option<String>)->String{observe(session_id,"fortress.wait")}
fn query_view(session:&OperationsSession,context:&OperationContext)->Result<QueryResponseProjection> {
    let source=session.state.source_digest()?;
    Ok(QueryResponseProjection {session_id:session.id.to_string(),request_id:context.request_id.to_string(),anchor:anchor_json(session.anchor()?),
        briefing:briefing(session),attention:Vec::new(),affordances:Vec::new(),coverage:coverage(),
        uncertainty:vec![json!({"domain":"fortress.operations.feasibility","epistemic_state":"unknown",
            "reason":"raw containment, flags, stacks and construction stages do not prove material eligibility, access, or job completion"})],
        budget:json!({"admitted":{"max_bytes":session.budget.max_bytes,"max_output_tokens":session.budget.max_output_tokens}}),
        references:vec![json!({"kind":"operations_observation","digest":source.to_string()})],
        maximum_bytes:packet_limit(session.budget)})
}
fn finish_query(view:&QueryResponseProjection,value:Value)->Result<String> {
    let encoded=view.finish(value)?;
    let mut value:Value=serde_json::from_str(&encoded).map_err(|_|error(ErrorCode::InternalInvariantViolation,"query packet decode failed"))?;
    value["agent_turn"]["turn_id"]=json!(format!("operations-turn-{}",view.request_id));
    let encoded=value.to_string();
    if encoded.len()>view.maximum_bytes{return Err(error(ErrorCode::BudgetExceeded,"operations packet overflow"));}Ok(encoded)
}
#[tool(description = "Query the coherent operations snapshot with typed filters, aggregates, graph relations, search, baselines and watches. Modes: schema, summary, jobs, buildings, items, production. History requires a compatible durable journal and is unavailable in the paged 1.4 profile. Production diagnosis joins observed conditions; inventory_plan allocates declared stack-unit demands without reserving or proving usable supply.")]
pub fn fortress_query(session_id:Option<String>,mode:Option<String>,query:Option<Value>)->String {
    with_session(session_id,"fortress.query",Capability::Query,|session,mut context| {
        if mode.is_some() && query.is_some(){return Err(error(ErrorCode::InvalidRequest,"do not combine mode and query"));}
        let schema=mode.as_deref()==Some("schema");
        let mut input=match query {
            Some(input)=>input,
            None=>match mode.as_deref() {
                None|Some("summary"|"schema")=>json!({"schema":"dfmcp.query/1","query":{"kind":"aggregate","group_by":{"kind":"entity_kind"}}}),
                Some("history")=>json!({"schema":"dfmcp.query/1","query":{"kind":"history"}}),
                Some("production")=>json!({"schema":"dfmcp.query/1","query":{"kind":"production_diagnosis"}}),
                Some("jobs")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["job"],"fields":["type_key","suspended","holder_native_id"]}}),
                Some("buildings")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["building"],"fields":["type_key","build_stage","max_build_stage"]}}),
                Some("items")=>json!({"schema":"dfmcp.query/1","query":{"kind":"entities","kinds":["item"],"fields":["type_key","stack_size","forbidden","in_job","container","holder_building"],"limit":1}}),
                _=>return Err(error(ErrorCode::InvalidRequest,"unsupported operations query mode")),
            },
        };
        let kind=input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str).unwrap_or("");
        if !schema && history::handles(&input) {
            if session.state.profile()!=OperationsProfile::V1_3 {
                return Err(error(ErrorCode::InvalidRequest,"durable archive queries are not implemented for operations/1.4; 1.3 archives cannot be reinterpreted"));
            }
            let result=history::execute(session,&context,&input);
            if matches!(&result,Err(failure) if failure.code==ErrorCode::CorruptLedger) {session.source.fence();}
            return result;
        }
        let local=matches!(kind,"watches"|"cancel_watch"|"release_watch"|"baselines"|"release_baseline");
        if session.source.poisoned() && !local && !schema {
            return Err(error(ErrorCode::AdapterUnavailable,"operations source fenced; reopen or manage local records"));
        }
        let mut refreshed=None;
        if !schema && kind=="await_watch" {
            let snapshot=session.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"snapshot missing"))?;
            if semantic_query::prepare_await(snapshot,&context,&input)? {
                let basis=context.anchor;let outcome=session.refresh(&context)?;
                context.anchor=session.anchor()?;
                context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
                context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
                refreshed=Some(json!({"basis":anchor_json(basis),"reset":outcome==JobPublication::Reset,
                    "kind":if outcome==JobPublication::Heartbeat{"heartbeat"}else{"snapshot"},"native_observations":1}));
            }
            if let Some(object)=input.as_object_mut(){object.remove("expected_anchor");}
            input["query"]["kind"]=json!("poll_watch");
        }
        let view=query_view(session,&context)?;
        let mut narrowed=context.clone();narrowed.budget.max_bytes=view.result_byte_budget()? as u64;
        if schema {
            let schema=if session.state.profile()==OperationsProfile::V1_3 {history::query_schema()?}else{production::query_schema()?};
            return semantic_query::publish_with_active_work(&context,json!({"query_schema":schema,"mode":"schema",
                "profile":format!("operations/{}",session.state.profile().protocol()),"source_stale":session.source.poisoned(),"truncated":false,"continuation":null}),
                |value|finish_query(&view,value));
        }
        if production::handles(&input) {
            let result_context=semantic_query::result_context(&narrowed)?;
            let value=production::execute(&session.state,&result_context,&input)?;
            return semantic_query::publish_with_active_work(&narrowed,value,|value|finish_query(&view,value));
        }
        let snapshot=session.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"snapshot missing"))?;
        semantic_query::execute_with_publisher(snapshot,&narrowed,&input,|mut value| {
            value["source_stale"]=json!(session.source.poisoned());
            if let Some(refresh)=refreshed{value["observation_refresh"]=refresh;}
            finish_query(&view,value)
        })
    })
}
#[tool(description = "Explain operations fields and observed job/item/building relations; no speculative blocker diagnosis or mutation authority.")]
pub fn fortress_explain(session_id:Option<String>)->String {
    with_session(session_id,"fortress.explain",Capability::Query,|session,context| {
        semantic_query::publish_with_active_work(&context,json!({"ok":true,
            "relations":{"contained_in":"job holder, item container, or item building holder","uses":"an observed job-item attachment, not requirement satisfaction"},
            "unknown":["material suitability","accessible inventory","labor eligibility","why suspended","completed successfully"],
            "item_position":"raw item.pos; use containment edges to inspect holders",
            "inspection":"Use query kind inspect with entity_id, generation and selected fields.",
            "production_analysis":{"diagnose":"production_diagnosis","allocate":"inventory_plan",
                "scope":"observed conditions and declared stack-unit models only","reservation_created":false}}),
            |value|packet(Some(session),Some(&context),"fortress.explain",value))
    })
}
#[tool(description = "Report the operations source health and coverage. Does not reconnect, mutate game state, or claim native qualification.")]
pub fn fortress_doctor(session_id:Option<String>)->String {
    with_session(session_id,"fortress.doctor",Capability::Doctor,|session,context|
        packet(Some(session),Some(&context),"fortress.doctor",json!({"ok":true,
            "status":if session.source.poisoned(){"source_fenced"}else{"read_only_unadmitted"},"production_admitted":false})))
}
fn no_effect(id:Option<String>,operation:&str)->String {
    with_session(id,operation,Capability::Query,|_,_|Err(error(ErrorCode::CapabilityDenied,
        "operations read profiles have no prepare, commit, cancellation, save, or restore effect")))
}
#[tool(description = "Unavailable: operations read profiles cannot plan live effects.")]
pub fn fortress_plan(session_id:Option<String>)->String{no_effect(session_id,"fortress.plan")}
#[tool(description = "Unavailable: operations read profiles commit no game effects.")]
pub fn fortress_commit(session_id:Option<String>)->String{no_effect(session_id,"fortress.commit")}
#[tool(description = "Unavailable for game actions. Local condition watches use query cancel_watch.")]
pub fn fortress_cancel(session_id:Option<String>)->String{no_effect(session_id,"fortress.cancel")}
#[tool(description = "Unavailable: operations read profiles create no game or save checkpoint.")]
pub fn fortress_checkpoint(session_id:Option<String>)->String{no_effect(session_id,"fortress.checkpoint")}
#[tool(description = "Unavailable: operations read profiles restore no game or save state.")]
pub fn fortress_restore(session_id:Option<String>)->String{no_effect(session_id,"fortress.restore")}

/// Independently gated when called as a library, not only through its binary.
pub fn run_stdio() {
    if let Err(failure)=validate_environment(){eprintln!("{failure}");std::process::exit(1);}
    let server=ServerBuilder::new("dfmcp-live-operations-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor).request_timeout(60)
        .instructions("Explicitly unadmitted operations/1.3. Open a session first. Jobs, buildings, items and their observed links share one native observation. Query filters, aggregates, traversal, baselines and foreground watches are available. production_diagnosis joins observed job/input conditions; inventory_plan returns a conditional allocation and shortage certificate, not game feasibility or a reservation. Do not infer material availability, access or completion from raw fields. No citizen data, map data, live mutation or production admission. When an operator configures a durable journal, history lists committed observations and historical_query replays a stateless query at an exact past record. Archived facts do not establish current freshness. Use schema mode for structured requests.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path = "live_operations_server_tests.rs"]
mod tests;
