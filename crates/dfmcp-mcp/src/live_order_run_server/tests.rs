use super::*;
use dfmcp_adapter::order_run::OrderRunRecord;
use dfmcp_adapter::order_run::rpc::OrderRunManifest;
use std::cell::RefCell;
use std::rc::Rc;
use std::io::{self,Read,Write,Seek,SeekFrom};

#[derive(Clone,Default)]
struct Memory{bytes:Rc<RefCell<Vec<u8>>>,offset:u64}
impl Read for Memory{fn read(&mut self,out:&mut[u8])->io::Result<usize>{let b=self.bytes.borrow();let at=(self.offset as usize).min(b.len());
    let n=out.len().min(b.len()-at);out[..n].copy_from_slice(&b[at..at+n]);self.offset+=n as u64;Ok(n)}}
impl Write for Memory{fn write(&mut self,data:&[u8])->io::Result<usize>{let mut b=self.bytes.borrow_mut();let at=self.offset as usize;
    if b.len()<at+data.len(){b.resize(at+data.len(),0);}b[at..at+data.len()].copy_from_slice(data);self.offset+=data.len() as u64;Ok(data.len())}
    fn flush(&mut self)->io::Result<()>{Ok(())}}
impl Seek for Memory{fn seek(&mut self,from:SeekFrom)->io::Result<u64>{let n=match from{SeekFrom::Start(n)=>i128::from(n),
    SeekFrom::Current(n)=>i128::from(self.offset)+i128::from(n),SeekFrom::End(n)=>self.bytes.borrow().len() as i128+i128::from(n)};
    self.offset=u64::try_from(n).map_err(|_|io::Error::other("seek"))?;Ok(self.offset)}}
impl EffectJournalStorage for Memory{fn sync(&mut self)->io::Result<()>{Ok(())}fn truncate(&mut self,_:u64)->io::Result<()>{Err(io::Error::other("immutable"))}}
#[derive(Default)]
struct Calls{connect:usize,observe:usize,prepare:usize,commit:usize,query:usize,cancel:usize,lost_commit:bool,record:Option<OrderRunRecord>}
struct Source{calls:Rc<RefCell<Calls>>,manifest:OrderRunManifest,fortress:FortressIdentity,before:OrderCapture}
fn fixture()->Result<OrderRunRecord>{
    let raw=include_str!("../../../dfmcp-adapter/tests/fixtures/order_run_predicate_v1_14.hex").trim();
    let bytes=raw.as_bytes().chunks_exact(2).map(|pair|{
        let text=std::str::from_utf8(pair).map_err(|_|error(ErrorCode::InvalidRequest,"fixture"))?;
        u8::from_str_radix(text,16).map_err(|_|error(ErrorCode::InvalidRequest,"fixture"))
    }).collect::<Result<Vec<_>>>()?;OrderRunRecord::decode(&bytes)
}
fn native_record(plan:&OrderRunPlan,phase:u8,reason:u8)->Result<OrderRunRecord>{
    fn field(out:&mut Vec<u8>,v:&[u8]){out.extend_from_slice(&(v.len() as u16).to_be_bytes());out.extend_from_slice(v);}
    let mut out=b"DFMOE014".to_vec();field(&mut out,plan.key().as_bytes());field(&mut out,plan.native_bytes());
    out.extend_from_slice(plan.digest().as_bytes());out.extend_from_slice(plan.token());
    let known=matches!(phase,1..=3);out.extend_from_slice(&[phase,reason,0,u8::from(known),u8::from(phase==3),u8::from(known)]);
    out.extend_from_slice(&(if known{plan.before().tick()}else{0}).to_be_bytes());out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&plan.before().tick().to_be_bytes());field(&mut out,&[]);
    let mut proof=b"dfmcp-order-run-receipt/1\0".to_vec();proof.extend_from_slice(&out);
    out.extend_from_slice(Digest32::of_bytes(&proof).as_bytes());OrderRunRecord::decode(&out)
}
impl Source{fn new(calls:Rc<RefCell<Calls>>)->Result<Self>{let p=fixture()?.plan().clone();Ok(Self{calls,
    manifest:OrderRunManifest{generation:41,df_version:"df".into(),dfhack_version:"dfhack".into()},fortress:p.before().fortress().clone(),before:p.before().clone()})}}
impl OrderRunSource for Source{
    fn manifest(&self)->&OrderRunManifest{&self.manifest}fn endpoint(&self)->Option<SocketAddr>{Some(SocketAddr::from(([127,0,0,1],5000)))}
    fn fortress(&self)->&FortressIdentity{&self.fortress}fn fence(&mut self){}
    fn observe(&mut self,_:u32,_:&OperationContext,_:Duration)->Result<OrderCapture>{self.calls.borrow_mut().observe+=1;Ok(self.before.clone())}
    fn prepare(&mut self,p:&OrderRunPlan,_:&OperationContext,_:Duration)->Result<OrderRunRecord>{let r=native_record(p,0,0)?;
        let mut c=self.calls.borrow_mut();c.prepare+=1;c.record=Some(r.clone());Ok(r)}
    fn commit(&mut self,p:&OrderRunPlan,_:&OperationContext,_:Duration)->Result<OrderRunRecord>{let r=native_record(p,1,0)?;
        let mut c=self.calls.borrow_mut();c.commit+=1;c.record=Some(r.clone());
        if c.lost_commit{return Err(error(ErrorCode::AdapterUnavailable,"lost commit reply"));}Ok(r)}
    fn query(&mut self,_:&OrderRunPlan,_:&OperationContext,_:Duration)->Result<Option<OrderRunRecord>>{let mut c=self.calls.borrow_mut();c.query+=1;Ok(c.record.clone())}
    fn cancel(&mut self,p:&OrderRunPlan,_:&OperationContext,_:Duration)->Result<OrderRunRecord>{let r=native_record(p,3,3)?;
        let mut c=self.calls.borrow_mut();c.cancel+=1;c.record=Some(r.clone());Ok(r)}
}
fn setup(memory:Memory,calls:Rc<RefCell<Calls>>,mode:OrderRunMode,initialize:bool)->Result<State<Memory>>{
    let n=Source::new(calls)?;let fortress=n.fortress().clone();let id=SessionId::new(1);
    let budget=WorkBudget{max_bytes:32*1024*1024,max_output_tokens:65536,max_wall_millis:5000,max_game_ticks:1200,max_entities:256,max_actions:1};
    let grants=grants(mode,&fortress,true)?;
    let c=OperationContext{session_id:id,request_id:RequestId::new(1),anchor:StateAnchor{fortress_id:fortress.fortress_id(),
        cursor:ObservationCursor::ORIGIN,tick:GameTick(100),state_hash:Digest32::ZERO},budget,grants:grants.clone(),cancellation_requested:false};
    let binding=if mode==OrderRunMode::Control{Some(OrderRunBinding::from_source(&n)?)}else{None};
    let journal=OrderRunJournal::open(memory,&c,mode,&fortress,binding,initialize)?;
    Ok(State{id,request:1,budget,grants,high_tick:100,journal,selected:None,cursors:Cursors::default()})
}
fn dispatch(state:&mut State<Memory>,calls:&Rc<RefCell<Calls>>,op:&str,action:Action)->Result<Value>{
    dispatch_limits(state,calls,op,action,Limits::default(),1)
}
fn dispatch_limits(state:&mut State<Memory>,calls:&Rc<RefCell<Calls>>,op:&str,action:Action,limits:Limits,rows:usize)->Result<Value>{
    let c=state.context(true,false)?;let calls=calls.clone();
    let output=run_action(state,c,Dispatch{operation:op,limits,rows,action:Ok(action)},move |_|{
        calls.borrow_mut().connect+=1;Source::new(calls)
    });serde_json::from_str(&output).map_err(|_|error(ErrorCode::InvalidRequest,"MCP JSON result"))
}
fn condition()->Result<Condition>{Condition::parse(r#"{"order_id":9,"predicate":"approved","game_ticks":20,"wall_millis":1000,"stable_samples":2,"interval_ticks":2}"#)}
fn prepare(state:&mut State<Memory>,calls:&Rc<RefCell<Calls>>,key:&str)->Result<Digest32>{
    let observed=dispatch(state,calls,"fortress.observe",Action::Observe(9))?;assert_eq!(observed["result"]["ok"],true);
    let witness=digest(observed["result"]["observation"]["witness"].as_str().ok_or_else(budget_error)?)?;
    let result=dispatch(state,calls,"fortress.plan",Action::Plan{key:key.into(),witness,condition:condition()?})?;
    assert_eq!(result["result"]["ok"],true);
    digest(result["result"]["effect"]["plan"]["plan_digest"].as_str().ok_or_else(budget_error)?)
}
#[test]
fn dispatcher_lifecycle_and_offline_recovery_keep_goal_pause_and_goods_distinct()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m.clone(),calls.clone(),OrderRunMode::Control,true)?;
    let p=prepare(&mut state,&calls,"approval")?;
    let result=dispatch(&mut state,&calls,"fortress.commit",Action::Commit{key:"approval".into(),plan:p,confirm:true})?;
    assert_eq!(result["result"]["effect"]["native"]["phase"],"running");calls.borrow_mut().record=Some(fixture()?);
    let result=dispatch(&mut state,&calls,"fortress.wait",Action::Wait{key:"approval".into(),plan:p})?;
    assert_eq!(result["result"]["effect"]["native"]["predicate_observed"],true);
    assert_eq!(result["result"]["effect"]["native"]["pause_verified"],true);assert_eq!(result["result"]["goal_completion_proven"],false);
    assert_eq!(result["agent_turn"]["anchor"]["scope"],"coordination_root_not_world_state");drop(state);
    let count=calls.borrow().connect;let mut offline=setup(m,calls.clone(),OrderRunMode::Offline,false)?;
    let recovered=dispatch(&mut offline,&calls,"fortress.explain",Action::Explain{key:"approval".into(),plan:p})?;
    assert_eq!(recovered["result"]["effect"],result["result"]["effect"]);assert_eq!(calls.borrow().connect,count);
    assert_eq!(calls.borrow().commit,1);assert_eq!(calls.borrow().query,1);Ok(())
}
#[test]
fn missing_commit_reply_reopen_cannot_recommit_or_erase_uncertainty()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m.clone(),calls.clone(),OrderRunMode::Control,true)?;
    let p=prepare(&mut state,&calls,"approval")?;calls.borrow_mut().lost_commit=true;
    assert_eq!(dispatch(&mut state,&calls,"fortress.commit",Action::Commit{key:"approval".into(),plan:p,confirm:true})?["result"]["ok"],false);drop(state);
    let mut state=setup(m,calls.clone(),OrderRunMode::Control,false)?;let connects=calls.borrow().connect;
    assert_eq!(dispatch(&mut state,&calls,"fortress.commit",Action::Commit{key:"approval".into(),plan:p,confirm:true})?["result"]["ok"],false);
    assert_eq!(calls.borrow().connect,connects);calls.borrow_mut().record=None;
    let absent=dispatch(&mut state,&calls,"fortress.wait",Action::Wait{key:"approval".into(),plan:p})?;
    assert_eq!(absent["result"]["ok"],false);assert_eq!(absent["agent_turn"]["active_work"]["counts"]["unresolved"],1);assert_eq!(calls.borrow().commit,1);Ok(())
}
#[test]
fn confirmation_and_mode_refusals_precede_connection_even_with_injected_grants()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m.clone(),calls.clone(),OrderRunMode::Control,true)?;
    let p=prepare(&mut state,&calls,"approval")?;let count=calls.borrow().connect;
    assert_eq!(dispatch(&mut state,&calls,"fortress.commit",Action::Commit{key:"approval".into(),plan:p,confirm:false})?["result"]["ok"],false);
    assert_eq!(calls.borrow().connect,count);drop(state);
    for mode in [OrderRunMode::Offline,OrderRunMode::Recover]{
        let mut state=setup(m.clone(),calls.clone(),mode,false)?;state.grants=grants(OrderRunMode::Control,fixture()?.plan().before().fortress(),true)?;
        assert_eq!(dispatch(&mut state,&calls,"fortress.commit",Action::Commit{key:"approval".into(),plan:p,confirm:true})?["result"]["ok"],false);
        assert_eq!(dispatch(&mut state,&calls,"fortress.cancel",Action::Cancel{key:"approval".into(),plan:p})?["result"]["ok"],false);
    }
    assert_eq!(calls.borrow().connect,count);Ok(())
}
#[test]
fn local_cancel_avoids_connection_active_cancel_sends_only_safety_request()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m,calls.clone(),OrderRunMode::Control,true)?;
    let p=prepare(&mut state,&calls,"local")?;let count=calls.borrow().connect;
    let cancelled=dispatch(&mut state,&calls,"fortress.cancel",Action::Cancel{key:"local".into(),plan:p})?;
    assert_eq!(cancelled["result"]["effect"]["state"],"cancelled_before_dispatch");assert_eq!(calls.borrow().connect,count);
    let p=prepare(&mut state,&calls,"active")?;
    dispatch(&mut state,&calls,"fortress.commit",Action::Commit{key:"active".into(),plan:p,confirm:true})?;
    let stopped=dispatch(&mut state,&calls,"fortress.cancel",Action::Cancel{key:"active".into(),plan:p})?;
    assert_eq!(stopped["result"]["effect"]["native"]["pause_verified"],true);assert_eq!(calls.borrow().cancel,1);assert_eq!(calls.borrow().commit,1);Ok(())
}
#[test]
fn output_work_and_tick_limits_refuse_before_native_effects()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m,calls.clone(),OrderRunMode::Control,true)?;
    let out=dispatch_limits(&mut state,&calls,"fortress.observe",Action::Observe(9),Limits{tokens:Some(1),..Limits::default()},1)?;
    assert_eq!(out["result"]["ok"],false);assert_eq!(calls.borrow().connect,0);
    let p=prepare(&mut state,&calls,"approval")?;let count=calls.borrow().connect;
    let out=dispatch_limits(&mut state,&calls,"fortress.commit",Action::Commit{key:"approval".into(),plan:p,confirm:true},Limits{ticks:Some(0),..Limits::default()},1)?;
    assert_eq!(out["result"]["ok"],false);assert_eq!(calls.borrow().connect,count);assert_eq!(calls.borrow().commit,0);Ok(())
}
#[test]
fn whole_row_pagination_binds_filter_head_and_session_without_native_calls()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m,calls.clone(),OrderRunMode::Control,true)?;
    for key in ["a","b","c"]{let p=prepare(&mut state,&calls,key)?;dispatch(&mut state,&calls,"fortress.cancel",Action::Cancel{key:key.into(),plan:p})?;}
    let count=calls.borrow().connect;
    let first=dispatch(&mut state,&calls,"fortress.query",Action::Query{filter:Filter::All,limit:1,continuation:None})?;
    let token=first["result"]["continuation"].as_str().ok_or_else(budget_error)?.to_owned();
    let second=dispatch(&mut state,&calls,"fortress.query",Action::Query{filter:Filter::All,limit:1,continuation:Some(token.clone())})?;
    assert_eq!(second["result"]["records"][0]["plan"]["idempotency_key"],"b");
    assert_eq!(second["result"]["complete_matching_set_in_this_response"],false);
    assert_eq!(dispatch(&mut state,&calls,"fortress.query",Action::Query{filter:Filter::Terminal,limit:1,continuation:Some(token.clone())})?["result"]["ok"],false);
    let c=state.context(true,false)?;let mut view=state.journal.view(&c)?;
    view.head=Digest32::ZERO;assert!(state.cursors.resolve(&token,state.id,&view,Filter::All,1).is_err());
    assert!(state.cursors.resolve(&token,SessionId::new(999),&view,Filter::All,1).is_err());
    assert_eq!(calls.borrow().connect,count);Ok(())
}
#[test]
fn native_failure_does_not_remove_healthy_local_history()->Result<()>{
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m,calls.clone(),OrderRunMode::Control,true)?;
    let p=prepare(&mut state,&calls,"approval")?;let c=state.context(true,false)?;
    let failed=run_action::<_,Source,_>(&mut state,c,Dispatch{operation:"fortress.observe",limits:Limits::default(),rows:1,action:Ok(Action::Observe(9))},
        |_|Err(error(ErrorCode::AdapterUnavailable,"offline native test")));
    assert!(state.selected.is_none());assert_eq!(serde_json::from_str::<Value>(&failed).map_err(|_|budget_error())?["result"]["ok"],false);
    assert_eq!(dispatch(&mut state,&calls,"fortress.explain",Action::Explain{key:"approval".into(),plan:p})?["result"]["ok"],true);Ok(())
}
#[test]
fn runtime_and_environment_gates_reject_without_caller_or_with_admission()->Result<()>{
    assert!(runtime_io().is_err());
    assert!(environment_contract(Some("1"),None,&[],false).is_ok());
    for (opt,clock,admitted) in [(None,None,false),(Some("true"),None,false),(Some("1"),Some("0"),false),(Some("1"),None,true)]{
        assert!(environment_contract(opt,clock,&[],admitted).is_err());
    }
    assert!(environment_contract(Some("1"),None,&["DFMCP_ADMITTED_BRIDGE_PROTOCOL".into()],false).is_err());
    assert!(grants(OrderRunMode::Control,&FortressIdentity::new("region1",7)?,false).is_err());Ok(())
}
#[test]
fn per_effect_guard_can_revoke_after_dispatch_marker_without_unpause()->Result<()>{
    fn deny(clock:bool,_:&FortressIdentity)->Result<()>{if clock{Err(error(ErrorCode::CapabilityDenied,"revoked"))}else{Ok(())}}
    let m=Memory::default();let calls=Rc::new(RefCell::new(Calls::default()));let mut state=setup(m,calls.clone(),OrderRunMode::Control,true)?;
    let p=prepare(&mut state,&calls,"approval")?;let c=state.context(true,false)?;let calls_for_source=calls.clone();
    let out=run_action(&mut state,c,Dispatch{operation:"fortress.commit",limits:Limits::default(),rows:1,
        action:Ok(Action::Commit{key:"approval".into(),plan:p,confirm:true})},move |_|Ok(CheckedSource{inner:Source::new(calls_for_source)?,check:deny}));
    let result:Value=serde_json::from_str(&out).map_err(|_|budget_error())?;
    assert_eq!(result["result"]["ok"],false);assert_eq!(calls.borrow().commit,0);
    let c=state.context(true,false)?;assert_eq!(state.journal.view(&c)?.entries[0].state(),OrderRunState::DispatchStarted);Ok(())
}
#[test]
fn strict_condition_and_digest_parsing_reject_unknown_or_ambiguous_inputs()->Result<()>{
    condition()?;
    for raw in [r#"{"order_id":9,"predicate":"shell","game_ticks":20,"wall_millis":1000}"#,
        r#"{"order_id":9,"predicate":"approved","game_ticks":20,"wall_millis":1000,"threshold":1}"#,
        r#"{"order_id":9,"predicate":"approved","game_ticks":20,"wall_millis":1000,"command":"x"}"#,
        r#"{"order_id":9,"order_id":8,"predicate":"approved","game_ticks":20,"wall_millis":1000}"#,
        r#"{"order_id":9,"predicate":"approved","game_ticks":20,"wall_millis":1000,"stable_samples":16,"interval_ticks":2}"#]{assert!(Condition::parse(raw).is_err());}
    assert!(Condition::parse(&" ".repeat(2049)).is_err());assert!(digest(&"A".repeat(64)).is_err());assert!(digest("0").is_err());Ok(())
}
#[test]
fn packets_keep_scope_and_do_not_invent_verified_empty_history()->Result<()>{
    let cause=error(ErrorCode::AdapterUnavailable,"no source");let encoded=unbound("fortress.observe",&cause);
    let parsed:Value=serde_json::from_str(&encoded).map_err(|_|budget_error())?;
    assert_eq!(parsed["agent_turn"]["schema"],"dfmcp.agent_turn/1");assert_eq!(parsed["agent_turn"]["active_work"]["absence_proven"],false);
    assert!(parsed["agent_turn"]["anchor"].is_null());assert!(parsed["agent_turn"]["briefing"]["admission"].is_null());
    assert_eq!(parsed["agent_turn"]["briefing"]["runtime_admitted"],false);assert!(encoded.len()<BASE_OUTPUT as usize);Ok(())
}

#[test]
fn generated_tool_definitions_keep_the_exact_eleven_dotted_names(){
    use fastmcp_rust::__private::server::ToolHandler;
    let definitions=[FortressOpenSession.definition(),FortressObserve.definition(),FortressQuery.definition(),
        FortressPlan.definition(),FortressCommit.definition(),FortressWait.definition(),FortressCancel.definition(),
        FortressCheckpoint.definition(),FortressRestore.definition(),FortressExplain.definition(),FortressDoctor.definition()];
    let names=definitions.iter().map(|d|d.name.as_str()).collect::<Vec<_>>();
    assert_eq!(names,vec!["fortress.open_session","fortress.observe","fortress.query","fortress.plan","fortress.commit",
        "fortress.wait","fortress.cancel","fortress.checkpoint","fortress.restore","fortress.explain","fortress.doctor"]);
}
