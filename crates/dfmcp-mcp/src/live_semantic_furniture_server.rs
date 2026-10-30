#![forbid(unsafe_code)]
//! Explicitly unadmitted original furniture construction MCP composition root.
//! All planning, effect custody and completion decisions remain in the runtime
//! and adapter. MCP arguments cannot choose sources, credentials or paths.
use crate::semantic_furniture_runtime::clock::RunRequest;
use crate::semantic_furniture_runtime::{
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

#[path = "semantic_furniture_presentation.rs"]
mod presentation;
use presentation::{RESPONSE_RESERVE, packet};

const FAMILY: u128 = 29u128 << 57;
const SEQUENCE_LIMIT: u64 = 1u64 << 57;
const OPEN_INSPECTION_BYTES: u64 = 256 * 1024 * 1024;
// Prepay native and optional clock custody, ownership and inventory before opening files.
const OPEN_RUNTIME_BYTES: u64 = 576 * 1024 * 1024;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State>> = Mutex::new(None);

struct State {
    runtime: NativeRuntime,
    display_budget: WorkBudget,
    // A failed call must not spend the operation budget again on a second
    // inspection. Keep the last complete view only as an explicitly stale
    // recovery reference until the caller asks for a new bounded inspection.
    last_view: Option<Value>,
    placement_uncertain: bool,
    clock_uncertain: bool,
}

enum Action {
    Observe,
    Inspect,
    Items {
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
    fn may_place(&self) -> bool {
        matches!(
            self,
            Self::Commit {
                confirmed: true,
                ..
            }
        )
    }
    fn may_control_clock(&self) -> bool {
        matches!(
            self,
            Self::CommitRun {
                confirmed: true,
                ..
            } | Self::CancelRun
        )
    }
    fn perform(self, runtime: &mut NativeRuntime, c: &OperationContext) -> Result<View> {
        match self {
            Self::Observe => runtime.observe(c),
            Self::Inspect => runtime.inspect(c),
            Self::Items {
                seal,
                offset,
                limit,
            } => runtime.items_page(seal, offset, limit, c),
            Self::Plan(request) => runtime.plan(request, c),
            Self::PlanRun(request) => runtime.prepare_run(request, c),
            Self::Commit { seal, confirmed } => runtime.commit(seal, confirmed, c),
            Self::CommitRun { seal, confirmed } => runtime.commit_run(seal, confirmed, c),
            Self::Wait => runtime.wait(c),
            Self::Cancel => runtime.cancel(c),
            Self::CancelRun => runtime.cancel_run(c),
            Self::Unavailable => Err(error(
                ErrorCode::CapabilityDenied,
                "this original furniture profile has no checkpoint or restore provider",
            )),
        }
    }
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn failure(cause: &DfmcpError, attempted_placement: bool, attempted_clock: bool) -> Value {
    json!({"ok":false,
        "error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,
        "game_mutation_dispatched":if attempted_placement || attempted_clock {Value::Null} else {json!(false)},
        "placement_may_have_started":attempted_placement,
        "clock_effect_may_have_started":attempted_clock,
        "retained_state_currently_verified":false,"current_goal_available":false,
        "recovery_guidance":"Preserve the original archive, paired review and clock journal, then reconcile retained placement and clock work. A lost reply never permits another placement or unpause attempt."})
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
            "unadmitted semantic furniture runtime refuses production admission provenance",
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
            "semantic furniture request cancelled",
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
            "invalid semantic furniture session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid semantic furniture session ID",
        )
    })?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 29
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
            "furniture limits require at most 1200 game ticks, 60000ms and 64GiB logical byte-work (2GiB default), with 32768 output proxy units reserved",
        ));
    }
    let mut work = display;
    work.max_bytes = display
        .max_bytes
        .checked_sub(RESPONSE_RESERVE + OPEN_INSPECTION_BYTES)
        .filter(|n| *n >= OPEN_RUNTIME_BYTES)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "complete runtime opening, inspection and response must fit before any I/O",
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
        "original_plan",
        "review",
        "recovery",
        "clock",
        "clock_effect_may_have_started",
        "partial_original_publication",
        "archive_publication_unknown",
        "paired_preparation_present",
    ] {
        if let Some(value) = reference.get(key) {
            retained[key] = value.clone();
        }
    }
    retained["ok"] = json!(false);
    retained["retained_state_currently_verified"] = json!(false);
    retained["development_placement_enabled"] = json!(false);
    retained["development_clock_enabled"] = json!(false);
    retained["prepared_in_this_opening"] = json!(false);
    retained["current_goal"] = Value::Null;
    retained["current_goal_available"] = json!(false);
    retained
}

fn render(
    state: &mut State,
    operation: &str,
    request: Option<RequestId>,
    outcome: Result<View>,
    attempted_placement: bool,
    attempted_clock: bool,
) -> String {
    let result = match outcome {
        Ok(view) => {
            let mut value = view.to_json();
            state.placement_uncertain = value["effect_uncertain"] == true;
            state.clock_uncertain = value["clock"]["unresolved"] == true
                || value["clock"]["may_be_running"] == true
                || value["clock_effect_may_have_started"] == true;
            value["session_id"] = json!(state.runtime.session_id().to_string());
            value["retained_state_currently_verified"] = json!(true);
            value["development_placement_enabled"] =
                json!(state.runtime.placement_control_enabled());
            value["development_clock_enabled"] = json!(state.runtime.clock_control_enabled());
            state.last_view = Some(value.clone());
            value
        }
        Err(cause) => {
            state.placement_uncertain |= attempted_placement;
            state.clock_uncertain |= attempted_clock;
            let mut reference = retain_failure_reference(
                state.last_view.take(),
                state.runtime.opening_recovery_reference(),
            );
            reference["placement_may_have_started"] = json!(state.placement_uncertain);
            reference["clock_effect_may_have_started"] = json!(state.clock_uncertain);
            state.last_view = Some(reference);
            failure(&cause, attempted_placement, attempted_clock)
        }
    };
    packet(
        operation,
        Some(state.runtime.session_id()),
        state.runtime.anchor(),
        Some(state.runtime.mode().as_str()),
        request,
        Some(state.display_budget),
        result,
        state.last_view.clone(),
        state.placement_uncertain || state.clock_uncertain,
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
        let mut attempted_placement = false;
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
            attempted_placement = action.may_place();
            attempted_clock = action.may_control_clock();
            let view = action.perform(&mut state.runtime, &context)?;
            // Preserve new recovery identities even if final response publication fails.
            state.last_view = Some(view.to_json());
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
            attempted_placement,
            attempted_clock,
        ))
    })();
    result.unwrap_or_else(|cause| unbound(operation, &cause))
}

#[tool(
    description = "Open one original furniture construction goal. Default offline reads exact retained custody; reconcile can acquire evidence without placement authority; control requires explicit operator placement enablement and checkpoint exception before planning. Byte-work defaults to 2GiB and may be explicitly bounded up to 64GiB for history replay; wall limit is 60s. Optional max_game_ticks defaults to zero and is capped at1200. Clock authority is independent and opening never unpauses. Sources, credentials and paths are operator configuration."
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
            placement_uncertain: false,
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
    description = "Acquire a complete paused canonical spatial observation before original planning. Returns the exact observation seal and canonical item identities. Once the original goal is retained, use wait or recovery; observation cannot replace that goal."
)]
pub fn fortress_observe(session_id: String) -> String {
    with_session(session_id, "fortress.observe", None, || Ok(Action::Observe))
}

#[tool(
    description = "Inspect original plan, paired review, placement certainty, historical achievement and the last anchored construction sample. Optional closed JSON {mode:items,expected_observation,offset,limit} pages 1..32 canonical item identities from that exact retained observation. No native calls."
)]
pub fn fortress_query(session_id: String, query: Option<String>) -> String {
    with_session(session_id, "fortress.query", None, || {
        query_action(query.as_deref())
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemQuery {
    mode: ItemQueryMode,
    expected_observation: String,
    offset: usize,
    limit: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum ItemQueryMode {
    Items,
}

fn query_action(raw: Option<&str>) -> Result<Action> {
    let Some(raw) = raw else {
        return Ok(Action::Inspect);
    };
    if raw.len() > 2048 {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "item query exceeds 2048 bytes",
        ));
    }
    let query: ItemQuery = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid closed item query"))?;
    if query.offset > 32768 || !(1..=32).contains(&query.limit) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "item offset must be at most 32768 and limit 1..32",
        ));
    }
    match query.mode {
        ItemQueryMode::Items => Ok(Action::Items {
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
            "plan request exceeds 4096 bytes",
        ));
    }
    let probe: Value = serde_json::from_str(raw).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid closed furniture or clock request",
        )
    })?;
    if probe["kind"] == "run" {
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
    description = "Compile and durably prepare one original construction goal. Closed JSON: expected_observation, kind bed/chair/table, canonical item, target [x,y,z], absolute deadline_tick, optional poll_interval_ticks and stable_samples. Returns a separate paired review seal and preserves original predicates, deadline and stability. Preparation does not place a building. Closed kind=run requests instead take original_paired_seal, game_ticks and wall_millis, producing a distinct finite clock review under separate authority."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || plan_action(&request))
}

#[tool(
    description = "scope=goal (default) confirms the exact original/native furniture review once. scope=run confirms its separate finite clock review. Both require confirmed=true, current independent authority and original-custody revalidation. Placement and clock receipts do not prove construction completion. Lost replies require exact reconciliation, never another placement or unpause attempt."
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
    description = "Reconcile original placement and retained clock work before acquiring one fresh paused receipt-bracketed construction sample. Preserve the original deadline, cadence, stability and durable first achievement. A running or uncertain clock returns pending without a construction read. Offline sessions inspect retained evidence with query. Wait never starts or extends a run, repeats placement, or starts background polling."
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
            state.runtime.anchor(),
            Some(state.runtime.mode().as_str()),
            None,
            Some(state.display_budget),
            json!({"ok":true,"scope":"session","closed":true,"session_closed":true,"effects_cancelled":false,
                "custody_deleted":false,"native_calls":0,"runtime_admitted":false,
                "automatic_redispatch":false,"retained_state_currently_verified":false}),
            state.last_view.clone(),
            state.placement_uncertain || state.clock_uncertain,
        );
        // Release held sockets and file locks even after cancellation, revoked
        // configuration or fenced custody. No native query or mutation runs.
        drop(guard.take());
        Ok(out)
    })();
    result.unwrap_or_else(|cause| unbound("fortress.cancel", &cause))
}

#[tool(
    description = "scope=goal (default) retires a still-prepared original placement without removing buildings or history. scope=run retires undispatched clock intent or requests its native owner's safety pause under independent clock authority. scope=session releases local resources even after cancellation or fencing, without cancelling game effects. Preserve original and clock recovery identities before closing."
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
    description = "Inspect exact original-plan custody and retained recovery evidence without native calls. Historical placement, current construction and first satisfaction remain distinct claims."
)]
pub fn fortress_explain(session_id: String) -> String {
    with_session(session_id, "fortress.explain", None, || Ok(Action::Inspect))
}

#[tool(
    description = "Check bounded local custody and report the unadmitted development scope, source acceptance and retained goal state. Does not contact the game or confer compatibility admission."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, || Ok(Action::Inspect))
}

#[tool(
    description = "Unavailable in this isolated furniture profile. Returns an explicit denial; checkpoint requirements cannot be removed from an already sealed plan."
)]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, || {
        Ok(Action::Unavailable)
    })
}

#[tool(
    description = "Unavailable in this isolated furniture profile. Restart recovery uses the exact original archive and journal identities; this tool cannot restore or rewrite game state."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, || {
        Ok(Action::Unavailable)
    })
}

pub fn run_stdio() {
    let server=ServerBuilder::new("dfmcp-live-semantic-furniture-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery)
        .tool(FortressPlan).tool(FortressCommit).tool(FortressWait)
        .tool(FortressCancel).tool(FortressCheckpoint).tool(FortressRestore)
        .tool(FortressExplain).tool(FortressDoctor)
        .instructions("Explicitly unadmitted original furniture construction runtime. Open, observe a canonical item, plan one bed/chair/table, review the paired seal, confirm one placement, then wait for the original construction goal. Optional finite runs need nonzero negotiated ticks, independent clock authority, plan(kind=run), and separate confirmation with scope=run. Wait never starts or chains runs. One retained goal; no automatic redispatch. Keep complete original and clock journal recovery handoffs outside private custody. Placement is historical; current receipt-linked construction and original temporal achievement are separate proof claims. Operator development source acceptance is not compatibility admission.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> Value {
        json!({"expected_observation":Digest32::of_bytes(b"basis").to_hex(),
            "kind":"bed","item":"2199023255563","target":[1,2,3],
            "deadline_tick":200,"poll_interval_ticks":10,"stable_samples":2})
    }

    #[test]
    fn native_runtime_can_be_owned_by_a_serialized_mcp_session() {
        fn require_send<T: Send>() {}
        require_send::<NativeRuntime>();
    }

    #[test]
    fn full_response_and_opening_inspection_are_reserved_before_io() -> Result<()> {
        let (display, work) = opening_budgets(Some(1000), None, None, None)?;
        assert_eq!(display.max_game_ticks, 0);
        assert_eq!(
            work.max_bytes + OPEN_INSPECTION_BYTES + RESPONSE_RESERVE,
            display.max_bytes
        );
        assert!(opening_budgets(Some(60_001), None, None, None).is_err());
        assert!(
            opening_budgets(
                None,
                Some(OPEN_RUNTIME_BYTES + OPEN_INSPECTION_BYTES + RESPONSE_RESERVE - 1),
                None,
                None
            )
            .is_err()
        );
        let minimum = OPEN_RUNTIME_BYTES + OPEN_INSPECTION_BYTES + RESPONSE_RESERVE;
        assert_eq!(
            opening_budgets(None, Some(minimum), None, None)?
                .1
                .max_bytes,
            OPEN_RUNTIME_BYTES
        );
        assert!(opening_budgets(None, None, Some(MAX_OUTPUT_TOKENS - 1), None).is_err());
        assert!(opening_budgets(None, Some(MAX_BYTES + 1), None, None).is_err());
        assert_eq!(
            opening_budgets(None, Some(MAX_BYTES), None, None)?
                .0
                .max_bytes,
            MAX_BYTES
        );
        let (display, work) = opening_budgets(None, None, None, Some(MAX_GAME_TICKS))?;
        assert_eq!(display.max_game_ticks, MAX_GAME_TICKS);
        assert_eq!(work.max_game_ticks, MAX_GAME_TICKS);
        assert!(opening_budgets(None, None, None, Some(MAX_GAME_TICKS + 1)).is_err());
        Ok(())
    }

    #[test]
    fn original_furniture_namespace_is_canonical_and_distinct() -> Result<()> {
        let id = SessionId::new((1u128 << 127) | FAMILY | 1);
        assert_eq!(session_id(&id.to_string())?, id);
        for family in [0, 1, 2, 8, 10, 16, 17, 18, 19, 30, 31] {
            let other = SessionId::new((1u128 << 127) | (family << 57) | 1);
            assert!(session_id(&other.to_string()).is_err());
        }
        assert!(session_id("1").is_err());
        assert!(session_id(&id.to_string().to_uppercase()).is_err());
        Ok(())
    }

    #[test]
    fn item_pages_are_closed_bounded_and_observation_bound() -> Result<()> {
        assert!(matches!(query_action(None)?, Action::Inspect));
        let seal = Digest32::of_bytes(b"items").to_hex();
        let q = json!({"mode":"items","expected_observation":seal,"offset":0,"limit":32});
        assert!(matches!(
            query_action(Some(&q.to_string()))?,
            Action::Items {
                offset: 0,
                limit: 32,
                ..
            }
        ));
        for (key, value) in [
            ("offset", json!(32769)),
            ("limit", json!(0)),
            ("limit", json!(33)),
            ("mode", json!("citizens")),
            ("expected_observation", json!("native-id")),
            ("source", json!("client-selected")),
        ] {
            let mut changed = q.clone();
            changed[key] = value;
            assert!(query_action(Some(&changed.to_string())).is_err());
        }
        assert!(query_action(Some(&"x".repeat(2049))).is_err());
        Ok(())
    }

    #[test]
    fn plan_intake_cannot_substitute_native_keys_sources_or_checkpoint_policy() -> Result<()> {
        let original = request();
        assert!(matches!(
            plan_action(&original.to_string())?,
            Action::Plan(_)
        ));
        for key in [
            "native_item_id",
            "idempotency_key",
            "path",
            "evidence_policy",
            "source",
            "checkpoint_exception",
            "requires_checkpoint",
            "command",
            "plan",
        ] {
            let mut changed = original.clone();
            changed[key] = json!(false);
            assert!(plan_action(&changed.to_string()).is_err(), "{key}");
        }
        for kind in ["run", "workshop", "PlaceFurniture", "door"] {
            let mut changed = original.clone();
            changed["kind"] = json!(kind);
            assert!(plan_action(&changed.to_string()).is_err());
        }
        let duplicate = original
            .to_string()
            .replacen('{', "{\"kind\":\"table\",", 1);
        assert!(plan_action(&duplicate).is_err());
        assert!(plan_action(&"x".repeat(4097)).is_err());
        Ok(())
    }

    #[test]
    fn confirmation_and_cancel_are_closed_to_original_placement() -> Result<()> {
        let seal = Digest32::of_bytes(b"paired").to_hex();
        assert!(matches!(
            commit_action(None, &seal, true)?,
            Action::Commit {
                confirmed: true,
                ..
            }
        ));
        assert!(matches!(
            commit_action(None, &seal, false)?,
            Action::Commit {
                confirmed: false,
                ..
            }
        ));
        assert!(commit_action(None, &seal.to_uppercase(), true).is_err());
        assert!(commit_action(None, &"0".repeat(64), true).is_err());
        assert!(matches!(cancel_action(None)?, Action::Cancel));
        assert!(matches!(cancel_action(Some("goal"))?, Action::Cancel));
        assert!(matches!(cancel_action(Some("run"))?, Action::CancelRun));
        assert!(commit_action(Some("all"), &seal, true).is_err());
        assert!(!commit_action(None, &seal, false)?.may_place());
        assert!(cancel_action(Some("destroy")).is_err());
        Ok(())
    }

    #[test]
    fn lost_reply_keeps_new_recovery_identity_without_cached_goal_or_control() {
        let old = json!({"original_plan":{"plan_digest":"old"},"current_goal":{"original_goal_proven":true},
            "development_placement_enabled":true,"prepared_in_this_opening":true});
        let reference = json!({"journal_id":"new-journal","original_plan":{"plan_digest":"new"},
            "review":{"paired_seal":"paired"},"recovery":{"object_digest":"new-object","history_present":true,
            "archive_publication_unknown":true},"partial_original_publication":true});
        let retained = retain_failure_reference(Some(old), reference);
        assert_eq!(retained["recovery"]["object_digest"], "new-object");
        assert_eq!(retained["recovery"]["history_present"], true);
        assert_eq!(retained["recovery"]["archive_publication_unknown"], true);
        assert_eq!(retained["partial_original_publication"], true);
        assert_eq!(retained["development_placement_enabled"], false);
        assert_eq!(retained["prepared_in_this_opening"], false);
        assert!(retained["current_goal"].is_null());
        assert_eq!(retained["current_goal_available"], false);
        let cause = error(ErrorCode::EffectIndeterminate, "lost placement reply");
        let lost = failure(&cause, true, false);
        assert!(lost["game_mutation_dispatched"].is_null());
        assert_eq!(lost["placement_may_have_started"], true);
        assert_eq!(lost["automatic_redispatch"], false);
        assert_eq!(
            failure(&cause, false, false)["game_mutation_dispatched"],
            false
        );
    }

    #[test]
    fn clock_intake_is_original_bound_and_separately_confirmed() -> Result<()> {
        let request = json!({"kind":"run", "original_paired_seal":"a".repeat(64),
            "game_ticks":10, "wall_millis":1000});
        let proposed = plan_action(&request.to_string())?;
        assert!(matches!(proposed, Action::PlanRun(_)));
        assert!(!proposed.may_place());
        assert!(!proposed.may_control_clock());
        for (key, value) in [
            ("game_ticks", json!(0)),
            ("game_ticks", json!(1201)),
            ("wall_millis", json!(0)),
            ("wall_millis", json!(60001)),
            ("original_paired_seal", json!("native-key")),
            ("path", json!("client-path")),
            ("source_generation", json!(7)),
            ("checkpoint_exception", json!(true)),
            ("deadline_tick", json!(999)),
        ] {
            let mut changed = request.clone();
            changed[key] = value;
            assert!(plan_action(&changed.to_string()).is_err(), "{key}");
        }
        let duplicate = request.to_string().replacen('{', "{\"kind\":\"run\",", 1);
        assert!(plan_action(&duplicate).is_err());
        let seal = "b".repeat(64);
        let confirmed = commit_action(Some("run"), &seal, true)?;
        assert!(matches!(
            confirmed,
            Action::CommitRun {
                confirmed: true,
                ..
            }
        ));
        assert!(confirmed.may_control_clock());
        assert!(!confirmed.may_place());
        assert!(!commit_action(Some("run"), &seal, false)?.may_control_clock());
        let cancelled = cancel_action(Some("run"))?;
        assert!(cancelled.may_control_clock());
        assert!(!cancelled.may_place());
        Ok(())
    }

    #[test]
    fn clock_failures_keep_new_custody_without_cached_control_or_goal_truth() {
        let old = json!({"current_goal":{"semantic_completion_proven":true},
            "development_clock_enabled":true,"development_placement_enabled":true});
        let newest = json!({"recovery":{"clock_journal_id":"a".repeat(64)},
            "clock":{"journal_id":"a".repeat(64),"head":"b".repeat(64),
                "unresolved":true,"active_run":{"key":"original-clock"}},
            "clock_effect_may_have_started":true});
        let retained = retain_failure_reference(Some(old), newest.clone());
        assert_eq!(retained["clock"], newest["clock"]);
        assert_eq!(retained["recovery"], newest["recovery"]);
        assert_eq!(retained["clock_effect_may_have_started"], true);
        assert_eq!(retained["development_clock_enabled"], false);
        assert_eq!(retained["development_placement_enabled"], false);
        assert!(retained["current_goal"].is_null());
        let lost = failure(
            &error(ErrorCode::EffectIndeterminate, "lost clock reply"),
            false,
            true,
        );
        assert!(lost["game_mutation_dispatched"].is_null());
        assert_eq!(lost["clock_effect_may_have_started"], true);
        assert_eq!(lost["placement_may_have_started"], false);
        assert_eq!(lost["automatic_redispatch"], false);
    }
}
