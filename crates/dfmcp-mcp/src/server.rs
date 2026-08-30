//! Modern-only (MCP 2026-07-28) stdio server for the fortress narrow waist.
//!
//! Slice-one scope (WP-13 gate 1): a single process-local laboratory session
//! over `MemoryAdapter`. Tool names render the logical `fortress.*` surface
//! with the dot as an underscore (`fortress_open_session`); the logical
//! dotted names remain the registry identities used by `schemas/` and the
//! design registries. Session-scoped state, Streamable HTTP, and the MCP
//! Tasks/obligation binding are the next gates; see
//! `docs/FASTMCP_INTEGRATION.md`.

use std::sync::{LazyLock, Mutex, MutexGuard};

use dfmcp_adapter::{CancelMode, GameAdapter, HealthStatus};
use dfmcp_core::{
    ActionId, Capability, CapabilityGrant, CapabilityScope, CheckpointId, FortressId, GameTick,
    IntentId, ObservationCursor, OperationContext, RequestId, RiskTier, SessionId, StateAnchor,
    WorkBudget,
};
use dfmcp_intent::{Action, Constraint, Intent, PreparedPlan, RequestedAction, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Predicate, WorldGraph, WorldSnapshot};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::json;

/// Process-local laboratory session state.
///
/// Slice-one scope: one session per process. Before any multi-client
/// deployment this must move into per-session storage (the facade's session
/// state map or an owned Tasks store) with capability grants negotiated per
/// session rather than baked into `context_for`.
struct LabState {
    adapter: Option<MemoryAdapter>,
    pending: Option<PendingPlan>,
    last_action: Option<ActionId>,
    last_commit: Option<(String, String)>,
    next_request_id: u128,
}

/// A plan sealed by `fortress_plan` and awaiting `fortress_commit`.
struct PendingPlan {
    plan: PreparedPlan,
    digest: String,
}

static LAB: LazyLock<Mutex<LabState>> = LazyLock::new(|| {
    Mutex::new(LabState {
        adapter: None,
        pending: None,
        last_action: None,
        last_commit: None,
        next_request_id: 0,
    })
});

fn lab() -> MutexGuard<'static, LabState> {
    // The guarded state stays internally consistent even if a caller panics
    // while holding the lock; never turn poisoning into a missed unlock.
    LAB.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn next_request_id(state: &mut LabState) -> u128 {
    state.next_request_id = state.next_request_id.wrapping_add(1);
    state.next_request_id
}

fn seed_snapshot(paused: bool) -> WorldSnapshot {
    WorldSnapshot::new(
        FortressId::new(1),
        GameTick(1),
        ObservationCursor::ORIGIN,
        paused,
        WorldGraph::default(),
    )
}

fn context_for(snapshot: &WorldSnapshot, request_id: u128) -> OperationContext {
    let capabilities = [
        (Capability::Observe, RiskTier::ReadOnly),
        (Capability::Query, RiskTier::ReadOnly),
        (Capability::Plan, RiskTier::ReadOnly),
        (Capability::ControlClock, RiskTier::Reversible),
        (Capability::Checkpoint, RiskTier::Guarded),
        (Capability::Restore, RiskTier::Guarded),
    ];
    OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(request_id),
        anchor: snapshot.anchor(),
        budget: WorkBudget::default(),
        grants: capabilities
            .into_iter()
            .map(|(capability, max_risk)| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(snapshot.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
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

// ============================================================================
// fortress.open_session
// ============================================================================

/// Open a laboratory fortress session: seed the deterministic adapter,
/// health-check it, and return the initial anchor with granted capabilities.
#[tool(
    description = "Open a fortress session against the deterministic laboratory adapter. Returns the initial anchor and the capabilities granted to this session."
)]
fn fortress_open_session(_ctx: &McpContext, paused: bool) -> String {
    let mut state = lab();
    let mut adapter = MemoryAdapter::new(seed_snapshot(paused));
    let rid = next_request_id(&mut state);
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.health(&ctx) {
        Ok(health) => {
            let payload = json!({
                "ok": true,
                "adapter": health.identity.name,
                "compatibility": format!("{:?}", health.identity.compatibility),
                "fortress_loaded": health.fortress_loaded,
                "granted_capabilities": [
                    "observe", "query", "plan", "control_clock", "checkpoint", "restore",
                ],
                "anchor": anchor_json(&adapter.snapshot().anchor()),
                "paused": adapter.snapshot().paused,
            });
            state.adapter = Some(adapter);
            state.pending = None;
            state.last_action = None;
            state.last_commit = None;
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
    description = "Observe the current fortress state: anchor, game tick, pause state, and canonical state hash. Bounded summary projection over the laboratory adapter."
)]
fn fortress_observe(_ctx: &McpContext) -> String {
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(adapter) = state.adapter.as_mut() else {
        return error_payload(
            "fortress.observe",
            "no open session; call fortress_open_session first",
        );
    };
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.health(&ctx) {
        Ok(_) => {
            let mut payload = snapshot_json(adapter.snapshot());
            payload["projection"] = json!("summary");
            payload.to_string()
        }
        Err(error) => error_payload("fortress.observe", &error.to_string()),
    }
}

// ============================================================================
// fortress.query
// ============================================================================

/// Bounded structured query. The laboratory slice supports `mode = "summary"`;
/// full DfQL execution is WP-04 and is rejected with a stable message here.
#[tool(
    description = "Run a bounded query against the fortress state. The laboratory slice supports mode=\"summary\" only; full DfQL arrives with WP-04."
)]
fn fortress_query(_ctx: &McpContext, mode: String) -> String {
    if mode != "summary" {
        return error_payload(
            "fortress.query",
            "only mode=\"summary\" is supported by the laboratory slice; full DfQL is WP-04",
        );
    }
    let state = lab();
    let Some(adapter) = state.adapter.as_ref() else {
        return error_payload(
            "fortress.query",
            "no open session; call fortress_open_session first",
        );
    };
    let mut payload = snapshot_json(adapter.snapshot());
    payload["matched"] = json!(0);
    payload["note"] = json!("laboratory summary projection; typed DfQL is WP-04");
    payload.to_string()
}

// ============================================================================
// fortress.plan
// ============================================================================

/// Compile a pause/resume intent into a sealed, inspectable plan. The
/// laboratory registry registers exactly one semantic action family
/// (clock pause/resume); broader registries extend schemas, not tools.
#[tool(
    description = "Compile an intent into an immutable, inspectable plan without effects. Returns the plan id, digest, and terminal condition. The laboratory registry supports the clock pause/resume action family."
)]
fn fortress_plan(_ctx: &McpContext, summary: String, paused_target: bool) -> String {
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(adapter) = state.adapter.as_ref() else {
        return error_payload(
            "fortress.plan",
            "no open session; call fortress_open_session first",
        );
    };
    let snapshot = adapter.snapshot();
    let ctx = context_for(snapshot, rid);
    let intent = Intent {
        id: IntentId::new(rid),
        anchor: snapshot.anchor(),
        summary,
        terminal_condition: Predicate::Paused(paused_target),
        constraints: vec![Constraint::MaxRisk(RiskTier::Reversible)],
        requested_actions: vec![RequestedAction {
            action: Action::Pause {
                paused: paused_target,
            },
            preconditions: vec![Predicate::Paused(!paused_target)],
            postconditions: Vec::new(),
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
                "plan_id": format!("{}", plan.id),
                "plan_digest": digest,
                "terminal_condition": format!("{:?}", intent.terminal_condition),
                "max_risk": "reversible",
                "note": "sealed plan; commit it with fortress_commit before expiry",
            });
            state.pending = Some(PendingPlan {
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
    description = "Commit the pending prepared plan: prepare/revalidate, dispatch, observe, and verify. Requires the exact plan digest returned by fortress_plan."
)]
fn fortress_commit(_ctx: &McpContext, plan_digest: String) -> String {
    let mut state = lab();
    let pending = match state.pending.take() {
        Some(pending) => pending,
        None => {
            // ADR-006 idempotency: a duplicate commit with the same digest
            // returns the prior receipt verbatim instead of erroring or
            // reapplying effects.
            if let Some((digest, payload)) = &state.last_commit
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
    let mut adapter = match state.adapter.take() {
        Some(adapter) => adapter,
        None => {
            return error_payload(
                "fortress.commit",
                "no open session; call fortress_open_session first",
            );
        }
    };
    let rid = next_request_id(&mut state);
    let prepare_ctx = context_for(adapter.snapshot(), rid);
    match adapter.prepare(&pending.plan, &prepare_ctx) {
        Ok(prepared) => {
            let rid = next_request_id(&mut state);
            let commit_ctx = context_for(adapter.snapshot(), rid);
            match adapter.commit(&pending.plan, &prepared, &commit_ctx) {
                Ok(receipt) => {
                    state.last_action = receipt.actions.first().map(|action| action.action_id);
                    let snapshot = adapter.snapshot();
                    let paused = snapshot.paused;
                    let payload = json!({
                        "ok": true,
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
                    state.last_commit = Some((plan_digest.clone(), payload_text.clone()));
                    state.adapter = Some(adapter);
                    payload_text
                }
                Err(error) => {
                    state.adapter = Some(adapter);
                    state.pending = Some(pending);
                    error_payload("fortress.commit", &error.to_string())
                }
            }
        }
        Err(error) => {
            state.adapter = Some(adapter);
            state.pending = Some(pending);
            error_payload("fortress.commit", &error.to_string())
        }
    }
}

// ============================================================================
// fortress.wait
// ============================================================================

/// Poll the most recent committed action and return its current state.
#[tool(
    description = "Poll the most recent committed action. Returns the action receipt state from the laboratory adapter's bounded obligation machinery."
)]
fn fortress_wait(_ctx: &McpContext) -> String {
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(action_id) = state.last_action else {
        return error_payload(
            "fortress.wait",
            "no committed action yet; call fortress_commit first",
        );
    };
    let Some(adapter) = state.adapter.as_mut() else {
        return error_payload(
            "fortress.wait",
            "no open session; call fortress_open_session first",
        );
    };
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.poll_action(action_id, &ctx) {
        Ok(receipt) => json!({
            "ok": true,
            "action_id": format!("{}", receipt.action_id),
            "state": format!("{:?}", receipt.state),
            "message": receipt.message,
            "observed_anchor": anchor_json(&receipt.observed_anchor),
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
    description = "Cancel the most recent committed action: request, drain, compensate when authorized, and finalize. Returns the final cancellation receipt."
)]
fn fortress_cancel(_ctx: &McpContext) -> String {
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(action_id) = state.last_action else {
        return error_payload(
            "fortress.cancel",
            "no committed action to cancel; call fortress_commit first",
        );
    };
    let Some(adapter) = state.adapter.as_mut() else {
        return error_payload(
            "fortress.cancel",
            "no open session; call fortress_open_session first",
        );
    };
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.request_cancel(action_id, CancelMode::CompensateReversible, &ctx) {
        Ok(request) => match adapter.finalize_cancel(action_id, &ctx) {
            Ok(finalized) => json!({
                "ok": true,
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
    description = "Create a labeled checkpoint: a content-addressed recovery point with an evidence record."
)]
fn fortress_checkpoint(_ctx: &McpContext, label: String) -> String {
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(adapter) = state.adapter.as_mut() else {
        return error_payload(
            "fortress.checkpoint",
            "no open session; call fortress_open_session first",
        );
    };
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.checkpoint(&label, &ctx) {
        Ok(receipt) => json!({
            "ok": true,
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
    description = "Restore a checkpoint by id. Creates a new observation epoch; stale plans and action handles are invalidated."
)]
fn fortress_restore(_ctx: &McpContext, checkpoint_id: String) -> String {
    let parsed = match checkpoint_id.parse::<u128>() {
        Ok(value) => value,
        Err(_) => {
            return error_payload(
                "fortress.restore",
                "checkpoint_id must be a u128 decimal string",
            );
        }
    };
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(adapter) = state.adapter.as_mut() else {
        return error_payload(
            "fortress.restore",
            "no open session; call fortress_open_session first",
        );
    };
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.restore(CheckpointId::new(parsed), &ctx) {
        Ok(receipt) => {
            state.pending = None;
            state.last_action = None;
            json!({
                "ok": true,
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

/// Explain recent state transitions from the immutable laboratory transcript.
#[tool(
    description = "Explain what happened: return the most recent laboratory transcript events as evidence-backed rationale for the current state."
)]
fn fortress_explain(_ctx: &McpContext) -> String {
    let state = lab();
    let Some(adapter) = state.adapter.as_ref() else {
        return error_payload(
            "fortress.explain",
            "no open session; call fortress_open_session first",
        );
    };
    let events = adapter.transcript();
    let start = events.len().saturating_sub(16);
    let recent: Vec<String> = events[start..]
        .iter()
        .map(|event| format!("{event:?}"))
        .collect();
    json!({
        "ok": true,
        "transcript_len": events.len(),
        "recent_events": recent,
        "note": "the laboratory transcript is the evidence ledger; production explanations cite durable evidence bundles",
    })
    .to_string()
}

// ============================================================================
// fortress.doctor
// ============================================================================

/// Diagnose adapter health, compatibility identity, and the live anchor.
#[tool(
    description = "Diagnose the control plane: adapter health, compatibility identity, fortress load state, and the current anchor."
)]
fn fortress_doctor(_ctx: &McpContext) -> String {
    let mut state = lab();
    let rid = next_request_id(&mut state);
    let Some(adapter) = state.adapter.as_mut() else {
        return error_payload(
            "fortress.doctor",
            "no open session; call fortress_open_session first",
        );
    };
    let ctx = context_for(adapter.snapshot(), rid);
    match adapter.health(&ctx) {
        Ok(health) => {
            let status = match health.status {
                HealthStatus::Healthy => "healthy",
                HealthStatus::Degraded => "degraded",
                HealthStatus::ReadOnly => "read_only",
                HealthStatus::Unavailable => "unavailable",
            };
            json!({
                "ok": true,
                "status": status,
                "adapter": health.identity.name,
                "compatibility": format!("{:?}", health.identity.compatibility),
                "fortress_loaded": health.fortress_loaded,
                "warnings": health.warnings,
                "current_anchor": health.current_anchor.as_ref().map(anchor_json),
            })
            .to_string()
        }
        Err(error) => error_payload("fortress.doctor", &error.to_string()),
    }
}

// ============================================================================
// Server assembly
// ============================================================================

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
            "Dwarf Fortress semantic control plane (laboratory slice). Open a session, \
             observe the bounded snapshot, compile pause/resume intents into sealed plans, \
             commit them with exact plan digests, and wait/cancel via the bounded \
             obligation machinery. Dispatch success is never goal success: only \
             evidence-backed postcondition verification counts.",
        )
        .build()
        .run_stdio();
}

#[cfg(test)]
mod tests {
    use super::seed_snapshot;

    #[test]
    fn seed_snapshot_carries_requested_pause_state() {
        assert!(seed_snapshot(true).paused);
        assert!(!seed_snapshot(false).paused);
    }

    #[test]
    fn seed_snapshot_is_origin_anchored() {
        let snapshot = seed_snapshot(true);
        assert_eq!(snapshot.cursor.epoch, 0);
        assert!(snapshot.hash_is_valid());
    }
}
