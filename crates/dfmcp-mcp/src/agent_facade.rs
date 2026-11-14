//! Agent-oriented facade over the authority-bearing phase-zero laboratory.
//!
//! The underlying `server` handlers retain all authority, validation, plan
//! sealing, idempotency, effect dispatch, observation, and proof semantics.
//! This module performs one bounded, deterministic, authority-free projection:
//! it attaches the canonical Agent Turn Packet to each tool-specific result.
//! Presentation cache contents can improve orientation but can never grant a
//! capability, satisfy a precondition, fence a writer, or prove an effect.

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

/// One committed plan action as the agent last observed it.
#[derive(Clone, Debug)]
struct PlanActionView {
    action_id: String,
    step: u64,
    state: String,
    /// Physical work is observed independently of the goal's terminal proof.
    work_state: Value,
    /// The sealed step summary from `fortress.plan` (kind, obligation, ...).
    sealed: Value,
    /// The plan forecast for this step: predicted terminal state and tick.
    predicted: Option<(String, Option<u64>)>,
    /// Resolution of that forecast, in game ticks.
    forecast_resolution: u64,
}

const MAX_SURPRISES: usize = 32;

#[derive(Clone, Debug)]
struct SessionOrientation {
    anchor: Option<Value>,
    budget: Value,
    /// Latest accepted output negotiation, ordered by semantic request ID.
    /// Presentation turn IDs cannot order concurrent owner publications.
    output_budget_request_id: u128,
    grants: Vec<String>,
    paused: Option<bool>,
    adapter: Option<String>,
    compatibility: Option<String>,
    pending_plan_digest: Option<String>,
    /// An exact fresh-Plan affordance, retained only for its unchanged digest.
    pending_resource_review: Option<(String, Value)>,
    /// Capabilities the pending plan needs to be committed.
    pending_plan_capabilities: Vec<String>,
    /// Sealed per-step summaries of the pending plan (obligations, entities).
    pending_plan_steps: Vec<Value>,
    /// Forecast returned with the pending plan.
    pending_forecast: Value,
    /// Surprise records: material divergence between prediction and
    /// observation (bounded; newest last).
    surprises: Vec<Value>,
    /// Surprises recorded by the current call, surfaced as attention.
    fresh_surprises: Vec<Value>,
    /// Game ticks the most recent wait let pass (explains coarse lateness).
    last_wait_advance: u64,
    /// Every action of the most recently committed plan.
    plan_actions: Vec<PlanActionView>,
    last_action_id: Option<String>,
    last_action_state: Option<String>,
    /// Snapshot work whose originating action handle is no longer retained.
    untracked_work: Value,
    last_checkpoint_id: Option<String>,
    turn_sequence: u64,
}

impl SessionOrientation {
    fn from_payload(payload: &Value) -> Self {
        Self {
            anchor: extract_anchor(payload, None),
            budget: match payload.get("budget") {
                Some(value) => value.clone(),
                None => Value::Null,
            },
            output_budget_request_id: 0,
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
            pending_resource_review: None,
            pending_plan_capabilities: Vec::new(),
            pending_plan_steps: Vec::new(),
            pending_forecast: Value::Null,
            surprises: Vec::new(),
            fresh_surprises: Vec::new(),
            last_wait_advance: 0,
            plan_actions: Vec::new(),
            last_action_id: None,
            last_action_state: None,
            untracked_work: value_or_null(payload.get("untracked_work")),
            last_checkpoint_id: None,
            turn_sequence: 0,
        }
    }

    fn resource_review_step(&self) -> Option<&Value> {
        self.pending_resource_review
            .as_ref()
            .filter(|(digest, _)| self.pending_plan_digest.as_ref() == Some(digest))
            .map(|(_, next_step)| next_step)
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

/// Close owns the server registration lock before removing this cache entry.
/// Delayed successful opens use the same order and cannot resurrect it.
pub(crate) fn release_session_orientation(session_id: &str) {
    orientation_sessions().remove(session_id);
}

#[cfg(test)]
pub(crate) fn has_session_orientation(session_id: &str) -> bool {
    orientation_sessions().contains_key(session_id)
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

fn value_or_null(value: Option<&Value>) -> Value {
    match value {
        Some(value) => value.clone(),
        None => Value::Null,
    }
}

fn text_or_unknown(value: Option<&Value>) -> String {
    match value.and_then(Value::as_str) {
        Some(value) => value.to_owned(),
        None => "unknown".to_owned(),
    }
}

fn normalize_anchor(mut anchor: Value, payload: &Value, prior: Option<&Value>) -> Value {
    let game_tick = payload
        .get("game_tick")
        .cloned()
        .or_else(|| prior.and_then(|value| value.get("game_tick").cloned()))
        .or_else(|| {
            payload
                .get("fortress_loaded")
                .and_then(Value::as_bool)
                .filter(|loaded| *loaded)
                .map(|_| json!(1))
        });
    if let Some(object) = anchor.as_object_mut() {
        object
            .entry("game_tick".to_owned())
            .or_insert_with(|| game_tick.unwrap_or(Value::Null));
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
        if let Some(value) = payload.get(field)
            && !value.is_null()
        {
            return Some(normalize_anchor(value.clone(), payload, prior));
        }
    }

    let fortress_id = payload.get("fortress_id")?.clone();
    let cursor = payload.get("cursor")?.clone();
    let state_hash = payload.get("state_hash")?.clone();
    Some(json!({
        "fortress_id": fortress_id,
        "epoch": value_or_null(cursor.get("epoch")),
        "sequence": value_or_null(cursor.get("sequence")),
        "game_tick": value_or_null(payload.get("game_tick")),
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

/// Same-response proof references cannot survive in a later turn's cache.
/// Expand complete shared reviews before retaining any commit or work hints.
fn cache_pending_plan(payload: &Value, state: &mut SessionOrientation) {
    state.pending_plan_digest = None;
    state.pending_resource_review = None;
    state.pending_plan_capabilities.clear();
    state.pending_plan_steps.clear();
    state.pending_forecast = Value::Null;

    let Some(digest) = payload.get("plan_digest").and_then(Value::as_str) else {
        return;
    };
    let shared_schema = matches!(
        payload["schema"].as_str(),
        Some(
            "dfmcp.resource-production/2" | "dfmcp.resource-room/2" | "dfmcp.production-reserve/1"
        )
    );
    let steps = if shared_schema
        || payload.get("predicate_encoding").is_some()
        || payload.get("predicates").is_some()
    {
        let version = if payload["schema"] == "dfmcp.production-reserve/1" {
            1
        } else {
            2
        };
        if !shared_schema || payload["version"] != version {
            return;
        }
        let Ok(steps) = crate::sealed_plan_review::expanded_steps(payload) else {
            // Publication has already succeeded. Discard unsafe presentation
            // hints without refitting, refusing, or changing published bytes.
            return;
        };
        steps
    } else {
        payload
            .get("steps")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    state.pending_plan_digest = Some(digest.to_owned());
    state.pending_plan_capabilities = string_array(payload.get("required_capabilities"));
    if state.pending_plan_capabilities.is_empty() {
        // Legacy pause plans do not list their capability set.
        state.pending_plan_capabilities = vec!["control_clock".to_owned()];
    }
    state.pending_plan_steps = steps;
    state.pending_forecast = value_or_null(payload.get("forecast"));
}

fn update_orientation(operation: &str, payload: &Value, state: &mut SessionOrientation) {
    if let Some(anchor) = extract_anchor(payload, state.anchor.as_ref()) {
        state.anchor = Some(anchor);
    }
    if let Some(paused) = payload.get("paused").and_then(Value::as_bool) {
        state.paused = Some(paused);
    }
    if let Some(work) = payload.get("untracked_work") {
        state.untracked_work = work.clone();
    }
    if let Some(adapter) = payload.get("adapter").and_then(Value::as_str) {
        state.adapter = Some(adapter.to_owned());
    }
    if let Some(compatibility) = payload.get("compatibility").and_then(Value::as_str) {
        state.compatibility = Some(compatibility.to_owned());
    }

    match operation {
        "fortress.plan" => {
            cache_pending_plan(payload, state);
        }
        "fortress.commit" => {
            state.pending_plan_digest = None;
            state.pending_resource_review = None;
            let sealed_steps = std::mem::take(&mut state.pending_plan_steps);
            let forecast = std::mem::replace(&mut state.pending_forecast, Value::Null);
            state.pending_plan_capabilities.clear();
            let commit_tick = payload
                .get("observed_anchor")
                .and_then(|anchor| anchor.get("game_tick"))
                .and_then(Value::as_u64);
            if let Some(actions) = payload.get("actions").and_then(Value::as_array) {
                // Earlier plans' unfinished actions remain active work.
                state.plan_actions.retain(|view| {
                    action_is_nonterminal(Some(&view.state))
                        || view.work_state.get("quiescent").and_then(Value::as_bool) != Some(true)
                });
                let committed: Vec<PlanActionView> = actions
                    .iter()
                    .enumerate()
                    .map(|(index, action)| {
                        let step = action
                            .get("step")
                            .and_then(Value::as_u64)
                            .unwrap_or(index as u64);
                        PlanActionView {
                            action_id: text_or_unknown(action.get("action_id")),
                            step,
                            state: text_or_unknown(action.get("state")),
                            work_state: value_or_null(action.get("work_state")),
                            sealed: sealed_steps
                                .iter()
                                .find(|sealed| {
                                    sealed.get("step").and_then(Value::as_u64) == Some(step)
                                })
                                .cloned()
                                .unwrap_or(Value::Null),
                            predicted: forecast_for_step(&forecast, step),
                            forecast_resolution: forecast
                                .get("resolution_ticks")
                                .and_then(Value::as_u64)
                                .unwrap_or(0),
                        }
                    })
                    .collect();
                for view in &committed {
                    if !action_is_nonterminal(Some(&view.state)) {
                        compare_with_forecast(state, view, &view.state.clone(), commit_tick);
                    }
                }
                for view in committed {
                    if !state
                        .plan_actions
                        .iter()
                        .any(|existing| existing.action_id == view.action_id)
                    {
                        state.plan_actions.push(view);
                    }
                }
                if let Some(action) = actions.first() {
                    state.last_action_id = action
                        .get("action_id")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    state.last_action_state = Some(text_or_unknown(action.get("state")));
                }
            }
        }
        "fortress.wait" => {
            if let Some(action_id) = payload.get("action_id").and_then(Value::as_str) {
                state.last_action_id = Some(action_id.to_owned());
            }
            state.last_wait_advance = payload
                .get("advanced_game_ticks")
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let observed_tick = payload.get("game_tick").and_then(Value::as_u64);
            if let Some(polled) = payload.get("polled_actions").and_then(Value::as_array) {
                for observed in polled {
                    let id = observed.get("action_id").and_then(Value::as_str);
                    let new_state = text_or_unknown(observed.get("state"));
                    let Some(index) = state
                        .plan_actions
                        .iter()
                        .position(|view| Some(view.action_id.as_str()) == id)
                    else {
                        continue;
                    };
                    let was_open = action_is_nonterminal(Some(&state.plan_actions[index].state));
                    state.plan_actions[index].state = new_state.clone();
                    state.plan_actions[index].work_state =
                        value_or_null(observed.get("work_state"));
                    if was_open && !action_is_nonterminal(Some(&new_state)) {
                        let view = state.plan_actions[index].clone();
                        let receipt_tick = observed
                            .get("observed_anchor")
                            .and_then(|anchor| anchor.get("game_tick"))
                            .and_then(Value::as_u64)
                            .or(observed_tick);
                        compare_with_forecast(state, &view, &new_state, receipt_tick);
                    }
                }
            }
            state.last_action_state = payload
                .get("commit_state")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| state.last_action_state.clone());
        }
        "fortress.cancel" => {
            if let Some(action_id) = payload.get("action_id").and_then(Value::as_str) {
                state.last_action_id = Some(action_id.to_owned());
            }
            if let Some(steps) = payload.get("steps").and_then(Value::as_array) {
                for step in steps {
                    let id = step.get("action_id").and_then(Value::as_str);
                    if let Some(view) = state
                        .plan_actions
                        .iter_mut()
                        .find(|view| Some(view.action_id.as_str()) == id)
                    {
                        view.state = text_or_unknown(step.get("after"));
                        view.work_state = value_or_null(step.get("work_after"));
                    }
                }
            }
            state.last_action_state = payload
                .get("final_state")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| state.last_action_state.clone());
            if let Some(id) = payload.get("action_id").and_then(Value::as_str)
                && let Some(view) = state
                    .plan_actions
                    .iter_mut()
                    .find(|view| view.action_id == id)
            {
                view.state = text_or_unknown(payload.get("final_state"));
                view.work_state = value_or_null(payload.get("work_state"));
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
            state.pending_resource_review = None;
            state.pending_plan_capabilities.clear();
            state.pending_plan_steps.clear();
            state.plan_actions.clear();
            state.last_action_id = None;
            state.last_action_state = None;
        }
        _ => {}
    }
}

fn has_grant(state: &SessionOrientation, grant: &str) -> bool {
    state.grants.iter().any(|candidate| candidate == grant)
}

fn action_is_nonterminal(state: Option<&str>) -> bool {
    match state {
        Some(value) => {
            let normalized = value.to_ascii_lowercase();
            !matches!(
                normalized.as_str(),
                "verified" | "compensated" | "cancelled" | "failed"
            )
        }
        None => false,
    }
}

fn forecast_for_step(forecast: &Value, step: u64) -> Option<(String, Option<u64>)> {
    if forecast.get("available").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    forecast
        .get("steps")?
        .as_array()?
        .iter()
        .find(|entry| entry.get("step").and_then(Value::as_u64) == Some(step))
        .map(|entry| {
            (
                text_or_unknown(entry.get("predicted_state")),
                entry.get("predicted_terminal_tick").and_then(Value::as_u64),
            )
        })
}

fn record_surprise(state: &mut SessionOrientation, record: Value) {
    if state.surprises.len() >= MAX_SURPRISES {
        state.surprises.remove(0);
    }
    state.surprises.push(record.clone());
    state.fresh_surprises.push(record);
}

/// Compare an action's observed terminal outcome with its forecast. A
/// different terminal state, or completion later than the forecast by more
/// than its resolution plus the last wait's step (which coarse waiting alone
/// explains), is a material divergence and becomes a surprise record.
fn compare_with_forecast(
    state: &mut SessionOrientation,
    view: &PlanActionView,
    observed_state: &str,
    observed_tick: Option<u64>,
) {
    let Some((predicted_state, predicted_tick)) = view.predicted.clone() else {
        return;
    };
    let kind = if !predicted_state.eq_ignore_ascii_case(observed_state) {
        Some((
            "outcome_diverged",
            if observed_state.eq_ignore_ascii_case("verified") {
                "the step succeeded although the forecast predicted otherwise"
            } else {
                "an expected effect was not proven: the forecast predicted a different terminal state"
            },
        ))
    } else {
        match (predicted_tick, observed_tick) {
            (Some(predicted), Some(observed))
                if observed
                    > predicted
                        .saturating_add(view.forecast_resolution)
                        .saturating_add(state.last_wait_advance) =>
            {
                Some((
                    "completion_late",
                    "completion took materially longer than forecast, beyond what wait cadence explains",
                ))
            }
            _ => None,
        }
    };
    if let Some((kind, explanation)) = kind {
        record_surprise(
            state,
            json!({
                "surprise_id": format!("surprise-{}-{}", view.action_id, kind),
                "kind": kind,
                "action_id": view.action_id,
                "step": view.step,
                "predicted": {"state": predicted_state, "terminal_tick": predicted_tick},
                "observed": {"state": observed_state, "tick": observed_tick},
                "epistemic_state": "observed",
                "explanation": explanation,
                "lesson_candidate": "re-forecast after observing; check for other agents or changed preconditions",
            }),
        );
    }
}

fn plan_work_pending(state: &SessionOrientation) -> bool {
    state
        .plan_actions
        .iter()
        .any(|view| action_is_nonterminal(Some(&view.state)))
}

fn terminal_physical_work(state: &SessionOrientation) -> Vec<&PlanActionView> {
    state
        .plan_actions
        .iter()
        .filter(|view| {
            !action_is_nonterminal(Some(&view.state))
                && view.work_state.get("quiescent").and_then(Value::as_bool) == Some(false)
        })
        .collect()
}

fn failed_plan_actions(state: &SessionOrientation) -> Vec<&PlanActionView> {
    state
        .plan_actions
        .iter()
        .filter(|view| view.state.eq_ignore_ascii_case("failed"))
        .collect()
}

/// A bounded game-time step for the next wait: the smallest pending poll
/// interval times ten, within the negotiated game-tick budget.
fn wait_ticks(state: &SessionOrientation) -> u64 {
    let cadence = state
        .plan_actions
        .iter()
        .filter(|view| action_is_nonterminal(Some(&view.state)))
        .filter_map(|view| {
            view.sealed
                .get("obligation")
                .and_then(|o| o.get("poll_interval_ticks"))
                .and_then(Value::as_u64)
        })
        .min()
        .unwrap_or(10)
        .saturating_mul(10);
    let budget = state
        .budget
        .get("max_game_ticks")
        .and_then(Value::as_u64)
        .unwrap_or(cadence);
    cadence.clamp(1, budget.max(1))
}

fn active_work(state: &SessionOrientation) -> Value {
    let review = state.resource_review_step();
    let pending_plans = match state.pending_plan_digest.as_ref() {
        Some(digest) => vec![json!({
            "plan_digest": digest,
            "state": if review.is_some() { "review_required" } else { "prepared" },
            "minimum_safe_next_step": review.cloned().unwrap_or_else(|| json!({
                "tool": "fortress.commit",
                "arguments": {"plan_digest": digest},
            })),
        })],
        None => Vec::new(),
    };
    let actions: Vec<Value> = if state.plan_actions.is_empty() {
        match state.last_action_id.as_ref() {
            Some(action_id) => vec![json!({
                "action_id": action_id,
                "state": state.last_action_state.as_deref().unwrap_or("unknown"),
            })],
            None => Vec::new(),
        }
    } else {
        state
            .plan_actions
            .iter()
            .map(|view| {
                json!({
                    "action_id": view.action_id,
                    "step": view.step,
                    "kind": value_or_null(view.sealed.get("kind")),
                    "state": view.state,
                    "work_state": view.work_state,
                    "creates_entity_id": value_or_null(view.sealed.get("creates_entity_id")),
                })
            })
            .collect()
    };
    // Every nonterminal action with a sealed obligation is an open obligation:
    // its terminal predicate must be observed before its deadline.
    let obligations: Vec<Value> = state
        .plan_actions
        .iter()
        .filter(|view| action_is_nonterminal(Some(&view.state)))
        .filter_map(|view| {
            let obligation = view.sealed.get("obligation").filter(|o| !o.is_null())?;
            Some(json!({
                "action_id": view.action_id,
                "step": view.step,
                "state": view.state,
                "terminal": value_or_null(obligation.get("terminal")),
                "deadline_tick": value_or_null(obligation.get("deadline_tick")),
                "blocked_by_pause": state.paused == Some(true),
                "next_step": {"tool": "fortress.wait", "arguments": {"max_game_ticks": wait_ticks(state)}},
            }))
        })
        .collect();
    json!({
        "pending_plans": pending_plans,
        "actions": actions,
        "obligations": obligations,
        "untracked_work": state.untracked_work,
        "cancellation_drains": [],
        "indeterminate_effects": [],
        "publications": [],
        "confirmations": [],
    })
}

#[allow(clippy::too_many_arguments)]
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
    let observe_granted = has_grant(state, "observe");
    let query_granted = has_grant(state, "query");
    let checkpoint_granted = has_grant(state, "checkpoint");
    let control_granted = has_grant(state, "control_clock");
    let mut result = vec![
        affordance(
            "observe-pulse",
            "fortress.observe",
            "observe",
            "read_only",
            false,
            observe_granted,
            (!observe_granted).then_some("observe capability is not granted"),
            json!({"profile": "pulse"}),
        ),
        affordance(
            "query-summary",
            "fortress.query",
            "query",
            "read_only",
            false,
            query_granted,
            (!query_granted).then_some("query capability is not granted"),
            json!({"mode": "summary"}),
        ),
    ];

    if let Some(digest) = state.pending_plan_digest.as_ref() {
        let review_required = state.resource_review_step().is_some();
        let commit_granted = !review_required
            && state
                .pending_plan_capabilities
                .iter()
                .all(|capability| has_grant(state, capability));
        result.push(affordance(
            "commit-pending-plan",
            "fortress.commit",
            "commit_prepared_plan",
            "reversible",
            true,
            commit_granted,
            if review_required {
                Some("the original resource objective requires a newly reviewed allocation")
            } else {
                (!commit_granted)
                    .then_some("the session lacks a capability the sealed plan requires")
            },
            json!({"plan_digest": digest}),
        ));
    } else if let Some(paused) = state.paused {
        let enabled = has_grant(state, "plan") && control_granted;
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
        result.extend([
            affordance(
                "inspect-last-action",
                "fortress.wait",
                "verify_active_work",
                "read_only",
                false,
                true,
                None,
                json!({"action_id": action_id}),
            ),
            affordance(
                "cancel-last-action",
                "fortress.cancel",
                "cancel_active_work",
                "reversible",
                true,
                control_granted,
                (!control_granted).then_some("control_clock capability is not granted"),
                json!({"mode": "compensate_reversible"}),
            ),
        ]);
    }

    result.push(affordance(
        "checkpoint-current-state",
        "fortress.checkpoint",
        "checkpoint",
        "guarded",
        false,
        checkpoint_granted,
        (!checkpoint_granted).then_some("checkpoint capability is not granted"),
        json!({"label": "agent-checkpoint"}),
    ));
    result
}

fn error_code(payload: &Value) -> &str {
    match payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
    {
        Some(code) => code,
        None => "unknown",
    }
}

/// This server-owned refusal retains the old proposal and the exact original
/// source. Observing again cannot make its stale allocation commit-ready.
fn resource_replan_step(payload: &Value) -> Option<&Value> {
    let next = payload.get("next_step")?;
    (!is_ok(payload)
        && error_code(payload) == "stale_anchor"
        && matches!(
            payload["rebase"]["method"].as_str(),
            Some(
                "fresh_resource_acquisition_required"
                    | "fresh_resource_production_required"
                    | "fresh_resource_room_required"
                    | "fresh_production_reserve_required"
            )
        )
        && payload["rebase"]["pending_plan_preserved"] == true
        && payload["rebase"]["from_digest"].is_string()
        && next["tool"] == "fortress.plan"
        && next["arguments"]["blueprint"].is_string())
    .then_some(next)
}

/// A fortress need with a concrete remedy outranks routine next steps when
/// the session holds the authority to plan it; otherwise it stays attention.
fn recommendations(
    operation: &str,
    ok: bool,
    payload: &Value,
    state: &SessionOrientation,
) -> Vec<Value> {
    let mut out = Vec::new();
    if ok
        && state.pending_plan_digest.is_none()
        && let Some(alerts) = payload.get("world_alerts").and_then(Value::as_array)
    {
        for alert in alerts {
            if let Some(remedy) = alert
                .get("remedy")
                .filter(|remedy| remedy["available"] == true)
            {
                let Some(risk) = remedy["risk"].as_str() else {
                    continue;
                };
                let reversibility = match remedy["reversibility"].as_str() {
                    Some(value) => value,
                    None => "unknown",
                };
                let mut next = recommendation(
                    format!("remedy-{}", alert["alert"].as_str().unwrap_or("need")),
                    "fortress.plan",
                    format!(
                        "{}; plan the remedy",
                        alert["finding"]
                            .as_str()
                            .unwrap_or("a fortress need is unmet")
                    ),
                    "high",
                    "low",
                    risk,
                    reversibility,
                    remedy["requires_confirmation"] == true,
                    remedy["arguments"].clone(),
                );
                next["requires_checkpoint"] = remedy["requires_checkpoint"].clone();
                out.push(next);
            }
        }
    }
    out.extend(routine_recommendations(operation, ok, payload, state));
    out
}

fn routine_recommendations(
    operation: &str,
    ok: bool,
    payload: &Value,
    state: &SessionOrientation,
) -> Vec<Value> {
    if ok && let Some(steps) = selected_wait_recommendations(payload, state) {
        return steps;
    }
    if let Some(next) =
        resource_replan_step(payload).or_else(|| ok.then(|| state.resource_review_step()).flatten())
    {
        return vec![recommendation(
            "review-resource-acquisition",
            "fortress.plan",
            "the original supply-dependent objective needs a fresh plan at the current anchor",
            "preserve the complete original objective and acquisition policy",
            "inspect current finite supply, demand and workforce before committing",
            "read_only",
            "not_applicable",
            false,
            next["arguments"].clone(),
        )];
    }
    if !ok {
        if let Some(consent) = payload.get("clock_consent") {
            return vec![recommendation(
                "await-unpause-consent",
                "fortress.observe",
                "your unpause consent is recorded; the shared fortress resumes only when every member consents, and your plan stays pending",
                "medium",
                "medium",
                "read_only",
                "not_applicable",
                false,
                json!({"clock_consent": consent}),
            )];
        }
        if let Some(digest) = payload
            .get("rebased_plan")
            .and_then(|plan| plan.get("plan_digest"))
            .and_then(Value::as_str)
        {
            return vec![recommendation(
                "commit-rebased-plan",
                "fortress.commit",
                "the anchor moved before commit; the request was replayed at the current anchor and re-sealed",
                "high",
                "medium",
                "reversible",
                "reversible",
                true,
                json!({"plan_digest": digest}),
            )];
        }
        return match error_code(payload) {
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
                "the plan is absent or conflicts with current protocol state",
                "high",
                "medium",
                "reversible",
                "reversible",
                false,
                json!({}),
            )],
            "indeterminate_effect" | "indeterminate" => vec![recommendation(
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
    if !terminal_physical_work(state).is_empty() {
        return vec![recommendation(
            "drain-remaining-physical-work",
            "fortress.cancel",
            "a terminal goal still has active or unresolved physical work; stop it under current authority",
            "high",
            "high",
            "reversible",
            "not_applicable",
            false,
            json!({"scope": "session", "mode": "stop_future_steps"}),
        )];
    }
    if plan_work_pending(state) {
        if state.paused == Some(true) {
            return vec![recommendation(
                "resume-to-progress-work",
                "fortress.plan",
                "committed work cannot progress while the fortress is paused",
                "high",
                "high",
                "reversible",
                "reversible",
                false,
                json!({"paused_target": false, "summary": "resume so committed work can progress"}),
            )];
        }
        return vec![recommendation(
            "advance-and-verify-work",
            "fortress.wait",
            "committed temporal work needs game time and a later observation to be proven",
            "high",
            "high",
            "read_only",
            "not_applicable",
            false,
            json!({"max_game_ticks": wait_ticks(state)}),
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
            "establish a bounded semantic baseline at the current observation epoch",
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

/// The most recently committed action can belong to a different project.
/// Guidance for a targeted wait must follow the selected original outcome.
fn selected_wait_recommendations(
    payload: &Value,
    state: &SessionOrientation,
) -> Option<Vec<Value>> {
    let selected = payload.get("selected_plan_wait")?.as_object()?;
    let digest = selected.get("requested_plan_digest")?.as_str()?;
    let reason = selected.get("stop_reason")?.as_str()?;
    if reason == "complete" {
        return Some(Vec::new());
    }
    let (id, tool, why, arguments) = match reason {
        "needs_continuation"
            if selected
                .get("continuation_request")
                .and_then(|next| next.get("arguments"))
                .is_some_and(Value::is_object) =>
        {
            let arguments = selected["continuation_request"]["arguments"].clone();
            (
                "review-selected-goal-continuation",
                "fortress.plan",
                "the selected stage is quiet but the original project needs a separately reviewed continuation",
                arguments,
            )
        }
        "tick_limit" if state.paused == Some(false) && has_grant(state, "control_clock") => (
            "continue-selected-plan-wait",
            "fortress.wait",
            "the selected original plan remains unfinished at this call's bounded time limit",
            json!({"plan_digest":digest, "max_game_ticks":wait_ticks(state)}),
        ),
        "paused" if has_grant(state, "plan") && has_grant(state, "control_clock") => (
            "review-resume-for-selected-plan",
            "fortress.plan",
            "the selected original work cannot progress while the shared fortress is paused",
            json!({"paused_target":false, "summary":"resume to progress the retained original plan"}),
        ),
        _ => (
            "inspect-selected-plan-outcome",
            "fortress.explain",
            "inspect the selected original goal, action proof and physical work before deciding how to recover",
            json!({"entity_id":format!("plan:{digest}")}),
        ),
    };
    Some(vec![recommendation(
        id,
        tool,
        why,
        "high",
        "high",
        "read_only",
        "not_applicable",
        false,
        arguments,
    )])
}

/// Observed world changes between the anchor this agent saw last and the
/// current one, from the session's retained canonical versions.
fn world_changes(
    session_id: Option<&String>,
    previous: Option<&Value>,
    current: Option<&Value>,
) -> Vec<Value> {
    let (Some(session_id), Some(previous), Some(current)) = (session_id, previous, current) else {
        return Vec::new();
    };
    let Some(from) = previous.get("state_hash").and_then(Value::as_str) else {
        return Vec::new();
    };
    if current.get("state_hash").and_then(Value::as_str) == Some(from) {
        return Vec::new();
    }
    crate::server::world_changes_since(session_id, from).unwrap_or_default()
}

fn changes(operation: &str, ok: bool, payload: &Value) -> Vec<Value> {
    if !ok {
        return Vec::new();
    }
    match operation {
        "fortress.query" if payload["mode"] == "query_changes" => vec![json!({
            "kind": "query_endpoint_comparison",
            "basis": payload["basis"],
            "target": payload["anchor"],
            "change_count": payload["change_count"],
            "returned": payload["returned"],
            "epistemic_state": "inferred",
            "evidence": [payload["basis_result_digest"], payload["target_result_digest"]],
            "detail_path": "changes",
            "intermediate_history_proven": false,
        })],
        "fortress.plan" => match payload.get("plan_digest").and_then(Value::as_str) {
            Some(digest) => vec![json!({
                "kind": "plan_prepared",
                "subject": {"plan_digest": digest},
                "epistemic_state": "observed",
                "invalidates": [],
                "evidence": [],
            })],
            None => Vec::new(),
        },
        "fortress.commit" => vec![json!({
            "kind": "plan_commit_observed",
            "subject": {"plan_digest": value_or_null(payload.get("plan_digest"))},
            "epistemic_state": "observed",
            "invalidates": ["pending_plan_handle"],
            "evidence": [],
        })],
        "fortress.cancel" => vec![json!({
            "kind": "cancellation_finalized",
            "subject": {"action_id": value_or_null(payload.get("action_id"))},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
        })],
        "fortress.checkpoint" => vec![json!({
            "kind": "checkpoint_created",
            "subject": {"checkpoint_id": value_or_null(payload.get("checkpoint_id"))},
            "epistemic_state": "observed",
            "invalidates": [],
            "evidence": [],
        })],
        "fortress.restore" => vec![json!({
            "kind": "observation_epoch_reset",
            "subject": {"checkpoint_id": value_or_null(payload.get("checkpoint_id"))},
            "epistemic_state": "observed",
            "invalidates": [
                "all_pre_restore_plans",
                "all_pre_restore_action_handles",
                "all_pre_restore_continuations"
            ],
            "evidence": [],
        })],
        _ => Vec::new(),
    }
}

fn attention(operation: &str, ok: bool, payload: &Value, state: &SessionOrientation) -> Vec<Value> {
    let mut items: Vec<Value> = state
        .fresh_surprises
        .iter()
        .map(|surprise| {
            json!({
                "attention_id": value_or_null(surprise.get("surprise_id")),
                "category": "surprise",
                "severity": "high",
                "urgency": "before_the_next_plan",
                "confidence": {"epistemic_state": "observed", "value": 1.0},
                "finding": value_or_null(surprise.get("explanation")),
                "surprise": surprise,
                "likely_consequence_if_ignored": "the agent keeps planning from a model the world just contradicted",
                "evidence": [],
            })
        })
        .collect();
    if let Some(alerts) = payload.get("world_alerts").and_then(Value::as_array) {
        items.extend(alerts.iter().map(|alert| {
            let kind = alert["alert"].as_str().unwrap_or("alert");
            let subject = alert["subject"].as_str()
                .or_else(|| alert["recovery"]["burrow"]["id"].as_str());
            let attention_id = match subject {
                Some(subject) => format!("world-{kind}-{subject}"),
                None => format!("world-{kind}"),
            };
            let source_anchor = alert.get("source_anchor")
                .or_else(|| alert["recovery"].get("source_anchor"));
            let epistemic = if alert["epistemic_state"] == "unknown"
                || alert["recovery"]["threat"] == "unknown"
                || alert["recovery"].get("fault").is_some_and(|fault| !fault.is_null())
            {
                "unknown"
            } else { "certified_derived" };
            json!({
                "attention_id": attention_id,
                "category": "fortress_needs",
                "severity": value_or_null(alert.get("severity")),
                "urgency": if alert["severity"] == "critical" { "now" } else { "before_the_next_plan" },
                "confidence": {
                    "epistemic_state": epistemic,
                    "value": null,
                },
                "finding": value_or_null(alert.get("finding")),
                "remedy": value_or_null(alert.get("remedy")),
                "likely_consequence_if_ignored": "dwarves go without and the fortress declines",
                "evidence": source_anchor.map(|anchor| vec![json!({
                    "source_anchor":anchor,
                    "scope":"laboratory_reference_world",
                    "source_scope":value_or_null(alert["recovery"].get("source_scope")),
                })]).unwrap_or_default(),
            })
        }));
    }
    items.extend(base_attention(operation, ok, payload, state));
    if state
        .untracked_work
        .get("quiescent")
        .and_then(Value::as_bool)
        == Some(false)
    {
        items.push(json!({
            "attention_id": "untracked-reference-work",
            "category": "active_work", "severity": "high", "urgency": "before_new_effects",
            "confidence": {"epistemic_state": "unknown", "value": null},
            "finding": "reference work has no retained action handle, or its complete coverage is unavailable",
            "work": state.untracked_work,
            "likely_consequence_if_ignored": "restored or unreconciled work can keep running without a monitor or a safe cancellation identity",
            "evidence": [],
        }));
    }
    items
}

/// Most attention items one turn carries; the rest are named in the
/// selection certificate under `coverage.attention_selection`.
const MAX_ATTENTION_ITEMS: usize = 6;

fn attention_key(item: &Value) -> dfmcp_world::AttentionKey {
    let rank = |value: &Value, scale: &[&str]| {
        value
            .as_str()
            .and_then(|text| scale.iter().position(|level| *level == text))
            .map_or(0, |position| (scale.len() - position) as u64)
    };
    dfmcp_world::AttentionKey {
        class: rank(&item["severity"], &["critical", "high", "medium", "low"]) as u8,
        magnitude: rank(
            &item["urgency"],
            &[
                "now",
                "before_the_next_plan",
                "before_formulating_an_unrelated_plan",
            ],
        ),
        freshness: rank(
            &item["category"],
            &[
                "control_plane",
                "continuity",
                "surprise",
                "active_work",
                "fortress_needs",
            ],
        ),
        identity: item["attention_id"].as_str().unwrap_or_default().to_owned(),
    }
}

/// Exact, certified top-k ranking: severity, then urgency, then category,
/// ties by attention id. Attention is advisory and never authorizes.
fn ranked_attention(items: Vec<Value>) -> (Vec<Value>, Value) {
    let keys: Vec<_> = items.iter().map(attention_key).collect();
    match dfmcp_world::select_top_k(&keys, MAX_ATTENTION_ITEMS) {
        Ok((picked, certificate)) => {
            let selection = json!({
                "policy": "severity > urgency > category, ties by attention_id ascending",
                "k": certificate.k,
                "considered": certificate.considered,
                "selected": certificate.selected,
                "excluded": certificate.excluded,
                "certificate_digest": certificate.digest.to_hex(),
                "certified": certificate.verify(&keys),
            });
            let ranked = picked
                .into_iter()
                .filter_map(|i| items.get(i).cloned())
                .collect();
            (ranked, selection)
        }
        Err(error) => (items, json!({"certified": false, "error": error.message})),
    }
}

fn base_attention(
    operation: &str,
    ok: bool,
    payload: &Value,
    state: &SessionOrientation,
) -> Vec<Value> {
    if !ok {
        return vec![json!({
            "attention_id": "protocol-error",
            "category": "control_plane",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": value_or_null(payload.get("error").and_then(|value| value.get("message"))),
            "likely_consequence_if_ignored": "the agent may operate from an invalid protocol state",
            "evidence": [],
        })];
    }
    let mut items = Vec::new();
    let failed = failed_plan_actions(state);
    if !failed.is_empty() {
        items.push(json!({
            "attention_id": "obligation-failed",
            "category": "active_work",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": format!(
                "{} committed action(s) failed their obligation (deadline or failure predicate)",
                failed.len()
            ),
            "subjects": failed.iter().map(|view| json!({"action_id": view.action_id, "step": view.step})).collect::<Vec<_>>(),
            "likely_consequence_if_ignored": "dependent work will never dispatch and the goal will not be reached",
            "evidence": [],
        }));
    }
    if state.pending_plan_digest.is_some() {
        items.push(json!({
            "attention_id": "prepared-plan-awaiting-decision",
            "category": "active_work",
            "severity": "medium",
            "urgency": "before_formulating_an_unrelated_plan",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "a sealed plan is awaiting commit, replacement, or explicit abandonment",
            "likely_consequence_if_ignored": "the agent may lose track of unfinished protocol state",
            "evidence": [],
        }));
    }
    let physical = terminal_physical_work(state);
    if !physical.is_empty() {
        items.push(json!({
            "attention_id": "terminal-goal-work-remains",
            "category": "active_work",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "a terminal goal outcome has not established physical quiescence",
            "subjects": physical.iter().map(|view| json!({
                "action_id": view.action_id, "step": view.step, "work_state": view.work_state,
            })).collect::<Vec<_>>(),
            "likely_consequence_if_ignored": "production, digging or construction may continue after the goal has finished",
            "evidence": [],
        }));
    }
    if action_is_nonterminal(state.last_action_state.as_deref()) {
        items.push(json!({
            "attention_id": "action-awaiting-verification",
            "category": "active_work",
            "severity": "high",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "an action is not yet in an ordinary terminal state",
            "likely_consequence_if_ignored": "goal completion or failure may be misclassified",
            "evidence": [],
        }));
    }
    if operation == "fortress.restore" {
        items.push(json!({
            "attention_id": "restore-invalidated-prior-context",
            "category": "continuity",
            "severity": "critical",
            "urgency": "now",
            "confidence": {"epistemic_state": "observed", "value": 1.0},
            "finding": "the observation epoch changed and all pre-restore handles are stale",
            "likely_consequence_if_ignored": "the agent may act on invalid plans or continuations",
            "evidence": [],
        }));
    }
    items
}

fn briefing(state: &SessionOrientation, payload: &Value) -> Value {
    let mut view = json!({
        "implementation_phase": LAB_IMPLEMENTATION_PHASE,
        "adapter": state.adapter,
        "compatibility": state.compatibility,
        "fortress_loaded": true,
        "paused": state.paused,
        "mission": null,
        "objective_status": payload.get("objectives").cloned().unwrap_or_else(|| json!([])),
        "mutation_admissible": has_grant(state, "plan") && has_grant(state, "control_clock"),
        "last_checkpoint_id": state.last_checkpoint_id,
        "highest_unresolved_uncertainty": "no live DFHack observation or mutation path is implemented",
    });
    // Use only the current authorized response. A prior scheduling decision
    // cannot be carried into a newer anchor as if it were observed there.
    if let Some(workforce) = payload.get("workforce").or_else(|| {
        payload
            .get("world")
            .and_then(|world| world.get("workforce"))
    }) {
        view["workforce"] = workforce.clone();
    }
    if let Some(materials) = payload.get("construction_materials").or_else(|| {
        payload
            .get("world")
            .and_then(|world| world.get("construction_materials"))
    }) {
        view["construction_materials"] = materials.clone();
    }
    if let Some(provisions) = payload.get("provisions").or_else(|| {
        payload
            .get("world")
            .and_then(|world| world.get("provisions"))
    }) {
        view["provisions"] = provisions.clone();
    }
    view
}

fn coverage(payload: &Value) -> Value {
    let fallback = extract_anchor(payload, None);
    // Historical rows name their own anchor even when the orientation spine
    // also names the current head. Coverage must follow the returned rows.
    crate::observation_projection::coverage(payload, payload.get("anchor").or(fallback.as_ref()))
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
    let mut result = vec![
        uncertainty(
            "live-adapter-not-implemented",
            "unknown",
            "the process-local laboratory is not connected to Dwarf Fortress or DFHack",
            "no response in this session establishes live fortress state or live effects",
            Some("fortress.doctor"),
            json!({}),
        ),
        uncertainty(
            "semantic-request-id-not-projected",
            "unknown",
            "the authority-bearing laboratory request identity is not yet exposed by the handler result",
            "the packet uses a separate presentation turn_id and leaves request_id null",
            None,
            json!({}),
        ),
    ];
    if state.anchor.is_none() {
        result.push(uncertainty(
            "anchor-not-projected",
            "unknown",
            "the underlying result did not expose a complete canonical anchor",
            "the response must not be used as a mutation basis",
            Some("fortress.observe"),
            json!({"profile": "briefing"}),
        ));
    }
    result
}

fn recovery_class(payload: &Value) -> RecoveryClass {
    if resource_replan_step(payload).is_some() {
        return RecoveryClass::RebaseRequired;
    }
    match error_code(payload) {
        "cursor_gap" | "stale_anchor" => RecoveryClass::RefreshAndRetry,
        "invalid_plan" | "conflict" => RecoveryClass::RebaseRequired,
        "indeterminate_effect" | "indeterminate" => RecoveryClass::ReconciliationRequired,
        "budget_exceeded" | "session_not_found" | "invalid_request" => {
            RecoveryClass::NeverUnchanged
        }
        "capability_denied" => RecoveryClass::OperatorActionRequired,
        _ => RecoveryClass::OperatorActionRequired,
    }
}

fn attach_recovery(payload: &mut Value) {
    if is_ok(payload) {
        return;
    }
    let class = recovery_class(payload);
    let resource_arguments = resource_replan_step(payload).map(|next| next["arguments"].clone());
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
            recovery_guidance(
                class,
                tool,
                reason,
                resource_arguments.unwrap_or_else(|| json!({})),
            ),
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
        return if matches!(
            error_code(payload),
            "indeterminate_effect" | "indeterminate"
        ) {
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
    if payload["mode"] == "history" {
        // Metadata describes retained identities and page delivery; it is not
        // a newly acknowledged observation or complete world-history claim.
        return ContinuityStatus::Partial;
    }
    if payload["mode"] == "condition_replay" {
        // Even an interval ending at the current world is retrospective sampled
        // evidence; it never establishes a fresh live condition or continuity.
        return ContinuityStatus::Stale;
    }
    if payload["mode"] == "query_changes" {
        return if payload["historical"] == true {
            ContinuityStatus::Stale
        } else if payload["truncated"] == true || payload["anchor_advanced"] == true {
            ContinuityStatus::Partial
        } else {
            ContinuityStatus::Heartbeat
        };
    }
    if payload["mode"] == "structured" {
        if payload["historical"] == true {
            return ContinuityStatus::Stale;
        }
        if payload["truncated"] == true {
            return ContinuityStatus::Partial;
        }
    }
    match (previous_anchor, current_anchor) {
        (Some(previous), Some(current))
            if previous == current && operation == "fortress.observe" =>
        {
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

fn has_drain_observation(operation: &str, payload: &Value) -> bool {
    operation == "fortress.cancel"
        && payload.get("observed_anchor").is_some_and(Value::is_object)
        && payload.get("drain_progress").is_some_and(Value::is_object)
}

fn has_wait_observation(operation: &str, payload: &Value) -> bool {
    operation == "fortress.wait"
        && payload.get("observed_anchor").is_some_and(Value::is_object)
        && payload.get("wait_progress").is_some_and(Value::is_object)
}

fn presentation_state(
    operation: &str,
    payload: &mut Value,
    session_id: Option<&String>,
) -> (SessionOrientation, Option<Value>, u64) {
    let Some(id) = session_id else {
        let mut fallback = SessionOrientation::from_payload(payload);
        let sequence = fallback.advance_turn();
        return (fallback, None, sequence);
    };

    if operation == "fortress.open_session" && is_ok(payload) {
        return match crate::server::with_registered_session(id, || {
            presentation_state_registered(operation, payload, id)
        }) {
            Ok(state) => state,
            Err(error) => {
                *payload = json!({"ok":false,"error":{
                    "operation":operation,"code":error.code.as_str(),
                    "message":error.message,"retryable":false,"details":[]}});
                let mut fallback = SessionOrientation::from_payload(payload);
                let sequence = fallback.advance_turn();
                (fallback, None, sequence)
            }
        };
    }
    presentation_state_registered(operation, payload, id)
}

fn presentation_state_registered(
    operation: &str,
    payload: &mut Value,
    id: &String,
) -> (SessionOrientation, Option<Value>, u64) {
    let mut registry = orientation_sessions();
    if operation == "fortress.open_session" && is_ok(payload) {
        if registry.len() >= MAX_PRESENTATION_SESSIONS && !registry.contains_key(id) {
            *payload = json!({
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
            registry.insert(id.clone(), SessionOrientation::from_payload(payload));
        }
    }

    match registry.get_mut(id) {
        Some(existing) => {
            let prior = existing.anchor.clone();
            existing.fresh_surprises.clear();
            // A wait or drain can refuse after publishing earlier progress.
            // Keep its authorized current work and clock state visible even
            // though the requested operation remains incomplete.
            if is_ok(payload)
                || has_drain_observation(operation, payload)
                || has_wait_observation(operation, payload)
            {
                update_orientation(operation, payload, existing);
            } else if let Some(next) = resource_replan_step(payload) {
                if let Some(digest) = payload["rebase"]["from_digest"].as_str()
                    && existing.pending_plan_digest.as_deref() == Some(digest)
                {
                    existing.pending_resource_review = Some((digest.to_owned(), next.clone()));
                }
            } else if let Some(rebased) = payload.get("rebased_plan") {
                record_surprise(
                    existing,
                    json!({
                        "surprise_id": format!(
                            "surprise-{}-anchor_moved_before_commit",
                            value_or_null(payload.get("rebase").and_then(|r| r.get("from_digest")))
                        ),
                        "kind": "anchor_moved_before_commit",
                        "predicted": {"anchor": value_or_null(payload.get("rebase").and_then(|r| r.get("from_anchor")))},
                        "observed": {"anchor": value_or_null(payload.get("rebase").and_then(|r| r.get("to_anchor")))},
                        "epistemic_state": "observed",
                        "explanation": "the world changed between plan and commit; the request was replayed and re-sealed",
                        "lesson_candidate": "commit promptly after planning, or expect replay in shared fortresses",
                    }),
                );
                // A stale commit was replayed into a new pending plan.
                cache_pending_plan(rebased, existing);
            }
            let sequence = existing.advance_turn();
            (existing.clone(), prior, sequence)
        }
        None => {
            let mut fallback = SessionOrientation::from_payload(payload);
            let sequence = fallback.advance_turn();
            (fallback, None, sequence)
        }
    }
}

fn refresh_published_plan_orientation(raw: &str) {
    // The owner published this complete response atomically with the pending
    // replacement. Comparisons and refusals leave existing plan hints intact.
    if let Ok(mut payload) = serde_json::from_str::<Value>(raw)
        && is_ok(&payload)
        && matches!(
            payload["kind"].as_str(),
            Some(
                "select_candidate"
                    | "resource_acquisition"
                    | "resource_acquisition_continuation"
                    | "resource_production"
                    | "resource_production_continuation"
                    | "production_reserve_continuation"
                    | "resource_room"
                    | "resource_room_continuation"
            )
        )
    {
        let resolved = payload["session_id"].as_str().map(str::to_owned);
        let _ = presentation_state("fortress.plan", &mut payload, resolved.as_ref());
    }
}

fn refresh_published_output_budget_orientation(raw: &str) {
    let Ok(payload) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let Some(id) = payload["session_id"].as_str() else {
        return;
    };
    let Some(request_id) = payload["request_id"]
        .as_str()
        .and_then(|id| u128::from_str_radix(id, 16).ok())
    else {
        return;
    };
    if is_ok(&payload)
        && payload["kind"] == "output_budget_renegotiation"
        && let Some(state) = orientation_sessions().get_mut(id)
    {
        // A later owner transaction may have completed while an earlier reply
        // waited for this presentation lock. Old replies cannot restore its cap.
        if request_id > state.output_budget_request_id {
            state.budget = payload["budget"].clone();
            state.output_budget_request_id = request_id;
            // Negotiation changes no world facts. A later Observe/Wait may
            // already have advanced these hints while this reply was delayed.
        }
        state.advance_turn();
    }
}

fn project_response(
    raw: String,
    operation: &str,
    phase: AgentPhase,
    profile: ObservationProfile,
    session_hint: Option<&str>,
) -> String {
    let mut payload = response_payload(&raw, operation);
    let presentation = ResponsePresentation::capture(operation, &mut payload, session_hint);
    let output = presentation.render(payload, operation, phase, profile);
    crate::task_service::notify_progress();
    output
}

fn response_payload(raw: &str, operation: &str) -> Value {
    let mut payload = match serde_json::from_str::<Value>(raw) {
        Ok(value) if value.is_object() => value,
        Ok(_) | Err(_) => malformed_payload(operation),
    };
    attach_recovery(&mut payload);
    payload
}

/// Capture mutable orientation and server-backed dependencies once. Rendering
/// prospective query pages from this value is pure and cannot acquire a session
/// lock, advance a turn, consume surprises, or change active work.
struct ResponsePresentation {
    session_id: Option<String>,
    state: SessionOrientation,
    previous_anchor: Option<Value>,
    turn_sequence: u64,
    world_delta: Vec<Value>,
    task_work: Option<(Value, Value)>,
}

impl ResponsePresentation {
    fn capture(operation: &str, payload: &mut Value, session_hint: Option<&str>) -> Self {
        let session_id = session_id_from(payload, session_hint);
        let (state, previous_anchor, turn_sequence) =
            presentation_state(operation, payload, session_id.as_ref());
        Self::from_state(session_id, state, previous_anchor, turn_sequence)
    }

    fn capture_query(orientation: &Value, session_hint: Option<&str>) -> (Self, Self) {
        Self::capture_query_with_changes(orientation, session_hint, true)
    }

    fn capture_metadata_query(orientation: &Value, session_hint: Option<&str>) -> (Self, Self) {
        Self::capture_query_with_changes(orientation, session_hint, false)
    }

    fn capture_query_with_changes(
        orientation: &Value,
        session_hint: Option<&str>,
        include_world_changes: bool,
    ) -> (Self, Self) {
        let session_id = session_id_from(orientation, session_hint);
        // Reserve one turn without publishing the provisional source as an
        // observation. A later validation or publication refusal must leave the
        // cached anchor and its next-call change basis untouched.
        let mut pending = json!({"ok":false, "session_id":session_id});
        let (mut state, previous_anchor, turn_sequence) =
            presentation_state("fortress.query", &mut pending, session_id.as_ref());
        let previous_state = state.clone();
        update_orientation("fortress.query", orientation, &mut state);
        let success = Self::from_state_with_changes(
            session_id,
            state,
            previous_anchor,
            turn_sequence,
            include_world_changes,
        );
        let failure = Self {
            session_id: success.session_id.clone(),
            state: previous_state,
            previous_anchor: success.previous_anchor.clone(),
            turn_sequence,
            world_delta: Vec::new(),
            task_work: success.task_work.clone(),
        };
        (success, failure)
    }

    fn publish_query_orientation(&self, payload: &Value) {
        if !is_ok(payload) {
            return;
        }
        let Some(id) = self.session_id.as_ref() else {
            return;
        };
        if let Some(state) = orientation_sessions().get_mut(id)
            && state.turn_sequence == self.turn_sequence
        {
            update_orientation("fortress.query", payload, state);
        }
    }

    fn from_state(
        session_id: Option<String>,
        state: SessionOrientation,
        previous_anchor: Option<Value>,
        turn_sequence: u64,
    ) -> Self {
        Self::from_state_with_changes(session_id, state, previous_anchor, turn_sequence, true)
    }

    fn from_state_with_changes(
        session_id: Option<String>,
        mut state: SessionOrientation,
        previous_anchor: Option<Value>,
        turn_sequence: u64,
        include_world_changes: bool,
    ) -> Self {
        // A cache is never response-budget authority. A concurrent negotiation
        // can commit before its presentation hint arrives; read the owner's
        // current allowance after releasing the orientation lock. The returned
        // value is this response's immutable capture, like its other inputs.
        if let Some(id) = session_id.as_deref()
            && let Ok(current) = crate::server::session_output_budget::current_budget(id)
        {
            state.budget = json!({
                "max_wall_millis": current.max_wall_millis,
                "max_game_ticks": current.max_game_ticks,
                "max_entities": current.max_entities,
                "max_bytes": current.max_bytes,
                "max_output_tokens": current.max_output_tokens,
                "max_actions": current.max_actions,
            });
        }
        // World changes disclose entity facts only under observation authority.
        let world_delta = if include_world_changes && has_grant(&state, "observe") {
            world_changes(
                session_id.as_ref(),
                previous_anchor.as_ref(),
                state.anchor.as_ref(),
            )
        } else {
            Vec::new()
        };
        let task_work = if has_grant(&state, "observe")
            && let Some(id) = session_id.as_deref()
            && crate::server::task_session::read_authority(id).is_ok()
        {
            let tasks = crate::task_service::session_handles(id);
            tasks
                .as_array()
                .is_none_or(|tasks| !tasks.is_empty())
                .then(|| (tasks, crate::task_service::session_handle_coverage(id)))
        } else {
            None
        };
        Self {
            session_id,
            state,
            previous_anchor,
            turn_sequence,
            world_delta,
            task_work,
        }
    }

    /// A successful row selection can still miss the final publication bound.
    /// Rebuild that refusal from the prior observation while preserving the
    /// original authority-bearing budget, even if fitting removed its metadata.
    fn finish_query(&self, failure: &Self, result: Value, started: std::time::Instant) -> String {
        let succeeded = is_ok(&result);
        let query_budget = result["query_budget"].clone();
        let mode = result["mode"].clone();
        let query_work = result
            .get("query_work")
            .filter(|work| work.is_object())
            .cloned();
        let presentation = if succeeded { self } else { failure };
        let rendered = presentation.render(
            result,
            "fortress.query",
            AgentPhase::Inspect,
            ObservationProfile::Tactical,
        );
        let response = crate::server::query_budget::finish_response(rendered, started.elapsed());
        if !succeeded || !query_budget.is_object() {
            return response;
        }
        let Ok(mut refusal) = serde_json::from_str::<Value>(&response) else {
            return response;
        };
        if !refusal.is_object() || refusal["ok"] != false {
            return response;
        }
        refusal["query_budget"] = query_budget;
        refusal["mode"] = mode;
        if let Some(query_work) = query_work {
            refusal["query_work"] = query_work;
        }
        let rendered = failure.render(
            refusal,
            "fortress.query",
            AgentPhase::Inspect,
            ObservationProfile::Tactical,
        );
        crate::server::query_budget::finish_response(rendered, started.elapsed())
    }

    fn render(
        &self,
        mut payload: Value,
        operation: &str,
        phase: AgentPhase,
        profile: ObservationProfile,
    ) -> String {
        attach_recovery(&mut payload);
        let state = &self.state;
        let session_id = self.session_id.clone();
        let previous_anchor = self.previous_anchor.clone();
        let turn_sequence = self.turn_sequence;
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
        let continuity_basis = if is_ok(&payload) && payload["mode"] == "query_changes" {
            payload.get("basis").cloned()
        } else {
            previous_anchor.clone()
        };
        let references = match session_id.as_ref() {
            Some(id) => vec![
                json!({"kind": "resource", "uri": format!("df://session/{id}/summary")}),
                json!({"kind": "resource", "uri": format!("df://session/{id}/capabilities")}),
                json!({"kind": "resource", "uri": format!("df://session/{id}/handoff")}),
            ],
            None => Vec::new(),
        };

        let (ranked, selection) =
            ranked_attention(attention(operation, is_ok(&payload), &payload, state));
        let mut builder = AgentTurnBuilder::new(operation, phase)
            .turn_id(format!("presentation-turn-{turn_sequence}"))
            .continuity(status, continuity_basis, None, reset_reason)
            .profile(profile)
            .briefing(briefing(state, &payload))
            .changes({
                let mut listed = changes(operation, is_ok(&payload), &payload);
                listed.extend(self.world_delta.clone());
                listed
            })
            .attention(ranked)
            .active_work({
                let mut work = active_work(state);
                if payload["query_work"].is_object() {
                    let catalog = &payload["query_work"];
                    work["query_work_coverage"] = json!({
                        "status": catalog["status"],
                        "anchor": catalog["anchor"],
                        "code": catalog["code"],
                        "reason": catalog["reason"],
                        "sampling": "explicit_foreground_poll_only",
                        "absence_proven": false,
                    });
                    if catalog["status"] == "available" {
                        for key in ["query_baselines", "query_watches", "condition_watches"] {
                            work[key] = catalog[key].clone();
                        }
                    }
                }
                if has_drain_observation(operation, &payload)
                    && payload["drain_progress"]["quiescent"] != true
                {
                    work["cancellation_drains"] = json!([{
                        "scope": payload["scope"],
                        "drain_progress": payload["drain_progress"],
                        "observed_anchor": payload["observed_anchor"],
                        "finalize_certificate": payload["finalize_certificate"],
                    }]);
                }
                if let Some((tasks, coverage)) = &self.task_work {
                    work["mcp_tasks"] = tasks.clone();
                    work["mcp_task_coverage"] = coverage.clone();
                }
                work
            })
            .affordances(affordances(state))
            .recommendations(recommendations(operation, is_ok(&payload), &payload, state))
            .uncertainty(uncertainties(state))
            .coverage({
                let mut covered = coverage(&payload);
                covered["attention_selection"] = selection;
                covered
            })
            .budget({
                let mut limits = budget(state);
                if operation == "fortress.wait" && payload["wait_response_budget"].is_object() {
                    limits["admitted"]["max_bytes"] =
                        payload["wait_response_budget"]["max_bytes"].clone();
                    limits["admitted"]["max_output_tokens"] =
                        payload["wait_response_budget"]["max_output_tokens"].clone();
                }
                limits
            })
            .references(references);
        if let Some(id) = session_id {
            builder = builder.session_id(id);
        }
        if let Some(anchor) = current_anchor {
            builder = builder.anchor(anchor);
        }
        let max_output_tokens =
            if operation == "fortress.wait" && payload["wait_response_budget"].is_object() {
                let bytes = payload["wait_response_budget"]["maximum_response_bytes"]
                    .as_u64()
                    .unwrap_or(0);
                let tokens = payload["wait_response_budget"]["max_output_tokens"]
                    .as_u64()
                    .unwrap_or(0);
                tokens.min(bytes / 4)
            } else {
                state
                    .budget
                    .get("max_output_tokens")
                    .and_then(Value::as_u64)
                    .unwrap_or(crate::output_budget::DEFAULT_MAX_OUTPUT_TOKENS)
            };
        let max_output_tokens = max_output_tokens.min(
            state
                .budget
                .get("max_bytes")
                .and_then(Value::as_u64)
                .map_or(u64::MAX, |bytes| bytes / 4),
        );
        // Query publication uses the current authority-bearing budget, not this
        // orientation's opening budget. Its final guard preserves whole legacy
        // pages/cursors and permits only lossless structured-page compaction.
        let budgeted_query = operation == "fortress.query" && payload["query_budget"].is_object();
        let shaped =
            crate::output_budget::shape_for_profile(&builder.attach(payload), profile.as_str());
        if budgeted_query {
            shaped
        } else {
            crate::output_budget::fit(&shaped, max_output_tokens)
        }
    }
}

/// Preserve the original direct Rust opening API.
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
    scenario: Option<String>,
    shared: Option<bool>,
    durable: Option<bool>,
) -> String {
    fortress_open_session_with_session(
        None,
        paused,
        fortress_selector,
        requested_capabilities,
        max_wall_millis,
        max_game_ticks,
        max_entities,
        max_bytes,
        max_output_tokens,
        max_actions,
        scenario,
        shared,
        durable,
    )
}

/// Opening is also the explicit negotiation boundary for an existing session's
/// output allowance. Every other source, work and authority limit stays fixed.
#[tool(
    name = "fortress_open_session",
    description = "Open an agent-oriented deterministic laboratory session with explicit capability and work budgets. Without session_id, scenario selects empty (default), starter_fortress, besieged_fortress, starter_fortress_workforce, starter_fortress_economy, starter_fortress_provisions, starter_fortress_multilevel, starter_fortress_history, starter_fortress_resources or starter_fortress_multilevel_resources. Effect capabilities must be requested explicitly. Workforce/economy/provisions scenarios model finite workers, workshop wood and shared ingredients; resource scenarios add finite WOOD/PLANT harvesting, and multilevel scenarios provide observed rock at z=8..12 for paired-stair access. shared=true joins a fortress_selector's world and clock with independently scoped sessions. durable=true requires operator DFMCP_LAB_STATE_DIR and resumes persisted custody in a new observation epoch, fencing older owners. With session_id, supply only max_output_tokens (1..65536) to explicitly renegotiate that same session's response allowance, for example after an oversized complete plan review. Current whole-fortress Observe and Query authority are required. The complete current-work acknowledgement must fit before the new limit is installed. World, pending reviews, actions, grants, expiry, source bytes/entities, work/tick/action budgets, watches and baselines remain intact; max_bytes still caps responses. No clock progress, grant renewal or implicit commit occurs. All other opening arguments must be omitted for this alternative."
)]
#[allow(clippy::too_many_arguments)]
pub fn fortress_open_session_with_session(
    session_id: Option<String>,
    paused: Option<bool>,
    fortress_selector: Option<String>,
    requested_capabilities: Option<Vec<(String, String)>>,
    max_wall_millis: Option<u64>,
    max_game_ticks: Option<u64>,
    max_entities: Option<u32>,
    max_bytes: Option<u64>,
    max_output_tokens: Option<u32>,
    max_actions: Option<u32>,
    scenario: Option<String>,
    shared: Option<bool>,
    durable: Option<bool>,
) -> String {
    let started = std::time::Instant::now();
    let recorded = json!({"paused": paused, "fortress_selector": fortress_selector.clone(), "requested_capabilities": requested_capabilities.clone(), "max_wall_millis": max_wall_millis, "max_game_ticks": max_game_ticks, "max_entities": max_entities, "max_bytes": max_bytes, "max_output_tokens": max_output_tokens, "max_actions": max_actions, "scenario": scenario.clone(), "shared": shared, "durable": durable});
    if let Some(id) = session_id {
        let mut recorded = recorded;
        // Replay remaps this marker to the replay's existing session, keeping
        // process-local handles out of the portable call log.
        recorded["existing_session"] = json!(true);
        let result = if paused.is_some()
            || fortress_selector.is_some()
            || requested_capabilities.is_some()
            || max_wall_millis.is_some()
            || max_game_ticks.is_some()
            || max_entities.is_some()
            || max_bytes.is_some()
            || max_actions.is_some()
            || scenario.is_some()
            || shared.is_some()
            || durable.is_some()
        {
            Err(dfmcp_core::DfmcpError::new(
                dfmcp_core::ErrorCode::InvalidRequest,
                "existing-session negotiation accepts only session_id and max_output_tokens; other opening arguments must be omitted",
            ))
        } else if let Some(tokens) = max_output_tokens {
            crate::server::session_output_budget::renegotiate(id.clone(), tokens, started)
        } else {
            Err(dfmcp_core::DfmcpError::new(
                dfmcp_core::ErrorCode::InvalidRequest,
                "existing-session negotiation requires an explicit max_output_tokens allowance",
            ))
        };
        let response = match result {
            Ok(raw) => {
                // Complete publication preceded the budget change. Refresh only
                // presentation hints; no later fitting can reverse this receipt.
                refresh_published_output_budget_orientation(&raw);
                raw
            }
            Err(error) => project_response(
                crate::server::dfmcp_error_payload("fortress.open_session", &error),
                "fortress.open_session",
                AgentPhase::Orient,
                ObservationProfile::Briefing,
                Some(&id),
            ),
        };
        crate::replay::record("fortress.open_session", Some(&id), recorded, &response);
        return response;
    }
    let response = project_response(
        crate::server::open_session_in_scenario(
            paused,
            fortress_selector,
            requested_capabilities,
            max_wall_millis,
            max_game_ticks,
            max_entities,
            max_bytes,
            max_output_tokens,
            max_actions,
            scenario,
            shared,
            durable,
        ),
        "fortress.open_session",
        AgentPhase::Bootstrap,
        ObservationProfile::Briefing,
        None,
    );
    crate::replay::record("fortress.open_session", None, recorded, &response);
    response
}

// Existing Rust integrations keep the registered handler's original name.
pub use FortressOpenSessionWithSession as FortressOpenSession;

#[tool(
    description = "Observe the current laboratory fortress through the canonical agent orientation packet. Live DFHack state remains explicitly unknown."
)]
pub fn fortress_observe(session_id: Option<String>) -> String {
    let recorded = json!({});
    let response = project_response(
        crate::server::fortress_observe(session_id.clone()),
        "fortress.observe",
        AgentPhase::Orient,
        ObservationProfile::Briefing,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.observe",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

#[tool(
    description = "Run a bounded laboratory query with explicit coverage, uncertainty, affordances, and next-step guidance. JSON {\"mode\":\"structured\",\"request\":{\"schema\":\"dfmcp.query/1\",\"query\":{\"kind\":\"entities|inspect|traverse|dependencies|aggregate|search|events|spatial_entities|related_entities|ranked_entities|condition_evaluation\",...}},\"at\"?:state_hash} runs typed field projections, filters, bound continuations, generation-aware inspection, graph dependency analysis, exact integer/fixed aggregates, lexical search, or bounded retained-event windows on the current or exact retained world. Aggregate accepts optional relation {edge_kinds,direction?,target_kinds?,target_ids?,where?} and spatial {region:{min:[x,y,z],max:[x,y,z]},geometry:{kind:point,field}|{kind:cuboid,min_field,max_field},relation?:intersects|within|contains}; source filters, positive relations and geometry precede grouping/metrics. Each entity counts once despite multiple edges; Unknown membership is reported separately and totals cover selected known members only. The closed temporal wrapper {mode:temporal,request:{schema:dfmcp.query/1,query:{kind:capture|changes|baselines|release_baseline|watch|poll_watch|poll_watches|watches|cancel_watch|release_watch,...}}} owns up to eight foreground selected baselines per session; capture uses key, select and max_game_ticks, changes uses baseline with optional limit/continuation, release_baseline uses baseline. Foreground watches use key, condition, optional failure_condition, deadline_tick, optional poll_interval_ticks and stable_observations; poll_watches evaluates one to eight selected handles (or all when omitted) at the same current snapshot with atomic publication. Field/generation, observed-projection entity counts, pause/tick and Boolean conditions use registered laboratory evidence. Polling never advances game time or acquires native observations. Structured condition_evaluation preflights the same predicates without retaining a watch, including exact historical at sources. Complete results and current active work are published atomically before private baseline/watch changes; restart loses handles, and explicit polling invalidates nonterminal watches after restore. The closed history wrapper {mode:history,limit?:1..32,order?:newest_first|oldest_first,continuation?} discovers exact retained recording ordinals and anchors without transcript-held hashes; each page includes bounded current-epoch condition_replay windows with distinct retention holes and observation cursor gaps. Its opaque cursor binds owner, current anchor and whole catalog; discovery never observes, pins or scans world content. The closed condition_replay wrapper takes from (exact retained hash), optional to (default current), definition (the same condition/failure/deadline/cadence/stability grammar), and optional detail:summary|evidence. It replays only consecutively recorded retained snapshots through the laboratory watch state machine, refuses collected interiors and unrecorded current endpoints, reports cursor gaps, and creates no handles or continuous-history proof. The closed query_changes wrapper takes before (exact retained hash), optional after (default current), select (entities with kinds/where/fields/order), optional limit and continuation. It reports generation-aware entered_result, left_result and changed_in_result rows between complete endpoint selections, with jointly charged source budgets and current-anchor-bound pages. It does not establish intermediate history. Structured results never authorize effects; historical reads recheck current Query authority. The complete page and Agent Turn must fit without dropping rows. Current mode \"civilian_recovery\" reports the world-owned stable-clear interval and exact roster; it accepts no historical at selector or caller clearance. Other mode: \"summary\" (default), \"entities\", or JSON: {\"mode\":\"observation\",\"completeness_profile\":\"control-minimum|operations|spatial|historical|research-full\",\"section\":\"entities|relations|chunks|events\",\"limit\"?,\"offset\"?} (bounded profile records with distinct source/projected anchors and explicit omissions), {\"mode\":\"entities\",\"kind\"?,\"limit\"?,\"offset\"?}, {\"mode\":\"terrain\",\"min\":[x,y,z],\"max\":[x,y,z]}, {\"mode\":\"search\",\"text\":...,\"limit\"?}, {\"mode\":\"path\",\"from\":[x,y,z],\"to\":[x,y,z]} (walkability route over observed terrain; unproven absence is reported as unknown); entities accept \"where\": {\"field\",\"op\": eq|ne|lt|le|gt|ge,\"value\"} or {\"all\"|\"any\":[...]} or {\"not\":{...}} (unknown facts never match); add \"at\":<state_hash> to read an earlier retained world version exactly; {\"mode\":\"changes\",\"since\":<state_hash>} lists observed changes from that version to now (each session retains its last 32 versions)."
)]
pub fn fortress_query(session_id: Option<String>, mode: Option<String>) -> String {
    let started = std::time::Instant::now();
    let recorded = json!({"mode": mode});
    let response =
        match crate::server::fortress_query_publication(session_id.clone(), mode, started) {
            // This packet was accepted before the retained-state transaction
            // committed. No later fit or deadline check may replace it.
            crate::server::QueryPublication::Published(response) => response,
            crate::server::QueryPublication::Pending(raw) => {
                let mut payload = response_payload(&raw, "fortress.query");
                if payload["mode"] == "history" {
                    let (presentation, failure) = ResponsePresentation::capture_metadata_query(
                        &payload,
                        session_id.as_deref(),
                    );
                    // No world-diff acquisition and no acknowledged observation
                    // publication for a metadata-only retained catalog.
                    presentation.finish_query(&failure, payload, started)
                } else {
                    let presentation = ResponsePresentation::capture(
                        "fortress.query",
                        &mut payload,
                        session_id.as_deref(),
                    );
                    let projected = presentation.render(
                        payload,
                        "fortress.query",
                        AgentPhase::Inspect,
                        ObservationProfile::Tactical,
                    );
                    crate::task_service::notify_progress();
                    crate::server::query_budget::finish_response(projected, started.elapsed())
                }
            }
            crate::server::QueryPublication::Projected { orientation, query } => {
                let orientation = response_payload(&orientation, "fortress.query");
                let (presentation, failure) =
                    ResponsePresentation::capture_query(&orientation, session_id.as_deref());
                let result = query.execute(started, &|page| {
                    let rendered = presentation.render(
                        page.clone(),
                        "fortress.query",
                        AgentPhase::Inspect,
                        ObservationProfile::Tactical,
                    );
                    crate::server::query_budget::lossless_page_fits(
                        &rendered,
                        &page["query_budget"],
                    )
                });
                crate::task_service::notify_progress();
                let response = presentation.finish_query(&failure, result, started);
                if let Ok(payload) = serde_json::from_str::<Value>(&response) {
                    presentation.publish_query_orientation(&payload);
                }
                response
            }
        };
    crate::replay::record("fortress.query", session_id.as_deref(), recorded, &response);
    response
}

#[tool(
    description = "Compare two to four same-objective alternatives with blueprint JSON {\"template\":\"compare_candidates\",\"version\":1,\"horizon_ticks\":1000,\"candidates\":[{\"key\":\"a\",\"blueprint\":{...}},{\"key\":\"b\",\"blueprint\":{...}}]}. Alternatives use exactly one blueprint, actions, or paused_target source; each may have its own summary. For complete production/provisioned_room/connected_room projects, use compare_candidates version 2 with forecast_scope=complete_project; it follows up to four discarded model stages per candidate and compares the full original goal with all-stage risk, checkpoint and action costs. Version 1 remains a current-stage forecast. Selections remain version 1 and prepare only the current executable stage. Comparisons are stateless conditional laboratory forecasts that preserve pending work. Pass an emitted select_candidate blueprint unchanged to prepare the chosen source under current authority, then use fortress.commit. Candidate wrappers cannot be combined with outer summary/actions/paused_target. Compile an intent into a sealed plan without effects and keep it visible as active work. Without actions it compiles pause/resume (paused_target). actions is a JSON array of steps {\"action\":{\"kind\":...},\"depends_on\":[step indices]} with kinds: pause{paused}, designate_dig{min,max,mode: mine|channel|up_stair|down_stair|up_down_stair|ramp|remove_construction}, build{building e.g. workshop:Still|furniture:Bed|farm_plot, location, min, max, materials?}, place_furniture{furniture: bed|door|table|chair, item: exact canonical item ID, location} (allocates a manufactured laboratory item and proves installation), harvest_resource{source: exact canonical source ID, source_generation, resource: WOOD|PLANT, amount: 1..100} (finite-harvest/1 laboratory sources only; requires guarded designate and checkpoint, consumes shared worker time and credits supply only on completed units), set_labor{units,labor,enabled}, create_work_order{name,job_token,amount}, configure_stockpile{stockpile,accepts,max_bins?,max_barrels?,max_wheelbarrows?}, assign_squad{units,squad}, set_burrow_membership{units,burrow,assigned}, set_standing_order{key,value}. Coordinates are [x,y,z]; entity IDs are decimal strings. Postconditions, game-time obligations and compensations are sealed from the reference action model and returned per step. Instead of actions, blueprint is an objective the planner decomposes. For finite laboratory raw-material stock goals, template resource_acquisition accepts one or two distinct quotas with resource WOOD|PLANT and positive minimum, optional assign_labor (default false), and fixed planner resource_acquisition_v1. It allocates complete source-qualified deficits into bounded harvest actions, optionally enables observed disabled labor, and retains the original stock objective. Pending harvest is never inventory; observe completed inputs before planning production. Existing active harvest or material-consuming work can require observation or explicit drain first. A changed anchor requires a fresh Plan/review of the original request. Use continue_goal with a retained resource plan digest after its exact work is quiet to pursue the unchanged minima and labor policy; completed yield is retained and satisfied stock causes no new work. For observed civilian protection use actions={\"workflow\":\"civilian_protection_v1\",\"burrow\":\"3001\",\"civilians\":[\"1001\"]} while paused. After the world-owned civilian_recovery query reports a stable complete clear interval, blueprint={\"template\":\"civilian_release\",\"burrow\":\"3001\"} seals a separately reviewed release of the complete current civilian roster. Every release effect checks current threats and exact generations; changed observations require a fresh review, and restart/restore resets clearance. For item-backed furnished rooms use {\"template\":\"provisioned_room\",\"layout\":{\"template\":\"bedroom_cluster\",\"origin\":[x,y,z],\"rooms\":n,\"room_size\":[w,h]}} or a dining_hall layout with origin,width,height. It reuses eligible exact furniture, manufactures missing items from finite WOOD, excavates, then installs them; the final goal proves every original terrain region and exact installed item. Optional prerequisites reuse the production setup grammar. Explicit acquire_resources:{assign_labor:false|true} on provisioned_room selects provisioned_room_resources_v1; missing finite inputs produce a harvest-only review followed by a separately reviewed continue_goal room stage. Connected wrappers select connected_room_resources_v1, or explicitly connected_room_stairs_resources_v1 for paired stairs. Complete original room truth remains separate from historical acquisition-stage proof. In shared-workforce laboratory worlds, prerequisites.workforce=complete_project_v1 also staffs required mining and construction; assign_labor=true permits qualified labor enables and false requires observed enabled workers. For access from an observed entrance, wrap that complete request as {\"template\":\"connected_room\",\"room\":{\"template\":\"provisioned_room\",\"layout\":{...}},\"access\":{\"entry\":[x,y,z],\"region\":{\"min\":[x,y,z],\"max\":[x,y,z]}}}. The default connected_room_v1 excavates safe same-level connectors and proves current floor access around building occupancy. Set planner=connected_room_stairs_v1 explicitly for a multi-level domain: it reuses paired laboratory stairs or excavates ordinary rock outside room floor masks as UpDownStair shafts, and proves current entrance-to-furnishing access across levels. Both generations are bounded to 1024 cells and preserve their exact access model through continue_goal; native directional stairs and hauling are not proved. Existing abstract blueprint forms remain: {\"template\":\"bedroom_cluster\",\"origin\":[x,y,z],\"rooms\":n,\"room_size\":[w,h]} (a bed per room), {\"template\":\"dining_hall\",\"origin\",\"width\",\"height\"} (table and chair), {\"template\":\"workshop_hub\",\"origin\",\"bays\"}, {\"template\":\"stockpile_vault\",\"origin\",\"width\",\"height\",\"category\"}, {\"template\":\"production\",\"quotas\":[{\"item\":\"DRINK|FOOD|BED|DOOR|TABLE|CHAIR\",\"minimum\":n}]} (work orders sized against observed stock by the production compiler; optional acquire_resources object with assign_labor=false pins resource_aware_v1 and retains output quotas across separately reviewed acquisition and production stages. It budgets WOOD/PLANT and consumption during harvesting, proposes only the current stage, and requires continue_goal after observed inputs and complete lineage quiescence. Acquisition labor permission is independent of production prerequisites. Adding reserve_until_tick selects resource_reserve_v1, which budgets the later of the explicit horizon and complete staged work and retains the same absolute tick through continuation /5. Current minima may still be True while the remaining future reserve needs production; fully funded reserves and expired horizons refuse new work. Every stage requires current Observe/Query/Plan and its ordinary effect capabilities; optional orders:\"dependency_preserving_v1\" adds source-bound unique order names, explicit per-batch material conditions and named predecessor completion conditions alongside the action DAG, preserved through replay and continuation), or {\"template\":\"continue_goal\",\"plan_digest\":\"retained original goal digest\"} (a new reviewed pursuit of a retained production, resource acquisition, provisioned-room or connected-room request; preserves all original quotas or room slots, resource labor policy, setup options and connected entrance/domain, keeps valid installed furnishings, and plans missing work including access repair; requires current Observe/Plan authority and proven quiet earlier lineage work; original historical action outcomes remain unchanged)."
)]
pub fn fortress_plan(
    session_id: Option<String>,
    summary: Option<String>,
    paused_target: Option<bool>,
    actions: Option<String>,
    blueprint: Option<String>,
) -> String {
    let started = std::time::Instant::now();
    let recorded = json!({"summary": summary, "paused_target": paused_target, "actions": actions, "blueprint": blueprint});
    let response = match crate::server::plan_request_publication(
        session_id.clone(),
        summary,
        paused_target,
        actions,
        blueprint,
        None,
        started,
    ) {
        crate::server::PlanPublication::Pending(raw) => project_response(
            raw,
            "fortress.plan",
            AgentPhase::Propose,
            ObservationProfile::Tactical,
            session_id.as_deref(),
        ),
        crate::server::PlanPublication::Published(raw) => {
            refresh_published_plan_orientation(&raw);
            raw
        }
    };
    crate::replay::record("fortress.plan", session_id.as_deref(), recorded, &response);
    response
}

#[tool(
    description = "Revalidate and idempotently commit the pending plan, then return observed effect and verification state."
)]
pub fn fortress_commit(session_id: Option<String>, plan_digest: String) -> String {
    let recorded = json!({"plan_digest": plan_digest});
    let response = project_response(
        crate::server::fortress_commit(session_id.clone(), plan_digest),
        "fortress.commit",
        AgentPhase::Commit,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.commit",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

/// Preserve the original direct Rust API and its untargeted wait semantics.
pub fn fortress_wait(session_id: Option<String>, max_game_ticks: Option<u64>) -> String {
    fortress_wait_for_selection(session_id, max_game_ticks, None, None)
}

/// Preserve the original selected-plan Rust API.
pub fn fortress_wait_for_plan(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    plan_digest: Option<String>,
) -> String {
    fortress_wait_for_selection(session_id, max_game_ticks, plan_digest, None)
}

/// Foreground wait for one exact retained laboratory condition.
pub fn fortress_wait_for_watch(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    watch_id: String,
) -> String {
    fortress_wait_for_selection(session_id, max_game_ticks, None, Some(watch_id))
}

/// Preserve the original four-argument selected-wait Rust API.
pub fn fortress_wait_for_selection(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    plan_digest: Option<String>,
    watch_id: Option<String>,
) -> String {
    fortress_wait_for_targets(
        session_id,
        max_game_ticks,
        plan_digest,
        watch_id,
        None,
        None,
    )
}

/// Foreground wait for one explicit set of independent laboratory conditions.
pub fn fortress_wait_for_watches(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    watch_ids: Vec<String>,
    watch_policy: Option<String>,
) -> String {
    fortress_wait_for_targets(
        session_id,
        max_game_ticks,
        None,
        None,
        Some(watch_ids),
        watch_policy,
    )
}

/// The wire still exposes one fortress_wait tool. A selector changes when its
/// bounded foreground work returns; every authority is checked independently.
#[tool(
    name = "fortress_wait",
    description = "Progress retained laboratory work in bounded foreground intervals. max_game_ticks caps time advanced while unpaused; each interval observes prerequisites and dispatches authorized ready successors, then persists progress before continuing. Optional plan_digest selects one exact original commit owned by this session or a verified durable original with complete recovered step evidence, stopping when its original goal, action proofs and physical work complete or need attention. Recovered selection observes original carried work without restoring dispatch handles, adopting another live session's work or dispatching previously deferred steps. Optional watch_id instead selects one retained laboratory condition returned by temporal watch registration; intervals stop at its original cadence and deadline, sample the exact published world, and return early on satisfaction, failure, expiry, invalidation, cancellation or unknown evidence. Alternatively watch_ids selects one to eight unique owned handles with watch_policy=all_satisfied (default) or any_satisfied. The whole selected set is sampled atomically at each published frontier using each original cadence and deadline. Failed, expired, invalidated, cancelled and blocked_unknown members take precedence over group completion. Group success aggregates retained watch outcomes, including historical terminals; it never proves simultaneous current truth. plan_digest, watch_id and watch_ids are mutually exclusive, and watch_policy is valid only with watch_ids. Watch selection requires current whole-fortress Observe and Query; positive time also requires ControlClock, with the requested horizon authorized before advancement. A zero-tick watch wait samples only the current world. Selection never changes predicates, deadlines, required stability, unpauses, cancels, follows descendant plans or grants mutation authority. Other committed work continues during shared world time. selected_watch_wait preserves the original definition and exact sample evidence; cursor gaps reset positive stability. Without a selector, duration-based behavior is unchanged. Reports actual elapsed and remaining ticks; pause and progression errors stop further advancement."
)]
pub fn fortress_wait_for_targets(
    session_id: Option<String>,
    max_game_ticks: Option<u64>,
    plan_digest: Option<String>,
    watch_id: Option<String>,
    watch_ids: Option<Vec<String>>,
    watch_policy: Option<String>,
) -> String {
    let mut recorded = json!({"max_game_ticks": max_game_ticks});
    if let Some(digest) = &plan_digest {
        recorded["plan_digest"] = json!(digest);
    }
    if let Some(watch) = &watch_id {
        recorded["watch_id"] = json!(watch);
    }
    if let Some(watches) = &watch_ids {
        recorded["watch_ids"] = json!(watches);
    }
    if let Some(policy) = &watch_policy {
        recorded["watch_policy"] = json!(policy);
    }
    let selector_error = if watch_policy.is_some() && watch_ids.is_none() {
        Some("watch_policy requires an explicit watch_ids selection")
    } else if watch_policy
        .as_deref()
        .is_some_and(|policy| !matches!(policy, "all_satisfied" | "any_satisfied"))
    {
        Some("watch_policy must be all_satisfied or any_satisfied")
    } else if plan_digest.is_some() && watch_id.is_some() && watch_ids.is_none() {
        Some("plan_digest and watch_id are mutually exclusive wait selectors")
    } else if usize::from(plan_digest.is_some())
        + usize::from(watch_id.is_some())
        + usize::from(watch_ids.is_some())
        > 1
    {
        Some("plan_digest, watch_id and watch_ids are mutually exclusive wait selectors")
    } else {
        None
    };
    let publication = if let Some(message) = selector_error {
        crate::server::WaitPublication::Pending(crate::server::coded_error_payload(
            "fortress.wait",
            dfmcp_core::ErrorCode::InvalidRequest,
            message,
        ))
    } else if let Some(watches) = watch_ids {
        crate::server::wait_for_watches(session_id.clone(), max_game_ticks, watches, watch_policy)
    } else {
        match (plan_digest, watch_id) {
            (Some(_), Some(_)) => {
                crate::server::WaitPublication::Pending(crate::server::coded_error_payload(
                    "fortress.wait",
                    dfmcp_core::ErrorCode::InvalidRequest,
                    "plan_digest and watch_id are mutually exclusive wait selectors",
                ))
            }
            (None, Some(watch)) => {
                crate::server::wait_for_watch(session_id.clone(), max_game_ticks, watch)
            }
            (Some(digest), None) => crate::server::WaitPublication::Pending(
                crate::server::wait_for_plan(session_id.clone(), max_game_ticks, digest),
            ),
            (None, None) => crate::server::WaitPublication::Pending(
                crate::server::wait_with_ticks(session_id.clone(), max_game_ticks),
            ),
        }
    };
    let response = match publication {
        crate::server::WaitPublication::Pending(raw) => project_response(
            raw,
            "fortress.wait",
            AgentPhase::Verify,
            ObservationProfile::Pulse,
            session_id.as_deref(),
        ),
        crate::server::WaitPublication::Published(raw) => {
            // The complete packet was admitted before the watch root changed.
            // Refresh orientation only; never refit or replace that publication.
            if let Ok(mut payload) = serde_json::from_str::<Value>(&raw) {
                let resolved = payload["session_id"].as_str().map(str::to_owned);
                let _ = presentation_state("fortress.wait", &mut payload, resolved.as_ref());
            }
            crate::task_service::notify_progress();
            raw
        }
    };
    crate::replay::record("fortress.wait", session_id.as_deref(), recorded, &response);
    response
}

// Existing Rust integrations retain every prior registered handler alias.
pub use FortressWaitForTargets as FortressWait;
pub use FortressWaitForTargets as FortressWaitForPlan;
pub use FortressWaitForTargets as FortressWaitForSelection;

#[tool(
    description = "Request, drain, compensate when authorized, and finalize cancellation while preserving recovery state. mode: compensate_reversible (default) | stop_future_steps | emergency_pause_and_drain. scope: last_action (default), plan (last committed plan), plan:<64-lowercase-original-plan-digest> (one exact commit owned by this session, leaving other plans and descendants alone), oldest_open_plan (earliest retained original plan, for bounded cleanup), or session (all retained open work across this session's plans). With scope=session or a selected plan, explicit emergency_pause_and_drain pauses under current Observe and ControlClock authority before draining, even when selected work is already quiet or a durable restart left no session action handles. Emergency drains require at least 1,500 output tokens and 6,000 bytes before effects. They create no replacement handles; exact pause evidence remains separate from unresolved carried work and physical drain success. Drain dependents first and return a finalize_certificate only when goal states are terminal and physical work is proven quiescent. Failed and Verified receipts remain history; their remaining work can be stopped with separate evidence. Separately, mode withdraw_goal with exact scope goal:<original-plan-digest> archives one verified, physically quiet original under current Observe, Query and reversible Plan authority, freeing an active goal slot while retaining its source and proof. Withdrawal does not stop work, prove success, withdraw descendants, or permit continuation of that original. Explicit mode close_session with scope session closes only after owned work is terminal and physically quiet and no Task monitor remains active; it discards pending reviews and volatile query/history handles, releases session membership and authority, and preserves durable state. Ordinary scope session retains its drain meaning."
)]
pub fn fortress_cancel(
    session_id: Option<String>,
    mode: Option<String>,
    scope: Option<String>,
) -> String {
    if mode.as_deref() == Some("close_session") {
        // Teardown fits and publishes its final packet before revoking its owner.
        // Ordinary projection and replay recording require an open session.
        return crate::server::cancel_in_scope(session_id, mode, scope);
    }
    let recorded = json!({"mode": mode, "scope": scope});
    let response = project_response(
        crate::server::cancel_in_scope(session_id.clone(), mode, scope),
        "fortress.cancel",
        AgentPhase::Reconcile,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.cancel",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

#[tool(
    description = "Create a content-addressed laboratory checkpoint and return recovery affordances and continuity."
)]
pub fn fortress_checkpoint(session_id: Option<String>, label: Option<String>) -> String {
    let recorded = json!({"label": label});
    let response = project_response(
        crate::server::fortress_checkpoint(session_id.clone(), label),
        "fortress.checkpoint",
        AgentPhase::Commit,
        ObservationProfile::Tactical,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.checkpoint",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

#[tool(
    description = "Restore a checkpoint into a new observation epoch and expose every invalidated pre-restore handle."
)]
pub fn fortress_restore(session_id: Option<String>, checkpoint_id: String) -> String {
    let recorded = json!({"checkpoint_id": checkpoint_id});
    let response = project_response(
        crate::server::fortress_restore(session_id.clone(), checkpoint_id),
        "fortress.restore",
        AgentPhase::Reconcile,
        ObservationProfile::Forensic,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.restore",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

#[tool(
    description = "Explain state transitions or numeric entity graph dependencies. entity_id also accepts plan:<64-lowercase-plan-digest> or goal:<digest> for exact original-plan/goal explanation: current unmet/unknown requirements, project/reserve truth, historical achievement, original receipts and physical work stay distinct. Follow returned next/original_steps/original_requirements calls for exact-source bounded pages. Typed inspection requires current whole-fortress Observe and Query and does not poll, change historical proof, dispatch effects or advance the clock."
)]
pub fn fortress_explain(session_id: Option<String>, entity_id: Option<String>) -> String {
    // The owner renders the complete, currently authorized immutable inspection
    // under one lock. Do not resample proof, alter replay/orientation, notify
    // Task progress or replace its packet after the read has completed.
    if entity_id
        .as_deref()
        .is_some_and(crate::server::goal_explanation::is_subject)
    {
        return crate::server::fortress_explain(session_id, entity_id);
    }
    let recorded = json!({"entity_id": entity_id});
    let response = project_response(
        crate::server::fortress_explain(session_id.clone(), entity_id),
        "fortress.explain",
        AgentPhase::Inspect,
        ObservationProfile::Forensic,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.explain",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

#[tool(
    description = "Diagnose the control plane and return findings, uncertainties, recovery guidance, and orientation."
)]
pub fn fortress_doctor(session_id: Option<String>) -> String {
    let recorded = json!({});
    let response = project_response(
        crate::server::fortress_doctor(session_id.clone()),
        "fortress.doctor",
        AgentPhase::Inspect,
        ObservationProfile::Forensic,
        session_id.as_deref(),
    );
    crate::replay::record(
        "fortress.doctor",
        session_id.as_deref(),
        recorded,
        &response,
    );
    response
}

/// Run the modern-only MCP 2026-07-28 server with the agent-oriented facade.
pub fn run_stdio() {
    let (tasks, runner) = match crate::task_service::LabTaskService::new() {
        Ok(service) => service,
        Err(error) => {
            eprintln!("laboratory Tasks startup failed: {error}");
            std::process::exit(1);
        }
    };
    if let Err(error) = crate::task_service::LabTaskService::install_global(tasks.clone()) {
        eprintln!("laboratory Tasks startup failed: {error}");
        std::process::exit(1);
    }
    let server = ServerBuilder::new("dwarf-fortress-mcp", env!("CARGO_PKG_VERSION"))
        .tool(FortressOpenSession)
        .tool(FortressObserve)
        .tool(FortressQuery)
        .tool(FortressPlan)
        .tool(crate::task_service::TaskAwareCommit)
        .tool(crate::task_service::TaskAwareWait)
        .tool(FortressCancel)
        .tool(FortressCheckpoint)
        .tool(FortressRestore)
        .tool(FortressExplain)
        .tool(FortressDoctor)
        // Every Agent Turn references these session resources; serve them.
        .resource(crate::resources::SessionViewResource)
        .resource(crate::resources::DoctorBundleResource)
        .request_timeout(30)
        .instructions(
            "Dwarf Fortress semantic control plane (laboratory slice). Call fortress_open_session \
             first and pass its session_id to every later call. Every result includes an \
             authority-free agent_turn packet answering what is known, what changed, what matters, \
             what active work exists, which semantic affordances are legal, what the safest next \
             protocol steps are, and what remains uncertain. Dispatch success is not goal \
             success; only authoritative observation and postcondition proof count. The current \
             adapter is process-local and does not claim live Dwarf Fortress or DFHack control. \
             fortress_commit(as_task=true) retains one original plan as a modern MCP Task. \
             fortress_wait(plan_digest=..., as_task=true) attaches a passive Task to an exact \
             existing or durable recovered original; omit max_game_ticks or set it to zero. \
             Only one task monitor may be active in this process; up to 256 task records are \
             retained until process exit. tasks/get includes completed results and failed \
             evidence. tasks/cancel drains a commit Task's original plan, but stops only \
             observation for a passive Task, leaving original physical work unchanged. \
             Explicit foreground fortress_wait calls advance laboratory game time. \
             Discover session tasks through the handoff or \
             df://session/{session_id}/tasks resource.",
        );
    let server = match server.final_tasks(tasks.runtime.clone()) {
        Ok(builder) => builder.build(),
        Err(error) => {
            eprintln!("laboratory Tasks registration failed: {error}");
            std::process::exit(1);
        }
    };
    match crate::run_with_runtime_cx(|cx| async move {
        crate::task_service::serve_with_tasks(server, runner, tasks, &cx).await
    }) {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("laboratory Tasks service stopped: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("laboratory Tasks runtime failed: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod projected_query_publication_tests {
    use super::*;
    use std::time::{Duration, Instant};

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn captured(id: &str) -> (ResponsePresentation, ResponsePresentation, Value) {
        let prior_anchor = json!({"fortress_id":"71", "epoch":1, "sequence":4,
            "game_tick":8, "state_hash":"1".repeat(64)});
        let current_anchor = json!({"fortress_id":"71", "epoch":1, "sequence":5,
            "game_tick":9, "state_hash":"2".repeat(64)});
        let mut prior = SessionOrientation::from_payload(&json!({
            "anchor":prior_anchor, "paused":true, "adapter":"memory",
            "granted_capabilities":["query"],
            "budget":{"max_output_tokens":32_768},
        }));
        prior.turn_sequence = 47;
        prior.pending_plan_digest = Some("retained-reviewed-plan".to_owned());
        orientation_sessions().insert(id.to_owned(), prior.clone());
        let mut current = prior.clone();
        current.anchor = Some(current_anchor.clone());
        current.paused = Some(false);
        let success = ResponsePresentation {
            session_id: Some(id.to_owned()),
            state: current,
            previous_anchor: Some(prior_anchor.clone()),
            turn_sequence: 47,
            world_delta: vec![json!({"kind":"unpublished_observation"})],
            task_work: None,
        };
        let failure = ResponsePresentation {
            session_id: Some(id.to_owned()),
            state: prior,
            previous_anchor: Some(prior_anchor),
            turn_sequence: 47,
            world_delta: Vec::new(),
            task_work: None,
        };
        let result = json!({"ok":true, "mode":"structured", "session_id":id,
            "anchor":current_anchor, "current_anchor":current_anchor, "paused":false,
            "matched":1, "returned":1,
            "rows":[{"entity_id":"1", "label":"unpublished row"}],
            "continuation":"sp1:unpublished-continuation",
            "query_budget":{"maximum_response_bytes":131_072,
                "max_output_tokens":32_768, "max_wall_millis":60_000}});
        (success, failure, result)
    }

    #[test]
    fn projected_query_late_refusals_keep_prior_orientation_and_the_reserved_turn() -> TestResult {
        let id = "projected-query-late-refusal";
        let (success, failure, result) = captured(id);
        let prior_anchor = failure
            .state
            .anchor
            .as_ref()
            .ok_or("prior anchor missing")?;
        for (tokens, wall_millis, elapsed, oversized_row) in [
            (32_768_u64, 0_u64, Duration::ZERO, false),
            (32_768, 1, Duration::from_millis(2), false),
            (4_096, 60_000, Duration::ZERO, true),
            (128, 0, Duration::ZERO, false),
        ] {
            let mut page = result.clone();
            page["query_work"] = json!({
                "status":"available", "anchor":result["current_anchor"],
                "query_baselines":[{"baseline":"qb1:retained", "status":"retained"}],
                "query_watches":[{"watch":"watch:retained", "terminal":false}],
                "condition_watches":[{"watch":"watch:retained", "terminal":false}],
            });
            page["query_budget"]["max_output_tokens"] = json!(tokens);
            page["query_budget"]["maximum_response_bytes"] = json!(tokens * 4);
            page["query_budget"]["max_wall_millis"] = json!(wall_millis);
            if oversized_row {
                page["rows"][0]["label"] = json!("large selected value".repeat(4_096));
            }
            let started = Instant::now()
                .checked_sub(elapsed)
                .ok_or("clock underflow")?;
            let response = success.finish_query(&failure, page, started);
            let refusal: Value = serde_json::from_str(&response)?;
            assert_eq!(refusal["ok"], false, "{refusal}");
            assert_eq!(refusal["error"]["code"], "budget_exceeded");
            assert!(response.len() as u64 <= tokens * 4, "{refusal}");
            assert!(refusal.get("rows").is_none());
            assert!(refusal.get("continuation").is_none());
            if tokens >= 4_096 {
                assert_eq!(&refusal["agent_turn"]["anchor"], prior_anchor);
                assert_eq!(refusal["agent_turn"]["briefing"]["paused"], true);
                assert_eq!(refusal["agent_turn"]["continuity"]["status"], "stale");
                assert_eq!(refusal["agent_turn"]["changes"], json!([]));
                assert_eq!(refusal["agent_turn"]["turn_id"], "presentation-turn-47");
                assert_eq!(refusal["query_budget"]["max_output_tokens"], tokens);
                assert_eq!(
                    refusal["agent_turn"]["active_work"]["query_baselines"][0]["baseline"],
                    "qb1:retained"
                );
                assert_eq!(
                    refusal["agent_turn"]["active_work"]["query_watches"][0]["watch"],
                    "watch:retained"
                );
                assert_eq!(
                    refusal["agent_turn"]["active_work"]["condition_watches"][0]["watch"],
                    "watch:retained"
                );
                assert_eq!(
                    refusal["agent_turn"]["active_work"]["pending_plans"][0]["plan_digest"],
                    "retained-reviewed-plan"
                );
            } else {
                assert_eq!(refusal["output_budget"]["tier"], "refused");
                assert!(refusal["agent_turn"]["anchor"].is_null());
            }
            success.publish_query_orientation(&refusal);
            let registry = orientation_sessions();
            let cached = registry.get(id).ok_or("orientation missing")?;
            assert_eq!(cached.anchor, failure.state.anchor);
            assert_eq!(cached.paused, Some(true));
            assert_eq!(cached.turn_sequence, 47);
        }
        orientation_sessions().remove(id);
        Ok(())
    }

    #[test]
    fn projected_query_minimal_refusal_keeps_the_original_budget_after_metadata_is_removed()
    -> TestResult {
        let id = "projected-query-minimal-refusal";
        let (success, failure, result) = captured(id);
        let prior_anchor = failure
            .state
            .anchor
            .as_ref()
            .ok_or("prior anchor missing")?;
        let mut exercised_missing_budget = false;
        for tokens in (128_u64..=2_048).step_by(32) {
            let mut page = result.clone();
            page["query_budget"]["max_output_tokens"] = json!(tokens);
            page["query_budget"]["maximum_response_bytes"] = json!(tokens * 4);
            page["query_budget"]["max_wall_millis"] = json!(0);
            let first = success.render(
                page.clone(),
                "fortress.query",
                AgentPhase::Inspect,
                ObservationProfile::Tactical,
            );
            let first = crate::server::query_budget::finish_response(first, Duration::ZERO);
            let first: Value = serde_json::from_str(&first)?;
            if first["ok"] != false
                || first["query_budget"].is_object()
                || first["agent_turn"]["anchor"] != result["current_anchor"]
            {
                continue;
            }
            exercised_missing_budget = true;
            let response = success.finish_query(&failure, page, Instant::now());
            let refusal: Value = serde_json::from_str(&response)?;
            assert_eq!(refusal["ok"], false, "{refusal}");
            assert!(response.len() as u64 <= tokens * 4, "{refusal}");
            assert_eq!(refusal["output_budget"]["max_output_tokens"], tokens);
            assert!(refusal.get("rows").is_none());
            assert!(refusal.get("continuation").is_none());
            if !refusal["agent_turn"]["anchor"].is_null() {
                assert_eq!(&refusal["agent_turn"]["anchor"], prior_anchor);
            }
            success.publish_query_orientation(&refusal);
            assert_eq!(
                orientation_sessions()
                    .get(id)
                    .ok_or("orientation missing")?
                    .anchor,
                failure.state.anchor
            );
            break;
        }
        orientation_sessions().remove(id);
        assert!(
            exercised_missing_budget,
            "minimal refusal branch was not exercised"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(raw: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(raw)?)
    }

    fn open_test_session(
        fortress_selector: &str,
    ) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        parsed(&fortress_open_session(
            Some(true),
            Some(fortress_selector.to_owned()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ))
    }

    #[test]
    fn cold_agent_is_oriented_in_one_response()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let response = open_test_session("918273645")?;
        assert_eq!(response["ok"], true);
        assert_eq!(response["agent_turn"]["phase"], "bootstrap");
        assert_eq!(response["agent_turn"]["continuity"]["status"], "bootstrap");
        assert_eq!(response["agent_turn"]["briefing"]["paused"], true);
        assert!(response["agent_turn"]["request_id"].is_null());
        assert!(response["agent_turn"]["turn_id"].is_string());
        assert!(response["agent_turn"]["affordances"].as_array().is_some());
        assert!(
            response["agent_turn"]["recommendations"]
                .as_array()
                .is_some()
        );
        Ok(())
    }

    #[test]
    fn prepared_plan_survives_context_loss_in_active_work()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let opened = open_test_session("918273646")?;
        let session_id = opened["session_id"]
            .as_str()
            .ok_or("open_session did not return a session_id")?
            .to_owned();
        let planned = parsed(&fortress_plan(
            Some(session_id),
            Some("resume the lab".to_owned()),
            Some(false),
            None,
            None,
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
    fn repeated_unchanged_observation_is_a_heartbeat()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let opened = open_test_session("918273647")?;
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
        assert_eq!(second["agent_turn"]["continuity"]["status"], "heartbeat");
        Ok(())
    }

    #[test]
    fn checkpoint_is_visible_in_later_briefings()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let opened = open_test_session("918273648")?;
        let session_id = opened["session_id"]
            .as_str()
            .ok_or("open_session did not return a session_id")?
            .to_owned();
        let checkpoint = parsed(&fortress_checkpoint(
            Some(session_id.clone()),
            Some("handoff".to_owned()),
        ))?;
        let checkpoint_id = checkpoint["checkpoint_id"].clone();
        let observed = parsed(&fortress_observe(Some(session_id)))?;
        assert_eq!(
            observed["agent_turn"]["briefing"]["last_checkpoint_id"],
            checkpoint_id
        );
        Ok(())
    }

    #[test]
    fn errors_keep_recovery_and_orientation() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let response = parsed(&fortress_observe(None))?;
        assert_eq!(response["ok"], false);
        assert_eq!(response["agent_turn"]["operation"], "fortress.observe");
        assert!(response["error"]["recovery"]["class"].is_string());
        assert!(response["agent_turn"]["uncertainty"].as_array().is_some());
        Ok(())
    }
}

#[cfg(test)]
#[path = "lab_actions_mcp_tests.rs"]
mod lab_actions_tests;

#[cfg(test)]
#[path = "resource_plan_orientation_tests.rs"]
mod resource_plan_orientation_tests;

#[cfg(test)]
#[path = "lab_alerts_mcp_tests.rs"]
mod lab_alerts_tests;

#[cfg(test)]
#[path = "laboratory_workforce_mcp_tests.rs"]
mod laboratory_workforce_mcp_tests;

#[cfg(test)]
#[path = "durable_lab_tests.rs"]
mod durable_lab_tests;

#[cfg(test)]
#[path = "threat_corpus_tests.rs"]
mod threat_corpus_tests;

#[cfg(test)]
#[path = "dpor_tests.rs"]
mod dpor_tests;

#[cfg(test)]
#[path = "selected_wait_facade_tests.rs"]
mod selected_wait_facade_tests;

#[cfg(test)]
#[path = "recovered_plan_wait_facade_tests.rs"]
mod recovered_plan_wait_facade_tests;

#[cfg(test)]
#[path = "durable_reserve_wait_facade_tests.rs"]
mod durable_reserve_wait_facade_tests;

#[cfg(test)]
#[path = "session_output_budget_facade_tests.rs"]
mod session_output_budget_facade_tests;

#[cfg(test)]
#[path = "lab_history_catalog_presentation_tests.rs"]
mod history_catalog_presentation_tests;

#[cfg(test)]
mod closed_presentation_tests {
    use super::*;

    #[test]
    fn delayed_open_projection_cannot_recreate_a_closed_orientation_slot()
    -> Result<(), Box<dyn std::error::Error>> {
        let _serial = crate::test_serial();
        let raw = crate::server::fortress_open_session(
            Some(true), Some("73907".into()), None, None, None, None, None, None, None,
        );
        let opened: Value = serde_json::from_str(&raw)?;
        assert_eq!(opened["ok"], true, "{opened}");
        let id = opened["session_id"].as_str().ok_or("session ID")?.to_owned();
        let closed: Value = serde_json::from_str(&fortress_cancel(
            Some(id.clone()), Some("close_session".into()), Some("session".into()),
        ))?;
        assert_eq!(closed["ok"], true, "{closed}");
        let late: Value = serde_json::from_str(&project_response(
            raw, "fortress.open_session", AgentPhase::Orient, ObservationProfile::Briefing, Some(&id),
        ))?;
        assert_eq!(late["ok"], false, "{late}");
        assert_eq!(late["error"]["code"], "session_not_found");
        assert!(!has_session_orientation(&id));
        Ok(())
    }
}
