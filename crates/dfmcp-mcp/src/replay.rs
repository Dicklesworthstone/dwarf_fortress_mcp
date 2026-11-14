//! Deterministic replay bundles for laboratory sessions.
//!
//! Every tool call is recorded with its exact arguments, outcome and canonical
//! anchor. Tool-only logs retain the unchanged dfmcp.replay.bundle/1 encoding.
//! Application-owned Task drain phases select /2, adding exact input/output
//! witnesses and normalized cancellation evidence. Neither format reconstructs
//! Tasks or reaches a native adapter.
//!
//! Shared or durable sessions depend on inputs outside their own call log and
//! remain nonreplayable. The replayer validates the entire transcript before
//! opening one fresh private laboratory session.

use dfmcp_core::{Digest32, StateAnchor};
use serde_json::{Value, json};

#[path = "replay_validation.rs"]
mod validation;

/// Schema identifier of a tool-only replay bundle.
pub const BUNDLE_SCHEMA: &str = "dfmcp.replay.bundle/1";
/// Schema identifier of a bundle containing original-plan Task drains.
pub const TASK_BUNDLE_SCHEMA: &str = "dfmcp.replay.bundle/2";
/// Most entries one session records; later entries mark the log truncated.
pub const MAX_RECORDED_CALLS: usize = 4_096;
/// Largest bundle the replayer accepts.
pub const MAX_BUNDLE_BYTES: usize = 32 * 1024 * 1024;

/// One recorded tool call.
#[derive(Clone, Debug)]
pub(crate) struct RecordedCall {
    tool: &'static str,
    arguments: Value,
    ok: bool,
    error_code: Option<String>,
    anchor_after: Option<StateAnchor>,
    /// Serialized only in /2, preserving every existing /1 byte.
    next_request_after: u128,
}

/// An application-owned engine transition, not a public fortress tool.
#[derive(Clone, Debug)]
pub(crate) struct RecordedTaskDrain {
    pub(crate) plan_digest: String,
    pub(crate) finalize: bool,
    pub(crate) before: Value,
    pub(crate) after: Value,
    pub(crate) ok: bool,
    pub(crate) error_code: Option<String>,
    pub(crate) evidence_digest: String,
}

#[derive(Clone, Debug)]
enum RecordedEntry {
    Tool(RecordedCall),
    TaskDrain(RecordedTaskDrain),
}

/// A session's bounded engine transcript.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReplayLog {
    calls: Vec<RecordedEntry>,
    truncated: bool,
    /// Why the log cannot be replayed from its own contents, if it cannot.
    not_replayable: Option<&'static str>,
}

impl ReplayLog {
    pub(crate) fn mark_not_replayable(&mut self, reason: &'static str) {
        self.not_replayable.get_or_insert(reason);
    }

    fn has_capacity(&mut self) -> bool {
        if self.calls.len() < MAX_RECORDED_CALLS {
            return true;
        }
        self.truncated = true;
        self.mark_not_replayable("the call log exceeded its bound and was truncated");
        false
    }

    /// Called under engine ownership, including after a partially applied
    /// failing phase. A late facade recorder cannot turn an unrecorded
    /// predecessor into a claimed reproducible ordering.
    pub(crate) fn record_task_drain(&mut self, event: RecordedTaskDrain) {
        let predecessor = self.calls.last().map(|entry| match entry {
            RecordedEntry::Tool(call) => (
                call.anchor_after.as_ref().map(anchor_json).unwrap_or(Value::Null),
                json!(call.next_request_after.to_string()),
            ),
            RecordedEntry::TaskDrain(event) => (
                event.after["anchor"].clone(),
                event.after["next_request_id"].clone(),
            ),
        });
        if predecessor
            != Some((
                event.before["anchor"].clone(),
                event.before["next_request_id"].clone(),
            ))
        {
            self.mark_not_replayable(
                "Task cancellation encountered an engine transition not yet recorded in call order",
            );
        }
        if self.has_capacity() {
            self.calls.push(RecordedEntry::TaskDrain(event));
        }
    }
}

fn anchor_json(anchor: &StateAnchor) -> Value {
    json!({
        "fortress_id": anchor.fortress_id.get().to_string(),
        "epoch": anchor.cursor.epoch,
        "sequence": anchor.cursor.sequence,
        "game_tick": anchor.tick.0,
        "state_hash": anchor.state_hash.to_hex(),
    })
}

fn outcome(payload: &str) -> (bool, Option<String>, Option<String>) {
    let parsed: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
    let ok = parsed["ok"].as_bool().unwrap_or(false);
    let code = parsed["error"]["code"].as_str().map(str::to_owned);
    let session = parsed["session_id"].as_str().map(str::to_owned);
    (ok, code, session)
}

/// Record one completed facade call against its addressed or created session.
pub(crate) fn record(
    tool: &'static str,
    session_id: Option<&str>,
    mut arguments: Value,
    payload: &str,
) {
    let (ok, error_code, created) = outcome(payload);
    let target = session_id.map(str::to_owned).or(created);
    let Some(target) = target else { return };
    let Ok(session) = crate::server::lookup_session_str(&target) else {
        return;
    };
    if let Some(object) = arguments.as_object_mut() {
        object.remove("session_id");
    }
    if ok && tool == "fortress.open_session" && arguments["existing_session"] == true {
        // Metadata transitions can share an anchor: bind their accepted
        // allowances so reordered negotiations cannot claim replay equality.
        let receipt: Value = serde_json::from_str(payload).unwrap_or(Value::Null);
        arguments["previous_max_output_tokens"] =
            receipt["previous_budget"]["max_output_tokens"].clone();
        arguments["admitted_max_output_tokens"] = receipt["budget"]["max_output_tokens"].clone();
    }
    let Ok(mut guard) = session.lock() else {
        return;
    };
    let anchor_after = Some(guard.adapter.snapshot().anchor());
    let next_request_after = crate::server::read_context(&guard).request_id.get();
    let log = &mut guard.replay;
    if ok
        && tool == "fortress.query"
        && arguments
            .get("mode")
            .and_then(Value::as_str)
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
            .is_some_and(|query| {
                (matches!(query["mode"].as_str(), Some("query_changes" | "history"))
                    && query["continuation"].is_string())
                    || (query["mode"] == "structured"
                        && query["request"]["query"]["continuation"].is_string())
            })
    {
        log.mark_not_replayable(
            "query continuations are bound to the original session and cannot be replayed into a new session",
        );
    }
    if tool == "fortress.wait"
        && (arguments["watch_id"].is_string() || arguments["watch_ids"].is_array())
    {
        log.mark_not_replayable(
            "watch-selected waits name process-local handles owned by the original session",
        );
    }
    if !log.has_capacity() {
        return;
    }
    log.calls.push(RecordedEntry::Tool(RecordedCall {
        tool,
        arguments,
        ok,
        error_code,
        anchor_after,
        next_request_after,
    }));
}

fn calls_json(log: &ReplayLog, task_events: bool) -> Value {
    Value::Array(
        log.calls
            .iter()
            .enumerate()
            .map(|(seq, entry)| match entry {
                RecordedEntry::Tool(call) => {
                    let mut value = json!({
                        "seq": seq,
                        "tool": call.tool,
                        "arguments": call.arguments,
                        "ok": call.ok,
                        "error_code": call.error_code,
                        "anchor_after": call.anchor_after.as_ref().map(anchor_json),
                    });
                    if task_events {
                        value["next_request_after"] = json!(call.next_request_after.to_string());
                    }
                    value
                }
                RecordedEntry::TaskDrain(event) => json!({
                    "seq": seq,
                    "event": "task_cancel_phase",
                    "arguments": {
                        "plan_digest": event.plan_digest,
                        "phase": if event.finalize { "finalize" } else { "request" },
                    },
                    "before": event.before,
                    "after": event.after,
                    "ok": event.ok,
                    "error_code": event.error_code,
                    "evidence_digest": event.evidence_digest,
                }),
            })
            .collect(),
    )
}

fn calls_digest(calls: &Value) -> String {
    let mut bytes = b"dfmcp-replay-bundle-calls/1\0".to_vec();
    bytes.extend_from_slice(calls.to_string().as_bytes());
    Digest32::of_bytes(&bytes).to_hex()
}

fn task_calls_digest(calls: &Value) -> String {
    let mut bytes = b"dfmcp-replay-bundle-calls/2\0".to_vec();
    bytes.extend_from_slice(calls.to_string().as_bytes());
    Digest32::of_bytes(&bytes).to_hex()
}

/// Export a session's log without changing legacy tool-only bundle bytes.
pub(crate) fn bundle_json(log: &ReplayLog) -> Value {
    let task_events = log.calls.iter().any(|entry| matches!(entry, RecordedEntry::TaskDrain(_)));
    let calls = calls_json(log, task_events);
    json!({
        "schema": if task_events { TASK_BUNDLE_SCHEMA } else { BUNDLE_SCHEMA },
        "server_version": env!("CARGO_PKG_VERSION"),
        "replayable": log.not_replayable.is_none(),
        "not_replayable_reason": log.not_replayable,
        "truncated": log.truncated,
        "calls_digest": if task_events { task_calls_digest(&calls) } else { calls_digest(&calls) },
        "calls": calls,
        "note": "replay with `dwarf-fortress-mcp replay <bundle.json>`; every call is re-executed in a fresh laboratory session and compared by outcome and resulting canonical anchor"
    })
}

fn opt_string(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::to_owned)
}

fn opt_watch_ids(args: &Value) -> Result<Option<Vec<String>>, String> {
    let Some(value) = args.get("watch_ids").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let handles = value.as_array().filter(|handles| !handles.is_empty() && handles.len() <= 8)
        .ok_or_else(|| "recorded watch_ids must be a nonempty set of at most eight strings".to_owned())?;
    handles.iter().map(|handle| handle.as_str().map(str::to_owned)
        .ok_or_else(|| "recorded watch_ids contains a non-string handle".to_owned()))
        .collect::<Result<Vec<_>, _>>().map(Some)
}

fn opt_bool(args: &Value, key: &str) -> Option<bool> {
    args[key].as_bool()
}

fn opt_u64(args: &Value, key: &str) -> Option<u64> {
    args[key].as_u64()
}

fn opt_u32(args: &Value, key: &str) -> Option<u32> {
    args[key]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
}

fn capabilities(args: &Value) -> Option<Vec<(String, String)>> {
    args["requested_capabilities"].as_array().map(|pairs| {
        pairs
            .iter()
            .filter_map(|pair| Some((pair[0].as_str()?.to_owned(), pair[1].as_str()?.to_owned())))
            .collect()
    })
}

/// Invoke one tool of the agent facade with recorded arguments.
fn invoke(tool: &str, session: Option<String>, args: &Value) -> Result<String, String> {
    use crate::agent_facade as f;
    let session_or = |name: &str| {
        session
            .clone()
            .ok_or_else(|| format!("{name} needs a session opened earlier in the bundle"))
    };
    Ok(match tool {
        "fortress.open_session" => f::fortress_open_session_with_session(
            if args["existing_session"] == true {
                Some(session_or(tool)?)
            } else {
                None
            },
            opt_bool(args, "paused"),
            opt_string(args, "fortress_selector"),
            capabilities(args),
            opt_u64(args, "max_wall_millis"),
            opt_u64(args, "max_game_ticks"),
            opt_u32(args, "max_entities"),
            opt_u64(args, "max_bytes"),
            opt_u32(args, "max_output_tokens"),
            opt_u32(args, "max_actions"),
            opt_string(args, "scenario"),
            opt_bool(args, "shared"),
            opt_bool(args, "durable"),
        ),
        "fortress.observe" => f::fortress_observe(Some(session_or(tool)?)),
        "fortress.query" => f::fortress_query(Some(session_or(tool)?), opt_string(args, "mode")),
        "fortress.plan" => f::fortress_plan(
            Some(session_or(tool)?),
            opt_string(args, "summary"),
            opt_bool(args, "paused_target"),
            opt_string(args, "actions"),
            opt_string(args, "blueprint"),
        ),
        "fortress.commit" => f::fortress_commit(
            Some(session_or(tool)?),
            opt_string(args, "plan_digest").unwrap_or_default(),
        ),
        "fortress.wait" => f::fortress_wait_for_targets(
            Some(session_or(tool)?),
            opt_u64(args, "max_game_ticks"),
            opt_string(args, "plan_digest"),
            opt_string(args, "watch_id"),
            opt_watch_ids(args)?,
            opt_string(args, "watch_policy"),
        ),
        "fortress.cancel" => f::fortress_cancel(
            Some(session_or(tool)?),
            opt_string(args, "mode"),
            opt_string(args, "scope"),
        ),
        "fortress.checkpoint" => {
            f::fortress_checkpoint(Some(session_or(tool)?), opt_string(args, "label"))
        }
        "fortress.restore" => f::fortress_restore(
            Some(session_or(tool)?),
            opt_string(args, "checkpoint_id").unwrap_or_default(),
        ),
        "fortress.explain" => {
            f::fortress_explain(Some(session_or(tool)?), opt_string(args, "entity_id"))
        }
        "fortress.doctor" => f::fortress_doctor(Some(session_or(tool)?)),
        other => return Err(format!("unknown tool {other:?} in bundle")),
    })
}

fn divergence(seq: usize, tool: &str, field: &str, expected: &Value, observed: &Value) -> Value {
    json!({
        "seq": seq,
        "tool": tool,
        "field": field,
        "expected": expected,
        "observed": observed,
    })
}

/// Re-execute a complete transcript in one fresh private laboratory session.
/// The first mismatch stops replay; event input mismatches stop before effects.
#[must_use]
pub fn replay_bundle(bundle: &Value) -> Value {
    let refuse = |reason: String| json!({"ok": false, "replayed": 0, "error": reason});
    let task_events = match validation::validate(bundle) {
        Ok(task_events) => task_events,
        Err(reason) => return refuse(reason),
    };
    if bundle["replayable"] != true {
        return refuse(format!(
            "bundle is not replayable: {}",
            bundle["not_replayable_reason"].as_str().unwrap_or("unspecified")
        ));
    }
    let Some(calls) = bundle["calls"].as_array() else {
        return refuse("bundle has no calls array".to_owned());
    };
    let observed_digest = if task_events {
        task_calls_digest(&bundle["calls"])
    } else {
        calls_digest(&bundle["calls"])
    };
    if observed_digest != bundle["calls_digest"].as_str().unwrap_or_default() {
        return refuse("bundle calls do not match calls_digest".to_owned());
    }
    let mut session: Option<String> = None;
    for (seq, call) in calls.iter().enumerate() {
        if call["event"] == "task_cancel_phase" {
            let Some(session_id) = session.as_deref() else {
                return refuse("Task drain event needs an earlier opened session".to_owned());
            };
            if let Some(report) = replay_task_drain(seq, call, session_id) {
                return report;
            }
            continue;
        }
        let tool = call["tool"].as_str().unwrap_or_default();
        let payload = match invoke(tool, session.clone(), &call["arguments"]) {
            Ok(payload) => payload,
            Err(reason) => return refuse(format!("call {seq}: {reason}")),
        };
        let (ok, error_code, created) = outcome(&payload);
        if seq == 0 {
            session = created;
        }
        for (field, observed) in [("ok", json!(ok)), ("error_code", json!(error_code))] {
            if observed != call[field] {
                return json!({
                    "ok": false, "replayed": seq + 1,
                    "first_divergence": divergence(seq, tool, field, &call[field], &observed),
                });
            }
        }
        if ok && tool == "fortress.open_session" && call["arguments"]["existing_session"] == true {
            let receipt: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
            for (field, budget) in [
                ("previous_max_output_tokens", "previous_budget"),
                ("admitted_max_output_tokens", "budget"),
            ] {
                let expected = &call["arguments"][field];
                let observed = &receipt[budget]["max_output_tokens"];
                if !expected.is_u64() || observed != expected {
                    return json!({
                        "ok": false, "replayed": seq + 1,
                        "first_divergence": divergence(seq, tool, field, expected, observed),
                    });
                }
            }
        }
        let observed = session
            .as_deref()
            .and_then(|id| crate::server::lookup_session_str(id).ok())
            .and_then(|s| {
                s.lock().ok().map(|g| {
                    (anchor_json(&g.adapter.snapshot().anchor()),
                        json!(crate::server::read_context(&g).request_id.get().to_string()))
                })
            });
        let (anchor_after, next_request_after) = observed.unwrap_or((Value::Null, Value::Null));
        if anchor_after != call["anchor_after"] {
            return json!({
                "ok": false, "replayed": seq + 1,
                "first_divergence": divergence(seq, tool, "anchor_after", &call["anchor_after"], &anchor_after),
            });
        }
        if task_events && next_request_after != call["next_request_after"] {
            return json!({
                "ok": false, "replayed": seq + 1,
                "first_divergence": divergence(seq, tool, "next_request_after", &call["next_request_after"], &next_request_after),
            });
        }
    }
    json!({
        "ok": true,
        "replayed": calls.len(),
        "first_divergence": null,
        "replay_session_id": session,
        "note": "every call reproduced its recorded outcome and canonical anchor",
    })
}

/// Events replay the existing authority-bearing drain with no Task object.
/// Expected witnesses describe inputs; they never install grants or state.
fn replay_task_drain(seq: usize, call: &Value, session_id: &str) -> Option<Value> {
    use crate::server::task_session;
    let Some(requested) = call["arguments"]["plan_digest"].as_str() else {
        return Some(json!({"ok":false,"replayed":seq,"error":"missing original plan selector"}));
    };
    let observed = match task_session::replay::checked_drain(
        session_id, requested, call["arguments"]["phase"] == "finalize", &call["before"],
    ) {
        Ok(task_session::replay::CheckedPhase::InputDivergence(before)) => {
            return Some(json!({
                "ok":false, "replayed":seq,
                "first_divergence":divergence(seq, "task_cancel_phase", "before", &call["before"], &before),
                "event_effects_attempted":false,
            }));
        }
        Ok(task_session::replay::CheckedPhase::Executed { result, after, evidence_digest }) => json!({
            "ok":result.is_ok(),
            "error_code":result.as_ref().err().map(|error| error.code.as_str()),
            "after":after,
            "evidence_digest":evidence_digest,
        }),
        Err(error) => {
            return Some(json!({
                "ok":false, "replayed":seq,
                "first_divergence":divergence(
                    seq, "task_cancel_phase", "before", &call["before"],
                    &json!({"error_code":error.code.as_str()}),
                ),
                "event_effects_attempted":false,
            }));
        }
    };
    for field in ["ok", "error_code", "after", "evidence_digest"] {
        if observed[field] != call[field] {
            return Some(json!({
                "ok":false,"replayed":seq + 1,
                "first_divergence":divergence(seq, "task_cancel_phase", field, &call[field], &observed[field]),
            }));
        }
    }
    None
}

#[cfg(test)]
#[path = "replay_task_tests.rs"]
mod task_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_facade as f;

    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

    fn parsed(raw: &str) -> std::result::Result<Value, Box<dyn std::error::Error>> {
        Ok(serde_json::from_str(raw)?)
    }

    #[test]
    fn a_recorded_session_replays_exactly_and_tampering_is_localized() -> TestResult {
        let caps = vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "reversible".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("restore".to_owned(), "guarded".to_owned()),
            ("designate".to_owned(), "guarded".to_owned()),
        ];
        let opened = parsed(&f::fortress_open_session(
            Some(false),
            Some("660123".to_owned()),
            Some(caps),
            None,
            Some(10_000),
            None,
            None,
            Some(8_192),
            None,
            Some("starter_fortress".to_owned()),
            None,
            None,
        ))?;
        let session = opened["session_id"].as_str().ok_or("session")?.to_owned();
        let plan = parsed(&f::fortress_plan(
            Some(session.clone()),
            None,
            None,
            Some(
                r#"[{"action":{"kind":"designate_dig","min":[1,3,10],"max":[4,4,10],"mode":"mine"}}]"#
                    .to_owned(),
            ),
            None,
        ))?;
        let digest = plan["plan_digest"].as_str().ok_or("digest")?.to_owned();
        parsed(&f::fortress_commit(Some(session.clone()), digest))?;
        let checkpoint = parsed(&f::fortress_checkpoint(Some(session.clone()), None))?;
        parsed(&f::fortress_wait(Some(session.clone()), Some(100)))?;
        // A refused call is part of the record too.
        let refused = parsed(&f::fortress_commit(Some(session.clone()), "00".repeat(32)))?;
        assert_eq!(refused["ok"], false);
        let checkpoint_id = checkpoint["checkpoint_id"].as_str().ok_or("cp")?.to_owned();
        parsed(&f::fortress_restore(Some(session.clone()), checkpoint_id))?;
        parsed(&f::fortress_observe(Some(session.clone())))?;

        let bundle = crate::server::replay_bundle_for(&session)?;
        assert_eq!(bundle["schema"], BUNDLE_SCHEMA);
        assert_eq!(bundle["replayable"], true);
        assert_eq!(bundle["calls"].as_array().map(Vec::len), Some(8));

        let report = replay_bundle(&bundle);
        assert_eq!(report["ok"], true, "{report}");
        assert_eq!(report["replayed"], 8);
        // The replayed session's own bundle is byte-identical to the original.
        let replayed = report["replay_session_id"]
            .as_str()
            .ok_or("replay session")?;
        assert_eq!(
            crate::server::replay_bundle_for(replayed)?.to_string(),
            bundle.to_string()
        );
        if std::env::var_os("DFMCP_REGENERATE_GOLDEN").is_some() {
            std::fs::write(
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../",
                    "schemas/examples/replay_bundle_v1.json"
                ),
                serde_json::to_string_pretty(&bundle)? + "\n",
            )?;
        }

        // Changing a recorded wait localizes the divergence to that call.
        let mut tampered = bundle.clone();
        tampered["calls"][4]["arguments"]["max_game_ticks"] = json!(99);
        tampered["calls_digest"] = json!(calls_digest(&tampered["calls"]));
        let report = replay_bundle(&tampered);
        assert_eq!(report["ok"], false);
        assert_eq!(report["first_divergence"]["seq"], 4, "{report}");
        assert_eq!(report["first_divergence"]["field"], "anchor_after");

        // An edited bundle without a matching digest is refused outright.
        let mut forged = bundle;
        forged["calls"][1]["arguments"]["summary"] = json!("other");
        assert_eq!(replay_bundle(&forged)["ok"], false);
        Ok(())
    }

    /// The checked-in golden bundle must keep replaying with zero divergence:
    /// a change in laboratory semantics is localized to its first call.
    #[test]
    fn the_golden_bundle_still_replays_exactly() -> TestResult {
        let golden: Value = serde_json::from_str(include_str!(
            "../../../schemas/examples/replay_bundle_v1.json"
        ))?;
        let report = replay_bundle(&golden);
        assert_eq!(report["ok"], true, "golden replay diverged: {report}");
        assert_eq!(report["first_divergence"], Value::Null);
        Ok(())
    }

    /// Replay-equality campaign: every laboratory scenario, driven through a
    /// broad tool mix (queries of every mode, plans, commits, waits,
    /// checkpoints, restores, cancels, explains, doctor, refused calls),
    /// replays with zero divergence and re-exports a byte-identical bundle.
    #[test]
    fn every_scenario_replays_with_zero_divergence() -> TestResult {
        let caps: Vec<(String, String)> = [
            ("observe", "read_only"),
            ("query", "read_only"),
            ("plan", "reversible"),
            ("control_clock", "reversible"),
            ("checkpoint", "guarded"),
            ("restore", "guarded"),
            ("designate", "guarded"),
            ("doctor", "read_only"),
            ("construct", "guarded"),
            ("configure_labor", "reversible"),
            ("configure_production", "reversible"),
            ("configure_military", "guarded"),
            ("configure_logistics", "guarded"),
        ]
        .iter()
        .map(|(c, r)| ((*c).to_owned(), (*r).to_owned()))
        .collect();
        for (index, scenario) in crate::lab_world::SCENARIOS.iter().enumerate() {
            let opened = parsed(&f::fortress_open_session(
                Some(false),
                Some(format!("66090{index}")),
                Some(caps.clone()),
                None,
                Some(50_000),
                None,
                None,
                Some(8_192),
                None,
                Some((*scenario).to_owned()),
                None,
                None,
            ))?;
            let session = opened["session_id"]
                .as_str()
                .ok_or_else(|| opened.to_string())?
                .to_owned();
            let s = || Some(session.clone());
            for mode in [
                None,
                Some(r#"{"mode":"entities","kind":"unit","limit":5}"#),
                Some(r#"{"mode":"terrain","min":[0,0,10],"max":[9,4,10]}"#),
                Some(r#"{"mode":"search","text":"Urist","limit":3}"#),
                Some(r#"{"mode":"path","from":[0,2,10],"to":[9,2,10]}"#),
                Some(r#"{"mode":"entities","where":{"field":"no.such","op":"lt","value":1}}"#),
            ] {
                parsed(&f::fortress_query(s(), mode.map(str::to_owned)))?;
            }
            let plan = parsed(&f::fortress_plan(
                s(),
                None,
                None,
                Some(r#"[{"action":{"kind":"designate_dig","min":[1,3,10],"max":[3,4,10],"mode":"mine"}}]"#.to_owned()),
                None,
            ))?;
            if let Some(digest) = plan["plan_digest"].as_str() {
                parsed(&f::fortress_commit(s(), digest.to_owned()))?;
            }
            let checkpoint = parsed(&f::fortress_checkpoint(s(), Some("mid".to_owned())))?;
            for _ in 0..3 {
                parsed(&f::fortress_wait(s(), Some(400)))?;
            }
            parsed(&f::fortress_cancel(s(), None, None))?;
            parsed(&f::fortress_explain(s(), Some("1001".to_owned())))?;
            parsed(&f::fortress_doctor(s()))?;
            parsed(&f::fortress_commit(s(), "ab".repeat(32)))?;
            if let Some(id) = checkpoint["checkpoint_id"].as_str() {
                parsed(&f::fortress_restore(s(), id.to_owned()))?;
            }
            parsed(&f::fortress_wait(s(), Some(250)))?;
            parsed(&f::fortress_observe(s()))?;

            let bundle = crate::server::replay_bundle_for(&session)?;
            assert_eq!(bundle["replayable"], true, "{scenario}");
            let calls = bundle["calls"].as_array().map_or(0, Vec::len);
            assert!(calls >= 18, "{scenario}: {calls} calls");
            let report = replay_bundle(&bundle);
            assert_eq!(report["ok"], true, "{scenario}: {report}");
            assert_eq!(report["replayed"], calls);
            let replayed = report["replay_session_id"]
                .as_str()
                .ok_or("replay session")?;
            assert_eq!(
                crate::server::replay_bundle_for(replayed)?.to_string(),
                bundle.to_string(),
                "{scenario}: replay re-exports a byte-identical bundle"
            );
        }
        Ok(())
    }
}
