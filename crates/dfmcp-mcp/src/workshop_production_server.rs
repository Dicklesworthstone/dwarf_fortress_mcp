#![forbid(unsafe_code)]
//! One guarded session for original workshop construction followed by production.
//! Child owners retain their native protocols, journals and review boundaries.

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, empty_active_work, recommendation,
};
use crate::live_production_server::{self as production, Action as ProductionAction};
use crate::semantic_workforce_runtime::Mode;
use crate::workshop_construction_server::runtime::Action as WorkshopAction;
use crate::workshop_production_config::{Config, Stage};
use crate::workshop_production_runtime::Runtime;
use dfmcp_core::{
    DfmcpError, ErrorCode, OperationContext, RequestId, Result, SessionId, WorkBudget,
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

const OPT_IN: &str = "DFMCP_ALLOW_UNADMITTED_WORKSHOP_PRODUCTION";
const RESPONSE_RESERVE: u64 = 128 * 1024;
const OPEN_INSPECTION_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WIRE_BYTES: usize = 60_000;
const FAMILY: u128 = 1u128 << 56;
const SEQUENCE_LIMIT: u64 = 1u64 << 56;
static NEXT: AtomicU64 = AtomicU64::new(1);
static SESSION: Mutex<Option<State>> = Mutex::new(None);

struct State {
    runtime: Runtime,
    budget: WorkBudget,
    workshop_view: Option<Value>,
    production_view: Option<Value>,
    uncertain: bool,
}

fn error(code: ErrorCode, message: &str) -> DfmcpError {
    DfmcpError::new(code, message)
}

pub(crate) fn runtime_io() -> Result<()> {
    if std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some()
        || crate::current_admission_provenance().is_some()
    {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "workshop production refuses production admission",
        ));
    }
    let cx = asupersync::Cx::current().ok_or_else(|| {
        error(
            ErrorCode::CapabilityDenied,
            "workshop production requires its owned runtime context",
        )
    })?;
    cx.checkpoint().map_err(|_| {
        error(
            ErrorCode::CancellationRequested,
            "workshop production request cancelled",
        )
    })?;
    if !cx.capabilities().io {
        return Err(error(
            ErrorCode::CapabilityDenied,
            "inherited context denies workshop production I/O",
        ));
    }
    Ok(())
}

fn lock() -> Result<MutexGuard<'static, Option<State>>> {
    SESSION.try_lock().map_err(|cause| match cause {
        TryLockError::WouldBlock => error(
            ErrorCode::BudgetExceeded,
            "another request owns this staged session",
        ),
        TryLockError::Poisoned(_) => error(
            ErrorCode::InternalInvariantViolation,
            "staged session poisoned; preserve every child recovery reference",
        ),
    })
}

fn session_id(raw: &str) -> Result<SessionId> {
    if raw.len() != 32
        || !raw
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid staged session ID",
        ));
    }
    let value = u128::from_str_radix(raw, 16)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid staged session ID"))?;
    let id = SessionId::new(value);
    if id.get() != value || !id.is_process_scoped_live() || (value & ((1u128 << 62) - 1)) >> 56 != 1
    {
        return Err(error(
            ErrorCode::SessionNotFound,
            "session is not a staged process owner",
        ));
    }
    Ok(id)
}

fn stage_name(stage: Stage) -> &'static str {
    match stage {
        Stage::Workshop => "workshop",
        Stage::Transition => "transition",
        Stage::Production => "production",
    }
}

fn budgets(
    wall: Option<u64>,
    bytes: Option<u64>,
    output: Option<u32>,
    ticks: Option<u64>,
) -> Result<(WorkBudget, WorkBudget)> {
    let display = WorkBudget {
        max_wall_millis: wall.unwrap_or(60_000),
        max_bytes: bytes.unwrap_or(8 * 1024 * 1024 * 1024),
        max_entities: 8192,
        max_actions: 5,
        max_output_tokens: output.unwrap_or(32_768),
        max_game_ticks: ticks.unwrap_or(0),
    };
    display.validate()?;
    if display.max_wall_millis > 60_000
        || display.max_bytes > 32 * 1024 * 1024 * 1024
        || display.max_output_tokens > 32_768
        || display.max_game_ticks > 1200
        || u64::from(display.max_output_tokens).saturating_mul(4) < RESPONSE_RESERVE
    {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "staged session requires complete output reservation and bounded bytes, wall time and explicit clock ticks",
        ));
    }
    let mut work = display;
    work.max_bytes = display
        .max_bytes
        .checked_sub(RESPONSE_RESERVE + OPEN_INSPECTION_BYTES)
        .filter(|n| *n >= 1024 * 1024 * 1024)
        .ok_or_else(|| {
            error(
                ErrorCode::BudgetExceeded,
                "both owners and complete recovery output must fit before opening",
            )
        })?;
    Ok((display, work))
}

fn remaining(started: Instant, wall: u64) -> Result<u64> {
    let elapsed = u64::try_from(started.elapsed().as_millis())
        .map_err(|_| error(ErrorCode::BudgetExceeded, "staged deadline overflow"))?;
    wall.checked_sub(elapsed)
        .filter(|n| *n > 0)
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "staged request deadline expired"))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BeginProduction {
    kind: String,
}

#[derive(Deserialize)]
struct KindProbe {
    #[serde(default, deserialize_with = "present_kind")]
    kind: Option<String>,
}

fn present_kind<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

fn is_begin_production(raw: &str) -> Result<bool> {
    if raw.is_empty() || raw.len() > 8192 {
        return Err(error(
            ErrorCode::InvalidRequest,
            "plan request must contain 1..8192 UTF-8 bytes",
        ));
    }
    let value: Value = serde_json::from_str(raw)
        .map_err(|_| error(ErrorCode::InvalidRequest, "invalid staged plan request"))?;
    if !value.is_object() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "staged plan request must be an object",
        ));
    }
    let probe: KindProbe = serde_json::from_str(raw).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "plan kind must be unique and well formed",
        )
    })?;
    if probe.kind.as_deref() != Some("begin_production") {
        return Ok(false);
    }
    let request: BeginProduction = serde_json::from_str(raw).map_err(|_| {
        error(
            ErrorCode::InvalidRequest,
            "begin_production accepts only its kind",
        )
    })?;
    Ok(request.kind == "begin_production")
}

enum Command {
    Observe(Option<String>),
    Query {
        mode: Option<String>,
        request: Option<String>,
        step: Option<u32>,
    },
    Plan(String),
    Commit {
        seal: String,
        workshop: bool,
        confirmed: bool,
        scope: Option<String>,
        step: Option<u32>,
    },
    Wait(Option<String>),
    Cancel {
        scope: Option<String>,
        step: Option<u32>,
    },
    Inspect,
    Unavailable,
}
impl Command {
    fn may_dispatch(&self) -> bool {
        matches!(self, Self::Commit { .. })
            || matches!(self, Self::Cancel { scope: Some(scope), .. }
                if matches!(scope.as_str(), "clock" | "run" | "inventory_run"))
    }
    fn perform(self, runtime: &mut Runtime, c: &OperationContext) -> Result<Value> {
        let stage = runtime.stage();
        match self {
            Self::Inspect => runtime.inspect(c),
            Self::Unavailable => Err(error(
                ErrorCode::CapabilityDenied,
                "this development session cannot checkpoint or restore the game",
            )),
            Self::Plan(raw) if is_begin_production(&raw)? => runtime.begin_production(c),
            Self::Plan(raw) => match stage {
                Stage::Workshop => runtime.workshop_action(WorkshopAction::Plan(raw), c),
                Stage::Production => runtime.production_action(production::plan_action(&raw)?, c),
                Stage::Transition => Err(error(
                    ErrorCode::Conflict,
                    "finish the retained begin_production handoff before planning",
                )),
            },
            Self::Observe(request) => match stage {
                Stage::Workshop if request.is_none() => {
                    runtime.workshop_action(WorkshopAction::Observe, c)
                }
                Stage::Production => {
                    let request = match request {
                        Some(raw) => crate::live_production_runtime::ObserveRequest::parse(&raw)?,
                        None => crate::live_production_runtime::ObserveRequest::default(),
                    };
                    runtime.production_action(ProductionAction::Observe(request), c)
                }
                _ => Err(error(
                    ErrorCode::InvalidRequest,
                    "selected workforce observation is available only in the production stage",
                )),
            },
            Self::Query {
                mode,
                request,
                step,
            } => match stage {
                Stage::Workshop if request.is_none() && step.is_none() => {
                    let action = match mode.as_deref().unwrap_or("inspect") {
                        "inspect" => WorkshopAction::Inspect,
                        "production_handoff" => WorkshopAction::ProductionHandoff,
                        _ => {
                            return Err(error(
                                ErrorCode::InvalidRequest,
                                "workshop query mode must be inspect or production_handoff",
                            ));
                        }
                    };
                    runtime.workshop_action(action, c)
                }
                Stage::Production if mode.as_deref().is_none_or(|m| m == "inspect") => {
                    let action = match request {
                        Some(raw) => ProductionAction::InspectProduction(
                            crate::live_production_runtime::InspectionRequest::parse(&raw)?,
                            step,
                        ),
                        None => {
                            step.map_or(ProductionAction::Inspect, ProductionAction::InspectOrder)
                        }
                    };
                    runtime.production_action(action, c)
                }
                Stage::Transition if mode.is_none() && request.is_none() && step.is_none() => {
                    runtime.inspect(c)
                }
                _ => Err(error(
                    ErrorCode::InvalidRequest,
                    "query selectors do not belong to the current stage",
                )),
            },
            Self::Commit {
                seal,
                workshop,
                confirmed,
                scope,
                step,
            } => match stage {
                Stage::Workshop if workshop && step.is_none() && scope.is_none() => {
                    runtime.workshop_action(WorkshopAction::Commit { seal, confirmed }, c)
                }
                Stage::Production if !workshop => runtime.production_action(
                    production::commit_action(scope.as_deref(), &seal, confirmed, step)?,
                    c,
                ),
                _ => Err(error(
                    ErrorCode::InvalidRequest,
                    "no matching current-stage effect review",
                )),
            },
            Self::Wait(scope) => match stage {
                Stage::Workshop => {
                    let action = match scope.as_deref().unwrap_or("goal") {
                        "goal" => WorkshopAction::Wait,
                        "clock" => WorkshopAction::WaitClock,
                        _ => {
                            return Err(error(
                                ErrorCode::InvalidRequest,
                                "workshop wait scope must be goal or clock",
                            ));
                        }
                    };
                    runtime.workshop_action(action, c)
                }
                Stage::Production if scope.as_deref().is_none_or(|s| s == "goal") => {
                    runtime.production_action(ProductionAction::Wait, c)
                }
                Stage::Transition if scope.is_none() => runtime.inspect(c),
                _ => Err(error(
                    ErrorCode::InvalidRequest,
                    "wait scope does not belong to the current stage",
                )),
            },
            Self::Cancel { scope, step } => match stage {
                Stage::Workshop if step.is_none() => {
                    let action = match scope.as_deref().unwrap_or("goal") {
                        "goal" => WorkshopAction::Cancel,
                        "clock" => WorkshopAction::CancelClock,
                        _ => {
                            return Err(error(
                                ErrorCode::InvalidRequest,
                                "workshop cancel scope must be goal or clock",
                            ));
                        }
                    };
                    runtime.workshop_action(action, c)
                }
                Stage::Production => {
                    runtime.production_action(production::cancel_action(scope.as_deref(), step)?, c)
                }
                _ => Err(error(
                    ErrorCode::InvalidRequest,
                    "transition can be inspected, retried read-only, or closed",
                )),
            },
        }
    }
}

fn fits(raw: &str, budget: WorkBudget) -> bool {
    let wire = json!({"jsonrpc":"2.0","id":u64::MAX,
        "result":{"content":[{"type":"text","text":raw}],"isError":false}});
    let bytes = wire.to_string().len();
    bytes <= MAX_WIRE_BYTES
        && bytes as u64 <= budget.max_bytes
        && (bytes as u64).div_ceil(4) <= u64::from(budget.max_output_tokens)
}

fn phase(operation: &str) -> AgentPhase {
    match operation {
        "fortress.open_session" => AgentPhase::Bootstrap,
        "fortress.observe" => AgentPhase::Orient,
        "fortress.plan" => AgentPhase::Propose,
        "fortress.commit" => AgentPhase::Commit,
        "fortress.wait" => AgentPhase::Verify,
        "fortress.cancel" => AgentPhase::Reconcile,
        _ => AgentPhase::Inspect,
    }
}

fn failure_value(cause: &DfmcpError, uncertain: bool) -> Value {
    json!({"ok":false,"error":{"code":cause.code.as_str(),"message":cause.message,"retryable":false},
        "runtime_admitted":false,"automatic_redispatch":false,
        "effect_may_have_occurred":uncertain,"current_goal_available":false,
        "retained_state_currently_verified":false,"current_state_proven":false,"original_goal_proven":false})
}

fn fallback(
    operation: &str,
    session: Option<SessionId>,
    request: Option<RequestId>,
    budget: WorkBudget,
    stage: Option<Stage>,
    recovery: Value,
    cause: &DfmcpError,
    uncertain: bool,
) -> String {
    let mut characters = cause.message.chars();
    let mut message: String = characters.by_ref().take(256).collect();
    let diagnostic_truncated = characters.next().is_some();
    if diagnostic_truncated {
        message.push_str("…");
    }
    let cause = error(cause.code, &message);
    let mut active = empty_active_work();
    if uncertain {
        active["indeterminate_effects"] = json!([{"scope":"staged_originals",
            "recovery_reference":"workflow","automatic_redispatch":false}]);
    }
    let mut turn = AgentTurnBuilder::new(operation, phase(operation))
        .continuity(ContinuityStatus::Indeterminate, None, None, None)
        .briefing(json!({"runtime_admitted":false,"current_goal_available":false,
            "workflow_stage":stage.map(stage_name),"canonical_join_between_stages":false}))
        .active_work(active)
        .uncertainty(vec![json!({"uncertainty_id":"staged_call_refused","epistemic_state":"unknown",
            "statement":cause.message,"automatic_redispatch":false})])
        .coverage(json!({"status":"unknown","complete_domains":[],"absence_proof_scope":[]}))
        .budget(json!({"admitted":{"max_bytes":budget.max_bytes,"max_output_tokens":budget.max_output_tokens,
            "max_wall_millis":budget.max_wall_millis,"max_game_ticks":budget.max_game_ticks}}));
    if let Some(session) = session {
        turn = turn.session_id(session.to_string());
    }
    if let Some(request) = request {
        turn = turn.request_id(request.to_string());
    }
    let mut packet = json!({"schema":"dfmcp.workshop-production-session/1",
        "session_id":session.map(|id|id.to_string()),"stage":stage.map(stage_name),
        "result":failure_value(&cause, uncertain),"workflow":recovery,"agent_turn":turn.build()});
    packet["result"]["error"]["diagnostic_truncated"] = json!(diagnostic_truncated);
    let raw = packet.to_string();
    if fits(&raw, budget) {
        return raw;
    }
    // Do not cut a recovery object into apparently complete fragments. The
    // runtime retains it; no subsequent mutation is authorized by this refusal.
    packet["workflow"] = json!({"withheld_for_output":true,"complete":false,
        "preserve_last_complete_checkpoint":true,"automatic_redispatch":false});
    packet["agent_turn"]["coverage"] = json!({"status":"partial","complete_domains":[],
        "omitted_domains":["complete_workflow_recovery"],"absence_proof_scope":[]});
    let raw = packet.to_string();
    if fits(&raw, budget) {
        return raw;
    }
    // All admitted sessions reserve 128 KiB for publication; unbound errors
    // use the 1,500-token default. This fixed spine fits both minima even when
    // a refused diagnostic consumed the last output bytes.
    packet["result"]["error"]["message"] =
        json!("Staged request refused; preserve the complete retained checkpoint.");
    packet["result"]["error"]["diagnostic_truncated"] = json!(true);
    packet["agent_turn"]["uncertainty"] = json!([{
        "uncertainty_id":"staged_call_refused","epistemic_state":"unknown",
        "statement":"Current stage and effects require inspection.",
        "automatic_redispatch":false
    }]);
    let raw = packet.to_string();
    debug_assert!(
        fits(&raw, budget),
        "fallback requires an admitted response allowance"
    );
    raw
}

fn transition_packet(
    operation: &str,
    session: SessionId,
    request: Option<RequestId>,
    budget: WorkBudget,
    result: Value,
) -> String {
    let mut turn = AgentTurnBuilder::new(operation, phase(operation))
        .session_id(session.to_string())
        .continuity(ContinuityStatus::Indeterminate, None, None, None)
        .briefing(json!({"workflow_stage":"transition","transition_pending":true,
            "runtime_admitted":false,"current_goal_available":false,
            "canonical_join_between_stages":false,"gameplay_permission":false}))
        .active_work(empty_active_work())
        .uncertainty(vec![json!({"uncertainty_id":"retained_workshop_import",
            "epistemic_state":"unknown","statement":"The exact completed Workshop handoff is retained; its import must finish before production requests become available.",
            "current_workshop_completion_proven":false,"automatic_redispatch":false})])
        .coverage(json!({"status":"partial","complete_domains":[],
            "partial_domains":["retained_workflow_custody"],"absence_proof_scope":[]}))
        .budget(json!({"admitted":{"max_bytes":budget.max_bytes,"max_output_tokens":budget.max_output_tokens,
            "max_wall_millis":budget.max_wall_millis,"max_game_ticks":budget.max_game_ticks}}));
    if let Some(request) = request {
        turn = turn.request_id(request.to_string());
    }
    json!({"result":result,"agent_turn":turn.build()}).to_string()
}

fn render(
    state: &mut State,
    operation: &str,
    request: Option<RequestId>,
    outcome: Result<Value>,
) -> String {
    let stage = state.runtime.stage();
    let successful = outcome.is_ok();
    let result = match outcome {
        Ok(mut view) => {
            view["session_id"] = json!(state.runtime.session_id().to_string());
            match stage {
                Stage::Workshop => {
                    if view["retained_state_currently_verified"] == true {
                        state.uncertain = state.runtime.effect_uncertain()
                            || view["original"]["unresolved"] == true
                            || view["clock"]["unresolved"] == true;
                    }
                    state.workshop_view = Some(view.clone());
                }
                Stage::Production => {
                    view["retained_state_currently_verified"] = json!(true);
                    state.uncertain = state.runtime.effect_uncertain()
                        || view["frontier"]["needs_reconciliation"] == true
                        || view["clock"]["unresolved"] == true;
                    state.production_view = Some(view.clone());
                }
                Stage::Transition => {}
            }
            view
        }
        Err(cause) => {
            state.uncertain |= state.runtime.effect_uncertain();
            return fallback(
                operation,
                Some(state.runtime.session_id()),
                request,
                state.budget,
                Some(stage),
                state.runtime.recovery(),
                &cause,
                state.uncertain,
            );
        }
    };
    let (raw, overflow) = match stage {
        Stage::Production => production::presentation::packet(
            operation,
            Some(state.runtime.session_id()),
            state.runtime.canonical_anchor(),
            Some(state.runtime.mode().as_str()),
            request,
            Some(state.budget),
            result,
            state.production_view.as_ref(),
            successful,
            state.uncertain,
        ),
        Stage::Workshop => (
            crate::workshop_construction_server::presentation::packet(
                operation,
                Some(state.runtime.session_id()),
                request,
                Some(state.budget),
                result,
                state.workshop_view.clone().unwrap_or(Value::Null),
            ),
            false,
        ),
        Stage::Transition => (
            transition_packet(
                operation,
                state.runtime.session_id(),
                request,
                state.budget,
                result,
            ),
            false,
        ),
    };
    let encoded = (|| -> Result<String> {
        if overflow {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "child response exceeds its complete publication allowance",
            ));
        }
        let mut child: Value = serde_json::from_str(&raw).map_err(|_| {
            error(
                ErrorCode::InternalInvariantViolation,
                "child presentation is not valid JSON",
            )
        })?;
        let mut turn = child["agent_turn"].take();
        if !turn.is_object() {
            return Err(error(
                ErrorCode::InternalInvariantViolation,
                "child presentation lost its Agent Turn",
            ));
        }
        let mut result = match stage {
            Stage::Production => child,
            _ => {
                let mut result = child["result"].take();
                if child["retained_reference"].is_object() {
                    result["retained_reference"] = child["retained_reference"].take();
                }
                result
            }
        };
        if result["ok"] != true {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete child presentation was refused",
            ));
        }
        if let Some(object) = result.as_object_mut() {
            object.remove("agent_turn");
        }
        turn["briefing"]["workflow_stage"] = json!(stage_name(stage));
        turn["briefing"]["canonical_join_between_stages"] = json!(false);
        if let Some(items) = turn["uncertainty"].as_array_mut() {
            for item in items {
                if item["uncertainty_id"] == "production_prerequisites" {
                    item["statement"] = json!(
                        "After the exact Workshop handoff, production requires its own fresh observations and independently reviewed effects; Manager and input-supply policies remain operator configured. Untouched-game output requires separate evidence."
                    );
                }
            }
        }
        if stage == Stage::Transition
            || (stage == Stage::Workshop
                && result["original"]["historical_goal"] == "complete"
                && result["original"]["unresolved"] == false
                && result["clock"]["pending"] != true
                && result["clock"]["unresolved"] != true
                && result["clock"]["advancement_fenced"] != true)
        {
            let next = recommendation(
                "begin_production",
                "fortress_plan",
                "Import the exact completed workshop and settled clock custody; retain the current mode and independently required production permissions",
                "continue from construction to the configured production owner",
                "verify the original handoff",
                "read_only",
                "no game effect",
                false,
                json!({"session_id":state.runtime.session_id().to_string(),"request":"{\"kind\":\"begin_production\"}"}),
            );
            if let Some(items) = turn["recommendations"].as_array_mut() {
                items.insert(0, next);
            }
        }
        let packet = json!({"schema":"dfmcp.workshop-production-session/1",
            "session_id":state.runtime.session_id().to_string(),"stage":stage_name(stage),
            "result":result,"workflow":state.runtime.recovery(),"agent_turn":turn});
        let raw = packet.to_string();
        if !fits(&raw, state.budget) {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "complete staged response and recovery do not fit; local reviews withdrawn",
            ));
        }
        Ok(raw)
    })();
    match encoded {
        Ok(raw) => raw,
        Err(cause) => {
            state.runtime.withdraw();
            state.uncertain |= state.runtime.effect_uncertain();
            fallback(
                operation,
                Some(state.runtime.session_id()),
                request,
                state.budget,
                Some(state.runtime.stage()),
                state.runtime.recovery(),
                &cause,
                state.uncertain,
            )
        }
    }
}

fn with_session(
    raw: String,
    operation: &str,
    wall: Option<u64>,
    command: impl FnOnce() -> Result<Command>,
) -> String {
    let result = (|| -> Result<String> {
        let started = Instant::now();
        let id = session_id(&raw)?;
        let mut guard = lock()?;
        let state = guard
            .as_mut()
            .filter(|s| s.runtime.session_id() == id)
            .ok_or_else(|| {
                error(
                    ErrorCode::SessionNotFound,
                    "staged session absent or closed",
                )
            })?;
        let total = wall
            .unwrap_or(state.budget.max_wall_millis)
            .min(state.budget.max_wall_millis);
        let mut request = None;
        let mut may_dispatch = false;
        let outcome = (|| -> Result<Value> {
            runtime_io()?;
            let mut context = state.runtime.context()?;
            request = Some(context.request_id);
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(remaining(started, total)?);
            let command = command()?;
            may_dispatch = command.may_dispatch();
            let value = command.perform(&mut state.runtime, &context)?;
            runtime_io()?;
            remaining(started, total)?;
            Ok(value)
        })();
        if outcome.is_err() {
            state.uncertain |= may_dispatch;
            state.runtime.withdraw();
        }
        let output = render(state, operation, request, outcome);
        if let Err(cause) = runtime_io().and_then(|()| remaining(started, total).map(|_| ())) {
            state.uncertain |= may_dispatch;
            state.runtime.withdraw();
            return Ok(render(state, operation, request, Err(cause)));
        }
        Ok(output)
    })();
    result.unwrap_or_else(|cause| {
        fallback(
            operation,
            None,
            None,
            WorkBudget::default(),
            None,
            Value::Null,
            &cause,
            false,
        )
    })
}

#[tool(
    description = "Open one unadmitted staged workshop-production session. Empty production owners open before construction is reachable. Mode defaults to offline; control requires every independent operator permission. Clocks need explicit max_game_ticks, default zero. Opening never constructs, inserts an order or unpauses."
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
        let mut guard = lock()?;
        if guard.is_some() {
            return Err(error(
                ErrorCode::Conflict,
                "close the current staged session first",
            ));
        }
        let sequence = NEXT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < SEQUENCE_LIMIT).then_some(n + 1)
            })
            .map_err(|_| error(ErrorCode::BudgetExceeded, "staged session IDs exhausted"))?;
        let id = SessionId::new((1u128 << 127) | FAMILY | u128::from(sequence));
        work.max_wall_millis = remaining(started, display.max_wall_millis)?;
        let runtime = Runtime::open(config, mode, id, work)?;
        let uncertain = runtime.effect_uncertain();
        *guard = Some(State {
            runtime,
            budget: display,
            workshop_view: None,
            production_view: None,
            uncertain,
        });
        let state = guard.as_mut().ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "staged owner disappeared",
            )
        })?;
        let mut request = None;
        let outcome = (|| -> Result<Value> {
            let mut context = state.runtime.context()?;
            request = Some(context.request_id);
            context.budget.max_bytes = OPEN_INSPECTION_BYTES;
            context.budget.max_wall_millis = context
                .budget
                .max_wall_millis
                .min(remaining(started, display.max_wall_millis)?);
            let value = state.runtime.inspect(&context)?;
            runtime_io()?;
            remaining(started, display.max_wall_millis)?;
            Ok(value)
        })();
        if outcome.is_err() {
            state.runtime.withdraw();
        }
        let output = render(state, "fortress.open_session", request, outcome);
        if let Err(cause) =
            runtime_io().and_then(|()| remaining(started, display.max_wall_millis).map(|_| ()))
        {
            state.runtime.withdraw();
            return Ok(render(state, "fortress.open_session", request, Err(cause)));
        }
        Ok(output)
    })();
    result.unwrap_or_else(|cause| {
        fallback(
            "fortress.open_session",
            None,
            None,
            WorkBudget::default(),
            None,
            Value::Null,
            &cause,
            false,
        )
    })
}

#[tool(
    description = "Observe the active stage. Workshop observes its exact material/footprint/workers; production accepts its existing optional workforce and inventory discovery request. Historical workshop evidence never becomes a joined canonical world."
)]
pub fn fortress_observe(session_id: String, request: Option<String>) -> String {
    with_session(session_id, "fortress.observe", None, || {
        Ok(Command::Observe(request))
    })
}

#[tool(
    description = "Inspect the active original and full workflow recovery. Workshop supports mode=production_handoff. Production supports its existing order_step_id and closed production-inspection request. Query never advances stages or time."
)]
pub fn fortress_query(
    session_id: String,
    mode: Option<String>,
    order_step_id: Option<u32>,
    request: Option<String>,
) -> String {
    with_session(session_id, "fortress.query", None, || {
        Ok(Command::Query {
            mode,
            request,
            step: order_step_id,
        })
    })
}

#[tool(
    description = "Use the active owner's existing closed plan request. After original workshop completion and clock settlement, kind=begin_production imports that exact custody without any game effect. Then observe fresh citizens and plan finite production; each effect still needs a new review and confirmation."
)]
pub fn fortress_plan(session_id: String, request: String) -> String {
    with_session(session_id, "fortress.plan", None, || {
        Ok(Command::Plan(request))
    })
}

#[tool(
    description = "Confirm exactly one active-stage review. Supply review_seal for Workshop or paired_seal for production, never both; confirmed must be true. Production scope and order_step_id retain their existing meanings. Stage transition never supplies an effect permit."
)]
pub fn fortress_commit(
    session_id: String,
    confirmed: bool,
    review_seal: Option<String>,
    paired_seal: Option<String>,
    scope: Option<String>,
    order_step_id: Option<u32>,
) -> String {
    with_session(session_id, "fortress.commit", None, || {
        let (seal, workshop) = match (review_seal, paired_seal) {
            (Some(seal), None) => (seal, true),
            (None, Some(seal)) => (seal, false),
            _ => {
                return Err(error(
                    ErrorCode::InvalidRequest,
                    "supply exactly one current-stage review seal",
                ));
            }
        };
        Ok(Command::Commit {
            seal,
            workshop,
            confirmed,
            scope,
            step: order_step_id,
        })
    })
}

#[tool(
    description = "Reconcile or sample the active original. Workshop scope=clock settles its retained interval before workshop progress; production Wait preserves complete-parent inventory/order semantics. No insertion, construction, automatic stage transition or unpause."
)]
pub fn fortress_wait(
    session_id: String,
    scope: Option<String>,
    max_wall_millis: Option<u64>,
) -> String {
    with_session(session_id, "fortress.wait", max_wall_millis, || {
        Ok(Command::Wait(scope))
    })
}

#[tool(
    description = "Cancel the active owner's preparation or request its separately authorized clock safety stop. scope=session releases both owners and preserves all custody; it does not cancel game effects or prove a pause."
)]
pub fn fortress_cancel(
    session_id: String,
    scope: Option<String>,
    order_step_id: Option<u32>,
) -> String {
    if scope.as_deref() == Some("session") && order_step_id.is_none() {
        let result = (|| -> Result<String> {
            let id = self::session_id(&session_id)?;
            let mut guard = lock()?;
            let state = guard
                .as_mut()
                .filter(|s| s.runtime.session_id() == id)
                .ok_or_else(|| {
                    error(
                        ErrorCode::SessionNotFound,
                        "staged session absent or closed",
                    )
                })?;
            state.runtime.withdraw();
            state.uncertain |= state.runtime.effect_uncertain();
            let recovery = state.runtime.recovery();
            let mut active = empty_active_work();
            if recovery["workshop"]["original"].is_object()
                || recovery["production"]["recovery"].is_object()
            {
                active["obligations"] = json!([{"obligation_id":"retain_original_workflow",
                    "recovery_reference":"workflow","physical_quiescence_proven":false}]);
            }
            if state.uncertain {
                active["indeterminate_effects"] = json!([{"scope":"retained_originals",
                    "recovery_reference":"workflow","automatic_redispatch":false}]);
            }
            let mut turn = AgentTurnBuilder::new("fortress.cancel", AgentPhase::Reconcile)
                .session_id(id.to_string())
                .briefing(json!({"session_closed":true,"effects_cancelled":false,"current_pause_proven":false}))
                .active_work(active).build();
            turn["anchor"] = Value::Null;
            let out =
                json!({"schema":"dfmcp.workshop-production-session/1","session_id":id.to_string(),
                "stage":stage_name(state.runtime.stage()),"result":{"ok":true,"closed":true,
                    "effects_cancelled":false,"custody_deleted":false,"native_calls":0},
                "workflow":recovery,"agent_turn":turn})
                .to_string();
            if !fits(&out, state.budget) {
                let cause = error(
                    ErrorCode::BudgetExceeded,
                    "complete recovery and close acknowledgement must fit before releasing owners",
                );
                return Ok(fallback(
                    "fortress.cancel",
                    Some(id),
                    None,
                    state.budget,
                    Some(state.runtime.stage()),
                    state.runtime.recovery(),
                    &cause,
                    state.uncertain,
                ));
            }
            drop(guard.take());
            Ok(out)
        })();
        return result.unwrap_or_else(|cause| {
            fallback(
                "fortress.cancel",
                None,
                None,
                WorkBudget::default(),
                None,
                Value::Null,
                &cause,
                false,
            )
        });
    }
    with_session(session_id, "fortress.cancel", None, || {
        Ok(Command::Cancel {
            scope,
            step: order_step_id,
        })
    })
}

#[tool(
    description = "Explain the current stage, distinct original goals and retained recovery custody without native contact or mutation authority."
)]
pub fn fortress_explain(session_id: String) -> String {
    with_session(session_id, "fortress.explain", None, || {
        Ok(Command::Inspect)
    })
}
#[tool(
    description = "Inspect staged development custody. No compatibility or production admission is granted."
)]
pub fn fortress_doctor(session_id: String) -> String {
    with_session(session_id, "fortress.doctor", None, || Ok(Command::Inspect))
}
#[tool(description = "Unavailable: this development profile does not create game checkpoints.")]
pub fn fortress_checkpoint(session_id: String) -> String {
    with_session(session_id, "fortress.checkpoint", None, || {
        Ok(Command::Unavailable)
    })
}
#[tool(
    description = "Unavailable: recover exact staged custody; this profile does not restore a game or replace uncertain original keys."
)]
pub fn fortress_restore(session_id: String) -> String {
    with_session(session_id, "fortress.restore", None, || {
        Ok(Command::Unavailable)
    })
}

pub fn run_stdio() {
    if std::env::var(OPT_IN).ok().as_deref() != Some("1")
        || std::env::var_os("DFMCP_ADMITTED_BRIDGE_PROTOCOL").is_some()
        || crate::current_admission_provenance().is_some()
    {
        eprintln!("capability_denied: explicit unadmitted workshop-production startup is required");
        return;
    }
    let server = ServerBuilder::new("dfmcp-workshop-production-dev", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession).tool(FortressObserve).tool(FortressQuery).tool(FortressPlan)
        .tool(FortressCommit).tool(FortressWait).tool(FortressCancel).tool(FortressCheckpoint)
        .tool(FortressRestore).tool(FortressExplain).tool(FortressDoctor)
        .instructions("One unadmitted staged owner, two immutable originals. Complete and settle the original Carpenter workshop using its exact review and optional independently authorized clock. Plan kind=begin_production imports that exact acknowledged original into already-open production custodians; it never initializes another native owner or dispatches an effect. A failed import stays in transition and only the same read-only import can be retried. Observe fresh citizens after transition, then use the combined owner's existing workforce choices, finite stock targets or portfolios and independently reviewed production clocks. Preserve the full workflow checkpoint and every child journal/ACK/clock recovery reference. Reopening never restores a review. Workshop native observations and production canonical state have no established joined continuity; historical construction completion does not prove current readiness or furniture production.")
        .build();
    crate::run_modern_stdio(server);
}

#[cfg(test)]
#[path = "workshop_production_server_tests.rs"]
mod tests;
