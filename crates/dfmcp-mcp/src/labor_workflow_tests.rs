//! Request decoding and the real eleven-tool laboratory facade, not native DF.
use super::parse_steps;
use crate::agent_facade as facade;
use dfmcp_core::ErrorCode;
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn request() -> Value {
    json!({
        "workflow": "labor_allocation_v1",
        "roster": [
            {"id":"1001", "skills":{"MINING":20,"BREW":10}, "stress_level":100, "assigned_labors":["BREW"]},
            {"id":"1002", "skills":{"BREW":9}, "stress_level":100, "assigned_labors":[]}
        ],
        "demands": [
            {"labor":"MINING", "skill":"MINING", "workers":1, "minimum_skill":1, "priority":0},
            {"labor":"BREW", "skill":"BREW", "workers":1, "minimum_skill":1, "priority":1}
        ]
    })
}

#[test]
fn workflow_preserves_legacy_arrays_and_produces_guarded_dependencies() -> TestResult {
    let legacy = r#"[{"action":{"kind":"set_labor","units":["1001"],"labor":"MINING","enabled":true}}]"#;
    assert_eq!(parse_steps(legacy)?, super::base::parse_steps(legacy)?);
    let value = request();
    let steps = parse_steps(&value.to_string())?;
    assert_eq!(steps.len(), 3);
    assert!(steps[0].depends_on.is_empty());
    assert_eq!(steps[1].depends_on, vec![0]);
    assert_eq!(steps[2].depends_on, vec![1]);
    assert!(steps.iter().all(|step| !step.preconditions.is_empty()));
    let mut reversed = value;
    reversed["roster"].as_array_mut().ok_or("roster")?.reverse();
    reversed["demands"].as_array_mut().ok_or("demands")?.reverse();
    assert_eq!(steps, parse_steps(&reversed.to_string())?);
    Ok(())
}

#[test]
fn workflow_rejects_ambiguity_forged_shapes_and_oversized_input() -> TestResult {
    for bad in ["0", "01", "+1", "18446744073709551616"] {
        let mut value = request(); value["roster"][0]["id"] = json!(bad);
        assert!(parse_steps(&value.to_string()).is_err());
    }
    for key in ["native_endpoint", "allow_shortages", "authority"] {
        let mut value = request(); value[key] = json!(true);
        assert!(parse_steps(&value.to_string()).is_err());
    }
    let duplicated = request().to_string().replace("\"MINING\":20", "\"MINING\":20,\"MINING\":0");
    assert!(duplicated.matches("\"MINING\":").count() >= 2);
    assert!(parse_steps(&duplicated).is_err());
    let mut value = request(); value["roster"][0]["assigned_labors"] = json!(["BREW","BREW"]);
    assert!(parse_steps(&value.to_string()).is_err());
    value = request(); value["roster"][1]["id"] = json!("1001");
    assert!(parse_steps(&value.to_string()).is_err());
    value = request(); value["demands"][0]["workers"] = json!(2);
    assert!(matches!(parse_steps(&value.to_string()), Err(e) if e.code == ErrorCode::PreconditionsFailed));
    assert!(matches!(parse_steps(&format!("{{{}", " ".repeat(super::MAX_ACTIONS_JSON_BYTES))), Err(e) if e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

fn parsed(raw: &str) -> TestResult<Value> { Ok(serde_json::from_str(raw)?) }
fn open(selector: &str, labor: bool) -> TestResult<String> {
    let mut caps = vec![("observe".to_owned(),"read_only".to_owned()), ("query".to_owned(),"read_only".to_owned()), ("plan".to_owned(),"reversible".to_owned()), ("control_clock".to_owned(),"reversible".to_owned())];
    if labor { caps.push(("configure_labor".to_owned(), "reversible".to_owned())); }
    let result = parsed(&facade::fortress_open_session(Some(false), Some(selector.to_owned()), Some(caps), Some(30_000), Some(2_000), None, None, Some(8_192), Some(64), Some("starter_fortress".to_owned()), Some(false), Some(false)))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["session_id"].as_str().ok_or("session ID")?.to_owned())
}
fn plan(session: &str, actions: Value) -> TestResult<Value> {
    parsed(&facade::fortress_plan(Some(session.to_owned()), None, None, Some(actions.to_string()), None))
}
fn commit(session: &str, planned: &Value) -> TestResult<Value> {
    parsed(&facade::fortress_commit(Some(session.to_owned()), planned["plan_digest"].as_str().ok_or("digest")?.to_owned()))
}
fn finish(session: &str) -> TestResult<Value> {
    let result = parsed(&facade::fortress_wait(Some(session.to_owned()), Some(10)))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result)
}
fn initialize_matrix(session: &str) -> TestResult {
    let planned = plan(session, json!([
        {"action":{"kind":"set_labor","units":["1001","1002"],"labor":"MINING","enabled":false}},
        {"action":{"kind":"set_labor","units":["1002"],"labor":"BREW","enabled":false}},
        {"action":{"kind":"set_labor","units":["1001"],"labor":"BREW","enabled":true}}
    ]))?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(commit(session, &planned)?["ok"], true);
    assert_eq!(finish(session)?["open_actions_remaining"], 0);
    Ok(())
}

#[test]
fn agent_can_plan_commit_and_prove_a_staffing_workflow() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7955101", true)?;
    initialize_matrix(&session)?;
    let planned = plan(&session, request())?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(planned["steps"].as_array().ok_or("steps")?.len(), 3);
    assert_eq!(commit(&session, &planned)?["ok"], true);
    let finished = finish(&session)?;
    assert_eq!(finished["open_actions_remaining"], 0, "{finished}");
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(commit(&session, &planned)?["ok"], true);
    assert_eq!(finish(&session)?["open_actions_remaining"], 0);
    Ok(())
}

#[test]
fn old_or_missing_initial_configuration_cannot_be_planned() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7955102", true)?;
    // The starter does not assert false for absent labor fields.
    let missing = plan(&session, request())?;
    assert_eq!(missing["ok"], false, "{missing}");
    initialize_matrix(&session)?;
    let mut stale = request(); stale["roster"][0]["assigned_labors"] = json!([]);
    let refused = plan(&session, stale)?;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(plan(&session, request())?["ok"], true);
    Ok(())
}

#[test]
fn workflow_does_not_bypass_configure_labor_authority() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("7955103", false)?;
    // This one existing starter field is explicitly known: the brewer brews.
    let planned = plan(&session, json!({
        "workflow":"labor_allocation_v1",
        "roster":[{"id":"1003","skills":{},"stress_level":100,"assigned_labors":["BREW"]}],
        "demands":[{"labor":"BREW","skill":"BREW","workers":0,"minimum_skill":0,"priority":0}]
    }))?;
    assert_eq!(planned["ok"], true, "{planned}");
    let denied = commit(&session, &planned)?;
    assert_eq!(denied["ok"], false, "{denied}");
    Ok(())
}

#[test]
fn checked_in_workflow_example_uses_the_same_decoder() -> TestResult {
    let raw = include_str!("../../../schemas/examples/labor_execution_v1.json");
    assert_eq!(parse_steps(raw)?, parse_steps(&request().to_string())?);
    Ok(())
}
