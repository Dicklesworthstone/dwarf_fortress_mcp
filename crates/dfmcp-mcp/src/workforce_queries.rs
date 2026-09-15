//! Workforce candidate analysis over one coherent spatial/1.8 capture. This is
//! observed skill/readiness + conservative terrain approach, never labor authority.
use dfmcp_adapter::live_spatial::{SpatialStateView,citizens::{LiveCitizen,LiveSpatialCitizenState,citizen_entity_id}};
use dfmcp_core::{Capability,DfmcpError,Digest32,ErrorCode,OperationContext,Result,RiskTier};
use dfmcp_world::map_reachability::Reachability;
use dfmcp_world::map_region::{MAX_ROUTE_WORK,MapRegion};
use serde::Deserialize;
use serde_json::{Value,json};
use super::anchor_json;

const MAX_INPUT_BYTES:usize=65_536;
fn invalid(text:&str)->DfmcpError{DfmcpError::new(ErrorCode::InvalidRequest,text)}
fn budget(text:&str)->DfmcpError{DfmcpError::new(ErrorCode::BudgetExceeded,text)}
#[derive(Deserialize)]#[serde(deny_unknown_fields)]struct Envelope{schema:String,expected_anchor:Option<Value>,query:Query}
#[derive(Deserialize)]#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]enum Query{
    WorkforceCandidates{target:[u32;3],skill_key:String,min_effective_skill:Option<i32>,preserve_social:Option<bool>,
        adults_only:Option<bool>,limit:Option<u32>,continuation:Option<String>,max_work:Option<u64>}}
#[derive(Clone)]struct Candidate{citizen_index:usize,steps:u32,approach:[u32;3],nominal:i32,effective:i32,experience:i32}

pub(super) fn handles(input:&Value)->bool{input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str)==Some("workforce_candidates")}
fn validate_shape(input:&Value)->Result<()>{
    let mut stack=vec![(input,0usize)];let mut nodes=0usize;let mut bytes=0usize;
    while let Some((value,depth))=stack.pop(){nodes+=1;bytes=bytes.saturating_add(16);if nodes>4096||depth>16{return Err(budget("workforce query exceeds shape bound"));}
        match value{Value::String(text)=>bytes=bytes.saturating_add(text.len()),Value::Array(values)=>stack.extend(values.iter().map(|v|(v,depth+1))),
            Value::Object(map)=>for(key,value)in map{bytes=bytes.saturating_add(key.len());stack.push((value,depth+1));},_=>{}}
        if bytes>MAX_INPUT_BYTES{return Err(budget("workforce query exceeds input byte bound"));}}
    Ok(())
}
fn token(offset:usize,identity:Digest32)->String{let mut bytes=b"dfmcp-workforce-candidates-page/1\0".to_vec();bytes.extend_from_slice(identity.as_bytes());bytes.extend_from_slice(&(offset as u64).to_be_bytes());
    format!("wc1:{offset}:{}",Digest32::of_bytes(&bytes))}
fn page_offset(raw:Option<&str>,identity:Digest32,count:usize)->Result<usize>{let Some(raw)=raw else{return Ok(0)};
    if raw.len()>128{return Err(budget("workforce continuation exceeds 128 bytes"));}let parts:Vec<_>=raw.split(':').collect();
    if parts.len()!=3||parts[0]!="wc1"||parts[1].is_empty()||parts[1].starts_with('0')||!parts[1].bytes().all(|b|b.is_ascii_digit()){
        return Err(invalid("invalid workforce continuation"));}let n=parts[1].parse::<usize>().map_err(|_|invalid("workforce offset overflow"))?;
    if raw!=token(n,identity){return Err(DfmcpError::new(ErrorCode::StaleAnchor,"workforce continuation belongs to another session/query/capture"));}
    if n>=count{return Err(DfmcpError::new(ErrorCode::CursorGap,"workforce continuation is past result set"));}Ok(n)}
fn approach(map:&MapRegion,field:&Reachability,position:[i32;3])->Option<([u32;3],u32)>{
    if position.iter().any(|v|*v<0){return None;}let p=[position[0] as u32,position[1] as u32,position[2] as u32];
    if map.region.index(p).is_none(){return None;}if let Some(distance)=field.distance_to(p){return Some((p,distance));}
    // Unit occupancy normally makes the citizen's exact tile non-candidate. A
    // horizontal adjacent candidate represents a conservative one-step exit;
    // vertical occupied-endpoint stair semantics are deliberately not inferred.
    let mut best=None;
    for axis in 0..2{for up in [false,true]{let mut next=p;let value=if up{next[axis].checked_add(1)}else{next[axis].checked_sub(1)};
        let Some(value)=value else{continue};next[axis]=value;let Some(index)=map.region.index(next)else{continue};if !map.candidate(index){continue;}
        let Some(distance)=field.distance_to(next)else{continue};let total=distance.saturating_add(1);
        if best.is_none_or(|(_,prior)|total<prior||(total==prior&&next<best.unwrap().0)){best=Some((next,total));}}}
    best
}
fn skill(citizen:&LiveCitizen,key:&str)->(i32,i32,i32){citizen.skills.iter().find(|s|s.key==key).map_or((0,0,0),|s|(s.nominal,s.effective,s.experience))}

pub(super) fn execute(state:&LiveSpatialCitizenState,c:&OperationContext,input:&Value)->Result<Value>{
    c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;validate_shape(input)?;
    let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|invalid("invalid workforce candidate fields"))?;
    if envelope.schema!="dfmcp.query/1"{return Err(invalid("workforce query requires dfmcp.query/1"));}
    let snapshot=state.snapshot().ok_or_else(||invalid("spatial/1.8 snapshot absent"))?;
    if snapshot.anchor()!=c.anchor||envelope.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(c.anchor)){
        return Err(DfmcpError::new(ErrorCode::StaleAnchor,"workforce query anchor differs"));}
    let Query::WorkforceCandidates{target,skill_key,min_effective_skill,preserve_social,adults_only,limit,continuation,max_work}=envelope.query;
    if skill_key.is_empty()||skill_key.len()>96||skill_key.contains('\0'){return Err(invalid("skill_key must be a bounded native DF skill key"));}
    let minimum=min_effective_skill.unwrap_or(0);if minimum<0{return Err(invalid("min_effective_skill must be nonnegative"));}
    let preserve_social=preserve_social.unwrap_or(true);let adults_only=adults_only.unwrap_or(true);let limit=limit.unwrap_or(16);
    if !(1..=128).contains(&limit){return Err(budget("workforce page limit must be 1..128"));}
    let maximum=max_work.unwrap_or(MAX_ROUTE_WORK);if maximum==0||maximum>MAX_ROUTE_WORK{return Err(budget("workforce max_work outside route bound"));}
    let observation=state.observation_full().ok_or_else(||invalid("spatial/1.8 source absent"))?;let map=&observation.spatial().terrain().map;
    if map.region.index(target).is_none(){return Err(invalid("workforce target lies outside observed region"));}
    let reserve=observation.citizens().len() as u64+16;if maximum<=reserve{return Err(budget("workforce max_work leaves no route budget"));}
    let field=Reachability::compute(map,&[target],maximum-reserve).map_err(|_|budget("workforce terrain reachability exceeded work bound or target is not a candidate tile"))?;
    let mut counts=std::collections::BTreeMap::<&'static str,u64>::new();let mut rows=Vec::new();
    for(index,citizen)in observation.citizens().iter().enumerate(){
        let class=if !citizen.alive()||!citizen.sane()||!citizen.active(){Some("inactive_or_unavailable_state")}
            else if adults_only&&!citizen.adult(){Some("not_adult")}
            else if preserve_social&&!citizen.job_available_preserve_social{Some("not_job_available_preserve_social")}
            else if !preserve_social&&!citizen.job_available_interrupt_social{Some("not_job_available_interrupt_social")}
            else{None};
        if let Some(class)=class{*counts.entry(class).or_default()+=1;continue;}
        let(nominal,effective,experience)=skill(citizen,&skill_key);if effective<minimum{*counts.entry("below_minimum_effective_skill").or_default()+=1;continue;}
        let Some((approach,steps))=approach(map,&field,[citizen.position.x,citizen.position.y,citizen.position.z])else{*counts.entry("no_candidate_approach_in_observed_model").or_default()+=1;continue;};
        rows.push(Candidate{citizen_index:index,steps,approach,nominal,effective,experience});*counts.entry("candidate").or_default()+=1;
    }
    rows.sort_by(|a,b|b.effective.cmp(&a.effective).then_with(||b.nominal.cmp(&a.nominal)).then_with(||a.steps.cmp(&b.steps))
        .then_with(||observation.citizens()[b.citizen_index].stress_category.cmp(&observation.citizens()[a.citizen_index].stress_category))
        .then_with(||observation.citizens()[a.citizen_index].native_id.cmp(&observation.citizens()[b.citizen_index].native_id)));
    let source=state.source_digest()?;let mut identity=b"dfmcp-workforce-candidates-query/1\0".to_vec();identity.extend_from_slice(&c.session_id.get().to_be_bytes());
    identity.extend_from_slice(c.anchor.state_hash.as_bytes());identity.extend_from_slice(source.as_bytes());for n in target{identity.extend_from_slice(&n.to_be_bytes());}
    identity.extend_from_slice(skill_key.as_bytes());identity.extend_from_slice(&minimum.to_be_bytes());identity.push(u8::from(preserve_social));identity.push(u8::from(adults_only));identity.extend_from_slice(&maximum.to_be_bytes());
    let identity=Digest32::of_bytes(&identity);let start=page_offset(continuation.as_deref(),identity,rows.len())?;
    let maximum_bytes=usize::try_from(c.budget.max_bytes.min(u64::from(c.budget.max_output_tokens)*4)).map_err(|_|budget("workforce output budget overflow"))?;
    let mut out=json!({"schema":"dfmcp.query.result/1","kind":"workforce_candidates","anchor":anchor_json(c.anchor),"source_digest":source.to_string(),
        "target":target,"skill_key":skill_key,"minimum_effective_skill":minimum,"preserve_social":preserve_social,"adults_only":adults_only,
        "candidate_policy":"strict-citizen-alive-sane-active-job-available-observed-skill-horizontal-approach/1","classification":counts,
        "unit_path_proven":false,"labor_eligibility_proven":false,"job_readiness_proven":false,"safety_proven":false,
        "interpretation":"Effective skill and DFHack job availability are observed in the same capture. Terrain distance reaches a candidate tile or one horizontal step from an occupied citizen tile; it is not DF unit pathfinding or permission to assign labor.",
        "rows":[],"returned":0,"total_rows":rows.len(),"truncated":false,"continuation":null,"reachable_tiles":field.visited_tiles,"touched_region_boundary":field.touched_region_boundary});
    let mut end=start;while end<rows.len()&&end-start<limit as usize{let candidate=&rows[end];let citizen=&observation.citizens()[candidate.citizen_index];let id=citizen_entity_id(citizen.native_id);
        let entity=snapshot.graph.entities.get(&id).ok_or_else(||DfmcpError::new(ErrorCode::InternalInvariantViolation,"candidate citizen entity absent"))?;
        let mut next=out.clone();next["rows"].as_array_mut().ok_or_else(||DfmcpError::new(ErrorCode::InternalInvariantViolation,"workforce rows absent"))?.push(json!({
            "citizen":{"entity_id":id.to_string(),"generation":entity.generation,"revision":entity.revision},"name":citizen.name,"profession":citizen.profession,
            "position":[citizen.position.x,citizen.position.y,citizen.position.z],"stress_category":citizen.stress_category,
            "job_available_preserve_social":citizen.job_available_preserve_social,"job_available_interrupt_social":citizen.job_available_interrupt_social,
            "skill":{"nominal":candidate.nominal,"effective":candidate.effective,"experience":candidate.experience},"candidate_steps":candidate.steps,"approach_tile":candidate.approach,
            "route_query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(c.anchor),"query":{"kind":"map_route","start":target,"goal":candidate.approach}},
            "unit_endpoint_step_modeled":candidate.approach!=[citizen.position.x as u32,citizen.position.y as u32,citizen.position.z as u32]}));
        next["returned"]=json!(end+1-start);next["truncated"]=json!(end+1<rows.len());next["continuation"]=if end+1<rows.len(){json!(token(end+1,identity))}else{Value::Null};
        if next.to_string().len()>maximum_bytes{break;}out=next;end+=1;}
    if start<rows.len()&&end==start{return Err(budget("one complete workforce candidate row cannot fit response budget"));}Ok(out)
}

pub(super) fn extend_schema(mut schema:Value)->Result<Value>{
    let variant:Value=serde_json::from_str(include_str!("../../../schemas/mcp_workforce_candidates_v1.json"))
        .map_err(|_|DfmcpError::new(ErrorCode::InternalInvariantViolation,"embedded workforce schema invalid"))?;
    schema["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(||DfmcpError::new(ErrorCode::InternalInvariantViolation,"query variants absent"))?.push(variant);Ok(schema)
}
