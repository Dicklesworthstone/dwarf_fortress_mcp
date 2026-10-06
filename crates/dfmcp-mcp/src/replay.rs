//! Deterministic replay bundles for laboratory sessions.
//!
//! Every tool call of a laboratory session is recorded with its exact
//! arguments, its outcome and the canonical anchor it left behind. The log is
//! exported as a `dfmcp.replay.bundle/1` (`df://session/{id}/replay`), and
//! [`replay_bundle`] re-executes a bundle in a fresh session and reports the
//! earliest call whose outcome or resulting anchor differs. Because the
//! laboratory is deterministic, a divergence localizes a change in semantics
//! (or a nondeterminism bug) to one call and one field.
//!
//! Sessions that share a fortress or resume durable state depend on inputs
//! outside their own call log, so their bundles are marked not replayable
//! rather than replayed into a false divergence.

use dfmcp_core::{Digest32, StateAnchor};
use serde_json::{Value, json};

/// Schema identifier of a replay bundle.
pub const BUNDLE_SCHEMA: &str = "dfmcp.replay.bundle/1";
/// Most calls one session records; later calls mark the log truncated.
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
}

/// A session's bounded call log.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReplayLog {
    calls: Vec<RecordedCall>,
    truncated: bool,
    /// Why the log cannot be replayed from its own contents, if it cannot.
    not_replayable: Option<&'static str>,
}

impl ReplayLog {
    pub(crate) fn mark_not_replayable(&mut self, reason: &'static str) {
        self.not_replayable.get_or_insert(reason);
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

/// Record one completed tool call against the session it addressed (or, for
/// `fortress.open_session`, the session it created).
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
    let Ok(mut guard) = session.lock() else {
        return;
    };
    let anchor_after = Some(guard.adapter.snapshot().anchor());
    let log = &mut guard.replay;
    if log.calls.len() >= MAX_RECORDED_CALLS {
        log.truncated = true;
        log.mark_not_replayable("the call log exceeded its bound and was truncated");
        return;
    }
    log.calls.push(RecordedCall {
        tool,
        arguments,
        ok,
        error_code,
        anchor_after,
    });
}

fn calls_json(log: &ReplayLog) -> Value {
    Value::Array(
        log.calls
            .iter()
            .enumerate()
            .map(|(seq, call)| {
                json!({
                    "seq": seq,
                    "tool": call.tool,
                    "arguments": call.arguments,
                    "ok": call.ok,
                    "error_code": call.error_code,
                    "anchor_after": call.anchor_after.as_ref().map(anchor_json),
                })
            })
            .collect(),
    )
}

fn calls_digest(calls: &Value) -> String {
    let mut bytes = b"dfmcp-replay-bundle-calls/1\0".to_vec();
    bytes.extend_from_slice(calls.to_string().as_bytes());
    Digest32::of_bytes(&bytes).to_hex()
}

/// Export a session's log as a replay bundle.
pub(crate) fn bundle_json(log: &ReplayLog) -> Value {
    let calls = calls_json(log);
    json!({
        "schema": BUNDLE_SCHEMA,
        "server_version": env!("CARGO_PKG_VERSION"),
        "replayable": log.not_replayable.is_none(),
        "not_replayable_reason": log.not_replayable,
        "truncated": log.truncated,
        "calls_digest": calls_digest(&calls),
        "calls": calls,
        "note": "replay with `dwarf-fortress-mcp replay <bundle.json>`; every call is re-executed in a fresh laboratory session and compared by outcome and resulting canonical anchor",
    })
}

fn opt_string(args: &Value, key: &str) -> Option<String> {
    args[key].as_str().map(str::to_owned)
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
        "fortress.open_session" => f::fortress_open_session(
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
        "fortress.wait" => {
            f::fortress_wait(Some(session_or(tool)?), opt_u64(args, "max_game_ticks"))
        }
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

/// Re-execute a replay bundle in a fresh laboratory session and report the
/// earliest divergence by outcome, error code or resulting anchor.
#[must_use]
pub fn replay_bundle(bundle: &Value) -> Value {
    let refuse = |reason: String| json!({"ok": false, "replayed": 0, "error": reason});
    if bundle["schema"] != BUNDLE_SCHEMA {
        return refuse(format!("not a {BUNDLE_SCHEMA} bundle"));
    }
    if bundle["replayable"] != true {
        return refuse(format!(
            "bundle is not replayable: {}",
            bundle["not_replayable_reason"]
                .as_str()
                .unwrap_or("unspecified")
        ));
    }
    let Some(calls) = bundle["calls"].as_array() else {
        return refuse("bundle has no calls array".to_owned());
    };
    if calls_digest(&bundle["calls"]) != bundle["calls_digest"].as_str().unwrap_or_default() {
        return refuse("bundle calls do not match calls_digest".to_owned());
    }
    let mut session: Option<String> = None;
    for (seq, call) in calls.iter().enumerate() {
        let tool = call["tool"].as_str().unwrap_or_default();
        if seq == 0 && tool != "fortress.open_session" {
            return refuse("a bundle must start with fortress.open_session".to_owned());
        }
        let payload = match invoke(tool, session.clone(), &call["arguments"]) {
            Ok(payload) => payload,
            Err(reason) => return refuse(format!("call {seq}: {reason}")),
        };
        let (ok, error_code, created) = outcome(&payload);
        if seq == 0 {
            session = created;
        }
        if json!(ok) != call["ok"] {
            return json!({
                "ok": false,
                "replayed": seq + 1,
                "first_divergence": divergence(seq, tool, "ok", &call["ok"], &json!(ok)),
            });
        }
        if json!(error_code) != call["error_code"] {
            return json!({
                "ok": false,
                "replayed": seq + 1,
                "first_divergence": divergence(seq, tool, "error_code", &call["error_code"], &json!(error_code)),
            });
        }
        let observed = session
            .as_deref()
            .and_then(|id| crate::server::lookup_session_str(id).ok())
            .and_then(|s| {
                s.lock()
                    .ok()
                    .map(|g| anchor_json(&g.adapter.snapshot().anchor()))
            });
        let observed = observed.unwrap_or(Value::Null);
        if observed != call["anchor_after"] {
            return json!({
                "ok": false,
                "replayed": seq + 1,
                "first_divergence": divergence(seq, tool, "anchor_after", &call["anchor_after"], &observed),
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
            let session = opened["session_id"].as_str().ok_or_else(|| opened.to_string())?.to_owned();
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
