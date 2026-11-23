#![forbid(unsafe_code)]
//! Isolated, explicitly unadmitted optional Manager -> Tree -> Workshop -> stock project.
//! All native effects stay inside the existing independently reviewed children.
mod config;
mod continuity;
mod journal;
mod presentation;
mod runtime;

use dfmcp_core::{DfmcpError, ErrorCode, RequestId, Result, SessionId, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use fastmcp_rust::{CompleteResult, FinalCallToolResult};
use journal::Mode;
use runtime::{Action, Runtime};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use std::time::Instant;

// The family-31 development profiles are mutually exclusive by their closed
// process environment. Each owner additionally requires its exact live handle.
const FAMILY: u128 = 31u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<Runtime>> = Mutex::new(None);

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

fn parse_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid resource project session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "invalid resource project session ID",
        )
    })?;
    let id = SessionId::new(value);
    if id.get() != value
        || !id.is_process_scoped_live()
        || (value & ((1u128 << 62) - 1)) >> 57 != 31
    {
        return Err(error(
            ErrorCode::SessionNotFound,
            "foreign resource project session handle",
        ));
    }
    Ok(id)
}

fn publish(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    outcome: Result<Value>,
    reference: Value,
    mut runtime: Option<&mut Runtime>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let successful = outcome.is_ok();
    let packet = match outcome {
        Ok(value) => presentation::packet(operation, session, request, value, reference.clone()),
        Err(cause) => presentation::failure(operation, session, request, cause, reference.clone()),
    };
    if let Some(result) = presentation::render(packet) {
        if successful
            && let Some(owner) = runtime.as_deref_mut()
            && let Err(cause) = owner.publication_current()
        {
            owner.withdraw();
            return presentation::render(presentation::failure(
                operation,
                session,
                request,
                cause,
                owner.reference(),
            ))
            .ok_or_else(|| {
                McpError::internal_error("resource project recovery reference exceeds output bound")
            });
        }
        return Ok(result);
    }
    // The caller still holds the session lock. No concurrent Commit can use a
    // review whose complete evidence failed the exact modern codec boundary.
    let reference = if let Some(runtime) = runtime {
        runtime.withdraw();
        runtime.reference()
    } else {
        reference
    };
    presentation::render(presentation::failure(
        operation,
        session,
        request,
        error(
            ErrorCode::BudgetExceeded,
            "complete project evidence exceeds output bound; reviews withdrawn",
        ),
        reference,
    ))
    .ok_or_else(|| {
        McpError::internal_error("resource project recovery reference exceeds output bound")
    })
}

fn call(
    operation: &str,
    raw: String,
    action: Action,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let session = match parse_id(&raw) {
        Ok(session) => session,
        Err(cause) => return publish(operation, None, None, Err(cause), Value::Null, None),
    };
    let mut guard = match SESSION.try_lock() {
        Ok(guard) => guard,
        Err(_) => {
            return publish(
                operation,
                Some(session),
                None,
                Err(error(
                    ErrorCode::BudgetExceeded,
                    "resource project session busy or poisoned",
                )),
                Value::Null,
                None,
            );
        }
    };
    let Some(runtime) = guard.as_mut().filter(|runtime| runtime.session == session) else {
        return publish(
            operation,
            Some(session),
            None,
            Err(error(
                ErrorCode::SessionNotFound,
                "original resource project session unavailable",
            )),
            Value::Null,
            None,
        );
    };
    let outcome = runtime.perform(action);
    let request = Some(runtime.last_request_id());
    let reference = runtime.reference();
    publish(
        operation,
        Some(session),
        request,
        outcome,
        reference,
        Some(runtime),
    )
}

fn rejected(
    operation: &str,
    raw: String,
    cause: DfmcpError,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let session = parse_id(&raw).ok();
    if let Ok(mut guard) = SESSION.try_lock()
        && let Some(runtime) = guard
            .as_mut()
            .filter(|runtime| Some(runtime.session) == session)
    {
        runtime.withdraw();
        let reference = runtime.reference();
        return publish(
            operation,
            session,
            Some(runtime.last_request_id()),
            Err(cause),
            reference,
            Some(runtime),
        );
    }
    publish(operation, session, None, Err(cause), Value::Null, None)
}

fn budget(
    wall: Option<u64>,
    bytes: Option<u64>,
    output: Option<u32>,
    ticks: Option<u64>,
) -> Result<WorkBudget> {
    let budget = WorkBudget {
        max_wall_millis: wall.unwrap_or(60_000),
        max_bytes: bytes.unwrap_or(16 * 1024 * 1024 * 1024),
        max_output_tokens: output.unwrap_or(65_536),
        max_game_ticks: ticks.unwrap_or(0),
        max_entities: 4096,
        max_actions: 5,
    };
    budget.validate()?;
    if budget.max_wall_millis > 60_000
        || !(4 * 1024 * 1024 * 1024..=runtime::MAX_BYTES).contains(&budget.max_bytes)
        || budget.max_output_tokens != 65_536
        || budget.max_game_ticks > 1200
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "project requires 4..32GiB bounded byte-work, at most60000ms, exactly65536 output tokens and at most1200 explicit clock ticks",
        ));
    }
    Ok(budget)
}

#[tool(
    name = "fortress_open_session",
    description = "Open one explicitly unadmitted native resource project: optionally appoint the fixed Manager, optionally harvest the fixed Tree, construct the fixed Carpenter Workshop, then pursue the fixed furniture stock goal or a portfolio of one to four minima. mode defaults offline; control requires independent host family permissions and new parent/child custody. Recovery opens existing custody only and restores no review. Defaults:60000ms,16GiB logical byte-work,65536 output tokens,zero game ticks. Every native effect has its own review; every clock remains separately configured and confirmed."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_game_ticks: Option<u64>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let start = Instant::now();
    let opened = (|| -> Result<_> {
        config::runtime_io()?;
        let mode = match mode.as_deref().unwrap_or("offline") {
            "control" => Mode::Control,
            "recover" => Mode::Recover,
            "offline" => Mode::Offline,
            _ => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "project mode must be control, recover or offline",
                ));
            }
        };
        let budget = budget(
            max_wall_millis,
            max_bytes,
            max_output_tokens,
            max_game_ticks,
        )?;
        let (config, workshop) = config::Config::load()?;
        let mut guard = SESSION
            .try_lock()
            .map_err(|_| error(ErrorCode::Conflict, "resource project session busy"))?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the original project session before opening another",
            ));
        }
        let sequence = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |sequence| {
                (sequence < (1u64 << 57)).then_some(sequence + 1)
            })
            .map_err(|_| {
                error(
                    ErrorCode::BudgetExceeded,
                    "resource project identities exhausted",
                )
            })?;
        let session = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        let (runtime, outcome) = Runtime::open(config, workshop, mode, session, budget, start)?;
        *guard = Some(runtime);
        let runtime = guard.as_mut().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "project owner not retained",
            )
        })?;
        let request = Some(runtime.last_request_id());
        let reference = runtime.reference();
        Ok(publish(
            "fortress.open_session",
            Some(session),
            request,
            outcome,
            reference,
            Some(runtime),
        ))
    })();
    match opened {
        Ok(result) => result,
        Err(cause) => publish(
            "fortress.open_session",
            None,
            None,
            Err(cause),
            Value::Null,
            None,
        ),
    }
}

#[tool(
    name = "fortress_observe",
    description = "Read the active child's bounded native evidence. Manager, Tree and Workshop accept no request selector. Production accepts its existing closed workforce-selection/production-inspection request. Observation never prepares a new stage or native effect."
)]
pub fn fortress_observe(
    session_id: String,
    request: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    call("fortress.observe", session_id, Action::Observe(request))
}

#[tool(
    name = "fortress_query",
    description = "Verify local parent and active-child custody and expose original goals, stage, reviews, source floors, clock uncertainty and exact recovery references. In production, optional order_step_id selects an exact original portfolio order. Optional closed dfmcp.query/1 request inspects a previously captured operations source for inventory totals, production diagnosis, declared supply chains and bounded structured queries. expected_anchor names production_inspection_source.anchor. Before original planning, Observe(include_production=true) supplies the source and order selectors are forbidden; afterwards Wait supplies the current selected source. Other stages reject both selectors. Inspection does not read the native game, advance time, renew observations or authorize effects."
)]
pub fn fortress_query(
    session_id: String,
    order_step_id: Option<u32>,
    request: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    match Action::query(request.as_deref(), order_step_id) {
        Ok(action) => call("fortress.query", session_id, action),
        Err(cause) => rejected("fortress.query", session_id, cause),
    }
}

#[tool(
    name = "fortress_plan",
    description = "Plan within the active original child. Optional Manager uses appoint_vacant_manager with its observed witness and fixed project deadline. Optional Tree uses fell_one_visible_tree with that deadline or separately configured run_tree. Workshop uses its observed material request or run_workshop. advance_from_manager, advance_to_workshop and advance_to_production require exact expected_head, freshly verify the completed original donor and durably transfer ownership without a native mutation. Production accepts stock_target or stock_targets matching every immutable root goal term, then step/review_prepared with original_plan_digest. Single-target roots also accept independently configured run or inventory_run; portfolio production clocks are unavailable. Each native effect requires its own current review and explicit Commit."
)]
pub fn fortress_plan(
    session_id: String,
    request: String,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    call("fortress.plan", session_id, Action::Plan(request))
}

#[tool(
    name = "fortress_commit",
    description = "Confirm one complete current child review. scope=goal for Manager appointment, Tree designation, Workshop construction or original labor/order step; scope=clock for a separately reviewed Tree/Workshop/order clock; scope=inventory_clock for production stability. Manager accepts only goal scope. The project never renews the original deadline or restores confirmation after restart."
)]
pub fn fortress_commit(
    session_id: String,
    review_seal: String,
    confirmed: bool,
    scope: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    call(
        "fortress.commit",
        session_id,
        Action::Commit {
            seal: review_seal,
            confirmed,
            scope,
        },
    )
}

#[tool(
    name = "fortress_wait",
    description = "Perform one foreground reconciliation or goal sample for the active original child. Manager reconciles its original appointment key. Tree and Workshop scope=clock reconcile their independently retained clock. Production reconciles its original goals and clocks. No background polling, redispatch, new stage or implicit unpause. Historical appointment or Tree supply alone is not a fresh handoff or final stock proof."
)]
pub fn fortress_wait(
    session_id: String,
    scope: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    call("fortress.wait", session_id, Action::Wait(scope))
}

#[tool(
    name = "fortress_cancel",
    description = "Default scope=project (goal is an alias) durably stops future stages/reviews, then retires the current undispatched preparation. It does not remove an appointed Manager or undo a designation, construction or inserted order. scope=clock or inventory_clock requests the active child's separately authorized safety stop; Manager has no clock scope. scope=session releases local ownership while preserving all custody; it does not claim a native pause or effect cancellation."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    if scope.as_deref() != Some("session") {
        return call("fortress.cancel", session_id, Action::Cancel(scope));
    }
    let closed = (|| -> Result<_> {
        let session = parse_id(&session_id)?;
        let mut guard = SESSION
            .try_lock()
            .map_err(|_| error(ErrorCode::Conflict, "resource project session busy"))?;
        let runtime = guard
            .as_mut()
            .filter(|runtime| runtime.session == session)
            .ok_or_else(|| {
                error(
                    ErrorCode::SessionNotFound,
                    "resource project session unavailable",
                )
            })?;
        runtime.withdraw();
        let reference = runtime.reference();
        let request = Some(runtime.last_request_id());
        // Closing creates no new native authority and remains possible after
        // the preceding operation expires. Keep ownership until the complete
        // retained reference passes the exact publication bound.
        let result = presentation::render(presentation::packet(
            "fortress.cancel",
            Some(session),
            request,
            json!({"ok":true,"session_closed":true,"native_effects_cancelled":false,
                "current_pause_proven":false,"automatic_redispatch":false}),
            reference,
        ))
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "complete close recovery exceeds output bound; original session retained",
            )
        })?;
        *guard = None;
        Ok(result)
    })();
    match closed {
        Ok(result) => Ok(result),
        Err(cause) => rejected("fortress.cancel", session_id, cause),
    }
}

#[tool(
    name = "fortress_explain",
    description = "Inspect the immutable root objective, current stage and complete original child custody. Historical receipts, qualified stock census, current monitor proof and clock exposure remain separate."
)]
pub fn fortress_explain(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    call("fortress.explain", session_id, Action::Inspect)
}

#[tool(
    name = "fortress_doctor",
    description = "Inspect exact local parent/child custody and development limitations. This is diagnosis; it grants no runtime admission, native authority, new key or recovered review."
)]
pub fn fortress_doctor(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    call("fortress.doctor", session_id, Action::Inspect)
}

#[tool(
    name = "fortress_checkpoint",
    description = "This native resource profile has no world checkpoint provider. Parent stage custody and native effect journals are not save-game checkpoints."
)]
pub fn fortress_checkpoint(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    rejected(
        "fortress.checkpoint",
        session_id,
        error(
            ErrorCode::CapabilityDenied,
            "native resource project has no world checkpoint provider",
        ),
    )
}

#[tool(
    name = "fortress_restore",
    description = "This profile cannot restore the world or carry original native identities across epochs. Preserve and reconcile existing custody."
)]
pub fn fortress_restore(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    rejected(
        "fortress.restore",
        session_id,
        error(
            ErrorCode::CapabilityDenied,
            "native resource project has no world restore provider",
        ),
    )
}

pub fn run_stdio() {
    if let Err(cause) = config::environment() {
        eprintln!("{}: {}", cause.code.as_str(), cause.message);
        return;
    }
    let server = ServerBuilder::new("dfmcp-native-resource-project-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery)
        .tool(FortressPlan).tool(FortressCommit).tool(FortressWait)
        .tool(FortressCancel).tool(FortressExplain).tool(FortressDoctor)
        .tool(FortressCheckpoint).tool(FortressRestore)
        .instructions("One explicitly unadmitted resource project with a fixed host stock goal or one-to-four-target portfolio and absolute deadline. The parent owns an optional Manager/1.23 appointment, an optional Tree harvest, Workshop construction and actual stock-target production, one child at a time. Review and confirm every native effect separately. Advance from Manager, to Workshop or to production only with the current parent head; each transition acquires fresh original donor evidence and any settled clock custody, then acknowledges the transition before releasing its writer. Production independently imports the exact Manager and Workshop donors Offline and freshly revalidates their readiness; historical appointment is not present production proof. Tree supply remains subject to current Workshop material validation. Defaults forbid game-time advancement. Stock portfolio roots require absent production and inventory clocks; their original minima are sampled together using separately managed game time. Optional Tree and Workshop clocks retain their own review and authority. Project cancellation durably stops new work but does not remove an appointed Manager or undo other existing native effects; explicitly reconcile/cancel active clocks. Retain every returned parent/child recovery reference, including after errors. Never initialize an existing opening marker again. Complete evidence may use numbered text blocks with byte counts, SHA-256 and exact JSON paths; reassemble them before confirming. Manager and Tree semantic reviews name capsule commitments; raw capsule bytes remain with their indicated owner or custody and are not inline evidence chunks. No world restore or production admission.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_handle_and_budget_boundaries_reject_foreign_or_widened_requests() -> Result<()> {
        assert!(parse_id("0").is_err());
        assert!(parse_id(&format!("{:032x}", 1u128 << 127)).is_err());
        let default = budget(None, None, None, None)?;
        assert_eq!(default.max_game_ticks, 0);
        assert_eq!(default.max_entities, 4096);
        assert!(budget(Some(60_001), None, None, None).is_err());
        assert!(budget(None, Some(runtime::MAX_BYTES + 1), None, None).is_err());
        assert!(budget(None, None, Some(32_768), None).is_err());
        assert!(budget(None, None, None, Some(1201)).is_err());
        Ok(())
    }
}
