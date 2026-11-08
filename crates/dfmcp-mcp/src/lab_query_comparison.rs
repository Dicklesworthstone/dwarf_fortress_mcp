//! Compare one complete entity selection at two exact retained laboratory worlds.
//! The session owner resolves the worlds; this module never creates retained work.

use std::time::{Duration, Instant};

use dfmcp_core::{
    Capability, DfmcpError, Digest32, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
use dfmcp_world::WorldSnapshot;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const MAX_REQUEST_BYTES: usize = 65_536;
const MAX_CONTINUATION_BYTES: usize = 128;

#[derive(Deserialize)]
struct Selector {
    mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    QueryChanges,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum SelectionKind {
    Entities,
}

/// Keep the shared entity/filter semantics, but reject query pagination and
/// duplicate selector fields while decoding the original request bytes.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Selection {
    kind: SelectionKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    kinds: Vec<String>,
    #[serde(rename = "where", skip_serializing_if = "Option::is_none")]
    predicate: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    fields: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    mode: Mode,
    pub(super) before: String,
    pub(super) after: Option<String>,
    select: Selection,
    limit: Option<u32>,
    continuation: Option<String>,
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
        "laboratory query comparison cannot be encoded",
    )
}

fn valid_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err(exhausted("laboratory query exceeds its request byte bound"));
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(None);
    }
    let selector: Selector = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("query requires one unambiguous mode: {error}")))?;
    if selector.mode != "query_changes" {
        return Ok(None);
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid laboratory query comparison: {error}")))?;
    let Mode::QueryChanges = request.mode;
    if !valid_hash(&request.before)
        || request
            .after
            .as_deref()
            .is_some_and(|hash| !valid_hash(hash))
    {
        return Err(invalid(
            "before and after must be exact 64-digit lowercase state hashes",
        ));
    }
    if request.limit.is_some_and(|limit| limit == 0 || limit > 128) {
        return Err(invalid("query_changes limit must be within 1..128"));
    }
    if request
        .continuation
        .as_ref()
        .is_some_and(|token| token.is_empty() || token.len() > MAX_CONTINUATION_BYTES)
    {
        return Err(invalid(
            "query_changes continuation must be a nonempty token of at most 128 bytes",
        ));
    }
    Ok(Some(request))
}

fn anchor_json(anchor: StateAnchor) -> Value {
    json!({"fortress_id": anchor.fortress_id.to_string(), "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence, "game_tick": anchor.tick.0,
        "state_hash": anchor.state_hash.to_string()})
}

/// An exact retained hash is a source selector, not permission to cross a restore
/// boundary, read a future world, or substitute another world at the same cursor.
fn validate_lineage(source: StateAnchor, current: StateAnchor) -> Result<()> {
    if source.fortress_id != current.fortress_id
        || source.cursor.epoch != current.cursor.epoch
        || source.cursor.sequence > current.cursor.sequence
        || source.tick > current.tick
        || (source.cursor == current.cursor && source != current)
    {
        return Err(stale(
            "query comparison endpoints must belong to the current fortress and epoch without future or forked anchors",
        ));
    }
    Ok(())
}

fn metadata(
    target: StateAnchor,
    current: &OperationContext,
    source_work: &Value,
    continuation: &Value,
) -> Value {
    let historical = target != current.anchor;
    json!({
        "mode": "query_changes", "ok": true,
        "session_id": current.session_id.to_string(), "game_tick": target.tick.0,
        "current_anchor": anchor_json(current.anchor), "historical": historical,
        "source_work": source_work, "evidence_scope": "laboratory_reference_world",
        "eligible_as_mutation_evidence": false,
        "observation_coverage": {
            "status": "partial", "complete_domains": ["laboratory.endpoint_anchors"],
            "partial_domains": [{"domain": "laboratory.selected_endpoint_changes",
                "reason": "only the selected endpoint projections are compared; intermediate changes and complete-world absence are not established"}],
            "absence_proof_scope": [], "continuation": continuation,
            "temporal_coverage": "endpoint_comparison_only",
            "intermediate_observations_evaluated": false,
            "historical": historical, "current_anchor": anchor_json(current.anchor),
            "eligible_as_mutation_evidence": false,
        },
    })
}

/// The enclosing runtime owns and resolves both exact sources under the current
/// session lock. Current authority is checked before examining either source and
/// is never replaced by the older source tick for the authorization decision.
pub(super) fn execute(
    before: &WorldSnapshot,
    after: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
) -> Result<Value> {
    let started = Instant::now();
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let basis = before.anchor();
    let target = after.anchor();
    if request.before != basis.state_hash.to_string()
        || request
            .after
            .as_ref()
            .is_some_and(|hash| *hash != target.state_hash.to_string())
        || (request.after.is_none() && target != current.anchor)
    {
        return Err(stale(
            "resolved query comparison sources do not match the requested exact hashes",
        ));
    }
    validate_lineage(basis, current.anchor)?;
    validate_lineage(target, current.anchor)?;
    let source_work = if basis == target {
        let work = super::query_budget::admit_sources(&[before], current)?;
        // The owner normally resolves this pair to one retained world. Bound
        // both actual inputs even if an internal caller supplies distinct values
        // claiming the same anchor; the shared engine verifies both hashes.
        super::query_budget::admit_sources(&[after], current)?;
        work
    } else {
        super::query_budget::admit_sources(&[before, after], current)?
    };
    let selection = serde_json::to_value(&request.select).map_err(|_| encoding_error())?;
    let binding = Digest32::of_bytes(
        &serde_json::to_vec(&json!({
            "domain": "dfmcp-laboratory-query-comparison/1",
            "basis": anchor_json(basis), "target": anchor_json(target),
        }))
        .map_err(|_| encoding_error())?,
    );

    // Reserve all wrapper fields and the longest permitted repeated cursor.
    // Only the output allowance is narrowed: distinct complete sources were
    // charged together against the original source byte and record budgets above.
    let maximum_bytes = current
        .budget
        .max_bytes
        .min(u64::from(current.budget.max_output_tokens).saturating_mul(4));
    let overhead = serde_json::to_vec(&metadata(
        target,
        current,
        &source_work,
        &json!("x".repeat(MAX_CONTINUATION_BYTES)),
    ))
    .map_err(|_| encoding_error())?
    .len() as u64;
    let output_bytes = maximum_bytes.checked_sub(overhead).ok_or_else(|| {
        exhausted("query comparison metadata exceeds the negotiated response budget")
    })?;
    let mut comparison_context = current.clone();
    comparison_context.budget.max_output_tokens = u32::try_from(output_bytes / 4)
        .map_err(|_| exhausted("query comparison output allowance overflow"))?;
    if comparison_context.budget.max_output_tokens == 0 {
        return Err(exhausted(
            "no query comparison result fits after reserving its source context",
        ));
    }
    let elapsed = started.elapsed().as_millis();
    if elapsed >= u128::from(current.budget.max_wall_millis) {
        return Err(exhausted("query comparison exceeded its wall-time budget"));
    }
    comparison_context.budget.max_wall_millis -= elapsed as u64;
    let mut result = super::semantic_query::compare_endpoints(
        before,
        after,
        &comparison_context,
        &selection,
        binding,
        request.limit,
        request.continuation.as_deref(),
    )?;
    let extra = metadata(target, current, &source_work, &result["continuation"]);
    let object = result.as_object_mut().ok_or_else(encoding_error)?;
    let extra = extra.as_object().ok_or_else(encoding_error)?;
    object.extend(
        extra
            .iter()
            .map(|(key, value)| (key.clone(), value.clone())),
    );
    if serde_json::to_vec(&result)
        .map_err(|_| encoding_error())?
        .len() as u64
        > maximum_bytes
    {
        return Err(exhausted(
            "the complete query comparison exceeds its response budget; narrow the selected fields or page limit",
        ));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(exhausted("query comparison exceeded its wall-time budget"));
    }
    Ok(result)
}

#[cfg(test)]
#[path = "lab_query_comparison_tests.rs"]
mod tests;
