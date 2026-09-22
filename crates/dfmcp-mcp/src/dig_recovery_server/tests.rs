use super::*;
use dfmcp_adapter::dig_designation::{DigEffect, DigObservation, DigPhase, DigPlan, DigRegion};
use dfmcp_adapter::dig_designation::rpc::{DigManifest, DigPreparation};
use dfmcp_adapter::dig_designation::journal::{DigGuard, DigJournal, DigStage};
use std::cell::RefCell;
use std::rc::Rc;
use std::io::{self,Cursor,Read,Seek,SeekFrom,Write};
use std::net::SocketAddr;

#[derive(Default)]
struct Disk {data:Cursor<Vec<u8>>,syncs:usize,fail_sync:bool,reads:usize,fail_read_at:Option<usize>}
#[derive(Clone,Default)]
struct Memory(Rc<RefCell<Disk>>);
impl Read for Memory {
    fn read(&mut self,out:&mut[u8])->io::Result<usize>{
        let mut d=self.0.borrow_mut();d.reads+=1;
        if d.fail_read_at==Some(d.reads){return Err(io::Error::other("injected read failure"));}d.data.read(out)
    }
}
impl Write for Memory {
    fn write(&mut self,raw:&[u8])->io::Result<usize>{
        let mut d=self.0.borrow_mut();
        if d.data.position()!=d.data.get_ref().len() as u64{return Err(io::Error::other("nonappend"));}d.data.write(raw)
    }
    fn flush(&mut self)->io::Result<()>{Ok(())}
}
impl Seek for Memory {fn seek(&mut self,at:SeekFrom)->io::Result<u64>{self.0.borrow_mut().data.seek(at)}}
impl EffectJournalStorage for Memory {
    fn sync(&mut self)->io::Result<()>{let mut d=self.0.borrow_mut();d.syncs+=1;if d.fail_sync{Err(io::Error::other("sync fault"))}else{Ok(())}}
    fn truncate(&mut self,_:u64)->io::Result<()>{Err(io::Error::other("no repair"))}
}
fn fixture(name:&str)->Result<Vec<u8>> {
    let raw=match name {
        "observation"=>include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/native/dig_designation/vectors/observation.hex")),
        "designated"=>include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/../../tests/native/dig_designation/vectors/designated.hex")),
        _=>return Err(error(ErrorCode::InvalidRequest,"fixture missing")),
    };
    let raw=raw.trim();let mut out=Vec::new();
    for pair in raw.as_bytes().chunks_exact(2){
        let text=std::str::from_utf8(pair).map_err(|_|exhausted())?;
        out.push(u8::from_str_radix(text,16).map_err(|_|exhausted())?);
    }Ok(out)
}
fn plan(key:&str)->Result<DigPlan>{DigPlan::new(key,false,DigObservation::decode(&fixture("observation")?)?)}
fn binding()->Result<DigBinding>{
    DigBinding::new("127.0.0.1:5000".parse().map_err(|_|exhausted())?,
        DigManifest{generation:7,df_version:"test-df".into(),dfhack_version:"test-dfhack".into()},plan("dig-001")?.before(),
        dfmcp_core::MapCuboid::new(dfmcp_core::MapCoord::new(0,0,0),dfmcp_core::MapCoord::new(63,63,7))?)
}
fn c()->Result<OperationContext>{
    let b=binding()?;let budget=WorkBudget{max_wall_millis:60_000,max_bytes:MAX_WORK_BYTES,
        max_output_tokens:8192,max_entities:1000,max_actions:1,max_game_ticks:0};
    Ok(context(SessionId::new(1),RequestId::new(1),b.fortress_id(),b.scope(),12345,budget))
}
fn control_context()->Result<OperationContext>{
    let mut c=c()?;let scope=c.grants[0].scope.clone();
    for capability in [Capability::Observe,Capability::Plan,Capability::Designate]{
        c.grants.push(CapabilityGrant{capability,scope:scope.clone(),max_risk:RiskTier::Guarded,expires_at_tick:None,remaining_uses:None});
    }Ok(c)
}
fn text(out:&mut Vec<u8>,s:&str){out.extend_from_slice(&(s.len() as u16).to_be_bytes());out.extend_from_slice(s.as_bytes());}
fn proof(domain:&[u8],raw:&[u8])->Digest32{let mut out=domain.to_vec();out.push(0);out.extend_from_slice(raw);Digest32::of_bytes(&out)}
fn effect(p:&DigPlan,phase:DigPhase)->Result<DigEffect>{
    let mut out=b"DFMDGE16".to_vec();
    for n in [p.before().generation(),p.before().sequence(),p.before().tick()]{out.extend_from_slice(&n.to_be_bytes());}
    for n in p.before().region().coordinates(){out.extend_from_slice(&n.to_be_bytes());}
    out.push(u8::from(p.allow_hidden_neighbors()));out.extend_from_slice(p.before().witness().as_bytes());
    out.extend_from_slice(p.digest().as_bytes());out.extend_from_slice(p.token());
    out.push(match phase{DigPhase::Prepared=>0,DigPhase::Unknown=>1,DigPhase::Designated=>2,DigPhase::Refused=>4});
    out.push(if phase==DigPhase::Refused{2}else{0});out.push(u8::from(phase==DigPhase::Designated));
    out.extend_from_slice(&(if phase==DigPhase::Designated{4u32}else{0u32}).to_be_bytes());
    if phase==DigPhase::Designated{out.extend_from_slice(&fixture("designated")?[140..172]);}else{out.extend_from_slice(&[0;32]);}
    let mut bytes=p.before().generation().to_be_bytes().to_vec();text(&mut bytes,p.key());
    bytes.extend_from_slice(p.digest().as_bytes());bytes.extend_from_slice(p.token());bytes.extend_from_slice(&out[133..172]);
    let receipt=if phase.terminal(){proof(b"dfmcp-dig-designation-receipt/1",&bytes)}else{Digest32::ZERO};
    out.extend_from_slice(receipt.as_bytes());text(&mut out,p.key());DigEffect::decode(&out,p)
}
#[derive(Default)]
struct Calls {connections:usize,queries:usize,mutations:usize,missing:bool,unknown:bool}
struct Source {calls:Rc<RefCell<Calls>>,binding:DigBinding}
impl DigSource for Source {
    fn manifest(&self)->&DigManifest{self.binding.manifest()}
    fn endpoint(&self)->Option<SocketAddr>{Some(self.binding.endpoint())}
    fn observe(&mut self,_:DigRegion,_:&OperationContext)->Result<DigObservation>{Ok(plan("dig-001")?.before().clone())}
    fn prepare(&mut self,p:&DigPlan,_:&OperationContext)->Result<DigPreparation>{
        self.calls.borrow_mut().mutations+=1;DigPreparation::decode(effect(p,DigPhase::Prepared)?.canonical_bytes(),false,p)
    }
    fn commit(&mut self,p:&DigPlan,_:&OperationContext)->Result<DigEffect>{self.calls.borrow_mut().mutations+=1;effect(p,DigPhase::Designated)}
    fn cancel(&mut self,p:&DigPlan,_:&OperationContext)->Result<DigEffect>{self.calls.borrow_mut().mutations+=1;effect(p,DigPhase::Refused)}
    fn query(&mut self,p:&DigPlan,_:&OperationContext)->Result<Option<DigEffect>>{
        let mut c=self.calls.borrow_mut();c.queries+=1;
        if c.missing{Ok(None)}else{Ok(Some(effect(p,if c.unknown{DigPhase::Unknown}else{DigPhase::Designated})?))}
    }
}
struct Guard {build:bool}
impl DigGuard for Guard {
    fn check(&mut self,s:DigStage,_:&DigPlan,_:&OperationContext)->Result<()>{
        if self.build||s==DigStage::Query{Ok(())}else{Err(error(ErrorCode::CapabilityDenied,"test query-only guard"))}
    }
}
impl DigSessionGuard for Guard {
    fn connect(&mut self,_:&DigBinding,_:DigRegion,_:&OperationContext)->Result<()>{Ok(())}
    fn observe(&mut self,_:&DigBinding,_:DigRegion,_:&OperationContext)->Result<()>{Err(error(ErrorCode::CapabilityDenied,"no terrain reads"))}
}
fn seeded(memory:Memory,terminal:usize,pending:bool)->Result<DigPlan>{
    let mut journal=DigJournal::open(memory,&control_context()?,DigMode::Control,Some(binding()?),Some([9;32]))?;
    let calls=Rc::new(RefCell::new(Calls::default()));let mut source=Source{calls,binding:binding()?};let mut guard=Guard{build:true};
    for i in 0..terminal {
        let p=plan(&format!("old-{i:03}"))?;journal.prepare(&mut source,&p,&control_context()?,&mut guard)?;
        journal.cancel(&mut source,p.key(),p.digest(),&control_context()?,&mut guard)?;
    }
    let p=plan("zz-pending")?;
    if pending{journal.prepare(&mut source,&p,&control_context()?,&mut guard)?;}
    Ok(p)
}
fn state(memory:Memory,mode:DigMode)->Result<State<Memory,QueryOnly<Source>>>{
    let journal=DigJournal::open(memory,&c()?,mode,Some(binding()?),None)?;
    State::new(DigSession::new(journal,&c()?)?,&c()?)
}
fn factory(calls:Rc<RefCell<Calls>>)->impl FnOnce(&DigBinding,DigRegion,&OperationContext)->Result<QueryOnly<Source>>{
    move|b,_,_|{calls.borrow_mut().connections+=1;Ok(QueryOnly(Source{calls,binding:b.clone()}))}
}
fn run(s:&mut State<Memory,QueryOnly<Source>>,op:&str,action:Result<Action>,calls:Rc<RefCell<Calls>>)->Result<Value>{
    let c=s.context(None)?;let output=run_action(s,c,Instant::now(),op,action,factory(calls),&mut Guard{build:false});
    assert!(output.len() as u64<=OUTPUT_BYTES);
    let value:Value=serde_json::from_str(&output).map_err(|_|exhausted())?;
    assert!(value["agent_turn"].is_object());assert_eq!(value["agent_turn"]["briefing"]["mutation_admissible"],false);
    assert!(!output.contains("private-token"));Ok(value)
}

#[test]
fn cold_arrival_discovers_pending_work_outside_the_first_records_page()->Result<()>{
    let memory=Memory::default();seeded(memory.clone(),9,true)?;let mut s=state(memory,DigMode::Offline)?;
    let calls=Rc::new(RefCell::new(Calls::default()));
    let value=run(&mut s,"fortress.query",Ok(Action::Query(Query::parse("{\"mode\":\"records\",\"limit\":8}")?)),calls.clone())?;
    assert_eq!(value["result"]["records"].as_array().map(Vec::len),Some(8));
    assert_eq!(value["agent_turn"]["active_work"]["indeterminate_effects"][0]["idempotency_key"],"zz-pending");
    assert_eq!(value["agent_turn"]["active_work"]["pending_absence_proven"],false);
    assert_eq!(calls.borrow().connections,0);Ok(())
}
#[test]
fn discovery_continuation_is_pinned_and_staged_without_native_calls()->Result<()>{
    let memory=Memory::default();seeded(memory.clone(),9,true)?;let mut s=state(memory,DigMode::Offline)?;
    let calls=Rc::new(RefCell::new(Calls::default()));
    let first=run(&mut s,"fortress.query",Ok(Action::Query(Query::Records{limit:Some(8),continuation:None})),calls.clone())?;
    let token=first["result"]["continuation"].as_str().ok_or_else(exhausted)?.to_owned();
    let second=run(&mut s,"fortress.query",Ok(Action::Query(Query::Records{limit:Some(8),continuation:Some(token.clone())})),calls.clone())?;
    assert_eq!(second["result"]["records"].as_array().map(Vec::len),Some(2));
    let wrong=run(&mut s,"fortress.query",Ok(Action::Query(Query::Records{limit:Some(1),continuation:Some(token)})),calls.clone())?;
    assert_eq!(wrong["result"]["ok"],false);assert_eq!(calls.borrow().queries,0);Ok(())
}
#[test]
fn exact_plan_explanation_and_tile_pages_remain_historical()->Result<()>{
    let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory,DigMode::Offline)?;
    let calls=Rc::new(RefCell::new(Calls::default()));
    let value=run(&mut s,"fortress.explain",Ok(Action::Explain(p.key().into(),p.digest())),calls.clone())?;
    assert_eq!(value["result"]["record"]["historical_evidence_only"],true);
    assert_eq!(value["result"]["record"]["dispatch_permitted"],false);
    let query=Query::Tiles{idempotency_key:p.key().into(),plan_digest:p.digest().to_string(),offset:32,limit:Some(16)};
    let page=run(&mut s,"fortress.query",Ok(Action::Query(query)),calls.clone())?;
    assert_eq!(page["result"]["capture"]["tiles"].as_array().map(Vec::len),Some(16));
    assert_eq!(page["result"]["capture"]["next_query"],Value::Null);assert_eq!(calls.borrow().connections,0);Ok(())
}
#[test]
fn query_only_reconciliation_persists_terminal_proof_without_any_mutation()->Result<()>{
    let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory.clone(),DigMode::Recover)?;
    let calls=Rc::new(RefCell::new(Calls::default()));let before=memory.0.borrow().syncs;
    let value=run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),p.digest())),calls.clone())?;
    assert_eq!(value["result"]["record"]["state"],"terminal");assert!(memory.0.borrow().syncs>before);
    assert_eq!(calls.borrow().queries,1);assert_eq!(calls.borrow().connections,1);assert_eq!(calls.borrow().mutations,0);
    drop(s);let mut offline=state(memory,DigMode::Offline)?;
    let recovered=run(&mut offline,"fortress.explain",Ok(Action::Explain(p.key().into(),p.digest())),calls.clone())?;
    assert_eq!(recovered["result"]["record"]["native"]["phase"],"designated");assert_eq!(calls.borrow().connections,1);Ok(())
}
#[test]
fn missing_native_evidence_does_not_clear_the_pending_record()->Result<()>{
    let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory,DigMode::Recover)?;
    let calls=Rc::new(RefCell::new(Calls{missing:true,..Calls::default()}));
    let value=run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),p.digest())),calls.clone())?;
    assert_eq!(value["result"]["ok"],false);assert_eq!(value["agent_turn"]["active_work"]["counts"]["unsettled_records"],1);
    assert_eq!(calls.borrow().mutations,0);Ok(())
}
#[test]
fn terminal_and_permanent_unknown_skip_subsequent_connection_factories()->Result<()>{
    for unknown in [false,true] {
        let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory,DigMode::Recover)?;
        let calls=Rc::new(RefCell::new(Calls{unknown,..Calls::default()}));
        run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),p.digest())),calls.clone())?;
        let value=run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),p.digest())),calls.clone())?;
        assert_eq!(value["result"]["native_calls"],0);assert_eq!(calls.borrow().connections,1);
        assert_eq!(value["agent_turn"]["active_work"]["pending_absence_proven"],!unknown);
    }Ok(())
}
#[test]
fn offline_wait_and_all_mutation_tools_refuse_without_evaluating_factory()->Result<()>{
    let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory.clone(),DigMode::Offline)?;
    let calls=Rc::new(RefCell::new(Calls::default()));let before=memory.0.borrow().data.get_ref().clone();
    let value=run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),p.digest())),calls.clone())?;
    assert_eq!(value["result"]["ok"],false);
    for op in ["fortress.plan","fortress.commit","fortress.checkpoint","fortress.restore"]{
        assert_eq!(run(&mut s,op,Ok(Action::Denied),calls.clone())?["result"]["ok"],false);
    }
    assert_eq!(calls.borrow().connections,0);assert_eq!(memory.0.borrow().data.get_ref(),&before);Ok(())
}
#[test]
fn native_query_wrapper_independently_rejects_every_write_and_terrain_read()->Result<()>{
    let calls=Rc::new(RefCell::new(Calls::default()));let mut source=QueryOnly(Source{calls:calls.clone(),binding:binding()?});let p=plan("dig-001")?;
    assert!(source.observe(p.before().region(),&control_context()?).is_err());
    assert!(source.prepare(&p,&control_context()?).is_err());assert!(source.commit(&p,&control_context()?).is_err());
    assert!(source.cancel(&p,&control_context()?).is_err());assert_eq!(calls.borrow().mutations,0);
    assert!(source.query(&p,&c()?)?.is_some());assert_eq!(calls.borrow().queries,1);Ok(())
}
#[test]
fn wrong_digest_and_closed_query_shapes_fail_before_native_work()->Result<()>{
    for raw in ["{}","{\"mode\":\"commit\"}","{\"mode\":\"records\",\"limit\":9}",
        "{\"mode\":\"schema\",\"path\":\"/private\"}","{\"mode\":\"records\",\"limit\":true}",
        "{\"mode\":\"records\",\"continuation\":\"x\"}","{\"mode\":\"records\",\"limit\":1,\"limit\":2}"]{
        assert!(Query::parse(raw).is_err(),"{raw}");
    }
    let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory,DigMode::Recover)?;
    let calls=Rc::new(RefCell::new(Calls::default()));
    assert_eq!(run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),Digest32::ZERO)),calls.clone())?["result"]["ok"],false);
    assert_eq!(calls.borrow().connections,0);Ok(())
}
#[test]
fn output_and_expired_wall_allowances_are_refused_before_journal_reads()->Result<()>{
    let memory=Memory::default();seeded(memory.clone(),0,true)?;let mut s=state(memory.clone(),DigMode::Offline)?;
    let reads=memory.0.borrow().reads;let calls=Rc::new(RefCell::new(Calls::default()));
    for expired in [false,true]{
        let mut c=s.context(None)?;if !expired{c.budget.max_output_tokens=1;}
        let started=if expired{Instant::now().checked_sub(Duration::from_secs(61)).ok_or_else(exhausted)?}else{Instant::now()};
        let value=run_action(&mut s,c,started,"fortress.query",Ok(Action::Inventory),factory(calls.clone()),&mut Guard{build:false});
        let parsed:Value=serde_json::from_str(&value).map_err(|_|exhausted())?;assert_eq!(parsed["result"]["ok"],false);
    }
    assert_eq!(memory.0.borrow().reads,reads);assert_eq!(calls.borrow().connections,0);Ok(())
}
#[test]
fn receipt_sync_failure_never_publishes_a_known_outcome_or_empty_active_work()->Result<()>{
    let memory=Memory::default();let p=seeded(memory.clone(),0,true)?;let mut s=state(memory.clone(),DigMode::Recover)?;
    memory.0.borrow_mut().fail_sync=true;let calls=Rc::new(RefCell::new(Calls::default()));
    let value=run(&mut s,"fortress.wait",Ok(Action::Wait(p.key().into(),p.digest())),calls.clone())?;
    assert_eq!(value["result"]["ok"],false);
    assert_eq!(value["agent_turn"]["active_work"]["inventory_verified"],false);
    assert_eq!(value["agent_turn"]["active_work"]["pending_absence_proven"],false);assert_eq!(calls.borrow().mutations,0);Ok(())
}
#[test]
fn unknown_custody_never_reuses_cached_success_metadata()->Result<()>{
    let memory=Memory::default();seeded(memory.clone(),0,true)?;let mut s=state(memory.clone(),DigMode::Offline)?;
    memory.0.borrow_mut().data.get_mut()[0]^=1;let calls=Rc::new(RefCell::new(Calls::default()));
    let value=run(&mut s,"fortress.doctor",Ok(Action::Inventory),calls.clone())?;
    assert_eq!(value["result"]["ok"],false);assert_eq!(value["agent_turn"]["active_work"]["inventory_verified"],false);
    assert_eq!(calls.borrow().connections,0);Ok(())
}
#[test]
fn recovery_state_refuses_a_control_journal_even_with_broad_injected_grants()->Result<()>{
    let memory=Memory::default();seeded(memory.clone(),0,false)?;
    let journal=DigJournal::open(memory,&control_context()?,DigMode::Control,Some(binding()?),None)?;
    let session:DigSession<_,QueryOnly<Source>>=DigSession::new(journal,&control_context()?)?;
    assert!(State::new(session,&control_context()?).is_err());Ok(())
}
#[test]
fn full_packets_and_fallbacks_fit_the_admitted_minimum_output_budget()->Result<()>{
    let memory=Memory::default();let p=seeded(memory.clone(),9,true)?;let mut s=state(memory,DigMode::Offline)?;
    let calls=Rc::new(RefCell::new(Calls::default()));
    for action in [Action::Inventory,Action::Explain(p.key().into(),p.digest()),Action::Denied,
        Action::Query(Query::Records{limit:Some(8),continuation:None}),Action::Query(Query::Schema),
        Action::Query(Query::Tiles{idempotency_key:p.key().into(),plan_digest:p.digest().to_string(),offset:0,limit:Some(16)})]{
        run(&mut s,"fortress.query",Ok(action),calls.clone())?;
    }
    for op in ["fortress.open_session","fortress.query","fortress.wait","fortress.cancel"]{
        let value=unbound(op,&error(ErrorCode::InternalInvariantViolation,"private-token"));
        assert!(value.len() as u64<=OUTPUT_BYTES);assert!(!value.contains("private-token"));
    }Ok(())
}

#[test]
fn post_page_custody_failure_does_not_publish_an_unreturned_cursor()->Result<()>{
    let memory=Memory::default();seeded(memory.clone(),9,true)?;let mut s=state(memory.clone(),DigMode::Offline)?;
    {let mut disk=memory.0.borrow_mut();let chunks=disk.data.get_ref().len().div_ceil(32768);
        disk.fail_read_at=Some(disk.reads+2*chunks+1);}
    let calls=Rc::new(RefCell::new(Calls::default()));
    let value=run(&mut s,"fortress.query",Ok(Action::Query(Query::Records{limit:Some(8),continuation:None})),calls)?;
    assert_eq!(value["result"]["ok"],false);assert!(s.cursors.entries.is_empty());Ok(())
}
#[test]
fn worst_bounded_source_metadata_and_result_shapes_fit_complete_packets()->Result<()>{
    let raw=fixture("observation")?;let mut large=raw[..69].to_vec();
    text(&mut large,&"\u{1}".repeat(512));large.extend_from_slice(&raw[78..]);
    let before=DigObservation::decode(&large)?;
    let b=DigBinding::new(binding()?.endpoint(),DigManifest{generation:7,df_version:"\u{1}".repeat(128),
        dfhack_version:"\u{1}".repeat(128)},&before,binding()?.scope())?;
    let pending=dfmcp_adapter::dig_designation::journal::DigSummary{key:"k".repeat(128),
        plan_digest:Digest32::from_bytes([255;32]),state:dfmcp_adapter::dig_designation::journal::DigState::Tracking,
        native_phase:Some(DigPhase::Unknown),receipt:None,dispatchable:false};
    let view=DigSessionView{journal_id:Digest32::from_bytes([255;32]),head:Digest32::from_bytes([255;32]),events:896,
        byte_len:MAX_JOURNAL_BYTES,total_records:128,pending:Some(pending.clone())};
    let row=json!({"coordinate":[32767,32767,32767],"presence":"visible","tiletype":u32::MAX,
        "designation_other":u32::MAX,"occupancy":u32::MAX,"priority":7000,"cooldown":u32::MAX,
        "block_other":u32::MAX,"temperatures":[u16::MAX,u16::MAX],"dig":7,"hazards":15,"flags":31});
    for result in [json!({"ok":true,"tiles":vec![row;16]}),
        json!({"ok":true,"records":vec![presentation::summary(&pending);8],"continuation":"f".repeat(64)}),
        failure(&exhausted())]{
        let output=packet("fortress.query",result,Some(&c()?),Some(DigMode::Recover),Some(&b),Some(&view));
        assert!(output.len() as u64<=OUTPUT_BYTES,"{}",output.len());
    }Ok(())
}
