//! Explicit read-only recovery at the same session's observation/watch boundary.
//! No automatic retry, admission, journal repair, game effect or source selector.
use super::super::*;
use std::time::Instant;

const KIND: &str = "recover_source";
const MAX_MESSAGE: usize = 256;

pub(super) fn handles(input: &Value) -> bool {
    input.get("query").and_then(|v|v.get("kind")).and_then(Value::as_str) == Some(KIND)
}

fn request(session: &Session, context: &OperationContext, input: &Value) -> Result<OperationContext> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
    if session.source.closed() { return Err(error(ErrorCode::SessionNotFound,"spatial session is closed")); }
    if session.source.archive_only() {
        return Err(error(ErrorCode::CapabilityDenied,"an archive-only session cannot acquire a live source"));
    }
    if !session.source.poisoned() {
        return Err(error(ErrorCode::Conflict,"source is healthy; use observe instead of recover_source"));
    }
    if context.session_id != session.id || context.anchor != session.anchor()? {
        return Err(error(ErrorCode::StaleAnchor,"source recovery names another session or anchor"));
    }
    let invalid=||error(ErrorCode::InvalidRequest,"recover_source requires schema, exact expected_anchor, and a bounded query with no source selector");
    let object=input.as_object().ok_or_else(invalid)?;
    if object.len()!=3 || object.keys().any(|k|!matches!(k.as_str(),"schema"|"expected_anchor"|"query"))
        || input["schema"]!="dfmcp.query/1" { return Err(invalid()); }
    if input["expected_anchor"]!=anchor_json(context.anchor) {
        return Err(error(ErrorCode::StaleAnchor,"recover_source expected_anchor differs from the retained complete anchor"));
    }
    let query=input["query"].as_object().ok_or_else(invalid)?;
    if !(1..=2).contains(&query.len()) || query.keys().any(|k|!matches!(k.as_str(),"kind"|"max_wall_millis"))
        || query.get("kind").and_then(Value::as_str)!=Some(KIND) { return Err(invalid()); }
    let mut narrowed=context.clone();
    if let Some(value)=query.get("max_wall_millis") {
        let millis=value.as_u64().ok_or_else(invalid)?;
        if millis==0 || millis>60_000 || millis>context.budget.max_wall_millis {
            return Err(error(ErrorCode::BudgetExceeded,"recovery wall-time must narrow the session allowance"));
        }
        narrowed.budget.max_wall_millis=millis;
    }
    narrowed.budget.validate()?;
    Ok(narrowed)
}

fn remaining(context: &OperationContext, elapsed: Duration) -> Result<OperationContext> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
    let allowance=Duration::from_millis(context.budget.max_wall_millis).checked_sub(elapsed)
        .filter(|v|*v>=Duration::from_millis(1))
        .ok_or_else(||error(ErrorCode::BudgetExceeded,"source recovery exhausted its shared foreground deadline"))?;
    let mut narrowed=context.clone();narrowed.budget.max_wall_millis=allowance.as_millis() as u64;
    Ok(narrowed)
}

fn diagnostic(failure: &DfmcpError) -> Value {
    let mut end=failure.message.len().min(MAX_MESSAGE);
    while !failure.message.is_char_boundary(end) { end-=1; }
    json!({"code":failure.code.as_str(),"message":&failure.message[..end],
        "message_truncated":end<failure.message.len()})
}

fn details(outcome: Option<JobPublication>, failure: Option<&DfmcpError>,
    connections: u32, captures: u32, pages: u32) -> Value {
    json!({"reconnected":failure.is_none(),"capture_published":outcome.is_some(),
        "capture_outcome":outcome.map(|v|match v { JobPublication::Bootstrap=>"bootstrap",
            JobPublication::Heartbeat=>"heartbeat",JobPublication::Advanced=>"advanced",JobPublication::Reset=>"reset" }),
        "connection_attempts":connections,"capture_attempts":captures,"transfer_pages":pages,
        "error":failure.map(diagnostic)})
}

/// This renderer deliberately does not recompute attention or perform queries.
/// Its whole fixed metadata and the candidate watch projection can be reserved
/// BEFORE any gap checkpoint or connection. The exact final size is checked too.
fn render(session: &Session, context: &OperationContext, target: Value,
    gap: &Value, mut work: Value, report: Value) -> Result<String> {
    let mut obligations=work.as_object_mut().and_then(|v|v.remove("_condition_watch_work"))
        .unwrap_or_else(||json!([]));
    if let Some(rows)=obligations.as_array_mut() {
        for row in rows {
            // The interruption is metadata, not a predicate evaluation, even
            // when reconnect obtains byte-identical state at the same anchor.
            row["evaluation_current"]=json!(false);
            row["predicate_evaluated_during_recovery"]=json!(false);
        }
    }
    let mut active=empty_active_work();active["obligations"]=obligations;
    let succeeded=report["reconnected"]==true;
    let value=json!({"schema":"dfmcp.query.result/1","kind":KIND,"ok":succeeded,
        "session_id":session.id.to_string(),"anchor":target,
        "source_recovery":report,"source_gap":gap,"source_stale":!succeeded,
        "watch_persistence":work.get("watch_persistence"),
        "watch_evaluated":false,"samples_added":0,"mutation_dispatched":false,
        "journal_repair_performed":false,"admission_changed":false,
        "truncated":false,"continuation":null,
        "next_step":{"tool":"fortress.query","arguments":{"session_id":session.id.to_string(),
            "query":{"schema":"dfmcp.query/1","query":{"kind":"watches"}}}},
        "interpretation":"Monitoring was interrupted before the reconnect attempt. Retained handles, definitions, deadlines and terminal evidence survive. Unfinished stability resets; only a later explicit poll/await can add a sample. An unchanged capture cannot create another sample. Recovery does not establish continuous history or game-effect success."});
    let packet=AgentTurnBuilder::new("fortress.query",AgentPhase::Inspect)
        .session_id(session.id.to_string()).request_id(context.request_id.to_string()).anchor(target)
        .active_work(active)
        .briefing(json!({"bridge_protocol":"1.8","runtime_admitted":false,
            "mutation_admissible":false,"read_only":true,"source_recovery":true}))
        .coverage(json!({"status":"partial","complete_domains":["source_recovery_attempt"],
            "partial_domains":[{"domain":"retained_observations","reason":"explicit source outage"}],
            "omitted_domains":["continuous_outage_history","watch_predicate_evaluation"],
            "continuation":null,"continuous_during_outage":false,
            "watch_conditions_evaluated":false,"mutation_success_proven":false}))
        .continuity(if succeeded {ContinuityStatus::Partial} else {ContinuityStatus::Stale},
            Some(anchor_json(context.anchor)),Some(json!({"reason":"explicit_source_recovery",
                "monitoring_gap_recorded":true,"continuous_during_outage":false})),None)
        .attach(value);
    let maximum=context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4);
    if packet.len() as u64>maximum {
        return Err(error(ErrorCode::BudgetExceeded,"complete source-recovery packet exceeds the output allowance"));
    }
    Ok(packet)
}

fn reserve(session: &Session, context: &OperationContext, mut work: Value) -> Result<String> {
    // Watch content will not change during connection/capture: the session lock
    // stays held and no predicate is evaluated. Only these booleans can grow.
    if let Some(rows)=work.get_mut("_condition_watch_work").and_then(Value::as_array_mut) {
        for row in rows { row["evaluation_current"]=json!(false); }
    }
    if let Some(persistence)=work.get_mut("watch_persistence") {
        persistence["checkpoint_changed"]=json!(false);
    }
    let mut target=anchor_json(context.anchor);
    for key in ["epoch","sequence","game_tick"] {target[key]=json!(u64::MAX);}
    let gap=work["source_gap"].clone();
    // All control characters expand under JSON serialization. This reserves
    // more than any allowed UTF-8 diagnostic, including hostile adapter text.
    let worst=json!({"reconnected":false,"capture_published":false,"capture_outcome":"heartbeat",
        "connection_attempts":u32::MAX,"capture_attempts":u32::MAX,"transfer_pages":u32::MAX,
        "error":{"code":"\0".repeat(64),"message":"\0".repeat(MAX_MESSAGE),"message_truncated":false}});
    let encoded=render(session,context,target.clone(),&gap,work.clone(),worst.clone())?;
    let mut success=worst;success["reconnected"]=json!(true);
    render(session,context,target,&gap,work,success)?;
    Ok(encoded)
}

pub(super) fn execute(session: &mut Session, context: &OperationContext, input: &Value) -> Result<String> {
    let started=Instant::now();
    let context=request(session,context,input)?;
    // Operator configuration only; nothing in this request selects a host,
    // token, protocol, plugin, method, journal, path or production admission.
    let endpoint=dfmcp_adapter::parse_loopback_endpoint(&std::env::var("DFMCP_SPATIAL_CITIZEN_ENDPOINT")
        .unwrap_or_else(|_|"127.0.0.1:5000".to_owned()))?;
    let token=std::env::var("DFMCP_SPATIAL_CITIZEN_TOKEN")
        .map_err(|_|error(ErrorCode::CapabilityDenied,"DFMCP_SPATIAL_CITIZEN_TOKEN required for source recovery"))?.into_bytes();
    if !(32..=256).contains(&token.len()) {
        return Err(error(ErrorCode::CapabilityDenied,"spatial source recovery credential length is invalid"));
    }
    let nonce=session.id.get().to_be_bytes().to_vec();let limits=session.limits;
    recover(session,&context,move |allowance| {
        CitizenSpatialRpcClient::connect(endpoint,token,nonce,allowance,limits)
            .map(|source|Box::new(source) as Box<dyn Source>)
    },||started.elapsed())
}

fn recover<F,C>(session: &mut Session, context: &OperationContext, connect: F, mut elapsed: C) -> Result<String>
where F: FnOnce(Duration)->Result<Box<dyn Source>>, C: FnMut()->Duration {
    // Repeat the entry gates at this effect boundary, including injected callers.
    request(session,context,&json!({"schema":"dfmcp.query/1",
        "expected_anchor":anchor_json(context.anchor),"query":{"kind":KIND}}))?;
    super::check_journal(session,context)?;
    if session.journal.as_ref().is_some_and(|journal|journal.recovery_only()) {
        return Err(error(ErrorCode::CapabilityDenied,"source recovery cannot write an archive-only journal"));
    }
    let preflight=remaining(context,elapsed())?;
    let snapshot=session.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"recovery snapshot absent"))?;
    // This is a separate, durable progress boundary. A later network failure
    // does not roll it back, and the returned error packet explicitly says so.
    let gap=semantic_query::WatchJournalGuard::interrupt_source(snapshot,&preflight,|value| {
        let encoded=reserve(session,context,value)?;
        remaining(context,elapsed())?;
        Ok(encoded)
    })?;
    let mut connection_attempts=0u32;let mut capture_attempts=0u32;
    let attempt=(|| {
        let allowance=remaining(context,elapsed())?;
        connection_attempts=1;
        let source=connect(Duration::from_millis(allowance.budget.max_wall_millis))?;
        if source.poisoned()||source.archive_only()||source.closed() {
            return Err(error(ErrorCode::AdapterRejected,"replacement source is not an unfenced live reader"));
        }
        let current=remaining(context,elapsed())?;
        session.source=source;
        capture_attempts=1;
        // Use the EXISTING coherent publication boundary. This validates the
        // exact world/software/profile, bounds and target authority, and syncs
        // the observation journal before publishing any new canonical root.
        observation::refresh_for_query(session,&current)
    })();
    if attempt.is_err() {session.source.fence();}
    let (outcome,failed)=match attempt {Ok(outcome)=>(Some(outcome),None),Err(e)=>(None,Some(e))};
    let mut target=context.clone();target.anchor=session.anchor()?;
    let report=details(outcome,failed.as_ref(),connection_attempts,capture_attempts,if capture_attempts==0 {0} else {session.source.pages()});
    // No post-sync wall-time check may hide an already published observation.
    // Current authority/custody are still checked by the normal watch publisher.
    semantic_query::publish_with_active_work(&target,json!({}),|work|
        render(session,context,anchor_json(target.anchor),&gap,work,report))
}

pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    let extension:Value=serde_json::from_str(include_str!("../../../schemas/mcp_source_recovery_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"source recovery schema is invalid"))?;
    schema["$defs"]["query"]["oneOf"].as_array_mut()
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"source recovery query variants absent"))?
        .push(extension["properties"]["query"].clone());
    let object=schema.as_object_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"query schema root is not an object"))?;
    object.entry("allOf").or_insert_with(||json!([])).as_array_mut()
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"query schema allOf is not an array"))?
        .push(json!({"if":{"required":["query"],"properties":{"query":{"required":["kind"],
            "properties":{"kind":{"const":KIND}}}}},"then":{"required":["expected_anchor"],
            "properties":{"expected_anchor":{"type":"object"}}}}));
    Ok(schema)
}

#[cfg(all(test,unix))]
#[path = "spatial_source_recovery_tests.rs"]
mod tests;
