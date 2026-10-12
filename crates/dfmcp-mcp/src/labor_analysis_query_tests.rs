//! Exercise the actual query decoder, compiler handoff and laboratory facade.
use super::{parse_steps, query};
use crate::agent_facade as facade;
use dfmcp_core::{Digest32, ErrorCode, FortressId, GameTick};
use dfmcp_world::{Fact, FactSource, Value as WorldValue, WorldSnapshot};
use serde_json::{Value, json};

type TestResult<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn request() -> Value {
    json!({"mode":"labor_allocation", "roster":[
        {"id":"1001","skills":{"MINING":20,"BREW":10},"stress_level":100},
        {"id":"1002","skills":{"BREW":9},"stress_level":100}],
        "demands":[
            {"labor":"MINING","skill":"MINING","workers":1,"minimum_skill":1,"priority":0},
            {"labor":"BREW","skill":"BREW","workers":1,"minimum_skill":1,"priority":1}]})
}
fn snapshot() -> TestResult<WorldSnapshot> {
    let mut snapshot = super::base::scenario_snapshot("starter_fortress", FortressId::new(9), true)?;
    for (id, brew) in [(1001, true), (1002, false)] {
        let entity = snapshot.graph.entities.get_mut(&dfmcp_core::EntityId::new(id)).ok_or("citizen")?;
        for (name, enabled) in [("labor.BREW", brew), ("labor.MINING", false)] {
            entity.fields.insert(name.to_owned(), Fact::known(WorldValue::Bool(enabled), GameTick(1),
                FactSource::Derived("dfmcp.lab-scenario/1".to_owned()), Digest32::ZERO));
        }
    }
    snapshot.refresh_hash();
    Ok(snapshot)
}

#[test]
fn query_handoff_uses_observed_initial_bits_and_the_original_workflow_decoder() -> TestResult {
    let snapshot = snapshot()?;
    let before = snapshot.clone();
    let result = query(&snapshot, &request().to_string())?;
    assert_eq!(result["readiness"], "proposal_available");
    assert_eq!(result["model_feasible"], true);
    assert_eq!(result["execution_ready"], false);
    let raw = result["proposed_workflow_json"].as_str().ok_or("workflow")?;
    let workflow: Value = serde_json::from_str(raw)?;
    assert_eq!(workflow["roster"][0]["assigned_labors"], json!(["BREW"]));
    assert_eq!(workflow["roster"][1]["assigned_labors"], json!([]));
    assert_eq!(result["proposed_workflow_digest"], Digest32::of_bytes(raw.as_bytes()).to_string());
    let steps = parse_steps(raw)?;
    assert_eq!(steps.len(), 3);
    assert_eq!(steps[1].depends_on, vec![0]);
    assert_eq!(steps[2].depends_on, vec![1]);
    assert_eq!(snapshot, before);
    Ok(())
}

#[test]
fn missing_evidence_is_actionable_but_never_a_partial_assignment() -> TestResult {
    let mut snapshot = snapshot()?;
    snapshot.graph.entities.get_mut(&dfmcp_core::EntityId::new(1002)).ok_or("citizen")?.fields.remove("labor.MINING");
    snapshot.refresh_hash();
    let result = query(&snapshot, &request().to_string())?;
    assert_eq!(result["readiness"], "missing_evidence");
    assert_eq!(result["problems_total"], 1);
    assert_eq!(result["problems"][0]["field"], "labor.MINING");
    assert!(result["model_feasible"].is_null());
    assert!(result["proposed_workflow_json"].is_null());
    assert_eq!(result["configuration_proven"], false);
    Ok(())
}

#[test]
fn infeasible_and_already_satisfied_models_are_distinct_without_fake_effects() -> TestResult {
    let snapshot = snapshot()?;
    let mut shortage = request(); shortage["demands"][0]["workers"] = json!(2);
    let result = query(&snapshot, &shortage.to_string())?;
    assert_eq!(result["readiness"], "staffing_shortage");
    assert_eq!(result["model_feasible"], false);
    assert_eq!(result["missing_workers"], 1);
    assert!(result["proposed_workflow_json"].is_null());
    let mut satisfied = request();
    satisfied["demands"] = json!([{"labor":"BREW","skill":"BREW","workers":1,"minimum_skill":1,"priority":0}]);
    let result = query(&snapshot, &satisfied.to_string())?;
    assert_eq!(result["readiness"], "already_configured");
    assert_eq!(result["configuration_proven"], true);
    assert!(result["proposed_workflow_json"].is_null());
    Ok(())
}

#[test]
fn canonical_handoffs_are_invariant_to_input_order() -> TestResult {
    let snapshot = snapshot()?;
    let expected = query(&snapshot, &request().to_string())?;
    let mut reversed = request();
    reversed["roster"].as_array_mut().ok_or("roster")?.reverse();
    reversed["demands"].as_array_mut().ok_or("demands")?.reverse();
    assert_eq!(expected, query(&snapshot, &reversed.to_string())?);
    Ok(())
}

#[test]
fn a_single_work_allowance_spans_reads_matching_and_executable_lowering() -> TestResult {
    let snapshot = snapshot()?;
    let result = query(&snapshot, &request().to_string())?;
    let used = result["work_used"].as_u64().ok_or("work")?;
    assert!(used > 1);
    let mut exact = request(); exact["max_work"] = json!(used);
    assert_eq!(query(&snapshot, &exact.to_string())?["work_used"], used);
    exact["max_work"] = json!(used - 1);
    assert!(matches!(query(&snapshot, &exact.to_string()), Err(e) if e.code == ErrorCode::BudgetExceeded));
    Ok(())
}

#[test]
fn closed_query_rejects_claimed_initial_state_ambiguity_and_invalid_limits() -> TestResult {
    let snapshot = snapshot()?;
    for key in ["assigned_labors", "native_endpoint", "source_policy", "authority"] {
        let mut bad = request(); bad["roster"][0][key] = json!([]);
        assert!(query(&snapshot, &bad.to_string()).is_err());
    }
    for id in ["0", "01", "+1", "18446744073709551616"] {
        let mut bad = request(); bad["roster"][0]["id"] = json!(id);
        assert!(query(&snapshot, &bad.to_string()).is_err());
    }
    let raw = request().to_string().replace("\"MINING\":20", "\"MINING\":20,\"MINING\":0");
    assert!(raw.matches("\"MINING\":").count() >= 2);
    assert!(query(&snapshot, &raw).is_err());
    for maximum in [0, 1_000_001] {
        let mut bad = request(); bad["max_work"] = json!(maximum);
        assert!(query(&snapshot, &bad.to_string()).is_err());
    }
    assert!(query(&snapshot, &format!("{{{}", " ".repeat(super::MAX_QUERY_JSON_BYTES))).is_err());
    Ok(())
}

#[test]
fn output_fitting_never_turns_the_handoff_into_a_different_executable_subset() -> TestResult {
    let snapshot = snapshot()?;
    let result = query(&snapshot, &request().to_string())?;
    for budget in [2, 10, 100, 500, 1_500, 8_192] {
        let fitted: Value = serde_json::from_str(&crate::output_budget::fit(&result.to_string(), budget))?;
        if let Some(Value::String(raw)) = fitted.get("proposed_workflow_json") {
            assert_eq!(raw, result["proposed_workflow_json"].as_str().ok_or("workflow")?);
            assert_eq!(parse_steps(raw)?.len(), 3);
        }
    }
    Ok(())
}

fn parsed(raw: &str) -> TestResult<Value> { Ok(serde_json::from_str(raw)?) }
fn open(selector: &str, effects: bool, query_grant: bool) -> TestResult<String> {
    let mut caps = vec![("observe".to_owned(), "read_only".to_owned()), ("plan".to_owned(), "reversible".to_owned())];
    if effects { caps.push(("configure_labor".to_owned(), "reversible".to_owned())); }
    if query_grant { caps.push(("query".to_owned(), "read_only".to_owned())); }
    let result = parsed(&facade::fortress_open_session(Some(true), Some(selector.to_owned()), Some(caps),
        Some(30_000), Some(2_000), None, None, Some(8_192), Some(64), Some("starter_fortress".to_owned()), Some(false), Some(false)))?;
    assert_eq!(result["ok"], true, "{result}");
    Ok(result["session_id"].as_str().ok_or("session")?.to_owned())
}
fn plan(session: &str, raw: String) -> TestResult<Value> {
    parsed(&facade::fortress_plan(Some(session.to_owned()), None, None, Some(raw), None))
}
fn commit(session: &str, planned: &Value) -> TestResult<Value> {
    parsed(&facade::fortress_commit(Some(session.to_owned()), planned["plan_digest"].as_str().ok_or("digest")?.to_owned()))
}
fn wait(session: &str) -> TestResult {
    for _ in 0..4 {
        let result = parsed(&facade::fortress_wait(Some(session.to_owned()), Some(0)))?;
        assert_eq!(result["ok"], true, "{result}");
        if result["open_actions_remaining"] == 0 { return Ok(()); }
    }
    Err("staffing actions did not verify within four foreground polls".into())
}

#[test]
fn real_facade_query_plan_commit_wait_rechecks_stale_initial_configuration() -> TestResult {
    let _serial = crate::test_serial();
    let session = open("8055101", true, true)?;
    let setup = include_str!("../../../schemas/examples/labor_execution_setup_lab.json");
    let prepared = plan(&session, setup.to_owned())?;
    assert_eq!(prepared["ok"], true, "{prepared}");
    assert_eq!(commit(&session, &prepared)?["ok"], true);
    wait(&session)?;
    let queried = parsed(&facade::fortress_query(Some(session.clone()), Some(request().to_string())))?;
    assert_eq!(queried["readiness"], "proposal_available", "{queried}");
    let workflow = queried["proposed_workflow_json"].as_str().ok_or("workflow")?.to_owned();
    let prepared = plan(&session, workflow.clone())?;
    assert_eq!(prepared["ok"], true, "{prepared}");
    assert_eq!(commit(&session, &prepared)?["ok"], true);
    wait(&session)?;
    let current = parsed(&facade::fortress_query(Some(session.clone()), Some(request().to_string())))?;
    assert_eq!(current["readiness"], "already_configured", "{current}");
    assert_eq!(current["configuration_proven"], true);
    assert_eq!(plan(&session, workflow)?["ok"], false);
    Ok(())
}

#[test]
fn observation_handoff_grants_neither_query_nor_mutation_authority() -> TestResult {
    let _serial = crate::test_serial();
    let query = json!({"mode":"labor_allocation","roster":[{"id":"1003","skills":{},"stress_level":100}],
        "demands":[{"labor":"BREW","skill":"BREW","workers":0,"minimum_skill":0,"priority":0}]});
    let denied = open("8055102", false, false)?;
    assert_eq!(parsed(&facade::fortress_query(Some(denied), Some(query.to_string())))?["ok"], false);
    let session = open("8055103", false, true)?;
    let out = parsed(&facade::fortress_query(Some(session.clone()), Some(query.to_string())))?;
    let planned = plan(&session, out["proposed_workflow_json"].as_str().ok_or("workflow")?.to_owned())?;
    assert_eq!(planned["ok"], true, "{planned}");
    assert_eq!(commit(&session, &planned)?["ok"], false);
    Ok(())
}

#[test]
fn checked_in_analysis_example_uses_the_actual_query_decoder() -> TestResult {
    let snapshot = snapshot()?;
    let example = include_str!("../../../schemas/examples/labor_analysis_query_v1.json");
    assert_eq!(query(&snapshot, example)?, query(&snapshot, &request().to_string())?);
    Ok(())
}
