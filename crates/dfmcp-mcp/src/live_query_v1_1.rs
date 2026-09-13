//! Protocol-1.1 query integration over session-owned canonical state.
//! Only await_watch explicitly refreshes the read-only bridge, at most once.

use super::*;

#[path = "semantic_query.rs"]
mod semantic_query;
#[path = "query_response.rs"]
mod query_response;
#[path = "watch_refresh.rs"]
mod watch_refresh;

use query_response::QueryResponseProjection;

pub(super) fn query(
    session_id: Option<String>,
    mode: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    mut query: Option<JsonValue>,
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
    let basis = match guard.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error(operation, AgentPhase::Inspect, &failure),
    };
    let (request_id, mut context) = match guard.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(&guard, operation, AgentPhase::Inspect,
                ObservationProfile::Tactical, RequestId::NIL, basis,
                ContinuityStatus::Continuous, &failure);
        }
    };
    let outcome = (|| -> Result<String> {
        context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
        if query.is_some() && (mode.is_some() || limit.is_some() || continuation.is_some()) {
            return Err(error(ErrorCode::InvalidRequest,
                "do not mix mode/limit/continuation with query; put structured options inside query.query"));
        }
        let kind = query.as_ref().and_then(|input| input.get("query"))
            .and_then(|input| input.get("kind")).and_then(JsonValue::as_str);
        let awaiting = kind == Some("await_watch");
        let metadata_only = matches!(kind, Some("watches" | "cancel_watch" | "release_watch"))
            || (query.is_none() && mode.as_deref() == Some("schema"));
        let needs_read = if awaiting {
            let projection = guard.adapter.current_projection().ok_or_else(|| {
                error(ErrorCode::InternalInvariantViolation, "condition wait has no canonical projection")
            })?;
            let input = query.as_ref().ok_or_else(|| {
                error(ErrorCode::InternalInvariantViolation, "condition wait lost its request")
            })?;
            semantic_query::prepare_await(&projection.snapshot, &context, input)?
        } else { false };
        if guard.source_poisoned() && !metadata_only && !(awaiting && !needs_read) {
            return Err(error(ErrorCode::AdapterFailure,
                "query source is poisoned; only watch listing/cancellation/release and schema discovery remain available"));
        }
        let refresh = if needs_read {
            let observation = watch_refresh::once(&context, |request, ctx| {
                guard.adapter.observe(request, ctx)
            })?;
            if guard.current_anchor()? != observation.anchor {
                return Err(error(ErrorCode::InternalInvariantViolation,
                    "condition wait reader and published adapter disagree on the target anchor"));
            }
            context.anchor = observation.anchor;
            // A read can advance beyond a grant's expiry. Recheck at the new
            // anchor before evaluating or publishing any watch transition.
            context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
            Some(observation.summary)
        } else { None };
        if awaiting {
            let input = query.as_mut().ok_or_else(|| {
                error(ErrorCode::InternalInvariantViolation, "condition wait lost its request")
            })?;
            input["query"]["kind"] = json!("poll_watch");
            // The client's expected anchor was checked before the read. The
            // internal poll intentionally consumes the newly published anchor.
            input["expected_anchor"] = anchor_json(context.anchor);
        }
        let anchor = guard.current_anchor()?;
        let source_stale = guard.source_poisoned();
        let view = QueryResponseProjection {
            session_id: guard.session_id.to_string(),
            request_id: request_id.to_string(),
            anchor: anchor_json(anchor),
            briefing: briefing_json(&guard),
            attention: announcement_attention(&guard),
            affordances: affordances_json(&guard),
            uncertainty: uncertainties_json(&guard),
            coverage: coverage_json(&guard),
            budget: budget_json(guard.budget),
            references: references_json(&guard),
            maximum_bytes: response_byte_limit(&guard),
        };
        let result_bytes = view.result_byte_budget()?;
        let mut result_context = context.clone();
        result_context.budget.max_bytes = u64::try_from(result_bytes).map_err(|_| {
            error(ErrorCode::BudgetExceeded, "remaining query budget cannot be represented")
        })?;
        let payload = if query.is_none() && mode.as_deref() == Some("schema") {
            if limit.is_some() || continuation.is_some() {
                return Err(error(ErrorCode::InvalidRequest,
                    "schema discovery does not accept pagination arguments"));
            }
            json!({
                "mode": "schema",
                "anchor": anchor_json(anchor),
                "truncated": false,
                "continuation": null,
                "source_stale": source_stale,
                "query_schema": query_schema()?,
                "usage": "Pass an envelope conforming to query_schema in the query argument; omit mode, limit and continuation at the top level. await_watch performs one authorized observation; other queries do not refresh.",
                "example": {
                    "schema": "dfmcp.query/1",
                    "query": {
                        "kind": "entities", "kinds": ["unit"],
                        "fields": ["name", "profession", "position", "sane"],
                        "limit": 4,
                        "where": {"op": "compare", "field": "sane", "comparison": "eq",
                            "value": {"type": "bool", "value": false}},
                    },
                },
            })
        } else if let Some(input) = query {
            let projection = guard.adapter.current_projection().ok_or_else(|| {
                error(ErrorCode::InternalInvariantViolation,
                    "structured query has no published canonical projection")
            })?;
            return semantic_query::execute_with_publisher(
                &projection.snapshot, &result_context, &input, |mut result| {
                    add_field_catalogs(&projection.snapshot, &mut result, result_bytes)?;
                    result["mode"] = json!("structured");
                    result["source_stale"] = json!(source_stale);
                    if awaiting {
                        result["kind"] = json!("await_watch");
                        result["observation_refresh"] = refresh.unwrap_or_else(|| json!({
                            "kind":"skipped_terminal_watch","read_calls":0,"advanced_game":false,
                            "basis":anchor_json(basis),"target":anchor_json(anchor),"reset":false
                        }));
                    }
                    view.finish(result)
                },
            );
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
            let narrowed = semantic_query::result_context(&result_context)?;
            let response = guard.adapter.query(&request, &narrowed)?;
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
        semantic_query::publish_with_active_work(&result_context, payload, |value| view.finish(value))
    })();
    match outcome {
        Ok(encoded) => encoded,
        Err(failure) => {
            let current = guard.current_anchor().unwrap_or(basis);
            let continuity = if guard.source_poisoned() { ContinuityStatus::Stale }
                else if current.cursor.epoch != basis.cursor.epoch { ContinuityStatus::Reset }
                else { ContinuityStatus::Continuous };
            let raw = session_error(&guard, operation, AgentPhase::Inspect,
                ObservationProfile::Tactical, request_id, basis, continuity, &failure);
            context.anchor = current;
            // Disclose retained work on authorized error paths too. Failure to
            // render this additive projection cannot mutate or discard work.
            let decorated = semantic_query::publish_with_active_work(&context, json!({}), |metadata| {
                let mut payload: JsonValue = serde_json::from_str(&raw).map_err(|_| {
                    error(ErrorCode::InternalInvariantViolation,"query error packet is not JSON")
                })?;
                payload["agent_turn"]["active_work"]["obligations"] = metadata["_condition_watch_work"].clone();
                let encoded = payload.to_string();
                if encoded.len() > response_byte_limit(&guard) {
                    return Err(error(ErrorCode::BudgetExceeded,"query error and active work exceed the response budget"));
                }
                Ok(encoded)
            });
            decorated.unwrap_or(raw)
        }
    }
}

fn query_schema() -> Result<JsonValue> {
    serde_json::from_str(include_str!("../../../schemas/mcp_query_v1.json"))
        .map_err(|failure| error(ErrorCode::InternalInvariantViolation,
            format!("embedded query schema is invalid: {failure}")))
}

fn add_field_catalogs(
    snapshot: &dfmcp_world::WorldSnapshot,
    payload: &mut JsonValue,
    byte_limit: usize,
) -> Result<()> {
    if !matches!(payload.get("kind").and_then(JsonValue::as_str), Some("entities" | "inspect")) {
        return Ok(());
    }
    let size = serde_json::to_vec(payload).map_err(|_| {
        error(ErrorCode::InternalInvariantViolation, "cannot measure query field catalogs")
    })?.len();
    let Some(mut remaining) = byte_limit.checked_sub(size.saturating_add(160)) else {
        return Ok(());
    };
    let mut omitted = false;
    let mut add = |row: &mut JsonValue| -> Result<()> {
        let Some(id) = row.get("entity_id").and_then(JsonValue::as_str)
            .and_then(|id| id.parse::<u64>().ok()) else { return Ok(()); };
        let Some(entity) = snapshot.graph.entities.get(&EntityId::new(id)) else { return Ok(()); };
        let fields = entity.fields.keys().filter(|field| field.len() <= 128)
            .take(128).collect::<Vec<_>>();
        let addition = json!({"available_fields":fields,
            "available_fields_truncated":fields.len() != entity.fields.len()});
        let added_bytes = serde_json::to_vec(&addition).map_err(|_| {
            error(ErrorCode::InternalInvariantViolation, "cannot measure query field catalog")
        })?.len();
        if added_bytes > remaining {
            omitted = true;
            return Ok(());
        }
        remaining -= added_bytes;
        if let (Some(row), Some(addition)) = (row.as_object_mut(), addition.as_object()) {
            row.extend(addition.iter().map(|(key, value)| (key.clone(), value.clone())));
        }
        Ok(())
    };
    if let Some(rows) = payload.get_mut("rows").and_then(JsonValue::as_array_mut) {
        for row in rows { add(row)?; }
    }
    if let Some(row) = payload.get_mut("row") { add(row)?; }
    if omitted {
        payload["field_catalogs_omitted_for_budget"] = json!(true);
    }
    Ok(())
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

    #[test]
    fn embedded_schema_lists_every_executable_query_variant() -> Result<()> {
        let schema = query_schema()?;
        assert_eq!(schema["properties"]["schema"]["const"], "dfmcp.query/1");
        let variants = schema["$defs"]["query"]["oneOf"].as_array().ok_or_else(|| {
            error(ErrorCode::InternalInvariantViolation, "query variants missing")
        })?;
        let kinds = variants.iter().filter_map(|variant| {
            variant["properties"]["kind"]["const"].as_str()
        }).collect::<Vec<_>>();
        assert_eq!(kinds, vec!["entities", "inspect", "traverse", "dependencies", "aggregate", "search",
            "capture", "changes", "baselines", "release_baseline", "watch", "poll_watch", "await_watch",
            "watches", "cancel_watch", "release_watch"]);
        Ok(())
    }

    #[test]
    fn registered_query_function_keeps_missing_session_errors_structured() -> Result<()> {
        let raw = super::super::fortress_query(None, None, Some(1), None, None);
        let result: JsonValue = serde_json::from_str(&raw).map_err(|failure| {
            error(ErrorCode::InternalInvariantViolation, failure.to_string())
        })?;
        assert_eq!(result["ok"], false);
        assert_eq!(result["agent_turn"]["operation"], "fortress.query");
        Ok(())
    }

    #[test]
    fn defaults_leave_room_for_the_spine_without_widening_explicit_budgets() -> Result<()> {
        let default = requested_budget(None, None, None, None, None, None)?;
        assert_eq!(default.max_output_tokens, DEFAULT_RESPONSE_TOKENS);
        assert!(default.max_output_tokens >= MIN_RESPONSE_TOKENS);
        let explicit = requested_budget(None, None, None, None, Some(MIN_RESPONSE_TOKENS), None)?;
        assert_eq!(explicit.max_output_tokens, MIN_RESPONSE_TOKENS);
        assert!(requested_budget(None, None, None, None, Some(1_500), None).is_err());
        Ok(())
    }
}
