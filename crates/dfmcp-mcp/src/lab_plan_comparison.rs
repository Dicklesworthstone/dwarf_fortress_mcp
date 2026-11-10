//! Same-source comparisons of a bounded set of original laboratory intents.
//!
//! Every alternative is compiled and simulated by the ordinary planner and
//! adapter. The discarded forks, candidate digests and recommendation grant no
//! dispatch authority. Selection returns an original source for a fresh normal
//! preparation; this module never installs a pending plan.

use std::collections::BTreeSet;
use std::fmt;
use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, IntentId, OperationContext, Result, RiskTier,
};
use dfmcp_intent::{Intent, PreparedPlan, StaticPlanner};
use dfmcp_lab::MemoryAdapter;
use dfmcp_world::{Predicate, WorldSnapshot};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{MapAccess, SeqAccess, Visitor},
};
use serde_json::{Value, json};

use super::PlanSource;
use super::plan_forecast::{ForecastBudget, forecast_plan_bounded};
use super::room_provisioning::{RoomCompileStage, classify_room_stage};

const MAX_REQUEST_BYTES: usize = 128 * 1024;
const MAX_CANDIDATES: usize = 4;
const MAX_KEY_BYTES: usize = 64;
const POLICY: &str =
    "complete_predictions_then_risk_checkpoint_completion_tick_action_count_key_v1";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Candidate {
    key: String,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    summary: Option<String>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    actions: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    blueprint: Option<Value>,
    #[serde(
        default,
        deserialize_with = "present",
        skip_serializing_if = "Option::is_none"
    )]
    paused_target: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Anchor {
    fortress_id: String,
    game_tick: u64,
    epoch: u64,
    sequence: u64,
    state_hash: String,
}

#[derive(Debug)]
pub(super) struct Comparison {
    horizon_ticks: u64,
    candidates: Vec<Candidate>,
}

#[derive(Debug)]
pub(super) struct Selection {
    expected_anchor: Anchor,
    candidate: Candidate,
    candidate_digest: String,
}

#[derive(Debug)]
pub(super) enum Request {
    Compare(Comparison),
    Select(Selection),
}

#[derive(Deserialize)]
#[serde(tag = "template", deny_unknown_fields)]
enum Envelope {
    #[serde(rename = "compare_candidates")]
    Compare {
        version: u32,
        horizon_ticks: u64,
        candidates: Vec<Candidate>,
    },
    #[serde(rename = "select_candidate")]
    Select {
        version: u32,
        expected_anchor: Anchor,
        candidate: Candidate,
        candidate_digest: String,
    },
}

fn present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

/// Detect this additive request family without claiming ordinary blueprints.
/// Decode the complete original bytes strictly before normalizing nested action
/// or blueprint JSON: duplicate keys must not disappear in a Value round trip.
pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err(exhausted(
            "candidate request exceeds its explicit byte bound",
        ));
    }
    let selected = serde_json::from_str::<Value>(raw).ok().is_some_and(|s| {
        matches!(
            s["template"].as_str(),
            Some("compare_candidates" | "select_candidate")
        )
    });
    if !selected {
        return Ok(None);
    }
    let strict: UniqueValue = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid candidate request: {error}")))?;
    let envelope: Envelope = serde_json::from_value(strict.0)
        .map_err(|error| invalid(format!("invalid candidate request: {error}")))?;
    match envelope {
        Envelope::Compare {
            version,
            horizon_ticks,
            candidates,
        } => {
            version_one(version)?;
            if horizon_ticks == 0 {
                return Err(invalid(
                    "candidate comparison needs a nonzero common horizon_ticks",
                ));
            }
            if !(2..=MAX_CANDIDATES).contains(&candidates.len()) {
                return Err(invalid(
                    "candidate comparison requires two to four alternatives",
                ));
            }
            let mut keys = BTreeSet::new();
            for candidate in &candidates {
                candidate.validate()?;
                if !keys.insert(&candidate.key) {
                    return Err(invalid("candidate keys must be distinct"));
                }
            }
            Ok(Some(Request::Compare(Comparison {
                horizon_ticks,
                candidates,
            })))
        }
        Envelope::Select {
            version,
            expected_anchor,
            candidate,
            candidate_digest,
        } => {
            version_one(version)?;
            candidate.validate()?;
            if !canonical_u64(&expected_anchor.fortress_id)
                || !lower_hex(&expected_anchor.state_hash, 64)
                || !lower_hex(&candidate_digest, 64)
            {
                return Err(invalid(
                    "selection requires exact canonical anchor and digest strings",
                ));
            }
            Ok(Some(Request::Select(Selection {
                expected_anchor,
                candidate,
                candidate_digest,
            })))
        }
    }
}

fn version_one(version: u32) -> Result<()> {
    if version == 1 {
        Ok(())
    } else {
        Err(invalid("candidate requests require version 1"))
    }
}

fn lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn canonical_u64(value: &str) -> bool {
    value.len() <= 20
        && value
            .parse::<u64>()
            .is_ok_and(|number| number.to_string() == value)
}

impl Candidate {
    fn validate(&self) -> Result<()> {
        if self.key.is_empty()
            || self.key.len() > MAX_KEY_BYTES
            || !self
                .key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err(invalid(
                "candidate key must contain 1..64 ASCII letters, digits, '_' or '-'",
            ));
        }
        if self
            .summary
            .as_ref()
            .is_some_and(|s| s.len() > super::MAX_SUMMARY_BYTES)
        {
            return Err(exhausted(
                "candidate summary exceeds its explicit byte bound",
            ));
        }
        if [
            self.actions.is_some(),
            self.blueprint.is_some(),
            self.paused_target.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            != 1
        {
            return Err(invalid(
                "each candidate names exactly one of actions, blueprint or paused_target",
            ));
        }
        if self
            .actions
            .as_ref()
            .is_some_and(|v| !v.is_array() && !v.is_object())
            || self.blueprint.as_ref().is_some_and(|v| !v.is_object())
        {
            return Err(invalid(
                "candidate actions must be a JSON array or workflow object; blueprint must be an object",
            ));
        }
        Ok(())
    }

    /// Normalize through the same versioned source parsers used by normal
    /// planning. Owned continuation and recovery sources require their own
    /// session workflow and cannot be fabricated by a snapshot-only comparison.
    fn source(&self) -> Result<PlanSource> {
        self.validate()?;
        if let Some(paused_target) = self.paused_target {
            return Ok(PlanSource::Pause {
                summary: self.summary.clone().unwrap_or_else(|| {
                    if paused_target {
                        "pause the simulation"
                    } else {
                        "unpause the simulation"
                    }
                    .to_owned()
                }),
                paused_target,
            });
        }
        if let Some(actions) = &self.actions {
            let raw = actions.to_string();
            return Ok(PlanSource::Actions {
                summary: self
                    .summary
                    .clone()
                    .unwrap_or_else(|| super::describe_actions(&raw)),
                raw,
            });
        }
        let blueprint = self
            .blueprint
            .as_ref()
            .ok_or_else(|| invalid("candidate source is missing"))?;
        let raw = blueprint.to_string();
        if matches!(
            blueprint["template"].as_str(),
            Some("compare_candidates" | "select_candidate" | "continue_goal" | "civilian_release")
        ) {
            return Err(invalid(
                "nested comparisons, selections, retained-goal continuation and civilian release are not snapshot-only candidate sources",
            ));
        }
        if crate::lab_world::is_production_objective(&raw) {
            let request = crate::lab_world::ProductionRequest::parse_current(&raw)?;
            return Ok(PlanSource::Production {
                summary: self
                    .summary
                    .clone()
                    .unwrap_or_else(|| "meet production quotas".to_owned()),
                raw: request.canonical_json(),
            });
        }
        let summary = self.summary.clone().unwrap_or_default();
        if super::connected_room::is_connected_room(&raw) {
            let request = super::connected_room::ConnectedRoomRequest::parse_current(&raw)?;
            return Ok(PlanSource::ConnectedRoom {
                summary,
                raw: request.canonical_json(),
            });
        }
        let raw = if super::resource_acquisition::is_request(&raw) {
            super::resource_acquisition::ResourceAcquisitionRequest::parse_current(&raw)?
                .canonical_json()
        } else if super::room_provisioning::is_provisioned_room(&raw) {
            super::room_provisioning::ProvisionedRoomRequest::parse_current(&raw)?.canonical_json()
        } else {
            raw
        };
        Ok(PlanSource::Blueprint { summary, raw })
    }

    fn normalized(&self, source: &PlanSource) -> Result<Self> {
        let mut candidate = Self {
            key: self.key.clone(),
            summary: None,
            actions: None,
            blueprint: None,
            paused_target: None,
        };
        match source {
            PlanSource::Pause {
                summary,
                paused_target,
            } => {
                candidate.summary = Some(summary.clone());
                candidate.paused_target = Some(*paused_target);
            }
            PlanSource::Actions { summary, raw } => {
                candidate.summary = Some(summary.clone());
                candidate.actions =
                    Some(serde_json::from_str(raw).map_err(|e| invalid(e.to_string()))?);
            }
            PlanSource::Blueprint { summary, raw }
            | PlanSource::Production { summary, raw }
            | PlanSource::ConnectedRoom { summary, raw } => {
                candidate.summary = Some(summary.clone());
                candidate.blueprint =
                    Some(serde_json::from_str(raw).map_err(|e| invalid(e.to_string()))?);
            }
            _ => {
                return Err(invalid(
                    "candidate normalization encountered an owned-history source",
                ));
            }
        }
        Ok(candidate)
    }
}

fn authorize(snapshot: &WorldSnapshot, context: &OperationContext) -> Result<()> {
    for capability in [Capability::Observe, Capability::Plan, Capability::Query] {
        context.authorize(capability, RiskTier::ReadOnly, &[], None)?;
    }
    if snapshot.anchor() != context.anchor {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "candidate planning requires the exact current source anchor",
        )
        .retryable(true));
    }
    Ok(())
}

fn check_wall(context: &OperationContext, started: Instant) -> Result<()> {
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(exhausted(
            "candidate comparison exceeded its aggregate wall-time allowance",
        ));
    }
    // Cancellation and grant expiry are current-context authority, including
    // the intervals between otherwise pure compilers and forecasts.
    context.authorize(Capability::Plan, RiskTier::ReadOnly, &[], None)
}

fn candidate_digest(snapshot: &WorldSnapshot, candidate: &Candidate) -> Result<String> {
    let source = json!({
        "schema": "dfmcp.lab-plan-candidate/1",
        "anchor": super::anchor_json(&snapshot.anchor()),
        "candidate": candidate,
    });
    Ok(
        Digest32::of_bytes(&serde_json::to_vec(&source).map_err(|e| invalid(e.to_string()))?)
            .to_hex(),
    )
}

/// Check source freshness and content identity, then return an original source
/// for normal preparation under current authority. The digest is an integrity
/// label, never proof that a previous comparison happened or a commit permit.
pub(super) fn selected_source(
    request: &Request,
    current: &WorldSnapshot,
    context: &OperationContext,
) -> Result<Option<PlanSource>> {
    let Request::Select(selection) = request else {
        return Ok(None);
    };
    authorize(current, context)?;
    let expected =
        serde_json::to_value(&selection.expected_anchor).map_err(|e| invalid(e.to_string()))?;
    if expected != super::anchor_json(&current.anchor()) {
        return Err(DfmcpError::new(
            ErrorCode::StaleAnchor,
            "selected candidate's exact source anchor is no longer current; compare again",
        )
        .retryable(true));
    }
    let source = selection.candidate.source()?;
    let normalized = selection.candidate.normalized(&source)?;
    if serde_json::to_value(&selection.candidate).map_err(|e| invalid(e.to_string()))?
        != serde_json::to_value(&normalized).map_err(|e| invalid(e.to_string()))?
        || candidate_digest(current, &normalized)? != selection.candidate_digest
    {
        return Err(invalid(
            "selected candidate is not the exact normalized source bound by its digest",
        ));
    }
    Ok(Some(source))
}

struct Compiled {
    candidate: Candidate,
    intent: Intent,
    reserve_until: Option<u64>,
    resource_room_request: Option<String>,
}

fn room_goal_scope(intent: &Intent) -> &'static str {
    match classify_room_stage(intent) {
        RoomCompileStage::ResourceAcquisition => "resource_acquisition_stage",
        RoomCompileStage::FurnishRoom => "reviewed_room_pursuit",
    }
}

fn reserve_until(source: &PlanSource) -> Result<Option<u64>> {
    match source {
        PlanSource::Production { raw, .. } => crate::lab_world::ProductionRequest::parse(raw)
            .map(|request| request.reserve_until_tick()),
        _ => Ok(None),
    }
}

fn unavailable(key: &str, stage: &str, error: &DfmcpError) -> Value {
    json!({
        "key": key,
        "status": "unavailable",
        "stage": stage,
        "epistemic_state": "predicted",
        "reason": {"code": error.code.as_str(), "message": error.message, "retryable": error.retryable},
        "selection_available": false,
    })
}

fn selection_step(
    snapshot: &WorldSnapshot,
    context: &OperationContext,
    candidate: &Candidate,
) -> Result<Value> {
    let wrapper = json!({
        "template": "select_candidate", "version": 1,
        "expected_anchor": super::anchor_json(&snapshot.anchor()),
        "candidate": candidate,
        "candidate_digest": candidate_digest(snapshot, candidate)?,
    });
    Ok(json!({
        "tool": "fortress.plan",
        "arguments": {"session_id": context.session_id.to_string(), "blueprint": wrapper.to_string()},
        "purpose": "revalidate this exact source and inspect a newly sealed plan before any commit",
    }))
}

/// Keep every decision-bearing outcome, including a separately requested reserve
/// frontier. Large per-step/model diagnostics belong to the selected ordinary
/// plan; they do not need to be repeated for every alternative.
pub(super) fn compact_prediction(prediction: &Value) -> Value {
    let mut compact = serde_json::Map::new();
    for key in [
        "epistemic_state",
        "available",
        "reason",
        "from_tick",
        "horizon_tick",
        "predicted_complete",
        "predicted_completion_tick",
        "predicted_actions_complete",
        "predicted_actions_completion_tick",
        "predicted_goal_complete",
        "predicted_goal_truth",
        "predicted_physical_quiescent",
        "predicted_frontier_tick",
        "resolution_ticks",
        "sampling_policy",
        "simulation_slices",
        "blocked_by_pause",
        "simulation_horizon_tick",
        "simulation_horizon_reached",
        "simulation_horizon_truncated",
        "reserve_forecast",
    ] {
        if let Some(value) = prediction.get(key) {
            compact.insert(key.to_owned(), value.clone());
        }
    }
    Value::Object(compact)
}

/// Compile every alternative from one admitted source, establish comparable
/// original goals, and forecast sequentially under one aggregate allowance.
/// Any whole-set refusal leaves all canonical and retained plan state untouched.
pub(super) fn compare(
    adapter: &MemoryAdapter,
    context: &OperationContext,
    _intent_id: IntentId,
    request: &Request,
    started: Instant,
) -> Result<Value> {
    let Request::Compare(comparison) = request else {
        return Err(invalid("comparison execution requires compare_candidates"));
    };
    let snapshot = adapter.snapshot();
    authorize(snapshot, context)?;
    check_wall(context, started)?;
    let mut forecast_budget = ForecastBudget::new(context, started, comparison.horizon_ticks)?;
    let source_work = forecast_budget.admit_source(snapshot, context)?;
    let mut candidates: Vec<_> = comparison.candidates.iter().collect();
    candidates.sort_by(|left, right| left.key.cmp(&right.key));
    // These identities live only on discarded forks. The same source and
    // candidate set get the same identity regardless of caller request counter
    // or candidate order; ordinary selection still receives a fresh real ID.
    // One common ID also preserves equality of action-derived original goals.
    let identity = json!({
        "schema": "dfmcp.lab-plan-comparison-fork/1",
        "anchor": super::anchor_json(&snapshot.anchor()),
        "candidates": candidates,
    });
    let intent_id = IntentId::new(
        Digest32::of_bytes(identity.to_string().as_bytes())
            .first_u128()
            .max(1),
    );
    let mut rows = Vec::with_capacity(candidates.len());
    let mut compiled = Vec::with_capacity(candidates.len());
    let mut common_goal: Option<Predicate> = None;
    let mut common_reserve: Option<Option<u64>> = None;
    let mut common_room: Option<Option<String>> = None;
    let mut total_actions = 0_usize;
    for candidate in candidates {
        check_wall(context, started)?;
        let result = candidate.source().and_then(|source| {
            let normalized = candidate.normalized(&source)?;
            let reserve_until = reserve_until(&source)?;
            // Equal WOOD acquisition terminals do not establish equal room
            // projects. Keep the full canonical room/access intent in the
            // comparison contract before either candidate forecast is run.
            let resource_room_request = match &source {
                PlanSource::Blueprint { raw, .. } | PlanSource::ConnectedRoom { raw, .. }
                    if source.is_resource_aware_room() =>
                {
                    Some(raw.clone())
                }
                _ => None,
            };
            let (intent, _) = source.compile(intent_id, snapshot)?;
            Ok(Compiled {
                candidate: normalized,
                intent,
                reserve_until,
                resource_room_request,
            })
        });
        let value = match result {
            Ok(value) => value,
            Err(error) => {
                rows.push(unavailable(&candidate.key, "compile", &error));
                continue;
            }
        };
        let goal = value.intent.terminal_condition.normalized();
        if common_room
            .as_ref()
            .is_some_and(|existing| existing != &value.resource_room_request)
        {
            return Err(invalid(
                "resource-aware room candidates must retain the same complete original room and access request; matching acquisition targets alone are not the same project",
            ));
        }
        if common_goal
            .as_ref()
            .is_some_and(|existing| existing != &goal)
            || common_reserve.is_some_and(|existing| existing != value.reserve_until)
        {
            return Err(invalid(
                "all successfully compiled candidates must have the same normalized original terminal predicate and reserve frontier",
            ));
        }
        common_goal = Some(goal);
        common_reserve = Some(value.reserve_until);
        common_room = Some(value.resource_room_request.clone());
        total_actions = total_actions.saturating_add(value.intent.requested_actions.len());
        if total_actions > context.budget.max_actions as usize {
            return Err(exhausted(
                "candidate compilation exceeds the aggregate action allowance",
            ));
        }
        compiled.push(value);
    }
    // Preserve the complete original room separately from the current stage's
    // terminal. A successful finite harvest forecast cannot imply that the
    // room, furnishings or access route have been built on the fork.
    let resource_room_scope = compiled
        .first()
        .filter(|candidate| candidate.resource_room_request.is_some())
        .map(|candidate| room_goal_scope(&candidate.intent));
    // Establish the entire common objective before a single candidate fork.
    let mut prepared: Vec<(Compiled, PreparedPlan)> = Vec::with_capacity(compiled.len());
    for candidate in compiled {
        check_wall(context, started)?;
        match StaticPlanner::default().prepare_laboratory(snapshot, &candidate.intent, context) {
            Ok(plan) => prepared.push((candidate, plan)),
            Err(error) => rows.push(unavailable(&candidate.candidate.key, "prepare", &error)),
        }
    }
    let mut ranking = Vec::new();
    let mut evidence_complete = rows.is_empty();
    for (candidate, plan) in prepared {
        check_wall(context, started)?;
        // The bounded evaluator returns ordinary model infeasibility as an
        // available:false prediction. A Result error is an aggregate budget,
        // cancellation, source or authority failure and aborts the whole set.
        let prediction = forecast_plan_bounded(
            adapter,
            &plan,
            context,
            candidate.reserve_until,
            &mut forecast_budget,
        )?;
        let available = prediction["available"] == true;
        let reserve = &prediction["reserve_forecast"];
        let reserve_known = candidate.reserve_until.is_none()
            || (reserve["available"] == true
                && matches!(
                    reserve["predicted_goal_truth"].as_str(),
                    Some("true" | "false")
                ));
        let reserve_complete = candidate.reserve_until.is_none()
            || (reserve["available"] == true && reserve["predicted_goal_truth"] == "true");
        let complete = available && prediction["predicted_complete"] == true && reserve_complete;
        let known_goal = matches!(
            prediction["predicted_goal_truth"].as_str(),
            Some("true" | "false")
        );
        evidence_complete &= available && known_goal && reserve_known;
        let completion_tick = prediction["predicted_completion_tick"].as_u64();
        if complete && let Some(completion_tick) = completion_tick {
            ranking.push((
                plan.max_risk,
                plan.requires_checkpoint,
                completion_tick,
                plan.steps.len(),
                candidate.candidate.key.clone(),
            ));
        }
        let mut row = json!({
            "key": candidate.candidate.key,
            "status": if available { "evaluated" } else { "unavailable" },
            "epistemic_state": "predicted",
            "candidate_digest": candidate_digest(snapshot, &candidate.candidate)?,
            "summary": plan.summary,
            "max_risk": plan.max_risk.as_str(),
            "requires_checkpoint": plan.requires_checkpoint,
            "action_count": plan.steps.len(),
            "required_capabilities": plan.required_capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "prediction": compact_prediction(&prediction),
            "selection_available": true,
            "next_step": selection_step(snapshot, context, &candidate.candidate)?,
        });
        if candidate.resource_room_request.is_some() {
            row["goal_scope"] = json!(room_goal_scope(&candidate.intent));
            row["forecast_scope"] = json!("current_reviewed_stage_only");
        }
        rows.push(row);
    }
    check_wall(context, started)?;
    rows.sort_by(|left, right| left["key"].as_str().cmp(&right["key"].as_str()));
    ranking.sort();
    let recommendation = evidence_complete
        .then(|| ranking.first().map(|row| row.4.clone()))
        .flatten();
    let reason = if !evidence_complete {
        "at least one candidate lacks a complete comparable prediction; no recommendation is justified"
    } else if recommendation.is_none() {
        "no candidate predicts the original goal, verified actions and physical quiescence together within the common horizon"
    } else {
        "among complete predictions, minimize risk, checkpoint burden, completion tick, action count, then lexical key"
    };
    let goal_digest = common_goal
        .as_ref()
        .map(|goal| Digest32::of_bytes(&goal.canonical_bytes()).to_hex());
    let mut result = json!({
        "ok": true,
        "schema": "dfmcp.plan-comparison/1",
        "kind": "compare_candidates",
        "mode": "compare_candidates",
        "version": 1,
        "session_id": context.session_id.to_string(),
        "anchor": super::anchor_json(&snapshot.anchor()),
        "epistemic_state": "predicted",
        "method": "same_original_goal_on_discarded_laboratory_forks",
        "assumptions": "no other agent acts; deterministic laboratory effects; only this candidate's deferred chain dispatches at the common cadence while physical world jobs advance; other retained plans' deferred actions do not dispatch; predictions are not effect evidence",
        "horizon_ticks": comparison.horizon_ticks,
        "original_terminal_condition": common_goal.as_ref().map(|goal| format!("{goal:?}")),
        "original_terminal_digest": goal_digest,
        "reserve_until_tick": common_reserve.flatten(),
        "candidates": rows,
        "recommended_candidate": recommendation,
        "recommendation": {"policy": POLICY, "reason": reason, "evidence_complete": evidence_complete},
        "source_work": source_work,
        "compiled_actions": total_actions,
        "forecast_work": forecast_budget.work_json(),
        "effect_evidence": false,
        "pending_plan_changed": false,
        "note": "predictions assume no other agent acts; selection creates a newly reviewed plan under current authority and never commits it",
    });
    if let Some(raw) = common_room.flatten() {
        result["semantic_room_request"] = serde_json::from_str(&raw).map_err(|error| {
            DfmcpError::new(
                ErrorCode::InternalInvariantViolation,
                format!("canonical resource room comparison source could not be decoded: {error}"),
            )
        })?;
        result["goal_scope"] = json!(resource_room_scope);
        result["forecast_scope"] = json!("current_reviewed_stage_only");
        result["recommendation_scope"] = json!("current_reviewed_stage_only");
        result["method"] =
            json!("same_original_room_request_current_stage_on_discarded_laboratory_forks");
        result["recommendation"]["reason"] = json!(if !evidence_complete {
            "at least one candidate lacks a complete comparable stage prediction; no recommendation is justified"
        } else if result["recommended_candidate"].is_null() {
            "no candidate predicts the reviewed stage goal, verified actions and physical quiescence together within the common horizon"
        } else {
            "among complete reviewed-stage predictions, minimize risk, checkpoint burden, completion tick, action count, then lexical key"
        });
        result["note"] = json!(
            "predictions cover only the currently reviewed stage; an acquisition forecast does not include later room or access work; selection prepares a new plan under current authority and never commits it"
        );
    }
    Ok(result)
}

// serde_json::Value accepts repeated object keys. A source candidate must retain
// one unambiguous meaning through normalization, digesting and later selection.
struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = UniqueValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON with distinct object keys")
            }
            fn visit_bool<E: serde::de::Error>(
                self,
                value: bool,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Bool(value)))
            }
            fn visit_i64<E: serde::de::Error>(
                self,
                value: i64,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_u64<E: serde::de::Error>(
                self,
                value: u64,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(value.into()))
            }
            fn visit_f64<E: serde::de::Error>(
                self,
                value: f64,
            ) -> std::result::Result<Self::Value, E> {
                serde_json::Number::from_f64(value)
                    .map(Value::Number)
                    .map(UniqueValue)
                    .ok_or_else(|| E::custom("nonfinite JSON number"))
            }
            fn visit_str<E: serde::de::Error>(
                self,
                value: &str,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value.to_owned())))
            }
            fn visit_string<E: serde::de::Error>(
                self,
                value: String,
            ) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::String(value)))
            }
            fn visit_none<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_unit<E: serde::de::Error>(self) -> std::result::Result<Self::Value, E> {
                Ok(UniqueValue(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(value) = seq.next_element::<UniqueValue>()? {
                    values.push(value.0);
                }
                Ok(UniqueValue(Value::Array(values)))
            }
            fn visit_map<A: MapAccess<'de>>(
                self,
                mut map: A,
            ) -> std::result::Result<Self::Value, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some((key, value)) = map.next_entry::<String, UniqueValue>()? {
                    if values.insert(key, value.0).is_some() {
                        return Err(serde::de::Error::custom("duplicate JSON object key"));
                    }
                }
                Ok(UniqueValue(Value::Object(values)))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

#[cfg(test)]
#[path = "lab_plan_comparison_tests.rs"]
mod tests;
