//! Modern MCP Tasks as an explicitly bounded laboratory execution projection.
//! The supervisor never advances game time. The original commit and every
//! cancellation still enter the session's normal capability gates.
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::task::{Poll, Waker};

use asupersync::Cx;
use fastmcp_rust::{
    ApplicationTaskSupervisor, AuthorizedTaskServiceRunner, Content, FinalTaskCallToolResult,
    FinalTaskError, FinalTaskRetentionAuthority, FinalTaskRuntime, FinalTaskRuntimeConfig,
    FinalTaskSupervisorFuture, FinalTaskSupervisorHandoff, FinalTaskWorkDescriptor,
    FinalToolOutcome, McpContext, McpError, McpResult, Tool, ToolHandler,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::task_store::LabTaskStore;
use crate::tasks::McpTaskStatus;

const WORK_SCHEMA: &str = "dfmcp.lab-plan-task-work/1";

fn unresolved_drain_progress() -> Value {
    json!({
        "actions_total": null, "remaining_nonterminal": null,
        "remaining_work": null, "unknown_work": null, "remaining_actions": null,
        "drained": null, "physical_quiescent": false, "quiescent": false,
    })
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskWork {
    schema: String,
    pub(crate) session_id: String,
    pub(crate) plan_digest: String,
}

impl TaskWork {
    pub(crate) fn parse(value: &Value) -> McpResult<Self> {
        let mut work: Self = serde_json::from_value(value.clone())
            .map_err(|_| McpError::invalid_params("invalid laboratory task work descriptor"))?;
        if work.schema != WORK_SCHEMA
            || work.session_id.len() != 32
            || work.plan_digest.len() != 64
            || !work
                .session_id
                .bytes()
                .chain(work.plan_digest.bytes())
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(McpError::invalid_params(
                "laboratory task work must bind one exact session and sealed plan digest",
            ));
        }
        work.session_id.make_ascii_lowercase();
        work.plan_digest.make_ascii_lowercase();
        Ok(work)
    }
}

#[derive(Default)]
struct ProgressSignal {
    waker: Mutex<Option<Waker>>,
    generation: AtomicU64,
}

impl ProgressSignal {
    fn register(&self, waker: &Waker) {
        if let Ok(mut current) = self.waker.lock() {
            *current = Some(waker.clone());
        }
    }
    fn wake(&self) {
        self.generation.fetch_add(1, Ordering::Release);
        let waker = self.waker.lock().ok().and_then(|current| current.clone());
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

pub(crate) struct LabTaskService {
    pub(crate) runtime: FinalTaskRuntime,
    store: Arc<LabTaskStore>,
    signal: Arc<ProgressSignal>,
    service_error: Mutex<Option<String>>,
}

static SERVICE: OnceLock<Arc<LabTaskService>> = OnceLock::new();

struct BoundedRetention;
impl FinalTaskRetentionAuthority for BoundedRetention {
    fn authorize_unlimited_retention(&self) -> McpResult<()> {
        Ok(())
    }
}

impl LabTaskService {
    pub(crate) fn new() -> McpResult<(Arc<Self>, AuthorizedTaskServiceRunner)> {
        let store = LabTaskStore::new()?;
        let config = FinalTaskRuntimeConfig::with_unlimited_ttl(&BoundedRetention, Some(100))?;
        let runtime = FinalTaskRuntime::new(store.clone(), config, Arc::new(|_| {}));
        let service = Arc::new(Self {
            runtime,
            store,
            signal: Arc::new(ProgressSignal::default()),
            service_error: Mutex::new(None),
        });
        let runner = service.runtime.install_task_service(8, service.clone())?;
        Ok((service, runner))
    }

    pub(crate) fn install_global(service: Arc<Self>) -> McpResult<()> {
        SERVICE
            .set(service)
            .map_err(|_| McpError::internal_error("laboratory task service already installed"))
    }

    fn bounded_terminal_evidence(
        &self,
        id: &fastmcp_rust::FinalTaskId,
        work: &TaskWork,
        payload: &Value,
        failure: Option<&str>,
    ) -> McpResult<Value> {
        let budget = crate::server::task_session::read_budget(&work.session_id)
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        self.store.retain_final_evidence(id, payload, failure)?;
        let max_bytes = budget.max_bytes.min(
            u64::from(budget.max_output_tokens)
                .saturating_mul(crate::output_budget::BYTES_PER_TOKEN as u64),
        );
        let fits = |evidence: &Value| -> McpResult<bool> {
            let mut wire = self.store.wire_task(id)?;
            if let Some(message) = failure {
                wire["status"] = json!("failed");
                wire["error"] = serde_json::to_value(task_error(message, evidence.clone())?)
                    .map_err(|error| McpError::internal_error(error.to_string()))?;
            } else {
                wire["status"] = json!("completed");
                wire["result"] = serde_json::to_value(task_result(evidence)?)
                    .map_err(|error| McpError::internal_error(error.to_string()))?;
            }
            // Include the actual modern task/result envelope, plus bounded
            // room for its final status message and JSON-RPC framing.
            Ok(wire.to_string().len().saturating_add(512) as u64 <= max_bytes)
        };
        if fits(payload)? {
            return Ok(payload.clone());
        }
        let full = payload.to_string();
        let mut counts = std::collections::BTreeMap::<String, usize>::new();
        for action in payload["actions"].as_array().into_iter().flatten() {
            *counts
                .entry(
                    action["state"]
                        .as_str()
                        .map_or("unknown", |state| state)
                        .to_owned(),
                )
                .or_default() += 1;
        }
        let anchor = payload
            .get("observed_anchor")
            .cloned()
            .map_or_else(|| payload["agent_turn"]["anchor"].clone(), |anchor| anchor);
        let mut active = crate::empty_active_work();
        if failure.is_some() {
            active["mcp_tasks"] = json!([{"task_id": id.as_str(), "status": "failed",
                "proof_status": payload["proof_status"],
                "remaining_work": payload["remaining_work"],
                "physical_quiescent": payload["physical_quiescent"].as_bool().unwrap_or(false),
                "blind_retry_allowed": false, "details": format!("df://session/{}/task-{}", work.session_id, id.as_str())}]);
        }
        if payload["physical_quiescent"].as_bool() != Some(true) {
            active["indeterminate_effects"] = json!([{
                "plan_digest": work.plan_digest, "state": "unknown", "quiescent": false,
                "reason": "the retained terminal evidence does not prove physical quiescence",
            }]);
        }
        let turn = crate::AgentTurnBuilder::new("fortress.commit", crate::AgentPhase::Verify)
            .session_id(work.session_id.clone())
            .anchor(anchor.clone())
            .active_work(active)
            .build();
        let compact = json!({"ok": failure.is_none(), "schema": "dfmcp.lab-task-summary/1",
            "session_id": work.session_id, "plan_digest": work.plan_digest,
            "status": if failure.is_some() { "failed" } else { "completed" },
            "proof_status": payload.get("proof_status").cloned().unwrap_or_else(|| json!("unknown")),
            "remaining_work": payload["remaining_work"],
            "physical_quiescent": payload["physical_quiescent"].as_bool().unwrap_or(false),
            "cleanup_required": payload.get("cleanup_required").cloned().unwrap_or(Value::Bool(true)),
            "drain_progress": payload.get("drain_progress").cloned().unwrap_or_else(unresolved_drain_progress),
            "physical_work": payload["physical_work"],
            "observed_anchor": anchor, "action_counts": counts,
            "indeterminate": payload["indeterminate"], "recovery_class": payload["recovery_class"],
            "blind_retry_allowed": false, "agent_turn": turn, "scope": "laboratory_process_only",
            "evidence": {"coverage": "summary_with_complete_evidence_retained",
                "digest": dfmcp_core::Digest32::of_bytes(full.as_bytes()).to_hex(), "bytes": full.len(),
                "details": format!("df://session/{}/task-{}", work.session_id, id.as_str()),
                "next": format!("df://session/{}/task-{}~evidence-0", work.session_id, id.as_str())},
            "output_budget": {"max_output_tokens": budget.max_output_tokens, "max_bytes": max_bytes,
                "clipped": true, "omitted": "full per-action proof is retained in digest-checked evidence pages"}});
        if !fits(&compact)? {
            return Err(McpError::invalid_params(
                "bounded task summary exceeds negotiated output budget; full evidence remains retained and the task must not be retried",
            ));
        }
        Ok(compact)
    }
}

/// A foreground semantic tool changed the known engine state. Wake only the
/// owned supervisor; this is neither a timer nor detached game work.
pub(crate) fn notify_progress() {
    if let Some(service) = SERVICE.get() {
        service.signal.wake();
    }
}

pub(crate) fn session_tasks(session_id: &str, offset: usize) -> Value {
    match SERVICE
        .get()
        .map(|service| service.store.session_page(session_id, offset))
    {
        Some(Ok(mut page)) => {
            if let Some(service) = SERVICE.get() {
                page["service_error"] = json!(
                    service
                        .service_error
                        .lock()
                        .ok()
                        .and_then(|error| error.clone())
                );
            }
            page
        }
        Some(Err(error)) => json!({"unavailable": error.to_string()}),
        None => json!({"tasks": [], "total": 0, "complete": true, "next": null}),
    }
}

pub(crate) fn session_handles(session_id: &str) -> Value {
    let page = session_tasks(session_id, 0);
    if page.get("unavailable").is_some() {
        return page;
    }
    Value::Array(
        page["tasks"]
            .as_array()
            .into_iter()
            .flatten()
            .take(4)
            .cloned()
            .collect(),
    )
}

pub(crate) fn session_handle_coverage(session_id: &str) -> Value {
    json!({"max_handles_returned": 4, "ordering": "active_first_then_newest",
        "history": "paged", "discovery": format!("df://session/{session_id}/tasks"),
        "note": "only mcp_tasks identifiers are valid for modern tasks/get; output_budget reports any further clipping"})
}

pub(crate) fn task_detail(session_id: &str, task_id: &str) -> McpResult<Value> {
    let id = fastmcp_rust::FinalTaskId::parse(task_id)
        .map_err(|error| McpError::invalid_params(error.to_string()))?;
    match SERVICE.get() {
        Some(service) => {
            let mut detail = service.store.task_detail(session_id, &id)?;
            detail["service_error"] = json!(
                service
                    .service_error
                    .lock()
                    .ok()
                    .and_then(|error| error.clone())
            );
            Ok(detail)
        }
        None => Err(McpError::invalid_params(
            "laboratory task service is not installed",
        )),
    }
}

fn task_error(message: &str, payload: Value) -> McpResult<FinalTaskError> {
    serde_json::from_value(json!({"code": -32000, "message": message, "data": payload}))
        .map_err(|_| McpError::internal_error("could not encode laboratory task evidence"))
}

fn task_result(payload: &Value) -> McpResult<FinalTaskCallToolResult> {
    serde_json::from_value(
        json!({"content": [{"type": "text", "text": "Laboratory plan task finished. Read structuredContent for the verified outcome and retained evidence."}],
        "structuredContent": payload}),
    )
    .map_err(|_| McpError::internal_error("could not encode laboratory task result"))
}

impl ApplicationTaskSupervisor for LabTaskService {
    fn resume<'a>(
        &'a self,
        cx: &'a Cx,
        handoff: FinalTaskSupervisorHandoff,
    ) -> FinalTaskSupervisorFuture<'a> {
        Box::pin(async move {
            let FinalTaskSupervisorHandoff::Initial(work) = handoff else {
                return Err(McpError::invalid_params(
                    "laboratory plan tasks do not request client input",
                ));
            };
            let definition = TaskWork::parse(work.work_descriptor().as_value())?;
            cx.checkpoint()
                .map_err(|error| McpError::internal_error(error.to_string()))?;
            if work.is_cancellation_requested()? {
                work.honor_cancellation(Some(
                    "the original plan has a retained cancellation certificate".to_owned(),
                ))?;
                return Ok(());
            }
            // A fence/output failure can occur after terminal proof was
            // captured. Recover that exact proof and outcome, including its
            // original anchor, before attempting any engine work again.
            if let Some((retained, failure)) = self.store.terminal_evidence(work.task_id())? {
                let bounded = self.bounded_terminal_evidence(
                    work.task_id(),
                    &definition,
                    &retained,
                    failure.as_deref(),
                )?;
                if let Some(failure) = failure {
                    work.fail_task(
                        task_error(&failure, bounded)?,
                        Some(
                            "original terminal evidence retained during task-service recovery"
                                .to_owned(),
                        ),
                    )?;
                } else {
                    work.complete_task(
                        task_result(&bounded)?,
                        Some(
                            "original verified evidence retained during task-service recovery"
                                .to_owned(),
                        ),
                    )?;
                }
                return Ok(());
            }
            let recovering = self.store.mark_dispatch_started(work.task_id())?;
            let commit = if recovering {
                // Receipt loss after restore must never reopen the original
                // mutation. A recovered monitor observes retained actions;
                // it does not invoke commit again.
                Value::Null
            } else {
                self.store
                    .record_progress(work.task_id(), json!({"stage": "dispatch_started"}))?;
                let commit = crate::agent_facade::fortress_commit(
                    Some(definition.session_id.clone()),
                    definition.plan_digest.clone(),
                );
                let commit: Value = serde_json::from_str(&commit).map_err(|_| {
                    McpError::internal_error("laboratory commit response is invalid")
                })?;
                self.store.record_progress(
                    work.task_id(),
                    json!({"stage": "commit_returned", "commit_response": commit}),
                )?;
                commit
            };
            // Output shaping cannot decide whether an effect happened. The
            // original receipt is authoritative even if the facade needed to
            // replace a large success response with a budget refusal.
            if !crate::server::task_session::has_original_receipt(
                &definition.session_id,
                &definition.plan_digest,
            )
            .map_err(|error| McpError::invalid_params(error.to_string()))?
            {
                let failure = if recovering {
                    "the original receipt is unavailable after recovery; past effects are unknown and must not be retried"
                } else {
                    "the original plan was not dispatched; inspect refusal or required shared-world consent"
                };
                let payload = json!({"ok": false, "session_id": definition.session_id,
                    "plan_digest": definition.plan_digest, "status": "failed",
                    "proof_status": if recovering { "unknown" } else { "failed" },
                    "original_plan_dispatched": if recovering { Value::Null } else { json!(false) },
                    "remaining_work": if recovering { Value::Null } else { json!(0) },
                    "physical_quiescent": !recovering, "cleanup_required": recovering,
                    "physical_work": {
                        "state": if recovering { "unknown" } else { "never_dispatched" },
                        "quiescent": !recovering, "reason": failure,
                    },
                    "drain_progress": if recovering { unresolved_drain_progress() } else { json!({
                        "actions_total": 0, "remaining_nonterminal": 0, "remaining_work": 0,
                        "unknown_work": 0, "remaining_actions": 0, "drained": 0,
                        "physical_quiescent": true, "quiescent": true,
                    }) },
                    "indeterminate": recovering, "blind_retry_allowed": false,
                    "recovery_class": if recovering { "reconciliation_required" } else { "operator_action_required" },
                    "next_step": {"tool": "fortress.observe", "arguments": {"session_id": definition.session_id},
                        "note": "inspect current state and the retained refusal or shared-world consent before choosing a new plan"},
                    "commit_response": commit});
                let bounded = self.bounded_terminal_evidence(
                    work.task_id(),
                    &definition,
                    &payload,
                    Some(failure),
                )?;
                work.fail_task(
                    task_error(failure, bounded)?,
                    Some(
                        "commit refused; inspect the retained Agent Turn before replanning"
                            .to_owned(),
                    ),
                )?;
                return Ok(());
            }
            std::future::poll_fn(|task_context| {
                self.signal.register(task_context.waker());
                let result = (|| -> McpResult<Option<()>> {
                    if work.is_cancellation_requested()? {
                        // Admission already retained both ordered drain
                        // phases and proved quiescence before the upstream
                        // store accepted cancellation intent. Lease recovery
                        // therefore cannot erase unresolved engine work.
                        work.honor_cancellation(Some("original plan drained before transport cancellation was admitted; both drain phases and the finalize certificate are retained".to_owned()))?;
                        return Ok(Some(()));
                    }
                    let view = match crate::server::task_session::view(&definition.session_id, &definition.plan_digest) {
                        Ok(view) => view,
                        Err(error) => {
                            let payload = json!({"ok": false, "session_id": definition.session_id,
                                "plan_digest": definition.plan_digest, "error": error.to_string(),
                                "status": "failed", "proof_status": "unknown", "indeterminate": true,
                                "remaining_work": null, "physical_quiescent": false, "cleanup_required": true,
                                "drain_progress": unresolved_drain_progress(),
                                "physical_work": {"state": "unknown", "quiescent": false,
                                    "reason": "the original plan's retained action evidence cannot be observed"},
                                "blind_retry_allowed": false, "recovery_class": "reconciliation_required"});
                            self.store.record_progress(work.task_id(), json!({"stage": "unresolved", "evidence": payload}))?;
                            let failure = "original plan can no longer be observed; do not retry the effect";
                            let bounded = self.bounded_terminal_evidence(work.task_id(), &definition, &payload, Some(failure))?;
                            work.fail_task(task_error(failure, bounded)?, None)?;
                            return Ok(Some(()));
                        }
                    };
                    self.store.record_progress(work.task_id(), json!({"stage": "observed", "obligation": view.payload}))?;
                    match view.status {
                        McpTaskStatus::Working | McpTaskStatus::InputRequired => Ok(None),
                        McpTaskStatus::Completed => {
                            let bounded = self.bounded_terminal_evidence(work.task_id(), &definition, &view.payload, None)?;
                            work.complete_task(task_result(&bounded)?, Some("every original plan action has verified evidence and quiescent physical work".to_owned()))?;
                            Ok(Some(()))
                        }
                        McpTaskStatus::Failed => {
                            let failure = "the original plan failed verification or needs reconciliation";
                            let bounded = self.bounded_terminal_evidence(work.task_id(), &definition, &view.payload, Some(failure))?;
                            work.fail_task(task_error(failure, bounded)?, Some("failure evidence retained; blind retry is forbidden".to_owned()))?;
                            Ok(Some(()))
                        }
                        McpTaskStatus::Cancelled => {
                            // A foreground fortress.cancel may have drained
                            // the plan before the transport cancellation.
                            self.runtime.cancel_task(work.task_id())?;
                            work.honor_cancellation(Some("original plan was already drained by fortress.cancel".to_owned()))?;
                            Ok(Some(()))
                        }
                    }
                })();
                match result {
                    Ok(None) => Poll::Pending,
                    Ok(Some(())) => Poll::Ready(Ok(())),
                    Err(error) => {
                        let _ = self.store.record_progress(work.task_id(), json!({
                            "stage": "reconciliation_required", "error": error.to_string(),
                            "remaining_work": null, "physical_quiescent": false,
                            "drain_progress": unresolved_drain_progress(),
                            "physical_work": {"state": "unknown", "quiescent": false},
                            "blind_retry_allowed": false,
                            "note": "the engine has not proved quiescence; the task service retains its original work",
                        }));
                        Poll::Ready(Err(error))
                    }
                }
            }).await
        })
    }
}

/// Only the stdio registration changes. Existing direct Rust callers and
/// deterministic replay keep the original two-argument fortress_commit.
pub(crate) struct TaskAwareCommit;

impl ToolHandler for TaskAwareCommit {
    fn definition(&self) -> Tool {
        let mut definition = crate::agent_facade::FortressCommit.definition();
        if let Some(properties) = definition
            .input_schema
            .get_mut("properties")
            .and_then(Value::as_object_mut)
        {
            properties.insert("as_task".to_owned(), json!({"type": "boolean", "description": "Retain and supervise this original sealed plan as a modern MCP Task; laboratory only, one active monitor per process, game time advances only through fortress_wait."}));
        }
        definition
    }

    fn declares_final_tasks(&self) -> bool {
        true
    }

    fn call(&self, ctx: &McpContext, mut arguments: Value) -> McpResult<Vec<Content>> {
        if arguments
            .get("as_task")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(McpError::invalid_params("as_task must be a boolean"));
        }
        if arguments.get("as_task") == Some(&Value::Bool(true)) {
            return Err(McpError::invalid_params(
                "as_task requires negotiated modern Tasks dispatch",
            ));
        }
        if let Some(arguments) = arguments.as_object_mut() {
            arguments.remove("as_task");
        }
        crate::agent_facade::FortressCommit.call(ctx, arguments)
    }

    fn call_final_outcome(
        &self,
        ctx: &McpContext,
        mut arguments: Value,
    ) -> McpResult<FinalToolOutcome> {
        if let Some(value) = arguments.get("as_task")
            && !value.is_boolean()
        {
            return Err(McpError::invalid_params("as_task must be a boolean"));
        }
        if arguments.get("as_task") != Some(&Value::Bool(true)) {
            if let Some(arguments) = arguments.as_object_mut() {
                arguments.remove("as_task");
            }
            return crate::agent_facade::FortressCommit.call_final_outcome(ctx, arguments);
        }
        ctx.checkpoint()?;
        let session_id = arguments
            .get("session_id")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("as_task requires an explicit session_id"))?;
        let plan_digest = arguments
            .get("plan_digest")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("as_task requires the exact plan_digest"))?;
        let work = TaskWork::parse(
            &json!({"schema": WORK_SCHEMA, "session_id": session_id, "plan_digest": plan_digest}),
        )?;
        crate::server::task_session::validate(&work.session_id, &work.plan_digest)
            .map_err(|error| McpError::invalid_params(error.to_string()))?;
        Ok(FinalToolOutcome::CreateTask { work_descriptor: FinalTaskWorkDescriptor::new(serde_json::to_value(work).map_err(|_| McpError::internal_error("could not encode original plan work"))?)?,
            status_message: Some("laboratory sealed plan queued; commit authority will be revalidated before dispatch".to_owned()) })
    }
}

/// Both futures live in the caller-owned root context. Poll Tasks first so
/// readiness exists before ingress; neither future escapes its owner.
pub(crate) async fn serve_with_tasks(
    server: fastmcp_rust::modern::Server,
    mut runner: AuthorizedTaskServiceRunner,
    owner: Arc<LabTaskService>,
    cx: &Cx,
) -> McpResult<()> {
    let mut serving = std::pin::pin!(server.run_stdio_with_cx(cx));
    loop {
        let result = {
            let mut service = std::pin::pin!(runner.run_service(cx));
            std::future::poll_fn(|context| {
                if let Poll::Ready(result) = service.as_mut().poll(context) {
                    return Poll::Ready(result);
                }
                match serving.as_mut().poll(context) {
                    Poll::Ready(never) => never,
                    Poll::Pending => Poll::Pending,
                }
            })
            .await
        };
        match result {
            Ok(()) => return Ok(()),
            Err(error) => {
                if let Ok(mut fault) = owner.service_error.lock() {
                    *fault = Some(error.to_string());
                }
                // A failed drain keeps its retained handoff. Keep diagnostics
                // and ordinary tools available, then re-enter recovery only
                // after new foreground evidence; never spin blind retries.
                let generation = owner.signal.generation.load(Ordering::Acquire);
                std::future::poll_fn(|context| {
                    owner.signal.register(context.waker());
                    if owner.signal.generation.load(Ordering::Acquire) != generation {
                        return Poll::Ready(());
                    }
                    match serving.as_mut().poll(context) {
                        Poll::Ready(never) => never,
                        Poll::Pending => Poll::Pending,
                    }
                })
                .await;
                if let Ok(mut fault) = owner.service_error.lock() {
                    *fault = None;
                }
            }
        }
    }
}
