//! Stateless DfQL over the laboratory session's exact current or retained world.
//! Current grants authorize a historical read; historical ticks never renew them.

// Reuse the same pure executors as the live read surfaces, without instantiating
// their watch, baseline, archive, or native-observation ownership machinery.
#[path = "semantic_query_core.rs"]
#[allow(clippy::duplicate_mod)]
mod core_query;
#[path = "operational_query.rs"]
#[allow(clippy::duplicate_mod)]
mod operational_query;

use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier};
use dfmcp_world::WorldSnapshot;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_REQUEST_BYTES: usize = 65_536;

#[derive(Deserialize)]
struct Selector {
    mode: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Structured,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Request {
    mode: Mode,
    request: Value,
    pub(super) at: Option<String>,
}

fn invalid(message: impl Into<String>) -> DfmcpError {
    DfmcpError::new(ErrorCode::InvalidRequest, message)
}

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

/// The original bytes are decoded again after selection, so duplicate wrapper
/// fields and unknown wrapper arguments cannot disappear in a Value round trip.
pub(super) fn parse(raw: &str) -> Result<Option<Request>> {
    if raw.len() > MAX_REQUEST_BYTES {
        return Err(exhausted("laboratory query exceeds its request byte bound"));
    }
    if !raw.trim_start().starts_with('{') {
        return Ok(None);
    }
    let selector: Selector = serde_json::from_str(raw).map_err(|error| {
        if raw.len() > crate::lab_world::MAX_QUERY_JSON_BYTES {
            exhausted("query mode exceeds its explicit byte bound")
        } else {
            invalid(format!("query requires one unambiguous mode: {error}"))
        }
    })?;
    if selector.mode != "structured" {
        return Ok(None);
    }
    let request: Request = serde_json::from_str(raw)
        .map_err(|error| invalid(format!("invalid structured laboratory query: {error}")))?;
    let Mode::Structured = request.mode;
    let envelope = request
        .request
        .as_object()
        .ok_or_else(|| invalid("structured request must be a dfmcp.query/1 object"))?;
    if request.request["schema"] != "dfmcp.query/1"
        || envelope
            .keys()
            .any(|key| !matches!(key.as_str(), "schema" | "query" | "expected_anchor"))
    {
        return Err(invalid(
            "structured request requires the closed dfmcp.query/1 envelope",
        ));
    }
    if let Some(hash) = &request.at
        && (hash.len() != 64
            || !hash
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        return Err(invalid("at must be a 64-digit lowercase state hash"));
    }
    if !matches!(
        request.request["query"]["kind"].as_str(),
        Some("entities" | "inspect" | "traverse" | "dependencies" | "aggregate" | "search")
    ) {
        return Err(invalid(
            "structured laboratory queries support only entities, inspect, traverse, dependencies, aggregate, and search",
        ));
    }
    Ok(Some(request))
}

/// `current` is the session's actual current context, even when `snapshot` is a
/// retained source. Reauthorize before replacing the evaluation anchor.
pub(super) fn execute(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
) -> Result<Value> {
    let started = Instant::now();
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if snapshot.fortress_id != current.anchor.fortress_id {
        return Err(invalid("retained query source belongs to another fortress"));
    }
    // Both executors verify the entire canonical source. A small page must not
    // bypass the work needed to verify chunks, events, or graph relationships.
    let records = snapshot
        .graph
        .entities
        .len()
        .saturating_add(snapshot.graph.edges.len())
        .saturating_add(snapshot.graph.chunks.len())
        .saturating_add(snapshot.graph.events.len());
    if records as u64 > u64::from(current.budget.max_entities) {
        return Err(exhausted(
            "structured query exceeds its source record budget",
        ));
    }
    let source_bytes = snapshot.canonical_bytes().len();
    if source_bytes as u64 > current.budget.max_bytes {
        return Err(exhausted(
            "structured query exceeds its canonical source byte budget",
        ));
    }
    let mut source_context = current.clone();
    source_context.anchor = snapshot.anchor();
    let mut result = match request.request["query"]["kind"].as_str() {
        Some("aggregate" | "search") => {
            operational_query::execute(snapshot, &source_context, &request.request)?
        }
        _ => core_query::execute(snapshot, &source_context, &request.request)?,
    };
    if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(exhausted("structured query exceeded its wall-time budget"));
    }
    let historical = snapshot.anchor() != current.anchor;
    result["mode"] = json!("structured");
    result["ok"] = json!(true);
    result["session_id"] = json!(current.session_id.to_string());
    result["game_tick"] = json!(snapshot.tick.0);
    result["current_anchor"] = core_query::anchor_json(current.anchor);
    result["historical"] = json!(historical);
    result["source_work"] = json!({"records": records, "canonical_bytes": source_bytes});
    result["evidence_scope"] = json!("laboratory_reference_world");
    result["observation_coverage"] = json!({
        "status": "partial",
        "complete_domains": ["laboratory.source_anchor"],
        "partial_domains": [{"domain": "laboratory.structured_query", "kind": result["kind"],
            "reason": "results describe the selected canonical projection, not complete-world absence"}],
        "absence_proof_scope": [],
        "continuation": result["continuation"],
        "query_coverage": result["coverage"],
        "historical": historical,
        "current_anchor": result["current_anchor"],
        "eligible_as_mutation_evidence": false,
    });
    Ok(result)
}

/// Carry the authority-bearing budget to final presentation. In particular, the
/// facade must not use its cached opening budget after an authorized narrowing.
pub(super) fn response(result: Result<Value>, context: &OperationContext) -> String {
    let mut payload = match result {
        Ok(payload) => payload,
        Err(error) => json!({"ok": false, "error": {
            "operation": "fortress.query", "code": error.code.as_str(),
            "message": error.message, "retryable": error.retryable,
            "details": error.details, "effect_may_have_occurred": false,
        }}),
    };
    payload["mode"] = json!("structured");
    payload["query_budget"] = json!({
        "maximum_response_bytes": context.budget.max_bytes.min(
            u64::from(context.budget.max_output_tokens).saturating_mul(4)),
        "max_output_tokens": context.budget.max_output_tokens,
        "max_wall_millis": context.budget.max_wall_millis,
    });
    payload.to_string()
}

/// A source-bound cursor describes exactly the rows produced by the executor.
/// Generic presentation clipping cannot remove rows and keep that cursor. Keep
/// only lossless full/compact-turn results, or publish a typed refusal instead.
pub(crate) fn finish_response(response: String, elapsed: Duration) -> String {
    let finishing = Instant::now();
    let Ok(payload) = serde_json::from_str::<Value>(&response) else {
        return response;
    };
    if payload["mode"] != "structured" || !payload["query_budget"].is_object() {
        return response;
    }
    let maximum_bytes = payload["query_budget"]["maximum_response_bytes"]
        .as_u64()
        .map_or(0, |value| value);
    let maximum_tokens = payload["query_budget"]["max_output_tokens"]
        .as_u64()
        .map_or(0, |value| value)
        .min(maximum_bytes / 4);
    let fitted = crate::output_budget::fit(&response, maximum_tokens);
    if payload["ok"] != true {
        return fitted;
    }
    let complete = serde_json::from_str::<Value>(&fitted).is_ok_and(|value| {
        matches!(
            value["output_budget"]["tier"].as_str(),
            Some("full" | "compact_turn")
        )
    });
    let wall_expired = elapsed.saturating_add(finishing.elapsed())
        >= Duration::from_millis(
            payload["query_budget"]["max_wall_millis"]
                .as_u64()
                .map_or(0, |value| value),
        );
    if !wall_expired && fitted.len() as u64 <= maximum_bytes && complete {
        return fitted;
    }
    let mut turn = payload["agent_turn"].clone();
    turn["coverage"] = json!({"status": "partial", "complete_domains": [],
        "partial_domains": [], "absence_proof_scope": [], "continuation": null,
        "reason": "structured query result was not published"});
    turn["continuity"]["status"] = json!("stale");
    let message = if wall_expired {
        "structured query and its Agent Turn exceeded the negotiated wall-time budget"
    } else {
        "the complete structured query and Agent Turn exceed the response budget; reduce limit, fields, metrics, or graph scope"
    };
    let refusal = json!({"ok": false, "mode": "structured",
        "session_id": payload["session_id"], "query_budget": payload["query_budget"],
        "error": {"operation": "fortress.query", "code": "budget_exceeded",
            "message": message, "retryable": false, "details": [],
            "effect_may_have_occurred": false,
            "recovery": {"class": "never_unchanged", "next_tool": "fortress.query",
                "reason": "narrow the original structured request or negotiate a larger budget"}},
        "agent_turn": turn});
    crate::output_budget::fit(&refusal.to_string(), maximum_tokens)
}
