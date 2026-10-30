//! Durable joint original-room monitoring with write-ahead read intent.
//!
//! Every replay recomputes conditions from full canonical evidence. An unfinished
//! read stays unknown across restart; only this live owner can publish its next
//! sample. This journal never authorizes or discharges a placement effect.
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier,
    SessionId,
};

use super::{Definition, MAX_DEFINITION_SIZE};
use super::{MAX_SAMPLE, Progress, Sample, SourceIdentity};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::construction_plan::origin::Origin;
use crate::control_effect_journal::EffectJournalStorage;

pub const MAX_STORE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_STORE_FRAMES: u32 = 1030;
const MAGIC: &[u8; 8] = b"DFMRCJ01";
const DOMAIN: &[u8] = b"dfmcp.original-room-condition-journal/1\0";
const HEADER: usize = 40;
const OVERHEAD: usize = HEADER + 1 + 32;
const MAX_BODY: usize = MAX_SAMPLE + 1;

/// Externally retained original monitor identity and acknowledged history head.
/// Complete descendants may recover a lost reply; earlier or unrelated
/// histories cannot replace this acknowledged custody.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub monitor_id: Digest32,
    pub head: Digest32,
}
impl Recovery {
    pub fn validate(self) -> Result<()> {
        if self.monitor_id == Digest32::ZERO || self.head == Digest32::ZERO {
            return Err(error(
                ErrorCode::InvalidRequest,
                "joint room recovery requires its original monitor identity and acknowledged head",
            ));
        }
        Ok(())
    }
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "joint room monitor custody or evidence changed; preserve and reopen original history without repair",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "joint room monitor foreground or retention allowance exhausted",
    )
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(bytes: &[u8]) -> Digest32 {
    let mut input = Vec::with_capacity(DOMAIN.len() + bytes.len());
    input.extend_from_slice(DOMAIN);
    input.extend_from_slice(bytes);
    Digest32::of_bytes(&input)
}
fn authorize(
    context: &OperationContext,
    definition: Option<&Definition>,
    progress: Option<&Progress>,
) -> Result<OperationContext> {
    let mut current = context.clone();
    if definition.is_some_and(|value| {
        value
            .origin()
            .definition()
            .binding()
            .fortress()
            .fortress_id()
            != current.anchor.fortress_id
    }) {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "joint room monitor belongs to another fortress",
        ));
    }
    // Original receipts establish a native clock floor before the first
    // monitored sample. An older caller anchor cannot revive expired Query.
    if let Some(definition) = definition {
        let original_floor = definition
            .origin()
            .receipts()
            .iter()
            .fold(0, |tick, record| {
                tick.max(record.plan().before().tick())
                    .max(record.after().map_or(0, |capture| capture.tick()))
            });
        current.anchor.tick = GameTick(current.anchor.tick.get().max(original_floor));
    }
    if let Some(tick) = progress.map(|value| value.observed_tick_floor) {
        current.anchor.tick = GameTick(current.anchor.tick.get().max(tick));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    Ok(current)
}
struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
    semantic_work: u64,
}
impl Work {
    fn new(context: &OperationContext) -> Result<Self> {
        let context = authorize(context, None, None)?;
        context.budget.validate()?;
        if context.budget.max_wall_millis > 60_000
            || context.budget.max_bytes > 2 * 1024 * 1024 * 1024
        {
            return Err(bounded());
        }
        let out = Self {
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(bounded)?,
            bytes: context.budget.max_bytes,
            context,
            semantic_work: 0,
        };
        out.current()?;
        Ok(out)
    }
    fn current(&self) -> Result<OperationContext> {
        let millis = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(bounded)?
            .as_millis() as u64;
        if millis == 0 {
            return Err(bounded());
        }
        let mut current = authorize(&self.context, None, None)?;
        current.budget.max_wall_millis = millis.min(current.budget.max_wall_millis);
        current.budget.max_bytes = self.bytes;
        Ok(current)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.current()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(bounded)?;
        Ok(())
    }
    fn reserve(&self, bytes: usize) -> Result<()> {
        self.current()?;
        if self.bytes < bytes as u64 {
            return Err(bounded());
        }
        Ok(())
    }
    fn child(&mut self, bytes: usize) -> Result<OperationContext> {
        self.charge(bytes)?;
        let mut current = self.current()?;
        current.budget.max_bytes = bytes as u64;
        Ok(current)
    }
}

fn read<S: EffectJournalStorage>(storage: &mut S, work: &mut Work) -> Result<Vec<u8>> {
    work.current()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    require(length <= MAX_STORE_BYTES as u64)?;
    work.charge(length as usize)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut bytes = vec![0; length as usize];
    let mut offset = 0;
    while offset < bytes.len() {
        work.current()?;
        let end = bytes.len().min(offset + 64 * 1024);
        let count = storage
            .read(&mut bytes[offset..end])
            .map_err(|_| corrupt())?;
        require(count > 0)?;
        offset += count;
    }
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.current()?;
    Ok(bytes)
}
fn frame(sequence: u32, previous: Digest32, kind: u8, payload: &[u8]) -> Result<Vec<u8>> {
    require(sequence < MAX_STORE_FRAMES && (1..=5).contains(&kind) && payload.len() < MAX_BODY)?;
    let mut out = Vec::with_capacity(OVERHEAD + payload.len());
    out.extend_from_slice(&((payload.len() + 1) as u32).to_be_bytes());
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(previous.as_bytes());
    out.push(kind);
    out.extend_from_slice(payload);
    let digest = hash(&out);
    out.extend_from_slice(digest.as_bytes());
    Ok(out)
}
fn tail(frame: &[u8]) -> Result<Digest32> {
    Ok(Digest32::from_bytes(
        frame
            .get(frame.len().saturating_sub(32)..)
            .ok_or_else(corrupt)?
            .try_into()
            .map_err(|_| corrupt())?,
    ))
}

/// Retain this pin before creating a journal so a complete header with a lost
/// synchronization acknowledgement can be reopened without inventing an origin.
pub fn initial_recovery(definition: &Definition) -> Result<Recovery> {
    Ok(Recovery {
        monitor_id: definition.id(),
        head: tail(&frame(0, Digest32::ZERO, 1, definition.canonical_bytes())?)?,
    })
}
fn transition(
    definition: &Definition,
    progress: &Progress,
    kind: u8,
    payload: &[u8],
    work: &mut Work,
) -> Result<Progress> {
    require(!progress.terminal())?;
    work.semantic_work = work.semantic_work.checked_add(1).ok_or_else(bounded)?;
    if work.semantic_work > crate::construction_plan::MAX_WORK {
        return Err(bounded());
    }
    match kind {
        2 => {
            require(payload.len() == 8)?;
            let floor = u64::from_be_bytes(payload.try_into().map_err(|_| corrupt())?);
            require(floor >= progress.observed_tick_floor)?;
            let mut next = progress.begin_read()?;
            next.observed_tick_floor = floor;
            Ok(next)
        }
        3 | 5 => {
            // Parsing and all member evaluation share this one reservation;
            // no target or frame renews the enclosing operation allowance.
            let current = work.child(payload.len().saturating_mul(6).saturating_add(16_384))?;
            let sample = Sample::decode(payload)?;
            if kind == 3 {
                super::advance_with_counter(
                    progress,
                    definition,
                    &sample,
                    &current,
                    &mut work.semantic_work,
                )
            } else {
                super::readiness::advance_with_counter(
                    progress,
                    definition,
                    &sample,
                    &current,
                    &mut work.semantic_work,
                )
            }
        }
        4 => {
            require(payload.len() == 8)?;
            let floor = u64::from_be_bytes(payload.try_into().map_err(|_| corrupt())?);
            require(floor >= progress.observed_tick_floor)?;
            let mut next = progress.cancel();
            next.observed_tick_floor = floor;
            Ok(next)
        }
        _ => Err(corrupt()),
    }
}
struct Replay {
    definition: Definition,
    progress: Progress,
    first_source: Option<SourceIdentity>,
    frames: u32,
    head: Digest32,
    heads: Vec<Digest32>,
}
fn replay(bytes: &[u8], work: &mut Work) -> Result<Replay> {
    require(
        bytes.len() >= MAGIC.len() + OVERHEAD
            && bytes.len() <= MAX_STORE_BYTES
            && bytes.get(..8) == Some(MAGIC.as_slice()),
    )?;
    let mut offset: usize = 8;
    let mut state: Option<Replay> = None;
    while offset < bytes.len() {
        work.current()?;
        require(bytes.len() - offset >= OVERHEAD)?;
        let number = |start: usize| -> Result<u32> {
            Ok(u32::from_be_bytes(
                bytes
                    .get(start..start + 4)
                    .ok_or_else(corrupt)?
                    .try_into()
                    .map_err(|_| corrupt())?,
            ))
        };
        let length = number(offset)? as usize;
        let sequence = number(offset + 4)?;
        require(
            (1..=MAX_BODY).contains(&length)
                && sequence < MAX_STORE_FRAMES
                && HEADER + length + 32 <= bytes.len() - offset,
        )?;
        let end = offset + HEADER + length + 32;
        let previous = state
            .as_ref()
            .map_or(Digest32::from_bytes([0; 32]), |value| value.head);
        require(
            sequence == state.as_ref().map_or(0, |value| value.frames)
                && &bytes[offset + 8..offset + HEADER] == previous.as_bytes(),
        )?;
        work.charge(HEADER + length + 32)?;
        let head = tail(&bytes[offset..end])?;
        require(head == hash(&bytes[offset..end - 32]))?;
        let kind = bytes[offset + HEADER];
        let payload = &bytes[offset + HEADER + 1..end - 32];
        let (definition, progress, first_source, mut heads) = if let Some(prior) = state {
            let progress = transition(&prior.definition, &prior.progress, kind, payload, work)?;
            (prior.definition, progress, prior.first_source, prior.heads)
        } else {
            require(kind == 1 && payload.len() <= MAX_DEFINITION_SIZE)?;
            work.charge(payload.len().saturating_mul(3))?;
            let definition = Definition::decode(payload)?;
            let progress = Progress::new(&definition);
            (definition, progress, None, Vec::new())
        };
        let first_source = first_source.or_else(|| progress.source.clone());
        // Membership is retained only after both the frame chain and the
        // complete semantic transition pass. MAX_STORE_FRAMES bounds this
        // evidence; a hash found in unvalidated payload bytes is never a head.
        heads.push(head);
        state = Some(Replay {
            definition,
            progress,
            first_source,
            frames: sequence + 1,
            head,
            heads,
        });
        offset = end;
    }
    state.ok_or_else(corrupt)
}

/// Local display only. The original source, clock floor, observation allowance
/// and exact last assessments stay retained; none can become a new positive
/// sample merely because the enclosing caller lost continuity.
fn withdrawn(progress: &Progress) -> Progress {
    let mut visible = progress.clone();
    if !visible.terminal() {
        visible.construction.phase = "active";
        visible.construction.reason = "continuity_withdrawn";
        visible.construction.reason_building = None;
        visible.construction.streak = 0;
        visible.construction.first_tick = None;
        visible.construction.counted_tick = None;
    }
    visible
}

/// The storage is owned for the complete session. Current source custody is
/// checked through a caller-supplied guard using the same shrinking deadline;
/// the caller reserves its source-file I/O separately from this journal's I/O.
pub struct MonitorStore<S> {
    storage: S,
    definition: Definition,
    durable_progress: Progress,
    first_source: Option<SourceIdentity>,
    visible: Progress,
    resume_reset: bool,
    raw: Vec<u8>,
    frames: u32,
    head: Digest32,
    heads: Vec<Digest32>,
    mode: BuildMode,
    owner: SessionId,
    read_owned: bool,
    fenced: bool,
    observed_floor: u64,
}
impl<S: EffectJournalStorage> MonitorStore<S> {
    fn completed<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.withdraw_unpublished();
        }
        result
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        let result = self.verify_inner(context);
        self.completed(result)
    }
    pub fn verify_recovery(
        &mut self,
        context: &OperationContext,
        expected: Recovery,
    ) -> Result<()> {
        let result = self.verify_recovery_inner(context, expected);
        self.completed(result)
    }
    pub fn verify_origin<G>(&mut self, context: &OperationContext, guard: &mut G) -> Result<()>
    where
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let result = self.verify_origin_inner(context, guard);
        self.completed(result)
    }
    pub fn begin_read<F, G>(
        &mut self,
        context: &OperationContext,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let result = self.begin_read_inner(context, admit, guard);
        self.completed(result)
    }
    pub fn publish_sample<F, G>(
        &mut self,
        context: &OperationContext,
        sample: &Sample,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let result = self.publish_acquisition(context, sample, 3, admit, guard);
        self.completed(result)
    }
    /// Retain one complete room acquisition for an independently authorized
    /// clock review. The full raw sample uses a distinct frame from ordinary
    /// completion evidence: replay consumes its original observation allowance
    /// but never awards stability credit or satisfaction. A compatible sample
    /// can preserve an existing streak; unmet or unknown facts withdraw it.
    /// `operation_reserve(true)` covers the same bounded publication path.
    pub fn publish_readiness<F, G>(
        &mut self,
        context: &OperationContext,
        sample: &Sample,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let result = self.publish_acquisition(context, sample, 5, admit, guard);
        self.completed(result)
    }
    pub fn cancel<F>(&mut self, context: &OperationContext, admit: &mut F) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
    {
        let result = self.cancel_inner(context, admit);
        self.completed(result)
    }
    /// Query authority can create monitoring custody in Control or Recover.
    /// The file shell supplies create only for a new exclusively opened file.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: BuildMode,
        expected: Option<Definition>,
        create: bool,
    ) -> Result<Self> {
        authorize(context, expected.as_ref(), None)?;
        if create && (mode == BuildMode::Offline || expected.is_none()) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "read-only monitoring cannot create a journal",
            ));
        }
        let mut work = Work::new(context)?;
        let mut raw = read(&mut storage, &mut work)?;
        let decoded = if create {
            require(raw.is_empty())?;
            let definition = expected.as_ref().ok_or_else(corrupt)?;
            raw = MAGIC.to_vec();
            raw.extend_from_slice(&frame(
                0,
                Digest32::from_bytes([0; 32]),
                1,
                definition.canonical_bytes(),
            )?);
            work.reserve(raw.len().saturating_mul(8))?;
            let checked = replay(&raw, &mut work)?;
            authorize(
                &work.current()?,
                Some(&checked.definition),
                Some(&checked.progress),
            )?;
            work.charge(raw.len())?;
            storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
            storage.write_all(&raw).map_err(|_| corrupt())?;
            work.current()?;
            storage
                .flush()
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
            require(read(&mut storage, &mut work)? == raw)?;
            checked
        } else {
            replay(&raw, &mut work)?
        };
        if let Some(expected) = expected {
            require(expected.canonical_bytes() == decoded.definition.canonical_bytes())?;
        }
        authorize(
            &work.current()?,
            Some(&decoded.definition),
            Some(&decoded.progress),
        )?;
        if !create && mode != BuildMode::Offline {
            // A complete surviving descendant may be the result of a lost
            // synchronization acknowledgement. Re-establish durable custody
            // before a writable recovery owner publishes it.
            work.current()?;
            storage
                .flush()
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
        }
        // Detect changed old bytes between replay and owner publication.
        require(read(&mut storage, &mut work)? == raw)?;
        let resume_reset = !create && !decoded.progress.terminal();
        let visible = if resume_reset {
            withdrawn(&decoded.progress)
        } else {
            decoded.progress.clone()
        };
        let observed_floor = decoded.progress.observed_tick_floor;
        Ok(Self {
            storage,
            definition: decoded.definition,
            durable_progress: decoded.progress,
            first_source: decoded.first_source,
            visible,
            resume_reset,
            raw,
            frames: decoded.frames,
            head: decoded.head,
            heads: decoded.heads,
            mode,
            owner: context.session_id,
            read_owned: false,
            fenced: false,
            observed_floor,
        })
    }
    pub fn definition(&self) -> &Definition {
        &self.definition
    }
    pub fn progress(&self) -> &Progress {
        &self.visible
    }
    /// Source of the first accepted sample, retained through later diagnostic
    /// source changes that terminally invalidate the monitor. An original
    /// clock uses this immutable association, never the replacement source.
    pub fn first_source(&self) -> Option<&SourceIdentity> {
        self.first_source.as_ref()
    }
    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub fn frames(&self) -> u32 {
        self.frames
    }
    pub fn authority_tick_floor(&self) -> u64 {
        self.observed_floor
    }
    /// Retain a clock already decoded by the owned native reader, including
    /// when the remainder of its bracket was lost. This only narrows future
    /// authority and source continuity: no evidence, streak, source identity,
    /// read permission or durable history is created by the latch itself.
    /// The next read intent or local cancellation synchronizes the floor.
    pub fn retain_observed_tick_floor(&mut self, tick: u64) {
        self.observed_floor = self.observed_floor.max(tick);
    }
    pub fn head(&self) -> Digest32 {
        self.head
    }
    /// Cached custody reference, not proof that the file is currently intact.
    pub fn recovery(&self) -> Recovery {
        Recovery {
            monitor_id: self.definition.id(),
            head: self.head,
        }
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub fn read_owned(&self) -> bool {
        self.read_owned && !self.fenced
    }
    /// A failed foreground acquisition cannot retain publication permission.
    /// Its already synchronized read intent remains unknown for the next read.
    pub fn abandon_read(&mut self) {
        self.read_owned = false;
        if self.durable_progress.reading {
            self.withdraw_unpublished();
        }
    }
    /// Withdraw an unfinished streak after enclosing runtime/publication failure
    /// or contradictory readiness evidence. No authority, I/O or history rewrite
    /// occurs here. Existing terminal historical evidence remains immutable.
    /// The next read durably records an interruption before source contact and
    /// cannot reuse this owner's previous publication permission.
    pub fn withdraw_unpublished(&mut self) {
        self.read_owned = false;
        if !self.durable_progress.terminal() {
            self.visible = withdrawn(&self.durable_progress);
            self.resume_reset = true;
        }
    }
    /// Conservative reservation for monitor I/O and semantic validation. Source
    /// guards and response bytes belong to the caller's separate reservations.
    pub fn operation_reserve(&self, sample: bool) -> u64 {
        (5 * self.raw.len() + if sample { 12 * MAX_SAMPLE } else { 0 } + 16_384) as u64
    }
    fn access(&self, context: &OperationContext) -> Result<OperationContext> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "joint room monitor has another session owner",
            ));
        }
        let mut context = context.clone();
        context.anchor.tick = GameTick(context.anchor.tick.get().max(self.observed_floor));
        let current = authorize(
            &context,
            Some(&self.definition),
            Some(&self.durable_progress),
        )?;
        if self.fenced {
            return Err(corrupt());
        }
        Ok(current)
    }
    fn writable(&self) -> Result<()> {
        if self.mode == BuildMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline joint room monitor cannot append",
            ));
        }
        Ok(())
    }
    fn verify_work(&mut self, work: &mut Work) -> Result<()> {
        self.access(&work.current()?)?;
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
                self.read_owned = false;
                Err(corrupt())
            }
        }
    }
    fn verify_inner(&mut self, context: &OperationContext) -> Result<()> {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)
    }
    /// Verify current exact custody against an externally acknowledged head.
    /// Ancestry comes from full semantic replay or this owner's synchronized
    /// appends. Verification writes no history and restores no authority or
    /// read ownership; changed custody retains the existing fail-closed fence.
    /// The existing `operation_reserve(false)` covers the bounded verification.
    fn verify_recovery_inner(
        &mut self,
        context: &OperationContext,
        expected: Recovery,
    ) -> Result<()> {
        let mut work = Work::new(&self.access(context)?)?;
        expected.validate()?;
        self.verify_work(&mut work)?;
        if expected.monitor_id != self.definition.id() || !self.heads.contains(&expected.head) {
            return Err(error(
                ErrorCode::StaleAnchor,
                "joint room monitor differs from its acknowledged recovery history",
            ));
        }
        work.current()?;
        Ok(())
    }
    fn verify_origin_inner<G>(&mut self, context: &OperationContext, guard: &mut G) -> Result<()>
    where
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)?;
        guard(self.definition.origin(), &work.current()?)?;
        work.current()?;
        Ok(())
    }
    fn append<G>(
        &mut self,
        kind: u8,
        payload: &[u8],
        proposed: Progress,
        work: &mut Work,
        guard: &mut G,
    ) -> Result<()>
    where
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        self.writable()?;
        let frame = frame(self.frames, self.head, kind, payload)?;
        if self.raw.len().saturating_add(frame.len()) > MAX_STORE_BYTES {
            return Err(bounded());
        }
        work.reserve(
            self.raw
                .len()
                .saturating_mul(2)
                .saturating_add(frame.len().saturating_mul(2)),
        )?;
        // A result callback may have performed arbitrary bounded work. Recheck
        // all retained bytes and original custody after it, before the append.
        self.verify_work(work)?;
        guard(self.definition.origin(), &work.current()?)?;
        authorize(&work.current()?, Some(&self.definition), Some(&proposed))?;
        let mut bytes = self.raw.clone();
        bytes.extend_from_slice(&frame);
        work.charge(frame.len())?;
        self.read_owned = false;
        self.fenced = true;
        self.storage.validate_identity().map_err(|_| corrupt())?;
        require(
            self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == self.raw.len() as u64,
        )?;
        self.storage.write_all(&frame).map_err(|_| corrupt())?;
        work.current()?;
        self.storage
            .flush()
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        require(read(&mut self.storage, work)? == bytes)?;
        guard(self.definition.origin(), &work.current()?)?;
        authorize(&work.current()?, Some(&self.definition), Some(&proposed))?;
        self.head = tail(&frame)?;
        self.heads.push(self.head);
        self.raw = bytes;
        self.frames += 1;
        if self.first_source.is_none() {
            self.first_source = proposed.source.clone();
        }
        self.visible = if self.resume_reset && !proposed.terminal() {
            withdrawn(&proposed)
        } else {
            proposed.clone()
        };
        self.observed_floor = self.observed_floor.max(proposed.observed_tick_floor);
        self.durable_progress = proposed;
        self.fenced = false;
        Ok(())
    }
    fn begin_read_inner<F, G>(
        &mut self,
        context: &OperationContext,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)?;
        guard(self.definition.origin(), &work.current()?)?;
        if self.durable_progress.terminal() {
            admit(&self.definition, &self.durable_progress)?;
            self.verify_work(&mut work)?;
            guard(self.definition.origin(), &work.current()?)?;
            work.current()?;
            return Ok(());
        }
        self.writable()?;
        if self.read_owned {
            return Err(error(
                ErrorCode::Conflict,
                "joint room read is already owned; finish or abandon that acquisition",
            ));
        }
        let reset_start = self.resume_reset && !self.durable_progress.reading;
        let reserve_frames = if reset_start { 4 } else { 3 };
        if self
            .raw
            .len()
            .saturating_add(MAX_SAMPLE + reserve_frames * (OVERHEAD + 8))
            > MAX_STORE_BYTES
            || self.frames.saturating_add(reserve_frames as u32) > MAX_STORE_FRAMES
        {
            return Err(bounded());
        }
        // Preserve the existing journal format: a second read intent is the
        // canonical interrupted-read transition. On restart or local withdrawal,
        // first persist an unowned intent if the prior history was not reading.
        // A crash between these two frames leaves an interrupted read, never a
        // reusable stability streak or permission to publish the old sample.
        if reset_start {
            let mut started = self.durable_progress.begin_read()?;
            started.observed_tick_floor = self.observed_floor;
            let reset = started.begin_read()?;
            admit(&self.definition, &reset)?;
            self.append(
                2,
                &self.observed_floor.to_be_bytes(),
                started,
                &mut work,
                guard,
            )?;
        }
        let mut proposed = self.durable_progress.begin_read()?;
        proposed.observed_tick_floor = self.observed_floor;
        admit(&self.definition, &proposed)?;
        self.append(
            2,
            &self.observed_floor.to_be_bytes(),
            proposed,
            &mut work,
            guard,
        )?;
        self.resume_reset = false;
        self.visible = self.durable_progress.clone();
        self.read_owned = true;
        Ok(())
    }
    fn publish_acquisition<F, G>(
        &mut self,
        context: &OperationContext,
        sample: &Sample,
        kind: u8,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        if !self.read_owned {
            return Err(error(
                ErrorCode::Conflict,
                "reopened or unowned joint room read cannot publish a sample",
            ));
        }
        self.read_owned = false;
        // Reserve bounded component decoding before touching any later shape
        // field: an empty trailing map must not hide an earlier consumed tick.
        work.charge(3 * MAX_SAMPLE)?;
        sample.retain_observed_tick(&mut self.observed_floor)?;
        work.context.anchor.tick =
            GameTick(work.context.anchor.tick.get().max(self.observed_floor));
        work.current()?;
        let payload = sample.canonical_bytes()?;
        work.charge(payload.len())?;
        self.verify_work(&mut work)?;
        guard(self.definition.origin(), &work.current()?)?;
        let proposed = transition(
            &self.definition,
            &self.durable_progress,
            kind,
            &payload,
            &mut work,
        )?;
        authorize(&work.current()?, Some(&self.definition), Some(&proposed))?;
        admit(&self.definition, &proposed)?;
        self.append(kind, &payload, proposed, &mut work, guard)
    }
    /// Cancels only this monitor. No original-source guard is consulted, so an
    /// active obligation can be retired when its original batch is unavailable.
    /// The caller must mark original custody unverified in that result.
    fn cancel_inner<F>(&mut self, context: &OperationContext, admit: &mut F) -> Result<()>
    where
        F: FnMut(&Definition, &Progress) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)?;
        if self.durable_progress.terminal() {
            admit(&self.definition, &self.durable_progress)?;
            self.verify_work(&mut work)?;
            return Ok(());
        }
        self.writable()?;
        let mut proposed = self.durable_progress.cancel();
        proposed.observed_tick_floor = self.observed_floor;
        admit(&self.definition, &proposed)?;
        self.append(
            4,
            &self.observed_floor.to_be_bytes(),
            proposed,
            &mut work,
            &mut |_, _| Ok(()),
        )
    }
}

/// Open operator-selected custody. An explicit definition permits Query-only
/// creation in Control or Recover; omitting it can only replay an existing file.
/// The caller reserves and verifies the complete result and original source
/// before creation, then repeats source verification before returning success.
pub fn open_private_monitor(
    path: &Path,
    context: &OperationContext,
    mode: BuildMode,
    expected: Option<Definition>,
) -> Result<MonitorStore<PrivateBuildFile>> {
    let context = authorize(context, expected.as_ref(), None)?;
    if expected.is_some() && mode == BuildMode::Offline {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "offline monitor cannot create a definition",
        ));
    }
    let allow_create = expected.is_some() && mode != BuildMode::Offline;
    let shell_mode = if allow_create {
        BuildMode::Control
    } else {
        mode
    };
    let (storage, created, current) =
        open_private_storage(path, &context, shell_mode, allow_create, MAX_STORE_BYTES)?;
    MonitorStore::open(storage, &current, mode, expected, created)
}
