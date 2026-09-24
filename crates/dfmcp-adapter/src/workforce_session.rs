//! Session-owned workforce selection over the existing durable coordinator.
//!
//! Connections are constructed only for an operation that needs one. Historical
//! evidence, duplicate preparation, local cancellation and permanent Unknown do
//! not reconnect. This layer never creates grants or changes native wire bytes.
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};

use crate::bounded_run::{error, validate_key};
use crate::control_effect_journal::EffectJournalStorage;
use crate::workforce_control::journal::{
    AssignmentRecord, AssignmentState, WorkforceBinding, WorkforceJournal, WorkforceMode,
    WorkforceView,
};
use crate::workforce_control::rpc::{CONNECT_BYTES, WorkforceSource, authorize};
use crate::workforce_control::{
    AssignmentPhase, AssignmentPlan, AssignmentSpec, WorkforceCapture, validate_ids,
};

fn exhausted() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::BudgetExceeded,
        "workforce session work allowance exhausted",
    )
}
fn stale() -> dfmcp_core::DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "observe the selected citizens and use that exact workforce witness",
    )
}

/// A view charges both the verified bytes and the full owned record copies.
/// Keep this aligned with WorkforceJournal::view; neither is an output budget.
pub fn view_cost(view: &WorkforceView) -> u64 {
    view.bytes as u64
        + view
            .records
            .iter()
            .map(|record| {
                record.plan().canonical_bytes().len() as u64
                    + record
                        .effect()
                        .map_or(0, |effect| effect.canonical_bytes().len() as u64)
            })
            .sum::<u64>()
}

struct Allowance {
    context: OperationContext,
    deadline: Instant,
    bytes: u64,
}
impl Allowance {
    fn new(context: OperationContext) -> Result<Self> {
        context.budget.validate()?;
        let deadline = Instant::now()
            .checked_add(Duration::from_millis(context.budget.max_wall_millis))
            .ok_or_else(exhausted)?;
        Ok(Self {
            bytes: context.budget.max_bytes,
            context,
            deadline,
        })
    }
    fn current(&self) -> Result<OperationContext> {
        let left = self
            .deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(exhausted)?;
        let millis = u64::try_from(left.as_millis()).map_err(|_| exhausted())?;
        if millis == 0 || self.bytes == 0 {
            return Err(exhausted());
        }
        let mut context = self.context.clone();
        context.budget.max_wall_millis = millis.min(context.budget.max_wall_millis);
        context.budget.max_bytes = self.bytes;
        Ok(context)
    }
    fn charge(&mut self, bytes: u64) -> Result<()> {
        self.bytes = self
            .bytes
            .checked_sub(bytes)
            .filter(|left| *left > 0)
            .ok_or_else(exhausted)?;
        self.current()?;
        Ok(())
    }
    fn connect<N, F>(&mut self, binding: &WorkforceBinding, factory: F) -> Result<N>
    where
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        let mut context = self.current()?;
        self.charge(CONNECT_BYTES)?; // Reserve before invoking a potentially blocking factory.
        context.budget.max_bytes = CONNECT_BYTES;
        let source = factory(binding, &context)?;
        self.current()?;
        Ok(source)
    }
}

pub struct WorkforceSession<S> {
    journal: WorkforceJournal<S>,
    binding: WorkforceBinding,
    id: SessionId,
    high_tick: u64,
    selected: Option<WorkforceCapture>,
}
impl<S: EffectJournalStorage> WorkforceSession<S> {
    /// Return the verified opening view too, so the caller need not replay again
    /// just to publish its opening result. Opening never selects a native capture.
    pub fn new(
        mut journal: WorkforceJournal<S>,
        context: &OperationContext,
    ) -> Result<(Self, WorkforceView)> {
        let view = journal.view(context)?;
        let high_tick = view
            .records
            .iter()
            .map(|r| r.plan().before().tick())
            .max()
            .unwrap_or(context.anchor.tick.get())
            .max(context.anchor.tick.get());
        authorize(context, view.binding.fortress(), high_tick, false)?;
        let session = Self {
            binding: view.binding.clone(),
            journal,
            id: context.session_id,
            high_tick,
            selected: None,
        };
        Ok((session, view))
    }
    pub fn binding(&self) -> &WorkforceBinding {
        &self.binding
    }
    pub fn mode(&self) -> WorkforceMode {
        self.journal.mode()
    }
    pub fn high_tick(&self) -> u64 {
        self.high_tick
    }
    pub fn clear_selection(&mut self) {
        self.selected = None;
    }

    fn context(&self, context: &OperationContext, write: bool) -> Result<OperationContext> {
        if context.session_id != self.id || (write && self.mode() != WorkforceMode::Control) {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "workforce session identity or fixed recovery mode denies control",
            ));
        }
        let mut current = context.clone();
        current.anchor.tick = GameTick(self.high_tick.max(current.anchor.tick.get()));
        authorize(
            &current,
            self.binding.fortress(),
            current.anchor.tick.get(),
            write,
        )?;
        Ok(current)
    }
    fn planning(&self, context: &OperationContext) -> Result<OperationContext> {
        let current = self.context(context, true)?;
        current.authorize(Capability::Plan, RiskTier::Guarded, &[], None)?;
        Ok(current)
    }
    fn view_with(&mut self, budget: &mut Allowance) -> Result<WorkforceView> {
        let view = self.journal.view(&budget.current()?)?;
        budget.charge(view_cost(&view))?;
        for record in &view.records {
            self.high_tick = self.high_tick.max(record.plan().before().tick());
        }
        budget.context.anchor.tick = GameTick(self.high_tick.max(budget.context.anchor.tick.get()));
        self.context(&budget.current()?, false)?;
        Ok(view)
    }
    pub fn view(&mut self, context: &OperationContext) -> Result<WorkforceView> {
        let mut budget = Allowance::new(self.context(context, false)?)?;
        self.view_with(&mut budget)
    }
    fn record(view: &WorkforceView, key: &str, digest: Digest32) -> Result<AssignmentRecord> {
        validate_key(key)?;
        let record = view
            .records
            .iter()
            .find(|r| r.plan().key() == key)
            .ok_or_else(|| {
                error(
                    ErrorCode::InvalidRequest,
                    "assignment key absent from this journal",
                )
            })?;
        if record.plan().digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "assignment key and reviewed digest disagree",
            ));
        }
        Ok(record.clone())
    }
    pub fn inspect(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<AssignmentRecord> {
        Self::record(&self.view(context)?, key, digest)
    }

    pub fn observe<N, F>(
        &mut self,
        ids: &[u32],
        context: &OperationContext,
        factory: F,
    ) -> Result<WorkforceCapture>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        self.clear_selection(); // Includes invalid, unauthorized and failed refreshes.
        let mut budget = Allowance::new(self.context(context, false)?)?;
        if self.mode() == WorkforceMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline workforce recovery cannot acquire native state",
            ));
        }
        validate_ids(ids)?;
        self.view_with(&mut budget)?; // Refuse changed custody before sending credentials.
        let mut source = budget.connect(&self.binding, factory)?;
        let capture = self.journal.observe(&mut source, ids, &budget.current()?)?;
        if capture.tick() < self.high_tick {
            return Err(stale());
        }
        self.high_tick = capture.tick();
        self.context(&budget.current()?, false)?;
        self.selected = Some(capture.clone());
        Ok(capture)
    }

    pub fn prepare<N, F>(
        &mut self,
        key: &str,
        spec: AssignmentSpec,
        witness: Digest32,
        context: &OperationContext,
        factory: F,
    ) -> Result<AssignmentRecord>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        validate_key(key)?;
        let mut budget = Allowance::new(self.planning(context)?)?;
        let view = self.view_with(&mut budget)?;
        self.planning(&budget.current()?)?;
        // An exact retry discovers durable intent even after restart. It does
        // not reacquire a connection, renew a preparation, or dispatch an effect.
        if let Some(old) = view.records.iter().find(|r| r.plan().key() == key) {
            if old.plan().spec() != spec || old.plan().before().witness() != witness {
                return Err(error(
                    ErrorCode::Conflict,
                    "assignment key already binds another specification or witness",
                ));
            }
            return Ok(old.clone());
        }
        if view.records.iter().any(|r| !r.state().settled()) {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "unsettled workforce work blocks a new key",
            ));
        }
        let before = self
            .selected
            .as_ref()
            .filter(|c| c.witness() == witness)
            .cloned()
            .ok_or_else(stale)?;
        let plan = AssignmentPlan::new(key, spec, before)?;
        self.clear_selection();
        let mut source = budget.connect(&self.binding, factory)?;
        self.journal.prepare(&mut source, &plan, &budget.current()?)
    }

    pub fn commit<N, F>(
        &mut self,
        key: &str,
        digest: Digest32,
        confirmed: bool,
        context: &OperationContext,
        factory: F,
    ) -> Result<AssignmentRecord>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        let mut budget = Allowance::new(self.context(context, true)?)?;
        if !confirmed {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "assignment commit requires explicit confirmation of its reviewed digest",
            ));
        }
        let record = Self::record(&self.view_with(&mut budget)?, key, digest)?;
        self.context(&budget.current()?, true)?;
        if record.state().settled() {
            return Ok(record);
        }
        if record.state() != AssignmentState::Prepared {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "assignment dispatch cannot be retried; inspect or reconcile instead",
            ));
        }
        self.clear_selection();
        let mut source = budget.connect(&self.binding, factory)?;
        // The journal re-observes the entire original paused workforce capture
        // and syncs DispatchStarted before the native assignment boundary.
        self.journal
            .commit(&mut source, key, digest, &budget.current()?)
    }

    pub fn reconcile<N, F>(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        factory: F,
    ) -> Result<AssignmentRecord>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        let mut budget = Allowance::new(self.context(context, false)?)?;
        let record = Self::record(&self.view_with(&mut budget)?, key, digest)?;
        if !record.needs_query() {
            return Ok(record);
        }
        if self.mode() == WorkforceMode::Offline {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline uncertainty requires explicit recover-mode reopening",
            ));
        }
        self.clear_selection();
        let mut source = budget.connect(&self.binding, factory)?;
        self.journal
            .reconcile(&mut source, key, digest, &budget.current()?)
    }

    pub fn cancel<N, F>(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
        factory: F,
    ) -> Result<AssignmentRecord>
    where
        N: WorkforceSource,
        F: FnOnce(&WorkforceBinding, &OperationContext) -> Result<N>,
    {
        let mut budget = Allowance::new(self.context(context, true)?)?;
        let record = Self::record(&self.view_with(&mut budget)?, key, digest)?;
        self.context(&budget.current()?, true)?;
        if record.state().settled()
            || record
                .effect()
                .is_some_and(|e| e.phase() == AssignmentPhase::Unknown)
        {
            return Ok(record); // Permanent Unknown stays unresolved, not "cancelled".
        }
        self.clear_selection();
        if matches!(
            record.state(),
            AssignmentState::Intent | AssignmentState::Prepared
        ) {
            return self
                .journal
                .cancel::<N>(None, key, digest, &budget.current()?);
        }
        let mut source = budget.connect(&self.binding, factory)?;
        self.journal
            .cancel(Some(&mut source), key, digest, &budget.current()?)
    }
}

#[cfg(test)]
mod tests;
