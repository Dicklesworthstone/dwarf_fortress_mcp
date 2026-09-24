//! Session-bound orchestration of the isolated job-control/1.9 coordinator.
//!
//! Selection is evidence, never authority. Callers supply current grants on
//! every operation; this layer neither creates grants nor accepts a raw native
//! plan from an agent. The journal remains the sole route to native dispatch.

use std::time::{Duration, Instant};

use dfmcp_adapter::control_effect_journal::EffectJournalStorage;
use dfmcp_adapter::job_suspension::coordinator::{
    DurableJobRecord, DurableJobState, JobControlJournal, JobJournalSummary, JobRecordPage,
    JobSuspensionSource, job_fortress_id,
};
use dfmcp_adapter::job_suspension::rpc::{
    JobControlManifest, JobControlRpcClient, JobControlStream,
};
use dfmcp_adapter::job_suspension::{JobObservation, SuspensionPlan, validate_key};
use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, FortressId, OperationContext, Result, RiskTier,
    SessionId,
};

/// Read acquisition in addition to the fixed, non-retrying effect boundary.
pub trait SelectedJobSource: JobSuspensionSource {
    fn read_job(&mut self, job: u32, timeout: Duration) -> Result<JobObservation>;
}

impl<S: JobControlStream> SelectedJobSource for JobControlRpcClient<S> {
    fn read_job(&mut self, job: u32, timeout: Duration) -> Result<JobObservation> {
        JobControlRpcClient::read_job(self, job, timeout)
    }
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn stale() -> DfmcpError {
    error(
        ErrorCode::StaleAnchor,
        "select this job again and use its exact witness; no effect was dispatched",
    )
}

struct Selection {
    observation: JobObservation,
    manifest: JobControlManifest,
}

/// One owner of selected evidence, a source, and the durable coordination log.
/// Dropping this value releases custody; it does not cancel or erase effects.
pub struct JobControlSession<S, N> {
    id: SessionId,
    fortress: FortressId,
    journal: JobControlJournal<S>,
    source: Option<N>,
    selected: Option<Selection>,
}

impl<S: EffectJournalStorage, N: SelectedJobSource> JobControlSession<S, N> {
    /// The caller must already have opened the journal under explicit authority.
    /// `None` is offline inspection: no credential or connection is required.
    pub fn new(
        journal: JobControlJournal<S>,
        source: Option<N>,
        context: &OperationContext,
    ) -> Result<Self> {
        let summary = journal.summary(context)?;
        Ok(Self {
            id: context.session_id,
            fortress: summary.fortress_id,
            journal,
            source,
            selected: None,
        })
    }

    fn access(&self, context: &OperationContext) -> Result<()> {
        if context.session_id != self.id || context.anchor.fortress_id != self.fortress {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "job evidence belongs to another session or fortress",
            ));
        }
        context.budget.validate()?;
        self.journal.validate_access(context)
    }

    fn production(&self, context: &OperationContext) -> Result<()> {
        self.access(context)?;
        context.authorize(
            Capability::ConfigureProduction,
            RiskTier::Reversible,
            &[],
            None,
        )?;
        if context.budget.max_actions < 1 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "job control requires one action of budget",
            ));
        }
        if self.journal.summary(context)?.read_only {
            return Err(error(
                ErrorCode::CapabilityDenied,
                "offline job recovery cannot prepare, commit, or cancel effects",
            ));
        }
        Ok(())
    }

    fn no_unresolved_dispatch(&self, context: &OperationContext) -> Result<()> {
        if self.journal.summary(context)?.unresolved != 0 {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "reconcile retained unknown job effects before preparing or dispatching another key",
            ));
        }
        Ok(())
    }

    pub fn summary(&self, context: &OperationContext) -> Result<JobJournalSummary> {
        self.access(context)?;
        self.journal.summary(context)
    }

    pub fn selected(&self, context: &OperationContext) -> Result<Option<&JobObservation>> {
        self.access(context)?;
        Ok(self.selected.as_ref().map(|selected| &selected.observation))
    }

    pub fn has_source(&self, context: &OperationContext) -> Result<bool> {
        self.access(context)?;
        Ok(self.source.is_some())
    }

    /// Selected native evidence only. A failed refresh invalidates the old
    /// selection rather than quietly allowing a later plan to use stale data.
    pub fn observe(&mut self, job: u32, context: &OperationContext) -> Result<JobObservation> {
        self.access(context)?;
        if job > i32::MAX as u32 {
            return Err(error(
                ErrorCode::InvalidRequest,
                "native job ID exceeds signed 32-bit range",
            ));
        }
        if context.budget.max_entities < 1 || context.budget.max_bytes < 278_528 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "selected-job acquisition requires one entity and the bounded RPC byte reserve",
            ));
        }
        let started = Instant::now();
        let timeout = Duration::from_millis(context.budget.max_wall_millis.min(60_000));
        self.selected = None;
        let source = self.source.as_mut().ok_or_else(|| {
            error(
                ErrorCode::CapabilityDenied,
                "offline job recovery has no native source",
            )
        })?;
        let manifest = source.manifest().clone();
        let result = (|| {
            let observation = source.read_job(job, timeout)?;
            if source.manifest() != &manifest
                || observation.generation() != manifest.generation
                || observation.job_id() != job
                || job_fortress_id(&observation) != self.fortress
            {
                return Err(stale());
            }
            // Re-decode an injected source's complete evidence before retaining it.
            let observation = JobObservation::decode(observation.canonical_bytes())?;
            if started.elapsed() >= timeout {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "selected-job acquisition exceeded its deadline",
                ));
            }
            // Grant expiry must be evaluated against the newly observed tick,
            // not merely the caller's older orientation anchor.
            let mut current = context.clone();
            current.anchor.tick = dfmcp_core::GameTick(observation.tick());
            current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            Ok(observation)
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
        job: u32,
        witness: Digest32,
        context: &OperationContext,
    ) -> Result<JobObservation> {
        let selected = self.selected.as_ref().ok_or_else(stale)?;
        let observation = &selected.observation;
        if observation.job_id() != job
            || observation.witness() != witness
            || context.anchor.tick.get() != observation.tick()
            || context.anchor.state_hash != witness
        {
            return Err(stale());
        }
        let source = self.source.as_ref().ok_or_else(stale)?;
        if source.manifest() != &selected.manifest {
            return Err(stale());
        }
        Ok(observation.clone())
    }

    /// Compile from retained evidence, then durably prepare. An agent supplies
    /// intent and an expected witness, never the plan digest or prepare token.
    pub fn plan(
        &mut self,
        key: &str,
        job: u32,
        desired: bool,
        witness: Digest32,
        context: &OperationContext,
    ) -> Result<DurableJobRecord> {
        self.production(context)?;
        validate_key(key)?;
        if let Some(record) = self.journal.lookup(key, context)? {
            let plan = record.plan();
            if plan.observation().job_id() != job
                || plan.desired() != desired
                || plan.observation().witness() != witness
            {
                return Err(error(
                    ErrorCode::Conflict,
                    "job key already names different sealed intent",
                ));
            }
            // Recovery and replay neither reread the game nor renew native TTL.
            return Ok(record.clone());
        }
        self.no_unresolved_dispatch(context)?;
        let plan = SuspensionPlan::new(self.selection(job, witness, context)?, key, desired)?;
        let source = self.source.as_mut().ok_or_else(stale)?;
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
    ) -> Result<DurableJobRecord> {
        self.access(context)?;
        let record = self.journal.lookup(key, context)?.ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "no retained job effect has this key",
            )
        })?;
        if record.plan().digest() != digest {
            return Err(error(
                ErrorCode::Conflict,
                "job key and sealed plan digest disagree",
            ));
        }
        Ok(record.clone())
    }

    /// At most one journal-mediated dispatch. Unknown attempts, including an
    /// unknown OTHER key, never become authorization for another setter.
    pub fn commit(
        &mut self,
        key: &str,
        digest: Digest32,
        witness: Digest32,
        context: &OperationContext,
    ) -> Result<DurableJobRecord> {
        self.production(context)?;
        let record = self.record(key, digest, context)?;
        if record.plan().observation().witness() != witness {
            return Err(stale());
        }
        if record.state().terminal() {
            return Ok(record);
        }
        if record.state() != DurableJobState::Prepared {
            return Err(error(
                ErrorCode::EffectIndeterminate,
                "this job attempt may have executed; query its receipt instead of committing again",
            ));
        }
        self.no_unresolved_dispatch(context)?;
        let plan = record.plan().clone();
        if self.selection(plan.observation().job_id(), witness, context)? != *plan.observation() {
            return Err(stale());
        }
        self.selected = None;
        let source = self.source.as_mut().ok_or_else(stale)?;
        // The coordinator syncs DispatchStarted before its only native commit
        // call. Do not reconnect or retry here, even after an error envelope.
        match self.journal.commit(source, &plan, context) {
            Ok(record) => Ok(record),
            Err(cause) => {
                let uncertain = self.journal.poisoned()
                    || self
                        .journal
                        .lookup(key, context)
                        .map(|r| r.is_some_and(|r| r.state().reconciliation_required()))
                        .unwrap_or(true);
                if uncertain {
                    Err(error(
                        ErrorCode::EffectIndeterminate,
                        "job commit acknowledgement is uncertain; retain the key and reopen/query recovery, never redispatch",
                    ))
                } else {
                    Err(cause)
                }
            }
        }
    }

    /// Read-only native recovery. Prepared and terminal records are returned
    /// without native work; polling a preparation must not retire it as unknown.
    pub fn reconcile(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<DurableJobRecord> {
        let record = self.record(key, digest, context)?;
        if !record.state().reconciliation_required() {
            return Ok(record);
        }
        let source = self.source.as_mut().ok_or_else(|| error(ErrorCode::CapabilityDenied,
            "offline recovery preserves unknown effects; reopen in reconciliation mode to query DFHack"))?;
        self.selected = None;
        self.journal.reconcile(source, record.plan(), context)
    }

    /// A durable retirement, not a native cancellation or an undo operation.
    pub fn cancel(
        &mut self,
        key: &str,
        digest: Digest32,
        context: &OperationContext,
    ) -> Result<DurableJobRecord> {
        self.production(context)?;
        let record = self.record(key, digest, context)?;
        self.journal.cancel_before_dispatch(record.plan(), context)
    }

    pub fn records_page(
        &self,
        head: Digest32,
        after: Option<&str>,
        limit: usize,
        context: &OperationContext,
    ) -> Result<JobRecordPage> {
        self.access(context)?;
        self.journal.records_page(head, after, limit, context)
    }
}

#[cfg(test)]
#[path = "job_control_session_tests.rs"]
mod tests;
