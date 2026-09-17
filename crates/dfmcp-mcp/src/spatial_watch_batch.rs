//! Foreground batch monitoring for the live spatial/1.8 runtime. Session locking
//! serializes capture and watch publication. Archive-only sessions remain reads
//! of historical facts, never an alternate watch evaluator or bridge source.
use super::*;
use std::time::Instant;

pub(super) fn handles(input: &Value) -> bool {
    matches!(input.get("query").and_then(|q|q.get("kind")).and_then(Value::as_str),
        Some("poll_watches"|"await_watches"|"register_watches"))
}

fn registration_schema(mut schema: Value) -> Result<Value> {
    let invalid=||error(ErrorCode::InternalInvariantViolation,"registration requires one canonical watch request schema");
    let variants=schema["$defs"]["query"]["oneOf"].as_array().ok_or_else(invalid)?;
    let mut found=None;
    for variant in variants {
        let resolved=match variant.get("$ref").and_then(Value::as_str) {
            Some(reference)=>schema.pointer(reference.strip_prefix('#').ok_or_else(invalid)?).ok_or_else(invalid)?,
            None=>variant,
        };
        if resolved["properties"]["kind"]["const"]=="watch" {
            if found.is_some() {return Err(invalid());}
            found=Some(resolved.clone());
        }
    }
    // Derive each member from the exact single-watch schema after population and
    // quantity conditions have been composed. No copied condition dialect.
    let mut member=found.ok_or_else(invalid)?;
    member["properties"].as_object_mut().ok_or_else(invalid)?.remove("kind");
    member["required"].as_array_mut().ok_or_else(invalid)?.retain(|name|name!="kind");
    let definitions=schema["$defs"].as_object_mut().ok_or_else(invalid)?;
    if definitions.contains_key("watch_set_member") {return Err(invalid());}
    definitions.insert("watch_set_member".into(),member);
    schema["$defs"]["query"]["oneOf"].as_array_mut().ok_or_else(invalid)?.push(json!({
        "type":"object","additionalProperties":false,"required":["kind","watches"],
        "description":"Atomically register 1..8 unique watch keys at the current capture. Exact existing definitions replay without sampling; changed keys refuse the entire set. Aggregate input/work/output budgets apply.",
        "properties":{"kind":{"const":"register_watches"},"watches":{
            "type":"array","minItems":1,"maxItems":8,"items":{"$ref":"#/$defs/watch_set_member"}}}
    }));
    Ok(schema)
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
    registration_schema(schema)
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
        return Err(error(ErrorCode::CapabilityDenied,"archive-only sessions cannot register, evaluate or await watch batches"));
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
    if input["query"]["kind"]=="register_watches" {
        return semantic_query::register_watch_set(snapshot,&remaining(context,started)?,input,|mut value| {
            value["source_stale"]=json!(false);
            let encoded=finish(&preview,value)?;
            remaining(context,started)?;
            Ok(encoded)
        });
    }
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
