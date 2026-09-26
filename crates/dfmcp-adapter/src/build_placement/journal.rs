//! Durable furniture intent, one-shot dispatch and exact-record recovery.
//!
//! Every native effect is preceded by synchronized intent/preparation/dispatch
//! records. Reopen restores evidence only. Native indeterminacy is immutable and
//! keeps the entire journal fenced against new placements. The caller owns all
//! blocking work and supplies current runtime, lease and checkpoint policy.
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};

use super::{
    BuildBinding, BuildCapture, BuildPhase, BuildPlan, BuildPreparation, BuildRecord,
    BuildSelection,
};
use crate::control_effect_journal::EffectJournalStorage;

pub mod private_file;

pub const MAX_ENTRIES: usize = 256;
pub const MAX_FRAMES: u32 = 1792;
pub const MAX_JOURNAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 10 * 1024;
pub const MAX_FRAME_BYTES: usize = MAX_BODY_BYTES + 92;
pub const RPC_RESERVE: u64 = 272 * 1024;
pub const SOURCE_RESERVE: u64 = 8 * RPC_RESERVE;
const MAGIC: &[u8; 8] = b"DFMBJ019";
const FRAME: &[u8; 8] = b"DFMBJF19";
const END: &[u8; 8] = b"DFMBJEND";
const MAX_BINDING_BYTES: usize = 1024;

pub(super) fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
pub(super) fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "furniture work or retention allowance exhausted",
    )
}
pub(super) fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "furniture journal changed or is incomplete; preserve original evidence",
    )
}
pub(super) fn uncertain(key: &str) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "furniture outcome requires original-key recovery; never retry placement",
    )
    .with_detail("key", key)
}
fn check(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut input = domain.to_vec();
    input.push(0);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}
fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
struct Reader<'a>(&'a [u8]);
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.0.len() {
            return Err(corrupt());
        }
        let (out, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(out)
    }
    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        self.take(N)?.try_into().map_err(|_| corrupt())
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }
    fn number(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.array()?))
    }
    fn field(&mut self, limit: usize) -> Result<&'a [u8]> {
        let n = self.number()? as usize;
        check(n <= limit)?;
        self.take(n)
    }
    fn finish(&self) -> Result<()> {
        check(self.0.is_empty())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildMode {
    Control,
    Recover,
    Offline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuildStage {
    Connect,
    Observe,
    Prepare,
    Commit,
    Query,
    Cancel,
}

/// Trusted supervising policy. No implementation is reconstructed from history
/// or MCP inputs. Checks repeat after dispatch sync and before the native call.
pub trait BuildGuard {
    fn check(
        &mut self,
        stage: BuildStage,
        binding: &BuildBinding,
        plan: Option<&BuildPlan>,
        selection: BuildSelection,
        context: &OperationContext,
    ) -> Result<()>;
}

/// Non-cloneable proof of write-ahead ordering. Only this module can construct it.
/// It is not a lease, capability, confirmation, or evidence of a game effect.
pub struct BuildDispatch<'a> {
    plan: &'a BuildPlan,
    journal_head: Digest32,
}
impl BuildDispatch<'_> {
    pub fn plan(&self) -> &BuildPlan {
        self.plan
    }
    pub fn journal_head(&self) -> Digest32 {
        self.journal_head
    }
}

/// Fixed native interface. The effect shell enforces credentials, cancellation,
/// absolute connection bounds and the shrinking operation timeout at send time.
pub trait BuildSource {
    fn binding(&self) -> &BuildBinding;
    fn fence(&mut self);
    fn observe(
        &mut self,
        selection: BuildSelection,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildCapture>;
    fn prepare(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildPreparation>;
    fn commit(
        &mut self,
        dispatch: BuildDispatch<'_>,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildRecord>;
    fn query(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<Option<BuildRecord>>;
    fn cancel(
        &mut self,
        plan: &BuildPlan,
        context: &OperationContext,
        timeout: Duration,
    ) -> Result<BuildRecord>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BuildState {
    Intent = 0,
    Prepared = 1,
    DispatchStarted = 2,
    Tracking = 3,
    CancelRequested = 4,
    Terminal = 5,
}
impl BuildState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
            Self::Prepared => "prepared",
            Self::DispatchStarted => "dispatch_started",
            Self::Tracking => "tracking",
            Self::CancelRequested => "cancel_requested",
            Self::Terminal => "terminal",
        }
    }
    fn decode(n: u8) -> Result<Self> {
        match n {
            0 => Ok(Self::Intent),
            1 => Ok(Self::Prepared),
            2 => Ok(Self::DispatchStarted),
            3 => Ok(Self::Tracking),
            4 => Ok(Self::CancelRequested),
            5 => Ok(Self::Terminal),
            _ => Err(corrupt()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildEntry {
    plan: BuildPlan,
    state: BuildState,
    native: Option<BuildRecord>,
    dispatch_started: bool,
    cancel_requested: bool,
}
impl BuildEntry {
    pub fn plan(&self) -> &BuildPlan {
        &self.plan
    }
    pub fn state(&self) -> BuildState {
        self.state
    }
    pub fn native(&self) -> Option<&BuildRecord> {
        self.native.as_ref()
    }
    pub fn unresolved(&self) -> bool {
        !self.native.as_ref().is_some_and(BuildRecord::resolved)
    }
    pub fn dispatch_started(&self) -> bool {
        self.dispatch_started
    }
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.state != BuildState::Terminal
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = vec![
            self.state as u8,
            u8::from(self.dispatch_started),
            u8::from(self.cancel_requested),
        ];
        field(&mut out, &self.plan.canonical_bytes());
        field(
            &mut out,
            self.native
                .as_ref()
                .map_or(&[], BuildRecord::canonical_bytes),
        );
        out
    }
    fn decode(bytes: &[u8], binding: &BuildBinding) -> Result<Self> {
        check(bytes.len() <= MAX_BODY_BYTES)?;
        let mut r = Reader(bytes);
        let state = BuildState::decode(r.byte()?)?;
        let dispatched = r.byte()?;
        let cancelled = r.byte()?;
        check(dispatched <= 1 && cancelled <= 1)?;
        let dispatch_started = dispatched == 1;
        let cancel_requested = cancelled == 1;
        let plan = BuildPlan::decode(r.field(3072)?)?;
        check(binding.capture_matches(plan.before()))?;
        let native = r.field(6144)?;
        let native = if native.is_empty() {
            None
        } else {
            Some(BuildRecord::decode(native)?)
        };
        if let Some(record) = &native {
            record.verify_plan(&plan)?;
        }
        r.finish()?;
        let phase = native.as_ref().map(BuildRecord::phase);
        check(match state {
            BuildState::Intent => phase.is_none() && !dispatch_started && !cancel_requested,
            BuildState::Prepared => {
                phase == Some(BuildPhase::Prepared) && !dispatch_started && !cancel_requested
            }
            BuildState::DispatchStarted => {
                phase == Some(BuildPhase::Prepared) && dispatch_started && !cancel_requested
            }
            BuildState::Tracking => phase == Some(BuildPhase::Prepared) && !cancel_requested,
            BuildState::CancelRequested => {
                cancel_requested && (phase.is_none() || phase == Some(BuildPhase::Prepared))
            }
            BuildState::Terminal => phase.is_some_and(|p| p != BuildPhase::Prepared),
        })?;
        Ok(Self {
            plan,
            state,
            native,
            dispatch_started,
            cancel_requested,
        })
    }
}
fn transition(old: Option<&BuildEntry>, next: &BuildEntry) -> Result<()> {
    use BuildState::*;
    let Some(old) = old else {
        return check(next.state == Intent && next.native.is_none());
    };
    check(old.plan == next.plan && old != next && old.state != Terminal)?;
    check(
        (!old.dispatch_started || next.dispatch_started)
            && (!old.cancel_requested || next.cancel_requested),
    )?;
    check(
        next.dispatch_started == old.dispatch_started
            || (old.state == Prepared && next.state == DispatchStarted),
    )?;
    check(next.cancel_requested == old.cancel_requested || next.state == CancelRequested)?;
    if let Some(prior) = &old.native {
        let new = next.native.as_ref().ok_or_else(corrupt)?;
        prior.validate_successor(new)?;
    }
    check(match (old.state, next.state) {
        (Intent, Prepared | Tracking | Terminal) => true,
        (Intent | Prepared | DispatchStarted | Tracking, CancelRequested) => {
            next.native == old.native
        }
        (Prepared, DispatchStarted) => next.native == old.native,
        (Prepared | DispatchStarted | Tracking, Tracking | Terminal) => true,
        (CancelRequested, CancelRequested | Terminal) => true,
        _ => false,
    })
}

/// Complete bounded inventory; presentation may page history while always
/// retaining the independent pending entry. It conveys historical evidence only.
#[derive(Clone, Debug)]
pub struct BuildInventory {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub frames: u32,
    pub byte_len: usize,
    entries: Vec<BuildEntry>,
}
impl BuildInventory {
    pub fn entries(&self) -> &[BuildEntry] {
        &self.entries
    }
    pub fn pending(&self) -> Option<&BuildEntry> {
        self.entries.iter().find(|entry| entry.unresolved())
    }
    pub fn pending_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.unresolved())
            .count()
    }
    pub fn total_records(&self) -> usize {
        self.entries.len()
    }
    pub fn entry(&self, key: &str) -> Option<&BuildEntry> {
        self.entries.iter().find(|entry| entry.plan.key() == key)
    }
}

pub(super) struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Work {
    pub(super) fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        if context.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "furniture operation cancelled",
            ));
        }
        if context.budget.max_wall_millis > 60_000 {
            return Err(exhausted());
        }
        Ok(Self {
            context: context.clone(),
            bytes: context.budget.max_bytes,
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(exhausted)?,
        })
    }
    pub(super) fn remaining(&self) -> Result<Duration> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        if remaining < Duration::from_millis(1) {
            return Err(exhausted());
        }
        Ok(remaining)
    }
    pub(super) fn current(&self) -> Result<OperationContext> {
        let mut context = self.context.clone();
        context.budget.max_wall_millis = self.remaining()?.as_millis() as u64;
        context.budget.max_bytes = self.bytes;
        Ok(context)
    }
    pub(super) fn charge(&mut self, bytes: usize) -> Result<()> {
        self.remaining()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(exhausted)?;
        Ok(())
    }
    pub(super) fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut context = self.current()?;
        self.charge(usize::try_from(bytes).map_err(|_| exhausted())?)?;
        context.budget.max_bytes = bytes;
        Ok(context)
    }
}

pub(super) fn authorize(
    context: &OperationContext,
    binding: &BuildBinding,
    tick: u64,
    stage: BuildStage,
) -> Result<OperationContext> {
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
    if current.anchor.fortress_id != binding.fortress().fortress_id() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "furniture journal belongs to another fortress",
        ));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if matches!(
        stage,
        BuildStage::Observe | BuildStage::Prepare | BuildStage::Commit
    ) {
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    }
    if matches!(stage, BuildStage::Prepare | BuildStage::Commit) {
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        current.authorize(Capability::Construct, RiskTier::Guarded, &[], None)?;
    }
    // Cancellation retires only native preparation. Current Query authority is
    // sufficient after placement authority has been revoked; no writer is run.
    Ok(current)
}

pub struct BuildJournal<S> {
    storage: S,
    binding: BuildBinding,
    raw: Vec<u8>,
    id: Digest32,
    head: Digest32,
    frames: u32,
    entries: BTreeMap<String, BuildEntry>,
    mode: BuildMode,
    owner: SessionId,
    fresh_key: Option<String>,
    fenced: bool,
}
fn read<S: EffectJournalStorage>(storage: &mut S, work: &mut Work) -> Result<Vec<u8>> {
    work.remaining()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    check(length <= MAX_JOURNAL_BYTES as u64)?;
    work.charge(length as usize)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut bytes = vec![0; length as usize];
    storage.read_exact(&mut bytes).map_err(|_| corrupt())?;
    check(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.remaining()?;
    Ok(bytes)
}
impl<S: EffectJournalStorage> BuildJournal<S> {
    /// A nonce is accepted only for an empty, exclusively created Control store.
    /// Existing empty files, wrong bindings and damaged tails are never repaired.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: BuildMode,
        expected: Option<BuildBinding>,
        nonce: Option<[u8; 32]>,
    ) -> Result<Self> {
        let mut work = Work::new(context)?;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if let Some(binding) = &expected {
            authorize(
                context,
                binding,
                context.anchor.tick.get(),
                if nonce.is_some() {
                    BuildStage::Prepare
                } else {
                    BuildStage::Query
                },
            )?;
        }
        let mut raw = read(&mut storage, &mut work)?;
        if let Some(nonce) = nonce {
            check(raw.is_empty() && mode == BuildMode::Control && nonce != [0; 32])?;
            let binding = expected.as_ref().ok_or_else(corrupt)?;
            authorize(
                context,
                binding,
                context.anchor.tick.get(),
                BuildStage::Prepare,
            )?;
            let bytes = binding.encode();
            check(bytes.len() <= MAX_BINDING_BYTES)?;
            raw.extend_from_slice(MAGIC);
            field(&mut raw, &bytes);
            raw.extend_from_slice(&nonce);
            let id = hash(b"dfmcp-build-journal/1", &raw);
            raw.extend_from_slice(id.as_bytes());
            work.charge(raw.len())?;
            storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
            storage
                .write_all(&raw)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
            check(read(&mut storage, &mut work)? == raw)?;
        }
        let mut r = Reader(&raw);
        check(r.take(8)? == MAGIC)?;
        let binding = BuildBinding::decode(r.field(MAX_BINDING_BYTES)?)?;
        check(r.take(32)? != [0; 32])?;
        let prefix = raw.len() - r.0.len();
        let id = Digest32::from_bytes(r.array()?);
        check(id == hash(b"dfmcp-build-journal/1", &raw[..prefix]))?;
        if let Some(expected) = expected {
            check(expected == binding)?;
        }
        authorize(
            &work.current()?,
            &binding,
            context.anchor.tick.get(),
            BuildStage::Query,
        )?;
        let mut head = id;
        let mut frames = 0;
        let mut entries = BTreeMap::new();
        while !r.0.is_empty() {
            work.remaining()?;
            check(frames < MAX_FRAMES && r.take(8)? == FRAME)?;
            let length = r.number()? as usize;
            check(length <= MAX_BODY_BYTES)?;
            let sequence = u64::from_be_bytes(r.array()?);
            check(sequence == u64::from(frames) + 1)?;
            check(r.take(32)? == head.as_bytes())?;
            let body = r.take(length)?;
            work.charge(length.saturating_mul(3) + 92)?;
            let next = BuildEntry::decode(body, &binding)?;
            let digest = Digest32::from_bytes(r.array()?);
            check(r.take(8)? == END)?;
            let encoded = Self::frame(frames + 1, head, body);
            check(digest.as_bytes() == &encoded[encoded.len() - 40..encoded.len() - 8])?;
            Self::check_next(&entries, &next)?;
            entries.insert(next.plan.key().to_owned(), next);
            head = digest;
            frames += 1;
        }
        let journal = Self {
            storage,
            binding,
            raw,
            id,
            head,
            frames,
            entries,
            mode,
            owner: context.session_id,
            fresh_key: None,
            fenced: false,
        };
        journal.access(&work.current()?)?;
        Ok(journal)
    }
    pub fn binding(&self) -> &BuildBinding {
        &self.binding
    }
    pub fn mode(&self) -> BuildMode {
        self.mode
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub fn high_tick(&self) -> u64 {
        self.entries.values().fold(0, |tick, entry| {
            tick.max(entry.plan.before().tick()).max(
                entry
                    .native
                    .as_ref()
                    .and_then(BuildRecord::after)
                    .map_or(0, BuildCapture::tick),
            )
        })
    }
    pub fn high_sequence(&self) -> u64 {
        self.entries.values().fold(0, |sequence, entry| {
            sequence.max(entry.plan.before().sequence().saturating_add(u64::from(
                entry.native.as_ref().is_some_and(BuildRecord::attempted),
            )))
        })
    }
    pub(super) fn abandon(&mut self) {
        self.fresh_key = None;
    }
    pub(super) fn has_permit(&self) -> bool {
        self.fresh_key.is_some() && !self.fenced
    }
    pub(super) fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub(super) fn operation_reserve(&self) -> u64 {
        24 * (self.raw.len() + 7 * MAX_FRAME_BYTES) as u64 + 4 * RPC_RESERVE
    }
    pub(super) fn access(&self, context: &OperationContext) -> Result<OperationContext> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture journal has another session owner",
            ));
        }
        authorize(context, &self.binding, self.high_tick(), BuildStage::Query)
    }
    pub(super) fn online(&self, control: bool) -> Result<()> {
        if self.mode == BuildMode::Offline || (control && self.mode != BuildMode::Control) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "furniture journal mode denies native operation",
            ));
        }
        Ok(())
    }
    pub(super) fn verify(&mut self, work: &mut Work) -> Result<()> {
        self.access(&work.current()?)?;
        if self.fenced {
            return Err(corrupt());
        }
        match read(&mut self.storage, work) {
            Ok(bytes) if bytes == self.raw => Ok(()),
            Err(cause)
                if matches!(
                    cause.code,
                    ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
                ) =>
            {
                Err(cause)
            }
            _ => {
                self.fenced = true;
                self.fresh_key = None;
                Err(corrupt())
            }
        }
    }
    fn frame(sequence: u32, head: Digest32, body: &[u8]) -> Vec<u8> {
        let mut out = FRAME.to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&u64::from(sequence).to_be_bytes());
        out.extend_from_slice(head.as_bytes());
        out.extend_from_slice(body);
        let digest = hash(b"dfmcp-build-journal-frame/1", &out);
        out.extend_from_slice(digest.as_bytes());
        out.extend_from_slice(END);
        out
    }
    fn check_next(entries: &BTreeMap<String, BuildEntry>, next: &BuildEntry) -> Result<()> {
        let old = entries.get(next.plan.key());
        if old.is_none() {
            check(entries.len() < MAX_ENTRIES && !entries.values().any(BuildEntry::unresolved))?;
            let tick = entries
                .values()
                .map(|e| e.plan.before().tick())
                .max()
                .unwrap_or(0);
            let sequence = entries
                .values()
                .map(|e| {
                    e.plan.before().sequence().saturating_add(u64::from(
                        e.native.as_ref().is_some_and(BuildRecord::attempted),
                    ))
                })
                .max()
                .unwrap_or(0);
            check(next.plan.before().tick() >= tick && next.plan.before().sequence() >= sequence)?;
        }
        transition(old, next)
    }
    fn retain(&mut self, entry: BuildEntry, work: &mut Work) -> Result<BuildEntry> {
        self.online(false)?;
        self.verify(work)?;
        if self.entries.get(entry.plan.key()) == Some(&entry) {
            return Ok(entry);
        }
        let body = entry.encode();
        let entry = BuildEntry::decode(&body, &self.binding)?;
        Self::check_next(&self.entries, &entry)?;
        // Reserve all remaining per-key transitions before exposing nonterminal intent.
        let remaining = match entry.state {
            BuildState::Intent => 6,
            BuildState::Prepared => 5,
            BuildState::DispatchStarted => 4,
            BuildState::Tracking => 3,
            BuildState::CancelRequested => 2,
            BuildState::Terminal => 0,
        };
        if self.frames + 1 + remaining > MAX_FRAMES
            || self.raw.len() + (1 + remaining as usize) * MAX_FRAME_BYTES > MAX_JOURNAL_BYTES
        {
            return Err(exhausted());
        }
        let frame = Self::frame(self.frames + 1, self.head, &body);
        work.charge(frame.len() + self.raw.len() + body.len())?;
        let mut raw = self.raw.clone();
        raw.extend_from_slice(&frame);
        self.fenced = true; // BEFORE writing: partial publication never remains usable.
        self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
        self.storage
            .write_all(&frame)
            .and_then(|_| self.storage.flush())
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        check(read(&mut self.storage, work)? == raw)?;
        self.head = Digest32::from_bytes(
            frame[frame.len() - 40..frame.len() - 8]
                .try_into()
                .map_err(|_| corrupt())?,
        );
        self.raw = raw;
        self.frames += 1;
        self.entries
            .insert(entry.plan.key().to_owned(), entry.clone());
        self.fenced = false;
        self.access(&work.current()?)?;
        Ok(entry)
    }
    pub(super) fn known(&self, key: &str, digest: Digest32) -> Result<BuildEntry> {
        super::validate_key(key)?;
        let entry = self.entries.get(key).ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "furniture key absent from this journal",
            )
        })?;
        if entry.plan.digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "furniture key and plan digest disagree",
            ));
        }
        Ok(entry.clone())
    }
    pub fn get(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<BuildEntry> {
        let mut work = Work::new(context)?;
        self.verify(&mut work)?;
        work.charge(MAX_BODY_BYTES)?;
        self.known(key, digest)
    }
    pub fn inventory(&mut self, context: &OperationContext) -> Result<BuildInventory> {
        let mut work = Work::new(context)?;
        self.verify(&mut work)?;
        if self.entries.len() > context.budget.max_entities as usize {
            return Err(exhausted());
        }
        work.charge(self.entries.len() * MAX_BODY_BYTES)?;
        Ok(BuildInventory {
            journal_id: self.id,
            head: self.head,
            frames: self.frames,
            byte_len: self.raw.len(),
            entries: self.entries.values().cloned().collect(),
        })
    }
    fn begin(&mut self, context: &OperationContext) -> Result<Work> {
        let current = self.access(context)?;
        if current.budget.max_bytes < self.operation_reserve() {
            return Err(exhausted());
        }
        let mut work = Work::new(&current)?;
        self.verify(&mut work)?;
        Ok(work)
    }
    fn edge<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &N,
        stage: BuildStage,
        plan: &BuildPlan,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<OperationContext> {
        self.verify(work)?;
        self.binding.source_matches(
            source.binding(),
            matches!(
                stage,
                BuildStage::Observe | BuildStage::Prepare | BuildStage::Commit
            ),
        )?;
        let current = authorize(
            &work.reserve(RPC_RESERVE)?,
            &self.binding,
            self.high_tick().max(plan.before().tick()),
            stage,
        )?;
        guard.check(
            stage,
            &self.binding,
            Some(plan),
            plan.before().selection(),
            &current,
        )?;
        let mut current = current;
        current.budget.max_wall_millis = work.remaining()?.as_millis() as u64;
        Ok(current)
    }
    fn after<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &N,
        stage: BuildStage,
        plan: &BuildPlan,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<()> {
        self.binding.source_matches(
            source.binding(),
            matches!(
                stage,
                BuildStage::Observe | BuildStage::Prepare | BuildStage::Commit
            ),
        )?;
        let current = authorize(
            &work.current()?,
            &self.binding,
            self.high_tick().max(plan.before().tick()),
            stage,
        )?;
        guard.check(
            stage,
            &self.binding,
            Some(plan),
            plan.before().selection(),
            &current,
        )?;
        self.verify(work)
    }
    fn fresh<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &mut N,
        plan: &BuildPlan,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<()> {
        let current = self.edge(source, BuildStage::Observe, plan, work, guard)?;
        let observed = source.observe(plan.before().selection(), &current, work.remaining()?)?;
        self.after(source, BuildStage::Observe, plan, work, guard)?;
        if &observed != plan.before() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "furniture state changed since review",
            ));
        }
        Ok(())
    }
    fn accept(
        &mut self,
        mut entry: BuildEntry,
        native: BuildRecord,
        fresh: bool,
        work: &mut Work,
    ) -> Result<BuildEntry> {
        let native = BuildRecord::decode(native.canonical_bytes())?;
        native.verify_plan(&entry.plan)?;
        entry.state = if native.phase() != BuildPhase::Prepared {
            BuildState::Terminal
        } else if fresh {
            BuildState::Prepared
        } else if entry.state == BuildState::CancelRequested {
            BuildState::CancelRequested
        } else {
            BuildState::Tracking
        };
        entry.native = Some(native);
        self.retain(entry, work)
    }
    pub fn prepare<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &mut N,
        plan: &BuildPlan,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<BuildEntry> {
        self.online(true)?;
        authorize(
            context,
            &self.binding,
            plan.before().tick().max(self.high_tick()),
            BuildStage::Prepare,
        )?;
        check(self.binding.capture_matches(plan.before()))?;
        let mut work = self.begin(context)?;
        if let Some(old) = self.entries.get(plan.key()) {
            if old.plan != *plan {
                return Err(error(
                    ErrorCode::Conflict,
                    "furniture key already names another plan",
                ));
            }
            return Ok(old.clone());
        }
        if self.entries.values().any(BuildEntry::unresolved) {
            return Err(uncertain(plan.key()));
        }
        let entry = BuildEntry {
            plan: plan.clone(),
            state: BuildState::Intent,
            native: None,
            dispatch_started: false,
            cancel_requested: false,
        };
        Self::check_next(&self.entries, &entry)?;
        let result = (|| {
            self.fresh(source, plan, &mut work, guard)?;
            let current = self.edge(source, BuildStage::Query, plan, &mut work, guard)?;
            if source.query(plan, &current, work.remaining()?)?.is_some() {
                return Err(error(
                    ErrorCode::Conflict,
                    "furniture key is already retained natively",
                ));
            }
            self.after(source, BuildStage::Query, plan, &mut work, guard)?;
            let entry = self.retain(entry, &mut work)?;
            let current = self.edge(source, BuildStage::Prepare, plan, &mut work, guard)?;
            let prepared = source.prepare(plan, &current, work.remaining()?)?;
            self.after(source, BuildStage::Prepare, plan, &mut work, guard)?;
            let fresh = !prepared.replayed() && prepared.record().phase() == BuildPhase::Prepared;
            let entry = self.accept(entry, prepared.record().clone(), fresh, &mut work)?;
            if fresh {
                self.fresh_key = Some(plan.key().to_owned());
            }
            Ok(entry)
        })();
        if result.is_err() {
            self.fresh_key = None;
            source.fence();
        }
        if result.is_err() && self.entries.contains_key(plan.key()) {
            return Err(uncertain(plan.key()));
        }
        result
    }
    pub fn commit<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<BuildEntry> {
        self.online(true)?;
        let mut work = self.begin(context)?;
        let mut entry = self.known(key, digest)?;
        authorize(
            context,
            &self.binding,
            self.high_tick().max(entry.plan.before().tick()),
            BuildStage::Commit,
        )?;
        if !entry.needs_reconciliation() {
            return Ok(entry);
        }
        if entry.state != BuildState::Prepared || self.fresh_key.as_deref() != Some(key) {
            return Err(uncertain(key));
        }
        self.fresh_key = None; // Failure at ANY later boundary cannot restore eligibility.
        let result = (|| {
            self.fresh(source, &entry.plan, &mut work, guard)?;
            entry.state = BuildState::DispatchStarted;
            entry.dispatch_started = true;
            let entry = self.retain(entry, &mut work)?;
            let current = self.edge(source, BuildStage::Commit, &entry.plan, &mut work, guard)?;
            let native = source.commit(
                BuildDispatch {
                    plan: &entry.plan,
                    journal_head: self.head,
                },
                &current,
                work.remaining()?,
            )?;
            self.after(source, BuildStage::Commit, &entry.plan, &mut work, guard)?;
            check(native.phase() != BuildPhase::Prepared)?;
            self.accept(entry, native, false, &mut work)
        })();
        if result.is_err() {
            source.fence();
            return Err(uncertain(key));
        }
        result
    }
    pub fn recover<N: BuildSource, G: BuildGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        cancel: bool,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<BuildEntry> {
        let mut work = self.begin(context)?;
        let mut entry = self.known(key, digest)?;
        if !entry.needs_reconciliation() {
            return Ok(entry);
        }
        self.online(false)?;
        self.fresh_key = None;
        let stage = if cancel {
            BuildStage::Cancel
        } else {
            BuildStage::Query
        };
        authorize(
            context,
            &self.binding,
            self.high_tick().max(entry.plan.before().tick()),
            stage,
        )?;
        let result = (|| {
            if cancel && entry.state != BuildState::CancelRequested {
                entry.state = BuildState::CancelRequested;
                entry.cancel_requested = true;
                entry = self.retain(entry, &mut work)?;
            }
            let current = self.edge(source, stage, &entry.plan, &mut work, guard)?;
            let native = if cancel {
                Some(source.cancel(&entry.plan, &current, work.remaining()?)?)
            } else {
                source.query(&entry.plan, &current, work.remaining()?)?
            };
            self.after(source, stage, &entry.plan, &mut work, guard)?;
            let native = native.ok_or_else(|| uncertain(key))?;
            if cancel {
                check(native.phase() != BuildPhase::Prepared)?;
            }
            self.accept(entry, native, false, &mut work)
        })();
        source.fence();
        result
    }
}

#[cfg(test)]
mod tests;
