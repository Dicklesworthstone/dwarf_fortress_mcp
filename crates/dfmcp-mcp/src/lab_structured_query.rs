//! Stateless DfQL over the laboratory session's exact current or retained world.
//! Current grants authorize a historical read; historical ticks never renew them.

// Reuse the same pure executors as the live read surfaces, without instantiating
// their watch, baseline, archive, or native-observation ownership machinery.
#[path = "semantic_query_core.rs"]
#[allow(clippy::duplicate_mod)]
mod core_query;
#[path = "event_query.rs"]
#[allow(clippy::duplicate_mod)]
mod event_query;
#[path = "operational_query.rs"]
#[allow(clippy::duplicate_mod)]
mod operational_query;
#[path = "ranked_entity_query.rs"]
#[allow(clippy::duplicate_mod)]
mod ranked_entity_query;
#[path = "related_entity_query.rs"]
#[allow(clippy::duplicate_mod)]
mod related_entity_query;
#[path = "spatial_entity_query.rs"]
#[allow(clippy::duplicate_mod)]
mod spatial_entity_query;

use std::time::{Duration, Instant};

use dfmcp_core::{DfmcpError, ErrorCode, OperationContext, Result};
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
        Some(
            "entities"
                | "inspect"
                | "traverse"
                | "dependencies"
                | "aggregate"
                | "search"
                | "events"
                | "spatial_entities"
                | "related_entities"
                | "ranked_entities"
        )
    ) {
        return Err(invalid(
            "structured laboratory queries support only entities, inspect, traverse, dependencies, aggregate, search, events, spatial_entities, related_entities, and ranked_entities",
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
    let source_work = super::query_budget::admit_sources(&[snapshot], current)?;
    let mut source_context = current.clone();
    source_context.anchor = snapshot.anchor();
    let mut result = match request.request["query"]["kind"].as_str() {
        Some("aggregate" | "search") => {
            operational_query::execute(snapshot, &source_context, &request.request)?
        }
        Some("events") => event_query::execute(snapshot, &source_context, &request.request)?,
        Some("spatial_entities") => {
            spatial_entity_query::execute(snapshot, &source_context, &request.request)?
        }
        Some("related_entities") => {
            related_entity_query::execute(snapshot, &source_context, &request.request)?
        }
        Some("ranked_entities") => {
            ranked_entity_query::execute(snapshot, &source_context, &request.request)?
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
    result["source_work"] = source_work;
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
