//! Completed original step maps survive retirement independently of live
//! handles, current physical work and renewed pursuit eligibility.

use super::*;

fn completed(session: &str) -> String {
    let digest = commit(session, &prepare(session, dig_actions(false)));
    let waited = parsed(&facade::fortress_wait_for_plan(
        Some(session.to_owned()),
        Some(500),
        Some(digest.clone()),
    ));
    assert_eq!(waited["ok"], true, "{waited}");
    assert_eq!(
        waited["selected_plan_wait"]["stop_reason"], "complete",
        "{waited}"
    );
    // Live completion publishes a terminal, physically drained frontier and
    // retires its commit in persist_durable_head before the tool returns.
    // Establish that fixture prerequisite explicitly: recovery must read an
    // already-archived map, not retire an unfinished live map on our behalf.
    let handle = resolve_session(Some(session.to_owned())).expect("session");
    let fortress = handle.lock().expect("lock").fortress_id;
    let original = Digest32::from_hex(&digest).expect("digest");
    crate::server::with_durable_store(|store| {
        assert!(
            store.commit(fortress, original).is_none(),
            "live completion retired its commit"
        );
        let archived = store
            .retired_commit(fortress, original)
            .expect("already-retired original");
        assert_eq!(archived.steps.len(), 2);
        assert_eq!(archived.step_anchors.len(), 2);
        assert!(archived.steps.values().all(|state| state == "verified"));
        Ok(())
    })
    .expect("exact retired fixture custody");
    digest
}

fn explain_steps(session: &str, digest: &str) -> Vec<Value> {
    let manifest = parsed(&facade::fortress_explain(
        Some(session.to_owned()),
        Some(format!("goal:{digest}")),
    ));
    assert_eq!(manifest["ok"], true, "{manifest}");
    let selector = manifest["original_steps"]["arguments"]["entity_id"]
        .as_str()
        .expect("step selector");
    let page = parsed(&facade::fortress_explain(
        Some(session.to_owned()),
        Some(selector.to_owned()),
    ));
    assert_eq!(page["ok"], true, "{page}");
    assert_eq!(page["section"], "steps");
    assert_eq!(page["complete_domain"], true);
    page["rows"].as_array().expect("steps").clone()
}

fn assert_archived(session: &str, digest: &str) -> Vec<Value> {
    let (_, rows) = all_rows(session);
    let rows: Vec<_> = rows
        .into_iter()
        .filter(|row| row["plan_digest"] == digest)
        .collect();
    assert_eq!(
        rows.len(),
        2,
        "complete archived original, without manufactured handles"
    );
    for row in &rows {
        assert_eq!(row["source"], "recovered_archived_disposition");
        assert_eq!(row["historical_only"], true);
        assert_eq!(row["proof_state"], "verified");
        assert!(row["action_id"].is_null());
        assert_eq!(row["physical_work"]["state"], "not_resampled");
        assert!(row["physical_work"]["quiescent"].is_null());
        assert_eq!(row["physical_work"]["current"], false);
        let detail = read(session, row["action"].as_str().expect("action detail"));
        assert_eq!(
            detail["archived_disposition"]["recorded_anchor"],
            row["proof_anchor"]
        );
        assert_eq!(
            detail["archived_disposition"]["action_record_origin"],
            "archived_retired_commit"
        );
        assert_eq!(
            detail["archived_disposition"]["dispatch_handles_restored"],
            false
        );
        assert_eq!(detail["current_goal_truth"], "not_evaluated");
        assert_eq!(detail["dispatch_authority"], false);
    }
    let explained = explain_steps(session, digest);
    assert_eq!(explained.len(), 2);
    for row in &explained {
        assert_eq!(row["dispatch_status"], "archived_historical_receipt");
        assert_eq!(row["recovered_evidence"]["recorded_state"], "verified");
        assert!(row["receipt"].is_null());
        assert_eq!(row["physical_work"]["state"], "not_resampled");
        assert!(row["physical_work"]["quiescent"].is_null());
    }
    rows
}

#[test]
fn public_work_resources_retain_completed_archives_through_compaction_and_repeated_reopen() {
    let _serial = crate::server::serialized_durable_tests();
    let directory = DurableDir::new();
    let first = open("3971121", true);
    let digest = completed(&first);
    let (_, live) = all_rows(&first);
    let original_object = live[0]["plan_object_digest"].clone();
    for _ in 0..2 {
        crate::server::simulate_durable_restart(Some(directory.0.clone()));
        {
            let mut store = dfmcp_lab::durable::DurableLabStore::open(&directory.0).expect("store");
            store.compact().expect("compact exact terminal maps");
        }
        let session = open("3971121", true);
        let before = fingerprint(&session);
        let files = custody_files(&directory.0);
        let rows = assert_archived(&session, &digest);
        assert!(
            rows.iter()
                .all(|row| row["plan_object_digest"] == original_object)
        );
        let handle = resolve_session(Some(session.clone())).expect("session");
        {
            let guard = handle.lock().expect("lock");
            let plan = goal(&guard, Digest32::from_hex(&digest).expect("digest"))
                .and_then(|goal| goal.plan)
                .expect("original");
            let (_, seal) = dfmcp_intent::plan_archive::encode(plan, Default::default())
                .expect("admitted original");
            let wrong = dfmcp_intent::plan_archive::PlanArchiveSeal {
                object_digest: Digest32::of_bytes(b"another typed original"),
                ..seal
            };
            let denied = action_history(&guard, plan.digest, Some(wrong))
                .expect("recovered history")
                .expect_err("exact object binding");
            assert_eq!(denied.code, dfmcp_core::ErrorCode::CorruptLedger);
        }
        assert_eq!(
            fingerprint(&session),
            before,
            "passive history never creates observations or receipts"
        );
        assert_eq!(
            custody_files(&directory.0),
            files,
            "history reads never republish custody"
        );
        let grants = handle.lock().expect("lock").grants.clone();
        handle
            .lock()
            .expect("lock")
            .grants
            .retain(|grant| grant.capability != Capability::Query);
        let uri = rows[0]["action"].as_str().expect("detail");
        let denied = error(
            route(&session, uri).expect_err("historical URI needs current Query"),
            "capability_denied",
        );
        assert!(denied["agent_turn"]["anchor"].is_null());
        handle.lock().expect("lock").grants = grants;
    }
}

#[test]
fn public_work_resources_keep_archive_history_after_restore_without_reviving_the_pursuit() {
    let _serial = crate::server::serialized_durable_tests();
    let directory = DurableDir::new();
    let first = open("3971122", true);
    let checkpoint = parsed(&facade::fortress_checkpoint(Some(first.clone()), None));
    assert_eq!(checkpoint["ok"], true, "{checkpoint}");
    let digest = completed(&first);
    crate::server::simulate_durable_restart(Some(directory.0.clone()));
    let session = open("3971122", true);
    let archived = assert_archived(&session, &digest);
    let restored = parsed(&facade::fortress_restore(
        Some(session.clone()),
        checkpoint["checkpoint_id"]
            .as_str()
            .expect("checkpoint")
            .to_owned(),
    ));
    assert_eq!(restored["ok"], true, "{restored}");
    let before = fingerprint(&session);
    let files = custody_files(&directory.0);
    error(
        route(&session, archived[0]["action"].as_str().expect("prior pin"))
            .expect_err("restore changes current anchor"),
        "cursor_gap",
    );
    let current = assert_archived(&session, &digest);
    assert_eq!(current[0]["proof_anchor"], archived[0]["proof_anchor"]);
    assert_eq!(current[0]["proof_state"], archived[0]["proof_state"]);
    assert_eq!(fingerprint(&session), before);
    assert_eq!(custody_files(&directory.0), files);
    let refused = parsed(&facade::fortress_wait_for_plan(
        Some(session.clone()),
        Some(0),
        Some(digest.clone()),
    ));
    assert_eq!(
        refused["ok"], false,
        "history inspection cannot renew abandoned pursuit"
    );

    // The existing recovery verifier deliberately does not capture a new
    // active-lineage action map for an already-abandoned original. Preserve
    // that limitation explicitly instead of inferring completion from goal
    // history or from a missing carried monitor.
    crate::server::simulate_durable_restart(Some(directory.0.clone()));
    let reopened = open("3971122", true);
    let before = fingerprint(&reopened);
    let files = custody_files(&directory.0);
    let (_, rows) = all_rows(&reopened);
    let missing = rows
        .iter()
        .find(|row| row["plan_digest"] == digest)
        .expect("explicit archive gap");
    assert_eq!(missing["source"], "recovered_archive_unavailable");
    assert_eq!(missing["archive_custody"]["status"], "unavailable");
    assert_eq!(missing["proof_state"], "unknown");
    assert!(missing["proof_terminal"].is_null());
    assert!(missing["physical_work"]["quiescent"].is_null());
    for step in explain_steps(&reopened, &digest) {
        assert_eq!(step["dispatch_status"], "archived_custody_unavailable");
        assert_eq!(step["recovered_evidence"]["available"], false);
    }
    assert_eq!(fingerprint(&reopened), before);
    assert_eq!(custody_files(&directory.0), files);
}
