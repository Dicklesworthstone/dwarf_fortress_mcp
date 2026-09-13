//! Protocol-1.1 query tool integration. Session ownership and bridge state stay
//! in the parent server; query execution never refreshes or mutates the game.

use super::*;

#[path = "semantic_query.rs"]
mod semantic_query;

pub(super) fn query(
    session_id: Option<String>,
    mode: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    query: Option<JsonValue>,
) -> String {
    let operation = "fortress.query";
    let session = match resolve_session(session_id) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let mut guard = match lock_session(&session) {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let anchor = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let (request_id, context) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(&guard, operation, AgentPhase::Inspect,
                ObservationProfile::Tactical, RequestId::NIL, anchor,
                ContinuityStatus::Continuous, &failure);
        }
    };
    let outcome = (|| -> Result<JsonValue> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if guard.source_poisoned() {
            return Err(error(ErrorCode::AdapterFailure,
                "query source is poisoned; open a new session instead of treating cached data as fresh"));
        }
        let mut payload = if let Some(input) = query {
            if mode.is_some() || limit.is_some() || continuation.is_some() {
                return Err(error(ErrorCode::InvalidRequest,
                    "do not mix mode/limit/continuation with query; put structured options inside query.query"));
            }
            let projection = guard.adapter.current_projection().ok_or_else(|| {
                error(ErrorCode::InternalInvariantViolation,
                    "structured query has no published canonical projection")
            })?;
            let mut result = semantic_query::execute(&projection.snapshot, &context, &input)?;
            result["mode"] = json!("structured");
            result
        } else {
            let mode = mode.map_or_else(|| "summary".to_owned(), |value| value);
            if mode.is_empty() || mode.len() > MAX_MODE_BYTES {
                return Err(error(ErrorCode::InvalidRequest, "query mode violates its byte bound"));
            }
            let kinds = query_kinds(&mode)?;
            let budget = guard.budget;
            let request = QueryRequest {
                anchor,
                query: WorldQuery {
                    kinds,
                    predicate: None,
                    order: QueryOrder::EntityIdAscending,
                    limit: limit.map_or(budget.max_entities.min(32), |value| value),
                    continuation: continuation.clone(),
                },
                max_output_tokens: budget.max_output_tokens,
                continuation,
            };
            let response = guard.adapter.query(&request, &context)?;
            if response.anchor != anchor {
                return Err(error(ErrorCode::InternalInvariantViolation,
                    "query adapter changed its anchor while producing a page"));
            }
            let projection = guard.adapter.current_projection().ok_or_else(|| {
                error(ErrorCode::InternalInvariantViolation, "query adapter lost its projection")
            })?;
            let rows = response.rows.iter().map(|row| {
                let entity = projection.snapshot.graph.entities.get(&row.entity_id).ok_or_else(|| {
                    error(ErrorCode::InternalInvariantViolation, "query row has no canonical entity")
                })?;
                Ok(json!({
                    "entity_id": row.entity_id.to_string(),
                    "generation": entity.generation,
                    "revision": row.revision,
                    "fields": row.fields,
                    "evidence": row.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>(),
                }))
            }).collect::<Result<Vec<_>>>()?;
            json!({
                "mode": mode,
                "anchor": anchor_json(response.anchor),
                "matched": response.matched,
                "returned": rows.len(),
                "truncated": response.truncated,
                "continuation": response.continuation,
                "rows": rows,
                "score_ledger": response.score_ledger,
            })
        };
        payload["ok"] = json!(true);
        payload["session_id"] = json!(guard.session_id.to_string());
        payload["request_id"] = json!(request_id.to_string());
        payload["source_evidence"] = json!(references_json(&guard));
        Ok(payload)
    })();
    match outcome {
        Ok(payload) => {
            let continuity = if payload.get("truncated").and_then(JsonValue::as_bool) == Some(true) {
                ContinuityStatus::Partial
            } else {
                ContinuityStatus::Continuous
            };
            attach_turn(&guard, operation, AgentPhase::Inspect, ObservationProfile::Tactical,
                request_id, continuity, Some(anchor), None, Vec::new(),
                announcement_attention(&guard), Vec::new(), payload)
        }
        Err(failure) => session_error(&guard, operation, AgentPhase::Inspect,
            ObservationProfile::Tactical, request_id, anchor,
            if guard.source_poisoned() { ContinuityStatus::Stale } else { ContinuityStatus::Continuous },
            &failure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structured_query_without_a_session_returns_the_agent_turn_contract() -> Result<()> {
        let raw = query(None, None, None, None, Some(json!({
            "schema":"dfmcp.query/1", "query":{"kind":"entities"}
        })));
        let result: JsonValue = serde_json::from_str(&raw).map_err(|failure| {
            error(ErrorCode::InternalInvariantViolation, failure.to_string())
        })?;
        assert_eq!(result["ok"], false);
        assert_eq!(result["agent_turn"]["operation"], "fortress.query");
        assert_eq!(result["agent_turn"]["briefing"]["runtime_admitted"], false);
        Ok(())
    }

    #[test]
    fn foreign_runtime_sessions_cannot_enter_structured_inspection() -> Result<()> {
        let raw = query(Some("80000000000000000000000000000001".to_owned()), None,
            None, None, Some(json!({"schema":"dfmcp.query/1", "query":{
                "kind":"dependencies", "edge_kinds":["requires"]
            }})));
        let result: JsonValue = serde_json::from_str(&raw).map_err(|failure| {
            error(ErrorCode::InternalInvariantViolation, failure.to_string())
        })?;
        assert_eq!(result["ok"], false);
        assert_eq!(result["error"]["code"], ErrorCode::InvalidRequest.as_str());
        assert!(result.get("components").is_none());
        Ok(())
    }
}
