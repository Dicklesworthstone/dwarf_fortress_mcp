//! Stateless queries against exact archived spatial/1.8 records. No monitoring
//! registration/evaluation, baseline mutation, observation or game effect is routed.
//! Historical monitor replay is request-owned analysis, not a live watch sample.
use super::*;

const READ_KINDS: [&str;16] = ["entities","inspect","traverse","dependencies","aggregate","search",
    "map_route","spatial_inventory_plan","workforce_candidates","workforce_plan",
    "production_diagnosis","inventory_plan","item_quantity","production_portfolio","condition_evaluation","map_connectivity"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope { schema:String, expected_anchor:Option<Value>, query:Value }
#[derive(Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
enum ArchiveRequest {
    History { limit:Option<u32>, continuation:Option<String> },
    HistoricalQuery { record:u64, record_digest:String, query:Value },
}
fn invalid(text:&str)->DfmcpError { error(ErrorCode::InvalidRequest,text) }
fn bounded(text:&str)->DfmcpError { error(ErrorCode::BudgetExceeded,text) }

fn validate_shape(input:&Value)->Result<()> {
    let mut pending=vec![(input,0usize)];let mut nodes=0usize;let mut bytes=0usize;
    while let Some((value,depth))=pending.pop() {
        nodes+=1;bytes=bytes.saturating_add(32);
        if nodes>4096 || depth>32 {return Err(bounded("archive query exceeds its node/depth bound"));}
        match value {
            Value::String(text)=>bytes=bytes.saturating_add(text.len()),
            Value::Array(values)=>{
                if nodes.saturating_add(pending.len()).saturating_add(values.len())>4096 {return Err(bounded("archive query array too wide"));}
                pending.extend(values.iter().map(|v|(v,depth+1)));
            }
            Value::Object(values)=>{
                if nodes.saturating_add(pending.len()).saturating_add(values.len())>4096 {return Err(bounded("archive query object too wide"));}
                for(key,value)in values {bytes=bytes.saturating_add(key.len());pending.push((value,depth+1));}
            }
            _=>{}
        }
        if bytes>128*1024 {return Err(bounded("archive query exceeds aggregate input bound"));}
    }
    Ok(())
}
fn kind(query:&Value)->&str {query.get("kind").and_then(Value::as_str).unwrap_or("")}

fn cursor(session:&Session,context:&OperationContext,offset:usize)->Result<String> {
    let journal=session.journal.as_ref().ok_or_else(||invalid("archive absent"))?;
    let identity=json!({"domain":"dfmcp-spatial-archive-page/1","session":context.session_id.to_string(),
        "journal":journal.id().to_string(),"head":journal.head().to_string(),"anchor":anchor_json(context.anchor),"offset":offset});
    Ok(format!("ar1:{offset}:{}",Digest32::of_bytes(identity.to_string().as_bytes())))
}
fn history_page(session:&Session,context:&OperationContext,limit:Option<u32>,continuation:Option<String>)->Result<String> {
    let journal=session.journal.as_ref().ok_or_else(||invalid("archive absent"))?;
    let entries=journal.entries();let limit=limit.unwrap_or(8);
    if !(1..=64).contains(&limit) {return Err(bounded("archive history limit must be 1..64"));}
    let start=match continuation {
        None=>0,
        Some(token)=>{
            if token.len()>128 {return Err(bounded("archive continuation exceeds byte bound"));}
            let mut parts=token.split(':');
            let(Some("ar1"),Some(raw),Some(_),None)=(parts.next(),parts.next(),parts.next(),parts.next())else{return Err(invalid("invalid archive continuation"));};
            if raw.is_empty()||raw.starts_with('0')||!raw.bytes().all(|b|b.is_ascii_digit()) {return Err(invalid("noncanonical archive offset"));}
            let offset=raw.parse::<usize>().map_err(|_|invalid("archive offset overflow"))?;
            if token!=cursor(session,context,offset)? {return Err(error(ErrorCode::StaleAnchor,"archive continuation names another session or journal head"));}
            if offset>=entries.len() {return Err(error(ErrorCode::CursorGap,"archive continuation is past retained history"));}
            offset
        }
    };
    let latest=entries.last().ok_or_else(||error(ErrorCode::CursorGap,"empty archive"))?;
    let budget=result_context(session,context,latest)?.budget.max_bytes;
    let mut out=json!({"schema":"dfmcp.query.result/1","kind":"history","matched":entries.len(),
        "rows":[],"returned":0,"truncated":false,"continuation":null});
    let mut end=start;
    for entry in entries.iter().skip(start).take(limit.min(context.budget.max_entities) as usize) {
        let mut next=out.clone();next["rows"].as_array_mut().ok_or_else(||invalid("history rows absent"))?.push(entry_json(entry));
        next["returned"]=json!(end+1-start);next["truncated"]=json!(end+1<entries.len());
        next["continuation"]=if end+1<entries.len(){json!(cursor(session,context,end+1)?)}else{Value::Null};
        if next.to_string().len() as u64>budget {break;}
        out=next;end+=1;
    }
    if start<entries.len()&&end==start {return Err(bounded("one complete archived-history row does not fit"));}
    packet(session,context,"fortress.query",None,out)
}

/// Replace the source's anchor-bound route drill-down with an exact-record
/// historical query. Its wrapper is smaller than the original full anchor;
/// enforce that property rather than overshooting the already reserved row size.
fn pin_routes(out:&mut Value,entry:&JournalEntry)->Result<()> {
    if let Some(rows)=out.get_mut("rows").and_then(Value::as_array_mut) {
        for row in rows {
            if let Some(route)=row.get("route_query") {
                let query=route.get("query").filter(|q|kind(q)=="map_route")
                    .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"route drill-down lacks a fixed map query"))?;
                let wrapper=json!({"schema":"dfmcp.query/1","query":{"kind":"historical_query",
                    "record":entry.number,"record_digest":entry.record_digest.to_string(),"query":query}});
                if wrapper.to_string().len()>route.to_string().len() {
                    return Err(error(ErrorCode::InternalInvariantViolation,"archived route wrapper exceeds source row reservation"));
                }
                row["route_query"]=wrapper;
            }
        }
    }
    Ok(())
}
fn execute_on(session:&Session,context:&OperationContext,state:&LiveSpatialCitizenState,
    entry:&JournalEntry,query:Value)->Result<String> {
    if !READ_KINDS.contains(&kind(&query)) {return Err(error(ErrorCode::CapabilityDenied,"archive-only queries permit stateless analysis, not watch/baseline changes or live acquisition"));}
    let snapshot=state.snapshot().ok_or_else(||error(ErrorCode::CorruptLedger,"archived snapshot absent"))?;
    if snapshot.anchor()!=entry.anchor {return Err(error(ErrorCode::CorruptLedger,"archived state does not reproduce selected record"));}
    validate_limits(state.observation_full().ok_or_else(||error(ErrorCode::CorruptLedger,"archived source absent"))?,session.limits)?;
    let narrowed=result_context(session,context,entry)?;
    let input=json!({"schema":"dfmcp.query/1","expected_anchor":anchor_json(entry.anchor),"query":query});
    let mut out=if production::handles(&input) {production::execute(state,&narrowed,&input)?}
        else if workforce_queries::handles(&input) {workforce_queries::execute(state,&narrowed,&input)?}
        else if spatial_queries::handles(&input) {spatial_queries::execute(state,&narrowed,&input)?}
        else {semantic_query::execute(snapshot,&narrowed,&input)?};
    production::pin_historical(&mut out,entry.number,entry.record_digest)?;
    pin_routes(&mut out,entry)?;
    packet(session,context,"fortress.query",Some(entry),out)
}

fn schema()->Result<Value> {
    let mut schema=workforce_queries::extend_schema(history::schema()?)?;
    let variants=schema["$defs"]["query"]["oneOf"].as_array().ok_or_else(||invalid("query schema variants absent"))?;
    let mut stateless=Vec::new();
    for variant in variants {
        let resolved=match variant.get("$ref").and_then(Value::as_str) {
            Some(reference)=>schema.pointer(reference.strip_prefix('#').ok_or_else(||invalid("nonlocal query schema reference"))?)
                .ok_or_else(||invalid("missing query schema reference"))?,
            None=>variant,
        };
        if resolved["properties"]["kind"]["const"].as_str().is_some_and(|k|READ_KINDS.contains(&k)) {stateless.push(variant.clone());}
    }
    if stateless.len()!=READ_KINDS.len() {return Err(error(ErrorCode::InternalInvariantViolation,"archive schema could not resolve every registered stateless query"));}
    schema["$defs"]["archive_stateless"]=json!({"oneOf":stateless});
    let mut all=stateless;
    all.push(json!({"type":"object","additionalProperties":false,"required":["kind"],"properties":{
        "kind":{"const":"history"},"limit":{"type":["integer","null"],"minimum":1,"maximum":64},
        "continuation":{"type":["string","null"],"maxLength":128,"pattern":"^ar1:[1-9][0-9]*:[0-9a-f]{64}$"}}}));
    all.push(json!({"type":"object","additionalProperties":false,"required":["kind","record","record_digest","query"],"properties":{
        "kind":{"const":"historical_query"},"record":{"type":"integer","minimum":1,"maximum":4096},
        "record_digest":{"type":"string","pattern":"^[0-9a-f]{64}$"},"query":{"$ref":"#/$defs/archive_stateless"}}}));
    all.push(history::changes::schema()?);
    all.push(history::changes::series::schema()?);
    all.push(history::changes::monitor::schema()?);
    schema["$defs"]["query"]["oneOf"]=json!(all);
    Ok(schema)
}

pub(in super::super) fn query(session:&mut Session,context:&OperationContext,input:&Value,schema_mode:bool)->Result<String> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    validate(session,context)?;
    if schema_mode {return packet(session,context,"fortress.query",None,
        json!({"mode":"schema","query_schema":schema()?,"profile":"spatial/1.8-archive","truncated":false,"continuation":null}));}
    if history::changes::handles(input) { return history::changes::execute(session,context,input); }
    validate_shape(input)?;
    let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|invalid("invalid archive query envelope"))?;
    if envelope.schema!="dfmcp.query/1" {return Err(invalid("archive query requires dfmcp.query/1"));}
    if envelope.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(context.anchor)) {
        return Err(error(ErrorCode::StaleAnchor,"expected_anchor differs from this archive session's latest retained observation"));
    }
    match kind(&envelope.query) {
        "history"|"historical_query"=>{
            let request:ArchiveRequest=serde_json::from_value(envelope.query).map_err(|_|invalid("invalid archive record selection"))?;
            match request {
                ArchiveRequest::History{limit,continuation}=>history_page(session,context,limit,continuation),
                ArchiveRequest::HistoricalQuery{record,record_digest,query}=>{
                    if !READ_KINDS.contains(&kind(&query)) {return Err(error(ErrorCode::CapabilityDenied,"historical queries cannot evaluate watches, change baselines or recursively select history"));}
                    let digest=Digest32::from_hex(&record_digest).filter(|d|d.to_string()==record_digest)
                        .ok_or_else(||invalid("record_digest must be canonical lowercase SHA-256"))?;
                    let replay=history::replay_context(session,context);
                    let journal=session.journal.as_mut().ok_or_else(||invalid("archive absent"))?;
                    let index=record.checked_sub(1).and_then(|n|usize::try_from(n).ok())
                        .filter(|&i|i<journal.entries().len()).ok_or_else(||error(ErrorCode::CursorGap,"selected record is not retained"))?;
                    let entry=journal.entries()[index].clone();
                    let state=journal.state_at(record,digest,&replay)?;
                    execute_on(session,context,&state,&entry,query)
                }
            }
        }
        _=>{
            let entry=session.journal.as_ref().and_then(|j|j.entries().last()).cloned()
                .ok_or_else(||error(ErrorCode::CursorGap,"archive has no retained observations"))?;
            execute_on(session,context,&session.state,&entry,envelope.query)
        }
    }
}

#[cfg(all(test,unix))]
#[path="spatial_archive_runtime_tests.rs"]
mod tests;