//! Modern-only (MCP 2026-07-28) stdio server for the fortress narrow waist.
//!
//! WP-13 gate 2 — session-scoped capability negotiation: per-session state and
//! negotiated grants replace the previous process-local laboratory state.
//! Transport identity grants nothing; every capability comes from the
//! `CapabilityGrant`s negotiated in `fortress_open_session`.
//!
//! Per-session state lives in `SESSIONS`, keyed by the freshly minted
//! `SessionId` returned by `fortress_open_session`. Subsequent tools take
//! `session_id` as an argument and dispatch against that session only.
//! Concurrent stdio clients therefore get independent adapters, anchors,
//! plans, and receipts. No transport type crosses the adapter seam:
//! `dfmcp-core` `CapabilityGrant` is the sole authority.
//!
//! See `docs/FASTMCP_INTEGRATION.md` §6 for the security posture and
//! `design/registries/CAPABILITIES.md` for the negotiated-capability registry.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, Mutex, MutexGuard};

use crate::doctor::DoctorInspector;
use dfmcp_adapter::{CancelMode, GameAdapter};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CheckpointId, DfmcpError, EntityId,
    ErrorCode, FortressId, GameTick, IntentId, ObservationCursor, OperationContext, RequestId,
    Result, RiskTier, SessionId, StateAnchor, WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, PreparedPlan, RequestedAction, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::topology::get_transitive_dependencies;
use dfmcp_world::{EdgeKind, Predicate, WorldGraph, WorldSnapshot};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::json;

/// One granted capability record returned to the client.
#[derive(Clone, Debug, PartialEq, Eq)]
struct NegotiatedCapability {
    capability: Capability,
    max_risk: RiskTier,
}

/// Per-session state. Lives behind an `Arc<Mutex<…>>` so multiple tool calls
/// within the same session share state without crossing `static` boundaries.
struct LabSession {
    session_id: SessionId,
    #[allow(dead_code)]
    fortress_id: FortressId,
    /// The capabilities the caller negotiated in `fortress_open_session`.
    /// Transport identity grants nothing; these are the only authority.
    grants: Vec<CapabilityGrant>,
    /// The budget the caller negotiated.
    budget: WorkBudget,
    /// Per-session request counter (used as dfmcp RequestId).
    next_request_id: u128,
    /// The owned lab adapter.
    adapter: MemoryAdapter,
    /// Pending prepared plan awaiting commit.
    pending: Option<PendingPlan>,
    /// Most recent committed action id (for wait/cancel).
    last_action: Option<ActionId>,
    /// (plan_digest, payload) for idempotent re-commit (per ADR-006).
    last_commit: Option<(String, String)>,
}

/// A plan sealed by `fortress_plan` and awaiting `fortress_commit`.
struct PendingPlan {
    #[allow(dead_code)]
    plan: PreparedPlan,
    digest: String,
}

/// Process-wide session registry, keyed by `SessionId`. Replaces the previous
/// `static LAB` so that two concurrent stdio sessions are independent.
static SESSIONS: LazyLock<Mutex<BTreeMap<SessionId, Arc<Mutex<LabSession>>>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

/// Counter for minting fresh `SessionId`s.
static NEXT_SESSION_COUNTER: LazyLock<Mutex<u128>> = LazyLock::new(|| Mutex::new(1));

fn sessions() -> MutexGuard<'static, BTreeMap<SessionId, Arc<Mutex<LabSession>>>> {
    match SESSIONS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn next_session_counter() -> Result<u128> {
    let mut counter = match NEXT_SESSION_COUNTER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let id = *counter;
    if id == 0 {
        return Err(DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            "session identifier zero is reserved",
        ));
    }
    *counter = counter.checked_add(1).ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::BudgetExceeded,
            "process-local session identifier space is exhausted",
        )
    })?;
    Ok(id)
}

fn parse_session_id_arg(value: &str) -> Result<SessionId> {
    let parsed: u128 = value.parse().map_err(|_| {
        DfmcpError::new(
            ErrorCode::InvalidRequest,
            "session_id must be a u128 decimal string returned by fortress_open_session",
        )
    })?;
    Ok(SessionId::new(parsed))
}

fn lookup_session(session_id: SessionId) -> Result<Arc<Mutex<LabSession>>> {
    let guard = sessions();
    guard.get(&session_id).cloned().ok_or_else(|| {
        DfmcpError::new(
            ErrorCode::SessionNotFound,
            "no open session with the supplied session_id; call fortress_open_session first",
        )
    })
}

fn resolve_session(session_id: Option<String>) -> Result<Arc<Mutex<LabSession>>> {
    if let Some(id_str) = session_id {
        let parsed = parse_session_id_arg(&id_str)?;
        lookup_session(parsed)
    } else {
        let guard = sessions();
        if guard.len() == 1 {
            guard.values().next().cloned().ok_or_else(|| {
                DfmcpError::new(
                    ErrorCode::SessionNotFound,
                    "no open session; call fortress_open_session first",
                )
            })
        } else if guard.is_empty() {
            Err(DfmcpError::new(
                ErrorCode::SessionNotFound,
                "no open session; call fortress_open_session first",
            ))
        } else {
            Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "multiple open sessions exist; specify session_id",
            ))
        }
    }
}

fn next_request_id(session: &mut LabSession) -> u128 {
    session.next_request_id = session.next_request_id.wrapping_add(1);
    session.next_request_id
}

fn seed_snapshot(fortress_id: FortressId, paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        fortress_id,
        GameTick(1),
        ObservationCursor::ORIGIN,
        paused,
        WorldGraph::default(),
    )
}

/// Build the per-request `OperationContext` from the **session's** negotiated
/// grants and budget. This is the gate that keeps transport identity from
/// granting authority: nothing here reads from `McpContext` or transport
/// state. Authority is exactly what `fortress_open_session` returned.
fn context_for(session: &LabSession, request_id: u128) -> OperationContext {
    OperationContext {
        session_id: session.session_id,
        request_id: RequestId::new(request_id),
        anchor: session.adapter.snapshot().anchor(),
        budget: session.budget,
        grants: session.grants.clone(),
        cancellation_requested: false,
    }
}

fn anchor_json(anchor: &StateAnchor) -> serde_json::Value {
    json!({
        "fortress_id": format!("{}", anchor.fortress_id),
        "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence,
        "state_hash": anchor.state_hash.to_string(),
    })
}

fn error_payload(operation: &str, message: &str) -> String {
    json!({
        "ok": false,
        "error": {"operation": operation, "message": message},
    })
    .to_string()
}

fn snapshot_json(snapshot: &WorldSnapshot) -> serde_json::Value {
    json!({
        "ok": true,
        "fortress_id": format!("{}", snapshot.fortress_id),
        "game_tick": snapshot.tick.0,
        "cursor": {
            "epoch": snapshot.cursor.epoch,
            "sequence": snapshot.cursor.sequence,
        },
        "paused": snapshot.paused,
        "state_hash": snapshot.state_hash.to_string(),
    })
}

/// Negotiate a `CapabilityGrant` list from requested capability strings and
/// their ceiling risk tiers. Each requested capability becomes one grant over
/// the session's fortress selector. The returned list is exactly what gets
/// installed on the session — no defaults, no extras.
fn negotiate_grants(
    fortress_id: FortressId,
    requested: &[NegotiatedCapability],
) -> Vec<CapabilityGrant> {
    requested
        .iter()
        .map(|req| CapabilityGrant {
            capability: req.capability,
            scope: CapabilityScope {
                fortress_id: Some(fortress_id),
                ..CapabilityScope::default()
            },
            max_risk: req.max_risk,
            expires_at_tick: None,
            remaining_uses: None,
        })
        .collect()
}

fn parse_capability_request(requested: &[(String, String)]) -> Result<Vec<NegotiatedCapability>> {
    let mut out = Vec::with_capacity(requested.len());
    let mut seen: BTreeSet<Capability> = BTreeSet::new();
    for (cap_str, risk_str) in requested {
        let capability = match cap_str.as_str() {
            "observe" => Capability::Observe,
            "query" => Capability::Query,
            "plan" => Capability::Plan,
            "designate" => Capability::Designate,
            "construct" => Capability::Construct,
            "configure_labor" => Capability::ConfigureLabor,
            "configure_production" => Capability::ConfigureProduction,
            "configure_logistics" => Capability::ConfigureLogistics,
            "configure_military" => Capability::ConfigureMilitary,
            "control_clock" => Capability::ControlClock,
            "checkpoint" => Capability::Checkpoint,
            "restore" => Capability::Restore,
            "extension" => Capability::Extension,
            "diagnostic_raw" => Capability::DiagnosticRaw,
            "doctor" => Capability::Doctor,
            "repair_plan" => Capability::RepairPlan,
            "repair_apply" => Capability::RepairApply,
            "admin" => Capability::Admin,
            other => {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unsupported capability {other:?}; check the registry"),
                ));
            }
        };
        if !seen.insert(capability) {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                format!("capability {cap_str} requested more than once"),
            ));
        }
        let max_risk = match risk_str.as_str() {
            "read_only" => RiskTier::ReadOnly,
            "reversible" => RiskTier::Reversible,
            "guarded" => RiskTier::Guarded,
            "irreversible" => RiskTier::Irreversible,
            other => {
                return Err(DfmcpError::new(
                    ErrorCode::InvalidRequest,
                    format!("unsupported risk tier {other:?} for {cap_str}"),
                ));
            }
        };
        out.push(NegotiatedCapability {
            capability,
            max_risk,
        });
    }
    Ok(out)
}

// ============================================================================
// fortress.open_session
// ============================================================================

/// Open a laboratory fortress session. Negotiation inputs are the fortress
/// selector, the requested capability set with risk ceilings, and the work
/// budget. The session is independent from every other session in this
/// process; transport identity grants nothing.
#[tool(
    description = "Open a fortress session against the deterministic laboratory adapter. Negotiates a per-session capability set and budget, then returns a session_id for all subsequent tool calls. Transport identity grants nothing; every authority comes from the negotiated grants."
)]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session(
    paused: Option<bool>,
    fortress_selector: Option<String>,
    requested_capabilities: Option<Vec<(String, String)>>,
    max_wall_millis: Option<u64>,
    max_game_ticks: Option<u64>,
    max_entities: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_actions: Option<u32>,
) -> String {
    let paused = paused.unwrap_or(true);
    let selector_str = fortress_selector.unwrap_or_else(|| "1".to_owned());
    let parsed_fortress: Result<FortressId> = match selector_str.parse::<u64>() {
        Ok(value) => Ok(FortressId::new(value)),
        Err(_) => Err(DfmcpError::new(
            ErrorCode::InvalidRequest,
            "fortress_selector must be a u64 decimal string",
        )),
    };
    let fortress_id = match parsed_fortress {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.open_session", &error.to_string()),
    };

    let default_caps = vec![
        ("observe".to_owned(), "read_only".to_owned()),
        ("query".to_owned(), "read_only".to_owned()),
        ("plan".to_owned(), "reversible".to_owned()),
        ("control_clock".to_owned(), "reversible".to_owned()),
        ("checkpoint".to_owned(), "guarded".to_owned()),
        ("restore".to_owned(), "guarded".to_owned()),
        ("doctor".to_owned(), "read_only".to_owned()),
    ];
    let requested_caps_raw = requested_capabilities.unwrap_or(default_caps);
    let requested_caps = match parse_capability_request(&requested_caps_raw) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.open_session", &error.to_string()),
    };

    let budget = WorkBudget {
        max_wall_millis: max_wall_millis
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_wall_millis),
        max_game_ticks: max_game_ticks.unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_game_ticks),
        max_entities: max_entities.unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_entities),
        max_bytes: max_bytes.unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_bytes),
        max_output_tokens: max_output_tokens
            .unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_output_tokens),
        max_actions: max_actions.unwrap_or(WorkBudget::CONSERVATIVE_DEFAULT.max_actions),
    };
    if let Err(error) = budget.validate() {
        return error_payload("fortress.open_session", &error.to_string());
    }

    let grants = negotiate_grants(fortress_id, &requested_caps);
    let mut probe_session = LabSession {
        session_id: SessionId::new(0), // placeholder; replaced below
        fortress_id,
        grants: grants.clone(),
        budget,
        next_request_id: 0,
        adapter: MemoryAdapter::new(seed_snapshot(fortress_id, paused)),
        pending: None,
        last_action: None,
        last_commit: None,
    };
    let probe_ctx = context_for(&probe_session, 1);
    match probe_session.adapter.health(&probe_ctx) {
        Ok(health) => {
            let session_counter = match next_session_counter() {
                Ok(value) => value,
                Err(error) => return error_payload("fortress.open_session", &error.to_string()),
            };
            let session_id = SessionId::new(session_counter);
            let snapshot_anchor = probe_session.adapter.snapshot().anchor();
            let paused_after = probe_session.adapter.snapshot().paused;
            // Move the probe adapter into the registered session.
            let LabSession {
                session_id: _,
                fortress_id: _,
                grants: _,
                budget: _,
                next_request_id: _,
                adapter,
                pending: _,
                last_action: _,
                last_commit: _,
            } = probe_session;
            let session = Arc::new(Mutex::new(LabSession {
                session_id,
                fortress_id,
                grants,
                budget,
                next_request_id: 0,
                adapter,
                pending: None,
                last_action: None,
                last_commit: None,
            }));
            sessions().insert(session_id, session);
            let granted_strings: Vec<&str> = requested_caps
                .iter()
                .map(|c| match c.capability {
                    Capability::Observe => "observe",
                    Capability::Query => "query",
                    Capability::Plan => "plan",
                    Capability::ControlClock => "control_clock",
                    Capability::Checkpoint => "checkpoint",
                    Capability::Restore => "restore",
                    Capability::DiagnosticRaw => "diagnostic_raw",
                    Capability::Doctor => "doctor",
                    _ => "other",
                })
                .collect();
            let payload = json!({
                "ok": true,
                "session_id": format!("{session_id}"),
                "adapter": health.identity.name,
                "compatibility": format!("{:?}", health.identity.compatibility),
                "fortress_loaded": health.fortress_loaded,
                "fortress_id": format!("{fortress_id}"),
                "granted_capabilities": granted_strings,
                "budget": {
                    "max_wall_millis": budget.max_wall_millis,
                    "max_game_ticks": budget.max_game_ticks,
                    "max_entities": budget.max_entities,
                    "max_bytes": budget.max_bytes,
                    "max_output_tokens": budget.max_output_tokens,
                    "max_actions": budget.max_actions,
                },
                "anchor": anchor_json(&snapshot_anchor),
                "paused": paused_after,
                "note": "session_id is required for all subsequent tool calls; transport identity grants nothing",
            });
            payload.to_string()
        }
        Err(error) => error_payload("fortress.open_session", &error.to_string()),
    }
}

// ============================================================================
// fortress.observe
// ============================================================================

/// Return the current bounded snapshot projection at the live anchor.
#[tool(
    description = "Observe the current fortress state for an open session. Requires the session_id returned by fortress_open_session."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.observe", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.observe", &error.to_string()),
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    match guard.adapter.health(&ctx) {
        Ok(_) => {
            let mut payload = snapshot_json(guard.adapter.snapshot());
            payload["projection"] = json!("summary");
            payload["session_id"] = json!(format!("{}", guard.session_id));
            payload.to_string()
        }
        Err(error) => error_payload("fortress.observe", &error.to_string()),
    }
}

// ============================================================================
// fortress.query
// ============================================================================

/// Return the bounded summary query supported by the laboratory slice.
#[tool(
    description = "Run the bounded summary query supported by the laboratory adapter. Full DfQL is not implemented."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    let mode = mode.unwrap_or_else(|| "summary".to_owned());
    if mode != "summary" {
        return error_payload(
            "fortress.query",
            "only mode=\"summary\" is supported by the laboratory slice; full DfQL is not implemented",
        );
    }
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.query", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.query", &error.to_string()),
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    if let Err(error) = ctx.authorize(Capability::Query, RiskTier::ReadOnly, &[], None) {
        return error_payload("fortress.query", &error.to_string());
    }
    let snapshot = guard.adapter.snapshot();
    let mut payload = snapshot_json(snapshot);
    payload["matched"] = json!(0);
    payload["session_id"] = json!(format!("{}", guard.session_id));
    payload.to_string()
}

// ============================================================================
// fortress.plan
// ============================================================================

/// Compile a pause/resume intent into an immutable, inspectable plan without effects.
#[tool(
    description = "Compile a laboratory pause/resume intent into an immutable, inspectable plan without effects."
)]
pub fn fortress_plan(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.plan", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.plan", &error.to_string()),
    };
    let rid = next_request_id(&mut guard);
    let snapshot = guard.adapter.snapshot();
    let ctx = context_for(&guard, rid);

    let paused_target = paused_target.unwrap_or(false);
    let intent = Intent {
        id: IntentId::new(rid),
        anchor: snapshot.anchor(),
        summary: summary.unwrap_or_else(|| "unpause the simulation".to_owned()),
        terminal_condition: Predicate::Paused(paused_target),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause {
                paused: paused_target,
            },
            preconditions: vec![Predicate::Paused(!paused_target)],
            postconditions: vec![Predicate::Paused(paused_target)],
            compensation: None,
            obligation: None,
            depends_on: Vec::new(),
        }],
    };

    match StaticPlanner::default().prepare(snapshot, &intent, &ctx) {
        Ok(plan) => {
            let digest = plan.digest.to_string();
            let pending_digest = digest.clone();
            let payload = json!({
                "ok": true,
                "session_id": format!("{}", guard.session_id),
                "plan_id": format!("{}", plan.id),
                "plan_digest": digest,
                "terminal_condition": format!("{:?}", intent.terminal_condition),
                "max_risk": "reversible",
                "note": "sealed plan; commit it with fortress_commit before expiry",
            });
            guard.pending = Some(PendingPlan {
                plan,
                digest: pending_digest,
            });
            payload.to_string()
        }
        Err(error) => error_payload("fortress.plan", &error.to_string()),
    }
}

// ============================================================================
// fortress.commit
// ============================================================================

/// Revalidate and idempotently commit the pending prepared plan. Requires the
/// exact plan digest; returns per-action receipts and the post-commit anchor.
#[tool(
    description = "Commit the pending prepared plan for the open session: prepare/revalidate, dispatch, observe, and verify. Requires the exact plan digest returned by fortress_plan."
)]
pub fn fortress_commit(session_id: Option<String>, plan_digest: String) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.commit", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.commit", &error.to_string()),
    };

    let pending = match guard.pending.take() {
        Some(pending) => pending,
        None => {
            // ADR-006 idempotency: a duplicate commit with the same digest
            // returns the prior receipt verbatim instead of erroring or
            // reapplying effects.
            if let Some((digest, payload)) = &guard.last_commit
                && digest == &plan_digest
            {
                return payload.clone();
            }
            return error_payload(
                "fortress.commit",
                "no pending plan; call fortress_plan first",
            );
        }
    };
    if pending.digest != plan_digest {
        return error_payload(
            "fortress.commit",
            "plan digest does not match the pending prepared plan; plans are sealed over their digest",
        );
    }
    let rid = next_request_id(&mut guard);
    let prepare_ctx = context_for(&guard, rid);
    match guard.adapter.prepare(&pending.plan, &prepare_ctx) {
        Ok(prepared) => {
            let rid = next_request_id(&mut guard);
            let commit_ctx = context_for(&guard, rid);
            match guard.adapter.commit(&pending.plan, &prepared, &commit_ctx) {
                Ok(receipt) => {
                    guard.last_action = receipt.actions.first().map(|action| action.action_id);
                    let snapshot = guard.adapter.snapshot();
                    let paused = snapshot.paused;
                    let payload = json!({
                        "ok": true,
                        "session_id": format!("{}", guard.session_id),
                        "plan_id": format!("{}", receipt.plan_id),
                        "plan_digest": receipt.plan_digest.to_string(),
                        "actions": receipt.actions.iter().map(|action| json!({
                            "action_id": format!("{}", action.action_id),
                            "state": format!("{:?}", action.state),
                            "message": action.message,
                        })).collect::<Vec<_>>(),
                        "observed_anchor": anchor_json(&receipt.observed_anchor),
                        "paused": paused,
                    });
                    let payload_text = payload.to_string();
                    guard.last_commit = Some((plan_digest.clone(), payload_text.clone()));
                    payload_text
                }
                Err(error) => {
                    guard.pending = Some(pending);
                    error_payload("fortress.commit", &error.to_string())
                }
            }
        }
        Err(error) => {
            guard.pending = Some(pending);
            error_payload("fortress.commit", &error.to_string())
        }
    }
}

// ============================================================================
// fortress.wait
// ============================================================================

/// Poll the most recent committed action and return its current state.
#[tool(
    description = "Poll the most recent committed action in this session. Returns the action receipt state from the laboratory adapter's bounded obligation machinery."
)]
pub fn fortress_wait(session_id: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.wait", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.wait", &error.to_string()),
    };
    let Some(action_id) = guard.last_action else {
        return error_payload(
            "fortress.wait",
            "no committed action yet; call fortress_commit first",
        );
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    match crate::tasks::project_action_task(&mut guard.adapter, action_id, &ctx) {
        Ok(task) => json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "action_id": format!("{}", action_id),
            "task_id": task.task_id,
            "status": task.status.as_str(),
            "commit_state": format!("{:?}", task.commit_state),
            "summary": task.summary,
            "observed_anchor": anchor_json(&ctx.anchor),
        })
        .to_string(),
        Err(error) => error_payload("fortress.wait", &error.to_string()),
    }
}

// ============================================================================
// fortress.cancel
// ============================================================================

/// Request, drain, and finalize cancellation of the most recent action with
/// authorized compensation. Cancellation never deletes records.
#[tool(
    description = "Cancel the most recent committed action in this session: request, drain, compensate when authorized, and finalize."
)]
pub fn fortress_cancel(session_id: Option<String>, mode: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.cancel", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.cancel", &error.to_string()),
    };
    let Some(action_id) = guard.last_action else {
        return error_payload(
            "fortress.cancel",
            "no committed action to cancel; call fortress_commit first",
        );
    };
    let cancel_mode = match mode.as_deref() {
        Some("emergency_pause_and_drain") => CancelMode::EmergencyPauseAndDrain,
        Some("stop_future_steps") => CancelMode::StopFutureSteps,
        _ => CancelMode::CompensateReversible,
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    match guard.adapter.request_cancel(action_id, cancel_mode, &ctx) {
        Ok(request) => match guard.adapter.finalize_cancel(action_id, &ctx) {
            Ok(finalized) => json!({
                "ok": true,
                "session_id": format!("{}", guard.session_id),
                "action_id": format!("{}", finalized.action_id),
                "requested_state": format!("{:?}", request.state),
                "final_state": format!("{:?}", finalized.state),
                "note": "cancellation is request/drain/compensate/finalize; records are never deleted",
            })
            .to_string(),
            Err(error) => error_payload("fortress.cancel", &error.to_string()),
        },
        Err(error) => error_payload("fortress.cancel", &error.to_string()),
    }
}

// ============================================================================
// fortress.checkpoint
// ============================================================================

/// Create a content-addressed, labeled recovery point.
#[tool(
    description = "Create a labeled checkpoint for the open session: a content-addressed recovery point with an evidence record."
)]
pub fn fortress_checkpoint(session_id: Option<String>, label: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.checkpoint", &error.to_string()),
    };
    let label = label.unwrap_or_else(|| "manual".to_owned());
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.checkpoint", &error.to_string()),
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    match guard.adapter.checkpoint(&label, &ctx) {
        Ok(receipt) => json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "checkpoint_id": format!("{}", receipt.checkpoint_id),
            "label": receipt.label,
            "content_digest": receipt.content_digest.to_string(),
            "durable": receipt.durable,
            "anchor": anchor_json(&receipt.anchor),
        })
        .to_string(),
        Err(error) => error_payload("fortress.checkpoint", &error.to_string()),
    }
}

// ============================================================================
// fortress.restore
// ============================================================================

/// Restore a checkpoint into a new observation epoch, invalidating stale
/// plans and action handles.
#[tool(
    description = "Restore a checkpoint by id into the open session. Creates a new observation epoch; stale plans and action handles are invalidated."
)]
pub fn fortress_restore(session_id: Option<String>, checkpoint_id: String) -> String {
    let parsed_checkpoint = match checkpoint_id.parse::<u128>() {
        Ok(value) => value,
        Err(_) => {
            return error_payload(
                "fortress.restore",
                "checkpoint_id must be a u128 decimal string",
            );
        }
    };
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.restore", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.restore", &error.to_string()),
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    match guard
        .adapter
        .restore(CheckpointId::new(parsed_checkpoint), &ctx)
    {
        Ok(receipt) => {
            guard.pending = None;
            guard.last_action = None;
            guard.last_commit = None;
            json!({
                "ok": true,
                "session_id": format!("{}", guard.session_id),
                "checkpoint_id": format!("{}", receipt.checkpoint_id),
                "prior_anchor": anchor_json(&receipt.prior_anchor),
                "restored_anchor": anchor_json(&receipt.restored_anchor),
                "content_digest": receipt.content_digest.to_string(),
                "note": "new observation epoch; pending plans and action handles were invalidated",
            })
            .to_string()
        }
        Err(error) => error_payload("fortress.restore", &error.to_string()),
    }
}

// ============================================================================
// fortress.explain
// ============================================================================

/// Explain recent state transitions or graph dependencies for a specific entity.
#[tool(
    description = "Explain what happened in this session: return recent transcript events, or graph dependencies and causal topology for a specified entity."
)]
pub fn fortress_explain(session_id: Option<String>, entity_id: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.explain", &error.to_string()),
    };
    let guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.explain", &error.to_string()),
    };
    let ctx = context_for(&guard, guard.next_request_id);
    if let Err(error) = ctx.authorize(Capability::Query, RiskTier::ReadOnly, &[], None) {
        return error_payload("fortress.explain", &error.to_string());
    }
    let snapshot = guard.adapter.snapshot();

    if let Some(ent_str) = entity_id {
        let parsed_id: Result<EntityId> = ent_str.parse::<u64>().map(EntityId::new).map_err(|_| {
            DfmcpError::new(ErrorCode::InvalidRequest, "entity_id must be a decimal u64")
        });
        let target_id = match parsed_id {
            Ok(id) => id,
            Err(err) => return error_payload("fortress.explain", &err.to_string()),
        };

        let deps = get_transitive_dependencies(&snapshot.graph, target_id, EdgeKind::Requires);
        let deps_str: Vec<String> = deps.iter().map(|id| format!("{}", id.get())).collect();
        let entity_record = snapshot.graph.entities.get(&target_id);

        json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "target_entity": format!("{}", target_id.get()),
            "entity_found": entity_record.is_some(),
            "transitive_dependencies": deps_str,
            "note": "causal explanation derived from directed fortress multigraph topology",
        })
        .to_string()
    } else {
        let events = guard.adapter.transcript();
        let start = events.len().saturating_sub(16);
        let recent: Vec<String> = events[start..]
            .iter()
            .map(|event| format!("{event:?}"))
            .collect();
        json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "transcript_len": events.len(),
            "recent_events": recent,
            "note": "the laboratory transcript is the evidence ledger; production explanations cite durable evidence bundles",
        })
        .to_string()
    }
}

// ============================================================================
// fortress.doctor
// ============================================================================

/// Diagnose adapter health, compatibility identity, telemetry inspector, and the live anchor.
#[tool(
    description = "Diagnose the control plane for the open session: adapter health, telemetry inspector, sessions count, and current anchor."
)]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.doctor", &error.to_string()),
    };
    let mut guard = match session.lock() {
        Ok(value) => value,
        Err(error) => return error_payload("fortress.doctor", &error.to_string()),
    };
    let rid = next_request_id(&mut guard);
    let ctx = context_for(&guard, rid);
    let health_res = guard.adapter.health(&ctx);

    let active_sessions_count = sessions().len();
    let health_opt = health_res.as_ref().ok();
    let report = DoctorInspector.generate_report(active_sessions_count, health_opt, None, 0, 0);

    match health_res {
        Ok(health) => json!({
            "ok": true,
            "session_id": format!("{}", guard.session_id),
            "status": if report.is_healthy { "healthy" } else { "degraded" },
            "active_sessions_count": report.active_sessions_count,
            "adapter": health.identity.name,
            "compatibility": format!("{:?}", health.identity.compatibility),
            "fortress_loaded": health.fortress_loaded,
            "findings": report.findings,
            "warnings": health.warnings,
            "current_anchor": health.current_anchor.as_ref().map(anchor_json),
        })
        .to_string(),
        Err(error) => error_payload("fortress.doctor", &error.to_string()),
    }
}

// ============================================================================
// Server assembly & Transport Admission (WP-14)
// ============================================================================

/// Validates that an HTTP bind address is strictly localhost.
/// Non-localhost binds are rejected by design until the transport-boundary
/// admission design lands (WP-14 / FASTMCP_INTEGRATION.md §6).
pub fn validate_localhost_bind(bind_addr: &str) -> Result<()> {
    let host = if let Some(idx) = bind_addr.rfind(':') {
        &bind_addr[..idx]
    } else {
        bind_addr
    };
    let trimmed = host.trim_matches('[').trim_matches(']');
    if trimmed == "127.0.0.1" || trimmed == "::1" || trimmed == "localhost" {
        Ok(())
    } else {
        Err(DfmcpError::new(
            ErrorCode::CapabilityDenied,
            format!(
                "non-localhost bind address '{bind_addr}' rejected; transport-boundary admission requires localhost-only binding"
            ),
        ))
    }
}

/// Run the modern-only MCP 2026-07-28 server on stdio.
pub fn run_stdio() {
    ServerBuilder::new("dwarf-fortress-mcp", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession)
        .tool(FortressObserve)
        .tool(FortressQuery)
        .tool(FortressPlan)
        .tool(FortressCommit)
        .tool(FortressWait)
        .tool(FortressCancel)
        .tool(FortressCheckpoint)
        .tool(FortressRestore)
        .tool(FortressExplain)
        .tool(FortressDoctor)
        .request_timeout(30)
        .instructions(
            "Dwarf Fortress semantic control plane (laboratory slice). Call fortress_open_session \
             first to negotiate capabilities, then supply the returned session_id to every other \
             tool. Each session has its own adapter, anchor, plans, and receipts; concurrent \
             sessions are independent. Transport identity grants nothing; every authority comes \
             from the negotiated grants. Dispatch success is never goal success: only \
             evidence-backed postcondition verification counts.",
        )
        .build()
        .run_stdio();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_snapshot_carries_requested_pause_state() {
        assert!(seed_snapshot(FortressId::new(1), true).paused);
        assert!(!seed_snapshot(FortressId::new(1), false).paused);
    }

    #[test]
    fn seed_snapshot_is_origin_anchored() {
        let snapshot = seed_snapshot(FortressId::new(1), true);
        assert_eq!(snapshot.cursor.epoch, 0);
        assert!(snapshot.hash_is_valid());
    }

    #[test]
    fn parse_capability_request_rejects_unknown_capability() {
        let result =
            parse_capability_request(&[("not_a_capability".to_owned(), "read_only".to_owned())]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_capability_request_rejects_unknown_risk() {
        let result = parse_capability_request(&[("observe".to_owned(), "yolo".to_owned())]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_capability_request_rejects_duplicate_capability() {
        let result = parse_capability_request(&[
            ("observe".to_owned(), "read_only".to_owned()),
            ("observe".to_owned(), "guarded".to_owned()),
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_capability_request_accepts_documented_capabilities()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let result = parse_capability_request(&[
            ("observe".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "reversible".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("restore".to_owned(), "guarded".to_owned()),
            ("doctor".to_owned(), "read_only".to_owned()),
        ]);
        let parsed = result?;
        assert_eq!(parsed.len(), 6);
        assert_eq!(parsed[0].capability, Capability::Observe);
        assert_eq!(parsed[0].max_risk, RiskTier::ReadOnly);
        assert_eq!(parsed[5].capability, Capability::Doctor);
        assert_eq!(parsed[5].max_risk, RiskTier::ReadOnly);
        Ok(())
    }

    #[test]
    fn negotiate_grants_scopes_every_grant_to_the_session_fortress() {
        let grants = negotiate_grants(
            FortressId::new(7),
            &[NegotiatedCapability {
                capability: Capability::Observe,
                max_risk: RiskTier::ReadOnly,
            }],
        );
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].capability, Capability::Observe);
        assert_eq!(grants[0].scope.fortress_id, Some(FortressId::new(7)));
        assert_eq!(grants[0].max_risk, RiskTier::ReadOnly);
    }

    #[test]
    fn session_counter_mints_unique_increasing_session_ids()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let first = next_session_counter()?;
        let second = next_session_counter()?;
        assert!(
            second > first,
            "session_id counter must be strictly monotonic"
        );
        Ok(())
    }

    #[test]
    fn parse_session_id_arg_round_trips_u128() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let raw = "12345";
        let parsed = parse_session_id_arg(raw)?;
        assert_eq!(parsed.get(), 12345u128);
        Ok(())
    }

    #[test]
    fn parse_session_id_arg_rejects_non_decimal() {
        let result = parse_session_id_arg("not-a-number");
        assert!(result.is_err());
    }

    #[test]
    fn lookup_session_returns_session_not_found_for_unknown_id() {
        let result = lookup_session(SessionId::new(u128::MAX));
        assert!(result.is_err());
    }

    #[test]
    fn budget_validation_rejects_zero_dimension() {
        let bad = WorkBudget {
            max_wall_millis: 0,
            ..WorkBudget::CONSERVATIVE_DEFAULT
        };
        assert!(bad.validate().is_err());
    }
}
