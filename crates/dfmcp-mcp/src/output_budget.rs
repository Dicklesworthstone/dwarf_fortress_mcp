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
    match value {
        Value::Object(map) => {
            map.retain(|_, v| !v.is_null());
            for v in map.values_mut() {
                strip_nulls(v);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(strip_nulls),
        _ => {}
    }
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
            *item = json!({
                "recommendation_id": item["recommendation_id"],
                "tool": item["tool"],
                "arguments": item["arguments"],
                "reason": item["reason"],
                "requires_confirmation": item["requires_confirmation"],
            });
        }
    }
    if let Some(items) = turn.get_mut("attention").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            *item = json!({
                "attention_id": item["attention_id"],
                "severity": item["severity"],
                "finding": item["finding"],
                "remedy": item["remedy"],
            });
        }
    }
    if let Some(items) = turn
        .get_mut("briefing")
        .and_then(|briefing| briefing.get_mut("objective_status"))
        .and_then(Value::as_array_mut)
    {
        for item in items.iter_mut() {
            *item = json!({
                "plan_digest": item["plan_digest"],
                "summary": item["summary"],
                "status": item["status"],
            });
        }
    }
    if let Some(budget) = turn.get_mut("budget") {
        *budget = json!({"admitted": budget["admitted"]});
    }
    if let Some(coverage) = turn.get_mut("coverage") {
        *coverage = json!({"status": coverage["status"]});
    }
    strip_nulls(turn);
}

/// Agent Turn sections a profile does not carry, per
/// `architecture/agent_turn_contract.json` (`required_sections`).
const PULSE_OMITS: [&str; 3] = ["briefing", "affordances", "references"];

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
            work.retain(|_, v| !v.as_array().is_some_and(Vec::is_empty));
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
        turn["coverage"] = json!({
            "status": turn["coverage"]["status"],
            "attention_selection": {
                "certified": turn["coverage"]["attention_selection"]["certified"],
                "excluded": turn["coverage"]["attention_selection"]["excluded"],
                "selected": turn["coverage"]["attention_selection"]["selected"],
            },
            "omitted_by_profile": PULSE_OMITS,
        });
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

/// Every array in `value` with at least two items, as (path, serialized size).
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
            if items.len() > 1 {
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
    let turn = &payload["agent_turn"];
    out.insert(
        "agent_turn".to_owned(),
        json!({
            "schema": turn["schema"],
            "operation": turn["operation"],
            "anchor": turn["anchor"],
            "continuity": {"status": turn["continuity"]["status"]},
            "active_work": turn["active_work"],
            "recommendations": turn["recommendations"].as_array().map(|r| r.iter().take(1).cloned().collect::<Vec<_>>()),
        }),
    );
    Value::Object(out)
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
            .entry(path)
            .or_insert_with(|| json!({"total": total}));
        entry["returned"] = json!(returned);
        record["truncated"] = Value::Object(truncated.clone());
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
            if let Some(text) = fits(&payload, &record, max_output_tokens) {
                return text;
            }
        }
    }

    record["tier"] = json!("minimal");
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
        if let Some(text) = fits(&small, &record, max_output_tokens) {
            return text;
        }
    }
    match fits(&small, &record, max_output_tokens) {
        Some(text) => text,
        None => {
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
}
