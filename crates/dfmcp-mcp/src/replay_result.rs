//! Versioned commitments to the rendered tool payload, not Agent Turn state.
//!
//! The profile retains every root payload field, including rows, ordering,
//! projections, coverage, query continuations expressed as ordinary objects,
//! plan reviews and structured errors. Only these operational paths change:
//! `agent_turn` is outside this profile; unsigned `query_budget.max_wall_millis`,
//! `output_budget.full_estimate_tokens` and doctor `active_sessions_count` are
//! tagged as telemetry, retaining their presence and malformed values. The
//! output tiers `full`/`compact_turn` both retain the complete tool payload;
//! only their presentation choice and Agent Turn-only minimal-truncation counts
//! are normalized. Every tool-data truncation tier and coverage fact remains.
//! Current session identities are tagged only at root `session_id`, observation goal
//! owner/continuation paths and the entity query's server-added row resource URI.
//! User fields and foreign identities are never recursively rewritten.
//!
//! Stateful tools/modes, opaque returned cursors, retained process-local query
//! work, observation withdrawal proofs and non-entity explanations explicitly
//! retain outcome/anchor verification.
//! They do not acquire an unsupported payload-equality claim. The commitment
//! itself is checked again on replay, so editing an eligible call to claim this
//! weaker profile cannot silently downgrade its comparison.

use dfmcp_core::Digest32;
use serde_json::{Value, json};

pub(super) const PAYLOAD_PROFILE: &str = "dfmcp.replay.tool-payload/1";
pub(super) const OUTCOME_PROFILE: &str = "dfmcp.replay.outcome-anchor/1";
const DOMAIN: &[u8] = b"dfmcp-replay-tool-payload/1\0";
const MAX_RESULT_BYTES: usize = 1024 * 1024;
const MAX_RESULT_NODES: usize = 65_536;
const MAX_RESULT_DEPTH: usize = 64;
const REASONS: [&str; 8] = [
    "tool_not_in_profile",
    "query_mode_not_in_profile",
    "opaque_query_continuation",
    "session_bound_objective_evidence",
    "process_local_query_work",
    "explanation_not_entity",
    "payload_outside_bounds",
    "malformed_payload",
];

fn excluded(reason: &'static str) -> Value {
    json!({"profile":OUTCOME_PROFILE, "digest":null, "reason":reason})
}

/// Validate the closed commitment before the replayer creates any session.
pub(super) fn valid(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != 3
        || !["profile", "digest", "reason"]
            .iter()
            .all(|key| object.contains_key(*key))
    {
        return false;
    }
    match value["profile"].as_str() {
        Some(PAYLOAD_PROFILE) => {
            value["reason"].is_null()
                && value["digest"].as_str().is_some_and(|digest| {
                    digest.len() == 64
                        && digest
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                })
        }
        Some(OUTCOME_PROFILE) => {
            value["digest"].is_null()
                && value["reason"]
                    .as_str()
                    .is_some_and(|reason| REASONS.contains(&reason))
        }
        _ => false,
    }
}

fn query_in_profile(arguments: &Value) -> bool {
    let Some(mode) = arguments.get("mode").filter(|mode| !mode.is_null()) else {
        return true;
    };
    let Some(raw) = mode.as_str().filter(|raw| raw.len() <= MAX_RESULT_BYTES) else {
        return false;
    };
    if matches!(raw, "summary" | "entities") {
        return true;
    }
    serde_json::from_str::<Value>(raw)
        .ok()
        .is_some_and(|query| {
            matches!(
                query["mode"].as_str(),
                Some(
                    "summary"
                        | "entities"
                        | "observation"
                        | "terrain"
                        | "path"
                        | "search"
                        | "structured"
                )
            )
        })
}

fn bounded(value: &Value, depth: usize, left: &mut usize) -> bool {
    if depth > MAX_RESULT_DEPTH || *left == 0 {
        return false;
    }
    *left -= 1;
    match value {
        Value::Array(values) => values.iter().all(|value| bounded(value, depth + 1, left)),
        Value::Object(values) => values.values().all(|value| bounded(value, depth + 1, left)),
        _ => true,
    }
}

// Tagged variants preserve absent, null, malformed and foreign values. In
// particular, a literal user value equal to our marker cannot become an owner.
fn owner_at(value: &mut Value, pointer: &str, session: &str) {
    if let Some(field) = value.pointer_mut(pointer) {
        *field = if field.as_str() == Some(session) {
            json!({"replay_binding":"current_session"})
        } else {
            json!({"literal":field.take()})
        };
    }
}

fn telemetry_at(value: &mut Value, pointer: &str) {
    if let Some(field) = value.pointer_mut(pointer) {
        *field = if field.is_u64() {
            json!({"replay_telemetry":"unsigned_measurement"})
        } else {
            json!({"literal":field.take()})
        };
    }
}

fn presentation_budget(value: &mut Value) {
    let Some(budget) = value
        .get_mut("output_budget")
        .and_then(Value::as_object_mut)
    else {
        return;
    };
    if let Some(tier) = budget.get_mut("tier") {
        *tier = if matches!(tier.as_str(), Some("full" | "compact_turn")) {
            json!({"replay_output":"tool_payload_intact"})
        } else {
            json!({"literal":tier.take()})
        };
    }
    if let Some(truncated) = budget
        .get_mut("minimal_truncated")
        .and_then(Value::as_object_mut)
    {
        let removed_presentation = truncated.keys().any(|path| path.starts_with("agent_turn."));
        truncated.retain(|path, _| !path.starts_with("agent_turn."));
        if removed_presentation && truncated.is_empty() {
            budget.remove("minimal_truncated");
        }
    }
}

fn entity_resource(row: &mut Value, session: &str) {
    let Some(field) = row.get_mut("resource_uri") else {
        return;
    };
    let prefix = format!("df://session/{session}/entity-");
    *field = if let Some(suffix) = field.as_str().and_then(|uri| uri.strip_prefix(&prefix)) {
        json!({"replay_binding":"current_session_entity_resource", "suffix":suffix})
    } else {
        json!({"literal":field.take()})
    };
}

/// Sort every object explicitly, independently of serde_json's map feature;
/// arrays retain their semantic order and primitive values keep JSON encoding.
pub(super) fn canonical_json(value: &Value, output: &mut Vec<u8>) -> Result<(), serde_json::Error> {
    match value {
        Value::Object(object) => {
            output.push(b'{');
            let mut entries = object.iter().collect::<Vec<_>>();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                serde_json::to_writer(&mut *output, key)?;
                output.push(b':');
                canonical_json(value, output)?;
            }
            output.push(b'}');
        }
        Value::Array(values) => {
            output.push(b'[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(b',');
                }
                canonical_json(value, output)?;
            }
            output.push(b']');
        }
        _ => serde_json::to_writer(output, value)?,
    }
    Ok(())
}

/// Capture a bounded, explicitly scoped comparison of one final tool response.
/// Only a fixed-size digest/reason is retained in the session transcript.
pub(super) fn commitment(tool: &str, arguments: &Value, payload: &str, session: &str) -> Value {
    match tool {
        "fortress.observe" | "fortress.plan" | "fortress.doctor" => {}
        "fortress.query" if query_in_profile(arguments) => {}
        "fortress.query" => return excluded("query_mode_not_in_profile"),
        "fortress.explain"
            if arguments["entity_id"]
                .as_str()
                .is_some_and(|id| id.parse::<u128>().is_ok()) => {}
        "fortress.explain" => return excluded("explanation_not_entity"),
        _ => return excluded("tool_not_in_profile"),
    }
    if payload.len() > MAX_RESULT_BYTES {
        return excluded("payload_outside_bounds");
    }
    let Ok(mut result) = serde_json::from_str::<Value>(payload) else {
        return excluded("malformed_payload");
    };
    if !result.is_object() {
        return excluded("malformed_payload");
    }
    let mut nodes_left = MAX_RESULT_NODES;
    if !bounded(&result, 0, &mut nodes_left) {
        return excluded("payload_outside_bounds");
    }
    if ["query_baselines", "query_watches", "condition_watches"]
        .iter()
        .any(|key| {
            result["query_work"][*key]
                .as_array()
                .is_some_and(|rows| !rows.is_empty())
        })
    {
        return excluded("process_local_query_work");
    }
    if tool == "fortress.query"
        && result["mode"] == "structured"
        && [
            "/continuation",
            "/coverage/continuation",
            "/observation_coverage/continuation",
        ]
        .iter()
        .any(|pointer| result.pointer(pointer).is_some_and(Value::is_string))
    {
        return excluded("opaque_query_continuation");
    }
    if tool == "fortress.observe" {
        if let Some(goals) = result.get_mut("objectives").and_then(Value::as_array_mut) {
            for goal in goals {
                if goal.get("withdrawal").is_some_and(|proof| !proof.is_null()) {
                    return excluded("session_bound_objective_evidence");
                }
                owner_at(goal, "/owner_session_id", session);
                owner_at(goal, "/continuation_request/arguments/session_id", session);
            }
        }
    }
    if tool == "fortress.query" && result["mode"] == "entities" {
        if let Some(rows) = result.get_mut("rows").and_then(Value::as_array_mut) {
            for row in rows {
                entity_resource(row, session);
            }
        }
    }
    owner_at(&mut result, "/session_id", session);
    telemetry_at(&mut result, "/query_budget/max_wall_millis");
    telemetry_at(&mut result, "/output_budget/full_estimate_tokens");
    presentation_budget(&mut result);
    if tool == "fortress.doctor" {
        telemetry_at(&mut result, "/active_sessions_count");
    }
    if let Some(object) = result.as_object_mut() {
        object.remove("agent_turn");
    }
    let mut bytes = DOMAIN.to_vec();
    if canonical_json(&json!({"tool":tool, "result":result}), &mut bytes).is_err() {
        return excluded("malformed_payload");
    }
    json!({"profile":PAYLOAD_PROFILE, "digest":Digest32::of_bytes(&bytes).to_hex(), "reason":null})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query(payload: &Value, session: &str) -> Value {
        commitment(
            "fortress.query",
            &json!({"mode":"entities"}),
            &payload.to_string(),
            session,
        )
    }

    fn rows(session: &str) -> Value {
        json!({
            "ok":true, "session_id":session, "mode":"entities", "returned":2,
            "rows":[
                {"id":"1", "resource_uri":format!("df://session/{session}/entity-1~1~abc"),
                 "fields":{"label":"Urist", "session_id":"user-owned", "max_wall_millis":18,
                           "resource_uri":"literal resource"}},
                {"id":"2", "resource_uri":format!("df://session/{session}/entity-2~1~abc")}
            ],
            "observation_coverage":{"page_complete":false,"absence_proven":false},
            "continuation":{"mode":"entities","offset":2,"limit":2},
            "query_budget":{"max_wall_millis":59999,"maximum_response_bytes":32768,"max_output_tokens":8192},
            "output_budget":{"full_estimate_tokens":1000,"max_output_tokens":8192,"tier":"full"},
            "agent_turn":{"session_id":session,"turn_id":"presentation-4","active_work":{"mcp_tasks":[]}}
        })
    }

    #[test]
    fn only_declared_owner_and_operational_paths_are_normalized() {
        let first = rows("first-owner");
        let mut second = rows("second-owner");
        second["query_budget"]["max_wall_millis"] = json!(59123);
        second["output_budget"]["full_estimate_tokens"] = json!(1800);
        second["output_budget"]["tier"] = json!("compact_turn");
        second["agent_turn"] =
            json!({"turn_id":"unrelated","active_work":{"mcp_tasks":["process-task"]}});
        assert_eq!(query(&first, "first-owner"), query(&second, "second-owner"));
        let expected = query(&first, "first-owner");
        assert_eq!(expected["profile"], PAYLOAD_PROFILE);
        for (pointer, changed) in [
            ("/rows/0/fields/session_id", json!("different-user-owned")),
            ("/rows/0/fields/max_wall_millis", json!(19)),
            (
                "/rows/0/fields/resource_uri",
                json!("different literal resource"),
            ),
            (
                "/rows/0/resource_uri",
                json!("df://session/foreign/entity-1~1~abc"),
            ),
            (
                "/rows/0/resource_uri",
                json!("df://session/first-owner/entity-1~2~abc"),
            ),
            ("/session_id", json!("foreign-owner")),
            ("/query_budget/max_wall_millis", json!(null)),
            ("/query_budget/max_output_tokens", json!(4000)),
            ("/query_budget/maximum_response_bytes", json!(16000)),
            ("/output_budget/max_output_tokens", json!(4000)),
            ("/output_budget/tier", json!("truncated")),
        ] {
            let mut changed_payload = first.clone();
            if let Some(value) = changed_payload.pointer_mut(pointer) {
                *value = changed;
            }
            assert_ne!(
                query(&changed_payload, "first-owner"),
                expected,
                "{pointer}"
            );
        }
        let mut missing = first.clone();
        if let Some(budget) = missing["query_budget"].as_object_mut() {
            budget.remove("max_wall_millis");
        }
        assert_ne!(query(&missing, "first-owner"), expected);
        let mut minimal = first.clone();
        minimal["output_budget"]["tier"] = json!("minimal");
        let mut presentation_only = minimal.clone();
        presentation_only["output_budget"]["minimal_truncated"] = json!({
            "agent_turn.active_work.mcp_tasks":{"total":2,"returned":1},
        });
        assert_eq!(
            query(&minimal, "first-owner"),
            query(&presentation_only, "first-owner")
        );
        presentation_only["output_budget"]["minimal_truncated"]["rows"] =
            json!({"total":2,"returned":1});
        assert_ne!(
            query(&minimal, "first-owner"),
            query(&presentation_only, "first-owner")
        );
    }

    #[test]
    fn rows_order_projections_coverage_and_ordinary_continuations_are_committed() {
        let payload = rows("owner");
        let expected = query(&payload, "owner");
        for (pointer, changed) in [
            ("/rows/0/fields/label", json!("changed")),
            ("/returned", json!(1)),
            ("/observation_coverage/page_complete", json!(true)),
            ("/observation_coverage/absence_proven", json!(true)),
            ("/continuation/offset", json!(3)),
            ("/continuation/limit", json!(1)),
        ] {
            let mut changed_payload = payload.clone();
            if let Some(value) = changed_payload.pointer_mut(pointer) {
                *value = changed;
            }
            assert_ne!(query(&changed_payload, "owner"), expected, "{pointer}");
        }
        let mut reordered = payload.clone();
        if let Some(rows) = reordered["rows"].as_array_mut() {
            rows.reverse();
        }
        assert_ne!(query(&reordered, "owner"), expected);
    }

    #[test]
    fn every_stateless_query_projection_and_review_retains_its_domain_values() {
        for mode in [
            "summary",
            "entities",
            "observation",
            "terrain",
            "path",
            "search",
            "structured",
        ] {
            let args = json!({"mode":json!({"mode":mode}).to_string()});
            let payload = json!({"ok":true,"session_id":"owner","mode":mode,
                "projection":{"values":[1,2]},"coverage":{"complete":false}});
            let captured = commitment("fortress.query", &args, &payload.to_string(), "owner");
            assert_eq!(captured["profile"], PAYLOAD_PROFILE, "{mode}");
            let mut changed = payload;
            changed["projection"]["values"][1] = json!(3);
            assert_ne!(
                captured,
                commitment("fortress.query", &args, &changed.to_string(), "owner")
            );
        }
        let review = json!({"ok":true,"session_id":"owner","plan_digest":"same seal",
            "steps":[{"risk":"guarded","duration":100}],
            "forecast":{"predicted_complete":false,"predicted_goal_truth":"unknown"}});
        let captured = commitment("fortress.plan", &json!({}), &review.to_string(), "owner");
        for (pointer, changed) in [
            ("/steps/0/risk", json!("reversible")),
            ("/steps/0/duration", json!(101)),
            ("/forecast/predicted_complete", json!(true)),
            ("/forecast/predicted_goal_truth", json!("true")),
        ] {
            let mut review = review.clone();
            if let Some(value) = review.pointer_mut(pointer) {
                *value = changed;
            }
            assert_ne!(
                captured,
                commitment("fortress.plan", &json!({}), &review.to_string(), "owner")
            );
        }
    }

    #[test]
    fn doctor_counts_and_goal_owners_are_typed_exceptions_not_recursive_erasure() {
        let doctor = json!({"ok":true,"session_id":"first","active_sessions_count":3,
            "active_leases_count":2,"status":"healthy","findings":[]});
        let mut another = doctor.clone();
        another["session_id"] = json!("second");
        another["active_sessions_count"] = json!(15);
        let captured = commitment("fortress.doctor", &json!({}), &doctor.to_string(), "first");
        assert_eq!(
            captured,
            commitment(
                "fortress.doctor",
                &json!({}),
                &another.to_string(),
                "second"
            )
        );
        another["active_leases_count"] = json!(1);
        assert_ne!(
            captured,
            commitment(
                "fortress.doctor",
                &json!({}),
                &another.to_string(),
                "second"
            )
        );
        let observation = |owner: &str| {
            json!({"ok":true,"session_id":owner,"objectives":[{
            "owner_session_id":owner,"original_source":{"session_id":"literal user source"},
            "continuation_request":{"arguments":{"session_id":owner,"summary":"same"}},
            "withdrawal":null,"observed_truth":"false"}]})
        };
        let first = observation("first");
        let mut second = observation("second");
        let captured = commitment("fortress.observe", &json!({}), &first.to_string(), "first");
        assert_eq!(
            captured,
            commitment(
                "fortress.observe",
                &json!({}),
                &second.to_string(),
                "second"
            )
        );
        second["objectives"][0]["original_source"]["session_id"] = json!("different user source");
        assert_ne!(
            captured,
            commitment(
                "fortress.observe",
                &json!({}),
                &second.to_string(),
                "second"
            )
        );
        second["objectives"][0]["withdrawal"] = json!({"proof_digest":"unverified"});
        assert_eq!(
            commitment(
                "fortress.observe",
                &json!({}),
                &second.to_string(),
                "second"
            )["reason"],
            "session_bound_objective_evidence"
        );
    }

    #[test]
    fn unsupported_stateful_results_and_opaque_handles_have_explicit_coverage() {
        for tool in [
            "fortress.open_session",
            "fortress.commit",
            "fortress.wait",
            "fortress.cancel",
            "fortress.checkpoint",
            "fortress.restore",
        ] {
            let captured = commitment(tool, &json!({}), "{}", "owner");
            assert_eq!(captured["profile"], OUTCOME_PROFILE);
            assert_eq!(captured["reason"], "tool_not_in_profile");
            assert!(valid(&captured));
        }
        for mode in [
            "history",
            "query_changes",
            "condition_replay",
            "forecast_originals",
            "civilian_recovery",
            "future_mode",
        ] {
            let args = json!({"mode":json!({"mode":mode}).to_string()});
            assert_eq!(
                commitment("fortress.query", &args, "{}", "owner")["reason"],
                "query_mode_not_in_profile"
            );
        }
        let args = json!({"mode":json!({"mode":"structured"}).to_string()});
        for pointer in [
            "/continuation",
            "/coverage/continuation",
            "/observation_coverage/continuation",
        ] {
            let mut payload = json!({"mode":"structured","continuation":null,
                "coverage":{"continuation":null},"observation_coverage":{"continuation":null}});
            if let Some(value) = payload.pointer_mut(pointer) {
                *value = json!("session-bound-cursor");
            }
            assert_eq!(
                commitment("fortress.query", &args, &payload.to_string(), "owner")["reason"],
                "opaque_query_continuation"
            );
        }
        assert_eq!(
            commitment("fortress.explain", &json!({}), "{}", "owner")["reason"],
            "explanation_not_entity"
        );
        assert_eq!(
            commitment(
                "fortress.explain",
                &json!({"entity_id":"12"}),
                "{}",
                "owner"
            )["profile"],
            PAYLOAD_PROFILE
        );
        for key in ["query_baselines", "query_watches", "condition_watches"] {
            let mut payload = json!({"ok":true,"query_work":{}});
            payload["query_work"][key] = json!([{"id":"process-local"}]);
            assert_eq!(
                commitment(
                    "fortress.observe",
                    &json!({}),
                    &payload.to_string(),
                    "owner"
                )["reason"],
                "process_local_query_work"
            );
        }
    }

    #[test]
    fn structured_errors_and_object_canonicalization_are_deterministic() {
        let first = r#"{"ok":false,"error":{"code":"invalid_request","details":{"b":2,"a":1},"message":"bad filter"}}"#;
        let second = r#"{"error":{"message":"bad filter","details":{"a":1,"b":2},"code":"invalid_request"},"ok":false}"#;
        let args = json!({"mode":"entities"});
        let captured = commitment("fortress.query", &args, first, "owner");
        assert_eq!(
            captured,
            commitment("fortress.query", &args, second, "owner")
        );
        assert_ne!(
            captured,
            commitment(
                "fortress.query",
                &args,
                &first.replace("bad filter", "different failure"),
                "owner"
            )
        );
        assert_ne!(
            captured,
            commitment(
                "fortress.query",
                &args,
                &first.replace("\"b\":2", "\"b\":3"),
                "owner"
            )
        );
        let mut material = DOMAIN.to_vec();
        material.extend_from_slice(br#"{"result":{"error":{"code":"invalid_request","details":{"a":1,"b":2},"message":"bad filter"},"ok":false},"tool":"fortress.query"}"#);
        assert_eq!(captured["digest"], Digest32::of_bytes(&material).to_hex());
    }

    #[test]
    fn malformed_oversized_and_excessive_work_results_do_not_claim_equality() {
        let args = json!({"mode":"entities"});
        for payload in ["not json", "null", "[]"] {
            assert_eq!(
                commitment("fortress.query", &args, payload, "owner")["reason"],
                "malformed_payload"
            );
        }
        assert_eq!(
            commitment(
                "fortress.query",
                &args,
                &" ".repeat(MAX_RESULT_BYTES + 1),
                "owner"
            )["reason"],
            "payload_outside_bounds"
        );
        let many = json!({"rows":vec![Value::Null; MAX_RESULT_NODES]});
        assert_eq!(
            commitment("fortress.query", &args, &many.to_string(), "owner")["reason"],
            "payload_outside_bounds"
        );
        let mut deep = json!(0);
        for _ in 0..=MAX_RESULT_DEPTH {
            deep = json!([deep]);
        }
        assert_eq!(
            commitment(
                "fortress.query",
                &args,
                &json!({"deep":deep}).to_string(),
                "owner"
            )["reason"],
            "payload_outside_bounds"
        );
    }

    #[test]
    fn commitments_are_closed_and_profiles_cannot_be_mixed() {
        let valid_payload =
            json!({"profile":PAYLOAD_PROFILE,"digest":"ab".repeat(32),"reason":null});
        assert!(valid(&valid_payload));
        for changed in [
            json!({"profile":PAYLOAD_PROFILE,"digest":"AB".repeat(32),"reason":null}),
            json!({"profile":PAYLOAD_PROFILE,"digest":null,"reason":null}),
            json!({"profile":PAYLOAD_PROFILE,"digest":"ab".repeat(32),"reason":"tool_not_in_profile"}),
            json!({"profile":OUTCOME_PROFILE,"digest":"ab".repeat(32),"reason":"tool_not_in_profile"}),
            json!({"profile":OUTCOME_PROFILE,"digest":null,"reason":"unknown_reason"}),
            json!({"profile":"dfmcp.replay.tool-payload/2","digest":"ab".repeat(32),"reason":null}),
            json!({"profile":PAYLOAD_PROFILE,"digest":"ab".repeat(32)}),
            json!({"profile":PAYLOAD_PROFILE,"digest":"ab".repeat(32),"reason":null,"extra":true}),
        ] {
            assert!(!valid(&changed), "{changed}");
        }
    }
}
