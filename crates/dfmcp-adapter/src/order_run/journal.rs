//! Durable conditional-run ownership. Storage and replay never synthesize grants.
use super::rpc::{OrderRunManifest, OrderRunSource, RPC_RESERVE_BYTES, authorize, authorize_plan};
use super::{
    FortressIdentity, MAX_INTENT_BYTES, MAX_RECORD_BYTES, OrderCapture, OrderRunPlan,
    OrderRunRecord, RunPhase, field, put_field, text,
};
use crate::bounded_run::{Reader, error, hash, require, validate_key};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, SessionId};
use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

pub const MAX_JOURNAL_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_TRANSITIONS: u64 = 4096;
pub const MAX_INTENTS: usize = 256;
const MAX_BODY: usize = 1 + 2 + MAX_INTENT_BYTES + 2 + MAX_RECORD_BYTES;
const MAX_FRAME: usize = MAX_BODY + 92;
const MAX_BINDING: usize = 1024;
fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "conditional-run journal corrupt or custody lost; preserve evidence",
    )
}
fn storage_error(_: io::Error) -> dfmcp_core::DfmcpError {
    corrupt()
}
fn uncertain() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "conditional run may have dispatched; query or cancel, never repeat commit",
    )
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "conditional-run work or durable retention budget exhausted",
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderRunMode {
    Control,
    Recover,
    Offline,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderRunBinding {
    endpoint: SocketAddr,
    manifest: OrderRunManifest,
    fortress: FortressIdentity,
}
impl OrderRunBinding {
    pub fn from_source<N: OrderRunSource>(source: &N) -> Result<Self> {
        let endpoint = source.endpoint().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "journal requires actual connected endpoint identity",
            )
        })?;
        let value = Self {
            endpoint,
            manifest: source.manifest().clone(),
            fortress: source.fortress().clone(),
        };
        value.encode()?;
        Ok(value)
    }
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn manifest(&self) -> &OrderRunManifest {
        &self.manifest
    }
    pub fn fortress(&self) -> &FortressIdentity {
        &self.fortress
    }
    fn encode(&self) -> Result<Vec<u8>> {
        require(
            self.endpoint.ip().is_loopback()
                && self.endpoint.port() != 0
                && self.manifest.generation > 0
                && self.manifest.generation < u64::MAX,
            "invalid order-run journal binding",
        )?;
        text(self.manifest.df_version.as_bytes(), 128)?;
        text(self.manifest.dfhack_version.as_bytes(), 128)?;
        let mut out = Vec::new();
        put_field(&mut out, self.endpoint.to_string().as_bytes());
        out.extend_from_slice(&self.manifest.generation.to_be_bytes());
        put_field(&mut out, self.manifest.df_version.as_bytes());
        put_field(&mut out, self.manifest.dfhack_version.as_bytes());
        put_field(&mut out, self.fortress.folder().as_bytes());
        out.extend_from_slice(&self.fortress.site().to_be_bytes());
        require(out.len() <= MAX_BINDING, "order-run binding exceeds bound")?;
        Ok(out)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        let raw = text(field(&mut r, 128)?, 128)?;
        let endpoint = raw.parse::<SocketAddr>().map_err(|_| corrupt())?;
        let manifest = OrderRunManifest {
            generation: r.u64()?,
            df_version: text(field(&mut r, 128)?, 128)?,
            dfhack_version: text(field(&mut r, 128)?, 128)?,
        };
        let folder = text(field(&mut r, 512)?, 512)?;
        let fortress = FortressIdentity::new(&folder, r.u32()?)?;
        r.finish()?;
        let value = Self {
            endpoint,
            manifest,
            fortress,
        };
        require(value.encode()? == bytes, "noncanonical journal binding")?;
        Ok(value)
    }
    fn source<N: OrderRunSource>(&self, source: &N, exact: bool) -> Result<()> {
        let m = source.manifest();
        if source.endpoint() != Some(self.endpoint)
            || source.fortress() != &self.fortress
            || m.df_version != self.manifest.df_version
            || m.dfhack_version != self.manifest.dfhack_version
            || m.generation < self.manifest.generation
            || (exact && m.generation != self.manifest.generation)
        {
            return Err(error(
                ErrorCode::StaleAnchor,
                "native source differs from exact journal binding",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum OrderRunState {
    Intent = 0,
    Prepared = 1,
    DispatchStarted = 2,
    Tracking = 3,
    Terminal = 4,
    CancelRequested = 5,
    CancelledBeforeDispatch = 6,
}
impl OrderRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intent => "intent",
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
pub struct OrderRunEntry {
    plan: OrderRunPlan,
    state: OrderRunState,
    native: Option<OrderRunRecord>,
}
impl OrderRunEntry {
    pub fn plan(&self) -> &OrderRunPlan {
        &self.plan
    }
    pub fn state(&self) -> OrderRunState {
        self.state
    }
    pub fn native(&self) -> Option<&OrderRunRecord> {
        self.native.as_ref()
    }
    pub fn settled(&self) -> bool {
        self.state == OrderRunState::CancelledBeforeDispatch
            || (self.state == OrderRunState::Terminal
                && self
                    .native
                    .as_ref()
                    .is_some_and(|n| n.phase() != RunPhase::SourceLost))
    }
    pub fn unresolved(&self) -> bool {
        matches!(
            self.state,
            OrderRunState::DispatchStarted
                | OrderRunState::Tracking
                | OrderRunState::CancelRequested
        ) || self
            .native
            .as_ref()
            .is_some_and(|n| n.phase() == RunPhase::SourceLost)
    }
    fn validate(&self) -> Result<()> {
        if let Some(native) = &self.native {
            require(
                native.plan() == &self.plan,
                "journal receipt differs from complete intent",
            )?;
        }
        let phase = self.native.as_ref().map(OrderRunRecord::phase);
        let valid = match self.state {
            OrderRunState::Intent => phase.is_none(),
            OrderRunState::Prepared | OrderRunState::DispatchStarted => {
                phase == Some(RunPhase::Prepared)
            }
            OrderRunState::Tracking | OrderRunState::CancelRequested => {
                phase.is_some_and(|p| !p.terminal())
            }
            OrderRunState::Terminal => phase.is_some_and(RunPhase::terminal),
            OrderRunState::CancelledBeforeDispatch => {
                phase.is_none() || phase == Some(RunPhase::Prepared)
            }
        };
        require(valid, "impossible coordinator/native state")
    }
    fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let mut out = vec![self.state as u8];
        put_field(&mut out, &self.plan.canonical_bytes());
        put_field(
            &mut out,
            self.native.as_ref().map_or(&[], |v| v.canonical_bytes()),
        );
        Ok(out)
    }
    fn decode(bytes: &[u8]) -> Result<Self> {
        let mut r = Reader(bytes);
        let state = match r.byte()? {
            0 => OrderRunState::Intent,
            1 => OrderRunState::Prepared,
            2 => OrderRunState::DispatchStarted,
            3 => OrderRunState::Tracking,
            4 => OrderRunState::Terminal,
            5 => OrderRunState::CancelRequested,
            6 => OrderRunState::CancelledBeforeDispatch,
            _ => return Err(corrupt()),
        };
        let plan = OrderRunPlan::decode(field(&mut r, MAX_INTENT_BYTES)?)?;
        let raw = field(&mut r, MAX_RECORD_BYTES)?;
        let native = if raw.is_empty() {
            None
        } else {
            Some(OrderRunRecord::decode(raw)?)
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
}
fn transition(old: Option<&OrderRunEntry>, next: &OrderRunEntry) -> Result<()> {
    next.validate()?;
    let Some(old) = old else {
        return require(
            next.state == OrderRunState::Intent,
            "new key does not start with intent",
        );
    };
    require(
        old.plan == next.plan
            && !matches!(
                old.state,
                OrderRunState::Terminal | OrderRunState::CancelledBeforeDispatch
            ),
        "sealed intent or terminal coordinator record changed",
    )?;
    let allowed = match old.state {
        OrderRunState::Intent => matches!(
            next.state,
            OrderRunState::Prepared
                | OrderRunState::Tracking
                | OrderRunState::Terminal
                | OrderRunState::CancelledBeforeDispatch
        ),
        OrderRunState::Prepared => matches!(
            next.state,
            OrderRunState::DispatchStarted
                | OrderRunState::Tracking
                | OrderRunState::Terminal
                | OrderRunState::CancelledBeforeDispatch
        ),
        OrderRunState::DispatchStarted | OrderRunState::Tracking => matches!(
            next.state,
            OrderRunState::Tracking | OrderRunState::Terminal | OrderRunState::CancelRequested
        ),
        OrderRunState::CancelRequested => matches!(
            next.state,
            OrderRunState::CancelRequested | OrderRunState::Terminal
        ),
        _ => false,
    };
    require(allowed, "illegal conditional-run coordinator transition")?;
    if let Some(old_native) = &old.native {
        let next_native = next.native.as_ref().ok_or_else(corrupt)?;
        old_native.validate_successor(next_native)?;
    }
    if matches!(
        next.state,
        OrderRunState::DispatchStarted
            | OrderRunState::CancelledBeforeDispatch
            | OrderRunState::CancelRequested
    ) && next.state != old.state
    {
        require(
            next.native == old.native,
            "coordination-only transition changed native evidence",
        )?;
    }
    Ok(())
}

pub struct OrderRunView {
    pub id: Digest32,
    pub head: Digest32,
    pub transitions: u64,
    pub binding: OrderRunBinding,
    pub entries: Vec<OrderRunEntry>,
}
struct Allowance {
    deadline: Instant,
    bytes: u64,
}
impl Allowance {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(c.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok(Self {
            deadline,
            bytes: c.budget.max_bytes,
        })
    }
    fn charge(&mut self, n: u64) -> Result<()> {
        self.bytes = self.bytes.checked_sub(n).ok_or_else(exhausted)?;
        self.remaining()?;
        Ok(())
    }
    fn remaining(&self) -> Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|v| *v >= Duration::from_millis(1))
            .ok_or_else(exhausted)
    }
    fn context(&self, c: &OperationContext) -> Result<OperationContext> {
        let mut out = c.clone();
        out.budget.max_wall_millis =
            u64::try_from(self.remaining()?.as_millis()).map_err(|_| exhausted())?;
        Ok(out)
    }
}
pub struct OrderRunJournal<S> {
    storage: S,
    owner: SessionId,
    mode: OrderRunMode,
    binding: OrderRunBinding,
    bytes: Vec<u8>,
    id: Digest32,
    head: Digest32,
    transitions: u64,
    entries: BTreeMap<String, OrderRunEntry>,
    fenced: bool,
}
impl<S: EffectJournalStorage> OrderRunJournal<S> {
    pub fn open(
        mut storage: S,
        c: &OperationContext,
        mode: OrderRunMode,
        fortress: &FortressIdentity,
        expected: Option<OrderRunBinding>,
        initialize: bool,
    ) -> Result<Self> {
        authorize(c, fortress, mode == OrderRunMode::Control)?;
        let mut budget = Allowance::new(c)?;
        storage.validate_identity().map_err(storage_error)?;
        let length = storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
        if length > MAX_JOURNAL_BYTES {
            return Err(exhausted());
        }
        let mut bytes;
        if length == 0 && initialize && mode == OrderRunMode::Control {
            let binding = expected.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "new journal requires native binding",
                )
            })?;
            require(
                binding.fortress() == fortress,
                "new journal fortress mismatch",
            )?;
            bytes = b"DFMOJ014".to_vec();
            put_field(&mut bytes, &binding.encode()?);
            bytes.extend_from_slice(&c.session_id.get().to_be_bytes());
            bytes.extend_from_slice(&c.request_id.get().to_be_bytes());
            let proof = hash(b"dfmcp-order-run-rust-journal/1", &bytes);
            bytes.extend_from_slice(proof.as_bytes());
            budget.charge(bytes.len() as u64)?;
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage
                .write_all(&bytes)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(storage_error)?;
        } else {
            if initialize || length == 0 {
                return Err(corrupt());
            }
            budget.charge(length)?;
            bytes = vec![0; length as usize];
            storage.seek(SeekFrom::Start(0)).map_err(storage_error)?;
            storage.read_exact(&mut bytes).map_err(storage_error)?;
        }
        let mut r = Reader(&bytes);
        require(r.take(8)? == b"DFMOJ014", "not a Rust order-run journal")?;
        let binding = OrderRunBinding::decode(field(&mut r, MAX_BINDING)?)?;
        require(
            binding.fortress() == fortress,
            "journal belongs to another exact fortress",
        )?;
        if let Some(expected) = expected {
            require(expected == binding, "journal source binding mismatch")?;
        }
        r.take(32)?;
        let prefix = bytes.len() - r.0.len();
        let id = Digest32::from_bytes(r.array()?);
        require(
            id == hash(b"dfmcp-order-run-rust-journal/1", &bytes[..prefix]),
            "journal header checksum mismatch",
        )?;
        let mut head = id;
        let mut transitions = 0u64;
        let mut entries: BTreeMap<String, OrderRunEntry> = BTreeMap::new();
        while !r.0.is_empty() {
            budget.remaining()?;
            let start = bytes.len() - r.0.len();
            require(r.take(8)? == b"DFMOF014", "journal frame magic")?;
            let n = r.u32()? as usize;
            require(n <= MAX_BODY, "journal frame too large")?;
            let sequence = r.u64()?;
            let previous = Digest32::from_bytes(r.array()?);
            require(
                sequence == transitions + 1 && sequence <= MAX_TRANSITIONS && previous == head,
                "journal gap/fork",
            )?;
            let entry = OrderRunEntry::decode(r.take(n)?)?;
            require(
                entry.plan.before().fortress() == binding.fortress()
                    && entry.plan.before().generation() == binding.manifest.generation,
                "journal plan source mismatch",
            )?;
            let end = bytes.len() - r.0.len();
            let proof = Digest32::from_bytes(r.array()?);
            require(
                proof == hash(b"dfmcp-order-run-rust-frame/1", &bytes[start..end])
                    && r.take(8)? == b"DFMOEND1",
                "incomplete/corrupt journal frame",
            )?;
            if !entries.contains_key(entry.plan.key()) {
                require(
                    entries.len() < MAX_INTENTS && entries.values().all(OrderRunEntry::settled),
                    "new intent bypassed unresolved run",
                )?;
            }
            transition(entries.get(entry.plan.key()), &entry)?;
            entries.insert(entry.plan.key().to_owned(), entry);
            head = proof;
            transitions = sequence;
        }
        if entries.len() > c.budget.max_entities as usize {
            return Err(exhausted());
        }
        storage.validate_identity().map_err(storage_error)?;
        if mode != OrderRunMode::Offline {
            storage.sync().map_err(storage_error)?;
        }
        budget.remaining()?;
        Ok(Self {
            storage,
            owner: c.session_id,
            mode,
            binding,
            bytes,
            id,
            head,
            transitions,
            entries,
            fenced: false,
        })
    }
    pub fn mode(&self) -> OrderRunMode {
        self.mode
    }
    pub fn binding(&self) -> &OrderRunBinding {
        &self.binding
    }
    fn access(&self, c: &OperationContext, clock: bool) -> Result<()> {
        if self.fenced {
            return Err(corrupt());
        }
        if c.session_id != self.owner {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "conditional-run journal owned by another session",
            ));
        }
        authorize(c, self.binding.fortress(), clock)?;
        if clock && self.mode != OrderRunMode::Control {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "recovery mode cannot be promoted to clock control",
            ));
        }
        Ok(())
    }
    fn verify(&mut self, c: &OperationContext, budget: &mut Allowance) -> Result<()> {
        self.access(c, false)?;
        let result: Result<()> = (|| {
            self.storage.validate_identity().map_err(storage_error)?;
            require(
                self.storage.seek(SeekFrom::End(0)).map_err(storage_error)?
                    == self.bytes.len() as u64,
                "journal extent changed",
            )?;
            self.storage
                .seek(SeekFrom::Start(0))
                .map_err(storage_error)?;
            let mut buffer = [0; 4096];
            for chunk in self.bytes.chunks(buffer.len()) {
                budget.charge(chunk.len() as u64)?;
                self.storage
                    .read_exact(&mut buffer[..chunk.len()])
                    .map_err(storage_error)?;
                require(&buffer[..chunk.len()] == chunk, "journal bytes changed")?;
            }
            self.storage.validate_identity().map_err(storage_error)?;
            Ok(())
        })();
        if result
            .as_ref()
            .is_err_and(|e: &dfmcp_core::DfmcpError| e.code != ErrorCode::BudgetExceeded)
        {
            self.fenced = true;
        }
        result
    }
    pub fn view(&mut self, c: &OperationContext) -> Result<OrderRunView> {
        let mut budget = Allowance::new(c)?;
        self.verify(c, &mut budget)?;
        if self.entries.len() > c.budget.max_entities as usize {
            return Err(exhausted());
        }
        budget.charge(self.entries.len() as u64 * MAX_BODY as u64)?;
        Ok(OrderRunView {
            id: self.id,
            head: self.head,
            transitions: self.transitions,
            binding: self.binding.clone(),
            entries: self.entries.values().cloned().collect(),
        })
    }
    fn lookup(&self, key: &str, digest: Digest32) -> Result<OrderRunEntry> {
        validate_key(key)?;
        let entry = self.entries.get(key).ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "no retained conditional run has this key",
            )
        })?;
        if entry.plan.digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "key/digest does not identify the retained run",
            ));
        }
        Ok(entry.clone())
    }
    fn room(&self, frames: u64) -> Result<()> {
        if self.transitions + frames > MAX_TRANSITIONS
            || self.bytes.len() as u64 + frames * MAX_FRAME as u64 > MAX_JOURNAL_BYTES
        {
            return Err(exhausted());
        }
        Ok(())
    }
    fn append(
        &mut self,
        next: OrderRunEntry,
        c: &OperationContext,
        budget: &mut Allowance,
    ) -> Result<OrderRunEntry> {
        self.access(c, false)?;
        if self.mode == OrderRunMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline journal is immutable",
            ));
        }
        if self.entries.get(next.plan.key()) == Some(&next) {
            return Ok(next);
        }
        transition(self.entries.get(next.plan.key()), &next)?;
        let reserve = if matches!(
            next.state,
            OrderRunState::Terminal | OrderRunState::CancelledBeforeDispatch
        ) {
            0
        } else if next.state == OrderRunState::CancelRequested {
            1
        } else {
            2
        };
        self.room(1 + reserve)?;
        let body = next.encode()?;
        let mut frame = b"DFMOF014".to_vec();
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.transitions + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let head = hash(b"dfmcp-order-run-rust-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(b"DFMOEND1");
        // Stage both projections and retained bytes BEFORE crossing the write boundary.
        budget.charge(
            frame.len() as u64
                + self.bytes.len() as u64
                + self.entries.len() as u64 * MAX_BODY as u64,
        )?;
        let mut next_entries = self.entries.clone();
        next_entries.insert(next.plan.key().to_owned(), next.clone());
        let mut next_bytes = self.bytes.clone();
        next_bytes.extend_from_slice(&frame);
        self.verify(c, budget)?;
        let write: Result<()> = (|| {
            self.storage.seek(SeekFrom::End(0)).map_err(storage_error)?;
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
            return Err(corrupt());
        }
        self.bytes = next_bytes;
        self.entries = next_entries;
        self.head = head;
        self.transitions += 1;
        budget.remaining()?;
        Ok(next)
    }
    fn retain(
        &mut self,
        old: &OrderRunEntry,
        native: OrderRunRecord,
        c: &OperationContext,
        b: &mut Allowance,
    ) -> Result<OrderRunEntry> {
        require(
            native.plan() == &old.plan,
            "native receipt does not match retained intent",
        )?;
        if let Some(prior) = &old.native {
            prior.validate_successor(&native)?;
        }
        let state = if native.phase().terminal() {
            OrderRunState::Terminal
        } else if old.state == OrderRunState::CancelRequested {
            OrderRunState::CancelRequested
        } else if matches!(old.state, OrderRunState::Intent | OrderRunState::Prepared)
            && native.phase() == RunPhase::Prepared
        {
            OrderRunState::Prepared
        } else {
            OrderRunState::Tracking
        };
        self.append(
            OrderRunEntry {
                plan: old.plan.clone(),
                state,
                native: Some(native),
            },
            c,
            b,
        )
    }
    pub fn observe<N: OrderRunSource>(
        &mut self,
        n: &mut N,
        id: u32,
        c: &OperationContext,
    ) -> Result<OrderCapture> {
        let mut b = Allowance::new(c)?;
        self.verify(c, &mut b)?;
        if self.mode == OrderRunMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline has no native observation authority",
            ));
        }
        self.binding.source(n, true)?;
        b.charge(RPC_RESERVE_BYTES)?;
        let value = n.observe(id, &b.context(c)?, b.remaining()?)?;
        self.binding.source(n, true)?;
        require(
            value.fortress() == self.binding.fortress()
                && value.generation() == self.binding.manifest.generation
                && value.order_id() == id,
            "native observation changed exact bound source",
        )?;
        b.remaining()?;
        Ok(value)
    }
    pub fn prepare<N: OrderRunSource>(
        &mut self,
        n: &mut N,
        p: &OrderRunPlan,
        c: &OperationContext,
    ) -> Result<OrderRunEntry> {
        self.access(c, true)?;
        authorize_plan(c, p)?;
        require(
            p.before().fortress() == self.binding.fortress()
                && p.before().generation() == self.binding.manifest.generation,
            "plan source differs from exact journal binding",
        )?;
        let mut b = Allowance::new(c)?;
        self.verify(c, &mut b)?;
        if let Some(old) = self.entries.get(p.key()) {
            require(&old.plan == p, "existing key binds another conditional run")?;
            return Ok(old.clone());
        }
        if self.entries.len() >= MAX_INTENTS || self.entries.values().any(|e| !e.settled()) {
            return Err(uncertain());
        }
        self.binding.source(n, true)?;
        self.room(4)?;
        b.charge(2 * RPC_RESERVE_BYTES)?;
        let before = n.observe(p.before().order_id(), &b.context(c)?, b.remaining()?)?;
        self.binding.source(n, true)?;
        require(
            &before == p.before(),
            "conditional-run preparation witness is stale",
        )?;
        self.access(c, true)?;
        authorize_plan(c, p)?;
        let intent = self.append(
            OrderRunEntry {
                plan: p.clone(),
                state: OrderRunState::Intent,
                native: None,
            },
            c,
            &mut b,
        )?;
        let result = n.prepare(p, &b.context(c)?, b.remaining()?);
        match result {
            Ok(native) => self.retain(&intent, native, c, &mut b),
            Err(e) => {
                n.fence();
                Err(e)
            }
        }
    }
    pub fn commit<N: OrderRunSource>(
        &mut self,
        n: &mut N,
        key: &str,
        digest: Digest32,
        c: &OperationContext,
    ) -> Result<OrderRunEntry> {
        self.access(c, true)?;
        let mut b = Allowance::new(c)?;
        self.verify(c, &mut b)?;
        let old = self.lookup(key, digest)?;
        authorize_plan(c, &old.plan)?;
        if old.settled() {
            return Ok(old);
        }
        if old.state != OrderRunState::Prepared {
            return Err(uncertain());
        }
        self.binding.source(n, true)?;
        self.room(4)?;
        b.charge(2 * RPC_RESERVE_BYTES)?;
        let before = n.observe(old.plan.before().order_id(), &b.context(c)?, b.remaining()?)?;
        self.binding.source(n, true)?;
        require(
            &before == old.plan.before(),
            "conditional-run commit witness is stale",
        )?;
        self.access(c, true)?;
        authorize_plan(c, &old.plan)?;
        let dispatched = self.append(
            OrderRunEntry {
                state: OrderRunState::DispatchStarted,
                ..old
            },
            c,
            &mut b,
        )?;
        // No path below can make this marker dispatchable again, including a
        // deadline or storage error before the request was actually transmitted.
        let result = (|| {
            self.access(c, true)?;
            let native = n.commit(&dispatched.plan, &b.context(c)?, b.remaining()?)?;
            self.retain(&dispatched, native, c, &mut b)
        })();
        result.map_err(|_| {
            n.fence();
            uncertain()
        })
    }
    pub fn reconcile<N: OrderRunSource>(
        &mut self,
        n: &mut N,
        key: &str,
        digest: Digest32,
        c: &OperationContext,
    ) -> Result<OrderRunEntry> {
        let mut b = Allowance::new(c)?;
        self.verify(c, &mut b)?;
        let old = self.lookup(key, digest)?;
        if matches!(
            old.state,
            OrderRunState::Terminal | OrderRunState::CancelledBeforeDispatch
        ) {
            return Ok(old);
        }
        if self.mode == OrderRunMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline uncertainty requires explicit recovery reopen",
            ));
        }
        self.binding.source(n, false)?;
        b.charge(RPC_RESERVE_BYTES)?;
        match n.query(&old.plan, &b.context(c)?, b.remaining()?) {
            Ok(Some(native)) => self.retain(&old, native, c, &mut b),
            Ok(None) => Err(uncertain()),
            Err(e) => {
                n.fence();
                Err(e)
            }
        }
    }
    pub fn cancel<N: OrderRunSource>(
        &mut self,
        source: Option<&mut N>,
        key: &str,
        digest: Digest32,
        c: &OperationContext,
    ) -> Result<OrderRunEntry> {
        self.access(c, true)?;
        let mut b = Allowance::new(c)?;
        self.verify(c, &mut b)?;
        let old = self.lookup(key, digest)?;
        if matches!(
            old.state,
            OrderRunState::Terminal | OrderRunState::CancelledBeforeDispatch
        ) {
            return Ok(old);
        }
        if matches!(old.state, OrderRunState::Intent | OrderRunState::Prepared) {
            return self.append(
                OrderRunEntry {
                    state: OrderRunState::CancelledBeforeDispatch,
                    ..old
                },
                c,
                &mut b,
            );
        }
        let n = source.ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "dispatched run cancellation requires native source",
            )
        })?;
        self.binding.source(n, true)?;
        self.room(if old.state == OrderRunState::CancelRequested {
            1
        } else {
            2
        })?;
        b.charge(RPC_RESERVE_BYTES)?;
        let requested = self.append(
            OrderRunEntry {
                state: OrderRunState::CancelRequested,
                ..old
            },
            c,
            &mut b,
        )?;
        let result = (|| {
            self.access(c, true)?;
            let native = n.cancel(&requested.plan, &b.context(c)?, b.remaining()?)?;
            self.retain(&requested, native, c, &mut b)
        })();
        result.map_err(|_| {
            n.fence();
            uncertain()
        })
    }
}

#[cfg(test)]
mod tests;
