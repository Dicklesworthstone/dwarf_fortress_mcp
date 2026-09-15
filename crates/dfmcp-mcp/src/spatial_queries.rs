//! Spatial-only route and supply queries. All facts share the native capture.
use dfmcp_adapter::live_spatial::SpatialStateView;
use dfmcp_adapter::live_map::map_error;
use dfmcp_adapter::spatial_inventory::{self as inventory, SPATIAL_SUPPLY_POLICY};
use dfmcp_adapter::operations_analysis::MaterialDemand;
use dfmcp_core::{Capability,DfmcpError,Digest32,ErrorCode,OperationContext,Result,RiskTier};
use dfmcp_world::map_region::ROUTE_POLICY;
use serde::Deserialize;
use serde_json::{Value,json};
use super::anchor_json;

fn invalid(s:&str)->DfmcpError{DfmcpError::new(ErrorCode::InvalidRequest,s)}
fn budget(s:&str)->DfmcpError{DfmcpError::new(ErrorCode::BudgetExceeded,s)}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope{schema:String,expected_anchor:Option<Value>,query:Query}
#[derive(Deserialize)]
#[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
enum Query{
    MapRoute{start:[u32;3],goal:[u32;3],limit:Option<u32>,continuation:Option<String>,max_work:Option<u64>},
    SpatialInventoryPlan{origin:[u32;3],quantity_unit:QuantityUnit,demands:Vec<DemandInput>,
        limit:Option<u32>,continuation:Option<String>,max_work:Option<u64>},
}
#[derive(Deserialize)]
#[serde(rename_all="snake_case")]
enum QuantityUnit{StackUnits}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DemandInput{key:String,units:u64,item_types:Vec<String>,subtype:Option<i32>,material_type:Option<i32>,material_index:Option<i32>}
impl From<DemandInput> for MaterialDemand{
    fn from(d:DemandInput)->Self{Self{key:d.key,units:d.units,item_types:d.item_types,
        subtype:d.subtype,material_type:d.material_type,material_index:d.material_index}}
}
pub(super) fn handles(input:&Value)->bool{
    matches!(input.get("query").and_then(|v|v.get("kind")).and_then(Value::as_str),Some("map_route"|"spatial_inventory_plan"))
}
fn validate(input:&Value)->Result<()>{
    let mut pending=vec![(input,0usize)];let mut nodes=0usize;let mut bytes=0usize;
    while let Some((v,depth))=pending.pop(){nodes+=1;bytes=bytes.saturating_add(16);
        if depth>12||nodes>4096{return Err(budget("spatial query shape exceeds bounds"));}
        match v{
            Value::String(s)=>bytes=bytes.saturating_add(s.len()),
            Value::Array(a)=>{if nodes+pending.len()+a.len()>4096{return Err(budget("spatial query has too many nodes"));}
                pending.extend(a.iter().map(|v|(v,depth+1)));},
            Value::Object(a)=>{if nodes+pending.len()+a.len()>4096{return Err(budget("spatial query has too many nodes"));}
                for(k,v)in a{bytes=bytes.saturating_add(k.len());pending.push((v,depth+1));}},
            _=>{},
        }
        if bytes>65536{return Err(budget("spatial query input exceeds 64 KiB"));}
    }Ok(())
}
fn identity(c:&OperationContext,source:Digest32,query:Value)->Digest32{
    let mut b=b"dfmcp-spatial-query-v1\0".to_vec();b.extend_from_slice(&c.session_id.get().to_be_bytes());
    b.extend_from_slice(anchor_json(c.anchor).to_string().as_bytes());b.extend_from_slice(source.as_bytes());
    b.extend_from_slice(ROUTE_POLICY.as_bytes());b.extend_from_slice(SPATIAL_SUPPLY_POLICY.as_bytes());
    b.extend_from_slice(query.to_string().as_bytes());Digest32::of_bytes(&b)
}
fn token(offset:usize,digest:Digest32)->String{
    let mut b=b"dfmcp-spatial-page-v1\0".to_vec();b.extend_from_slice(digest.as_bytes());b.extend_from_slice(&(offset as u64).to_be_bytes());
    format!("sp1:{offset}:{}",Digest32::of_bytes(&b))
}
fn offset(raw:Option<&str>,digest:Digest32,count:usize)->Result<usize>{
    let Some(raw)=raw else{return Ok(0);};if raw.len()>128{return Err(budget("spatial cursor too large"));}
    let p:Vec<_>=raw.split(':').collect();
    if p.len()!=3||p[0]!="sp1"||p[1].is_empty()||p[1].starts_with('0')||p[1].len()>6||!p[1].bytes().all(|b|b.is_ascii_digit()){
        return Err(invalid("invalid spatial continuation"));}
    let n=p[1].parse::<usize>().map_err(|_|invalid("invalid spatial offset"))?;
    if raw!=token(n,digest){return Err(DfmcpError::new(ErrorCode::StaleAnchor,"spatial continuation names another session, query or capture"));}
    if n>=count{return Err(DfmcpError::new(ErrorCode::CursorGap,"spatial continuation is beyond the result"));}Ok(n)
}
fn paginate(mut out:Value,count:usize,raw:Option<&str>,limit:u32,id:Digest32,c:&OperationContext,
    row:impl Fn(usize)->Result<Value>)->Result<Value>{
    if !(1..=128).contains(&limit){return Err(budget("spatial page limit must be 1..128"));}
    let first=offset(raw,id,count)?;
    let maximum=usize::try_from(c.budget.max_bytes.min(u64::from(c.budget.max_output_tokens)*4)).map_err(|_|budget("output budget overflow"))?;
    out["analysis_digest"]=json!(id.to_string());out["total_rows"]=json!(count);out["rows"]=json!([]);
    out["returned"]=json!(0);out["truncated"]=json!(false);out["continuation"]=Value::Null;
    let mut end=first;
    while end<count&&end-first<limit as usize{
        let mut candidate=out.clone();candidate["rows"].as_array_mut().ok_or_else(||invalid("spatial rows absent"))?.push(row(end)?);
        candidate["returned"]=json!(end+1-first);candidate["truncated"]=json!(end+1<count);
        candidate["continuation"]=if end+1<count{json!(token(end+1,id))}else{Value::Null};
        if candidate.to_string().len()>maximum{break;}out=candidate;end+=1;
    }
    if (first==end&&first<count)||out.to_string().len()>maximum{return Err(budget("one spatial row or its summary cannot fit the complete response budget"));}Ok(out)
}
fn base(c:&OperationContext,s:Digest32,kind:&str)->Value{
    json!({"schema":"dfmcp.query.result/1","kind":kind,"anchor":anchor_json(c.anchor),"source_digest":s.to_string(),
        "unit_path_proven":false,"safety_proven":false,"global_unreachability_proven":false,
        "reservation_created":false,"commit_compatible":false,"route_policy":ROUTE_POLICY})
}
pub(super) fn execute<T:SpatialStateView>(state:&T,c:&OperationContext,input:&Value)->Result<Value>{
    c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;validate(input)?;
    let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|invalid("invalid spatial query fields"))?;
    if envelope.schema!="dfmcp.query/1"{return Err(invalid("spatial query requires dfmcp.query/1"));}
    let snapshot=state.snapshot().ok_or_else(||invalid("spatial snapshot absent"))?;
    if snapshot.anchor()!=c.anchor||envelope.expected_anchor.is_some_and(|a|a!=anchor_json(c.anchor)){
        return Err(DfmcpError::new(ErrorCode::StaleAnchor,"spatial query anchor differs"));}
    if snapshot.graph.entities.len()>c.budget.max_entities as usize{return Err(budget("spatial query scan exceeds session budget"));}
    let source=state.source_digest()?;
    match envelope.query{
        Query::MapRoute{start,goal,limit,continuation,max_work}=>{
            let max_work=max_work.unwrap_or(1_000_000);
            let map=&state.spatial_observation().ok_or_else(||invalid("spatial source absent"))?.terrain().map;
            let route=map.route(start,goal,max_work).map_err(map_error)?;
            let id=identity(c,source,json!({"kind":"map_route","start":start,"goal":goal,"max_work":max_work}));
            let mut out=base(c,source,"map_route");out["status"]=json!(if route.endpoint_excluded{"endpoint_excluded"}
                else if route.path.is_empty(){"no_route_in_observed_model"}else{"candidate_found"});
            out["model_steps"]=json!(route.path.len().checked_sub(1));out["visited_tiles"]=json!(route.visited_tiles);
            out["work_units"]=json!(route.work_units);out["touched_region_boundary"]=json!(route.touched_region_boundary);
            paginate(out,route.path.len(),continuation.as_deref(),limit.unwrap_or(16),id,c,|i|Ok(json!({"position":route.path[i]})))
        }
        Query::SpatialInventoryPlan{origin,quantity_unit:QuantityUnit::StackUnits,demands,limit,continuation,max_work}=>{
            let requested:Vec<_>=demands.into_iter().map(MaterialDemand::from).collect();let max_work=max_work.unwrap_or(10_000_000);
            let report=inventory::plan(state,c,origin,&requested,max_work)?;
            let normalized:Vec<_>=report.demands.iter().map(|d|json!({"key":d.key,"units":d.units,"item_types":d.item_types,
                "subtype":d.subtype,"material_type":d.material_type,"material_index":d.material_index})).collect();
            let id=identity(c,source,json!({"kind":"spatial_inventory_plan","origin":origin,"demands":normalized,"max_work":max_work}));
            let mut out=base(c,source,"spatial_inventory_plan");out["origin"]=json!(origin);out["supply_policy"]=json!(SPATIAL_SUPPLY_POLICY);
            out["quantity_unit"]=json!("stack_units");out["model_feasible"]=json!(report.allocation.shortage.is_none());
            out["summary"]=json!({"requested_units":report.allocation.requested_units,"allocated_units":report.allocation.allocated_units,
                "item_classification":report.item_counts,"reachable_tiles":report.reachable_tiles,"work_units":report.work_units});
            out["certificate"]=json!({"flow_units":report.allocation.allocated_units,"cut_capacity":report.allocation.cut_capacity,
                "shortage":report.allocation.shortage.as_ref().map(|s|json!({"demand_keys":s.demand_indices.iter().map(|&i|&report.demands[i].key).collect::<Vec<_>>(),
                    "required_units":s.required_units,"eligible_units":s.eligible_units,"deficit":s.deficit}))});
            out["interpretation"]=json!("Conditional on declared interchangeable stack units, conservative item policy and this bounded observed route model; not native job feasibility or proof that excluded supply is globally inaccessible.");
            out["touched_region_boundary"]=json!(report.touched_region_boundary);
            paginate(out,report.allocation.assignments.len(),continuation.as_deref(),limit.unwrap_or(8),id,c,|i|{
                let a=&report.allocation.assignments[i];let location=report.locations.get(&a.supply_id).ok_or_else(||invalid("allocation location absent"))?;
                Ok(json!({"demand_key":report.demands[a.demand_index].key,"units":a.units,
                    "item":{"entity_id":location.item_id.to_string(),"generation":location.generation},
                    "ground_root":{"entity_id":location.outermost_item_id.to_string(),"generation":location.outermost_generation},
                    "position":location.position,"candidate_steps":location.candidate_steps,
                    "route_query":{"schema":"dfmcp.query/1","expected_anchor":anchor_json(c.anchor),
                        "query":{"kind":"map_route","start":origin,"goal":location.position}}}))
            })
        }
    }
}
pub(super) fn schema()->Result<Value>{
    let mut base:Value=serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json")).map_err(|_|invalid("base schema invalid"))?;
    let mut route:Value=serde_json::from_str(include_str!("../../../schemas/mcp_map_route_v1.json")).map_err(|_|invalid("map schema invalid"))?;
    route["properties"]["limit"]["maximum"]=json!(128);
    route["properties"]["continuation"]["oneOf"][1]["pattern"]=json!("^sp1:[1-9][0-9]*:[0-9a-f]{64}$");
    let inventory:Value=serde_json::from_str(include_str!("../../../schemas/mcp_spatial_inventory_v1.json")).map_err(|_|invalid("spatial schema invalid"))?;
    let variants=base["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(||invalid("base query variants absent"))?;
    variants.push(route);variants.push(inventory);Ok(base)
}
