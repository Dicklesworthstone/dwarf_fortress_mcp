//! One foreground owner for furniture observation, preparation and dispatch.
//!
//! The original native connection survives between calls. Commit has no source
//! factory, and every recovery/refresh abandons local dispatch permission.
use std::time::Duration;

use dfmcp_core::{Digest32, ErrorCode, GameTick, OperationContext, Result};

use super::journal::{
    BuildEntry, BuildGuard, BuildInventory, BuildJournal, BuildMode, BuildSource, BuildStage,
    BuildState, MAX_BODY_BYTES, MAX_FRAME_BYTES, RPC_RESERVE, SOURCE_RESERVE, Work, authorize,
    error, exhausted, uncertain,
};
use super::{BuildBinding, BuildCapture, BuildNativeSummary, BuildPlan, BuildSelection};
use crate::control_effect_journal::EffectJournalStorage;

pub struct BuildSession<S, N> {
    journal: BuildJournal<S>,
    source: Option<N>,
    selected: Option<BuildCapture>,
    last_native_summary: Option<BuildNativeSummary>,
    high_tick: u64,
    high_sequence: u64,
}
impl<S: EffectJournalStorage, N: BuildSource> BuildSession<S, N> {
    pub fn new(mut journal: BuildJournal<S>, context: &OperationContext) -> Result<Self> {
        journal.abandon();
        let mut work = Work::new(context)?;
        journal.verify(&mut work)?;
        let high_tick = context.anchor.tick.get().max(journal.high_tick());
        let high_sequence = journal.high_sequence();
        Ok(Self {
            journal,
            source: None,
            selected: None,
            last_native_summary: None,
            high_tick,
            high_sequence,
        })
    }
    pub fn binding(&self) -> &BuildBinding {
        self.journal.binding()
    }
    pub fn mode(&self) -> BuildMode {
        self.journal.mode()
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn selected(&self) -> Option<&BuildCapture> {
        self.selected.as_ref()
    }
    /// Historical metadata from the latest validated native reply. Retaining it
    /// after abandonment avoids confusing an empty local journal with native
    /// readiness; it never substitutes for a new native preflight.
    pub fn native_summary(&self) -> Option<BuildNativeSummary> {
        self.last_native_summary
    }
    pub fn is_fenced(&self) -> bool {
        self.journal.is_fenced()
    }
    pub fn has_preparation_connection(&self) -> bool {
        self.source.is_some() && self.journal.has_permit()
    }
    pub fn abandon_preparation(&mut self) {
        if let Some(source) = self.source.as_mut() {
            self.last_native_summary = Some(source.native_summary());
            source.fence();
        }
        self.source = None;
        self.selected = None;
        self.journal.abandon();
    }
    fn current(&self, context: &OperationContext) -> Result<OperationContext> {
        let mut current = context.clone();
        current.anchor.tick = GameTick(self.high_tick.max(current.anchor.tick.get()));
        self.journal.access(&current)
    }
    /// Conservative whole-call allowance. The presentation layer additionally
    /// reserves complete output and its final custody/inventory read.
    pub fn operation_reserve(&self, connect: bool) -> u64 {
        self.journal.operation_reserve()
            + 4 * (self.journal.byte_len() + 2 * MAX_FRAME_BYTES) as u64
            + if connect {
                SOURCE_RESERVE + RPC_RESERVE
            } else {
                0
            }
    }
    fn begin(&mut self, context: &OperationContext, connect: bool) -> Result<Work> {
        let current = self.current(context)?;
        if current.budget.max_bytes < self.operation_reserve(connect) {
            return Err(exhausted());
        }
        let mut work = Work::new(&current)?;
        self.journal.verify(&mut work)?;
        Ok(work)
    }
    fn connect<F, G>(
        &self,
        selection: BuildSelection,
        plan: Option<&BuildPlan>,
        exact: bool,
        work: &mut Work,
        factory: F,
        guard: &mut G,
    ) -> Result<N>
    where
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
    {
        let mut context = work.reserve(SOURCE_RESERVE)?;
        guard.check(
            BuildStage::Connect,
            self.binding(),
            plan,
            selection,
            &context,
        )?;
        context.budget.max_wall_millis = work.remaining()?.as_millis() as u64;
        let mut source = factory(self.binding(), &context, work.remaining()?)?;
        let checked = (|| {
            self.binding().source_matches(source.binding(), exact)?;
            guard.check(
                BuildStage::Connect,
                self.binding(),
                plan,
                selection,
                &work.current()?,
            )
        })();
        if let Err(cause) = checked {
            source.fence();
            return Err(cause);
        }
        Ok(source)
    }
    /// A refresh explicitly retires a prior local preparation connection. Its
    /// durable native obligation remains visible and continues fencing new keys.
    pub fn observe<F, G>(
        &mut self,
        selection: BuildSelection,
        context: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<BuildCapture>
    where
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
    {
        self.journal.online(true)?;
        let current = authorize(
            &self.current(context)?,
            self.binding(),
            self.high_tick,
            BuildStage::Observe,
        )?;
        self.abandon_preparation();
        let result = (|| {
            let mut work = self.begin(&current, true)?;
            guard.check(
                BuildStage::Observe,
                self.binding(),
                None,
                selection,
                &work.current()?,
            )?;
            let mut source = self.connect(selection, None, true, &mut work, factory, guard)?;
            let observed = (|| {
                let mut request = work.reserve(RPC_RESERVE)?;
                guard.check(
                    BuildStage::Observe,
                    self.binding(),
                    None,
                    selection,
                    &request,
                )?;
                request.budget.max_wall_millis = work.remaining()?.as_millis() as u64;
                let capture = source.observe(selection, &request, work.remaining()?)?;
                self.binding().source_matches(source.binding(), true)?;
                if !self.binding().capture_matches(&capture)
                    || capture.selection() != selection
                    || capture.tick() < self.high_tick
                    || capture.sequence() < self.high_sequence
                {
                    return Err(error(
                        ErrorCode::StaleAnchor,
                        "furniture capture regressed or changed its selected source",
                    ));
                }
                // Valid source clocks advance the authority floor even if the
                // new tick proves the caller's grant has already expired.
                self.high_tick = self.high_tick.max(capture.tick());
                self.high_sequence = self.high_sequence.max(capture.sequence());
                let current = authorize(
                    &work.current()?,
                    self.binding(),
                    capture.tick(),
                    BuildStage::Observe,
                )?;
                guard.check(
                    BuildStage::Observe,
                    self.binding(),
                    None,
                    selection,
                    &current,
                )?;
                self.journal.verify(&mut work)?;
                Ok(capture)
            })();
            self.last_native_summary = Some(source.native_summary());
            match observed {
                Ok(capture) => {
                    self.high_tick = self.high_tick.max(capture.tick());
                    self.high_sequence = self.high_sequence.max(capture.sequence());
                    self.selected = Some(capture.clone());
                    self.source = Some(source);
                    Ok(capture)
                }
                Err(cause) => {
                    source.fence();
                    Err(cause)
                }
            }
        })();
        if result.is_err() {
            self.abandon_preparation();
        }
        result
    }
    pub fn prepare<G: BuildGuard>(
        &mut self,
        key: &str,
        witness: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<BuildEntry> {
        self.journal.online(true)?;
        let mut work = self.begin(context, false)?;
        let capture = self
            .selected
            .as_ref()
            .filter(|capture| capture.witness() == witness)
            .ok_or_else(|| {
                error(
                    ErrorCode::StaleAnchor,
                    "furniture planning requires this owner's exact retained observation",
                )
            })?;
        let plan = BuildPlan::new(key, capture.clone())?;
        let request = work.reserve(self.journal.operation_reserve())?;
        let source = self.source.as_mut().ok_or_else(|| uncertain(key))?;
        let result = self.journal.prepare(source, &plan, &request, guard);
        self.last_native_summary = Some(source.native_summary());
        self.selected = None;
        if result
            .as_ref()
            .is_ok_and(|entry| entry.state() == BuildState::Prepared)
            && self.journal.has_permit()
        {
            if let Err(cause) = self.journal.verify(&mut work) {
                self.abandon_preparation();
                return Err(cause);
            }
        } else {
            self.abandon_preparation();
        }
        result
    }
    /// The exact digest is confirmation input; supervising policy must validate
    /// its own review seal. There is intentionally no reconnection parameter.
    pub fn commit<G: BuildGuard>(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        guard: &mut G,
    ) -> Result<BuildEntry> {
        self.journal.online(true)?;
        let mut work = self.begin(context, false)?;
        let entry = self.journal.known(key, digest)?;
        if !entry.needs_reconciliation() {
            return Ok(entry);
        }
        let request = work.reserve(self.journal.operation_reserve())?;
        let source = self.source.as_mut().ok_or_else(|| uncertain(key))?;
        let result = self.journal.commit(source, key, digest, &request, guard);
        self.abandon_preparation();
        self.high_tick = self.high_tick.max(self.journal.high_tick());
        self.high_sequence = self.high_sequence.max(self.journal.high_sequence());
        result
    }
    pub fn get(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<BuildEntry> {
        let current = self.current(context)?;
        self.journal.get(key, digest, &current)
    }
    pub fn inventory(&mut self, context: &OperationContext) -> Result<BuildInventory> {
        let current = self.current(context)?;
        self.journal.inventory(&current)
    }
    pub fn recover<F, G>(
        &mut self,
        key: &str,
        digest: Digest32,
        cancel: bool,
        context: &OperationContext,
        factory: F,
        guard: &mut G,
    ) -> Result<BuildEntry>
    where
        F: FnOnce(&BuildBinding, &OperationContext, Duration) -> Result<N>,
        G: BuildGuard,
    {
        let current = self.current(context)?;
        let mut work = Work::new(&current)?;
        self.journal.verify(&mut work)?;
        work.charge(MAX_BODY_BYTES)?;
        let entry = self.journal.known(key, digest)?;
        if !entry.needs_reconciliation() {
            return Ok(entry);
        }
        self.journal.online(false)?;
        authorize(
            &current,
            self.binding(),
            self.high_tick,
            if cancel {
                BuildStage::Cancel
            } else {
                BuildStage::Query
            },
        )?;
        self.abandon_preparation();
        if work.current()?.budget.max_bytes < SOURCE_RESERVE + self.journal.operation_reserve() {
            return Err(exhausted());
        }
        let mut source = self.connect(
            entry.plan().before().selection(),
            Some(entry.plan()),
            false,
            &mut work,
            factory,
            guard,
        )?;
        let request = work.reserve(self.journal.operation_reserve())?;
        let result = self
            .journal
            .recover(&mut source, key, digest, cancel, &request, guard);
        self.last_native_summary = Some(source.native_summary());
        source.fence();
        self.high_tick = self.high_tick.max(self.journal.high_tick());
        self.high_sequence = self.high_sequence.max(self.journal.high_sequence());
        result
    }
}
