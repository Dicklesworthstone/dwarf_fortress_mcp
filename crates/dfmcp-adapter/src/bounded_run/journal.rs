//! Append-only, source-bound bounded-run coordination. Never reconstructs authority.
use super::rpc::{RunManifest, RunSource, authorize, authorize_plan};
use super::{Reader, RunObservation, RunPhase, RunPlan, RunRecord, error, hash, require};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};
use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::net::SocketAddr;
use std::time::Instant;

pub const MAX_JOURNAL_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_RUNS: usize = 256;
pub const MAX_TRANSITIONS: u64 = 4096;
pub const RPC_RESERVE_BYTES: u64 = 270_336;
const MAX_BODY: usize = 452;
const FRAME_OVERHEAD: usize = 92;

fn corrupt(message: &str) -> dfmcp_core::DfmcpError {
    error(ErrorCode::CorruptLedger, message)
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    corrupt("run journal I/O/custody failed; reopen without dispatch replay")
}
fn uncertain() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "run outcome requires query/cancel reconciliation; never repeat unpause",
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunMode {
    Control,
    Recover,
    Offline,
}
impl RunMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Recover => "recover",
            Self::Offline => "offline",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunBinding {
    endpoint: SocketAddr,
    manifest: RunManifest,
}
impl RunBinding {
    pub fn new(endpoint: SocketAddr, manifest: RunManifest) -> Result<Self> {
        manifest.validate()?;
        require(
            endpoint.ip().is_loopback() && endpoint.port() != 0,
            "run binding must be numeric loopback",
        )?;
        Ok(Self { endpoint, manifest })
    }
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_text(&mut out, &self.endpoint.to_string());
        out.extend_from_slice(&self.manifest.generation.to_be_bytes());
        put_text(&mut out, &self.manifest.df_version);
        put_text(&mut out, &self.manifest.dfhack_version);
        out
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        let text = read_text(&mut r)?;
        let endpoint = text
            .parse::<SocketAddr>()
            .map_err(|_| corrupt("invalid run endpoint"))?;
        require(endpoint.to_string() == text, "noncanonical run endpoint")?;
        let manifest = RunManifest {
            generation: r.u64()?,
            df_version: read_text(&mut r)?,
            dfhack_version: read_text(&mut r)?,
        };
        r.finish()?;
        Self::new(endpoint, manifest)
    }
}
fn put_text(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(&(text.len() as u16).to_be_bytes());
    out.extend_from_slice(text.as_bytes());
}
fn read_text(r: &mut Reader<'_>) -> Result<String> {
    let n = usize::from(u16::from_be_bytes(r.array()?));
    require((1..=128).contains(&n), "invalid journal text bound")?;
    let value = std::str::from_utf8(r.take(n)?).map_err(|_| corrupt("invalid journal UTF-8"))?;
    require(
        !value.chars().any(char::is_control),
        "control character in journal identity",
    )?;
    Ok(value.to_owned())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum RunState {
    Intent = 0,
    Prepared = 1,
    DispatchStarted = 2,
    Tracking = 3,
    Terminal = 4,
    CancelRequested = 5,
    CancelledBeforeDispatch = 6,
}
impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent_recorded",
            Self::Prepared => "prepared",
            Self::DispatchStarted => "dispatch_started",
            Self::Tracking => "tracking",
            Self::Terminal => "terminal",
            Self::CancelRequested => "cancel_requested",
            Self::CancelledBeforeDispatch => "cancelled_before_dispatch",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableRun {
    plan: RunPlan,
    state: RunState,
    native: Option<RunRecord>,
}
impl DurableRun {
    pub fn plan(&self) -> &RunPlan {
        &self.plan
    }
    pub fn state(&self) -> RunState {
        self.state
    }
    pub fn native(&self) -> Option<&RunRecord> {
        self.native.as_ref()
    }
    pub fn unresolved(&self) -> bool {
        matches!(
            self.state,
            RunState::DispatchStarted | RunState::Tracking | RunState::CancelRequested
        ) || self
            .native
            .as_ref()
            .is_some_and(|r| r.phase() == RunPhase::SourceLost)
    }
    pub fn terminal(&self) -> bool {
        matches!(
            self.state,
            RunState::Terminal | RunState::CancelledBeforeDispatch
        )
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = vec![self.state as u8];
        let plan = self.plan.canonical_bytes();
        out.extend_from_slice(&(plan.len() as u16).to_be_bytes());
        out.extend_from_slice(&plan);
        let native = self
            .native
            .as_ref()
            .map_or(&[][..], |r| r.canonical_bytes());
        out.extend_from_slice(&(native.len() as u16).to_be_bytes());
        out.extend_from_slice(native);
        out
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        require(bytes.len() <= MAX_BODY, "run journal record exceeds bound")?;
        let mut r = Reader(bytes);
        let state = match r.byte()? {
            0 => RunState::Intent,
            1 => RunState::Prepared,
            2 => RunState::DispatchStarted,
            3 => RunState::Tracking,
            4 => RunState::Terminal,
            5 => RunState::CancelRequested,
            6 => RunState::CancelledBeforeDispatch,
            _ => return Err(corrupt("unknown durable run state")),
        };
        let n = usize::from(u16::from_be_bytes(r.array()?));
        let plan = RunPlan::decode(r.take(n)?)?;
        let n = usize::from(u16::from_be_bytes(r.array()?));
        let native = if n == 0 {
            None
        } else {
            Some(RunRecord::decode(r.take(n)?)?)
        };
        r.finish()?;
        let value = Self {
            plan,
            state,
            native,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<()> {
        require(
            self.native.as_ref().is_none_or(|r| r.plan() == &self.plan),
            "durable/native run identities disagree",
        )?;
        let phase = self.native.as_ref().map(RunRecord::phase);
        let valid = match self.state {
            RunState::Intent | RunState::CancelledBeforeDispatch => self.native.is_none(),
            RunState::Prepared => phase == Some(RunPhase::Prepared),
            RunState::DispatchStarted => phase == Some(RunPhase::Prepared),
            RunState::Tracking => phase.is_some_and(|p| !p.terminal()),
            RunState::Terminal => phase.is_some_and(RunPhase::terminal),
            RunState::CancelRequested => true,
        };
        require(
            valid,
            "durable run state lacks corresponding native evidence",
        )
    }
}
fn transition(old: Option<&DurableRun>, next: &DurableRun) -> Result<()> {
    next.validate()?;
    let Some(old) = old else {
        return require(
            next.state == RunState::Intent,
            "first run transition must retain intent",
        );
    };
    require(old.plan == next.plan, "run key cannot change intent")?;
    require(!old.terminal(), "terminal run evidence is immutable")?;
    let allowed = match next.state {
        RunState::Intent => false,
        RunState::Prepared => matches!(old.state, RunState::Intent | RunState::Prepared),
        RunState::DispatchStarted => old.state == RunState::Prepared,
        RunState::Tracking | RunState::Terminal => true,
        RunState::CancelRequested => !matches!(old.state, RunState::Intent | RunState::Prepared),
        RunState::CancelledBeforeDispatch => {
            matches!(old.state, RunState::Intent | RunState::Prepared)
        }
    };
    require(allowed, "illegal durable run transition or dispatch replay")?;
    if let (Some(before), Some(after)) = (&old.native, &next.native) {
        let allowed = match before.phase() {
            RunPhase::Prepared => true,
            RunPhase::Running => matches!(
                after.phase(),
                RunPhase::Running | RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost
            ),
            RunPhase::Stopping => matches!(
                after.phase(),
                RunPhase::Stopping | RunPhase::Stopped | RunPhase::SourceLost
            ),
            _ => before == after,
        };
        require(allowed, "native run evidence regressed")?;
    }
    Ok(())
}

struct Allowance {
    started: Instant,
    context: OperationContext,
    left: u64,
}
impl Allowance {
    fn new(context: &OperationContext) -> Result<Self> {
        authorize(context, false)?;
        if context.budget.max_wall_millis > 60_000 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "run foreground work is bounded to 60000 milliseconds",
            ));
        }
        Ok(Self {
            started: Instant::now(),
            context: context.clone(),
            left: context.budget.max_bytes,
        })
    }
    fn remaining(&self) -> Result<OperationContext> {
        let elapsed = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let mut c = self.context.clone();
        c.budget.max_bytes = self.left;
        c.budget.max_wall_millis = c
            .budget
            .max_wall_millis
            .checked_sub(elapsed)
            .filter(|n| *n > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "run foreground deadline exhausted",
                )
            })?;
        c.budget.validate()?;
        Ok(c)
    }
    fn charge(&mut self, n: u64) -> Result<()> {
        self.left = self.left.checked_sub(n).filter(|n| *n > 0).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "run work byte allowance exhausted",
            )
        })?;
        self.remaining().map(|_| ())
    }
    fn rpc(&mut self) -> Result<OperationContext> {
        self.charge(RPC_RESERVE_BYTES)?;
        let mut c = self.remaining()?;
        c.budget.max_bytes = RPC_RESERVE_BYTES;
        Ok(c)
    }
}

pub struct RunJournal<S> {
    storage: S,
    binding: RunBinding,
    id: Digest32,
    head: Digest32,
    frames: u64,
    bytes: Vec<u8>,
    records: BTreeMap<String, DurableRun>,
    owner: SessionId,
    mode: RunMode,
    fenced: bool,
}
impl<S: EffectJournalStorage> RunJournal<S> {
    /// initialize is accepted ONLY for newly created storage in Control mode.
    /// Existing files, including empty files, are never overwritten or repaired.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: RunMode,
        binding: Option<RunBinding>,
        initialize: bool,
    ) -> Result<Self> {
        let mut allowance = Allowance::new(context)?;
        storage.validate_identity().map_err(storage_error)?;
        let size = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if size > MAX_JOURNAL_BYTES as u64 {
            return Err(corrupt("run journal exceeds retention bound"));
        }
        if initialize {
            require(
                size == 0 && mode == RunMode::Control,
                "only new control storage can be initialized",
            )?;
            authorize(context, true)?;
            let binding =
                binding.ok_or_else(|| corrupt("new run journal requires exact source binding"))?;
            let payload = binding.encode();
            let mut bytes = b"DFMRJ001".to_vec();
            bytes.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            bytes.extend_from_slice(&payload);
            bytes.extend_from_slice(&context.session_id.get().to_be_bytes());
            bytes.extend_from_slice(&context.request_id.get().to_be_bytes());
            let id = hash(b"dfmcp-run-journal/1", &bytes);
            bytes.extend_from_slice(id.as_bytes());
            allowance.charge(bytes.len() as u64)?;
            storage
                .write_all(&bytes)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(storage_error)?;
            storage.validate_identity().map_err(storage_error)?;
            return Ok(Self {
                storage,
                binding,
                id,
                head: id,
                frames: 0,
                bytes,
                records: BTreeMap::new(),
                owner: context.session_id,
                mode,
                fenced: false,
            });
        }
        require(size >= 74, "existing run journal is empty or incomplete")?;
        allowance.charge(size)?;
        storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
        let mut bytes = vec![0; size as usize];
        storage.read_exact(&mut bytes).map_err(storage_error)?;
        storage.validate_identity().map_err(storage_error)?;
        let mut r = Reader(&bytes);
        require(r.take(8)? == b"DFMRJ001", "wrong run journal format")?;
        let n = usize::from(u16::from_be_bytes(r.array()?));
        require(n <= 400, "run binding exceeds bound")?;
        let stored = RunBinding::decode(r.take(n)?)?;
        r.take(32)?;
        let header_end = bytes.len() - r.0.len();
        let id = Digest32::from_bytes(r.array()?);
        require(
            id == hash(b"dfmcp-run-journal/1", &bytes[..header_end]),
            "run journal header checksum mismatch",
        )?;
        if let Some(expected) = binding {
            require(
                stored == expected,
                "run journal belongs to another endpoint/source generation",
            )?;
        }
        let mut head = id;
        let mut frames = 0;
        let mut records = BTreeMap::new();
        while !r.0.is_empty() {
            allowance.remaining()?;
            let start = r.0;
            require(r.take(8)? == b"DFMRF001", "bad run journal frame")?;
            let n = r.u32()? as usize;
            require(n <= MAX_BODY, "oversized run journal body")?;
            frames += 1;
            require(
                frames <= MAX_TRANSITIONS && r.u64()? == frames,
                "run journal sequence gap",
            )?;
            require(
                r.take(32)? == head.as_bytes(),
                "run journal digest-chain fork",
            )?;
            let record = DurableRun::decode(r.take(n)?)?;
            let digest = Digest32::from_bytes(r.array()?);
            require(
                r.take(8)? == b"DFMREND1" && digest == hash(b"dfmcp-run-frame/1", &start[..52 + n]),
                "incomplete/corrupt run journal frame",
            )?;
            require(
                record.plan.before().generation() == stored.manifest.generation,
                "run record crosses source generation",
            )?;
            transition(records.get(record.plan.key()), &record)?;
            if !records.contains_key(record.plan.key()) {
                require(
                    records.len() < MAX_RUNS,
                    "run journal key capacity exceeded",
                )?;
            }
            records.insert(record.plan.key().to_owned(), record);
            head = digest;
        }
        allowance.remaining()?;
        if mode != RunMode::Offline {
            storage.sync().map_err(storage_error)?;
        }
        let mut journal = Self {
            storage,
            binding: stored,
            id,
            head,
            frames,
            bytes,
            records,
            owner: context.session_id,
            mode,
            fenced: false,
        };
        journal.verify(&mut allowance)?;
        Ok(journal)
    }
    pub fn binding(&self) -> &RunBinding {
        &self.binding
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn head(&self) -> Digest32 {
        self.head
    }
    pub fn transitions(&self) -> u64 {
        self.frames
    }
    /// Historical in-memory root only, not a custody check or current game state.
    /// Network handlers must authorize Query before projecting these values.
    pub fn cached_records(&self) -> impl Iterator<Item = &DurableRun> {
        self.records.values()
    }
    pub fn mode(&self) -> RunMode {
        self.mode
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    fn access(&self, context: &OperationContext, write: bool, control: bool) -> Result<()> {
        authorize(context, control)?;
        if context.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "run journal belongs to another session",
            ));
        }
        if self.fenced {
            return Err(corrupt("run journal fenced; reopen for verified recovery"));
        }
        if (write && self.mode == RunMode::Offline) || (control && self.mode != RunMode::Control) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "run recovery mode cannot widen its authority",
            ));
        }
        self.storage.validate_identity().map_err(storage_error)
    }
    fn verify(&mut self, allowance: &mut Allowance) -> Result<()> {
        self.access(&allowance.context, false, false)?;
        allowance.charge(self.bytes.len() as u64)?;
        let result = (|| {
            let size = self.storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
            require(
                size == self.bytes.len() as u64,
                "run journal length changed outside coordinator",
            )?;
            self.storage
                .seek(SeekFrom::Start(0))
                .map_err(storage_error)?;
            let mut bytes = vec![0; self.bytes.len()];
            self.storage.read_exact(&mut bytes).map_err(storage_error)?;
            require(
                bytes == self.bytes,
                "run journal bytes changed outside coordinator",
            )?;
            self.storage.validate_identity().map_err(storage_error)?;
            Ok(())
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result
    }
    fn append(&mut self, next: DurableRun, allowance: &mut Allowance) -> Result<DurableRun> {
        self.access(&allowance.remaining()?, true, false)?;
        transition(self.records.get(next.plan.key()), &next)?;
        require(
            next.plan.before().generation() == self.binding.manifest.generation,
            "run intent crosses journal source",
        )?;
        if !self.records.contains_key(next.plan.key()) {
            require(self.records.len() < MAX_RUNS, "run key retention exhausted")?;
        }
        let body = next.encode();
        let length = FRAME_OVERHEAD + body.len();
        // Monitoring cannot consume the last cancellation/terminal slots.
        let other_pending = self
            .records
            .values()
            .filter(|r| r.plan.key() != next.plan.key() && !r.terminal())
            .count();
        let own_reserve = match next.state {
            RunState::Intent | RunState::Prepared => 2,
            RunState::DispatchStarted | RunState::Tracking => 3,
            RunState::CancelRequested => 1,
            RunState::Terminal | RunState::CancelledBeforeDispatch => 0,
        };
        let reserve = other_pending * 2 + own_reserve;
        require(
            self.frames + 1 + reserve as u64 <= MAX_TRANSITIONS
                && self.bytes.len() + length + reserve * (MAX_BODY + FRAME_OVERHEAD)
                    <= MAX_JOURNAL_BYTES,
            "run journal retention exhausted",
        )?;
        allowance.charge((self.bytes.len() + length) as u64)?;
        let mut frame = b"DFMRF001".to_vec();
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.frames + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let digest = hash(b"dfmcp-run-frame/1", &frame);
        frame.extend_from_slice(digest.as_bytes());
        frame.extend_from_slice(b"DFMREND1");
        // Stage the complete new root before irreversible publication.
        let mut staged_bytes = self.bytes.clone();
        staged_bytes.extend_from_slice(&frame);
        let mut staged_records = self.records.clone();
        staged_records.insert(next.plan.key().to_owned(), next.clone());
        let write = (|| {
            self.storage.validate_identity().map_err(storage_error)?;
            self.storage
                .seek(SeekFrom::Start(self.bytes.len() as u64))
                .map_err(storage_error)?;
            self.storage
                .write_all(&frame)
                .and_then(|_| self.storage.flush())
                .and_then(|_| self.storage.sync())
                .map_err(storage_error)?;
            self.storage.validate_identity().map_err(storage_error)?;
            Ok(())
        })();
        if write.is_err() {
            self.fenced = true;
        }
        write?;
        self.bytes = staged_bytes;
        self.records = staged_records;
        self.frames += 1;
        self.head = digest;
        Ok(next)
    }
    fn source(&self, source: &impl RunSource, exact: bool) -> Result<()> {
        source.manifest().validate()?;
        require(
            source.endpoint() == Some(self.binding.endpoint)
                && source.manifest().same_software(&self.binding.manifest)
                && if exact {
                    source.manifest().generation == self.binding.manifest.generation
                } else {
                    source.manifest().generation >= self.binding.manifest.generation
                },
            "run source differs from journal binding",
        )
    }
    fn reserve_effect(&self, allowance: &mut Allowance) -> Result<()> {
        // Leave ample room for dispatch marker, cancellation intent and receipts.
        require(
            self.frames + 8 <= MAX_TRANSITIONS
                && self.bytes.len() + 8 * (MAX_BODY + FRAME_OVERHEAD) <= MAX_JOURNAL_BYTES,
            "no durable capacity for run dispatch and reconciliation",
        )?;
        if allowance.left < 4 * self.bytes.len() as u64 + 4 * RPC_RESERVE_BYTES + 8192 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "reserve durable run dispatch and recovery work before native effects",
            ));
        }
        Ok(())
    }
    fn lookup(&self, key: &str, digest: Digest32) -> Result<DurableRun> {
        super::validate_key(key)?;
        let record = self
            .records
            .get(key)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "no durable run has this key"))?;
        require(
            record.plan.digest() == digest,
            "run key and plan digest disagree",
        )?;
        Ok(record.clone())
    }
    pub fn records(&mut self, context: &OperationContext) -> Result<Vec<DurableRun>> {
        let mut allowance = Allowance::new(context)?;
        self.verify(&mut allowance)?;
        if self.records.len() > context.budget.max_entities as usize {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "run record selection exceeds entity allowance",
            ));
        }
        allowance.charge(self.records.values().map(|r| r.encode().len() as u64).sum())?;
        Ok(self.records.values().cloned().collect())
    }
    pub fn observe(
        &mut self,
        source: &mut impl RunSource,
        context: &OperationContext,
    ) -> Result<RunObservation> {
        let mut allowance = Allowance::new(context)?;
        self.verify(&mut allowance)?;
        self.access(context, false, false)?;
        if self.mode == RunMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline run evidence cannot contact native source",
            ));
        }
        self.source(source, true)?;
        let observation = source.observe(&allowance.rpc()?)?;
        self.source(source, true)?;
        require(
            observation.generation() == self.binding.manifest.generation,
            "run observation changed source",
        )?;
        let mut current = allowance.remaining()?;
        if let Some(tick) = observation.tick() {
            current.anchor.tick = GameTick(tick.max(current.anchor.tick.get()));
        }
        authorize(&current, false)?;
        Ok(observation)
    }
    pub fn prepare(
        &mut self,
        source: &mut impl RunSource,
        plan: RunPlan,
        context: &OperationContext,
    ) -> Result<DurableRun> {
        let mut allowance = Allowance::new(context)?;
        self.verify(&mut allowance)?;
        self.access(context, true, true)?;
        context.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        authorize_plan(context, &plan)?;
        if self.records.contains_key(plan.key()) {
            let old = self.lookup(plan.key(), plan.digest())?;
            require(old.plan == plan, "run intent differs")?;
            return Ok(old);
        }
        if self.records.values().any(DurableRun::unresolved) {
            return Err(uncertain());
        }
        self.reserve_effect(&mut allowance)?;
        self.source(source, true)?;
        require(
            context.anchor.state_hash == plan.before().witness()
                && context.anchor.tick.get() == plan.before().tick().unwrap_or(0)
                && context.anchor.cursor.epoch == plan.before().generation()
                && context.anchor.cursor.sequence == plan.before().sequence(),
            "run plan anchor differs from selected observation",
        )?;
        let observed = source.observe(&allowance.rpc()?)?;
        self.source(source, true)?;
        require(
            &observed == plan.before(),
            "run preparation witness became stale",
        )?;
        let intent = DurableRun {
            plan: plan.clone(),
            state: RunState::Intent,
            native: None,
        };
        self.append(intent, &mut allowance)?;
        let native = source.prepare(&plan, &allowance.rpc()?)?;
        self.source(source, false)?;
        self.accept_native(&plan, native, &mut allowance, false)
    }
    fn accept_native(
        &mut self,
        plan: &RunPlan,
        native: RunRecord,
        allowance: &mut Allowance,
        started: bool,
    ) -> Result<DurableRun> {
        let native = RunRecord::decode(native.canonical_bytes())?;
        require(
            native.plan() == plan,
            "native run receipt does not match intent",
        )?;
        let state = if native.phase().terminal() {
            RunState::Terminal
        } else if native.phase() == RunPhase::Prepared && !started {
            RunState::Prepared
        } else {
            RunState::Tracking
        };
        let next = DurableRun {
            plan: plan.clone(),
            state,
            native: Some(native),
        };
        if self.records.get(plan.key()) == Some(&next) {
            return Ok(next);
        }
        self.append(next, allowance)
    }
    pub fn commit(
        &mut self,
        source: &mut impl RunSource,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<DurableRun> {
        let mut allowance = Allowance::new(context)?;
        self.verify(&mut allowance)?;
        self.access(context, true, true)?;
        let record = self.lookup(key, digest)?;
        authorize_plan(context, &record.plan)?;
        if record.terminal() {
            return Ok(record);
        }
        if record.state != RunState::Prepared || self.records.values().any(DurableRun::unresolved) {
            return Err(uncertain());
        }
        self.reserve_effect(&mut allowance)?;
        self.source(source, true)?;
        let observed = source.observe(&allowance.rpc()?)?;
        self.source(source, true)?;
        require(
            &observed == record.plan.before(),
            "run commit observation is stale",
        )?;
        let mut current = allowance.remaining()?;
        current.anchor.tick = GameTick(observed.tick().unwrap_or(0));
        authorize_plan(&current, &record.plan)?;
        let mut started = record.clone();
        started.state = RunState::DispatchStarted;
        self.append(started, &mut allowance)?;
        // A synced marker is no longer dispatchable, even if this process stops
        // before the actual call. Reopening cannot promote it back to Prepared.
        let result = (|| {
            self.verify(&mut allowance)?;
            self.access(&allowance.remaining()?, true, true)?;
            let native = source.commit(&record.plan, &allowance.rpc()?)?;
            self.source(source, false)?;
            require(
                native.phase() != RunPhase::Prepared,
                "commit reply cannot remain prepared",
            )?;
            self.accept_native(&record.plan, native, &mut allowance, true)
        })();
        result.map_err(|_| uncertain())
    }
    pub fn reconcile(
        &mut self,
        source: &mut impl RunSource,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<DurableRun> {
        let mut allowance = Allowance::new(context)?;
        self.verify(&mut allowance)?;
        self.access(context, true, false)?;
        let record = self.lookup(key, digest)?;
        if record.terminal() {
            return Ok(record);
        }
        self.source(source, false)?;
        let native = source
            .query(&record.plan, &allowance.rpc()?)?
            .ok_or_else(uncertain)?;
        self.source(source, false)?;
        let started = !matches!(record.state, RunState::Intent | RunState::Prepared);
        self.accept_native(&record.plan, native, &mut allowance, started)
    }
    pub fn cancel(
        &mut self,
        source: Option<&mut dyn RunSource>,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<DurableRun> {
        let mut allowance = Allowance::new(context)?;
        self.verify(&mut allowance)?;
        self.access(context, true, true)?;
        let record = self.lookup(key, digest)?;
        if record.terminal() {
            return Ok(record);
        }
        if matches!(record.state, RunState::Intent | RunState::Prepared) {
            return self.append(
                DurableRun {
                    plan: record.plan,
                    state: RunState::CancelledBeforeDispatch,
                    native: None,
                },
                &mut allowance,
            );
        }
        let source = source.ok_or_else(uncertain)?;
        require(
            source.endpoint() == Some(self.binding.endpoint)
                && source.manifest().same_software(&self.binding.manifest)
                && source.manifest().generation >= self.binding.manifest.generation,
            "cancel source software changed",
        )?;
        if record.state != RunState::CancelRequested {
            let mut next = record.clone();
            next.state = RunState::CancelRequested;
            self.append(next, &mut allowance)?;
        }
        let result = (|| {
            let native = source.cancel(&record.plan, &allowance.rpc()?)?;
            require(
                !matches!(native.phase(), RunPhase::Prepared | RunPhase::Running),
                "cancellation reply did not enter a stop state",
            )?;
            self.accept_native(&record.plan, native, &mut allowance, true)
        })();
        result.map_err(|_| uncertain())
    }
}

#[cfg(test)]
mod tests;
