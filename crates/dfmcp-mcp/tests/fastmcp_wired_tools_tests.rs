#![forbid(unsafe_code)]

//! Integration tests for wired FastMCP tools (Search Query, Blueprint/Logistics Planning, Topology Explain, Doctor Telemetry).

use dfmcp_mcp::server::{
    fortress_commit, fortress_doctor, fortress_explain, fortress_open_session, fortress_plan,
    fortress_query,
};
use serde_json::Value;

#[test]
fn test_wired_fastmcp_tools_suite() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // 1. Open session
    let open_raw = fortress_open_session(
        Some(true),
        Some("1".to_owned()),
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "guarded".to_owned()),
            ("checkpoint".to_owned(), "guarded".to_owned()),
            ("designate".to_owned(), "guarded".to_owned()),
            ("configure_production".to_owned(), "reversible".to_owned()),
            ("control_clock".to_owned(), "reversible".to_owned()),
            ("doctor".to_owned(), "read_only".to_owned()),
        ]),
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let open_json: Value = serde_json::from_str(&open_raw)?;
    assert_eq!(open_json["ok"], true);
    let session_id = open_json["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned();

    // 2. Query with FrankenSearch mode
    let search_raw = fortress_query(
        Some(session_id.clone()),
        Some("search".to_owned()),
        Some("Entity".to_owned()),
    );
    let search_json: Value = serde_json::from_str(&search_raw)?;
    assert_eq!(search_json["ok"], true);
    assert_eq!(search_json["mode"], "search");

    // 3. Plan Dining Hall Blueprint
    let bp_plan_raw = fortress_plan(
        Some(session_id.clone()),
        Some("excavate great dining hall".to_owned()),
        None,
        Some("dining_hall".to_owned()),
        Some(0),
        Some(0),
        Some(100),
        Some(6),
        Some(6),
        None,
        None,
    );
    let bp_plan_json: Value = serde_json::from_str(&bp_plan_raw)?;
    assert_eq!(bp_plan_json["ok"], true);
    assert!(bp_plan_json["plan_digest"].is_string());

    // 4. Plan Logistics Quota Work Orders
    let log_plan_raw = fortress_plan(
        Some(session_id.clone()),
        Some("brew drink quota".to_owned()),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("DRINK".to_owned()),
        Some(30),
    );
    let log_plan_json: Value = serde_json::from_str(&log_plan_raw)?;
    assert_eq!(log_plan_json["ok"], true);
    assert!(log_plan_json["plan_digest"].is_string());

    // 5. Plan and Commit Pause Simulation Plan (supported by MemoryAdapter)
    let pause_plan_raw = fortress_plan(
        Some(session_id.clone()),
        Some("unpause simulation".to_owned()),
        Some(false),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    );
    let pause_plan_json: Value = serde_json::from_str(&pause_plan_raw)?;
    assert_eq!(pause_plan_json["ok"], true);
    let pause_digest = pause_plan_json["plan_digest"]
        .as_str()
        .ok_or("plan_digest missing")?
        .to_owned();

    let commit_pause_raw = fortress_commit(Some(session_id.clone()), pause_digest);
    let commit_pause_json: Value = serde_json::from_str(&commit_pause_raw)?;
    assert_eq!(commit_pause_json["ok"], true);

    // 6. Poll Wait / Modern Tasks Projection
    let wait_raw = dfmcp_mcp::server::fortress_wait(Some(session_id.clone()));
    let wait_json: Value = serde_json::from_str(&wait_raw)?;
    assert_eq!(wait_json["ok"], true);
    assert!(
        wait_json["task_id"]
            .as_str()
            .unwrap_or("")
            .starts_with("task_act_")
    );

    // 6. Explain Entity Causal Topology
    let explain_raw = fortress_explain(Some(session_id.clone()), Some("10".to_owned()));
    let explain_json: Value = serde_json::from_str(&explain_raw)?;
    assert_eq!(explain_json["ok"], true);
    assert_eq!(explain_json["target_entity"], "10");

    // 7. Doctor Telemetry Diagnostics
    let doc_raw = fortress_doctor(Some(session_id));
    let doc_json: Value = serde_json::from_str(&doc_raw)?;
    assert_eq!(doc_json["ok"], true);
    assert_eq!(doc_json["status"], "healthy");
    assert!(doc_json["active_sessions_count"].as_u64().unwrap_or(0) >= 1);

    Ok(())
}
