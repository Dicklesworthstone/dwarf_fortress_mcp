#![forbid(unsafe_code)]
//! Dedicated, explicitly unadmitted manager-appointment/1.22 modern stdio consumer.
mod presentation;
mod runtime;
#[cfg(test)]
mod server_tests;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_adapter::manager_appointment::journal::Mode;
use dfmcp_core::{ErrorCode, RequestId, Result, SessionId, WorkBudget};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use fastmcp_rust::{CompleteResult, FinalCallToolResult};
use runtime::{Action, Config, RESPONSE_BYTES, Runtime, error};
use serde_json::{Value, json};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
const FAMILY: u128 = 22u128 << 57;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<Runtime>> = Mutex::new(None);
fn spine(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
) -> AgentTurnBuilder {
    let phase = match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    };
    let mut turn=AgentTurnBuilder::new(operation,phase).continuity(ContinuityStatus::Indeterminate,None,
        Some(json!({"canonical_world_continuity":"unestablished","canonical_world_anchor_available":false})),None);
    if let Some(id) = session {
        turn = turn.session_id(id.to_string());
    }
    if let Some(id) = request {
        turn = turn.request_id(id.to_string());
    }
    turn
}
fn packet(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    result: Value,
    reference: Value,
) -> String {
    let verified = result["ok"] == true && result["retained_state_currently_verified"] == true;
    let original = if verified {
        result["original"].clone()
    } else {
        Value::Null
    };
    let prepared = verified && original["prepared_in_this_connection"] == true;
    let proved = verified && original["appointment_proved"] == true;
    let refs = if reference.is_object() {
        vec![reference.clone()]
    } else {
        vec![]
    };
    let uncertain = if reference["uncertain_effect"] == true {
        refs.clone()
    } else {
        vec![]
    };
    let active = json!({"key":reference["original_key"],"plan_digest":reference["original_plan_digest"],"review_seal":reference["original_review_seal"],"native_phase":reference["native_phase"],"dispatch_started":reference["dispatch_started"],"automatic_redispatch":false});
    let mut next = vec![];
    if let Some(session) = session
        && result["session_closed"] != true
        && (verified || reference.is_object())
    {
        let mut args = json!({"session_id":session.to_string()});
        let (tool, reason) = if !verified || result["mode"] == "offline" {
            (
                "fortress_query",
                "Inspect original private custody and preserve its key, ACK and uncertainty.",
            )
        } else if prepared {
            args["review_seal"] = original["review_seal"].clone();
            args["confirmed"] = json!(true);
            (
                "fortress_commit",
                "Review the exact citizen/HF, vacancy, unchanged assignment flags, new POSITION link, one history event and source witness; confirm this seal once.",
            )
        } else if proved || matches!(original["native_phase"].as_u64(), Some(3 | 4)) {
            (
                "fortress_explain",
                "Inspect retained appointment proof or refusal; a proved Manager is a prerequisite for a separately reviewed finite order.",
            )
        } else if original.is_object() {
            (
                "fortress_wait",
                "Reconcile only the original key; a missing or uncertain record never authorizes redispatch.",
            )
        } else if result["observation"]["eligible_at_anchor"] == true {
            args["request"]=json!(json!({"kind":"appoint_vacant_manager","expected_observation":result["observation"]["witness"],"deadline_tick":result["recommended_deadline_tick"]}).to_string());
            (
                "fortress_plan",
                "Prepare the one observed eligible current Expedition Leader for vacant MANAGER without changing the game.",
            )
        } else {
            (
                "fortress_observe",
                "Inspect the bounded assignment, HF, history and dual-getter witness before planning.",
            )
        };
        let mut rec = recommendation(
            "continue_original_manager_appointment",
            tool,
            reason,
            "high",
            "high",
            if prepared {
                "irreversible"
            } else {
                "read_only"
            },
            if prepared {
                "no automatic removal or rollback"
            } else {
                "not_applicable"
            },
            prepared,
            args,
        );
        rec["confidence"]["evidence"] = json!(refs);
        rec["estimated_cost"] = json!({"game_ticks":0,"native_writer_calls_max":u8::from(prepared),"output_bytes_max":RESPONSE_BYTES});
        rec["invalidating_conditions"] = json!([
            "source, generation, sequence, tick, vacancy, HF identity or witnessed fields change",
            "authority, preparation or budget expiry",
            "lost preparation connection",
            "changed journal or independent ACK"
        ]);
        rec["grants_authority"] = json!(false);
        next.push(rec);
    }
    let has_work = reference["original_key"].is_string();
    let mut turn=spine(operation,session,request)
        .briefing(json!({"profile":"manager/1.22","runtime_admitted":false,"mutation_admissible":false,"development_commit_available":prepared,
            "canonical_world_anchor_available":false,"native_projection_anchor":reference["native_anchor"],"appointment_proved":proved,"clock_advanced":false}))
        .active_work(json!({"pending_plans":if prepared{vec![active.clone()]}else{vec![]},"actions":if has_work{vec![active.clone()]}else{vec![]},"obligations":[],"cancellation_drains":[],"indeterminate_effects":uncertain,
            "publications":[],"confirmations":if prepared{vec![json!({"review_seal":original["review_seal"],"required":true,"confirmed":false})]}else{vec![]}}))
        .affordances(if verified && result["observation"]["eligible_at_anchor"]==true {vec![json!({"intent":"appoint_vacant_manager","unit_id":result["observation"]["unit_id"],"histfig_id":result["observation"]["histfig_id"],"capability":"configure_production","risk":"irreversible","confirmation_required":true,"grants_authority":false})]}else{vec![]})
        .recommendations(next)
        .uncertainty(vec![json!({"uncertainty_id":"projection_scope","epistemic_state":"unknown","statement":"Exact evidence covers the declared stable native projection, not all game fields or canonical world continuity. Historical appointment proof is not current scheduling, order activation or bed production."}),json!({"uncertainty_id":"admission","epistemic_state":"unknown","statement":"Protocol 1.22 is unadmitted development only. Original .14 and .14.2 require independent positive evidence."})])
        .coverage(json!({"status":"partial","source":"manager/1.22","canonical_world_adapter":false,"retained_custody_verified":verified,"native_projection_anchor":reference["native_anchor"]}))
        .budget(json!({"max_output_bytes":RESPONSE_BYTES,"max_game_ticks":0})).references(refs).build();
    if let Some(b) = turn["briefing"].as_object_mut() {
        b.remove("admission");
    }
    let value =
        json!({"agent_turn":turn,"result":result,"retained_reference":reference}).to_string();
    if value.len() <= RESPONSE_BYTES as usize {
        return value;
    }
    overflow_packet(operation, session, request, reference)
}
fn overflow_packet(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    reference: Value,
) -> String {
    let references = if reference.is_object() {
        vec![reference.clone()]
    } else {
        vec![]
    };
    let indeterminate = if reference.is_object() && reference["uncertain_effect"] == true {
        vec![reference.clone()]
    } else {
        vec![]
    };
    let minimal=spine(operation,session,request).briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"current_goal_available":false,"canonical_world_anchor_available":false}))
        .active_work(json!({"pending_plans":[],"actions":[],"obligations":[],"cancellation_drains":[],"indeterminate_effects":indeterminate,"publications":[],"confirmations":[]}))
        .uncertainty(vec![json!({"uncertainty_id":"output_bound","epistemic_state":"indeterminate","statement":"Evidence output exceeded its bound. Preserve the original custody and reconcile; never redispatch."})])
        .coverage(json!({"status":"unavailable","omitted_domains":["full_native_evidence"]})).references(references).build();
    json!({"agent_turn":minimal,"result":{"ok":false,"error":{"code":"budget_exceeded"},"custody":reference},"retained_reference":reference}).to_string()
}
fn failed(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    e: dfmcp_core::DfmcpError,
    reference: Value,
) -> String {
    packet(
        operation,
        session,
        request,
        json!({"ok":false,"error":{"code":e.code.as_str(),"message":e.message},"current_goal_available":false,"automatic_redispatch":false}),
        reference,
    )
}
fn parse_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid manager session ID",
        ));
    }
    let n = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid manager session ID"))?;
    let id = SessionId::new(n);
    if id.get() != n || !id.is_process_scoped_live() || (n & ((1u128 << 62) - 1)) >> 57 != 22 {
        return Err(error(
            ErrorCode::SessionNotFound,
            "foreign manager session handle",
        ));
    }
    Ok(id)
}
fn call(operation: &str, id: String, action: Action) -> String {
    let session = match parse_id(&id) {
        Ok(v) => v,
        Err(e) => return failed(operation, None, None, e, Value::Null),
    };
    let mut guard = match SESSION.try_lock() {
        Ok(v) => v,
        Err(_) => {
            return failed(
                operation,
                Some(session),
                None,
                error(
                    ErrorCode::BudgetExceeded,
                    "manager session busy or poisoned",
                ),
                Value::Null,
            );
        }
    };
    let Some(runtime) = guard.as_mut().filter(|r| r.session == session) else {
        return failed(
            operation,
            Some(session),
            None,
            error(
                ErrorCode::SessionNotFound,
                "original manager session unavailable",
            ),
            Value::Null,
        );
    };
    let result = runtime.perform(action);
    let request = Some(runtime.last_request_id());
    let reference = runtime.reference();
    match result {
        Ok(value) => packet(operation, Some(session), request, value, reference),
        Err(e) => failed(operation, Some(session), request, e, reference),
    }
}
#[tool(
    name = "fortress_open_session",
    description = "Open one isolated manager/1.22 vacant MANAGER appointment. Default offline. Control requires host source/candidate pins, disposable-fortress permission and fresh journal/ACK custody. Opening never appoints or advances time."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present({
        let result = (|| {
            let start = std::time::Instant::now();
            runtime::runtime_io()?;
            let mode = match mode.as_deref().unwrap_or("offline") {
                "control" => Mode::Control,
                "recover" => Mode::Recover,
                "offline" => Mode::Offline,
                _ => return Err(error(ErrorCode::InvalidRequest, "unknown manager mode")),
            };
            let budget = WorkBudget {
                max_wall_millis: max_wall_millis.unwrap_or(60000),
                max_bytes: max_bytes.unwrap_or(256 * 1024 * 1024),
                max_output_tokens: max_output_tokens.unwrap_or(131072),
                max_entities: 4096,
                max_actions: 1,
                max_game_ticks: 0,
            };
            budget.validate()?;
            if budget.max_wall_millis > 60000
                || budget.max_bytes < 64 * 1024 * 1024
                || budget.max_bytes > 512 * 1024 * 1024
                || budget.max_output_tokens < 131072
                || budget.max_output_tokens > 131072
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "manager profile work/output limits",
                ));
            }
            let config = Config::load()?;
            let mut guard = SESSION
                .try_lock()
                .map_err(|_| error(ErrorCode::Conflict, "manager session busy"))?;
            if guard.is_some() {
                return Err(error(
                    ErrorCode::Conflict,
                    "close the original manager session first",
                ));
            }
            let sequence = NEXT
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < (1u64 << 57)).then_some(n + 1)
                })
                .map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "manager session identities exhausted",
                    )
                })?;
            let session = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
            let (runtime, value) = Runtime::open(config, mode, session, budget, start)?;
            *guard = Some(runtime);
            let runtime = guard.as_mut().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "manager runtime not retained",
                )
            })?;
            let reference = runtime.reference();
            let request = Some(runtime.last_request_id());
            Ok(packet(
                "fortress.open_session",
                Some(session),
                request,
                value,
                reference,
            ))
        })();
        match result {
            Ok(v) => v,
            Err(e) => failed("fortress.open_session", None, None, e, Value::Null),
        }
    })
}
#[tool(
    name = "fortress_observe",
    description = "Read one host-offered current Expedition Leader, vacant MANAGER assignment, group/civ/HF/history projection and dual-getter evidence."
)]
pub fn fortress_observe(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.observe", session_id, Action::Observe))
}
#[tool(
    name = "fortress_query",
    description = "Verify and inspect original local manager custody; no native call, designation or clock."
)]
pub fn fortress_query(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.query", session_id, Action::Query))
}
#[tool(
    name = "fortress_plan",
    description = "Prepare one original vacant MANAGER appointment. Closed request: kind=appoint_vacant_manager, expected_observation, deadline_tick within 12000 ticks. Returns one review seal without game writes; no role or candidate selector."
)]
pub fn fortress_plan(
    session_id: String,
    request: String,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.plan", session_id, Action::Plan(request)))
}
#[tool(
    name = "fortress_commit",
    description = "Confirm the exact original manager seal once on its prepared connection. Journal and independent ACK sync before the full native appointment transaction. Lost replies never permit another writer."
)]
pub fn fortress_commit(
    session_id: String,
    review_seal: String,
    confirmed: bool,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call(
        "fortress.commit",
        session_id,
        Action::Commit(review_seal, confirmed),
    ))
}
#[tool(
    name = "fortress_wait",
    description = "One foreground original-key reconciliation. No redispatch, polling loop or clock. Success requires the complete assignment/HF/history projection and both getter proofs."
)]
pub fn fortress_wait(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.wait", session_id, Action::Wait))
}
#[tool(
    name = "fortress_cancel",
    description = "scope=goal retires only undispatched native preparation or inspects original uncertainty. Never removes or replaces an appointed Manager. scope=session releases local sockets and locks while preserving custody."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(cancel_packet(session_id, scope))
}

fn cancel_packet(session_id: String, scope: Option<String>) -> String {
    if scope.as_deref().unwrap_or("goal") == "goal" {
        return call("fortress.cancel", session_id, Action::Cancel);
    }
    if scope.as_deref() != Some("session") {
        return failed(
            "fortress.cancel",
            None,
            None,
            error(
                ErrorCode::InvalidRequest,
                "unknown manager cancellation scope",
            ),
            Value::Null,
        );
    }
    let result = (|| {
        let session = parse_id(&session_id)?;
        let mut guard = SESSION
            .try_lock()
            .map_err(|_| error(ErrorCode::Conflict, "manager session busy"))?;
        let runtime = guard
            .as_ref()
            .filter(|r| r.session == session)
            .ok_or_else(|| error(ErrorCode::SessionNotFound, "manager session unavailable"))?;
        let reference = runtime.reference();
        *guard = None;
        Ok(packet(
            "fortress.cancel",
            Some(session),
            None,
            json!({"ok":true,"session_closed":true,"automatic_redispatch":false}),
            reference,
        ))
    })();
    match result {
        Ok(v) => v,
        Err(e) => failed("fortress.cancel", None, None, e, Value::Null),
    }
}

#[tool(
    name = "fortress_explain",
    description = "Inspect original manager plan, exact assignment/HF/history evidence and uncertainty and recovery identity without native work."
)]
pub fn fortress_explain(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.explain", session_id, Action::Query))
}
#[tool(
    name = "fortress_doctor",
    description = "Inspect original manager custody and explicit unadmitted limitations without native work."
)]
pub fn fortress_doctor(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.doctor", session_id, Action::Query))
}
#[tool(
    name = "fortress_checkpoint",
    description = "This isolated profile has no checkpoint provider; retain the original custody and operator-owned disposable-save evidence."
)]
pub fn fortress_checkpoint(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(unsupported("fortress.checkpoint", session_id))
}
#[tool(
    name = "fortress_restore",
    description = "This isolated profile cannot restore a world or carry original manager identity across epochs. Preserve and reconcile original custody."
)]
pub fn fortress_restore(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(unsupported("fortress.restore", session_id))
}
fn unsupported(operation: &str, id: String) -> String {
    let session = parse_id(&id).ok();
    let mut reference = Value::Null;
    if let Ok(mut guard) = SESSION.try_lock()
        && let Some(runtime) = guard.as_mut().filter(|r| Some(r.session) == session)
    {
        runtime.withdraw_permission();
        reference = runtime.reference();
    }
    failed(
        operation,
        session,
        None,
        error(
            ErrorCode::CapabilityDenied,
            "manager profile has no checkpoint/restore provider",
        ),
        reference,
    )
}
pub fn run_stdio() {
    if let Err(e) = runtime::environment() {
        eprintln!("{}: {}", e.code.as_str(), e.message);
        return;
    }
    let server=ServerBuilder::new("dfmcp-manager-appointment-v1-22-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit).tool(FortressWait)
        .tool(FortressCancel).tool(FortressExplain).tool(FortressDoctor).tool(FortressCheckpoint).tool(FortressRestore)
        .instructions("Unadmitted manager/1.22: one current Expedition Leader for vacant MANAGER. Observe, prepare, review and confirm once, reconcile original key. Independent ACK precedes the full native assignment/HF/history transaction. Success requires exact poststate and dual getters with unchanged tick. No displacement, arbitrary roles, clock, order activation or production admission. Original .14 and .14.2 remain open for independent verification.").build();
    crate::run_modern_stdio(server);
}
