#![forbid(unsafe_code)]
//! Dedicated, explicitly unadmitted tree-harvest/1.21 modern stdio consumer.
mod clock_presentation;
mod presentation;
mod runtime;
#[cfg(test)]
mod server_tests;
use crate::agent_turn::{AgentPhase, AgentTurnBuilder, ContinuityStatus, recommendation};
use dfmcp_adapter::tree_harvest::journal::Mode;
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
const FAMILY: u128 = 31u128 << 57;
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
        Some(json!({"canonical_world_continuity":"unestablished","canonical_world_anchor_available":false,"stable_plant_id_available":false})),None);
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
    let active = json!({"key":original["key"],"plan_digest":original["plan_digest"],"review_seal":original["review_seal"],"deadline_tick":original["deadline_tick"],"state":original["state"],"stable_plant_id":null});
    let mut next = Vec::new();
    if let Some(session) = session
        && result["session_closed"] != true
        && (verified || reference.is_object())
    {
        let mut args = json!({"session_id":session.to_string()});
        let (tool, reason) = if !verified || result["mode"] == "offline" {
            (
                "fortress_query",
                "Verify original local custody without native effects.",
            )
        } else if prepared {
            args["review_seal"] = original["review_seal"].clone();
            args["confirmed"] = json!(true);
            (
                "fortress_commit",
                "Review full root, body, source, worker, tool offer, scope and deadline before confirming this exact seal.",
            )
        } else if original.is_object() {
            if original["historical_supply_observed_tick"].is_number()
                || original["continuity_lost"] == true
                || original["sample_publication_pending"] == true
                || matches!(original["native_phase"].as_u64(), Some(3 | 4))
            {
                (
                    "fortress_explain",
                    "Inspect original terminal evidence; do not replace the key or imply more waiting can restore continuity.",
                )
            } else {
                (
                    "fortress_wait",
                    "Reconcile the original key or acquire one foreground progress sample; never redispatch or start a clock.",
                )
            }
        } else if result["observation"]["eligible_at_anchor"] == true {
            let request=json!({"kind":"fell_one_visible_tree","expected_observation":result["observation"]["witness"],"deadline_tick":result["recommended_deadline_tick"]}).to_string();
            args["request"] = json!(request);
            (
                "fortress_plan",
                "Prepare this exact coordinate-predicate offer for review without changing the game.",
            )
        } else if result["observation"].is_object() {
            (
                "fortress_explain",
                "Inspect the observed eligibility blockers; tree identity and future scheduling remain unproved.",
            )
        } else {
            (
                "fortress_observe",
                "Acquire a fresh bounded tree, approach, worker and tool-offer witness.",
            )
        };
        let mut rec = recommendation(
            "continue_original_tree_supply",
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
                "deferred felling may be irreversible; no automatic undo"
            } else {
                "not_applicable"
            },
            prepared,
            args,
        );
        rec["confidence"]["evidence"] = json!([reference.clone()]);
        rec["estimated_cost"] = json!({"game_ticks":0,"native_writer_calls_max":u8::from(prepared),"output_bytes_max":RESPONSE_BYTES});
        rec["invalidating_conditions"] = json!([
            "source/epoch/tick/root/species/body/approach/worker/tool/reference changes",
            "lease or authority expiry",
            "lost preparation connection",
            "changed journal or independent ACK"
        ]);
        rec["grants_authority"] = json!(false);
        next.push(rec);
    }
    let mut turn=spine(operation,session,request).briefing(json!({"profile":"tree-harvest/1.21","runtime_admitted":false,"mutation_admissible":false,
        "development_commit_available":prepared,"canonical_world_anchor_available":false,"stable_plant_id":null,"tree_identity_continuity":"unknown",
        "historical_supply_observed_tick":original["historical_supply_observed_tick"],"causal_item_provenance":"unknown","clock_advanced":false}))
        .active_work(json!({"pending_plans":if prepared{vec![active.clone()]}else{vec![]},"actions":[],"obligations":if original.is_object(){vec![active.clone()]}else{vec![]},
            "cancellation_drains":[],"indeterminate_effects":indeterminate.clone(),"publications":[],
            "confirmations":if prepared{vec![json!({"review_seal":original["review_seal"],"required":true})]}else{vec![]}}))
        .recommendations(next).uncertainty(vec![json!({"uncertainty_id":"tree_predicate","epistemic_state":"unknown",
            "statement":"A designation ACK is not a felled tree or wood. There is no stable plant ID. Tool offers do not establish equipment, assignment or scheduling. Root-predicate absence and new eligible ground wood are observed separately; causal item provenance remains unknown."}),
            json!({"uncertainty_id":"future_effects","epistemic_state":"unknown","statement":"Designation authority and local lease cover the observed tree/halo and touched scheduling block, without globally fencing other controllers. Visible body checks do not certify future falling-tree safety. Broad world evolution requires separately authorized clock exposure. Original .14 remains open."})])
        .coverage(json!({"status":"partial","source":"tree-harvest/1.21","canonical_world_adapter":false,"stable_plant_id_available":false,"retained_custody_verified":verified}))
        .budget(json!({"max_output_bytes":RESPONSE_BYTES,"max_game_ticks":0})).references(references.clone()).build();
    if let Some(b) = turn["briefing"].as_object_mut() {
        b.remove("admission");
    }
    turn["budget"]["max_game_ticks"] = result.get("max_game_ticks").cloned().unwrap_or(json!(0));
    clock_presentation::augment(&mut turn, &result, &reference, session);
    let out=json!({"agent_turn":turn,"result":result,"retained_reference":reference,"retained_reference_current":false}).to_string();
    if out.len() as u64 <= RESPONSE_BYTES {
        return out;
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
    let mut minimal=spine(operation,session,request).briefing(json!({"runtime_admitted":false,"mutation_admissible":false,"current_goal_available":false,"canonical_world_anchor_available":false}))
        .active_work(json!({"pending_plans":[],"actions":[],"obligations":[],"cancellation_drains":[],"indeterminate_effects":indeterminate,"publications":[],"confirmations":[]}))
        .uncertainty(vec![json!({"uncertainty_id":"output_bound","epistemic_state":"indeterminate","statement":"Evidence output exceeded its bound. Preserve the original custody and reconcile; never redispatch."})])
        .coverage(json!({"status":"unavailable","omitted_domains":["full_native_evidence"]})).references(references).build();
    clock_presentation::augment(&mut minimal, &json!({"ok":false}), &reference, session);
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
        return Err(error(ErrorCode::InvalidRequest, "invalid tree session ID"));
    }
    let n = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid tree session ID"))?;
    let id = SessionId::new(n);
    if id.get() != n || !id.is_process_scoped_live() || (n & ((1u128 << 62) - 1)) >> 57 != 31 {
        return Err(error(
            ErrorCode::SessionNotFound,
            "foreign tree session handle",
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
                error(ErrorCode::BudgetExceeded, "tree session busy or poisoned"),
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
                "original tree session unavailable",
            ),
            Value::Null,
        );
    };
    let result = runtime.perform(action);
    let request = Some(runtime.last_request_id());
    let reference = runtime.reference();
    match result {
        Ok(value) => {
            let raw = packet(operation, Some(session), request, value, reference);
            if serde_json::from_str::<Value>(&raw).is_ok_and(|v| v["result"]["ok"] == true)
                && presentation::publishable(&raw)
            {
                raw
            } else {
                // A review that could not be published must not remain usable.
                runtime.withdraw_permission();
                overflow_packet(operation, Some(session), request, runtime.reference())
            }
        }
        Err(e) => failed(operation, Some(session), request, e, reference),
    }
}

pub(super) fn withdraw_unpublished_review(session: Option<SessionId>, retained: &Value) -> Value {
    if let Ok(mut guard) = SESSION.try_lock()
        && let Some(runtime) = guard
            .as_mut()
            .filter(|runtime| Some(runtime.session) == session)
    {
        runtime.withdraw_permission();
        runtime.reference()
    } else {
        retained.clone()
    }
}
#[tool(
    name = "fortress_open_session",
    description = "Open one isolated tree/1.21 root-predicate obligation. Default offline. Control requires explicit fortress-registry read scope, host-bounded tree/block effect scope, source pins, disposable-fortress permission and fresh journal/ACK custody. An optional independently configured clock requires explicit max_game_ticks (1..1200); its default is zero. Byte-work defaults to 512MiB with a clock, otherwise 256MiB. Opening never designates or advances time."
)]
pub fn fortress_open_session(
    mode: Option<String>,
    max_wall_millis: Option<u64>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_game_ticks: Option<u64>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present({
        let result = (|| {
            let start = std::time::Instant::now();
            runtime::runtime_io()?;
            let mode = match mode.as_deref().unwrap_or("offline") {
                "control" => Mode::Control,
                "recover" => Mode::Recover,
                "offline" => Mode::Offline,
                _ => return Err(error(ErrorCode::InvalidRequest, "unknown tree mode")),
            };
            let config = Config::load()?;
            let budget = WorkBudget {
                max_wall_millis: max_wall_millis.unwrap_or(60000),
                max_bytes: max_bytes.unwrap_or(if config.has_clock() {
                    512 * 1024 * 1024
                } else {
                    256 * 1024 * 1024
                }),
                max_output_tokens: max_output_tokens.unwrap_or(131072),
                max_entities: 4096,
                max_actions: 1,
                max_game_ticks: max_game_ticks.unwrap_or(0),
            };
            budget.validate()?;
            if budget.max_wall_millis > 60000
                || budget.max_bytes < 64 * 1024 * 1024
                || budget.max_bytes > 512 * 1024 * 1024
                || budget.max_output_tokens < 131072
                || budget.max_output_tokens > 131072
                || budget.max_game_ticks > 1200
                || (budget.max_game_ticks > 0 && !config.has_clock())
            {
                return Err(error(
                    ErrorCode::BudgetExceeded,
                    "tree profile work/output limits",
                ));
            }
            let mut guard = SESSION
                .try_lock()
                .map_err(|_| error(ErrorCode::Conflict, "tree session busy"))?;
            if guard.is_some() {
                return Err(error(
                    ErrorCode::Conflict,
                    "close the original tree session first",
                ));
            }
            let sequence = NEXT
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < (1u64 << 57)).then_some(n + 1)
                })
                .map_err(|_| {
                    error(
                        ErrorCode::BudgetExceeded,
                        "tree session identities exhausted",
                    )
                })?;
            let session = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
            let (runtime, value) = Runtime::open(config, mode, session, budget, start)?;
            *guard = Some(runtime);
            let runtime = guard.as_mut().ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "tree runtime not retained",
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
    description = "Read one exact visible root, computed southeast designation, complete bounded visible body, approach, CUTWOOD worker and axe-offer witness. No stable plant ID or scheduling guarantee."
)]
pub fn fortress_observe(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.observe", session_id, Action::Observe))
}
#[tool(
    name = "fortress_query",
    description = "Verify and inspect original local tree custody; no native call, designation or clock."
)]
pub fn fortress_query(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.query", session_id, Action::Query))
}
#[tool(
    name = "fortress_plan",
    description = "Prepare one original root-predicate intent: kind=fell_one_visible_tree, expected_observation, deadline_tick within12000 ticks. After acknowledged designation, kind=run_tree with target_id, game_ticks (1..1200), wall_millis (1..60000) prepares one independently authorized finite interval under the same original deadline and remaining sample capacity. Each exact seal needs separate confirmation. Planning never advances time."
)]
pub fn fortress_plan(
    session_id: String,
    request: String,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let clock = request.len() <= 4096
        && serde_json::from_str::<Value>(&request).is_ok_and(|value| value["kind"] == "run_tree");
    presentation::present(call(
        "fortress.plan",
        session_id,
        if clock {
            Action::ClockPlan(request)
        } else {
            Action::Plan(request)
        },
    ))
}
#[tool(
    name = "fortress_commit",
    description = "Confirm an exact current review. Default scope=goal designates the original tree once on its prepared connection after durable journal and independent ACK. scope=clock confirms only the separately prepared finite Tree clock interval; fresh original Tree evidence is checked after its durable dispatch marker. Lost replies never authorize another designation or unpause."
)]
pub fn fortress_commit(
    session_id: String,
    review_seal: String,
    confirmed: bool,
    scope: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let action = match scope.as_deref().unwrap_or("goal") {
        "goal" => Action::Commit(review_seal, confirmed),
        "clock" => Action::ClockCommit(review_seal, confirmed),
        _ => {
            return presentation::present(rejected(
                "fortress.commit",
                session_id,
                error(ErrorCode::InvalidRequest, "unknown Tree commit scope"),
            ));
        }
    };
    presentation::present(call("fortress.commit", session_id, action))
}
#[tool(
    name = "fortress_wait",
    description = "Default scope=goal performs one foreground original designation reconciliation or fresh root/job/new-ground-WOOD sample. scope=clock reconciles the retained clock key once and, after settlement, obtains fresh original Tree progress. An unresolved clock blocks goal sampling. No polling loop, redispatch or new unpause. Designation, clock settlement, root absence and supply remain separate claims."
)]
pub fn fortress_wait(
    session_id: String,
    scope: Option<String>,
) -> McpResult<CompleteResult<FinalCallToolResult>> {
    let action = match scope.as_deref().unwrap_or("goal") {
        "goal" => Action::Wait,
        "clock" => Action::ClockWait,
        _ => {
            return presentation::present(rejected(
                "fortress.wait",
                session_id,
                error(ErrorCode::InvalidRequest, "unknown Tree wait scope"),
            ));
        }
    };
    presentation::present(call("fortress.wait", session_id, action))
}
#[tool(
    name = "fortress_cancel",
    description = "scope=goal retires only undispatched Tree preparation or inspects original uncertainty. scope=clock retires its local preparation or requests a separately authorized safety pause of the retained run. Never undesignates or undoes felling. scope=session releases local sockets and locks while preserving both custody families."
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
    if scope.as_deref() == Some("clock") {
        return call("fortress.cancel", session_id, Action::ClockCancel);
    }
    if scope.as_deref() != Some("session") {
        return rejected(
            "fortress.cancel",
            session_id,
            error(ErrorCode::InvalidRequest, "unknown tree cancellation scope"),
        );
    }
    let result = (|| {
        let session = parse_id(&session_id)?;
        let mut guard = SESSION
            .try_lock()
            .map_err(|_| error(ErrorCode::Conflict, "tree session busy"))?;
        let runtime = guard
            .as_ref()
            .filter(|r| r.session == session)
            .ok_or_else(|| error(ErrorCode::SessionNotFound, "tree session unavailable"))?;
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
    description = "Inspect original tree plan, exact evidence, tool/scheduling uncertainty and recovery identity without native work."
)]
pub fn fortress_explain(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(call("fortress.explain", session_id, Action::Query))
}
#[tool(
    name = "fortress_doctor",
    description = "Inspect original tree custody and explicit unadmitted limitations without native work."
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
    description = "This isolated profile cannot restore a world or carry original tree identity across epochs. Preserve and reconcile original custody."
)]
pub fn fortress_restore(session_id: String) -> McpResult<CompleteResult<FinalCallToolResult>> {
    presentation::present(unsupported("fortress.restore", session_id))
}
fn unsupported(operation: &str, id: String) -> String {
    rejected(
        operation,
        id,
        error(
            ErrorCode::CapabilityDenied,
            "tree profile has no checkpoint/restore provider",
        ),
    )
}
fn rejected(operation: &str, id: String, e: dfmcp_core::DfmcpError) -> String {
    let session = parse_id(&id).ok();
    let mut reference = Value::Null;
    if let Ok(mut guard) = SESSION.try_lock()
        && let Some(runtime) = guard.as_mut().filter(|r| Some(r.session) == session)
    {
        runtime.withdraw_permission();
        reference = runtime.reference();
    }
    failed(operation, session, None, e, reference)
}
pub fn run_stdio() {
    if let Err(e) = runtime::environment() {
        eprintln!("{}: {}", e.code.as_str(), e.message);
        return;
    }
    let server=ServerBuilder::new("dfmcp-tree-harvest-v1-21-dev",env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan).tool(FortressCommit).tool(FortressWait)
        .tool(FortressCancel).tool(FortressExplain).tool(FortressDoctor).tool(FortressCheckpoint).tool(FortressRestore)
        .instructions("Unadmitted tree/1.21: one coordinate/source predicate, no stable plant ID. Observe, plan, review, confirm once, reconcile/prove foreground. markPlant ACK is not wood. Tools are offers, never assigned equipment. Independent ACK precedes dispatch; uncertainty never permits new keys. An optional independent Run/1.13 clock uses explicit session ticks and kind=run_tree with the original target_id. Review and confirm each interval with scope=clock, then Wait scope=clock once and obtain fresh Tree evidence after settlement. Every interval inherits the original deadline and needs enough remaining samples; time or a Run receipt never proves harvesting. Default tick allowance is zero. Retain both original Tree ACK and clock recovery references through failures, close and Offline recovery. No workshop construction or production admission. Original .14 remains open.").build();
    crate::run_modern_stdio(server);
}
