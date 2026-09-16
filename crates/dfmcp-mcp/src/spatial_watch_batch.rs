//! Foreground batch monitoring for the live spatial/1.8 runtime. Session locking
//! serializes capture and watch publication. Archive-only sessions remain reads
//! of historical facts, never an alternate watch evaluator or bridge source.
use super::*;
use std::time::Instant;

pub(super) fn handles(input: &Value) -> bool {
    matches!(input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str),
        Some("poll_watches"|"await_watches"))
}
pub(super) fn extend_schema(mut schema: Value) -> Result<Value> {
    // Shared quantity queries and population predicates are composed by history::schema,
    // also used for archive discovery. Only live batch operations are added here.
    let additions:Value=serde_json::from_str(include_str!("../../../schemas/mcp_watch_batch_v1.json"))
        .map_err(|_|error(ErrorCode::InternalInvariantViolation,"watch batch schema is invalid"))?;
    let variants=additions["oneOf"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"batch variants absent"))?;
    schema["$defs"]["query"]["oneOf"].as_array_mut()
        .ok_or_else(||error(ErrorCode::InternalInvariantViolation,"spatial query variants absent"))?
        .extend(variants.iter().cloned());
    Ok(schema)
}
fn remaining(context: &OperationContext, started: Instant) -> Result<OperationContext> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    let elapsed=started.elapsed().as_millis();
    if elapsed>=u128::from(context.budget.max_wall_millis) {
        return Err(error(ErrorCode::BudgetExceeded,"watch batch exhausted its shared preflight/capture/evaluation deadline"));
    }
    let mut narrowed=context.clone(); narrowed.budget.max_wall_millis-=elapsed as u64; Ok(narrowed)
}
fn check_journal(session: &mut Session, context: &OperationContext) -> Result<()> {
    if let Some(journal)=session.journal.as_mut() {
        journal.validate_custody(context)?;
        if journal.state().snapshot().map(|s|s.anchor())!=Some(context.anchor) {
            return Err(error(ErrorCode::CorruptLedger,"watch batch observation archive disagrees with published state"));
        }
    }
    Ok(())
}

pub(super) fn execute(session: &mut Session, context: &OperationContext, input: &Value) -> Result<String> {
    let started=Instant::now();remaining(context,started)?;
    if session.source.archive_only() {
        return Err(error(ErrorCode::CapabilityDenied,"archive-only sessions cannot evaluate or await watch batches"));
    }
    if session.source.poisoned() {
        return Err(error(ErrorCode::AdapterUnavailable,"watch batch source is fenced; reopen before evaluating watches"));
    }
    if context.anchor!=session.anchor()? {
        return Err(error(ErrorCode::StaleAnchor,"watch batch context differs from the session anchor"));
    }
    check_journal(session,context)?;
    let preview=situation_presentation::tactical(session,context)?;
    let snapshot=session.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"batch snapshot absent"))?;
    let prepared=semantic_query::prepare_watch_batch(snapshot,&remaining(context,started)?,input,|mut value| {
        value["native_captures"]=json!(0);value["source_stale"]=json!(false);
        // This preflight checks the complete current selection and custody, not
        // a prediction of its next outcome. The final packet is checked again.
        finish(&preview,value)
    })?;
    let acquired=prepared.needs_observation();
    let mut current=context.clone();
    let refresh=if acquired {
        let outcome=session.refresh(&remaining(context,started)?)?;
        current.anchor=session.anchor()?;
        current.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
        current.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
        Some(json!({"basis":anchor_json(context.anchor),"reset":outcome==JobPublication::Reset,
            "kind":if outcome==JobPublication::Heartbeat {"heartbeat"} else {"snapshot"},
            "native_captures":1,"transfer_pages":session.source.pages()}))
    } else {None};
    check_journal(session,&current)?;
    let mut projection=situation_presentation::tactical(session,&remaining(&current,started)?)?;
    projection.coverage["condition_watches"]=json!({"status":"sampled_observations_only",
        "continuous_between_observations":false,"game_effect_success_proven":false});
    let evaluation=remaining(&current,started)?;
    let snapshot=session.state.snapshot().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"batch target snapshot absent"))?;
    semantic_query::complete_watch_batch(snapshot,&evaluation,prepared,acquired,|mut value| {
        value["native_captures"]=json!(u32::from(acquired));value["source_stale"]=json!(false);
        if let Some(refresh)=refresh {value["observation_refresh"]=refresh;}
        let mut next=input.clone();
        if let Some(object)=next.as_object_mut() {object.remove("expected_anchor");}
        next["query"]["kind"]=json!("await_watches");
        value["next_step"]=if value["all_terminal"]==true {Value::Null} else {
            json!({"tool":"fortress.query","arguments":{"session_id":session.id.to_string(),"query":next}})
        };
        let encoded=finish(&projection,value)?;
        remaining(&current,started)?;
        Ok(encoded)
    })
    // No fallible post-sync check: successful complete_watch_batch has already
    // durably committed its whole candidate set before swapping the watch root.
}

#[cfg(all(test,unix))]
#[path="spatial_watch_batch_tests.rs"]
mod tests;
