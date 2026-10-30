//! Real eleven-tool facade tests. No DFHack or physical evacuation is simulated.
use super::parse_steps;
use crate::agent_facade as facade;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn request() -> Value {
    json!({"workflow":"civilian_protection_v1", "burrow":"3001", "civilians":["1001","1002"]})
}
fn query_request() -> Value {
    json!({"mode":"civilian_protection", "burrow":"3001", "civilians":["1001","1002"]})
}
fn parsed(raw: &str) -> TestResult<Value> {
    Ok(serde_json::from_str(raw)?)
}
fn open(selector: &str, paused: bool, missing: Option<&str>) -> TestResult<String> {
    let caps = [
        ("observe", "read_only"),
        ("query", "read_only"),
        ("plan", "guarded"),
        ("configure_logistics", "guarded"),
        ("checkpoint", "guarded"),
    ]
    .into_iter()
    .filter(|(cap, _)| Some(*cap) != missing)
    .map(|(cap, risk)| (cap.to_owned(), risk.to_owned()))
    .collect();
    let result = parsed(&facade::fortress_open_session(
        Some(paused),
        Some(selector.to_owned()),
        Some(caps),
        Some(30_000),
        Some(100),
        None,
        None,
        Some(8_192),
        Some(64),
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(false),
    ))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["session_id"]
        .as_str()
        .ok_or("session ID missing")?
        .to_owned())
}
fn plan(session: &str, value: Value) -> TestResult<Value> {
    parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        Some("protect named civilians".to_owned()),
        None,
        Some(value.to_string()),
        None,
    ))
}
fn commit(session: &str, planned: &Value) -> TestResult<Value> {
    parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        planned["plan_digest"]
            .as_str()
            .ok_or("digest missing")?
            .to_owned(),
    ))
}
fn status(session: &str) -> TestResult<Value> {
    parsed(&facade::fortress_query(
        Some(session.to_owned()),
        Some(query_request().to_string()),
    ))
}
fn finish(session: &str) -> TestResult<Value> {
    for _ in 0..3 {
        let value = parsed(&facade::fortress_wait(Some(session.to_owned()), Some(0)))?;
        assert_eq!(value["ok"], true, "{value}");
        if value["open_actions_remaining"] == 0 {
            return Ok(value);
        }
    }
    Err("instantaneous protection did not finish after three bounded foreground polls".into())
}
fn execute(session: &str, value: Value) -> TestResult<Value> {
    let planned = plan(session, value)?;
    assert_eq!(planned["ok"], true, "{planned}");
    let committed = commit(session, &planned)?;
    assert_eq!(committed["ok"], true, "{committed}");
    finish(session)?;
    Ok(planned)
}

#[test]
fn protection_decoding_is_canonical_and_preserves_legacy_action_arrays() -> TestResult {
    let steps = parse_steps(&request().to_string())?;
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[1].depends_on, vec![0]);
    assert!(
        steps
            .iter()
            .all(|s| s.compensation.as_ref() == Some(&s.action))
    );
    let mut reversed = request();
    reversed["civilians"] = json!(["1002", "1001"]);
    assert_eq!(steps, parse_steps(&reversed.to_string())?);
    let legacy = r#"[{"action":{"kind":"pause","paused":true}}]"#;
    assert_eq!(parse_steps(legacy)?, super::base::parse_steps(legacy)?);
    let example = include_str!("../../../schemas/examples/civilian_protection_v1.json");
    assert_eq!(parse_steps(example)?.len(), 2);
    Ok(())
}

#[test]
fn malformed_and_authority_smuggling_requests_refuse() -> TestResult {
    for id in ["0", "01", "+1", "-1", "18446744073709551616", "١"] {
        let mut value = request();
        value["civilians"][0] = json!(id);
        assert!(parse_steps(&value.to_string()).is_err());
    }
    for key in [
        "release",
        "clear_samples",
        "threat_clear",
        "native_endpoint",
        "authority",
    ] {
        let mut value = request();
        value[key] = json!(true);
        assert!(parse_steps(&value.to_string()).is_err());
    }
    for ids in [
        json!([]),
        json!(["1001", "1001"]),
        json!(["3001"]),
        json!(vec!["1001"; 65]),
    ] {
        let mut value = request();
        value["civilians"] = ids;
        assert!(parse_steps(&value.to_string()).is_err());
    }
    assert!(parse_steps(r#"{"workflow":"civilian_protection_v1","workflow":"labor_allocation_v1","burrow":"3001","civilians":["1001"]}"#).is_err());
    assert!(parse_steps(r#"{"workflow":"civilian_protection_v1","burrow":"3001","burrow":"3002","civilians":["1001"]}"#).is_err());
    assert!(parse_steps(&format!("{{{}", " ".repeat(super::MAX_ACTIONS_JSON_BYTES))).is_err());
    Ok(())
}

#[test]
fn agent_can_plan_commit_prove_and_query_protection_without_advancing_time() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7966101", true, None)?;
    let before = status(&session)?;
    assert_eq!(before["ok"], true, "{before}");
    assert_eq!(before["configuration_truth"], "unknown");
    assert_eq!(before["memberships"]["unknown"], 2);
    let planned = plan(&session, request())?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(planned["steps"].as_array().ok_or("steps missing")?.len(), 2);
    assert_eq!(status(&session)?["anchor"], before["anchor"]);
    assert_eq!(commit(&session, &planned)?["ok"], true);
    finish(&session)?;
    let current = status(&session)?;
    assert_eq!(current["configuration_truth"], "true", "{current}");
    assert_eq!(current["memberships"]["assigned"], 2);
    assert_eq!(current["memberships"]["unknown"], 0);
    assert_eq!(current["game_tick"], before["game_tick"]);
    for field in [
        "physical_arrival_proven",
        "threat_clearance_proven",
        "native_evidence",
    ] {
        assert_eq!(current[field], false);
    }
    assert_eq!(commit(&session, &planned)?["ok"], true);
    assert_eq!(status(&session)?["anchor"], current["anchor"]);
    Ok(())
}

#[test]
fn current_query_cannot_reuse_a_proof_after_the_restriction_changes() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7966102", true, None)?;
    execute(&session, request())?;
    let achieved = status(&session)?;
    assert_eq!(achieved["configuration_proven"], true);
    execute(
        &session,
        json!([{"action":{"kind":"set_standing_order", "key":"CIVILIAN_BURROW_RESTRICTION", "value":"INACTIVE"}}]),
    )?;
    let current = status(&session)?;
    assert_eq!(current["configuration_truth"], "false", "{current}");
    assert_eq!(current["memberships"]["assigned"], 2);
    assert_eq!(current["configuration_proven"], false);
    let mut at = query_request();
    at["at"] = achieved["anchor"]["state_hash"].clone();
    let historical = parsed(&facade::fortress_query(Some(session), Some(at.to_string())))?;
    assert_eq!(historical["configuration_proven"], true, "{historical}");
    assert_eq!(historical["historical"], true);
    assert_eq!(historical["current_anchor"], current["anchor"]);
    Ok(())
}

#[test]
fn cancellation_after_instant_protection_preserves_applied_memberships() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7966103", true, None)?;
    execute(
        &session,
        json!([{"action":{"kind":"set_burrow_membership", "burrow":"3001", "units":["1001"], "assigned":true}}]),
    )?;
    let planned = plan(&session, request())?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(commit(&session, &planned)?["ok"], true);
    let applied = status(&session)?;
    assert_eq!(applied["memberships"]["assigned"], 2);
    assert_eq!(applied["configuration_proven"], true);
    let cancelled = parsed(&facade::fortress_cancel(
        Some(session.clone()),
        Some("stop_future_steps".to_owned()),
        Some("plan".to_owned()),
    ))?;
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let current = status(&session)?;
    // Both instantaneous configuration effects were already observed at
    // commit. Stopping future steps does not undo a completed assignment.
    assert_eq!(current["memberships"]["assigned"], 2, "{current}");
    assert_eq!(current["memberships"]["unknown"], 0);
    assert_eq!(current["configuration_proven"], true);
    assert_eq!(current["anchor"], applied["anchor"]);
    Ok(())
}

#[test]
fn missing_effect_checkpoint_or_query_authority_is_not_supplied_by_the_workflow() -> TestResult {
    let _serial = crate::test_serial();
    for (selector, missing) in [
        ("7966104", "configure_logistics"),
        ("7966105", "checkpoint"),
    ] {
        let session = open(selector, true, Some(missing))?;
        let planned = plan(&session, request())?;
        assert_eq!(planned["ok"], true, "{planned}");
        let before = status(&session)?;
        let denied = commit(&session, &planned)?;
        assert_eq!(denied["ok"], false, "{denied}");
        assert_eq!(status(&session)?["anchor"], before["anchor"]);
    }
    let session = open("7966106", true, Some("query"))?;
    assert_eq!(status(&session)?["ok"], false);
    Ok(())
}

#[test]
fn running_world_missing_citizens_wrong_burrow_and_forged_query_fields_refuse() -> TestResult {
    let _serial = crate::test_serial();
    let running = open("7966107", false, None)?;
    assert_eq!(plan(&running, request())?["ok"], false);
    let session = open("7966108", true, None)?;
    let mut wrong = request();
    wrong["burrow"] = json!("1003");
    assert_eq!(plan(&session, wrong)?["ok"], false);
    let mut missing = request();
    missing["civilians"] = json!(["999"]);
    assert_eq!(plan(&session, missing)?["ok"], false);
    let mut query = query_request();
    query["evidence"] = json!("complete");
    let refused = parsed(&facade::fortress_query(
        Some(session),
        Some(query.to_string()),
    ))?;
    assert_eq!(refused["ok"], false);
    Ok(())
}

#[path = "../../dfmcp-intent/tests/support/civilian_protection.rs"]
#[allow(dead_code)]
mod projection_fixture;

#[test]
fn query_projection_preserves_source_unknowns_and_other_query_modes() -> TestResult {
    use dfmcp_world::{FactSource, Value as WorldValue};
    use projection_fixture as fixture;
    let mut snapshot = fixture::complete(fixture::world())?;
    let query =
        json!({"mode":"civilian_protection", "burrow":"20", "civilians":["11","12"]}).to_string();
    assert_eq!(
        super::query(&snapshot, &query)?["configuration_proven"],
        true
    );
    let mut forged = fixture::fact(WorldValue::Bool(true));
    forged.source = FactSource::AgentAssertion("not an observation".to_owned());
    fixture::set(
        &mut snapshot,
        fixture::SECOND,
        &fixture::field_name(),
        forged,
    )?;
    let current = super::query(&snapshot, &query)?;
    assert_eq!(current["configuration_truth"], "unknown");
    assert_eq!(current["memberships"]["unknown"], 1);
    assert_eq!(
        super::query(&snapshot, "entities")?,
        super::base::query(&snapshot, "entities")?
    );
    Ok(())
}
