//! Shared admission and complete-response budgets for every laboratory query.
//! Query syntax and a small page cannot bypass the work of its canonical source.

use std::time::{Duration, Instant};

use dfmcp_core::{Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier};
use dfmcp_world::WorldSnapshot;
use serde_json::{Value, json};

fn exhausted(message: &str) -> DfmcpError {
    DfmcpError::new(ErrorCode::BudgetExceeded, message)
}

/// Authorize at the actual current tick before inspecting selected sources.
/// Changes charge both distinct endpoints together; a page charges the complete
/// source, including records outside its selectors and presentation profile.
pub(super) fn admit_sources(
    sources: &[&WorldSnapshot],
    current: &OperationContext,
) -> Result<Value> {
    let started = Instant::now();
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mut records = 0_u64;
    for source in sources {
        if source.fortress_id != current.anchor.fortress_id {
            return Err(DfmcpError::new(
                ErrorCode::InvalidRequest,
                "retained query source belongs to another fortress",
            ));
        }
        for count in [
            source.graph.entities.len(),
            source.graph.edges.len(),
            source.graph.chunks.len(),
            source.graph.events.len(),
        ] {
            records = records.saturating_add(count as u64);
        }
    }
    if records > u64::from(current.budget.max_entities) {
        return Err(exhausted("query exceeds its complete source record budget"));
    }
    let mut canonical_bytes = 0_u64;
    for source in sources {
        canonical_bytes = canonical_bytes.saturating_add(source.canonical_bytes().len() as u64);
        if canonical_bytes > current.budget.max_bytes {
            return Err(exhausted("query exceeds its canonical source byte budget"));
        }
        if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
            return Err(exhausted(
                "query source admission exceeded its wall-time budget",
            ));
        }
    }
    Ok(json!({"records": records, "canonical_bytes": canonical_bytes}))
}

/// Carry the authority-bearing budget through presentation, including refusals.
/// A cached opening budget cannot undo a later session-budget narrowing.
pub(super) fn response(
    result: Result<Value>,
    context: &OperationContext,
    structured: bool,
) -> String {
    let mut payload = match result {
        Ok(payload) => payload,
        Err(error) => json!({"ok": false, "error": {
            "operation": "fortress.query", "code": error.code.as_str(),
            "message": error.message, "retryable": error.retryable,
            "details": error.details, "effect_may_have_occurred": false,
        }}),
    };
    if structured {
        payload["mode"] = json!("structured");
    }
    payload["query_budget"] = json!({
        "maximum_response_bytes": context.budget.max_bytes.min(
            u64::from(context.budget.max_output_tokens).saturating_mul(4)),
        "max_output_tokens": context.budget.max_output_tokens,
        "max_wall_millis": context.budget.max_wall_millis,
    });
    payload.to_string()
}

/// Enforce the negotiated bytes, tokens and wall time on the entire Agent Turn.
/// Structured cursors describe exactly the executor's rows, so those pages are
/// lossless or refused. Legacy pages may use the existing whole-row clipping,
/// which repairs offset continuations and withdraws completeness together.
pub(crate) fn finish_response(response: String, elapsed: Duration) -> String {
    let finishing = Instant::now();
    let Ok(payload) = serde_json::from_str::<Value>(&response) else {
        return response;
    };
    if !payload["query_budget"].is_object() {
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
    let structured = matches!(
        payload["mode"].as_str(),
        Some("structured" | "query_changes")
    );
    let publishable = serde_json::from_str::<Value>(&fitted).is_ok_and(|value| {
        matches!(
            value["output_budget"]["tier"].as_str(),
            Some("full" | "compact_turn")
        ) || (!structured && value["output_budget"]["tier"] == "truncated")
    });
    let wall_expired = elapsed.saturating_add(finishing.elapsed())
        >= Duration::from_millis(
            payload["query_budget"]["max_wall_millis"]
                .as_u64()
                .map_or(0, |value| value),
        );
    if !wall_expired && fitted.len() as u64 <= maximum_bytes && publishable {
        return fitted;
    }
    let mut turn = payload["agent_turn"].clone();
    // No successful comparison or detail path survives withdrawal of its page.
    // Keep independent active work, but do not summarize unpublished evidence.
    turn["changes"] = json!([]);
    turn["coverage"] = json!({"status": "partial", "complete_domains": [],
        "partial_domains": [], "absence_proof_scope": [], "continuation": null,
        "reason": "query result was not published"});
    turn["continuity"]["status"] = json!("stale");
    let message = if wall_expired {
        "query and its Agent Turn exceeded the negotiated wall-time budget"
    } else if structured {
        "the complete structured query and Agent Turn exceed the response budget; reduce limit, fields, metrics, or graph scope"
    } else {
        "a complete query page and its Agent Turn exceed the response budget; narrow the request or negotiate a larger budget"
    };
    let refusal = json!({"ok": false, "mode": payload["mode"],
        "session_id": payload["session_id"], "query_budget": payload["query_budget"],
        "error": {"operation": "fortress.query", "code": "budget_exceeded",
            "message": message, "retryable": false, "details": [],
            "effect_may_have_occurred": false,
            "recovery": {"class": "never_unchanged", "next_tool": "fortress.query",
                "reason": "narrow the original request or negotiate a larger budget"}},
        "agent_turn": turn});
    // As for every laboratory tool, budgets smaller than even the refusal
    // envelope receive the explicit minimal budget error, never a success.
    crate::output_budget::fit(&refusal.to_string(), maximum_tokens)
}

#[cfg(test)]
#[path = "lab_query_budget_tests.rs"]
mod tests;
