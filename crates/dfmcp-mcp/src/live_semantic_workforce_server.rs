#![forbid(unsafe_code)]
//! Explicitly unadmitted, one-goal native labor MCP composition root.
//! All planning, effect custody and completion decisions remain in the runtime
//! and adapter. MCP arguments cannot choose sources, credentials or paths.
use crate::semantic_workforce_runtime::clock::RunRequest;
use crate::semantic_workforce_runtime::{
    Config, DEFAULT_BYTES, MAX_BYTES, MAX_GAME_TICKS, MAX_OUTPUT_TOKENS, MAX_WALL_MILLIS, Mode,
    NativeRuntime, Request, View, digest,
};
use dfmcp_core::{
    DfmcpError, Digest32, ErrorCode, OperationContext, RequestId, Result, SessionId, WorkBudget,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Instant;

#[path = "semantic_workforce_presentation.rs"]
mod presentation;
use presentation::{RESPONSE_RESERVE, packet};

#[cfg(test)]
pub(crate) fn render_fixture_packet(operation: &str, view: &View) -> String {
    let value = view.to_json();
    packet(
        operation,
        None,
        Some(view.anchor),
        Some("control"),
        None,
        None,
        value.clone(),
        Some(value),
        false,
    )
}

const FAMILY: u128 = 31u128 << 57;
const SEQUENCE_LIMIT: u64 = 1u64 << 57;
const OPEN_INSPECTION_BYTES: u64 = 64 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State>> = Mutex::new(None);

struct State {
    runtime: NativeRuntime,
    display_budget: WorkBudget,
    // A failed call must not spend the operation budget again on a second
    // inspection. Keep the last complete view only as an explicitly stale
    // recovery reference until the caller asks for a new bounded inspection.
    last_view: Option<Value>,
    assignment_uncertain: bool,
    clock_uncertain: bool,
}

enum Action {
    Observe,
    Inspect,
    Citizens {
        seal: Digest32,
        offset: usize,
        limit: usize,
    },
    Plan(Request),
    PlanRun(RunRequest),
    Commit {
        seal: Digest32,
        confirmed: bool,
    },
    CommitRun {
        seal: Digest32,
        confirmed: bool,
    },
    Wait,
    Cancel,
    CancelRun,
    Unavailable,
}
impl Action {
    fn may_assign(&self) -> bool {
        matches!(self, Self::Commit { .. })
    }
    fn may_control_clock(&self) -> bool {
        matches!(self, Self::CommitRun { .. } | Self::CancelRun)
    }
    fn perform(self, runtime: &mut NativeRuntime, c: &OperationContext) -> Result<View> {
        match self {
            Self::Observe => runtime.observe(c),
            Self::Inspect => runtime.inspect(c),
            Self::Citizens {
                seal,
                offset,
                limit,
            } => runtime.citizens_page(seal, offset, limit, c),
            Self::Plan(request) => runtime.plan(request, c),
            Self::PlanRun(request) => runtime.prepare_run(request, c),
            Self::Commit { seal, confirmed } => runtime.commit(seal, confirmed, c),
            Self::CommitRun { seal, confirmed } => runtime.commit_run(seal, confirmed, c),
            Self::Wait => runtime.wait(c),
            Self::Cancel => runtime.cancel(c),
            Self::CancelRun => runtime.cancel_run(c),
            Self::Unavailable => Err(error(
                ErrorCode::CapabilityDenied,
                "this semantic workforce runtime does not implement checkpoint or restore",
            )),
        }
    }
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn failure(cause: &DfmcpError, attempted_assignment: bool, attempted_clock: bool) -> Value {
    json!({
        "ok":false,
        "error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,
        "game_mutation_dispatched":if attempted_assignment || attempted_clock { Value::Null } else { json!(false) },
        "assignment_may_have_started":attempted_assignment,
        "clock_effect_may_have_started":attempted_clock,
        "retained_state_currently_verified":false,
        "current_goal_available":false,
        "recovery_guidance":"Inspect retained custody and reconcile the original assignment and any clock run. Never replace the original goal, repeat assignment or repeat unpause to resolve uncertainty."
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
            "unadmitted semantic workforce runtime refuses production admission provenance",
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
            "semantic workforce request cancelled",
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
            "invalid semantic workforce session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid semantic workforce session ID",
        )
    })?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 31
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
        max_bytes: bytes.unwrap_or(DEFAULT_BYTES),
        max_output_tokens: output.unwrap_or(MAX_OUTPUT_TOKENS),
        max_entities: 8192,
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
            "semantic limits require at most 1200 game ticks, 60000ms and 32GiB logical byte-work (2GiB default), with 32768 output proxy units reserved for a complete response",
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

/// Recoveries can acquire new durable identities before the overall call fails.
/// Keep those pure in-memory references even if cancellation or revoked authority
/// makes another inspection impossible. None of this is current verified state.
fn retain_failure_reference(last: Option<Value>, reference: Value) -> Value {
    let mut retained = last.filter(Value::is_object).unwrap_or_else(|| json!({}));
    for key in [
        "mode",
        "anchor",
        "journal_id",
        "identity_digest",
        "review",
        "recovery",
        "clock",
    ] {
        if let Some(value) = reference.get(key) {
            retained[key] = value.clone();
        }
    }
    if retained["original_plan"]["plan_digest"] != reference["original_plan"]["plan_digest"]
        || !retained["original_plan"].is_object()
    {
        retained["original_plan"] = reference["original_plan"].clone();
    }
    retained["partial_goal_publication"] =
        json!(reference["identity_digest"].is_string() && !reference["recovery"].is_object());
    retained["ok"] = json!(false);
    retained["retained_state_currently_verified"] = json!(false);
    retained["development_labor_enabled"] = json!(false);
    retained["development_clock_enabled"] = json!(false);
    retained["current_goal"] = Value::Null;
    retained["current_goal_available"] = json!(false);
    retained
}

fn render(
    state: &mut State,
    operation: &str,
    request: Option<RequestId>,
    outcome: Result<View>,
    attempted_assignment: bool,
    attempted_clock: bool,
) -> String {
    let result = match outcome {
        Ok(view) => {
            let mut value = view.to_json();
            state.assignment_uncertain = value["effect_uncertain"] == true;
            state.clock_uncertain =
                value["clock"]["unresolved"] == true || value["clock"]["may_be_running"] == true;
            value["session_id"] = json!(state.runtime.session_id().to_string());
            value["retained_state_currently_verified"] = json!(true);
            value["development_labor_enabled"] = json!(state.runtime.labor_control_enabled());
            value["development_clock_enabled"] = json!(state.runtime.clock_control_enabled());
            state.last_view = Some(value.clone());
            value
        }
        Err(cause) => {
            state.assignment_uncertain |= attempted_assignment;
            state.clock_uncertain |= attempted_clock;
            state.last_view = Some(retain_failure_reference(
                state.last_view.take(),
                state.runtime.opening_recovery_reference(),
            ));
            if let Some(reference) = state.last_view.as_mut() {
                reference["clock_effect_may_have_started"] = json!(state.clock_uncertain);
            }
            failure(&cause, attempted_assignment, attempted_clock)
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
        state.assignment_uncertain || state.clock_uncertain,
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
        let mut attempted_assignment = false;
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
            attempted_assignment = action.may_assign();
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
            attempted_assignment,
            attempted_clock,
        ))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

#[tool(
    description = "Open one unadmitted semantic labor goal. Default offline reads existing custody; reconcile can acquire evidence without control; labor and optional clock control require separate operator enablement. Byte-work defaults to 2GiB; max_bytes can request up to 32GiB for retained-history replay within unchanged native and 60s wall limits. Optional max_game_ticks defaults to zero and is capped at 1200. Sources and private custody are operator configuration. Opening never unpauses."
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
        let opening_reference = runtime.opening_recovery_reference();
        // Retain custody even if the separately reserved opening inspection
        // fails, so the returned handle can inspect or explicitly close it.
        *guard = Some(State {
            runtime,
            display_budget: display,
            last_view: Some(opening_reference),
            assignment_uncertain: false,
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
    description = "Inspect the complete retained original goal, paired review, assignment certainty, historical achievement and last goal sample. Optional closed query JSON {mode:citizens,expected_observation,offset,limit} pages 1..32 canonical citizen identities from that exact retained observation. No native calls. Current truth remains as of its sample anchor."
)]
pub fn fortress_query(session_id: String, query: Option<String>) -> String {
    with_session(session_id, "fortress.query", None, || {
        query_action(query.as_deref())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CitizenQuery {
    mode: CitizenQueryMode,
    expected_observation: String,
    offset: usize,
    limit: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum CitizenQueryMode {
    Citizens,
}

fn query_action(raw: Option<&str>) -> Result<Action> {
    let Some(raw) = raw else {
        return Ok(Action::Inspect);
    };
    if raw.len() > 2048 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "citizen query exceeds 2048 bytes",
        ));
    }
    let query: CitizenQuery = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed citizen query"))?;
    if query.offset > 4096 || !(1..=32).contains(&query.limit) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "citizen offset must be at most 4096 and limit 1..32",
        ));
    }
    match query.mode {
        CitizenQueryMode::Citizens => Ok(Action::Citizens {
            seal: digest(&query.expected_observation)?,
            offset: query.offset,
            limit: query.limit,
        }),
    }
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
    // Typed parsing repeats deserialization so duplicate fields are rejected
    // instead of inheriting this discriminator's last-value behavior.
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
    description = "Compile and durably prepare one original labor goal. Closed request JSON: expected_observation, units, labor, enabled, absolute deadline_tick, optional poll_interval_ticks and stable_samples. Returns the paired review seal without assigning labor or renewing the goal. After verified assignment, kind=run with original_paired_seal, game_ticks and wall_millis separately prepares finite game-time exposure for original labor stability within its unchanged deadline. Preparation never unpauses."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || plan_action(&request))
}

#[tool(
    description = "scope=goal (default) confirms the original paired labor review once under current labor authority. scope=run confirms a separate clock review in paired_seal under current clock authority. Both require confirmed=true and exact original custody. Historical assignment and clock receipts are separate from original-goal proof. Lost replies require exact reconciliation, never another assignment or unpause attempt."
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
    description = "Reconcile retained clock uncertainty before attempting one fresh paused original labor-goal sample. Preserve the original absolute deadline, cadence, stability and durable first achievement. A running or unresolved clock returns pending evidence without a paused goal read. Wait never starts a run, retries assignment or unpause, or starts background polling. Offline returns retained evidence only."
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
            state.assignment_uncertain || state.clock_uncertain,
        );
        // Release held sockets and file locks even after cancellation, revoked
        // configuration or fenced custody. No native query or mutation runs.
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause))
}

#[tool(
    description = "scope=goal (default) retires a still-prepared original assignment under labor authority without undoing applied membership or erasing history. scope=run retires undispatched clock intent or requests its native owner's safety pause under separate clock authority. scope=session releases resources even after cancellation or fencing without cancelling game effects. Preserve all original and clock recovery identities before closing."
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
    description = "Inspect exact retained original-plan and recovery evidence without native calls. Historical native assignment, first satisfaction and latest original labor-goal truth remain separate claims."
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
    description = "Unavailable in this isolated semantic workforce runtime. Returns an explicit denial; no checkpoint or game command is dispatched."
)]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, || {
        Ok(Action::Unavailable)
    })
}

#[tool(
    description = "Unavailable in this isolated semantic workforce runtime. Restart recovery uses operator-provided exact original archive seals; this tool cannot restore or rewrite game state."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, || {
        Ok(Action::Unavailable)
    })
}

pub fn run_stdio() {
    let server = ServerBuilder::new("dfmcp-live-semantic-workforce-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery)
        .tool(FortressPlan).tool(FortressCommit).tool(FortressWait)
        .tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore)
        .tool(FortressExplain).tool(FortressDoctor)
        .instructions("Explicitly unadmitted native semantic labor development runtime. Open, observe, plan, review the paired seal, commit once, then wait for original-goal evidence. Optional bounded runs require negotiated ticks, separate clock authority, plan(kind=run), and confirmation of the distinct clock review with scope=run. Wait never starts a run. One retained goal; no automatic chaining or redispatch. Keep the complete recovery handoff, including original citizen and clock journal identities, outside private custody. Historical assignment or clock receipts are not current goal completion; the clock source does not identify a named fortress. Operator source acceptance is not compatibility admission.")
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
        assert_eq!(display.max_bytes, DEFAULT_BYTES);
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
        let (recovery_display, recovery_work) =
            opening_budgets(None, Some(8 * 1024 * 1024 * 1024), None, None)?;
        assert_eq!(recovery_display.max_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(recovery_work.max_game_ticks, 0);
        assert_eq!(recovery_work.max_entities, 8192);
        assert_eq!(recovery_work.max_wall_millis, display.max_wall_millis);
        let (clock_display, clock_work) = opening_budgets(None, None, None, Some(MAX_GAME_TICKS))?;
        assert_eq!(clock_display.max_game_ticks, MAX_GAME_TICKS);
        assert_eq!(clock_work.max_game_ticks, MAX_GAME_TICKS);
        assert_eq!(clock_work.max_bytes, work.max_bytes);
        assert!(opening_budgets(None, None, None, Some(MAX_GAME_TICKS + 1)).is_err());
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
        assert_eq!(value["assignment_may_have_started"], true);
        assert_eq!(value["clock_effect_may_have_started"], false);
        let value = failure(
            &error(ErrorCode::AdapterFailure, "clock reply lost"),
            false,
            true,
        );
        assert!(value["game_mutation_dispatched"].is_null());
        assert_eq!(value["assignment_may_have_started"], false);
        assert_eq!(value["clock_effect_may_have_started"], true);
        assert_eq!(value["automatic_redispatch"], false);
    }

    #[test]
    fn citizen_pages_are_closed_bounded_and_observation_bound() -> Result<()> {
        assert!(matches!(query_action(None)?, Action::Inspect));
        let query =
            json!({"mode":"citizens","expected_observation":"a".repeat(64),"offset":32,"limit":32});
        assert!(matches!(
            query_action(Some(&query.to_string()))?,
            Action::Citizens {
                offset: 32,
                limit: 32,
                ..
            }
        ));
        for field in ["native_unit_ids", "endpoint", "token", "path", "cursor"] {
            let mut value = query.clone();
            value[field] = json!(1);
            assert!(query_action(Some(&value.to_string())).is_err(), "{field}");
        }
        for (field, value) in [
            ("limit", json!(0)),
            ("limit", json!(33)),
            ("offset", json!(-1)),
            ("offset", json!(4097)),
            ("offset", json!(true)),
            ("mode", json!("all")),
            ("expected_observation", json!("bad")),
        ] {
            let mut invalid = query.clone();
            invalid[field] = value;
            assert!(query_action(Some(&invalid.to_string())).is_err());
        }
        let duplicate = format!(
            "{{\"mode\":\"citizens\",\"mode\":\"citizens\",\"expected_observation\":\"{}\",\"offset\":0,\"limit\":32}}",
            "a".repeat(64)
        );
        assert!(query_action(Some(&duplicate)).is_err());
        assert!(query_action(Some(&" ".repeat(2049))).is_err());
        Ok(())
    }

    #[test]
    fn client_cannot_substitute_native_action_keys_details_or_sources() -> Result<()> {
        let request = json!({"expected_observation":"a".repeat(64),"units":["12","13"],
            "labor":"MINING","enabled":true,"deadline_tick":1000});
        assert!(matches!(
            plan_action(&request.to_string())?,
            Action::Plan(_)
        ));
        for field in [
            "detail_index",
            "native_unit_ids",
            "idempotency_key",
            "endpoint",
            "token",
            "kind",
            "predicate",
        ] {
            let mut value = request.clone();
            value[field] = json!(1);
            assert!(plan_action(&value.to_string()).is_err(), "{field}");
        }
        let action = commit_action(None, &"b".repeat(64), false)?;
        assert!(action.may_assign());
        assert!(!action.may_control_clock());
        assert!(matches!(
            action,
            Action::Commit {
                confirmed: false,
                ..
            }
        ));
        assert!(commit_action(None, "bad", true).is_err());
        assert!(matches!(cancel_action(None)?, Action::Cancel));
        assert!(matches!(cancel_action(Some("goal"))?, Action::Cancel));
        for scope in ["all", "session", ""] {
            assert!(cancel_action(Some(scope)).is_err());
            assert!(commit_action(Some(scope), &"b".repeat(64), true).is_err());
        }
        Ok(())
    }

    #[test]
    fn clock_intake_is_closed_original_bound_and_separately_confirmed() -> Result<()> {
        let request = json!({"kind":"run","original_paired_seal":"a".repeat(64),
            "game_ticks":10,"wall_millis":1000});
        let action = plan_action(&request.to_string())?;
        assert!(matches!(action, Action::PlanRun(_)));
        assert!(!action.may_assign());
        assert!(!action.may_control_clock());
        for field in [
            "idempotency_key",
            "endpoint",
            "token",
            "path",
            "source_generation",
            "native_unit_ids",
            "stop_predicate",
            "deadline_tick",
            "authority",
        ] {
            let mut value = request.clone();
            value[field] = json!(1);
            assert!(plan_action(&value.to_string()).is_err(), "{field}");
        }
        for (field, value) in [
            ("kind", json!("clock")),
            ("kind", json!(true)),
            ("game_ticks", json!(0)),
            ("game_ticks", json!(1201)),
            ("game_ticks", json!(true)),
            ("game_ticks", json!(-1)),
            ("wall_millis", json!(0)),
            ("wall_millis", json!(60001)),
            ("original_paired_seal", json!("bad")),
        ] {
            let mut invalid = request.clone();
            invalid[field] = value;
            assert!(plan_action(&invalid.to_string()).is_err(), "{field}");
        }
        let duplicate = format!(
            "{{\"kind\":\"run\",\"kind\":\"run\",\"original_paired_seal\":\"{}\",\"game_ticks\":10,\"wall_millis\":1000}}",
            "a".repeat(64)
        );
        assert!(plan_action(&duplicate).is_err());
        assert!(plan_action(&format!("{}{}", request, " ".repeat(2049))).is_err());
        let commit = commit_action(Some("run"), &"b".repeat(64), false)?;
        assert!(matches!(
            commit,
            Action::CommitRun {
                confirmed: false,
                ..
            }
        ));
        assert!(!commit.may_assign());
        assert!(commit.may_control_clock());
        let cancel = cancel_action(Some("run"))?;
        assert!(matches!(cancel, Action::CancelRun));
        assert!(!cancel.may_assign());
        assert!(cancel.may_control_clock());
        for action in [
            Action::Observe,
            Action::Inspect,
            Action::Wait,
            Action::Cancel,
            Action::Unavailable,
        ] {
            assert!(!action.may_assign());
            assert!(!action.may_control_clock());
        }
        Ok(())
    }

    #[test]
    fn failed_operation_retains_new_original_seals_without_current_proof() {
        let reference = json!({"ok":false,"mode":"control","journal_id":"a".repeat(64),
            "identity_digest":"b".repeat(64),"anchor":{"tick":40},
            "original_plan":{"plan_digest":"c".repeat(64),"anchor":{"tick":40}},
            "review":{"paired_seal":"d".repeat(64)},
            "recovery":{"plan_digest":"c".repeat(64),"object_digest":"e".repeat(64),
                "identity_digest":"b".repeat(64),"journal_id":"a".repeat(64),"history_present":false}});
        let old = json!({"ok":true,"original_plan":null,"recovery":null,
            "current_goal":{"original_goal_proven":true},"development_labor_enabled":true});
        let retained = retain_failure_reference(Some(old), reference.clone());
        assert_eq!(retained["recovery"], reference["recovery"]);
        assert_eq!(retained["review"], reference["review"]);
        assert_eq!(retained["journal_id"], reference["journal_id"]);
        assert_eq!(retained["identity_digest"], reference["identity_digest"]);
        assert_eq!(retained["ok"], false);
        assert_eq!(retained["retained_state_currently_verified"], false);
        assert_eq!(retained["development_labor_enabled"], false);
        assert!(retained["current_goal"].is_null());
        assert_eq!(retained["current_goal_available"], false);
    }

    #[test]
    fn failed_first_poll_preserves_new_history_identity_and_partial_publication() {
        let old = json!({"ok":true,"original_plan":{"plan_digest":"c".repeat(64),"summary":"retained goal"},
            "native_state":"terminal","recovery":{"history_present":false},
            "current_goal":{"original_goal_proven":true}});
        let reference = json!({"journal_id":"a".repeat(64),"identity_digest":"b".repeat(64),
            "original_plan":{"plan_digest":"c".repeat(64)},"recovery":{"history_present":true}});
        let retained = retain_failure_reference(Some(old), reference);
        assert_eq!(retained["recovery"]["history_present"], true);
        assert_eq!(retained["original_plan"]["summary"], "retained goal");
        assert_eq!(retained["native_state"], "terminal");
        assert!(retained["current_goal"].is_null());
        let partial = retain_failure_reference(
            None,
            json!({"journal_id":"a".repeat(64),
            "identity_digest":"b".repeat(64),"original_plan":null,"recovery":null}),
        );
        assert_eq!(partial["partial_goal_publication"], true);
        assert!(partial["recovery"].is_null());
        assert_eq!(partial["identity_digest"], "b".repeat(64));
        assert_eq!(partial["retained_state_currently_verified"], false);
    }

    #[test]
    fn failed_clock_call_keeps_new_custody_without_cached_control_or_goal_truth() {
        let old = json!({"ok":true,"development_labor_enabled":true,
            "development_clock_enabled":true,"clock":{"journal_id":"a".repeat(64),"active_run":null},
            "recovery":{"history_present":false},"current_goal":{"original_goal_proven":true}});
        let reference = json!({"journal_id":"b".repeat(64),"identity_digest":"c".repeat(64),
            "original_plan":{"plan_digest":"d".repeat(64)},
            "recovery":{"history_present":true,"clock_journal_id":"a".repeat(64)},
            "clock":{"journal_id":"a".repeat(64),"head":"e".repeat(64),
                "active_run":{"key":"retained-run","state":"dispatch_started",
                    "review_seal":"f".repeat(64)},"unresolved":true}});
        let retained = retain_failure_reference(Some(old), reference.clone());
        assert_eq!(retained["clock"], reference["clock"]);
        assert_eq!(retained["recovery"], reference["recovery"]);
        assert_eq!(retained["development_clock_enabled"], false);
        assert_eq!(retained["development_labor_enabled"], false);
        assert_eq!(retained["retained_state_currently_verified"], false);
        assert!(retained["current_goal"].is_null());
    }
}
