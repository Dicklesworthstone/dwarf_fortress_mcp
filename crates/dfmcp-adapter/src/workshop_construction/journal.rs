//! Durable workshop intent, one-shot dispatch, exact recovery and goal samples.
//!
//! Every native effect is preceded by synchronized intent/preparation/dispatch
//! frames. Reopen restores evidence only, never dispatch permission. Native
//! indeterminacy fences new keys until the original key's exact receipt is
//! retained. Completion samples are append-only fresh observations of the
//! original building; only an exact-continuity `Complete` sample satisfies the
//! original goal. The caller owns blocking work and supplies current runtime,
//! lease, scope and operator policy through the guard.
use std::collections::BTreeMap;
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};

use super::rpc::WorkshopSource;
use super::{
    MAX_PLAN_BYTES, MAX_PROGRESS_BYTES, MAX_RECORD_BYTES, WorkshopBinding, WorkshopGoal,
    WorkshopPhase, WorkshopPlan, WorkshopProgress, WorkshopRecord, WorkshopSelection,
    WorkshopUnknown,
};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;

pub const MAX_ENTRIES: usize = 16;
pub const MAX_SAMPLES: u16 = 64;
pub const MAX_FRAMES: u32 = (MAX_ENTRIES as u32) * (8 + MAX_SAMPLES as u32);
pub const MAX_BODY_BYTES: usize =
    3 + 4 + MAX_PLAN_BYTES + 4 + MAX_RECORD_BYTES + 2 + 4 + MAX_PROGRESS_BYTES + 17;
pub const MAX_FRAME_BYTES: usize = MAX_BODY_BYTES + 92;
pub const MAX_JOURNAL_BYTES: usize = 16 * 1024 * 1024;
pub const RPC_RESERVE: u64 = 64 * 1024;
const MAGIC: &[u8; 8] = b"DFMWJ020";
const FRAME: &[u8; 8] = b"DFMWJF20";
const END: &[u8; 8] = b"DFMWJEND";
const MAX_BINDING_BYTES: usize = 1024;

pub(super) fn error(code: ErrorCode, message: &str) -> dfmcp_core::DfmcpError {
    dfmcp_core::DfmcpError::new(code, message)
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "workshop work or retention allowance exhausted",
    )
}
fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "workshop journal changed or is incomplete; preserve original evidence",
    )
}
pub fn uncertain(key: &str) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "workshop outcome requires original-key recovery; never retry construction",
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
pub enum WorkshopMode {
    Control,
    Recover,
    Offline,
}
impl WorkshopMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Recover => "recover",
            Self::Offline => "offline",
        }
    }
    fn storage(self) -> BuildMode {
        match self {
            Self::Control => BuildMode::Control,
            Self::Recover => BuildMode::Recover,
            Self::Offline => BuildMode::Offline,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkshopStage {
    Observe,
    Prepare,
    Commit,
    Query,
    Cancel,
    Sample,
}
impl WorkshopStage {
    fn exact_source(self) -> bool {
        matches!(self, Self::Observe | Self::Prepare | Self::Commit)
    }
}

/// Trusted supervising policy: spatial scope, protected regions, live lease,
/// checkpoint policy and operator gates. Never reconstructed from history or
/// MCP inputs. Checks repeat after dispatch synchronization, before the writer.
pub trait WorkshopGuard {
    /// Synchronize independent custody before any newly sent native mutation.
    /// Returning success asserts durable acknowledgement, not just a cached pin.
    fn acknowledge(&mut self, pin: WorkshopPin, context: &OperationContext) -> Result<()>;
    fn check(
        &mut self,
        stage: WorkshopStage,
        binding: &WorkshopBinding,
        selection: WorkshopSelection,
        item_position: Option<[u32; 3]>,
        context: &OperationContext,
    ) -> Result<()>;
}

/// Non-cloneable proof of write-ahead ordering. Only this module constructs it.
/// It is not a lease, capability, confirmation or evidence of a game effect.
pub struct WorkshopDispatch<'a> {
    plan: &'a WorkshopPlan,
    journal_head: Digest32,
}
impl WorkshopDispatch<'_> {
    pub fn plan(&self) -> &WorkshopPlan {
        self.plan
    }
    pub fn journal_head(&self) -> Digest32 {
        self.journal_head
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum WorkshopState {
    Intent = 0,
    Prepared = 1,
    DispatchStarted = 2,
    Tracking = 3,
    CancelRequested = 4,
    Terminal = 5,
}
impl WorkshopState {
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

/// One original goal: the sealed plan, the immutable native receipt when
/// available, and the latest of a bounded count of fresh completion samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkshopEntry {
    plan: WorkshopPlan,
    state: WorkshopState,
    native: Option<WorkshopRecord>,
    dispatch_started: bool,
    cancel_requested: bool,
    samples: u16,
    latest: Option<WorkshopProgress>,
    high_tick: u64,
    high_generation: u64,
    continuity_lost: bool,
}
impl WorkshopEntry {
    pub fn plan(&self) -> &WorkshopPlan {
        &self.plan
    }
    pub fn state(&self) -> WorkshopState {
        self.state
    }
    pub fn native(&self) -> Option<&WorkshopRecord> {
        self.native.as_ref()
    }
    pub fn dispatch_started(&self) -> bool {
        self.dispatch_started
    }
    pub fn cancel_requested(&self) -> bool {
        self.cancel_requested
    }
    pub fn samples(&self) -> u16 {
        self.samples
    }
    pub fn latest_sample(&self) -> Option<&WorkshopProgress> {
        self.latest.as_ref()
    }
    /// Unresolved means the native outcome is unknown or still uncertain.
    pub fn unresolved(&self) -> bool {
        !self.native.as_ref().is_some_and(WorkshopRecord::resolved)
    }
    pub fn placed(&self) -> bool {
        self.native
            .as_ref()
            .is_some_and(|r| r.phase() == WorkshopPhase::Placed)
    }
    /// Verdict of the latest retained sample; `None` means no sample exists.
    pub fn goal(&self) -> Option<WorkshopGoal> {
        if self.continuity_lost {
            return Some(WorkshopGoal::Unknown(
                WorkshopUnknown::NativeGenerationChanged,
            ));
        }
        Some(WorkshopGoal::evaluate(
            self.native.as_ref()?,
            self.latest.as_ref()?,
        ))
    }
    pub fn goal_complete(&self) -> bool {
        self.goal().is_some_and(WorkshopGoal::satisfied)
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn high_generation(&self) -> u64 {
        self.high_generation
    }
    pub fn needs_reconciliation(&self) -> bool {
        self.state != WorkshopState::Terminal
            || self
                .native
                .as_ref()
                .is_some_and(|r| r.phase() == WorkshopPhase::Indeterminate)
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
                .map_or(&[], WorkshopRecord::canonical_bytes),
        );
        out.extend_from_slice(&self.samples.to_be_bytes());
        field(
            &mut out,
            self.latest
                .as_ref()
                .map_or(&[], WorkshopProgress::canonical_bytes),
        );
        out.extend_from_slice(&self.high_tick.to_be_bytes());
        out.extend_from_slice(&self.high_generation.to_be_bytes());
        out.push(u8::from(self.continuity_lost));
        out
    }
    fn decode(bytes: &[u8], binding: &WorkshopBinding) -> Result<Self> {
        check(bytes.len() <= MAX_BODY_BYTES)?;
        let mut r = Reader(bytes);
        let state = WorkshopState::decode(r.byte()?)?;
        let dispatched = r.byte()?;
        let cancelled = r.byte()?;
        check(dispatched <= 1 && cancelled <= 1)?;
        let dispatch_started = dispatched == 1;
        let cancel_requested = cancelled == 1;
        let plan = WorkshopPlan::decode(r.field(MAX_PLAN_BYTES)?)?;
        check(binding.capture_matches(plan.before()))?;
        let native = r.field(MAX_RECORD_BYTES)?;
        let native = if native.is_empty() {
            None
        } else {
            Some(WorkshopRecord::decode(native)?)
        };
        if let Some(record) = &native {
            record.verify_plan(&plan)?;
        }
        let samples = u16::from_be_bytes(r.array()?);
        let latest = r.field(MAX_PROGRESS_BYTES)?;
        let latest = if latest.is_empty() {
            None
        } else {
            Some(WorkshopProgress::decode(latest)?)
        };
        let high_tick = u64::from_be_bytes(r.array()?);
        let high_generation = u64::from_be_bytes(r.array()?);
        let lost = r.byte()?;
        check(lost <= 1)?;
        let continuity_lost = lost == 1;
        r.finish()?;
        let phase = native.as_ref().map(WorkshopRecord::phase);
        check(match state {
            WorkshopState::Intent => phase.is_none() && !dispatch_started && !cancel_requested,
            WorkshopState::Prepared => {
                phase == Some(WorkshopPhase::Prepared) && !dispatch_started && !cancel_requested
            }
            WorkshopState::DispatchStarted => {
                phase == Some(WorkshopPhase::Prepared) && dispatch_started && !cancel_requested
            }
            WorkshopState::Tracking => phase == Some(WorkshopPhase::Prepared) && !cancel_requested,
            WorkshopState::CancelRequested => {
                cancel_requested && (phase.is_none() || phase == Some(WorkshopPhase::Prepared))
            }
            WorkshopState::Terminal => phase.is_some_and(|p| p != WorkshopPhase::Prepared),
        })?;
        // Samples exist only for a retained Placed receipt and are bounded.
        check(
            samples <= MAX_SAMPLES
                && (samples == 0) == latest.is_none()
                && (latest.is_none()
                    || (state == WorkshopState::Terminal && phase == Some(WorkshopPhase::Placed))),
        )?;
        Ok(Self {
            plan,
            state,
            native,
            dispatch_started,
            cancel_requested,
            samples,
            latest,
            high_tick,
            high_generation,
            continuity_lost,
        })
    }
    fn observed_tick(&self) -> u64 {
        self.plan
            .before()
            .tick()
            .max(
                self.native
                    .as_ref()
                    .and_then(WorkshopRecord::after)
                    .map_or(0, |c| c.tick()),
            )
            .max(self.latest.as_ref().map_or(0, WorkshopProgress::tick))
    }
    fn observed_generation(&self) -> u64 {
        self.plan
            .before()
            .generation()
            .max(self.latest.as_ref().map_or(0, WorkshopProgress::generation))
    }
}
fn transition(old: Option<&WorkshopEntry>, next: &WorkshopEntry) -> Result<()> {
    use WorkshopState::*;
    let Some(old) = old else {
        check(
            next.high_tick == next.observed_tick()
                && next.high_generation == next.observed_generation()
                && !next.continuity_lost,
        )?;
        return check(next.state == Intent && next.native.is_none() && next.samples == 0);
    };
    check(
        next.high_tick == old.high_tick.max(next.observed_tick())
            && next.high_generation == old.high_generation.max(next.observed_generation())
            && next.continuity_lost
                == (old.continuity_lost
                    || next.observed_generation() != next.plan.before().generation()),
    )?;
    check(old.plan == next.plan && old != next)?;
    if old.state == Terminal {
        let (Some(prior), Some(new)) = (&old.native, &next.native) else {
            return Err(corrupt());
        };
        check(next.state == Terminal && old.dispatch_started == next.dispatch_started)?;
        if prior.phase() == WorkshopPhase::Indeterminate {
            // Only read-only original-key reconciliation may discharge it.
            prior.validate_successor(new)?;
            return check(new.phase() == WorkshopPhase::Placed && next.samples == 0);
        }
        // A Placed receipt is immutable; only one more fresh sample may append.
        check(prior == new && prior.phase() == WorkshopPhase::Placed)?;
        check(!old.goal_complete() && next.samples == old.samples + 1)?;
        let latest = next.latest.as_ref().ok_or_else(corrupt)?;
        check(latest.generation() >= old.high_generation)?;
        if latest.generation() == old.high_generation {
            check(latest.tick() >= old.high_tick)?;
        }
        return Ok(());
    }
    check(next.samples == 0)?;
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

/// Complete bounded inventory; historical evidence only, no authority.
#[derive(Clone, Debug)]
pub struct WorkshopInventory {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub frames: u32,
    pub byte_len: usize,
    entries: Vec<WorkshopEntry>,
}

/// Independently retained acknowledgement. A self-consistent prefix cannot
/// prove descent from this pin. It is rejection evidence, never authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkshopPin {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub frames: u32,
}
impl WorkshopInventory {
    pub fn entries(&self) -> &[WorkshopEntry] {
        &self.entries
    }
    pub fn pending(&self) -> Option<&WorkshopEntry> {
        self.entries.iter().find(|entry| entry.unresolved())
    }
    pub fn entry(&self, key: &str) -> Option<&WorkshopEntry> {
        self.entries.iter().find(|entry| entry.plan.key() == key)
    }
}

struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        if context.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "workshop operation cancelled",
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
    fn remaining(&self) -> Result<Duration> {
        let remaining = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        if remaining < Duration::from_millis(1) {
            return Err(exhausted());
        }
        Ok(remaining)
    }
    fn current(&self) -> Result<OperationContext> {
        let mut context = self.context.clone();
        context.budget.max_wall_millis = self.remaining()?.as_millis() as u64;
        context.budget.max_bytes = self.bytes;
        Ok(context)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.remaining()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(exhausted)?;
        Ok(())
    }
    fn reserve(&mut self, bytes: u64) -> Result<OperationContext> {
        let mut context = self.current()?;
        self.charge(usize::try_from(bytes).map_err(|_| exhausted())?)?;
        context.budget.max_bytes = bytes;
        Ok(context)
    }
}

pub fn authorize(
    context: &OperationContext,
    binding: &WorkshopBinding,
    tick: u64,
    stage: WorkshopStage,
) -> Result<OperationContext> {
    let mut current = context.clone();
    current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
    if current.anchor.fortress_id != binding.fortress().fortress_id() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "workshop journal belongs to another fortress",
        ));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if matches!(
        stage,
        WorkshopStage::Observe
            | WorkshopStage::Prepare
            | WorkshopStage::Commit
            | WorkshopStage::Sample
    ) {
        current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], None)?;
    }
    if matches!(stage, WorkshopStage::Prepare | WorkshopStage::Commit) {
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        current.authorize(Capability::Construct, RiskTier::Guarded, &[], None)?;
    }
    Ok(current)
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

pub struct WorkshopJournal<S> {
    storage: S,
    binding: WorkshopBinding,
    raw: Vec<u8>,
    id: Digest32,
    head: Digest32,
    frames: u32,
    entries: BTreeMap<String, WorkshopEntry>,
    mode: WorkshopMode,
    owner: SessionId,
    fresh_key: Option<String>,
    fenced: bool,
}

/// Open descriptor-pinned private custody: an absolute canonical path in an
/// owned `0700` directory, a regular single-link `0600` file and an exclusive
/// lock. Only Control with an exact operator binding and fresh nonce can create.
pub fn open_private_workshop(
    path: &Path,
    context: &OperationContext,
    mode: WorkshopMode,
    expected: Option<WorkshopBinding>,
    nonce: [u8; 32],
    acknowledged: Option<WorkshopPin>,
) -> Result<WorkshopJournal<PrivateBuildFile>> {
    if expected.is_none() && mode != WorkshopMode::Offline && acknowledged.is_none() {
        return Err(corrupt());
    }
    let create = mode == WorkshopMode::Control && expected.is_some();
    let (storage, created, current) =
        open_private_storage(path, context, mode.storage(), create, MAX_JOURNAL_BYTES)?;
    WorkshopJournal::open(
        storage,
        &current,
        mode,
        expected,
        created.then_some(nonce),
        acknowledged,
    )
}

impl<S: EffectJournalStorage> WorkshopJournal<S> {
    /// A nonce is accepted only for an empty, exclusively created Control store.
    /// Existing empty files, wrong bindings and damaged tails are never repaired.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: WorkshopMode,
        expected: Option<WorkshopBinding>,
        nonce: Option<[u8; 32]>,
        acknowledged: Option<WorkshopPin>,
    ) -> Result<Self> {
        check(nonce.is_some() || mode == WorkshopMode::Offline || acknowledged.is_some())?;
        let mut work = Work::new(context)?;
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        let mut raw = read(&mut storage, &mut work)?;
        if let Some(nonce) = nonce {
            check(raw.is_empty() && mode == WorkshopMode::Control && nonce != [0; 32])?;
            let binding = expected.as_ref().ok_or_else(corrupt)?;
            authorize(
                context,
                binding,
                context.anchor.tick.get(),
                WorkshopStage::Prepare,
            )?;
            let bytes = binding.encode();
            check(bytes.len() <= MAX_BINDING_BYTES)?;
            raw.extend_from_slice(MAGIC);
            field(&mut raw, &bytes);
            raw.extend_from_slice(&nonce);
            let id = hash(b"dfmcp-workshop-journal/1", &raw);
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
        let binding = WorkshopBinding::decode(r.field(MAX_BINDING_BYTES)?)?;
        check(r.take(32)? != [0; 32])?;
        let prefix = raw.len() - r.0.len();
        let id = Digest32::from_bytes(r.array()?);
        check(id == hash(b"dfmcp-workshop-journal/1", &raw[..prefix]))?;
        if let Some(expected) = &expected {
            check(*expected == binding)?;
        }
        authorize(
            &work.current()?,
            &binding,
            context.anchor.tick.get(),
            WorkshopStage::Query,
        )?;
        let mut head = id;
        let mut frames = 0;
        if let Some(pin) = acknowledged {
            check(pin.journal_id == id && pin.frames <= MAX_FRAMES)?;
            if pin.frames == 0 {
                check(pin.head == head)?;
            }
        }
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
            let digest = Digest32::from_bytes(r.array()?);
            check(r.take(8)? == END)?;
            // Chain integrity first; any decoding failure is retained corruption.
            let encoded = Self::frame(frames + 1, head, body);
            check(digest.as_bytes() == &encoded[encoded.len() - 40..encoded.len() - 8])?;
            let next = WorkshopEntry::decode(body, &binding).map_err(|_| corrupt())?;
            Self::check_next(&entries, &next)?;
            entries.insert(next.plan.key().to_owned(), next);
            head = digest;
            frames += 1;
            if let Some(pin) = acknowledged
                && frames == pin.frames
            {
                check(head == pin.head)?;
            }
        }
        if let Some(pin) = acknowledged {
            check(frames >= pin.frames)?;
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
    pub fn binding(&self) -> &WorkshopBinding {
        &self.binding
    }
    pub fn mode(&self) -> WorkshopMode {
        self.mode
    }
    pub fn head(&self) -> Digest32 {
        self.head
    }
    pub fn pin(&self) -> WorkshopPin {
        WorkshopPin {
            journal_id: self.id,
            head: self.head,
            frames: self.frames,
        }
    }
    /// Pure last-retained memory for the single-goal consumer's failure handoff.
    /// Not verified custody, current game facts, permission or recovery authority.
    pub fn unverified_original_reference(&self) -> Option<&WorkshopEntry> {
        self.entries.values().next()
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    /// Local one-use commit permission exists only after a fresh preparation in
    /// this process; reopening never restores it.
    pub fn has_permit(&self, key: &str) -> bool {
        !self.fenced && self.fresh_key.as_deref() == Some(key)
    }
    pub fn abandon(&mut self) {
        self.fresh_key = None;
    }
    pub fn high_tick(&self) -> u64 {
        self.entries
            .values()
            .map(|e| e.high_tick)
            .max()
            .unwrap_or(0)
    }
    fn access(&self, context: &OperationContext) -> Result<OperationContext> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "workshop journal has another session owner",
            ));
        }
        authorize(
            context,
            &self.binding,
            self.high_tick(),
            WorkshopStage::Query,
        )
    }
    fn online(&self, control: bool) -> Result<()> {
        if self.mode == WorkshopMode::Offline || (control && self.mode != WorkshopMode::Control) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "workshop journal mode denies native operation",
            ));
        }
        Ok(())
    }
    fn verify(&mut self, work: &mut Work) -> Result<()> {
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
        let digest = hash(b"dfmcp-workshop-journal-frame/1", &out);
        out.extend_from_slice(digest.as_bytes());
        out.extend_from_slice(END);
        out
    }
    fn check_next(entries: &BTreeMap<String, WorkshopEntry>, next: &WorkshopEntry) -> Result<()> {
        let old = entries.get(next.plan.key());
        if old.is_none() {
            // A new key never coexists with unresolved work and never regresses.
            check(entries.len() < MAX_ENTRIES && !entries.values().any(WorkshopEntry::unresolved))?;
            let tick = entries.values().map(|e| e.high_tick).max().unwrap_or(0);
            check(next.plan.before().tick() >= tick)?;
        }
        transition(old, next)
    }
    fn retain(&mut self, mut entry: WorkshopEntry, work: &mut Work) -> Result<WorkshopEntry> {
        self.online(false)?;
        self.verify(work)?;
        entry.high_tick = entry.high_tick.max(entry.observed_tick());
        entry.high_generation = entry.high_generation.max(entry.observed_generation());
        entry.continuity_lost |= entry.observed_generation() != entry.plan.before().generation();
        let body = entry.encode();
        let entry = WorkshopEntry::decode(&body, &self.binding)?;
        Self::check_next(&self.entries, &entry)?;
        // Reserve every remaining per-key transition before exposing intent.
        let remaining = match entry.state {
            WorkshopState::Terminal => u32::from(MAX_SAMPLES - entry.samples) + 1,
            _ => 6 + u32::from(MAX_SAMPLES),
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
    fn known(&self, key: &str, digest: Digest32) -> Result<WorkshopEntry> {
        crate::bounded_run::validate_key(key)?;
        let entry = self.entries.get(key).ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "workshop key absent from this journal",
            )
        })?;
        if entry.plan.digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "workshop key and plan digest disagree",
            ));
        }
        Ok(entry.clone())
    }
    pub fn get(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<WorkshopEntry> {
        let mut work = Work::new(context)?;
        self.verify(&mut work)?;
        self.known(key, digest)
    }
    pub fn inventory(&mut self, context: &OperationContext) -> Result<WorkshopInventory> {
        let mut work = Work::new(context)?;
        self.verify(&mut work)?;
        work.charge(self.entries.len() * MAX_BODY_BYTES)?;
        Ok(WorkshopInventory {
            journal_id: self.id,
            head: self.head,
            frames: self.frames,
            byte_len: self.raw.len(),
            entries: self.entries.values().cloned().collect(),
        })
    }
    fn begin(&mut self, context: &OperationContext) -> Result<Work> {
        let current = self.access(context)?;
        let mut work = Work::new(&current)?;
        self.verify(&mut work)?;
        Ok(work)
    }
    fn source_matches<N: WorkshopSource>(&self, source: &N, stage: WorkshopStage) -> Result<()> {
        let m = source.manifest();
        let current = WorkshopBinding {
            endpoint: m.endpoint,
            generation: m.generation,
            fortress: m.fortress.clone(),
            dimensions: self.binding.dimensions(),
            df_version: m.df_version.clone(),
            dfhack_version: m.dfhack_version.clone(),
        };
        self.binding
            .source_matches(&current, stage.exact_source())?;
        check(
            current.generation()
                >= self
                    .entries
                    .values()
                    .map(|e| e.high_generation)
                    .max()
                    .unwrap_or(self.binding.generation()),
        )
    }
    fn edge<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &N,
        stage: WorkshopStage,
        plan: &WorkshopPlan,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<OperationContext> {
        self.verify(work)?;
        self.source_matches(source, stage)?;
        let mut current = authorize(
            &work.reserve(RPC_RESERVE)?,
            &self.binding,
            self.high_tick().max(plan.before().tick()),
            stage,
        )?;
        guard.check(
            stage,
            &self.binding,
            plan.before().selection(),
            plan.before().item_position(),
            &current,
        )?;
        // A guard may perform bounded I/O; custody must still be exact after it.
        self.verify(work)?;
        current.budget.max_wall_millis = work.remaining()?.as_millis() as u64;
        Ok(current)
    }
    fn after<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &N,
        stage: WorkshopStage,
        plan: &WorkshopPlan,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<()> {
        self.source_matches(source, stage)?;
        let current = authorize(
            &work.current()?,
            &self.binding,
            self.high_tick().max(plan.before().tick()),
            stage,
        )?;
        guard.check(
            stage,
            &self.binding,
            plan.before().selection(),
            plan.before().item_position(),
            &current,
        )?;
        self.verify(work)
    }
    fn fresh<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &mut N,
        plan: &WorkshopPlan,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<()> {
        let current = self.edge(source, WorkshopStage::Observe, plan, work, guard)?;
        let observed = source.observe(plan.before().selection(), &current, work.remaining()?)?;
        self.after(source, WorkshopStage::Observe, plan, work, guard)?;
        if &observed != plan.before() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "workshop state changed since review",
            ));
        }
        Ok(())
    }
    fn accept(
        &mut self,
        mut entry: WorkshopEntry,
        native: WorkshopRecord,
        fresh: bool,
        work: &mut Work,
    ) -> Result<WorkshopEntry> {
        let native = WorkshopRecord::decode(native.canonical_bytes())?;
        native.verify_plan(&entry.plan)?;
        entry.state = if native.phase() != WorkshopPhase::Prepared {
            WorkshopState::Terminal
        } else if fresh {
            WorkshopState::Prepared
        } else if entry.state == WorkshopState::CancelRequested {
            WorkshopState::CancelRequested
        } else {
            WorkshopState::Tracking
        };
        if entry.native.as_ref() == Some(&native) && entry.state == WorkshopState::Terminal {
            return Ok(entry); // Immutable replay retains nothing new.
        }
        entry.native = Some(native);
        self.retain(entry, work)
    }
    pub fn prepare<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &mut N,
        plan: &WorkshopPlan,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<WorkshopEntry> {
        self.online(true)?;
        authorize(
            context,
            &self.binding,
            plan.before().tick().max(self.high_tick()),
            WorkshopStage::Prepare,
        )?;
        check(self.binding.capture_matches(plan.before()))?;
        let mut work = self.begin(context)?;
        if let Some(old) = self.entries.get(plan.key()) {
            if old.plan != *plan {
                return Err(error(
                    ErrorCode::Conflict,
                    "workshop key already names another plan",
                ));
            }
            return Ok(old.clone()); // Replay never renews permission.
        }
        if let Some(pending) = self.entries.values().find(|e| e.unresolved()) {
            return Err(uncertain(pending.plan.key()));
        }
        if plan.before().tick() < self.high_tick() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "workshop plan predates consumed source high-water",
            ));
        }
        let entry = WorkshopEntry {
            plan: plan.clone(),
            state: WorkshopState::Intent,
            native: None,
            dispatch_started: false,
            cancel_requested: false,
            samples: 0,
            latest: None,
            high_tick: plan.before().tick(),
            high_generation: plan.before().generation(),
            continuity_lost: false,
        };
        Self::check_next(&self.entries, &entry)?;
        let result = (|| {
            self.fresh(source, plan, &mut work, guard)?;
            let current = self.edge(source, WorkshopStage::Query, plan, &mut work, guard)?;
            if source.query(plan, &current, work.remaining()?)?.is_some() {
                return Err(error(
                    ErrorCode::Conflict,
                    "workshop key is already retained natively",
                ));
            }
            self.after(source, WorkshopStage::Query, plan, &mut work, guard)?;
            let summary = source.native_summary();
            if summary.unresolved() {
                return Err(error(
                    ErrorCode::EffectIndeterminate,
                    "native workshop history is unresolved; no new intent was retained",
                ));
            }
            if !summary.prepare_available() {
                return Err(exhausted());
            }
            let entry = self.retain(entry, &mut work)?;
            guard.acknowledge(self.pin(), &work.current()?)?;
            let current = self.edge(source, WorkshopStage::Prepare, plan, &mut work, guard)?;
            let prepared = source.prepare(plan, &current, work.remaining()?)?;
            self.after(source, WorkshopStage::Prepare, plan, &mut work, guard)?;
            let fresh =
                !prepared.replayed() && prepared.record().phase() == WorkshopPhase::Prepared;
            let entry = self.accept(entry, prepared.record().clone(), fresh, &mut work)?;
            if fresh {
                self.fresh_key = Some(plan.key().to_owned());
            }
            Ok(entry)
        })();
        if result.is_err() {
            self.fresh_key = None;
            source.fence();
            if self.entries.contains_key(plan.key()) {
                return Err(uncertain(plan.key()));
            }
        }
        result
    }
    pub fn commit<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<WorkshopEntry> {
        self.online(true)?;
        let mut work = self.begin(context)?;
        let mut entry = self.known(key, digest)?;
        authorize(
            context,
            &self.binding,
            self.high_tick().max(entry.plan.before().tick()),
            WorkshopStage::Commit,
        )?;
        if entry.state == WorkshopState::Terminal {
            return Ok(entry);
        }
        if entry.state != WorkshopState::Prepared || self.fresh_key.as_deref() != Some(key) {
            return Err(uncertain(key));
        }
        self.fresh_key = None; // Failure at ANY later boundary cannot restore eligibility.
        let result = (|| {
            self.fresh(source, &entry.plan, &mut work, guard)?;
            entry.state = WorkshopState::DispatchStarted;
            entry.dispatch_started = true;
            let entry = self.retain(entry, &mut work)?;
            guard.acknowledge(self.pin(), &work.current()?)?;
            let current =
                self.edge(source, WorkshopStage::Commit, &entry.plan, &mut work, guard)?;
            let native = source.commit(
                WorkshopDispatch {
                    plan: &entry.plan,
                    journal_head: self.head,
                },
                &current,
                work.remaining()?,
            )?;
            self.after(source, WorkshopStage::Commit, &entry.plan, &mut work, guard)?;
            check(native.phase() != WorkshopPhase::Prepared)?;
            self.accept(entry, native, false, &mut work)
        })();
        if result.is_err() {
            source.fence();
            return Err(uncertain(key));
        }
        result
    }
    /// One read-only original-key query, or retirement of an unattempted
    /// preparation. Never a second writer call; absence stays unresolved.
    pub fn recover<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        cancel: bool,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<WorkshopEntry> {
        let mut work = self.begin(context)?;
        let mut entry = self.known(key, digest)?;
        if !entry.needs_reconciliation() {
            return Ok(entry);
        }
        self.online(false)?;
        self.fresh_key = None;
        let stage = if cancel {
            WorkshopStage::Cancel
        } else {
            WorkshopStage::Query
        };
        authorize(
            context,
            &self.binding,
            self.high_tick().max(entry.plan.before().tick()),
            stage,
        )?;
        let result = (|| {
            let attempted = entry.dispatch_started
                || entry.native.as_ref().is_some_and(WorkshopRecord::attempted);
            if cancel && !attempted && entry.state != WorkshopState::CancelRequested {
                entry.state = WorkshopState::CancelRequested;
                entry.cancel_requested = true;
                entry = self.retain(entry, &mut work)?;
            }
            if cancel && !attempted {
                guard.acknowledge(self.pin(), &work.current()?)?;
            }
            let current = self.edge(source, stage, &entry.plan, &mut work, guard)?;
            let native = if cancel && !attempted {
                Some(source.cancel(&entry.plan, &current, work.remaining()?)?)
            } else {
                source.query(&entry.plan, &current, work.remaining()?)?
            };
            self.after(source, stage, &entry.plan, &mut work, guard)?;
            let native = native.ok_or_else(|| uncertain(key))?;
            if cancel && !attempted {
                check(native.phase() != WorkshopPhase::Prepared)?;
            }
            self.accept(entry, native, false, &mut work)
        })();
        source.fence();
        result
    }
    /// One fresh progress read of the original building, durably appended.
    /// It never dispatches, plans or cancels, and requires neither the original
    /// log nor any worker to remain as observed before construction.
    pub fn sample<N: WorkshopSource, G: WorkshopGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<WorkshopEntry> {
        self.online(false)?;
        let mut work = self.begin(context)?;
        let mut entry = self.known(key, digest)?;
        let Some(insertion) = entry
            .native
            .as_ref()
            .and_then(WorkshopRecord::insertion)
            .cloned()
        else {
            return Err(error(
                ErrorCode::InvalidRequest,
                "workshop goal has no placed receipt to sample",
            ));
        };
        if entry.goal_complete() {
            return Ok(entry); // The proved original goal is immutable history.
        }
        if entry.samples >= MAX_SAMPLES {
            return Err(exhausted());
        }
        self.fresh_key = None;
        let result = (|| {
            let current =
                self.edge(source, WorkshopStage::Sample, &entry.plan, &mut work, guard)?;
            let progress = source.progress(
                insertion.building_id(),
                insertion.item_id(),
                &current,
                work.remaining()?,
            )?;
            if progress.generation() < entry.high_generation
                || (progress.generation() == entry.high_generation
                    && progress.tick() < entry.high_tick)
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "workshop sample regressed consumed source high-water",
                ));
            }
            self.after(source, WorkshopStage::Sample, &entry.plan, &mut work, guard)?;
            entry.samples += 1;
            entry.latest = Some(progress);
            self.retain(entry, &mut work)
        })();
        source.fence();
        result
    }
}

#[cfg(test)]
#[path = "journal_tests.rs"]
mod tests;
