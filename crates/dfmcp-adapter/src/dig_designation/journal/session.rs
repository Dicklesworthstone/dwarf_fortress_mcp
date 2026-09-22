//! Foreground session ownership for the fixed mining coordinator.
//!
//! The native connection is retained from observation through preparation and
//! commit. Recovery has a separate connection factory; commit has none. All I/O
//! still belongs to the supervising runtime's blocking region and trusted guard.
use dfmcp_core::{Digest32, ErrorCode, GameTick, OperationContext, Result};

use super::{Allowance, DigBinding, DigCursor, DigGuard, DigJournal, DigMode, DigPage,
    DigRecord, DigState, DigSummary, MAX_FRAME_BYTES, exhausted, unknown};
use super::record::MAX_BODY_BYTES;
use super::super::{DigObservation, DigPlan, DigRegion, error, validate_key};
use super::super::rpc::{CONNECT_BYTES, RPC_BYTES, DigSource, authorize};
use crate::control_effect_journal::EffectJournalStorage;

/// A trusted runtime boundary, never deserialized from MCP or retained evidence.
/// Connection checks apply even to query-only recovery. Initial observations
/// cannot use DigGuard::check because an ineligible capture is not a DigPlan.
pub trait DigSessionGuard: DigGuard {
    fn connect(&mut self, binding: &DigBinding, region: DigRegion,
        context: &OperationContext) -> Result<()>;
    fn observe(&mut self, binding: &DigBinding, region: DigRegion,
        context: &OperationContext) -> Result<()>;
}

/// Reserve negotiation and six native calls on the original connection. Later
/// request budgets cannot replenish the RPC client's absolute connection budget.
pub const SOURCE_RESERVATION_BYTES: u64 = CONNECT_BYTES + 6 * RPC_BYTES;

/// Constant-sized orientation for every response. The journal invariant permits
/// at most one nonterminal key, regardless of its position in a records page.
#[derive(Clone, Debug)]
pub struct DigSessionView {
    pub journal_id: Digest32,
    pub head: Digest32,
    pub events: u64,
    pub byte_len: usize,
    pub total_records: usize,
    pub pending: Option<DigSummary>,
}

pub struct DigSession<S, N> {
    journal: DigJournal<S>,
    source: Option<N>,
    selected: Option<DigObservation>,
    high_tick: u64,
    high_sequence: u64,
}

impl<S: EffectJournalStorage, N: DigSource> DigSession<S, N> {
    /// Opening a session never imports a source or a dispatch permit. In
    /// particular, moving a prepared journal into a new owner revokes its permit.
    pub fn new(mut journal: DigJournal<S>, context: &OperationContext) -> Result<Self> {
        journal.fresh_key = None;
        let mut work = Allowance::new(context)?;
        journal.verify(&mut work)?;
        let (high_tick, high_sequence) = journal.records.values().fold(
            (context.anchor.tick.get(), 0), |(tick, sequence), record| {
                let after = record.plan.before().sequence()
                    + u64::from(record.effect.as_ref().is_some_and(|e| e.phase().terminal()
                        && e.designated_count().is_some()));
                (tick.max(record.plan.before().tick()), sequence.max(after))
            });
        Ok(Self { journal, source: None, selected: None, high_tick, high_sequence })
    }

    pub fn mode(&self) -> DigMode { self.journal.mode() }
    pub fn binding(&self) -> &DigBinding { self.journal.binding() }
    pub fn high_tick(&self) -> u64 { self.high_tick }
    pub fn selected(&self) -> Option<&DigObservation> { self.selected.as_ref() }
    pub fn has_preparation_connection(&self) -> bool {
        self.source.is_some() && self.journal.fresh_key.is_some()
    }

    /// Drop only ephemeral selection, connection and dispatch permission. The
    /// durable obligation remains discoverable and still blocks new keys.
    pub fn abandon_preparation(&mut self) {
        self.source = None;
        self.selected = None;
        self.journal.fresh_key = None;
    }

    fn current(&self, context: &OperationContext) -> Result<OperationContext> {
        let mut current = context.clone();
        current.anchor.tick = GameTick(self.high_tick.max(context.anchor.tick.get()));
        self.journal.access(&current)?;
        Ok(current)
    }

    fn in_scope(&self, region: DigRegion) -> Result<()> {
        let scope = self.binding().scope();
        if !scope.contains_cuboid(region.halo()) || !scope.contains_cuboid(region.write_area()) {
            return Err(error(ErrorCode::CapabilityDenied, "dig selection exceeds the operator-owned journal scope"));
        }
        Ok(())
    }

    fn journal_reservation(&self) -> u64 {
        30 * (self.journal.raw.len() + MAX_FRAME_BYTES) as u64 + 3 * RPC_BYTES
    }

    fn begin(&mut self, context: &OperationContext, connect: bool,
        journal_operation: bool) -> Result<Allowance>
    {
        let context = self.current(context)?;
        let mut required = 3 * (self.journal.raw.len() + 2 * MAX_FRAME_BYTES) as u64
            + 2 * MAX_BODY_BYTES as u64;
        if connect { required += SOURCE_RESERVATION_BYTES + RPC_BYTES; }
        if journal_operation { required += self.journal_reservation(); }
        if context.budget.max_bytes < required { return Err(exhausted()); }
        let mut work = Allowance::new(&context)?;
        self.journal.verify(&mut work)?;
        Ok(work)
    }

    fn reserve(work: &mut Allowance, bytes: u64) -> Result<OperationContext> {
        let mut context = work.current()?;
        let amount = usize::try_from(bytes).map_err(|_| exhausted())?;
        work.charge(amount)?;
        context.budget.max_bytes = bytes;
        Ok(context)
    }

    fn connect<F, G>(&self, region: DigRegion, work: &mut Allowance,
        factory: F, guard: &mut G) -> Result<N>
    where F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        self.in_scope(region)?;
        let mut context = Self::reserve(work, SOURCE_RESERVATION_BYTES)?;
        self.binding().authorize(&context, self.high_tick)?;
        guard.connect(self.binding(), region, &context)?;
        context.budget.max_wall_millis = work.current()?.budget.max_wall_millis;
        let source = factory(self.binding(), region, &context)?;
        self.binding().source(&source)?;
        guard.connect(self.binding(), region, &work.current()?)?;
        Ok(source)
    }

    /// A new observation explicitly abandons any older local preparation. It
    /// cannot clear the journal or make an unfinished operation disappear.
    pub fn observe<F, G>(&mut self, region: DigRegion, context: &OperationContext,
        factory: F, guard: &mut G) -> Result<DigObservation>
    where F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        let current = self.current(context)?;
        self.journal.online(&current, true)?;
        self.in_scope(region)?;
        authorize(&current, self.binding().fortress_id(), self.high_tick, region, true, false, false)?;
        if region.halo_count() > current.budget.max_entities as usize { return Err(exhausted()); }
        self.abandon_preparation();
        let result = (|| {
            let mut work = self.begin(&current, true, false)?;
            guard.observe(self.binding(), region, &work.current()?)?;
            let mut source = self.connect(region, &mut work, factory, guard)?;
            let mut rpc = Self::reserve(&mut work, RPC_BYTES)?;
            guard.observe(self.binding(), region, &rpc)?;
            rpc.budget.max_wall_millis = work.current()?.budget.max_wall_millis;
            let capture = source.observe(region, &rpc)?;
            self.binding().source(&source)?;
            self.binding().capture(&capture)?;
            if capture.region() != region || capture.tick() < self.high_tick
                || capture.sequence() < self.high_sequence {
                return Err(error(ErrorCode::StaleAnchor, "dig observation changed selection or regressed behind retained evidence"));
            }
            self.high_tick = capture.tick();
            self.high_sequence = capture.sequence();
            work.context.anchor.tick = GameTick(self.high_tick.max(work.context.anchor.tick.get()));
            authorize(&work.current()?, capture.fortress_id(), capture.tick(), region, true, false, false)?;
            guard.observe(self.binding(), region, &work.current()?)?;
            self.journal.verify(&mut work)?;
            self.selected = Some(capture.clone());
            self.source = Some(source);
            Ok(capture)
        })();
        if result.is_err() { self.abandon_preparation(); }
        result
    }

    pub fn prepare<G: DigSessionGuard>(&mut self, key: &str, allow_hidden: bool,
        witness: Digest32, context: &OperationContext, guard: &mut G) -> Result<DigRecord>
    {
        validate_key(key)?;
        let current = self.current(context)?;
        self.journal.online(&current, true)?;
        let mut work = self.begin(&current, false, true)?;
        if let Some(old) = self.journal.records.get(key) {
            if old.plan.allow_hidden_neighbors() != allow_hidden || old.plan.before().witness() != witness {
                return Err(error(ErrorCode::Conflict, "existing dig key binds different review inputs"));
            }
            authorize(&work.current()?, self.binding().fortress_id(), old.plan.before().tick(),
                old.plan.before().region(), true, true, true)?;
            return Ok(old.clone());
        }
        if self.journal.records.values().any(|r| !r.state.terminal()) { return Err(unknown(key)); }
        let capture = self.selected.as_ref().filter(|c| c.witness() == witness)
            .ok_or_else(|| error(ErrorCode::StaleAnchor, "dig planning requires this session's exact retained observation"))?;
        let plan = DigPlan::new(key, allow_hidden, capture.clone())?;
        authorize(&work.current()?, self.binding().fortress_id(), plan.before().tick(),
            plan.before().region(), true, true, true)?;
        let operation = Self::reserve(&mut work, self.journal_reservation())?;
        let source = self.source.as_mut().ok_or_else(|| unknown(key))?;
        let result = self.journal.prepare(source, &plan, &operation, guard);
        self.selected = None;
        if result.as_ref().is_ok_and(|record| record.state() == DigState::Prepared)
            && self.journal.fresh_key.as_deref() == Some(key) {
            if let Err(failure) = self.journal.verify(&mut work) {
                self.abandon_preparation();
                return Err(failure);
            }
        } else {
            self.abandon_preparation();
        }
        result
    }

    /// No factory is accepted here: a tool call cannot reconnect to manufacture
    /// a replacement for the native preparation's original connection.
    pub fn commit<G: DigSessionGuard>(&mut self, key: &str, confirmation: Digest32,
        context: &OperationContext, guard: &mut G) -> Result<DigRecord>
    {
        let current = self.current(context)?;
        self.journal.online(&current, true)?;
        let mut work = self.begin(&current, false, true)?;
        let record = self.journal.known(key, confirmation)?;
        authorize(&work.current()?, self.binding().fortress_id(), record.plan.before().tick(),
            record.plan.before().region(), true, true, false)?;
        if record.state.terminal() { return Ok(record); }
        if self.source.is_none() || self.journal.fresh_key.as_deref() != Some(key) { return Err(unknown(key)); }
        let operation = Self::reserve(&mut work, self.journal_reservation())?;
        let source = self.source.as_mut().ok_or_else(|| unknown(key))?;
        let result = self.journal.commit(source, key, confirmation, &operation, guard);
        self.abandon_preparation();
        if let Ok(record) = &result { self.advance_floor(record); }
        result
    }

    fn advance_floor(&mut self, record: &DigRecord) {
        self.high_tick = self.high_tick.max(record.plan.before().tick());
        let sequence = record.plan.before().sequence()
            + u64::from(record.effect.as_ref().is_some_and(|e| e.designated_count().is_some()));
        self.high_sequence = self.high_sequence.max(sequence);
    }

    pub fn get(&mut self, key: &str, digest: Digest32,
        context: &OperationContext) -> Result<DigRecord>
    {
        let current = self.current(context)?;
        self.journal.get(key, digest, &current)
    }

    pub fn view(&mut self, context: &OperationContext) -> Result<DigSessionView> {
        let current = self.current(context)?;
        let mut work = Allowance::new(&current)?;
        self.journal.verify(&mut work)?;
        work.charge(1024)?;
        let mut unsettled = self.journal.records.values().filter(|r| !r.state.terminal());
        let pending = unsettled.next().map(|r| DigSummary {
            key: r.plan.key().to_owned(), plan_digest: r.plan.digest(), state: r.state,
            native_phase: r.effect.as_ref().map(super::super::DigEffect::phase),
            receipt: r.effect.as_ref().and_then(super::super::DigEffect::receipt),
            dispatchable: self.source.is_some() && r.state == DigState::Prepared
                && self.journal.fresh_key.as_deref() == Some(r.plan.key()),
        });
        if unsettled.next().is_some() {
            return Err(error(ErrorCode::InternalInvariantViolation, "multiple unsettled mining keys violate journal custody"));
        }
        work.current()?;
        Ok(DigSessionView { journal_id: self.journal.id, head: self.journal.head,
            events: self.journal.events, byte_len: self.journal.raw.len(),
            total_records: self.journal.records.len(), pending })
    }

    pub fn list(&mut self, context: &OperationContext, limit: usize,
        cursor: Option<&DigCursor>) -> Result<DigPage>
    {
        let current = self.current(context)?;
        let mut page = self.journal.list(&current, limit, cursor)?;
        for row in &mut page.records { row.dispatchable &= self.source.is_some(); }
        Ok(page)
    }

    pub fn reconcile<F, G>(&mut self, key: &str, digest: Digest32,
        context: &OperationContext, factory: F, guard: &mut G) -> Result<DigRecord>
    where F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        self.recover(key, digest, context, factory, guard, false)
    }

    pub fn cancel<F, G>(&mut self, key: &str, digest: Digest32,
        context: &OperationContext, factory: F, guard: &mut G) -> Result<DigRecord>
    where F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        self.recover(key, digest, context, factory, guard, true)
    }

    fn recover<F, G>(&mut self, key: &str, digest: Digest32, context: &OperationContext,
        factory: F, guard: &mut G, cancel: bool) -> Result<DigRecord>
    where F: FnOnce(&DigBinding, DigRegion, &OperationContext) -> Result<N>,
        G: DigSessionGuard,
    {
        // Lookup, connection and coordinator work share one allowance. Do not
        // restart the deadline or refund lookup bytes before online recovery.
        let current = self.current(context)?;
        let mut work = Allowance::new(&current)?;
        self.journal.verify(&mut work)?;
        work.charge(MAX_BODY_BYTES)?;
        let record = self.journal.known(key, digest)?;
        if record.plan.before().region().halo_count() > current.budget.max_entities as usize {
            return Err(exhausted());
        }
        if !record.needs_reconciliation() { return Ok(record); }
        self.journal.online(&work.current()?, false)?;
        let region = record.plan.before().region();
        authorize(&work.current()?, self.binding().fortress_id(), record.plan.before().tick(),
            region, false, cancel, false)?;
        if work.bytes < SOURCE_RESERVATION_BYTES + self.journal_reservation() {
            return Err(exhausted());
        }
        self.abandon_preparation();
        let mut source = self.connect(region, &mut work, factory, guard)?;
        let operation = Self::reserve(&mut work, self.journal_reservation())?;
        let result = if cancel {
            self.journal.cancel(&mut source, key, digest, &operation, guard)
        } else {
            self.journal.reconcile(&mut source, key, digest, &operation, guard)
        };
        if let Ok(record) = &result { self.advance_floor(record); }
        result
    }
}

#[cfg(test)]
mod tests;
