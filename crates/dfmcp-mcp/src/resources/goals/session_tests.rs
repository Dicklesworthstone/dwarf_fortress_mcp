//! Public facade and registered resource-handler regressions; no substitute
//! planner, adapter, authority check, or response renderer is used here.

use super::*;
use crate::agent_facade as facade;
use crate::resources::SessionViewResource;
use crate::server::{
    Objective, read_context, resolve_session, resource_replay_bundle, with_session_read,
};
use dfmcp_core::{Capability, EntityId, GameTick};
use fastmcp_rust::prelude::{McpContext, McpError, McpResult, ResourceContent, ResourceHandler};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};

fn parsed(raw: &str) -> Value {
    serde_json::from_str(raw).expect("public JSON")
}

fn open(selector: &str, durable: bool) -> String {
    let value = parsed(&facade::fortress_open_session(
        Some(true), Some(selector.to_owned()), None, None, None, None, None,
        Some(4_096), None, Some("empty".to_owned()), Some(false), Some(durable),
    ));
    assert_eq!(value["ok"], true, "{value}");
    value["session_id"].as_str().expect("session").to_owned()
}

fn prepare(session: &str, raw: Option<String>) -> Value {
    let value = parsed(&facade::fortress_plan(
        Some(session.to_owned()), Some("Exact original for Urïst".to_owned()),
        raw.is_none().then_some(false), raw, None,
    ));
    assert_eq!(value["ok"], true, "{value}");
    value
}

fn commit(session: &str, prepared: &Value) -> String {
    let digest = prepared["plan_digest"].as_str().expect("plan digest").to_owned();
    let value = parsed(&facade::fortress_commit(Some(session.to_owned()), digest.clone()));
    assert_eq!(value["ok"], true, "{value}");
    digest
}

fn route(session: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let view = uri.rsplit('/').next().expect("view");
    let params: HashMap<String, String> = [
        ("session_id".to_owned(), session.to_owned()), ("view".to_owned(), view.to_owned()),
    ].into();
    let runtime = asupersync::runtime::RuntimeBuilder::current_thread()
        .build().expect("owned runtime");
    runtime.block_on(async {
        let context = McpContext::new(asupersync::Cx::current().expect("runtime context"), 1);
        SessionViewResource.read_with_uri(&context, uri, &params)
    })
}

fn read(session: &str, uri: &str) -> Value {
    let contents = route(session, uri).expect("authorized resource");
    assert_eq!(contents.len(), 1);
    let text = contents[0].text.as_deref().expect("resource text");
    let maximum_id: Value = serde_json::from_str(&"9".repeat(4_096)).expect("exact numeric ID");
    let wire = json!({"jsonrpc":"2.0", "id":maximum_id, "result":{
        "contents":[{"uri":uri,"mimeType":"application/json","text":text}],
        "ttlMs":0,"cacheScope":"private"}}).to_string();
    assert!(wire.len() <= 4_096 * 4,
        "complete escaped result and maximum numeric ID must fit the default token budget");
    parsed(text)
}

fn discovery(session: &str, digest: &str) -> String {
    format!("df://session/{session}/goal-{digest}")
}

fn assert_error(error: McpError, code: &str) -> Value {
    let value = error.data.expect("structured resource error");
    assert_eq!(value["error"]["code"], code, "{value}");
    assert_eq!(value["error"]["effect_may_have_occurred"], false);
    assert_eq!(value["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert_eq!(value["agent_turn"]["coverage"]["absence_proof_scope"], json!([]));
    value
}

fn fingerprint(session: &str) -> Value {
    let handle = resolve_session(Some(session.to_owned())).expect("session");
    with_session_read(&handle, |error| panic!("{error}"), |guard| json!({
        "request_id": read_context(guard).request_id.to_string(),
        "anchor": crate::server::anchor_json(&guard.adapter.snapshot().anchor()),
        "history": Digest32::of_bytes(format!("{:?}", guard.objectives).as_bytes()).to_hex(),
        "pending": guard.pending.as_ref().map(|pending| pending.digest.clone()),
        "replay": resource_replay_bundle(guard),
        "open_actions": guard.open_actions.iter().map(ToString::to_string).collect::<Vec<_>>(),
        "commits": guard.commit_receipts,
    }))
}

fn reassemble(session: &str, manifest: &Value) -> Value {
    let mut uri = manifest["source"]["uri"].as_str().expect("source URI").to_owned();
    let mut bytes = Vec::new();
    let mut pages = 0;
    loop {
        let page = read(session, &uri);
        assert_eq!(page["source_digest"], manifest["source_digest"]);
        assert_eq!(page["offset"].as_u64(), Some(bytes.len() as u64));
        assert_eq!(page["current_goal_truth"], "not_evaluated");
        assert_eq!(page["historical_proof_updated"], false);
        let data = page["data_hex"].as_str().expect("hex source page");
        assert_eq!(page["returned_bytes"].as_u64(), Some((data.len() / 2) as u64));
        for offset in (0..data.len()).step_by(2) {
            bytes.push(u8::from_str_radix(&data[offset..offset + 2], 16).expect("byte"));
        }
        pages += 1;
        if let Some(next) = page["next"].as_str() {
            assert_eq!(page["complete"], false);
            uri = next.to_owned();
        } else {
            assert_eq!(page["complete"], true);
            break;
        }
        assert!(pages < 128, "bounded original source traversal");
    }
    assert_eq!(bytes.len() as u64, manifest["source"]["bytes"].as_u64().expect("length"));
    assert_eq!(Digest32::of_bytes(&bytes).to_hex(), manifest["source_digest"].as_str().expect("digest"));
    serde_json::from_slice(&bytes).expect("complete original JSON artifact")
}

#[test]
fn public_goal_resources_reassemble_exact_requests_and_preserve_historical_custody() {
    let session = open("3971001", false);
    let raw = format!("[{}{{\"action\":{{\"kind\":\"pause\",\"paused\":false}}}}]", " \n".repeat(5_000));
    let prepared = prepare(&session, Some(raw.clone()));
    let digest = commit(&session, &prepared);
    let pending = parsed(&facade::fortress_plan(
        Some(session.clone()),
        Some("A separate reviewed pause remains pending".to_owned()),
        Some(true),
        None,
        None,
    ));
    assert_eq!(pending["ok"], true, "{pending}");
    let before = fingerprint(&session);
    assert_eq!(before["pending"], pending["plan_digest"]);
    let manifest = read(&session, &discovery(&session, &digest));
    assert_eq!(manifest["schema"], "dfmcp.lab-goal-manifest/1");
    assert_eq!(manifest["historical_achievement"], "verified");
    assert_eq!(manifest["first_satisfied_anchor"], manifest["recorded_first_satisfied_anchor"]);
    assert_eq!(manifest["current_goal_truth"], "not_evaluated");
    assert_eq!(manifest["current_semantic_project_truth"], "not_evaluated");
    assert_eq!(manifest["dispatch_authority"], false);
    let exact = manifest["canonical_uri"].as_str().expect("exact detail");
    assert!(exact.contains(manifest["source_digest"].as_str().expect("source pin")));
    assert_eq!(read(&session, exact)["source_digest"], manifest["source_digest"]);
    let source = reassemble(&session, &manifest);
    assert_eq!(source["original_source"]["request_json"], raw);
    assert_eq!(source["original_source"]["kind"], "actions");
    assert_eq!(source["summary"], "Exact original for Urïst");
    assert_eq!(source["sealed_anchor"], manifest["sealed_anchor"]);
    let handle = resolve_session(Some(session.clone())).expect("session");
    let terminal = with_session_read(&handle, |error| panic!("{error}"), |guard| {
        crate::lab_world::predicate_json(&goal(guard, Digest32::from_hex(&digest).expect("digest"))
            .and_then(|entry| entry.plan).expect("original plan").terminal_condition)
    });
    assert_eq!(source["terminal_condition"], terminal);
    assert_eq!(source["plan_object_digest"], manifest["plan"]["object_digest"]);
    assert_eq!(fingerprint(&session), before,
        "resources cannot certify history, consume semantic IDs, replace pending plans or record calls");
    let original_budget = handle.lock().expect("lock").budget;
    handle.lock().expect("lock").budget.max_bytes = raw.len() as u64 - 1;
    assert_error(
        route(&session, manifest["source"]["uri"].as_str().expect("source"))
            .expect_err("a small output page cannot bypass complete raw source admission"),
        "budget_exceeded",
    );
    assert_eq!(fingerprint(&session), before);
    handle.lock().expect("lock").budget = original_budget;
}

#[test]
fn public_goal_resources_refuse_stale_source_pins_and_never_infer_pending_goals() {
    let session = open("3971002", false);
    let prepared = prepare(&session, None);
    let digest = prepared["plan_digest"].as_str().expect("digest");
    assert_error(route(&session, &discovery(&session, digest)).expect_err("pending is not a goal"), "stale_anchor");
    commit(&session, &prepared);
    let manifest = read(&session, &discovery(&session, digest));
    let handle = resolve_session(Some(session.clone())).expect("session");
    {
        let mut guard = handle.lock().expect("lock");
        let original = goal(&guard, Digest32::from_hex(digest).expect("digest"))
            .and_then(|entry| entry.plan).expect("exact original").clone();
        // Deliberate semantic-digest alias: source custody must distinguish
        // another retained request even if its prepared-plan digest repeats.
        guard.objectives = vec![Objective::committed(&original, PlanSource::Pause {
            summary: "Different original source".to_owned(), paused_target: false,
        }, guard.session_id)];
    }
    for uri in [manifest["canonical_uri"].as_str().expect("manifest"),
        manifest["source"]["uri"].as_str().expect("source")] {
        assert_error(route(&session, uri).expect_err("old pin"), "stale_anchor");
    }
    let before = fingerprint(&session);
    let replacement = read(&session, &discovery(&session, digest));
    assert_ne!(replacement["source_digest"], manifest["source_digest"]);
    assert_eq!(replacement["historical_achievement"], "not_recorded");
    assert!(replacement["first_satisfied_anchor"].is_null());
    assert_eq!(fingerprint(&session), before,
        "even an already true original cannot acquire its first achievement through resource reads");
    handle.lock().expect("lock").objectives.clear();
    assert_error(route(&session, replacement["canonical_uri"].as_str().expect("pin"))
        .expect_err("collected goal"), "stale_anchor");
}

#[test]
fn public_goal_resources_authorize_before_existence_and_enforce_complete_work_budgets() {
    let session = open("3971003", false);
    let prepared = prepare(&session, None);
    let digest = commit(&session, &prepared);
    let uri = discovery(&session, &digest);
    let missing = discovery(&session, &"f".repeat(64));
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
                    1 => for grant in guard.grants.iter_mut().filter(|grant| grant.capability == capability) {
                        grant.scope.entity_ids = [EntityId::new(1)].into();
                    },
                    _ => for grant in guard.grants.iter_mut().filter(|grant| grant.capability == capability) {
                        grant.expires_at_tick = Some(GameTick(0));
                    },
                }
            }
            let before = fingerprint(&session);
            let denied = assert_error(route(&session, &uri).expect_err("denied"), "capability_denied");
            let absent = assert_error(route(&session, &missing).expect_err("denied missing"), "capability_denied");
            assert_eq!(denied, absent, "unauthorized errors cannot disclose original existence");
            assert!(denied["agent_turn"]["anchor"].is_null());
            assert_eq!(fingerprint(&session), before);
        }
    }
    for limit in 0..4 {
        {
            let mut guard = handle.lock().expect("lock");
            guard.grants = grants.clone();
            guard.budget = budget;
            match limit {
                0 => guard.budget.max_entities = 0,
                1 => guard.budget.max_bytes = 64,
                2 => guard.budget.max_output_tokens = 1,
                _ => guard.budget.max_wall_millis = 0,
            }
        }
        let before = fingerprint(&session);
        assert_error(route(&session, &uri).expect_err("budget denied"), "budget_exceeded");
        assert_eq!(fingerprint(&session), before);
    }
    handle.lock().expect("lock").budget = budget;
    assert_eq!(read(&session, &uri)["ok"], true);
    let wrong_case = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    assert_error(route(wrong_case, &discovery(wrong_case, &digest))
        .expect_err("noncanonical session"), "invalid_request");
}

struct DurableDir(std::path::PathBuf);

impl DurableDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("dfmcp-goal-resources-{}", std::process::id()));
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
                files.insert(entry.path(), Digest32::of_bytes(&std::fs::read(entry.path()).expect("custody bytes")));
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(path, &mut files);
    files
}

#[test]
fn public_goal_resources_preserve_restored_history_and_recovered_source_without_writes() {
    let _serial = crate::server::serialized_durable_tests();
    let directory = DurableDir::new();
    let first = open("3971004", true);
    let checkpoint = parsed(&facade::fortress_checkpoint(Some(first.clone()), None));
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let digest = commit(&first, &prepare(&first, None));
    let before_restore = read(&first, &discovery(&first, &digest));
    let restored = parsed(&facade::fortress_restore(Some(first.clone()),
        checkpoint["checkpoint_id"].as_str().expect("checkpoint").to_owned()));
    assert_eq!(restored["ok"], true, "{restored}");
    let after_restore = read(&first, before_restore["canonical_uri"].as_str().expect("exact original"));
    assert_eq!(after_restore["source_digest"], before_restore["source_digest"]);
    assert_eq!(after_restore["first_satisfied_anchor"], before_restore["first_satisfied_anchor"]);
    assert!(!after_restore["restore_abandoned_anchor"].is_null());
    assert_eq!(after_restore["current_goal_truth"], "not_evaluated");
    crate::server::simulate_durable_restart(Some(directory.0.clone()));
    let second = open("3971004", true);
    let before = fingerprint(&second);
    let files = custody_files(&directory.0);
    let recovered_uri = before_restore["canonical_uri"].as_str().expect("exact source").replace(&first, &second);
    let recovered = read(&second, &recovered_uri);
    assert_eq!(recovered["source_digest"], before_restore["source_digest"]);
    assert_eq!(recovered["first_satisfied_anchor"], before_restore["first_satisfied_anchor"]);
    assert_eq!(recovered["restore_abandoned_anchor"], after_restore["restore_abandoned_anchor"]);
    assert_eq!(recovered["owned_by_current_session"], false);
    reassemble(&second, &recovered);
    assert_eq!(fingerprint(&second), before);
    assert_eq!(custody_files(&directory.0), files, "resource reads cannot persist custody");
}

#[test]
fn public_goal_resources_keep_unverifiable_recovered_achievement_unknown() {
    let _serial = crate::server::serialized_durable_tests();
    let directory = DurableDir::new();
    let session = open("3971005", true);
    let digest = commit(&session, &prepare(&session, None));
    let handle = resolve_session(Some(session.clone())).expect("session");
    let fortress = handle.lock().expect("lock").fortress_id;
    let wanted = Digest32::from_hex(&digest).expect("digest");
    let invalid = crate::server::with_durable_store(|store| {
        let mut retained = store.objective(fortress, wanted).cloned().expect("retained goal");
        // The original source is paused, so it cannot prove this goal's
        // requested unpaused state despite carrying an exact valid world hash.
        retained.first_satisfied_anchor = Some(store.load_snapshot(retained.sealed_state_hash)?.anchor());
        let head = store.head(fortress).expect("head");
        let persisted = store.load_snapshot(head.anchor.state_hash)?;
        let recovered = dfmcp_lab::MemoryAdapter::recovered(persisted.clone())?;
        let lineage = crate::server::goal_continuation::RecoveredGoalLineage::verify(
            &persisted, head.anchor, recovered.snapshot().anchor(),
        )?;
        Ok(crate::server::objectives::recover_objective(store, &retained, &lineage))
    }).expect("invalid historical fixture");
    handle.lock().expect("lock").objectives = vec![invalid];
    let before = fingerprint(&session);
    let files = custody_files(&directory.0);
    let manifest = read(&session, &discovery(&session, &digest));
    assert_eq!(manifest["original_source_verification"], "unavailable");
    assert_eq!(manifest["historical_achievement"], "unverifiable");
    assert!(manifest["first_satisfied_anchor"].is_null());
    assert!(!manifest["recorded_first_satisfied_anchor"].is_null());
    assert!(!manifest["verification_error"].is_null());
    assert_eq!(manifest["current_goal_truth"], "not_evaluated");
    assert!(manifest["plan"].is_null());
    let source = reassemble(&session, &manifest);
    assert_eq!(source["exact_plan_available"], false);
    assert!(source["terminal_condition"].is_null());
    assert_eq!(fingerprint(&session), before);
    assert_eq!(custody_files(&directory.0), files);
}
