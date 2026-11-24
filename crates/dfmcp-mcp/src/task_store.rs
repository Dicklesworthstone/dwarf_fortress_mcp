//! Bounded process-local Tasks custody for the laboratory only.
//!
//! The pinned upstream store owns every Tasks state transition and dispatch
//! fence. This wrapper bounds concurrent monitors globally and per session,
//! preserving session-bound discovery and original game-work custody.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration as StdDuration, Instant};

use asupersync::Cx;
use fastmcp_core::crypto::Sha256Digest;
use fastmcp_rust::{
    FinalTask, FinalTaskAcceptedInputClaim, FinalTaskAccessMetadata, FinalTaskExecutionBudget,
    FinalTaskExecutionLimits, FinalTaskId, FinalTaskInitialWorkClaim, FinalTaskInputResponses,
    FinalTaskOperation, FinalTaskRetentionDeadline, FinalTaskSnapshot, FinalTaskStatusNotification,
    FinalTaskStore, FinalTaskWorkDescriptor, InMemoryFinalTaskStore, McpError, McpResult,
    TaskRetentionAuditPage, TaskRetentionPolicy, TaskRetentionReason, TaskRetentionReclamation,
};
use serde_json::{Value, json};

use crate::task_service::TaskWork;

/// Terminal task history is retained until process exit, with an explicit
/// hard bound. Task TTL therefore cannot erase an unfinished game obligation.
pub(crate) const MAX_RETAINED_TASKS: usize = 256;
/// One monitor owns each session; independent sessions share a bounded runner.
pub(crate) const MAX_ACTIVE_TASKS: usize = 8;

#[derive(Clone)]
struct TaskRecord {
    work: TaskWork,
    /// Group-only immutable membership, independent of later proof currentness.
    observation_binding: Option<Value>,
    progress: Value,
    ordinal: usize,
    final_evidence: Option<Value>,
    terminal_failure: Option<String>,
    /// Framework execution expiry ends the monitor, never its original game
    /// work. Keep that failure separate from immutable game-proof evidence.
    execution_failure: Option<Value>,
    dispatch_started: bool,
    /// The application supervisor future still owns this monitor, even if its
    /// terminal wire transition has just been published.
    supervisor_active: bool,
}

pub(crate) struct LabTaskStore {
    inner: InMemoryFinalTaskStore,
    records: Mutex<BTreeMap<FinalTaskId, TaskRecord>>,
}

pub(crate) struct MonitorScope {
    store: Arc<LabTaskStore>,
    id: FinalTaskId,
}

impl Drop for MonitorScope {
    fn drop(&mut self) {
        // This only releases process-local supervisor bookkeeping. Poisoning
        // still makes later ordinary store access fail closed.
        let mut records = match self.store.records.lock() {
            Ok(records) => records,
            Err(poison) => poison.into_inner(),
        };
        if let Some(record) = records.get_mut(&self.id) {
            record.supervisor_active = false;
        }
    }
}

impl LabTaskStore {
    pub(crate) fn new() -> McpResult<Arc<Self>> {
        Self::with_clock(Arc::new(Instant::now))
    }

    fn with_clock(clock: Arc<dyn Fn() -> Instant + Send + Sync>) -> McpResult<Arc<Self>> {
        let inner = InMemoryFinalTaskStore::with_clock_payload_and_execution_limits(
            MAX_RETAINED_TASKS,
            InMemoryFinalTaskStore::DEFAULT_MAX_PAYLOAD_BYTES,
            FinalTaskExecutionLimits::new(
                StdDuration::from_secs(24 * 60 * 60),
                StdDuration::from_secs(60 * 60),
                8,
            )?,
            clock,
        )?
        // Upstream owners are authenticated principals. All unprotected
        // sessions share one principal; local session admission is below.
        .with_execution_capacity(MAX_ACTIVE_TASKS, MAX_ACTIVE_TASKS)?
        .with_owner_capacity(
            MAX_RETAINED_TASKS,
            InMemoryFinalTaskStore::DEFAULT_MAX_PAYLOAD_BYTES,
        )?;
        Ok(Arc::new(Self {
            inner,
            records: Mutex::new(BTreeMap::new()),
        }))
    }

    #[cfg(test)]
    pub(crate) fn with_test_clock(
        clock: Arc<dyn Fn() -> Instant + Send + Sync>,
    ) -> McpResult<Arc<Self>> {
        Self::with_clock(clock)
    }

    fn records(&self) -> McpResult<MutexGuard<'_, BTreeMap<FinalTaskId, TaskRecord>>> {
        self.records
            .lock()
            .map_err(|_| McpError::internal_error("laboratory task custody poisoned"))
    }

    fn create_owned(
        &self,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
        descriptor: FinalTaskWorkDescriptor,
        principal: Option<Sha256Digest>,
        operation: Option<FinalTaskOperation>,
    ) -> McpResult<FinalTaskSnapshot> {
        let work = TaskWork::parse(descriptor.as_value())?;
        let observation_binding = if work.is_set_observer() {
            let observed = work
                .observation_view()
                .map_err(|error| McpError::invalid_params(error.to_string()))?;
            Some(work.admitted_set_binding(&observed.payload)?)
        } else if work.is_observer() {
            // Recheck the current complete census at the creation boundary.
            // Do not hold the Task record lock while acquiring a session lock.
            work.validate_observation()?;
            None
        } else {
            None
        };
        // Admission and the upstream create share this lock. Concurrent task
        // calls cannot exceed the process or canonical-session allowance.
        let mut records = self.records()?;
        // Validation can finish before a concurrent explicit session close.
        // Close holds this custody lock through registration removal, so this
        // late check cannot create work for an already released owner.
        crate::server::with_registered_session(work.session_id(), || ())
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        if records.len() >= MAX_RETAINED_TASKS {
            return Err(McpError::invalid_params(
                "laboratory retained task capacity reached; no plan was dispatched",
            ));
        }
        let mut active = 0;
        for (id, record) in records.iter() {
            let retained = self.inner.get_task(id)?.ok_or_else(|| {
                McpError::internal_error(
                    "retained laboratory task is unavailable; creation refused",
                )
            })?;
            if record.supervisor_active
                || matches!(
                    retained,
                    FinalTask::Working(_) | FinalTask::InputRequired { .. }
                )
            {
                if record.work.session_id() == work.session_id() {
                    return Err(McpError::invalid_params(
                        "one laboratory task monitor already owns this session; finish or cancel it before creating another task; no plan was dispatched",
                    ));
                }
                active += 1;
            }
        }
        if active >= MAX_ACTIVE_TASKS {
            return Err(McpError::invalid_params(
                "laboratory active task capacity reached; at most eight session monitors may run; no plan was dispatched",
            ));
        }
        let id = task.base().task_id.clone();
        let ordinal = records.len();
        let committed = match (operation, principal) {
            (Some(operation), principal) => self.inner.create_task_with_operation(
                task,
                notification,
                descriptor,
                principal,
                operation,
            )?,
            (None, Some(principal)) => self.inner.create_task_with_authenticated_work(
                task,
                notification,
                descriptor,
                principal,
            )?,
            (None, None) => self
                .inner
                .create_task_with_work(task, notification, descriptor)?,
        };
        records.insert(
            id,
            TaskRecord {
                work,
                observation_binding,
                progress: json!({"stage": "queued", "effects_dispatched": false}),
                ordinal,
                final_evidence: None,
                terminal_failure: None,
                execution_failure: None,
                dispatch_started: false,
                supervisor_active: false,
            },
        );
        Ok(committed)
    }

    pub(crate) fn monitor_scope(self: &Arc<Self>, id: &FinalTaskId) -> McpResult<MonitorScope> {
        let mut records = self.records()?;
        let record = records
            .get_mut(id)
            .ok_or_else(|| McpError::internal_error("laboratory task binding missing"))?;
        crate::server::with_registered_session(record.work.session_id(), || ())
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        if record.supervisor_active {
            return Err(McpError::internal_error(
                "laboratory monitor already has an owned supervisor",
            ));
        }
        record.supervisor_active = true;
        Ok(MonitorScope {
            store: self.clone(),
            id: id.clone(),
        })
    }

    /// Hold the same custody lock used by creation until the session has
    /// published its final close receipt and removed its registration.
    pub(crate) fn with_session_idle<T>(
        &self,
        session_id: &str,
        publish: impl FnOnce(usize) -> dfmcp_core::Result<T>,
    ) -> dfmcp_core::Result<T> {
        let unavailable = |message: String| {
            dfmcp_core::DfmcpError::new(dfmcp_core::ErrorCode::InternalInvariantViolation, message)
        };
        let records = self
            .records()
            .map_err(|error| unavailable(error.to_string()))?;
        let mut terminal = 0;
        for (id, record) in records
            .iter()
            .filter(|(_, record)| record.work.session_id().eq_ignore_ascii_case(session_id))
        {
            let task = self
                .inner
                .get_task(id)
                .map_err(|error| unavailable(error.to_string()))?
                .ok_or_else(|| {
                    unavailable("retained laboratory task is unavailable; close refused".to_owned())
                })?;
            if record.supervisor_active
                || matches!(
                    task,
                    FinalTask::Working(_) | FinalTask::InputRequired { .. }
                )
            {
                return Err(dfmcp_core::DfmcpError::new(
                    dfmcp_core::ErrorCode::Conflict,
                    "a Task monitor still owns this session; finish or cancel the monitor before close",
                ));
            }
            terminal += 1;
        }
        publish(terminal)
    }

    pub(crate) fn record_progress(&self, id: &FinalTaskId, progress: Value) -> McpResult<()> {
        let mut records = self.records()?;
        let record = records
            .get_mut(id)
            .ok_or_else(|| McpError::internal_error("laboratory task binding missing"))?;
        record.progress = progress;
        Ok(())
    }

    fn original_work(&self, id: &FinalTaskId) -> McpResult<TaskWork> {
        self.records()?
            .get(id)
            .map(|record| record.work.clone())
            .ok_or_else(|| McpError::invalid_params("Task not found"))
    }

    /// Access metadata must not clone original work or observation payloads.
    fn readable_task(&self, id: &FinalTaskId) -> McpResult<bool> {
        let session_id = {
            let records = self.records()?;
            let Some(record) = records.get(id) else {
                return Ok(false);
            };
            record.work.session_id().to_owned()
        };
        crate::server::task_session::read_authority(&session_id)
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        Ok(true)
    }

    pub(crate) fn terminal_evidence(
        &self,
        id: &FinalTaskId,
    ) -> McpResult<Option<(Value, Option<String>)>> {
        let records = self.records()?;
        let record = records
            .get(id)
            .ok_or_else(|| McpError::invalid_params("Task not found"))?;
        Ok(record
            .final_evidence
            .clone()
            .map(|evidence| (evidence, record.terminal_failure.clone())))
    }

    /// A second monitor of the same commit retains the first completed action
    /// evidence. Current goal truth and physical-work admission are still read
    /// by task_session; historical quiescence must never hide newly active work.
    pub(crate) fn retain_original_actions(
        &self,
        work: &TaskWork,
        payload: &mut Value,
    ) -> McpResult<()> {
        if work.is_observer() {
            return Ok(());
        }
        if payload["physical_quiescent"] != true {
            return Ok(());
        }
        let records = self.records()?;
        let prior = records
            .values()
            .filter(|record| {
                record.work.session_id() == work.session_id()
                    && record.work.plan_digest() == work.plan_digest()
                    && !record.work.is_observer()
                    && record.terminal_failure.is_none()
            })
            .filter_map(|record| {
                record
                    .final_evidence
                    .as_ref()
                    .map(|value| (record.ordinal, value))
            })
            .filter(|(_, value)| value["status"] == "completed")
            .min_by_key(|(ordinal, _)| *ordinal);
        if let Some((_, prior)) = prior
            && same_drained_receipts(&prior["actions"], &payload["actions"])
        {
            payload["actions"] = prior["actions"].clone();
            payload["action_evidence_scope"] =
                json!("original_terminal_receipts_and_quiescence_at_their_observed_anchors");
        }
        Ok(())
    }

    pub(crate) fn retain_final_evidence(
        &self,
        id: &FinalTaskId,
        evidence: &Value,
        failure: Option<&str>,
    ) -> McpResult<()> {
        let mut records = self.records()?;
        let record = records
            .get_mut(id)
            .ok_or_else(|| McpError::invalid_params("Task not found"))?;
        match &record.final_evidence {
            Some(retained) if retained != evidence => Err(McpError::invalid_params(
                "terminal task evidence is immutable",
            )),
            Some(_) => Ok(()),
            None => {
                record.final_evidence = Some(evidence.clone());
                record.terminal_failure = failure.map(str::to_owned);
                Ok(())
            }
        }
    }

    pub(crate) fn mark_dispatch_started(&self, id: &FinalTaskId) -> McpResult<bool> {
        let mut records = self.records()?;
        let record = records
            .get_mut(id)
            .ok_or_else(|| McpError::invalid_params("Task not found"))?;
        if record.work.is_observer() {
            return Err(McpError::invalid_params(
                "observation Tasks cannot enter original dispatch",
            ));
        }
        let previously_started = record.dispatch_started;
        record.dispatch_started = true;
        Ok(previously_started)
    }

    pub(crate) fn wire_task(&self, id: &FinalTaskId) -> McpResult<Value> {
        let task = self
            .inner
            .get_task(id)?
            .ok_or_else(|| McpError::invalid_params("Task not found"))?;
        serde_json::to_value(task).map_err(|error| McpError::internal_error(error.to_string()))
    }

    pub(crate) fn observation_binding(&self, id: &FinalTaskId) -> McpResult<Option<Value>> {
        self.records()?
            .get(id)
            .map(|record| record.observation_binding.clone())
            .ok_or_else(|| McpError::invalid_params("Task not found"))
    }

    /// The laboratory can prove its bounded drain synchronously. Retain both
    /// ordered phases before admitting transport cancellation: the upstream
    /// store may finalize cancellation during lease recovery, so it must never
    /// hold accepted cancellation intent for unresolved game work.
    fn ensure_drained(&self, id: &FinalTaskId) -> McpResult<()> {
        let (work, progress, observation_binding) = {
            let records = self.records()?;
            let record = records.get(id).ok_or_else(|| {
                McpError::internal_error("original laboratory task work is unavailable")
            })?;
            (
                record.work.clone(),
                record.progress.clone(),
                record.observation_binding.clone(),
            )
        };
        if work.is_observer() {
            // Cancellation releases only this process-local observation. It is
            // independent of original goal truth, dispatch custody, Query or
            // effect grants, and cannot run the owning-plan drain below.
            crate::server::task_session::read_authority(work.session_id())
                .map_err(|error| McpError::invalid_params(error.to_string()))?;
            if progress["stage"] == "monitor_finalized" {
                return Ok(());
            }
            let mut finalized = json!({
                "stage":"monitor_finalized","monitor_kind":work.monitor_kind(),
                "session_id":work.session_id(),"plan_digest":work.plan_digest(),
                "cancellation_scope":"monitor_only","monitor_stopped":true,
                "effect_cancellation_requested":false,"effects_dispatched_by_task":false,
                "physical_work_changed":false,"physical_quiescence_certified":false,
                "original_receipts_changed":false,"dispatch_handles_restored":false,
                "last_observation":progress,
            });
            work.attach_selection(&mut finalized);
            if let Some(binding) = observation_binding {
                finalized["actual_plan_digests"] = binding["plan_digests"].clone();
                finalized["observation_binding"] = binding;
            }
            return self.record_progress(id, finalized);
        }
        if progress["stage"] == "finalized"
            && progress["finalization"]["finalize_certificate"].is_object()
        {
            return Ok(());
        }
        let committed = crate::server::task_session::cancellation_admission(
            work.session_id(),
            work.original_digest()?,
        )
        .map_err(|error| McpError::invalid_params(error.to_string()))?;
        if !committed && progress["stage"] == "queued" {
            let proof = json!({"plan_digest": work.plan_digest(), "effects_dispatched": false,
                "statement": "this task was cancelled before the original plan dispatched"});
            return self.record_progress(id, json!({"stage": "finalized",
                "request": {"stage": "cancel_requested", "effects_dispatched": false},
                "finalization": {"stage": "finalized", "drain_progress": {
                    "actions_total": 0, "remaining_nonterminal": 0, "drained": 0, "quiescent": true},
                    "finalize_certificate": {"digest": dfmcp_core::Digest32::of_bytes(proof.to_string().as_bytes()).to_hex(),
                    "statement": proof["statement"], "effects_dispatched": false}}}));
        }
        let requested =
            crate::server::task_session::drain(work.session_id(), work.original_digest()?, false)
                .map_err(|error| McpError::invalid_params(error.to_string()))?;
        self.record_progress(id, requested.clone())?;
        let finalized =
            crate::server::task_session::drain(work.session_id(), work.original_digest()?, true)
                .map_err(|error| McpError::invalid_params(error.to_string()))?;
        self.record_progress(
            id,
            json!({"stage": "finalized", "request": requested, "finalization": finalized}),
        )
    }

    /// Opaque task identifiers confer no authority. Callers recheck the
    /// session's current Observe grant before reading this bounded projection.
    pub(crate) fn session_page(&self, session_id: &str, offset: usize) -> McpResult<Value> {
        const PAGE_SIZE: usize = 16;
        let records = self.records()?;
        let mut rows = Vec::new();
        for (id, record) in records
            .iter()
            .filter(|(_, record)| record.work.session_id().eq_ignore_ascii_case(session_id))
        {
            let task = self
                .inner
                .get_task(id)?
                .ok_or_else(|| McpError::internal_error("retained laboratory task missing"))?;
            let active = record.supervisor_active
                || matches!(
                    task,
                    FinalTask::Working(_) | FinalTask::InputRequired { .. }
                );
            let mut row = json!({
                "task_id": id.as_str(), "plan_digest": record.work.plan_digest(),
                "monitor_kind":record.work.monitor_kind(),
                "cancellation_scope":record.work.cancellation_scope(),
                "status": task.base().status, "stage": record.progress["stage"],
                "get": {"method": "tasks/get", "params": {"taskId": id.as_str()}},
                "details": format!("df://session/{}/task-{}", record.work.session_id(), id.as_str()),
            });
            if record.execution_failure.is_some() {
                row["stage"] = json!("monitor_expired");
                row["execution_failure"] = json!({
                    "scope":"task_monitor_only", "original_work_preserved":true,
                    "physical_quiescence_certified":false, "blind_retry_allowed":false,
                });
            }
            record.work.attach_selection(&mut row);
            if let Some(binding) = &record.observation_binding {
                row["actual_plan_digests"] = binding["plan_digests"].clone();
            }
            rows.push((active, record.ordinal, row));
        }
        // Never displace active or draining work with terminal history. The
        // remainder is newest-first, with a stable creation ordinal.
        rows.sort_by_key(|(active, ordinal, _)| {
            (std::cmp::Reverse(*active), std::cmp::Reverse(*ordinal))
        });
        let total = rows.len();
        if offset > total {
            return Err(McpError::invalid_params(
                "task page offset exceeds retained task count",
            ));
        }
        let tasks = rows
            .into_iter()
            .skip(offset)
            .take(PAGE_SIZE)
            .map(|(_, _, row)| row)
            .collect::<Vec<_>>();
        let end = offset + tasks.len();
        Ok(json!({"tasks": tasks, "total": total, "offset": offset,
            "complete": offset == 0 && end == total,
            "active_coverage_complete": offset == 0,
            "next": (end < total).then(|| format!("df://session/{session_id}/tasks-{end}")),
            "ordering": "active_first_then_newest; each page reflects current task state",
        }))
    }

    pub(crate) fn task_detail(&self, session_id: &str, id: &FinalTaskId) -> McpResult<Value> {
        let records = self.records()?;
        let record = records
            .get(id)
            .filter(|record| record.work.session_id().eq_ignore_ascii_case(session_id))
            .ok_or_else(|| McpError::invalid_params("Task not found in this session"))?;
        let task = self
            .inner
            .get_task(id)?
            .ok_or_else(|| McpError::invalid_params("Task not found"))?;
        let mut detail = json!({"task_id": id.as_str(), "plan_digest": record.work.plan_digest(),
            "monitor_kind":record.work.monitor_kind(),"cancellation_scope":record.work.cancellation_scope(),
            "task": task, "progress": record.progress, "final_evidence": record.final_evidence,
            "scope": "laboratory_process_only"});
        if let Some(failure) = &record.execution_failure {
            detail["execution_failure"] = failure.clone();
            detail["progress_scope"] = json!("historical_retained_monitor_observations");
        }
        record.work.attach_selection(&mut detail);
        if let Some(binding) = &record.observation_binding {
            detail["observation_binding"] = binding.clone();
        }
        Ok(detail)
    }
}

fn same_drained_receipts(prior: &Value, current: &Value) -> bool {
    let (Some(prior), Some(current)) = (prior.as_array(), current.as_array()) else {
        return false;
    };
    !prior.is_empty()
        && prior.len() == current.len()
        && prior.iter().zip(current).all(|(old, new)| {
            old["fully_drained"] == true
                && new["fully_drained"] == true
                && [
                    "action_id",
                    "step",
                    "state",
                    "message",
                    "receipt_digest",
                    "observed_anchor",
                    "obligation",
                    "evidence",
                ]
                .iter()
                .all(|field| old.get(*field).is_some() && old.get(*field) == new.get(*field))
        })
}

impl FinalTaskStore for LabTaskStore {
    fn retention_administration_policy(&self) -> McpResult<TaskRetentionPolicy> {
        self.inner.retention_administration_policy()
    }

    fn reclaim_terminal_task_if_current(
        &self,
        cx: &Cx,
        task_id: &FinalTaskId,
        generation: u64,
        reason: TaskRetentionReason,
    ) -> McpResult<TaskRetentionReclamation> {
        // The installed administrator denies public reclamation. Keep this
        // trusted backend contract coherent without releasing a live tail.
        let mut records = self.records()?;
        if records
            .get(task_id)
            .is_some_and(|record| record.supervisor_active)
        {
            return Ok(TaskRetentionReclamation::NotCurrent);
        }
        let outcome = self
            .inner
            .reclaim_terminal_task_if_current(cx, task_id, generation, reason)?;
        if matches!(&outcome, TaskRetentionReclamation::Reclaimed(_)) {
            records.remove(task_id);
        }
        Ok(outcome)
    }

    fn task_retention_audit(
        &self,
        cx: &Cx,
        after: Option<u64>,
        limit: usize,
    ) -> McpResult<TaskRetentionAuditPage> {
        self.inner.task_retention_audit(cx, after, limit)
    }

    fn execution_limits(&self) -> McpResult<FinalTaskExecutionLimits> {
        self.inner.execution_limits()
    }

    fn task_execution_budget_if_current(
        &self,
        expected: &FinalTaskSnapshot,
    ) -> McpResult<Option<FinalTaskExecutionBudget>> {
        self.inner.task_execution_budget_if_current(expected)
    }

    fn expire_task_execution_if_current(
        &self,
        expected: &FinalTaskSnapshot,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        let id = &expected.task().base().task_id;
        let mut records = self.records()?;
        let Some(record) = records.get_mut(id) else {
            return Ok(None);
        };
        let committed = self.inner.expire_task_execution_if_current(expected)?;
        if let Some(snapshot) = &committed {
            // Framework expiry retires only the monitor. Its original work
            // and any already captured terminal proof keep their identities.
            let message = match snapshot.task() {
                FinalTask::Failed { error, .. } => error.message.as_str(),
                _ => "Task execution budget exhausted",
            };
            let mut failure = json!({
                "schema":"dfmcp.lab-task-execution-failure/1",
                "task_id":id.as_str(), "session_id":record.work.session_id(),
                "plan_digest":record.work.plan_digest(),
                "monitor_kind":record.work.monitor_kind(),
                "cancellation_scope":record.work.cancellation_scope(),
                "scope":"task_monitor_only", "status":"failed",
                "reason":{"code":"task_execution_budget_exhausted", "message":message},
                "proof_status":"unknown", "physical_quiescent":null,
                "physical_quiescence_certified":false,
                "effect_cancellation_requested":false, "physical_work_changed":false,
                "original_work_preserved":true, "original_receipts_changed":false,
                "blind_retry_allowed":false, "dispatch_started":record.dispatch_started,
                "last_observation":{
                    "scope":"historical_retained_monitor_progress",
                    "stage":record.progress["stage"],
                    "observed_anchor":record.progress["obligation"]["observed_anchor"],
                    "details":format!("df://session/{}/task-{}", record.work.session_id(), id.as_str()),
                },
                "final_evidence_preserved":true,
                "final_evidence_available_at_expiry":record.final_evidence.is_some(),
                "next_step":{
                    "tool":"fortress.observe",
                    "arguments":{"session_id":record.work.session_id()},
                    "note":"Inspect retained original work before separately authorized continuation or cancellation.",
                },
            });
            record.work.attach_selection(&mut failure);
            if let Some(binding) = &record.observation_binding {
                failure["actual_plan_digests"] = binding["plan_digests"].clone();
                failure["observation_binding"] = binding.clone();
            }
            record.execution_failure = Some(failure);
        }
        Ok(committed)
    }

    fn next_expired_execution_snapshot(&self) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.next_expired_execution_snapshot()
    }

    fn create_task(
        &self,
        _task: FinalTask,
        _notification: FinalTaskStatusNotification,
    ) -> McpResult<()> {
        Err(McpError::invalid_params(
            "laboratory tasks require original sealed plan work",
        ))
    }

    fn create_task_with_work(
        &self,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
        descriptor: FinalTaskWorkDescriptor,
    ) -> McpResult<FinalTaskSnapshot> {
        self.create_owned(task, notification, descriptor, None, None)
    }

    fn create_task_with_authenticated_work(
        &self,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
        descriptor: FinalTaskWorkDescriptor,
        principal: Sha256Digest,
    ) -> McpResult<FinalTaskSnapshot> {
        self.create_owned(task, notification, descriptor, Some(principal), None)
    }

    fn create_task_with_operation(
        &self,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
        descriptor: FinalTaskWorkDescriptor,
        principal: Option<Sha256Digest>,
        operation: FinalTaskOperation,
    ) -> McpResult<FinalTaskSnapshot> {
        self.create_owned(task, notification, descriptor, principal, Some(operation))
    }

    fn get_task_access_metadata(
        &self,
        task_id: &FinalTaskId,
    ) -> McpResult<Option<FinalTaskAccessMetadata>> {
        if !self.readable_task(task_id)? {
            return Ok(None);
        }
        self.inner.get_task_access_metadata(task_id)
    }

    fn get_task(&self, task_id: &FinalTaskId) -> McpResult<Option<FinalTask>> {
        if !self.readable_task(task_id)? {
            return Ok(None);
        }
        self.inner.get_task(task_id)
    }

    fn get_task_snapshot(&self, task_id: &FinalTaskId) -> McpResult<Option<FinalTaskSnapshot>> {
        if !self.readable_task(task_id)? {
            return Ok(None);
        }
        self.inner.get_task_snapshot(task_id)
    }

    fn replace_task(
        &self,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
    ) -> McpResult<()> {
        self.inner.replace_task(task, notification)
    }

    fn replace_task_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
    ) -> McpResult<bool> {
        self.inner
            .replace_task_if_current(expected, task, notification)
    }

    fn replace_task_and_append_input_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
        input_responses: FinalTaskInputResponses,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.replace_task_and_append_input_if_current(
            expected,
            task,
            notification,
            input_responses,
        )
    }

    fn replace_task_and_clear_input_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner
            .replace_task_and_clear_input_if_current(expected, task, notification)
    }

    fn replace_task_and_clear_input_for_handoff_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        owner_id: &str,
        dispatch_fence: u64,
        cancellation_required: bool,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        if cancellation_required {
            self.ensure_drained(&expected.task().base().task_id)?;
        }
        self.inner
            .replace_task_and_clear_input_for_handoff_if_current(
                expected,
                owner_id,
                dispatch_fence,
                cancellation_required,
                task,
                notification,
            )
    }

    fn update_working_status_for_handoff_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        owner_id: &str,
        dispatch_fence: u64,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.update_working_status_for_handoff_if_current(
            expected,
            owner_id,
            dispatch_fence,
            task,
            notification,
        )
    }

    fn get_handoff_access_metadata(
        &self,
        task_id: &FinalTaskId,
        claim_generation: u64,
        owner_id: &str,
        dispatch_fence: u64,
    ) -> McpResult<Option<FinalTaskAccessMetadata>> {
        if !self.readable_task(task_id)? {
            return Ok(None);
        }
        self.inner
            .get_handoff_access_metadata(task_id, claim_generation, owner_id, dispatch_fence)
    }

    fn take_input_handoff_for_owner_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        owner_id: &str,
    ) -> McpResult<Option<FinalTaskAcceptedInputClaim>> {
        // Forward the attested claim intact: reconstructing it would discard
        // the upstream execution-permit custody of this owner.
        self.inner
            .take_input_handoff_for_owner_if_current(expected, owner_id)
    }

    fn take_input_if_current(
        &self,
        expected: &FinalTaskSnapshot,
    ) -> McpResult<Option<FinalTaskInputResponses>> {
        self.inner.take_input_if_current(expected)
    }

    fn take_input_for_owner_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        owner_id: &str,
    ) -> McpResult<Option<FinalTaskInputResponses>> {
        self.inner
            .take_input_for_owner_if_current(expected, owner_id)
    }

    fn work_descriptor_if_current(
        &self,
        expected: &FinalTaskSnapshot,
    ) -> McpResult<Option<FinalTaskWorkDescriptor>> {
        self.inner.work_descriptor_if_current(expected)
    }

    fn next_initial_work_snapshot(&self) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.next_initial_work_snapshot()
    }

    fn next_initial_work_snapshot_after(
        &self,
        after_task_id: Option<&FinalTaskId>,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.next_initial_work_snapshot_after(after_task_id)
    }

    fn take_initial_work_handoff_for_owner_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        owner_id: &str,
    ) -> McpResult<Option<FinalTaskInitialWorkClaim>> {
        self.inner
            .take_initial_work_handoff_for_owner_if_current(expected, owner_id)
    }

    fn take_initial_work_if_current(
        &self,
        expected: &FinalTaskSnapshot,
    ) -> McpResult<Option<FinalTaskWorkDescriptor>> {
        self.inner.take_initial_work_if_current(expected)
    }

    fn take_initial_work_for_owner_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        owner_id: &str,
    ) -> McpResult<Option<FinalTaskWorkDescriptor>> {
        self.inner
            .take_initial_work_for_owner_if_current(expected, owner_id)
    }

    fn restore_initial_work_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        work_descriptor: FinalTaskWorkDescriptor,
    ) -> McpResult<bool> {
        self.inner
            .restore_initial_work_if_current(task_id, generation, work_descriptor)
    }

    fn restore_initial_work_for_owner_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        owner_id: &str,
        dispatch_fence: Option<u64>,
        work_descriptor: FinalTaskWorkDescriptor,
    ) -> McpResult<bool> {
        self.inner.restore_initial_work_for_owner_if_current(
            task_id,
            generation,
            owner_id,
            dispatch_fence,
            work_descriptor,
        )
    }

    fn next_accepted_input_snapshot(&self) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.next_accepted_input_snapshot()
    }

    fn next_accepted_input_snapshot_after(
        &self,
        after_task_id: Option<&FinalTaskId>,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        self.inner.next_accepted_input_snapshot_after(after_task_id)
    }

    fn restore_input_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        input_responses: FinalTaskInputResponses,
    ) -> McpResult<bool> {
        self.inner
            .restore_input_if_current(task_id, generation, input_responses)
    }

    fn restore_input_for_owner_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        owner_id: &str,
        dispatch_fence: Option<u64>,
        input_responses: FinalTaskInputResponses,
    ) -> McpResult<bool> {
        self.inner.restore_input_for_owner_if_current(
            task_id,
            generation,
            owner_id,
            dispatch_fence,
            input_responses,
        )
    }

    fn begin_handoff_dispatch_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
    ) -> McpResult<bool> {
        self.inner
            .begin_handoff_dispatch_if_current(task_id, generation)
    }

    fn begin_handoff_dispatch_for_owner_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        owner_id: &str,
    ) -> McpResult<Option<u64>> {
        self.inner
            .begin_handoff_dispatch_for_owner_if_current(task_id, generation, owner_id)
    }

    fn renew_handoff_dispatch_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        owner_id: &str,
        dispatch_fence: u64,
    ) -> McpResult<bool> {
        self.inner
            .renew_handoff_dispatch_if_current(task_id, generation, owner_id, dispatch_fence)
    }

    fn handoff_dispatch_lease_heartbeat_interval(&self) -> McpResult<StdDuration> {
        self.inner.handoff_dispatch_lease_heartbeat_interval()
    }

    fn finish_handoff_dispatch_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
    ) -> McpResult<bool> {
        self.inner
            .finish_handoff_dispatch_if_current(task_id, generation)
    }

    fn finish_handoff_dispatch_for_owner_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
        owner_id: &str,
        dispatch_fence: u64,
    ) -> McpResult<bool> {
        if self.inner.is_cancellation_requested(task_id)? {
            self.ensure_drained(task_id)?;
        }
        self.inner.finish_handoff_dispatch_for_owner_if_current(
            task_id,
            generation,
            owner_id,
            dispatch_fence,
        )
    }

    fn request_cancellation_and_clear_input_if_current(
        &self,
        expected: &FinalTaskSnapshot,
        cancelled_task: FinalTask,
        cancelled_notification: FinalTaskStatusNotification,
    ) -> McpResult<Option<FinalTaskSnapshot>> {
        let id = &expected.task().base().task_id;
        let Some(current) = self.inner.get_task_snapshot(id)? else {
            return Ok(None);
        };
        if current.generation() != expected.generation() {
            return Ok(None);
        }
        let work = self.original_work(id)?;
        if !work.is_observer() {
            crate::server::task_session::cancellation_admission(
                work.session_id(),
                work.original_digest()?,
            )
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        }
        self.ensure_drained(id)?;
        self.inner.request_cancellation_and_clear_input_if_current(
            expected,
            cancelled_task,
            cancelled_notification,
        )
    }

    fn request_cancellation(&self, task_id: &FinalTaskId) -> McpResult<()> {
        self.ensure_drained(task_id)?;
        self.inner.request_cancellation(task_id)
    }

    fn request_cancellation_if_current(&self, expected: &FinalTaskSnapshot) -> McpResult<bool> {
        let id = &expected.task().base().task_id;
        let Some(current) = self.inner.get_task_snapshot(id)? else {
            return Ok(false);
        };
        if current.generation() != expected.generation() {
            return Ok(false);
        }
        self.ensure_drained(id)?;
        self.inner.request_cancellation_if_current(expected)
    }

    fn is_cancellation_requested(&self, task_id: &FinalTaskId) -> McpResult<bool> {
        self.inner.is_cancellation_requested(task_id)
    }

    fn retention_clock_now(&self) -> Instant {
        self.inner.retention_clock_now()
    }

    fn task_retention_deadline_if_current(
        &self,
        task_id: &FinalTaskId,
        generation: u64,
    ) -> McpResult<Option<FinalTaskRetentionDeadline>> {
        self.inner
            .task_retention_deadline_if_current(task_id, generation)
    }
}

#[cfg(test)]
mod retained_action_tests {
    use super::*;

    fn receipt() -> Value {
        json!([{"action_id":"original", "step":0, "state":"Verified", "message":"proved",
            "receipt_digest":"digest", "observed_anchor":{"sequence":2}, "obligation":null,
            "evidence":[], "fully_drained":true, "work_state":{"state":"quiescent", "observed_anchor":{"sequence":3}}}])
    }

    #[test]
    fn retained_quiescence_requires_the_same_complete_immutable_receipts() {
        let prior = receipt();
        let mut current = prior.clone();
        current[0]["work_state"]["observed_anchor"]["sequence"] = json!(4);
        assert!(same_drained_receipts(&prior, &current));
        for field in [
            "action_id",
            "step",
            "state",
            "message",
            "receipt_digest",
            "observed_anchor",
            "obligation",
            "evidence",
        ] {
            let mut different = current.clone();
            different[0][field] = json!("changed");
            assert!(!same_drained_receipts(&prior, &different), "{field}");
        }
        current[0]["fully_drained"] = json!(false);
        assert!(!same_drained_receipts(&prior, &current));
        assert!(!same_drained_receipts(&json!([]), &json!([])));
        assert!(!same_drained_receipts(&prior, &Value::Null));
    }
}
