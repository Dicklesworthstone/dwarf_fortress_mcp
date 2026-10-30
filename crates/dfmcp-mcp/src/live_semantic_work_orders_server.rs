#![forbid(unsafe_code)]
//! Explicitly unadmitted, one-goal native work-order MCP composition root.
//! All planning, effect custody and completion decisions remain in the runtime
//! and adapter. MCP arguments cannot choose sources, credentials or paths.
use crate::semantic_work_order_runtime::clock::RunRequest;
use crate::semantic_work_order_runtime::{
    Config, MAX_BYTES, MAX_GAME_TICKS, MAX_OUTPUT_TOKENS, MAX_WALL_MILLIS, Mode, NativeRuntime,
    Request, View, digest,
};
use dfmcp_core::{
    DfmcpError, Digest32, ErrorCode, OperationContext, RequestId, Result, SessionId, WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Instant;

#[path = "semantic_work_order_presentation.rs"]
mod presentation;
use presentation::{RESPONSE_RESERVE, packet};

const FAMILY: u128 = 30u128 << 57;
const SEQUENCE_LIMIT: u64 = 1u64 << 57;
const OPEN_INSPECTION_BYTES: u64 = 32 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State>> = Mutex::new(None);

struct State {
    runtime: NativeRuntime,
    display_budget: WorkBudget,
    // A failed call must not spend the operation budget again on a second
    // inspection. Keep the last complete view only as an explicitly stale
    // recovery reference until the caller asks for a new bounded inspection.
    last_view: Option<Value>,
    insertion_uncertain: bool,
    clock_uncertain: bool,
}

enum Action {
    Observe,
    Inspect,
    Plan(Request),
    PlanRun(RunRequest),
    Commit { seal: Digest32, confirmed: bool },
    CommitRun { seal: Digest32, confirmed: bool },
    Wait,
    Cancel,
    CancelRun,
    Unavailable,
}
impl Action {
    fn may_insert(&self) -> bool {
        matches!(self, Self::Commit { .. })
    }
    fn may_control_clock(&self) -> bool {
        matches!(self, Self::CommitRun { .. } | Self::CancelRun)
    }
    fn perform(self, runtime: &mut NativeRuntime, c: &OperationContext) -> Result<View> {
        match self {
            Self::Observe => runtime.observe(c),
            Self::Inspect => runtime.inspect(c),
            Self::Plan(request) => runtime.plan(request, c),
            Self::PlanRun(request) => runtime.plan_run(request, c),
            Self::Commit { seal, confirmed } => runtime.commit(seal, confirmed, c),
            Self::CommitRun { seal, confirmed } => runtime.commit_run(seal, confirmed, c),
            Self::Wait => runtime.wait(c),
            Self::Cancel => runtime.cancel(c),
            Self::CancelRun => runtime.cancel_run(c),
            Self::Unavailable => Err(error(
                ErrorCode::CapabilityDenied,
                "this semantic work-order runtime does not implement checkpoint or restore",
            )),
        }
    }
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn failure(cause: &DfmcpError, attempted_insertion: bool, attempted_clock: bool) -> Value {
    json!({
        "ok":false,
        "error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,
        "game_mutation_dispatched":if attempted_insertion || attempted_clock { Value::Null } else { json!(false) },
        "insertion_may_have_started":attempted_insertion,
        "clock_effect_may_have_started":attempted_clock,
        "retained_state_currently_verified":false,
        "current_goal_available":false,
        "recovery_guidance":"Inspect retained custody; reconcile the original insertion and any clock run. Never replace the original goal, repeat insertion or repeat unpause to resolve uncertainty."
    })
}

fn unbound(operation: &str, cause: &DfmcpError) -> String {
    packet(
        operation,
        None,
        None,
        None,
        None,
        None,
        failure(cause, false, false),
        None,
        false,
    )
}

fn validate_environment() -> Result<()> {
    if crate::admission::current_admission_provenance().is_some() {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "unadmitted semantic work-order runtime refuses production admission provenance",
        ));
    }
    Config::validate_environment()
}

fn runtime_io() -> Result<()> {
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "semantic MCP I/O requires its owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "semantic work-order request cancelled",
        )
    })?;
    if !cx.capabilities().io {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited context denies semantic MCP I/O",
        ));
    }
    Ok(())
}

fn session_lock() -> Result<MutexGuard<'static, Option<State>>> {
    match SESSION.try_lock() {
        Ok(guard) => Ok(guard),
        Err(TryLockError::WouldBlock) => Err(error(
            ErrorCode::BudgetExceeded,
            "semantic session is serving another bounded request",
        )),
        Err(TryLockError::Poisoned(_)) => Err(error(
            ErrorCode::InternalInvariantViolation,
            "semantic session poisoned; restart using exact original recovery custody",
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
            "invalid semantic work-order session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid semantic work-order session ID",
        )
    })?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 30
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "session belongs to another runtime or process",
        ));
    }
    Ok(id)
}

fn opening_budgets(
    wall: Option<u64>,
    bytes: Option<u64>,
    output: Option<u32>,
    game_ticks: Option<u64>,
) -> Result<(WorkBudget, WorkBudget)> {
    let display = WorkBudget {
        max_wall_millis: wall.unwrap_or(5000),
        max_bytes: bytes.unwrap_or(MAX_BYTES),
        max_output_tokens: output.unwrap_or(MAX_OUTPUT_TOKENS),
        max_entities: 4096,
        max_actions: 1,
        max_game_ticks: game_ticks.unwrap_or(0),
    };
    display.validate()?;
    if display.max_game_ticks > MAX_GAME_TICKS
        || display.max_wall_millis > MAX_WALL_MILLIS
        || display.max_bytes > MAX_BYTES
        || display.max_output_tokens > MAX_OUTPUT_TOKENS
        || u64::from(display.max_output_tokens) * 4 < RESPONSE_RESERVE
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "semantic limits require at most 1200 game ticks, 60000ms and 2GiB byte-work, with 32768 output proxy units reserved for a complete response",
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
                "complete opening and response reservations do not fit; no I/O started",
            )
        })?;
    Ok((display, work))
}

fn remaining_millis(started: Instant, total: u64) -> Result<u64> {
    let elapsed = u64::try_from(started.elapsed().as_millis()).map_err(|_| {
        error(
            ErrorCode::BudgetExceeded,
            "semantic request deadline overflow",
        )
    })?;
    total
        .checked_sub(elapsed)
        .filter(|n| *n > 0)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "semantic request deadline expired",
            )
        })
}

fn render(
    state: &mut State,
    operation: &str,
    request: Option<RequestId>,
    outcome: Result<View>,
    attempted_insertion: bool,
    attempted_clock: bool,
) -> String {
    let result = match outcome {
        Ok(view) => {
            let mut value = view.to_json();
            state.insertion_uncertain = matches!(
                value["native_state"].as_str(),
                Some("dispatch_started" | "indeterminate")
            );
            state.clock_uncertain = value["clock"]["unresolved"] == true;
            value["session_id"] = json!(state.runtime.session_id().to_string());
            value["retained_state_currently_verified"] = json!(true);
            value["development_production_enabled"] =
                json!(state.runtime.production_control_enabled());
            value["development_clock_enabled"] = json!(state.runtime.clock_control_enabled());
            state.last_view = Some(value.clone());
            value
        }
        Err(cause) => {
            state.insertion_uncertain |= attempted_insertion;
            state.clock_uncertain |= attempted_clock;
            failure(&cause, attempted_insertion, attempted_clock)
        }
    };
    packet(
        operation,
        Some(state.runtime.session_id()),
        Some(state.runtime.anchor()),
        Some(state.runtime.mode().as_str()),
        request,
        Some(state.display_budget),
        result,
        state.last_view.clone(),
        state.insertion_uncertain || state.clock_uncertain,
    )
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
                    "semantic session absent or closed",
                )
            })?;
        let mut request = None;
        let mut attempted_insertion = false;
        let mut attempted_clock = false;
        let outcome = (|| -> Result<View> {
            runtime_io()?;
            validate_environment()?;
            let mut context = state.runtime.context(false)?;
            request = Some(context.request_id);
            let total = wall
                .unwrap_or(state.runtime.budget().max_wall_millis)
                .min(state.runtime.budget().max_wall_millis);
            if total == 0 {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "wall budget must be nonzero",
                ));
            }
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(remaining_millis(started, total)?);
            let action = action()?;
            runtime_io()?;
            validate_environment()?;
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(remaining_millis(started, total)?);
            attempted_insertion = action.may_insert();
            attempted_clock = action.may_control_clock();
            let view = action.perform(&mut state.runtime, &context)?;
            // Preserve newly published recovery seals even if the caller is
            // cancelled before response publication. The error projection will
            // still mark this cache unverified and cannot authorize control.
            state.last_view = Some(view.to_json());
            // A completed effect is not undone when cancellation or authority
            // changes during the call. Such a response retains uncertainty.
            runtime_io()?;
            validate_environment()?;
            remaining_millis(started, total)?;
            Ok(view)
        })();
        Ok(render(
            state,
            operation,
            request,
            outcome,
            attempted_insertion,
            attempted_clock,
        ))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

#[tool(
    description = "Open one unadmitted semantic furniture-order goal. Default offline reads existing custody; reconcile can acquire evidence without insertion authority; control requires separately enabled production or clock authority. Sources, credentials, endpoint and private custody are operator configuration. Optional max_game_ticks defaults to zero and is capped at 1200; opening never unpauses."
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
        validate_environment()?;
        let mode = Mode::parse(mode.as_deref().unwrap_or("offline"))?;
        let (display, mut work) = opening_budgets(
            max_wall_millis,
            max_bytes,
            max_output_tokens,
            max_game_ticks,
        )?;
        let config = Config::from_environment()?;
        let mut guard = session_lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the retained semantic session before opening another",
            ));
        }
        let sequence = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < SEQUENCE_LIMIT).then_some(n + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "semantic session identities exhausted",
                )
            })?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        work.max_wall_millis = remaining_millis(started, display.max_wall_millis)?;
        let runtime = NativeRuntime::open_with_runtime_check(config, mode, id, work, runtime_io)?;
        // Retain custody even if the separately reserved opening inspection
        // fails, so the returned handle can inspect or explicitly close it.
        let opening_reference = runtime.opening_recovery_reference();
        *guard = Some(State {
            runtime,
            display_budget: display,
            last_view: Some(opening_reference),
            insertion_uncertain: false,
            clock_uncertain: false,
        });
        let state = guard.as_mut().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "opening lost its owned session",
            )
        })?;
        let mut request = None;
        let outcome = (|| -> Result<View> {
            runtime_io()?;
            validate_environment()?;
            let mut context = state.runtime.context(false)?;
            request = Some(context.request_id);
            context.budget.max_bytes = OPEN_INSPECTION_BYTES;
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(remaining_millis(started, display.max_wall_millis)?);
            let view = state.runtime.inspect(&context)?;
            state.last_view = Some(view.to_json());
            runtime_io()?;
            validate_environment()?;
            remaining_millis(started, display.max_wall_millis)?;
            Ok(view)
        })();
        Ok(render(
            state,
            "fortress.open_session",
            request,
            outcome,
            false,
            false,
        ))
    })();
    result.unwrap_or_else(|cause| unbound("fortress.open_session", &cause))
}

#[tool(
    description = "Acquire an exact paused canonical observation for this original goal. Returns the observation seal required by plan. After recovery a prepared plan may reacquire only its exact original anchor. This tool never advances the clock or replaces an existing goal."
)]
pub fn fortress_observe(session_id: String) -> String {
    with_session(session_id, "fortress.observe", None, || Ok(Action::Observe))
}

#[tool(
    description = "Inspect the complete retained original goal, paired review, insertion certainty, historical achievement and last goal sample. No native calls. Current truth is explicitly as of its sample anchor; offline recovery cannot claim a fresh sample."
)]
pub fn fortress_query(session_id: String) -> String {
    with_session(session_id, "fortress.query", None, || Ok(Action::Inspect))
}

fn plan_action(raw: &str) -> Result<Action> {
    if raw.len() > 4096 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "semantic request exceeds 4096 bytes",
        ));
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid semantic request JSON"))?;
    // The typed parsers perform a second, strict deserialize, rejecting duplicate
    // fields as well as unknown values. This discriminator does not authorize.
    if value.get("kind").and_then(Value::as_str) == Some("run") {
        RunRequest::parse(raw).map(Action::PlanRun)
    } else {
        Request::parse(raw).map(Action::Plan)
    }
}
fn commit_action(scope: Option<&str>, paired_seal: &str, confirmed: bool) -> Result<Action> {
    let seal = digest(paired_seal)?;
    match scope {
        None | Some("goal") => Ok(Action::Commit { seal, confirmed }),
        Some("run") => Ok(Action::CommitRun { seal, confirmed }),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "commit scope must be goal or run",
        )),
    }
}
fn cancel_action(scope: Option<&str>) -> Result<Action> {
    match scope {
        None | Some("goal") => Ok(Action::Cancel),
        Some("run") => Ok(Action::CancelRun),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "cancel scope must be goal, run or session",
        )),
    }
}

#[tool(
    description = "Compile and durably prepare one finite original furniture-order goal. Request is closed JSON: expected_observation, recipe (wooden_bed, wooden_door, wooden_table or wooden_chair), amount 1..100, absolute deadline_tick, optional poll_interval_ticks and stable_samples. Returns the paired review seal; does not insert. Repeating the same request cannot renew its deadline or replace the goal. After insertion, kind=run with original_paired_seal, game_ticks and wall_millis separately prepares a finite receipt-linked clock run within that deadline; this never unpauses."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || plan_action(&request))
}

#[tool(
    description = "scope=goal (default) confirms the original paired review and inserts once under production authority. scope=run confirms a separate clock review in paired_seal and starts its bounded native run once under independent clock authority. Both require confirmed=true. Insertion, historical clock stopping and original goal completion are separate. Lost replies require exact reconciliation, never repeated insertion or unpause."
)]
pub fn fortress_commit(
    session_id: String,
    paired_seal: String,
    confirmed: bool,
    scope: Option<String>,
) -> String {
    with_session(session_id, "fortress.commit", None, || {
        commit_action(scope.as_deref(), &paired_seal, confirmed)
    })
}

#[tool(
    description = "Reconcile one pending clock run before attempting a fresh paused original-goal sample. An unsettled clock returns pending evidence without a paused read. Preserve the original deadline, cadence, stability and durable first achievement. Wait never starts a run, retries insertion, unpauses or starts background polling. Offline returns retained evidence only."
)]
pub fn fortress_wait(session_id: String, max_wall_millis: Option<u64>) -> String {
    with_session(session_id, "fortress.wait", max_wall_millis, || {
        Ok(Action::Wait)
    })
}

fn close(raw: String) -> String {
    let result = (|| -> Result<String> {
        let id = session_id(&raw)?;
        let mut guard = session_lock()?;
        let state = guard
            .as_ref()
            .filter(|s| s.runtime.session_id() == id)
            .ok_or_else(|| {
                error(
                    ErrorCode::SessionNotFound,
                    "semantic session absent or already closed",
                )
            })?;
        let out = packet(
            "fortress.cancel",
            Some(id),
            Some(state.runtime.anchor()),
            Some(state.runtime.mode().as_str()),
            None,
            Some(state.display_budget),
            json!({"ok":true,"scope":"session","closed":true,"effects_cancelled":false,
                "custody_deleted":false,"native_calls":0,"runtime_admitted":false,
                "automatic_redispatch":false,"retained_state_currently_verified":false}),
            state.last_view.clone(),
            state.insertion_uncertain || state.clock_uncertain,
        );
        // Release held sockets and file locks even after cancellation, revoked
        // configuration or fenced custody. No native query or mutation runs.
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause))
}

#[tool(
    description = "scope=goal (default) retires a still-prepared original insertion under current production authority; it never deletes a manager order or erases history. scope=run retires a prepared clock run or requests the native owner safety pause for its dispatched run, under current clock authority. scope=session releases resources even after cancellation or fencing without cancelling game effects. Preserve the complete original and clock recovery handoff before closing."
)]
pub fn fortress_cancel(session_id: String, scope: Option<String>) -> String {
    if scope.as_deref() == Some("session") {
        return close(session_id);
    }
    with_session(session_id, "fortress.cancel", None, || {
        cancel_action(scope.as_deref())
    })
}

#[tool(
    description = "Inspect exact retained original-plan and recovery evidence without native calls. Native insertion, historical first satisfaction, latest predicate truth and physical completion remain separate claims."
)]
pub fn fortress_explain(session_id: String) -> String {
    with_session(session_id, "fortress.explain", None, || Ok(Action::Inspect))
}

#[tool(
    description = "Check bounded local custody and report explicit unadmitted development scope, source acceptance, retained goal and recovery state. Does not contact the game or confer compatibility admission."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, || Ok(Action::Inspect))
}

#[tool(
    description = "Unavailable in this isolated semantic work-order runtime. Returns an explicit denial; no checkpoint or game command is dispatched."
)]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, || {
        Ok(Action::Unavailable)
    })
}

#[tool(
    description = "Unavailable in this isolated semantic work-order runtime. Restart recovery uses operator-provided exact original archive seals; this tool cannot restore or rewrite game state."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, || {
        Ok(Action::Unavailable)
    })
}

pub fn run_stdio() {
    let server = ServerBuilder::new("dfmcp-live-semantic-work-orders-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery)
        .tool(FortressPlan).tool(FortressCommit).tool(FortressWait)
        .tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore)
        .tool(FortressExplain).tool(FortressDoctor)
        .instructions("Explicitly unadmitted native semantic furniture-order development runtime. Open, observe, plan, review the paired seal, commit once, then wait for original-goal evidence. One retained goal. Optional separate clock authority permits explicitly reviewed bounded runs through plan(kind=run), commit(scope=run), wait and cancel(scope=run); no automatic chaining or redispatch. Keep original and clock recovery seals outside private custody. Insertion is not goal completion and operator source acceptance is not compatibility admission.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_output_and_opening_inspection_are_reserved_before_io() -> Result<()> {
        let (display, work) = opening_budgets(None, None, None, None)?;
        assert_eq!(
            display.max_bytes,
            work.max_bytes + RESPONSE_RESERVE + OPEN_INSPECTION_BYTES
        );
        assert_eq!(display.max_game_ticks, 0);
        assert_eq!(work.max_actions, 1);
        assert!(opening_budgets(None, None, Some(MAX_OUTPUT_TOKENS - 1), None).is_err());
        assert!(
            opening_budgets(
                None,
                Some(RESPONSE_RESERVE + OPEN_INSPECTION_BYTES),
                None,
                None
            )
            .is_err()
        );
        assert!(opening_budgets(Some(0), None, None, None).is_err());
        assert!(opening_budgets(Some(MAX_WALL_MILLIS + 1), None, None, None).is_err());
        assert!(opening_budgets(None, Some(MAX_BYTES + 1), None, None).is_err());
        Ok(())
    }

    #[test]
    fn semantic_namespace_rejects_other_runtime_and_noncanonical_ids() -> Result<()> {
        let id = SessionId::new((1u128 << 127) | FAMILY | 1);
        assert_eq!(session_id(&id.to_string())?, id);
        let other = SessionId::new((1u128 << 127) | (10u128 << 57) | 1);
        assert!(session_id(&other.to_string()).is_err());
        assert!(session_id(&SessionId::NIL.to_string()).is_err());
        assert!(session_id(&id.to_string().to_uppercase()).is_err());
        assert!(session_id("1").is_err());
        Ok(())
    }

    #[test]
    fn failed_commit_never_claims_no_dispatch_or_safe_retry() {
        let value = failure(&error(ErrorCode::AdapterFailure, "reply lost"), true, false);
        assert!(value["game_mutation_dispatched"].is_null());
        assert_eq!(value["automatic_redispatch"], false);
        assert_eq!(value["retained_state_currently_verified"], false);
        assert_eq!(value["current_goal_available"], false);
    }
    #[test]
    fn closed_run_requests_cannot_choose_targets_predicates_or_unbounded_time() -> Result<()> {
        let seal = "a".repeat(64);
        let request = json!({"kind":"run","original_paired_seal":seal,
            "game_ticks":1200,"wall_millis":60000})
        .to_string();
        assert!(matches!(plan_action(&request)?, Action::PlanRun(_)));
        for field in [
            "native_order_id",
            "idempotency_key",
            "predicate",
            "endpoint",
            "deadline_tick",
        ] {
            let mut value: Value = serde_json::from_str(&request)
                .map_err(|_| error(ErrorCode::InvalidRequest, "fixture"))?;
            value[field] = json!(1);
            assert!(plan_action(&value.to_string()).is_err(), "{field}");
        }
        for ticks in [0, 1201] {
            let value = json!({"kind":"run","original_paired_seal":seal,"game_ticks":ticks,"wall_millis":1000});
            assert!(plan_action(&value.to_string()).is_err());
        }
        let duplicate = format!(
            "{{\"kind\":\"run\",\"kind\":\"run\",\"original_paired_seal\":\"{seal}\",\"game_ticks\":10,\"wall_millis\":1000}}"
        );
        assert!(plan_action(&duplicate).is_err());
        assert!(plan_action(&" ".repeat(4097)).is_err());
        let goal = json!({"expected_observation":seal,"recipe":"wooden_bed", "amount":5,
            "deadline_tick":1000})
        .to_string();
        assert!(matches!(plan_action(&goal)?, Action::Plan(_)));
        Ok(())
    }

    #[test]
    fn explicit_clock_scope_never_dispatches_an_insertion() -> Result<()> {
        let seal = "b".repeat(64);
        let run = commit_action(Some("run"), &seal, true)?;
        assert!(!run.may_insert());
        assert!(run.may_control_clock());
        assert!(matches!(
            run,
            Action::CommitRun {
                confirmed: true,
                ..
            }
        ));
        let insertion = commit_action(None, &seal, true)?;
        assert!(insertion.may_insert());
        assert!(!insertion.may_control_clock());
        assert!(matches!(
            commit_action(Some("goal"), &seal, false)?,
            Action::Commit {
                confirmed: false,
                ..
            }
        ));
        assert!(matches!(cancel_action(Some("run"))?, Action::CancelRun));
        assert!(matches!(cancel_action(None)?, Action::Cancel));
        for unknown in ["clock", "all", "session", ""] {
            assert!(commit_action(Some(unknown), &seal, true).is_err());
            assert!(cancel_action(Some(unknown)).is_err());
        }
        assert!(commit_action(Some("run"), "bad", true).is_err());
        Ok(())
    }

    #[test]
    fn clock_budget_is_explicit_and_clock_failure_retains_uncertainty() -> Result<()> {
        assert_eq!(opening_budgets(None, None, None, None)?.0.max_game_ticks, 0);
        let (display, work) = opening_budgets(None, None, None, Some(MAX_GAME_TICKS))?;
        assert_eq!(display.max_game_ticks, MAX_GAME_TICKS);
        assert_eq!(work.max_game_ticks, MAX_GAME_TICKS);
        assert!(opening_budgets(None, None, None, Some(MAX_GAME_TICKS + 1)).is_err());
        assert_eq!(OPEN_INSPECTION_BYTES, 32 * 1024 * 1024);
        let value = failure(
            &error(ErrorCode::AdapterFailure, "unpause reply lost"),
            false,
            true,
        );
        assert!(value["game_mutation_dispatched"].is_null());
        assert_eq!(value["clock_effect_may_have_started"], true);
        assert_eq!(value["insertion_may_have_started"], false);
        assert_eq!(value["automatic_redispatch"], false);
        assert_eq!(value["current_goal_available"], false);
        Ok(())
    }
}
