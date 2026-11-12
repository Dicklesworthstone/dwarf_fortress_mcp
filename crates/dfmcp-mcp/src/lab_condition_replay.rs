//! Retrospective sampled conditions over an exact retained laboratory interval.
//!
//! The session owner supplies immutable versions. Every recorded version must
//! still exist, and the complete interval is admitted before the ordinary watch
//! machine evaluates it. No watch, baseline, observation or game work is created.

use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::{PredicateEvidence, WorldSnapshot, retention::VersionRetention};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::query_condition::Condition;

const MAX_REQUEST_BYTES: usize = 65_536;
const MAX_VERSIONS: usize = 32;

#[derive(Deserialize)]
struct Selector {
    mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    ConditionReplay,
}

#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Detail {
    #[default]
    Summary,
    Evidence,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    condition: Condition,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_condition: Option<Condition>,
    deadline_tick: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    poll_interval_ticks: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stable_observations: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    mode: Mode,
    from: String,
    to: Option<String>,
    definition: Definition,
    #[serde(default)]
    detail: Detail,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

fn stale(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::StaleAnchor, message)
}

fn encoding_error() -> DfmcpError {
    DfmcpError::new(
        ErrorCode::InternalInvariantViolation,
        "laboratory condition replay cannot be encoded",
    )
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Decode original bytes using the laboratory condition grammar. Duplicate
/// fields, native-only predicates and caller-selected trust policies refuse.
pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err(exhausted("laboratory query exceeds its request byte bound"));
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(None);
    }
    let selector: Selector = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("query requires one unambiguous mode: {error}")))?;
    if selector.mode != "condition_replay" {
        return Ok(None);
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid laboratory condition replay: {error}")))?;
    let Mode::ConditionReplay = request.mode;
    if !valid_hash(&request.from) || request.to.as_deref().is_some_and(|hash| !valid_hash(hash)) {
        return Err(invalid(
            "condition replay endpoints must be exact 64-digit lowercase state hashes",
        ));
    }
    Ok(Some(request))
}

fn remaining(current: &OperationContext, started: Instant) -> Result<OperationContext> {
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(exhausted(
            "condition replay exhausted its shared wall-time budget",
        ));
    }
    let mut context = current.clone();
    context.budget.max_wall_millis -= elapsed.as_millis() as u64;
    Ok(context)
}

fn validate_lineage(source: StateAnchor, current: StateAnchor) -> Result<()> {
    if source.fortress_id != current.fortress_id
        || source.cursor.epoch != current.cursor.epoch
        || source.cursor.sequence > current.cursor.sequence
        || source.tick > current.tick
        || (source.cursor == current.cursor && source != current)
    {
        return Err(stale(
            "condition replay requires the current fortress and epoch without future or forked observations",
        ));
    }
    Ok(())
}

/// The caller owns the session throughout this read. The current world is used
/// only for current authority and endpoint resolution; no missing historical
/// version, intermediate game tick or newly encountered shared frontier is
/// manufactured to complete a requested interval.
pub(super) fn execute(
    history: &VersionRetention,
    current_world: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
) -> Result<Value> {
    let started = Instant::now();
    remaining(current, started)?;
    if current_world.anchor() != current.anchor {
        return Err(stale("condition replay lost its current owned world"));
    }
    let from = history.find_hex(&request.from).ok_or_else(|| {
        stale("condition replay start was never retained by this session or its history was collected")
    })?;
    let target_hash = request
        .to
        .clone()
        .unwrap_or_else(|| current.anchor.state_hash.to_string());
    let to = history.find_hex(&target_hash).ok_or_else(|| {
        stale("condition replay end is not retained by this session; observe the current shared frontier first or choose an exact retained endpoint")
    })?;
    let sources = history.consecutive_range(&from, &to, MAX_VERSIONS)?;
    let first = sources.first().ok_or_else(encoding_error)?.anchor();
    let target = sources.last().ok_or_else(encoding_error)?.anchor();
    if request.from != first.state_hash.to_string()
        || target_hash != target.state_hash.to_string()
        || (request.to.is_none() && target != current.anchor)
    {
        return Err(stale(
            "condition replay endpoints differ from their exact requested worlds",
        ));
    }
    for source in &sources {
        validate_lineage(source.anchor(), current.anchor)?;
    }
    let mut cursor_gaps = 0usize;
    for pair in sources.windows(2) {
        let before = pair[0].anchor();
        let after = pair[1].anchor();
        if after.tick < before.tick || after.cursor.sequence <= before.cursor.sequence {
            return Err(stale(
                "condition replay observations do not advance in original cursor order",
            ));
        }
        if after.cursor.sequence > before.cursor.sequence.saturating_add(1) {
            cursor_gaps += 1;
        }
    }
    // Full interval admission precedes all predicate evaluation, even when the
    // first sample alone would make the monitor terminal. No early success can
    // hide an oversized, corrupt or noncanonical later selected source.
    let source_work = super::query_budget::admit_sources(&sources, &remaining(current, started)?)?;
    for source in &sources {
        PredicateEvidence::laboratory(source)?;
        remaining(current, started)?;
    }
    let anchors: Vec<_> = sources
        .iter()
        .map(|source| super::anchor_json(&source.anchor()))
        .collect();
    let binding = Digest32::of_bytes(
        &serde_json::to_vec(&json!({
            "domain":"dfmcp-laboratory-condition-replay-range/1",
            "anchors":anchors,
        }))
        .map_err(|_| encoding_error())?,
    );
    let mut definition = serde_json::to_value(&request.definition).map_err(|_| encoding_error())?;
    definition["poll_interval_ticks"] = json!(request.definition.poll_interval_ticks.unwrap_or(1));
    definition["stable_observations"] = json!(request.definition.stable_observations.unwrap_or(2));
    let mut replay = super::semantic_query::laboratory_condition_replay(
        &definition,
        first,
        sources.len() as u64,
        binding,
        &remaining(current, started)?,
    )?;
    for (index, source) in sources.iter().enumerate() {
        replay.advance(index as u64 + 1, source, &remaining(current, started)?)?;
    }
    let mut result = replay.finish(
        matches!(request.detail, Detail::Evidence),
        &remaining(current, started)?,
    )?;
    result["ok"] = json!(true);
    result["mode"] = json!("condition_replay");
    result["session_id"] = json!(current.session_id.to_string());
    result["game_tick"] = json!(target.tick.0);
    result["anchor"] = super::anchor_json(&target);
    result["current_anchor"] = super::anchor_json(&current.anchor);
    result["definition"] = definition;
    result["source_work"] = source_work;
    result["evidence_scope"] = json!("laboratory_reference_world");
    result["eligible_as_mutation_evidence"] = json!(false);
    result["advances_game"] = json!(false);
    result["range"] = json!({
        "from":first.state_hash.to_string(), "to":target.state_hash.to_string(),
        "binding":binding.to_string(), "records":sources.len(),
        "consecutive_retained_versions":true, "observation_cursor_gaps":cursor_gaps,
        "current_endpoint":target == current.anchor, "anchors":anchors,
        "record_numbers":"one-based positions within this exact selected interval",
    });
    result["observation_coverage"] = json!({
        "status":"partial", "complete_domains":["laboratory.selected_retained_versions"],
        "partial_domains":[{"domain":"laboratory.sampled_condition_history",
            "reason":"only the exact retained versions are evaluated; cursor gaps reset stability and unrecorded observations are never reconstructed"}],
        "absence_proof_scope":[], "continuation":null,
        "temporal_coverage":"selected_retained_observations_only",
        "continuous_between_observations":false, "historical":true,
        "current_anchor":super::anchor_json(&current.anchor),
        "eligible_as_mutation_evidence":false,
    });
    let maximum = current
        .budget
        .max_bytes
        .min(u64::from(current.budget.max_output_tokens) * 4);
    if serde_json::to_vec(&result)
        .map_err(|_| encoding_error())?
        .len() as u64
        > maximum
    {
        return Err(exhausted(
            "complete condition replay evidence exceeds its response allowance",
        ));
    }
    remaining(current, started)?;
    Ok(result)
}

#[cfg(test)]
#[path = "lab_condition_replay_tests.rs"]
mod tests;
