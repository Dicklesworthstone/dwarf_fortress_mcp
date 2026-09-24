//! Durable, fixed-profile mining coordination, not production admission.
//!
//! Intent and dispatch are synced before their native edges. Terminal proof is
//! synced before acknowledgement. Reopen restores evidence, NEVER a commit permit.
//! All synchronous I/O belongs in the caller's supervised blocking region.
use std::collections::BTreeMap;
use std::io::{self, SeekFrom};
use std::time::{Duration, Instant};

use dfmcp_core::{Digest32, ErrorCode, OperationContext, Result, SessionId};

use super::rpc::{DigSource, RPC_BYTES, authorize};
use super::{DigEffect, DigObservation, DigPhase, DigPlan, Reader, error, hash, validate_key};
use crate::control_effect_journal::EffectJournalStorage;

pub mod private_file;
mod record;
pub mod session;
pub use record::{DigBinding, DigRecord, DigState, DigSummary};
use record::{MAX_BINDING_BYTES, MAX_BODY_BYTES, check, reserve, transition};

pub const MAX_KEYS: usize = 128;
pub const MAX_EVENTS: u64 = 896;
pub const MAX_JOURNAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_FRAME_BYTES: usize = MAX_BODY_BYTES + 92;
const MAGIC: &[u8; 8] = b"DFMDJ001";
const FRAME: &[u8; 8] = b"DFMDJR01";
const END: &[u8; 8] = b"DFMDJEND";

fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "dig journal work or retention allowance exhausted",
    )
}
fn corrupt(_: io::Error) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "dig journal I/O or custody failed; reopen without repair",
    )
}
fn unknown(key: &str) -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::EffectIndeterminate,
        "dig outcome requires exact-record recovery; never retry dispatch",
    )
    .with_detail("key", key)
    .retryable(false)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigMode {
    Control,
    Recover,
    Offline,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigStage {
    Observe,
    Prepare,
    Commit,
    Query,
    Cancel,
}

/// Implemented by the trusted supervising runtime, not deserialized requests.
/// Recheck runtime cancellation/I/O, operator opt-in, exact fortress selection
/// and applicable live lease/checkpoint policy. No permissive implementation is
/// supplied. The coordinator separately enforces core capabilities and custody.
pub trait DigGuard {
    fn check(&mut self, stage: DigStage, plan: &DigPlan, context: &OperationContext) -> Result<()>;
}

struct Allowance {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Allowance {
    fn new(context: &OperationContext) -> Result<Self> {
        if context.cancellation_requested {
            return Err(error(
                ErrorCode::CancellationRequested,
                "dig operation cancelled",
            ));
        }
        context.budget.validate()?;
        if context.budget.max_wall_millis > 60_000 {
            return Err(exhausted());
        }
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok(Self {
            context: context.clone(),
            deadline,
            bytes: context.budget.max_bytes,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        let ms = left.as_millis() as u64;
        if ms == 0 {
            return Err(exhausted());
        }
        let mut context = self.context.clone();
        context.budget.max_wall_millis = ms.min(context.budget.max_wall_millis);
        context.budget.max_bytes = self.bytes;
        Ok(context)
    }
    fn charge(&mut self, n: usize) -> Result<()> {
        self.current()?;
        self.bytes = self.bytes.checked_sub(n as u64).ok_or_else(exhausted)?;
        Ok(())
    }
    fn rpc(&mut self) -> Result<OperationContext> {
        let mut context = self.current()?;
        self.charge(RPC_BYTES as usize)?;
        context.budget.max_bytes = RPC_BYTES;
        Ok(context)
    }
}

/// Opaque cursor: pages cannot move to a different session, journal or head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigCursor {
    session: SessionId,
    id: Digest32,
    head: Digest32,
    offset: usize,
    limit: usize,
}
#[derive(Clone, Debug)]
pub struct DigPage {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub total_records: usize,
    pub unsettled_records: usize,
    pub records: Vec<DigSummary>,
    pub continuation: Option<DigCursor>,
}

pub struct DigJournal<S> {
    storage: S,
    raw: Vec<u8>,
    binding: DigBinding,
    id: Digest32,
    head: Digest32,
    events: u64,
    records: BTreeMap<String, DigRecord>,
    mode: DigMode,
    session: SessionId,
    fresh_key: Option<String>,
    fenced: bool,
}
impl<S: EffectJournalStorage> DigJournal<S> {
    /// A nonce initializes ONLY an empty, exclusively created Control store and
    /// requires an expected binding. Recovery never creates or repairs history.
    /// An optional expected binding is checked before any online resynchronization.
    pub fn open(
        mut storage: S,
        context: &OperationContext,
        mode: DigMode,
        expected: Option<DigBinding>,
        nonce: Option<[u8; 32]>,
    ) -> Result<Self> {
        let mut budget = Allowance::new(context)?;
        storage.validate_identity().map_err(corrupt)?;
        let length = storage.seek(SeekFrom::End(0)).map_err(corrupt)?;
        if length > MAX_JOURNAL_BYTES as u64 {
            return Err(exhausted());
        }
        budget.charge(length as usize)?;
        storage.seek(SeekFrom::Start(0)).map_err(corrupt)?;
        let mut raw = vec![0; length as usize];
        storage.read_exact(&mut raw).map_err(corrupt)?;
        if let Some(nonce) = nonce {
            check(raw.is_empty() && mode == DigMode::Control && nonce != [0; 32])?;
            let binding = expected.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "dig journal creation requires an exact binding",
                )
            })?;
            binding.authorize(context, context.anchor.tick.get())?;
            let binding = binding.encode();
            check(binding.len() <= MAX_BINDING_BYTES)?;
            raw.extend_from_slice(MAGIC);
            raw.extend_from_slice(&(binding.len() as u16).to_be_bytes());
            raw.extend_from_slice(&binding);
            raw.extend_from_slice(&nonce);
            let id = hash(b"dfmcp-dig-journal/1", &raw);
            raw.extend_from_slice(id.as_bytes());
            budget.charge(raw.len())?;
            storage
                .write_all(&raw)
                .and_then(|_| storage.flush())
                .and_then(|_| storage.sync())
                .map_err(corrupt)?;
        }
        let decoded = (|| -> Result<_> {
            let mut r = Reader(&raw);
            check(r.take(8)? == MAGIC)?;
            let n = usize::from(u16::from_be_bytes(r.array()?));
            check(n <= MAX_BINDING_BYTES)?;
            let binding = DigBinding::decode(r.take(n)?)?;
            check(r.take(32)? != &[0; 32])?;
            let prefix_end = raw.len() - r.0.len();
            let id = Digest32::from_bytes(r.array()?);
            check(id == hash(b"dfmcp-dig-journal/1", &raw[..prefix_end]))?;
            if let Some(expected) = &expected {
                check(expected == &binding)?;
            }
            binding.authorize(&budget.current()?, context.anchor.tick.get())?;
            let mut head = id;
            let mut events = 0;
            let mut records = BTreeMap::new();
            while !r.0.is_empty() {
                budget.current()?;
                let start = raw.len() - r.0.len();
                check(r.take(8)? == FRAME)?;
                let n = r.u32()? as usize;
                check(n <= MAX_BODY_BYTES && events < MAX_EVENTS)?;
                check(r.u64()? == events + 1 && r.take(32)? == head.as_bytes())?;
                let next = DigRecord::decode(r.take(n)?, &binding)?;
                let end = raw.len() - r.0.len();
                let digest = Digest32::from_bytes(r.array()?);
                check(
                    digest == hash(b"dfmcp-dig-journal-frame/1", &raw[start..end])
                        && r.take(8)? == END,
                )?;
                Self::check_next(&records, &next)?;
                records.insert(next.plan.key().to_owned(), next);
                events += 1;
                head = digest;
            }
            Ok((binding, id, head, events, records))
        })();
        let (binding, id, head, events, records) = decoded?;
        let mut journal = Self {
            storage,
            raw,
            binding,
            id,
            head,
            events,
            records,
            mode,
            session: context.session_id,
            fresh_key: None,
            fenced: false,
        };
        journal.verify(&mut budget)?;
        // A complete frame surviving an earlier uncertain sync can be recovered.
        // Offline reads are historical verification, not power-loss qualification.
        if mode != DigMode::Offline {
            journal.storage.sync().map_err(corrupt)?;
            journal.verify(&mut budget)?;
        }
        Ok(journal)
    }
    pub fn mode(&self) -> DigMode {
        self.mode
    }
    pub fn binding(&self) -> &DigBinding {
        &self.binding
    }
    pub fn fenced(&self) -> bool {
        self.fenced
    }
    pub fn id(&self) -> Digest32 {
        self.id
    }
    pub fn head(&self) -> Digest32 {
        self.head
    }

    fn check_next(records: &BTreeMap<String, DigRecord>, next: &DigRecord) -> Result<()> {
        if !records.contains_key(next.plan.key()) {
            check(records.len() < MAX_KEYS && records.values().all(|r| r.state.terminal()))?;
            check(records.values().all(|r| {
                let sequence = r.plan.before().sequence()
                    + u64::from(
                        r.effect
                            .as_ref()
                            .is_some_and(|e| e.phase() == DigPhase::Designated),
                    );
                next.plan.before().tick() >= r.plan.before().tick()
                    && next.plan.before().sequence() >= sequence
            }))?;
        }
        transition(records.get(next.plan.key()), next)
    }
    fn access(&self, context: &OperationContext) -> Result<()> {
        if self.fenced {
            return Err(error(
                ErrorCode::CorruptLedger,
                "dig journal fenced; reopen for verified recovery",
            ));
        }
        if context.session_id != self.session {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "dig journal belongs to another session",
            ));
        }
        let tick = self
            .records
            .values()
            .map(|r| r.plan.before().tick())
            .max()
            .map_or(context.anchor.tick.get(), |t| {
                t.max(context.anchor.tick.get())
            });
        self.binding.authorize(context, tick)
    }
    fn online(&self, context: &OperationContext, control: bool) -> Result<()> {
        self.access(context)?;
        if self.mode == DigMode::Offline || (control && self.mode != DigMode::Control) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "fixed dig recovery mode denies this native operation",
            ));
        }
        Ok(())
    }
    fn operation(&mut self, context: &OperationContext) -> Result<Allowance> {
        self.access(context)?;
        // Covers all journal rereads, two appends/root copies, three RPCs and
        // complete retained results for the longest operation. Check BEFORE I/O.
        let required = 30 * (self.raw.len() + MAX_FRAME_BYTES) as u64 + 3 * RPC_BYTES;
        if context.budget.max_bytes < required {
            return Err(exhausted());
        }
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        Ok(budget)
    }
    fn verify(&mut self, budget: &mut Allowance) -> Result<()> {
        self.access(&budget.current()?)?;
        budget.charge(self.raw.len())?;
        let result = (|| -> io::Result<()> {
            self.storage.validate_identity()?;
            if self.storage.seek(SeekFrom::End(0))? != self.raw.len() as u64 {
                return Err(io::Error::other("dig journal extent changed"));
            }
            self.storage.seek(SeekFrom::Start(0))?;
            let mut buffer = [0; 32768];
            for chunk in self.raw.chunks(buffer.len()) {
                self.storage.read_exact(&mut buffer[..chunk.len()])?;
                if &buffer[..chunk.len()] != chunk {
                    return Err(io::Error::other("dig journal bytes changed"));
                }
            }
            self.storage.validate_identity()
        })();
        if result.is_err() {
            self.fenced = true;
            self.fresh_key = None;
        }
        result.map_err(corrupt)?;
        budget.current()?;
        Ok(())
    }
    fn capacity(&self, next: &DigRecord) -> Result<()> {
        Self::check_next(&self.records, next)?;
        let extra = reserve(next);
        if self.events + 1 + extra as u64 > MAX_EVENTS
            || self.raw.len() + MAX_FRAME_BYTES * (1 + extra) > MAX_JOURNAL_BYTES
        {
            return Err(exhausted());
        }
        Ok(())
    }
    fn retain(&mut self, next: DigRecord, budget: &mut Allowance) -> Result<DigRecord> {
        self.access(&budget.current()?)?;
        if self.mode == DigMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline dig journal is read-only",
            ));
        }
        if self.records.get(next.plan.key()) == Some(&next) {
            self.verify(budget)?;
            return Ok(next);
        }
        let body = next.encode();
        let next = DigRecord::decode(&body, &self.binding)?;
        self.capacity(&next)?;
        let mut frame = FRAME.to_vec();
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&(self.events + 1).to_be_bytes());
        frame.extend_from_slice(self.head.as_bytes());
        frame.extend_from_slice(&body);
        let head = hash(b"dfmcp-dig-journal-frame/1", &frame);
        frame.extend_from_slice(head.as_bytes());
        frame.extend_from_slice(END);
        self.verify(budget)?;
        budget.charge(frame.len() + self.raw.len())?; // Includes bounded retained-root copy.
        self.raw.try_reserve(frame.len()).map_err(|_| exhausted())?;
        let mut records = self.records.clone();
        records.insert(next.plan.key().to_owned(), next.clone());
        let written = self
            .storage
            .write_all(&frame)
            .and_then(|_| self.storage.flush())
            .and_then(|_| self.storage.sync());
        if written.is_err() {
            self.fenced = true;
            self.fresh_key = None;
        }
        written.map_err(corrupt)?;
        self.raw.extend_from_slice(&frame);
        self.records = records;
        self.head = head;
        self.events += 1;
        self.verify(budget)?;
        Ok(next)
    }
    fn known(&self, key: &str, digest: Digest32) -> Result<DigRecord> {
        validate_key(key)?;
        let record = self
            .records
            .get(key)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "dig key absent from journal"))?;
        if record.plan.digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "dig key and plan digest disagree",
            ));
        }
        Ok(record.clone())
    }
    pub fn get(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<DigRecord> {
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        budget.charge(MAX_BODY_BYTES)?;
        let record = self.known(key, digest)?;
        if (context.budget.max_entities as usize) < record.plan.before().region().halo_count() {
            return Err(exhausted());
        }
        Ok(record)
    }
    pub fn list(
        &mut self,
        context: &OperationContext,
        limit: usize,
        cursor: Option<&DigCursor>,
    ) -> Result<DigPage> {
        if !(1..=8).contains(&limit) || limit > context.budget.max_entities as usize {
            return Err(exhausted());
        }
        let mut budget = Allowance::new(context)?;
        self.verify(&mut budget)?;
        let offset = if let Some(cursor) = cursor {
            if cursor.session != self.session
                || cursor.id != self.id
                || cursor.head != self.head
                || cursor.limit != limit
                || cursor.offset >= self.records.len()
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "dig continuation does not match this session and exact journal head",
                ));
            }
            cursor.offset
        } else {
            0
        };
        budget.charge(limit * 512)?;
        let records = self
            .records
            .values()
            .skip(offset)
            .take(limit)
            .map(|r| DigSummary {
                key: r.plan.key().to_owned(),
                plan_digest: r.plan.digest(),
                state: r.state,
                native_phase: r.effect.as_ref().map(DigEffect::phase),
                receipt: r.effect.as_ref().and_then(DigEffect::receipt),
                dispatchable: r.state == DigState::Prepared
                    && self.fresh_key.as_deref() == Some(r.plan.key()),
            })
            .collect::<Vec<_>>();
        let end = offset + records.len();
        let continuation = (end < self.records.len()).then_some(DigCursor {
            session: self.session,
            id: self.id,
            head: self.head,
            offset: end,
            limit,
        });
        budget.current()?;
        Ok(DigPage {
            journal_id: self.id,
            head: self.head,
            total_records: self.records.len(),
            unsettled_records: self
                .records
                .values()
                .filter(|r| !r.state.terminal())
                .count(),
            records,
            continuation,
        })
    }
    fn edge<N: DigSource, G: DigGuard>(
        &mut self,
        source: &N,
        guard: &mut G,
        stage: DigStage,
        plan: &DigPlan,
        budget: &mut Allowance,
    ) -> Result<OperationContext> {
        self.verify(budget)?;
        self.binding.source(source)?;
        self.binding.capture(plan.before())?;
        if (budget.context.budget.max_entities as usize) < plan.before().region().halo_count() {
            return Err(exhausted());
        }
        let context = budget.rpc()?;
        authorize(
            &context,
            self.binding.fortress_id(),
            plan.before().tick(),
            plan.before().region(),
            stage == DigStage::Observe,
            matches!(
                stage,
                DigStage::Prepare | DigStage::Commit | DigStage::Cancel
            ),
            stage == DigStage::Prepare,
        )?;
        guard.check(stage, plan, &context)?;
        // Guard work must not refresh or escape the same wall-time allowance.
        let left = budget.current()?.budget.max_wall_millis;
        let mut context = context;
        context.budget.max_wall_millis = left;
        self.binding.source(source)?;
        Ok(context)
    }
    fn fresh<N: DigSource, G: DigGuard>(
        &mut self,
        source: &mut N,
        guard: &mut G,
        plan: &DigPlan,
        budget: &mut Allowance,
    ) -> Result<()> {
        let context = self.edge(source, guard, DigStage::Observe, plan, budget)?;
        let capture: DigObservation = source.observe(plan.before().region(), &context)?;
        self.binding.source(source)?;
        self.binding.capture(&capture)?;
        if capture != *plan.before() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "dig terrain changed; no dispatch permitted",
            ));
        }
        authorize(
            &budget.current()?,
            self.binding.fortress_id(),
            capture.tick(),
            capture.region(),
            true,
            false,
            false,
        )?;
        self.verify(budget)
    }
    fn accept(
        &mut self,
        mut record: DigRecord,
        effect: DigEffect,
        budget: &mut Allowance,
        fresh_preparation: bool,
    ) -> Result<DigRecord> {
        let effect = DigEffect::decode(effect.canonical_bytes(), &record.plan)?;
        record.state = if effect.phase().terminal() {
            DigState::Terminal
        } else if fresh_preparation && effect.phase() == DigPhase::Prepared {
            DigState::Prepared
        } else if record.state == DigState::CancelRequested {
            DigState::CancelRequested
        } else {
            DigState::Tracking
        };
        record.effect = Some(effect);
        self.retain(record, budget)
    }
    pub fn prepare<N: DigSource, G: DigGuard>(
        &mut self,
        source: &mut N,
        plan: &DigPlan,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<DigRecord> {
        self.online(context, true)?;
        authorize(
            context,
            self.binding.fortress_id(),
            plan.before().tick(),
            plan.before().region(),
            true,
            true,
            true,
        )?;
        self.binding.capture(plan.before())?;
        let mut budget = self.operation(context)?;
        if let Some(old) = self.records.get(plan.key()) {
            if old.plan != *plan {
                return Err(error(
                    ErrorCode::Conflict,
                    "dig key already binds a different plan",
                ));
            }
            return Ok(old.clone());
        }
        if self.records.values().any(|r| !r.state.terminal()) {
            return Err(unknown(plan.key()));
        }
        let next = DigRecord {
            plan: plan.clone(),
            state: DigState::Intent,
            effect: None,
        };
        self.capacity(&next)?;
        self.fresh(source, guard, plan, &mut budget)?;
        let record = self.retain(next, &mut budget)?;
        let result = (|| -> Result<DigRecord> {
            let context = self.edge(source, guard, DigStage::Prepare, plan, &mut budget)?;
            let prepared = source.prepare(plan, &context)?;
            self.binding.source(source)?;
            guard.check(DigStage::Prepare, plan, &budget.current()?)?;
            let fresh = !prepared.replayed() && prepared.effect().phase() == DigPhase::Prepared;
            let record = self.accept(record, prepared.effect().clone(), &mut budget, fresh)?;
            if fresh {
                self.fresh_key = Some(plan.key().to_owned());
            }
            Ok(record)
        })();
        result.map_err(|_| unknown(plan.key()))
    }
    /// Confirmation must be the exact reviewed plan digest; this does not attest
    /// human review. DigGuard supplies the supervising runtime's actual policy.
    pub fn commit<N: DigSource, G: DigGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        confirmation: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<DigRecord> {
        self.online(context, true)?;
        let mut budget = self.operation(context)?;
        let mut record = self.known(key, confirmation)?;
        authorize(
            context,
            self.binding.fortress_id(),
            record.plan.before().tick(),
            record.plan.before().region(),
            true,
            true,
            false,
        )?;
        if record.state.terminal() {
            return Ok(record);
        }
        if record.state != DigState::Prepared || self.fresh_key.as_deref() != Some(key) {
            return Err(unknown(key));
        }
        self.fresh(source, guard, &record.plan, &mut budget)?;
        self.fresh_key = None; // Never restored, even if dispatch synchronization fails.
        record.state = DigState::DispatchStarted;
        let record = self.retain(record, &mut budget)?;
        let result = (|| -> Result<DigRecord> {
            let context = self.edge(source, guard, DigStage::Commit, &record.plan, &mut budget)?;
            let effect = source.commit(&record.plan, &context)?;
            self.binding.source(source)?;
            guard.check(DigStage::Commit, &record.plan, &budget.current()?)?;
            check(effect.phase() != DigPhase::Prepared)?;
            self.accept(record, effect, &mut budget, false)
        })();
        result.map_err(|_| unknown(key))
    }
    pub fn reconcile<N: DigSource, G: DigGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<DigRecord> {
        let mut budget = self.operation(context)?;
        let record = self.known(key, digest)?;
        if !record.needs_reconciliation() {
            return Ok(record);
        }
        self.online(context, false)?;
        self.fresh_key = None; // Queried Prepared evidence is never dispatch permission.
        let rpc = self.edge(source, guard, DigStage::Query, &record.plan, &mut budget)?;
        let effect = source.query(&record.plan, &rpc)?;
        self.binding.source(source)?;
        guard.check(DigStage::Query, &record.plan, &budget.current()?)?;
        self.verify(&mut budget)?;
        let effect = effect.ok_or_else(|| unknown(key))?;
        self.accept(record, effect, &mut budget, false)
    }
    /// Retire a preparation through its exact native token. No local cancellation
    /// infers nonapplication, and cancellation cannot undo completed excavation.
    pub fn cancel<N: DigSource, G: DigGuard>(
        &mut self,
        source: &mut N,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<DigRecord> {
        let mut budget = self.operation(context)?;
        let mut record = self.known(key, digest)?;
        if !record.needs_reconciliation() {
            return Ok(record);
        }
        self.online(context, false)?;
        authorize(
            context,
            self.binding.fortress_id(),
            record.plan.before().tick(),
            record.plan.before().region(),
            true,
            true,
            false,
        )?;
        self.fresh_key = None;
        record.state = DigState::CancelRequested;
        let record = self.retain(record, &mut budget)?;
        let result = (|| -> Result<DigRecord> {
            let rpc = self.edge(source, guard, DigStage::Cancel, &record.plan, &mut budget)?;
            let effect = source.cancel(&record.plan, &rpc)?;
            self.binding.source(source)?;
            guard.check(DigStage::Cancel, &record.plan, &budget.current()?)?;
            check(effect.phase() != DigPhase::Prepared)?;
            self.accept(record, effect, &mut budget, false)
        })();
        result.map_err(|_| unknown(key))
    }
}

#[cfg(test)]
mod tests;
