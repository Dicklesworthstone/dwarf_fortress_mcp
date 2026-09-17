//! Actual spatial MCP handlers with coherent injected captures and private files.
//! These fixtures do not execute DFHack or qualify native labor/material semantics.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_portfolio_spatial.rs"]
mod fixture;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use dfmcp_adapter::operations_journal::TailRecovery;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError { error(ErrorCode::CorruptLedger, "portfolio fixture I/O") }
struct Files { directory: PathBuf, observations: PathBuf, watches: PathBuf }
impl Files {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir().canonicalize().map_err(io_error)?.join(format!(
            "dfmcp-portfolio-{}-{}",std::process::id(),FILE_ID.fetch_add(1,Ordering::Relaxed)));
        fs::DirBuilder::new().mode(0o700).create(&directory).map_err(io_error)?;
        Ok(Self { observations:directory.join("observations.bin"), watches:directory.join("watches.bin"), directory })
    }
}
impl Drop for Files { fn drop(&mut self) {
    let _=fs::remove_file(&self.watches); let _=fs::remove_file(&self.observations); let _=fs::remove_dir(&self.directory);
}}
struct Script { values: VecDeque<LiveSpatialCitizenObservation>, calls: Arc<AtomicUsize>, fenced: bool }
impl Source for Script {
    fn read(&mut self,_:Duration)->Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1,Ordering::SeqCst);
        self.values.pop_front().ok_or_else(||error(ErrorCode::AdapterUnavailable,"portfolio source exhausted"))
    }
    fn poisoned(&self)->bool {self.fenced}
    fn fence(&mut self) {self.fenced=true;}
    fn pages(&self)->u32 {1}
}
struct Registered {id:SessionId,calls:Arc<AtomicUsize>}
impl Registered {fn handle(&self)->Option<String> {Some(self.id.to_string())}}
impl Drop for Registered {fn drop(&mut self) {if let Ok(mut sessions)=SESSIONS.lock(){sessions.remove(&self.id);}}}
fn decode(raw:&str)->Result<Value> {serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"portfolio fixture JSON"))}
fn ok(value:Value)->Value {assert_eq!(value["ok"],true,"{value}");value}
fn register(files:&Files,next:Vec<LiveSpatialCitizenObservation>)->Result<Registered> {
    let first=fixture::observation(3,2,2,false)?;let region=first.spatial().terrain().map.region;
    let limits=CitizenSpatialLimits {spatial:SpatialLimits {operations:PagedOperationsLimits::default(),region},citizens:4096};
    let mut state=LiveSpatialCitizenState::default();state.publish(first)?;
    let anchor=state.snapshot().ok_or_else(||error(ErrorCode::InvalidRequest,"fixture state absent"))?.anchor();
    let calls=Arc::new(AtomicUsize::new(0));
    let mut s=Session {id:next_id()?,source:Box::new(Script {values:next.into(),calls:calls.clone(),fenced:false}),state,limits,journal:None,
        budget:WorkBudget {max_entities:limits.entity_limit(),max_bytes:1024*1024,max_output_tokens:65536,
            max_wall_millis:60000,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe,Capability::Doctor].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope {fortress_id:Some(anchor.fortress_id),..CapabilityScope::default()},
            max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None}).collect(),
        request:0,_watch_journal:None,_slot:Slot::reserve()?};
    let mut c=s.context()?;history::attach(&mut s,&files.observations,TailRecovery::Refuse,&c)?;c.anchor=s.anchor()?;
    durable_watches::finish_open(&mut s,&c,Some(&files.watches),json!({"ok":true}))?;
    let id=s.id;lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(s)));Ok(Registered {id,calls})
}
fn ask(s:&Registered,query:Value)->Result<Value> {
    decode(&fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query}))))
}
fn task(key:&str,priority:u32,workers:u32,units:u32)->Value {
    json!({"key":key,"priority":priority,"workers":workers,"skill_key":"CARPENTRY",
        "materials":[{"key":"wood","units":units,"item_types":["item_type_3"]}]})
}
fn request()->Value {
    json!({"kind":"production_portfolio","origin":[0,0,5],"quantity_unit":"stack_units",
        "tasks":[task("a",3,2,2),task("b",2,1,1),task("c",2,1,1)],"limit":128})
}
fn historical(entry:&Value,query:Value)->Value {
    json!({"kind":"historical_query","record":entry["record"],"record_digest":entry["record_digest"],"query":query})
}
fn retain_watch(s:&Registered)->Result<()> {
    ok(ask(s,json!({"kind":"watch","key":"portfolio-progress","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"stable_observations":64}))?);Ok(())
}

#[test]
fn portfolio_handler_selects_complete_tasks_without_mutating_watches_or_observations()->Result<()> {
    let _serial=lock(&SERIAL)?;let f=Files::new()?;let s=register(&f,vec![])?;retain_watch(&s)?;
    let before=ok(ask(&s,json!({"kind":"watches"}))?);
    let watch_bytes=fs::read(&f.watches).map_err(io_error)?;let archive_bytes=fs::read(&f.observations).map_err(io_error)?;
    let result=ok(ask(&s,request())?);
    assert_eq!(result["selected_task_keys"],json!(["b","c"]));assert_eq!(result["selected_priority"],4);
    assert_eq!(result["assigned_workers"],2);assert_eq!(result["assigned_stack_units"],2);
    assert_eq!(result["native_captures"],0);assert_eq!(result["reservations_created"],false);
    assert_eq!(result["native_job_readiness_proven"],false);assert_eq!(result["production_schedule_proven"],false);
    let rows=result["rows"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows absent"))?;
    assert_eq!(rows.iter().filter(|r|r["row_kind"]=="worker_assignment").count(),2);
    assert_eq!(rows.iter().filter(|r|r["row_kind"]=="material_assignment").count(),2);
    assert_eq!(rows.iter().filter(|r|r["row_kind"]=="rejected_combination").count(),3);
    assert_eq!(result["optimization"]["higher_ranked_sets_rejected"],3);
    assert_eq!(result["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len),Some(1));
    assert_eq!(ok(ask(&s,json!({"kind":"watches"}))?)?["records"],before["records"]);
    assert_eq!(fs::read(&f.watches).map_err(io_error)?,watch_bytes);
    assert_eq!(fs::read(&f.observations).map_err(io_error)?,archive_bytes);
    assert_eq!(s.calls.load(Ordering::SeqCst),0);Ok(())
}

#[test]
fn portfolio_pagination_preserves_the_optimum_and_rejects_other_models()->Result<()> {
    let _serial=lock(&SERIAL)?;let f=Files::new()?;let s=register(&f,vec![fixture::observation(4,2,2,false)?])?;
    retain_watch(&s)?;
    {let h=resolve(s.handle())?;lock(&h)?.budget.max_output_tokens=8192;}
    let mut query=request();query["limit"]=json!(1);
    let mut workers=BTreeSet::new();let mut units=0u64;let mut cuts=0usize;let mut pages=0usize;
    let mut first_token=Value::Null;let mut evidence=Value::Null;
    loop {
        let raw=fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query.clone()})));
        assert!(raw.len()<=32768);let result=ok(decode(&raw)?);
        assert_eq!(result["selected_task_mask"],6);assert_eq!(result["assigned_stack_units"],2);
        if evidence.is_null(){evidence=result["optimization"]["exclusion_evidence_digest"].clone();}
        assert_eq!(result["optimization"]["exclusion_evidence_digest"],evidence);
        for row in result["rows"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows absent"))? {
            match row["row_kind"].as_str() {
                Some("worker_assignment")=>{assert!(workers.insert(row["citizen"]["entity_id"].to_string()));}
                Some("material_assignment")=>units+=row["units"].as_u64().unwrap_or(0),
                Some("rejected_combination")=>{cuts+=1;assert!(row["deficit"].as_u64().unwrap_or(0)>0);}
                _=>return Err(error(ErrorCode::InternalInvariantViolation,"unknown portfolio row")),
            }
        }
        pages+=1;assert!(pages<=7);
        if result["continuation"].is_null(){break;}
        if first_token.is_null(){first_token=result["continuation"].clone();}
        query["continuation"]=result["continuation"].clone();query["limit"]=json!(2);
    }
    assert_eq!((workers.len(),units,cuts),(2,2,3));assert!(pages>1);
    let mut different=request();different["continuation"]=first_token.clone();different["tasks"][0]["priority"]=json!(20);
    assert_eq!(ask(&s,different)?["error"]["code"],"stale_anchor");
    ok(decode(&fortress_observe(s.handle()))?);
    let mut stale=request();stale["continuation"]=first_token;
    assert_eq!(ask(&s,stale)?["error"]["code"],"stale_anchor");Ok(())
}

#[test]
fn historical_portfolios_and_each_route_remain_bound_to_the_selected_record()->Result<()> {
    let _serial=lock(&SERIAL)?;let f=Files::new()?;let s=register(&f,vec![fixture::observation(4,2,0,false)?])?;
    retain_watch(&s)?;
    let entry=ok(ask(&s,json!({"kind":"history","limit":1}))?)["rows"][0].clone();
    let old_current=ok(ask(&s,request())?);
    ok(decode(&fortress_observe(s.handle()))?);
    let current=ok(ask(&s,request())?);assert_eq!(current["selected_tasks"],0);
    let watched=ok(ask(&s,json!({"kind":"watches"}))?);
    let bytes=fs::read(&f.watches).map_err(io_error)?;
    let old=ok(ask(&s,historical(&entry,request()))?);
    assert_eq!(old["selected_task_mask"],old_current["selected_task_mask"]);assert_eq!(old["anchor"],entry["anchor"]);
    assert_eq!(old["historical"],true);
    for row in old["rows"].as_array().ok_or_else(||error(ErrorCode::InvalidRequest,"rows absent"))? {
        if let Some(route)=row.get("route_query") {
            assert_eq!(route["query"]["record"],entry["record"]);
            let result=ok(decode(&fortress_query(s.handle(),None,Some(route.clone())))?);
            assert_eq!(result["anchor"],entry["anchor"]);assert_eq!(result["historical"],true);
        }
    }
    assert_eq!(ok(ask(&s,request())?)["anchor"],current["anchor"]);
    assert_eq!(ok(ask(&s,json!({"kind":"watches"}))?)["records"],watched["records"]);
    assert_eq!(fs::read(&f.watches).map_err(io_error)?,bytes);assert_eq!(s.calls.load(Ordering::SeqCst),1);Ok(())
}

#[test]
fn offline_reopening_can_plan_but_cannot_claim_current_readiness()->Result<()> {
    let _serial=lock(&SERIAL)?;let f=Files::new()?;let s=register(&f,vec![])?;
    let entry=ok(ask(&s,json!({"kind":"history","limit":1}))?)["rows"][0].clone();
    let (limits,budget)={let h=resolve(s.handle())?;let mut session=lock(&h)?;session.source.fence();(session.limits,session.budget)};
    assert_eq!(ask(&s,request())?["error"]["code"],"adapter_unavailable");
    ok(ask(&s,historical(&entry,request()))?);drop(s);
    let archive_before=fs::read(&f.observations).map_err(io_error)?;let watches_before=fs::read(&f.watches).map_err(io_error)?;
    let session=archive::open(next_id()?,Slot::reserve()?,&f.observations,limits,budget,&[Capability::Query])?;
    let id=session.id;lock(&SESSIONS)?.insert(id,Arc::new(Mutex::new(session)));
    let s=Registered {id,calls:Arc::new(AtomicUsize::new(0))};
    let result=ok(ask(&s,request())?);
    assert_eq!(result["selected_task_mask"],6);assert_eq!(result["historical"],true);
    assert_eq!(result["bridge_connection_present"],false);assert_eq!(result["current_freshness_proven"],false);
    assert_eq!(result["native_captures"],0);
    let schema=ok(decode(&fortress_query(s.handle(),Some("schema".into()),None))?);
    assert!(schema["query_schema"]["$defs"]["query"]["oneOf"].as_array()
        .is_some_and(|a|a.iter().any(|v|v["properties"]["kind"]["const"]=="production_portfolio")));
    assert_eq!(fs::read(&f.observations).map_err(io_error)?,archive_before);
    assert_eq!(fs::read(&f.watches).map_err(io_error)?,watches_before);Ok(())
}

#[test]
fn malformed_tasks_small_budgets_and_revoked_authority_leave_state_unchanged()->Result<()> {
    let _serial=lock(&SERIAL)?;let f=Files::new()?;let s=register(&f,vec![])?;
    let before=fs::read(&f.observations).map_err(io_error)?;let watches=fs::read(&f.watches).map_err(io_error)?;
    for case in 0..6 {
        let mut q=request();
        match case {
            0=>q["tasks"][0]["materials"][0]["item_types"]=json!(["item_type_3";9]),
            1=>q["tasks"][1]["key"]=json!("a"),
            2=>q["tasks"][0]["materials"][0]["material_index"]=json!(1),
            3=>q["max_work"]=json!(1),
            4=>q["origin"]=json!([9,9,5]),
            _=>q["unknown"]=json!(true),
        }
        assert_eq!(ask(&s,q)?["ok"],false,"case={case}");
    }
    {let h=resolve(s.handle())?;lock(&h)?.budget.max_output_tokens=1;}
    assert_eq!(ask(&s,request())?["error"]["code"],"budget_exceeded");
    {let h=resolve(s.handle())?;let mut session=lock(&h)?;session.budget.max_output_tokens=65536;session.grants.clear();}
    assert_eq!(ask(&s,request())?["error"]["code"],"capability_denied");
    assert_eq!(fs::read(&f.observations).map_err(io_error)?,before);
    assert_eq!(fs::read(&f.watches).map_err(io_error)?,watches);assert_eq!(s.calls.load(Ordering::SeqCst),0);Ok(())
}

#[test]
fn current_production_queries_refuse_changed_observation_journal_custody()->Result<()> {
    use std::io::Write;
    let _serial=lock(&SERIAL)?;let f=Files::new()?;let s=register(&f,vec![])?;
    let watches=fs::read(&f.watches).map_err(io_error)?;
    fs::OpenOptions::new().append(true).open(&f.observations).and_then(|mut file|file.write_all(b"x")).map_err(io_error)?;
    assert_eq!(ask(&s,request())?["error"]["code"],"corrupt_ledger");
    assert_eq!(fs::read(&f.watches).map_err(io_error)?,watches);assert_eq!(s.calls.load(Ordering::SeqCst),0);Ok(())
}
