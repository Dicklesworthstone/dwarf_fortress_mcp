#![forbid(unsafe_code)]

//! Golden tests for the modern MCP 2026-07-28 stdio session shape.
//!
//! These tests drive the real `dwarf-fortress-mcp serve` subprocess via
//! newline-delimited JSON-RPC and assert the exact response shape for the
//! frozen 11-tool `fortress.*` waist. They feed WP-21/TEST-023 conformance
//! work and document the modern client quickstart found in
//! `docs/FASTMCP_INTEGRATION.md`.
//!
//! Each test fixes one fixture pair in `tests/fixtures/` so a future pin bump
//! that changes the wire shape surfaces immediately here rather than as a
//! silent regression in a downstream agent's plan/commit flow.

use std::error::Error;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

struct StdioClient {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl StdioClient {
    fn spawn() -> Result<Self, Box<dyn Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dwarf-fortress-mcp"))
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or("failed to capture stdout from child process")?;
        let reader = BufReader::new(stdout);
        Ok(Self { child, reader })
    }

    fn send(&mut self, request: &Value) -> Result<Value, Box<dyn Error>> {
        let stdin = self.child.stdin.as_mut().ok_or("child stdin unavailable")?;
        let line = request.to_string();
        stdin.write_all(line.as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;

        let mut response_line = String::new();
        while self.reader.read_line(&mut response_line)? > 0 {
            let trimmed = response_line.trim();
            if trimmed.starts_with('{') {
                let parsed: Value = serde_json::from_str(trimmed)?;
                return Ok(parsed);
            }
            response_line.clear();
        }
        Err("child process stdout closed without emitting JSON-RPC line".into())
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn modern_meta() -> Value {
    // MCP 2026-07-28 server/discover requires clientInfo in _meta;
    // minimal {protocolVersion, clientCapabilities} silently fails the
    // dispatch (no JSON-RPC response is emitted). The unfiled finding is
    // recorded in docs/DOGFOODING_FASTMCP.md.
    json!({
        "io.modelcontextprotocol/protocolVersion": "2026-07-28",
        "io.modelcontextprotocol/clientCapabilities": {
            "tools": {"listChanged": true}
        },
        "io.modelcontextprotocol/clientInfo": {
            "name": "dfmcp-modern-handshake-golden",
            "version": "0.0.1"
        }
    })
}

fn assert_modern_envelope(value: &Value, expected_id: u64) {
    assert_eq!(value["jsonrpc"], "2.0");
    assert_eq!(value["id"], expected_id);
    assert!(
        value.get("error").is_none(),
        "unexpected JSON-RPC error: {value}"
    );
}

#[test]
#[ignore = "KNOWN UPSTREAM BLOCKER, NOT A PASS: fastmcp_rust v0.8.0 stops writing responses at tools/list after a successful modern server/discover, so this blocking stdio harness cannot complete. The defect is still DRAFT/unfiled in docs/DOGFOODING_FASTMCP.md; remove this ignore only with a conforming pin bump. In-process semantic coverage is not transport conformance."]
fn test_modern_handshake_full_lifecycle_and_plan_commit() -> Result<(), Box<dyn Error>> {
    let mut client = StdioClient::spawn()?;

    // 1. Discover — modern MCP 2026-07-28 server/discover shape: a
    //    `supportedVersions` array and `serverInfo` nested under `_meta`
    //    (the legacy `result.protocolVersion` / `result.serverInfo` fields
    //    are NOT emitted in modern-only mode).
    let discover_req = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "server/discover",
        "params": { "_meta": modern_meta() }
    });
    let discover_resp = client.send(&discover_req)?;
    assert_eq!(discover_resp["jsonrpc"], "2.0");
    assert_eq!(discover_resp["id"], 1);
    let supported = discover_resp["result"]["supportedVersions"]
        .as_array()
        .ok_or("supportedVersions array missing")?;
    assert!(
        supported
            .iter()
            .any(|value| value.as_str() == Some("2026-07-28")),
        "supportedVersions must contain 2026-07-28, got {supported:?}"
    );
    assert_eq!(
        discover_resp["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "dwarf-fortress-mcp"
    );
    assert_eq!(
        discover_resp["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        "0.0.1"
    );

    // 2. Tools list — frozen 11-tool fortress.* waist (dots rendered as underscores).
    let tools_list_req = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": { "_meta": modern_meta() }
    });
    let tools_list_resp = client.send(&tools_list_req)?;
    assert_modern_envelope(&tools_list_resp, 2);
    let tools = tools_list_resp["result"]["tools"]
        .as_array()
        .ok_or("tools should be an array")?;
    let tool_names: Vec<&str> = tools.iter().filter_map(|t| t["name"].as_str()).collect();
    assert_eq!(
        tool_names,
        vec![
            "fortress_open_session",
            "fortress_observe",
            "fortress_query",
            "fortress_plan",
            "fortress_commit",
            "fortress_wait",
            "fortress_cancel",
            "fortress_checkpoint",
            "fortress_restore",
            "fortress_explain",
            "fortress_doctor",
        ]
    );

    // 3. Open session (paused = true). lab adapter seed. Returns a session_id
    //    that all subsequent calls must echo back via the `session_id`
    //    argument (WP-13 gate 2 — session-scoped capability negotiation).
    let open_session_req = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_open_session",
            "arguments": { "paused": true }
        }
    });
    let open_session_resp = client.send(&open_session_req)?;
    assert_modern_envelope(&open_session_resp, 3);
    let open_text = open_session_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    let open_data: Value = serde_json::from_str(open_text)?;
    assert_eq!(open_data["ok"], true);
    assert_eq!(open_data["adapter"], "dfmcp-memory-lab");
    assert_eq!(open_data["paused"], true);
    let session_id = open_data["session_id"]
        .as_str()
        .ok_or("session_id string missing from open_session response")?
        .to_owned();
    assert!(
        !session_id.is_empty(),
        "session_id must be a non-empty u128 hex string"
    );

    // 4. Observe — bounded summary projection, scoped to the session.
    let observe_req = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_observe",
            "arguments": { "session_id": session_id.clone() }
        }
    });
    let observe_resp = client.send(&observe_req)?;
    assert_modern_envelope(&observe_resp, 4);
    let observe_text = observe_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    let observe_data: Value = serde_json::from_str(observe_text)?;
    assert_eq!(observe_data["ok"], true);
    assert_eq!(observe_data["projection"], "summary");
    assert_eq!(observe_data["paused"], true);

    // 5. Plan — unpause simulation, sealed plan with digest.
    let plan_req = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_plan",
            "arguments": {
                "session_id": session_id.clone(),
                "summary": "unpause the simulation",
                "paused_target": false
            }
        }
    });
    let plan_resp = client.send(&plan_req)?;
    assert_modern_envelope(&plan_resp, 5);
    let plan_text = plan_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    let plan_data: Value = serde_json::from_str(plan_text)?;
    assert_eq!(plan_data["ok"], true);
    let plan_digest = plan_data["plan_digest"]
        .as_str()
        .ok_or("plan_digest string missing")?;
    assert_eq!(plan_digest.len(), 64, "plan_digest must be a SHA-256 hex");
    // 6. Commit — digest-matched prepare → commit → observe → verify.
    let commit_req = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_commit",
            "arguments": {
                "session_id": session_id.clone(),
                "plan_digest": plan_digest
            }
        }
    });
    let commit_resp = client.send(&commit_req)?;
    assert_modern_envelope(&commit_resp, 6);
    let commit_text = commit_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    let commit_data: Value = serde_json::from_str(commit_text)?;
    assert_eq!(commit_data["ok"], true);
    assert_eq!(commit_data["paused"], false);
    assert_eq!(commit_data["actions"][0]["state"], "Verified");

    // 7. Commit idempotency — repeated commit with same plan_digest returns the
    //    prior receipt verbatim (ADR-006). Use a fresh JSON-RPC id (id=7)
    //    because the transport layer rejects duplicate ids even when the
    //    underlying commit is a server-side replay.
    let commit_repeat_req = json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_commit",
            "arguments": {
                "session_id": session_id.clone(),
                "plan_digest": plan_digest
            }
        }
    });
    let commit_repeat_resp = client.send(&commit_repeat_req)?;
    assert_modern_envelope(&commit_repeat_resp, 7);
    let repeat_text = commit_repeat_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    assert_eq!(repeat_text, commit_text);

    // 8. Explain — transcript-tail evidence.
    let explain_req = json!({
        "jsonrpc": "2.0",
        "id": 8,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_explain",
            "arguments": { "session_id": session_id.clone() }
        }
    });
    let explain_resp = client.send(&explain_req)?;
    assert_modern_envelope(&explain_resp, 8);
    let explain_text = explain_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    let explain_data: Value = serde_json::from_str(explain_text)?;
    assert_eq!(explain_data["ok"], true);
    assert!(explain_data["transcript_len"].as_u64().unwrap_or(0) > 0);

    // 9. Doctor — health and compatibility report.
    let doctor_req = json!({
        "jsonrpc": "2.0",
        "id": 9,
        "method": "tools/call",
        "params": {
            "_meta": modern_meta(),
            "name": "fortress_doctor",
            "arguments": { "session_id": session_id.clone() }
        }
    });
    let doctor_resp = client.send(&doctor_req)?;
    assert_modern_envelope(&doctor_resp, 9);
    let doctor_text = doctor_resp["result"]["content"][0]["text"]
        .as_str()
        .ok_or("content text missing")?;
    let doctor_data: Value = serde_json::from_str(doctor_text)?;
    assert_eq!(doctor_data["ok"], true);
    assert_eq!(doctor_data["status"], "healthy");
    Ok(())
}

/// Assert that a JSON-RPC error response matches the exact era-refusal
/// shape emitted by the fastmcp_rust v0.8.0 `StdioEraClassifier`: code
/// `-32600` (Invalid Request), fixed message, and a `supported` array
/// that advertises both protocol eras.
fn assert_era_refusal(resp: &Value, expected_id: u64) {
    assert_eq!(resp["jsonrpc"], "2.0", "envelope must be JSON-RPC 2.0");
    assert_eq!(resp["id"], expected_id, "response id must echo request id");
    let error = &resp["error"];
    assert!(error.is_object(), "expected error object, got {resp}");
    assert_eq!(
        error["code"], -32600,
        "era refusal must use JSON-RPC Invalid Request code (-32600)"
    );
    assert_eq!(
        error["message"], "Request does not match the connection's negotiated MCP protocol era",
        "era refusal message must match the fastmcp_rust v0.8.0 wire text"
    );
    let is_array = error["data"]["supported"].is_array();
    let supported = if let Some(supported) = error["data"]["supported"].as_array() {
        supported
    } else {
        assert!(is_array, "error.data.supported must be an array");
        return;
    };
    assert!(
        supported.iter().any(|v| v.as_str() == Some("2026-07-28")),
        "supported array must include 2026-07-28"
    );
    assert!(
        supported.iter().any(|v| v.as_str() == Some("2024-11-05")),
        "upstream era refusal currently advertises the disabled legacy era"
    );
}

#[test]
fn test_negative_era_refusal_and_marker_validations() -> Result<(), Box<dyn Error>> {
    // Negative 1: Mixed era markers on initialize — both `protocolVersion`
    // and `_meta["io.modelcontextprotocol/protocolVersion"]` present at once
    // is rejected as `MixedEraMarkers` by the fastmcp_rust
    // `StdioEraClassifier`.
    {
        let mut client = StdioClient::spawn()?;
        let mixed_req: Value =
            serde_json::from_str(include_str!("fixtures/neg_mixed_era_request.json"))?;
        let mixed_resp = client.send(&mixed_req)?;
        assert_era_refusal(&mixed_resp, 100);
        let expected: Value =
            serde_json::from_str(include_str!("fixtures/neg_mixed_era_response.json"))?;
        assert_eq!(mixed_resp, expected);
    }

    // Negative 2: Bare legacy initialize without modern marker — the
    // legacy era is refused because the server is modern-only.
    {
        let mut client = StdioClient::spawn()?;
        let bare_req: Value =
            serde_json::from_str(include_str!("fixtures/neg_bare_initialize_request.json"))?;
        let bare_resp = client.send(&bare_req)?;
        assert_era_refusal(&bare_resp, 101);
        let expected: Value =
            serde_json::from_str(include_str!("fixtures/neg_bare_initialize_response.json"))?;
        assert_eq!(bare_resp, expected);
    }

    // Negative 3: Once modern era is established, a request missing the
    // modern `_meta` marker must be rejected as `CrossEraTraffic`.
    {
        let mut client = StdioClient::spawn()?;
        let discover_req: Value =
            serde_json::from_str(include_str!("fixtures/01_discover_request.json"))?;
        let _ = client.send(&discover_req)?;

        let missing_meta_req: Value =
            serde_json::from_str(include_str!("fixtures/neg_missing_meta_request.json"))?;
        let missing_meta_resp = client.send(&missing_meta_req)?;
        assert_era_refusal(&missing_meta_resp, 2);
        let expected: Value =
            serde_json::from_str(include_str!("fixtures/neg_missing_meta_response.json"))?;
        assert_eq!(missing_meta_resp, expected);
    }
    Ok(())
}
