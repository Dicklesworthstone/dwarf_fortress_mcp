//! Durable, foreground-only coordination for the isolated job-control profile.
//! No dispatch is reachable before a synced DispatchStarted frame. Reopening
//! never converts that state, an unknown reply, or a missing native record back
//! into a dispatchable preparation. This journal is not canonical game history.

use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, Digest32, ErrorCode, FortressId, OperationContext, Result, RiskTier};
use crate::control_effect_journal::EffectJournalStorage;
use super::{JobObservation, Reader, SuspensionEffect, SuspensionPlan, SuspensionState, put_text, validate_key};
use super::rpc::{JobControlManifest, JobControlRpcClient, JobControlStream};

#[path = "journal_file.rs"]
mod journal_file;
pub use journal_file::{PrivateJobJournalFile, open_private_job_journal, open_private_job_recovery, open_private_job_reconciliation};
#[path = "discovery.rs"]
mod discovery;
pub use discovery::{JobJournalSummary, JobRecordPage};

const MAGIC: &[u8; 8] = b"DFMJJ019";
const FRAME: &[u8; 8] = b"DFMJJR19";
const FOOTER: &[u8; 8] = b"DFMJJEND";
const HEADER_BYTES: usize = 80;
const FRAME_PREFIX: usize = 52;
const MAX_BODY: usize = 2048;
const MAX_FRAME: usize = FRAME_PREFIX + MAX_BODY + 40;
const MAX_LEDGER: u64 = 64 * 1024 * 1024;
const MAX_RECORDS: usize = 4096;
const MAX_TRANSITIONS: u64 = 16_384;
const RPC_BYTES: u64 = 2 * 8192 + 262_144;

fn fail(code: ErrorCode, message: &str) -> DfmcpError { DfmcpError::new(code, message) }
fn corrupt(message: &str) -> DfmcpError { fail(ErrorCode::CorruptLedger, message) }
fn io_error(_: io::Error) -> DfmcpError { corrupt("job journal I/O or custody failed; reopen without redispatch") }
fn conflict(message: &str) -> DfmcpError { fail(ErrorCode::Conflict, message) }
fn exhausted() -> DfmcpError { fail(ErrorCode::BudgetExceeded, "job coordination exceeds its explicit work or retention budget") }

/// Same lineage domain as live_identity, applied only after the complete
/// selected-job observation has been validated. No world snapshot is invented.
pub fn job_fortress_id(observation: &JobObservation) -> FortressId {
    let mut value = b"dfmcp-live-fortress-id-v1\0".to_vec();
    value.extend_from_slice(observation.world_folder().as_bytes()); value.push(0);
    value.extend_from_slice(&observation.site_id().to_be_bytes());
    let hash = Digest32::of_bytes(&value); let mut bytes = [0; 8]; bytes.copy_from_slice(&hash.as_bytes()[..8]);
    FortressId::new(u64::from_be_bytes(bytes) | 1)
}

struct Budget { until: Instant, bytes: u64 }
impl Budget {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        let until = Instant::now().checked_add(Duration::from_millis(context.budget.max_wall_millis.min(60_000)))
            .ok_or_else(exhausted)?;
        Ok(Self { until, bytes: context.budget.max_bytes })
    }
    fn remaining(&self) -> Result<Duration> {
        self.until.checked_duration_since(Instant::now()).filter(|d| *d >= Duration::from_millis(1)).ok_or_else(exhausted)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.remaining()?; self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?; Ok(())
    }
}
fn authorize(context: &OperationContext, fortress: FortressId, write: bool) -> Result<()> {
    if context.anchor.fortress_id != fortress { return Err(fail(ErrorCode::CapabilityDenied, "job journal belongs to another fortress")); }
    // Native IDs are not fabricated canonical EntityIds. Scoped entity/map
    // grants are therefore refused; this development coordinator requires an
    // explicit fortress-wide grant. Limited-use grants are refused by core.
    context.authorize(if write { Capability::ConfigureProduction } else { Capability::Query },
        if write { RiskTier::Reversible } else { RiskTier::ReadOnly }, &[], None)
}
fn valid_manifest(manifest: &JobControlManifest, plan: &SuspensionPlan) -> Result<()> {
    if manifest.generation != plan.observation().generation()
        || [&manifest.df_version, &manifest.dfhack_version].iter().any(|s| s.is_empty() || s.len() > 128 || s.contains('\0'))
    { return Err(fail(ErrorCode::VersionMismatch, "job plan and native manifest disagree")); }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DurableJobState { Prepared = 1, DispatchStarted = 2, Indeterminate = 3, Applied = 4,
    NotApplied = 5, Refused = 6, CancelledBeforeDispatch = 7 }
impl DurableJobState {
    pub fn terminal(self) -> bool { matches!(self, Self::Applied | Self::NotApplied | Self::Refused | Self::CancelledBeforeDispatch) }
    pub fn reconciliation_required(self) -> bool { matches!(self, Self::DispatchStarted | Self::Indeterminate) }
    fn from_effect(effect: &SuspensionEffect) -> Self {
        match effect.state() { SuspensionState::Prepared => Self::Prepared, SuspensionState::Unknown => Self::Indeterminate,
            SuspensionState::Applied => Self::Applied, SuspensionState::NotApplied => Self::NotApplied, SuspensionState::Refused => Self::Refused }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableJobRecord { plan: SuspensionPlan, manifest: JobControlManifest, effect: SuspensionEffect, state: DurableJobState }
impl DurableJobRecord {
    pub fn plan(&self) -> &SuspensionPlan { &self.plan }
    pub fn manifest(&self) -> &JobControlManifest { &self.manifest }
    pub fn effect(&self) -> &SuspensionEffect { &self.effect }
    pub fn state(&self) -> DurableJobState { self.state }
}

/// Trusted injected native boundary. Implementations must not retry a commit.
pub trait JobSuspensionSource {
    fn manifest(&self) -> &JobControlManifest;
    fn fence(&mut self);
    fn prepare(&mut self, plan: &SuspensionPlan, timeout: Duration) -> Result<SuspensionEffect>;
    fn commit(&mut self, plan: &SuspensionPlan, prepared: &SuspensionEffect, timeout: Duration) -> Result<SuspensionEffect>;
    fn query(&mut self, plan: &SuspensionPlan, timeout: Duration) -> Result<Option<SuspensionEffect>>;
}
impl<S: JobControlStream> JobSuspensionSource for JobControlRpcClient<S> {
    fn manifest(&self) -> &JobControlManifest { JobControlRpcClient::manifest(self) }
    fn fence(&mut self) { JobControlRpcClient::fence(self); }
    fn prepare(&mut self, plan: &SuspensionPlan, timeout: Duration) -> Result<SuspensionEffect> {
        JobControlRpcClient::prepare(self, plan, timeout).map(|r| r.effect().clone())
    }
    fn commit(&mut self, plan: &SuspensionPlan, prepared: &SuspensionEffect, timeout: Duration) -> Result<SuspensionEffect> {
        self.commit_prepared(plan, prepared, timeout)
    }
    fn query(&mut self, plan: &SuspensionPlan, timeout: Duration) -> Result<Option<SuspensionEffect>> {
        JobControlRpcClient::query(self, plan, timeout)
    }
}

pub struct JobControlJournal<S> {
    storage: S, fortress: FortressId, id: Digest32, head: Digest32, length: u64,
    transitions: u64, records: BTreeMap<String, DurableJobRecord>, fenced: bool, read_only: bool,
}
impl<S: EffectJournalStorage> JobControlJournal<S> {
    /// `initialize_empty` is only for a newly created, exclusively owned store.
    /// Existing empty, truncated or corrupt stores are never repaired or erased.
    pub fn open(storage: S, context: &OperationContext, initialize_empty: bool) -> Result<Self> {
        Self::open_inner(storage, context, initialize_empty, false)
    }
    pub fn open_read_only(storage: S, context: &OperationContext) -> Result<Self> {
        Self::open_inner(storage, context, false, true)
    }
    fn open_inner(mut storage: S, context: &OperationContext, initialize: bool, read_only: bool) -> Result<Self> {
        let fortress = context.anchor.fortress_id; authorize(context, fortress, !read_only)?;
        let mut budget = Budget::new(context)?;
        storage.validate_identity().map_err(io_error)?;
        let mut length = storage.seek(SeekFrom::End(0)).map_err(io_error)?;
        if length > MAX_LEDGER { return Err(exhausted()); }
        budget.charge(length.max(HEADER_BYTES as u64))?;
        if length == 0 && initialize && !read_only {
            budget.charge(HEADER_BYTES as u64)?;
            let mut identity = b"dfmcp-job-journal-incarnation/1\0".to_vec();
            identity.extend_from_slice(&context.session_id.get().to_be_bytes());
            identity.extend_from_slice(&context.request_id.get().to_be_bytes());
            identity.extend_from_slice(&fortress.get().to_be_bytes());
            identity.extend_from_slice(&context.anchor.cursor.epoch.to_be_bytes());
            let mut header = MAGIC.to_vec(); header.extend_from_slice(Digest32::of_bytes(&identity).as_bytes());
            header.extend_from_slice(&fortress.get().to_be_bytes());
            let digest = Digest32::of_bytes(&header); header.extend_from_slice(digest.as_bytes());
            storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
            storage.write_all(&header).map_err(io_error)?; storage.sync().map_err(io_error)?; length = HEADER_BYTES as u64;
        }
        if length < HEADER_BYTES as u64 { return Err(corrupt("job journal header is incomplete; bytes left unchanged")); }
        storage.seek(SeekFrom::Start(0)).map_err(io_error)?;
        let mut header = [0; HEADER_BYTES]; storage.read_exact(&mut header).map_err(io_error)?;
        let mut r = Reader { remaining: &header };
        if r.take(8)? != MAGIC { return Err(corrupt("not a job-control journal")); }
        let id = r.digest()?; let named_fortress = r.u64()?; let head = r.digest()?; r.finish()?;
        if named_fortress != fortress.get() || head != Digest32::of_bytes(&header[..48]) { return Err(corrupt("job journal header identity or checksum failed")); }
        let mut out = Self { storage, fortress, id, head, length: HEADER_BYTES as u64, transitions: 0,
            records: BTreeMap::new(), fenced: false, read_only };
        while out.length < length {
            budget.remaining()?;
            if out.transitions >= MAX_TRANSITIONS || length - out.length < FRAME_PREFIX as u64 { return Err(corrupt("incomplete or excessive job journal transitions")); }
            let mut prefix = [0; FRAME_PREFIX]; out.storage.read_exact(&mut prefix).map_err(io_error)?;
            let mut r = Reader { remaining: &prefix };
            if r.take(8)? != FRAME { return Err(corrupt("job journal frame marker failed")); }
            let count = r.u32()? as usize; let number = r.u64()?; let previous = r.digest()?;
            if count > MAX_BODY || number != out.transitions + 1 || previous != out.head { return Err(corrupt("job journal frame ordering or body bound failed")); }
            let frame_length = FRAME_PREFIX + count + 40;
            if frame_length as u64 > length - out.length { return Err(corrupt("job journal tail is incomplete; no automatic truncation")); }
            let mut body = vec![0; count]; out.storage.read_exact(&mut body).map_err(io_error)?;
            let mut trailer = [0; 40]; out.storage.read_exact(&mut trailer).map_err(io_error)?;
            let mut r = Reader { remaining: &trailer }; let digest = r.digest()?;
            if r.take(8)? != FOOTER || digest != frame_hash(id, &prefix, &body) { return Err(corrupt("job journal checksum or commit footer failed")); }
            let record = decode_record(&body)?; out.validate_transition(&record)?;
            out.records.insert(record.plan.key().to_owned(), record);
            if out.records.len() > context.budget.max_entities as usize { return Err(exhausted()); }
            out.head = digest; out.transitions = number; out.length += frame_length as u64;
        }
        out.storage.validate_identity().map_err(io_error)?; budget.remaining()?; Ok(out)
    }
    pub fn poisoned(&self) -> bool { self.fenced }
    pub fn lookup(&self, key: &str, context: &OperationContext) -> Result<Option<&DurableJobRecord>> {
        self.validate_access(context)?; validate_key(key)?;
        Ok(self.records.get(key))
    }
    /// Return all unresolved work or fail the declared bound; never silently
    /// omit a dispatch that an agent needs to reconcile after transcript loss.
    pub fn unresolved(&self, context: &OperationContext) -> Result<Vec<DurableJobRecord>> {
        self.validate_access(context)?;
        let mut budget = Budget::new(context)?; let mut out = Vec::new();
        for record in self.records.values().filter(|r| !r.state.terminal()) {
            if out.len() >= context.budget.max_entities as usize { return Err(exhausted()); }
            budget.charge(MAX_BODY as u64)?; out.push(record.clone());
        }
        Ok(out)
    }
    fn writable(&self, context: &OperationContext, production: bool) -> Result<()> {
        authorize(context, self.fortress, production)?;
        if self.read_only { return Err(fail(ErrorCode::CapabilityDenied, "recovery-only job journal cannot be written")); }
        if self.fenced { return Err(corrupt("job journal fenced; reopen without redispatch")); }
        self.storage.validate_identity().map_err(io_error)
    }
    fn plan_context(&self, plan: &SuspensionPlan, context: &OperationContext) -> Result<()> {
        if job_fortress_id(plan.observation()) != self.fortress || context.anchor.tick.get() != plan.observation().tick() {
            return Err(fail(ErrorCode::StaleAnchor, "job preparation requires authority at the selected fortress and observed tick"));
        }
        Ok(())
    }
    fn reserve(&self, transitions: u64) -> Result<()> {
        if self.transitions + transitions > MAX_TRANSITIONS || self.length + transitions * MAX_FRAME as u64 > MAX_LEDGER { return Err(exhausted()); }
        Ok(())
    }
    fn matching(&self, plan: &SuspensionPlan) -> Result<DurableJobRecord> {
        let record = self.records.get(plan.key()).ok_or_else(|| conflict("unknown durable job preparation"))?;
        if &record.plan != plan { return Err(conflict("job idempotency key was already bound to another plan")); }
        Ok(record.clone())
    }
    fn validate_transition(&self, next: &DurableJobRecord) -> Result<()> {
        valid_manifest(&next.manifest, &next.plan)?;
        if job_fortress_id(next.plan.observation()) != self.fortress { return Err(corrupt("job frame crosses fortress lineage")); }
        let native = SuspensionEffect::decode(next.effect.canonical_bytes(), &next.plan)?;
        let shape = match next.state {
            DurableJobState::Prepared | DurableJobState::DispatchStarted | DurableJobState::CancelledBeforeDispatch => native.state() == SuspensionState::Prepared,
            DurableJobState::Indeterminate => matches!(native.state(), SuspensionState::Prepared | SuspensionState::Unknown),
            _ => DurableJobState::from_effect(&native) == next.state,
        };
        if !shape { return Err(corrupt("durable job state contradicts native evidence")); }
        let Some(old) = self.records.get(next.plan.key()) else {
            if self.records.len() >= MAX_RECORDS { return Err(exhausted()); }
            if matches!(next.state, DurableJobState::DispatchStarted | DurableJobState::CancelledBeforeDispatch) { return Err(corrupt("job dispatch or cancellation has no preparation")); }
            return Ok(());
        };
        if old.plan != next.plan || old.manifest != next.manifest { return Err(conflict("job key identity or software changed")); }
        if old.state.terminal() { return if old == next { Ok(()) } else { Err(corrupt("terminal job evidence was rewritten")) }; }
        if old.effect.state() == SuspensionState::Unknown && next.effect != old.effect { return Err(corrupt("native Unknown record is immutable")); }
        let allowed = match old.state {
            DurableJobState::Prepared => next.state != DurableJobState::Prepared || old == next,
            DurableJobState::DispatchStarted | DurableJobState::Indeterminate => matches!(next.state,
                DurableJobState::Indeterminate | DurableJobState::Applied | DurableJobState::NotApplied | DurableJobState::Refused),
            _ => false,
        };
        if !allowed { return Err(corrupt("job journal transition could re-enable dispatch")); } Ok(())
    }
    fn append(&mut self, record: DurableJobRecord, budget: &mut Budget) -> Result<DurableJobRecord> {
        budget.remaining()?; self.validate_transition(&record)?;
        if self.records.get(record.plan.key()) == Some(&record) { return Ok(record); }
        self.reserve(1)?; let body = encode_record(&record)?;
        let mut prefix = FRAME.to_vec(); prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
        prefix.extend_from_slice(&(self.transitions + 1).to_be_bytes()); prefix.extend_from_slice(self.head.as_bytes());
        let digest = frame_hash(self.id, &prefix, &body);
        let mut bytes = prefix; bytes.extend_from_slice(&body); bytes.extend_from_slice(digest.as_bytes()); bytes.extend_from_slice(FOOTER);
        budget.charge(bytes.len() as u64)?;
        let result = (|| {
            self.storage.validate_identity().map_err(io_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(io_error)? != self.length { return Err(corrupt("job journal extent changed outside its owner")); }
            self.storage.write_all(&bytes).map_err(io_error)?; self.storage.sync().map_err(io_error)?;
            self.storage.validate_identity().map_err(io_error)?; Ok(())
        })();
        if result.is_err() { self.fenced = true; }
        result?;
        self.head = digest; self.transitions += 1; self.length += bytes.len() as u64;
        self.records.insert(record.plan.key().to_owned(), record.clone());
        // Never publish a success when the caller's acknowledgement budget expired.
        budget.remaining()?; Ok(record)
    }
    pub fn prepare<N: JobSuspensionSource>(&mut self, source: &mut N, plan: &SuspensionPlan,
        context: &OperationContext) -> Result<DurableJobRecord>
    {
        self.writable(context, true)?; self.plan_context(plan, context)?;
        let manifest = source.manifest().clone(); valid_manifest(&manifest, plan)?;
        if self.records.contains_key(plan.key()) {
            let old = self.matching(plan)?;
            if old.manifest != manifest { return Err(fail(ErrorCode::StaleAnchor, "job source software changed")); }
            return Ok(old);
        }
        if self.records.len() >= MAX_RECORDS { return Err(exhausted()); }
        self.reserve(3)?; let mut budget = Budget::new(context)?; budget.charge(RPC_BYTES)?;
        if budget.bytes < MAX_FRAME as u64 { return Err(exhausted()); }
        let result = source.prepare(plan, budget.remaining()?);
        let effect = checked_reply(source, &manifest, plan, result)?;
        let state = DurableJobState::from_effect(&effect);
        self.append(DurableJobRecord { plan: plan.clone(), manifest, effect, state }, &mut budget)
    }
    pub fn commit<N: JobSuspensionSource>(&mut self, source: &mut N, plan: &SuspensionPlan,
        context: &OperationContext) -> Result<DurableJobRecord>
    {
        self.writable(context, true)?;
        let old = self.matching(plan)?;
        if old.state.terminal() { return Ok(old); }
        if old.state != DurableJobState::Prepared {
            return Err(fail(ErrorCode::EffectIndeterminate, "job dispatch already started; only query reconciliation is allowed").retryable(false));
        }
        self.plan_context(plan, context)?;
        if source.manifest() != &old.manifest { return Err(fail(ErrorCode::StaleAnchor, "job source incarnation or software changed")); }
        self.reserve(2)?; let mut budget = Budget::new(context)?; budget.charge(RPC_BYTES)?;
        if budget.bytes < 2 * MAX_FRAME as u64 { return Err(exhausted()); }
        let mut started = old.clone(); started.state = DurableJobState::DispatchStarted;
        self.append(started, &mut budget)?; // Sync BEFORE the only reachable native setter.
        authorize(context, self.fortress, true)?;
        let timeout = budget.remaining()?;
        let result = source.commit(plan, &old.effect, timeout);
        let checked = checked_reply(source, &old.manifest, plan, result);
        let mut next = old; next.state = DurableJobState::Indeterminate;
        if let Ok(effect) = checked {
            if effect.state() == SuspensionState::Prepared { source.fence(); }
            else { next.state = DurableJobState::from_effect(&effect); next.effect = effect; }
        }
        self.append(next, &mut budget)
    }
    /// Query authority suffices: this path can only read the bridge and append
    /// verified recovery evidence. It has no prepare or commit call edge.
    pub fn reconcile<N: JobSuspensionSource>(&mut self, source: &mut N, plan: &SuspensionPlan,
        context: &OperationContext) -> Result<DurableJobRecord>
    {
        self.writable(context, false)?;
        let old = self.matching(plan)?;
        if old.state.terminal() { return Ok(old); }
        let mut budget = Budget::new(context)?; self.reserve(1)?;
        let mut next = old.clone(); next.state = DurableJobState::Indeterminate;
        if source.manifest() == &old.manifest {
            budget.charge(RPC_BYTES)?;
            if budget.bytes < MAX_FRAME as u64 { return Err(exhausted()); }
            match source.query(plan, budget.remaining()?) {
                Ok(Some(effect)) => match checked_reply(source, &old.manifest, plan, Ok(effect)) {
                    Ok(effect) => {
                        if old.effect.state() == SuspensionState::Unknown && effect != old.effect {
                            source.fence(); return Err(corrupt("native Unknown outcome changed during recovery"));
                        }
                        if effect.state() != SuspensionState::Prepared { next.state = DurableJobState::from_effect(&effect); next.effect = effect; }
                    }
                    Err(_) => { /* Retain ambiguity, not a fabricated negative receipt. */ }
                }
                Ok(None) => { /* Absence never re-enables dispatch. */ }
                Err(_) => { source.fence(); }
            }
        }
        self.append(next, &mut budget)
    }
    pub fn cancel_before_dispatch(&mut self, plan: &SuspensionPlan, context: &OperationContext) -> Result<DurableJobRecord> {
        self.writable(context, true)?; let mut record = self.matching(plan)?;
        if record.state == DurableJobState::CancelledBeforeDispatch { return Ok(record); }
        if record.state != DurableJobState::Prepared { return Err(conflict("cannot cancel after job dispatch or native terminal evidence")); }
        record.state = DurableJobState::CancelledBeforeDispatch;
        self.append(record, &mut Budget::new(context)?)
    }
}

fn checked_reply<N: JobSuspensionSource>(source: &mut N, manifest: &JobControlManifest, plan: &SuspensionPlan,
    reply: Result<SuspensionEffect>) -> Result<SuspensionEffect>
{
    let result = (|| {
        if source.manifest() != manifest { return Err(fail(ErrorCode::StaleAnchor, "job source changed during request")); }
        let effect = reply?; SuspensionEffect::decode(effect.canonical_bytes(), plan)
    })();
    if result.is_err() { source.fence(); } result
}
fn frame_hash(id: Digest32, prefix: &[u8], body: &[u8]) -> Digest32 {
    let mut data = b"dfmcp-job-journal-frame/1\0".to_vec(); data.extend_from_slice(id.as_bytes());
    data.extend_from_slice(prefix); data.extend_from_slice(body); Digest32::of_bytes(&data)
}
fn put_blob(out: &mut Vec<u8>, value: &[u8]) { out.extend_from_slice(&(value.len() as u16).to_be_bytes()); out.extend_from_slice(value); }
fn blob<'a>(r: &mut Reader<'a>, maximum: usize) -> Result<&'a [u8]> {
    let size = usize::from(u16::from_be_bytes(r.array()?)); if size > maximum { return Err(corrupt("job journal blob exceeds its bound")); } r.take(size)
}
fn encode_record(record: &DurableJobRecord) -> Result<Vec<u8>> {
    let mut out = vec![record.state as u8]; put_text(&mut out, record.plan.key());
    put_blob(&mut out, record.plan.observation().canonical_bytes()); out.push(u8::from(record.plan.desired()));
    put_text(&mut out, &record.manifest.df_version); put_text(&mut out, &record.manifest.dfhack_version);
    put_blob(&mut out, record.effect.canonical_bytes());
    if out.len() > MAX_BODY { return Err(exhausted()); } Ok(out)
}
fn decode_record(bytes: &[u8]) -> Result<DurableJobRecord> {
    let mut r = Reader { remaining: bytes };
    let state = match r.byte()? { 1 => DurableJobState::Prepared, 2 => DurableJobState::DispatchStarted,
        3 => DurableJobState::Indeterminate, 4 => DurableJobState::Applied, 5 => DurableJobState::NotApplied,
        6 => DurableJobState::Refused, 7 => DurableJobState::CancelledBeforeDispatch,
        _ => return Err(corrupt("unknown durable job state")) };
    let key = r.text(super::MAX_IDEMPOTENCY_KEY_BYTES, false)?;
    let observation = JobObservation::decode(blob(&mut r, super::MAX_OBSERVATION_BYTES)?)?;
    let plan = SuspensionPlan::new(observation, &key, r.boolean()?)?;
    let manifest = JobControlManifest { generation: plan.observation().generation(), df_version: r.text(128, false)?, dfhack_version: r.text(128, false)? };
    let effect = SuspensionEffect::decode(blob(&mut r, super::MAX_EFFECT_BYTES)?, &plan)?; r.finish()?;
    Ok(DurableJobRecord { plan, manifest, effect, state })
}

#[cfg(test)]
#[path = "coordinator_tests.rs"]
mod tests;
