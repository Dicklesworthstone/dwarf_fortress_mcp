//! Agent-oriented facade over the phase-zero MCP laboratory.
//!
//! The underlying `server` module remains the authority-bearing laboratory
//! implementation. This facade calls those handlers unchanged and adds the
//! canonical Agent Turn Packet as an authority-free presentation projection.
//! No value cached here participates in authorization, plan sealing,
//! precondition proof, idempotency, effect dispatch, or postcondition proof.

use std::collections::BTreeMap;
use std::sync::{LazyLock, Mutex, MutexGuard};

use crate::agent_turn::{
    AgentPhase, AgentTurnBuilder, ContinuityStatus, ObservationProfile, RecoveryClass,
    recommendation, recovery_guidance, uncertainty,
};
use fastmcp_rust::modern::ServerBuilder;
use fastmcp_rust::prelude::*;
use serde_json::{Value, json};

const MAX_PRESENTATION_SESSIONS: usize = 1_024;
const LAB_IMPLEMENTATION_PHASE: &str = "phase_0c_semantic_contract_laboratory";

#[derive(Clone, Debug)]
struct SessionOrientation {
    anchor: Option<Value>,
    budget: Value,
    grants: Vec<String>,
    paused: Option<bool>,
    adapter: Option<String>,
    compatibility: Option<String>,
    pending_plan_digest: Option<String>,
    last_action_id: Option<String>,
    last_action_state: Option<String>,
    last_checkpoint_id: Option<String>,
    turn_sequence: u64,
}

impl SessionOrientation {
    fn from_open_session(payload: &Value) -> Self {
        Self {
            anchor: extract_anchor(payload, None),
            budget: payload.get("budget").cloned().unwrap_or(Value::Null),
            grants: string_array(payload.get("granted_capabilities")),
            paused: payload.get("paused").and_then(Value::as_bool),
            adapter: payload
                .get("adapter")
                .and_then(Value::as_str)
                .map(str::to_owned),
            compatibility: payload
                .get("compatibility")
                .and_then(Value::as_str)
                .map(str::to_owned),
            pending_plan_digest: None,
            last_action_id: None,
            last_action_state: None,
            last_checkpoint_id: None,
            turn_sequence: 0,
        }
    }

    fn advance_turn(&mut self) -> u64 {
        if let Some(next) = self.turn_sequence.checked_add(1) {
            self.turn_sequence = next;
        }
        self.turn_sequence
    }
}

static ORIENTATION_SESSIONS: LazyLock<Mutex<BTreeMap<String, SessionOrientation>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn orientation_sessions() -> MutexGuard<'static, BTreeMap<String, SessionOrientation>> {
    match ORIENTATION_SESSIONS.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    match value.and_then(Value::as_array) {
        Some(values) => values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        None => Vec::new(),
    }
}

fn is_ok(payload: &Value) -> bool {
    matches!(payload.get("ok").and_then(Value::as_bool), Some(true))
}

fn normalize_anchor(mut anchor: Value, payload: &Value, prior: Option<&Value>) -> Value {
    let game_tick = payload
        .get("game_tick")
        .cloned()
        .or_else(|| prior.and_then(|value| value.get("game_tick").cloned()))
        .or_else(|| {
            payload
                .get("adapter")
                .and_then(Value::as_str)
                .filter(|name| *name == "memory")
                .map(|_| json!(1))
        })
        .unwrap_or(Value::Null);
    if let Some(object) = anchor.as_object_mut() {
        object
            .entry("game_tick".to_owned())
            .or_insert(game_tick);
    }
    anchor
}

fn extract_anchor(payload: &Value, prior: Option<&Value>) -> Option<Value> {
    for field in [
        "restored_anchor",
        "observed_anchor",
        "current_anchor",
        "anchor",
    ] {
        if let Some(value) = payload.get(field) {
            if !value.is_null() {
                return Some(normalize_anchor(value.clone(), payload, prior));
            }
        }
    }

    let fortress_id = payload.get("fortress_id")?.clone();
    let cursor = payload.get("cursor")?.clone();
    let state_hash = payload.get("state_hash")?.clone();
    Some(json!({
        "fortress_id": fortress_id,
        "epoch": cursor.get("epoch").cloned().unwrap_or(Value::Null),
        "sequence": cursor.get("sequence").cloned().unwrap_or(Value::Null),
        "game_tick": payload.get("game_tick").cloned().unwrap_or(Value::Null),
        "state_hash": state_hash,
    }))
}

fn session_id_from(payload: &Value, hint: Option<&str>) -> Option<String> {
    payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| hint.map(str::to_owned))
}

fn first_action(payload: &Value) -> Option<(String, String)> {
    let action = payload.get("actions")?.as_array()?.first()?;
    let id = action.get("action_id")?.as_str()?.to_owned();
    let state = action
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    Some((id, state))
}

fn update_orientation(operation: &str, payload: &Value, state: &mut SessionOrientation) {
    if let Some(anchor) = extract_anchor(payload, state.anchor.as_ref()) {
        state.anchor = Some(anchor);
    }
    if let Some(paused) = payload.get("paused").and_then(Value::as_bool) {
        state.paused = Some(paused);
    }
    if let Some(adapter) = payload.get("adapter").and_then(Value::as_str) {
        state.adapter = Some(adapter.to_owned());
    }
    if let Some(compatibility) = payload.get("compatibility").and_then(Value::as_str) {
        state.compatibility = Some(compatibility.to_owned());
    }
    match operation {
        "fortress.plan" => {
            state.pending_plan_digest = payload
                .get("plan_digest")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        "fortress.commit" => {
            state.pending_plan_digest = None;
            if let Some((id, action_state)) = first_action(payload) {
                state.last_action_id = Some(id);
                state.last_action_state = Some(action_state);
            }
        }
        "fortress.wait" => {
            if let Some(id) = payload.get("action_id").and_then(Value::as_str) {
                state.last_action_id = Some(id.to_owned());
            }
            if let Some(action_state) = payload.get("commit_state").and_then(Value::as_str) {
                state.last_action_state = Some(action_state.to_owned());
            }
        }
        "fortress.cancel" => {
            if let Some(id) = payload.get("action_id").and_then(Value::as_str) {
                state.last_action_id = Some(id.to_owned());
            }
            if let Some(action_state) = payload.get("final_state").and_then(Value::as_str) {
                state.last_action_state = Some(action_state.to_owned());
            }
        }
        "fortress.checkpoint" => {
            state.last_checkpoint_id = payload
                .get("checkpoint_id")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        "fortress.restore" => {
            state.pending_plan_digest = None;
            state.last_action_id = None;
            state.last_action_state = None;
        }
        _ => {}
    }
}

fn active_work(state: &SessionOrientation) -> Value {
    let pending_plans = state
        .pending_plan_digest
        .as_ref()
        .map(|digest| {
            vec![json!({
                "plan_digest": digest,
                "state": "prepared",
                "minimum_safe_next_step": {
                    "tool": "fortress.commit",
                    "arguments": {"plan_digest": digest},
                },
            })]
        })
        .unwrap_or_default();
    let actions = state
        .last_action_id
        .as_ref()
        .map(|action_id| {
            vec![json!({
                "action_id": action_id,
                "state": state.last_action_state.as_deref().unwrap_or("unknown"),
            })]
        })
        .unwrap_or_default();
    json!({
        "pending_plans": pending_plans,
        "actions": actions,
        "obligations": [],
        "cancellation_drains": [],
        "indeterminate_effects": [],
        "publications": [],
        "confirmations": [],
    })
}

fn has_grant(state: &SessionOrientation, grant: &str) -> bool {
    state.grants.iter().any(|candidate| candidate == grant)
}

fn affordance(
    id: &str,
    tool: &str,
    family: &str,
    risk: &str,
    reversible: bool,
    enabled: bool,
    disabled_reason: Option<&str>,
    arguments: Value,
) -> Value {
    json!({
        "affordance_id": id,
        "tool": tool,
        "intent_family": family,
        "risk": risk,
        "reversibility": if reversible { "reversible" } else { "not_applicable" },
        "enabled": enabled,
        "disabled_reason": disabled_reason,
        "known_preconditions": [],
        "unverified_preconditions": [],
        "checkpoint_policy": "registered_policy",
        "confirmation_policy": "registered_policy",
        "estimated_cost": {
            "actions": null,
            "bridge_bytes": null,
            "wall_millis": null,
            "game_ticks": null,
        },
        "arguments": arguments,
    })
}

fn affordances(state: &SessionOrientation) -> Vec<Value> {
    let mut result = Vec::new();
    result.push(affordance(
        "observe-pulse",
        "fortress.observe",
        "observe",
        "read_only",
        false,
        has_grant(state, "observe"),
        (!has_grant(state, "observe")).then_some("observe capability is not granted"),
        json!({"profile": "pulse"}),
    ));
    result.push(affordance(
        "query-summary",
        "fortress.query",
        "query",
        "read_only",
        false,
        has_grant(state, "query"),
        (!has_grant(state, "query")).then_some("query capability is not granted"),
        json!({"mode": "summary"}),
    ));

    if let Some(digest) = state.pending_plan_digest.as_ref() {
        result.push(affordance(
            "commit-pending-plan",
            "fortress.commit",
            "commit_prepared_plan",
            "reversible",
            true,
            has_grant(state, "control_clock"),
            (!has_grant(state, "control_clock"))
                .then_some("control_clock capability is not granted"),
            json!({"plan_digest": digest}),
        ));
    } else if let Some(paused) = state.paused {
        let enabled = has_grant(state, "plan") && has_grant(state, "control_clock");
        result.push(affordance(
            if paused { "plan-resume" } else { "plan-pause" },
            "fortress.plan",
            "control_clock",
            "reversible",
            true,
            enabled,
            (!enabled).then_some("plan and control_clock capabilities are both required"),
            json!({
                "paused_target": !paused,
                "summary": if paused {
                    "resume the laboratory simulation"
                } else {
                    "pause the laboratory simulation"
                },
            }),
        ));
    }

    if let Some(action_id) = state.last_action_id.as_ref() {
        result.push(affordance(
            "inspect-last-action",
            "fortress.wait",
            "verify_active_work",
            "read_only",
            false,
            true,
            None,
            json!({"action_id": action_id}),
        ));
        result.push(affordance(
            "cancel-last-action",
            "fortress.cancel",
            "cancel_active_work",
            "reversible",
            true,
            has_grant(state, "control_clock"),
            (!has_grant(state, "control_clock"))
                .then_some("control_clock capability is not granted"),
            json!({"mode": "compensate_reversible"}),
        ));
    }

    result.push(affordance(
        "checkpoint-current-state",
        "fortress.checkpoint",
        "checkpoint",
        "guarded",
        false,
        has_grant(state, "checkpoint"),
        (!has_grant(state, "checkpoint")).then_some("checkpoint capability is not granted"),
        json!({"label": "agent-checkpoint"}),
    ));
    result
}

fn action_is_nonterminal(state: Option<&str>) -> bool {
    match state {
        Some(value) => !matches!(
            value.to_ascii_lowercase().as_str(),
            "verified" | "compensated" | "cancelled" | "failed"
        ),
        None => false,
    }
}

fn recommendations(
    operation: &str,
    ok: bool,
    payload: &Value,
    state: &SessionOrientation,
) -> Vec<Value> {
    if !ok {
        let code = payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        return match code {
            "session_not_found" => vec![recommendation(
                "recover-open-session",
                "fortress.open_session",
                "the supplied session does not exist",
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({}),
            )],
            "cursor_gap" | "stale_anchor" => vec![recommendation(
                "recover-refresh-anchor",
                "fortress.observe",
                "refresh the canonical anchor before retrying",
                "high",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({"profile": "briefing"}),
            )],
            "invalid_plan" | "conflict" => vec![recommendation(
                "recover-replan",
                "fortress.plan",
                "the prepared plan is absent or conflicts with current protocol state",
                "high",
                "medium",
                "reversible",
                "reversible",
                false,
                json!({}),
            )],
            "indeterminate_effect" => vec![recommendation(
                "recover-reconcile",
                "fortress.wait",
                "blind retry could duplicate an effect; reconcile first",
                "critical",
                "high",
                "read_only",
                "not_applicable",
                false,
                json!({}),
            )],
            _ => Vec::new(),
        };
    }

    if let Some(digest) = state.pending_plan_digest.as_ref() {
        return vec![recommendation(
            "commit-prepared-plan",
            "fortress.commit",
            "a sealed prepared plan is awaiting the exact commit protocol step",
            "high",
            "low",
            "reversible",
            "reversible",
            false,
            json!({"plan_digest": digest}),
        )];
    }
    if action_is_nonterminal(state.last_action_state.as_deref()) {
        return vec![recommendation(
            "verify-active-action",
            "fortress.wait",
            "active work has not reached an ordinary terminal state",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({}),
        )];
    }
    if matches!(operation, "fortress.open_session" | "fortress.restore") {
        return vec![recommendation(
            "orient-briefing",
            "fortress.observe",
            "establish an explicit bounded semantic baseline at the current observation epoch",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({"profile": "briefing"}),
        )];
    }
    Vec::new()
}

fn changes(operation: &str, ok: bool, payload: &Value) -> Vec<Value> {
    if !ok {
        return Vec::new();
    }
    match operation {
        "fortress.plan" => payload
            .get("plan_digest")
            .and_then(Value::as_str)
            .map(|digest| {
                vec![json!({
                    "kind": "plan_prepared",
                    "subject": {"plan_digest": digest},
                    "epistemic_state": "observed",
                    "invalidates": [],
                    "evidence": [],
                })]
            })
            .unwrap_or_default(),
        "fortress.commit" => vec![json!({
            "kind": "plan_commit_observed",
            "subject": {"plan_digest": payload.get("plan_digest").cloned().unwrap_or(Value::Null)},
            "epistemic_state": "observed",
            "invalidates": ["pending_plan_handle"],
            "evidence": [],
        })],
        "fortress.cancel" => vec![json!({
            "kind": "cancellation_finalized",
            "subject": {"action_id": payload.get("action_id").cloned().unwrap_or(Value::Null)},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
        })],
        "fortress.checkpoint" => vec![json!({
            "kind": "checkpoint_created",
            "subject": {"checkpoint_id": payload.get("checkpoint_id").cloned().unwrap_or(Value::Null)},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
        })],
        "fortress.restore" => vec![json!({
            "kind": "observation_epoch_reset",
            "subject": {"checkpoint_id": payload.get("checkpoint_id").cloned().unwrap_or(Value::Null)},
            "epistemic_state": "observed",
            "invalidates": ["all_pre_restore_plans", "all_pre_restore_action_handles", "all_pre_restore_continuations"],
            "evidence": [],
        })],
        _ => Vec::new(),
    }
}

fn attention(operation: &str, ok: bool, payload: &Value, state: &SessionOrientation) -> Vec<Value> {
    if !ok {
        return vec![json!({
            "attention_id": "protocol-error",
            "category": "control_plane",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": payload.get("error").and_then(|value| value.get("message")).cloned().unwrap_or(json!("tool call failed")),
            "likely_consequence_if_ignored": "the agent may operate from an invalid protocol state",
            "evidence": [],
        })];
    }
    if state.pending_plan_digest.is_some() {
        return vec![json!({
            "attention_id": "prepared-plan-awaiting-decision",
            "category": "active_work",
            "severity": "medium",
            "urgency": "before_formulating_an_unrelated_plan",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "a sealed plan is awaiting commit, replacement, or explicit abandonment",
            "likely_consequence_if_ignored": "the agent may lose track of unfinished protocol state",
            "evidence": [],
        })];
    }
    if action_is_nonterminal(state.last_action_state.as_deref()) {
        return vec![json!({
            "attention_id": "action-awaiting-verification",
            "category": "active_work",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "an action is not yet in an ordinary terminal state",
            "likely_consequence_if_ignored": "goal completion or failure may be misclassified",
            "evidence": [],
        })];
    }
    if operation == "fortress.restore" {
        return vec![json!({
            "attention_id": "restore-invalidated-prior-context",
            "category": "continuity",
            "severity": "critical",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "the observation epoch changed and all pre-restore handles are stale",
            "likely_consequence_if_ignored": "the agent may act on invalid plans or continuations",
            "evidence": [],
        })];
    }
    Vec::new()
}

fn briefing(state: &SessionOrientation) -> Value {
    let mutation_admissible = has_grant(state, "plan") && has_grant(state, "control_clock");
    json!({
        "implementation_phase": LAB_IMPLEMENTATION_PHASE,
        "adapter": state.adapter,
        "compatibility": state.compatibility,
        "fortress_loaded": true,
        "paused": state.paused,
        "mission": null,
        "objective_status": [],
        "mutation_admissible": mutation_admissible,
        "highest_unresolved_uncertainty": "no live DFHack observation or mutation path is implemented",
    })
}

fn coverage(payload: &Value) -> Value {
    let truncated = matches!(payload.get("truncated").and_then(Value::as_bool), Some(true));
    json!({
        "status": if truncated { "partial" } else { "complete_for_named_projection" },
        "complete_domains": ["laboratory.pause_state", "laboratory.protocol_state"],
        "partial_domains": [],
        "omitted_domains": [
            {"domain": "live_dwarf_fortress", "reason": "no live DFHack adapter is implemented"},
            {"domain": "units_items_jobs_map", "reason": "the phase-zero memory adapter models pause state only"}
        ],
        "continuation": payload.get("continuation").cloned().unwrap_or(Value::Null),
        "absence_proof_scope": ["laboratory.pause_state", "laboratory.protocol_state"],
    })
}

fn budget(state: &SessionOrientation) -> Value {
    json!({
        "requested": state.budget,
        "admitted": state.budget,
        "consumed": {
            "canonical_reads": null,
            "derived_queries": null,
            "bridge_bytes": 0,
            "graph_operations": null,
            "search_operations": 0,
            "planning_expansions": null,
            "candidate_simulation": 0,
            "evidence_materialization": null,
            "output_bytes": null,
            "output_tokens": null,
        },
        "remaining": null,
        "soft_stop_reason": null,
        "hard_stop_reason": null,
    })
}

fn uncertainties(state: &SessionOrientation) -> Vec<Value> {
    let mut result = vec![uncertainty(
        "live-adapter-not-implemented",
        "unknown",
        "the process-local laboratory is not connected to Dwarf Fortress or DFHack",
        "no response in this session establishes live fortress state or live effects",
        Some("fortress.doctor"),
        json!({}),
    )];
    if state.anchor.is_none() {
        result.push(uncertainty(
            "anchor-not-projected",
            "unknown",
            "the underlying tool response did not expose a complete canonical anchor",
            "the response must not be used as a mutation basis",
            Some("fortress.observe"),
            json!({"profile": "briefing"}),
        ));
    }
    result
}

fn recovery_class(payload: &Value) -> RecoveryClass {
    let code = payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    match code {
        "cursor_gap" | "stale_anchor" => RecoveryClass::RefreshAndRetry,
        "invalid_plan" | "conflict" => RecoveryClass::RebaseRequired,
        "indeterminate_effect" => RecoveryClass::ReconciliationRequired,
        "capability_denied" => RecoveryClass::OperatorActionRequired,
        "budget_exceeded" => RecoveryClass::NeverUnchanged,
        "session_not_found" | "invalid_request" => RecoveryClass::NeverUnchanged,
        _ => RecoveryClass::OperatorActionRequired,
    }
}

fn attach_recovery(payload: &mut Value) {
    if is_ok(payload) {
        return;
    }
    let class = recovery_class(payload);
    let (tool, reason) = match class {
        RecoveryClass::RefreshAndRetry => (
            Some("fortress.observe"),
            "refresh the canonical anchor before retrying",
        ),
        RecoveryClass::RebaseRequired => (
            Some("fortress.plan"),
            "recompile intent against the current anchor",
        ),
        RecoveryClass::ReconciliationRequired => (
            Some("fortress.wait"),
            "reconcile the possible effect before any retry",
        ),
        RecoveryClass::NeverUnchanged => (None, "change the request or establish a new session"),
        RecoveryClass::SafeReadRetry => (None, "retry the bounded read"),
        RecoveryClass::Backoff => (None, "retry only after bounded backoff"),
        RecoveryClass::ConfirmationRequired => (None, "obtain the registered confirmation"),
        RecoveryClass::OperatorActionRequired => (None, "operator action is required"),
    };
    if let Some(error) = payload.get_mut("error").and_then(Value::as_object_mut) {
        error.insert(
            "recovery".to_owned(),
            recovery_guidance(class, tool, reason, json!({})),
        );
    }
}

fn continuity_status(
    operation: &str,
    ok: bool,
    previous_anchor: Option<&Value>,
    current_anchor: Option<&Value>,
    payload: &Value,
) -> ContinuityStatus {
    if !ok {
        let code = payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str);
        return if code == Some("indeterminate_effect") {
            ContinuityStatus::Indeterminate
        } else if previous_anchor.is_some() {
            ContinuityStatus::Stale
        } else {
            ContinuityStatus::Bootstrap
        };
    }
    if operation == "fortress.open_session" {
        return ContinuityStatus::Bootstrap;
    }
    if operation == "fortress.restore" {
        return ContinuityStatus::Reset;
    }
    match (previous_anchor, current_anchor) {
        (Some(previous), Some(current)) if previous == current && operation == "fortress.observe" => {
            ContinuityStatus::Heartbeat
        }
        (Some(_), Some(_)) => ContinuityStatus::Continuous,
        (None, Some(_)) => ContinuityStatus::Bootstrap,
        (Some(_), None) => ContinuityStatus::Stale,
        (None, None) => ContinuityStatus::Stale,
    }
}

fn malformed_payload(operation: &str) -> Value {
    json!({
        "ok": false,
        "error": {
            "operation": operation,
            "code": "internal_invariant_violation",
            "message": "the authority-bearing handler returned malformed JSON",
            "retryable": false,
            "details": [],
        },
    })
}

fn project_response(
    raw: String,
    operation: &str,
    phase: AgentPhase,
    profile: ObservationProfile,
    session_hint: Option<&str>,
) -> String {
    let mut payload: Value = match serde_json::from_str(&raw) {
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => malformed_payload(operation),
    };
    attach_recovery(&mut payload);
    let ok = is_ok(&payload);
    let session_id = session_id_from(&payload, session_hint);

    let (state, previous_anchor, turn_sequence) = if let Some(id) = session_id.as_ref() {
        let mut registry = orientation_sessions();
        if operation == "fortress.open_session" && ok {
            if registry.len() >= MAX_PRESENTATION_SESSIONS && !registry.contains_key(id) {
                payload = json!({
                    "ok": false,
                    "error": {
                        "operation": operation,
                        "code": "budget_exceeded",
                        "message": "agent-orientation projection reached its explicit session bound",
                        "retryable": false,
                        "details": [],
                    },
                });
            } else {
                registry.insert(id.clone(), SessionOrientation::from_open_session(&payload));
            }
        }
        if let Some(existing) = registry.get_mut(id) {
            let prior = existing.anchor.clone();
            if is_ok(&payload) {
                update_orientation(operation, &payload, existing);
            }
            let sequence = existing.advance_turn();
            (existing.clone(), prior, sequence)
        } else {
            let mut fallback = SessionOrientation::from_open_session(&payload);
            let sequence = fallback.advance_turn();
            (fallback, None, sequence)
        }
    } else {
        let mut fallback = SessionOrientation::from_open_session(&payload);
        let sequence = fallback.advance_turn();
        (fallback, None, sequence)
    };

    let current_anchor = state.anchor.clone();
    let status = continuity_status(
        operation,
        is_ok(&payload),
        previous_anchor.as_ref(),
        current_anchor.as_ref(),
        &payload,
    );
    let reset_reason = (status == ContinuityStatus::Reset)
        .then_some("checkpoint_restore_created_new_observation_epoch".to_owned());
    let refs = match session_id.as_ref() {
        Some(id) => vec![
            json!({"kind": "resource", "uri": format!("df://session/{id}/summary")}),
            json!({"kind": "resource", "uri": format!("df://session/{id}/capabilities")}),
        ],
        None => Vec::new(),
    };

    AgentTurnBuilder::new(operation, phase)
        .request_id(format!("presentation-turn-{turn_sequence}"))
        .continuity(
            status,
            previous_anchor,
            None,
            reset_reason,
        )
        .profile(profile)
        .briefing(briefing(&state))
        .changes(changes(operation, is_ok(&payload), &payload))
        .attention(attention(operation, is_ok(&payload), &payload, &state))
        .active_work(active_work(&state))
        .affordances(affordances(&state))
        .recommendations(recommendations(operation, is_ok(&payload), &payload, &state))
        .uncertainty(uncertainties(&state))
        .coverage(coverage(&payload))
        .budget(budget(&state))
        .references(refs)
        .pipe(|builder| match session_id {
            Some(id) => builder.session_id(id),
            None => builder,
        })
        .pipe(|builder| match current_anchor {
            Some(anchor) => builder.anchor(anchor),
            None => builder,
        })
        .attach(payload)
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[tool(
    description = "Open an agent-oriented fortress session against the deterministic laboratory. The result includes the canonical orientation packet, negotiated capabilities, budget, continuity, affordances, uncertainty, and safest next protocol step."
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
    let raw = crate::server::fortress_open_session(
        paused,
        fortress_selector,
        requested_capabilities,
        max_wall_millis,
        max_game_ticks,
        max_entities,
        max_bytes,
        max_output_tokens,
        max_actions,
    );
    project_response(
        raw,
        "fortress.open_session",
        AgentPhase::Bootstrap,
        ObservationProfile::Briefing,
        None,
    )
}

#[tool(
    description = "Observe the current laboratory fortress through the canonical agent orientation packet. The current executable slice provides a bounded briefing over pause and protocol state; live DFHack state is explicitly unknown."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    let raw = crate::server::fortress_observe(session_id.clone());
    project_response(
        raw,
        "fortress.observe",
        AgentPhase::Orient,
        ObservationProfile::Briefing,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Run the bounded laboratory summary query and return explicit coverage, epistemic limits, affordances, and next-step guidance in the canonical agent packet."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    let raw = crate::server::fortress_query(session_id.clone(), mode);
    project_response(
        raw,
        "fortress.query",
        AgentPhase::Inspect,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Compile a pause/resume intent without effects. The result keeps the sealed plan visible as active work and recommends the exact safe protocol continuation."
)]
pub fn fortress_plan(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
) -> String {
    let raw = crate::server::fortress_plan(session_id.clone(), summary, paused_target);
    project_response(
        raw,
        "fortress.plan",
        AgentPhase::Propose,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Revalidate and idempotently commit the pending plan, then return observed effect state, continuity, active work, and verification guidance."
)]
pub fn fortress_commit(session_id: Option<String>, plan_digest: String) -> String {
    let raw = crate::server::fortress_commit(session_id.clone(), plan_digest);
    project_response(
        raw,
        "fortress.commit",
        AgentPhase::Commit,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Poll active work and return meaningful verification state in the canonical agent packet."
)]
pub fn fortress_wait(session_id: Option<String>) -> String {
    let raw = crate::server::fortress_wait(session_id.clone());
    project_response(
        raw,
        "fortress.wait",
        AgentPhase::Verify,
        ObservationProfile::Pulse,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Request, drain, compensate when authorized, and finalize cancellation while preserving the active-work and recovery record."
)]
pub fn fortress_cancel(session_id: Option<String>, mode: Option<String>) -> String {
    let raw = crate::server::fortress_cancel(session_id.clone(), mode);
    project_response(
        raw,
        "fortress.cancel",
        AgentPhase::Reconcile,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Create a content-addressed laboratory checkpoint and return continuity, recovery affordances, and bounded state coverage."
)]
pub fn fortress_checkpoint(session_id: Option<String>, label: Option<String>) -> String {
    let raw = crate::server::fortress_checkpoint(session_id.clone(), label);
    project_response(
        raw,
        "fortress.checkpoint",
        AgentPhase::Commit,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Restore a checkpoint into a new observation epoch and make every invalidated pre-restore handle explicit."
)]
pub fn fortress_restore(session_id: Option<String>, checkpoint_id: String) -> String {
    let raw = crate::server::fortress_restore(session_id.clone(), checkpoint_id);
    project_response(
        raw,
        "fortress.restore",
        AgentPhase::Reconcile,
        ObservationProfile::Forensic,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Explain recent state transitions or graph dependencies with explicit epistemic and coverage limits."
)]
pub fn fortress_explain(session_id: Option<String>, entity_id: Option<String>) -> String {
    let raw = crate::server::fortress_explain(session_id.clone(), entity_id);
    project_response(
        raw,
        "fortress.explain",
        AgentPhase::Inspect,
        ObservationProfile::Forensic,
        session_id.as_deref(),
    )
}

#[tool(
    description = "Diagnose the control plane and return findings, uncertainties, recovery guidance, and the canonical orientation packet."
)]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    let raw = crate::server::fortress_doctor(session_id.clone());
    project_response(
        raw,
        "fortress.doctor",
        AgentPhase::Inspect,
        ObservationProfile::Forensic,
        session_id.as_deref(),
    )
}

/// Run the modern-only MCP 2026-07-28 server with the agent-oriented facade.
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
             first and pass its session_id to every later call. Every result includes an \
             agent_turn packet answering what is known, what changed, what matters, what active \
             work exists, which semantic affordances are legal, what the safest next protocol \
             steps are, and what remains uncertain. The packet is a projection, never authority. \
             Dispatch success is not goal success; only authoritative observation and \
             postcondition proof count. The current adapter is process-local and does not claim \
             live Dwarf Fortress or DFHack control.",
        )
        .build()
        .run_stdio();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(raw: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(raw)?)
    }

    #[test]
    fn open_session_orients_a_cold_agent_in_one_response()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let response = parsed(&fortress_open_session(
            Some(true),
            Some("918273645".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ))?;
        assert_eq!(response["ok"], true);
        assert_eq!(response["agent_turn"]["phase"], "bootstrap");
        assert_eq!(response["agent_turn"]["continuity"]["status"], "bootstrap");
        assert_eq!(response["agent_turn"]["briefing"]["paused"], true);
        assert!(response["agent_turn"]["affordances"].as_array().is_some());
        assert!(response["agent_turn"]["recommendations"].as_array().is_some());
        assert_eq!(
            response["agent_turn"]["uncertainty"][0]["uncertainty_id"],
            "live-adapter-not-implemented"
        );
        Ok(())
    }

    #[test]
    fn prepared_plan_is_visible_without_remembering_an_old_handle()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let opened = parsed(&fortress_open_session(
            Some(true),
            Some("918273646".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ))?;
        let session_id = opened["session_id"]
            .as_str()
            .ok_or("open_session did not return a session_id")?
            .to_owned();
        let planned = parsed(&fortress_plan(
            Some(session_id),
            Some("resume the lab".to_owned()),
            Some(false),
        ))?;
        assert_eq!(planned["ok"], true);
        assert_eq!(
            planned["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
            planned["plan_digest"]
        );
        assert_eq!(
            planned["agent_turn"]["recommendations"][0]["tool"],
            "fortress.commit"
        );
        Ok(())
    }

    #[test]
    fn repeated_unchanged_observation_is_an_explicit_heartbeat()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let opened = parsed(&fortress_open_session(
            Some(true),
            Some("918273647".to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ))?;
        let session_id = opened["session_id"]
            .as_str()
            .ok_or("open_session did not return a session_id")?
            .to_owned();
        let first = parsed(&fortress_observe(Some(session_id.clone())))?;
        let second = parsed(&fortress_observe(Some(session_id)))?;
        assert!(matches!(
            first["agent_turn"]["continuity"]["status"].as_str(),
            Some("continuous") | Some("heartbeat")
        ));
        assert_eq!(
            second["agent_turn"]["continuity"]["status"],
            "heartbeat"
        );
        Ok(())
    }

    #[test]
    fn errors_keep_recovery_and_agent_orientation() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let response = parsed(&fortress_observe(None))?;
        assert_eq!(response["ok"], false);
        assert_eq!(response["agent_turn"]["operation"], "fortress.observe");
        assert!(response["error"]["recovery"]["class"].is_string());
        assert!(response["agent_turn"]["uncertainty"].as_array().is_some());
        Ok(())
    }
}
