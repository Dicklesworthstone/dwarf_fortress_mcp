//! Enforce the session's negotiated `max_output_tokens` on every response.
//!
//! A negotiated budget that responses ignore is no budget. Each finished tool
//! response is measured with the conservative estimator used across the
//! laboratory (4 bytes per token) and, when it does not fit, degraded in
//! deterministic tiers until it does, always remaining one valid JSON object:
//!
//! 1. `compact_turn` — the Agent Turn keeps its meaning but drops null fields
//!    and disabled affordances, and slims affordance records;
//! 2. `truncated` — the largest payload arrays (outside the Agent Turn) are
//!    halved, largest first, ties by path; each truncation records the path,
//!    the items returned and the total, so the agent can page for the rest;
//! 3. `sections_omitted` — optional bulky payload sections (live routing,
//!    forecasts, world briefings, step detail) are replaced by an omission
//!    marker, least essential first;
//! 4. `minimal` — only the outcome, error, session, anchor, active work, the
//!    top recommendation and the budget record survive.
//! A published goal withdrawal has one final receipt tier: its exact outcome
//! and certificate remain complete while oversized orientation is explicitly
//! omitted with a read-only inspection step. Cancellation likewise retains its
//! exact pause/drain outcome, including partial failures and absent certificates.
//!
//! Every response carries `output_budget` with the estimate and the tier.

use serde_json::{Map, Value, json};

/// Conservative bytes per output token (shared laboratory estimator).
pub(crate) const BYTES_PER_TOKEN: usize = 4;
/// Budget applied when no session budget is known.
pub(crate) const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 1_500;

fn estimate_tokens(text: &str) -> u64 {
    text.len().div_ceil(BYTES_PER_TOKEN) as u64
}

fn strip_nulls(value: &mut Value) {
    // Only presentation defaults at this object level may be removed. Nested
    // nulls can be known values in evidence or exact query/action arguments.
    if let Value::Object(map) = value {
        map.retain(|_, v| !v.is_null());
    }
}

/// World alerts carry one compact source witness, not an arbitrary proof dump.
/// Preserve its complete source and epistemic class through both cheap projections;
/// an unexpected larger proof is explicitly omitted rather than sliced.
const MAX_COMPACT_ALERT_EVIDENCE_BYTES: usize = 512;

fn alert_qualification(item: &Value) -> Option<(Value, Value)> {
    if !item["attention_id"]
        .as_str()
        .is_some_and(|id| id.starts_with("world-"))
    {
        return None;
    }
    let epistemic = item["confidence"]["epistemic_state"].as_str()?;
    if !matches!(epistemic, "unknown" | "certified_derived") {
        return None;
    }
    let evidence = match item["evidence"].as_array().map(Vec::as_slice) {
        Some([]) => json!([]),
        Some([witness])
            if witness["scope"] == "laboratory_reference_world"
                && witness["source_anchor"].is_object()
                && witness.to_string().len() <= MAX_COMPACT_ALERT_EVIDENCE_BYTES =>
        {
            json!([witness])
        }
        _ => json!([{"omitted_for_profile":true}]),
    };
    Some((json!({"epistemic_state":epistemic,"value":null}), evidence))
}

/// A completed stage is not necessarily a completed original objective. Keep
/// the current project/output truth, reserve prediction, historical scope and
/// next-step eligibility together whenever an objective remains visible.
/// These are copied from the authorized observation, never inferred from its
/// historical `status`. Bulky source programs and predicates stay in the full
/// objective projection instead of being repeated in the compact briefing.
fn compact_objective(item: &mut Value) {
    let Some(object) = item.as_object_mut() else {
        return;
    };
    object.retain(|key, _| {
        matches!(
            key.as_str(),
            "plan_digest"
                | "summary"
                | "status"
                | "predicate_truth"
                | "epistemic_state"
                | "evidence_scope"
                | "observation_error"
                | "observed_anchor"
                | "goal_scope"
                | "historical_achievement_scope"
                | "historical_achievement"
                | "first_satisfied_anchor"
                | "restore_abandoned_anchor"
                | "withdrawal"
                | "archived"
                | "retained_reserve_endpoint"
                | "abandonment_publication_pending"
                | "semantic_room_goal"
                | "semantic_output_goal"
                | "resource_reserve"
                | "production_reserve"
                | "physical_quiescent"
                | "pursuit_active"
                | "needs_replan"
                | "continuation_request"
                | "continuation_refused"
                | "replacement_work_dispatched"
                | "blind_retry_allowed"
        )
    });
    object.insert("detail_omitted_for_output_budget".to_owned(), json!(true));
}

fn compact_turn(turn: &mut Value) {
    if let Some(affordances) = turn.get_mut("affordances").and_then(Value::as_array_mut) {
        affordances.retain(|a| a["enabled"] != false);
        for affordance in affordances.iter_mut() {
            *affordance = json!({
                "affordance_id": affordance["affordance_id"],
                "tool": affordance["tool"],
            });
        }
    }
    if let Some(items) = turn.get_mut("uncertainty").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            *item = json!({"uncertainty_id": item["uncertainty_id"]});
        }
    }
    if let Some(items) = turn
        .get_mut("recommendations")
        .and_then(Value::as_array_mut)
    {
        for item in items.iter_mut() {
            let risk = item.get("risk").cloned();
            let reversibility = item.get("reversibility").cloned();
            let requires_checkpoint = item.get("requires_checkpoint").and_then(Value::as_bool);
            *item = json!({
                "recommendation_id": item["recommendation_id"],
                "tool": item["tool"],
                "arguments": item["arguments"],
                "reason": item["reason"],
                "requires_confirmation": item["requires_confirmation"],
            });
            if let Some(risk) = risk {
                item["risk"] = risk;
            }
            if let Some(reversibility) = reversibility {
                item["reversibility"] = reversibility;
            }
            if let Some(required) = requires_checkpoint {
                item["requires_checkpoint"] = json!(required);
            }
        }
    }
    if let Some(items) = turn.get_mut("attention").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            let qualification = alert_qualification(item);
            *item = json!({
                "attention_id": item["attention_id"],
                "severity": item["severity"],
                "finding": item["finding"],
                "remedy": item["remedy"],
            });
            if let Some((confidence, evidence)) = qualification {
                item["confidence"] = confidence;
                item["evidence"] = evidence;
            }
        }
    }
    if let Some(items) = turn
        .get_mut("briefing")
        .and_then(|briefing| briefing.get_mut("objective_status"))
        .and_then(Value::as_array_mut)
    {
        for item in items.iter_mut() {
            compact_objective(item);
        }
    }
    if let Some(budget) = turn.get_mut("budget") {
        *budget = json!({"admitted": budget["admitted"]});
    }
    // Coverage and continuations are part of the truth of the response. A
    // smaller envelope must retain what an empty or partial result proves.
    strip_nulls(turn);
}

/// Agent Turn sections a profile does not carry, per
/// `architecture/agent_turn_contract.json` (`required_sections`).
const PULSE_OMITS: [&str; 3] = ["briefing", "affordances", "references"];

fn retained_query_catalog(key: &str) -> bool {
    matches!(
        key,
        "query_baselines" | "query_watches" | "condition_watches"
    )
}

/// Shape a response to its observation profile before budgeting. Profiles are
/// semantic contracts: a pulse is the cheapest safe heartbeat, so it carries
/// identity, continuity, changes, attention, active work, recommendations and
/// compact uncertainty/coverage/budget, and names what it left out. Every
/// Agent Turn key stays present so the contract's field order holds.
pub(crate) fn shape_for_profile(response: &str, profile: &str) -> String {
    if profile != "pulse" && profile != "briefing" {
        return response.to_owned();
    }
    let Ok(mut shaped) = serde_json::from_str::<Value>(response) else {
        return response.to_owned();
    };
    if profile == "briefing" {
        // A briefing keeps every affordance but elides default-valued fields:
        // registered confirmation/checkpoint policy, an all-unknown cost
        // estimate, empty precondition lists and a null disabled reason.
        if let Some(turn) = shaped.get_mut("agent_turn").filter(|t| t.is_object()) {
            if let Some(affordances) = turn.get_mut("affordances").and_then(Value::as_array_mut) {
                for affordance in affordances.iter_mut() {
                    if let Some(map) = affordance.as_object_mut() {
                        map.retain(|key, value| match key.as_str() {
                            "checkpoint_policy" | "confirmation_policy" => {
                                value != "registered_policy"
                            }
                            "estimated_cost" => value
                                .as_object()
                                .is_none_or(|cost| cost.values().any(|v| !v.is_null())),
                            _ => !value.is_null() && !value.as_array().is_some_and(Vec::is_empty),
                        });
                    }
                }
            }
            turn["coverage"]["defaults_elided"] =
                json!("affordance fields at their registered defaults are omitted");
        }
        return shaped.to_string();
    }
    if let Some(turn) = shaped.get_mut("agent_turn").filter(|t| t.is_object()) {
        // Verification state stays: objective status is what a pulse verifies.
        turn["briefing"] = json!({"objective_status": turn["briefing"]["objective_status"]});
        turn["affordances"] = json!([]);
        turn["references"] = json!([]);
        if let Some(items) = turn.get_mut("uncertainty").and_then(Value::as_array_mut) {
            for item in items.iter_mut() {
                *item = json!({"uncertainty_id": item["uncertainty_id"]});
            }
        }
        turn["budget"] = json!({"admitted": turn["budget"]["admitted"]});
        // Unbroken continuity needs no basis: it is the agent's previous anchor.
        if turn["continuity"]["status"] == "continuous"
            && let Some(continuity) = turn.get_mut("continuity").and_then(Value::as_object_mut)
        {
            continuity.remove("basis");
        }
        // Only active work that exists is listed.
        if let Some(work) = turn.get_mut("active_work").and_then(Value::as_object_mut) {
            work.retain(|key, v| {
                retained_query_catalog(key) || !v.as_array().is_some_and(Vec::is_empty)
            });
        }
        // Attention and recommendations keep what to act on, not boilerplate.
        if let Some(items) = turn.get_mut("attention").and_then(Value::as_array_mut) {
            for item in items.iter_mut() {
                let qualification = alert_qualification(item);
                if let Some(map) = item.as_object_mut() {
                    map.retain(|key, _| {
                        matches!(
                            key.as_str(),
                            "attention_id"
                                | "category"
                                | "severity"
                                | "urgency"
                                | "finding"
                                | "remedy"
                                | "surprise"
                                | "subjects"
                        )
                    });
                    if let Some((confidence, evidence)) = qualification {
                        map.insert("confidence".to_owned(), confidence);
                        map.insert("evidence".to_owned(), evidence);
                    }
                }
            }
        }
        if let Some(items) = turn
            .get_mut("recommendations")
            .and_then(Value::as_array_mut)
        {
            for item in items.iter_mut() {
                if let Some(map) = item.as_object_mut() {
                    map.retain(|key, _| {
                        matches!(
                            key.as_str(),
                            "recommendation_id"
                                | "tool"
                                | "arguments"
                                | "reason"
                                | "risk"
                                | "reversibility"
                                | "requires_confirmation"
                                | "requires_checkpoint"
                        )
                    });
                }
            }
        }
        // Changes keep what changed; the anchors they span are in continuity.
        if let Some(changes) = turn.get_mut("changes").and_then(Value::as_array_mut) {
            for change in changes.iter_mut() {
                if let Some(map) = change.as_object_mut() {
                    map.remove("basis");
                    map.retain(|_, v| !v.as_array().is_some_and(Vec::is_empty));
                }
            }
        }
        if !turn["coverage"].is_object() {
            turn["coverage"] = json!({});
        }
        turn["coverage"]["omitted_by_profile"] = json!(PULSE_OMITS);
        strip_nulls(turn);
    }
    shaped.to_string()
}

/// Payload sections that can be re-requested or recomputed, least essential
/// first. The outcome, identifiers and Agent Turn are never in this list.
const OPTIONAL_SECTIONS: [&str; 10] = [
    "live_routing",
    "forecast",
    "objectives",
    "world",
    "replay",
    "rebased_plan",
    "polled_actions",
    "levels",
    "rows",
    "steps",
];

/// Only whole presentation records may be removed. Arrays inside a field,
/// predicate, action, coordinate, evidence chain or query are semantic values;
/// shortening them would invent a different known fact or plan.
fn arrays(value: &Value, path: &mut Vec<String>, out: &mut Vec<(String, usize)>, skip_turn: bool) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if path.is_empty() && ((skip_turn && key == "agent_turn") || key == "output_budget")
                {
                    continue;
                }
                path.push(key.clone());
                arrays(child, path, out, skip_turn);
                path.pop();
            }
        }
        Value::Array(items) => {
            let removable = match path.as_slice() {
                [name] => matches!(
                    name.as_str(),
                    "rows" | "levels" | "hits" | "polled_actions" | "steps"
                ),
                [parent, name] if parent == "world" => {
                    matches!(name.as_str(), "dwarves" | "active_work")
                }
                [turn, work, name]
                    if !skip_turn && turn == "agent_turn" && work == "active_work" =>
                {
                    // These bounded owner catalogs carry explicit completeness.
                    // Keep every handle or refuse; shortening them would hide
                    // retained work while still reporting an available catalog.
                    !retained_query_catalog(name)
                }
                [turn, name] if !skip_turn && turn == "agent_turn" && name == "recommendations" => {
                    true
                }
                _ => false,
            };
            if removable && items.len() > 1 {
                out.push((path.join("."), value.to_string().len()));
            }
            for (index, child) in items.iter().enumerate() {
                path.push(index.to_string());
                arrays(child, path, out, skip_turn);
                path.pop();
            }
        }
        _ => {}
    }
}

fn at_path<'a>(value: &'a mut Value, path: &str) -> Option<&'a mut Value> {
    let mut current = value;
    for segment in path.split('.').filter(|s| !s.is_empty()) {
        current = match current {
            Value::Object(map) => map.get_mut(segment)?,
            Value::Array(items) => items.get_mut(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current)
}

fn with_record(mut payload: Value, record: &Value) -> String {
    if let Some(object) = payload.as_object_mut() {
        object.insert("output_budget".to_owned(), record.clone());
    }
    payload.to_string()
}

fn fits(payload: &Value, record: &Value, budget: u64) -> Option<String> {
    let text = with_record(payload.clone(), record);
    (estimate_tokens(&text) <= budget).then_some(text)
}

fn partial_coverage(coverage: &mut Value) {
    if !coverage.is_object() {
        *coverage = json!({});
    }
    let mut partial = coverage["partial_domains"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if let Some(complete) = coverage["complete_domains"].as_array() {
        for domain in complete {
            if !partial.contains(domain) {
                partial.push(domain.clone());
            }
        }
    }
    coverage["status"] = json!("partial");
    coverage["partial_domains"] = json!(partial);
    coverage["complete_domains"] = json!([]);
    coverage["absence_proof_scope"] = json!([]);
    coverage["page_complete"] = json!(false);
    coverage["omitted_for_output_budget"] = json!(true);
}

/// Keep the page cursor consistent with the records the client actually got,
/// and withdraw response-level completeness after any presentation omission.
fn record_omission(payload: &mut Value, path: &str) {
    if payload.get("complete_domain").is_some() {
        payload["complete_domain"] = json!(false);
    }
    if payload.get("absence_proven").is_some() {
        payload["absence_proven"] = json!(false);
    }
    payload["truncated"] = json!(true);
    if let Some(coverage) = payload.get_mut("observation_coverage") {
        partial_coverage(coverage);
    }
    if let Some(turn) = payload
        .get_mut("agent_turn")
        .filter(|turn| turn.is_object())
    {
        if !turn["coverage"].is_object() {
            turn["coverage"] = json!({});
        }
        partial_coverage(&mut turn["coverage"]);
    }
    if path != "rows" || !matches!(payload["mode"].as_str(), Some("entities" | "observation")) {
        return;
    }
    let returned = payload["rows"].as_array().map_or(0, Vec::len) as u64;
    let offset = payload["offset"].as_u64().unwrap_or(0);
    let end = offset.saturating_add(returned);
    payload["returned"] = json!(returned);
    let more = payload["total"].as_u64().is_some_and(|total| end < total)
        && payload["section_included"].as_bool() != Some(false);
    if !more {
        payload["next_offset"] = Value::Null;
        payload["continuation"] = Value::Null;
        if let Some(coverage) = payload.get_mut("observation_coverage") {
            coverage["continuation"] = Value::Null;
        }
        if let Some(turn) = payload
            .get_mut("agent_turn")
            .filter(|turn| turn.is_object())
        {
            turn["coverage"]["continuation"] = Value::Null;
        }
        return;
    }
    if let Some(mut next) = payload
        .get("query")
        .filter(|query| query.is_object())
        .cloned()
    {
        next["offset"] = json!(end);
        next["limit"] = json!(returned.max(1));
        payload["next_offset"] = json!(end);
        payload["continuation"] = next.clone();
        if let Some(coverage) = payload.get_mut("observation_coverage") {
            coverage["continuation"] = next.clone();
        }
        if let Some(turn) = payload
            .get_mut("agent_turn")
            .filter(|turn| turn.is_object())
        {
            turn["coverage"]["continuation"] = next;
        }
    }
}

/// Cancellation may already have paused or stopped physical work. Preserve the
/// exact selected outcome independently of optional per-step detail and session
/// orientation. In particular, a successful request is not proof of quiescence.
fn cancellation_outcome(payload: &Value) -> Option<Map<String, Value>> {
    if payload["agent_turn"]["operation"] != "fortress.cancel"
        || !payload["drain_progress"].is_object()
        || !payload["observed_anchor"].is_object()
        || !payload["paused"].is_boolean()
        || !payload["ok"].is_boolean()
    {
        return None;
    }
    let mut out = Map::new();
    for key in [
        "ok",
        "error",
        "session_id",
        "scope",
        "action_id",
        "requested_state",
        "final_state",
        "work_state",
        "physical_drain",
        "requested_plan_digest",
        "plan_digest",
        "plan_id",
        "selection_scope",
        "descendant_plans_included",
        "drain_progress",
        "paused",
        "emergency_pause",
        "finalize_certificate",
        "observed_anchor",
        "durability_pending",
    ] {
        if let Some(value) = payload.get(key) {
            out.insert(key.to_owned(), value.clone());
        }
    }
    Some(out)
}

fn minimal(payload: &Value) -> Value {
    let mut out = Map::new();
    for key in [
        "ok",
        "error",
        "session_id",
        "plan_digest",
        "checkpoint_id",
        "action_id",
    ] {
        if let Some(value) = payload.get(key) {
            out.insert(key.to_owned(), value.clone());
        }
    }
    // A selected wait may already have advanced the shared world. Preserve its
    // exact stop decision and elapsed work as one unit at every output tier;
    // falling back to the incidental latest action would change the answer.
    if payload.get("selected_plan_wait").is_some()
        || payload.get("selected_watch_wait").is_some()
        || payload.get("selected_watch_set_wait").is_some()
    {
        for key in [
            "selected_plan_wait",
            "selected_watch_wait",
            "selected_watch_set_wait",
            "wait_progress",
            "advanced_game_ticks",
            "game_tick",
            "observed_anchor",
            "paused",
            "partial_progress",
            "durability_pending",
        ] {
            if let Some(value) = payload.get(key) {
                out.insert(key.to_owned(), value.clone());
            }
        }
    }
    if let Some(outcome) = cancellation_outcome(payload) {
        out.extend(outcome);
    }
    // Withdrawal has already changed pursuit custody. Its exact historical
    // disposition cannot disappear into a generic successful cancellation.
    if payload.get("withdrawal").is_some_and(Value::is_object) {
        for key in [
            "status",
            "selection_scope",
            "descendant_plans_included",
            "already_withdrawn",
            "withdrawal",
            "archived",
            "physical_work_changed",
            "goal_success_claimed",
            "active_objectives",
            "archived_objectives",
            "active_objective_limit",
            "archive_limit",
            "observed_anchor",
        ] {
            if let Some(value) = payload.get(key) {
                out.insert(key.to_owned(), value.clone());
            }
        }
    }
    let turn = &payload["agent_turn"];
    out.insert(
        "agent_turn".to_owned(),
        json!({
            "schema": turn["schema"],
            "operation": turn["operation"],
            "anchor": turn["anchor"],
            "continuity": {"status": turn["continuity"]["status"]},
            "coverage": turn["coverage"],
            "active_work": turn["active_work"],
            "recommendations": turn["recommendations"].as_array().map(|r| r.iter().take(1).cloned().collect::<Vec<_>>()),
        }),
    );
    Value::Object(out)
}

/// Withdrawal publishes an immutable disposition before the facade attaches
/// orientation. A single retained replan recommendation may contain a large
/// original blueprint, so even the ordinary minimal turn need not fit the
/// receipt's pre-admitted envelope. Keep the whole receipt and explicitly defer
/// orientation instead of reporting failure after the archive was published.
///
/// The fixed turn and budget record add less than the 2,048 bytes reserved by
/// `objective_withdrawal::admit_receipt`, including maximum-width canonical
/// anchors and session IDs. No source text, work catalog, or truncation trace
/// is copied into this final tier.
fn withdrawal_receipt(payload: &Value, budget: u64, original: u64) -> Option<String> {
    if payload["ok"] != true
        || payload["status"] != "withdrawn"
        || payload["withdrawal"]["schema"] != "dfmcp.original-goal-withdrawal/1"
        || payload["archived"] != true
        || payload["physical_work_changed"] != false
        || payload["goal_success_claimed"] != false
    {
        return None;
    }
    let status = match payload["agent_turn"]["continuity"]["status"].as_str() {
        Some(
            status @ ("bootstrap" | "continuous" | "heartbeat" | "partial" | "gap" | "reset"
            | "stale" | "indeterminate"),
        ) => status,
        _ => "indeterminate",
    };
    let next = json!({
        "tool":"fortress.observe",
        "arguments":{"session_id":payload["session_id"],"profile":"briefing"},
    });
    let mut small = minimal(payload);
    small["truncated"] = json!(true);
    small["agent_turn"] = json!({
        "schema":"dfmcp.agent_turn/1", "operation":"fortress.cancel", "phase":"reconcile",
        "anchor":payload["observed_anchor"],
        "continuity":{"status":status,"detail_omitted_for_output_budget":true},
        "profile":"tactical",
        "active_work":{"status":"omitted_for_output_budget","absence_proven":false},
        "recommendations":[{
            "tool":next["tool"], "arguments":next["arguments"],
            "reason":"withdrawal succeeded; inspect retained work before further action",
            "risk":"read_only", "reversibility":"not_applicable", "requires_confirmation":false,
        }],
        "coverage":{
            "status":"partial", "complete_domains":["original_goal_withdrawal_receipt"],
            "absence_proof_scope":[], "omitted_domains":["session_orientation"],
            "continuation":next,
        },
    });
    let record = json!({
        "max_output_tokens":budget, "full_estimate_tokens":original,
        "estimator":"ceil(bytes / 4)", "tier":"withdrawal_receipt",
        "note":"complete withdrawal receipt; session orientation omitted for output budget",
    });
    fits(&small, &record, budget)
}

/// Large singleton recommendations can prevent even the ordinary minimal
/// orientation from fitting after cancellation. Report the exact pause and
/// drain result, including failures, without claiming that omitted work is
/// absent or that a paused world has been drained. Historical step detail is
/// explicitly omitted; the read-only follow-up inspects current work only.
fn cancellation_receipt(payload: &Value, budget: u64, original: u64) -> Option<String> {
    let outcome = cancellation_outcome(payload)?;
    let status = match payload["agent_turn"]["continuity"]["status"].as_str() {
        Some(
            status @ ("bootstrap" | "continuous" | "heartbeat" | "partial" | "gap" | "reset"
            | "stale" | "indeterminate"),
        ) => status,
        _ => "indeterminate",
    };
    let next = json!({
        "tool":"fortress.observe",
        "arguments":{"session_id":payload["session_id"],"profile":"briefing"},
    });
    let mut small = Value::Object(outcome);
    small["truncated"] = json!(true);
    small["agent_turn"] = json!({
        "schema":"dfmcp.agent_turn/1", "operation":"fortress.cancel", "phase":"reconcile",
        "anchor":payload["observed_anchor"],
        "continuity":{"status":status,"detail_omitted_for_output_budget":true},
        "profile":"tactical",
        "active_work":{"status":"omitted_for_output_budget","absence_proven":false},
        "recommendations":[{
            "tool":next["tool"], "arguments":next["arguments"],
            "reason":"inspect current work after the reported cancellation outcome",
            "risk":"read_only", "reversibility":"not_applicable", "requires_confirmation":false,
        }],
        "coverage":{
            "status":"partial", "complete_domains":["cancellation_outcome"],
            "absence_proof_scope":[],
            "omitted_domains":["session_orientation","per_step_drain_evidence","untracked_work_detail"],
            "continuation":next,
        },
    });
    let record = json!({
        "max_output_tokens":budget, "full_estimate_tokens":original,
        "estimator":"ceil(bytes / 4)", "tier":"cancellation_receipt",
        "note":"exact pause/drain outcome; current-work inspection does not reconstruct omitted historical step detail",
    });
    fits(&small, &record, budget)
}

/// Fit one finished response into `max_output_tokens`.
#[must_use]
pub(crate) fn fit(response: &str, max_output_tokens: u64) -> String {
    let Ok(mut payload) = serde_json::from_str::<Value>(response) else {
        return response.to_owned();
    };
    if !payload.is_object() {
        return response.to_owned();
    }
    let original = estimate_tokens(response);
    let mut record = json!({
        "max_output_tokens": max_output_tokens,
        "full_estimate_tokens": original,
        "estimator": "ceil(bytes / 4)",
        "tier": "full",
    });
    if let Some(text) = fits(&payload, &record, max_output_tokens) {
        return text;
    }

    record["tier"] = json!("compact_turn");
    if let Some(turn) = payload.get_mut("agent_turn") {
        compact_turn(turn);
    }
    if let Some(text) = fits(&payload, &record, max_output_tokens) {
        return text;
    }

    record["tier"] = json!("truncated");
    record["note"] = json!(
        "arrays were shortened to fit the negotiated output budget; page with offset/limit or narrower regions, or open a session with a larger max_output_tokens"
    );
    let mut truncated: Map<String, Value> = Map::new();
    loop {
        let mut found = Vec::new();
        arrays(&payload, &mut Vec::new(), &mut found, true);
        // Largest first; ties by path, so the result is deterministic.
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let Some((path, _)) = found.into_iter().next() else {
            break;
        };
        let Some(Value::Array(items)) = at_path(&mut payload, &path) else {
            break;
        };
        let total = items.len();
        items.truncate(total / 2);
        let returned = items.len();
        let entry = truncated
            .entry(path.clone())
            .or_insert_with(|| json!({"total": total}));
        entry["returned"] = json!(returned);
        record["truncated"] = Value::Object(truncated.clone());
        record_omission(&mut payload, &path);
        if let Some(text) = fits(&payload, &record, max_output_tokens) {
            return text;
        }
    }

    record["tier"] = json!("sections_omitted");
    let mut omitted = Vec::new();
    for key in OPTIONAL_SECTIONS {
        if let Some(object) = payload.as_object_mut()
            && let Some(section) = object.get_mut(key)
            && !section.is_null()
        {
            *section = json!({"omitted_for_output_budget": true});
            omitted.push(key);
            record["omitted"] = json!(omitted);
            record_omission(&mut payload, key);
            if let Some(text) = fits(&payload, &record, max_output_tokens) {
                return text;
            }
        }
    }

    record["tier"] = json!("minimal");
    record_omission(&mut payload, "minimal_envelope");
    let mut small = minimal(&payload);
    if let Some(text) = fits(&small, &record, max_output_tokens) {
        return text;
    }
    // Shorten even the active-work lists, largest first, before refusing.
    loop {
        let mut found = Vec::new();
        arrays(&small, &mut Vec::new(), &mut found, false);
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let Some((path, _)) = found.into_iter().next() else {
            break;
        };
        let Some(Value::Array(items)) = at_path(&mut small, &path) else {
            break;
        };
        let total = items.len();
        items.truncate(total / 2);
        record["minimal_truncated"][path.as_str()] = json!({"total": total, "returned": total / 2});
        record_omission(&mut small, &path);
        if let Some(text) = fits(&small, &record, max_output_tokens) {
            return text;
        }
    }
    match fits(&small, &record, max_output_tokens) {
        Some(text) => text,
        None => {
            if let Some(receipt) = withdrawal_receipt(&payload, max_output_tokens, original) {
                return receipt;
            }
            if let Some(receipt) = cancellation_receipt(&payload, max_output_tokens, original) {
                return receipt;
            }
            // The budget is below even the minimal envelope: say so in the
            // smallest valid form rather than exceed it silently.
            json!({
                "ok": false,
                "error": {"code": "budget_exceeded", "message": "max_output_tokens is below the minimal response envelope"},
                "output_budget": {"max_output_tokens": max_output_tokens, "tier": "refused"},
            })
            .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn big_response() -> String {
        json!({
            "ok": true,
            "session_id": "s",
            "rows": (0..200).map(|i| json!({"entity_id": i, "label": format!("dwarf number {i}")})).collect::<Vec<_>>(),
            "levels": [{"rows": (0..48).map(|_| "#".repeat(48)).collect::<Vec<_>>()}],
            "agent_turn": {
                "schema": "dfmcp.agent_turn/1",
                "operation": "fortress.query",
                "anchor": {"state_hash": "ab"},
                "continuity": {"status": "continuous", "gap": null},
                "affordances": (0..10).map(|i| json!({"affordance_id": i, "tool": "fortress.observe", "arguments": {}, "risk": "read_only", "enabled": i % 2 == 0, "estimated_cost": {"actions": null}})).collect::<Vec<_>>(),
                "recommendations": [{"tool": "fortress.wait", "arguments": {}, "estimated_cost": {"x": null}}],
            },
        })
        .to_string()
    }

    #[test]
    fn every_budget_is_honoured_with_valid_json_and_monotone_tiers() {
        let response = big_response();
        let mut last_tier = 0;
        for budget in [100_000u64, 4_000, 2_000, 1_000, 500, 200, 120, 60, 10] {
            let fitted = fit(&response, budget);
            let parsed: Value = serde_json::from_str(&fitted).unwrap_or(Value::Null);
            assert!(parsed.is_object(), "budget {budget}: invalid JSON");
            let tier = parsed["output_budget"]["tier"]
                .as_str()
                .unwrap_or("?")
                .to_owned();
            if tier != "refused" {
                assert!(
                    estimate_tokens(&fitted) <= budget,
                    "budget {budget} exceeded by tier {tier}"
                );
            }
            let rank = [
                "full",
                "compact_turn",
                "truncated",
                "sections_omitted",
                "minimal",
                "refused",
            ]
            .iter()
            .position(|t| *t == tier)
            .unwrap_or(9);
            assert!(rank >= last_tier, "tiers only degrade as budgets shrink");
            last_tier = rank;
        }
    }

    #[test]
    fn truncation_records_paths_and_is_deterministic() {
        let response = big_response();
        let a = fit(&response, 900);
        let b = fit(&response, 900);
        assert_eq!(a, b);
        let parsed: Value = serde_json::from_str(&a).unwrap_or(Value::Null);
        assert_eq!(parsed["output_budget"]["tier"], "truncated", "{parsed}");
        let truncated = parsed["output_budget"]["truncated"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        assert!(truncated.contains_key("rows"), "{truncated:?}");
        assert_eq!(truncated["rows"]["total"], 200);
        let returned = truncated["rows"]["returned"].as_u64().unwrap_or(0) as usize;
        assert_eq!(parsed["rows"].as_array().map(Vec::len), Some(returned));
        // The agent turn survived intact enough to orient.
        assert_eq!(parsed["agent_turn"]["anchor"]["state_hash"], "ab");
    }

    #[test]
    fn fitting_responses_only_gain_the_budget_record() {
        let small = json!({"ok": true, "x": 1}).to_string();
        let parsed: Value = serde_json::from_str(&fit(&small, 1_500)).unwrap_or(Value::Null);
        assert_eq!(parsed["x"], 1);
        assert_eq!(parsed["output_budget"]["tier"], "full");
    }

    #[test]
    fn retained_query_catalogs_remain_complete_at_every_fitting_tier()
    -> Result<(), serde_json::Error> {
        let records: Vec<_> = (0..8)
            .map(|id| {
                json!({
                    "watch":format!("watch:{id}"), "label":"x".repeat(500),
                    "next_step":{"tool":"fortress.query","arguments":{"mode":"retained"}},
                })
            })
            .collect();
        for key in ["query_baselines", "query_watches", "condition_watches"] {
            let mut payload: Value = serde_json::from_str(&big_response())?;
            payload["agent_turn"]["active_work"] = json!({
                "query_work_coverage":{"status":"available"},
            });
            payload["agent_turn"]["active_work"][key] = json!(records);
            for budget in [20_000, 4_000, 2_000, 1_000, 500, 200] {
                let fitted: Value = serde_json::from_str(&fit(&payload.to_string(), budget))?;
                if fitted["output_budget"]["tier"] != "refused" {
                    assert_eq!(fitted["agent_turn"]["active_work"][key], json!(records));
                } else {
                    assert_eq!(fitted["ok"], false);
                }
            }
            payload["agent_turn"]["active_work"][key] = json!([]);
            let pulse: Value =
                serde_json::from_str(&shape_for_profile(&payload.to_string(), "pulse"))?;
            assert_eq!(pulse["agent_turn"]["active_work"][key], json!([]));
        }
        Ok(())
    }

    fn withdrawal_with_oversized_orientation(status: &str) -> (Value, Value) {
        let anchor = json!({
            "fortress_id":u128::MAX.to_string(), "game_tick":u64::MAX,
            "epoch":u64::MAX, "sequence":u64::MAX, "state_hash":"f".repeat(64),
        });
        let raw = json!({
            "ok":true, "session_id":"f".repeat(32), "plan_digest":"e".repeat(64),
            "status":"withdrawn", "selection_scope":"exact_original_goal",
            "descendant_plans_included":false, "already_withdrawn":false,
            "withdrawal":{
                "schema":"dfmcp.original-goal-withdrawal/1", "anchor":anchor,
                "requester_session_id":"f".repeat(32), "proof_digest":"d".repeat(64),
                "verification":"verified",
                "never_dispatched_steps":(u32::MAX - 63..=u32::MAX).collect::<Vec<_>>(),
                "physical_quiescent_at_withdrawal":true, "goal_success_claimed":false,
                "historical_evidence_retained":true, "dispatch_authority_restored":false,
            },
            "archived":true, "physical_work_changed":false, "goal_success_claimed":false,
            "active_objectives":64, "archived_objectives":256,
            "active_objective_limit":64, "archive_limit":256, "observed_anchor":anchor,
        });
        // A stale resource proposal retains its complete blueprint both in the
        // pending plan's safe next step and in the replan recommendation. Each
        // list has one element, so ordinary array fitting cannot shorten it.
        let next = json!({"tool":"fortress.plan","arguments":{
            "blueprint":format!("{{\"summary\":\"{}\"}}", "original source ".repeat(2_000)),
        }});
        let mut response = raw.clone();
        response["agent_turn"] = json!({
            "schema":"dfmcp.agent_turn/1", "operation":"fortress.cancel",
            "anchor":anchor, "continuity":{"status":status},
            "coverage":{"status":"complete_for_named_projection","complete_domains":["laboratory.anchor"]},
            "active_work":{"pending_plans":[{
                "plan_digest":"c".repeat(64), "state":"review_required",
                "minimum_safe_next_step":next,
            }]},
            "recommendations":[next],
        });
        (raw, response)
    }

    #[test]
    fn published_withdrawal_receipt_fits_its_admitted_reserve_despite_large_single_work_records()
    -> Result<(), serde_json::Error> {
        for status in [
            "continuous",
            "heartbeat",
            "gap",
            "reset",
            "stale",
            "indeterminate",
        ] {
            let (raw, response) = withdrawal_with_oversized_orientation(status);
            // This is the exact admission boundary used before publishing U;
            // the anchor and receipt use conservative maximum-width values.
            let raw_bytes = raw.to_string().len() as u64;
            let budget = (raw_bytes + 2_048).div_ceil(4);
            let original = response.to_string();
            let fitted = fit(&original, budget);
            let output: Value = serde_json::from_str(&fitted)?;
            assert_eq!(output["ok"], true, "{fitted}");
            assert_eq!(output["output_budget"]["tier"], "withdrawal_receipt");
            if let Some(fields) = raw.as_object() {
                for (key, value) in fields {
                    assert_eq!(&output[key], value, "lost original receipt field {key}");
                }
            }
            assert!(fitted.len() as u64 <= raw_bytes + 2_048);
            assert!(estimate_tokens(&fitted) <= budget);
            assert_eq!(fit(&original, budget), fitted);
            let turn = &output["agent_turn"];
            assert_eq!(turn["anchor"], raw["observed_anchor"]);
            assert_eq!(turn["continuity"]["status"], status);
            assert_eq!(turn["active_work"]["status"], "omitted_for_output_budget");
            assert_eq!(turn["active_work"]["absence_proven"], false);
            assert_eq!(turn["coverage"]["status"], "partial");
            assert_eq!(turn["coverage"]["absence_proof_scope"], json!([]));
            assert_eq!(turn["coverage"]["continuation"]["tool"], "fortress.observe");
            assert_eq!(turn["recommendations"][0]["risk"], "read_only");
            assert_eq!(
                turn["recommendations"][0]["arguments"]["session_id"],
                raw["session_id"]
            );
        }
        Ok(())
    }

    #[test]
    fn withdrawal_receipt_fallback_neither_rewrites_fitting_turns_nor_invents_success()
    -> Result<(), serde_json::Error> {
        let (raw, response) = withdrawal_with_oversized_orientation("continuous");
        let source = response.to_string();
        let full: Value = serde_json::from_str(&fit(&source, estimate_tokens(&source) + 1_000))?;
        assert_eq!(full["output_budget"]["tier"], "full");
        assert_eq!(full["agent_turn"], response["agent_turn"]);
        assert_eq!(full["withdrawal"], raw["withdrawal"]);
        let budget = (raw.to_string().len() as u64 + 2_048).div_ceil(4);
        for (key, value) in [
            ("ok", json!(false)),
            ("status", json!("cancelled")),
            ("archived", json!(false)),
            ("physical_work_changed", json!(true)),
            ("goal_success_claimed", json!(true)),
        ] {
            let mut unproven = response.clone();
            unproven[key] = value;
            let fitted: Value = serde_json::from_str(&fit(&unproven.to_string(), budget))?;
            assert_eq!(fitted["ok"], false, "{key}: {fitted}");
            assert_eq!(fitted["output_budget"]["tier"], "refused");
            assert!(fitted.get("withdrawal").is_none());
        }
        Ok(())
    }

    fn cancellation_with_oversized_orientation(
        status: &str,
        succeeded: bool,
        quiescent: bool,
    ) -> (Value, Value) {
        let (_, orientation) = withdrawal_with_oversized_orientation(status);
        let anchor = orientation["agent_turn"]["anchor"].clone();
        let mut raw = json!({
            "ok":succeeded, "session_id":"f".repeat(32),
            "scope":format!("plan:{}", "e".repeat(64)),
            "requested_plan_digest":"e".repeat(64), "plan_digest":"d".repeat(64),
            "plan_id":"c".repeat(32), "selection_scope":"exact_original_commit",
            "descendant_plans_included":false,
            "paused":true, "observed_anchor":anchor,
            "drain_progress":{
                "actions_total":1, "drained":u64::from(quiescent),
                "remaining_nonterminal":u64::from(!quiescent),
                "remaining_work":u64::from(!quiescent),
                "remaining_carried":u64::from(!quiescent),
                "untracked_work_quiescent":quiescent, "quiescent":quiescent,
            },
            "finalize_certificate":if quiescent {json!({
                "digest":"b".repeat(64), "anchor":anchor,
                "statement":"every original action is terminal and its physical work is proven quiescent",
            })} else {Value::Null},
            "emergency_pause":{
                "requested":true, "changed":true, "paused":true,
                "before_anchor":anchor, "observed_anchor":anchor,
                "game_ticks_advanced":0, "action_handles_created":false,
                "physical_work_drained":false,
                "authority":"current_observe_and_control_clock",
                "evidence_digest":"a".repeat(64),
                "durable":true, "publication_confirmed":succeeded,
            },
        });
        if !succeeded {
            raw["error"] = json!({
                "code":"corrupt_ledger", "message":"pause publication requires recovery",
            });
            raw["durability_pending"] = json!(true);
        }
        let mut response = raw.clone();
        response["agent_turn"] = orientation["agent_turn"].clone();
        response["steps"] = json!([{"historical_detail":"step evidence ".repeat(2_000)}]);
        (raw, response)
    }

    #[test]
    fn bounded_cancellation_preserves_exact_pause_drain_and_failure_outcomes()
    -> Result<(), serde_json::Error> {
        for status in ["continuous", "gap", "reset", "indeterminate"] {
            for (succeeded, quiescent) in [(true, true), (true, false), (false, false)] {
                let (raw, response) =
                    cancellation_with_oversized_orientation(status, succeeded, quiescent);
                let raw_bytes = raw.to_string().len() as u64;
                let budget = (raw_bytes + 2_048).div_ceil(4);
                let original = response.to_string();
                let fitted = fit(&original, budget);
                let output: Value = serde_json::from_str(&fitted)?;
                assert_eq!(
                    output["output_budget"]["tier"], "cancellation_receipt",
                    "{output}"
                );
                if let Some(fields) = raw.as_object() {
                    for (key, value) in fields {
                        assert_eq!(&output[key], value, "lost cancellation outcome field {key}");
                    }
                }
                assert_eq!(output["ok"], succeeded);
                assert_eq!(output["drain_progress"]["quiescent"], quiescent);
                assert_eq!(output["finalize_certificate"].is_null(), !quiescent);
                assert_eq!(output["emergency_pause"]["physical_work_drained"], false);
                assert_eq!(output["agent_turn"]["continuity"]["status"], status);
                assert_eq!(output["agent_turn"]["anchor"], raw["observed_anchor"]);
                assert_eq!(output["agent_turn"]["active_work"]["absence_proven"], false);
                assert_eq!(output["agent_turn"]["coverage"]["status"], "partial");
                assert_eq!(
                    output["agent_turn"]["coverage"]["complete_domains"],
                    json!(["cancellation_outcome"]),
                );
                assert_eq!(
                    output["agent_turn"]["coverage"]["omitted_domains"],
                    json!([
                        "session_orientation",
                        "per_step_drain_evidence",
                        "untracked_work_detail"
                    ]),
                );
                assert_eq!(
                    output["agent_turn"]["recommendations"][0]["risk"],
                    "read_only"
                );
                assert!(estimate_tokens(&fitted) <= budget);
                assert_eq!(fit(&original, budget), fitted);
                let minimal = minimal(&response);
                assert_eq!(minimal["drain_progress"], raw["drain_progress"]);
                assert_eq!(minimal["emergency_pause"], raw["emergency_pause"]);
                assert_eq!(minimal["finalize_certificate"], raw["finalize_certificate"]);
            }
        }
        Ok(())
    }

    #[test]
    fn bounded_single_action_failure_keeps_exact_action_and_physical_state()
    -> Result<(), serde_json::Error> {
        let (mut raw, mut response) =
            cancellation_with_oversized_orientation("indeterminate", false, false);
        for key in [
            "plan_digest",
            "requested_plan_digest",
            "plan_id",
            "selection_scope",
            "descendant_plans_included",
            "finalize_certificate",
            "emergency_pause",
        ] {
            if let Some(fields) = raw.as_object_mut() {
                fields.remove(key);
            }
            if let Some(fields) = response.as_object_mut() {
                fields.remove(key);
            }
        }
        for (key, value) in [
            ("scope", json!("last_action")),
            ("action_id", json!("a".repeat(32))),
            ("final_state", json!("AppliedAwaitingVerification")),
            (
                "work_state",
                json!({
                    "state":"unknown", "quiescent":false,
                    "observed_anchor":raw["observed_anchor"],
                }),
            ),
        ] {
            raw[key] = value.clone();
            response[key] = value;
        }
        let budget = (raw.to_string().len() as u64 + 2_048).div_ceil(4);
        let fitted = fit(&response.to_string(), budget);
        let output: Value = serde_json::from_str(&fitted)?;
        assert_eq!(
            output["output_budget"]["tier"], "cancellation_receipt",
            "{output}"
        );
        if let Some(fields) = raw.as_object() {
            for (key, value) in fields {
                assert_eq!(
                    &output[key], value,
                    "lost single-action outcome field {key}"
                );
            }
        }
        assert_eq!(output["ok"], false);
        assert_eq!(output["scope"], "last_action");
        assert_eq!(output["action_id"], "a".repeat(32));
        assert_eq!(output["work_state"]["quiescent"], false);
        assert!(output.get("finalize_certificate").is_none());
        assert!(estimate_tokens(&fitted) <= budget);
        Ok(())
    }

    #[test]
    fn cancellation_receipt_requires_an_observed_outcome_and_preserves_fitting_turns()
    -> Result<(), serde_json::Error> {
        let (raw, response) = cancellation_with_oversized_orientation("continuous", true, false);
        let source = response.to_string();
        let full: Value = serde_json::from_str(&fit(&source, estimate_tokens(&source) + 1_000))?;
        assert_eq!(full["output_budget"]["tier"], "full");
        assert_eq!(full["agent_turn"], response["agent_turn"]);
        assert_eq!(full["drain_progress"], raw["drain_progress"]);
        let budget = (raw.to_string().len() as u64 + 2_048).div_ceil(4);
        for (key, value) in [
            ("drain_progress", Value::Null),
            ("observed_anchor", Value::Null),
            ("paused", json!("unknown")),
            ("ok", Value::Null),
        ] {
            let mut unavailable = response.clone();
            unavailable[key] = value;
            assert!(cancellation_outcome(&unavailable).is_none());
            let fitted: Value = serde_json::from_str(&fit(&unavailable.to_string(), budget))?;
            assert_eq!(fitted["ok"], false, "{key}: {fitted}");
            assert_eq!(fitted["output_budget"]["tier"], "refused");
        }
        let mut other_operation = response;
        other_operation["agent_turn"]["operation"] = json!("fortress.wait");
        assert!(cancellation_outcome(&other_operation).is_none());
        Ok(())
    }
}

#[cfg(test)]
mod objective_projection_tests {
    use super::*;

    fn objective() -> Value {
        json!({
            "plan_digest":"ab".repeat(32), "summary":"furnish the original room",
            "status":"achieved", "predicate_truth":"true",
            "epistemic_state":"certified_derived", "evidence_scope":"laboratory_reference_world",
            "observed_anchor":{"state_hash":"cd".repeat(32),"game_tick":101},
            "historical_achievement":"verified", "goal_scope":"resource_acquisition_stage",
            "historical_achievement_scope":"resource_acquisition_stage",
            "semantic_room_goal":{
                "truth":"false", "predicate_truth":"false", "status":"not_yet_observed",
                "epistemic_state":"certified_derived", "evidence_scope":"laboratory_reference_world",
                "observed_anchor":{"state_hash":"cd".repeat(32),"game_tick":101},
                "completion_scope":"complete_original_room_and_access",
                "historical_action_proof_rewritten":false,
            },
            "physical_quiescent":true, "pursuit_active":false, "needs_replan":true,
            "replacement_work_dispatched":false, "blind_retry_allowed":false,
            "continuation_request":{
                "tool":"fortress.plan", "arguments":{"session_id":"s",
                    "blueprint":"{\"template\":\"continue_goal\",\"plan_digest\":\"original\"}"},
            },
            "original_source":{"program":"original source detail ".repeat(1000)},
        })
    }

    fn response(goal: &Value) -> Value {
        json!({
            "ok":true,"session_id":"s", "objectives":[goal],
            "agent_turn":{
                "schema":"dfmcp.agent_turn/1","operation":"fortress.observe",
                "anchor":goal["observed_anchor"], "continuity":{"status":"continuous"},
                "briefing":{"objective_status":[goal]},"coverage":{},
                "active_work":{}, "recommendations":[], "affordances":[],
            },
        })
    }

    fn fitted_goal(goal: &Value, profile: &str) -> Result<Value, serde_json::Error> {
        let shaped = shape_for_profile(&response(goal).to_string(), profile);
        let text = fit(&shaped, 1_000);
        assert!(estimate_tokens(&text) <= 1_000);
        assert_eq!(
            text,
            fit(&shaped, 1_000),
            "projection must be deterministic"
        );
        let fitted: Value = serde_json::from_str(&text)?;
        assert_eq!(fitted["ok"], true, "{fitted}");
        assert_eq!(
            fitted["output_budget"]["tier"], "sections_omitted",
            "{fitted}"
        );
        assert_eq!(fitted["objectives"]["omitted_for_output_budget"], true);
        let compact = fitted["agent_turn"]["briefing"]["objective_status"][0].clone();
        assert!(compact.is_object(), "the compact briefing still fits");
        assert_eq!(compact["detail_omitted_for_output_budget"], true);
        assert!(compact.get("original_source").is_none());
        Ok(compact)
    }

    #[test]
    fn compact_harvest_achievement_keeps_unfinished_room_and_explicit_continuation()
    -> Result<(), serde_json::Error> {
        let goal = objective();
        for profile in ["pulse", "briefing"] {
            let compact = fitted_goal(&goal, profile)?;
            assert_eq!(compact["status"], "achieved");
            assert_eq!(compact["predicate_truth"], "true");
            assert_eq!(compact["goal_scope"], "resource_acquisition_stage");
            assert_eq!(
                compact["historical_achievement_scope"],
                "resource_acquisition_stage"
            );
            assert_eq!(compact["semantic_room_goal"], goal["semantic_room_goal"]);
            assert_eq!(compact["semantic_room_goal"]["truth"], "false");
            assert_eq!(compact["needs_replan"], true);
            assert_eq!(
                compact["continuation_request"],
                goal["continuation_request"]
            );
            assert_eq!(compact["physical_quiescent"], true);
            assert_eq!(compact["replacement_work_dispatched"], false);
            assert_eq!(compact["blind_retry_allowed"], false);
        }
        Ok(())
    }

    #[test]
    fn compact_room_completion_does_not_rewrite_its_spent_harvest_or_unknown_evidence()
    -> Result<(), serde_json::Error> {
        let mut goal = objective();
        goal["status"] = json!("no_longer_holds");
        goal["predicate_truth"] = json!("false");
        goal["semantic_room_goal"]["truth"] = json!("true");
        goal["semantic_room_goal"]["predicate_truth"] = json!("true");
        goal["semantic_room_goal"]["status"] = json!("achieved");
        goal["needs_replan"] = json!(false);
        if let Some(object) = goal.as_object_mut() {
            object.remove("continuation_request");
        }
        for profile in ["pulse", "briefing"] {
            let compact = fitted_goal(&goal, profile)?;
            assert_eq!(compact["status"], "no_longer_holds");
            assert_eq!(compact["predicate_truth"], "false");
            assert_eq!(compact["semantic_room_goal"], goal["semantic_room_goal"]);
            assert_eq!(compact["historical_achievement"], "verified");
            assert_eq!(compact["needs_replan"], false);
            assert!(compact.get("continuation_request").is_none());
        }

        goal["semantic_room_goal"] = json!({
            "truth":"unknown","predicate_truth":"unknown","epistemic_state":"unknown",
            "observation_error":{"code":"capability_denied","message":"current Query authority is required"},
        });
        goal["physical_quiescent"] = Value::Null;
        goal["continuation_refused"] = json!({"code":"capability_denied"});
        for profile in ["pulse", "briefing"] {
            let compact = fitted_goal(&goal, profile)?;
            assert_eq!(compact["semantic_room_goal"], goal["semantic_room_goal"]);
            assert!(compact.get("physical_quiescent").is_some());
            assert!(compact["physical_quiescent"].is_null());
            assert_eq!(
                compact["continuation_refused"],
                goal["continuation_refused"]
            );
            assert!(compact.get("continuation_request").is_none());
        }
        Ok(())
    }

    #[test]
    fn compact_current_output_success_keeps_future_reserve_and_its_epistemic_class()
    -> Result<(), serde_json::Error> {
        let mut goal = objective();
        if let Some(object) = goal.as_object_mut() {
            object.remove("semantic_room_goal");
        }
        goal["semantic_output_goal"] = json!({
            "predicate_truth":"true", "epistemic_state":"certified_derived",
            "evidence_scope":"laboratory_reference_world",
            "historical_action_proof_rewritten":false,
        });
        for (status, epistemic, work) in [
            ("work_required", "predicted", json!(true)),
            ("funded_from_current_stock", "predicted", json!(false)),
            ("unknown", "unknown", Value::Null),
            ("expired", "observed", Value::Null),
        ] {
            goal["needs_replan"] = json!(status == "work_required");
            if status != "work_required"
                && let Some(object) = goal.as_object_mut()
            {
                object.remove("continuation_request");
            }
            goal["resource_reserve"] = json!({
                "reserve_until_tick":12_001, "status":status, "epistemic":epistemic,
                "work_required":work,"continuous_maintenance_proven":false,
                "new_request_required":status == "expired",
                "historical_action_proof_rewritten":false,
            });
            // The same reserve distinction applies when production is funded
            // from current ingredients without an acquisition stage.
            goal["production_reserve"] = goal["resource_reserve"].clone();
            for profile in ["pulse", "briefing"] {
                let compact = fitted_goal(&goal, profile)?;
                assert_eq!(compact["predicate_truth"], "true");
                assert_eq!(
                    compact["semantic_output_goal"],
                    goal["semantic_output_goal"]
                );
                assert_eq!(compact["resource_reserve"], goal["resource_reserve"]);
                assert_eq!(compact["production_reserve"], goal["production_reserve"]);
                assert_eq!(
                    compact["resource_reserve"]["continuous_maintenance_proven"],
                    false
                );
                assert!(compact.get("semantic_room_goal").is_none());
            }
        }
        Ok(())
    }

    #[test]
    fn compact_legacy_objectives_do_not_gain_project_or_continuation_claims() {
        let mut goal = json!({"plan_digest":"old", "summary":"pause", "status":"achieved"});
        compact_objective(&mut goal);
        assert_eq!(goal["status"], "achieved");
        for field in [
            "predicate_truth",
            "semantic_room_goal",
            "semantic_output_goal",
            "resource_reserve",
            "production_reserve",
            "physical_quiescent",
            "needs_replan",
            "continuation_request",
        ] {
            assert!(goal.get(field).is_none(), "{goal}");
        }
    }
}

#[cfg(test)]
mod alert_projection_tests {
    use super::*;

    fn response(epistemic: &str) -> Value {
        let anchor = json!({
            "fortress_id":"7", "game_tick":123, "state_hash":"ab".repeat(32),
            "cursor":{"epoch":2,"sequence":41},
        });
        json!({
            "ok":true, "session_id":"s",
            "agent_turn":{
                "schema":"dfmcp.agent_turn/1", "operation":"fortress.observe",
                "anchor":anchor, "continuity":{"status":"continuous"},
                "coverage":{}, "active_work":{}, "recommendations":[],
                "affordances":[{"enabled":false,"description":"optional detail".repeat(1000)}],
                "attention":[{
                    "attention_id":"world-drink_supply", "category":"fortress_needs",
                    "severity":"high", "urgency":"before_the_next_plan",
                    "confidence":{"epistemic_state":epistemic,"value":null},
                    "finding":"inspect the qualified supply state", "remedy":null,
                    "evidence":[{"source_anchor":anchor,"scope":"laboratory_reference_world",
                        "source_scope":"cd".repeat(32)}],
                }],
            },
        })
    }

    fn assert_qualification(actual: &Value, expected: &Value) {
        assert_eq!(actual["confidence"], expected["confidence"]);
        assert!(actual["confidence"]["value"].is_null());
        assert_eq!(actual["evidence"], expected["evidence"]);
        assert_eq!(actual["remedy"], expected["remedy"]);
    }

    #[test]
    fn pulse_keeps_unknown_and_derived_alert_source_qualification() -> Result<(), serde_json::Error>
    {
        for epistemic in ["unknown", "certified_derived"] {
            let original = response(epistemic);
            let pulse = shape_for_profile(&original.to_string(), "pulse");
            let fitted = fit(&pulse, 750);
            let parsed: Value = serde_json::from_str(&fitted)?;
            assert!(estimate_tokens(&fitted) <= 750);
            assert_eq!(parsed["output_budget"]["tier"], "full");
            assert_qualification(
                &parsed["agent_turn"]["attention"][0],
                &original["agent_turn"]["attention"][0],
            );
        }
        Ok(())
    }

    #[test]
    fn output_compaction_keeps_unknown_and_derived_alert_source_qualification()
    -> Result<(), serde_json::Error> {
        for epistemic in ["unknown", "certified_derived"] {
            let original = response(epistemic);
            let fitted = fit(&original.to_string(), 750);
            let parsed: Value = serde_json::from_str(&fitted)?;
            assert!(estimate_tokens(&fitted) <= 750);
            assert_eq!(parsed["output_budget"]["tier"], "compact_turn");
            assert_qualification(
                &parsed["agent_turn"]["attention"][0],
                &original["agent_turn"]["attention"][0],
            );
        }
        Ok(())
    }

    #[test]
    fn alert_projection_bounds_evidence_without_slicing_a_proof() -> Result<(), serde_json::Error> {
        let original = response("unknown");
        let witness = original["agent_turn"]["attention"][0]["evidence"][0].clone();
        let mut oversized = witness.clone();
        oversized["detail"] = json!("unbounded proof".repeat(1000));
        for evidence in [json!([witness.clone(), witness]), json!([oversized])] {
            let mut payload = original.clone();
            payload["agent_turn"]["attention"][0]["evidence"] = evidence;
            for profile in ["pulse", "briefing"] {
                let shaped = shape_for_profile(&payload.to_string(), profile);
                let fitted = fit(&shaped, 750);
                let parsed: Value = serde_json::from_str(&fitted)?;
                let alert = &parsed["agent_turn"]["attention"][0];
                assert!(estimate_tokens(&fitted) <= 750);
                assert_eq!(
                    alert["confidence"],
                    original["agent_turn"]["attention"][0]["confidence"]
                );
                assert_eq!(alert["evidence"], json!([{"omitted_for_profile":true}]));
            }
        }
        Ok(())
    }

    #[test]
    fn cheap_remedy_recommendations_keep_risk_and_checkpoint_requirements()
    -> Result<(), serde_json::Error> {
        for (risk, required) in [("military", true), ("guarded", true), ("reversible", false)] {
            let mut payload = response("certified_derived");
            let remedy = json!({
                "recommendation_id":"remedy-hostile", "tool":"fortress.plan",
                "reason":"review the observed defense need", "risk":risk,
                "reversibility":"not_compensable",
                "requires_checkpoint":required, "requires_confirmation":false,
                "arguments":{"actions":"[]"},
            });
            payload["agent_turn"]["recommendations"] = json!([remedy]);
            for profile in ["pulse", "briefing"] {
                let shaped = shape_for_profile(&payload.to_string(), profile);
                let fitted = fit(&shaped, 750);
                let parsed: Value = serde_json::from_str(&fitted)?;
                let actual = &parsed["agent_turn"]["recommendations"][0];
                assert!(estimate_tokens(&fitted) <= 750);
                assert_eq!(actual["risk"], risk);
                assert_eq!(actual["reversibility"], "not_compensable");
                assert_eq!(actual["requires_checkpoint"], required);
                assert_eq!(actual["arguments"], remedy["arguments"]);
            }
        }
        Ok(())
    }

    #[test]
    fn older_recommendations_do_not_gain_missing_safety_fields() -> Result<(), serde_json::Error> {
        let mut payload = response("unknown");
        payload["agent_turn"]["recommendations"] = json!([{"tool":"fortress.wait","arguments":{}}]);
        for profile in ["pulse", "briefing"] {
            let shaped = shape_for_profile(&payload.to_string(), profile);
            let fitted = fit(&shaped, 750);
            let parsed: Value = serde_json::from_str(&fitted)?;
            let recommendation = &parsed["agent_turn"]["recommendations"][0];
            assert!(recommendation.is_object());
            for field in ["risk", "reversibility", "requires_checkpoint"] {
                assert!(recommendation.get(field).is_none(), "{recommendation}");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod semantic_omission_tests {
    use super::*;

    fn covered_page() -> Value {
        let coverage = json!({"status":"complete_for_named_projection",
            "complete_domains":[{"domain":"laboratory.entities","kind":"unit"}],
            "partial_domains":[], "absence_proof_scope":["laboratory.entities"],
            "source_complete":true, "page_complete":true});
        json!({
            "ok":true,"mode":"entities","session_id":"s","offset":0,"returned":100,"total":100,
            "complete_domain":true,"absence_proven":false,
            "query":{"mode":"entities","kind":"unit","limit":100,"offset":0,"at":"exact-source-hash",
                "where":{"field":"alive","value":true}},
            "rows":(0..100).map(|i| json!({"entity_id":i.to_string(),"label":"a complete row",
                "fields":{"assignment":[1,2,3,null]},"field_presence":{"assignment":{"state":"known"}}})).collect::<Vec<_>>(),
            "observation_coverage":coverage,
            "agent_turn":{"schema":"dfmcp.agent_turn/1","operation":"fortress.query",
                "anchor":{"state_hash":"exact-source-hash"},"continuity":{"status":"continuous"},
                "coverage":coverage,"active_work":{},"recommendations":[]}
        })
    }

    #[test]
    fn output_truncation_repairs_page_cursor_and_withdraws_absence_claims()
    -> Result<(), Box<dyn std::error::Error>> {
        let original = covered_page();
        let output: Value = serde_json::from_str(&fit(&original.to_string(), 1800))?;
        let rows = output["rows"]
            .as_array()
            .ok_or("budget should retain a page of rows")?;
        assert!(!rows.is_empty() && rows.len() < 100, "{output}");
        assert_eq!(output["returned"], rows.len());
        assert_eq!(output["next_offset"], rows.len());
        assert_eq!(output["continuation"]["offset"], rows.len());
        assert_eq!(output["continuation"]["at"], "exact-source-hash");
        assert_eq!(output["continuation"]["where"], original["query"]["where"]);
        assert_eq!(output["complete_domain"], false);
        assert_eq!(output["observation_coverage"]["status"], "partial");
        assert_eq!(output["agent_turn"]["coverage"]["status"], "partial");
        assert_eq!(
            output["observation_coverage"]["partial_domains"],
            original["observation_coverage"]["complete_domains"]
        );
        assert_eq!(
            output["agent_turn"]["coverage"]["absence_proof_scope"],
            json!([])
        );
        for row in rows {
            assert_eq!(row["fields"]["assignment"], json!([1, 2, 3, null]));
        }
        Ok(())
    }

    #[test]
    fn a_single_known_list_is_omitted_as_a_record_instead_of_rewritten()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = covered_page();
        let value = json!((0..4000).collect::<Vec<_>>());
        response["rows"] = json!([{"entity_id":"1","fields":{"members":value},
            "field_presence":{"members":{"state":"known"}}}]);
        response["total"] = json!(1);
        response["returned"] = json!(1);
        let output: Value = serde_json::from_str(&fit(&response.to_string(), 900))?;
        if let Some(rows) = output["rows"].as_array() {
            assert_eq!(rows[0]["fields"]["members"], value);
        } else {
            assert!(matches!(
                output["output_budget"]["tier"].as_str(),
                Some("sections_omitted" | "minimal" | "refused")
            ));
            if output["output_budget"]["tier"] != "refused" {
                assert_eq!(output["agent_turn"]["coverage"]["status"], "partial");
            }
        }
        Ok(())
    }

    #[test]
    fn profile_shaping_and_compaction_preserve_semantic_nulls_and_coverage()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = covered_page();
        response["rows"] = json!([]);
        response["agent_turn"]["changes"] = json!([{"before":{"slot":null},"after":{"slot":7}}]);
        let pulse: Value =
            serde_json::from_str(&shape_for_profile(&response.to_string(), "pulse"))?;
        assert!(
            pulse["agent_turn"]["changes"][0]["before"]
                .get("slot")
                .is_some()
        );
        assert_eq!(
            pulse["agent_turn"]["coverage"]["absence_proof_scope"],
            response["agent_turn"]["coverage"]["absence_proof_scope"]
        );
        let mut turn = response["agent_turn"].clone();
        compact_turn(&mut turn);
        assert!(turn["changes"][0]["before"].get("slot").is_some());
        assert_eq!(turn["coverage"], response["agent_turn"]["coverage"]);
        Ok(())
    }
    #[test]
    fn profiled_pages_keep_section_source_anchor_and_complete_chunk_values()
    -> Result<(), Box<dyn std::error::Error>> {
        let mut response = covered_page();
        response["mode"] = json!("observation");
        response["query"] = json!({"mode":"observation","section":"chunks",
            "completeness_profile":"spatial","at":"exact-source-hash","offset":0,"limit":100});
        let runs = json!([{"tile_code":1,"length":100},{"tile_code":2,"length":156}]);
        response["rows"] = json!(
            (0..100)
                .map(|i| json!({"coord":[i,0,10],
            "terrain_runs":runs}))
                .collect::<Vec<_>>()
        );
        let output: Value = serde_json::from_str(&fit(&response.to_string(), 1600))?;
        let rows = output["rows"]
            .as_array()
            .ok_or("budget should retain profiled rows")?;
        assert!(!rows.is_empty() && rows.len() < 100);
        assert_eq!(output["returned"], rows.len());
        assert_eq!(output["continuation"]["offset"], rows.len());
        assert_eq!(output["continuation"]["at"], "exact-source-hash");
        assert_eq!(output["continuation"]["completeness_profile"], "spatial");
        assert_eq!(output["continuation"]["section"], "chunks");
        for row in rows {
            assert_eq!(row["terrain_runs"], runs);
        }
        assert_eq!(output["agent_turn"]["coverage"]["status"], "partial");
        Ok(())
    }
    #[test]
    fn omitting_an_empty_or_finished_page_does_not_invent_a_continuation() {
        for (included, total, offset) in [(false, 0, 0), (true, 0, 0), (true, 4, 4)] {
            let mut response = covered_page();
            response["mode"] = json!("observation");
            response["section_included"] = json!(included);
            response["total"] = json!(total);
            response["offset"] = json!(offset);
            response["rows"] = json!([]);
            response["query"] = json!({"mode":"observation","section":"events",
                "completeness_profile":"control-minimum","at":"exact-source-hash",
                "offset":offset,"limit":100});
            record_omission(&mut response, "rows");
            assert_eq!(response["returned"], 0);
            assert!(response["next_offset"].is_null());
            assert!(response["continuation"].is_null());
            assert!(response["observation_coverage"]["continuation"].is_null());
            assert!(response["agent_turn"]["coverage"]["continuation"].is_null());
        }
    }
}
