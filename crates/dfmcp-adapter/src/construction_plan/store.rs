//! Durable whole-plan monitoring with write-ahead read intent.
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

use super::origin::{MAX_DEFINITION_SIZE, MonitorDefinition, Origin};
use super::{LinkedSample, MAX_SAMPLE, Progress};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{PrivateBuildFile, open_private_storage};
use crate::control_effect_journal::EffectJournalStorage;

pub const MAX_STORE_BYTES: usize = 128 * 1024 * 1024;
pub const MAX_STORE_FRAMES: u32 = 1030;
const MAGIC: &[u8; 8] = b"DFMFRJ01";
const DOMAIN: &[u8] = b"dfmcp.furniture-completion-rust-journal/1\0";
const HEADER: usize = 40;
const OVERHEAD: usize = HEADER + 1 + 32;
const MAX_BODY: usize = MAX_SAMPLE + 1;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "construction monitor custody or evidence changed; preserve and reopen original history without repair",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "construction monitor foreground or retention allowance exhausted",
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
    definition: Option<&MonitorDefinition>,
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
            "construction monitor belongs to another fortress",
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
    if let Some(tick) = progress.and_then(|value| value.last_tick) {
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
        if context.budget.max_wall_millis > 60_000 || context.budget.max_bytes > 1024 * 1024 * 1024
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
    require(sequence < MAX_STORE_FRAMES && (1..=4).contains(&kind) && payload.len() < MAX_BODY)?;
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
fn transition(
    definition: &MonitorDefinition,
    progress: &Progress,
    kind: u8,
    payload: &[u8],
    work: &mut Work,
) -> Result<Progress> {
    require(!progress.terminal())?;
    work.semantic_work = work.semantic_work.checked_add(1).ok_or_else(bounded)?;
    if work.semantic_work > super::MAX_WORK {
        return Err(bounded());
    }
    match kind {
        2 => {
            require(payload.is_empty())?;
            progress.begin_read()
        }
        3 => {
            // Parsing and all member evaluation share this one reservation;
            // no target or frame renews the enclosing operation allowance.
            let current = work.child(payload.len().saturating_mul(4).saturating_add(1024))?;
            let sample = LinkedSample::decode(payload)?;
            // Replay must enforce the same placement-time software binding as
            // live publication, including the first accepted sample.
            let binding = definition.origin().definition().binding();
            require(
                sample.before.df_version == binding.df_version()
                    && sample.before.dfhack_version == binding.dfhack_version(),
            )?;
            super::advance_with_counter(
                progress,
                definition.goal(),
                &sample,
                &current,
                &mut work.semantic_work,
            )
        }
        4 => {
            require(payload.is_empty())?;
            Ok(progress.cancel())
        }
        _ => Err(corrupt()),
    }
}
struct Replay {
    definition: MonitorDefinition,
    progress: Progress,
    frames: u32,
    head: Digest32,
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
        state = Some(if let Some(prior) = state {
            let progress = transition(&prior.definition, &prior.progress, kind, payload, work)?;
            Replay {
                definition: prior.definition,
                progress,
                frames: sequence + 1,
                head,
            }
        } else {
            require(kind == 1 && payload.len() <= MAX_DEFINITION_SIZE)?;
            work.charge(payload.len().saturating_mul(3))?;
            let definition = MonitorDefinition::decode(payload)?;
            let progress = Progress::new(definition.goal());
            Replay {
                definition,
                progress,
                frames: 1,
                head,
            }
        });
        offset = end;
    }
    state.ok_or_else(corrupt)
}

/// The storage is owned for the complete session. Current source custody is
/// checked through a caller-supplied guard using the same shrinking deadline;
/// the caller reserves its source-file I/O separately from this journal's I/O.
pub struct MonitorStore<S> {
    storage: S,
    definition: MonitorDefinition,
    progress: Progress,
    raw: Vec<u8>,
    frames: u32,
    head: Digest32,
    mode: BuildMode,
    owner: SessionId,
    read_owned: bool,
    fenced: bool,
}
impl<S: EffectJournalStorage> MonitorStore<S> {
    /// Query authority can create monitoring custody in Control or Recover.
    /// The file shell supplies create only for a new exclusively opened file.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: BuildMode,
        expected: Option<MonitorDefinition>,
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
        // Detect changed old bytes between replay and owner publication.
        require(read(&mut storage, &mut work)? == raw)?;
        Ok(Self {
            storage,
            definition: decoded.definition,
            progress: decoded.progress,
            raw,
            frames: decoded.frames,
            head: decoded.head,
            mode,
            owner: context.session_id,
            read_owned: false,
            fenced: false,
        })
    }
    pub fn definition(&self) -> &MonitorDefinition {
        &self.definition
    }
    pub fn progress(&self) -> &Progress {
        &self.progress
    }
    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub fn frames(&self) -> u32 {
        self.frames
    }
    pub fn head(&self) -> Digest32 {
        self.head
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
    }
    /// Conservative reservation for monitor I/O and semantic validation. Source
    /// guards and response bytes belong to the caller's separate reservations.
    pub fn operation_reserve(&self, sample: bool) -> u64 {
        (3 * self.raw.len() + if sample { 7 * MAX_SAMPLE } else { 0 } + 16_384) as u64
    }
    fn access(&self, context: &OperationContext) -> Result<OperationContext> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "construction monitor has another session owner",
            ));
        }
        let current = authorize(context, Some(&self.definition), Some(&self.progress))?;
        if self.fenced {
            return Err(corrupt());
        }
        Ok(current)
    }
    fn writable(&self) -> Result<()> {
        if self.mode == BuildMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline construction monitor cannot append",
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
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)
    }
    pub fn verify_origin<G>(&mut self, context: &OperationContext, guard: &mut G) -> Result<()>
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
        self.raw = bytes;
        self.frames += 1;
        self.progress = proposed;
        self.fenced = false;
        Ok(())
    }
    pub fn begin_read<F, G>(
        &mut self,
        context: &OperationContext,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&MonitorDefinition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)?;
        guard(self.definition.origin(), &work.current()?)?;
        if self.progress.terminal() {
            admit(&self.definition, &self.progress)?;
            self.verify_work(&mut work)?;
            guard(self.definition.origin(), &work.current()?)?;
            work.current()?;
            return Ok(());
        }
        self.writable()?;
        if self.read_owned {
            return Err(error(
                ErrorCode::Conflict,
                "construction read is already owned; finish or abandon that acquisition",
            ));
        }
        if self.raw.len().saturating_add(MAX_SAMPLE + 3 * OVERHEAD) > MAX_STORE_BYTES
            || self.frames.saturating_add(3) > MAX_STORE_FRAMES
        {
            return Err(bounded());
        }
        let proposed = self.progress.begin_read()?;
        admit(&self.definition, &proposed)?;
        self.append(2, &[], proposed, &mut work, guard)?;
        self.read_owned = true;
        Ok(())
    }
    pub fn publish_sample<F, G>(
        &mut self,
        context: &OperationContext,
        sample: &LinkedSample,
        admit: &mut F,
        guard: &mut G,
    ) -> Result<()>
    where
        F: FnMut(&MonitorDefinition, &Progress) -> Result<()>,
        G: FnMut(&Origin, &OperationContext) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        if !self.read_owned {
            return Err(error(
                ErrorCode::Conflict,
                "reopened or unowned construction read cannot publish a sample",
            ));
        }
        self.read_owned = false;
        self.verify_work(&mut work)?;
        guard(self.definition.origin(), &work.current()?)?;
        let payload = sample.canonical_bytes()?;
        work.charge(payload.len())?;
        let proposed = transition(&self.definition, &self.progress, 3, &payload, &mut work)?;
        authorize(&work.current()?, Some(&self.definition), Some(&proposed))?;
        admit(&self.definition, &proposed)?;
        self.append(3, &payload, proposed, &mut work, guard)
    }
    /// Cancels only this monitor. No original-source guard is consulted, so an
    /// active obligation can be retired when its original batch is unavailable.
    /// The caller must mark original custody unverified in that result.
    pub fn cancel<F>(&mut self, context: &OperationContext, admit: &mut F) -> Result<()>
    where
        F: FnMut(&MonitorDefinition, &Progress) -> Result<()>,
    {
        let mut work = Work::new(&self.access(context)?)?;
        self.verify_work(&mut work)?;
        if self.progress.terminal() {
            admit(&self.definition, &self.progress)?;
            self.verify_work(&mut work)?;
            return Ok(());
        }
        self.writable()?;
        let proposed = self.progress.cancel();
        admit(&self.definition, &proposed)?;
        self.append(4, &[], proposed, &mut work, &mut |_, _| Ok(()))
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
    expected: Option<MonitorDefinition>,
) -> Result<MonitorStore<PrivateBuildFile>> {
    authorize(context, expected.as_ref(), None)?;
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
        open_private_storage(path, context, shell_mode, allow_create, MAX_STORE_BYTES)?;
    MonitorStore::open(storage, &current, mode, expected, created)
}

#[cfg(test)]
mod tests;
