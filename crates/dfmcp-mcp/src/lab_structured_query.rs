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
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::query_condition::Condition;

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

// Decode preflight from the original bytes with the same closed condition
// grammar as retained watches. A Value round trip would hide duplicate keys.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConditionRequest {
    mode: Mode,
    request: ConditionEnvelope,
    at: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConditionEnvelope {
    schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_anchor: Option<Value>,
    query: ConditionQuery,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ConditionQuery {
    ConditionEvaluation {
        condition: Condition,
        #[serde(skip_serializing_if = "Option::is_none")]
        failure_condition: Option<Condition>,
    },
}

impl Request {
    pub(super) fn is_projected_page(&self) -> bool {
        matches!(
            self.request["query"]["kind"].as_str(),
            Some(
                "entities" | "events" | "spatial_entities" | "related_entities" | "ranked_entities"
            )
        )
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
    let mut request: Request = serde_json::from_str(raw)
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
                | "condition_evaluation"
        )
    ) {
        return Err(invalid(
            "structured laboratory queries support only entities, inspect, traverse, dependencies, aggregate, search, events, spatial_entities, related_entities, ranked_entities, and condition_evaluation",
        ));
    }
    if request.request["query"]["kind"] == "condition_evaluation" {
        let parsed: ConditionRequest = serde_json::from_str(raw)
            .map_err(|error| invalid(format!("invalid laboratory condition preflight: {error}")))?;
        let Mode::Structured = parsed.mode;
        request.at = parsed.at;
        request.request = serde_json::to_value(parsed.request)
            .map_err(|_| invalid("laboratory condition preflight cannot be encoded"))?;
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
        Some("condition_evaluation") => super::semantic_query::evaluate_laboratory_condition(
            snapshot,
            &source_context,
            &request.request,
        )?,
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
pub(super) fn orientation(
    source: StateAnchor,
    current: &OperationContext,
    request: &Request,
) -> Value {
    json!({"ok":true, "mode":"structured", "kind":request.request["query"]["kind"],
        "session_id":current.session_id.to_string(), "anchor":core_query::anchor_json(source),
        "current_anchor":core_query::anchor_json(current.anchor), "game_tick":source.tick.0,
        "historical":source != current.anchor})
}

/// Fit each whole selected page after adding the exact lab wrapper and the
/// caller's immutable Agent Turn. Source admission retains its own allowance.
pub(super) fn execute_projected(
    snapshot: &WorldSnapshot,
    current: &OperationContext,
    request: &Request,
    started: Instant,
    fits: &dyn Fn(&Value) -> Result<bool>,
) -> Result<Value> {
    let execute_page = match request.request["query"]["kind"].as_str() {
        Some("entities") => core_query::execute_with_page_budget,
        Some("events") => event_query::execute_with_page_budget,
        Some("spatial_entities") => spatial_entity_query::execute_with_page_budget,
        Some("related_entities") => related_entity_query::execute_with_page_budget,
        Some("ranked_entities") => ranked_entity_query::execute_with_page_budget,
        _ => {
            return Err(invalid(
                "projected publication requires a supported paged query",
            ));
        }
    };
    current.authorize(Capability::Query, RiskTier::ReadOnly, &[], None)?;
    let mut source_context = current.clone();
    source_context.budget.max_wall_millis = remaining_wall_millis(current, started)?;
    let source_work = super::query_budget::admit_sources(&[snapshot], &source_context)?;
    source_context.budget.max_wall_millis = remaining_wall_millis(current, started)?;
    source_context.anchor = snapshot.anchor();
    let result = execute_page(
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
        CapabilityGrant, CapabilityScope, Digest32, EdgeId, EntityId, EventId, FortressId,
        GameTick, MapCoord, ObservationCursor, RequestId, SessionId, WorkBudget,
    };
    use dfmcp_world::{
        EdgeKind, EdgeRecord, EntityKind, EntityRecord, Fact, FactSource, Value as WorldValue,
        WorldEvent, WorldEventKind, WorldGraph,
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

    fn page_family_fixture(count: u64, selected_bytes: usize) -> (WorldSnapshot, OperationContext) {
        let (mut source, mut current) = fixture(count, 16_384);
        for entity in source.graph.entities.values_mut() {
            for (name, value) in [
                ("small", WorldValue::Text("s".repeat(selected_bytes))),
                ("position", WorldValue::Coord(MapCoord::new(1, 1, 1))),
                ("rank", WorldValue::U64(entity.id.get())),
            ] {
                entity.fields.insert(
                    name.to_owned(),
                    Fact::known(value, source.tick, FactSource::Replay, Digest32::ZERO),
                );
            }
            let edge = EdgeId::new(u128::from(entity.id.get()));
            source.graph.edges.insert(
                edge,
                EdgeRecord {
                    id: edge,
                    revision: 1,
                    kind: EdgeKind::Requires,
                    from: entity.id,
                    to: entity.id,
                    fields: BTreeMap::new(),
                },
            );
            let event = EventId::new(u128::from(entity.id.get()));
            source.graph.events.insert(
                event,
                WorldEvent {
                    id: event,
                    tick: source.tick,
                    kind: WorldEventKind::UnitChanged,
                    subject: Some(entity.id),
                    summary: "recorded change".to_owned(),
                    fields: BTreeMap::from([(
                        "small".to_owned(),
                        WorldValue::Text("s".repeat(selected_bytes)),
                    )]),
                },
            );
        }
        source.refresh_hash();
        current.anchor = source.anchor();
        (source, current)
    }

    fn family_request(kind: &str, limit: usize) -> Request {
        let query_kind = match kind {
            "ranked_related_entities" | "ranked_spatial_entities" | "ranked_compound_entities" => {
                "ranked_entities"
            }
            other => other,
        };
        let mut query = json!({"kind":query_kind, "fields":["small"], "limit":limit});
        if kind != "events" {
            query["kinds"] = json!(["unit"]);
        }
        match kind {
            "spatial_entities" => {
                query["region"] = json!({"min":[0,0,0],"max":[2,2,2]});
                query["geometry"] = json!({"kind":"point","field":"position"});
            }
            "related_entities" => {
                query["relation"] = json!({"edge_kinds":["requires"],"target_kinds":["unit"]});
            }
            "ranked_entities"
            | "ranked_related_entities"
            | "ranked_spatial_entities"
            | "ranked_compound_entities" => {
                query["order_by"] =
                    json!([{"field":"rank","numeric_type":"u64","direction":"ascending"}]);
                if matches!(kind, "ranked_related_entities" | "ranked_compound_entities") {
                    query["relation"] =
                        json!({"edge_kinds":["requires"],"target_kinds":["unit"]});
                }
                if matches!(kind, "ranked_spatial_entities" | "ranked_compound_entities") {
                    query["spatial"] = json!({
                        "region":{"min":[0,0,0],"max":[2,2,2]},
                        "geometry":{"kind":"point","field":"position"}
                    });
                }
            }
            _ => {}
        }
        Request {
            mode: Mode::Structured,
            at: None,
            request: json!({"schema":"dfmcp.query/1","query":query}),
        }
    }

    const PAGE_FAMILIES: [&str; 7] = [
        "spatial_entities",
        "related_entities",
        "ranked_entities",
        "ranked_related_entities",
        "ranked_spatial_entities",
        "ranked_compound_entities",
        "events",
    ];

    #[test]
    fn projected_query_families_fit_complete_packets_and_resume_without_losing_rows() -> TestResult
    {
        for kind in PAGE_FAMILIES {
            let (source, mut current) = page_family_fixture(8, 1024);
            current.budget.max_output_tokens = 32_768;
            let one = execute_projected(
                &source,
                &current,
                &family_request(kind, 1),
                Instant::now(),
                &|_| Ok(true),
            )?;
            let one = super::super::query_budget::response_payload(Ok(one), &current, true);
            current.budget.max_output_tokens =
                u32::try_from((packet(&one).len() + 128).div_ceil(4))?;
            let mut selected = family_request(kind, 8);
            assert!(selected.is_projected_page());
            let mut ids = Vec::new();
            for index in 0..8 {
                let result =
                    execute_projected(&source, &current, &selected, Instant::now(), &|page| {
                        super::super::query_budget::lossless_page_fits(
                            &packet(page),
                            &page["query_budget"],
                        )
                    })?;
                let complete =
                    super::super::query_budget::response_payload(Ok(result), &current, true);
                let encoded =
                    super::super::query_budget::finish_response(packet(&complete), Duration::ZERO);
                let page: Value = serde_json::from_str(&encoded)?;
                assert_eq!(page["ok"], true, "{kind}: {page}");
                assert!(encoded.len() <= current.budget.max_output_tokens as usize * 4);
                let rows = page["rows"].as_array().ok_or("rows missing")?;
                assert!(!rows.is_empty());
                if matches!(kind, "ranked_related_entities" | "ranked_compound_entities") {
                    for row in rows {
                        assert_eq!(row["relations"]["matched_edges"], 1);
                        assert_eq!(row["relations"]["witnesses_returned"], 1);
                        assert_eq!(row["ranking"]["all_keys_usable"], true);
                    }
                }
                if matches!(kind, "ranked_spatial_entities" | "ranked_compound_entities") {
                    for row in rows {
                        assert_eq!(row["geometry"]["facts"]["position"]["presence"], "known");
                        assert_eq!(row["geometry"]["min"], json!([1, 1, 1]));
                        assert_eq!(row["ranking"]["all_keys_usable"], true);
                    }
                }
                if index == 0 {
                    assert!(rows.len() < 8, "{kind}");
                }
                let id_field = if kind == "events" {
                    "event_id"
                } else {
                    "entity_id"
                };
                ids.extend(rows.iter().map(|row| row[id_field].clone()));
                assert_eq!(
                    page["observation_coverage"]["continuation"],
                    page["continuation"]
                );
                assert_eq!(
                    page["agent_turn"]["coverage"]["continuation"],
                    page["continuation"]
                );
                assert_eq!(
                    page["source_work"]["canonical_bytes"],
                    source.canonical_bytes().len()
                );
                if page["continuation"].is_null() {
                    break;
                }
                selected.request["query"]["continuation"] = page["continuation"].clone();
            }
            let expected = (1..=8)
                .map(|id| {
                    json!(if kind == "events" {
                        EventId::new(id).to_string()
                    } else {
                        id.to_string()
                    })
                })
                .collect::<Vec<_>>();
            assert_eq!(ids, expected, "{kind}");
        }
        Ok(())
    }

    #[test]
    fn projected_query_families_do_not_skip_a_required_oversized_row() -> TestResult {
        for kind in PAGE_FAMILIES {
            let (mut source, mut current) = page_family_fixture(3, 256);
            source
                .graph
                .entities
                .get_mut(&EntityId::new(2))
                .ok_or("entity missing")?
                .fields
                .insert(
                    "small".to_owned(),
                    Fact::known(
                        WorldValue::Text("large".repeat(4096)),
                        source.tick,
                        FactSource::Replay,
                        Digest32::ZERO,
                    ),
                );
            source
                .graph
                .events
                .get_mut(&EventId::new(2))
                .ok_or("event missing")?
                .fields
                .insert("small".to_owned(), WorldValue::Text("large".repeat(4096)));
            source.refresh_hash();
            current.anchor = source.anchor();
            current.budget.max_output_tokens = 32_768;
            let one = execute_projected(
                &source,
                &current,
                &family_request(kind, 1),
                Instant::now(),
                &|_| Ok(true),
            )?;
            let one = super::super::query_budget::response_payload(Ok(one), &current, true);
            let maximum = packet(&one).len() + 128;
            let fits = |page: &Value| Ok(packet(page).len() <= maximum);
            let mut selected = family_request(kind, 3);
            let first = execute_projected(&source, &current, &selected, Instant::now(), &fits)?;
            assert_eq!(first["returned"], 1, "{kind}: {first}");
            selected.request["query"]["continuation"] = first["continuation"].clone();
            assert_eq!(
                execute_projected(&source, &current, &selected, Instant::now(), &fits)
                    .map_err(|error| error.code),
                Err(ErrorCode::BudgetExceeded),
                "{kind}"
            );
        }
        Ok(())
    }

    #[test]
    fn projected_query_families_consider_smaller_terminal_packets() -> TestResult {
        // Complete presentations may reference one continuation in several
        // places. Removing those references can outweigh one final small row.
        for kind in PAGE_FAMILIES {
            let (source, mut current) = page_family_fixture(2, 0);
            current.budget.max_output_tokens = 32_768;
            let full = execute_projected(
                &source,
                &current,
                &family_request(kind, 2),
                Instant::now(),
                &|_| Ok(true),
            )?;
            let prefix = execute_projected(
                &source,
                &current,
                &family_request(kind, 1),
                Instant::now(),
                &|_| Ok(true),
            )?;
            let full = super::super::query_budget::response_payload(Ok(full), &current, true);
            let prefix = super::super::query_budget::response_payload(Ok(prefix), &current, true);
            // Combined rows carry more mandatory witnesses. Construct the same
            // terminal-smaller case from exact encoded sizes for every family.
            assert!(prefix["continuation"].is_string());
            let saved_per_reference = prefix["continuation"].to_string().len() - 4;
            let references = full
                .to_string()
                .len()
                .saturating_sub(prefix.to_string().len())
                / saved_per_reference
                + 2;
            let render = |page: &Value| {
                json!({"page":page,
                    "resume":vec![page["continuation"].clone(); references]})
                .to_string()
            };
            let maximum = render(&full).len();
            assert!(render(&prefix).len() > maximum, "{kind}");
            let result = execute_projected(
                &source,
                &current,
                &family_request(kind, 2),
                Instant::now(),
                &|page| Ok(render(page).len() <= maximum),
            )?;
            assert_eq!(result["returned"], 2, "{kind}: {result}");
            assert_eq!(result["truncated"], false);
            assert!(result["continuation"].is_null());
        }
        Ok(())
    }

    #[test]
    fn projected_query_families_bound_empty_packets_and_propagate_fit_errors() -> TestResult {
        for count in [0, 2] {
            for kind in PAGE_FAMILIES {
                let (source, current) = page_family_fixture(count, 0);
                let selected = family_request(kind, 2);
                assert_eq!(
                    execute_projected(&source, &current, &selected, Instant::now(), &|_| Ok(false))
                        .map_err(|error| error.code),
                    Err(ErrorCode::BudgetExceeded),
                    "{kind}/{count}"
                );
                assert_eq!(
                    execute_projected(&source, &current, &selected, Instant::now(), &|_| Err(
                        invalid("publication unavailable")
                    ))
                    .map_err(|error| error.message),
                    Err("publication unavailable".to_owned()),
                    "{kind}/{count}"
                );
            }
        }
        Ok(())
    }
}
