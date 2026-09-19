//! Durable coordination for isolated work-orders/1.10. This is not game history.
//!
//! DispatchStarted is synced before the sole creation call. Neither restart,
//! missing native retention, nor a different idempotency key bypasses uncertainty.
//! The native receipt proves insertion only, not approval or completed production.

use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::ops::Bound::{Excluded, Unbounded};
use std::time::{Duration, Instant};

use crate::control_effect_journal::EffectJournalStorage;
use crate::work_orders::{
    MAX_EFFECT_BYTES, MAX_OBSERVATION_BYTES, WorkOrderEffect, WorkOrderObservation,
    WorkOrderPlan, WorkOrderRecipe, WorkOrderSpec, WorkOrderState, validate_key,
};
use crate::work_orders::rpc::{WorkOrderManifest, WorkOrderRpcClient, WorkOrderStream};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, FortressId, OperationContext, Result, RiskTier,
};

mod private_file;
pub mod session;
pub use private_file::{PrivateWorkOrderFile, open_private_work_orders};

const MAGIC: &[u8; 8] = b"DFMWOJ10";
const FRAME: &[u8; 8] = b"DFMWOR10";
const FOOTER: &[u8; 8] = b"DFMWEND0";
const HEADER_BYTES: usize = 80;
const PREFIX_BYTES: usize = 52;
pub const MAX_RECORD_BYTES: usize = 19 * 1024;
const MAX_FRAME_BYTES: usize = PREFIX_BYTES + MAX_RECORD_BYTES + 40;
pub const MAX_JOURNAL_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_RECORDS: usize = 4096;
const MAX_TRANSITIONS: u64 = 16_384;
/// Request/reply frames, headers and the complete notification allowance.
pub const RPC_RESERVE_BYTES: u64 = 2 * 32 * 1024 + 256 * 1024 + 10 * 8;

fn error(code: ErrorCode, message: &str) -> DfmcpError { DfmcpError::new(code, message) }
fn corrupt(message: &str) -> DfmcpError { error(ErrorCode::CorruptLedger, message) }
fn exhausted() -> DfmcpError { error(ErrorCode::BudgetExceeded, "work-order coordination budget exhausted") }
fn uncertain() -> DfmcpError {
    error(ErrorCode::EffectIndeterminate,
        "creation may have occurred; recover this journal and query the same key, never retry insertion")
}
fn storage_error(_: io::Error) -> DfmcpError {
    corrupt("work-order journal I/O or custody failed; retain bytes and reopen for recovery")
}

pub(super) struct Allowance { until: Instant, bytes: u64 }
impl Allowance {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        let until = Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis.min(60_000)))
            .ok_or_else(exhausted)?;
        Ok(Self { until, bytes: context.budget.max_bytes })
    }
    fn remaining(&self) -> Result<Duration> {
        self.until.checked_duration_since(Instant::now())
            .filter(|d| *d >= Duration::from_millis(1)).ok_or_else(exhausted)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.remaining()?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        Ok(())
    }
}
fn authorize(context: &OperationContext, fortress: FortressId, production: bool) -> Result<()> {
    if context.anchor.fortress_id != fortress {
        return Err(error(ErrorCode::CapabilityDenied, "work-order custody belongs to another fortress"));
    }
    // Native IDs are not canonical EntityIds. Empty entity/map request scopes
    // deliberately refuse narrower and limited-use grants rather than widening them.
    context.authorize(
        if production { Capability::ConfigureProduction } else { Capability::Query },
        if production { RiskTier::Reversible } else { RiskTier::ReadOnly }, &[], None,
    )
}
fn manifest_matches(manifest: &WorkOrderManifest, plan: &WorkOrderPlan) -> Result<()> {
    if manifest.generation != plan.observation().generation()
        || [&manifest.df_version, &manifest.dfhack_version].iter()
            .any(|s| s.is_empty() || s.len() > 128 || s.contains('\0'))
    {
        return Err(error(ErrorCode::VersionMismatch, "work-order source and sealed plan disagree"));
    }
    Ok(())
}

/// Reconciliation is compiled against a boundary with no prepare/commit method.
pub trait WorkOrderQuerySource {
    fn manifest(&self) -> &WorkOrderManifest;
    fn fence(&mut self);
    fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation>;
    fn query(&mut self, plan: &WorkOrderPlan, timeout: Duration) -> Result<Option<WorkOrderEffect>>;
}
pub trait WorkOrderSource: WorkOrderQuerySource {
    fn prepare(&mut self, plan: &WorkOrderPlan, timeout: Duration) -> Result<WorkOrderEffect>;
    fn commit(&mut self, plan: &WorkOrderPlan, prepared: &WorkOrderEffect,
        timeout: Duration) -> Result<WorkOrderEffect>;
}
impl<S: WorkOrderStream> WorkOrderQuerySource for WorkOrderRpcClient<S> {
    fn manifest(&self) -> &WorkOrderManifest { WorkOrderRpcClient::manifest(self) }
    fn fence(&mut self) { WorkOrderRpcClient::fence(self); }
    fn read_orders(&mut self, timeout: Duration) -> Result<WorkOrderObservation> {
        WorkOrderRpcClient::read_orders(self, timeout)
    }
    fn query(&mut self, plan: &WorkOrderPlan, timeout: Duration) -> Result<Option<WorkOrderEffect>> {
        WorkOrderRpcClient::query(self, plan, timeout)
    }
}
impl<S: WorkOrderStream> WorkOrderSource for WorkOrderRpcClient<S> {
    fn prepare(&mut self, plan: &WorkOrderPlan, timeout: Duration) -> Result<WorkOrderEffect> {
        WorkOrderRpcClient::prepare(self, plan, timeout).map(|r| r.effect().clone())
    }
    fn commit(&mut self, plan: &WorkOrderPlan, prepared: &WorkOrderEffect,
        timeout: Duration) -> Result<WorkOrderEffect>
    {
        self.commit_prepared(plan, prepared, timeout)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JournalMode { Control, Reconcile, Offline }
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum CreationState {
    Prepared = 1, DispatchStarted = 2, Indeterminate = 3,
    Created = 4, Refused = 5, CancelledBeforeDispatch = 6,
}
impl CreationState {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Created | Self::Refused | Self::CancelledBeforeDispatch)
    }
    pub fn unresolved(self) -> bool { matches!(self, Self::DispatchStarted | Self::Indeterminate) }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared", Self::DispatchStarted => "dispatch_started",
            Self::Indeterminate => "indeterminate", Self::Created => "created",
            Self::Refused => "refused", Self::CancelledBeforeDispatch => "cancelled_before_dispatch",
        }
    }
    fn from_effect(effect: &WorkOrderEffect) -> Self {
        match effect.state() {
            WorkOrderState::Prepared => Self::Prepared, WorkOrderState::Unknown => Self::Indeterminate,
            WorkOrderState::Created => Self::Created, WorkOrderState::Refused => Self::Refused,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationRecord {
    plan: WorkOrderPlan, manifest: WorkOrderManifest, effect: WorkOrderEffect, state: CreationState,
}
impl CreationRecord {
    pub fn plan(&self) -> &WorkOrderPlan { &self.plan }
    pub fn manifest(&self) -> &WorkOrderManifest { &self.manifest }
    pub fn effect(&self) -> &WorkOrderEffect { &self.effect }
    pub fn state(&self) -> CreationState { self.state }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationSummary {
    pub fortress_id: FortressId, pub journal_id: Digest32, pub head: Digest32,
    pub records: usize, pub prepared: usize, pub unresolved: usize, pub terminal: usize,
    pub transitions: u64, pub retained_bytes: u64, pub mode: JournalMode,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreationPage {
    pub head: Digest32, pub records: Vec<CreationRecord>, pub next_after: Option<String>,
}

pub struct WorkOrderJournal<S> {
    storage: S, fortress: FortressId, id: Digest32, head: Digest32,
    length: u64, transitions: u64, records: BTreeMap<String, CreationRecord>,
    mode: JournalMode, fenced: bool,
}
impl<S: EffectJournalStorage> WorkOrderJournal<S> {
    /// `initialize` is valid only for new exclusively owned empty storage in
    /// Control mode. Existing empty/torn files are refused unchanged, never repaired.
    pub fn open(mut storage: S, context: &OperationContext,
        mode: JournalMode, initialize: bool) -> Result<Self>
    {
        let fortress = context.anchor.fortress_id;
        authorize(context, fortress, false)?;
        if mode == JournalMode::Control { authorize(context, fortress, true)?; }
        if initialize && mode != JournalMode::Control {
            return Err(error(ErrorCode::CapabilityDenied, "recovery cannot create a work-order journal"));
        }
        let mut budget = Allowance::new(context)?;
        storage.validate_identity().map_err(storage_error)?;
        let mut length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if length > MAX_JOURNAL_BYTES { return Err(exhausted()); }
        budget.charge(length.max(HEADER_BYTES as u64))?;
        if length == 0 && initialize {
            let mut identity = b"dfmcp-work-order-journal-incarnation/1\0".to_vec();
            identity.extend_from_slice(&context.session_id.get().to_be_bytes());
            identity.extend_from_slice(&context.request_id.get().to_be_bytes());
            identity.extend_from_slice(&fortress.get().to_be_bytes());
            identity.extend_from_slice(&context.anchor.cursor.epoch.to_be_bytes());
            let mut header = MAGIC.to_vec();
            header.extend_from_slice(Digest32::of_bytes(&identity).as_bytes());
            header.extend_from_slice(&fortress.get().to_be_bytes());
            let hash = Digest32::of_bytes(&header); header.extend_from_slice(hash.as_bytes());
            budget.charge(HEADER_BYTES as u64)?;
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage.write_all(&header).map_err(storage_error)?;
            storage.sync().map_err(storage_error)?;
            length = HEADER_BYTES as u64;
        }
        if length < HEADER_BYTES as u64 { return Err(corrupt("incomplete work-order journal header")); }
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut header = [0; HEADER_BYTES]; storage.read_exact(&mut header).map_err(storage_error)?;
        let mut input = Input(&header);
        if input.take(8)? != MAGIC { return Err(corrupt("not a work-orders/1.10 journal")); }
        let id = input.digest()?; let named = input.u64()?; let head = input.digest()?;
        if named != fortress.get() || head != Digest32::of_bytes(&header[..48]) {
            return Err(corrupt("work-order journal identity or header checksum mismatch"));
        }
        let mut journal = Self { storage, fortress, id, head, length: HEADER_BYTES as u64,
            transitions: 0, records: BTreeMap::new(), mode, fenced: false };
        while journal.length < length {
            budget.remaining()?;
            if journal.transitions >= MAX_TRANSITIONS || length - journal.length < PREFIX_BYTES as u64 {
                return Err(corrupt("incomplete or excessive creation transitions"));
            }
            let mut prefix = [0; PREFIX_BYTES];
            journal.storage.read_exact(&mut prefix).map_err(storage_error)?;
            let mut input = Input(&prefix);
            if input.take(8)? != FRAME { return Err(corrupt("creation frame marker mismatch")); }
            let size = input.u32()? as usize; let number = input.u64()?; let previous = input.digest()?;
            if size > MAX_RECORD_BYTES || number != journal.transitions + 1 || previous != journal.head {
                return Err(corrupt("creation frame bound or chain ordering mismatch"));
            }
            let total = PREFIX_BYTES + size + 40;
            if total as u64 > length - journal.length {
                return Err(corrupt("torn creation frame; evidence left unchanged"));
            }
            let mut body = vec![0; size]; journal.storage.read_exact(&mut body).map_err(storage_error)?;
            let mut trailer = [0; 40]; journal.storage.read_exact(&mut trailer).map_err(storage_error)?;
            let mut input = Input(&trailer); let hash = input.digest()?;
            if input.take(8)? != FOOTER || hash != frame_hash(id, &prefix, &body) {
                return Err(corrupt("creation frame checksum or footer mismatch"));
            }
            let record = decode_record(&body)?;
            journal.transition(&record)?;
            journal.records.insert(record.plan.key().to_owned(), record);
            if journal.records.len() > context.budget.max_entities as usize { return Err(exhausted()); }
            journal.head = hash; journal.transitions = number; journal.length += total as u64;
        }
        journal.validate_access(context)?; budget.remaining()?;
        Ok(journal)
    }
    pub fn fenced(&self) -> bool { self.fenced }
    pub fn mode(&self) -> JournalMode { self.mode }
    pub fn validate_access(&self, context: &OperationContext) -> Result<()> {
        authorize(context, self.fortress, false)?;
        if self.fenced { return Err(corrupt("creation journal fenced; reopen for verified recovery")); }
        self.storage.validate_identity().map_err(storage_error)
    }
    fn writable(&self, context: &OperationContext, production: bool) -> Result<()> {
        self.validate_access(context)?;
        if self.mode == JournalMode::Offline || (production && self.mode != JournalMode::Control) {
            return Err(error(ErrorCode::CapabilityDenied, "this journal mode cannot perform the requested write"));
        }
        authorize(context, self.fortress, production)
    }
    fn plan_context(&self, plan: &WorkOrderPlan, context: &OperationContext) -> Result<()> {
        if plan.observation().order_ids().len() > context.budget.max_entities as usize {
            return Err(exhausted());
        }
        if plan.observation().fortress_id() != self.fortress
            || plan.observation().tick() != context.anchor.tick.get()
            || plan.observation().witness() != context.anchor.state_hash
        { return Err(error(ErrorCode::StaleAnchor, "creation requires the exact retained queue witness and tick")); }
        Ok(())
    }
    fn no_uncertainty(&self) -> Result<()> {
        if self.records.values().any(|r| r.state.unresolved()) { return Err(uncertain()); }
        Ok(())
    }
    fn reserve(&self, frames: u64) -> Result<()> {
        if self.transitions + frames > MAX_TRANSITIONS
            || self.length + frames * MAX_FRAME_BYTES as u64 > MAX_JOURNAL_BYTES
        { return Err(exhausted()); }
        Ok(())
    }
    fn matching(&self, plan: &WorkOrderPlan) -> Result<CreationRecord> {
        let record = self.records.get(plan.key())
            .ok_or_else(|| error(ErrorCode::Conflict, "no durable creation preparation has this key"))?;
        if &record.plan != plan { return Err(error(ErrorCode::Conflict, "creation key already binds different intent")); }
        Ok(record.clone())
    }
    fn transition(&self, next: &CreationRecord) -> Result<()> {
        manifest_matches(&next.manifest, &next.plan)?;
        if next.plan.observation().fortress_id() != self.fortress {
            return Err(corrupt("creation frame crosses fortress lineage"));
        }
        let effect = WorkOrderEffect::decode(next.effect.canonical_bytes(), &next.plan)?;
        let shape = match next.state {
            CreationState::Prepared | CreationState::DispatchStarted | CreationState::CancelledBeforeDispatch =>
                effect.state() == WorkOrderState::Prepared,
            CreationState::Indeterminate => matches!(effect.state(), WorkOrderState::Prepared | WorkOrderState::Unknown),
            _ => CreationState::from_effect(&effect) == next.state,
        };
        if !shape { return Err(corrupt("creation state contradicts native evidence")); }
        let Some(old) = self.records.get(next.plan.key()) else {
            if self.records.len() >= MAX_RECORDS { return Err(exhausted()); }
            if matches!(next.state, CreationState::DispatchStarted | CreationState::CancelledBeforeDispatch) {
                return Err(corrupt("creation dispatch/cancellation has no preparation"));
            }
            self.no_uncertainty()?;
            // Initial terminal/Unknown evidence can be an exact native prepare replay.
            // It never creates another dispatch edge and must remain discoverable.
            return Ok(());
        };
        if old.plan != next.plan || old.manifest != next.manifest {
            return Err(corrupt("creation key, plan or source was rewritten"));
        }
        if old == next { return Ok(()); }
        if old.state.terminal() { return Err(corrupt("terminal creation evidence was rewritten")); }
        if old.effect.state() == WorkOrderState::Unknown && old.effect != next.effect {
            return Err(corrupt("native Unknown creation record is immutable"));
        }
        let allowed = match old.state {
            CreationState::Prepared => matches!(next.state,
                CreationState::DispatchStarted | CreationState::CancelledBeforeDispatch),
            CreationState::DispatchStarted | CreationState::Indeterminate => matches!(next.state,
                CreationState::Indeterminate | CreationState::Created | CreationState::Refused),
            _ => false,
        };
        if !allowed { return Err(corrupt("illegal creation transition could re-enable dispatch")); }
        if next.state == CreationState::DispatchStarted { self.no_uncertainty()?; }
        Ok(())
    }
    fn append(&mut self, record: CreationRecord, budget: &mut Allowance) -> Result<CreationRecord> {
        budget.remaining()?; self.transition(&record)?;
        if self.records.get(record.plan.key()) == Some(&record) { return Ok(record); }
        self.reserve(1)?;
        let body = encode_record(&record)?;
        let mut prefix = FRAME.to_vec(); prefix.extend_from_slice(&(body.len() as u32).to_be_bytes());
        prefix.extend_from_slice(&(self.transitions + 1).to_be_bytes()); prefix.extend_from_slice(self.head.as_bytes());
        let hash = frame_hash(self.id, &prefix, &body);
        let mut bytes = prefix; bytes.extend_from_slice(&body);
        bytes.extend_from_slice(hash.as_bytes()); bytes.extend_from_slice(FOOTER);
        budget.charge(bytes.len() as u64)?;
        let result = (|| {
            self.storage.validate_identity().map_err(storage_error)?;
            if self.storage.seek(SeekFrom::End(0)).map_err(storage_error)? != self.length {
                return Err(corrupt("creation journal extent changed outside its owner"));
            }
            self.storage.write_all(&bytes).map_err(storage_error)?;
            self.storage.sync().map_err(storage_error)?;
            self.storage.validate_identity().map_err(storage_error)
        })();
        if result.is_err() { self.fenced = true; }
        result?;
        self.head = hash; self.length += bytes.len() as u64; self.transitions += 1;
        self.records.insert(record.plan.key().to_owned(), record.clone());
        budget.remaining()?; // Expired acknowledgement never restores dispatch authority.
        Ok(record)
    }
    pub fn prepare<N: WorkOrderSource>(&mut self, source: &mut N, plan: &WorkOrderPlan,
        context: &OperationContext) -> Result<CreationRecord>
    {
        let mut budget = Allowance::new(context)?;
        self.writable(context, true)?;
        if self.records.contains_key(plan.key()) {
            budget.charge(MAX_RECORD_BYTES as u64)?;
            return self.matching(plan);
        }
        self.plan_context(plan, context)?; self.no_uncertainty()?;
        let manifest = source.manifest().clone(); manifest_matches(&manifest, plan)?;
        if self.records.len() >= MAX_RECORDS { return Err(exhausted()); }
        self.reserve(3)?; budget.charge(RPC_RESERVE_BYTES)?;
        if budget.bytes < MAX_FRAME_BYTES as u64 { return Err(exhausted()); }
        let reply = source.prepare(plan, budget.remaining()?);
        let effect = checked(source, &manifest, plan, reply)?;
        self.append(CreationRecord { plan: plan.clone(), manifest,
            state: CreationState::from_effect(&effect), effect }, &mut budget)
    }
    pub fn commit<N: WorkOrderSource>(&mut self, source: &mut N, plan: &WorkOrderPlan,
        context: &OperationContext) -> Result<CreationRecord>
    {
        let mut budget = Allowance::new(context)?;
        self.writable(context, true)?;
        budget.charge(MAX_RECORD_BYTES as u64)?;
        let old = self.matching(plan)?;
        if old.state.terminal() { return Ok(old); }
        if old.state != CreationState::Prepared { return Err(uncertain()); }
        self.no_uncertainty()?; self.plan_context(plan, context)?;
        if source.manifest() != &old.manifest {
            return Err(error(ErrorCode::StaleAnchor, "creation source incarnation or software changed"));
        }
        self.reserve(2)?; budget.charge(RPC_RESERVE_BYTES)?;
        if budget.bytes < 2 * MAX_FRAME_BYTES as u64 { return Err(exhausted()); }
        // Any failure from this point is an uncertain acknowledgement. Even a
        // failed sync can leave DispatchStarted bytes on disk. Never redispatch.
        let result = (|| {
            let mut started = old.clone(); started.state = CreationState::DispatchStarted;
            self.append(started, &mut budget)?;
            self.writable(context, true)?;
            let reply = source.commit(plan, &old.effect, budget.remaining()?);
            let reply = checked(source, &old.manifest, plan, reply);
            let mut next = old; next.state = CreationState::Indeterminate;
            if let Ok(effect) = reply {
                if effect.state() == WorkOrderState::Prepared { source.fence(); }
                else { next.state = CreationState::from_effect(&effect); next.effect = effect; }
            }
            self.append(next, &mut budget)
        })();
        result.map_err(|_| { source.fence(); uncertain() })
    }
    pub fn reconcile<N: WorkOrderQuerySource>(&mut self, source: &mut N, plan: &WorkOrderPlan,
        context: &OperationContext) -> Result<CreationRecord>
    {
        let mut budget = Allowance::new(context)?;
        self.writable(context, false)?;
        budget.charge(MAX_RECORD_BYTES as u64)?;
        let old = self.matching(plan)?;
        if !old.state.unresolved() { return Ok(old); }
        self.reserve(1)?; budget.charge(RPC_RESERVE_BYTES)?;
        if budget.bytes < MAX_FRAME_BYTES as u64 { return Err(exhausted()); }
        let mut next = old.clone(); next.state = CreationState::Indeterminate;
        if source.manifest() == &old.manifest {
            match source.query(plan, budget.remaining()?) {
                Ok(Some(effect)) => {
                    if let Ok(effect) = checked(source, &old.manifest, plan, Ok(effect)) {
                        if old.effect.state() == WorkOrderState::Unknown && effect != old.effect {
                            source.fence(); return Err(corrupt("native Unknown creation changed during reconciliation"));
                        }
                        if effect.state() != WorkOrderState::Prepared {
                            next.state = CreationState::from_effect(&effect); next.effect = effect;
                        }
                    }
                }
                Ok(None) => { /* Absence is not a negative receipt. */ }
                Err(_) => { source.fence(); }
            }
        }
        // Mismatched source or missing retention does not clear global uncertainty.
        self.append(next, &mut budget)
    }
    pub fn cancel(&mut self, plan: &WorkOrderPlan, context: &OperationContext) -> Result<CreationRecord> {
        let mut budget = Allowance::new(context)?;
        self.writable(context, true)?;
        let mut record = self.matching(plan)?;
        if record.state == CreationState::CancelledBeforeDispatch { return Ok(record); }
        if record.state != CreationState::Prepared {
            return Err(error(ErrorCode::Conflict, "cannot cancel creation after dispatch or native terminal evidence"));
        }
        record.state = CreationState::CancelledBeforeDispatch;
        self.append(record, &mut budget)
    }
    pub fn lookup(&self, key: &str, context: &OperationContext) -> Result<Option<&CreationRecord>> {
        self.validate_access(context)?; validate_key(key)?;
        Ok(self.records.get(key))
    }
    pub fn summary(&self, context: &OperationContext) -> Result<CreationSummary> {
        let budget = Allowance::new(context)?;
        self.validate_access(context)?;
        let (mut prepared, mut unresolved, mut terminal) = (0, 0, 0);
        for record in self.records.values() {
            budget.remaining()?;
            if record.state.terminal() { terminal += 1; }
            else if record.state.unresolved() { unresolved += 1; }
            else { prepared += 1; }
        }
        self.validate_access(context)?; budget.remaining()?;
        Ok(CreationSummary { fortress_id: self.fortress, journal_id: self.id, head: self.head,
            records: self.records.len(), prepared, unresolved, terminal, transitions: self.transitions,
            retained_bytes: self.length, mode: self.mode })
    }
    pub fn records_page(&self, head: Digest32, after: Option<&str>, limit: usize,
        context: &OperationContext) -> Result<CreationPage>
    {
        let mut budget = Allowance::new(context)?;
        self.validate_access(context)?;
        if head != self.head { return Err(error(ErrorCode::StaleAnchor, "creation journal changed; restart discovery")); }
        if limit == 0 || limit > 64 || limit > context.budget.max_entities as usize { return Err(exhausted()); }
        let start = match after {
            None => Unbounded,
            Some(key) => {
                validate_key(key)?;
                if !self.records.contains_key(key) { return Err(error(ErrorCode::Conflict, "unknown creation cursor key")); }
                Excluded(key)
            }
        };
        let mut values = self.records.range::<str, _>((start, Unbounded));
        let mut records = Vec::new();
        for (_, record) in values.by_ref().take(limit) {
            budget.charge(MAX_RECORD_BYTES as u64)?;
            records.push(record.clone());
        }
        let more = values.next().is_some();
        let next_after = if more { records.last().map(|r| r.plan.key().to_owned()) } else { None };
        self.validate_access(context)?; budget.remaining()?;
        Ok(CreationPage { head, records, next_after })
    }
}
fn checked<N: WorkOrderQuerySource>(source: &mut N, manifest: &WorkOrderManifest,
    plan: &WorkOrderPlan, reply: Result<WorkOrderEffect>) -> Result<WorkOrderEffect>
{
    let result = (|| {
        if source.manifest() != manifest { return Err(error(ErrorCode::StaleAnchor, "creation source changed during RPC")); }
        let effect = reply?;
        WorkOrderEffect::decode(effect.canonical_bytes(), plan)
    })();
    if result.is_err() { source.fence(); }
    result
}
fn frame_hash(id: Digest32, prefix: &[u8], body: &[u8]) -> Digest32 {
    let mut data = b"dfmcp-work-order-journal-frame/1\0".to_vec();
    data.extend_from_slice(id.as_bytes()); data.extend_from_slice(prefix); data.extend_from_slice(body);
    Digest32::of_bytes(&data)
}
fn put_blob(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes()); out.extend_from_slice(data);
}
fn encode_record(record: &CreationRecord) -> Result<Vec<u8>> {
    let mut data = vec![record.state as u8];
    put_blob(&mut data, record.plan.key().as_bytes());
    put_blob(&mut data, record.plan.observation().canonical_bytes());
    data.push(record.plan.spec().recipe() as u8);
    data.extend_from_slice(&record.plan.spec().amount().to_be_bytes());
    put_blob(&mut data, record.manifest.df_version.as_bytes());
    put_blob(&mut data, record.manifest.dfhack_version.as_bytes());
    put_blob(&mut data, record.effect.canonical_bytes());
    if data.len() > MAX_RECORD_BYTES { return Err(exhausted()); }
    Ok(data)
}
struct Input<'a>(&'a [u8]);
impl<'a> Input<'a> {
    fn take(&mut self, size: usize) -> Result<&'a [u8]> {
        let value = self.0.get(..size).ok_or_else(|| corrupt("incomplete creation record"))?;
        self.0 = &self.0[size..]; Ok(value)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| corrupt("creation field width mismatch"))
    }
    fn u32(&mut self) -> Result<u32> { Ok(u32::from_be_bytes(self.array()?)) }
    fn u64(&mut self) -> Result<u64> { Ok(u64::from_be_bytes(self.array()?)) }
    fn digest(&mut self) -> Result<Digest32> { Ok(Digest32::from_bytes(self.array()?)) }
    fn blob(&mut self, maximum: usize) -> Result<&'a [u8]> {
        let length = self.u32()? as usize;
        if length > maximum { return Err(corrupt("creation blob bound exceeded")); }
        self.take(length)
    }
    fn text(&mut self, maximum: usize) -> Result<String> {
        let value = std::str::from_utf8(self.blob(maximum)?)
            .map_err(|_| corrupt("invalid creation UTF-8"))?;
        if value.is_empty() || value.contains('\0') { return Err(corrupt("invalid creation text")); }
        Ok(value.to_owned())
    }
}
fn decode_record(data: &[u8]) -> Result<CreationRecord> {
    let mut input = Input(data);
    let state = match input.array::<1>()?[0] {
        1 => CreationState::Prepared, 2 => CreationState::DispatchStarted,
        3 => CreationState::Indeterminate, 4 => CreationState::Created,
        5 => CreationState::Refused, 6 => CreationState::CancelledBeforeDispatch,
        _ => return Err(corrupt("unknown creation journal state")),
    };
    let key = input.text(128)?;
    let observation = WorkOrderObservation::decode(input.blob(MAX_OBSERVATION_BYTES)?)?;
    let recipe = WorkOrderRecipe::from_code(u32::from(input.array::<1>()?[0]))?;
    let spec = WorkOrderSpec::new(recipe, input.u32()?)?;
    let plan = WorkOrderPlan::new(observation, &key, spec)?;
    let manifest = WorkOrderManifest { generation: plan.observation().generation(),
        df_version: input.text(128)?, dfhack_version: input.text(128)? };
    let effect = WorkOrderEffect::decode(input.blob(MAX_EFFECT_BYTES)?, &plan)?;
    if !input.0.is_empty() { return Err(corrupt("trailing creation journal bytes")); }
    Ok(CreationRecord { plan, manifest, effect, state })
}

#[cfg(test)]
mod tests;
