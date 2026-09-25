//! Append-only excavation-run coordination over the existing effect-storage seam.
//!
//! Only `start` can create a one-use dispatch permit, after synced intent,
//! preparation and dispatch frames. Reopening never recreates such a permit.
//! Storage must own exclusive custody and sync both file and directory. This
//! module supplies no filesystem path, global lease, Cx region or native socket.
use super::*;
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{Capability, GameTick, OperationContext, RiskTier};
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

const MAGIC: &[u8; 8] = b"DFMEJ018";
const DOMAIN: &[u8] = b"dfmcp-excavation-coordinator/1";
pub const MAX_JOURNAL_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_ENTRIES: usize = 256;
pub const MAX_FRAMES: u32 = 2048;
const MAX_PAYLOAD: usize = 4096;
const FRAME_OVERHEAD: usize = 72;
const RPC_RESERVE: u64 = 272 * 1024;

fn corrupt() -> dfmcp_core::DfmcpError {
    error(ErrorCode::CorruptLedger, "excavation journal changed or is incomplete; preserve evidence")
}
fn uncertain() -> dfmcp_core::DfmcpError {
    error(ErrorCode::EffectIndeterminate, "excavation outcome unknown; query or cancel, never recommit")
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(ErrorCode::BudgetExceeded, "excavation operation or retention allowance exhausted")
}
fn denied() -> dfmcp_core::DfmcpError {
    error(ErrorCode::CapabilityDenied, "excavation authority or source binding differs")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcavationBinding {
    endpoint: SocketAddr,
    generation: u64,
    fortress: FortressIdentity,
    dimensions: [u32; 3],
    df_version: String,
    dfhack_version: String,
}
impl ExcavationBinding {
    pub fn new(endpoint: SocketAddr, df: &str, dfhack: &str, capture: &ExcavationCapture) -> Result<Self> {
        require(endpoint.ip().is_loopback() && endpoint.port() != 0, "not a loopback endpoint")?;
        Ok(Self { endpoint, generation: capture.generation(), fortress: capture.fortress().clone(),
            dimensions: capture.dimensions(), df_version: text(df.as_bytes(), 128)?,
            dfhack_version: text(dfhack.as_bytes(), 128)? })
    }
    pub fn endpoint(&self) -> SocketAddr { self.endpoint }
    pub fn generation(&self) -> u64 { self.generation }
    pub fn fortress(&self) -> &FortressIdentity { &self.fortress }
    pub fn dimensions(&self) -> [u32; 3] { self.dimensions }
    pub fn df_version(&self) -> &str { &self.df_version }
    pub fn dfhack_version(&self) -> &str { &self.dfhack_version }
    /// Handshake generation for a recovery-only source. Folder/site/map remain
    /// the journal's EXPECTED historical scope, not a new observation or grant.
    pub(super) fn recovery_generation(&self, generation: u64) -> Result<Self> {
        require(generation >= self.generation && generation < u64::MAX,
            "excavation recovery generation regressed")?;
        let mut binding = self.clone(); binding.generation = generation;
        Ok(binding)
    }
    fn capture_matches(&self, capture: &ExcavationCapture) -> bool {
        self.generation == capture.generation() && &self.fortress == capture.fortress()
            && self.dimensions == capture.dimensions()
    }
    fn source_matches(&self, current: &Self, exact: bool) -> Result<()> {
        if self.endpoint != current.endpoint || self.fortress != current.fortress
            || self.dimensions != current.dimensions || self.df_version != current.df_version
            || self.dfhack_version != current.dfhack_version || current.generation < self.generation
            || (exact && current.generation != self.generation)
        { return Err(denied()); }
        Ok(())
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_field(&mut out, self.endpoint.to_string().as_bytes());
        out.extend_from_slice(&self.generation.to_be_bytes());
        out.extend_from_slice(&self.fortress.site().to_be_bytes());
        put_field(&mut out, self.fortress.folder().as_bytes());
        for n in self.dimensions { out.extend_from_slice(&n.to_be_bytes()); }
        put_field(&mut out, self.df_version.as_bytes());
        put_field(&mut out, self.dfhack_version.as_bytes());
        out
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        let endpoint = text(field(&mut r, 128)?, 128)?.parse::<SocketAddr>().map_err(|_| corrupt())?;
        let generation = r.u64()?;
        let site = r.u32()?;
        let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, site)?;
        let dimensions = [r.u32()?, r.u32()?, r.u32()?];
        let df_version = text(field(&mut r, 128)?, 128)?;
        let dfhack_version = text(field(&mut r, 128)?, 128)?;
        r.finish()?;
        require(endpoint.ip().is_loopback() && endpoint.port() != 0 && generation > 0
            && generation < u64::MAX && dimensions.iter().all(|n| (1..=32768).contains(n)),
            "invalid excavation binding")?;
        let out = Self { endpoint, generation, fortress, dimensions, df_version, dfhack_version };
        require(out.encode() == bytes, "noncanonical excavation binding")?;
        Ok(out)
    }
}

/// Cannot be constructed, cloned or reconstructed from a native receipt by a caller.
/// This is a durable-dispatch ordering proof, not a clock lease or capability.
pub struct ExcavationDispatch<'a> {
    plan: &'a ExcavationRunPlan,
    journal_head: Digest32,
}
impl ExcavationDispatch<'_> {
    pub fn plan(&self) -> &ExcavationRunPlan { self.plan }
    pub fn journal_head(&self) -> Digest32 { self.journal_head }
}

/// The effect shell must enforce current credentials, context cancellation and
/// the supplied shrinking timeout at the actual send boundary. No implicit retry.
/// Its implementation is owned by the caller's supervised runtime region.
pub trait ExcavationRunSource {
    fn binding(&self) -> &ExcavationBinding;
    fn fence(&mut self);
    fn observe(&mut self, region: ExcavationRegion, context: &OperationContext,
        timeout: Duration) -> Result<ExcavationCapture>;
    fn prepare(&mut self, plan: &ExcavationRunPlan, context: &OperationContext,
        timeout: Duration) -> Result<ExcavationRunRecord>;
    fn commit(&mut self, permit: ExcavationDispatch<'_>, context: &OperationContext,
        timeout: Duration) -> Result<ExcavationRunRecord>;
    fn query(&mut self, plan: &ExcavationRunPlan, context: &OperationContext,
        timeout: Duration) -> Result<Option<ExcavationRunRecord>>;
    fn cancel(&mut self, plan: &ExcavationRunPlan, context: &OperationContext,
        timeout: Duration) -> Result<ExcavationRunRecord>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExcavationEntry {
    plan: ExcavationRunPlan,
    prepared: bool,
    dispatch_started: bool,
    cancel_requested: bool,
    native: Option<ExcavationRunRecord>,
}
impl ExcavationEntry {
    pub fn plan(&self) -> &ExcavationRunPlan { &self.plan }
    pub fn native(&self) -> Option<&ExcavationRunRecord> { self.native.as_ref() }
    pub fn dispatch_started(&self) -> bool { self.dispatch_started }
    pub fn cancel_requested(&self) -> bool { self.cancel_requested }
    pub fn unresolved(&self) -> bool { !self.native.as_ref().is_some_and(ExcavationRunRecord::resolved) }
}

struct Work<'a> { context: &'a OperationContext, deadline: Instant, bytes: u64 }
impl<'a> Work<'a> {
    fn new(context: &'a OperationContext) -> Result<Self> {
        context.budget.validate()?;
        if context.budget.max_wall_millis > 60_000 { return Err(exhausted()); }
        let deadline = Instant::now().checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok(Self { context, deadline, bytes: context.budget.max_bytes })
    }
    fn remaining(&self) -> Result<Duration> {
        if self.context.cancellation_requested {
            return Err(error(ErrorCode::CancellationRequested, "excavation operation cancelled"));
        }
        let remaining = self.deadline.checked_duration_since(Instant::now()).ok_or_else(exhausted)?;
        if remaining < Duration::from_millis(1) { return Err(exhausted()); }
        Ok(remaining)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.remaining()?;
        self.bytes = self.bytes.checked_sub(bytes).ok_or_else(exhausted)?;
        Ok(())
    }
}
fn authorize(context: &OperationContext, binding: &ExcavationBinding, clock: bool) -> Result<()> {
    if context.anchor.fortress_id != binding.fortress.fortress_id() { return Err(denied()); }
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if clock { context.authorize(Capability::ControlClock, RiskTier::Guarded, &[], None)?; }
    Ok(())
}
pub(super) fn authorize_start(context: &OperationContext, binding: &ExcavationBinding,
    plan: &ExcavationRunPlan) -> Result<()>
{
    authorize(context, binding, true)?;
    context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
    if context.anchor.tick.get() != plan.before().tick() || !binding.capture_matches(plan.before()) {
        return Err(error(ErrorCode::StaleAnchor, "excavation confirmation anchor changed"));
    }
    if context.budget.max_actions < 1 || context.budget.max_entities < plan.before().region().cell_count() as u32
        || context.budget.max_game_ticks < u64::from(plan.spec().clock().game_ticks())
        || context.budget.max_wall_millis < u64::from(plan.spec().clock().wall_ms())
    { return Err(exhausted()); }
    let mut horizon = context.clone();
    horizon.anchor.tick = GameTick(plan.before().tick() + u64::from(plan.spec().clock().game_ticks()));
    authorize(&horizon, binding, true)
}

/// Bounded, no-repair storage. `sync` on the supplied storage must sync its
/// containing directory too. No mode here upgrades an untrusted filesystem.
pub struct ExcavationCoordinator<S> {
    storage: S,
    binding: ExcavationBinding,
    bytes: Vec<u8>,
    head: Digest32,
    frames: u32,
    entries: BTreeMap<String, ExcavationEntry>,
    fenced: bool,
}
fn read_storage<S: EffectJournalStorage>(storage: &mut S, work: &mut Work<'_>) -> Result<Vec<u8>> {
    work.remaining()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let len = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    if len > MAX_JOURNAL_BYTES as u64 { return Err(corrupt()); }
    work.charge(len)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut bytes = vec![0; len as usize];
    storage.read_exact(&mut bytes).map_err(|_| corrupt())?;
    if storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? != len { return Err(corrupt()); }
    storage.validate_identity().map_err(|_| corrupt())?;
    work.remaining()?;
    Ok(bytes)
}
fn frame(sequence: u32, head: Digest32, payload: &[u8]) -> Result<Vec<u8>> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD || sequence >= MAX_FRAMES { return Err(exhausted()); }
    let mut out = (payload.len() as u32).to_be_bytes().to_vec();
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(head.as_bytes());
    out.extend_from_slice(payload);
    let digest = hash(DOMAIN, &out);
    out.extend_from_slice(digest.as_bytes());
    Ok(out)
}
fn replay_event(entries: &mut BTreeMap<String, ExcavationEntry>, binding: &ExcavationBinding,
    payload: &[u8]) -> Result<()>
{
    let mut r = Reader(payload);
    let tag = r.byte()?;
    if tag == 1 {
        let plan = ExcavationRunPlan::decode(field(&mut r, MAX_PLAN_BYTES)?)?;
        r.finish()?;
        if entries.len() >= MAX_ENTRIES || entries.contains_key(plan.key())
            || entries.values().any(ExcavationEntry::unresolved) || !binding.capture_matches(plan.before())
        { return Err(corrupt()); }
        entries.insert(plan.key().to_owned(), ExcavationEntry { plan, prepared: false,
            dispatch_started: false, cancel_requested: false, native: None });
        return Ok(());
    }
    let key = text(field(&mut r, 128)?, 128)?;
    let entry = entries.get_mut(&key).ok_or_else(corrupt)?;
    if entry.native.as_ref().is_some_and(ExcavationRunRecord::terminal) { return Err(corrupt()); }
    match tag {
        2 | 4 => {
            let record = ExcavationRunRecord::decode(field(&mut r, MAX_RECORD_BYTES)?)?;
            if record.plan() != &entry.plan { return Err(corrupt()); }
            if let Some(old) = &entry.native { old.validate_successor(&record)?; }
            if tag == 2 {
                if entry.prepared || entry.native.is_some() || entry.dispatch_started || entry.cancel_requested
                    || record.phase() != RunPhase::Prepared { return Err(corrupt()); }
                entry.prepared = true;
            }
            entry.native = Some(record);
        }
        3 => {
            if !entry.prepared || entry.dispatch_started || entry.cancel_requested
                || !entry.native.as_ref().is_some_and(|n| n.phase() == RunPhase::Prepared)
                || r.take(32)? != entry.plan.digest().as_bytes()
            { return Err(corrupt()); }
            entry.dispatch_started = true;
        }
        5 => {
            if entry.cancel_requested { return Err(corrupt()); }
            entry.cancel_requested = true;
        }
        _ => return Err(corrupt()),
    }
    r.finish()
}
impl<S: EffectJournalStorage> ExcavationCoordinator<S> {
    pub fn create(mut storage: S, binding: ExcavationBinding, context: &OperationContext) -> Result<Self> {
        authorize(context, &binding, true)?;
        let mut work = Work::new(context)?;
        if !read_storage(&mut storage, &mut work)?.is_empty() { return Err(corrupt()); }
        let mut payload = vec![0]; payload.extend_from_slice(&binding.encode());
        let header = frame(0, Digest32::ZERO, &payload)?;
        let mut bytes = MAGIC.to_vec(); bytes.extend_from_slice(&header);
        work.charge(bytes.len() as u64)?;
        authorize(context, &binding, true)?;
        storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
        storage.write_all(&bytes).map_err(|_| corrupt())?;
        storage.flush().map_err(|_| corrupt())?;
        storage.sync().map_err(|_| corrupt())?;
        if read_storage(&mut storage, &mut work)? != bytes { return Err(corrupt()); }
        let head = Digest32::from_bytes(header[header.len() - 32..].try_into().map_err(|_| corrupt())?);
        Ok(Self { storage, binding, bytes, head, frames: 1, entries: BTreeMap::new(), fenced: false })
    }
    /// A missing, empty, partial or corrupt journal is never initialized/repaired.
    pub fn open(mut storage: S, expected: &FortressIdentity, context: &OperationContext) -> Result<Self> {
        if context.anchor.fortress_id != expected.fortress_id() { return Err(denied()); }
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let mut work = Work::new(context)?;
        let bytes = read_storage(&mut storage, &mut work)?;
        let mut r = Reader(&bytes);
        require(r.take(8)? == MAGIC, "not an excavation-run Rust journal")?;
        let mut head = Digest32::ZERO;
        let mut frames = 0;
        let mut binding = None;
        let mut entries = BTreeMap::new();
        while !r.0.is_empty() {
            work.remaining()?;
            let n = r.u32()? as usize;
            if n == 0 || n > MAX_PAYLOAD || frames >= MAX_FRAMES || r.u32()? != frames
                || r.take(32)? != head.as_bytes() { return Err(corrupt()); }
            let payload = r.take(n)?;
            let digest = Digest32::from_bytes(r.array()?);
            let canonical = frame(frames, head, payload)?;
            if canonical[canonical.len() - 32..] != digest.as_bytes()[..] { return Err(corrupt()); }
            if frames == 0 {
                if payload[0] != 0 { return Err(corrupt()); }
                let decoded = ExcavationBinding::decode(&payload[1..])?;
                if &decoded.fortress != expected { return Err(denied()); }
                binding = Some(decoded);
            } else {
                replay_event(&mut entries, binding.as_ref().ok_or_else(corrupt)?, payload)?;
            }
            head = digest; frames += 1;
        }
        let binding = binding.ok_or_else(corrupt)?;
        Ok(Self { storage, binding, bytes, head, frames, entries, fenced: false })
    }
    pub fn binding(&self) -> &ExcavationBinding { &self.binding }
    pub fn entry(&self, key: &str) -> Option<&ExcavationEntry> { self.entries.get(key) }
    pub fn entries(&self) -> impl Iterator<Item = &ExcavationEntry> { self.entries.values() }
    pub fn pending_count(&self) -> usize { self.entries.values().filter(|e| e.unresolved()).count() }
    pub fn is_fenced(&self) -> bool { self.fenced }
    fn current(&mut self, work: &mut Work<'_>) -> Result<()> {
        if self.fenced { return Err(corrupt()); }
        let result = read_storage(&mut self.storage, work);
        match result {
            Ok(bytes) if bytes == self.bytes => Ok(()),
            _ => { self.fenced = true; Err(corrupt()) }
        }
    }
    fn publish(&mut self, payload: &[u8], work: &mut Work<'_>) -> Result<()> {
        self.current(work)?;
        let mut entries = self.entries.clone();
        replay_event(&mut entries, &self.binding, payload)?;
        let appended = frame(self.frames, self.head, payload)?;
        if self.bytes.len() + appended.len() > MAX_JOURNAL_BYTES { return Err(exhausted()); }
        work.charge(appended.len() as u64)?;
        let mut next = self.bytes.clone(); next.extend_from_slice(&appended);
        // Fence BEFORE the first write; no failed publication can remain usable.
        self.fenced = true;
        self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
        self.storage.write_all(&appended).map_err(|_| corrupt())?;
        self.storage.flush().map_err(|_| corrupt())?;
        self.storage.sync().map_err(|_| corrupt())?;
        if read_storage(&mut self.storage, work)? != next { return Err(corrupt()); }
        self.head = Digest32::from_bytes(appended[appended.len() - 32..].try_into().map_err(|_| corrupt())?);
        self.frames += 1; self.entries = entries; self.bytes = next; self.fenced = false;
        Ok(())
    }
    fn boundary<N: ExcavationRunSource>(&mut self, source: &N, context: &OperationContext,
        clock: bool, exact: bool, work: &mut Work<'_>) -> Result<Duration>
    {
        authorize(context, &self.binding, clock)?;
        self.binding.source_matches(source.binding(), exact)?;
        self.current(work)?;
        work.charge(RPC_RESERVE)?;
        work.remaining()
    }
    fn evidence(&mut self, key: &str, record: &ExcavationRunRecord, work: &mut Work<'_>) -> Result<()> {
        self.current(work)?;
        let entry = self.entries.get(key).ok_or_else(corrupt)?;
        if entry.plan != *record.plan() { return Err(corrupt()); }
        if let Some(old) = &entry.native {
            old.validate_successor(record)?;
            if old == record { return Ok(()); }
        }
        // Progress must never consume the slots reserved for cancellation and terminal evidence.
        if !record.terminal() && (self.frames + 3 > MAX_FRAMES
            || self.bytes.len() + 3 * (MAX_PAYLOAD + FRAME_OVERHEAD) > MAX_JOURNAL_BYTES)
        { return Err(exhausted()); }
        let mut payload = vec![4]; put_field(&mut payload, key.as_bytes());
        put_field(&mut payload, record.canonical_bytes());
        self.publish(&payload, work)
    }
    /// Fresh plans only. There is intentionally no public prepare/resume/commit API.
    pub fn start<N: ExcavationRunSource>(&mut self, source: &mut N, plan: ExcavationRunPlan,
        confirmed: Digest32, context: &OperationContext) -> Result<ExcavationRunRecord>
    {
        let mut work = Work::new(context)?;
        authorize_start(context, &self.binding, &plan)?;
        if confirmed != plan.digest() || self.entries.contains_key(plan.key())
            || self.pending_count() != 0 || self.entries.len() >= MAX_ENTRIES
        { return Err(error(ErrorCode::Conflict, "unconfirmed, reused or blocked excavation intent")); }
        // Reserve enough retention AND work for the entire start before native contact.
        let reserve = 5 * (MAX_PAYLOAD + FRAME_OVERHEAD);
        if self.frames + 5 > MAX_FRAMES || self.bytes.len() + reserve > MAX_JOURNAL_BYTES
            || work.bytes < 4 * RPC_RESERVE + 16 * (self.bytes.len() + reserve) as u64
        { return Err(exhausted()); }
        let result = (|| {
            let timeout = self.boundary(source, context, true, true, &mut work)?;
            if source.observe(plan.before().region(), context, timeout)? != *plan.before() {
                return Err(error(ErrorCode::StaleAnchor, "excavation capture changed since confirmation"));
            }
            let timeout = self.boundary(source, context, true, true, &mut work)?;
            if source.query(&plan, context, timeout)?.is_some() {
                return Err(error(ErrorCode::Conflict, "native excavation key already retained"));
            }
            let mut intent = vec![1]; put_field(&mut intent, &plan.canonical_bytes());
            self.publish(&intent, &mut work)?;
            let timeout = self.boundary(source, context, true, true, &mut work)?;
            authorize_start(context, &self.binding, &plan)?;
            let prepared = source.prepare(&plan, context, timeout)?;
            let mut payload = vec![2]; put_field(&mut payload, plan.key().as_bytes());
            put_field(&mut payload, prepared.canonical_bytes());
            self.publish(&payload, &mut work)?;
            let mut dispatch = vec![3]; put_field(&mut dispatch, plan.key().as_bytes());
            dispatch.extend_from_slice(plan.digest().as_bytes());
            self.publish(&dispatch, &mut work)?;
            // A cancelled context, lost custody or expired grant cannot use the synced marker.
            let timeout = self.boundary(source, context, true, true, &mut work)?;
            authorize_start(context, &self.binding, &plan)?;
            let permit = ExcavationDispatch { plan: &plan, journal_head: self.head };
            let record = source.commit(permit, context, timeout).map_err(|_| uncertain())?;
            self.binding.source_matches(source.binding(), false)?;
            self.evidence(plan.key(), &record, &mut work).map_err(|_| uncertain())?;
            Ok(record)
        })();
        if result.is_err() { source.fence(); }
        result
    }
    /// One query, or one cancellation attempt. Waiting belongs to a supervised
    /// caller with an overall budget; there is no polling/reconnect loop here.
    pub fn recover<N: ExcavationRunSource>(&mut self, source: &mut N, key: &str,
        cancel: bool, context: &OperationContext) -> Result<Option<ExcavationRunRecord>>
    {
        let mut work = Work::new(context)?;
        authorize(context, &self.binding, false)?;
        self.current(&mut work)?;
        let entry = self.entries.get(key).ok_or_else(corrupt)?.clone();
        if entry.native.as_ref().is_some_and(ExcavationRunRecord::terminal) { return Ok(entry.native); }
        // Leave room for a cancellation marker and a maximum-sized terminal receipt.
        let needed = if cancel && !entry.cancel_requested { 2 } else { 1 };
        let reserve = needed as usize * (MAX_PAYLOAD + FRAME_OVERHEAD);
        if self.frames + needed > MAX_FRAMES || self.bytes.len() + reserve > MAX_JOURNAL_BYTES
            || work.bytes < RPC_RESERVE + 8 * (self.bytes.len() + reserve) as u64
        { return Err(exhausted()); }
        let result = (|| {
            if cancel {
                authorize(context, &self.binding, true)?;
                self.binding.source_matches(source.binding(), true)?;
                if !entry.cancel_requested {
                    let mut payload = vec![5]; put_field(&mut payload, key.as_bytes());
                    self.publish(&payload, &mut work)?;
                }
            }
            let timeout = self.boundary(source, context, cancel, cancel, &mut work)?;
            let record = if cancel { Some(source.cancel(&entry.plan, context, timeout).map_err(|_| uncertain())?) }
                else { source.query(&entry.plan, context, timeout)? };
            self.binding.source_matches(source.binding(), false)?;
            if let Some(value) = &record {
                // Old-source active evidence is not current owned work.
                if !value.terminal() && source.binding().generation != self.binding.generation {
                    return Err(denied());
                }
                self.evidence(key, value, &mut work)?;
            }
            self.current(&mut work)?;
            Ok(record) // Absence writes nothing and leaves the entry unresolved.
        })();
        if result.is_err() { source.fence(); }
        result
    }
}

#[cfg(test)]
mod tests;
