//! Closed watch requests inside the existing progress-history query argument.
//! No native source, game effect, or client-selected path exists in this dispatcher.
use dfmcp_adapter::operations_journal::JournalStorage;
use dfmcp_adapter::work_order_progress::archive::ProgressArchive;
use dfmcp_adapter::work_order_progress::watches::{WatchBook, WatchBookSummary, WatchDefinition,
    WatchEvaluation, WatchGoal, WatchRecordRef, WatchSpec, RetainedWatch, MAX_WATCH_KEY};
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, SessionId};
use serde::Deserialize;
use serde_json::{Value, json};
use super::{error, parse_digest};

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all="snake_case")]
enum Goal { Validated, Active, RemainingAtMost }
#[derive(Debug, Deserialize)]
#[serde(tag="mode", deny_unknown_fields)]
enum WireRequest {
    #[serde(rename="watch_register")]
    Register { archive_id:String, key:String, native_order_id:u32, goal:Goal,
        threshold:Option<u32>, deadline_game_tick:u64, cadence_game_ticks:u64,
        stable_samples:u8, origin_number:u64, origin_digest:String },
    #[serde(rename="watch_list")]
    List {},
    #[serde(rename="watch_status")]
    Status { archive_id:String, key:String, definition_digest:String },
    #[serde(rename="watch_cancel")]
    Cancel { archive_id:String, key:String, definition_digest:String, expected_archive_head:String },
}
#[derive(Debug)]
pub(super) enum Request {
    Register { archive:Digest32, spec:WatchSpec, origin:WatchRecordRef },
    List,
    Status { archive:Digest32, key:String, digest:Digest32 },
    Cancel { archive:Digest32, key:String, digest:Digest32, head:Digest32 },
}
fn key(raw:&str)->Result<()> {
    if raw.is_empty() || raw.len()>MAX_WATCH_KEY || !raw.bytes().all(|b|b.is_ascii_alphanumeric()||matches!(b,b'-'|b'_'|b'.')) {
        return Err(error(ErrorCode::InvalidRequest,"invalid progress watch key"));
    } Ok(())
}
impl Request {
    pub(super) fn parse(raw:&str)->Result<Self>{
        if raw.is_empty() || raw.len()>2048 || raw.contains('\0') {
            return Err(error(ErrorCode::InvalidRequest,"watch request must be 1..2048 UTF-8 bytes without NUL"));
        }
        let request:WireRequest=serde_json::from_str(raw).map_err(|_|error(ErrorCode::InvalidRequest,"invalid closed progress-watch request"))?;
        match request {
            WireRequest::Register {archive_id,key,native_order_id,goal,threshold,deadline_game_tick,
                cadence_game_ticks,stable_samples,origin_number,origin_digest}=>{
                let goal=match (goal,threshold){(Goal::Validated,None)=>WatchGoal::Validated,
                    (Goal::Active,None)=>WatchGoal::Active,(Goal::RemainingAtMost,Some(n))=>WatchGoal::RemainingAtMost(n),
                    _=>return Err(error(ErrorCode::InvalidRequest,"threshold belongs only to remaining_at_most"))};
                if !(1..=4096).contains(&origin_number){return Err(error(ErrorCode::InvalidRequest,"invalid watch origin number"));}
                Ok(Self::Register{archive:parse_digest(&archive_id)?,
                    spec:WatchSpec::new(&key,native_order_id,goal,deadline_game_tick,cadence_game_ticks,stable_samples)?,
                    origin:WatchRecordRef{number:origin_number,digest:parse_digest(&origin_digest)?}})
            }
            WireRequest::List{}=>Ok(Self::List),
            WireRequest::Status{archive_id,key:raw,definition_digest}=>{key(&raw)?;
                Ok(Self::Status{archive:parse_digest(&archive_id)?,key:raw,digest:parse_digest(&definition_digest)?})}
            WireRequest::Cancel{archive_id,key:raw,definition_digest,expected_archive_head}=>{key(&raw)?;
                Ok(Self::Cancel{archive:parse_digest(&archive_id)?,key:raw,digest:parse_digest(&definition_digest)?,head:parse_digest(&expected_archive_head)?})}
        }
    }
    fn archive(&self)->Option<Digest32>{match self{Self::List=>None,
        Self::Register{archive,..}|Self::Status{archive,..}|Self::Cancel{archive,..}=>Some(*archive)}}
}
fn reference(value:WatchRecordRef)->Value{json!({"number":value.number,"record_digest":value.digest.to_string()})}
fn definition(value:&WatchDefinition)->Value{
    let spec=value.spec();json!({"key":spec.key(),"definition_digest":value.digest().to_string(),
        "archive_id":value.archive_id().to_string(),"native_order_id":spec.native_order_id(),
        "goal":spec.goal().name(),"threshold":match spec.goal(){WatchGoal::RemainingAtMost(n)=>Some(n),_=>None},
        "deadline_game_tick":spec.deadline(),"cadence_game_ticks":spec.cadence(),"stable_samples":spec.stable_samples(),
        "origin":reference(value.origin()),"registered_tick":value.registered_tick(),"comparison_segment":value.segment()})
}
fn retained(value:&RetainedWatch)->Value{json!({"definition":definition(&value.definition),
    "cancelled_at":value.cancelled_at.map(reference),"outcome_evaluated":false})}
fn evaluation(value:&WatchEvaluation)->Value{json!({"definition":definition(value.definition()),
    "state":value.state().name(),"terminal":value.state().terminal(),"evaluated_through":reference(value.through()),
    "evaluated_records":value.evaluated_records(),"positive_samples":value.samples().iter().copied().map(reference).collect::<Vec<_>>(),
    "next_sample_tick":value.next_sample_tick(),"production_completion_proven":false,"continuous_history_proven":false,
    "historical_creation_identity_proven":false,"current_freshness_proven":false})}

pub(super) fn query<S:JournalStorage,A:JournalStorage>(book:&mut WatchBook<S>,archive:&mut ProgressArchive<A>,
    request:Request,c:&OperationContext)->Result<Value>
{
    let a=archive.summary(c)?;
    if request.archive().is_some_and(|id|id!=a.archive_id){return Err(error(ErrorCode::StaleAnchor,"watch request belongs to another archive"));}
    let result=match request {
        Request::Register{spec,origin,..}=>{
            let value=book.register(spec,origin,archive,c)?;
            json!({"watch":retained(&value),"coordination_changed_or_replayed":true,
                "next_step":{"tool":"fortress.query","session_id":c.session_id.to_string(),
                    "history":json!({"mode":"watch_status","archive_id":a.archive_id.to_string(),
                        "key":value.definition.spec().key(),"definition_digest":value.definition.digest().to_string()}).to_string()}})
        }
        Request::Cancel{key,digest,head,..}=>{
            let value=book.cancel(&key,digest,head,archive,c)?;
            json!({"watch":retained(&value),"state":"cancelled","orders_cancelled":false,"coordination_changed_or_replayed":true})
        }
        Request::List=>{
            let batch=book.evaluate(archive,c)?;
            json!({"watches":batch.results.iter().map(evaluation).collect::<Vec<_>>(),
                "complete_watch_set":true,"archive_head":batch.archive_head.to_string(),"archive_records":batch.archive_records,
                "watch_book_head":batch.book_head.to_string(),"pending":batch.results.iter().filter(|e|!e.state().terminal()).count()})
        }
        Request::Status{key,digest,..}=>{
            book.definition(&key,digest,archive,c)?;
            let batch=book.evaluate(archive,c)?;
            let value=batch.results.iter().find(|e|e.definition().spec().key()==key)
                .ok_or_else(||error(ErrorCode::CorruptLedger,"watch evaluation omitted retained intent"))?;
            json!({"watch":evaluation(value),"archive_head":batch.archive_head.to_string(),
                "archive_records":batch.archive_records,"watch_book_head":batch.book_head.to_string()})
        }
    };
    Ok(json!({"ok":true,"watch_result":result,"native_calls":0,"game_mutation_dispatched":false,
        "production_completion_proven":false,"observations_are_historical":true}))
}

/// Summaries deliberately do not infer pending absence from an unevaluated index.
/// Replaying all outcomes belongs to an explicitly budgeted watch query.
pub(super) fn attach(packet:&mut Value,book:Option<&WatchBookSummary>,id:SessionId){
    let value=match book {
        Some(b)=>json!({"configured":true,"book_id":b.book_id.to_string(),"book_head":b.head.to_string(),
            "archive_id":b.archive_id.to_string(),"retained_bytes":b.retained_bytes,"events":b.events,"read_only":b.read_only,
            "definitions":b.definitions.iter().map(|(key,digest)|json!({"key":key,"definition_digest":digest.to_string()})).collect::<Vec<_>>(),
            "definition_count":b.definitions.len(),"outcomes_evaluated":false,"pending_absence_proven":b.definitions.is_empty(),
            "discovery":{"tool":"fortress.query","session_id":id.to_string(),"history":"{\"mode\":\"watch_list\"}"}}),
        None=>json!({"configured":false,"outcomes_evaluated":false,"pending_absence_proven":false,
            "scope":"No watch book was selected; persisted monitors elsewhere were not examined."}),
    };
    packet["agent_turn"]["active_work"]["scope"]=json!("Configured progress-watch definitions are listed separately; game effects and canonical obligations in other profiles were not examined.");
    packet["agent_turn"]["active_work"]["progress_watches"]=value;
}
pub(super) fn unavailable(packet:&mut Value,configured:bool){
    packet["agent_turn"]["active_work"]["progress_watches"]=json!({"configured":configured,
        "status":"unavailable","pending_absence_proven":false,"outcomes_evaluated":false});
}

#[cfg(test)]
#[path="progress_watches_tests.rs"]
mod tests;
