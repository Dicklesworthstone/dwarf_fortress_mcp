//! Citizen-specific analysis over one coherent spatial/1.8 snapshot. Assignment
//! is an observed Job.getWorker relation, not labor eligibility or readiness.
use dfmcp_adapter::live_spatial::{SpatialStateView,citizens::LiveSpatialCitizenState};
use dfmcp_core::{Capability,DfmcpError,Digest32,EntityId,ErrorCode,OperationContext,Result,RiskTier};
use dfmcp_world::{EdgeKind,EntityKind,FactPresence,Value};
use serde::Deserialize;
use serde_json::{Value as Json,json};
use super::anchor_json;

fn invalid(s:&str)->DfmcpError{DfmcpError::new(ErrorCode::InvalidRequest,s)}
fn budget(s:&str)->DfmcpError{DfmcpError::new(ErrorCode::BudgetExceeded,s)}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Envelope{schema:String,expected_anchor:Option<Json>,query:Query}
#[derive(Deserialize)]#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
enum Query{WorkforceSummary{include_idle:Option<bool>,limit:Option<u32>,continuation:Option<String>}}

pub(super) fn handles(input:&Json)->bool{input.get("query").and_then(|q|q.get("kind")).and_then(Json::as_str)==Some("workforce_summary")}
fn token(offset:usize,id:Digest32)->String{let mut bytes=b"dfmcp-workforce-page/1\0".to_vec();bytes.extend_from_slice(id.as_bytes());bytes.extend_from_slice(&(offset as u64).to_be_bytes());
    format!("wf1:{offset}:{}",Digest32::of_bytes(&bytes))}
fn offset(raw:Option<&str>,id:Digest32,count:usize)->Result<usize>{let Some(raw)=raw else{return Ok(0)};
    if raw.len()>128{return Err(budget("workforce continuation exceeds 128 bytes"));}let parts:Vec<_>=raw.split(':').collect();
    if parts.len()!=3||parts[0]!="wf1"||parts[1].is_empty()||parts[1].starts_with('0')||!parts[1].bytes().all(|b|b.is_ascii_digit()){
        return Err(invalid("invalid workforce continuation"));}let n=parts[1].parse::<usize>().map_err(|_|invalid("workforce offset overflow"))?;
    if raw!=token(n,id){return Err(DfmcpError::new(ErrorCode::StaleAnchor,"workforce continuation belongs to another session or observation"));}
    if n>=count{return Err(DfmcpError::new(ErrorCode::CursorGap,"workforce continuation is past the result"));}Ok(n)}
fn bool_fact(entity:&dfmcp_world::EntityRecord,name:&str)->Option<bool>{entity.fields.get(name).and_then(|f|if f.presence.is_none(){match f.value{Value::Bool(v)=>Some(v),_=>None}}else{None})}
fn text_fact(entity:&dfmcp_world::EntityRecord,name:&str)->Option<String>{entity.fields.get(name).and_then(|f|if f.presence.is_none(){match &f.value{Value::Text(v)=>Some(v.clone()),_=>None}}else{None})}
fn i64_fact(entity:&dfmcp_world::EntityRecord,name:&str)->Option<i64>{entity.fields.get(name).and_then(|f|if f.presence.is_none(){match f.value{Value::I64(v)=>Some(v),_=>None}}else{None})}
fn coord_fact(entity:&dfmcp_world::EntityRecord,name:&str)->Option<[i32;3]>{entity.fields.get(name).and_then(|f|if f.presence.is_none(){match f.value{Value::Coord(v)=>Some([v.x,v.y,v.z]),_=>None}}else{None})}

pub(super) fn execute(state:&LiveSpatialCitizenState,c:&OperationContext,input:&Json)->Result<Json>{
    c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|invalid("invalid workforce query"))?;
    if envelope.schema!="dfmcp.query/1"{return Err(invalid("workforce query requires dfmcp.query/1"));}
    let snapshot=state.snapshot().ok_or_else(||invalid("spatial/1.8 snapshot absent"))?;
    if snapshot.anchor()!=c.anchor||envelope.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(c.anchor)){return Err(DfmcpError::new(ErrorCode::StaleAnchor,"workforce query anchor differs"));}
    if snapshot.graph.entities.len()>c.budget.max_entities as usize{return Err(budget("workforce scan exceeds session entity budget"));}
    let Query::WorkforceSummary{include_idle,limit,continuation}=envelope.query;let include_idle=include_idle.unwrap_or(true);let limit=limit.unwrap_or(16);
    if !(1..=128).contains(&limit){return Err(budget("workforce page limit must be 1..128"));}
    let mut assignments=std::collections::BTreeMap::<EntityId,Vec<EntityId>>::new();
    for edge in snapshot.graph.edges.values(){if edge.kind==EdgeKind::Performs{assignments.entry(edge.from).or_default().push(edge.to);}}
    for jobs in assignments.values_mut(){jobs.sort();jobs.dedup();}
    let mut citizens:Vec<_>=snapshot.graph.entities.values().filter(|e|e.kind==EntityKind::Unit).collect();citizens.sort_by_key(|e|e.id);
    let rows:Vec<_>=citizens.into_iter().filter_map(|entity|{
        let jobs=assignments.get(&entity.id).cloned().unwrap_or_default();if !include_idle&&jobs.is_empty(){return None;}
        Some(json!({"citizen":{"entity_id":entity.id.to_string(),"generation":entity.generation,"revision":entity.revision},
            "name":text_fact(entity,"name"),"profession":i64_fact(entity,"profession"),"position":coord_fact(entity,"position"),
            "alive":bool_fact(entity,"alive"),"sane":bool_fact(entity,"sane"),"active":bool_fact(entity,"active"),"visible":bool_fact(entity,"visible"),
            "assigned_job_count":jobs.len(),"assigned_jobs":jobs.iter().take(8).filter_map(|id|snapshot.graph.entities.get(id)
                .map(|job|json!({"entity_id":job.id.to_string(),"generation":job.generation,"revision":job.revision,"label":job.label}))).collect::<Vec<_>>(),
            "assigned_jobs_truncated":jobs.len()>8,"labor_eligibility_proven":false,"job_readiness_proven":false}))
    }).collect();
    let mut jobs_total=0u64;let mut jobs_unassigned=0u64;let mut jobs_strict=0u64;let mut jobs_other=0u64;
    for job in snapshot.graph.entities.values().filter(|e|e.kind==EntityKind::Job){jobs_total+=1;
        match job.fields.get("worker_entity"){Some(f)if matches!(f.value,Value::Entity(_))&&f.presence.is_none()=>jobs_strict+=1,
            Some(f)if matches!(f.presence,Some(FactPresence::Absent))=>jobs_unassigned+=1,_=>jobs_other+=1}}
    let strict_with_jobs=assignments.keys().count() as u64;let idle=(snapshot.graph.entities.values().filter(|e|e.kind==EntityKind::Unit).count() as u64).saturating_sub(strict_with_jobs);
    let mut identity=b"dfmcp-workforce-query/1\0".to_vec();identity.extend_from_slice(&c.session_id.get().to_be_bytes());identity.extend_from_slice(c.anchor.state_hash.as_bytes());
    identity.extend_from_slice(state.source_digest()?.as_bytes());identity.push(u8::from(include_idle));let id=Digest32::of_bytes(&identity);let start=offset(continuation.as_deref(),id,rows.len())?;
    let maximum=usize::try_from(c.budget.max_bytes.min(u64::from(c.budget.max_output_tokens)*4)).map_err(|_|budget("workforce output budget overflow"))?;
    let mut out=json!({"schema":"dfmcp.query.result/1","kind":"workforce_summary","anchor":anchor_json(c.anchor),"source_digest":state.source_digest()?.to_string(),
        "summary":{"strict_citizens":idle+strict_with_jobs,"strict_citizens_with_observed_jobs":strict_with_jobs,"strict_citizens_without_observed_jobs":idle,
            "jobs":jobs_total,"jobs_without_worker":jobs_unassigned,"jobs_with_strict_citizen_worker":jobs_strict,"jobs_with_worker_outside_strict_roster":jobs_other},
        "interpretation":"Assignment is observed from Job.getWorker in the same capture; no labor eligibility, skill suitability, job readiness, or causality is inferred.",
        "rows":[],"returned":0,"total_rows":rows.len(),"truncated":false,"continuation":null});
    let mut end=start;while end<rows.len()&&end-start<limit as usize{let mut next=out.clone();next["rows"].as_array_mut().ok_or_else(||invalid("workforce rows absent"))?.push(rows[end].clone());
        next["returned"]=json!(end+1-start);next["truncated"]=json!(end+1<rows.len());next["continuation"]=if end+1<rows.len(){json!(token(end+1,id))}else{Json::Null};
        if next.to_string().len()>maximum{break;}out=next;end+=1;}
    if start<rows.len()&&end==start{return Err(budget("one complete workforce row cannot fit response budget"));}Ok(out)
}

pub(super) fn extend_schema(mut base:Json)->Result<Json>{
    let variant=json!({"type":"object","additionalProperties":false,"required":["kind"],"properties":{
        "kind":{"const":"workforce_summary"},"include_idle":{"type":["boolean","null"]},"limit":{"type":["integer","null"],"minimum":1,"maximum":128},
        "continuation":{"type":["string","null"],"maxLength":128}}});
    base["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(||invalid("base query variants absent"))?.push(variant);Ok(base)
}
