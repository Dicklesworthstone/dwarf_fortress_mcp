use super::*;

#[test]
fn work_addresses_accept_only_exact_canonical_pins_and_offsets() {
    let hash = "a".repeat(64);
    assert!(matches!(
        address("work"),
        Ok(Address::Catalog {
            digest: None,
            offset: 0
        })
    ));
    assert!(matches!(
        address(&format!("work-{hash}~8")),
        Ok(Address::Catalog { offset: 8, .. })
    ));
    assert!(matches!(
        address(&format!("action-{hash}")),
        Ok(Address::Action(_))
    ));
    assert!(matches!(
        address(&format!("obligation-{hash}")),
        Ok(Address::Obligation(_))
    ));
    for bad in [
        format!("work-{hash}~08"),
        format!("work-{hash}~+8"),
        format!("work-{hash}~-1"),
        format!("work-{hash}~0~1"),
        format!("action-{}", "A".repeat(64)),
        format!("obligation-{hash}~0"),
        "action-deadbeef".to_owned(),
    ] {
        assert!(address(&bad).is_err(), "{bad}");
    }
}

fn open(selector: &str) -> String {
    let raw = crate::agent_facade::fortress_open_session(
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
        Some(false),
    );
    let value: Value = serde_json::from_str(&raw).expect("opened JSON");
    assert_eq!(value["ok"], true, "{value}");
    value["session_id"].as_str().expect("session").to_owned()
}

#[test]
fn work_read_checks_the_original_wall_deadline_after_rendering() {
    let session = open("3971111");
    let uri = format!("df://session/{session}/work");
    let handle = resolve_session(Some(session.clone())).expect("session");
    let before = {
        let guard = handle.lock().expect("lock");
        (
            guard.adapter.snapshot().anchor(),
            read_context(&guard).request_id,
            crate::server::resource_replay_bundle(&guard),
        )
    };
    let mut calls = 0;
    let refused = read_with_elapsed(&session, &uri, "work", || {
        calls += 1;
        if calls < 2 {
            Duration::ZERO
        } else {
            Duration::MAX
        }
    })
    .expect_err("a late deadline cannot publish success");
    assert_eq!(
        refused.data.expect("structured refusal")["error"]["code"],
        "budget_exceeded"
    );
    let guard = handle.lock().expect("lock");
    assert_eq!(
        before,
        (
            guard.adapter.snapshot().anchor(),
            read_context(&guard).request_id,
            crate::server::resource_replay_bundle(&guard)
        )
    );
}

#[test]
fn work_catalog_never_clips_one_indivisible_escaped_row_to_fit() {
    let session = open("3971112");
    let uri = format!("df://session/{session}/work");
    let handle = resolve_session(Some(session.clone())).expect("session");
    let guard = handle.lock().expect("lock");
    let source = SourceWork {
        records: 1,
        bytes: 24_000,
        archive_bytes: 0,
    };
    let rows = [json!({"progress_digest":"a".repeat(64),
        "physical_work":{"state":"unknown", "reason":"\"\\\n".repeat(3_000)}})];
    let error = catalog(
        &session,
        &uri,
        None,
        0,
        &rows,
        guard.adapter.snapshot().anchor(),
        guard.budget,
        &source,
        &mut || Duration::ZERO,
    )
    .expect_err("one intact row cannot fit");
    assert_eq!(error.code, ErrorCode::BudgetExceeded);
}
