//! One bounded watch-set transition, not a loop of independently published polls.
//! The enclosing runtime owns the optional single capture. A private preparation
//! binds the selected handles and session registry across that I/O boundary.
use super::*;
use std::time::Instant;

#[path = "query_watch_registration.rs"]
pub(crate) mod registration;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BatchEnvelope { schema: String, expected_anchor: Option<Value>, query: BatchRequest }
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum BatchRequest {
    PollWatches { watches: Option<Vec<String>> },
    AwaitWatches { watches: Option<Vec<String>> },
}

pub(crate) struct Prepared {
    session: SessionId,
    basis: StateAnchor,
    selected: Vec<String>,
    registry: Digest32,
    awaiting: bool,
    needs_observation: bool,
}
impl Prepared {
    pub(crate) fn needs_observation(&self) -> bool { self.needs_observation }
    fn kind(&self) -> &'static str { if self.awaiting { "await_watches" } else { "poll_watches" } }
}
fn check(context: &OperationContext, started: Instant) -> Result<()> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed().as_millis() >= u128::from(context.budget.max_wall_millis) {
        return Err(bounded("watch batch exhausted its foreground evaluation budget"));
    }
    Ok(())
}
fn registry(store: &Store, session: SessionId) -> Result<Digest32> {
    digest(&json!({"domain":"dfmcp-watch-batch-registry/1",
        "session":session.to_string(),
        "records":store.entries.iter().filter(|((id,_),_)|*id==session)
            .map(|((_,handle),watch)|json!([handle,watch.evidence_digest.to_string()])).collect::<Vec<_>>()}))
}
fn record<'a>(store: &'a Store, session: SessionId, handle: &str) -> Result<&'a Watch> {
    store.entries.get(&(session,handle.to_owned())).ok_or_else(||invalid("batch watch is not retained by this session"))
}
fn output(before: &Store, after: &Store, context: &OperationContext, prepared: &Prepared) -> Result<Value> {
    let mut value=payload(context,prepared.kind());
    let mut rows=Vec::with_capacity(prepared.selected.len());
    let mut advanced=0usize; let mut sampled=0usize; let mut terminal=0usize; let mut satisfied=0usize;
    for handle in &prepared.selected {
        let old=record(before,context.session_id,handle)?;
        let current=record(after,context.session_id,handle)?;
        let changed=current.evidence_digest!=old.evidence_digest;
        let sample_added=current.samples>old.samples;
        advanced+=usize::from(changed); sampled+=usize::from(sample_added);
        terminal+=usize::from(current.status.terminal()); satisfied+=usize::from(current.status==Status::Satisfied);
        let mut row=current.summary(context.anchor);
        // Definitions, labels and the potentially large predicate witness tree
        // remain in poll_watch detail. Recovery flags and evidence identities stay.
        if let Some(object)=row.as_object_mut() { object.remove("label"); object.remove("recovery"); }
        row["sample_count"]=json!(current.samples);
        row["condition"]=current.evaluation.get("condition").cloned().unwrap_or(Value::Null);
        row["failure_condition"]=current.evaluation.get("failure_condition").cloned().unwrap_or(Value::Null);
        row["advanced_in_batch"]=json!(changed); row["sample_added"]=json!(sample_added);
        row["became_terminal"]=json!(!old.status.terminal()&&current.status.terminal());
        row["terminal_replayed"]=json!(old.status.terminal());
        rows.push(row);
    }
    value["batch_basis"]=anchor(prepared.basis);
    value["records"]=json!(rows); value["selected"]=json!(prepared.selected.len());
    value["advanced"]=json!(advanced); value["sampled"]=json!(sampled);
    value["terminal"]=json!(terminal); value["remaining"]=json!(prepared.selected.len()-terminal);
    value["all_terminal"]=json!(terminal==prepared.selected.len());
    value["all_satisfied"]=json!(!prepared.selected.is_empty()&&satisfied==prepared.selected.len());
    value["atomic_watch_publication"]=json!(true);
    value["evaluation_scope"]=json!("selected_nonterminal_watches_at_one_snapshot");
    value["terminal_evidence_is_historical"]=json!(true);
    value["detail_query_kind"]=json!("poll_watch");
    value["game_effect_success_proven"]=json!(false);
    Ok(value)
}
fn publish<F>(store: &Store, context: &OperationContext, value: Value, publisher: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    publish_work(store,context,value,|mut value| {
        // The existing checkpoint covers the complete candidate set once. Its
        // persistence metadata, not the request name, determines durability.
        if value["watch_persistence"]["durable"]==true { value["durable"]=json!(true); }
        publisher(value)
    })
}

pub(crate) fn prepare<F>(snapshot: &WorldSnapshot, context: &OperationContext, input: &Value,
    preview: F) -> Result<Prepared>
where F: FnOnce(Value) -> Result<String> {
    prepare_in(&WATCHES,snapshot,context,input,preview)
}
fn prepare_in<F>(watches: &Mutex<Store>, snapshot: &WorldSnapshot, context: &OperationContext,
    input: &Value, preview: F) -> Result<Prepared>
where F: FnOnce(Value) -> Result<String> {
    let started=Instant::now(); check(context,started)?; authorize(snapshot,context)?; validate_input(input)?;
    let request:BatchEnvelope=serde_json::from_value(input.clone()).map_err(|_|invalid("invalid watch batch envelope"))?;
    if request.schema!="dfmcp.query/1" { return Err(invalid("watch batch requires dfmcp.query/1")); }
    if request.expected_anchor.as_ref().is_some_and(|a|a!=&anchor(context.anchor)) {
        return Err(failure(ErrorCode::StaleAnchor,"watch batch expected_anchor differs before acquisition"));
    }
    let (awaiting,selection)=match request.query {
        BatchRequest::PollWatches {watches}=>(false,watches),
        BatchRequest::AwaitWatches {watches}=>(true,watches),
    };
    let store=lock(watches)?;
    let mut selected=match selection {
        Some(handles)=>{
            if handles.is_empty()||handles.len()>MAX_PER_SESSION { return Err(invalid("select one to eight watch handles, or omit watches to select the session set")); }
            handles
        }
        None=>store.entries.keys().filter(|(id,_)|*id==context.session_id).map(|(_,handle)|handle.clone()).collect(),
    };
    if selected.len()>MAX_PER_SESSION { return Err(bounded("watch batch exceeds session retention bound")); }
    selected.sort();
    if selected.windows(2).any(|pair|pair[0]==pair[1]) { return Err(invalid("watch batch handles must be unique")); }
    let mut needs_observation=false;
    for handle in &selected {
        validate_handle(handle)?;
        needs_observation|=awaiting&&!record(&store,context.session_id,handle)?.status.terminal();
    }
    if needs_observation { context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?; }
    let prepared=Prepared {session:context.session_id,basis:context.anchor,selected,
        registry:registry(&store,context.session_id)?,awaiting,needs_observation};
    // Check current custody and render the existing complete selection before
    // I/O. No watch is advanced here. A later state may still require more space;
    // final rendering remains authoritative and precedes any watch publication.
    let value=output(&store,&store,context,&prepared)?;
    publish(&store,context,value,|value| { let result=preview(value)?; check(context,started)?; Ok(result) })?;
    Ok(prepared)
}

pub(crate) fn complete<F>(snapshot: &WorldSnapshot, context: &OperationContext, prepared: Prepared,
    observation_acquired: bool, publisher: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    complete_in(&WATCHES,snapshot,context,prepared,observation_acquired,publisher)
}
fn complete_in<F>(watches: &Mutex<Store>, snapshot: &WorldSnapshot, context: &OperationContext,
    prepared: Prepared, observation_acquired: bool, publisher: F) -> Result<String>
where F: FnOnce(Value) -> Result<String> {
    let started=Instant::now(); check(context,started)?; authorize(snapshot,context)?;
    let mut evaluation_budget=counts::EvaluationBudget::new(context.budget.max_wall_millis);
    if context.session_id!=prepared.session || context.anchor.fortress_id!=prepared.basis.fortress_id
        || observation_acquired!=prepared.needs_observation
        || (!observation_acquired&&context.anchor!=prepared.basis) {
        return Err(failure(ErrorCode::StaleAnchor,"watch batch session or acquisition boundary changed"));
    }
    if observation_acquired { context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?; }
    let mut store=lock(watches)?;
    if registry(&store,context.session_id)?!=prepared.registry {
        return Err(failure(ErrorCode::Conflict,"watch set changed during acquisition; no batch watch transition published"));
    }
    let mut candidate=Store {serial:store.serial,entries:store.entries.clone()};
    for handle in &prepared.selected {
        check(context,started)?;
        let watch=candidate.entries.get_mut(&(context.session_id,handle.clone()))
            .ok_or_else(||invalid("selected batch watch disappeared"))?;
        watch.advance_bounded(snapshot,false,&mut evaluation_budget)?;
    }
    let value=output(&store,&candidate,context,&prepared)?;
    let encoded=publish(&candidate,context,value,|value| {
        check(context,started)?;
        let encoded=publisher(value)?;
        check(context,started)?;
        Ok(encoded)
    })?;
    // A durable checkpoint has now synced. Never perform a fallible check after
    // this point that could hide an acknowledged candidate-set publication.
    *store=candidate;
    Ok(encoded)
}

#[cfg(test)]
#[path="query_watch_batch_tests.rs"]
mod tests;
