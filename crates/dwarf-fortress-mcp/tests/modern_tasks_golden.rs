#![forbid(unsafe_code)]

//! Real-process regressions for df-fastmcp-conformance-5pj.3. Tasks retain
//! the original plan, and only explicit fortress_wait requests advance the
//! injected laboratory game clock. No test sleeps to manufacture progress.

use std::error::Error;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, sync_channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use dfmcp_core::Digest32;
use serde_json::{Value, json};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const TASKS_EXTENSION: &str = "io.modelcontextprotocol/tasks";
const TILE: [i32; 3] = [3, 5, 10];

/// The reader belongs to this client and is joined after the process exits.
/// Notifications may precede replies, so requests match their JSON-RPC ID.
struct StdioClient {
    child: Child,
    responses: Option<Receiver<Result<Value, String>>>,
    reader: Option<JoinHandle<()>>,
    deadline: Instant,
    next_id: u64,
    tasks_negotiated: bool,
}

impl StdioClient {
    fn spawn(tasks_negotiated: bool) -> TestResult<Self> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_dwarf-fortress-mcp"))
            .arg("serve")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            return Err("child stdout unavailable".into());
        };
        let (sender, responses) = sync_channel(1);
        let reader = match std::thread::Builder::new()
            .name("dfmcp-modern-tasks-test-reader".to_owned())
            .spawn(move || {
                let mut reader = BufReader::new(stdout);
                loop {
                    const MAX_FRAME_BYTES: u64 = 8 * 1024 * 1024;
                    let mut line = Vec::new();
                    let result = match (&mut reader)
                        .take(MAX_FRAME_BYTES + 1)
                        .read_until(b'\n', &mut line)
                    {
                        Ok(0) => Err("child stdout closed before a response".to_owned()),
                        Ok(length) if length as u64 > MAX_FRAME_BYTES => {
                            Err("child response exceeded the frame bound".to_owned())
                        }
                        Ok(_) => match std::str::from_utf8(&line) {
                            Ok(line) if line.trim().starts_with('{') => {
                                serde_json::from_str(line).map_err(|error| error.to_string())
                            }
                            Ok(_) => continue,
                            Err(error) => Err(error.to_string()),
                        },
                        Err(error) => Err(error.to_string()),
                    };
                    let terminal = result.is_err();
                    if sender.send(result).is_err() || terminal {
                        break;
                    }
                }
            }) {
            Ok(reader) => reader,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error.into());
            }
        };
        Ok(Self {
            child,
            responses: Some(responses),
            reader: Some(reader),
            deadline: Instant::now() + Duration::from_secs(60),
            next_id: 1,
            tasks_negotiated,
        })
    }

    fn meta(&self) -> Value {
        let mut capabilities = json!({"tools": {"listChanged": true}});
        if self.tasks_negotiated {
            capabilities["extensions"] = json!({"io.modelcontextprotocol/tasks": {}});
        }
        json!({
            "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": capabilities,
            "io.modelcontextprotocol/clientInfo": {
                "name": "dfmcp-modern-tasks-golden", "version": "0.0.1"
            }
        })
    }

    fn rpc(&mut self, method: &str, mut params: Value) -> TestResult<Value> {
        let id = self.next_id;
        self.next_id += 1;
        params["_meta"] = self.meta();
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let stdin = self.child.stdin.as_mut().ok_or("child stdin unavailable")?;
        stdin.write_all(request.to_string().as_bytes())?;
        stdin.write_all(b"\n")?;
        stdin.flush()?;

        for _ in 0..128 {
            let remaining = self
                .deadline
                .checked_duration_since(Instant::now())
                .ok_or("Tasks lifecycle exceeded its independent 60-second deadline")?;
            let frame = self
                .responses
                .as_ref()
                .ok_or("response reader unavailable")?
                .recv_timeout(remaining.min(Duration::from_secs(10)))
                .map_err(|error| {
                    format!("{method} response deadline/channel failure: {error}")
                })??;
            assert_eq!(frame["jsonrpc"], "2.0", "{frame}");
            if frame.get("id").is_none() {
                assert!(frame["method"].is_string(), "invalid notification: {frame}");
                continue;
            }
            assert_eq!(frame["id"], id, "reply must match {method}: {frame}");
            return Ok(frame);
        }
        Err("notification flood exceeded the response bound".into())
    }

    fn method(&mut self, method: &str, params: Value) -> TestResult<Value> {
        let response = self.rpc(method, params)?;
        assert!(response.get("error").is_none(), "{method}: {response}");
        assert!(response["result"].is_object(), "{method}: {response}");
        Ok(response["result"].clone())
    }

    fn discover(&mut self) -> TestResult<Value> {
        let discovered = self.method("server/discover", json!({}))?;
        assert_eq!(discovered["supportedVersions"], json!(["2026-07-28"]));
        Ok(discovered)
    }

    fn tool(&mut self, name: &str, arguments: Value) -> TestResult<Value> {
        let result = self.method("tools/call", json!({"name": name, "arguments": arguments}))?;
        let payload = tool_payload(&result)?;
        assert_eq!(payload["ok"], true, "{name}: {payload}");
        Ok(payload)
    }

    fn open(&mut self, fortress: &str, paused: bool) -> TestResult<String> {
        self.open_with_output_budget(fortress, paused, 32_768)
    }

    fn open_with_output_budget(
        &mut self,
        fortress: &str,
        paused: bool,
        max_output_tokens: u32,
    ) -> TestResult<String> {
        let opened = self.tool(
            "fortress_open_session",
            json!({
                "fortress_selector": fortress,
                "paused": paused,
                "scenario": "starter_fortress",
                "max_wall_millis": 60_000,
                "max_game_ticks": 2_000,
                "max_output_tokens": max_output_tokens,
                "requested_capabilities": [
                    ["observe", "read_only"], ["query", "read_only"],
                    ["plan", "reversible"], ["control_clock", "reversible"],
                    ["checkpoint", "guarded"], ["designate", "guarded"],
                    ["construct", "guarded"], ["configure_labor", "reversible"]
                ]
            }),
        )?;
        Ok(opened["session_id"]
            .as_str()
            .ok_or("session ID missing")?
            .to_owned())
    }

    fn plan(&mut self, session: &str, with_build: bool) -> TestResult<Value> {
        let mut actions = vec![json!({"action": {
            "kind": "designate_dig", "min": TILE, "max": TILE, "mode": "mine"
        }})];
        if with_build {
            actions.push(json!({"action": {
                "kind": "build", "building": "furniture:Bed",
                "location": TILE, "min": TILE, "max": TILE
            }, "depends_on": [0]}));
        }
        self.tool(
            "fortress_plan",
            json!({
                "session_id": session, "summary": "retain this original excavation plan",
                "actions": serde_json::to_string(&actions)?
            }),
        )
    }

    fn start_task(&mut self, session: &str, digest: &str) -> TestResult<String> {
        let task = self.method(
            "tools/call",
            json!({
                "name": "fortress_commit", "arguments": {
                    "session_id": session, "plan_digest": digest, "as_task": true
                }
            }),
        )?;
        assert_eq!(task["resultType"], "task", "{task}");
        assert_eq!(task["status"], "working", "{task}");
        assert!(
            task.get("result").is_none(),
            "queued work has no completion proof: {task}"
        );
        assert!(
            task.get("task").is_none(),
            "modern task fields are flattened: {task}"
        );
        assert!(task["createdAt"].is_string());
        assert!(task["lastUpdatedAt"].is_string());
        assert_eq!(task.get("ttlMs"), Some(&Value::Null));
        Ok(task["taskId"]
            .as_str()
            .ok_or("opaque task ID missing")?
            .to_owned())
    }

    fn task(&mut self, id: &str) -> TestResult<Value> {
        let task = self.method("tasks/get", json!({"taskId": id}))?;
        assert_eq!(task["resultType"], "complete", "{task}");
        assert_eq!(task["taskId"], id);
        assert!(
            task.get("task").is_none(),
            "modern task fields are flattened: {task}"
        );
        Ok(task)
    }

    fn resource(&mut self, session: &str, view: &str) -> TestResult<Value> {
        let uri = format!("df://session/{session}/{view}");
        let result = self.method("resources/read", json!({"uri": uri}))?;
        let text = result["contents"][0]["text"]
            .as_str()
            .ok_or("resource text missing")?;
        Ok(serde_json::from_str(text)?)
    }

    fn task_record(&mut self, session: &str, id: &str) -> TestResult<Value> {
        let resource = self.resource(session, &format!("task-{id}"))?;
        assert_eq!(resource["task_id"], id);
        Ok(resource)
    }

    fn await_dispatch(&mut self, session: &str, id: &str) -> TestResult<Value> {
        for _ in 0..256 {
            let record = self.task_record(session, id)?;
            assert_ne!(
                record["task"]["status"], "failed",
                "initial commit failed: {record}"
            );
            if record["progress"]["obligation"]["actions"].is_array() {
                return Ok(record);
            }
        }
        Err("accepted task never observed its original committed actions".into())
    }

    fn await_terminal(&mut self, id: &str, expected: &str) -> TestResult<Value> {
        for _ in 0..256 {
            let task = self.task(id)?;
            if task["status"] != "working" {
                assert_eq!(task["status"], expected, "{task}");
                return Ok(task);
            }
        }
        Err(format!("task did not become {expected} after explicit engine progress").into())
    }

    fn query(&mut self, session: &str, spec: Value) -> TestResult<Value> {
        self.tool(
            "fortress_query",
            json!({"session_id": session, "mode": spec.to_string()}),
        )
    }

    fn entities(&mut self, session: &str, kind: &str) -> TestResult<Value> {
        self.query(
            session,
            json!({"mode": "entities", "kind": kind, "limit": 100}),
        )
    }

    fn terrain(&mut self, session: &str) -> TestResult<Value> {
        self.query(
            session,
            json!({"mode": "terrain", "min": TILE, "max": TILE}),
        )
    }

    fn advance(&mut self, session: &str, ticks: u64) -> TestResult<Value> {
        self.tool(
            "fortress_wait",
            json!({"session_id": session, "max_game_ticks": ticks}),
        )
    }
}

impl Drop for StdioClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        drop(self.responses.take());
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn tool_payload(result: &Value) -> TestResult<Value> {
    assert_ne!(result["isError"], true, "tool result: {result}");
    if let Some(payload) = result
        .get("structuredContent")
        .filter(|payload| payload.is_object())
    {
        return Ok(payload.clone());
    }
    let text = result["content"][0]["text"]
        .as_str()
        .ok_or("tool text missing")?;
    Ok(serde_json::from_str(text)?)
}

fn plan_digest(plan: &Value) -> TestResult<&str> {
    plan["plan_digest"]
        .as_str()
        .ok_or("sealed plan digest missing".into())
}

fn assert_refused(response: &Value) {
    let resource_refusal = response["result"]["contents"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .is_some_and(|payload| payload["ok"] == false);
    assert!(
        response["error"].is_object() || response["result"]["isError"] == true || resource_refusal,
        "operation must refuse before producing work: {response}"
    );
    assert_ne!(response["result"]["resultType"], "task");
}

fn assert_actions_proven(
    payload: &Value,
    digest: &str,
    state: &str,
    expected_actions: usize,
) -> TestResult {
    assert_eq!(payload["plan_digest"], digest);
    let actions = payload["actions"]
        .as_array()
        .ok_or("original action evidence missing")?;
    assert_eq!(
        actions.len(),
        expected_actions,
        "every original step must be represented: {payload}"
    );
    for (step, action) in actions.iter().enumerate() {
        assert_eq!(action["step"], step);
        assert_eq!(action["state"], state, "{action}");
        assert_eq!(action["receipt_digest"].as_str().map(str::len), Some(64));
        let evidence = action["evidence"]
            .as_array()
            .ok_or("evidence array missing")?;
        assert!(!evidence.is_empty());
        for evidence in evidence {
            assert_eq!(evidence["digest"].as_str().map(str::len), Some(64));
            assert_eq!(evidence["anchor"], action["observed_anchor"]);
        }
    }
    Ok(())
}

fn find_entity<'a>(page: &'a Value, id: &str) -> Option<&'a Value> {
    page["rows"]
        .as_array()?
        .iter()
        .find(|entity| entity["entity_id"] == id)
}

#[test]
fn modern_task_proves_every_original_step_and_preserves_terminal_evidence() -> TestResult {
    let mut client = StdioClient::spawn(true)?;
    let discovered = client.discover()?;
    assert_eq!(
        discovered["capabilities"]["extensions"][TASKS_EXTENSION],
        json!({})
    );
    let tools = client.method("tools/list", json!({}))?;
    let list = tools["tools"].as_array().ok_or("tool list missing")?;
    assert_eq!(list.len(), 11, "Tasks must preserve the frozen tool waist");
    let commit_tool = list
        .iter()
        .find(|tool| tool["name"] == "fortress_commit")
        .ok_or("fortress_commit definition missing")?;
    assert_eq!(
        commit_tool["inputSchema"]["properties"]["as_task"]["type"],
        "boolean"
    );
    for method in ["tasks/get", "tasks/cancel"] {
        assert_refused(&client.rpc(method, json!({"taskId": "unknown-task-handle"}))?);
    }

    let session = client.open("901", false)?;
    let plan = client.plan(&session, true)?;
    let digest = plan_digest(&plan)?.to_owned();
    let created_building = plan["steps"][1]["creates_entity_id"]
        .as_str()
        .ok_or("planned building identity missing")?
        .to_owned();
    let before_admission = client.terrain(&session)?;
    assert_refused(&client.rpc(
        "tools/call",
        json!({
            "name": "fortress_commit", "arguments": {
                "session_id": session, "plan_digest": digest, "as_task": "true"
            }
        }),
    )?);
    assert_eq!(
        client.terrain(&session)?["anchor"],
        before_admission["anchor"]
    );
    let id = client.start_task(&session, &digest)?;
    let initial = client.await_dispatch(&session, &id)?;
    assert_eq!(initial["plan_digest"], digest);
    let discovery = client.resource(&session, "tasks")?;
    assert_eq!(discovery["max_active"], 1);
    assert_eq!(discovery["retention"], "process_lifetime");
    assert_eq!(discovery["tasks"].as_array().map(Vec::len), Some(1));
    assert_eq!(discovery["tasks"][0]["task_id"], id);
    assert_eq!(
        discovery["tasks"][0]["details"],
        format!("df://session/{session}/task-{id}")
    );
    assert!(
        discovery["tasks"][0].get("progress").is_none(),
        "bounded task discovery must link to detailed evidence"
    );
    assert_eq!(
        initial["progress"]["obligation"]["actions"][0]["state"],
        "AppliedAwaitingVerification"
    );
    assert_eq!(
        initial["progress"]["obligation"]["actions"][1]["state"],
        "Prepared"
    );
    let initial_terrain = client.terrain(&session)?;
    assert_eq!(initial_terrain["levels"][0]["rows"][0], "#");
    for _ in 0..3 {
        let waiting = client.task(&id)?;
        assert_eq!(waiting["status"], "working");
        assert!(waiting.get("result").is_none());
    }
    let observation = client.tool("fortress_observe", json!({"session_id": session}))?;
    assert!(
        observation["agent_turn"]["active_work"]["mcp_tasks"]
            .as_array()
            .ok_or("Agent Turn must retain task handles")?
            .iter()
            .any(|task| task["task_id"] == id)
    );
    assert_eq!(
        client.terrain(&session)?["anchor"],
        initial_terrain["anchor"],
        "task inspection cannot advance the game or dispatch the deferred step"
    );

    let mut completed = None;
    for _ in 0..16 {
        client.advance(&session, 50)?;
        let task = client.task(&id)?;
        if task["status"] == "completed" {
            completed = Some(task);
            break;
        }
        assert_eq!(task["status"], "working", "{task}");
    }
    let completed =
        completed.ok_or("dig and dependent build did not complete in bounded game time")?;
    let payload = tool_payload(&completed["result"])?;
    assert_eq!(payload["schema"], "dfmcp.lab-plan-task/1");
    assert_eq!(payload["status"], "completed");
    assert_eq!(completed["result"]["structuredContent"], payload);
    assert_actions_proven(&payload, &digest, "Verified", 2)?;
    let buildings = client.entities(&session, "building")?;
    assert_eq!(
        find_entity(&buildings, &created_building).ok_or("verified building absent")?["fields"]["construction_stage"],
        "complete"
    );

    // A normal repeat of the same commit must replay its original action
    // identities, including after Tasks supervision has completed them.
    let before_repeat = client.terrain(&session)?;
    let repeat = client.tool(
        "fortress_commit",
        json!({"session_id": session, "plan_digest": digest}),
    )?;
    let repeated_again = client.tool(
        "fortress_commit",
        json!({"session_id": session, "plan_digest": digest}),
    )?;
    assert_eq!(repeat["actions"], repeated_again["actions"]);
    for step in 0..2 {
        assert_eq!(
            repeat["actions"][step]["action_id"],
            payload["actions"][step]["action_id"]
        );
    }
    assert_eq!(client.terrain(&session)?["anchor"], before_repeat["anchor"]);
    assert_refused(&client.rpc("tasks/cancel", json!({"taskId": id}))?);
    client.advance(&session, 1)?;
    assert_eq!(
        client.task(&id)?,
        completed,
        "terminal task evidence must not be refreshed to a later anchor"
    );
    let before_recovery = client.terrain(&session)?;
    let recovered_id = client.start_task(&session, &digest)?;
    let recovered = client.await_terminal(&recovered_id, "completed")?;
    let recovered_payload = tool_payload(&recovered["result"])?;
    assert_eq!(
        recovered_payload["actions"], payload["actions"],
        "supervising an already committed digest must reuse its original receipts"
    );
    assert_eq!(
        client.terrain(&session)?["anchor"],
        before_recovery["anchor"]
    );
    Ok(())
}

#[test]
fn unnegotiated_task_refuses_without_consuming_the_sealed_plan() -> TestResult {
    let mut client = StdioClient::spawn(false)?;
    client.discover()?;
    let session = client.open("902", true)?;
    let plan = client.plan(&session, false)?;
    let digest = plan_digest(&plan)?;
    let before = client.terrain(&session)?;
    let refused = client.rpc(
        "tools/call",
        json!({"name": "fortress_commit", "arguments": {
            "session_id": session, "plan_digest": digest, "as_task": true
        }}),
    )?;
    assert_refused(&refused);
    assert_eq!(client.terrain(&session)?["anchor"], before["anchor"]);
    assert_eq!(client.entities(&session, "dig_designation")?["total"], 0);
    assert_eq!(client.resource(&session, "tasks")?["tasks"], json!([]));
    let ordinary = client.tool(
        "fortress_commit",
        json!({
            "session_id": session, "plan_digest": digest, "as_task": false
        }),
    )?;
    assert_eq!(ordinary["plan_digest"], digest);
    assert_eq!(
        ordinary["actions"][0]["state"],
        "AppliedAwaitingVerification"
    );
    assert_eq!(client.entities(&session, "dig_designation")?["total"], 1);
    Ok(())
}

#[test]
fn task_capacity_refuses_before_effects_in_an_independent_session() -> TestResult {
    let mut client = StdioClient::spawn(true)?;
    client.discover()?;
    let first = client.open("903", true)?;
    let first_plan = client.plan(&first, false)?;
    let id = client.start_task(&first, plan_digest(&first_plan)?)?;
    client.await_dispatch(&first, &id)?;
    let second = client.open("904", true)?;
    assert_refused(&client.rpc(
        "resources/read",
        json!({
            "uri": format!("df://session/{second}/task-{id}")
        }),
    )?);
    let second_plan = client.tool("fortress_plan", json!({
        "session_id": second, "summary": "resume only if admission succeeds", "paused_target": false
    }))?;
    let digest = plan_digest(&second_plan)?;
    let before = client.terrain(&second)?;
    let refused = client.rpc(
        "tools/call",
        json!({"name": "fortress_commit", "arguments": {
            "session_id": second, "plan_digest": digest, "as_task": true
        }}),
    )?;
    assert_refused(&refused);
    assert_eq!(client.terrain(&second)?["anchor"], before["anchor"]);
    assert_eq!(client.resource(&second, "tasks")?["tasks"], json!([]));
    assert_eq!(client.task(&id)?["status"], "working");
    let ordinary = client.tool(
        "fortress_commit",
        json!({"session_id": second, "plan_digest": digest}),
    )?;
    assert_eq!(ordinary["actions"][0]["state"], "Verified");
    assert_eq!(ordinary["paused"], false);
    client.method("tasks/cancel", json!({"taskId": id}))?;
    client.await_terminal(&id, "cancelled")?;
    Ok(())
}

#[test]
fn cancellation_drains_original_plan_after_later_commit_without_dispatching_child() -> TestResult {
    let mut client = StdioClient::spawn(true)?;
    client.discover()?;
    let session = client.open("905", true)?;
    let plan = client.plan(&session, true)?;
    let digest = plan_digest(&plan)?.to_owned();
    let designation = plan["steps"][0]["creates_entity_id"]
        .as_str()
        .ok_or("designation identity missing")?
        .to_owned();
    let building = plan["steps"][1]["creates_entity_id"]
        .as_str()
        .ok_or("building identity missing")?
        .to_owned();
    let id = client.start_task(&session, &digest)?;
    let original = client.await_dispatch(&session, &id)?;
    let original_actions = &original["progress"]["obligation"]["actions"];

    let later_plan = client.tool(
        "fortress_plan",
        json!({
            "session_id": session, "summary": "later foreground resume", "paused_target": false
        }),
    )?;
    let later_digest = plan_digest(&later_plan)?.to_owned();
    let later = client.tool(
        "fortress_commit",
        json!({"session_id": session, "plan_digest": later_digest}),
    )?;
    assert_eq!(later["actions"][0]["state"], "Verified");
    let acknowledgement = client.method("tasks/cancel", json!({"taskId": id}))?;
    assert_eq!(acknowledgement["resultType"], "complete");
    let cancelled = client.await_terminal(&id, "cancelled")?;
    assert!(cancelled.get("result").is_none());
    let record = client.task_record(&session, &id)?;
    assert_eq!(record["plan_digest"], digest);
    let requested = &record["progress"]["request"];
    let finalized = &record["progress"]["finalization"];
    assert_eq!(requested["stage"], "cancel_requested");
    assert_eq!(requested["drain_progress"]["remaining_nonterminal"], 2);
    assert_eq!(requested["steps"][1]["before"], "Prepared");
    assert_eq!(requested["steps"][1]["after"], "CancelRequested");
    assert_eq!(finalized["stage"], "finalized");
    assert_eq!(finalized["plan_digest"], digest);
    assert_eq!(finalized["drain_progress"]["quiescent"], true);
    assert_eq!(finalized["drain_progress"]["remaining_nonterminal"], 0);
    assert_eq!(
        finalized["finalize_certificate"]["digest"]
            .as_str()
            .map(str::len),
        Some(64)
    );
    for step in 0..2 {
        assert_eq!(
            finalized["steps"][step]["action_id"],
            original_actions[step]["action_id"]
        );
        assert_eq!(finalized["steps"][step]["after"], "Cancelled");
        assert_ne!(
            finalized["steps"][step]["action_id"],
            later["actions"][0]["action_id"]
        );
    }
    let repeat = client.tool(
        "fortress_commit",
        json!({"session_id": session, "plan_digest": later_digest}),
    )?;
    assert_eq!(
        repeat["actions"], later["actions"],
        "later verified work must remain history"
    );
    client.advance(&session, 50)?;
    assert_eq!(client.terrain(&session)?["levels"][0]["rows"][0], "#");
    assert!(find_entity(&client.entities(&session, "building")?, &building).is_none());
    let designations = client.entities(&session, "dig_designation")?;
    assert_eq!(
        find_entity(&designations, &designation).ok_or("cancelled designation absent")?["fields"]["status"],
        "cancelled"
    );
    assert_eq!(
        client.task_record(&session, &id)?["progress"],
        record["progress"],
        "the finalized drain certificate remains bound to its original anchor"
    );
    Ok(())
}

#[test]
fn late_observed_completion_fails_task_and_prevents_dependent_dispatch() -> TestResult {
    let mut client = StdioClient::spawn(true)?;
    client.discover()?;
    let session = client.open("906", false)?;
    let plan = client.plan(&session, true)?;
    let digest = plan_digest(&plan)?.to_owned();
    let deadline = plan["steps"][0]["obligation"]["deadline_tick"]
        .as_u64()
        .ok_or("dig deadline missing")?;
    let building = plan["steps"][1]["creates_entity_id"]
        .as_str()
        .ok_or("building identity missing")?
        .to_owned();
    let id = client.start_task(&session, &digest)?;
    client.await_dispatch(&session, &id)?;
    let before = client.terrain(&session)?;
    let tick = before["game_tick"].as_u64().ok_or("game tick missing")?;
    let late = deadline
        .checked_sub(tick)
        .and_then(|remaining| remaining.checked_add(1))
        .ok_or("plan deadline must be after dispatch")?;
    client.advance(&session, late)?;
    let failed = client.await_terminal(&id, "failed")?;
    assert!(failed.get("result").is_none());
    assert!(failed["error"]["message"].is_string());
    let evidence = &failed["error"]["data"];
    assert_eq!(evidence["ok"], false);
    assert_eq!(evidence["blind_retry_allowed"], false);
    assert_actions_proven(evidence, &digest, "Failed", 2)?;
    assert_eq!(
        client.terrain(&session)?["levels"][0]["rows"][0],
        ".",
        "observing a completed excavation after its deadline cannot prove on-time success"
    );
    assert!(find_entity(&client.entities(&session, "building")?, &building).is_none());
    assert_eq!(client.task(&id)?, failed);
    Ok(())
}

#[test]
fn retained_task_discovery_pages_handles_and_keeps_older_evidence_addressable() -> TestResult {
    let mut client = StdioClient::spawn(true)?;
    client.discover()?;
    let session = client.open("907", true)?;
    let mut ids = Vec::new();
    let mut first_result = None;
    for index in 0..17 {
        let plan = client.tool(
            "fortress_plan",
            json!({
                "session_id": session,
                "summary": format!("bounded retained task {index}"),
                "paused_target": index % 2 != 0
            }),
        )?;
        let digest = plan_digest(&plan)?;
        let id = client.start_task(&session, digest)?;
        let completed = client.await_terminal(&id, "completed")?;
        let payload = tool_payload(&completed["result"])?;
        assert_eq!(payload["plan_digest"], digest);
        assert_eq!(payload["actions"][0]["state"], "Verified");
        if index == 0 {
            first_result = Some(completed["result"].clone());
        }
        ids.push(id);
    }

    let before = client.terrain(&session)?;
    let first = client.resource(&session, "tasks")?;
    assert_eq!(first["schema"], "dfmcp.lab-tasks/1");
    assert_eq!(first["total"], 17);
    assert_eq!(first["offset"], 0);
    assert_eq!(first["complete"], false);
    assert_eq!(first["active_coverage_complete"], true);
    assert_eq!(first["tasks"].as_array().map(Vec::len), Some(16));
    assert_eq!(first["next"], format!("df://session/{session}/tasks-16"));
    for (index, handle) in first["tasks"]
        .as_array()
        .ok_or("first task page missing")?
        .iter()
        .enumerate()
    {
        assert_eq!(handle["task_id"], ids[16 - index]);
        assert_eq!(handle["status"], "completed");
        assert!(handle.get("progress").is_none());
        assert!(handle.get("task").is_none());
    }

    let second = client.resource(&session, "tasks-16")?;
    assert_eq!(second["total"], 17);
    assert_eq!(second["offset"], 16);
    assert_eq!(second["active_coverage_complete"], false);
    assert_eq!(second["tasks"].as_array().map(Vec::len), Some(1));
    assert_eq!(second["tasks"][0]["task_id"], ids[0]);
    assert!(second["next"].is_null());
    let oldest = client.task_record(&session, &ids[0])?;
    assert_eq!(oldest["task"]["status"], "completed");
    assert_eq!(Some(oldest["task"]["result"].clone()), first_result);
    assert_eq!(client.terrain(&session)?["anchor"], before["anchor"]);
    assert_refused(&client.rpc(
        "resources/read",
        json!({
            "uri": format!("df://session/{session}/tasks-18")
        }),
    )?);
    Ok(())
}

#[test]
fn small_output_budget_retains_complete_digest_checked_action_evidence() -> TestResult {
    let mut client = StdioClient::spawn(true)?;
    client.discover()?;
    let session = client.open_with_output_budget("908", true, 1_500)?;
    let actions = (0..16)
        .map(|index| {
            json!({"action": {
                "kind": "set_labor", "units": [(1001 + index % 7).to_string()],
                "labor": (["MINE", "WOODCUT", "CARPENTRY"][index / 7]), "enabled": true
            }})
        })
        .collect::<Vec<_>>();
    let plan = client.tool(
        "fortress_plan",
        json!({
            "session_id": session, "summary": "sixteen individually proved labor assignments",
            "actions": serde_json::to_string(&actions)?
        }),
    )?;
    let digest = plan_digest(&plan)?.to_owned();
    let id = client.start_task(&session, &digest)?;
    let completed = client.await_terminal(&id, "completed")?;
    assert!(
        completed.to_string().len() <= 6_000,
        "the complete Tasks result envelope must honor the caller's byte/token budget"
    );
    let summary = tool_payload(&completed["result"])?;
    assert_eq!(summary["schema"], "dfmcp.lab-task-summary/1");
    assert_eq!(summary["plan_digest"], digest);
    assert_eq!(summary["action_counts"]["Verified"], 16);
    assert_eq!(summary["blind_retry_allowed"], false);
    assert_eq!(
        summary["evidence"]["coverage"],
        "summary_with_complete_evidence_retained"
    );
    let evidence_digest = summary["evidence"]["digest"]
        .as_str()
        .ok_or("full evidence digest missing")?
        .to_owned();
    let total_bytes = summary["evidence"]["bytes"]
        .as_u64()
        .ok_or("full evidence byte length missing")?;
    let mut next = Some(
        summary["evidence"]["next"]
            .as_str()
            .ok_or("evidence continuation missing")?
            .to_owned(),
    );
    let mut assembled = String::new();
    let mut pages = 0;
    for _ in 0..128 {
        let Some(uri) = next.take() else {
            break;
        };
        assert!(uri.starts_with(&format!("df://session/{session}/task-{id}~evidence-")));
        let result = client.method("resources/read", json!({"uri": uri}))?;
        assert!(
            result.to_string().len() <= 6_000,
            "every evidence resource response must honor the negotiated output budget"
        );
        let page: Value = serde_json::from_str(
            result["contents"][0]["text"]
                .as_str()
                .ok_or("evidence page text missing")?,
        )?;
        assert_eq!(page["schema"], "dfmcp.lab-task-evidence-page/1");
        assert_eq!(page["source"], "final_evidence");
        assert_eq!(page["digest"], evidence_digest);
        assert_eq!(page["offset"], assembled.len());
        assert_eq!(page["total_bytes"], total_bytes);
        let part = page["part"].as_str().ok_or("evidence page part missing")?;
        assert!(!part.is_empty(), "nonterminal page must make byte progress");
        assembled.push_str(part);
        next = page["next"].as_str().map(str::to_owned);
        assert_eq!(page["complete"], next.is_none());
        pages += 1;
    }
    assert!(
        next.is_none(),
        "full evidence must fit its bounded page traversal"
    );
    assert!(
        pages > 1,
        "this fixture must exercise multiple evidence pages"
    );
    assert_eq!(assembled.len() as u64, total_bytes);
    assert_eq!(
        Digest32::of_bytes(assembled.as_bytes()).to_hex(),
        evidence_digest
    );
    let full: Value = serde_json::from_str(&assembled)?;
    assert_eq!(full["schema"], "dfmcp.lab-plan-task/1");
    assert_actions_proven(&full, &digest, "Verified", 16)?;
    let detail = client.task_record(&session, &id)?;
    assert_eq!(detail["evidence"]["source"], "final_evidence");
    assert_eq!(detail["evidence"]["digest"], evidence_digest);
    assert_eq!(
        client.task(&id)?,
        completed,
        "reading proof pages cannot rewrite terminal output"
    );
    Ok(())
}
