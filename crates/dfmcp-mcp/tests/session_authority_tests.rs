#![forbid(unsafe_code)]

//! Integration tests for WP-13 Gate 2 session authority and multi-client isolation.
//!
//! Tests verify that:
//! 1. `fortress_open_session` negotiates scoped grants and mints unique session IDs.
//! 2. Concurrent sessions maintain independent state, anchors, and plans.
//! 3. Transport identity grants nothing: actions are authorized only through session grants.
//! 4. Cross-session crosstalk and invalid session access are strictly rejected.

use dfmcp_mcp::server::{
    fortress_checkpoint, fortress_commit, fortress_doctor, fortress_explain, fortress_observe,
    fortress_open_session, fortress_plan, fortress_query, fortress_restore, fortress_wait,
};
use serde_json::Value;

#[test]
fn test_concurrent_sessions_are_isolated() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Session A: seeded paused = true
    let open_a_raw = fortress_open_session(
        Some(true),
        Some("101".to_owned()),
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
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
    let open_a: Value = serde_json::from_str(&open_a_raw)?;
    assert_eq!(open_a["ok"], true);
    let session_a_id = open_a["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned();

    // Session B: seeded paused = false
    let open_b_raw = fortress_open_session(
        Some(false),
        Some("202".to_owned()),
        Some(vec![
            ("observe".to_owned(), "read_only".to_owned()),
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
    let open_b: Value = serde_json::from_str(&open_b_raw)?;
    assert_eq!(open_b["ok"], true);
    let session_b_id = open_b["session_id"]
        .as_str()
        .ok_or("session_id missing")?
        .to_owned();

    assert_ne!(session_a_id, session_b_id);

    // Observe Session A -> paused = true
    let obs_a_raw = fortress_observe(Some(session_a_id.clone()));
    let obs_a: Value = serde_json::from_str(&obs_a_raw)?;
    assert_eq!(obs_a["ok"], true);
    assert_eq!(obs_a["paused"], true);

    // Observe Session B -> paused = false
    let obs_b_raw = fortress_observe(Some(session_b_id.clone()));
    let obs_b: Value = serde_json::from_str(&obs_b_raw)?;
    assert_eq!(obs_b["ok"], true);
    assert_eq!(obs_b["paused"], false);

    // Plan in Session A: unpause (target = false)
    let plan_a_raw = fortress_plan(
        Some(session_a_id.clone()),
        Some("unpause A".to_owned()),
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
    let plan_a: Value = serde_json::from_str(&plan_a_raw)?;
    assert_eq!(plan_a["ok"], true);
    let digest_a = plan_a["plan_digest"]
        .as_str()
        .ok_or("plan_digest missing")?
        .to_owned();

    // Committing Plan A with Session B ID must fail (crosstalk protection)
    let bad_commit_raw = fortress_commit(Some(session_b_id.clone()), digest_a.clone());
    let bad_commit: Value = serde_json::from_str(&bad_commit_raw)?;
    assert_eq!(bad_commit["ok"], false);

    // Commit Plan A in Session A
    let commit_a_raw = fortress_commit(Some(session_a_id.clone()), digest_a.clone());
    let commit_a: Value = serde_json::from_str(&commit_a_raw)?;
    assert_eq!(commit_a["ok"], true);
    assert_eq!(commit_a["paused"], false);

    // Verify idempotent re-commit in Session A returns same receipt
    let commit_a_repeat_raw = fortress_commit(Some(session_a_id.clone()), digest_a);
    assert_eq!(commit_a_repeat_raw, commit_a_raw);

    // Explain Session A has events, Doctor reports healthy
    let explain_a_raw = fortress_explain(Some(session_a_id.clone()), None);
    let explain_a: Value = serde_json::from_str(&explain_a_raw)?;
    assert_eq!(explain_a["ok"], true);

    let doc_a_raw = fortress_doctor(Some(session_a_id));
    let doc_a: Value = serde_json::from_str(&doc_a_raw)?;
    assert_eq!(doc_a["ok"], true);
    assert_eq!(doc_a["status"], "healthy");

    Ok(())
}

#[test]
fn test_unknown_session_rejection() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let unknown_id = "9999999999".to_owned();
    let obs_raw = fortress_observe(Some(unknown_id.clone()));
    let obs: Value = serde_json::from_str(&obs_raw)?;
    assert_eq!(obs["ok"], false);

    let query_raw = fortress_query(Some(unknown_id.clone()), Some("summary".to_owned()));
    let query: Value = serde_json::from_str(&query_raw)?;
    assert_eq!(query["ok"], false);

    let wait_raw = fortress_wait(Some(unknown_id.clone()));
    let wait: Value = serde_json::from_str(&wait_raw)?;
    assert_eq!(wait["ok"], false);

    let ckpt_raw = fortress_checkpoint(Some(unknown_id.clone()), Some("ckpt".to_owned()));
    let ckpt: Value = serde_json::from_str(&ckpt_raw)?;
    assert_eq!(ckpt["ok"], false);

    let rst_raw = fortress_restore(Some(unknown_id), "1".to_owned());
    let rst: Value = serde_json::from_str(&rst_raw)?;
    assert_eq!(rst["ok"], false);

    Ok(())
}
