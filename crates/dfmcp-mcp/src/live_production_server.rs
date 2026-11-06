#![forbid(unsafe_code)]
//! Unadmitted combined labor and finite-order MCP host.
use crate::live_production_runtime::{Config, Mode, ObserveRequest, Request, Runtime, View};
use crate::semantic_workforce_runtime::digest;
use dfmcp_core::{
    DfmcpError, Digest32, ErrorCode, OperationContext, RequestId, Result, SessionId, WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::{
    Mutex, MutexGuard, TryLockError,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

#[path = "live_production_presentation.rs"]
mod presentation;
use presentation::{RESPONSE_RESERVE, packet};

const FAMILY: u128 = 24u128 << 57;
const MAX_BYTES: u64 = 32 * 1024 * 1024 * 1024;
const DEFAULT_BYTES: u64 = 8 * 1024 * 1024 * 1024;
const OPEN_INSPECTION_BYTES: u64 = 512 * 1024 * 1024;
const MAX_OUTPUT: u32 = 32_768;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State>> = Mutex::new(None);

struct State {
    runtime: Runtime,
    budget: WorkBudget,
    retained: Option<Value>,
    uncertain: bool,
}
impl State {
    /// Preserve the newest in-memory custody pin without another I/O operation.
    /// A discarded local review is never presented as still confirmable.
    fn withdraw_and_retain(&mut self) {
        self.runtime.abandon_preparation();
        let latest = self.runtime.opening_recovery_reference();
        merge_recovery(&mut self.retained, &latest);
    }
}
fn merge_recovery(old: &mut Option<Value>, latest: &Value) {
    let retained = old.get_or_insert_with(|| json!({}));
    if !latest["recovery"].is_null() {
        retained["recovery"] = latest["recovery"].clone();
    }
    if let Some(floor) = latest["observation_tick_floor"].as_u64() {
        retained["observation_tick_floor"] =
            json!(floor.max(retained["observation_tick_floor"].as_u64().unwrap_or(0)));
    }
    retained["review"] = Value::Null;
    retained["selected_inventory_goal"] = latest["selected_inventory_goal"].clone();
    if retained["frontier"].is_object() {
        retained["frontier"]["review"] = Value::Null;
    }
}
enum Action {
    Observe(ObserveRequest),
    Inspect,
    Plan(Request),
    Prepare(Digest32),
    Commit(Digest32, bool),
    Wait,
    Cancel,
    Unavailable,
}
impl Action {
    fn may_dispatch(&self) -> bool {
        matches!(self, Self::Commit(..))
    }
    fn may_need_reconciliation(&self) -> bool {
        matches!(
            self,
            Self::Prepare(_) | Self::Commit(..) | Self::Wait | Self::Cancel
        )
    }
    fn perform(self, runtime: &mut Runtime, c: &OperationContext) -> Result<View> {
        match self {
            Self::Observe(request) => runtime.observe(request, c),
            Self::Inspect => runtime.inspect(c),
            Self::Plan(request) => runtime.plan(request, c),
            Self::Prepare(id) => runtime.prepare_next(id, c),
            Self::Commit(seal, confirmed) => runtime.commit(seal, confirmed, c),
            Self::Wait => runtime.wait(c),
            Self::Cancel => runtime.cancel(c),
            Self::Unavailable => Err(error(
                ErrorCode::CapabilityDenied,
                "combined development runtime cannot checkpoint or restore",
            )),
        }
    }
}
fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}
fn runtime_io() -> Result<()> {
    if crate::admission::current_admission_provenance().is_some() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "combined development refuses production admission",
        ));
    }
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "combined I/O requires its owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "combined request cancelled",
        )
    })?;
    if !cx.capabilities().io {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited context denies combined I/O",
        ));
    }
    Ok(())
}
fn session_lock() -> Result<MutexGuard<'static, Option<State>>> {
    match SESSION.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => {
            Err(error(ErrorCode::BudgetExceeded, "combined session is busy"))
        }
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "combined session poisoned; retain original recovery custody",
        )),
    }
}
fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid combined session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid combined session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 24
    {
        return Err(error(
            ErrorCode::SessionNotFound,
            "foreign combined session ID",
        ));
    }
    Ok(id)
}
fn budgets(
    wall: Option<u64>,
    bytes: Option<u64>,
    output: Option<u32>,
) -> Result<(WorkBudget, WorkBudget)> {
    let display = WorkBudget {
        max_wall_millis: wall.unwrap_or(5000),
        max_bytes: bytes.unwrap_or(DEFAULT_BYTES),
        max_entities: 8192,
        max_actions: 2,
        max_output_tokens: output.unwrap_or(MAX_OUTPUT),
        max_game_ticks: 0,
    };
    display.validate()?;
    if display.max_wall_millis > 60_000
        || display.max_bytes > MAX_BYTES
        || display.max_output_tokens > MAX_OUTPUT
        || u64::from(display.max_output_tokens) * 4 < RESPONSE_RESERVE
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "combined limits require at most 60000ms, 32GiB and a full response reservation",
        ));
    }
    let mut work = display;
    work.max_bytes = display
        .max_bytes
        .checked_sub(RESPONSE_RESERVE + OPEN_INSPECTION_BYTES)
        .filter(|n| *n >= OPEN_INSPECTION_BYTES)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "opening and complete response do not fit; no I/O started",
            )
        })?;
    Ok((display, work))
}
fn remaining(start: Instant, total: u64) -> Result<u64> {
    let elapsed = u64::try_from(start.elapsed().as_millis())
        .map_err(|_| error(ErrorCode::BudgetExceeded, "deadline overflow"))?;
    total
        .checked_sub(elapsed)
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "combined request deadline expired",
            )
        })
}
/// A completed packet is not published until its final cancellation/deadline
/// check succeeds. The rejection path withdraws any process-local review.
fn checked_publication(
    output: String,
    check: impl FnOnce() -> Result<()>,
    reject: impl FnOnce(DfmcpError) -> String,
) -> String {
    match check() {
        Ok(()) => output,
        Err(cause) => reject(cause),
    }
}
fn failure(cause: &DfmcpError, may_dispatch: bool) -> Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,
        "game_mutation_dispatched":if may_dispatch {Value::Null} else {json!(false)},
        "retained_state_currently_verified":false,"current_state_proven":false,"original_goal_proven":false})
}
fn unbound(operation: &str, cause: &DfmcpError) -> String {
    packet(
        operation,
        None,
        None,
        None,
        None,
        None,
        failure(cause, false),
        None,
        false,
        false,
    )
    .0
}
fn render(
    state: &mut State,
    operation: &str,
    request: Option<RequestId>,
    outcome: Result<View>,
    dispatched: bool,
) -> String {
    let verified = outcome.is_ok();
    let result = match outcome {
        Ok(view) => {
            let mut value = view.to_json();
            state.uncertain = value["frontier"]["needs_reconciliation"] == true;
            value["session_id"] = json!(state.runtime.session_id().to_string());
            value["retained_state_currently_verified"] = json!(true);
            state.retained = Some(value.clone());
            value
        }
        Err(cause) => {
            state.uncertain |= dispatched;
            let mut value = failure(&cause, dispatched);
            value["retained_reference"] = state
                .retained
                .as_ref()
                .map(presentation::compact)
                .unwrap_or(Value::Null);
            value
        }
    };
    let (out, overflow) = packet(
        operation,
        Some(state.runtime.session_id()),
        Some(state.runtime.anchor()),
        Some(state.runtime.mode().as_str()),
        request,
        Some(state.budget),
        result,
        state.retained.as_ref(),
        verified,
        state.uncertain,
    );
    if overflow {
        state.withdraw_and_retain();
        let fallback = json!({"ok":false,"error":{"code":"budget_exceeded",
            "message":"complete response exceeds output limit; local review withdrawn","retryable":false},
            "output_limit_bytes":60000,"retained_reference":state.retained.as_ref().map(presentation::compact),
            "retained_state_currently_verified":false,"automatic_redispatch":false,
            "current_state_proven":false,"original_goal_proven":false});
        return packet(
            operation,
            Some(state.runtime.session_id()),
            Some(state.runtime.anchor()),
            Some(state.runtime.mode().as_str()),
            request,
            Some(state.budget),
            fallback,
            state.retained.as_ref().map(presentation::compact).as_ref(),
            false,
            state.uncertain,
        )
        .0;
    }
    out
}
fn with_session(
    raw: String,
    operation: &str,
    wall: Option<u64>,
    action: impl FnOnce() -> Result<Action>,
) -> String {
    let result = (|| -> Result<String> {
        let started = Instant::now();
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.runtime.session_id() == id)
            .ok_or_else(|| {
                error(
                    ErrorCode::SessionNotFound,
                    "combined session absent or closed",
                )
            })?;
        let mut request = None;
        let mut dispatched = false;
        let mut may_need_reconciliation = false;
        let total = wall
            .unwrap_or(state.runtime.budget().max_wall_millis)
            .min(state.runtime.budget().max_wall_millis);
        let outcome = (|| -> Result<View> {
            runtime_io()?;
            let mut context = state.runtime.context(false)?;
            request = Some(context.request_id);
            if total == 0 {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "wall budget must be nonzero",
                ));
            }
            context.budget.max_wall_millis = remaining(started, total)?;
            let action = action()?;
            runtime_io()?;
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(remaining(started, total)?);
            let native_execution = state.runtime.has_native_execution();
            dispatched = action.may_dispatch() && native_execution;
            may_need_reconciliation = action.may_need_reconciliation() && native_execution;
            let view = action.perform(&mut state.runtime, &context)?;
            state.retained = Some(view.to_json());
            runtime_io()?;
            remaining(started, total)?;
            Ok(view)
        })();
        if outcome.is_err() {
            state.uncertain |= may_need_reconciliation;
            state.withdraw_and_retain();
        }
        let successful = outcome.is_ok();
        let output = render(state, operation, request, outcome, dispatched);
        if !successful {
            return Ok(output);
        }
        Ok(checked_publication(
            output,
            || {
                runtime_io()?;
                remaining(started, total).map(|_| ())
            },
            |cause| {
                state.uncertain |= may_need_reconciliation;
                state.withdraw_and_retain();
                render(state, operation, request, Err(cause), dispatched)
            },
        ))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepRequest {
    kind: String,
    original_plan_digest: String,
}
fn plan_action(raw: &str) -> Result<Action> {
    if raw.len() > 4096 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "combined request exceeds 4096 bytes",
        ));
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid combined request"))?;
    if value.get("kind").and_then(Value::as_str) == Some("step") {
        let request: StepRequest = serde_json::from_str(raw)
            .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed step request"))?;
        if request.kind != "step" {
            return Err(error(ErrorCode::InvalidRequest, "invalid step kind"));
        }
        Ok(Action::Prepare(digest(&request.original_plan_digest)?))
    } else {
        Ok(Action::Plan(Request::parse(raw)?))
    }
}

#[tool(
    description = "Open one operator-configured unadmitted combined labor and finite-order session. No game ticks are granted."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let result = (|| -> Result<String> {
        let started = Instant::now();
        runtime_io()?;
        let mode = Mode::parse(mode.as_deref().unwrap_or("offline"))?;
        let (display, mut work) = budgets(max_wall_millis, max_bytes, max_output_tokens)?;
        let config = Config::from_environment(runtime_io)?;
        let mut guard = session_lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the existing combined session first",
            ));
        }
        let sequence = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < (1u64 << 57)).then_some(n + 1)
            })
            .map_err(|_| error(ErrorCode::BudgetExceeded, "combined session IDs exhausted"))?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        work.max_wall_millis = remaining(started, display.max_wall_millis)?;
        let runtime = Runtime::open(config, mode, id, work)?;
        let opening = runtime.opening_recovery_reference();
        *guard = Some(State {
            runtime,
            budget: display,
            retained: Some(opening),
            uncertain: false,
        });
        let state = guard.as_mut().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "combined session disappeared",
            )
        })?;
        let mut request = None;
        let outcome = (|| -> Result<View> {
            runtime_io()?;
            let mut c = state.runtime.context(false)?;
            request = Some(c.request_id);
            c.budget.max_bytes = OPEN_INSPECTION_BYTES;
            c.budget.max_wall_millis = remaining(started, display.max_wall_millis)?;
            let view = state.runtime.inspect(&c)?;
            state.retained = Some(view.to_json());
            runtime_io()?;
            remaining(started, display.max_wall_millis)?;
            Ok(view)
        })();
        if outcome.is_err() {
            state.withdraw_and_retain();
        }
        let successful = outcome.is_ok();
        let output = render(state, "fortress.open_session", request, outcome, false);
        if !successful {
            return Ok(output);
        }
        Ok(checked_publication(
            output,
            || {
                runtime_io()?;
                remaining(started, display.max_wall_millis).map(|_| ())
            },
            |cause| {
                state.withdraw_and_retain();
                render(state, "fortress.open_session", request, Err(cause), false)
            },
        ))
    })();
    result.unwrap_or_else(|cause| unbound("fortress.open_session", &cause))
}
#[tool(
    description = "Observe paused canonical citizens and advisory labor choices. Optional closed JSON request selects 1-32 canonical units and an option_offset; every page is a fresh observation. No clock advance."
)]
pub fn fortress_observe(session_id: String, request: Option<String>) -> String {
    with_session(session_id, "fortress.observe", None, || {
        let request = match request {
            Some(raw) => ObserveRequest::parse(&raw)?,
            None => ObserveRequest::default(),
        };
        Ok(Action::Observe(request))
    })
}
#[tool(description = "Inspect retained original plan, frontier, review and recovery custody.")]
pub fn fortress_query(session_id: String) -> String {
    with_session(session_id, "fortress.query", None, || Ok(Action::Inspect))
}
#[tool(
    description = "Compile one original labor and finite-order plan, optionally with an explicit observed_inventory_minimum objective; kind=step with original_plan_digest prepares the next eligible original step."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || plan_action(&request))
}
#[tool(
    description = "Confirm exactly one current paired step review; lost replies require reconciliation of the same key."
)]
pub fn fortress_commit(session_id: String, paired_seal: String, confirmed: bool) -> String {
    with_session(session_id, "fortress.commit", None, || {
        Ok(Action::Commit(digest(&paired_seal)?, confirmed))
    })
}
#[tool(
    description = "Reconcile the original active key once; after both effects settle, sample an explicitly selected inventory goal with durable original timing. No dispatch or game-time advance."
)]
pub fn fortress_wait(session_id: String, max_wall_millis: Option<u64>) -> String {
    with_session(session_id, "fortress.wait", max_wall_millis, || {
        Ok(Action::Wait)
    })
}
#[tool(
    description = "Cancel a prepared step, or scope=session releases local resources without erasing custody or claiming cancellation."
)]
pub fn fortress_cancel(session_id: String, scope: Option<String>) -> String {
    if scope.as_deref() == Some("session") {
        let result = (|| -> Result<String> {
            let id = self::session_id(&session_id)?;
            let mut guard = session_lock()?;
            let state = guard
                .as_ref()
                .filter(|s| s.runtime.session_id() == id)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "combined session absent"))?;
            let out = packet("fortress.cancel", Some(id), Some(state.runtime.anchor()), Some(state.runtime.mode().as_str()), None, Some(state.budget),
                json!({"ok":true,"scope":"session","closed":true,"effects_cancelled":false,"custody_deleted":false,"native_calls":0,
                    "retained_reference":state.retained.as_ref().map(presentation::compact)}), state.retained.as_ref(), false, state.uncertain).0;
            drop(guard.take());
            Ok(out)
        })();
        return result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause));
    }
    with_session(session_id, "fortress.cancel", None, || {
        match scope.as_deref() {
            None | Some("step") => Ok(Action::Cancel),
            _ => Err(error(
                ErrorCode::InvalidRequest,
                "cancel scope must be step or session",
            )),
        }
    })
}
#[tool(
    description = "Explain retained original plan and recovery evidence without granting authority."
)]
pub fn fortress_explain(session_id: String) -> String {
    with_session(session_id, "fortress.explain", None, || Ok(Action::Inspect))
}
#[tool(description = "Inspect unadmitted development custody; no native effect.")]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, || Ok(Action::Inspect))
}
#[tool(description = "Unavailable in combined development; no checkpoint is written.")]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, || {
        Ok(Action::Unavailable)
    })
}
#[tool(description = "Unavailable in combined development; this does not restore game state.")]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, || {
        Ok(Action::Unavailable)
    })
}

pub fn run_stdio() {
    let server = ServerBuilder::new("dfmcp-live-combined-production-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit)
        .tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted development only. Observe, compile one original labor and finite-order plan, prepare and confirm each dependency-satisfied step, and reconcile uncertainty under its original key. All-applied does not prove the original goal or physical goods. An explicit inventory objective can be sampled with Wait after both effects settle; its success does not prove current labor or whole-plan completion. No clock is exposed.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn step_requests_are_closed_and_session_namespace_is_distinct() -> Result<()> {
        let id = SessionId::new((1u128 << 127) | FAMILY | 1);
        assert_eq!(session_id(&id.to_string())?, id);
        assert!(
            session_id(&SessionId::new((1u128 << 127) | (9u128 << 57) | 1).to_string()).is_err()
        );
        let seal = "a".repeat(64);
        assert!(matches!(
            plan_action(&json!({"kind":"step","original_plan_digest":seal}).to_string())?,
            Action::Prepare(_)
        ));
        for extra in ["key", "path", "source", "step_id"] {
            let mut value = json!({"kind":"step","original_plan_digest":seal});
            value[extra] = json!("forged");
            assert!(plan_action(&value.to_string()).is_err());
        }
        assert!(
            plan_action(&format!(
                "{{\"kind\":\"step\",\"kind\":\"step\",\"original_plan_digest\":\"{seal}\"}}"
            ))
            .is_err()
        );
        Ok(())
    }

    #[test]
    fn opening_reserves_full_output_and_never_grants_game_ticks() -> Result<()> {
        let (display, work) = budgets(None, None, None)?;
        assert_eq!(display.max_bytes, DEFAULT_BYTES);
        assert_eq!(
            display.max_bytes - work.max_bytes,
            RESPONSE_RESERVE + OPEN_INSPECTION_BYTES
        );
        assert_eq!(work.max_game_ticks, 0);
        assert_eq!(work.max_actions, 2);
        assert!(budgets(Some(60_001), None, None).is_err());
        assert!(budgets(None, Some(MAX_BYTES + 1), None).is_err());
        assert!(budgets(None, None, Some(MAX_OUTPUT - 1)).is_err());
        Ok(())
    }

    #[test]
    fn failed_publication_keeps_latest_floor_and_withdraws_old_review() {
        let mut retained = Some(
            json!({"observation_tick_floor":3,"recovery":{"frontier":{"authority_tick_floor":3}},
            "frontier":{"entries":[{"phase":"dispatch_started"}],"review":{"paired_seal":"old"}},
            "review":{"paired_seal":"old"}}),
        );
        merge_recovery(
            &mut retained,
            &json!({"observation_tick_floor":8,"recovery":{"frontier":{"authority_tick_floor":8}}}),
        );
        merge_recovery(&mut retained, &json!({"observation_tick_floor":7}));
        let value = retained.expect("retained test value");
        assert_eq!(value["observation_tick_floor"], 8);
        assert_eq!(value["recovery"]["frontier"]["authority_tick_floor"], 8);
        assert_eq!(value["frontier"]["entries"][0]["phase"], "dispatch_started");
        assert!(value["frontier"]["review"].is_null());
        assert!(value["review"].is_null());
    }

    #[test]
    fn failed_inventory_publication_keeps_new_history_custody_without_old_success() {
        let mut retained = Some(json!({"selected_inventory_goal":{"proven_at_anchor":true},
            "recovery":{"goal_history_present":false}}));
        merge_recovery(
            &mut retained,
            &json!({"recovery":{"goal_history_present":true},
            "selected_inventory_goal":{"scope":"selected_original_order_step","step_id":1,
                "proven_at_anchor":false,"current_truth":null,"history":{"present":true},
                "first_satisfied_anchor":{"tick":130}}}),
        );
        let value = retained.expect("retained custody");
        assert_eq!(value["recovery"]["goal_history_present"], true);
        assert_eq!(value["selected_inventory_goal"]["proven_at_anchor"], false);
        assert_eq!(
            value["selected_inventory_goal"]["current_truth"],
            Value::Null
        );
        assert_eq!(
            value["selected_inventory_goal"]["first_satisfied_anchor"]["tick"],
            130
        );
    }

    #[test]
    fn final_guard_failure_withdraws_rendered_review_and_retains_new_recovery() {
        let seal = "a".repeat(64);
        let session = SessionId::new((1u128 << 127) | FAMILY | 1);
        for code in [ErrorCode::CancellationRequested, ErrorCode::BudgetExceeded] {
            let mut retained = Some(json!({"recovery":{"frontier":{"authority_tick_floor":3}},
                "frontier":{"entries":[{"step_id":0,"idempotency_key":"original","phase":"prepared"}],
                    "review":{"paired_seal":seal}}, "review":{"paired_seal":seal}}));
            let published = packet(
                "fortress.plan",
                Some(session),
                None,
                Some("control"),
                None,
                None,
                json!({"ok":true,"review":{"paired_seal":seal}}),
                retained.as_ref(),
                true,
                false,
            )
            .0;
            assert!(
                published.contains(&seal),
                "precondition: review was fully rendered"
            );
            let out = checked_publication(
                published,
                || Err(error(code, "late guard failed")),
                |cause| {
                    merge_recovery(
                        &mut retained,
                        &json!({"recovery":{"frontier":{"authority_tick_floor":8}}}),
                    );
                    let mut failure = failure(&cause, false);
                    failure["retained_reference"] = retained
                        .as_ref()
                        .map(presentation::compact)
                        .unwrap_or(Value::Null);
                    packet(
                        "fortress.plan",
                        Some(session),
                        None,
                        Some("control"),
                        None,
                        None,
                        failure,
                        retained.as_ref(),
                        false,
                        true,
                    )
                    .0
                },
            );
            let result: Value = serde_json::from_str(&out).expect("failure packet");
            assert_eq!(result["ok"], false);
            assert_eq!(
                result["retained_reference"]["recovery"]["frontier"]["authority_tick_floor"],
                8
            );
            assert_eq!(
                result["retained_reference"]["frontier"]["entries"][0]["idempotency_key"],
                "original"
            );
            assert!(!out.contains(&seal), "withdrawn review is not exposed");
            assert!(retained.as_ref().expect("retained")["review"].is_null());
        }
    }

    #[test]
    fn uncertain_native_lifecycle_never_implies_setter_was_dispatched() {
        let id = Digest32::of_bytes(b"original plan");
        let prepare = Action::Prepare(id);
        assert!(prepare.may_need_reconciliation());
        assert!(!prepare.may_dispatch());
        assert!(Action::Commit(id, true).may_need_reconciliation());
        assert!(Action::Commit(id, true).may_dispatch());
        assert!(Action::Wait.may_need_reconciliation());
        assert!(Action::Cancel.may_need_reconciliation());
        assert!(!Action::Observe(ObserveRequest::default()).may_need_reconciliation());
        assert!(!Action::Inspect.may_need_reconciliation());
        assert!(!Action::Unavailable.may_need_reconciliation());
    }
}
