#![forbid(unsafe_code)]
//! Explicitly unadmitted, one-goal native labor MCP composition root.
//! All planning, effect custody and completion decisions remain in the runtime
//! and adapter. MCP arguments cannot choose sources, credentials or paths.
use crate::semantic_workforce_runtime::{
    Config, DEFAULT_BYTES, MAX_BYTES, MAX_OUTPUT_TOKENS, MAX_WALL_MILLIS, Mode, NativeRuntime,
    Request, View, digest,
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
    Commit {
        seal: Digest32,
        confirmed: bool,
    },
    Wait,
    Cancel,
    Unavailable,
}
impl Action {
    fn may_assign(&self) -> bool {
        matches!(self, Self::Commit { .. })
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
            Self::Commit { seal, confirmed } => runtime.commit(seal, confirmed, c),
            Self::Wait => runtime.wait(c),
            Self::Cancel => runtime.cancel(c),
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

fn failure(cause: &DfmcpError, attempted_assignment: bool) -> Value {
    json!({
        "ok":false,
        "error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,
        "game_mutation_dispatched":if attempted_assignment { Value::Null } else { json!(false) },
        "retained_state_currently_verified":false,
        "current_goal_available":false,
        "recovery_guidance":"Inspect retained custody and reconcile the original assignment. Never replace the original goal or repeat assignment to resolve uncertainty."
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
        failure(cause, false),
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
) -> Result<(WorkBudget, WorkBudget)> {
    let display = WorkBudget {
        max_wall_millis: wall.unwrap_or(5000),
        max_bytes: bytes.unwrap_or(DEFAULT_BYTES),
        max_output_tokens: output.unwrap_or(MAX_OUTPUT_TOKENS),
        max_entities: 8192,
        max_actions: 1,
        max_game_ticks: 0,
    };
    display.validate()?;
    if display.max_wall_millis > MAX_WALL_MILLIS
        || display.max_bytes > MAX_BYTES
        || display.max_output_tokens > MAX_OUTPUT_TOKENS
        || u64::from(display.max_output_tokens) * 4 < RESPONSE_RESERVE
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "semantic limits require at most 60000ms and 32GiB logical byte-work (2GiB default), with 32768 output proxy units reserved for a complete response",
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
) -> String {
    let result = match outcome {
        Ok(view) => {
            let mut value = view.to_json();
            state.assignment_uncertain = value["effect_uncertain"] == true;
            value["session_id"] = json!(state.runtime.session_id().to_string());
            value["retained_state_currently_verified"] = json!(true);
            value["development_labor_enabled"] = json!(state.runtime.labor_control_enabled());
            state.last_view = Some(value.clone());
            value
        }
        Err(cause) => {
            state.assignment_uncertain |= attempted_assignment;
            state.last_view = Some(retain_failure_reference(
                state.last_view.take(),
                state.runtime.opening_recovery_reference(),
            ));
            failure(&cause, attempted_assignment)
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
        state.assignment_uncertain,
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
        ))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

#[tool(
    description = "Open one unadmitted semantic labor goal. Default offline reads existing custody; reconcile can acquire evidence without labor authority; control additionally requires operator labor enablement. Byte-work defaults to 2GiB; max_bytes can explicitly request up to 32GiB for replaying long retained histories, without increasing native cardinality or the 60s wall ceiling. Sources and private custody are operator configuration. No game time is advanced."
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
        validate_environment()?;
        let mode = Mode::parse(mode.as_deref().unwrap_or("offline"))?;
        let (display, mut work) = opening_budgets(max_wall_millis, max_bytes, max_output_tokens)?;
        let config = Config::from_environment()?;
        let mut guard = session_lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the retained semantic session before opening another",
            ));
        }
        let sequence = NEXT
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
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
    Request::parse(raw).map(Action::Plan)
}

fn commit_action(paired_seal: &str, confirmed: bool) -> Result<Action> {
    Ok(Action::Commit {
        seal: digest(paired_seal)?,
        confirmed,
    })
}

fn cancel_action(scope: Option<&str>) -> Result<Action> {
    match scope {
        None | Some("goal") => Ok(Action::Cancel),
        _ => Err(error(
            ErrorCode::InvalidRequest,
            "cancel scope must be goal or session",
        )),
    }
}

#[tool(
    description = "Compile and durably prepare one original labor goal. Request is closed JSON: expected_observation, units, labor, enabled, absolute deadline_tick, optional poll_interval_ticks and stable_samples. Selected canonical citizen identities and one exact labor are retained. Returns the paired review seal without assigning labor. Repeating the request cannot replace its goal or renew its deadline."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || plan_action(&request))
}

#[tool(
    description = "Confirm and apply the exact paired semantic/native labor review once. Requires paired_seal and confirmed=true plus current operator labor authority. Historical assignment verification is separate from current labor-goal proof. A lost reply requires wait/reconciliation, never redispatch or a new goal."
)]
pub fn fortress_commit(session_id: String, paired_seal: String, confirmed: bool) -> String {
    with_session(session_id, "fortress.commit", None, || {
        commit_action(&paired_seal, confirmed)
    })
}

#[tool(
    description = "Perform one bounded receipt reconciliation and original-goal sample. Preserves the original absolute deadline, cadence and stability requirement; retains durable first achievement separately from current truth and exact native action verification. No assignment retry, background polling or time advancement. Offline returns retained evidence only."
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
            state.assignment_uncertain,
        );
        // Release held sockets and file locks even after cancellation, revoked
        // configuration or fenced custody. No native query or mutation runs.
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause))
}

#[tool(
    description = "scope=goal (default) retires a still-prepared original assignment under current labor authority; it never undoes applied membership or erases history. scope=session releases owned resources even after cancellation or fencing, without cancelling game effects. Preserve the recovery handoff before closing."
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
        .instructions("Explicitly unadmitted native semantic labor development runtime. Open, observe, plan, review the paired seal, commit once, then wait for original-goal evidence. One retained goal; no automatic clock advancement or redispatch. Keep the complete recovery handoff, including the original citizen identity digest, outside private custody. Historical assignment is not current goal completion and operator source acceptance is not compatibility admission.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_output_and_opening_inspection_are_reserved_before_io() -> Result<()> {
        let (display, work) = opening_budgets(None, None, None)?;
        assert_eq!(
            display.max_bytes,
            work.max_bytes + RESPONSE_RESERVE + OPEN_INSPECTION_BYTES
        );
        assert_eq!(display.max_game_ticks, 0);
        assert_eq!(display.max_bytes, DEFAULT_BYTES);
        assert_eq!(work.max_actions, 1);
        assert!(opening_budgets(None, None, Some(MAX_OUTPUT_TOKENS - 1)).is_err());
        assert!(
            opening_budgets(None, Some(RESPONSE_RESERVE + OPEN_INSPECTION_BYTES), None).is_err()
        );
        assert!(opening_budgets(Some(0), None, None).is_err());
        assert!(opening_budgets(Some(MAX_WALL_MILLIS + 1), None, None).is_err());
        assert!(opening_budgets(None, Some(MAX_BYTES + 1), None).is_err());
        let (recovery_display, recovery_work) =
            opening_budgets(None, Some(8 * 1024 * 1024 * 1024), None)?;
        assert_eq!(recovery_display.max_bytes, 8 * 1024 * 1024 * 1024);
        assert_eq!(recovery_work.max_game_ticks, 0);
        assert_eq!(recovery_work.max_entities, 8192);
        assert_eq!(recovery_work.max_wall_millis, display.max_wall_millis);
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
        let value = failure(&error(ErrorCode::AdapterFailure, "reply lost"), true);
        assert!(value["game_mutation_dispatched"].is_null());
        assert_eq!(value["automatic_redispatch"], false);
        assert_eq!(value["retained_state_currently_verified"], false);
        assert_eq!(value["current_goal_available"], false);
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
        let action = commit_action(&"b".repeat(64), false)?;
        assert!(action.may_assign());
        assert!(matches!(
            action,
            Action::Commit {
                confirmed: false,
                ..
            }
        ));
        assert!(commit_action("bad", true).is_err());
        assert!(matches!(cancel_action(None)?, Action::Cancel));
        assert!(matches!(cancel_action(Some("goal"))?, Action::Cancel));
        for scope in ["run", "all", "session", ""] {
            assert!(cancel_action(Some(scope)).is_err());
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
}
