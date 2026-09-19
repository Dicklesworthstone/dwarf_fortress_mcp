//! Fixed spatial/1.8 history. Archived citizen/job/terrain facts remain distinct
//! from current session authority, watches, baselines and any control runtime.
#[path = "spatial_history_changes.rs"]
pub(super) mod changes;
use super::*;
use std::path::{Path,PathBuf};
use dfmcp_adapter::operations_journal::{JournalEntry,JournalLimits,SpatialCitizenJournal,
    PrivateJournalFile,Spatial18,TailRecovery,open_profile_journal};
use dfmcp_core::Digest32;

pub(super) type Journal=SpatialCitizenJournal<PrivateJournalFile>;
const STATELESS: [&str;13] = ["entities","inspect","traverse","dependencies","aggregate","search",
    "map_route","spatial_inventory_plan","production_diagnosis","inventory_plan","item_quantity","production_portfolio",
    "condition_evaluation"];

pub(super) fn configuration()->Result<Option<(PathBuf,TailRecovery)>>{
    let path=match std::env::var("DFMCP_SPATIAL_CITIZEN_JOURNAL"){
        Ok(path)if !path.is_empty()=>Some(PathBuf::from(path)),
        Err(std::env::VarError::NotPresent)=>None,
        _=>return Err(error(ErrorCode::InvalidRequest,"DFMCP_SPATIAL_CITIZEN_JOURNAL must be nonempty UTF-8")),
    };
    let recovery=match std::env::var("DFMCP_SPATIAL_CITIZEN_JOURNAL_REPAIR"){
        Err(std::env::VarError::NotPresent)=>TailRecovery::Refuse,
        Ok(value)if value=="1"&&path.is_some()=>TailRecovery::TruncateIncomplete,
        _=>return Err(error(ErrorCode::InvalidRequest,"spatial/1.8 journal repair requires configured path and exact value 1")),
    };
    Ok(path.map(|path|(path,recovery)))
}
pub(super) fn replay_context(session:&Session,c:&OperationContext)->OperationContext{
    let mut replay=c.clone();replay.budget.max_bytes=session.limits.spatial.operations.payload_bytes as u64;replay
}
fn validate_limits(value:&LiveSpatialCitizenObservation,limits:CitizenSpatialLimits)->Result<()>{
    let op=value.spatial().operations();let counts=limits.spatial.operations;
    if op.jobs.jobs.len()>counts.jobs as usize||op.buildings.len()>counts.buildings as usize||op.items.len()>counts.items as usize
        ||value.citizens().len()>limits.citizens as usize||value.spatial().terrain().map.region!=limits.spatial.region
        ||value.encode_payload()?.len()>counts.payload_bytes{
        return Err(error(ErrorCode::BudgetExceeded,"archived spatial/1.8 capture exceeds current acquisition bounds"));}
    Ok(())
}
pub(super) fn attach(session:&mut Session,path:&Path,recovery:TailRecovery,c:&OperationContext)->Result<()>{
    let current=session.state.observation_full().cloned().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial/1.8 bootstrap source missing"))?;
    validate_limits(&current,session.limits)?;let replay=replay_context(session,c);
    let mut journal=open_profile_journal::<Spatial18>(path,&replay,JournalLimits::default(),recovery)?;
    if let Some(prior)=journal.state().observation_full(){validate_limits(prior,session.limits)?;}
    journal.append(current,&replay)?;session.state=journal.state().clone();session.journal=Some(journal);Ok(())
}
pub(super) fn summary(journal:Option<&Journal>)->Value{match journal{
    None=>json!({"durable_observation_history":false}),
    Some(j)=>json!({"storage":"synced_observation_journal","profile":j.profile(),"durable_observation_history":true,
        "journal_id":j.id().to_string(),"head":j.head().to_string(),"records":j.entries().len(),"retained_bytes":j.retained_bytes(),
        "repaired_tail_bytes":j.repaired_tail_bytes(),"fenced":j.fenced(),"continuous_game_history":false,
        "watches_and_baselines_durable":false,"citizen_generation_history_durable":true}),}}

#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Envelope{schema:String,expected_anchor:Option<Value>,query:Request}
#[derive(Deserialize)]#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]enum Request{
    History{limit:Option<u32>,continuation:Option<String>},HistoricalQuery{record:u64,record_digest:String,query:Value}}
fn parse(input:&Value,c:&OperationContext)->Result<Request>{
    let mut pending=vec![(input,0usize)];let mut nodes=0usize;let mut bytes=0usize;
    while let Some((value,depth))=pending.pop(){nodes+=1;bytes=bytes.saturating_add(16);if nodes>4096||depth>32{return Err(error(ErrorCode::BudgetExceeded,"spatial/1.8 history query exceeds shape bound"));}
        match value{Value::String(text)=>bytes=bytes.saturating_add(text.len()),Value::Object(map)=>{if nodes+pending.len()+map.len()>4096{return Err(error(ErrorCode::BudgetExceeded,"history object too wide"));}
            for(key,value)in map{bytes=bytes.saturating_add(key.len());pending.push((value,depth+1));}},Value::Array(array)=>{if nodes+pending.len()+array.len()>4096{return Err(error(ErrorCode::BudgetExceeded,"history array too wide"));}
            pending.extend(array.iter().map(|v|(v,depth+1)));},_=>{}}
        if bytes>128*1024{return Err(error(ErrorCode::BudgetExceeded,"spatial/1.8 history query exceeds byte bound"));}}
    let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|error(ErrorCode::InvalidRequest,"invalid spatial/1.8 history envelope"))?;
    if envelope.schema!="dfmcp.query/1"{return Err(error(ErrorCode::InvalidRequest,"history requires dfmcp.query/1"));}
    if envelope.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(c.anchor)){return Err(error(ErrorCode::StaleAnchor,"history expected_anchor differs from current session"));}
    Ok(envelope.query)
}
fn entry_json(e:&JournalEntry)->Value{json!({"record":e.number,"anchor":anchor_json(e.anchor),"source_digest":e.source_digest.to_string(),
    "record_digest":e.record_digest.to_string(),"previous_digest":e.previous_digest.to_string(),"encoded_bytes":e.encoded_bytes})}
fn cursor(j:&Journal,c:&OperationContext,offset:usize)->String{
    let identity=json!({"domain":"dfmcp-spatial-citizen-history-page/1","journal":j.id().to_string(),"head":j.head().to_string(),
        "session":c.session_id.to_string(),"anchor":anchor_json(c.anchor),"offset":offset});
    format!("sch1:{offset}:{}",Digest32::of_bytes(identity.to_string().as_bytes()))
}
fn history_page(j:&Journal,c:&OperationContext,maximum:usize,limit:Option<u32>,continuation:Option<String>)->Result<Value>{
    let limit=limit.unwrap_or(8);if !(1..=64).contains(&limit){return Err(error(ErrorCode::BudgetExceeded,"history page limit must be 1..64"));}
    let start=match continuation{None=>0,Some(token)=>{if token.len()>128{return Err(error(ErrorCode::BudgetExceeded,"history continuation exceeds bound"));}
        let mut parts=token.split(':');let(Some("sch1"),Some(raw),Some(_),None)=(parts.next(),parts.next(),parts.next(),parts.next())else{return Err(error(ErrorCode::InvalidRequest,"invalid spatial/1.8 history continuation"));};
        let n=raw.parse::<usize>().map_err(|_|error(ErrorCode::InvalidRequest,"invalid history offset"))?;
        if n==0||n>=j.entries().len()||token!=cursor(j,c,n){return Err(error(ErrorCode::StaleAnchor,"history continuation names another session, archive head or anchor"));}n}};
    let mut result=json!({"schema":"dfmcp.query.result/1","kind":"history","anchor":anchor_json(c.anchor),"history":summary(Some(j)),
        "matched":j.entries().len(),"rows":[],"returned":0,"truncated":false,"continuation":null,"native_captures":0,"temporal_coverage":"retained_observation_endpoints_only"});
    let mut end=start;for entry in j.entries().iter().skip(start).take(limit as usize){let mut candidate=result.clone();
        candidate["rows"].as_array_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"history rows missing"))?.push(entry_json(entry));
        candidate["returned"]=json!(end+1-start);candidate["truncated"]=json!(end+1<j.entries().len());candidate["continuation"]=if end+1<j.entries().len(){json!(cursor(j,c,end+1))}else{Value::Null};
        if candidate.to_string().len()>maximum{break;}result=candidate;end+=1;}
    if(start==end&&start<j.entries().len())||result.to_string().len()>maximum{return Err(error(ErrorCode::BudgetExceeded,"one complete historical metadata row does not fit"));}Ok(result)
}
pub(super) fn handles(input:&Value)->bool{changes::handles(input)||matches!(input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str),Some("history"|"historical_query"))}
fn stateless(kind:&str)->bool{STATELESS.contains(&kind)}

pub(super) fn execute(session:&mut Session,c:&OperationContext,input:&Value)->Result<String>{
    if changes::handles(input){return changes::execute(session,c,input);}
    c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;let request=parse(input,c)?;
    if let Request::HistoricalQuery{query,record,..}=&request{if !stateless(query.get("kind").and_then(Value::as_str).unwrap_or(""))||!(1..=4096).contains(record){
        return Err(error(ErrorCode::InvalidRequest,"historical_query permits retained records and stateless reads only"));}}
    let mut projection=view(session,c)?;let replay=replay_context(session,c);let source_fenced=session.source.poisoned();let limits=session.limits;
    let journal=session.journal.as_mut().ok_or_else(||error(ErrorCode::InvalidRequest,"spatial/1.8 history is not configured; journal paths are operator configuration"))?;
    journal.validate_custody(c)?;
    match request{
        Request::History{limit,continuation}=>{let mut rc=c.clone();rc.budget.max_bytes=projection.result_byte_budget()? as u64;let rc=semantic_query::result_context(&rc)?;
            let mut value=history_page(journal,c,rc.budget.max_bytes as usize,limit,continuation)?;value["source_stale"]=json!(source_fenced);
            semantic_query::publish_with_active_work(c,value,|v|finish(&projection,v))}
        Request::HistoricalQuery{record,record_digest,query}=>{
            let digest=Digest32::from_hex(&record_digest).filter(|d|d.to_string()==record_digest).ok_or_else(||error(ErrorCode::InvalidRequest,"record_digest must be canonical lowercase SHA-256"))?;
            let state=journal.state_at(record,digest,&replay)?;validate_limits(state.observation_full().ok_or_else(||error(ErrorCode::CorruptLedger,"archived spatial/1.8 source missing"))?,limits)?;
            let snapshot=state.snapshot().ok_or_else(||error(ErrorCode::CorruptLedger,"archived spatial/1.8 snapshot missing"))?;
            let entry=journal.entries().get((record-1)as usize).cloned().ok_or_else(||error(ErrorCode::CursorGap,"historical record disappeared"))?;
            projection.anchor=anchor_json(snapshot.anchor());projection.briefing=json!({"runtime":"unadmitted_development","bridge_protocol":"1.8",
                "observation_profile":"coherent-citizen-spatial","runtime_admitted":false,"mutation_admissible":false,"read_only":true,"historical":true,"live":false,
                "current_live_anchor":anchor_json(c.anchor),"live_source_fenced":source_fenced,"active_work_basis":"current_session_not_archived","query_source":"verified_spatial_citizen_replay"});
            projection.references=vec![json!({"kind":"archived_coherent_citizen_spatial_capture","digest":entry.source_digest.to_string(),
                "record_digest":entry.record_digest.to_string(),"journal_id":journal.id().to_string()})];projection.coverage["temporal_coverage"]=json!("historical_observation_only");projection.coverage["current_freshness_proven"]=json!(false);
            let metadata=json!({"historical":true,"journal_record":entry_json(&entry),"current_live_anchor":anchor_json(c.anchor),"native_captures":0});
            let mut qc=c.clone();qc.budget.max_bytes=projection.result_byte_budget()?.checked_sub(metadata.to_string().len()+256)
                .ok_or_else(||error(ErrorCode::BudgetExceeded,"historical context leaves no result budget"))? as u64;qc=semantic_query::result_context(&qc)?;qc.anchor=snapshot.anchor();
            let envelope=json!({"schema":"dfmcp.query/1","query":query});let mut value=
                if production::handles(&envelope){production::execute(&state,&qc,&envelope)?}
                else if spatial_queries::handles(&envelope){spatial_queries::execute(&state,&qc,&envelope)?}
                else{semantic_query::execute(snapshot,&qc,&envelope)?};
            production::pin_historical(&mut value,record,digest)?;
            if let Some(rows)=value.get_mut("rows").and_then(Value::as_array_mut){for row in rows{if let Some(route)=row.get("route_query"){
                let wrapped=json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query","record":record,"record_digest":record_digest,"query":route.get("query")}});
                if wrapped.to_string().len()>route.to_string().len(){return Err(error(ErrorCode::InternalInvariantViolation,"archived route wrapper exceeds reserved row size"));}row["route_query"]=wrapped;}}}
            journal.validate_custody(c)?;
            let object=value.as_object_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"historical query result is not object"))?;
            if let Some(additions)=metadata.as_object(){object.extend(additions.iter().map(|(k,v)|(k.clone(),v.clone())));}
            semantic_query::publish_with_active_work(c,value,|v|{let raw=finish(&projection,v)?;let mut packet:Value=serde_json::from_str(&raw)
                .map_err(|_|error(ErrorCode::InternalInvariantViolation,"historical packet invalid"))?;packet["agent_turn"]["continuity"]["status"]=json!("partial");
                packet["agent_turn"]["continuity"]["gap"]=json!({"reason":"historical_snapshot_not_current_state"});let out=packet.to_string();
                if out.len()>projection.maximum_bytes{return Err(error(ErrorCode::BudgetExceeded,"complete historical response exceeds budget"));}Ok(out)})
        }
    }
}

pub(super) fn schema()->Result<Value>{
    let mut schema=semantic_query::extend_watch_count_schema(production::extend_schema(spatial_queries::schema()?)?)?;
    let mut extra:Value=serde_json::from_str(include_str!("../../../schemas/mcp_spatial_history_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"embedded spatial history schema invalid"))?;
    // The shared source file also serves spatial/1.6. Specialize only this
    // profile's cursor and inner-query allowlist, from the same runtime list.
    extra["oneOf"][0]["properties"]["continuation"]["oneOf"][1]["pattern"]=json!("^sch1:[1-9][0-9]*:[0-9a-f]{64}$");
    extra["oneOf"][1]["properties"]["query"]["allOf"][1]["properties"]["kind"]["enum"]=json!(STATELESS);
    let additions=extra["oneOf"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"history schema has no variants"))?;
    let variants=schema["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial query schema has no variants"))?;
    variants.extend(additions.iter().cloned());variants.push(changes::schema()?);
    variants.push(changes::series::schema()?);
    variants.push(changes::monitor::schema()?);Ok(schema)
}
