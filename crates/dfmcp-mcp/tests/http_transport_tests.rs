#![forbid(unsafe_code)]

//! Integration tests for WP-MCP-02 Streamable HTTP Transport & Resumption.

use dfmcp_core::{Result, SessionId};
use dfmcp_mcp::http_transport::{HttpSessionResumeToken, HttpTransportSessionManager};

#[test]
fn test_http_session_token_validation() -> Result<()> {
    let session = SessionId::new(42);
    let token = HttpSessionResumeToken::new(session, 10);
    assert!(token.verify_signature());

    let mut tampered = token.clone();
    tampered.resume_offset = 20;
    assert!(!tampered.verify_signature());

    Ok(())
}

#[test]
fn test_http_transport_session_isolation() -> Result<()> {
    let mut manager = HttpTransportSessionManager::new();
    let s1 = SessionId::new(1);
    let s2 = SessionId::new(2);

    let t1 = manager.open_session(s1);
    let t2 = manager.open_session(s2);

    manager.buffer_message(s1, "msg for s1".to_owned())?;
    manager.buffer_message(s2, "msg for s2".to_owned())?;

    let s1_msgs = manager.resume_session(&t1)?;
    assert_eq!(s1_msgs, vec!["msg for s1"]);

    let s2_msgs = manager.resume_session(&t2)?;
    assert_eq!(s2_msgs, vec!["msg for s2"]);

    Ok(())
}
