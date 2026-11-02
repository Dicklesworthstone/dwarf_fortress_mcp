//! One original predicate intent with independently acknowledged write-ahead custody.
//! Reopen restores evidence only. No method can issue a second native writer.
use super::rpc::Binding;
use super::*;
use crate::build_placement::journal::private_file::{
    PrivateBuildFile, open_private_storage_scoped,
};
use crate::control_effect_journal::EffectJournalStorage;
pub use crate::workshop_construction::journal::WorkshopMode as Mode;
use crate::workshop_construction::{
    acknowledgement::{WorkshopAcknowledgements, open_private_tree_acknowledgements},
    journal::{WorkshopMode, WorkshopPin},
};
use dfmcp_core::{Capability, GameTick, OperationContext, RiskTier, SessionId};
use std::io::SeekFrom;
use std::path::Path;
use std::time::{Duration, Instant};
#[cfg(all(test, target_os = "linux"))]
#[path = "journal_tests.rs"]
pub(crate) mod journal_tests;
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_FRAMES: u32 = 40;
pub const MAX_HORIZON_TICKS: u64 = 12000;
const MAX_BODY: usize = MAX_CAPTURE_BYTES + MAX_RECORD_BYTES + 8192;
fn corrupt() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "tree custody missing, corrupt or rolled back; preserve journal and ACK",
    )
}
fn check(v: bool) -> Result<()> {
    if v { Ok(()) } else { Err(corrupt()) }
}
pub fn uncertain() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "original tree intent requires reconciliation; never redispatch or replace its key",
    )
    .retryable(false)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Intent = 0,
    Prepared = 1,
    Dispatched = 2,
    Tracking = 3,
    Terminal = 4,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Original {
    pub plan: Plan,
    pub deadline_tick: u64,
}
impl Original {
    pub fn new(plan: Plan, deadline_tick: u64) -> Result<Self> {
        require(
            deadline_tick > plan.before.tick
                && deadline_tick - plan.before.tick <= MAX_HORIZON_TICKS,
            "tree deadline must be original and within 12000 ticks",
        )?;
        Ok(Self {
            plan,
            deadline_tick,
        })
    }
    pub fn encode(&self) -> Vec<u8> {
        let mut out = b"DFMTO021".to_vec();
        out.extend_from_slice(&self.deadline_tick.to_be_bytes());
        blob(&mut out, &self.plan.encode());
        out
    }
    pub fn decode(raw: &[u8]) -> Result<Self> {
        let mut r = Reader(raw);
        require(r.take(8)? == b"DFMTO021", "not original tree intent")?;
        let deadline = r.u64()?;
        let plan = Plan::decode(read_blob(&mut r, MAX_CAPTURE_BYTES + 256)?)?;
        r.finish()?;
        Self::new(plan, deadline)
    }
    pub fn seal(&self) -> Digest32 {
        hash(b"dfmcp-tree-harvest-review/1", &self.encode())
    }
}
pub struct Dispatch<'a> {
    plan: &'a Plan,
    head: Digest32,
}
impl Dispatch<'_> {
    pub fn plan(&self) -> &Plan {
        self.plan
    }
    pub fn journal_head(&self) -> Digest32 {
        self.head
    }
}
struct Work {
    start: Instant,
    left: u64,
    wall: u64,
    context: OperationContext,
}
impl Work {
    fn new(c: &OperationContext) -> Result<Self> {
        c.budget.validate()?;
        if c.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "tree custody operation cancelled",
            ));
        }
        if c.budget.max_wall_millis > 60000 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "tree custody deadline exceeds the fixed profile bound",
            ));
        }
        Ok(Self {
            start: Instant::now(),
            left: c.budget.max_bytes,
            wall: c.budget.max_wall_millis,
            context: c.clone(),
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let mut c = self.context.clone();
        c.budget.max_wall_millis = self
            .wall
            .checked_sub(self.start.elapsed().as_millis() as u64)
            .filter(|v| *v > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "original tree operation deadline exhausted",
                )
            })?;
        c.budget.max_bytes = self.left;
        Ok(c)
    }
    fn charge(&mut self, n: usize) -> Result<()> {
        self.left = self.left.checked_sub(n as u64).ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "original tree operation byte allowance exhausted",
            )
        })?;
        if self.start.elapsed() >= Duration::from_millis(self.wall) {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "original tree operation deadline exhausted",
            ));
        }
        Ok(())
    }
    fn reserve(&mut self, n: usize) -> Result<OperationContext> {
        self.charge(n)?;
        let mut c = self.current()?;
        c.budget.max_bytes = n as u64;
        Ok(c)
    }
}
fn read<S: EffectJournalStorage>(s: &mut S, w: &mut Work) -> Result<Vec<u8>> {
    w.current()?;
    s.validate_identity().map_err(|_| corrupt())?;
    w.current()?;
    let n = s.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
    w.current()?;
    check(n <= MAX_BYTES as u64)?;
    w.charge(n as usize)?;
    s.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
    w.current()?;
    let mut out = vec![0; n as usize];
    s.read_exact(&mut out).map_err(|_| corrupt())?;
    w.current()?;
    check(s.seek(SeekFrom::End(0)).map_err(|_| corrupt())? == n)?;
    w.current()?;
    s.validate_identity().map_err(|_| corrupt())?;
    w.current()?;
    Ok(out)
}
pub struct Journal<S> {
    storage: S,
    binding: Binding,
    root: [u32; 3],
    raw: Vec<u8>,
    id: Digest32,
    head: Digest32,
    frames: u32,
    owner: SessionId,
    mode: Mode,
    fenced: bool,
    permit: bool,
    sample_permit: bool,
    pub sample_pending: bool,
    pub dispatch_started: bool,
    pub effect_uncertain: bool,
    pub original: Option<Original>,
    pub state: State,
    pub native: Option<Record>,
    pub latest: Option<Progress>,
    pub samples: u32,
    pub high_tick: u64,
    pub high_generation: u64,
    pub high_sequence: u64,
    pub high_item: u32,
    pub continuity_lost: bool,
    pub achieved_tick: Option<u64>,
}
impl<S: EffectJournalStorage> Journal<S> {
    fn empty(
        storage: S,
        binding: Binding,
        root: [u32; 3],
        raw: Vec<u8>,
        id: Digest32,
        c: &OperationContext,
        mode: Mode,
    ) -> Self {
        Self {
            storage,
            binding,
            root,
            raw,
            id,
            head: id,
            frames: 0,
            owner: c.session_id,
            mode,
            fenced: false,
            permit: false,
            sample_permit: false,
            sample_pending: false,
            dispatch_started: false,
            effect_uncertain: false,
            original: None,
            state: State::Intent,
            native: None,
            latest: None,
            samples: 0,
            high_tick: 0,
            high_generation: 0,
            high_sequence: 0,
            high_item: 0,
            continuity_lost: false,
            achieved_tick: None,
        }
    }
    fn authorize(
        c: &OperationContext,
        binding: &Binding,
        write: bool,
        observe: bool,
        tick: u64,
    ) -> Result<()> {
        let mut current = c.clone();
        current.anchor.tick = GameTick(tick.max(c.anchor.tick.get()));
        check(current.anchor.fortress_id == binding.source.fortress_id())?;
        let scope = Some(binding.read_scope()?);
        current.authorize(Capability::Query, RiskTier::ReadOnly, &[], scope)?;
        if observe {
            current.authorize(Capability::Observe, RiskTier::ReadOnly, &[], scope)?;
        }
        if write {
            current.authorize(
                Capability::Plan,
                RiskTier::Irreversible,
                &[],
                Some(binding.effect_scope()?),
            )?;
            current.authorize(
                Capability::Designate,
                RiskTier::Irreversible,
                &[],
                Some(binding.effect_scope()?),
            )?;
        }
        Ok(())
    }
    fn header(binding: &Binding, root: [u32; 3], nonce: [u8; 32]) -> Vec<u8> {
        let mut out = b"DFMTJ021".to_vec();
        blob(&mut out, &binding.encode());
        put_coord(&mut out, root);
        out.extend_from_slice(&nonce);
        out
    }
    pub fn open(
        storage: S,
        c: &OperationContext,
        mode: Mode,
        binding: &Binding,
        root: [u32; 3],
        nonce: Option<[u8; 32]>,
        pin: Option<WorkshopPin>,
    ) -> Result<Self> {
        let mut work = Work::new(c)?;
        Self::open_work(storage, &mut work, mode, binding, root, nonce, pin)
    }
    fn open_work(
        mut storage: S,
        w: &mut Work,
        mode: Mode,
        binding: &Binding,
        root: [u32; 3],
        nonce: Option<[u8; 32]>,
        pin: Option<WorkshopPin>,
    ) -> Result<Self> {
        let c = w.current()?;
        Self::authorize(&c, binding, false, false, c.anchor.tick.get())?;
        let mut raw = read(&mut storage, w)?;
        if let Some(n) = nonce {
            check(mode == Mode::Control && raw.is_empty() && pin.is_none() && n != [0; 32])?;
            raw = Self::header(binding, root, n);
            let id = hash(b"dfmcp-tree-journal/1", &raw);
            raw.extend_from_slice(id.as_bytes());
            // Reserve both the header write and its required reread before either effect.
            // read() debits its own bytes; only the write is charged here.
            if w.left < 2 * raw.len() as u64 {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "tree initialization header exceeds combined write/read budget",
                ));
            }
            w.charge(raw.len())?;
            storage.seek(SeekFrom::Start(0)).map_err(|_| corrupt())?;
            storage
                .write_all(&raw)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(|_| corrupt())?;
            w.current()?;
            check(read(&mut storage, w)? == raw)?;
        } else {
            check(pin.is_some())?;
        }
        let mut r = Reader(&raw);
        check(r.take(8)? == b"DFMTJ021")?;
        check(read_blob(&mut r, 2048)? == binding.encode())?;
        check(coord(&mut r)? == root)?;
        check(r.take(32)? != [0; 32])?;
        let n = raw.len() - r.0.len();
        let id = Digest32::from_bytes(r.array()?);
        check(id == hash(b"dfmcp-tree-journal/1", &raw[..n]))?;
        let mut records = Vec::new();
        let mut head = id;
        let mut frames = 0;
        let mut acknowledged =
            pin.is_some_and(|p| p.journal_id == id && p.frames == 0 && p.head == id);
        while !r.0.is_empty() {
            w.current()?;
            check(frames < MAX_FRAMES && r.take(8)? == b"DFMTF021")?;
            let n = r.u32()? as usize;
            check(n <= MAX_BODY)?;
            w.charge(n.saturating_mul(8))?;
            check(r.u32()? == frames + 1 && r.take(32)? == head.as_bytes())?;
            let body = r.take(n)?.to_vec();
            let digest = Digest32::from_bytes(r.array()?);
            check(
                r.take(8)? == b"DFMTEND1"
                    && digest
                        == hash(
                            b"dfmcp-tree-frame/1",
                            &Self::frame_prefix(frames + 1, head, &body),
                        ),
            )?;
            frames += 1;
            head = digest;
            acknowledged |= pin
                == Some(WorkshopPin {
                    journal_id: id,
                    head,
                    frames,
                });
            records.push(body);
        }
        check(nonce.is_some() || acknowledged)?;
        let mut out = Self::empty(storage, binding.clone(), root, raw, id, &c, mode);
        for body in records {
            w.current()?;
            out.apply(&body)?;
        }
        out.sample_permit = false;
        out.effect_uncertain =
            out.dispatch_started && out.native.as_ref().is_none_or(|n| n.phase < 2);
        out.head = head;
        out.frames = frames;
        Self::authorize(&w.current()?, binding, false, false, out.high_tick)?;
        Ok(out)
    }
    pub fn pin(&self) -> WorkshopPin {
        WorkshopPin {
            journal_id: self.id,
            head: self.head,
            frames: self.frames,
        }
    }
    pub fn is_fenced(&self) -> bool {
        self.fenced
    }
    pub fn abandon(&mut self) {
        self.permit = false;
    }
    pub fn dispatchable(&self) -> bool {
        self.permit && !self.fenced && self.state == State::Prepared && self.mode == Mode::Control
    }
    pub fn verify(&mut self, c: &OperationContext) -> Result<()> {
        Self::authorize(c, &self.binding, false, false, self.high_tick)?;
        check(!self.fenced && self.owner == c.session_id)?;
        let mut w = Work::new(c)?;
        if read(&mut self.storage, &mut w)? != self.raw {
            self.fenced = true;
            self.permit = false;
            return Err(corrupt());
        }
        Self::authorize(&w.current()?, &self.binding, false, false, self.high_tick)?;
        Ok(())
    }
    fn frame_prefix(sequence: u32, head: Digest32, body: &[u8]) -> Vec<u8> {
        let mut out = b"DFMTF021".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        out.extend_from_slice(&sequence.to_be_bytes());
        out.extend_from_slice(head.as_bytes());
        out.extend_from_slice(body);
        out
    }
    fn apply(&mut self, body: &[u8]) -> Result<()> {
        let mut r = Reader(body);
        let state = match r.byte()? {
            0 => State::Intent,
            1 => State::Prepared,
            2 => State::Dispatched,
            3 => State::Tracking,
            4 => State::Terminal,
            _ => return Err(corrupt()),
        };
        let original = Original::decode(read_blob(&mut r, MAX_CAPTURE_BYTES + 512)?)?;
        self.binding.capture(&original.plan.before)?;
        check(
            original.plan.before.root == self.root
                && self
                    .binding
                    .scope
                    .contains_cuboid(original.plan.before.effect_scope()?),
        )?;
        let record = read_blob(&mut r, MAX_RECORD_BYTES)?;
        let native = if record.is_empty() {
            None
        } else {
            Some(Record::decode(record, &original.plan)?)
        };
        let progress = read_blob(&mut r, 4096)?;
        let sample = if progress.is_empty() {
            None
        } else {
            Some(Progress::decode(progress, &original.plan)?)
        };
        let credit = r.boolean()?;
        let sampling = r.boolean()?;
        r.finish()?;
        check((sample.is_some() || !credit) && !(sampling && sample.is_some()))?;
        if let Some(old) = &self.original {
            check(old == &original)?;
        } else {
            check(state == State::Intent && native.is_none() && sample.is_none())?;
        }
        let legal = match self.original.as_ref() {
            None => true,
            Some(_) => match self.state {
                State::Intent => {
                    matches!(state, State::Prepared | State::Tracking | State::Terminal)
                }
                State::Prepared => {
                    matches!(state, State::Dispatched | State::Tracking | State::Terminal)
                }
                State::Dispatched | State::Tracking => {
                    matches!(state, State::Tracking | State::Terminal)
                }
                State::Terminal => {
                    state == State::Terminal
                        && self.native == native
                        && (sample.is_some() || sampling)
                }
            },
        };
        check(legal)?;
        if let (Some(old), Some(new)) = (&self.native, &native) {
            check(old.plan == new.plan)?;
            match old.phase {
                1 => check(new.phase == 1 || new.phase == 2)?,
                2..=4 => check(old == new)?,
                _ => {}
            }
        }
        if state == State::Prepared {
            check(native.as_ref().is_some_and(|n| n.phase == 0) && sample.is_none())?;
        }
        if state == State::Dispatched {
            check(self.native == native && sample.is_none())?;
            self.dispatch_started = true;
            self.effect_uncertain = true;
        }
        if state == State::Terminal {
            check(native.as_ref().is_some_and(|n| (2..=4).contains(&n.phase)))?;
        }
        if self.original.is_none() {
            self.high_tick = original.plan.before.tick;
            self.high_generation = original.plan.before.generation;
            self.high_sequence = original.plan.before.sequence;
            self.high_item = original.plan.before.next_item;
        }
        if let Some(after) = native.as_ref().and_then(|n| n.after.as_ref())
            && after.generation == self.high_generation
        {
            self.high_tick = self.high_tick.max(after.tick);
            self.high_sequence = self.high_sequence.max(after.sequence);
            self.high_item = self.high_item.max(after.next_item);
        }
        if sampling {
            check(
                state == State::Terminal
                    && !self.sample_pending
                    && native.as_ref().is_some_and(|n| n.phase == 2),
            )?;
            self.sample_pending = true;
        }
        if let Some(p) = &sample {
            check(self.sample_pending)?;
            self.sample_pending = false;
            check(
                state == State::Terminal
                    && native.as_ref().is_some_and(|n| n.phase == 2)
                    && self.samples < MAX_SAMPLES,
            )?;
            check(
                p.generation >= self.high_generation
                    && p.tick >= self.high_tick
                    && p.next_item >= self.high_item
                    && (p.generation != self.high_generation || p.sequence >= self.high_sequence),
            )?;
            self.continuity_lost |= p.generation != original.plan.before.generation;
            self.high_tick = p.tick;
            self.high_generation = p.generation;
            self.high_sequence = p.sequence;
            self.high_item = p.next_item;
            self.samples += 1;
            if self.achieved_tick.is_none()
                && !self.continuity_lost
                && credit
                && p.tick <= original.deadline_tick
                && p.supply_observed()
            {
                self.achieved_tick = Some(p.tick);
            }
        }
        self.original = Some(original);
        self.state = state;
        self.native = native;
        if sample.is_some() {
            self.latest = sample;
        }
        Ok(())
    }
    fn append_work(
        &mut self,
        state: State,
        original: &Original,
        native: Option<&Record>,
        progress: Option<&Progress>,
        credit: bool,
        sampling: bool,
        w: &mut Work,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        let write = matches!(state, State::Intent | State::Prepared | State::Dispatched);
        let mut requested = self.binding.clone();
        requested.scope = original.plan.before.effect_scope()?;
        check(self.binding.scope.contains_cuboid(requested.scope))?;
        Self::authorize(
            &w.current()?,
            &requested,
            write,
            true,
            progress.map_or(self.high_tick, |p| p.tick.max(self.high_tick)),
        )?;
        check(
            self.mode != Mode::Offline
                && self.frames < MAX_FRAMES
                && !self.fenced
                && self.owner == w.context.session_id,
        )?;
        check(read(&mut self.storage, w)? == self.raw)?;
        ack.verify(
            &w.reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
        )?;
        w.current()?;
        let mut body = vec![state as u8];
        blob(&mut body, &original.encode());
        blob(&mut body, native.map_or(&[], Record::bytes));
        blob(&mut body, progress.map_or(&[], Progress::bytes));
        body.extend([u8::from(credit), u8::from(sampling)]);
        check(body.len() <= MAX_BODY)?;
        w.charge(body.len().saturating_mul(8))?;
        self.fenced = true;
        self.permit = false;
        self.apply(&body)?;
        let mut frame = Self::frame_prefix(self.frames + 1, self.head, &body);
        let head = hash(b"dfmcp-tree-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(b"DFMTEND1");
        check(self.raw.len() + frame.len() <= MAX_BYTES)?;
        w.current()?;
        self.storage.seek(SeekFrom::End(0)).map_err(|_| corrupt())?;
        self.storage
            .write_all(&frame)
            .and_then(|_| self.storage.flush())
            .and_then(|_| self.storage.sync())
            .map_err(|_| corrupt())?;
        self.raw.extend_from_slice(&frame);
        check(read(&mut self.storage, w)? == self.raw)?;
        self.frames += 1;
        self.head = head;
        ack.acknowledge(
            self.pin(),
            &w.reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
        )?;
        w.current()?;
        self.fenced = false;
        self.effect_uncertain =
            self.dispatch_started && self.native.as_ref().is_none_or(|n| n.phase < 2);
        Ok(())
    }
    fn append(
        &mut self,
        state: State,
        original: &Original,
        native: Option<&Record>,
        progress: Option<&Progress>,
        credit: bool,
        sampling: bool,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        let mut work = Work::new(c)?;
        self.append_work(
            state, original, native, progress, credit, sampling, &mut work, ack,
        )
    }
    pub fn intent(
        &mut self,
        original: Original,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        check(self.mode == Mode::Control && self.original.is_none())?;
        self.append(State::Intent, &original, None, None, false, false, c, ack)
    }
    pub fn prepared(
        &mut self,
        record: Record,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        check(self.state == State::Intent && record.phase == 0)?;
        let original = self.original.clone().ok_or_else(corrupt)?;
        self.append(
            State::Prepared,
            &original,
            Some(&record),
            None,
            false,
            false,
            c,
            ack,
        )?;
        self.permit = true;
        Ok(())
    }
    pub fn dispatch<G, F>(
        &mut self,
        seal: Digest32,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
        guard: &mut G,
        writer: F,
    ) -> Result<Record>
    where
        G: FnMut(&OperationContext) -> Result<()>,
        F: FnOnce(Dispatch<'_>, &OperationContext) -> Result<Record>,
    {
        // Every attempt consumes local authority, including invalid/cancelled entry checks.
        let permitted = self.dispatchable();
        self.permit = false;
        let mut work = Work::new(c)?;
        check(self.owner == c.session_id && self.mode == Mode::Control && !self.fenced)?;
        let original = self.original.clone().ok_or_else(corrupt)?;
        let mut requested = self.binding.clone();
        requested.scope = original.plan.before.effect_scope()?;
        check(self.binding.scope.contains_cuboid(requested.scope))?;
        Self::authorize(&work.current()?, &requested, true, true, self.high_tick)?;
        check(permitted && original.seal() == seal)?;
        let retained = self.native.clone();
        guard(&work.current()?)?;
        self.append_work(
            State::Dispatched,
            &original,
            retained.as_ref(),
            None,
            false,
            false,
            &mut work,
            ack,
        )?;
        let result: Result<Record> = (|| {
            guard(&work.current()?)?;
            Self::authorize(&work.current()?, &requested, true, true, self.high_tick)?;
            let record = writer(
                Dispatch {
                    plan: &original.plan,
                    head: self.head,
                },
                &work.current()?,
            )?;
            let state = if record.phase >= 2 {
                State::Terminal
            } else {
                State::Tracking
            };
            self.append_work(
                state,
                &original,
                Some(&record),
                None,
                false,
                false,
                &mut work,
                ack,
            )?;
            Ok(record)
        })();
        result.map_err(|e| {
            uncertain()
                .with_detail("key", original.plan.key())
                .with_detail("cause", e.code.as_str())
        })
    }
    pub fn retain(
        &mut self,
        record: Record,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        self.permit = false;
        let original = self.original.clone().ok_or_else(corrupt)?;
        check(record.plan == original.plan)?;
        let state = if record.phase >= 2 {
            State::Terminal
        } else {
            State::Tracking
        };
        self.append(state, &original, Some(&record), None, false, false, c, ack)
    }
    pub fn begin_sample(
        &mut self,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        check(
            !self.sample_pending
                && self.frames + 2 <= MAX_FRAMES
                && self.raw.len() + 2 * (MAX_BODY + 84) <= MAX_BYTES,
        )?;
        let original = self.original.clone().ok_or_else(corrupt)?;
        let native = self.native.clone();
        self.append(
            State::Terminal,
            &original,
            native.as_ref(),
            None,
            false,
            true,
            c,
            ack,
        )?;
        self.sample_permit = true;
        Ok(())
    }
    pub fn consume_uncredited(
        &mut self,
        progress: Progress,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        check(self.sample_permit)?;
        self.sample_permit = false;
        self.permit = false;
        let original = self.original.clone().ok_or_else(corrupt)?;
        let native = self.native.clone();
        self.append(
            State::Terminal,
            &original,
            native.as_ref(),
            Some(&progress),
            false,
            false,
            c,
            ack,
        )
    }
    pub fn sample(
        &mut self,
        progress: Progress,
        c: &OperationContext,
        ack: &mut WorkshopAcknowledgements<PrivateBuildFile>,
    ) -> Result<()> {
        check(self.sample_permit)?;
        self.sample_permit = false;
        self.permit = false;
        let original = self.original.clone().ok_or_else(corrupt)?;
        let native = self.native.clone();
        self.append(
            State::Terminal,
            &original,
            native.as_ref(),
            Some(&progress),
            true,
            false,
            c,
            ack,
        )
    }
}
pub fn open_private(
    directory: &Path,
    c: &OperationContext,
    mode: Mode,
    binding: &Binding,
    root: [u32; 3],
    initialize: bool,
    external: Option<WorkshopPin>,
    nonce: [u8; 32],
) -> Result<(
    Journal<PrivateBuildFile>,
    WorkshopAcknowledgements<PrivateBuildFile>,
)> {
    check(
        (initialize && external.is_none() && mode == Mode::Control)
            || (!initialize && external.is_some()),
    )?;
    let scope = binding.read_scope()?;
    let mut work = Work::new(c)?;
    Journal::<PrivateBuildFile>::authorize(
        &work.current()?,
        binding,
        false,
        false,
        c.anchor.tick.get(),
    )?;
    let mut ack = open_private_tree_acknowledgements(
        &directory.join("tree-harvest.ack"),
        &work.reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
        mode,
        initialize,
        external,
        scope,
    )?;
    work.current()?;
    let (storage, created, _) = open_private_storage_scoped(
        &directory.join("tree-harvest.journal"),
        &work.reserve(8192)?,
        match mode {
            WorkshopMode::Control => crate::build_placement::journal::BuildMode::Control,
            WorkshopMode::Recover => crate::build_placement::journal::BuildMode::Recover,
            WorkshopMode::Offline => crate::build_placement::journal::BuildMode::Offline,
        },
        initialize,
        MAX_BYTES,
        Some(scope),
    )?;
    check(created == initialize)?;
    work.current()?;
    let journal = Journal::open_work(
        storage,
        &mut work,
        mode,
        binding,
        root,
        created.then_some(nonce),
        ack.latest(),
    )?;
    if mode != Mode::Offline {
        ack.acknowledge(
            journal.pin(),
            &work.reserve(crate::workshop_construction::acknowledgement::WORK_BYTES as usize)?,
        )?;
    }
    work.current()?;
    Ok((journal, ack))
}
