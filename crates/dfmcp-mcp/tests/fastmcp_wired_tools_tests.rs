#![forbid(unsafe_code)]

//! Integration coverage for the MCP functions that are honestly executable in
//! the process-local laboratory.

use dfmcp_mcp::server::{
    fortress_commit, fortress_doctor, fortress_explain, fortress_open_session, fortress_plan,
    fortress_query, fortress_wait,
};
use serde_json::Value;

#[test]
fn laboratory_tools_execute_supported_pause_flow()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    let open_raw = fortress_open_session(
        Some(true),
        Some("1".to_owned()),
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
            ("query".to_owned(), "read_only".to_owned()),
            ("plan".to_owned(), "reversible".to_owned()),
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
    let open: Value = serde_json::from_str(&open_raw)?;
    assert_eq!(open["ok"], true);
    let session_id = open["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned();

    let query: Value = serde_json::from_str(&fortress_query(
        Some(session_id.clone()),
        Some("summary".to_owned()),
    ))?;
    assert_eq!(query["ok"], true);

    let plan: Value = serde_json::from_str(&fortress_plan(
        Some(session_id.clone()),
        Some("unpause simulation".to_owned()),
        Some(false),
    ))?;
    assert_eq!(plan["ok"], true);
    let digest = plan["plan_digest"]
        .as_str()
        .ok_or("plan_digest missing")?
        .to_owned();

    let wrong_digest = if digest == "0".repeat(64) {
        "1".repeat(64)
    } else {
        "0".repeat(64)
    };
    let rejected: Value =
        serde_json::from_str(&fortress_commit(Some(session_id.clone()), wrong_digest))?;
    assert_eq!(rejected["ok"], false);
    assert_eq!(rejected["error"]["code"], "conflict");

    // A mismatched digest must not consume the pending sealed plan.
    let commit_raw = fortress_commit(Some(session_id.clone()), digest.clone());
    let commit: Value = serde_json::from_str(&commit_raw)?;
    assert_eq!(commit["ok"], true);

    let wait: Value = serde_json::from_str(&fortress_wait(Some(session_id.clone())))?;
    assert_eq!(wait["ok"], true);
    assert!(
        wait["task_id"]
            .as_str()
            .is_some_and(|value| value.starts_with("task_act_"))
    );

    let second_plan: Value = serde_json::from_str(&fortress_plan(
        Some(session_id.clone()),
        Some("pause simulation again".to_owned()),
        Some(true),
    ))?;
    let second_digest = second_plan["plan_digest"]
        .as_str()
        .ok_or("second plan_digest missing")?
        .to_owned();
    let second_commit: Value =
        serde_json::from_str(&fortress_commit(Some(session_id.clone()), second_digest))?;
    assert_eq!(second_commit["ok"], true);
    assert_eq!(second_commit["paused"], true);

    let pending_plan: Value = serde_json::from_str(&fortress_plan(
        Some(session_id.clone()),
        Some("leave a newer plan pending".to_owned()),
        Some(false),
    ))?;
    let pending_digest = pending_plan["plan_digest"]
        .as_str()
        .ok_or("pending plan_digest missing")?
        .to_owned();

    // Replaying an older receipt remains stable after a newer plan commits and
    // must not consume an unrelated plan that was prepared afterward.
    assert_eq!(
        fortress_commit(Some(session_id.clone()), digest),
        commit_raw
    );
    let pending_commit: Value =
        serde_json::from_str(&fortress_commit(Some(session_id.clone()), pending_digest))?;
    assert_eq!(pending_commit["ok"], true);
    assert_eq!(pending_commit["paused"], false);

    let explain: Value = serde_json::from_str(&fortress_explain(
        Some(session_id.clone()),
        Some("10".to_owned()),
    ))?;
    assert_eq!(explain["ok"], true);

    let doctor: Value = serde_json::from_str(&fortress_doctor(Some(session_id)))?;
    assert_eq!(doctor["ok"], true);
    assert_eq!(doctor["status"], "healthy");
    Ok(())
}
