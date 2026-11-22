//! Checkpoint discovery and exact recovery through the actual laboratory facade.
//!
//! These tests exercise the resource consumer and ordinary checkpoint/restore
//! calls. Source-bound metadata never grants restore authority or live evidence.

use super::*;
use crate::server::{
    fortress_checkpoint, fortress_restore, next_context, open_session_in_scenario,
    serialized_durable_tests, simulate_durable_restart, with_session,
};
use dfmcp_core::{EntityId, GameTick};
use std::collections::BTreeSet;
use std::time::Duration;

fn open(fortress: u64, tokens: u32, durable: bool) -> Value {
    let raw = open_session_in_scenario(
        Some(true),
        Some(fortress.to_string()),
        None,
        None,
        None,
        None,
        None,
        Some(tokens),
        None,
        Some("empty".to_owned()),
        Some(false),
        Some(durable),
    );
    let value: Value = serde_json::from_str(&raw).expect("open JSON");
    assert_eq!(value["ok"], true, "{value}");
    value
}

fn session_id(opened: &Value) -> String {
    opened["session_id"]
        .as_str()
        .expect("session id")
        .to_owned()
}

fn checkpoint(session: &str, label: &str) -> Value {
    let value: Value = serde_json::from_str(&fortress_checkpoint(
        Some(session.to_owned()),
        Some(label.to_owned()),
    ))
    .expect("checkpoint JSON");
    assert_eq!(value["ok"], true, "{value}");
    value
}

fn view(session: &str, view: &str) -> McpResult<Vec<ResourceContent>> {
    read(session, &format!("df://session/{session}/{view}"), view)
}

fn from_uri(session: &str, uri: &str) -> McpResult<Vec<ResourceContent>> {
    let prefix = format!("df://session/{session}/");
    read(
        session,
        uri,
        uri.strip_prefix(&prefix).expect("session URI"),
    )
}

fn payload(contents: Vec<ResourceContent>) -> Value {
    assert_eq!(contents.len(), 1);
    serde_json::from_str(contents[0].text.as_deref().expect("resource text"))
        .expect("resource JSON")
}

fn error_data(error: McpError, code: &str) -> Value {
    let data = error.data.expect("typed resource refusal");
    assert_eq!(data["error"]["code"], code);
    assert_eq!(data["error"]["effect_may_have_occurred"], false);
    assert_eq!(data["agent_turn"]["schema"], "dfmcp.agent_turn/1");
    assert_eq!(
        data["agent_turn"]["coverage"]["complete_domains"],
        json!([])
    );
    data
}

#[test]
fn checkpoint_addresses_are_closed_canonical_and_session_bound() {
    let id = CheckpointId::new(7);
    let hash = Digest32::of_bytes(b"checkpoint catalog");
    assert_eq!(
        address("checkpoints").expect("catalog"),
        Address::Catalog {
            digest: None,
            offset: 0
        }
    );
    assert_eq!(
        address(&format!("checkpoint-{id}")).expect("detail"),
        Address::Checkpoint(id)
    );
    assert_eq!(
        address(&format!("checkpoints-{hash}~17")).expect("page"),
        Address::Catalog {
            digest: Some(hash),
            offset: 17
        }
    );
    for bad in [
        "",
        "checkpoint",
        "checkpoints-",
        "checkpoint-0",
        "checkpoint-00000000000000000000000000000000",
        "checkpoint-0000000000000000000000000000000A",
        "checkpoint-00000000000000000000000000000007/extra",
    ] {
        assert!(address(bad).is_err(), "{bad}");
    }
    for suffix in [
        "",
        "~",
        "~+1",
        "~-1",
        "~01",
        "~1~2",
        "~0?x=1",
        "~184467440737095516160",
    ] {
        assert!(
            address(&format!("checkpoints-{hash}{suffix}")).is_err(),
            "{suffix}"
        );
    }
    assert!(address(&format!("checkpoints-{}~0", "A".repeat(64))).is_err());
    let opened = open(391_201, 4_096, false);
    let session = session_id(&opened);
    error_data(
        read(&session, "df://session/foreign/checkpoints", "checkpoints")
            .expect_err("URI cannot choose another session"),
        "invalid_request",
    );
}

#[test]
fn public_checkpoint_discovery_retains_labels_and_inspection_has_no_effects() {
    let opened = open(391_202, 16_384, false);
    let id = session_id(&opened);
    let uri = format!("df://session/{id}/checkpoints");
    assert_eq!(opened["checkpoint_catalog"], uri);
    let empty = payload(from_uri(&id, &uri).expect("empty catalog"));
    assert_eq!(empty["absence_proven"], true);
    assert_eq!(empty["complete_catalog"], true);
    assert_eq!(empty["coverage"]["absence_proof_scope"], json!([DOMAIN]));
    assert_eq!(
        empty["coverage"]["game_checkpoint_completeness"],
        "not_established"
    );

    let created = checkpoint(&id, "before the first dig");
    assert_eq!(created["checkpoint_catalog"], uri);
    let detail_uri = created["resource_uri"].as_str().expect("discovered detail");
    let session = resolve_session(Some(id.clone())).expect("session");
    let (anchor, transcript) = {
        let guard = session.lock().expect("lock");
        (
            guard.adapter.snapshot().anchor(),
            guard.adapter.transcript().clone(),
        )
    };
    let before_request = next_context(&mut session.lock().expect("lock"))
        .expect("request before inspection")
        .0;
    let first = from_uri(&id, &uri).expect("catalog");
    let again = from_uri(&id, &uri).expect("same catalog");
    assert_eq!(first[0].text, again[0].text);
    let catalog = payload(first);
    assert_eq!(catalog["total"], 1);
    assert_eq!(
        catalog["rows"][0]["checkpoint_id"],
        created["checkpoint_id"]
    );
    assert_eq!(catalog["rows"][0]["label"], "before the first dig");
    assert_eq!(catalog["rows"][0]["label_status"], "retained");
    assert_eq!(catalog["rows"][0]["uri"], detail_uri);
    assert_eq!(
        catalog["rows"][0]["content_digest"],
        created["content_digest"]
    );
    assert_eq!(catalog["rows"][0]["source_anchor"], created["anchor"]);
    assert_eq!(catalog["source_work"]["snapshot_contents_scanned"], false);
    let detail = payload(from_uri(&id, detail_uri).expect("detail"));
    assert_eq!(detail["schema"], "dfmcp.lab-checkpoint/1");
    assert_eq!(detail["label"], created["label"]);
    assert_eq!(
        detail["restore"]["arguments"]["checkpoint_id"],
        created["checkpoint_id"]
    );
    assert_eq!(detail["restore"]["authorization"], "not_evaluated");
    assert_eq!(detail["restore_authority_granted"], false);
    assert_eq!(detail["game_mutation_dispatched"], false);
    assert_eq!(detail["durability"]["durable"], Value::Null);
    assert_eq!(detail["agent_turn"]["anchor"], detail["source_anchor"]);
    let state_uri = detail["state"].as_str().expect("exact state");
    let state_view = state_uri.rsplit('/').next().expect("state view");
    let state = payload(
        crate::resources::state::read(&id, state_uri, state_view).expect("checkpoint state"),
    );
    assert_eq!(state["anchor"], created["anchor"]);
    let after_request = next_context(&mut session.lock().expect("lock"))
        .expect("request after inspection")
        .0;
    assert_eq!(after_request, before_request + 1);
    let guard = session.lock().expect("lock");
    assert_eq!(guard.adapter.snapshot().anchor(), anchor);
    assert_eq!(guard.adapter.transcript(), &transcript);
    assert_eq!(guard.adapter.checkpoint_count(), 1);
}

#[test]
fn historical_unknown_labels_stay_unknown_and_empty_retained_labels_stay_exact() {
    let id = session_id(&open(391_203, 16_384, false));
    let session = resolve_session(Some(id.clone())).expect("session");
    {
        let mut guard = session.lock().expect("lock");
        let source = guard.adapter.snapshot().clone();
        guard
            .adapter
            .adopt_checkpoint(CheckpointId::new(2), source.clone())
            .expect("unlabeled historical checkpoint");
        guard
            .adapter
            .adopt_checkpoint_labeled(CheckpointId::new(1), source, "")
            .expect("exact historical empty label");
    }
    let catalog = payload(view(&id, "checkpoints").expect("catalog"));
    assert_eq!(
        catalog["rows"][0]["checkpoint_id"],
        CheckpointId::new(1).to_string()
    );
    assert_eq!(catalog["rows"][0]["label"], "");
    assert_eq!(catalog["rows"][0]["label_status"], "retained");
    assert_eq!(
        catalog["rows"][1]["checkpoint_id"],
        CheckpointId::new(2).to_string()
    );
    assert_eq!(catalog["rows"][1]["label"], Value::Null);
    assert_eq!(catalog["rows"][1]["label_status"], "unavailable");
    assert_eq!(catalog["absence_proven"], false);
    assert_eq!(catalog["coverage"]["absence_proof_scope"], json!([]));
}

#[test]
fn checkpoint_pages_keep_complete_identities_and_reject_changed_catalog_or_anchor() {
    let id = session_id(&open(391_204, 4_096, false));
    let mut expected = BTreeSet::new();
    for index in 0..31 {
        let created = checkpoint(&id, &format!("{index:02}-{}", "\"\\é".repeat(32)));
        expected.insert(created["checkpoint_id"].as_str().expect("id").to_owned());
    }
    let first = payload(view(&id, "checkpoints").expect("first page"));
    let mut uri = first["canonical_uri"]
        .as_str()
        .expect("canonical page")
        .to_owned();
    let old_next = first["next"]
        .as_str()
        .expect("bounded continuation")
        .to_owned();
    let digest = first["catalog_digest"].clone();
    let mut seen = BTreeSet::new();
    let mut pages = 0;
    loop {
        let contents = from_uri(&id, &uri).expect("pinned page");
        let text = contents[0].text.as_deref().expect("text");
        assert!(super::super::task_resource_bytes(&uri, text) <= 4_096 * 4);
        let page = payload(contents);
        assert_eq!(page["catalog_digest"], digest);
        assert_eq!(page["complete_catalog"], false);
        assert_eq!(page["absence_proven"], false);
        for row in page["rows"].as_array().expect("rows") {
            assert!(seen.insert(row["checkpoint_id"].as_str().expect("id").to_owned()));
            assert!(row["source_anchor"]["state_hash"].is_string());
            assert!(row["source_anchor_v2"]["digest"].is_string());
        }
        pages += 1;
        assert!(pages <= 31);
        if let Some(next) = page["next"].as_str() {
            uri = next.to_owned();
        } else {
            assert_eq!(page["complete"], true);
            break;
        }
    }
    assert_eq!(seen, expected);
    assert!(pages > 1);

    checkpoint(&id, "a later checkpoint at the same world anchor");
    let gap = error_data(
        from_uri(&id, &old_next).expect_err("catalog changed"),
        "cursor_gap",
    );
    assert_eq!(gap["agent_turn"]["continuity"]["status"], "gap");
    let fresh = payload(view(&id, "checkpoints").expect("fresh catalog"));
    let fresh_uri = fresh["canonical_uri"]
        .as_str()
        .expect("fresh URI")
        .to_owned();
    let session = resolve_session(Some(id.clone())).expect("session");
    with_session(
        &session,
        || panic!("session unavailable"),
        |guard| {
            guard
                .adapter
                .advance_ticks(1)
                .expect("advance current world");
        },
    );
    error_data(
        from_uri(&id, &fresh_uri).expect_err("anchor changed"),
        "cursor_gap",
    );
}

#[test]
fn checkpoint_reads_reauthorize_before_existence_and_never_grant_restore() {
    for mode in ["revoked", "expired", "entity-scoped"] {
        let id = session_id(&open(391_205, 4_096, false));
        let created = checkpoint(&id, "private checkpoint");
        let known = created["resource_uri"]
            .as_str()
            .expect("known URI")
            .to_owned();
        let session = resolve_session(Some(id.clone())).expect("session");
        {
            let mut guard = session.lock().expect("lock");
            if mode == "revoked" {
                guard
                    .grants
                    .retain(|grant| grant.capability != Capability::Observe);
            } else {
                for grant in &mut guard.grants {
                    if grant.capability == Capability::Observe {
                        if mode == "expired" {
                            grant.expires_at_tick = Some(GameTick(0));
                        } else {
                            grant.scope.entity_ids.insert(EntityId::new(1));
                        }
                    }
                }
            }
        }
        for uri in [
            format!("df://session/{id}/checkpoints"),
            known,
            format!("df://session/{id}/checkpoint-ffffffffffffffffffffffffffffffff"),
        ] {
            let denied = error_data(from_uri(&id, &uri).expect_err(mode), "capability_denied");
            assert_eq!(denied["agent_turn"]["anchor"], Value::Null);
            assert!(!denied.to_string().contains("private checkpoint"));
        }
    }
    let id = session_id(&open(391_206, 4_096, false));
    let created = checkpoint(&id, "inspect without restoring");
    let session = resolve_session(Some(id.clone())).expect("session");
    session
        .lock()
        .expect("lock")
        .grants
        .retain(|grant| grant.capability == Capability::Observe);
    let uri = created["resource_uri"].as_str().expect("detail URI");
    let detail = payload(from_uri(&id, uri).expect("read-only metadata"));
    assert_eq!(detail["restore_authority_granted"], false);
    let restored: Value = serde_json::from_str(&fortress_restore(
        Some(id),
        created["checkpoint_id"].as_str().expect("id").to_owned(),
    ))
    .expect("restore JSON");
    assert_eq!(restored["error"]["code"], "capability_denied");
}

#[test]
fn checkpoint_metadata_source_output_and_wall_work_share_explicit_budgets() {
    let id = session_id(&open(391_207, 16_384, false));
    let first = checkpoint(&id, "first");
    checkpoint(&id, "second");
    let session = resolve_session(Some(id.clone())).expect("session");
    let budget = session.lock().expect("lock").budget;
    session.lock().expect("lock").budget.max_entities = 1;
    error_data(
        view(&id, "checkpoints").expect_err("complete catalog source bound"),
        "budget_exceeded",
    );
    let detail_uri = first["resource_uri"].as_str().expect("detail");
    assert!(
        from_uri(&id, detail_uri).is_ok(),
        "one selected metadata row fits"
    );

    session.lock().expect("lock").budget = budget;
    session.lock().expect("lock").budget.max_bytes = 1;
    error_data(
        view(&id, "checkpoints").expect_err("metadata bytes"),
        "budget_exceeded",
    );
    session.lock().expect("lock").budget = budget;
    session.lock().expect("lock").budget.max_output_tokens = 1;
    error_data(
        view(&id, "checkpoints").expect_err("indivisible output"),
        "budget_exceeded",
    );
    session.lock().expect("lock").budget = budget;

    let detail_view = detail_uri.rsplit('/').next().expect("detail view");
    for expire_at in 0..3 {
        let mut calls = 0;
        let result = read_with_elapsed(&id, detail_uri, detail_view, || {
            let expired = calls >= expire_at;
            calls += 1;
            Duration::from_millis(if expired { budget.max_wall_millis } else { 0 })
        });
        error_data(
            result.expect_err("shared wall allowance"),
            "budget_exceeded",
        );
        assert_eq!(calls, expire_at + 1);
    }
    assert!(read_with_elapsed(&id, detail_uri, detail_view, || Duration::ZERO).is_ok());
}

#[test]
fn durable_reopen_discovers_original_checkpoint_label_world_and_restore_without_old_ids() {
    struct DurableDirectory(std::path::PathBuf);
    impl Drop for DurableDirectory {
        fn drop(&mut self) {
            simulate_durable_restart(None);
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let _serial = serialized_durable_tests();
    let path = std::env::temp_dir().join(format!(
        "dfmcp-checkpoint-resource-recovery-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    simulate_durable_restart(Some(path.clone()));
    let _directory = DurableDirectory(path.clone());
    let original_session = session_id(&open(391_208, 16_384, true));
    let created = checkpoint(&original_session, "before first workshop");
    assert_eq!(created["durable"], true, "{created}");
    let original_uri = created["resource_uri"]
        .as_str()
        .expect("old URI")
        .to_owned();
    let source_anchor = created["anchor"].clone();
    let original_digest = created["content_digest"].clone();

    simulate_durable_restart(Some(path));
    let reopened = open(391_208, 16_384, true);
    let current_session = session_id(&reopened);
    assert_ne!(current_session, original_session);
    assert_ne!(reopened["anchor"], source_anchor);
    let catalog_uri = reopened["checkpoint_catalog"]
        .as_str()
        .expect("cold discovery URI");
    let catalog = payload(from_uri(&current_session, catalog_uri).expect("recovered catalog"));
    assert_eq!(catalog["total"], 1);
    let row = &catalog["rows"][0];
    assert_eq!(row["checkpoint_id"], created["checkpoint_id"]);
    assert_eq!(row["label"], "before first workshop");
    assert_eq!(row["label_status"], "retained");
    assert_eq!(row["content_digest"], original_digest);
    assert_eq!(row["source_anchor"], source_anchor);
    let detail_uri = row["uri"].as_str().expect("recovered detail");
    let detail = payload(from_uri(&current_session, detail_uri).expect("recovered metadata"));
    assert_eq!(detail["historical"], true);
    assert_eq!(detail["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(detail["durability"]["durable"], Value::Null);

    // This source predates the new session's observation history. The exact
    // checkpoint world remains inspectable without rewriting its old anchor.
    let state_uri = detail["state"].as_str().expect("original state");
    let state_view = state_uri.rsplit('/').next().expect("state view");
    let state = payload(
        crate::resources::state::read(&current_session, state_uri, state_view)
            .expect("original checkpoint world after process restart"),
    );
    assert_eq!(state["anchor"], source_anchor);
    assert_eq!(state["historical"], true);
    assert_eq!(state["current_anchor"], reopened["anchor"]);

    error_data(
        from_uri(&original_session, &original_uri).expect_err("old durable owner"),
        "conflict",
    );
    let restored: Value = serde_json::from_str(&fortress_restore(
        Some(current_session.clone()),
        row["checkpoint_id"]
            .as_str()
            .expect("discovered checkpoint id")
            .to_owned(),
    ))
    .expect("restore JSON");
    assert_eq!(restored["ok"], true, "{restored}");
    assert_eq!(restored["content_digest"], original_digest);
    assert!(
        restored["restored_anchor"]["epoch"]
            .as_u64()
            .expect("restored epoch")
            > reopened["anchor"]["epoch"]
                .as_u64()
                .expect("recovered epoch")
    );
    let retained =
        payload(from_uri(&current_session, detail_uri).expect("original remains retained"));
    assert_eq!(retained["source_anchor"], source_anchor);
    assert_eq!(retained["label"], "before first workshop");
}
