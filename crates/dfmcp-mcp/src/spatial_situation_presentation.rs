//! Fixed presentation profiles, chosen before query execution. Tactical results
//! retain the highest-priority signal and a complete briefing request; full
//! evidence and every signal count are returned by the situation mode. Nothing
//! is removed reactively after a response-budget failure.
use super::*;

pub(super) fn tactical(s:&Session,c:&OperationContext)->Result<QueryResponseProjection>{
    let report=situation_report(s,c)?;
    let mut projection=view(s,c)?;
    let Some(report)=report else{
        projection.briefing=situation_briefing(s,None);
        return Ok(projection);
    };
    let summary=report.summary();
    let groups=summary["attention_groups"].as_u64().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"situation group count absent"))?;
    let unknown=summary["signals"].as_object().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"situation signal counts absent"))?
        .values().filter(|signal|signal["unestablished"].as_u64().is_some_and(|n|n>0)).count();
    projection.briefing["situation"]=json!({"policy":situation::POLICY,"projection":"compact",
        "attention_groups":groups,"attention_groups_omitted":groups.saturating_sub(1),
        "unestablished_groups":unknown,"all_clear_proven":false,
        "detail_query":{"tool":"fortress.query","arguments":{"session_id":s.id.to_string(),"mode":"situation"}}});
    projection.attention=report.attention(s.id).into_iter().take(1).map(|mut finding|{
        if let Some(fields)=finding.as_object_mut(){fields.retain(|name,_|matches!(name.as_str(),
            "attention_id"|"rule"|"severity"|"observed_count"|"unestablished_count"|"epistemic_state"));}
        finding
    }).collect();
    Ok(projection)
}

/// The general query reservation includes endpoint-comparison metadata that
/// watch operations never emit. Reserve their own worst metadata shape using
/// the actual final renderer, counting the result payload only once. The
/// existing watch publisher still checks its complete result and Agent Turn
/// before any in-memory publication or durable checkpoint append.
pub(super) fn result_budget(projection:&QueryResponseProjection,input:&Value)->Result<usize>{
    let kind=input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str);
    if !matches!(kind,Some("watch"|"poll_watch"|"watches"|"cancel_watch"|"release_watch")){
        return projection.result_byte_budget();
    }
    let sample=json!({"kind":"watch","anchor":projection.anchor,"truncated":true,"continuation":"x".repeat(256),
        "record":{"watch":format!("watch:{}","f".repeat(64))},
        "observation_refresh":{"basis":projection.anchor,"reset":true,"kind":"snapshot",
            "native_captures":u64::MAX,"transfer_pages":u32::MAX}});
    let payload_bytes=sample.to_string().len();
    let packet_bytes=finish(projection,sample)?.len();
    let overhead=packet_bytes.checked_sub(payload_bytes).and_then(|n|n.checked_add(256))
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"watch response reservation overflow"))?;
    projection.maximum_bytes.checked_sub(overhead).filter(|n|*n>0)
        .ok_or_else(||error(ErrorCode::BudgetExceeded,"watch metadata leaves no result allowance"))
}

pub(super) fn detail(s:&Session,c:&OperationContext)->Result<String>{
    c.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    if s.source.archive_only(){return Err(error(ErrorCode::CapabilityDenied,"live situation mode is unavailable in an archive session"));}
    if s.source.poisoned(){return Err(error(ErrorCode::AdapterUnavailable,"situation source is fenced; no current operational finding is published"));}
    semantic_query::publish_with_active_work(c,json!({"ok":true,"kind":"situation","native_captures":0,
        "inspection_scope":"last_coherent_capture_not_realtime_state","cause_or_safety_proven":false}),
        |value|packet(Some(s),Some(c),"fortress.query",value))
}
