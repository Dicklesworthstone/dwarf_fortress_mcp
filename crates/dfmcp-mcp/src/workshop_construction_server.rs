#![forbid(unsafe_code)]
//! Explicitly unadmitted eleven-tool workshop/1.20 stdio consumer.
//! No tool accepts endpoints, paths, native commands or native addresses.
mod presentation;
mod runtime;
use dfmcp_adapter::workshop_construction::journal::WorkshopMode;
use dfmcp_core::{ErrorCode, RequestId, Result, SessionId, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use runtime::{Action, Config, RESPONSE_BYTES, Runtime, error, runtime_io, validate_environment};
use serde_json::{Value, json};
use std::sync::{
    Mutex, MutexGuard, TryLockError,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

const FAMILY: u128 = 31u128 << 57;
const SEQUENCE_LIMIT: u64 = 1u64 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State>> = Mutex::new(None);
struct State {
    runtime: Runtime,
    last_reference: Value,
}
fn retained(view: &Value) -> Value {
    let o = &view["original"];
    json!({"acknowledged":view["acknowledged"],"original":{"key":o["key"],"plan_digest":o["plan_digest"],
        "review_seal":o["review_seal"],"native_receipt":o["native_receipt"],"state":o["state"],
        "dispatch_started":o["dispatch_started"],"unresolved":o["unresolved"]},"current_goal_available":false,"currently_verified":false})
}
fn lock() -> Result<MutexGuard<'static, Option<State>>> {
    SESSION.try_lock().map_err(|e| match e {
        TryLockError::WouldBlock => error(
            ErrorCode::BudgetExceeded,
            "another bounded workshop request owns this session",
        ),
        TryLockError::Poisoned(_) => error(
            ErrorCode::InternalInvariantViolation,
            "workshop session poisoned; preserve both stores and recover",
        ),
    })
}
fn id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid workshop session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid workshop session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 31
    {
        return Err(error(
            ErrorCode::SessionNotFound,
            "session belongs to another process/profile",
        ));
    }
    Ok(id)
}
fn budgets(wall: Option<u64>, bytes: Option<u64>, output: Option<u32>) -> Result<WorkBudget> {
    let b = WorkBudget {
        max_wall_millis: wall.unwrap_or(60000),
        max_bytes: bytes.unwrap_or(128 * 1024 * 1024),
        max_output_tokens: output.unwrap_or(32768),
        max_entities: 128,
        max_actions: 1,
        max_game_ticks: 0,
    };
    b.validate()?;
    if b.max_wall_millis > 60000
        || !(64 * 1024 * 1024..=512 * 1024 * 1024).contains(&b.max_bytes)
        || b.max_output_tokens > 32768
        || u64::from(b.max_output_tokens) * 4 < RESPONSE_BYTES
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "workshop needs bounded complete custody/output reserves, at most 60000ms and zero game ticks",
        ));
    }
    Ok(b)
}
fn failure(
    operation: &str,
    cause: dfmcp_core::DfmcpError,
    session: Option<SessionId>,
    budget: Option<WorkBudget>,
    reference: Value,
) -> String {
    failure_with_request(operation, cause, session, None, budget, reference)
}
fn failure_with_request(
    operation: &str,
    cause: dfmcp_core::DfmcpError,
    session: Option<SessionId>,
    request: Option<RequestId>,
    budget: Option<WorkBudget>,
    reference: Value,
) -> String {
    presentation::packet(
        operation,
        session,
        request,
        budget,
        json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,"current_goal_available":false,
        "retained_state_currently_verified":false,"effect_may_have_occurred":true}),
        reference,
    )
}
fn with_session(raw: String, operation: &str, action: Action) -> String {
    let result = (|| -> Result<String> {
        let session = id(&raw)?;
        let mut guard = lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.runtime.session == session)
            .ok_or_else(|| {
                error(
                    ErrorCode::SessionNotFound,
                    "workshop session absent or closed",
                )
            })?;
        let context = match state.runtime.context() {
            Ok(c) => c,
            Err(cause) => {
                state.runtime.withdraw();
                return Ok(failure(
                    operation,
                    cause,
                    Some(session),
                    Some(state.runtime.budget),
                    state.runtime.failure_reference(),
                ));
            }
        };
        let outcome = state.runtime.perform(action, &context);
        let reference = state.runtime.failure_reference();
        match outcome {
            Ok(view) => {
                state.last_reference = retained(&view);
                Ok(presentation::packet(
                    operation,
                    Some(session),
                    Some(context.request_id),
                    Some(state.runtime.budget),
                    view,
                    state.last_reference.clone(),
                ))
            }
            Err(cause) => {
                let retained = json!({"last_complete_reference":state.last_reference,"latest_unverified_reference":reference,"current_goal_available":false});
                Ok(failure_with_request(
                    operation,
                    cause,
                    Some(session),
                    Some(context.request_id),
                    Some(state.runtime.budget),
                    retained,
                ))
            }
        }
    })();
    result.unwrap_or_else(|e| failure(operation, e, None, None, Value::Null))
}

#[tool(
    description = "Open one explicitly unadmitted workshop/1.20 goal. mode=control initializes exact operator-pinned sources; recover/offline require both original custody files and an externally held acknowledged pin. No opening advances time. The default wall budget is 60000ms; the original prepared connection deadline is never renewed."
)]
pub fn fortress_open_session(
    mode: String,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> String {
    let result = (|| -> Result<String> {
        let start = Instant::now();
        runtime_io()?;
        validate_environment()?;
        let mode = match mode.as_str() {
            "control" => WorkshopMode::Control,
            "recover" => WorkshopMode::Recover,
            "offline" => WorkshopMode::Offline,
            _ => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "workshop mode must be control, recover or offline",
                ));
            }
        };
        let display = budgets(max_wall_millis, max_bytes, max_output_tokens)?;
        let config = Config::environment()?;
        let mut guard = lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the original workshop session before another opening",
            ));
        }
        let seq = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < SEQUENCE_LIMIT).then_some(n + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "workshop session identities exhausted",
                )
            })?;
        let session = SessionId::new((1u128 << 127) | FAMILY | u128::from(seq));
        let mut runtime = Runtime::open(config, mode, session, display)?;
        let mut context = runtime.context()?;
        context.budget.max_wall_millis = context
            .budget
            .max_wall_millis
            .checked_sub(start.elapsed().as_millis() as u64)
            .filter(|n| *n > 0)
            .ok_or_else(|| {
                error(
                    ErrorCode::BudgetExceeded,
                    "workshop opening deadline expired",
                )
            })?;
        context.budget.max_bytes = 4 * 1024 * 1024;
        let reference = runtime.failure_reference();
        // Hold custody even if final inspection fails, so the client can preserve
        // original references and explicitly release this session's locks.
        *guard = Some(State {
            runtime,
            last_reference: reference,
        });
        let state = guard.as_mut().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "workshop opening lost its owner",
            )
        })?;
        match state.runtime.inspect(&context) {
            Ok(view) => {
                state.last_reference = retained(&view);
                Ok(presentation::packet(
                    "fortress.open_session",
                    Some(session),
                    Some(context.request_id),
                    Some(display),
                    view,
                    state.last_reference.clone(),
                ))
            }
            Err(e) => Ok(failure_with_request(
                "fortress.open_session",
                e,
                Some(session),
                Some(context.request_id),
                Some(display),
                state.last_reference.clone(),
            )),
        }
    })();
    result.unwrap_or_else(|e| failure("fortress.open_session", e, None, None, Value::Null))
}
#[tool(
    description = "Observe the one original operator-scoped Carpenter workshop footprint, exact log, complete bounded worker census and current native source. Observed availability does not guarantee worker assignment. Cannot replace a retained original goal."
)]
pub fn fortress_observe(session_id: String) -> String {
    with_session(session_id, "fortress.observe", Action::Observe)
}
#[tool(
    description = "Inspect the original workshop plan, material/worker/footprint/source witnesses, exact review, insertion receipt, historical goal samples and independent durable acknowledgement. This local read does not establish current game facts."
)]
pub fn fortress_query(session_id: String) -> String {
    with_session(session_id, "fortress.query", Action::Inspect)
}
#[tool(
    description = "Prepare the original Carpenter workshop without constructing it. Closed JSON request contains kind=carpenters, expected_observation and the exact observed material handle. Returns the whole original plan and separate exact review seal. One goal, no replacement keys, no game clock or manager changes."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", Action::Plan(request))
}
#[tool(
    description = "Confirm the exact original review_seal with confirmed=true. Revalidate original source/footprint/material/worker, current scoped authority and lease; fsync the dispatch journal AND its independent acknowledgement before the sole native writer. Insertion is not completion. Lost replies never permit redispatch."
)]
pub fn fortress_commit(session_id: String, review_seal: String, confirmed: bool) -> String {
    with_session(
        session_id,
        "fortress.commit",
        Action::Commit {
            seal: review_seal,
            confirmed,
        },
    )
}
#[tool(
    description = "Reconcile only the original native key, or take one fresh workshop/1.20 original-building progress sample after insertion. Exact subtype, material, geometry, stage and source continuity are mandatory. Old log availability and worker idleness are not completion conditions. Never unpauses or starts background work."
)]
pub fn fortress_wait(session_id: String) -> String {
    with_session(session_id, "fortress.wait", Action::Wait)
}
#[tool(
    description = "scope=goal retires an unattempted native preparation or queries uncertain original work; it never deconstructs. scope=session releases local sockets/locks even after fencing or permission loss, preserving both custody files and all original evidence."
)]
pub fn fortress_cancel(session_id: String, scope: Option<String>) -> String {
    if scope.as_deref() == Some("session") {
        let result = (|| -> Result<String> {
            let session = id(&session_id)?;
            let mut guard = lock()?;
            let state = guard
                .as_ref()
                .filter(|s| s.runtime.session == session)
                .ok_or_else(|| error(ErrorCode::SessionNotFound, "workshop session absent"))?;
            let out = presentation::packet(
                "fortress.cancel",
                Some(session),
                None,
                Some(state.runtime.budget),
                json!({"ok":true,"session_closed":true,"effects_cancelled":false,"custody_deleted":false,"automatic_redispatch":false,"current_goal_available":false}),
                state.last_reference.clone(),
            );
            drop(guard.take());
            Ok(out)
        })();
        return result.unwrap_or_else(|e| failure("fortress.cancel", e, None, None, Value::Null));
    }
    if scope.as_deref().is_some_and(|s| s != "goal") {
        return failure(
            "fortress.cancel",
            error(ErrorCode::InvalidRequest, "scope must be goal or session"),
            None,
            None,
            Value::Null,
        );
    }
    with_session(session_id, "fortress.cancel", Action::Cancel)
}
#[tool(
    description = "Explain exact original workshop evidence and recovery identities. Local historical inspection only; no native call or mutation authority."
)]
pub fn fortress_explain(session_id: String) -> String {
    with_session(session_id, "fortress.explain", Action::Inspect)
}
#[tool(
    description = "Verify both bounded local workshop custody files and report unadmitted posture, original goal and explicit manager/activation/finite-bed gaps. Does not grant compatibility or runtime admission."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", Action::Inspect)
}
#[tool(
    description = "Unavailable: the workshop profile has no checkpoint provider. An operator disposable-fortress exception must precede preparation and cannot be supplied through tool arguments."
)]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", Action::Unavailable)
}
#[tool(
    description = "Unavailable: this workshop profile cannot restore game state or create a new identity for an uncertain goal. Reopen the exact original custody and acknowledged pin."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", Action::Unavailable)
}

/// Public wrapper rejects production state before entering the private stdio
/// implementation. It never consumes tickets or adds admitted provenance.
pub fn run_stdio() {
    if let Err(e) = validate_environment() {
        eprintln!("{}: {}", e.code.as_str(), e.message);
        return;
    }
    let server=ServerBuilder::new("dfmcp-workshop-v1-20-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("Unadmitted workshop/1.20 development profile. Observe one exact original Carpenter/log offer, plan, review the whole source/worker/material/footprint seal, confirm one construction, then wait for authoritative original-building completion. Independent acknowledgement is synchronized before dispatch. Preserve both custody files and external acknowledgement; uncertain effects never permit replacement keys or redispatch. No game clock, manager, activation or finite-bed production path. Original .14 remains open.").build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests;
