//! Actual dispatcher/renderer tests with the existing native byte fixtures and
//! coordinator. Injected storage/native effects are not a live DFHack campaign.
use super::*;
use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::excavation_run::coordinator::{ExcavationBinding, ExcavationCoordinator,
    ExcavationDispatch, ExcavationRunSource};
use dfmcp_adapter::excavation_run::session::{ExcavationAttempt, ExcavationObservation};
use dfmcp_adapter::excavation_run::rpc::ExcavationCancellation;
use std::cell::RefCell;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::net::SocketAddr;
use std::rc::Rc;
use std::time::Duration;

fn hex(raw: &str) -> Vec<u8> {
    let raw=raw.trim();
    (0..raw.len()).step_by(2).map(|i|u8::from_str_radix(&raw[i..i+2],16).unwrap()).collect()
}
fn fixture_plan() -> ExcavationRunPlan {
    ExcavationRunPlan::decode(&hex(include_str!("../../../dfmcp-adapter/tests/fixtures/excavation_run_intent_v1_18.hex"))).unwrap()
}
fn native_record(plan: &ExcavationRunPlan, stopped: bool) -> ExcavationRunRecord {
    let fixture=hex(if stopped {
        include_str!("../../../dfmcp-adapter/tests/fixtures/excavation_run_stopped_v1_18.hex")
    }else{include_str!("../../../dfmcp-adapter/tests/fixtures/excavation_run_prepared_v1_18.hex")});
    let start=fixture_plan().canonical_bytes().len()+48;
    let mut raw=b"DFMER018".to_vec();
    raw.extend_from_slice(&plan.canonical_bytes()[8..]);
    raw.extend_from_slice(plan.digest().as_bytes());
    raw.extend_from_slice(plan.token());
    raw.extend_from_slice(&fixture[start..fixture.len()-32]);
    reseal(raw)
}
fn reseal(mut raw: Vec<u8>) -> ExcavationRunRecord {
    let mut domain=b"dfmcp-excavation-run-receipt/1\0".to_vec();domain.extend_from_slice(&raw);
    raw.extend_from_slice(Digest32::of_bytes(&domain).as_bytes());
    ExcavationRunRecord::decode(&raw).unwrap()
}
#[derive(Default)]
struct Memory(Cursor<Vec<u8>>);
impl Read for Memory {fn read(&mut self,out:&mut[u8])->io::Result<usize>{self.0.read(out)}}
impl Write for Memory {
    fn write(&mut self,raw:&[u8])->io::Result<usize>{self.0.write(raw)}
    fn flush(&mut self)->io::Result<()>{Ok(())}
}
impl Seek for Memory {fn seek(&mut self,p:SeekFrom)->io::Result<u64>{self.0.seek(p)}}
impl EffectJournalStorage for Memory {
    fn sync(&mut self)->io::Result<()>{Ok(())}
    fn truncate(&mut self,_:u64)->io::Result<()>{panic!("no repair")}
}
struct Native {binding:ExcavationBinding,calls:Rc<RefCell<Vec<&'static str>>>,lost:bool}
impl ExcavationRunSource for Native {
    fn binding(&self)->&ExcavationBinding{&self.binding}
    fn fence(&mut self){}
    fn observe(&mut self,_:ExcavationRegion,_:&OperationContext,_:Duration)->Result<ExcavationCapture>{
        self.calls.borrow_mut().push("observe");Ok(fixture_plan().before().clone())
    }
    fn prepare(&mut self,p:&ExcavationRunPlan,_:&OperationContext,_:Duration)->Result<ExcavationRunRecord>{
        self.calls.borrow_mut().push("prepare");Ok(native_record(p,false))
    }
    fn commit(&mut self,p:ExcavationDispatch<'_>,_:&OperationContext,_:Duration)->Result<ExcavationRunRecord>{
        self.calls.borrow_mut().push("commit");
        if self.lost{return Err(error(ErrorCode::AdapterUnavailable,"injected reply loss"));}
        Ok(native_record(p.plan(),true))
    }
    fn query(&mut self,_:&ExcavationRunPlan,_:&OperationContext,_:Duration)->Result<Option<ExcavationRunRecord>>{
        self.calls.borrow_mut().push("query");Ok(None)
    }
    fn cancel(&mut self,p:&ExcavationRunPlan,_:&OperationContext,_:Duration)->Result<ExcavationRunRecord>{
        self.calls.borrow_mut().push("cancel");Ok(native_record(p,true))
    }
}
fn ctx(mode:ExcavationMode)->OperationContext {
    let p=fixture_plan();
    context(SessionId::new(1),1,p.before().fortress(),p.before().tick(),WorkBudget{
        max_wall_millis:60000,max_bytes:MAX_SESSION_BYTES,max_output_tokens:8192,
        max_game_ticks:1200,max_entities:256,max_actions:1,
    },grants(mode,p.before().fortress(),None))
}
fn inventory(complete:usize,pending:bool)->ExcavationInventory {
    let original=fixture_plan();let c=ctx(ExcavationMode::Control);
    let binding=ExcavationBinding::new(SocketAddr::from(([127,0,0,1],5000)),"df","dfhack",original.before()).unwrap();
    let mut coordinator=ExcavationCoordinator::create(Memory::default(),binding.clone(),&c).unwrap();
    for i in 0..complete+usize::from(pending) {
        let p=ExcavationRunPlan::new(&format!("goal-{i:03}"),original.spec(),original.before().clone()).unwrap();
        let mut native=Native{binding:binding.clone(),calls:Rc::default(),lost:pending&&i==complete};
        let result=coordinator.start(&mut native,p.clone(),p.digest(),&c);
        assert_eq!(result.is_err(),native.lost);
    }
    ExcavationInventory::new(binding,coordinator.entries().cloned().collect()).unwrap()
}
#[derive(Clone)]
struct Backend {inventory:Rc<RefCell<ExcavationInventory>>,calls:Rc<RefCell<Vec<&'static str>>>,fortress:FortressIdentity}
impl ExcavationSessionBackend for Backend {
    fn fortress(&self)->&FortressIdentity{&self.fortress}
    fn region(&self)->ExcavationRegion{fixture_plan().before().region()}
    fn inspect(&mut self,_:&OperationContext,_:&dyn ExcavationSessionGuard)->Result<ExcavationInventory>{
        self.calls.borrow_mut().push("inspect");Ok(self.inventory.borrow().clone())
    }
    fn initialize(&mut self,_:&OperationContext,_:&dyn ExcavationSessionGuard)->Result<ExcavationObservation>{panic!("explicit initialization only")}
    fn observe(&mut self,v:&ExcavationInventory,_:&OperationContext,_:&dyn ExcavationSessionGuard)->Result<ExcavationObservation>{
        self.calls.borrow_mut().push("observe");Ok(ExcavationObservation{binding:v.binding().clone(),capture:fixture_plan().before().clone()})
    }
    fn start(&mut self,_:&ExcavationInventory,_:ExcavationRunPlan,_:&OperationContext,_:&dyn ExcavationSessionGuard)->Result<ExcavationRunRecord>{panic!("unexpected dispatch")}
    fn recover(&mut self,_:&ExcavationInventory,_:&str,_:bool,_:&OperationContext,_:&dyn ExcavationSessionGuard)->Result<Option<ExcavationRunRecord>>{panic!("unexpected native recovery")}
}
struct Guard;
impl ExcavationSessionGuard for Guard {
    fn checkpoint(&self)->Result<()>{Ok(())}
    fn allow_start(&self)->Result<()>{Ok(())}
    fn cancellation(&self)->ExcavationCancellation{ExcavationCancellation::default()}
}
fn state(view:ExcavationInventory,mode:ExcavationMode)->(State<Backend>,Backend) {
    let c=ctx(mode);
    let backend=Backend{fortress:view.binding().fortress().clone(),inventory:Rc::new(RefCell::new(view)),calls:Rc::default()};
    let session=ExcavationSession::open(backend.clone(),mode,false,&c,Instant::now(),&Guard).unwrap();
    (State{id:c.session_id,request:1,budget:c.budget,grants:c.grants,session,cursors:Cursors::default()},backend)
}
fn invoke(state:&mut State<Backend>,op:&str,action:Result<ExcavationCommand>,query:QueryRequest)->Value{
    serde_json::from_str(&dispatch(state,op,None,action,Ok(query),Instant::now(),&Guard)).unwrap()
}
fn records(filter:Filter,limit:u32,continuation:Option<String>)->QueryRequest{
    QueryRequest::Records{state:Some(filter),limit:Some(limit),continuation}
}
fn turn(view:ExcavationInventory)->ExcavationTurn{
    ExcavationTurn{outcome:Ok(ExcavationOutcome::Inventory),inventory:Some(view),historical_prior:None,
        plan:None,uncertain_attempt:None,native_operation_attempted:false,released:false}
}

#[test]
fn closed_requests_reject_unknown_duplicate_and_wrong_typed_fields(){
    let digest="a".repeat(64);
    let good=json!({"key":"goal","plan_digest":digest,"confirm":true}).to_string();
    assert!(requests::parse::<CommitRequest>(&good).unwrap().command().is_ok());
    for extra in ["path","token","native_method","lua","force"]{
        let mut v:Value=serde_json::from_str(&good).unwrap();v[extra]=json!("injected");
        assert!(requests::parse::<CommitRequest>(&v.to_string()).is_err());
    }
    assert!(requests::parse::<Identity>(&format!(r#"{{"key":"a","key":"b","plan_digest":"{digest}"}}"#)).is_err());
    for raw in [r#"{"kind":"schema","force":true}"#,r#"{"kind":"records","limit":true}"#,
        r#"{"kind":"records","limit":1.0}"#,r#"{"kind":"records","state":"control"}"#]{
        assert!(requests::parse::<QueryRequest>(raw).is_err(),"{raw}");
    }
    assert!(requests::parse::<QueryRequest>(&" ".repeat(2049)).is_err());
    for key in ["","a\n","a\0b","a/b","é"]{assert!(requests::key(key).is_err());}
    for d in ["A".repeat(64),format!("{}\n","a".repeat(63)),"a".repeat(65)]{assert!(requests::digest(&d).is_err());}
}
#[test]
fn plan_sampling_relations_are_checked_by_the_real_rust_constructor(){
    let p=fixture_plan();let raw=json!({"key":"goal","observation_witness":p.before().witness().to_string(),
        "game_ticks":100,"wall_millis":1000});
    assert!(requests::parse::<PlanRequest>(&raw.to_string()).unwrap().command().is_ok());
    for (key,value) in [("game_ticks",1),("samples",0),("samples",128),("interval_ticks",100),("stable_ticks",99),("wall_millis",60001)]{
        let mut invalid=raw.clone();invalid[key]=json!(value);
        assert!(requests::parse::<PlanRequest>(&invalid.to_string()).unwrap().command().is_err());
    }
}
#[test]
fn pending_work_remains_visible_beyond_first_history_page(){
    let (mut state,backend)=state(inventory(5,true),ExcavationMode::Offline);
    let result=invoke(&mut state,"fortress.query",Ok(ExcavationCommand::Inventory),records(Filter::All,4,None));
    assert_eq!(result["result"]["rows"].as_array().unwrap().len(),4);
    assert_eq!(result["result"]["matching_records"],6);
    assert_eq!(result["agent_turn"]["active_work"]["obligations"][0]["key"],"goal-005");
    assert_eq!(result["agent_turn"]["active_work"]["pending_absence_proven"],false);
    let token=result["result"]["continuation"].as_str().unwrap().to_owned();
    let next=invoke(&mut state,"fortress.query",Ok(ExcavationCommand::Inventory),records(Filter::All,4,Some(token)));
    assert_eq!(next["result"]["rows"].as_array().unwrap().len(),2);
    assert!(backend.calls.borrow().iter().all(|op|*op=="inspect"));
}
#[test]
fn continuations_bind_inventory_session_filter_and_page_width(){
    let view=inventory(6,false);let c=ctx(ExcavationMode::Offline);let t=turn(view.clone());
    let mut cursors=Cursors::default();
    let raw=presentation::render("fortress.query",&t,&c,ExcavationMode::Offline,&records(Filter::All,4,None),&mut cursors).unwrap();
    let result:Value=serde_json::from_str(&raw).unwrap();let token=result["result"]["continuation"].as_str().unwrap().to_owned();
    for request in [records(Filter::Terminal,4,Some(token.clone())),records(Filter::All,2,Some(token.clone()))]{
        assert!(presentation::render("fortress.query",&t,&c,ExcavationMode::Offline,&request,&mut cursors).is_err());
    }
    let query=records(Filter::All,4,Some(token.clone()));let mut other=c.clone();other.session_id=SessionId::new(2);
    assert!(presentation::render("fortress.query",&t,&other,ExcavationMode::Offline,&query,&mut cursors).is_err());
    assert!(presentation::render("fortress.query",&turn(inventory(7,false)),&c,ExcavationMode::Offline,&query,&mut cursors).is_err());
    let mut new_session=Cursors::default();
    assert!(presentation::render("fortress.query",&t,&c,ExcavationMode::Offline,&query,&mut new_session).is_err());
}
#[test]
fn invalid_request_returns_active_work_without_dispatching(){
    let (mut state,backend)=state(inventory(0,true),ExcavationMode::Offline);
    let result=invoke(&mut state,"fortress.commit",Err(invalid()),QueryRequest::default());
    assert_eq!(result["result"]["ok"],false);
    assert_eq!(result["agent_turn"]["active_work"]["inventory_verified"],true);
    assert_eq!(result["agent_turn"]["active_work"]["obligations"][0]["key"],"goal-000");
    assert!(backend.calls.borrow().iter().all(|op|*op=="inspect"));
}
#[test]
fn local_review_is_visible_and_not_reported_as_native_preparation(){
    let (mut state,backend)=state(inventory(0,false),ExcavationMode::Control);
    assert_eq!(invoke(&mut state,"fortress.observe",Ok(ExcavationCommand::Observe),QueryRequest::default())["result"]["ok"],true);
    let p=fixture_plan();let result=invoke(&mut state,"fortress.plan",Ok(ExcavationCommand::Plan{
        key:p.key().into(),witness:p.before().witness(),spec:p.spec()}),QueryRequest::default());
    assert_eq!(result["result"]["native_preparation_created"],false);
    assert_eq!(result["agent_turn"]["active_work"]["pending_plans"][0]["key"],p.key());
    assert_eq!(result["agent_turn"]["active_work"]["pending_absence_proven"],false);
    let cancelled=invoke(&mut state,"fortress.cancel",Ok(ExcavationCommand::CancelPlan{key:p.key().into(),digest:p.digest()}),QueryRequest::default());
    assert_eq!(cancelled["result"]["native_cancellation_dispatched"],false);
    assert!(backend.calls.borrow().iter().all(|op|matches!(*op,"inspect"|"observe")));
}
#[test]
fn unknown_final_inventory_keeps_attempt_and_marks_prior_unverified(){
    let c=ctx(ExcavationMode::Control);let mut t=turn(inventory(0,true));
    t.historical_prior=t.inventory.take();t.outcome=Err(error(ErrorCode::CorruptLedger,"injected"));
    t.uncertain_attempt=Some(ExcavationAttempt{key:"goal-000".into(),digest:fixture_plan().digest()});
    let raw=presentation::render("fortress.commit",&t,&c,ExcavationMode::Control,&QueryRequest::default(),&mut Cursors::default()).unwrap();
    let result:Value=serde_json::from_str(&raw).unwrap();
    assert_eq!(result["agent_turn"]["active_work"]["inventory_verified"],false);
    assert_eq!(result["agent_turn"]["active_work"]["pending_absence_proven"],false);
    assert!(result["agent_turn"]["anchor"].is_null());
    assert_eq!(result["agent_turn"]["active_work"]["indeterminate_effects"][0]["key"],"goal-000");
    assert!(result["result"]["unverified_historical_prior"].is_object());
}
#[test]
fn redacted_terrain_has_no_fabricated_fields_and_explanation_is_historical(){
    let p=fixture_plan();let original=p.before().canonical_bytes();let mut raw=original[..original.len()-16].to_vec();
    raw.extend_from_slice(&[0,1,0,1]);let capture=ExcavationCapture::decode(&raw).unwrap();
    let value=presentation::observation(&capture);
    for row in value["cells"].as_array().unwrap(){assert!(row.get("shape").is_none());assert!(row.get("liquid_depth").is_none());}
    let (mut state,_)=state(inventory(1,false),ExcavationMode::Offline);
    let digest=state.session.inventory().entries()[0].plan().digest();
    let result=invoke(&mut state,"fortress.explain",Ok(ExcavationCommand::Explain{key:"goal-000".into(),digest}),QueryRequest::default());
    assert_eq!(result["result"]["record"]["native"]["historical_pause_verified"],true);
    assert_eq!(result["result"]["record"]["native"]["sampled_floor_reported"],true);
    assert_eq!(result["result"]["current_pause_proven"],false);
    assert_eq!(result["result"]["mining_causality_proven"],false);
    assert!(result["result"]["last_native_sample"]["cells"].is_array());
}
#[test]
fn offline_mode_and_expired_grants_cannot_be_widened_by_requests(){
    assert_eq!(mode(None).unwrap(),ExcavationMode::Offline);
    assert!(mode(Some("admitted")).is_err());
    let (mut state,backend)=state(inventory(0,false),ExcavationMode::Offline);
    let denied=invoke(&mut state,"fortress.observe",Ok(ExcavationCommand::Observe),QueryRequest::default());
    assert_eq!(denied["result"]["ok"],false);
    for grant in &mut state.grants{grant.expires_at_tick=Some(GameTick(1));}
    let (mut expired,_)=self::state(inventory(1,false),ExcavationMode::Offline);
    for grant in &mut expired.grants{grant.expires_at_tick=Some(GameTick(1));}
    let result=invoke(&mut expired,"fortress.query",Ok(ExcavationCommand::Inventory),QueryRequest::default());
    assert_eq!(result["result"]["ok"],false);
    assert_eq!(result["agent_turn"]["active_work"]["inventory_verified"],false);
    assert!(result["result"].get("inventory").is_none());
    assert!(backend.calls.borrow().iter().all(|op|*op=="inspect"));
}
#[test]
fn published_schema_and_isolation_are_closed(){
    assert_eq!(inventory(0,false).digest().to_string(),"bbfbebc25dfc02666084a76ee467e135a016f4d31b98a015642f865c99e5bbd9");
    let schema:Value=serde_json::from_str(include_str!("../../../../schemas/mcp_excavation_run_v1.json")).unwrap();
    assert_eq!(schema["oneOf"].as_array().unwrap().len(),5);
    let names=vec!["DFMCP_ALLOW_UNADMITTED_EXCAVATION_RUN_V1_18".to_owned()];
    assert!(runtime::isolated(Some("1"),names.clone(),false).is_ok());
    for extra in ["DFMCP_ADMITTED_BRIDGE_PROTOCOL","DFMCP_ADMISSION_TICKET","DFMCP_DIG_TOKEN","DFMCP_EXCAVATION_RUN_JOURNAL"]{
        let mut invalid=names.clone();invalid.push(extra.into());assert!(runtime::isolated(Some("1"),invalid,false).is_err());
    }
    assert!(runtime::isolated(Some("1"),names.clone(),true).is_err());
    assert!(runtime::isolated(Some("true"),names,false).is_err());
}
#[test]
fn all_response_families_fit_minimum_admitted_output(){
    let c=ctx(ExcavationMode::Offline);let mut cursors=Cursors::default();
    for query in [records(Filter::All,4,None),QueryRequest::Schema{}]{
        let raw=presentation::render("fortress.query",&turn(inventory(4,true)),&c,ExcavationMode::Offline,&query,&mut cursors).unwrap();
        assert!(raw.len()<=RESPONSE_BYTES as usize);
    }
    let mut t=turn(inventory(1,false));let entry=t.inventory.as_ref().unwrap().entries()[0].clone();
    t.outcome=Ok(ExcavationOutcome::Effect{key:entry.plan().key().into(),native_record_found:None});
    let raw=presentation::render("fortress.explain",&t,&c,ExcavationMode::Offline,&QueryRequest::default(),&mut cursors).unwrap();
    assert!(raw.len()<=RESPONSE_BYTES as usize);
    let result:Value=serde_json::from_str(&raw).unwrap();
    assert_eq!(result["agent_turn"]["anchor"]["scope"],"retained_inventory_not_canonical_world");
    assert!(result["agent_turn"]["briefing"].get("admission").is_none());
}
#[test]
fn cursor_retention_is_bounded_and_old_tokens_expire(){
    let c=ctx(ExcavationMode::Offline);let t=turn(inventory(5,false));let mut cursors=Cursors::default();let mut first=String::new();
    for index in 0..65{
        let raw=presentation::render("fortress.query",&t,&c,ExcavationMode::Offline,&records(Filter::All,4,None),&mut cursors).unwrap();
        if index==0{first=serde_json::from_str::<Value>(&raw).unwrap()["result"]["continuation"].as_str().unwrap().into();}
    }
    assert!(presentation::render("fortress.query",&t,&c,ExcavationMode::Offline,&records(Filter::All,4,Some(first)),&mut cursors).is_err());
}
