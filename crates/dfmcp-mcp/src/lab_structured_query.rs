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

use dfmcp_core::{
    Capability, DfmcpError, ErrorCode, OperationContext, Result, RiskTier, StateAnchor,
};
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

impl Request {
    pub(super) fn is_entities(&self) -> bool {
        self.request["query"]["kind"] == "entities"
    }
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
    let result = match request.request["query"]["kind"].as_str() {
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
    Ok(decorate(result, snapshot.anchor(), current, source_work))
}

/// Internal orientation only; no provisional page is ever published. The
/// source is selected under current authority before the facade captures its
/// presentation dependencies outside the laboratory's session locks.
pub(super) fn orientation(source: StateAnchor, current: &OperationContext) -> Value {
    json!({"ok":true, "mode":"structured", "kind":"entities",
        "session_id":current.session_id.to_string(), "anchor":core_query::anchor_json(source),
        "current_anchor":core_query::anchor_json(current.anchor), "game_tick":source.tick.0,
        "historical":source != current.anchor})
}

/// Fit each whole entity page after adding the exact lab wrapper and the
/// caller's immutable Agent Turn. Source admission retains its own allowance.
pub(super) fn execute_projected(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
    started: Instant,
    fits: &dyn Fn(&Value) -> Result<bool>,
) -> Result<Value> {
    if !request.is_entities() {
        return Err(invalid("projected publication requires an entities query"));
    }
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mut source_context = current.clone();
    source_context.budget.max_wall_millis = remaining_wall_millis(current, started)?;
    let source_work = super::query_budget::admit_sources(&[snapshot], &source_context)?;
    source_context.budget.max_wall_millis = remaining_wall_millis(current, started)?;
    source_context.anchor = snapshot.anchor();
    let result = core_query::execute_with_page_budget(
        snapshot,
        &source_context,
        &request.request,
        current.budget.max_bytes,
        &|page| {
            fits(&super::query_budget::response_payload(
                Ok(decorate(
                    page.clone(),
                    snapshot.anchor(),
                    current,
                    source_work.clone(),
                )),
                current,
                true,
            ))
        },
    )?;
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    if started.elapsed() >= Duration::from_millis(current.budget.max_wall_millis) {
        return Err(exhausted("structured query exceeded its wall-time budget"));
    }
    Ok(decorate(result, snapshot.anchor(), current, source_work))
}

fn remaining_wall_millis(current: &OperationContext, started: Instant) -> Result<u64> {
    let remaining = Duration::from_millis(current.budget.max_wall_millis)
        .checked_sub(started.elapsed())
        .filter(|remaining| remaining.as_millis() > 0)
        .ok_or_else(|| exhausted("structured query exceeded its wall-time budget"))?;
    u64::try_from(remaining.as_millis())
        .map_err(|_| exhausted("remaining query wall-time cannot be represented"))
}

fn decorate(
    mut result: Value,
    source: StateAnchor,
    current: &OperationContext,
    source_work: Value,
) -> Value {
    let historical = source != current.anchor;
    result["mode"] = json!("structured");
    result["ok"] = json!(true);
    result["session_id"] = json!(current.session_id.to_string());
    result["game_tick"] = json!(source.tick.0);
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
    result
}

#[cfg(test)]
mod projected_tests {
    use super::*;
    use dfmcp_core::{
        CapabilityGrant, CapabilityScope, Digest32, EntityId, FortressId, GameTick,
        ObservationCursor, RequestId, SessionId, WorkBudget,
    };
    use dfmcp_world::{
        EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue, WorldGraph,
    };
    use std::collections::BTreeMap;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn fixture(count: u64, hidden_bytes: usize) -> (WorldSnapshot, OperationContext) {
        let mut graph = WorldGraph::default();
        for id in 1..=count {
            let fields = if hidden_bytes == 0 {
                BTreeMap::new()
            } else {
                BTreeMap::from([
                    (
                        "alive".to_owned(),
                        Fact::known(
                            WorldValue::Bool(true),
                            GameTick(10),
                            FactSource::Replay,
                            Digest32::ZERO,
                        ),
                    ),
                    (
                        "unselected".to_owned(),
                        Fact::known(
                            WorldValue::Text("x".repeat(hidden_bytes)),
                            GameTick(10),
                            FactSource::Replay,
                            Digest32::ZERO,
                        ),
                    ),
                ])
            };
            graph.entities.insert(
                EntityId::new(id),
                EntityRecord {
                    id: EntityId::new(id),
                    generation: 1,
                    revision: 1,
                    kind: EntityKind::Unit,
                    label: format!("Urist {id}"),
                    fields,
                },
            );
        }
        let source = WorldSnapshot::new(
            FortressId::new(1),
            GameTick(10),
            ObservationCursor::ORIGIN,
            true,
            graph,
        );
        let current = OperationContext {
            session_id: SessionId::new(1),
            request_id: RequestId::new(1),
            anchor: source.anchor(),
            budget: WorkBudget {
                max_wall_millis: 60_000,
                max_entities: 512,
                max_bytes: 2 * 1024 * 1024,
                max_output_tokens: 1800,
                ..WorkBudget::default()
            },
            grants: vec![CapabilityGrant {
                capability: Capability::Query,
                scope: CapabilityScope {
                    fortress_id: Some(source.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            }],
            cancellation_requested: false,
        };
        (source, current)
    }

    fn request(limit: u32, fields: &[&str]) -> Request {
        Request {
            mode: Mode::Structured,
            at: None,
            request: json!({"schema":"dfmcp.query/1", "query":{
                "kind":"entities", "kinds":["unit"], "fields":fields, "limit":limit}}),
        }
    }

    // An immutable publisher fixture: its retained work cannot be clipped to
    // make room for a page. Actual facade coverage is exercised separately.
    fn packet(page: &Value) -> String {
        let mut packet = page.clone();
        packet["agent_turn"] = json!({
            "schema":"dfmcp.agent_turn/1", "operation":"fortress.query",
            "anchor":page["current_anchor"], "continuity":{"status":"partial"},
            "active_work":{"obligations":[{"retained_evidence":"w".repeat(4000)}]},
            "coverage":page["observation_coverage"], "recommendations":[], "uncertainty":[],
        });
        packet.to_string()
    }

    #[test]
    fn projected_lab_pages_fit_complete_wrappers_and_resume_without_losing_rows() -> TestResult {
        let (source, mut current) = fixture(16, 16_384);
        let one = execute_projected(
            &source,
            &current,
            &request(1, &["alive"]),
            Instant::now(),
            &|_| Ok(true),
        )?;
        let one = super::super::query_budget::response_payload(Ok(one), &current, true);
        current.budget.max_output_tokens = u32::try_from((packet(&one).len() + 256).div_ceil(4))?;
        let mut selected = request(16, &["alive"]);
        let fits = |page: &Value| {
            super::super::query_budget::lossless_page_fits(&packet(page), &page["query_budget"])
        };
        let mut ids = Vec::new();
        for page_index in 0..16 {
            let result = execute_projected(&source, &current, &selected, Instant::now(), &fits)?;
            let complete = super::super::query_budget::response_payload(Ok(result), &current, true);
            let encoded =
                super::super::query_budget::finish_response(packet(&complete), Duration::ZERO);
            let page: Value = serde_json::from_str(&encoded)?;
            assert_eq!(page["ok"], true, "{page}");
            assert!(encoded.len() <= current.budget.max_output_tokens as usize * 4);
            assert!(matches!(
                page["output_budget"]["tier"].as_str(),
                Some("full" | "compact_turn")
            ));
            let rows = page["rows"].as_array().ok_or("rows missing")?;
            assert!(!rows.is_empty());
            if page_index == 0 {
                assert!(rows.len() < 16);
            }
            for row in rows {
                assert_eq!(row["fields"]["alive"]["value"]["value"], true);
                assert!(row["fields"].get("unselected").is_none());
                ids.push(
                    row["entity_id"]
                        .as_str()
                        .ok_or("entity missing")?
                        .to_owned(),
                );
            }
            assert_eq!(
                page["observation_coverage"]["continuation"],
                page["continuation"]
            );
            assert_eq!(
                page["agent_turn"]["coverage"]["continuation"],
                page["continuation"]
            );
            if page["continuation"].is_null() {
                break;
            }
            selected.request["query"]["continuation"] = page["continuation"].clone();
        }
        assert_eq!(ids, (1..=16).map(|id| id.to_string()).collect::<Vec<_>>());
        let mut tiny = current.clone();
        tiny.budget.max_output_tokens = 400;
        assert_eq!(
            execute_projected(
                &source,
                &tiny,
                &request(1, &["alive"]),
                Instant::now(),
                &fits
            )
            .map_err(|error| error.code),
            Err(ErrorCode::BudgetExceeded)
        );
        Ok(())
    }

    #[test]
    fn projected_lab_terminal_page_can_fit_when_a_cursor_bearing_prefix_does_not() -> TestResult {
        let (source, current) = fixture(2, 0);
        let full = execute_projected(&source, &current, &request(2, &[]), Instant::now(), &|_| {
            Ok(true)
        })?;
        let prefix =
            execute_projected(&source, &current, &request(1, &[]), Instant::now(), &|_| {
                Ok(true)
            })?;
        let full = super::super::query_budget::response_payload(Ok(full), &current, true);
        let prefix = super::super::query_budget::response_payload(Ok(prefix), &current, true);
        let maximum = packet(&full).len();
        assert!(packet(&prefix).len() > maximum);
        let result = execute_projected(
            &source,
            &current,
            &request(2, &[]),
            Instant::now(),
            &|page| Ok(packet(page).len() <= maximum),
        )?;
        assert_eq!(result["returned"], 2);
        assert_eq!(result["truncated"], false);
        assert!(result["continuation"].is_null());
        Ok(())
    }

    #[test]
    fn projected_lab_source_admits_the_exact_canonical_byte_boundary() -> TestResult {
        let (source, mut current) = fixture(200, 0);
        let bytes = source.canonical_bytes().len() as u64;
        current.budget.max_bytes = bytes;
        let result =
            execute_projected(&source, &current, &request(1, &[]), Instant::now(), &|_| {
                Ok(true)
            })?;
        assert_eq!(
            result["source_work"],
            json!({"records":200, "canonical_bytes":bytes})
        );
        assert_eq!(result["returned"], 1);
        current.budget.max_bytes = bytes - 1;
        assert_eq!(
            execute_projected(
                &source,
                &current,
                &request(1, &[]),
                Instant::now(),
                &|_| Ok(true)
            )
            .map_err(|error| error.code),
            Err(ErrorCode::BudgetExceeded)
        );
        Ok(())
    }

    #[test]
    fn projected_lab_current_authority_and_overall_deadline_precede_publication() -> TestResult {
        let (source, mut current) = fixture(2, 0);
        current.anchor.tick = GameTick(20);
        current.grants[0].expires_at_tick = Some(source.tick);
        let calls = std::cell::Cell::new(0);
        let fits = |_: &Value| {
            calls.set(calls.get() + 1);
            Ok(true)
        };
        assert_eq!(
            execute_projected(&source, &current, &request(1, &[]), Instant::now(), &fits)
                .map_err(|error| error.code),
            Err(ErrorCode::CapabilityDenied)
        );
        current.grants[0].expires_at_tick = None;
        let expired = Instant::now()
            .checked_sub(Duration::from_millis(60_001))
            .ok_or("clock underflow")?;
        assert_eq!(
            execute_projected(&source, &current, &request(1, &[]), expired, &fits)
                .map_err(|error| error.code),
            Err(ErrorCode::BudgetExceeded)
        );
        assert_eq!(calls.get(), 0);
        Ok(())
    }
}
