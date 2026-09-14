//! Durable observation history, never archived session authority or live effects.
use super::*;
use dfmcp_adapter::operations_journal::{JournalEntry,JournalLimits,OperationsJournal,PrivateJournalFile,TailRecovery,open_private_journal};
use dfmcp_core::Digest32;
use serde::Deserialize;
use std::path::PathBuf;

type Journal = OperationsJournal<PrivateJournalFile>;

pub(super) fn configuration()->Result<Option<(PathBuf,TailRecovery)>> {
    let path=match std::env::var("DFMCP_OPERATIONS_JOURNAL") {
        Ok(path) if !path.is_empty()=>Some(PathBuf::from(path)),
        Err(std::env::VarError::NotPresent)=>None,
        _=>return Err(error(ErrorCode::InvalidRequest,"DFMCP_OPERATIONS_JOURNAL must be a nonempty UTF-8 operator path")),
    };
    let repair=match std::env::var("DFMCP_OPERATIONS_JOURNAL_REPAIR") {
        Err(std::env::VarError::NotPresent)=>TailRecovery::Refuse,
        Ok(value) if value=="1" && path.is_some()=>TailRecovery::TruncateIncomplete,
        _=>return Err(error(ErrorCode::InvalidRequest,"journal repair requires a configured journal and the exact operator opt-in value 1")),
    };
    Ok(path.map(|path|(path,repair)))
}

pub(super) fn attach(session:&mut OperationsSession,path:&std::path::Path,repair:TailRecovery,context:&OperationContext)->Result<()> {
    let current=session.state.observation().cloned().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"bootstrap observation missing"))?;
    let mut journal=open_private_journal(path,context,JournalLimits::default(),repair)?;
    journal.append(current,context)?;
    session.state=journal.state().clone();session.journal=Some(journal);Ok(())
}

pub(super) fn summary(journal:Option<&Journal>)->Value {
    match journal {
        None=>json!({"storage":"process_local","durable_observation_history":false}),
        Some(journal)=>json!({"storage":"synced_operations_journal","durable_observation_history":true,
            "journal_id":journal.id().to_string(),"head":journal.head().to_string(),"records":journal.entries().len(),
            "retained_bytes":journal.retained_bytes(),"repaired_tail_bytes":journal.repaired_tail_bytes(),"fenced":journal.fenced(),
            "continuous_game_history":false,"watches_and_baselines_durable":false}),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {schema:String,expected_anchor:Option<Value>,query:Request}
#[derive(Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
enum Request {
    History {limit:Option<u32>,continuation:Option<String>},
    HistoricalQuery {record:u64,record_digest:String,query:Value},
}
fn parse(input:&Value,context:&OperationContext)->Result<Request> {
    let mut stack=vec![(input,0usize)];let mut nodes=0usize;let mut bytes=0usize;
    while let Some((value,depth))=stack.pop() {
        nodes+=1;
        if nodes>4096 || depth>32{return Err(error(ErrorCode::BudgetExceeded,"historical query shape exceeds its bound"));}
        match value {
            Value::Object(map)=>{
                if stack.len().saturating_add(map.len())>4096{return Err(error(ErrorCode::BudgetExceeded,"history object is too wide"));}
                for (key,value) in map {bytes=bytes.saturating_add(key.len());stack.push((value,depth+1));}
            }
            Value::Array(values)=>{
                if stack.len().saturating_add(values.len())>4096{return Err(error(ErrorCode::BudgetExceeded,"history array is too wide"));}
                stack.extend(values.iter().map(|value|(value,depth+1)));
            }
            Value::String(text)=>bytes=bytes.saturating_add(text.len()),_=>{},
        }
        if bytes>128*1024{return Err(error(ErrorCode::BudgetExceeded,"historical query text exceeds its bound"));}
    }
    let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|error(ErrorCode::InvalidRequest,"invalid operations history envelope"))?;
    if envelope.schema!="dfmcp.query/1" {return Err(error(ErrorCode::InvalidRequest,"history requires schema dfmcp.query/1"));}
    if envelope.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(context.anchor)) {
        return Err(error(ErrorCode::StaleAnchor,"history expected_anchor differs from the current session anchor"));
    }
    Ok(envelope.query)
}
fn entry_json(entry:&JournalEntry)->Value {
    json!({"record":entry.number,"anchor":anchor_json(entry.anchor),"source_digest":entry.source_digest.to_string(),
        "record_digest":entry.record_digest.to_string(),"previous_digest":entry.previous_digest.to_string(),"encoded_bytes":entry.encoded_bytes})
}
fn cursor(journal:&Journal,context:&OperationContext,offset:usize)->String {
    let identity=json!({"domain":"dfmcp-journal-list/1","journal":journal.id().to_string(),
        "head":journal.head().to_string(),"session":context.session_id.to_string(),"anchor":anchor_json(context.anchor),"offset":offset});
    format!("jh1:{offset}:{}",Digest32::of_bytes(identity.to_string().as_bytes()))
}
fn history_page(journal:&Journal,context:&OperationContext,maximum:usize,limit:Option<u32>,continuation:Option<String>)->Result<Value> {
    let limit=limit.unwrap_or(8);
    if !(1..=64).contains(&limit){return Err(error(ErrorCode::BudgetExceeded,"history limit must be 1..64"));}
    let start=match continuation {
        None=>0,
        Some(token)=>{
            if token.len()>128{return Err(error(ErrorCode::BudgetExceeded,"history cursor exceeds its bound"));}
            let mut parts=token.split(':');
            let (Some("jh1"),Some(offset),Some(_),None)=(parts.next(),parts.next(),parts.next(),parts.next())
                else {return Err(error(ErrorCode::InvalidRequest,"invalid history continuation"));};
            let value=offset.parse::<usize>().map_err(|_|error(ErrorCode::InvalidRequest,"invalid history offset"))?;
            if value==0 || value>=journal.entries().len() || cursor(journal,context,value)!=token {
                return Err(error(ErrorCode::StaleAnchor,"history cursor belongs to another session, journal head or snapshot"));
            }
            value
        }
    };
    let mut result=json!({"schema":"dfmcp.query.result/1","kind":"history","anchor":anchor_json(context.anchor),
        "history":summary(Some(journal)),"matched":journal.entries().len(),"returned":0,"rows":[],
        "truncated":false,"continuation":null,"native_observations":0,"temporal_coverage":"retained_observation_endpoints_only"});
    let mut end=start;
    for entry in journal.entries().iter().skip(start).take(limit as usize) {
        let mut next=result.clone();
        next["rows"].as_array_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"history rows missing"))?.push(entry_json(entry));
        next["returned"]=json!(end+1-start);next["truncated"]=json!(end+1<journal.entries().len());
        next["continuation"]=if end+1<journal.entries().len(){json!(cursor(journal,context,end+1))}else{Value::Null};
        if next.to_string().len()>maximum{break;}result=next;end+=1;
    }
    if (end==start && start<journal.entries().len()) || result.to_string().len()>maximum {
        return Err(error(ErrorCode::BudgetExceeded,"one journal metadata row and required context cannot fit"));
    }
    Ok(result)
}

pub(super) fn handles(input:&Value)->bool {
    matches!(input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str),Some("history"|"historical_query"))
}
pub(super) fn execute(session:&mut OperationsSession,context:&OperationContext,input:&Value)->Result<String> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    let request=parse(input,context)?;
    let mut view=query_view(session,context)?;
    let mut result_context=context.clone();result_context.budget.max_bytes=view.result_byte_budget()? as u64;
    let narrowed=semantic_query::result_context(&result_context)?;
    let live_source_fenced=session.source.poisoned();
    let journal=session.journal.as_mut().ok_or_else(||error(ErrorCode::InvalidRequest,
        "durable observation history is not configured; an operator must select DFMCP_OPERATIONS_JOURNAL before opening the session"))?;
    match request {
        Request::History{limit,continuation}=>{
            let mut value=history_page(journal,context,narrowed.budget.max_bytes as usize,limit,continuation)?;
            value["source_stale"]=json!(live_source_fenced);
            semantic_query::publish_with_active_work(context,value,|value|finish_query(&view,value))
        }
        Request::HistoricalQuery{record,record_digest,query}=>{
            let kind=query.get("kind").and_then(Value::as_str).unwrap_or("");
            if !matches!(kind,"entities"|"inspect"|"traverse"|"dependencies"|"aggregate"|"search") {
                return Err(error(ErrorCode::InvalidRequest,"historical_query permits only stateless observation queries; no watches, baselines, effects or nested history"));
            }
            let digest=Digest32::from_hex(&record_digest).filter(|digest|digest.to_string()==record_digest)
                .ok_or_else(||error(ErrorCode::InvalidRequest,"record_digest must be canonical lowercase SHA-256"))?;
            // Full acquisition/replay budget, not the narrowed output-page budget.
            let snapshot=journal.snapshot_at(record,digest,context)?;
            let entry=journal.entries().get((record-1) as usize).cloned()
                .ok_or_else(||error(ErrorCode::CursorGap,"journal record is not retained"))?;
            view.anchor=anchor_json(snapshot.anchor());
            view.briefing=json!({"runtime":"unadmitted_development","bridge_protocol":"1.3","read_only":true,
                "runtime_admitted":false,"mutation_admissible":false,"live":false,"historical":true,
                "paused":snapshot.paused,"current_live_anchor":anchor_json(context.anchor),"live_source_fenced":live_source_fenced,
                "active_work_basis":"current_session_not_archived","query_source":"verified_operations_replay"});
            view.references=vec![json!({"kind":"archived_operations_observation","digest":entry.source_digest.to_string(),
                "record_digest":entry.record_digest.to_string(),"journal_id":journal.id().to_string()})];
            view.coverage["temporal_coverage"]=json!("historical_observation_only");
            view.coverage["current_freshness_proven"]=json!(false);
            let metadata=json!({"historical":true,"journal_record":entry_json(&entry),
                "current_live_anchor":anchor_json(context.anchor),"native_observations":0});
            let mut historical_context=context.clone();
            historical_context.budget.max_bytes=view.result_byte_budget()?.checked_sub(metadata.to_string().len()+128)
                .ok_or_else(||error(ErrorCode::BudgetExceeded,"historical metadata leaves no query budget"))? as u64;
            historical_context=semantic_query::result_context(&historical_context)?;
            historical_context.anchor=snapshot.anchor();
            let mut value=semantic_query::execute(&snapshot,&historical_context,&json!({"schema":"dfmcp.query/1","query":query}))?;
            if let (Some(object),Some(additions))=(value.as_object_mut(),metadata.as_object()) {
                object.extend(additions.iter().map(|(key,value)|(key.clone(),value.clone())));
            } else {return Err(error(ErrorCode::InternalInvariantViolation,"historical query returned a non-object"));}
            semantic_query::publish_with_active_work(context,value,|value|{
                let raw=finish_query(&view,value)?;
                let mut packet:Value=serde_json::from_str(&raw).map_err(|_|error(ErrorCode::InternalInvariantViolation,"historical packet decode failed"))?;
                packet["agent_turn"]["continuity"]["status"]=json!("partial");
                packet["agent_turn"]["continuity"]["gap"]=json!({"reason":"historical_snapshot_not_current_state"});
                let encoded=packet.to_string();
                if encoded.len()>view.maximum_bytes{return Err(error(ErrorCode::BudgetExceeded,"historical packet exceeds response budget"));}Ok(encoded)
            })
        }
    }
}

pub(super) fn query_schema()->Result<Value> {
    let mut schema=production::query_schema()?;
    let extension:Value=serde_json::from_str(include_str!("../../../schemas/mcp_operations_history_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"invalid embedded history schema"))?;
    let variants=schema["$defs"]["query"]["oneOf"].as_array_mut()
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"query schema variants missing"))?;
    let additions=extension["oneOf"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"history variants missing"))?;
    variants.extend(additions.iter().cloned());Ok(schema)
}
