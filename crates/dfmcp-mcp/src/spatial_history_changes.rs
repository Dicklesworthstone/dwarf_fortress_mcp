//! Durable endpoint comparisons for live and archive-only spatial/1.8 sessions.
//! Record replay and selection comparison share one foreground deadline. No
//! baseline, watch, live capture, journal append or effect is created here.
use super::super::*;
use std::time::Instant;
use dfmcp_adapter::operations_journal::JournalEntry;
use dfmcp_core::Digest32;
use dfmcp_world::WorldSnapshot;

#[path = "spatial_history_series.rs"]
pub(in super::super) mod series;
#[path = "spatial_history_watch_replay.rs"]
pub(in super::super) mod monitor;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecordRef { record: u64, record_digest: String }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope { schema: String, expected_anchor: Option<Value>, query: Request }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    HistoricalChanges { from: RecordRef, to: RecordRef, select: Value,
        limit: Option<u32>, continuation: Option<String> },
}
fn invalid(text: &str) -> DfmcpError { error(ErrorCode::InvalidRequest, text) }
fn bounded(text: &str) -> DfmcpError { error(ErrorCode::BudgetExceeded, text) }

pub(in super::super) fn handles(input: &Value) -> bool {
    monitor::handles(input) || series::handles(input) || input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str) == Some("historical_changes")
}
pub(in super::super) fn schema() -> Result<Value> {
    serde_json::from_str(include_str!("../../../schemas/mcp_historical_changes_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"historical changes schema is invalid"))
}
fn shape(input: &Value) -> Result<()> {
    let mut pending=vec![(input,0usize)]; let mut nodes=0usize; let mut bytes=0usize;
    while let Some((value,depth))=pending.pop() {
        nodes+=1; bytes=bytes.saturating_add(32);
        if nodes>4096 || depth>24 { return Err(bounded("historical changes input exceeds shape bounds")); }
        match value {
            Value::String(text)=>bytes=bytes.saturating_add(text.len()),
            Value::Array(values)=>{
                if nodes.saturating_add(pending.len()).saturating_add(values.len())>4096 { return Err(bounded("historical changes array too wide")); }
                pending.extend(values.iter().map(|v|(v,depth+1)));
            }
            Value::Object(values)=>{
                if nodes.saturating_add(pending.len()).saturating_add(values.len())>4096 { return Err(bounded("historical changes object too wide")); }
                for (key,value) in values { bytes=bytes.saturating_add(key.len()); pending.push((value,depth+1)); }
            }
            _=>{}
        }
        if bytes>65536 { return Err(bounded("historical changes input exceeds byte bound")); }
    }
    Ok(())
}
fn remaining(context: &OperationContext, started: Instant) -> Result<OperationContext> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    let elapsed=started.elapsed().as_millis();
    if elapsed>=u128::from(context.budget.max_wall_millis) { return Err(bounded("historical changes exhausted the shared replay/comparison deadline")); }
    let mut narrowed=context.clone(); narrowed.budget.max_wall_millis-=elapsed as u64; Ok(narrowed)
}
fn selected(journal: &history::Journal, requested: &RecordRef) -> Result<JournalEntry> {
    if !(1..=4096).contains(&requested.record) { return Err(invalid("historical record must be 1..4096")); }
    let digest=Digest32::from_hex(&requested.record_digest).filter(|d|d.to_string()==requested.record_digest)
        .ok_or_else(||invalid("record_digest must be canonical lowercase SHA-256"))?;
    let index=usize::try_from(requested.record-1).map_err(|_|invalid("record index overflow"))?;
    let entry=journal.entries().get(index).ok_or_else(||error(ErrorCode::CursorGap,"comparison record is not retained"))?;
    if entry.record_digest!=digest { return Err(error(ErrorCode::StaleAnchor,"comparison record digest differs from retained history")); }
    Ok(entry.clone())
}
fn replay(journal: &mut history::Journal, entry: &JournalEntry, context: &OperationContext,
    limits: CitizenSpatialLimits) -> Result<WorldSnapshot> {
    let state=journal.state_at(entry.number,entry.record_digest,context)?;
    archive::validate_limits(state.observation_full().ok_or_else(||error(ErrorCode::CorruptLedger,"comparison source absent"))?,limits)?;
    state.snapshot().cloned().ok_or_else(||error(ErrorCode::CorruptLedger,"comparison projection absent"))
}

fn render(session: &Session, context: &OperationContext, target: &JournalEntry,
    projection: Option<&QueryResponseProjection>, value: Value) -> Result<String> {
    let basis=value.get("basis").cloned().unwrap_or(Value::Null);
    let summary=json!({"kind":"archived_endpoint_comparison","change_count":value.get("change_count"),
        "returned":value.get("returned"),"detail_path":"changes","intermediate_history_proven":false,
        "evidence":[value.get("basis_result_digest"),value.get("target_result_digest")]});
    let raw=match projection {
        Some(projection)=>finish(projection,value)?,
        None=>archive::packet(session,context,"fortress.query",Some(target),value)?,
    };
    let mut packet:Value=serde_json::from_str(&raw).map_err(|_|error(ErrorCode::InternalInvariantViolation,"comparison packet is not JSON"))?;
    packet["agent_turn"]["continuity"]["status"]=json!("partial");
    packet["agent_turn"]["continuity"]["basis"]=basis;
    packet["agent_turn"]["continuity"]["gap"]=json!({"reason":"historical_endpoint_comparison_not_continuous_or_current_game_state"});
    packet["agent_turn"]["changes"]=json!([summary]);
    let encoded=packet.to_string();
    if encoded.len() as u64>context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4) {
        return Err(bounded("complete historical change packet exceeds output budget"));
    }
    Ok(encoded)
}

pub(in super::super) fn execute(session: &mut Session, context: &OperationContext, input: &Value) -> Result<String> {
    if monitor::handles(input) { return monitor::execute(session,context,input); }
    if series::handles(input) { return series::execute(session,context,input); }
    let started=Instant::now(); remaining(context,started)?; shape(input)?;
    if session.anchor()?!=context.anchor { return Err(error(ErrorCode::StaleAnchor,"comparison context is not the current session anchor")); }
    let envelope:Envelope=serde_json::from_value(input.clone()).map_err(|_|invalid("invalid historical changes envelope"))?;
    if envelope.schema!="dfmcp.query/1" { return Err(invalid("historical changes requires dfmcp.query/1")); }
    if envelope.expected_anchor.as_ref().is_some_and(|a|a!=&anchor_json(context.anchor)) {
        return Err(error(ErrorCode::StaleAnchor,"historical changes expected_anchor differs from current session"));
    }
    let Request::HistoricalChanges {from,to,select,limit,continuation}=envelope.query;
    if from.record>to.record { return Err(invalid("historical comparison requires from.record <= to.record")); }
    if limit.is_some_and(|n|n==0||n>128||n>context.budget.max_entities)
        || continuation.as_ref().is_some_and(|token|token.len()>128) { return Err(invalid("historical changes page arguments exceed their bounds")); }
    let selector=select.as_object().ok_or_else(||invalid("select must be an entities selection"))?;
    if select["kind"]!="entities" || selector.keys().any(|k|!matches!(k.as_str(),"kind"|"kinds"|"where"|"fields"|"order")) {
        return Err(invalid("select supports only entities, kinds, where, fields and order; comparison owns pagination"));
    }
    let journal=session.journal.as_mut().ok_or_else(||invalid("historical changes requires the operator-configured spatial/1.8 journal"))?;
    journal.validate_custody(context)?;
    let first=selected(journal,&from)?; let last=selected(journal,&to)?;
    if first.anchor.cursor.epoch!=last.anchor.cursor.epoch || last.anchor.tick<first.anchor.tick {
        return Err(error(ErrorCode::StaleAnchor,"comparison crosses an observation reset or regressed game clock"));
    }
    let archive_id=journal.id(); let archive_head=journal.head();
    let metadata=json!({"historical":true,"live":false,"current_freshness_proven":false,"native_captures":0,
        "comparison":{"journal_id":archive_id.to_string(),"journal_head":archive_head.to_string(),
            "from":archive::entry_json(&first),"to":archive::entry_json(&last)},
        "current_session_anchor":anchor_json(context.anchor)});
    let binding=Digest32::of_bytes(metadata.to_string().as_bytes());
    let mut projection=if session.source.archive_only() { None } else { Some(view(session,context)?) };
    if let Some(p)=projection.as_mut() {
        p.anchor=anchor_json(last.anchor);
        p.briefing=json!({"runtime":"unadmitted_development","bridge_protocol":"1.8","historical":true,
            "read_only":true,"live":false,"runtime_admitted":false,"mutation_admissible":false,
            "current_live_anchor":anchor_json(context.anchor),"live_source_fenced":session.source.poisoned(),
            "active_work_basis":"current_session_not_archived"});
        p.coverage=json!({"status":"partial","complete_domains":["selected_endpoint_projections"],
            "omitted_domains":["current_game_state","events_between_endpoints"],"absence_proven":false,
            "temporal_coverage":"endpoint_comparison_only","current_freshness_proven":false,"continuation":null});
        p.references=vec![json!({"kind":"verified_comparison_endpoints","journal_id":archive_id.to_string(),
            "from_digest":first.record_digest.to_string(),"to_digest":last.record_digest.to_string()})];
    }
    // Reserve the full mode-specific packet, both record witnesses, duplicate
    // continuation and largest comparison summary BEFORE any historical replay.
    let mut sample=metadata.clone();
    sample["basis"]=anchor_json(first.anchor); sample["anchor"]=anchor_json(last.anchor);
    sample["kind"]=json!("historical_changes"); sample["change_count"]=json!(u64::MAX);
    sample["returned"]=json!(u64::MAX); sample["truncated"]=json!(true); sample["continuation"]=json!("x".repeat(128));
    sample["basis_result_digest"]=json!("f".repeat(64)); sample["target_result_digest"]=json!("f".repeat(64));
    let overhead=render(session,context,&last,projection.as_ref(),sample)?.len() as u64;
    let mut comparison=remaining(context,started)?;
    comparison.budget.max_bytes=context.budget.max_bytes.min(u64::from(context.budget.max_output_tokens)*4)
        .checked_sub(overhead+64).filter(|n|*n>0).ok_or_else(||bounded("archive evidence leaves no comparison result budget"))?;
    if projection.is_some() { comparison=semantic_query::result_context(&comparison)?; }
    let mut read_context=history::replay_context(session,&remaining(context,started)?);
    let limits=session.limits;
    let journal=session.journal.as_mut().ok_or_else(||invalid("comparison journal disappeared"))?;
    let before=replay(journal,&first,&read_context,limits)?;
    read_context.budget.max_wall_millis=remaining(context,started)?.budget.max_wall_millis;
    let after=if first.number==last.number {before.clone()} else {replay(journal,&last,&read_context,limits)?};
    comparison.budget.max_wall_millis=remaining(context,started)?.budget.max_wall_millis;
    let mut value=semantic_query::compare_endpoints(&before,&after,&comparison,&select,binding,limit,continuation.as_deref())?;
    journal.validate_custody(context)?;
    if journal.id()!=archive_id || journal.head()!=archive_head { return Err(error(ErrorCode::StaleAnchor,"comparison archive head changed")); }
    let result=value.as_object_mut().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"comparison result is not an object"))?;
    if let Some(fields)=metadata.as_object() { result.extend(fields.iter().map(|(k,v)|(k.clone(),v.clone()))); }
    remaining(context,started)?;
    if projection.is_some() {
        semantic_query::publish_with_active_work(context,value,|v| {
            let result=render(session,context,&last,projection.as_ref(),v)?;
            remaining(context,started)?; Ok(result)
        })
    } else {
        let result=render(session,context,&last,None,value)?;
        remaining(context,started)?; Ok(result)
    }
}
