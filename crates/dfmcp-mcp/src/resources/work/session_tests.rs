//! Exercise the public facade and registered resource handler with real
//! reference actions, obligations, recovery and current-session authority.

use super::*;
use crate::agent_facade as facade;
use crate::resources::SessionViewResource;
use crate::server::{read_context, resolve_session, resource_replay_bundle, with_session_read};
use dfmcp_core::{Capability, EntityId, GameTick};
use fastmcp_rust::prelude::{McpContext, McpError, McpResult, ResourceContent, ResourceHandler};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

fn parsed(raw: &str) -> Value {
    serde_json::from_str(raw).expect("public JSON")
}

fn open(selector: &str, durable: bool) -> String {
    let value = parsed(&facade::fortress_open_session(
        Some(true),
        Some(selector.to_owned()),
        None,
        None,
        None,
        None,
        None,
        Some(4_096),
        None,
        Some("starter_fortress".to_owned()),
        Some(false),
        Some(durable),
    ));
    assert_eq!(value["ok"], true, "{value}");
    value["session_id"].as_str().expect("session").to_owned()
}

fn prepare(session: &str, actions: Value) -> Value {
    let value = parsed(&facade::fortress_plan(
        Some(session.to_owned()),
        Some("Retained work for Urïst".to_owned()),
        None,
        Some(actions.to_string()),
        None,
    ));
    assert_eq!(value["ok"], true, "{value}");
    value
}

fn commit(session: &str, planned: &Value) -> String {
    let digest = planned["plan_digest"].as_str().expect("digest").to_owned();
    let value = parsed(&facade::fortress_commit(
        Some(session.to_owned()),
        digest.clone(),
    ));
    assert_eq!(value["ok"], true, "{value}");
    digest
}

fn dig_actions(deferred: bool) -> Value {
    let mut actions = vec![
        json!({"action":{"kind":"pause","paused":false}}),
        json!({"action":{"kind":"designate_dig","min":[0,3,10],"max":[7,5,10],"mode":"mine"}}),
    ];
    if deferred {
        actions.push(json!({"action":{"kind":"pause","paused":true},"depends_on":[1]}));
    }
    json!(actions)
}

fn catalog_uri(session: &str) -> String {
    format!("df://session/{session}/work")
}

fn route(session: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let params: HashMap<String, String> = [
        ("session_id".to_owned(), session.to_owned()),
        (
            "view".to_owned(),
            uri.rsplit('/').next().expect("view").to_owned(),
        ),
    ]
    .into();
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build()
        .expect("owned runtime");
    runtime.block_on(async {
        let context = McpContext::new(asupersync::Cx::current().expect("runtime context"), 1);
        SessionViewResource.read_with_uri(&context, uri, &params)
    })
}

fn read(session: &str, uri: &str) -> Value {
    let contents = route(session, uri).expect("authorized work resource");
    assert_eq!(contents.len(), 1);
    let text = contents[0].text.as_deref().expect("resource text");
    let maximum_id: Value = serde_json::from_str(&"9".repeat(4_096)).expect("exact numeric ID");
    let wire = json!({"jsonrpc":"2.0", "id":maximum_id, "result":{
        "contents":[{"uri":uri,"mimeType":"application/json","text":text}],
        "ttlMs":0,"cacheScope":"private"}})
    .to_string();
    assert!(
        wire.len() <= 4_096 * 4,
        "complete escaped response must fit the negotiated budget"
    );
    let payload = parsed(text);
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["game_mutation_dispatched"], false);
    assert_eq!(
        payload["agent_turn"]["coverage"]["absence_proof_scope"],
        json!([])
    );
    payload
}

fn error(error: McpError, code: &str) -> Value {
    let value = error.data.expect("structured error");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert_eq!(value["error"]["effect_may_have_occurred"], false);
    value
}

fn fingerprint(session: &str) -> Value {
    let handle = resolve_session(Some(session.to_owned())).expect("session");
    with_session_read(
        &handle,
        |error| panic!("{error}"),
        |guard| {
            json!({
                "request_id":read_context(guard).request_id.to_string(),
                "anchor":crate::server::anchor_json(&guard.adapter.snapshot().anchor()),
                "objectives":Digest32::of_bytes(format!("{:?}", guard.objectives).as_bytes()).to_hex(),
                "pending":guard.pending.as_ref().map(|pending|pending.digest.clone()),
                "carried":format!("{:?}", guard.carried),
                "replay":resource_replay_bundle(guard),
                "open_actions":guard.open_actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
                "receipts":guard.adapter.retained_action_ids().map(|id|json!({
                    "receipt":format!("{:?}", guard.adapter.action_receipt(id)),
                    "obligation":format!("{:?}", guard.adapter.action_obligation_status(id))})).collect::<Vec<_>>(),
                "commits":guard.commit_receipts,
            })
        },
    )
}

fn all_rows(session: &str) -> (Value, Vec<Value>) {
    let first = read(session, &catalog_uri(session));
    let mut page = first.clone();
    let mut rows = Vec::new();
    loop {
        assert_eq!(page["offset"].as_u64(), Some(rows.len() as u64));
        assert_eq!(page["catalog_digest"], first["catalog_digest"]);
        rows.extend(page["rows"].as_array().expect("rows").iter().cloned());
        let Some(next) = page["next"].as_str() else {
            break;
        };
        assert_eq!(page["complete"], false);
        page = read(session, next);
        assert!(rows.len() < 100, "bounded fixture traversal");
    }
    assert_eq!(page["complete"], true);
    assert_eq!(first["total"].as_u64(), Some(rows.len() as u64));
    (first, rows)
}

#[test]
fn public_work_resources_expose_active_deferred_and_terminal_receipts_without_tasks() {
    let session = open("3971101", false);
    let digest = commit(&session, &prepare(&session, dig_actions(true)));
    let pending = prepare(
        &session,
        json!([{"action":{"kind":"pause","paused":false}}]),
    );
    let before = fingerprint(&session);
    let (_, rows) = all_rows(&session);
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().all(|row| row["plan_digest"] == digest));
    assert!(rows.iter().any(|row| row["proof_state"] == "Verified"));
    assert!(rows.iter().any(|row| row["proof_state"] == "Prepared"));
    let active = rows
        .iter()
        .find(|row| row["action_kind"] == "designate_dig")
        .expect("active dig");
    assert_eq!(active["proof_state"], "AppliedAwaitingVerification");
    assert_eq!(active["physical_work"]["state"], "active");
    assert_eq!(active["physical_work"]["current"], true);
    let detail = read(&session, active["action"].as_str().expect("action link"));
    assert_eq!(detail["schema"], "dfmcp.lab-action-progress/1");
    assert_eq!(detail["original_custody"], "exact_original_step");
    assert_eq!(detail["current_goal_truth"], "not_evaluated");
    assert_eq!(detail["owned_by_current_session"], true);
    assert_eq!(detail["dispatch_authority"], false);
    let obligation = read(
        &session,
        active["obligation"].as_str().expect("obligation link"),
    );
    assert_eq!(obligation["schema"], "dfmcp.lab-obligation-progress/1");
    assert_eq!(obligation["obligation"]["monitor"]["state"], "active");
    assert!(
        obligation["obligation"]["required_stable_observations"]
            .as_u64()
            .is_some_and(|n| n > 0)
    );
    assert!(
        obligation["obligation"]["next_observation_tick"]
            .as_u64()
            .is_some()
    );
    assert_eq!(obligation["obligation"]["sampled_by_this_read"], false);
    assert_eq!(
        fingerprint(&session),
        before,
        "resources cannot poll, consume IDs, or alter pending review"
    );
    assert_eq!(before["pending"], pending["plan_digest"]);
    let handoff_contents =
        route(&session, &format!("df://session/{session}/handoff")).expect("handoff");
    let handoff = parsed(handoff_contents[0].text.as_deref().expect("handoff text"));
    assert_eq!(handoff["work_catalog"], catalog_uri(&session));
}

#[test]
fn public_work_catalog_pages_preserve_every_receipt_and_reject_cross_session_pins() {
    let session = open("3971102", false);
    let actions: Vec<_> = (0..20)
        .map(|_| json!({"action":{"kind":"pause","paused":false}}))
        .collect();
    commit(&session, &prepare(&session, json!(actions)));
    let before = fingerprint(&session);
    let (first, rows) = all_rows(&session);
    assert_eq!(rows.len(), 20);
    assert_eq!(first["complete_catalog"], false);
    assert!(first["next"].is_string());
    let steps: std::collections::BTreeSet<_> = rows
        .iter()
        .map(|row| row["step_id"].as_u64().expect("step"))
        .collect();
    assert_eq!(steps, (0..20).collect());
    let other = open("3971103", false);
    for uri in [
        first["canonical_uri"].as_str().expect("catalog pin"),
        rows[0]["action"].as_str().expect("detail pin"),
    ] {
        error(
            route(&other, &uri.replace(&session, &other)).expect_err("owner-bound pin"),
            "cursor_gap",
        );
    }
    assert_eq!(fingerprint(&session), before);
}

#[test]
fn public_work_pins_change_when_only_deferred_proof_is_cancelled() {
    let session = open("3971104", false);
    commit(&session, &prepare(&session, dig_actions(true)));
    let (first, rows) = all_rows(&session);
    let deferred = rows
        .iter()
        .find(|row| row["proof_state"] == "Prepared")
        .expect("deferred step");
    let anchor = first["current_anchor"].clone();
    let cancelled = parsed(&facade::fortress_cancel(
        Some(session.clone()),
        Some("stop_future_steps".to_owned()),
        Some("last_action".to_owned()),
    ));
    assert_eq!(cancelled["ok"], true, "{cancelled}");
    let (after, after_rows) = all_rows(&session);
    assert_eq!(
        after["current_anchor"], anchor,
        "cancelling a deferred setter has no world effect"
    );
    assert_ne!(after["catalog_digest"], first["catalog_digest"]);
    assert!(
        after_rows
            .iter()
            .any(|row| row["proof_state"] == "Cancelled")
    );
    assert!(
        after_rows
            .iter()
            .any(|row| row["physical_work"]["state"] == "active")
    );
    for stale in [
        first["canonical_uri"].as_str().expect("catalog"),
        deferred["action"].as_str().expect("detail"),
    ] {
        error(
            route(&session, stale).expect_err("same-anchor receipt changed"),
            "cursor_gap",
        );
    }
    let active = after_rows
        .iter()
        .find(|row| row["physical_work"]["state"] == "active")
        .expect("dig");
    let advanced = parsed(&facade::fortress_wait(Some(session.clone()), Some(1)));
    assert_eq!(advanced["ok"], true, "{advanced}");
    error(
        route(&session, active["action"].as_str().expect("detail"))
            .expect_err("current anchor changed"),
        "cursor_gap",
    );
}

#[test]
fn public_work_resources_require_current_full_authority_before_disclosing_existence() {
    let session = open("3971105", false);
    commit(&session, &prepare(&session, dig_actions(false)));
    let (_, rows) = all_rows(&session);
    let actual = rows[0]["action"].as_str().expect("detail");
    let missing = format!("df://session/{session}/action-{}", "f".repeat(64));
    let handle = resolve_session(Some(session.clone())).expect("session");
    let (grants, budget) = {
        let guard = handle.lock().expect("lock");
        (guard.grants.clone(), guard.budget)
    };
    for capability in [Capability::Observe, Capability::Query] {
        for restriction in 0..3 {
            {
                let mut guard = handle.lock().expect("lock");
                guard.grants = grants.clone();
                match restriction {
                    0 => guard.grants.retain(|grant| grant.capability != capability),
                    1 => {
                        for grant in guard
                            .grants
                            .iter_mut()
                            .filter(|grant| grant.capability == capability)
                        {
                            grant.scope.entity_ids = [EntityId::new(1)].into();
                        }
                    }
                    _ => {
                        for grant in guard
                            .grants
                            .iter_mut()
                            .filter(|grant| grant.capability == capability)
                        {
                            grant.expires_at_tick = Some(GameTick(0));
                        }
                    }
                }
            }
            let before = fingerprint(&session);
            let denied = error(
                route(&session, actual).expect_err("denied"),
                "capability_denied",
            );
            let absent = error(
                route(&session, &missing).expect_err("denied absent"),
                "capability_denied",
            );
            assert_eq!(denied, absent);
            assert!(denied["agent_turn"]["anchor"].is_null());
            assert_eq!(fingerprint(&session), before);
        }
    }
    for limit in 0..3 {
        {
            let mut guard = handle.lock().expect("lock");
            guard.grants = grants.clone();
            guard.budget = budget;
            match limit {
                0 => guard.budget.max_entities = 1,
                1 => guard.budget.max_bytes = 64,
                _ => guard.budget.max_output_tokens = 1,
            }
        }
        let before = fingerprint(&session);
        error(
            route(&session, actual).expect_err("complete source/output must fit"),
            "budget_exceeded",
        );
        assert_eq!(fingerprint(&session), before);
    }
    handle.lock().expect("lock").budget = budget;
    assert_eq!(read(&session, actual)["ok"], true);
}

#[test]
fn public_work_resources_keep_failed_proof_separate_from_running_physical_work() {
    let session = open("3971106", false);
    let mut planned = prepare(&session, dig_actions(false));
    // Seal a real short-deadline original so the reference runtime can fail
    // its temporal proof while the already-dispatched excavation keeps running.
    // Only fixture construction changes the proposed plan; normal commit and
    // wait still execute the real adapter and obligation runtime.
    let handle = resolve_session(Some(session.clone())).expect("session");
    {
        let mut guard = handle.lock().expect("lock");
        let pending = guard.pending.as_mut().expect("pending");
        let deadline = GameTick(pending.plan.anchor.tick.0 + 1);
        for step in &mut pending.plan.steps {
            if let Some(spec) = &mut step.obligation {
                spec.deadline_tick = deadline;
                spec.poll_interval_ticks = 1;
            }
        }
        pending.plan.digest = pending.plan.compute_digest();
        pending.plan.id = pending.plan.expected_id();
        pending.digest = pending.plan.digest.to_hex();
        planned["plan_digest"] = json!(pending.digest);
    }
    commit(&session, &planned);
    let waited = parsed(&facade::fortress_wait(Some(session.clone()), Some(2)));
    assert_eq!(waited["ok"], true, "{waited}");
    let before = fingerprint(&session);
    let (_, rows) = all_rows(&session);
    let failed = rows
        .iter()
        .find(|row| row["action_kind"] == "designate_dig")
        .expect("dig");
    assert_eq!(failed["proof_state"], "Failed");
    assert_eq!(failed["proof_terminal"], true);
    assert_eq!(failed["physical_work"]["state"], "active");
    assert_eq!(failed["physical_work"]["quiescent"], false);
    let detail = read(&session, failed["obligation"].as_str().expect("obligation"));
    assert_eq!(detail["obligation"]["monitor"]["state"], "failed");
    assert!(detail["obligation"]["next_observation_tick"].is_null());
    assert_eq!(detail["current_goal_truth"], "not_evaluated");
    assert_eq!(fingerprint(&session), before);
}

struct DurableDir(std::path::PathBuf);

impl DurableDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("dfmcp-work-resources-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        crate::server::simulate_durable_restart(Some(path.clone()));
        Self(path)
    }
}

impl Drop for DurableDir {
    fn drop(&mut self) {
        crate::server::simulate_durable_restart(None);
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn custody_files(path: &std::path::Path) -> BTreeMap<std::path::PathBuf, Digest32> {
    fn visit(path: &std::path::Path, files: &mut BTreeMap<std::path::PathBuf, Digest32>) {
        for entry in std::fs::read_dir(path).expect("custody directory") {
            let entry = entry.expect("entry");
            if entry.file_type().expect("type").is_dir() {
                visit(&entry.path(), files);
            } else {
                files.insert(
                    entry.path(),
                    Digest32::of_bytes(&std::fs::read(entry.path()).expect("custody bytes")),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(path, &mut files);
    files
}

#[test]
fn public_work_resources_inspect_recovery_without_handles_sampling_or_custody_writes() {
    let _serial = crate::server::serialized_durable_tests();
    let directory = DurableDir::new();
    let first = open("3971107", true);
    let digest = commit(&first, &prepare(&first, dig_actions(false)));
    let (_, live) = all_rows(&first);
    let live_dig = live
        .iter()
        .find(|row| row["action_kind"] == "designate_dig")
        .expect("dig");
    crate::server::simulate_durable_restart(Some(directory.0.clone()));
    let second = open("3971107", true);
    let before = fingerprint(&second);
    let files = custody_files(&directory.0);
    let (_, rows) = all_rows(&second);
    let recovered = rows
        .iter()
        .find(|row| row["plan_digest"] == digest && row["source"] == "recovered_original_step")
        .expect("recovered original");
    assert_eq!(
        rows.len(),
        2,
        "archived verified pause and active carried dig appear exactly once each"
    );
    assert_eq!(recovered["source"], "recovered_original_step");
    assert!(recovered["action_id"].is_null());
    assert_eq!(recovered["step_id"], live_dig["step_id"]);
    assert_eq!(
        recovered["plan_object_digest"],
        live_dig["plan_object_digest"]
    );
    assert_eq!(recovered["proof_state"], "dispatched");
    let detail = read(
        &second,
        recovered["obligation"]
            .as_str()
            .expect("recovered obligation"),
    );
    assert_eq!(detail["obligation"]["monitor"]["state"], "active");
    assert_eq!(
        detail["obligation"]["monitor"]["consecutive_stable_observations"],
        0
    );
    assert!(detail["last_monitor_observation_anchor"].is_null());
    assert_eq!(detail["recovery_anchor"], detail["current_anchor"]);
    assert_ne!(detail["proof_anchor"], detail["current_anchor"]);
    assert_eq!(detail["owned_by_current_session"], false);
    assert_eq!(detail["blind_retry_allowed"], false);
    assert_eq!(detail["dispatch_authority"], false);
    error(
        route(
            &second,
            &live_dig["action"]
                .as_str()
                .expect("old handle")
                .replace(&first, &second),
        )
        .expect_err("old source pin"),
        "cursor_gap",
    );
    assert!(
        route(&first, &catalog_uri(&first)).is_err(),
        "old owner cannot inspect through a saved URI"
    );
    assert_eq!(fingerprint(&second), before);
    assert_eq!(
        custody_files(&directory.0),
        files,
        "reads cannot republish durable custody"
    );
}

#[path = "archive_tests.rs"]
mod archived_history;
