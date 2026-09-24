//! Session-owned queue selection and sealed creation intent; never grant synthesis.
use super::{
    Allowance, CreationPage, CreationRecord, CreationState, CreationSummary, EffectJournalStorage,
    JournalMode, RPC_RESERVE_BYTES, WorkOrderJournal, WorkOrderManifest, WorkOrderObservation,
    WorkOrderPlan, WorkOrderSource, WorkOrderSpec, error, exhausted, validate_key,
};
use dfmcp_core::{
    Capability, Digest32, ErrorCode, GameTick, OperationContext, Result, RiskTier, SessionId,
};

struct Selection {
    observation: WorkOrderObservation,
    manifest: WorkOrderManifest,
}

pub struct WorkOrderSession<S, N> {
    id: SessionId,
    journal: WorkOrderJournal<S>,
    source: Option<N>,
    selected: Option<Selection>,
}
impl<S: EffectJournalStorage, N: WorkOrderSource> WorkOrderSession<S, N> {
    pub fn new(
        journal: WorkOrderJournal<S>,
        source: Option<N>,
        context: &OperationContext,
    ) -> Result<Self> {
        journal.validate_access(context)?;
        if journal.mode() == JournalMode::Offline && source.is_some() {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline creation recovery cannot retain a native connection",
            ));
        }
        Ok(Self {
            id: context.session_id,
            journal,
            source,
            selected: None,
        })
    }
    fn access(&self, context: &OperationContext) -> Result<()> {
        if context.session_id != self.id {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "creation evidence belongs to another session",
            ));
        }
        self.journal.validate_access(context)
    }
    fn production(&self, context: &OperationContext) -> Result<()> {
        self.access(context)?;
        self.journal.writable(context, true)
    }
    pub fn summary(&self, context: &OperationContext) -> Result<CreationSummary> {
        self.access(context)?;
        self.journal.summary(context)
    }
    pub fn has_source(&self, context: &OperationContext) -> Result<bool> {
        self.access(context)?;
        Ok(self.source.is_some())
    }
    pub fn selected(&self, context: &OperationContext) -> Result<Option<&WorkOrderObservation>> {
        self.access(context)?;
        Ok(self
            .selected
            .as_ref()
            .map(|selection| &selection.observation))
    }
    /// A failed refresh never leaves the old witness eligible for new control.
    pub fn observe(&mut self, context: &OperationContext) -> Result<WorkOrderObservation> {
        self.access(context)?;
        self.selected = None;
        let mut budget = Allowance::new(context)?;
        budget.charge(RPC_RESERVE_BYTES)?;
        let source = self.source.as_mut().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "offline recovery has no native queue source",
            )
        })?;
        let manifest = source.manifest().clone();
        let result = (|| {
            let value = source.read_orders(budget.remaining()?)?;
            let value = WorkOrderObservation::decode(value.canonical_bytes())?;
            if source.manifest() != &manifest
                || value.generation() != manifest.generation
                || value.fortress_id() != context.anchor.fortress_id
            {
                return Err(error(
                    ErrorCode::StaleAnchor,
                    "native queue source identity changed",
                ));
            }
            if value.order_ids().len() > context.budget.max_entities as usize {
                return Err(exhausted());
            }
            let mut current = context.clone();
            current.anchor.tick = GameTick(value.tick());
            current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            budget.remaining()?;
            Ok(value)
        })();
        if result.is_err() {
            source.fence();
        }
        let observation = result?;
        self.access(context)?;
        self.selected = Some(Selection {
            observation: observation.clone(),
            manifest,
        });
        Ok(observation)
    }
    fn selection(
        &self,
        witness: Digest32,
        context: &OperationContext,
    ) -> Result<WorkOrderObservation> {
        let stale = || {
            error(
                ErrorCode::StaleAnchor,
                "observe the current queue and use its exact witness before creation",
            )
        };
        let selected = self.selected.as_ref().ok_or_else(stale)?;
        let source = self.source.as_ref().ok_or_else(stale)?;
        if source.manifest() != &selected.manifest || selected.observation.witness() != witness {
            return Err(stale());
        }
        if context.anchor.state_hash != witness
            || context.anchor.tick.get() != selected.observation.tick()
        {
            return Err(stale());
        }
        Ok(selected.observation.clone())
    }
    pub fn plan(
        &mut self,
        key: &str,
        spec: WorkOrderSpec,
        witness: Digest32,
        context: &OperationContext,
    ) -> Result<CreationRecord> {
        self.production(context)?;
        validate_key(key)?;
        if let Some(record) = self.journal.lookup(key, context)? {
            if record.plan().spec() != spec || record.plan().observation().witness() != witness {
                return Err(error(
                    ErrorCode::Conflict,
                    "creation key already binds another spec or queue witness",
                ));
            }
            return Ok(record.clone()); // No native call or TTL renewal.
        }
        self.journal.no_uncertainty()?;
        let plan = WorkOrderPlan::new(self.selection(witness, context)?, key, spec)?;
        let source = self.source.as_mut().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "creation requires a connection",
            )
        })?;
        let result = self.journal.prepare(source, &plan, context);
        if result.is_err() {
            self.selected = None;
        }
        result
    }
    pub fn record(
        &self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<CreationRecord> {
        self.access(context)?;
        let record = self.journal.lookup(key, context)?.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "no retained creation record has this key",
            )
        })?;
        if record.plan().digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "creation key and digest disagree",
            ));
        }
        Ok(record.clone())
    }
    pub fn commit(
        &mut self,
        key: &str,
        digest: Digest32,
        witness: Digest32,
        context: &OperationContext,
    ) -> Result<CreationRecord> {
        self.production(context)?;
        let record = self.record(key, digest, context)?;
        if record.plan().observation().witness() != witness {
            return Err(error(
                ErrorCode::StaleAnchor,
                "commit witness differs from the sealed creation plan",
            ));
        }
        if record.state().terminal() {
            return Ok(record);
        }
        if record.state() != CreationState::Prepared {
            return Err(super::uncertain());
        }
        self.journal.no_uncertainty()?;
        if self.selection(witness, context)? != *record.plan().observation() {
            return Err(error(
                ErrorCode::StaleAnchor,
                "selected queue no longer matches preparation",
            ));
        }
        self.selected = None;
        let source = self.source.as_mut().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "creation requires a connection",
            )
        })?;
        self.journal.commit(source, record.plan(), context)
    }
    /// Prepared/terminal replay does not contact the bridge or modify evidence.
    pub fn reconcile(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<CreationRecord> {
        let record = self.record(key, digest, context)?;
        if !record.state().unresolved() {
            return Ok(record);
        }
        let source = self.source.as_mut().ok_or_else(|| error(ErrorCode::CapabilityDenied,
            "offline uncertainty is retained; reopen in reconcile mode to query native receipts"))?;
        self.selected = None;
        self.journal.reconcile(source, record.plan(), context)
    }
    pub fn cancel(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<CreationRecord> {
        self.production(context)?;
        let record = self.record(key, digest, context)?;
        self.journal.cancel(record.plan(), context)
    }
    pub fn records_page(
        &self,
        head: Digest32,
        after: Option<&str>,
        limit: usize,
        context: &OperationContext,
    ) -> Result<CreationPage> {
        self.access(context)?;
        self.journal.records_page(head, after, limit, context)
    }
}

#[cfg(test)]
mod tests;
