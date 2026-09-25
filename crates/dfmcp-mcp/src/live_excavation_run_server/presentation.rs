//! Complete, bounded Agent Turns over verified excavation evidence.
use super::*;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work,
    empty_budget, recommendation, uncertainty};
use std::collections::VecDeque;

pub(super) fn failure(cause: &dfmcp_core::DfmcpError) -> Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),
        "message":"Excavation authority, review, source, budget or custody verification failed. Preserve the original journal and inspect its exact key."},
        "retry_commit_permitted":false,"native_nonapplication_proven":false})
}
pub(super) fn plan(value: &ExcavationRunPlan) -> Value {
    let [ticks, wall, samples, stable, interval, gap] = value.spec().values();
    json!({"key":value.key(),"plan_digest":value.digest().to_string(),
        "observation_witness":value.before().witness().to_string(),"before_tick":value.before().tick(),
        "region":value.before().region().values(),"game_ticks":ticks,"wall_millis":wall,
        "samples":samples,"stable_ticks":stable,"interval_ticks":interval,"max_gap_ticks":gap,
        "deadline_tick":value.before().tick()+u64::from(ticks),"designation_dispatched":false})
}
fn phase(value: RunPhase) -> &'static str {
    match value { RunPhase::Prepared=>"prepared",RunPhase::Running=>"running",RunPhase::Stopping=>"stopping",
        RunPhase::Stopped=>"stopped",RunPhase::Refused=>"refused",RunPhase::SourceLost=>"source_lost" }
}
fn trigger(value: ExcavationTrigger) -> &'static str {
    match value { ExcavationTrigger::None=>"none",ExcavationTrigger::FloorObserved=>"floor_observed",
        ExcavationTrigger::SourceChanged=>"source_changed",ExcavationTrigger::CaptureFailure=>"capture_failure",
        ExcavationTrigger::Unobservable=>"unobservable",ExcavationTrigger::LiquidObserved=>"liquid_observed" }
}
pub(super) fn record(entry: &ExcavationEntry) -> Value {
    let native = entry.native().map(|r| json!({"phase":phase(r.phase()),"reason":r.reason().as_str(),
        "trigger":trigger(r.trigger()),"receipt_digest":r.receipt().to_string(),"observed_tick":r.observed_tick(),
        "stable_samples":r.stable_samples(),"first_stable_tick":r.first_stable_tick(),
        "last_sample_tick":r.last_capture_tick(),"historical_pause_verified":r.historical_pause_verified(),
        "sampled_floor_reported":r.sampled_floor_reported()}));
    json!({"key":entry.plan().key(),"plan_digest":entry.plan().digest().to_string(),
        "dispatch_started":entry.dispatch_started(),"cancel_requested":entry.cancel_requested(),
        "terminal":entry.native().is_some_and(ExcavationRunRecord::terminal),"unresolved":entry.unresolved(),
        "native":native,"current_pause_proven":false,"mining_causality_proven":false,"retry_commit_permitted":false})
}
pub(super) fn observation(value: &ExcavationCapture) -> Value {
    let [x,y,z,width,_] = value.region().values();
    let mut matching = 0;
    let mut hidden = 0;
    let mut missing = 0;
    let cells = value.cells().iter().enumerate().map(|(index, cell)| {
        let coord = [x + index as u32 % width, y + index as u32 / width, z];
        match cell {
            ExcavationCell::Hidden => { hidden += 1; json!({"coordinate":coord,"presence":"hidden"}) }
            ExcavationCell::Missing => { missing += 1; json!({"coordinate":coord,"presence":"missing"}) }
            ExcavationCell::Visible { shape, liquid, dig } => {
                matching += usize::from(cell.floor_observed());
                json!({"coordinate":coord,"presence":"visible","shape":shape,"liquid_depth":liquid,"dig":dig})
            }
        }
    }).collect::<Vec<_>>();
    json!({"witness":value.witness().to_string(),"generation":value.generation(),"sequence":value.sequence(),
        "game_tick":value.tick(),"paused_at_capture":value.paused(),"region":value.region().values(),
        "dimensions":value.dimensions(),"matched_floor_cells":matching,"hidden_cells":hidden,"missing_cells":missing,
        "cells":cells,"historical_evidence_only":true,"current_terrain_proven":false,"safety_proven":false})
}
fn anchor(view: &ExcavationInventory) -> Value {
    json!({"schema":"dfmcp.excavation-inventory-anchor/1","fortress_id":view.binding().fortress().fortress_id().to_string(),
        "bridge_generation":view.binding().generation(),"inventory_digest":view.digest().to_string(),
        "high_observed_tick":view.high_tick(),"scope":"retained_inventory_not_canonical_world"})
}
fn summary(view: &ExcavationInventory) -> Value {
    let b = view.binding();
    json!({"anchor":anchor(view),"total_records":view.entries().len(),"unresolved_records":view.pending_count(),
        "world_folder":b.fortress().folder(),"site_id":b.fortress().site(),"dimensions":b.dimensions(),
        "df_version":b.df_version(),"dfhack_version":b.dfhack_version(),
        "global_controller_inventory":false,"external_anti_rollback_verified":false})
}
#[derive(Clone)]
struct Cursor { session: SessionId, token: String, root: Digest32, filter: Filter, limit: usize, offset: usize }
#[derive(Clone, Default)]
pub(super) struct Cursors { serial: u64, entries: VecDeque<Cursor> }
impl Cursors {
    fn issue(&mut self, root: Digest32, filter: Filter, limit: usize, offset: usize, c: &OperationContext) -> Result<String> {
        self.serial = self.serial.checked_add(1).ok_or_else(exhausted)?;
        let mut bytes = b"dfmcp-excavation-mcp-cursor/1\0".to_vec();
        bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
        bytes.extend_from_slice(root.as_bytes());
        bytes.extend_from_slice(&self.serial.to_be_bytes());
        let token = Digest32::of_bytes(&bytes).to_string();
        if self.entries.len() == 64 { self.entries.pop_front(); }
        self.entries.push_back(Cursor { session: c.session_id, token: token.clone(), root, filter, limit, offset });
        Ok(token)
    }
    fn resolve(&self, token: &str, root: Digest32, filter: Filter, limit: usize, session: SessionId) -> Result<usize> {
        requests::digest(token)?;
        self.entries.iter().find(|entry| entry.token == token && entry.session == session && entry.root == root
            && entry.filter == filter && entry.limit == limit).map(|entry| entry.offset)
            .ok_or_else(|| error(ErrorCode::StaleAnchor,"excavation page no longer names this exact inventory"))
    }
}
fn listing(view: &ExcavationInventory, request: &QueryRequest, cursors: &mut Cursors, c: &OperationContext) -> Result<Value> {
    match request {
        QueryRequest::Schema {} => {
            let schema: Value = serde_json::from_str(include_str!("../../../../schemas/mcp_excavation_run_v1.json"))
                .map_err(|_| error(ErrorCode::InternalInvariantViolation,"invalid published excavation schema"))?;
            Ok(json!({"ok":true,"request_contract":schema}))
        }
        QueryRequest::Records { state, limit, continuation } => {
            let filter = state.unwrap_or_default();
            let limit = limit.unwrap_or(requests::MAX_PAGE as u32) as usize;
            request.validate()?;
            let entries = view.entries().iter().filter(|entry| filter.includes(entry)).collect::<Vec<_>>();
            let offset = match continuation {
                Some(token) => cursors.resolve(token,view.digest(),filter,limit,c.session_id)?, None=>0,
            };
            if offset > entries.len() { return Err(invalid()); }
            let rows = entries.iter().skip(offset).take(limit).map(|entry| record(entry)).collect::<Vec<_>>();
            let next = offset + rows.len();
            let continuation = if next < entries.len() { Some(cursors.issue(view.digest(),filter,limit,next,c)?) } else { None };
            Ok(json!({"ok":true,"state":filter.name(),"rows":rows,"matching_records":entries.len(),
                "offset":offset,"continuation":continuation,"historical_evidence_only":true}))
        }
    }
}
/// Cursor mutations are made on a caller-owned candidate and published only
/// after this COMPLETE response and final runtime checks succeed.
pub(super) fn render(op: &str, turn: &ExcavationTurn, c: &OperationContext, mode: ExcavationMode,
    query: &QueryRequest, cursors: &mut Cursors) -> Result<String>
{
    let view = turn.inventory.as_ref();
    let payload = match &turn.outcome {
        Err(cause) => failure(cause),
        Ok(ExcavationOutcome::Inventory) => match view {
            Some(view) => listing(view,query,cursors,c)?, None=>failure(&exhausted()),
        },
        Ok(ExcavationOutcome::Observation(value)) => json!({"ok":true,"observation":observation(value)}),
        Ok(ExcavationOutcome::Plan(value)) => json!({"ok":true,"plan":plan(value),"local_review":true,
            "native_preparation_created":false,"requires_confirmation":true}),
        Ok(ExcavationOutcome::Effect { key, native_record_found }) => {
            let entry = view.and_then(|v| v.entry(key)).ok_or_else(exhausted)?;
            let mut body = json!({"ok":true,"record":record(entry),"native_record_found_this_call":native_record_found});
            if op == "fortress.explain" {
                body["plan"] = plan(entry.plan());
                body["before"] = observation(entry.plan().before());
                body["last_native_sample"] = entry.native().and_then(ExcavationRunRecord::sample).map(observation).unwrap_or(Value::Null);
            }
            body
        }
        Ok(ExcavationOutcome::PlanCancelled) => json!({"ok":true,"local_review_cancelled":true,"native_cancellation_dispatched":false}),
        Ok(ExcavationOutcome::Released) => json!({"ok":true,"session_released":true,"journal_preserved":true,
            "native_cancellation_dispatched":false,"native_quiescence_proven":false}),
    };
    let out = packet(op,payload,Some(c),Some(mode),Some(turn));
    if out.len() as u64 > RESPONSE_BYTES { return Err(exhausted()); }
    Ok(out)
}
pub(super) fn packet(op: &str, mut payload: Value, c: Option<&OperationContext>, mode: Option<ExcavationMode>,
    turn: Option<&ExcavationTurn>) -> String
{
    let verified = turn.and_then(|t| t.inventory.as_ref());
    let prior = turn.and_then(|t| t.historical_prior.as_ref());
    let retained = verified.or(prior);
    let mut active = empty_active_work();
    active["scope"] = json!("this_excavation_journal_and_local_session_only");
    active["inventory_verified"] = json!(verified.is_some());
    active["pending_absence_proven"] = json!(verified.is_some_and(|v| v.pending_count()==0)
        && turn.is_some_and(|t|t.uncertain_attempt.is_none() && t.plan.is_none()));
    if let Some(view) = retained {
        let pending = view.entries().iter().filter(|entry|entry.unresolved()).take(4).map(record).collect::<Vec<_>>();
        active["obligations"] = json!(pending);
        active["unresolved_count"] = json!(view.pending_count());
        active["omitted_obligations"] = json!(view.pending_count().saturating_sub(4));
        active["records_are_verified_history"] = json!(verified.is_some());
        payload[if verified.is_some(){"inventory"}else{"unverified_historical_prior"}] = summary(view);
    }
    if let Some(turn) = turn {
        if let Some(review) = &turn.plan { active["pending_plans"] = json!([plan(review)]); }
        if let Some(attempt) = &turn.uncertain_attempt {
            active["indeterminate_effects"] = json!([{"key":attempt.key,"plan_digest":attempt.digest.to_string(),
                "journal_presence_verified":false,"retry_commit_permitted":false}]);
        }
        payload["native_operation_attempted"] = json!(turn.native_operation_attempted);
    }
    payload["runtime_admitted"] = json!(false);
    payload["mining_causality_proven"] = json!(false);
    payload["current_pause_proven"] = json!(false);
    payload["continuous_stability_proven"] = json!(false);
    payload["global_clock_fence_verified"] = json!(false);
    let phase = match op {
        "fortress.open_session"=>AgentPhase::Bootstrap,"fortress.observe"=>AgentPhase::Orient,
        "fortress.plan"=>AgentPhase::Propose,"fortress.commit"=>AgentPhase::Commit,
        "fortress.wait"|"fortress.cancel"=>AgentPhase::Reconcile,_=>AgentPhase::Inspect,
    };
    let mut builder = AgentTurnBuilder::new(op,phase)
        .continuity(if verified.is_some(){ContinuityStatus::Stale}else{ContinuityStatus::Indeterminate},
            retained.map(anchor),Some(json!({"world_history":"unestablished","inventory_is_not_current_terrain":true})),None)
        .briefing(json!({"runtime":"unadmitted_excavation_run","bridge_protocol":"1.18","mode":mode.map(mode_name),
            "runtime_admitted":false,"mutation_admissible":false,"current_pause_proven":false}))
        .active_work(active)
        .coverage(json!({"status":"partial","complete_domains":if verified.is_some(){json!(["retained_excavation_inventory"])}else{json!([])},
            "partial_domains":["sampled_native_terrain_and_clock"],"omitted_domains":["current_world","other_controllers","continuous_stability","mining_causality","game_checkpoint"]}))
        .uncertainty(vec![uncertainty("excavation-evidence-limits","unknown",
            "Sampled floor evidence and verified historical pause do not prove current pause, mining causality or continuous safety.",
            "Recover the exact original key; absent replies or native records never authorize a new commit.",None,Value::Null)]);
    if let Some(view) = verified { builder=builder.anchor(anchor(view)); }
    if let Some(c) = c {
        payload["session_id"]=json!(c.session_id.to_string());
        if op=="fortress.open_session" {
            payload["capabilities"]=json!(c.grants.iter().map(|g|g.capability.as_str()).collect::<Vec<_>>());
        }
        let mut budget=empty_budget();
        budget["admitted"]=json!({"max_wall_millis":c.budget.max_wall_millis,"max_bytes":c.budget.max_bytes,
            "max_game_ticks":c.budget.max_game_ticks,"max_output_tokens":c.budget.max_output_tokens});
        budget["reserved_output_bytes"]=json!(RESPONSE_BYTES);
        budget["token_count_measured"]=json!(false);
        builder=builder.session_id(c.session_id.to_string()).request_id(c.request_id.to_string()).budget(budget)
            .recommendations(vec![recommendation("discover-excavation","fortress.query","Inspect retained obligations before new work.",
                "high","high","read_only","not_applicable",false,json!({"session_id":c.session_id.to_string(),
                "request":"{\"kind\":\"records\",\"state\":\"unresolved\",\"limit\":4}"}))]);
    }
    let mut spine=builder.build();
    if let Some(briefing)=spine["briefing"].as_object_mut(){briefing.remove("admission");}
    json!({"schema":"dfmcp.excavation-run-mcp/1","result":payload,"agent_turn":spine}).to_string()
}
