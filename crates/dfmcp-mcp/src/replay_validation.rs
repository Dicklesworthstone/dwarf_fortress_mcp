//! Validate the whole transcript before opening a disposable session.
//! A replayable flag and content hash cannot authorize shared or durable
//! ownership, unknown engine events, or caller-selected process handles.

use super::*;

fn exact_fields(value: &Value, names: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == names.len() && names.iter().all(|name| object.contains_key(*name))
    })
}

fn hex(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.len() == 64
            && value.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn counter(value: &Value) -> bool {
    value.as_str().is_some_and(|value| {
        value.parse::<u128>().is_ok_and(|number| number.to_string() == value)
    })
}

fn anchor(value: &Value) -> bool {
    exact_fields(value, &["fortress_id", "epoch", "sequence", "game_tick", "state_hash"])
        && value["fortress_id"].as_str().is_some_and(|value| {
            value.parse::<u64>().is_ok_and(|number| number.to_string() == value)
        })
        && ["epoch", "sequence", "game_tick"].iter().all(|key| value[*key].is_u64())
        && hex(&value["state_hash"])
}

fn frontier(value: &Value) -> bool {
    const BUDGET: [&str; 6] = [
        "max_wall_millis", "max_game_ticks", "max_entities",
        "max_bytes", "max_output_tokens", "max_actions",
    ];
    exact_fields(value, &["anchor", "next_request_id", "authority_digest", "original_actions_digest", "budget"])
        && anchor(&value["anchor"])
        && counter(&value["next_request_id"])
        && hex(&value["authority_digest"])
        && hex(&value["original_actions_digest"])
        && exact_fields(&value["budget"], &BUDGET)
        && BUDGET.iter().all(|key| {
            value["budget"][*key].as_u64().is_some_and(|number| number > 0 || *key == "max_game_ticks")
        })
}

pub(super) fn validate(bundle: &Value) -> Result<bool, String> {
    let task_events = match bundle["schema"].as_str() {
        Some(BUNDLE_SCHEMA) => false,
        Some(TASK_BUNDLE_SCHEMA) => true,
        _ => return Err(format!("not a {BUNDLE_SCHEMA} or {TASK_BUNDLE_SCHEMA} bundle")),
    };
    if bundle.to_string().len() > MAX_BUNDLE_BYTES {
        return Err("replay bundle exceeds the explicit byte bound".to_owned());
    }
    if !exact_fields(bundle, &[
        "schema", "server_version", "replayable", "not_replayable_reason",
        "truncated", "calls_digest", "calls", "note",
    ]) || !hex(&bundle["calls_digest"])
        || !bundle["replayable"].is_boolean()
        || !bundle["server_version"].as_str().is_some_and(|value| !value.is_empty() && value.len() <= 64)
        || !bundle["note"].is_string()
        || (bundle["replayable"] == true && !bundle["not_replayable_reason"].is_null())
        || (bundle["replayable"] == false
            && !bundle["not_replayable_reason"].as_str().is_some_and(|value| !value.is_empty() && value.len() <= 256))
    {
        return Err("malformed replay envelope".to_owned());
    }
    if bundle["truncated"] != false {
        return Err("a truncated replay transcript cannot establish equality".to_owned());
    }
    let calls = bundle["calls"].as_array()
        .filter(|calls| !calls.is_empty() && calls.len() <= MAX_RECORDED_CALLS)
        .ok_or_else(|| "replay requires one through 4096 complete ordered entries".to_owned())?;
    for (seq, call) in calls.iter().enumerate() {
        let invalid = || format!("call {seq}: malformed or unknown replay entry");
        if call["seq"].as_u64() != Some(seq as u64)
            || !call["ok"].is_boolean()
            || !(call["error_code"].is_null()
                || call["error_code"].as_str().is_some_and(|code| code.len() <= 64))
        {
            return Err(invalid());
        }
        if call.get("event").is_some() {
            if !task_events || seq == 0 || call["event"] != "task_cancel_phase"
                || !exact_fields(call, &[
                    "seq", "event", "arguments", "before", "after", "ok", "error_code", "evidence_digest",
                ])
                || !exact_fields(&call["arguments"], &["plan_digest", "phase"])
                || !hex(&call["arguments"]["plan_digest"])
                || call["arguments"]["plan_digest"] == "0".repeat(64)
                || !matches!(call["arguments"]["phase"].as_str(), Some("request" | "finalize"))
                || !frontier(&call["before"]) || !frontier(&call["after"])
                || !hex(&call["evidence_digest"])
            {
                return Err(invalid());
            }
            continue;
        }
        let tool = call["tool"].as_str().ok_or_else(invalid)?;
        if !matches!(tool,
            "fortress.open_session" | "fortress.observe" | "fortress.query" |
            "fortress.plan" | "fortress.commit" | "fortress.wait" | "fortress.cancel" |
            "fortress.checkpoint" | "fortress.restore" | "fortress.explain" | "fortress.doctor")
            || (seq == 0 && tool != "fortress.open_session")
            || !call["arguments"].is_object()
            || call["arguments"].get("session_id").is_some()
            || !(call["anchor_after"].is_null() || anchor(&call["anchor_after"]))
        {
            return Err(invalid());
        }
        let fields = if task_events {
            &["seq", "tool", "arguments", "ok", "error_code", "anchor_after", "next_request_after"][..]
        } else {
            &["seq", "tool", "arguments", "ok", "error_code", "anchor_after"][..]
        };
        if !exact_fields(call, fields) || (task_events && !counter(&call["next_request_after"])) {
            return Err(invalid());
        }
        if tool == "fortress.open_session" {
            let args = &call["arguments"];
            if ["shared", "durable"].iter().any(|key| {
                args.get(*key).is_some_and(|value| !value.is_null() && *value != false)
            }) {
                return Err(format!("call {seq}: replay cannot join shared or durable fortress state"));
            }
            if (seq == 0 && args["existing_session"] == true)
                || (seq > 0 && args["existing_session"] != true)
            {
                return Err(format!("call {seq}: replay opens exactly one fresh private session"));
            }
        }
    }
    Ok(task_events)
}
