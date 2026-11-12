#![forbid(unsafe_code)]
//! Unadmitted combined labor and finite-order MCP host.
use crate::live_production_runtime::{
    Config, ExistingLaborRequest, InspectionRequest, Mode, ObserveRequest, Request, Runtime,
    StockTargetRequest, StockTargetsRequest, View,
};
use crate::semantic_work_order_runtime::clock::RunRequest;
use crate::semantic_work_order_runtime::inventory_clock::RunRequest as InventoryRunRequest;
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
const MAX_GAME_TICKS: u64 = 1200;
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
    retained["selected_order_goal"] = latest["selected_order_goal"].clone();
    retained["portfolio"] = latest["portfolio"].clone();
    retained["selected_parent_goal"] = Value::Null;
    retained["production_inspection_source"] = Value::Null;
    retained["production_inspection"] = Value::Null;
    retained["stock_target"] = latest["stock_target"].clone();
    if retained["stock_target"].is_object() {
        retained["stock_target"]["last_check_witness"] = Value::Null;
        retained["stock_target"]["fresh_for_current_call"] = json!(false);
        retained["stock_target"]["current_inventory_proven"] = json!(false);
    }
    retained["stock_targets"] = latest["stock_targets"].clone();
    if retained["stock_targets"].is_object() {
        retained["stock_targets"]["last_check_witness"] = Value::Null;
        retained["stock_targets"]["last_check_order_step_id"] = Value::Null;
        retained["stock_targets"]["fresh_for_current_call"] = json!(false);
        retained["stock_targets"]["current_inventory_proven"] = json!(false);
        if let Some(targets) = retained["stock_targets"]["targets"].as_array_mut() {
            for target in targets {
                target["last_check_witness"] = Value::Null;
                target["fresh_for_current_call"] = json!(false);
                target["current_inventory_proven"] = json!(false);
            }
        }
    }
    // A failed pre-plan native read has no parent recovery yet. Keep its exact
    // caller-held source checkpoint, while withdrawing any earlier completion.
    if latest["workshop_prerequisite"].is_object() {
        retained["workshop_prerequisite"] = latest["workshop_prerequisite"].clone();
    }
    if retained["workshop_prerequisite"].is_object() {
        retained["workshop_prerequisite"]["last_qualified"] = Value::Null;
    }
    if latest.get("clock").is_some() {
        retained["clock"] = latest["clock"].clone();
    }
    retained["clock_invalidates_goal"] = json!(
        retained["clock_invalidates_goal"] == true || latest["clock_invalidates_goal"] == true
    );
    if latest.get("inventory_clock").is_some() {
        retained["inventory_clock"] = latest["inventory_clock"].clone();
    }
    retained["inventory_clock_invalidates_goal"] = json!(
        retained["inventory_clock_invalidates_goal"] == true
            || latest["inventory_clock_invalidates_goal"] == true
    );
    retained["current_parent_goal"] = Value::Null;
    // A negative source fence survives an incomplete later recovery reference;
    // retaining it grants no current truth or native-effect authority.
    retained["parent_goal_observation_fenced"] = json!(
        retained["parent_goal_observation_fenced"] == true
            || latest["parent_goal_observation_fenced"] == true
    );
    retained["original_goal_proven"] = json!(false);
    retained["current_state_proven"] = json!(false);
    if retained["frontier"].is_object() {
        retained["frontier"]["review"] = Value::Null;
    }
}
enum Action {
    Observe(ObserveRequest),
    Inspect,
    InspectOrder(u32),
    InspectProduction(InspectionRequest, Option<u32>),
    Plan(Request),
    PlanExistingLabor(ExistingLaborRequest),
    PlanStockTarget(StockTargetRequest),
    PlanStockTargets(StockTargetsRequest),
    PlanExistingLaborPortfolio(crate::live_production_runtime::ExistingLaborPortfolioRequest),
    PlanPortfolio(crate::live_production_runtime::PortfolioRequest),
    PlanRun(RunRequest, Option<u32>),
    PlanInventoryRun(InventoryRunRequest, Option<u32>),
    Prepare(Digest32),
    Commit(Digest32, bool),
    CommitRun(Digest32, bool, Option<u32>),
    CommitInventoryRun(Digest32, bool, Option<u32>),
    Wait,
    Cancel,
    CancelRun(Option<u32>),
    CancelInventoryRun(Option<u32>),
    Unavailable,
}
impl Action {
    fn may_dispatch(&self) -> bool {
        matches!(
            self,
            Self::Commit(..)
                | Self::CommitRun(..)
                | Self::CommitInventoryRun(..)
                | Self::CancelRun(..)
                | Self::CancelInventoryRun(..)
        )
    }
    fn may_need_reconciliation(&self) -> bool {
        matches!(
            self,
            Self::Prepare(_)
                | Self::PlanRun(..)
                | Self::PlanInventoryRun(..)
                | Self::Commit(..)
                | Self::CommitRun(..)
                | Self::CommitInventoryRun(..)
                | Self::Wait
                | Self::Cancel
                | Self::CancelRun(..)
                | Self::CancelInventoryRun(..)
        )
    }
    fn perform(self, runtime: &mut Runtime, c: &OperationContext) -> Result<View> {
        match self {
            Self::Observe(request) => runtime.observe(request, c),
            Self::Inspect => runtime.inspect(c),
            Self::InspectOrder(step) => runtime.inspect_order(Some(step), c),
            Self::InspectProduction(request, step) => runtime.inspect_production(request, step, c),
            Self::Plan(request) => runtime.plan(request, c),
            Self::PlanExistingLabor(request) => runtime.plan_existing_labor(request, c),
            Self::PlanStockTarget(request) => runtime.plan_stock_target(request, c),
            Self::PlanStockTargets(request) => runtime.plan_stock_targets(request, c),
            Self::PlanExistingLaborPortfolio(request) => {
                runtime.plan_existing_labor_portfolio(request, c)
            }
            Self::PlanPortfolio(request) => runtime.plan_portfolio(request, c),
            Self::PlanRun(request, step) => {
                runtime.select_order(step, c)?;
                runtime.plan_run(request, c)
            }
            Self::PlanInventoryRun(request, step) => {
                runtime.select_order(step, c)?;
                runtime.plan_inventory_run(request, c)
            }
            Self::Prepare(id) => runtime.prepare_next(id, c),
            Self::Commit(seal, confirmed) => runtime.commit(seal, confirmed, c),
            Self::CommitRun(seal, confirmed, step) => {
                runtime.select_order(step, c)?;
                runtime.commit_run(seal, confirmed, c)
            }
            Self::CommitInventoryRun(seal, confirmed, step) => {
                runtime.select_order(step, c)?;
                runtime.commit_inventory_run(seal, confirmed, c)
            }
            Self::Wait => runtime.wait(c),
            Self::Cancel => runtime.cancel(c),
            Self::CancelRun(step) => {
                runtime.select_order(step, c)?;
                runtime.cancel_run(c)
            }
            Self::CancelInventoryRun(step) => {
                runtime.select_order(step, c)?;
                runtime.cancel_inventory_run(c)
            }
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
    game_ticks: Option<u64>,
) -> Result<(WorkBudget, WorkBudget)> {
    let display = WorkBudget {
        max_wall_millis: wall.unwrap_or(5000),
        max_bytes: bytes.unwrap_or(DEFAULT_BYTES),
        max_entities: 8192,
        max_actions: 5,
        max_output_tokens: output.unwrap_or(MAX_OUTPUT),
        max_game_ticks: game_ticks.unwrap_or(0),
    };
    display.validate()?;
    if display.max_wall_millis > 60_000
        || display.max_bytes > MAX_BYTES
        || display.max_output_tokens > MAX_OUTPUT
        || display.max_game_ticks > MAX_GAME_TICKS
        || u64::from(display.max_output_tokens) * 4 < RESPONSE_RESERVE
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "combined limits require at most 60000ms, 32GiB, 1200 ticks and a full response reservation",
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
            state.uncertain = value["frontier"]["needs_reconciliation"] == true
                || value["clock"]["unresolved"] == true;
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
    if raw.len() > 8192 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "combined request exceeds 8192 bytes",
        ));
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid combined request"))?;
    if value.get("kind").and_then(Value::as_str) == Some("run") {
        let (raw, step) = scoped_run(raw)?;
        RunRequest::parse(&raw).map(|request| Action::PlanRun(request, step))
    } else if value.get("kind").and_then(Value::as_str) == Some("inventory_run") {
        let (raw, step) = scoped_run(raw)?;
        InventoryRunRequest::parse(&raw).map(|request| Action::PlanInventoryRun(request, step))
    } else if value.get("kind").and_then(Value::as_str) == Some("portfolio") {
        crate::live_production_runtime::PortfolioRequest::parse(raw).map(Action::PlanPortfolio)
    } else if value.get("kind").and_then(Value::as_str) == Some("existing_labor") {
        ExistingLaborRequest::parse(raw).map(Action::PlanExistingLabor)
    } else if value.get("kind").and_then(Value::as_str) == Some("stock_target") {
        StockTargetRequest::parse(raw).map(Action::PlanStockTarget)
    } else if value.get("kind").and_then(Value::as_str) == Some("stock_targets") {
        StockTargetsRequest::parse(raw).map(Action::PlanStockTargets)
    } else if value.get("kind").and_then(Value::as_str) == Some("existing_labor_portfolio") {
        crate::live_production_runtime::ExistingLaborPortfolioRequest::parse(raw)
            .map(Action::PlanExistingLaborPortfolio)
    } else if value.get("kind").and_then(Value::as_str) == Some("step") {
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedRun {
    kind: String,
    original_paired_seal: String,
    game_ticks: u32,
    wall_millis: u32,
    #[serde(default, deserialize_with = "explicit_order_step")]
    order_step_id: Option<u32>,
}
fn explicit_order_step<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<u32>, D::Error> {
    u32::deserialize(deserializer).map(Some)
}
fn scoped_run(raw: &str) -> Result<(String, Option<u32>)> {
    let value: ScopedRun = serde_json::from_str(raw).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid closed original clock request",
        )
    })?;
    if value.order_step_id.is_some_and(|step| step > 4) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "order_step_id must name an original order from zero through four",
        ));
    }
    Ok((
        json!({"kind":value.kind,"original_paired_seal":value.original_paired_seal,
        "game_ticks":value.game_ticks,"wall_millis":value.wall_millis})
        .to_string(),
        value.order_step_id,
    ))
}

fn commit_action(
    scope: Option<&str>,
    seal: &str,
    confirmed: bool,
    step: Option<u32>,
) -> Result<Action> {
    let seal = digest(seal)?;
    match scope {
        None | Some("step") if step.is_none() => Ok(Action::Commit(seal, confirmed)),
        Some("run") => Ok(Action::CommitRun(seal, confirmed, step)),
        Some("inventory_run") => Ok(Action::CommitInventoryRun(seal, confirmed, step)),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "commit scope must be step, run or inventory_run",
        )),
    }
}

fn cancel_action(scope: Option<&str>, step: Option<u32>) -> Result<Action> {
    match scope {
        None | Some("step") if step.is_none() => Ok(Action::Cancel),
        Some("run") => Ok(Action::CancelRun(step)),
        Some("inventory_run") => Ok(Action::CancelInventoryRun(step)),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "cancel scope must be step, run, inventory_run or session",
        )),
    }
}

#[tool(
    description = "Open one operator-configured unadmitted combined labor and finite-order session. Optional max_game_ticks defaults to zero and is capped at 1200; clock control additionally requires independent operator authority. Opening never unpauses."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_game_ticks: Option<u64>,
) -> String {
    let result = (|| -> Result<String> {
        let started = Instant::now();
        runtime_io()?;
        let mode = Mode::parse(mode.as_deref().unwrap_or("offline"))?;
        let (display, mut work) = budgets(
            max_wall_millis,
            max_bytes,
            max_output_tokens,
            max_game_ticks,
        )?;
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
    description = "Observe paused canonical citizens and advisory labor choices. Optional closed JSON request selects 1-32 canonical units, option_offset or existing_labor_offset; every page is a fresh observation. Before original planning, include_production=true (alias include_operations=true) additionally captures complete source-pinned jobs, buildings and inventory at the same paused tick for read-only Query inspection. No clock advance or production authority is granted."
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
#[tool(
    description = "Inspect retained state, original plan, frontier, review and recovery custody. Optional order_step_id selects an original portfolio order. After Observe(include_production=true) before planning, or after a successful original-goal Wait, optional closed dfmcp.query/1 JSON request supports production_diagnosis, inventory_plan, aggregate, entities, inspect, traverse, dependencies, search, events, spatial_entities, related_entities and ranked_entities over that exact captured operations state. Aggregates report observed stock, including ineligible items; they do not establish usable supply. expected_anchor names production_inspection_source.anchor, not the citizen or parent anchor. Pre-plan inspection has no order selector and grants no mutation or goal authority. Diagnoses are observed conditions across captured jobs, not order-attributed causes; inventory plans allocate declared stack units without reserving items or proving native recipe feasibility. No native contact or clock advance."
)]
pub fn fortress_query(
    session_id: String,
    order_step_id: Option<u32>,
    request: Option<String>,
) -> String {
    with_session(session_id, "fortress.query", None, || match request {
        Some(raw) => Ok(Action::InspectProduction(
            InspectionRequest::parse(&raw)?,
            order_step_id,
        )),
        None => Ok(order_step_id.map_or(Action::Inspect, Action::InspectOrder)),
    })
}
#[tool(
    description = "Compile kind=stock_targets with expected_observation, units, labor, targets and original timing to size one to four distinct furniture minima from one qualified census. Each target supplies recipe, minimum_stack_units and optional max_order_amount (1..100, default100). Only deficits create orders; every requested minimum remains in the original goal. It uses already enabled eligible workers by default; optional assign_labor=true adds one separately reviewed labor-enable step before the dependent orders, for at most five original steps. All-met targets create no original or effect; any oversized deficit refuses the whole proposal. Each insertion revalidates the original census and exact receipt-backed prior-order queue. Compile kind=stock_target with expected_observation, units, labor, recipe, minimum_stack_units and original timing to derive one finite deficit from a fresh complete inventory census using already-enabled eligible workers by default. Optional assign_labor=true instead plans an explicitly confirmed labor enable before the dependent order; each step requires separate review. Optional max_order_amount is 1..100 (default100); an already-satisfied target creates no original or effect, and an oversized deficit refuses. Insertion requires the exact original census and an empty native order queue at each protected boundary. Otherwise compile one original labor and finite-order plan, kind=existing_labor for one finite order using observed already-enabled eligible workers without a labor assignment, kind=existing_labor_portfolio for one to four orders using the same qualified current workers, or kind=portfolio to assign labor before one to four distinct furniture orders; every order may select observed_inventory_minimum or observed_inventory_increase with item_type and increase_stack_units (1..1000000). Increase seals a server-qualified complete original inventory baseline before preparation; clients cannot supply or renew that baseline; kind=step with original_plan_digest prepares the next eligible original step. After all original effects settle, kind=run prepares an independently authorized clock for the original retained-order or inventory goal. After Wait observes True original labor and explicitly selected inventory, kind=inventory_run prepares an independent stability clock even after order retirement. Both name original_paired_seal, game_ticks and wall_millis; portfolio clock requests also require the exact original order_step_id. When an original workshop prerequisite is configured, the plan retains its exact identity and each native order boundary rereads its completion. Planning never unpauses."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || plan_action(&request))
}
#[tool(
    description = "scope=step (default) confirms one current paired step review. scope=run or scope=inventory_run confirms that separate clock owner's review in paired_seal under its independent authority. All require confirmed=true; portfolio clock confirmation also requires its original order_step_id. Lost replies require reconciliation of the original key without repeated insertion or unpause."
)]
pub fn fortress_commit(
    session_id: String,
    paired_seal: String,
    confirmed: bool,
    scope: Option<String>,
    order_step_id: Option<u32>,
) -> String {
    with_session(session_id, "fortress.commit", None, || {
        commit_action(scope.as_deref(), &paired_seal, confirmed, order_step_id)
    })
}
#[tool(
    description = "Reconcile original effect and clock keys before sampling current labor and the original retained-order or explicit inventory goal together under original timing. Unresolved clocks block paused sampling. Wait never inserts, starts a run or retries unpause."
)]
pub fn fortress_wait(session_id: String, max_wall_millis: Option<u64>) -> String {
    with_session(session_id, "fortress.wait", max_wall_millis, || {
        Ok(Action::Wait)
    })
}
#[tool(
    description = "scope=step (default) cancels a prepared step. scope=run or scope=inventory_run retires the selected prepared clock or requests its retained native safety pause under its independent authority. scope=session releases local resources without erasing custody or claiming effects were cancelled."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: Option<String>,
    order_step_id: Option<u32>,
) -> String {
    if scope.as_deref() == Some("session") && order_step_id.is_none() {
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
        cancel_action(scope.as_deref(), order_step_id)
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
        .instructions("Unadmitted development only. Observe, use kind=stock_targets to size one to four distinct furniture stock targets from one fresh qualified census with existing eligible workers by default, or explicit assign_labor=true to review one labor enable before the dependent orders. Every requested minimum, including initially met stock, remains in the complete original goal; each nonzero deficit needs its own reviewed order. A wholly satisfied request creates no original or effect. Use kind=stock_target to size one finite stock deficit from a fresh independently qualified census using already-enabled workers by default, with optional assign_labor=true to review a labor enable before the dependent order; a met target creates neither effect, or compile one original labor and finite-order plan, kind=existing_labor for one order using current qualified labor without assigning it, kind=existing_labor_portfolio for up to four orders using the same workers, or kind=portfolio to assign labor before up to four furniture orders, prepare and confirm each dependency-satisfied step, and reconcile uncertainty under its original key. All-applied does not prove the original goal or physical goods. Wait joins current labor with the original retained-order completion or explicitly selected inventory goal under the original stability requirement. Separately configured clock control can prepare kind=run and confirm scope=run for either objective while the exact original order remains eligible; after a True original labor/inventory sample, kind/scope=inventory_run can finish explicitly selected inventory stability after native order retirement. Portfolio clock Plan/Commit/Cancel calls require the original order_step_id; Query can select that leaf. Portfolio Wait samples one original leaf per call and proves the parent only after a coherent complete cohort. Both clock owners preserve every original deadline and exclude competing pending clocks across all leaves; no automatic advancement or repeated unpause is permitted. Unresolved runs must reconcile before paused sampling. A disappeared order leaves default completion unknown. Inventory presence does not prove order-attributed production. Parent proof is process-local and withdrawn on failure or reopening. An optionally configured independently acknowledged Carpenter workshop is bound to the original plan and freshly checked at each order preparation and dispatch boundary. Preserve its source checkpoint even when planning fails before an original exists; historical workshop completion is not a production-goal proof.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {

    #[test]
    fn stock_targets_route_is_closed_and_sizes_only_server_owned_deficits() {
        let request = serde_json::json!({
            "kind":"stock_targets", "expected_observation":"11".repeat(32),
            "units":["43"], "labor":"CARPENTER", "deadline_tick":300,
            "targets":[
                {"recipe":"wooden_bed","minimum_stack_units":7,"max_order_amount":10},
                {"recipe":"wooden_door","minimum_stack_units":2}
            ]
        });
        assert!(matches!(
            super::plan_action(&request.to_string()),
            Ok(super::Action::PlanStockTargets(_))
        ));
        for (field, value) in [
            ("amount", serde_json::json!(3)),
            ("baseline_stack_units", serde_json::json!(4)),
            ("source_digest", serde_json::json!("22".repeat(32))),
            ("assign_labor", serde_json::json!("true")),
            ("targets", serde_json::json!([])),
        ] {
            let mut bad = request.clone();
            bad[field] = value;
            assert!(super::plan_action(&bad.to_string()).is_err(), "{field}");
        }
        for (field, value) in [
            ("amount", serde_json::json!(3)),
            ("baseline_stack_units", serde_json::json!(4)),
            ("source_digest", serde_json::json!("22".repeat(32))),
            ("max_order_amount", serde_json::Value::Null),
            ("minimum_stack_units", serde_json::json!(0)),
        ] {
            let mut bad = request.clone();
            bad["targets"][0][field] = value;
            assert!(super::plan_action(&bad.to_string()).is_err(), "{field}");
        }
        let duplicate = request.to_string().replacen("{", "{\"targets\":[],", 1);
        assert!(super::plan_action(&duplicate).is_err());
    }

    #[test]
    fn stock_targets_labor_assignment_is_an_explicit_boolean_opt_in() {
        let mut request = serde_json::json!({
            "kind":"stock_targets", "expected_observation":"11".repeat(32),
            "units":["43"], "labor":"CARPENTER", "deadline_tick":300,
            "targets":[{"recipe":"wooden_bed","minimum_stack_units":7}]
        });
        for selection in [None, Some(false), Some(true)] {
            if let Some(value) = selection {
                request["assign_labor"] = value.into();
            }
            let action = super::plan_action(&request.to_string()).expect("stock targets route");
            let super::Action::PlanStockTargets(parsed) = action else {
                panic!("stock targets lost its request route");
            };
            assert_eq!(parsed.assign_labor, selection == Some(true));
        }
        for value in [
            serde_json::Value::Null,
            serde_json::json!(1),
            serde_json::json!("true"),
            serde_json::json!({"enabled":true}),
        ] {
            request["assign_labor"] = value;
            assert!(super::plan_action(&request.to_string()).is_err());
        }
        request["assign_labor"] = true.into();
        for field in ["enabled", "amount", "baseline_stack_units", "order_step_id"] {
            let mut forged = request.clone();
            forged[field] = true.into();
            assert!(super::plan_action(&forged.to_string()).is_err(), "{field}");
        }
        let duplicate = request
            .to_string()
            .replacen("{", "{\"assign_labor\":false,", 1);
        assert!(super::plan_action(&duplicate).is_err());
    }

    #[test]
    fn failed_stock_targets_publication_retains_every_original_target_without_proof() {
        let mut retained = Some(serde_json::json!({
            "observation_tick_floor":120,
            "review":{"paired_seal":"a".repeat(64)}
        }));
        let latest = serde_json::json!({
            "observation_tick_floor":130,
            "stock_targets":{
                "schema":"dfmcp.stock-targets/1","status":"planned",
                "revalidation_failed":true,"consumed_tick_floor":130,
                "assessment_witness":"b".repeat(64),
                "fresh_for_current_call":true,"current_inventory_proven":true,
                "last_check_witness":"c".repeat(64),"last_check_order_step_id":0,
                "targets":[
                    {"recipe":"wooden_bed","target_stack_units":7,"amount":3,
                     "order_step_id":0,"current_inventory_proven":true},
                    {"recipe":"wooden_door","target_stack_units":2,"amount":0,
                     "order_step_id":null,"current_inventory_proven":true}
                ]
            }
        });
        super::merge_recovery(&mut retained, &latest);
        let retained = retained.expect("retained stock targets");
        let targets = &retained["stock_targets"];
        assert_eq!(retained["observation_tick_floor"], 130);
        assert_eq!(targets["assessment_witness"], "b".repeat(64));
        assert_eq!(targets["targets"].as_array().expect("targets").len(), 2);
        assert_eq!(targets["targets"][0]["amount"], 3);
        assert_eq!(targets["targets"][1]["target_stack_units"], 2);
        assert!(targets["targets"][1]["order_step_id"].is_null());
        assert_eq!(targets["revalidation_failed"], true);
        assert_eq!(targets["current_inventory_proven"], false);
        assert!(targets["last_check_witness"].is_null());
        assert!(targets["last_check_order_step_id"].is_null());
        assert!(targets["targets"].as_array().expect("targets").iter().all(
            |row| row["current_inventory_proven"] == false
                && row["fresh_for_current_call"] == false
                && row["last_check_witness"].is_null()
        ));
        assert!(retained["review"].is_null());
    }

    #[test]
    fn stock_target_plan_route_is_closed_and_never_accepts_a_client_amount_or_baseline() {
        let request = serde_json::json!({"kind":"stock_target", "expected_observation":"11".repeat(32),
            "units":["43"],"labor":"CARPENTER","recipe":"wooden_bed",
            "minimum_stack_units":7,"max_order_amount":10,"deadline_tick":300});
        assert!(matches!(
            super::plan_action(&request.to_string()),
            Ok(super::Action::PlanStockTarget(_))
        ));
        for (field, value) in [
            ("amount", serde_json::json!(3)),
            ("baseline_stack_units", serde_json::json!(4)),
            ("source_digest", serde_json::json!("22".repeat(32))),
            ("enabled", serde_json::json!(true)),
            ("max_order_amount", serde_json::Value::Null),
            ("minimum_stack_units", serde_json::json!(0)),
        ] {
            let mut bad = request.clone();
            bad[field] = value;
            assert!(super::plan_action(&bad.to_string()).is_err(), "{field}");
        }
    }

    #[test]
    fn stock_target_labor_assignment_is_an_explicit_boolean_opt_in() {
        let mut request = serde_json::json!({
            "kind":"stock_target", "expected_observation":"11".repeat(32),
            "units":["43"], "labor":"CARPENTER", "recipe":"wooden_bed",
            "minimum_stack_units":7, "deadline_tick":300
        });
        for selection in [None, Some(false), Some(true)] {
            if let Some(value) = selection {
                request["assign_labor"] = value.into();
            }
            let action = super::plan_action(&request.to_string()).expect("stock target route");
            let super::Action::PlanStockTarget(parsed) = action else {
                panic!("stock target lost its request route");
            };
            assert_eq!(parsed.assign_labor, selection == Some(true));
        }
        for value in [
            serde_json::Value::Null,
            serde_json::json!(1),
            serde_json::json!("true"),
            serde_json::json!({"enabled":true}),
        ] {
            request["assign_labor"] = value;
            assert!(super::plan_action(&request.to_string()).is_err());
        }
        request["assign_labor"] = true.into();
        for field in ["enabled", "amount", "baseline_stack_units", "order_step_id"] {
            let mut forged = request.clone();
            forged[field] = true.into();
            assert!(super::plan_action(&forged.to_string()).is_err(), "{field}");
        }
        let duplicate = request
            .to_string()
            .replacen("{", "{\"assign_labor\":false,", 1);
        assert!(super::plan_action(&duplicate).is_err());
    }

    #[test]
    fn failed_stock_target_publication_preserves_sizing_and_consumed_floor() {
        let mut retained = Some(serde_json::json!({"stock_target":{"status":"planned",
            "amount":3,"target_stack_units":7,"last_check_witness":"a".repeat(64)},
            "observation_tick_floor":120,"review":{"paired_seal":"b".repeat(64)}}));
        let latest = serde_json::json!({"stock_target":{"status":"planned",
            "amount":3,"target_stack_units":7,"assessment_witness":"c".repeat(64),
            "last_check_witness":null,"revalidation_failed":true,
            "consumed_tick_floor":130},"observation_tick_floor":130});
        super::merge_recovery(&mut retained, &latest);
        let retained = retained.expect("retained stock target");
        assert_eq!(retained["observation_tick_floor"], 130);
        assert_eq!(retained["stock_target"]["amount"], 3);
        assert_eq!(retained["stock_target"]["target_stack_units"], 7);
        assert_eq!(
            retained["stock_target"]["assessment_witness"],
            "c".repeat(64)
        );
        assert_eq!(retained["stock_target"]["revalidation_failed"], true);
        assert_eq!(retained["stock_target"]["current_inventory_proven"], false);
        assert!(retained["stock_target"]["last_check_witness"].is_null());
        assert!(retained["review"].is_null());
    }

    #[test]
    fn existing_labor_portfolio_is_closed_and_zero_order_clock_selectors_remain_explicit() {
        let mut raw = serde_json::json!({"kind":"existing_labor_portfolio","expected_observation":"11".repeat(32),
            "units":["43"],"labor":"MINE","orders":[{"recipe":"wooden_bed","amount":1},{"recipe":"wooden_door","amount":2}],
            "deadline_tick":300});
        assert!(matches!(
            super::plan_action(&raw.to_string()),
            Ok(super::Action::PlanExistingLaborPortfolio(_))
        ));
        raw["enabled"] = true.into();
        assert!(super::plan_action(&raw.to_string()).is_err());
        let mut run = serde_json::json!({"kind":"run","original_paired_seal":"11".repeat(32),
            "game_ticks":5,"wall_millis":1000,"order_step_id":0});
        assert_eq!(
            super::scoped_run(&run.to_string())
                .expect("zero original order")
                .1,
            Some(0)
        );
        for invalid in [
            serde_json::Value::Null,
            serde_json::json!(5),
            serde_json::json!(-1),
        ] {
            run["order_step_id"] = invalid;
            assert!(super::scoped_run(&run.to_string()).is_err());
        }
    }
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
    fn opening_reserves_full_output_and_defaults_to_no_game_ticks() -> Result<()> {
        let (display, work) = budgets(None, None, None, None)?;
        assert_eq!(display.max_bytes, DEFAULT_BYTES);
        assert_eq!(
            display.max_bytes - work.max_bytes,
            RESPONSE_RESERVE + OPEN_INSPECTION_BYTES
        );
        assert_eq!(work.max_game_ticks, 0);
        assert_eq!(work.max_actions, 5);
        assert!(budgets(Some(60_001), None, None, None).is_err());
        assert!(budgets(None, Some(MAX_BYTES + 1), None, None).is_err());
        assert!(budgets(None, None, Some(MAX_OUTPUT - 1), None).is_err());
        let (display, work) = budgets(None, None, None, Some(MAX_GAME_TICKS))?;
        assert_eq!(display.max_game_ticks, MAX_GAME_TICKS);
        assert_eq!(work.max_game_ticks, MAX_GAME_TICKS);
        assert!(budgets(None, None, None, Some(MAX_GAME_TICKS + 1)).is_err());
        Ok(())
    }

    #[test]
    fn clock_requests_keep_original_review_identity_and_distinct_commit_scope() -> Result<()> {
        let seal = "a".repeat(64);
        let request = json!({"kind":"run","original_paired_seal":seal,
            "game_ticks":5,"wall_millis":1000});
        assert!(matches!(
            plan_action(&request.to_string())?,
            Action::PlanRun(..)
        ));
        for field in [
            "path",
            "order_id",
            "generation",
            "native_method",
            "deadline_tick",
        ] {
            let mut invalid = request.clone();
            invalid[field] = json!(1);
            assert!(plan_action(&invalid.to_string()).is_err());
        }
        for ticks in [0, 1201] {
            let mut invalid = request.clone();
            invalid["game_ticks"] = json!(ticks);
            assert!(plan_action(&invalid.to_string()).is_err());
        }
        assert!(matches!(
            commit_action(None, &seal, true, None)?,
            Action::Commit(..)
        ));
        assert!(matches!(
            commit_action(Some("step"), &seal, false, None)?,
            Action::Commit(_, false)
        ));
        assert!(matches!(
            commit_action(Some("run"), &seal, true, None)?,
            Action::CommitRun(..)
        ));
        assert!(matches!(
            cancel_action(Some("run"), None)?,
            Action::CancelRun(..)
        ));
        for scope in ["goal", "all", "session"] {
            assert!(commit_action(Some(scope), &seal, true, None).is_err());
            assert!(cancel_action(Some(scope), None).is_err());
        }
        assert!(commit_action(Some("run"), "invalid", true, None).is_err());
        assert!(Action::CommitRun(digest(&seal)?, true, None).may_dispatch());
        assert!(Action::CancelRun(None).may_dispatch());
        assert!(Action::CancelRun(None).may_need_reconciliation());
        let mut inventory = request;
        inventory["kind"] = json!("inventory_run");
        assert!(matches!(
            plan_action(&inventory.to_string())?,
            Action::PlanInventoryRun(..)
        ));
        assert!(matches!(
            commit_action(Some("inventory_run"), &seal, true, None)?,
            Action::CommitInventoryRun(..)
        ));
        assert!(matches!(
            cancel_action(Some("inventory_run"), None)?,
            Action::CancelInventoryRun(..)
        ));
        assert!(Action::CommitInventoryRun(digest(&seal)?, true, None).may_dispatch());
        assert!(Action::CancelInventoryRun(None).may_dispatch());
        assert!(Action::CancelInventoryRun(None).may_need_reconciliation());
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
    fn failed_preplan_publication_retains_latest_workshop_checkpoint() {
        let mut retained = Some(json!({"workshop_prerequisite":{
            "origin_witness":"a".repeat(64),"source_fenced":false,
            "last_qualified":{"status":"complete"}}}));
        let checkpoint = json!({"origin_witness":"a".repeat(64),"tick":190,
            "generation":9,"next_building_id":70,"fenced":true});
        merge_recovery(
            &mut retained,
            &json!({"recovery":null,
            "observation_tick_floor":190,"workshop_prerequisite":{
            "origin_witness":"a".repeat(64),"source_fenced":true,
            "checkpoint":checkpoint,"last_qualified":null}}),
        );
        merge_recovery(&mut retained, &json!({"observation_tick_floor":180}));
        let value = retained.expect("retained workshop checkpoint");
        assert!(value["recovery"].is_null());
        assert!(value["workshop_prerequisite"]["last_qualified"].is_null());
        assert_eq!(value["workshop_prerequisite"]["checkpoint"], checkpoint);
        assert_eq!(value["workshop_prerequisite"]["source_fenced"], true);
        assert_eq!(value["observation_tick_floor"], 190);
    }

    #[test]
    fn failed_inventory_publication_keeps_new_history_custody_without_old_success() {
        let mut retained = Some(json!({"selected_inventory_goal":{"proven_at_anchor":true},
            "current_parent_goal":{"current_truth":"true","original_goal_proven":true},
            "original_goal_proven":true,
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
        assert!(value["current_parent_goal"].is_null());
        assert_eq!(value["original_goal_proven"], false);
        assert_eq!(value["current_state_proven"], false);
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
    fn failed_parent_publication_retains_source_fence_without_current_proof() {
        for (old_fenced, new_fenced) in [(true, false), (false, true)] {
            let mut retained = Some(json!({
                "parent_goal_observation_fenced":old_fenced,
                "current_parent_goal":{"current_truth":"true","original_goal_proven":true},
                "original_goal_proven":true}));
            merge_recovery(
                &mut retained,
                &json!({"parent_goal_observation_fenced":new_fenced}),
            );
            // An incomplete later recovery reference cannot erase a refusal.
            merge_recovery(
                &mut retained,
                &json!({"parent_goal_observation_fenced":false}),
            );
            let value = retained.expect("retained source refusal");
            assert_eq!(value["parent_goal_observation_fenced"], true);
            assert!(value["current_parent_goal"].is_null());
            assert_eq!(value["original_goal_proven"], false);
            let compacted = presentation::compact(&value);
            assert_eq!(compacted["parent_goal_observation_fenced"], true);
            assert!(compacted["current_parent_goal"].is_null());
            assert_eq!(compacted["original_goal_proven"], false);
        }
    }

    #[test]
    fn failed_clock_publication_keeps_newest_custody_without_current_goal_or_review() {
        let mut retained = Some(json!({"clock_invalidates_goal":false,
            "clock":{"head":"old","review_available":true},
            "current_parent_goal":{"original_goal_proven":true},
            "original_goal_proven":true}));
        merge_recovery(
            &mut retained,
            &json!({"clock_invalidates_goal":true,
            "clock":{"head":"new","review_available":false,"unresolved":true},
            "recovery":{"clock_journal_id":"a".repeat(64)},
            "selected_inventory_goal":{"proven_at_anchor":false}}),
        );
        merge_recovery(&mut retained, &json!({"clock_invalidates_goal":false}));
        let value = retained.expect("clock recovery retained");
        assert_eq!(value["clock_invalidates_goal"], true);
        assert_eq!(value["clock"]["head"], "new");
        assert_eq!(value["clock"]["review_available"], false);
        assert_eq!(value["clock"]["unresolved"], true);
        assert_eq!(value["recovery"]["clock_journal_id"], "a".repeat(64));
        assert_eq!(value["original_goal_proven"], false);
        assert!(value["current_parent_goal"].is_null());
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
