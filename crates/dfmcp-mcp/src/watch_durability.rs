//! Session-owned durable watch roots. Lock order is WATCHES, then JOURNALS.
//! Checkpoints retain intent/evidence, never session grants or game authority.

#[path = "watch_checkpoint.rs"]
mod checkpoint;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{LazyLock, Mutex};
use dfmcp_adapter::operations_journal::PrivateJournalFile;
use dfmcp_core::{Capability, Digest32, ErrorCode, FortressId, GameTick, ObservationCursor,
    OperationContext, Result, RiskTier, SessionId, StateAnchor};
use dfmcp_world::WorldSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use super::{Definition, Status, Store, Watch, WATCHES, MAX_PER_SESSION, MAX_TOTAL,
    active_work, anchor, authorize, bounded, digest, failure, invalid, validate_definition, validate_handle};

const SCHEMA: &str = "dfmcp.watch-checkpoint/1";
const PROFILE: &str = "spatial/1.8";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recovery {
    pub(super) prior_watch: String,
    pub(super) prior_evidence_digest: String,
    prior_checkpoint_digest: String,
    pub(super) restart_count: u64,
    continuity_proven: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedAnchor { fortress: u64, epoch: u64, sequence: u64, tick: u64, hash: String }
impl From<StateAnchor> for SavedAnchor {
    fn from(a: StateAnchor) -> Self { Self { fortress:a.fortress_id.get(), epoch:a.cursor.epoch,
        sequence:a.cursor.sequence, tick:a.tick.0, hash:a.state_hash.to_string() } }
}
impl SavedAnchor {
    fn decode(&self) -> Result<StateAnchor> {
        Ok(StateAnchor { fortress_id:FortressId::new(self.fortress),
            cursor:ObservationCursor { epoch:self.epoch, sequence:self.sequence },
            tick:GameTick(self.tick), state_hash:parse_digest(&self.hash)? })
    }
}
fn parse_digest(value: &str) -> Result<Digest32> {
    if value.len()!=64 || !value.bytes().all(|b|b.is_ascii_digit()||(b'a'..=b'f').contains(&b)) {
        return Err(corrupt("noncanonical digest in watch checkpoint"));
    }
    let mut bytes=[0;32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte=u8::from_str_radix(&value[index*2..index*2+2],16)
            .map_err(|_|corrupt("invalid watch checkpoint digest"))?;
    }
    let digest=Digest32::from_bytes(bytes);
    if digest==Digest32::ZERO{return Err(corrupt("zero watch checkpoint evidence digest"));}
    Ok(digest)
}
fn corrupt(text: &str) -> dfmcp_core::DfmcpError { failure(ErrorCode::CorruptLedger,text) }
fn status(text: &str) -> Result<Status> {
    match text {
        "waiting"=>Ok(Status::Waiting),"candidate"=>Ok(Status::Candidate),
        "blocked_unknown"=>Ok(Status::BlockedUnknown),"satisfied"=>Ok(Status::Satisfied),
        "failed"=>Ok(Status::Failed),"expired"=>Ok(Status::Expired),
        "invalidated"=>Ok(Status::Invalidated),"cancelled"=>Ok(Status::Cancelled),
        _=>Err(corrupt("unknown durable watch status")),
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedWatch {
    handle: String,
    definition: Definition,
    created_at: SavedAnchor,
    last_seen: SavedAnchor,
    last_sample_tick: Option<u64>,
    streak: u32,
    samples: u64,
    status: String,
    evaluation: Value,
    evidence_digest: String,
    recovery: Option<Recovery>,
}
impl From<&Watch> for SavedWatch {
    fn from(w: &Watch) -> Self { Self { handle:w.handle.clone(),definition:w.definition.clone(),
        created_at:w.created_at.into(),last_seen:w.last_seen.into(),last_sample_tick:w.last_sample_tick,
        streak:w.streak,samples:w.samples,status:w.status.text().to_owned(),evaluation:w.evaluation.clone(),
        evidence_digest:w.evidence_digest.to_string(),recovery:w.recovery.clone() } }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedSet { schema: String, profile: String, checkpoint_anchor: SavedAnchor, watches: Vec<SavedWatch> }
impl SavedSet {
    fn capture(store: &Store, context: &OperationContext) -> Self {
        Self { schema:SCHEMA.to_owned(), profile:PROFILE.to_owned(), checkpoint_anchor:context.anchor.into(),
            watches:store.entries.iter().filter(|((session,_),_)|*session==context.session_id)
                .map(|(_,w)|SavedWatch::from(w)).collect() }
    }
}

/// Every stored anchor must be an exact retained observation, not merely a
/// matching tick, fortress name, source generation, or caller-supplied hash.
fn decode_saved(bytes: &[u8], anchors: &BTreeMap<StateAnchor,usize>) -> Result<SavedSet> {
    if bytes.len()>checkpoint::MAX_PAYLOAD{return Err(bounded("watch checkpoint exceeds its byte limit"));}
    let set:SavedSet=serde_json::from_slice(bytes).map_err(|_|corrupt("invalid watch checkpoint JSON"))?;
    if set.schema!=SCHEMA||set.profile!=PROFILE||set.watches.len()>MAX_PER_SESSION {
        return Err(corrupt("watch checkpoint schema/profile/count differs"));
    }
    let current=set.checkpoint_anchor.decode()?;
    let checkpoint_index=*anchors.get(&current).ok_or_else(||corrupt("watch checkpoint anchor is absent from its observation archive"))?;
    let mut handles=BTreeSet::new();let mut keys=BTreeSet::new();let mut previous=None;
    for w in &set.watches {
        validate_handle(&w.handle)?;validate_definition(&w.definition)?;parse_digest(&w.evidence_digest)?;
        let state=status(&w.status)?;let created=w.created_at.decode()?;let last=w.last_seen.decode()?;
        let created_index=*anchors.get(&created).ok_or_else(||corrupt("watch creation observation is not retained"))?;
        let last_index=*anchors.get(&last).ok_or_else(||corrupt("watch evidence observation is not retained"))?;
        if created.fortress_id!=current.fortress_id||last.fortress_id!=current.fortress_id
            ||created_index>last_index||last_index>checkpoint_index
            ||!handles.insert(w.handle.as_str())||!keys.insert(w.definition.key.as_str())
            ||previous.is_some_and(|p:&str|p>=w.handle.as_str())
            ||w.streak>w.definition.stable_observations||u64::from(w.streak)>w.samples
            ||(!state.terminal()&&w.streak>=w.definition.stable_observations)
            ||(state==Status::Satisfied&&w.streak!=w.definition.stable_observations)
            ||(!state.terminal()&&w.last_sample_tick.is_some_and(|t|t>last.tick.0)) {
            return Err(corrupt("inconsistent durable watch identity, order, evidence or counters"));
        }
        if let Some(recovery)=&w.recovery {
            validate_handle(&recovery.prior_watch)?;parse_digest(&recovery.prior_evidence_digest)?;
            parse_digest(&recovery.prior_checkpoint_digest)?;
            if recovery.restart_count==0||recovery.continuity_proven {
                return Err(corrupt("invalid watch recovery continuity claim"));
            }
        }
        previous=Some(w.handle.as_str());
    }
    Ok(set)
}

struct Entry {
    journal: checkpoint::Journal<PrivateJournalFile>,
    saved: Option<SavedSet>,
    fortress: FortressId,
    observation_journal: Digest32,
}
static JOURNALS: LazyLock<Mutex<BTreeMap<SessionId,Entry>>> = LazyLock::new(||Mutex::new(BTreeMap::new()));
fn journals() -> Result<std::sync::MutexGuard<'static,BTreeMap<SessionId,Entry>>> {
    JOURNALS.lock().map_err(|_|failure(ErrorCode::InternalInvariantViolation,"watch journal registry poisoned"))
}
fn binding(archive: Digest32, fortress: FortressId) -> Digest32 {
    let mut bytes=b"dfmcp-watch-observation-binding/1\0spatial/1.8\0".to_vec();
    bytes.extend_from_slice(archive.as_bytes());bytes.extend_from_slice(&fortress.get().to_be_bytes());
    Digest32::of_bytes(&bytes)
}

fn write_candidate<F>(entry: &mut Entry, store: &Store, context: &OperationContext,
    mut value: Value, publish: F) -> Result<String>
where F:FnOnce(Value)->Result<String> {
    context.authorize(Capability::Query,RiskTier::ReadOnly,&[],None)?;
    if context.anchor.fortress_id!=entry.fortress{return Err(corrupt("watch request names another fortress"));}
    entry.journal.verify(context)?;
    let candidate=SavedSet::capture(store,context);
    let changed=entry.saved.as_ref().is_none_or(|old|old.watches!=candidate.watches);
    let pending=if changed {
        let bytes=serde_json::to_vec(&candidate).map_err(|_|corrupt("watch state cannot be encoded"))?;
        Some(entry.journal.stage(bytes,context)?)
    }else{None};
    value["durable"]=json!(true);
    value["watch_persistence"]=json!({"schema":SCHEMA,"profile":PROFILE,
        "observation_journal_id":entry.observation_journal.to_string(),
        "journal_id":entry.journal.id().to_string(),
        "checkpoint":pending.as_ref().map_or(entry.journal.count(),checkpoint::Pending::number),
        "head":pending.as_ref().map_or(entry.journal.head(),checkpoint::Pending::head).to_string(),
        "retained_bytes":pending.as_ref().map_or(entry.journal.retained_bytes(),checkpoint::Pending::retained_bytes),
        "checkpoint_changed":changed,"sync_before_acknowledgement":true,
        "restart_resets_stability":true,"game_effect":"none"});
    // Rendering is pure and may fail. Never persist an invisible registration,
    // cancellation, release, or sample merely because its JSON did not fit.
    let encoded=publish(value)?;
    if let Some(pending)=pending {
        entry.journal.commit(pending,context)?;
        entry.saved=Some(candidate);
    }
    Ok(encoded)
}

pub(super) fn publish<F>(store: &Store, context: &OperationContext, value: Value, publish: F) -> Result<String>
where F:FnOnce(Value)->Result<String> {
    let mut registry=journals()?;
    match registry.get_mut(&context.session_id) {
        Some(entry)=>write_candidate(entry,store,context,value,publish),
        None=>publish(value),
    }
}

/// Dropping a session removes only in-memory ownership and releases the file
/// lock. It never cancels, releases, or deletes the durable monitoring intent.
pub(crate) struct WatchJournalGuard { session: SessionId }
impl Drop for WatchJournalGuard {
    fn drop(&mut self) {
        if let Ok(mut store)=WATCHES.lock(){store.entries.retain(|(id,_),_|*id!=self.session);}
        if let Ok(mut registry)=JOURNALS.lock(){registry.remove(&self.session);}
    }
}

fn recover_watch(saved: &SavedWatch, context: &OperationContext, checkpoint: Digest32,
    serial: u64) -> Result<Watch> {
    let prior=parse_digest(&saved.evidence_digest)?;
    let identity=digest(&json!({"domain":"dfmcp-recovered-watch/1","session":context.session_id.to_string(),
        "serial":serial,"prior_watch":saved.handle,"checkpoint":checkpoint.to_string()}))?;
    let mut watch=Watch {handle:format!("watch:{identity}"),definition:saved.definition.clone(),
        created_at:saved.created_at.decode()?,last_seen:saved.last_seen.decode()?,last_sample_tick:saved.last_sample_tick,
        streak:saved.streak,samples:saved.samples,status:status(&saved.status)?,
        evaluation:saved.evaluation.clone(),evidence_digest:prior,
        recovery:Some(Recovery{prior_watch:saved.handle.clone(),prior_evidence_digest:saved.evidence_digest.clone(),
            prior_checkpoint_digest:checkpoint.to_string(),continuity_proven:false,
            restart_count:saved.recovery.as_ref().map_or(0,|r|r.restart_count).checked_add(1)
                .ok_or_else(||bounded("watch restart count exhausted"))?})};
    if !watch.status.terminal() {
        let previous=watch.last_seen;
        let incompatible=watch.created_at.fortress_id!=context.anchor.fortress_id
            ||watch.created_at.cursor.epoch!=context.anchor.cursor.epoch
            ||context.anchor.tick<previous.tick||context.anchor.cursor.sequence<previous.cursor.sequence
            ||(context.anchor.cursor==previous.cursor&&context.anchor!=previous);
        watch.status=if incompatible{Status::Invalidated}
            else if context.anchor.tick.0>=watch.definition.deadline_tick{Status::Expired}
            else{Status::BlockedUnknown};
        watch.streak=0;watch.last_sample_tick=None;watch.last_seen=context.anchor;
        watch.evaluation=json!({"reason":if incompatible{"restart_observation_epoch_or_identity_changed"}
            else if watch.status==Status::Expired{"deadline_passed_during_restart"}
            else{"restart_gap_requires_fresh_observation"},"previous_anchor":anchor(previous),
            "continuous_between_observations":false,"sample_due":false,
            "recovery":watch.recovery});
    }else{
        // Keep original terminal observations/counters; rebind only the handle.
        watch.evaluation["recovery"]=json!(watch.recovery);
    }
    watch.seal()?;
    Ok(watch)
}

/// Called only by the spatial/1.8 runtime after its observation archive has
/// replayed and synced the fresh capture. No arbitrary path/profile is an MCP input.
pub(crate) fn attach<F>(snapshot: &WorldSnapshot, context: &OperationContext, path: &Path,
    archive: Digest32, observations: &[StateAnchor], mut value: Value, publish: F)
    -> Result<(String,WatchJournalGuard)>
where F:FnOnce(Value)->Result<String> {
    authorize(snapshot,context)?;
    context.authorize(Capability::Observe,RiskTier::ReadOnly,&[],None)?;
    if archive==Digest32::ZERO||observations.is_empty()||observations.len()>4096
        ||observations.last()!=Some(&context.anchor) {
        return Err(corrupt("durable watches require the exact current spatial/1.8 observation archive"));
    }
    let anchors:BTreeMap<_,_>=observations.iter().copied().enumerate().map(|(i,a)|(a,i)).collect();
    if anchors.len()!=observations.len()||observations.iter().any(|a|a.fortress_id!=context.anchor.fortress_id) {
        return Err(corrupt("watch observation archive has duplicate or foreign anchors"));
    }
    let mut store=super::lock(&WATCHES)?;
    let mut registry=journals()?;
    if registry.contains_key(&context.session_id)||store.entries.keys().any(|(id,_)|*id==context.session_id) {
        return Err(failure(ErrorCode::Conflict,"attach durable watches before creating session-local watches"));
    }
    let (file,created)=PrivateJournalFile::open(path,context)?;
    let mut saved=None;
    let journal=checkpoint::Journal::open(file,context,binding(archive,context.anchor.fortress_id),created,
        |bytes|{saved=Some(decode_saved(bytes,&anchors)?);Ok(())})?;
    let restored=saved.as_ref().map_or(0,|s|s.watches.len());
    if store.entries.len().saturating_add(restored)>MAX_TOTAL{return Err(bounded("global watch retention full"));}
    let mut candidate=Store{serial:store.serial,entries:store.entries.clone()};
    if let Some(saved)=&saved {
        for old in &saved.watches {
            candidate.serial=candidate.serial.checked_add(1).ok_or_else(||bounded("watch identity space exhausted"))?;
            let recovered=recover_watch(old,context,journal.head(),candidate.serial)?;
            candidate.entries.insert((context.session_id,recovered.handle.clone()),recovered);
        }
    }
    let mut entry=Entry{journal,saved,fortress:context.anchor.fortress_id,observation_journal:archive};
    value["watch_recovery"]=json!({"restored":restored,"stability_reset":true,
        "old_handles_valid":false,"continuous_during_downtime":false,
        "discovery":{"tool":"fortress.query","arguments":{"session_id":context.session_id.to_string(),
            "query":{"schema":"dfmcp.query/1","query":{"kind":"watches"}}}}});
    value["_condition_watch_work"]=json!(active_work(&candidate,context));
    let encoded=write_candidate(&mut entry,&candidate,context,value,|value|super::publish_plain(context,value,publish))?;
    *store=candidate;
    registry.insert(context.session_id,entry);
    Ok((encoded,WatchJournalGuard{session:context.session_id}))
}

#[cfg(all(test,unix))]
#[path = "watch_durability_tests.rs"]
mod tests;
