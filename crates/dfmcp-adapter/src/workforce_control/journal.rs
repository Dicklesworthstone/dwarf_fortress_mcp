//! Append-only workforce coordination. Sync intent/dispatch before native calls;
//! sync verified receipts before publishing a new in-memory root. Never repair tails.
use super::rpc::{RPC_BYTES, WorkforceManifest, WorkforceSource, authorize};
use super::{
    AssignmentEffect, AssignmentPhase, AssignmentPlan, MAX_EFFECT, MAX_PLAN, Reader,
    WorkforceCapture, error, fortress_id, hash, put_text, require, text, u16, validate_ids,
};
use crate::control_effect_journal::EffectJournalStorage;
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};
use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::net::SocketAddr;
use std::time::{Duration, Instant};

pub const MAX_KEYS: usize = 64;
pub const MAX_EVENTS: u64 = 512;
pub const MAX_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_BODY: usize = MAX_PLAN + MAX_EFFECT + 9;
pub const MAX_FRAME: usize = MAX_BODY + 92;
const MAGIC: &[u8; 8] = b"DFMWJ001";
const FRAME: &[u8; 8] = b"DFMWFR01";
const END: &[u8; 8] = b"DFMWEND1";

fn corrupt(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "workforce journal I/O or custody failed; reopen without repair",
    )
}
fn check(ok: bool) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(error(
            ErrorCode::CorruptLedger,
            "invalid workforce journal history",
        ))
    }
}
fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "workforce work or retention allowance exhausted",
    )
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkforceMode {
    Control,
    Recover,
    Offline,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkforceBinding {
    endpoint: SocketAddr,
    manifest: WorkforceManifest,
    folder: String,
    site: u32,
}
impl WorkforceBinding {
    pub fn new(
        endpoint: SocketAddr,
        manifest: WorkforceManifest,
        folder: String,
        site: u32,
    ) -> Result<Self> {
        require(
            endpoint.ip().is_loopback()
                && endpoint.port() != 0
                && endpoint.to_string().len() <= 128,
            "workforce binding requires numeric loopback",
        )?;
        require(
            manifest.generation > 0 && manifest.generation < u64::MAX && site <= i32::MAX as u32,
            "invalid workforce binding identity",
        )?;
        for (value, limit) in [
            (&folder, 512),
            (&manifest.df_version, 128),
            (&manifest.dfhack_version, 128),
        ] {
            require(
                !value.is_empty() && value.len() <= limit && !value.contains('\0'),
                "invalid workforce binding text",
            )?;
        }
        Ok(Self {
            endpoint,
            manifest,
            folder,
            site,
        })
    }
    pub fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }
    pub fn manifest(&self) -> &WorkforceManifest {
        &self.manifest
    }
    pub fn folder(&self) -> &str {
        &self.folder
    }
    pub fn site(&self) -> u32 {
        self.site
    }
    pub fn fortress(&self) -> dfmcp_core::FortressId {
        fortress_id(&self.folder, self.site)
    }
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        put_text(&mut out, &self.endpoint.to_string());
        out.extend_from_slice(&self.manifest.generation.to_be_bytes());
        put_text(&mut out, &self.manifest.df_version);
        put_text(&mut out, &self.manifest.dfhack_version);
        put_text(&mut out, &self.folder);
        out.extend_from_slice(&self.site.to_be_bytes());
        out
    }
    fn decode(raw: &[u8]) -> Result<Self> {
        let mut r = Reader(raw);
        let address = text(&mut r, 128, false)?;
        let endpoint: SocketAddr = address
            .parse()
            .map_err(|_| error(ErrorCode::CorruptLedger, "invalid workforce endpoint"))?;
        check(endpoint.to_string() == address)?;
        let generation = r.u64()?;
        let df_version = text(&mut r, 128, false)?;
        let dfhack_version = text(&mut r, 128, false)?;
        let folder = text(&mut r, 512, false)?;
        let site = r.u32()?;
        r.finish()?;
        Self::new(
            endpoint,
            WorkforceManifest {
                generation,
                df_version,
                dfhack_version,
            },
            folder,
            site,
        )
    }
    fn capture(&self, value: &WorkforceCapture) -> Result<()> {
        require(
            value.generation() == self.manifest.generation
                && value.folder() == self.folder
                && value.site() == self.site,
            "workforce capture differs from the exact configured fortress",
        )
    }
    fn source<N: WorkforceSource>(&self, source: &N) -> Result<()> {
        require(
            source.endpoint() == Some(self.endpoint) && source.manifest() == &self.manifest,
            "workforce source endpoint, software or generation changed",
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AssignmentState {
    Intent = 0,
    Prepared = 1,
    DispatchStarted = 2,
    Tracking = 3,
    Terminal = 4,
    CancelRequested = 5,
    CancelledBeforeDispatch = 6,
}
impl AssignmentState {
    pub fn settled(self) -> bool {
        matches!(self, Self::Terminal | Self::CancelledBeforeDispatch)
    }
    pub fn unresolved(self) -> bool {
        matches!(
            self,
            Self::DispatchStarted | Self::Tracking | Self::CancelRequested
        )
    }
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
    fn decode(n: u8) -> Result<Self> {
        match n {
            0 => Ok(Self::Intent),
            1 => Ok(Self::Prepared),
            2 => Ok(Self::DispatchStarted),
            3 => Ok(Self::Tracking),
            4 => Ok(Self::Terminal),
            5 => Ok(Self::CancelRequested),
            6 => Ok(Self::CancelledBeforeDispatch),
            _ => Err(error(
                ErrorCode::CorruptLedger,
                "unknown workforce coordinator state",
            )),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssignmentRecord {
    plan: AssignmentPlan,
    state: AssignmentState,
    effect: Option<AssignmentEffect>,
}
impl AssignmentRecord {
    pub fn plan(&self) -> &AssignmentPlan {
        &self.plan
    }
    pub fn state(&self) -> AssignmentState {
        self.state
    }
    pub fn effect(&self) -> Option<&AssignmentEffect> {
        self.effect.as_ref()
    }
    pub fn needs_query(&self) -> bool {
        (self.state == AssignmentState::Intent || self.state.unresolved())
            && !self
                .effect
                .as_ref()
                .is_some_and(|e| e.phase() == AssignmentPhase::Unknown)
    }
    fn encode(&self) -> Vec<u8> {
        let plan = self.plan.canonical_bytes();
        let effect = self
            .effect
            .as_ref()
            .map_or(&[][..], AssignmentEffect::canonical_bytes);
        let mut out = vec![self.state as u8];
        out.extend_from_slice(&(plan.len() as u32).to_be_bytes());
        out.extend_from_slice(&plan);
        out.extend_from_slice(&(effect.len() as u32).to_be_bytes());
        out.extend_from_slice(effect);
        out
    }
    fn decode(raw: &[u8], binding: &WorkforceBinding) -> Result<Self> {
        check(raw.len() <= MAX_BODY)?;
        let mut r = Reader(raw);
        let state = AssignmentState::decode(r.byte()?)?;
        let n = r.u32()? as usize;
        check(n <= MAX_PLAN)?;
        let plan = AssignmentPlan::decode(r.take(n)?)?;
        binding.capture(plan.before())?;
        let n = r.u32()? as usize;
        check(n <= MAX_EFFECT)?;
        let effect = if n == 0 {
            None
        } else {
            Some(AssignmentEffect::decode(r.take(n)?, &plan)?)
        };
        r.finish()?;
        let phase = effect.as_ref().map(AssignmentEffect::phase);
        check(match state {
            AssignmentState::Intent => phase.is_none(),
            AssignmentState::Prepared | AssignmentState::DispatchStarted => {
                phase == Some(AssignmentPhase::Prepared)
            }
            AssignmentState::Tracking => matches!(
                phase,
                Some(AssignmentPhase::Prepared | AssignmentPhase::Unknown)
            ),
            AssignmentState::Terminal => phase.is_some_and(AssignmentPhase::settled),
            AssignmentState::CancelRequested => matches!(
                phase,
                Some(AssignmentPhase::Prepared | AssignmentPhase::Unknown)
            ),
            AssignmentState::CancelledBeforeDispatch => {
                phase.is_none() || phase == Some(AssignmentPhase::Prepared)
            }
        })?;
        Ok(Self {
            plan,
            state,
            effect,
        })
    }
}
fn transition(old: Option<&AssignmentRecord>, next: &AssignmentRecord) -> Result<()> {
    use AssignmentState::*;
    let Some(old) = old else {
        return check(next.state == Intent && next.effect.is_none());
    };
    check(next.plan == old.plan && next != old && !old.state.settled())?;
    if let Some(effect) = &old.effect {
        next.effect
            .as_ref()
            .ok_or_else(|| error(ErrorCode::CorruptLedger, "workforce evidence disappeared"))?
            .follows(effect)?;
    }
    check(match (old.state, next.state) {
        (Intent, Prepared | Tracking | Terminal | CancelledBeforeDispatch) => true,
        (Prepared, DispatchStarted | CancelledBeforeDispatch)
        | (DispatchStarted | Tracking, CancelRequested) => next.effect == old.effect,
        (DispatchStarted | Tracking, Tracking | Terminal)
        | (CancelRequested, CancelRequested | Terminal) => true,
        _ => false,
    })
}
fn reserve(state: AssignmentState) -> usize {
    match state {
        AssignmentState::Intent => 4,
        AssignmentState::Prepared => 3,
        AssignmentState::DispatchStarted | AssignmentState::Tracking => 2,
        AssignmentState::CancelRequested => 1,
        _ => 0,
    }
}
struct Allowance {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Allowance {
    fn new(context: &OperationContext) -> Result<Self> {
        context.budget.validate()?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok(Self {
            context: context.clone(),
            deadline,
            bytes: context.budget.max_bytes,
        })
    }
    fn context(&self) -> Result<OperationContext> {
        let duration = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        let ms = duration.as_millis();
        if ms == 0 {
            return Err(exhausted());
        }
        let mut c = self.context.clone();
        c.budget.max_wall_millis = (ms as u64).min(c.budget.max_wall_millis);
        c.budget.max_bytes = self.bytes;
        Ok(c)
    }
    fn charge(&mut self, bytes: usize) -> Result<()> {
        self.context()?;
        self.bytes = self.bytes.checked_sub(bytes as u64).ok_or_else(exhausted)?;
        Ok(())
    }
    fn rpc(&mut self) -> Result<OperationContext> {
        let mut c = self.context()?;
        self.charge(RPC_BYTES as usize)?;
        c.budget.max_bytes = RPC_BYTES;
        Ok(c)
    }
}
#[derive(Clone, Debug)]
pub struct WorkforceView {
    pub id: Digest32,
    pub head: Digest32,
    pub events: u64,
    pub bytes: usize,
    pub binding: WorkforceBinding,
    pub records: Vec<AssignmentRecord>,
}
pub struct WorkforceJournal<S> {
    storage: S,
    raw: Vec<u8>,
    binding: WorkforceBinding,
    id: Digest32,
    head: Digest32,
    events: u64,
    records: BTreeMap<String, AssignmentRecord>,
    mode: WorkforceMode,
    session: SessionId,
    fenced: bool,
}
impl<S: EffectJournalStorage> WorkforceJournal<S> {
    /// Existing journals ignore no stored authority: every reopen binds a fresh
    /// session and grants. Initialize only an empty, exclusively created file.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: WorkforceMode,
        initialize: Option<(WorkforceBinding, [u8; 32])>,
    ) -> Result<Self> {
        authorize(
            context,
            context.anchor.fortress_id,
            context.anchor.tick.get(),
            mode == WorkforceMode::Control,
        )?;
        let mut budget = Allowance::new(context)?;
        storage.validate_identity().map_err(corrupt)?;
        let length = storage.seek(SeekFrom::End(0)).map_err(corrupt)?;
        if length > MAX_BYTES as u64 {
            return Err(exhausted());
        }
        budget.charge(length as usize)?;
        storage.seek(SeekFrom::Start(0)).map_err(corrupt)?;
        let mut raw = vec![0; length as usize];
        storage.read_exact(&mut raw).map_err(corrupt)?;
        if let Some((binding, nonce)) = initialize {
            check(raw.is_empty() && mode == WorkforceMode::Control && nonce != [0; 32])?;
            authorize(context, binding.fortress(), context.anchor.tick.get(), true)?;
            let b = binding.encode();
            raw.extend_from_slice(MAGIC);
            raw.extend_from_slice(&(b.len() as u16).to_be_bytes());
            raw.extend_from_slice(&b);
            raw.extend_from_slice(&nonce);
            let id = hash(b"dfmcp-workforce-journal/1", &raw);
            raw.extend_from_slice(id.as_bytes());
            budget.charge(raw.len())?;
            storage
                .write_all(&raw)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(corrupt)?;
        }
        let mut r = Reader(&raw);
        check(r.take(8)? == MAGIC)?;
        let n = u16(&mut r)?;
        check(n <= 916)?;
        let binding = WorkforceBinding::decode(r.take(n)?)?;
        check(r.take(32)? != &[0u8; 32])?;
        let header_len = raw.len() - r.0.len();
        let id = Digest32::from_bytes(r.array()?);
        check(id == hash(b"dfmcp-workforce-journal/1", &raw[..header_len]))?;
        authorize(
            context,
            binding.fortress(),
            context.anchor.tick.get(),
            mode == WorkforceMode::Control,
        )?;
        let mut records = BTreeMap::new();
        let mut events = 0u64;
        let mut head = id;
        while !r.0.is_empty() {
            budget.context()?;
            let start = raw.len() - r.0.len();
            check(r.take(8)? == FRAME)?;
            let length = r.u32()? as usize;
            check(length <= MAX_BODY && events < MAX_EVENTS)?;
            check(r.u64()? == events + 1 && r.take(32)? == head.as_bytes())?;
            let record = AssignmentRecord::decode(r.take(length)?, &binding)?;
            let end = raw.len() - r.0.len();
            let digest = Digest32::from_bytes(r.array()?);
            check(
                digest == hash(b"dfmcp-workforce-frame/1", &raw[start..end]) && r.take(8)? == END,
            )?;
            if !records.contains_key(record.plan.key()) {
                check(
                    records.len() < MAX_KEYS
                        && records
                            .values()
                            .all(|v: &AssignmentRecord| v.state.settled()),
                )?;
            }
            transition(records.get(record.plan.key()), &record)?;
            records.insert(record.plan.key().to_owned(), record);
            events += 1;
            head = digest;
        }
        budget.context()?;
        storage.validate_identity().map_err(corrupt)?;
        // A complete frame surviving a failed earlier sync is resynchronized
        // before online work. Offline parsing never certifies power-loss durability.
        if mode != WorkforceMode::Offline {
            storage.sync().map_err(corrupt)?;
        }
        let mut result = Self {
            storage,
            raw,
            binding,
            id,
            head,
            events,
            records,
            mode,
            session: context.session_id,
            fenced: false,
        };
        result.verify(&mut budget)?;
        Ok(result)
    }
    pub fn mode(&self) -> WorkforceMode {
        self.mode
    }
    fn access(&self, context: &OperationContext, write: bool) -> Result<()> {
        if self.fenced {
            return Err(error(
                ErrorCode::CorruptLedger,
                "workforce journal fenced; reopen for verified recovery",
            ));
        }
        if context.session_id != self.session || (write && self.mode != WorkforceMode::Control) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "workforce session or fixed recovery mode denies control",
            ));
        }
        authorize(
            context,
            self.binding.fortress(),
            context.anchor.tick.get(),
            write,
        )
    }
    fn verify(&mut self, budget: &mut Allowance) -> Result<()> {
        self.access(&budget.context()?, false)?;
        budget.charge(self.raw.len())?;
        let result = (|| -> io::Result<()> {
            self.storage.validate_identity()?;
            if self.storage.seek(SeekFrom::End(0))? != self.raw.len() as u64 {
                return Err(io::Error::other("extent changed"));
            }
            self.storage.seek(SeekFrom::Start(0))?;
            let mut buffer = [0; 32768];
            for chunk in self.raw.chunks(buffer.len()) {
                self.storage.read_exact(&mut buffer[..chunk.len()])?;
                if &buffer[..chunk.len()] != chunk {
                    return Err(io::Error::other("journal bytes changed"));
                }
            }
            self.storage.validate_identity()
        })();
        if result.is_err() {
            self.fenced = true;
        }
        result.map_err(corrupt)?;
        budget.context()?;
        Ok(())
    }
    fn retain(
        &mut self,
        next: AssignmentRecord,
        budget: &mut Allowance,
    ) -> Result<AssignmentRecord> {
        self.access(&budget.context()?, false)?;
        if self.mode == WorkforceMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline workforce journal is read-only",
            ));
        }
        if self.records.get(next.plan.key()) == Some(&next) {
            self.verify(budget)?;
            return Ok(next);
        }
        let next = AssignmentRecord::decode(&next.encode(), &self.binding)?;
        if !self.records.contains_key(next.plan.key()) {
            check(
                self.records.len() < MAX_KEYS && self.records.values().all(|v| v.state.settled()),
            )?;
        }
        transition(self.records.get(next.plan.key()), &next)?;
        let body = next.encode();
        let extra = if next
            .effect
            .as_ref()
            .is_some_and(|e| e.phase() == AssignmentPhase::Unknown)
        {
            0
        } else {
            reserve(next.state)
        };
        if self.events + 1 + extra as u64 > MAX_EVENTS
            || self.raw.len() + 92 + body.len() + extra * MAX_FRAME > MAX_BYTES
        {
            return Err(exhausted());
        }
        let mut frame = FRAME.to_vec();
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.events + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let head = hash(b"dfmcp-workforce-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(END);
        self.verify(budget)?;
        budget.charge(frame.len())?;
        // All fallible allocation needed to publish roots happens before storage.
        self.raw.try_reserve(frame.len()).map_err(|_| exhausted())?;
        budget.charge(
            self.records
                .values()
                .map(|v| {
                    v.plan.canonical_bytes().len()
                        + v.effect.as_ref().map_or(0, |e| e.canonical_bytes().len())
                })
                .sum(),
        )?;
        let mut records = self.records.clone();
        records.insert(next.plan.key().to_owned(), next.clone());
        let result = self
            .storage
            .write_all(&frame)
            .and_then(|_| self.storage.flush())
            .and_then(|_| self.storage.sync());
        if result.is_err() {
            self.fenced = true;
        }
        result.map_err(corrupt)?;
        self.raw.extend_from_slice(&frame);
        self.records = records;
        self.head = head;
        self.events += 1;
        self.verify(budget)?;
        Ok(next)
    }
    fn known(&self, key: &str, digest: Digest32) -> Result<AssignmentRecord> {
        super::validate_key(key)?;
        let record = self.records.get(key).ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "assignment key absent from journal",
            )
        })?;
        if record.plan.digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "assignment key and digest disagree",
            ));
        }
        Ok(record.clone())
    }
    pub fn view(&mut self, context: &OperationContext) -> Result<WorkforceView> {
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        if self.records.len() > context.budget.max_entities as usize {
            return Err(exhausted());
        }
        budget.charge(
            self.records
                .values()
                .map(|v| {
                    v.plan.canonical_bytes().len()
                        + v.effect.as_ref().map_or(0, |e| e.canonical_bytes().len())
                })
                .sum(),
        )?;
        Ok(WorkforceView {
            id: self.id,
            head: self.head,
            events: self.events,
            bytes: self.raw.len(),
            binding: self.binding.clone(),
            records: self.records.values().cloned().collect(),
        })
    }
    pub fn get(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<AssignmentRecord> {
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        self.known(key, digest)
    }
    fn fresh<N: WorkforceSource>(
        &mut self,
        source: &mut N,
        plan: &AssignmentPlan,
        budget: &mut Allowance,
    ) -> Result<()> {
        self.binding.source(source)?;
        self.binding.capture(plan.before())?;
        let value = source.observe(&plan.before().ids(), &budget.rpc()?)?;
        self.binding.source(source)?;
        self.binding.capture(&value)?;
        require(
            value.entity_cost() <= budget.context.budget.max_entities as usize,
            "workforce entity allowance exhausted",
        )?;
        require(
            value == *plan.before(),
            "workforce capture changed; refresh and prepare a new intent",
        )?;
        let c = budget.context()?;
        authorize(&c, self.binding.fortress(), value.tick(), true)?;
        self.verify(budget)
    }
    pub fn observe<N: WorkforceSource>(
        &mut self,
        source: &mut N,
        ids: &[u32],
        context: &OperationContext,
    ) -> Result<WorkforceCapture> {
        self.access(context, false)?;
        validate_ids(ids)?;
        if self.mode == WorkforceMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline workforce session has no native reads",
            ));
        }
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        self.binding.source(source)?;
        let value = source.observe(ids, &budget.rpc()?)?;
        self.binding.source(source)?;
        self.binding.capture(&value)?;
        require(
            value.entity_cost() <= context.budget.max_entities as usize,
            "workforce entity allowance exhausted",
        )?;
        require(
            value.ids().as_slice() == ids,
            "workforce source changed selected citizens",
        )?;
        authorize(&budget.context()?, value.fortress_id(), value.tick(), false)?;
        self.verify(&mut budget)?;
        Ok(value)
    }
    pub fn prepare<N: WorkforceSource>(
        &mut self,
        source: &mut N,
        plan: &AssignmentPlan,
        context: &OperationContext,
    ) -> Result<AssignmentRecord> {
        self.access(context, true)?;
        let mut current = context.clone();
        current.anchor.tick = GameTick(plan.before().tick().max(context.anchor.tick.get()));
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        let mut budget = Allowance::new(&current)?;
        self.verify(&mut budget)?;
        if let Some(old) = self.records.get(plan.key()) {
            require(
                old.plan == *plan,
                "existing workforce key binds a different plan",
            )?;
            return Ok(old.clone());
        }
        if self.records.values().any(|v| !v.state.settled()) {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "unsettled assignment blocks new keys",
            ));
        }
        self.fresh(source, plan, &mut budget)?;
        let record = self.retain(
            AssignmentRecord {
                plan: plan.clone(),
                state: AssignmentState::Intent,
                effect: None,
            },
            &mut budget,
        )?;
        self.binding.source(source)?;
        let effect = source.prepare(plan, &budget.rpc()?)?;
        self.accept(record, effect, source, &mut budget)
    }
    pub fn commit<N: WorkforceSource>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<AssignmentRecord> {
        self.access(context, true)?;
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        let mut record = self.known(key, digest)?;
        if record.state.settled() {
            return Ok(record);
        }
        if record.state != AssignmentState::Prepared {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "workforce dispatch is not retryable; query retained evidence",
            ));
        }
        self.fresh(source, &record.plan, &mut budget)?;
        record.state = AssignmentState::DispatchStarted;
        let record = self.retain(record, &mut budget)?;
        self.binding.source(source)?;
        let effect = source.commit(&record.plan, &budget.rpc()?)?;
        self.accept(record, effect, source, &mut budget)
    }
    fn accept<N: WorkforceSource>(
        &mut self,
        mut record: AssignmentRecord,
        effect: AssignmentEffect,
        source: &N,
        budget: &mut Allowance,
    ) -> Result<AssignmentRecord> {
        self.binding.source(source)?;
        let effect = AssignmentEffect::decode(effect.canonical_bytes(), &record.plan)?;
        if let Some(old) = &record.effect {
            effect.follows(old)?;
        }
        record.state = if effect.phase().settled() {
            AssignmentState::Terminal
        } else if record.state == AssignmentState::Intent
            && effect.phase() == AssignmentPhase::Prepared
        {
            AssignmentState::Prepared
        } else if record.state == AssignmentState::CancelRequested {
            AssignmentState::CancelRequested
        } else {
            AssignmentState::Tracking
        };
        record.effect = Some(effect);
        self.retain(record, budget)
    }
    /// One QueryAssignment sample. Missing records preserve all prior evidence.
    pub fn reconcile<N: WorkforceSource>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<AssignmentRecord> {
        self.access(context, false)?;
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        let record = self.known(key, digest)?;
        if !record.needs_query() {
            return Ok(record);
        }
        if self.mode == WorkforceMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline uncertainty requires explicit online recovery",
            ));
        }
        self.binding.source(source)?;
        match source.query(&record.plan, &budget.rpc()?)? {
            Some(effect) => self.accept(record, effect, source, &mut budget),
            None => {
                self.binding.source(source)?;
                self.verify(&mut budget)?;
                Err(error(
                    ErrorCode::EffectIndeterminate,
                    "native assignment absent; no nonapplication proof and no retry permission",
                ))
            }
        }
    }
    pub fn cancel<N: WorkforceSource>(
        &mut self,
        source: Option<&mut N>,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<AssignmentRecord> {
        self.access(context, true)?;
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        let mut record = self.known(key, digest)?;
        if record.state.settled() {
            return Ok(record);
        }
        if matches!(
            record.state,
            AssignmentState::Intent | AssignmentState::Prepared
        ) {
            record.state = AssignmentState::CancelledBeforeDispatch;
            return self.retain(record, &mut budget);
        }
        if record
            .effect
            .as_ref()
            .is_some_and(|e| e.phase() == AssignmentPhase::Unknown)
        {
            return Ok(record);
        }
        let source = source.ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "native retirement requires a connection",
            )
        })?;
        self.binding.source(source)?;
        record.state = AssignmentState::CancelRequested;
        let record = self.retain(record, &mut budget)?;
        let effect = source.cancel(&record.plan, &budget.rpc()?)?;
        self.accept(record, effect, source, &mut budget)
    }
}

#[cfg(test)]
mod tests;
