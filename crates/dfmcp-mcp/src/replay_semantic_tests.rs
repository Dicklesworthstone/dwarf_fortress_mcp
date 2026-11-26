//! Actual facade transcripts: read-result divergence must not hide behind an
//! unchanged world anchor, and old generations must retain their old bytes.

use super::*;
use crate::agent_facade as f;

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}

fn field(value: &Value, key: &str) -> TestResult<String> {
    Ok(value[key]
        .as_str()
        .ok_or_else(|| format!("missing {key}: {value}"))?
        .to_owned())
}

fn open(selector: &str) -> TestResult<String> {
    let grants = [
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "guarded"),
        ("designate", "guarded"),
        ("control_clock", "reversible"),
    ];
    let opened = parsed(&f::fortress_open_session(
        Some(false),
        Some(selector.to_owned()),
        Some(
            grants
                .into_iter()
                .map(|(capability, risk)| (capability.to_owned(), risk.to_owned()))
                .collect(),
        ),
        Some(60_000),
        Some(2_000),
        Some(8_192),
        None,
        Some(16_384),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(opened["ok"], true, "{opened}");
    field(&opened, "session_id")
}

fn query_session(selector: &str) -> TestResult<(String, Value)> {
    let session = open(selector)?;
    let result = parsed(&f::fortress_query(
        Some(session.clone()),
        Some(json!({"mode":"entities", "kind":"unit", "limit":5}).to_string()),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    assert!(
        result["rows"].as_array().is_some_and(|rows| rows.len() > 1),
        "{result}"
    );
    let bundle = crate::server::replay_bundle_for(&session)?;
    Ok((session, bundle))
}

pub(super) fn legacy(bundle: &Value, generation: BundleGeneration) -> TestResult<Value> {
    assert_ne!(generation, BundleGeneration::SemanticV3);
    let mut legacy = bundle.clone();
    legacy["schema"] = json!(generation.schema());
    legacy["note"] = json!(
        "replay with `dwarf-fortress-mcp replay <bundle.json>`; every call is re-executed in a fresh laboratory session and compared by outcome and resulting canonical anchor"
    );
    for call in legacy["calls"].as_array_mut().ok_or("calls")? {
        if let Some(call) = call
            .as_object_mut()
            .filter(|call| call.contains_key("tool"))
        {
            call.remove("result_commitment");
            if generation == BundleGeneration::ToolsV1 {
                call.remove("next_request_after");
            }
        }
    }
    legacy["calls_digest"] = json!(generation.digest(&legacy["calls"]));
    Ok(legacy)
}

#[test]
fn changed_query_rows_diverge_even_when_outcome_anchor_and_request_counter_match() -> TestResult {
    let _serial = crate::test_serial();
    let (_, bundle) = query_session("7824601")?;
    assert_eq!(bundle["schema"], SEMANTIC_BUNDLE_SCHEMA);
    assert_eq!(
        bundle["calls"][0]["anchor_after"],
        bundle["calls"][1]["anchor_after"]
    );
    assert_eq!(
        bundle["calls"][1]["result_commitment"]["profile"],
        result::PAYLOAD_PROFILE
    );
    let report = replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["verification"]["mode"], "semantic_tool_results");
    assert_eq!(
        report["verification"]["result_profile"],
        result::PAYLOAD_PROFILE
    );
    assert_eq!(report["verification"]["semantic_result_calls"], 1);
    assert_eq!(report["verification"]["outcome_only_calls"], 1);
    assert_eq!(report["verification"]["task_drain_events"], 0);
    assert_eq!(
        report["verification"]["excluded_results"][0]["tool"],
        "fortress.open_session"
    );
    assert_eq!(
        crate::server::replay_bundle_for(&field(&report, "replay_session_id")?)?,
        bundle
    );

    let mut changed = bundle.clone();
    changed["calls"][1]["arguments"]["mode"] = json!(
        json!({
            "mode":"entities", "kind":"unit", "limit":1,
        })
        .to_string()
    );
    changed["calls_digest"] = json!(semantic_calls_digest(&changed["calls"]));
    let report = replay_bundle(&changed);
    assert_eq!(report["ok"], false, "{report}");
    assert_eq!(report["first_divergence"]["seq"], 1);
    assert_eq!(report["first_divergence"]["field"], "result_commitment");
    assert_eq!(
        report["first_divergence"]["expected"],
        bundle["calls"][1]["result_commitment"]
    );
    assert_eq!(
        report["first_divergence"]["observed"]["profile"],
        result::PAYLOAD_PROFILE
    );

    // A syntactically valid weaker claim is recomputed, not trusted.
    let mut weakened = bundle;
    weakened["calls"][1]["result_commitment"] = json!({
        "profile":result::OUTCOME_PROFILE,"digest":null,"reason":"query_mode_not_in_profile",
    });
    weakened["calls_digest"] = json!(semantic_calls_digest(&weakened["calls"]));
    let report = replay_bundle(&weakened);
    assert_eq!(report["ok"], false, "{report}");
    assert_eq!(report["first_divergence"]["field"], "result_commitment");
    Ok(())
}

#[test]
fn malformed_late_commitments_are_refused_before_replaying_any_call() -> TestResult {
    let _serial = crate::test_serial();
    let (_, bundle) = query_session("7824602")?;
    for invalid in [
        Value::Null,
        json!({"profile":"unknown/1","digest":"ab".repeat(32),"reason":null}),
        json!({"profile":result::PAYLOAD_PROFILE,"digest":"AB".repeat(32),"reason":null}),
        json!({"profile":result::PAYLOAD_PROFILE,"digest":null,"reason":null}),
        json!({"profile":result::PAYLOAD_PROFILE,"digest":"ab".repeat(32),"reason":null,"extra":true}),
        json!({"profile":result::OUTCOME_PROFILE,"digest":null,"reason":"invented omission"}),
    ] {
        let mut malformed = bundle.clone();
        malformed["calls"][1]["result_commitment"] = invalid;
        malformed["calls_digest"] = json!(semantic_calls_digest(&malformed["calls"]));
        let report = replay_bundle(&malformed);
        assert_eq!(report["ok"], false, "{report}");
        assert_eq!(report["replayed"], 0, "{report}");
        assert!(report.get("replay_session_id").is_none());
        assert!(
            report["error"]
                .as_str()
                .is_some_and(|error| error.contains("call 1")),
            "{report}"
        );
    }
    Ok(())
}

#[test]
fn legacy_tool_and_task_archives_keep_their_exact_encoding_and_weaker_claim() -> TestResult {
    let _serial = crate::test_serial();
    let (session, bundle) = query_session("7824603")?;
    let mut v1 = legacy(&bundle, BundleGeneration::ToolsV1)?;
    v1["server_version"] = json!("historic-test-server/0");
    v1["note"] = json!("historical capture note retained verbatim");
    let report = replay_bundle(&v1);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["verification"]["mode"], "outcome_and_anchor");
    assert_eq!(report["verification"]["result_profile"], Value::Null);
    assert_eq!(report["verification"]["semantic_result_calls"], 0);
    assert_eq!(report["verification"]["outcome_only_calls"], 2);
    assert_eq!(
        crate::server::replay_bundle_for(&field(&report, "replay_session_id")?)?,
        v1
    );

    let planned = parsed(&f::fortress_plan(
        Some(session.clone()), Some("legacy original drain".to_owned()), None,
        Some(json!([{"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}]).to_string()),
        None,
    ))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let digest = field(&planned, "plan_digest")?;
    let committed = parsed(&f::fortress_commit(Some(session.clone()), digest.clone()))?;
    assert_eq!(committed["ok"], true, "{committed}");
    crate::server::task_session::drain(&session, &digest, false)?;
    crate::server::task_session::drain(&session, &digest, true)?;
    let v3 = crate::server::replay_bundle_for(&session)?;
    assert_eq!(v3["schema"], SEMANTIC_BUNDLE_SCHEMA);
    let report = replay_bundle(&v3);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["verification"]["task_drain_events"], 2);
    assert_eq!(report["verification"]["semantic_result_calls"], 2);
    let v2 = legacy(&v3, BundleGeneration::TasksV2)?;
    let report = replay_bundle(&v2);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["verification"]["mode"], "outcome_and_anchor");
    assert_eq!(report["verification"]["semantic_result_calls"], 0);
    assert_eq!(report["verification"]["outcome_only_calls"], 4);
    assert_eq!(report["verification"]["task_drain_events"], 2);
    assert_eq!(
        crate::server::replay_bundle_for(&field(&report, "replay_session_id")?)?,
        v2
    );
    Ok(())
}

#[test]
fn uppercase_session_aliases_replay_with_canonical_owner_commitments() -> TestResult {
    let _serial = crate::test_serial();
    // Require an alphabetic digit so the bounded fixture exercises an alias.
    let mut selected = None;
    for _ in 0..32 {
        let candidate = open("7824604")?;
        if candidate.bytes().any(|byte| (b'a'..=b'f').contains(&byte)) {
            selected = Some(candidate);
            break;
        }
    }
    let session =
        selected.ok_or("no alphabetic session identity within bounded fixture allocation")?;
    let alias = session.to_ascii_uppercase();
    assert_ne!(alias, session);
    let queried = parsed(&f::fortress_query(
        Some(alias.clone()),
        Some(json!({"mode":"entities", "kind":"unit", "limit":2}).to_string()),
    ))?;
    assert_eq!(queried["ok"], true, "{queried}");
    assert_eq!(queried["session_id"], session);
    assert!(
        queried["rows"][0]["resource_uri"]
            .as_str()
            .is_some_and(|uri| uri.starts_with(&format!("df://session/{session}/entity-"))),
        "{queried}"
    );
    let observed = parsed(&f::fortress_observe(Some(alias)))?;
    assert_eq!(observed["ok"], true, "{observed}");
    let bundle = crate::server::replay_bundle_for(&session)?;
    assert_eq!(bundle["calls"].as_array().map(Vec::len), Some(3));
    let report = replay_bundle(&bundle);
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["verification"]["semantic_result_calls"], 2);
    assert_eq!(
        crate::server::replay_bundle_for(&field(&report, "replay_session_id")?)?,
        bundle
    );
    Ok(())
}
