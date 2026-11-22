//! Source-bound protocol-1.0 query pages over one already published capsule.
//! Querying never reads the bridge or changes the canonical observation.

use super::*;
use dfmcp_world::ContinuationToken;
use fastmcp_rust::{
    CompleteResult, Content, ContentBlock, CoreResult, FinalCallToolResult, FinalCoreResult,
    McpContext, McpResult, ResultMeta, Tool, ToolHandler,
};
use serde::Deserialize;
use std::time::Instant;

const MAX_PAGE_ROWS: u32 = 256;
const DEFAULT_PAGE_ROWS: u32 = 32;
const MAX_CURSOR_BYTES: usize = 96;
// The pinned modern codec permits a 4096-byte numeric JSON-RPC ID. The extra
// metadata allowance also covers promotion of the generated String result.
const WIRE_OVERHEAD: usize = 4096 + 512;
const MAX_TEXT_BYTES: usize = 60_000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    session_id: Option<String>,
    mode: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
}

/// Close the generated tool's input object without adding a twelfth tool or
/// exposing deployment configuration. Direct Rust calls share query_session.
pub(super) struct FortressPagedQuery;

impl ToolHandler for FortressPagedQuery {
    fn definition(&self) -> Tool {
        let mut definition = FortressQuery.definition();
        definition.input_schema["additionalProperties"] = json!(false);
        definition.input_schema["properties"]["mode"]["enum"] =
            json!(["summary", "citizens", "all", null]);
        definition.input_schema["properties"]["limit"]["minimum"] = json!(1);
        definition.input_schema["properties"]["limit"]["maximum"] = json!(MAX_PAGE_ROWS);
        definition.input_schema["properties"]["continuation"]["maxLength"] =
            json!(MAX_CURSOR_BYTES);
        definition
    }

    fn call(&self, ctx: &McpContext, arguments: JsonValue) -> McpResult<Vec<Content>> {
        ctx.checkpoint()?;
        let result = match serde_json::from_value::<Arguments>(arguments) {
            Ok(arguments) => fortress_query(
                arguments.session_id,
                arguments.mode,
                arguments.limit,
                arguments.continuation,
            ),
            Err(_) => unbound_error(
                "fortress.query",
                AgentPhase::Inspect,
                &error(
                    ErrorCode::InvalidRequest,
                    "query arguments must contain only session_id, mode, limit and continuation with their declared types",
                ),
            ),
        };
        Ok(vec![Content::text(result)])
    }
}

fn deadline(context: &OperationContext, started: Instant) -> Result<()> {
    if started.elapsed() >= Duration::from_millis(context.budget.max_wall_millis) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "live query exhausted its wall-time allowance",
        ));
    }
    Ok(())
}

fn cursor(context: &OperationContext, mode: &str, offset: u32) -> String {
    let anchor = context.anchor;
    let mut bytes = b"dfmcp.live-query-page/1\0protocol-1.0\0entity-id-ascending\0label-kind-generation-evidence\0".to_vec();
    bytes.extend_from_slice(&context.session_id.get().to_le_bytes());
    bytes.extend_from_slice(&anchor.fortress_id.get().to_le_bytes());
    bytes.extend_from_slice(&anchor.cursor.epoch.to_le_bytes());
    bytes.extend_from_slice(&anchor.cursor.sequence.to_le_bytes());
    bytes.extend_from_slice(&anchor.tick.get().to_le_bytes());
    bytes.extend_from_slice(anchor.state_hash.as_bytes());
    bytes.extend_from_slice(mode.as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(&offset.to_le_bytes());
    format!("lq10:{offset}:{}", Digest32::of_bytes(&bytes))
}

fn offset(context: &OperationContext, mode: &str, token: Option<&str>) -> Result<u32> {
    let Some(token) = token else { return Ok(0) };
    if token.is_empty() || token.len() > MAX_CURSOR_BYTES {
        return Err(error(
            ErrorCode::InvalidRequest,
            "live query continuation exceeds its byte bound",
        ));
    }
    let mut parts = token.split(':');
    let prefix = parts.next();
    let raw = parts.next();
    let digest = parts.next();
    if prefix != Some("lq10")
        || parts.next().is_some()
        || digest.is_none_or(|value| {
            value.len() != 64
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
    {
        return Err(error(
            ErrorCode::InvalidRequest,
            "invalid live query continuation",
        ));
    }
    let value = raw
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| {
            error(
                ErrorCode::InvalidRequest,
                "invalid live query continuation offset",
            )
        })?;
    if value == 0 || raw != Some(value.to_string().as_str()) {
        return Err(error(
            ErrorCode::InvalidRequest,
            "live query continuation offset is not canonical",
        ));
    }
    // A cursor is a checksummed source selector, never authority. Every page
    // independently checks current grants before accepting this binding.
    if cursor(context, mode, value) != token {
        return Err(error(
            ErrorCode::CursorGap,
            "live query continuation does not match this session, complete anchor or mode; restart the query",
        ));
    }
    Ok(value)
}

fn fits(encoded: &str, budget: WorkBudget) -> bool {
    if encoded.len() > MAX_TEXT_BYTES
        || encoded.len() as u64 > u64::from(budget.max_output_tokens) * 4
    {
        return false;
    }
    let result = CompleteResult::new(
        FinalCallToolResult {
            content: vec![ContentBlock::text(encoded.to_owned())],
            is_error: false,
            structured_content: None,
        },
        ResultMeta::empty(),
    );
    CoreResult::Final(FinalCoreResult::ToolsCall {
        result,
        diagnostic: None,
    })
    .encode()
    .is_ok_and(|wire| wire.len().saturating_add(WIRE_OVERHEAD) as u64 <= budget.max_bytes)
}

pub(super) fn query_session(
    session: &mut LiveSession<impl LiveSessionAdapter>,
    mode: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    started: Instant,
) -> String {
    let anchor = match session.current_anchor() {
        Ok(value) => value,
        Err(failure) => return unbound_error("fortress.query", AgentPhase::Inspect, &failure),
    };
    let (request_id, context) = match session.next_context() {
        Ok(value) => value,
        Err(failure) => {
            return session_error(
                session,
                "fortress.query",
                AgentPhase::Inspect,
                ObservationProfile::Tactical,
                RequestId::NIL,
                anchor,
                ContinuityStatus::Continuous,
                &failure,
            );
        }
    };
    let result = query_page(session, &context, mode, limit, continuation, started);
    match result {
        Ok(encoded) => encoded,
        Err(failure) => session_error(
            session,
            "fortress.query",
            AgentPhase::Inspect,
            ObservationProfile::Tactical,
            request_id,
            anchor,
            if session.source_poisoned() {
                ContinuityStatus::Stale
            } else {
                ContinuityStatus::Continuous
            },
            &failure,
        ),
    }
}

fn query_page(
    session: &mut LiveSession<impl LiveSessionAdapter>,
    context: &OperationContext,
    mode: Option<String>,
    limit: Option<u32>,
    continuation: Option<String>,
    started: Instant,
) -> Result<String> {
    context.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    deadline(context, started)?;
    if session.source_poisoned() {
        return Err(error(
            ErrorCode::AdapterFailure,
            "live query source is poisoned; open a fresh session",
        ));
    }
    let mode = mode.map_or_else(|| "summary".to_owned(), |value| value);
    if mode.is_empty() || mode.len() > MAX_MODE_BYTES {
        return Err(error(
            ErrorCode::InvalidRequest,
            "query mode violates its byte bound",
        ));
    }
    let kinds = match mode.as_str() {
        "summary" => vec![EntityKind::Fortress],
        "citizens" => vec![EntityKind::Unit],
        "all" => Vec::new(),
        _ => {
            return Err(error(
                ErrorCode::InvalidRequest,
                "query mode must be summary, citizens, or all",
            ));
        }
    };
    let limit = limit.map_or(
        DEFAULT_PAGE_ROWS.min(context.budget.max_entities),
        |value| value,
    );
    if limit == 0 || limit > MAX_PAGE_ROWS || limit > context.budget.max_entities {
        return Err(error(
            ErrorCode::InvalidRequest,
            "query limit must be 1..256 within the session entity budget",
        ));
    }
    let offset = offset(context, &mode, continuation.as_deref())?;
    let inner = (offset > 0).then(|| {
        ContinuationToken::new(context.anchor.fortress_id, context.anchor.cursor, offset).encode()
    });
    let response = session.adapter.query(
        &QueryRequest {
            anchor: context.anchor,
            query: WorldQuery {
                kinds,
                predicate: None,
                order: QueryOrder::EntityIdAscending,
                limit,
                continuation: inner.clone(),
            },
            max_output_tokens: context.budget.max_output_tokens,
            continuation: inner,
        },
        context,
    )?;
    deadline(context, started)?;
    if response.anchor != context.anchor || session.current_anchor()? != context.anchor {
        return Err(error(
            ErrorCode::InternalInvariantViolation,
            "live query changed its published anchor",
        ));
    }
    let projection = session.adapter.query_projection().ok_or_else(|| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live query lost its canonical projection",
        )
    })?;
    let rows = response.rows.iter().map(|row| {
        let entity = projection.snapshot.graph.entities.get(&row.entity_id).ok_or_else(|| {
            error(ErrorCode::InternalInvariantViolation, "live query row has no canonical entity")
        })?;
        Ok(json!({"entity_id":row.entity_id.to_string(), "generation":entity.generation,
            "revision":row.revision, "fields":row.fields,
            "evidence":row.evidence.iter().map(|value| value.digest.to_string()).collect::<Vec<_>>()}))
    }).collect::<Result<Vec<_>>>()?;
    if rows.is_empty() && response.matched > u64::from(offset) {
        return Err(error(
            ErrorCode::BudgetExceeded,
            "live query cannot publish an empty advancing page",
        ));
    }
    let mut count = rows.len();
    loop {
        deadline(context, started)?;
        let encoded = render_page(
            session,
            context,
            Page {
                mode: &mode,
                offset,
                limit,
                matched: response.matched,
                rows: &rows[..count],
                scores: &response.score_ledger,
            },
        )?;
        if fits(&encoded, context.budget) {
            deadline(context, started)?;
            return Ok(encoded);
        }
        if count <= 1 {
            return Err(error(
                ErrorCode::BudgetExceeded,
                "one live query row and required Agent Turn do not fit the response allowance; open a session with a larger output budget",
            ));
        }
        // This trims only presentation rows after one bounded adapter query.
        // The next cursor starts after the actual published prefix, including
        // when the adapter already clipped its canonical-record byte budget.
        count /= 2;
    }
}

struct Page<'a> {
    mode: &'a str,
    offset: u32,
    limit: u32,
    matched: u64,
    rows: &'a [JsonValue],
    scores: &'a [String],
}

fn render_page(
    session: &LiveSession<impl LiveSessionAdapter>,
    context: &OperationContext,
    Page {
        mode,
        offset,
        limit,
        matched,
        rows,
        scores,
    }: Page<'_>,
) -> Result<String> {
    let consumed = offset
        .checked_add(u32::try_from(rows.len()).map_err(|_| {
            error(
                ErrorCode::BudgetExceeded,
                "live query page offset exceeds its bound",
            )
        })?)
        .ok_or_else(|| error(ErrorCode::BudgetExceeded, "live query page offset overflow"))?;
    let truncated = u64::from(consumed) < matched;
    let next = truncated.then(|| cursor(context, mode, consumed));
    let complete = offset == 0 && !truncated;
    let mut payload = json!({"ok":true, "session_id":context.session_id.to_string(),
        "request_id":context.request_id.to_string(), "mode":mode, "anchor":anchor_json(context.anchor),
        "matched":matched, "returned":rows.len(), "offset":offset,
        "truncated":truncated, "continuation":next, "complete_domain":complete,
        "rows":rows, "score_ledger":scores});
    payload["next_query"] = next.as_ref().map_or(JsonValue::Null, |token| {
        json!({"tool":"fortress_query", "arguments":{"session_id":context.session_id.to_string(),
            "mode":mode,"limit":limit,"continuation":token}})
    });
    let raw = attach_turn(
        session,
        "fortress.query",
        AgentPhase::Inspect,
        ObservationProfile::Tactical,
        context.request_id,
        if complete {
            ContinuityStatus::Continuous
        } else {
            ContinuityStatus::Partial
        },
        Some(context.anchor),
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        payload,
    );
    let mut packet: JsonValue = serde_json::from_str(&raw).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "live query packet is not JSON",
        )
    })?;
    // Tactical pages expose their two useful next read operations. The other
    // read-only tools remain discoverable in tools/list; repeating their complete
    // affordance descriptions would crowd out rows at the default token allowance.
    if let Some(affordances) = packet["agent_turn"]["affordances"].as_array_mut() {
        affordances.retain(|item| {
            matches!(
                item["tool"].as_str(),
                Some("fortress.query" | "fortress.observe")
            )
        });
        for affordance in affordances {
            if let Some(fields) = affordance.as_object_mut() {
                fields.retain(|key, _| {
                    matches!(
                        key.as_str(),
                        "affordance_id"
                            | "tool"
                            | "risk"
                            | "enabled"
                            | "disabled_reason"
                            | "arguments"
                    )
                });
            }
        }
    }
    // The seven exact omission reasons already appear in source coverage. One
    // unknown-domain warning names every affected domain without repeating the
    // same prose in seven independent uncertainty envelopes. Other warnings,
    // including mutation unavailability, remain complete.
    let mut uncertainties = uncertainties_json(session);
    uncertainties.retain(|item| {
        !OMITTED_LIVE_DOMAINS.iter().any(|(domain, _)| {
            item["uncertainty_id"].as_str() == Some(format!("omitted-{domain}").as_str())
        })
    });
    let mut omitted = uncertainty(
        "live-unobserved-domains",
        "unknown",
        "bridge protocol V1 omits the listed domains",
        "their exact omission reasons remain in coverage.omitted_domains; omitted state cannot prove absence",
        None,
        json!({}),
    );
    omitted["domains"] = json!(
        OMITTED_LIVE_DOMAINS
            .iter()
            .map(|(domain, _)| *domain)
            .collect::<Vec<_>>()
    );
    uncertainties.insert(0, omitted);
    packet["agent_turn"]["uncertainty"] = json!(uncertainties);
    // These values are identical in the live session; retain the complete
    // admitted budget and state the equality instead of serializing it twice.
    if let Some(budget) = packet["agent_turn"]["budget"].as_object_mut() {
        budget.remove("requested");
        budget.insert("requested_equals_admitted".to_owned(), json!(true));
    }
    let coverage = &mut packet["agent_turn"]["coverage"];
    coverage["query_result"] = json!({"mode":mode, "offset":offset, "returned":rows.len(),
        "matched":matched, "complete_domain":complete});
    coverage["continuation"] = json!(next);
    if !complete {
        coverage["status"] = json!("partial");
        if let Some(partial) = coverage["partial_domains"].as_array_mut() {
            partial.push(json!({"domain":"query.result", "reason":"this response contains one bounded page of the selected result"}));
        }
    }
    Ok(packet.to_string())
}

#[cfg(test)]
#[path = "live_query_v1_0_tests.rs"]
mod tests;
