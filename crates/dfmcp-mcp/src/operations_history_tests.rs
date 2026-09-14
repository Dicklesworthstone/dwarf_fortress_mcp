use super::*;
use dfmcp_adapter::operations_journal::TailRecovery;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

static SERIAL_HISTORY:Mutex<()>=Mutex::new(());
struct Archived {session:Registered,path:PathBuf,dir:PathBuf}
impl Drop for Archived {
    fn drop(&mut self){
        if let Ok(mut registry)=SESSIONS.lock(){registry.remove(&self.session.id);}
        let _=std::fs::remove_file(&self.path);let _=std::fs::remove_dir(&self.dir);
    }
}
fn archived(tokens:u32)->Result<Archived> {
    let session=register(tokens,vec![Capability::Observe,Capability::Query,Capability::Doctor])?;
    let dir=std::env::temp_dir().join(format!("dfmcp-handler-history-{}",session.id));
    std::fs::create_dir(&dir).map_err(|_|error(ErrorCode::InternalInvariantViolation,"test directory"))?;
    std::fs::set_permissions(&dir,std::fs::Permissions::from_mode(0o700))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"test mode"))?;
    let dir=dir.canonicalize().map_err(|_|error(ErrorCode::InternalInvariantViolation,"test canonical directory"))?;
    let path=dir.join("operations.bin");
    let handle=resolve(session.handle())?;
    {let mut guard=lock(&handle)?;let context=guard.context()?;history::attach(&mut guard,&path,TailRecovery::Refuse,&context)?;}
    Ok(Archived{session,path,dir})
}
fn listing(session:&Registered)->Result<Value>{ask(session,json!({"kind":"history"}))}
fn past_request(row:&Value)->Value {
    json!({"kind":"historical_query","record":row["record"],"record_digest":row["record_digest"],
        "query":{"kind":"inspect","entity_id":item_entity_id(30).to_string(),"generation":1,"fields":["stack_size"]}})
}

#[test]
fn archived_inventory_is_queryable_without_changing_live_state_or_reading_bridge()->Result<()> {
    let _serial=lock(&SERIAL_HISTORY)?;let fixture=archived(8192)?;let session=&fixture.session;
    let before=listing(session)?;assert_eq!(before["ok"],true);assert_eq!(before["matched"],1);
    assert_eq!(session.calls.load(Ordering::SeqCst),0);
    let request=past_request(&before["rows"][0]);
    let live=decode(&fortress_observe(session.handle()))?;assert_eq!(live["ok"],true);
    assert_eq!(listing(session)?["matched"],2);
    let past=ask(session,request.clone())?;
    assert_eq!(past["ok"],true);assert_eq!(past["historical"],true);
    assert_eq!(past["row"]["fields"]["stack_size"]["value"]["value"],5);
    assert_eq!(past["anchor"],before["rows"][0]["anchor"]);assert_eq!(past["agent_turn"]["anchor"],past["anchor"]);
    assert_eq!(past["current_live_anchor"],live["anchor"]);
    assert_eq!(past["agent_turn"]["briefing"]["live"],false);
    assert_eq!(past["agent_turn"]["continuity"]["status"],"partial");
    assert_eq!(past["agent_turn"]["coverage"]["current_freshness_proven"],false);
    assert_eq!(ask(session,request)?["row"],past["row"]);
    let current=ask(session,json!({"kind":"inspect","entity_id":item_entity_id(30).to_string(),"generation":1,"fields":["stack_size"]}))?;
    assert_eq!(current["row"]["fields"]["stack_size"]["value"]["value"],8);
    assert_eq!(current["anchor"],live["anchor"]);assert_eq!(session.calls.load(Ordering::SeqCst),1);Ok(())
}

#[test]
fn journal_reopen_restores_the_exact_version_chain_but_not_session_handles()->Result<()> {
    let _serial=lock(&SERIAL_HISTORY)?;let mut fixture=archived(8192)?;
    assert_eq!(decode(&fortress_observe(fixture.session.handle()))?["ok"],true);
    let old_listing=listing(&fixture.session)?;let old_id=fixture.session.id;
    let source={let handle=resolve(fixture.session.handle())?;let guard=lock(&handle)?;
        guard.state.observation().cloned().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"test source"))?};
    lock(&SESSIONS)?.remove(&old_id);
    let next=full()?;
    {let handle=resolve(next.handle())?;let mut guard=lock(&handle)?;
        guard.state.publish(source)?;let context=guard.context()?;
        history::attach(&mut guard,&fixture.path,TailRecovery::Refuse,&context)?;}
    fixture.session=next;
    let restored=listing(&fixture.session)?;
    assert_ne!(fixture.session.id,old_id);assert!(resolve(Some(old_id.to_string())).is_err());
    assert_eq!(restored["rows"],old_listing["rows"]);assert_eq!(restored["anchor"],old_listing["anchor"]);
    assert_eq!(restored["history"]["journal_id"],old_listing["history"]["journal_id"]);
    assert_eq!(ask(&fixture.session,past_request(&restored["rows"][0]))?["ok"],true);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst),0);Ok(())
}

#[test]
fn history_pages_fit_minimum_budget_keep_active_watches_and_bind_the_head()->Result<()> {
    let _serial=lock(&SERIAL_HISTORY)?;let fixture=archived(2048)?;let session=&fixture.session;
    let watch=ask(session,json!({"kind":"watch","key":"history-test","label":"Retain current work",
        "condition":{"op":"tick_at_least","value":105u64*403200+500},
        "deadline_tick":105u64*403200+1000,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(watch["ok"],true);
    {let handle=resolve(session.handle())?;let mut guard=lock(&handle)?;
        let original=observation()?;
        let observations=(4..=16).map(|tick|{let mut v=original.clone();v.jobs.year_tick=tick;v}).collect();
        guard.source=Box::new(Script{observations,fenced:false,calls:Arc::clone(&session.calls)});}
    for _ in 0..12{assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],true);}
    let mut query=json!({"kind":"history","limit":64});let mut records=Vec::new();let mut first_cursor=None;let mut pages=0;
    loop {
        let raw=fortress_query(session.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":query})));
        assert!(raw.len()<=8192);let result=decode(&raw)?;assert_eq!(result["ok"],true,"{raw}");
        assert_eq!(result["agent_turn"]["active_work"]["obligations"][0]["watch"],watch["record"]["watch"]);
        let rows=result["rows"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"test history rows"))?;
        for row in rows{records.push(row["record"].as_u64().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"test record number"))?);}
        pages+=1;assert!(pages<=13);
        if result["continuation"].is_null(){break;}
        if first_cursor.is_none(){first_cursor=Some(result["continuation"].clone());}
        query["continuation"]=result["continuation"].clone();query["limit"]=json!(3);
    }
    assert_eq!(records,(1..=13).collect::<Vec<_>>());assert!(pages>1);
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],true);
    let stale=ask(session,json!({"kind":"history","continuation":first_cursor}))?;
    assert_eq!(stale["error"]["code"],"stale_anchor");
    assert_eq!(ask(session,json!({"kind":"cancel_watch","watch":watch["record"]["watch"]}))?["ok"],true);
    assert_eq!(ask(session,json!({"kind":"release_watch","watch":watch["record"]["watch"]}))?["ok"],true);Ok(())
}

#[test]
fn historical_reads_remain_available_after_native_failure_but_never_create_work()->Result<()> {
    let _serial=lock(&SERIAL_HISTORY)?;let fixture=archived(8192)?;let session=&fixture.session;
    let row=listing(session)?["rows"][0].clone();
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],true);
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"],false);
    let calls=session.calls.load(Ordering::SeqCst);
    assert_eq!(listing(session)?["source_stale"],true);
    assert_eq!(ask(session,past_request(&row))?["ok"],true);
    for kind in ["watch","capture","await_watch","inventory_plan","historical_query"] {
        let mut request=past_request(&row);request["query"]=json!({"kind":kind});
        assert_eq!(ask(session,request)?["error"]["code"],"invalid_request");
    }
    assert_eq!(ask(session,json!({"kind":"entities"}))?["ok"],false);
    assert_eq!(decode(&fortress_commit(session.handle()))?["error"]["code"],"capability_denied");
    assert_eq!(session.calls.load(Ordering::SeqCst),calls);Ok(())
}

#[test]
fn missing_history_bad_record_and_expired_query_authority_fail_closed()->Result<()> {
    let _serial=lock(&SERIAL_HISTORY)?;
    {let session=full()?;assert_eq!(listing(&session)?["error"]["code"],"invalid_request");}
    let fixture=archived(8192)?;let session=&fixture.session;
    let mut request=past_request(&listing(session)?["rows"][0]);request["record_digest"]=json!("0".repeat(64));
    assert_eq!(ask(session,request)?["error"]["code"],"stale_anchor");
    assert_eq!(ask(session,json!({"kind":"history","path":"/etc/passwd"}))?["error"]["code"],"invalid_request");
    assert_eq!(ask(session,json!({"kind":"history","limit":0}))?["error"]["code"],"budget_exceeded");
    {let handle=resolve(session.handle())?;let mut guard=lock(&handle)?;
        let now=guard.anchor()?.tick;
        for grant in &mut guard.grants{if grant.capability==Capability::Query{grant.expires_at_tick=Some(dfmcp_core::GameTick(now.0-1));}}}
    let denied=listing(session)?;assert_eq!(denied["error"]["code"],"capability_denied");assert!(denied.get("anchor").is_none());
    assert_eq!(session.calls.load(Ordering::SeqCst),0);Ok(())
}
