use super::*;
use dfmcp_core::{CapabilityGrant,CapabilityScope,FortressId,GameTick,ObservationCursor,RequestId,WorkBudget};
use dfmcp_world::{WorldGraph,WorldSnapshot};

fn snapshot(sequence:u64,tick:u64)->WorldSnapshot {
    WorldSnapshot::new(FortressId::new(7),GameTick(tick),ObservationCursor {epoch:1,sequence},true,WorldGraph::default())
}
fn context(snapshot:&WorldSnapshot)->OperationContext {
    OperationContext {session_id:SessionId::new(8401),request_id:RequestId::new(1),anchor:snapshot.anchor(),
        budget:WorkBudget {max_entities:1000,max_game_ticks:1000,max_wall_millis:60000,
            max_bytes:262144,max_output_tokens:65536,..WorkBudget::default()},
        grants:[Capability::Query,Capability::Observe].into_iter().map(|capability|CapabilityGrant {
            capability,scope:CapabilityScope::default(),max_risk:RiskTier::ReadOnly,expires_at_tick:None,remaining_uses:None
        }).collect(),cancellation_requested:false}
}
fn encoded(value:Value)->Result<String> {Ok(value.to_string())}
fn decode(text:&str)->Result<Value> {serde_json::from_str(text).map_err(|_|invalid("test JSON"))}
fn register(store:&Mutex<Store>,s:&WorldSnapshot,c:&OperationContext,key:&str,condition:Value,stable:u32)->Result<String> {
    let result=super::super::execute_in(store,s,c,&json!({"schema":"dfmcp.query/1","query":{
        "kind":"watch","key":key,"condition":condition,"deadline_tick":100,
        "poll_interval_ticks":1,"stable_observations":stable}}),encoded)?;
    decode(&result)?["record"]["watch"].as_str().map(str::to_owned).ok_or_else(||invalid("test handle"))
}
fn paused(store:&Mutex<Store>,s:&WorldSnapshot,c:&OperationContext,key:&str,stable:u32)->Result<String> {
    register(store,s,c,key,json!({"op":"paused","value":true}),stable)
}
fn request(awaiting:bool,handles:Option<&[String]>)->Value {
    let mut q=json!({"schema":"dfmcp.query/1","query":{"kind":if awaiting {"await_watches"} else {"poll_watches"}}});
    if let Some(handles)=handles {q["query"]["watches"]=json!(handles);}q
}
fn row<'a>(value:&'a Value,key:&str)->Result<&'a Value> {
    value["records"].as_array().and_then(|rows|rows.iter().find(|r|r["key"]==key)).ok_or_else(||invalid("batch row missing"))
}

#[test]
fn batches_advance_competing_watches_on_every_shared_capture()->Result<()> {
    let store=Mutex::new(Store::default());let mut s=snapshot(0,1);let mut c=context(&s);
    paused(&store,&s,&c,"a",3)?;paused(&store,&s,&c,"b",3)?;
    for n in 1..=2 {
        let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;assert!(prepared.needs_observation());
        s=snapshot(n,n+1);c.anchor=s.anchor();
        let result=decode(&complete_in(&store,&s,&c,prepared,true,encoded)?)?;
        assert_eq!(result["selected"],2);assert_eq!(result["sampled"],2);
        for key in ["a","b"] {assert_eq!(row(&result,key)?["stable_observations"],n+1);}
        assert_eq!(result["all_satisfied"],n==2);
    }
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    assert!(!prepared.needs_observation());
    let result=decode(&complete_in(&store,&s,&c,prepared,false,encoded)?)?;
    assert_eq!(result["advanced"],0);assert_eq!(result["all_terminal"],true);
    assert_eq!(row(&result,"a")?["terminal_replayed"],true);Ok(())
}

#[test]
fn heartbeat_and_cadence_do_not_manufacture_samples()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
    paused(&store,&s,&c,"a",3)?;
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    let result=decode(&complete_in(&store,&s,&c,prepared,true,encoded)?)?;
    assert_eq!(result["sampled"],0);assert_eq!(result["advanced"],0);
    let later=snapshot(1,1);let mut next=c.clone();next.anchor=later.anchor();
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    let result=decode(&complete_in(&store,&later,&next,prepared,true,encoded)?)?;
    assert_eq!(result["sampled"],0);assert_eq!(row(&result,"a")?["stable_observations"],1);Ok(())
}

#[test]
fn explicit_selection_is_canonical_and_does_not_advance_unselected_work()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
    let a=paused(&store,&s,&c,"a",3)?;let b=paused(&store,&s,&c,"b",3)?;
    let prior=record(&*lock(&store)?,c.session_id,&b)?.evidence_digest;
    let prepared=prepare_in(&store,&s,&c,&request(true,Some(&[a])),encoded)?;
    let later=snapshot(1,2);let mut next=c;next.anchor=later.anchor();
    let result=decode(&complete_in(&store,&later,&next,prepared,true,encoded)?)?;
    assert_eq!(result["selected"],1);assert_eq!(result["sampled"],1);
    assert_eq!(record(&*lock(&store)?,next.session_id,&b)?.evidence_digest,prior);Ok(())
}

#[test]
fn malformed_duplicate_foreign_and_stale_selections_fail_before_preview()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
    let a=paused(&store,&s,&c,"a",3)?;
    let mut stale=request(true,None);stale["expected_anchor"]=json!({});
    let mut extra=request(true,None);extra["query"]["wait_forever"]=json!(true);
    for input in [request(true,Some(&[])),request(true,Some(&[a.clone(),a.clone()])),
        request(true,Some(&[format!("watch:{}","f".repeat(64))])),request(true,Some(&["bad".into()])),stale,extra] {
        let mut previewed=false;
        assert!(prepare_in(&store,&s,&c,&input,|v|{previewed=true;encoded(v)}).is_err());assert!(!previewed);
    }
    let mut other=c;other.session_id=SessionId::new(8402);
    assert!(prepare_in(&store,&s,&other,&request(true,Some(&[a])),encoded).is_err());Ok(())
}

#[test]
fn empty_and_terminal_only_batches_need_no_observe_authority()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let mut c=context(&s);
    c.grants.retain(|g|g.capability==Capability::Query);
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;assert!(!prepared.needs_observation());
    let result=decode(&complete_in(&store,&s,&c,prepared,false,encoded)?)?;
    assert_eq!(result["selected"],0);assert_eq!(result["all_terminal"],true);assert_eq!(result["all_satisfied"],false);
    paused(&store,&s,&c,"terminal",1)?;
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;assert!(!prepared.needs_observation());
    paused(&store,&s,&c,"pending",3)?;
    assert!(matches!(prepare_in(&store,&s,&c,&request(true,None),encoded),Err(e)if e.code==ErrorCode::CapabilityDenied));
    assert!(prepare_in(&store,&s,&c,&request(false,None),encoded).is_ok());Ok(())
}

#[test]
fn failed_render_or_late_watch_error_publishes_none_of_the_batch()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
    paused(&store,&s,&c,"a",3)?;let b=paused(&store,&s,&c,"b",3)?;
    let prior=registry(&*lock(&store)?,c.session_id)?;
    let later=snapshot(1,2);let mut next=c.clone();next.anchor=later.anchor();
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    assert!(complete_in(&store,&later,&next,prepared,true,|_|Err(bounded("injected renderer failure"))).is_err());
    assert_eq!(registry(&*lock(&store)?,c.session_id)?,prior);
    {let mut guard=lock(&store)?;let w=guard.entries.get_mut(&(c.session_id,b)).ok_or_else(||invalid("test watch"))?;
        w.samples=u64::MAX;w.seal()?;}
    let prior=registry(&*lock(&store)?,c.session_id)?;
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    assert!(complete_in(&store,&later,&next,prepared,true,encoded).is_err());
    assert_eq!(registry(&*lock(&store)?,c.session_id)?,prior);Ok(())
}

#[test]
fn registry_change_during_capture_is_a_conflict_not_a_partial_poll()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
    paused(&store,&s,&c,"a",3)?;
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    paused(&store,&s,&c,"new",3)?;
    let prior=registry(&*lock(&store)?,c.session_id)?;
    let later=snapshot(1,2);let mut next=c.clone();next.anchor=later.anchor();
    assert!(matches!(complete_in(&store,&later,&next,prepared,true,encoded),Err(e)if e.code==ErrorCode::Conflict));
    assert_eq!(registry(&*lock(&store)?,c.session_id)?,prior);
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    let mut other=c.clone();other.session_id=SessionId::new(8403);paused(&store,&s,&other,"other",3)?;
    assert!(complete_in(&store,&later,&next,prepared,true,encoded).is_ok());Ok(())
}

#[test]
fn reset_expiration_and_unknown_remain_distinct_batch_outcomes()->Result<()> {
    for reset in [false,true] {
        let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
        paused(&store,&s,&c,"a",3)?;paused(&store,&s,&c,"b",3)?;
        let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
        let later=if reset {WorldSnapshot::new(FortressId::new(7),GameTick(2),ObservationCursor {epoch:2,sequence:0},true,WorldGraph::default())}
            else {snapshot(1,101)};
        let mut next=c;next.anchor=later.anchor();
        let result=decode(&complete_in(&store,&later,&next,prepared,true,encoded)?)?;
        assert_eq!(result["all_terminal"],true);assert_eq!(result["all_satisfied"],false);
        assert_eq!(row(&result,"a")?["status"],if reset {"invalidated"} else {"expired"});
    }
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);
    register(&store,&s,&c,"unknown",json!({"op":"field","entity_id":"1","generation":1,
        "field":"alive","comparison":"eq","value":{"type":"bool","value":true}}),2)?;
    let prepared=prepare_in(&store,&s,&c,&request(false,None),encoded)?;
    let result=decode(&complete_in(&store,&s,&c,prepared,false,encoded)?)?;
    assert_eq!(row(&result,"unknown")?["condition"],"unknown");assert_eq!(result["all_satisfied"],false);Ok(())
}

#[test]
fn acquisition_identity_and_post_refresh_authority_are_rechecked()->Result<()> {
    let store=Mutex::new(Store::default());let s=snapshot(0,1);let c=context(&s);paused(&store,&s,&c,"a",3)?;
    let later=snapshot(1,2);let mut next=c.clone();next.anchor=later.anchor();
    for case in 0..4 {
        let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
        let mut denied=next.clone();match case {
            0=>denied.grants.clear(),1=>denied.cancellation_requested=true,
            2=>denied.grants.iter_mut().for_each(|g|g.expires_at_tick=Some(GameTick(1))),
            _=>denied.session_id=SessionId::new(8404),
        }
        assert!(complete_in(&store,&later,&denied,prepared,true,encoded).is_err());
    }
    let prepared=prepare_in(&store,&s,&c,&request(true,None),encoded)?;
    assert!(complete_in(&store,&later,&next,prepared,false,encoded).is_err());Ok(())
}
