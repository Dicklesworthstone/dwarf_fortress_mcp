//! Bounded process-local Tasks custody for the laboratory only.
//!
//! The pinned upstream store owns every Tasks state transition and dispatch
//! fence. This wrapper adds the application's one-active-monitor admission
//! bound and session-bound discovery; it does not redefine the wire protocol.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration as StdDuration;

use fastmcp_core::crypto::Sha256Digest;
use fastmcp_rust::{
    FinalTask, FinalTaskId, FinalTaskInputResponses, FinalTaskSnapshot,
    FinalTaskStatusNotification, FinalTaskStore, FinalTaskWorkDescriptor, InMemoryFinalTaskStore,
    McpError, McpResult,
};
use serde_json::{Value, json};

use crate::task_service::TaskWork;

/// Terminal task history is retained until process exit, with an explicit
/// hard bound. Task TTL therefore cannot erase an unfinished game obligation.
pub(crate) const MAX_RETAINED_TASKS: usize = 256;

#[derive(Clone)]
struct TaskRecord {
    work: TaskWork,
    progress: Value,
    ordinal: usize,
    final_evidence: Option<Value>,
    terminal_failure: Option<String>,
    dispatch_started: bool,
}

pub(crate) struct LabTaskStore {
    inner: InMemoryFinalTaskStore,
    records: Mutex<BTreeMap<FinalTaskId, TaskRecord>>,
}

impl LabTaskStore {
    pub(crate) fn new() -> McpResult<Arc<Self>> {
        Ok(Arc::new(Self {
            inner: InMemoryFinalTaskStore::new(MAX_RETAINED_TASKS)?,
            records: Mutex::new(BTreeMap::new()),
        }))
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
    ) -> McpResult<()> {
        let work = TaskWork::parse(descriptor.as_value())?;
        // Admission and the upstream create share this lock. A concurrent
        // task call cannot slip past the single-monitor bound.
        let mut records = self.records()?;
        if records.len() >= MAX_RETAINED_TASKS {
            return Err(McpError::invalid_params(
                "laboratory retained task capacity reached; no plan was dispatched",
            ));
        }
        for id in records.keys() {
            if self.inner.get_task(id)?.is_some_and(|task| {
                matches!(
                    task,
                    FinalTask::Working(_) | FinalTask::InputRequired { .. }
                )
            }) {
                return Err(McpError::invalid_params(
                    "one laboratory task monitor is already active; finish or cancel it before creating another task; no plan was dispatched",
                ));
            }
        }
        let id = task.base().task_id.clone();
        let ordinal = records.len();
        match principal {
            Some(principal) => self.inner.create_task_with_authenticated_work(
                task,
                notification,
                descriptor,
                principal,
            )?,
            None => self
                .inner
                .create_task_with_work(task, notification, descriptor)?,
        }
        records.insert(
            id,
            TaskRecord {
                work,
                progress: json!({"stage": "queued", "effects_dispatched": false}),
                ordinal,
                final_evidence: None,
                terminal_failure: None,
                dispatch_started: false,
            },
        );
        Ok(())
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
        if payload["physical_quiescent"] != true {
            return Ok(());
        }
        let records = self.records()?;
        let prior = records
            .values()
            .filter(|record| {
                record.work.session_id == work.session_id
                    && record.work.plan_digest == work.plan_digest
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

    /// The laboratory can prove its bounded drain synchronously. Retain both
    /// ordered phases before admitting transport cancellation: the upstream
    /// store may finalize cancellation during lease recovery, so it must never
    /// hold accepted cancellation intent for unresolved game work.
    fn ensure_drained(&self, id: &FinalTaskId) -> McpResult<()> {
        let (work, progress) = {
            let records = self.records()?;
            let record = records.get(id).ok_or_else(|| {
                McpError::internal_error("original laboratory task work is unavailable")
            })?;
            (record.work.clone(), record.progress.clone())
        };
        if progress["stage"] == "finalized"
            && progress["finalization"]["finalize_certificate"].is_object()
        {
            return Ok(());
        }
        let committed = crate::server::task_session::cancellation_admission(
            &work.session_id,
            &work.plan_digest,
        )
        .map_err(|error| McpError::invalid_params(error.to_string()))?;
        if !committed && progress["stage"] == "queued" {
            let proof = json!({"plan_digest": work.plan_digest, "effects_dispatched": false,
                "statement": "this task was cancelled before the original plan dispatched"});
            return self.record_progress(id, json!({"stage": "finalized",
                "request": {"stage": "cancel_requested", "effects_dispatched": false},
                "finalization": {"stage": "finalized", "drain_progress": {
                    "actions_total": 0, "remaining_nonterminal": 0, "drained": 0, "quiescent": true},
                    "finalize_certificate": {"digest": dfmcp_core::Digest32::of_bytes(proof.to_string().as_bytes()).to_hex(),
                    "statement": proof["statement"], "effects_dispatched": false}}}));
        }
        let requested =
            crate::server::task_session::drain(&work.session_id, &work.plan_digest, false)
                .map_err(|error| McpError::invalid_params(error.to_string()))?;
        self.record_progress(id, requested.clone())?;
        let finalized =
            crate::server::task_session::drain(&work.session_id, &work.plan_digest, true)
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
            .filter(|(_, record)| record.work.session_id.eq_ignore_ascii_case(session_id))
        {
            let task = self
                .inner
                .get_task(id)?
                .ok_or_else(|| McpError::internal_error("retained laboratory task missing"))?;
            let active = matches!(
                task,
                FinalTask::Working(_) | FinalTask::InputRequired { .. }
            );
            rows.push((active, record.ordinal, json!({
                "task_id": id.as_str(), "plan_digest": record.work.plan_digest,
                "status": task.base().status, "stage": record.progress["stage"],
                "get": {"method": "tasks/get", "params": {"taskId": id.as_str()}},
                "details": format!("df://session/{}/task-{}", record.work.session_id, id.as_str()),
            })));
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
            .filter(|record| record.work.session_id.eq_ignore_ascii_case(session_id))
            .ok_or_else(|| McpError::invalid_params("Task not found in this session"))?;
        let task = self
            .inner
            .get_task(id)?
            .ok_or_else(|| McpError::invalid_params("Task not found"))?;
        Ok(
            json!({"task_id": id.as_str(), "plan_digest": record.work.plan_digest,
            "task": task, "progress": record.progress, "final_evidence": record.final_evidence,
            "scope": "laboratory_process_only"}),
        )
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
    ) -> McpResult<()> {
        self.create_owned(task, notification, descriptor, None)
    }

    fn create_task_with_authenticated_work(
        &self,
        task: FinalTask,
        notification: FinalTaskStatusNotification,
        descriptor: FinalTaskWorkDescriptor,
        principal: Sha256Digest,
    ) -> McpResult<()> {
        self.create_owned(task, notification, descriptor, Some(principal))
    }

    fn get_task(&self, task_id: &FinalTaskId) -> McpResult<Option<FinalTask>> {
        if !self.records()?.contains_key(task_id) {
            return Ok(None);
        }
        let work = self.original_work(task_id)?;
        crate::server::task_session::read_authority(&work.session_id)
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        self.inner.get_task(task_id)
    }

    fn get_task_snapshot(&self, task_id: &FinalTaskId) -> McpResult<Option<FinalTaskSnapshot>> {
        if !self.records()?.contains_key(task_id) {
            return Ok(None);
        }
        let work = self.original_work(task_id)?;
        crate::server::task_session::read_authority(&work.session_id)
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
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
    ) -> McpResult<bool> {
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
    ) -> McpResult<bool> {
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
    ) -> McpResult<bool> {
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
        crate::server::task_session::cancellation_admission(&work.session_id, &work.plan_digest)
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
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
