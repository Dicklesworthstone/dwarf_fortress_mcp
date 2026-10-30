//! Append-only custody of whole-blueprint observations and their fixed policy.
//!
//! Read intent is durable before native contact. Raw typed captures are retained
//! before newly learned clocks can invalidate authority; positive progress is
//! published only after a separate durable completion. No event grants native
//! mutation authority or replaces any original designation receipt.
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, GameTick, MapCuboid, OperationContext, RequestId,
    Result, RiskTier, SessionId,
};

use super::{
    MAX_ATTEMPTS, MAX_CAPTURE_BYTES, MAX_SPEC_BYTES, MonitorSpec, MonitorState, Status,
    decode_capture, encode_capture,
};
use crate::build_placement::journal::BuildMode;
use crate::build_placement::journal::private_file::{
    PrivateBuildFile, open_private_storage_scoped,
};
use crate::control_effect_journal::EffectJournalStorage;
use crate::dig_designation::journal::DigMode;
use crate::live_map::LiveMapObservation;

pub const MAX_STORE_BYTES: usize = 8 * 1024 * 1024;
pub const OPEN_RESERVE: u64 = 40 * 1024 * 1024;
pub const VERIFY_RESERVE: u64 = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 4 * MAX_ATTEMPTS as usize + 2;
const MAGIC: &[u8; 8] = b"DFMBGM01";
const HEADER_END: &[u8; 8] = b"DFMBGHE1";
const FRAME: &[u8; 8] = b"DFMBGF01";
const FRAME_END: &[u8; 8] = b"DFMBGFE1";
const HEADER_OVERHEAD: usize = 8 + 4 + 32 + 8;
const FRAME_OVERHEAD: usize = 8 + 4 + 4 + 32 + 32 + 8;
const MAX_FRAME_BYTES: usize = FRAME_OVERHEAD + 1 + MAX_CAPTURE_BYTES;

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn corrupt() -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "blueprint monitoring custody or history changed; retain the original bytes without repair",
    )
}
fn bounded() -> DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "blueprint monitoring history allowance exhausted",
    )
}
fn conflict(message: &str) -> DfmcpError {
    error(ErrorCode::Conflict, message)
}
fn require(value: bool) -> Result<()> {
    if value { Ok(()) } else { Err(corrupt()) }
}
fn hash(domain: &[u8], bytes: &[u8]) -> Digest32 {
    let mut data = Vec::with_capacity(domain.len() + bytes.len());
    data.extend_from_slice(domain);
    data.extend_from_slice(bytes);
    Digest32::of_bytes(&data)
}
fn retained(context: &OperationContext, tick: u64) -> OperationContext {
    let mut current = context.clone();
    current.anchor.tick = current.anchor.tick.max(GameTick(tick));
    current
}
fn query_candidate(context: &OperationContext) -> Result<()> {
    context.budget.validate()?;
    context
        .grants
        .iter()
        .filter(|grant| matches!(grant.capability, Capability::Query | Capability::Admin))
        .find_map(|grant| {
            context
                .authorize(
                    Capability::Query,
                    RiskTier::ReadOnly,
                    &[],
                    grant.scope.map_area,
                )
                .ok()
        })
        .ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "blueprint monitoring requires current scoped Query authority",
            )
        })
}

/// Externally retained immutable goal identity and acknowledged history head.
/// A complete descendant can recover a lost reply; a rollback cannot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Recovery {
    pub monitor_id: Digest32,
    pub head: Digest32,
}
impl Recovery {
    fn validate(self) -> Result<()> {
        if self.monitor_id == Digest32::ZERO || self.head == Digest32::ZERO {
            return Err(error(
                ErrorCode::InvalidRequest,
                "monitor recovery requires its original nonzero identity and acknowledged head",
            ));
        }
        Ok(())
    }
}

struct Work {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Work {
    fn new(context: &OperationContext) -> Result<Self> {
        query_candidate(context)?;
        if context.budget.max_wall_millis > 60_000 {
            return Err(bounded());
        }
        let work = Self {
            context: context.clone(),
            deadline: Instant::now()
                .checked_add(Duration::from_millis(context.budget.max_wall_millis))
                .ok_or_else(bounded)?,
            bytes: context.budget.max_bytes,
        };
        work.check()?;
        Ok(work)
    }
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(bounded());
        }
        query_candidate(&self.context)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.check()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(bounded)?;
        Ok(())
    }
    fn reserve(&self, bytes: u64) -> Result<()> {
        self.check()?;
        if self.bytes < bytes {
            Err(bounded())
        } else {
            Ok(())
        }
    }
}

fn read<S: EffectJournalStorage>(storage: &mut S, work: &mut Work) -> Result<Vec<u8>> {
    work.check()?;
    storage.validate_identity().map_err(|_| corrupt())?;
    let length = storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    require(length <= MAX_STORE_BYTES as u64)?;
    work.charge(length as usize)?;
    storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    let mut raw = Vec::new();
    raw.try_reserve_exact(length as usize)
        .map_err(|_| bounded())?;
    raw.resize(length as usize, 0);
    let mut offset = 0;
    while offset < raw.len() {
        work.check()?;
        let count = storage.read(&mut raw[offset..]).map_err(|_| corrupt())?;
        require(count > 0)?;
        offset += count;
    }
    work.check()?;
    require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == length)?;
    storage.validate_identity().map_err(|_| corrupt())?;
    work.check()?;
    Ok(raw)
}

fn header(spec: &MonitorSpec) -> Result<Vec<u8>> {
    let body = spec.canonical_bytes();
    require(!body.is_empty() && body.len() <= MAX_SPEC_BYTES)?;
    let mut raw = Vec::with_capacity(HEADER_OVERHEAD + body.len());
    raw.extend_from_slice(MAGIC);
    raw.extend_from_slice(&(body.len() as u32).to_be_bytes());
    raw.extend_from_slice(&body);
    let digest = hash(b"dfmcp-blueprint-monitor-header/1\0", &raw);
    raw.extend_from_slice(digest.as_bytes());
    raw.extend_from_slice(HEADER_END);
    Ok(raw)
}
fn header_head(raw: &[u8]) -> Digest32 {
    hash(b"dfmcp-blueprint-monitor-origin/1\0", raw)
}
/// Available before file publication so a lost create reply retains identity.
pub fn initial_recovery(spec: &MonitorSpec) -> Result<Recovery> {
    Ok(Recovery {
        monitor_id: spec.id(),
        head: header_head(&header(spec)?),
    })
}

enum Event {
    Started(u64),
    Captured(Vec<u8>),
    Published,
    Interrupted { changed: bool, tick: u64 },
    Cancelled(u64),
}
impl Event {
    fn body(&self) -> Vec<u8> {
        match self {
            Self::Started(tick) => {
                let mut body = vec![1];
                body.extend_from_slice(&tick.to_be_bytes());
                body
            }
            Self::Captured(raw) => {
                let mut body = Vec::with_capacity(raw.len() + 1);
                body.push(2);
                body.extend_from_slice(raw);
                body
            }
            Self::Published => vec![3],
            Self::Interrupted { changed, tick } => {
                let mut body = vec![4, u8::from(*changed)];
                body.extend_from_slice(&tick.to_be_bytes());
                body
            }
            Self::Cancelled(tick) => {
                let mut body = vec![5];
                body.extend_from_slice(&tick.to_be_bytes());
                body
            }
        }
    }
    fn decode(body: &[u8]) -> Result<Self> {
        match body {
            [1, rest @ ..] if rest.len() == 8 => Ok(Self::Started(u64::from_be_bytes(
                rest.try_into().map_err(|_| corrupt())?,
            ))),
            [2, rest @ ..] if !rest.is_empty() && rest.len() <= MAX_CAPTURE_BYTES => {
                Ok(Self::Captured(rest.to_vec()))
            }
            [3] => Ok(Self::Published),
            [4, changed @ 0..=1, rest @ ..] if rest.len() == 8 => Ok(Self::Interrupted {
                changed: *changed == 1,
                tick: u64::from_be_bytes(rest.try_into().map_err(|_| corrupt())?),
            }),
            [5, rest @ ..] if rest.len() == 8 => Ok(Self::Cancelled(u64::from_be_bytes(
                rest.try_into().map_err(|_| corrupt())?,
            ))),
            _ => Err(corrupt()),
        }
    }
}
fn frame(event: &Event, previous: Digest32, index: usize) -> Result<Vec<u8>> {
    require(index > 0 && index <= MAX_EVENTS)?;
    let body = event.body();
    require(!body.is_empty() && body.len() <= 1 + MAX_CAPTURE_BYTES)?;
    let mut raw = Vec::with_capacity(FRAME_OVERHEAD + body.len());
    raw.extend_from_slice(FRAME);
    raw.extend_from_slice(&(body.len() as u32).to_be_bytes());
    raw.extend_from_slice(&(index as u32).to_be_bytes());
    raw.extend_from_slice(previous.as_bytes());
    raw.extend_from_slice(&body);
    let digest = hash(b"dfmcp-blueprint-monitor-event/1\0", &raw);
    raw.extend_from_slice(digest.as_bytes());
    raw.extend_from_slice(FRAME_END);
    Ok(raw)
}
fn frame_head(raw: &[u8]) -> Digest32 {
    hash(b"dfmcp-blueprint-monitor-link/1\0", raw)
}

struct Replay {
    state: MonitorState,
    high_tick: u64,
    candidate: Option<MonitorState>,
    head: Digest32,
    heads: Vec<Digest32>,
    events: usize,
}
impl Replay {
    fn apply(&mut self, event: Event) -> Result<()> {
        require(!self.state.status().terminal())?;
        match event {
            Event::Started(tick) => {
                require(
                    !self.state.pending() && self.candidate.is_none() && tick >= self.high_tick,
                )?;
                self.state = self.state.begin_attempt().map_err(|_| corrupt())?;
                self.high_tick = tick;
            }
            Event::Captured(raw) => {
                require(self.state.pending() && self.candidate.is_none())?;
                let capture = decode_capture(&raw).map_err(|_| corrupt())?;
                let tick = capture.tick().get();
                let candidate = if tick < self.high_tick {
                    self.state.invalidate()
                } else {
                    self.state.sample(capture)
                }
                .map_err(|_| corrupt())?;
                self.high_tick = self.high_tick.max(tick);
                self.candidate = Some(candidate);
            }
            Event::Published => {
                require(self.state.pending())?;
                self.state = self.candidate.take().ok_or_else(corrupt)?;
            }
            Event::Interrupted { changed, tick } => {
                require(tick >= self.high_tick)?;
                self.high_tick = tick;
                // A retained source change or expired observation cannot become
                // a retryable gap merely because a later publication failed.
                let candidate = self.candidate.take();
                self.state = if let Some(negative) = candidate
                    .filter(|state| matches!(state.status(), Status::Invalidated | Status::Expired))
                {
                    negative
                } else if changed {
                    self.state.invalidate().map_err(|_| corrupt())?
                } else {
                    self.state.interrupt().map_err(|_| corrupt())?
                };
            }
            Event::Cancelled(tick) => {
                require(tick >= self.high_tick)?;
                self.high_tick = tick;
                self.state =
                    if let Some(negative) = self.candidate.take().filter(|state| {
                        matches!(state.status(), Status::Invalidated | Status::Expired)
                    }) {
                        negative
                    } else {
                        self.state.cancel().map_err(|_| corrupt())?
                    };
            }
        }
        self.high_tick = self.high_tick.max(self.state.high_tick());
        Ok(())
    }
    fn pending(&self) -> bool {
        self.state.pending()
    }
    fn safe_pending_state(&self) -> Result<MonitorState> {
        if let Some(negative) = self
            .candidate
            .as_ref()
            .filter(|state| matches!(state.status(), Status::Invalidated | Status::Expired))
        {
            Ok(negative.clone())
        } else {
            self.state.reopened()
        }
    }
}
fn u32_at(raw: &[u8], start: usize) -> Result<usize> {
    Ok(u32::from_be_bytes(
        raw.get(start..start + 4)
            .ok_or_else(corrupt)?
            .try_into()
            .map_err(|_| corrupt())?,
    ) as usize)
}
fn decode(raw: &[u8]) -> Result<Replay> {
    require(raw.len() >= HEADER_OVERHEAD && raw.len() <= MAX_STORE_BYTES)?;
    require(raw.get(..8) == Some(MAGIC.as_slice()))?;
    let length = u32_at(raw, 8)?;
    require(length > 0 && length <= MAX_SPEC_BYTES)?;
    let spec = MonitorSpec::decode(raw.get(12..12 + length).ok_or_else(corrupt)?)
        .map_err(|_| corrupt())?;
    let expected = header(&spec)?;
    require(raw.get(..expected.len()) == Some(expected.as_slice()))?;
    let mut replay = Replay {
        high_tick: spec.start_tick(),
        state: MonitorState::new(spec),
        candidate: None,
        head: header_head(&expected),
        heads: vec![header_head(&expected)],
        events: 0,
    };
    let mut offset = expected.len();
    while offset < raw.len() {
        require(replay.events < MAX_EVENTS)?;
        let remaining = raw.get(offset..).ok_or_else(corrupt)?;
        require(remaining.len() >= FRAME_OVERHEAD && remaining.get(..8) == Some(FRAME.as_slice()))?;
        let length = u32_at(remaining, 8)?;
        require(length > 0 && length <= 1 + MAX_CAPTURE_BYTES)?;
        let end = FRAME_OVERHEAD.checked_add(length).ok_or_else(corrupt)?;
        let encoded = remaining.get(..end).ok_or_else(corrupt)?;
        require(u32_at(encoded, 12)? == replay.events + 1)?;
        require(encoded.get(16..48) == Some(replay.head.as_bytes().as_slice()))?;
        let event = Event::decode(encoded.get(48..48 + length).ok_or_else(corrupt)?)?;
        require(frame(&event, replay.head, replay.events + 1)? == encoded)?;
        replay.apply(event)?;
        replay.head = frame_head(encoded);
        replay.heads.push(replay.head);
        replay.events += 1;
        offset += end;
    }
    Ok(replay)
}

fn opening(
    context: &OperationContext,
    mode: DigMode,
    expected: Option<&MonitorSpec>,
    create: bool,
    recovery: Option<Recovery>,
) -> Result<()> {
    query_candidate(context)?;
    if create {
        if mode == DigMode::Offline || expected.is_none() || recovery.is_some() {
            return Err(error(
                ErrorCode::InvalidRequest,
                "new monitor history requires writable custody, its full specification and no recovery pin",
            ));
        }
        expected.ok_or_else(corrupt)?.authorize(context, true)?;
    } else {
        let recovery = recovery.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "existing monitor history requires its original recovery pin",
            )
        })?;
        recovery.validate()?;
        if expected.is_some_and(|spec| spec.id() != recovery.monitor_id) {
            return Err(conflict(
                "monitor specification and recovery identity differ",
            ));
        }
    }
    Ok(())
}

/// Opaque live read ownership. This value cannot be reconstructed from history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attempt {
    monitor_id: Digest32,
    started: Digest32,
    owner: SessionId,
    request: RequestId,
    number: u32,
}
struct Active {
    token: Attempt,
    work: Work,
}

/// One exclusively held history. Cache accessors are historical metadata;
/// callers must `verify` before disclosing or combining them with other files.
pub struct MonitorStore<S> {
    storage: S,
    raw: Vec<u8>,
    replay: Replay,
    visible: MonitorState,
    high_tick: u64,
    mode: DigMode,
    owner: SessionId,
    active: Option<Active>,
    resume_reset: bool,
    sampling_fenced: bool,
    custody_fenced: bool,
}
pub type PrivateMonitorStore = MonitorStore<PrivateBuildFile>;

impl<S: EffectJournalStorage> MonitorStore<S> {
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: DigMode,
        expected: Option<MonitorSpec>,
        create: bool,
        recovery: Option<Recovery>,
    ) -> Result<Self> {
        opening(context, mode, expected.as_ref(), create, recovery)?;
        let current = retained(
            context,
            expected.as_ref().map_or(0, MonitorSpec::start_tick),
        );
        let mut work = Work::new(&current)?;
        let mut raw = read(&mut storage, &mut work)?;
        if create {
            require(raw.is_empty())?;
            let spec = expected.as_ref().ok_or_else(corrupt)?;
            raw = header(spec)?;
            work.reserve(raw.len() as u64 * 9)?;
            work.charge(raw.len() * 3)?;
            let decoded = decode(&raw)?;
            require(decoded.state.spec().canonical_bytes() == spec.canonical_bytes())?;
            spec.authorize(&work.context, true)?;
            work.check()?;
            storage.validate_identity().map_err(|_| corrupt())?;
            require(storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == 0)?;
            storage.write_all(&raw).map_err(|_| corrupt())?;
            work.check()?;
            storage
                .flush()
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
            require(read(&mut storage, &mut work)? == raw)?;
        }
        work.charge(raw.len() * 3)?;
        let replay = decode(&raw)?;
        if let Some(expected) = expected {
            require(replay.state.spec().canonical_bytes() == expected.canonical_bytes())?;
        }
        if let Some(recovery) = recovery {
            require(
                recovery.monitor_id == replay.state.spec().id()
                    && replay.heads.contains(&recovery.head),
            )?;
        }
        work.context = retained(&work.context, replay.high_tick);
        replay.state.spec().authorize(&work.context, false)?;
        if !create && mode != DigMode::Offline {
            work.reserve(raw.len() as u64)?;
            work.check()?;
            storage.validate_identity().map_err(|_| corrupt())?;
            storage.sync().map_err(|_| corrupt())?;
            require(read(&mut storage, &mut work)? == raw)?;
        }
        work.check()?;
        let sampling_fenced = !create && replay.pending();
        let resume_reset = !create && !replay.state.status().terminal() && !sampling_fenced;
        let visible = if sampling_fenced {
            replay.safe_pending_state()?
        } else if !create {
            replay.state.reopened()?
        } else {
            replay.state.clone()
        };
        Ok(Self {
            storage,
            raw,
            high_tick: replay.high_tick,
            replay,
            visible,
            mode,
            owner: context.session_id,
            active: None,
            resume_reset,
            sampling_fenced,
            custody_fenced: false,
        })
    }
    pub fn spec(&self) -> &MonitorSpec {
        self.replay.state.spec()
    }
    pub fn state(&self) -> &MonitorState {
        &self.visible
    }
    pub fn progress(&self) -> &MonitorState {
        self.state()
    }
    pub fn head(&self) -> Digest32 {
        self.replay.head
    }
    pub fn recovery(&self) -> Recovery {
        Recovery {
            monitor_id: self.spec().id(),
            head: self.head(),
        }
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn byte_len(&self) -> usize {
        self.raw.len()
    }
    pub fn is_fenced(&self) -> bool {
        self.sampling_fenced || self.custody_fenced || self.active.is_some()
    }
    /// Withdraw unfinished derived progress after an enclosing runtime or
    /// response publication fails. This local operation needs no authority or
    /// I/O: it preserves terminal historical evidence and held read ownership.
    /// A subsequent attempt must durably reset continuity before source contact.
    /// An active attempt may still retain already consumed bytes, but cannot
    /// publish them after this withdrawal; interruption or cancellation settles it.
    pub fn withdraw_unpublished(&mut self) -> Result<()> {
        self.visible = self.replay.safe_pending_state()?;
        if self.replay.pending() || self.active.is_some() {
            self.sampling_fenced = true;
        } else if !self.replay.state.status().terminal() {
            self.resume_reset = true;
        }
        Ok(())
    }
    /// Whole begin/capture/publication allowance, including rereads, replay,
    /// possible restart reset, failure completion and bounded response space.
    /// The owner retains one absolute deadline and allowance across the token.
    pub fn operation_reserve(&self) -> u64 {
        ((self.raw.len() + MAX_FRAME_BYTES + 4 * FRAME_OVERHEAD) as u64)
            .saturating_mul(32)
            .saturating_add(256 * 1024)
    }
    fn access(&self, context: &OperationContext, observe: bool) -> Result<OperationContext> {
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "monitor history has another session owner",
            ));
        }
        if self.custody_fenced {
            return Err(corrupt());
        }
        let current = retained(context, self.high_tick);
        self.spec().authorize(&current, observe)?;
        Ok(current)
    }
    fn writable(&self) -> Result<()> {
        if self.mode == DigMode::Offline {
            Err(error(
                ErrorCode::CapabilityDenied,
                "offline monitor history cannot append",
            ))
        } else {
            Ok(())
        }
    }
    fn verify_work(&mut self, work: &mut Work) -> Result<()> {
        if self.custody_fenced {
            return Err(corrupt());
        }
        match read(&mut self.storage, work) {
            Ok(raw) if raw == self.raw => Ok(()),
            Err(cause)
                if matches!(
                    cause.code,
                    ErrorCode::BudgetExceeded | ErrorCode::CancellationRequested
                ) =>
            {
                Err(cause)
            }
            _ => {
                self.custody_fenced = true;
                Err(corrupt())
            }
        }
    }
    pub fn verify(&mut self, context: &OperationContext) -> Result<()> {
        let current = self.access(context, false)?;
        self.verify_work(&mut Work::new(&current)?)
    }
    fn append(&mut self, event: Event, work: &mut Work) -> Result<()> {
        self.writable()?;
        self.verify_work(work)?;
        let encoded = frame(&event, self.replay.head, self.replay.events + 1)?;
        let next_len = self
            .raw
            .len()
            .checked_add(encoded.len())
            .ok_or_else(bounded)?;
        if next_len > MAX_STORE_BYTES {
            return Err(bounded());
        }
        work.reserve((next_len * 5 + encoded.len()) as u64)?;
        work.charge(next_len * 3 + encoded.len())?;
        let mut proposed = Vec::new();
        proposed
            .try_reserve_exact(next_len)
            .map_err(|_| bounded())?;
        proposed.extend_from_slice(&self.raw);
        proposed.extend_from_slice(&encoded);
        let replay = decode(&proposed)?;
        require(replay.state.spec().canonical_bytes() == self.spec().canonical_bytes())?;
        work.check()?;
        self.custody_fenced = true;
        self.storage.validate_identity().map_err(|_| corrupt())?;
        require(
            self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == self.raw.len() as u64,
        )?;
        self.storage.write_all(&encoded).map_err(|_| corrupt())?;
        work.check()?;
        self.storage
            .flush()
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        require(read(&mut self.storage, work)? == proposed)?;
        work.check()?;
        self.raw = proposed;
        self.high_tick = self.high_tick.max(replay.high_tick);
        self.visible = replay.state.clone();
        self.replay = replay;
        self.custody_fenced = false;
        Ok(())
    }
    pub fn begin_attempt(&mut self, context: &OperationContext) -> Result<Attempt> {
        let current = self.access(context, true)?;
        self.writable()?;
        if self.sampling_fenced || self.active.is_some() {
            return Err(conflict(
                "unfinished monitor read requires verified inspection or cancellation",
            ));
        }
        self.visible.begin_attempt()?;
        // Prepay the largest capture, completion, optional restart reset and a
        // separate cancellation before durable intent or any native contact.
        if self.replay.events + 4 > MAX_EVENTS
            || self.raw.len() + MAX_FRAME_BYTES + 3 * (FRAME_OVERHEAD + 10) > MAX_STORE_BYTES
        {
            return Err(bounded());
        }
        let mut work = Work::new(&current)?;
        work.reserve(self.operation_reserve())?;
        self.verify_work(&mut work)?;
        if self.resume_reset {
            self.append(
                Event::Interrupted {
                    changed: false,
                    tick: current.anchor.tick.get(),
                },
                &mut work,
            )?;
            self.resume_reset = false;
        }
        self.append(Event::Started(current.anchor.tick.get()), &mut work)?;
        let token = Attempt {
            monitor_id: self.spec().id(),
            started: self.head(),
            owner: self.owner,
            request: context.request_id,
            number: self.replay.state.attempts(),
        };
        self.active = Some(Active {
            token: token.clone(),
            work,
        });
        Ok(token)
    }
    fn take_active(&mut self, token: &Attempt, context: &OperationContext) -> Result<Active> {
        if self.custody_fenced {
            return Err(corrupt());
        }
        let active = self
            .active
            .as_ref()
            .ok_or_else(|| conflict("monitor read has no live owner"))?;
        if &active.token != token
            || context.session_id != token.owner
            || context.request_id != token.request
        {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "monitor read belongs to another owner or request",
            ));
        }
        self.active.take().ok_or_else(corrupt)
    }
    /// Persist already consumed typed bytes under the original read allowance.
    /// New-tick Query/Observe checks belong to publication, after this evidence
    /// exists. Even a failed append retains the learned floor in this owner.
    pub fn capture(
        &mut self,
        token: &Attempt,
        capture: LiveMapObservation,
        context: &OperationContext,
    ) -> Result<()> {
        let mut active = self.take_active(token, context)?;
        let previously_fenced = self.sampling_fenced;
        let result = (|| {
            if self.replay.candidate.is_some() {
                return Err(conflict("monitor read already has a capture"));
            }
            capture.validate()?;
            self.high_tick = self.high_tick.max(capture.tick().get());
            self.sampling_fenced = true;
            let raw = encode_capture(&capture)?;
            self.append(Event::Captured(raw), &mut active.work)?;
            self.visible = self.replay.safe_pending_state()?;
            self.sampling_fenced = previously_fenced;
            Ok(())
        })();
        if !self.custody_fenced {
            self.active = Some(active);
        }
        result
    }
    pub fn publish(&mut self, token: &Attempt, context: &OperationContext) -> Result<()> {
        let mut active = self.take_active(token, context)?;
        let result = (|| {
            if self.sampling_fenced {
                return Err(conflict(
                    "monitor read cannot publish after an incomplete capture",
                ));
            }
            self.access(context, true)?;
            if self.replay.candidate.is_none() {
                return Err(conflict("monitor read has no retained capture"));
            }
            self.append(Event::Published, &mut active.work)?;
            self.access(context, true)?;
            Ok(())
        })();
        if result.is_err() {
            self.sampling_fenced = true;
            self.visible = self.replay.safe_pending_state()?;
            if !self.custody_fenced {
                self.active = Some(active);
            }
        }
        result
    }
    fn finish_gap(
        &mut self,
        token: &Attempt,
        context: &OperationContext,
        changed: bool,
    ) -> Result<()> {
        let mut active = self.take_active(token, context)?;
        let result = (|| {
            let current = self.access(context, false)?;
            let changed = changed || (self.sampling_fenced && self.replay.candidate.is_none());
            self.append(
                Event::Interrupted {
                    changed,
                    tick: current.anchor.tick.get(),
                },
                &mut active.work,
            )?;
            self.sampling_fenced = false;
            Ok(())
        })();
        if result.is_err() {
            self.sampling_fenced = true;
            self.visible = self.replay.safe_pending_state()?;
            if !self.custody_fenced {
                self.active = Some(active);
            }
        }
        result
    }
    pub fn interrupt(&mut self, token: &Attempt, context: &OperationContext) -> Result<()> {
        self.finish_gap(token, context, false)
    }
    pub fn invalidate(&mut self, token: &Attempt, context: &OperationContext) -> Result<()> {
        self.finish_gap(token, context, true)
    }
    /// Local read cancellation needs current Query only, even if the original
    /// native journal or parent can no longer be opened. It dispatches nothing.
    pub fn cancel(&mut self, context: &OperationContext) -> Result<()> {
        let current = self.access(context, false)?;
        self.writable()?;
        let mut work = Work::new(&current)?;
        work.reserve(self.operation_reserve())?;
        self.verify_work(&mut work)?;
        if self.replay.state.status().terminal() {
            return Ok(());
        }
        self.append(Event::Cancelled(current.anchor.tick.get()), &mut work)?;
        self.active = None;
        self.resume_reset = false;
        self.sampling_fenced = false;
        Ok(())
    }
}

/// Exclusively opens the operator-selected private path. File access mode
/// grants no game capability: Recover can register this read-only obligation
/// using its existing Query/Observe grants; Offline never creates or appends.
pub fn open_private_monitor_store(
    path: &Path,
    context: &OperationContext,
    mode: DigMode,
    expected: Option<MonitorSpec>,
    create: bool,
    recovery: Option<Recovery>,
    scope: MapCuboid,
) -> Result<PrivateMonitorStore> {
    MapCuboid::new(scope.min, scope.max)?;
    opening(context, mode, expected.as_ref(), create, recovery)?;
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], Some(scope))?;
    if let Some(spec) = expected.as_ref() {
        require(scope == spec.parent().binding().scope())?;
        if create {
            let work = Work::new(&retained(context, spec.start_tick()))?;
            work.reserve(header(spec)?.len() as u64 * 9)?;
        }
    }
    let file_mode = if create {
        BuildMode::Control
    } else {
        match mode {
            DigMode::Control => BuildMode::Control,
            DigMode::Recover => BuildMode::Recover,
            DigMode::Offline => BuildMode::Offline,
        }
    };
    let (storage, created, current) = open_private_storage_scoped(
        path,
        context,
        file_mode,
        create,
        MAX_STORE_BYTES,
        Some(scope),
    )?;
    if created != create {
        return Err(conflict(
            "monitor initialization and existing private custody differ",
        ));
    }
    let store = MonitorStore::open(storage, &current, mode, expected, create, recovery)?;
    require(scope == store.spec().parent().binding().scope())?;
    Ok(store)
}

#[cfg(test)]
mod tests;
