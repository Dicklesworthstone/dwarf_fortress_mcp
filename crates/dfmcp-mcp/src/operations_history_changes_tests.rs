// Historical comparisons through the actual operations public query handler.
// Included by live_operations_server_tests.rs to reuse its registered-session
// and injected native-source boundary. Storage and query execution are real.

use super::*;
use dfmcp_adapter::live_operations::{LiveBuilding, LiveItem};
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::{GameTick, MapCoord};
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn test_io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|failure| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            format!("historical changes fixture: {failure}"),
        )
    })
}

fn source(count: u32, tick: u32) -> Result<LiveOperationsObservation> {
    let mut value = observation()?;
    value.jobs.year_tick = tick;
    value.jobs.next_job_id = 0;
    value.jobs.jobs.clear();
    value.attachments.clear();
    value.next_building_id = 21;
    value.buildings = vec![LiveBuilding {
        native_id: 20,
        building_type: 1,
        type_key: "Workshop".to_owned(),
        x1: 1,
        y1: 1,
        x2: 3,
        y2: 3,
        z: 0,
        build_stage: 1,
        max_build_stage: 3,
    }];
    value.next_item_id = 30 + count;
    value.items = (0..count)
        .map(|index| LiveItem {
            native_id: 30 + index,
            item_type: 1,
            type_key: "BAR".to_owned(),
            subtype: -1,
            material_type: 0,
            material_index: 1,
            stack_size: if index == 0 { 5 } else { 1 },
            raw_position: MapCoord::new(0, 0, 0),
            flags: 64,
            container_native_id: None,
            holder_building_native_id: None,
        })
        .collect();
    Ok(value)
}

fn register_source(
    profile: OperationsProfile,
    first: LiveOperationsObservation,
    pending: Vec<LiveOperationsObservation>,
    tokens: u32,
) -> Result<Registered> {
    let registered = register(
        tokens,
        vec![Capability::Observe, Capability::Query, Capability::Doctor],
    )?;
    let handle = resolve(registered.handle())?;
    let mut session = lock(&handle)?;
    let mut state = LiveOperationsState::with_profile(profile);
    state.publish(first)?;
    session.state = state;
    session.source = Box::new(Script {
        observations: pending.into(),
        calls: Arc::clone(&registered.calls),
        fenced: false,
    });
    session.limits.items = if profile == OperationsProfile::V1_3 {
        32768
    } else {
        65536
    };
    session.limits.payload_bytes = profile.maximum_bytes();
    session.budget.max_entities = session.limits.entity_limit();
    session.budget.max_bytes = session.limits.payload_bytes as u64;
    session.budget.max_wall_millis = 60_000;
    drop(session);
    Ok(registered)
}

struct ChangesArchive {
    session: Registered,
    path: PathBuf,
    directory: PathBuf,
}

impl Drop for ChangesArchive {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.session.id);
        }
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn attach(session: &Registered, path: &std::path::Path) -> Result<()> {
    let handle = resolve(session.handle())?;
    let mut session = lock(&handle)?;
    let context = session.context()?;
    history::attach(&mut session, path, TailRecovery::Refuse, &context)
}

fn archive(
    profile: OperationsProfile,
    first: LiveOperationsObservation,
    pending: Vec<LiveOperationsObservation>,
    tokens: u32,
) -> Result<ChangesArchive> {
    let session = register_source(profile, first, pending, tokens)?;
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-history-changes-{}-{}",
        std::process::id(),
        session.id
    ));
    test_io(fs::create_dir(&directory))?;
    test_io(fs::set_permissions(
        &directory,
        fs::Permissions::from_mode(0o700),
    ))?;
    let directory = test_io(directory.canonicalize())?;
    let fixture = ChangesArchive {
        session,
        path: directory.join("operations.bin"),
        directory,
    };
    attach(&fixture.session, &fixture.path)?;
    Ok(fixture)
}

fn listing(session: &Registered) -> Result<Value> {
    ask(session, json!({"kind":"history","limit":64}))
}

fn record(row: &Value) -> Value {
    json!({"record":row["record"],"record_digest":row["record_digest"]})
}

fn selection() -> Value {
    json!({"kind":"entities","kinds":["item","building"],
        "fields":["stack_size","build_stage"]})
}

fn request(rows: &Value, from: usize, to: usize, select: Value) -> Value {
    json!({"kind":"historical_changes","from":record(&rows[from]),
        "to":record(&rows[to]),"select":select})
}

fn observe(session: &Registered) -> Result<Value> {
    decode(&fortress_observe(session.handle()))
}

fn current_anchor(session: &Registered) -> Result<StateAnchor> {
    let handle = resolve(session.handle())?;
    lock(&handle)?.anchor()
}

fn change_for(result: &Value, id: dfmcp_core::EntityId) -> Result<&Value> {
    result["changes"]
        .as_array()
        .and_then(|changes| {
            changes
                .iter()
                .find(|change| change["entity_id"] == id.to_string())
        })
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "expected historical change absent",
            )
        })
}

#[test]
fn historical_changes_report_observed_item_and_building_differences_without_native_reads()
-> Result<()> {
    let _serial = crate::test_serial();
    for profile in [OperationsProfile::V1_3, OperationsProfile::PagedV1_4] {
        let first = source(3, 3)?;
        let mut second = first.clone();
        second.jobs.year_tick = 4;
        second.items[0].stack_size = 8;
        second.buildings[0].build_stage = 2;
        let fixture = archive(profile, first, vec![second], 8192)?;
        let watch = ask(
            &fixture.session,
            json!({"kind":"watch","key":"comparison-work",
            "label":"Retain current monitor","condition":{"op":"tick_at_least","value":105u64*403200+50},
            "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}),
        )?;
        assert_eq!(watch["ok"], true, "{watch}");
        assert_eq!(observe(&fixture.session)?["ok"], true);
        let history = listing(&fixture.session)?;
        let anchor = current_anchor(&fixture.session)?;
        let work_before = ask(&fixture.session, json!({"kind":"watches"}))?;
        let baselines_before = ask(&fixture.session, json!({"kind":"baselines"}))?;
        assert_eq!(work_before["records"].as_array().map(Vec::len), Some(1));
        assert_eq!(baselines_before["baselines"], json!([]));
        let query = request(&history["rows"], 0, 1, selection());
        let result = ask(&fixture.session, query.clone())?;
        assert_eq!(result["ok"], true, "profile {profile:?}: {result}");
        assert_eq!(result["kind"], "historical_changes");
        assert_eq!(result["change_count"], 2);
        assert_eq!(result["change_counts"]["changed_in_result"], 2);
        assert_eq!(result["change_counts"]["entered_result"], 0);
        assert_eq!(result["change_counts"]["left_result"], 0);
        let item = change_for(&result, item_entity_id(30))?;
        assert_eq!(item["before"]["fields"]["stack_size"]["value"]["value"], 5);
        assert_eq!(item["after"]["fields"]["stack_size"]["value"]["value"], 8);
        let building = change_for(&result, building_entity_id(20))?;
        assert_eq!(
            building["before"]["fields"]["build_stage"]["value"]["value"],
            1
        );
        assert_eq!(
            building["after"]["fields"]["build_stage"]["value"]["value"],
            2
        );
        assert_eq!(result["basis"], history["rows"][0]["anchor"]);
        assert_eq!(result["anchor"], history["rows"][1]["anchor"]);
        assert_eq!(result["current_live_anchor"], anchor_json(anchor));
        assert_eq!(
            result["agent_turn"]["briefing"]["bridge_protocol"],
            profile.protocol()
        );
        assert_eq!(result["agent_turn"]["continuity"]["status"], "partial");
        assert_eq!(
            result["agent_turn"]["coverage"]["current_freshness_proven"],
            false
        );
        assert_eq!(result["native_observations"], 0);
        assert_eq!(result["baseline_created"], false);
        assert_eq!(result["eligible_as_mutation_evidence"], false);
        assert_eq!(
            result["agent_turn"]["active_work"]["obligations"][0]["watch"],
            watch["record"]["watch"]
        );
        let retry = ask(&fixture.session, query)?;
        assert_eq!(retry["changes"], result["changes"]);
        assert_eq!(retry["comparison_digest"], result["comparison_digest"]);
        assert_eq!(
            ask(&fixture.session, json!({"kind":"watches"}))?["records"],
            work_before["records"]
        );
        assert_eq!(
            ask(&fixture.session, json!({"kind":"baselines"}))?["baselines"],
            baselines_before["baselines"]
        );
        assert_eq!(current_anchor(&fixture.session)?, anchor);
        assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            ask(
                &fixture.session,
                json!({"kind":"cancel_watch","watch":watch["record"]["watch"]})
            )?["ok"],
            true
        );
        assert_eq!(
            ask(
                &fixture.session,
                json!({"kind":"release_watch","watch":watch["record"]["watch"]})
            )?["ok"],
            true
        );
    }
    Ok(())
}

#[test]
fn historical_changes_use_current_grants_when_retained_ticks_exceed_a_reset_clock() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 50)?;
    let mut second = first.clone();
    second.jobs.year_tick = 51;
    second.items[0].stack_size = 8;
    let mut reset = second.clone();
    reset.jobs.year_tick = 3;
    reset.jobs.bridge_generation += 1;
    let fixture = archive(
        OperationsProfile::PagedV1_4,
        first,
        vec![second, reset],
        8192,
    )?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let history = listing(&fixture.session)?;
    let current = current_anchor(&fixture.session)?;
    {
        let handle = resolve(fixture.session.handle())?;
        let mut session = lock(&handle)?;
        session
            .grants
            .retain(|grant| grant.capability != Capability::Observe);
        for grant in &mut session.grants {
            if grant.capability == Capability::Query {
                grant.expires_at_tick = Some(GameTick(current.tick.0 + 1));
            }
        }
    }
    let result = ask(
        &fixture.session,
        request(&history["rows"], 0, 1, selection()),
    )?;
    assert_eq!(
        result["ok"], true,
        "historical ticks cannot revoke current authority: {result}"
    );
    assert_eq!(result["change_count"], 1);
    assert_eq!(result["current_live_anchor"], anchor_json(current));
    assert_eq!(result["anchor"], history["rows"][1]["anchor"]);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        observe(&fixture.session)?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn historical_changes_cannot_restore_expired_authority_from_earlier_endpoints() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let mut second = first.clone();
    second.jobs.year_tick = 4;
    second.items[0].stack_size = 8;
    let mut current = second.clone();
    current.jobs.year_tick = 50;
    let fixture = archive(
        OperationsProfile::PagedV1_4,
        first,
        vec![second, current],
        8192,
    )?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let history = listing(&fixture.session)?;
    {
        let handle = resolve(fixture.session.handle())?;
        let mut session = lock(&handle)?;
        for grant in &mut session.grants {
            if grant.capability == Capability::Query {
                grant.expires_at_tick = Some(GameTick(105u64 * 403200 + 4));
            }
        }
    }
    let denied = ask(
        &fixture.session,
        request(&history["rows"], 0, 1, selection()),
    )?;
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied.get("changes").is_none());
    assert!(denied.get("comparison").is_none());
    assert!(denied.get("anchor").is_none());
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn historical_changes_reconstruct_departure_and_arrival_after_durable_reopen() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let mut removed = first.clone();
    removed.jobs.year_tick = 4;
    removed.items.remove(0);
    let mut returned = first.clone();
    returned.jobs.year_tick = 5;
    returned.items[0].stack_size = 9;
    let mut fixture = archive(
        OperationsProfile::PagedV1_4,
        first,
        vec![removed, returned.clone()],
        8192,
    )?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let before = listing(&fixture.session)?;
    let request = request(
        &before["rows"],
        0,
        2,
        json!({"kind":"entities","kinds":["item"],"fields":["stack_size"]}),
    );
    let old_session = fixture.session.id;
    lock(&SESSIONS)?.remove(&old_session);
    let reopened = register_source(OperationsProfile::PagedV1_4, returned, Vec::new(), 8192)?;
    attach(&reopened, &fixture.path)?;
    fixture.session = reopened;
    assert_ne!(fixture.session.id, old_session);
    let after = listing(&fixture.session)?;
    assert_eq!(after["rows"], before["rows"]);
    let result = ask(&fixture.session, request)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["change_count"], 2);
    assert_eq!(result["change_counts"]["changed_in_result"], 0);
    assert_eq!(result["change_counts"]["left_result"], 1);
    assert_eq!(result["change_counts"]["entered_result"], 1);
    assert_eq!(
        result["changes"][0]["entity_id"],
        item_entity_id(30).to_string()
    );
    assert_eq!(
        result["changes"][1]["entity_id"],
        item_entity_id(30).to_string()
    );
    assert_eq!(result["changes"][0]["generation"], 1);
    assert_eq!(result["changes"][1]["generation"], 2);
    assert!(result["changes"][0]["after"].is_null());
    assert!(result["changes"][1]["before"].is_null());
    assert_eq!(result["coverage"]["absence_proven"], false);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn historical_change_pages_bind_record_pair_selection_session_and_current_head() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let mut second = first.clone();
    second.jobs.year_tick = 4;
    for item in &mut second.items {
        item.stack_size += 2;
    }
    let mut third = second.clone();
    third.jobs.year_tick = 5;
    let mut fixture = archive(
        OperationsProfile::PagedV1_4,
        first,
        vec![second.clone()],
        8192,
    )?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let rows = listing(&fixture.session)?["rows"].clone();
    let mut query = request(
        &rows,
        0,
        1,
        json!({"kind":"entities","kinds":["item"],"fields":["stack_size"]}),
    );
    query["limit"] = json!(1);
    let first_page = ask(&fixture.session, query.clone())?;
    assert_eq!(first_page["ok"], true, "{first_page}");
    assert_eq!(first_page["returned"], 1);
    assert!(first_page["continuation"].is_string());
    query["continuation"] = first_page["continuation"].clone();
    query["limit"] = json!(2);
    let last_page = ask(&fixture.session, query.clone())?;
    assert_eq!(last_page["ok"], true, "{last_page}");
    assert_eq!(last_page["returned"], 2);
    assert_eq!(last_page["continuation"], Value::Null);
    assert_ne!(
        first_page["changes"][0]["entity_id"],
        last_page["changes"][0]["entity_id"]
    );
    let mut changed_selection = query.clone();
    changed_selection["select"]["fields"] = json!(["forbidden"]);
    assert_eq!(ask(&fixture.session, changed_selection)?["ok"], false);
    let mut changed_digest = query.clone();
    changed_digest["from"]["record_digest"] = json!("0".repeat(64));
    assert_eq!(
        ask(&fixture.session, changed_digest)?["error"]["code"],
        "stale_anchor"
    );
    let old_anchor = current_anchor(&fixture.session)?;
    let old_session = fixture.session.id;
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    lock(&SESSIONS)?.remove(&old_session);
    let reopened = register_source(OperationsProfile::PagedV1_4, second, vec![third], 8192)?;
    attach(&reopened, &fixture.path)?;
    fixture.session = reopened;
    assert_ne!(fixture.session.id, old_session);
    assert_eq!(current_anchor(&fixture.session)?, old_anchor);
    assert_eq!(listing(&fixture.session)?["rows"], rows);
    assert_eq!(
        ask(&fixture.session, query.clone())?["error"]["code"],
        "stale_anchor"
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
    let mut renewed = request(
        &rows,
        0,
        1,
        json!({"kind":"entities","kinds":["item"],"fields":["stack_size"]}),
    );
    renewed["limit"] = json!(1);
    let page = ask(&fixture.session, renewed.clone())?;
    assert_eq!(page["ok"], true, "{page}");
    assert!(page["continuation"].is_string());
    renewed["continuation"] = page["continuation"].clone();
    assert_eq!(observe(&fixture.session)?["ok"], true);
    assert_eq!(
        ask(&fixture.session, renewed)?["error"]["code"],
        "stale_anchor"
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn same_endpoint_is_an_empty_complete_selected_diff_and_reset_crossings_refuse() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let mut reset = first.clone();
    reset.jobs.bridge_generation += 1;
    reset.jobs.year_tick = 2;
    let fixture = archive(OperationsProfile::PagedV1_4, first, vec![reset], 8192)?;
    let before = listing(&fixture.session)?;
    let empty = ask(
        &fixture.session,
        request(&before["rows"], 0, 0, selection()),
    )?;
    assert_eq!(empty["ok"], true, "{empty}");
    assert_eq!(empty["change_count"], 0);
    assert_eq!(empty["returned"], 0);
    assert_eq!(empty["changes"], json!([]));
    assert_eq!(empty["continuation"], Value::Null);
    assert_eq!(empty["truncated"], false);
    assert_eq!(empty["source_work"]["distinct_endpoints"], 1);
    assert_eq!(empty["basis_result_digest"], empty["target_result_digest"]);
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let rows = listing(&fixture.session)?["rows"].clone();
    let refused = ask(&fixture.session, request(&rows, 0, 1, selection()))?;
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert!(refused.get("changes").is_none());
    assert_eq!(
        ask(&fixture.session, request(&rows, 1, 0, selection()))?["error"]["code"],
        "invalid_request"
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn historical_change_output_allowance_does_not_replace_complete_source_admission() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(120, 3)?;
    let mut second = first.clone();
    second.jobs.year_tick = 4;
    second.items[0].stack_size = 8;
    let fixture = archive(OperationsProfile::PagedV1_4, first, vec![second], 2048)?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let rows = listing(&fixture.session)?["rows"].clone();
    let query = request(
        &rows,
        0,
        1,
        json!({"kind":"entities","kinds":["item"],"fields":["stack_size"]}),
    );
    let raw = fortress_query(
        fixture.session.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":query})),
    );
    assert!(raw.len() <= 8192);
    let result = decode(&raw)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["change_count"], 1);
    assert_eq!(result["matched_before"], 120);
    assert_eq!(result["matched_after"], 120);
    assert!(
        result["source_work"]["canonical_bytes"]
            .as_u64()
            .is_some_and(|value| value > 8192)
    );
    let before = current_anchor(&fixture.session)?;
    {
        let handle = resolve(fixture.session.handle())?;
        let mut session = lock(&handle)?;
        // Each source fits individually. The full pair must share one allowance.
        session.budget.max_entities = 122;
    }
    let one_endpoint = ask(
        &fixture.session,
        request(
            &rows,
            0,
            0,
            json!({"kind":"entities","kinds":["item"],"fields":["stack_size"]}),
        ),
    )?;
    assert_eq!(one_endpoint["ok"], true, "{one_endpoint}");
    assert_eq!(one_endpoint["source_work"]["distinct_endpoints"], 1);
    let refused = ask(&fixture.session, query)?;
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    assert!(refused.get("changes").is_none());
    assert_eq!(current_anchor(&fixture.session)?, before);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn historical_changes_require_exact_records_and_current_custody_before_disclosure() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let second = source(3, 4)?;
    let fixture = archive(OperationsProfile::PagedV1_4, first, vec![second], 8192)?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let rows = listing(&fixture.session)?["rows"].clone();
    let base = request(&rows, 0, 1, selection());
    for invalid in [
        json!({"kind":"historical_changes","from":{"record":0,"record_digest":rows[0]["record_digest"]},"to":record(&rows[1]),"select":selection()}),
        json!({"kind":"historical_changes","from":record(&rows[0]),"to":record(&rows[1]),"select":{"kind":"entities","limit":1}}),
        json!({"kind":"historical_changes","from":record(&rows[0]),"to":record(&rows[1]),"select":{"kind":"watch"}}),
        json!({"kind":"historical_changes","from":record(&rows[0]),"to":record(&rows[1]),"select":selection(),"path":"/not-a-client-path"}),
    ] {
        assert_eq!(
            ask(&fixture.session, invalid)?["error"]["code"],
            "invalid_request"
        );
    }
    let mut wrong_digest = base.clone();
    wrong_digest["from"]["record_digest"] = json!("0".repeat(64));
    assert_eq!(
        ask(&fixture.session, wrong_digest)?["error"]["code"],
        "stale_anchor"
    );
    let before = current_anchor(&fixture.session)?;
    let bytes = test_io(fs::read(&fixture.path))?;
    test_io(fs::set_permissions(
        &fixture.path,
        fs::Permissions::from_mode(0o400),
    ))?;
    let result = ask(&fixture.session, base)?;
    assert_eq!(result["error"]["code"], "corrupt_ledger");
    assert!(result.get("changes").is_none());
    assert!(result.get("comparison").is_none());
    assert_eq!(current_anchor(&fixture.session)?, before);
    assert_eq!(test_io(fs::read(&fixture.path))?, bytes);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    test_io(fs::set_permissions(
        &fixture.path,
        fs::Permissions::from_mode(0o600),
    ))?;
    Ok(())
}

#[test]
fn corrupt_historical_endpoint_never_produces_a_partial_comparison() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let mut second = first.clone();
    second.jobs.year_tick = 4;
    second.items[0].stack_size = 8;
    let fixture = archive(OperationsProfile::PagedV1_4, first, vec![second], 8192)?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let rows = listing(&fixture.session)?["rows"].clone();
    let before = current_anchor(&fixture.session)?;
    let mut file = test_io(fs::OpenOptions::new().write(true).open(&fixture.path))?;
    test_io(file.seek(SeekFrom::End(-1)))?;
    test_io(file.write_all(b"!"))?;
    test_io(file.sync_all())?;
    drop(file);
    let result = ask(&fixture.session, request(&rows, 0, 1, selection()))?;
    assert_eq!(result["error"]["code"], "corrupt_ledger");
    assert!(result.get("changes").is_none());
    assert!(result.get("comparison").is_none());
    assert_eq!(current_anchor(&fixture.session)?, before);
    assert_eq!(
        observe(&fixture.session)?["error"]["code"],
        "adapter_unavailable"
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn furniture_arrivals_separate_new_ids_from_stock_changes_and_bound_only_examples() -> Result<()> {
    let _serial = crate::test_serial();
    for profile in [OperationsProfile::V1_3, OperationsProfile::PagedV1_4] {
        let mut first = source(3, 3)?;
        first.next_item_id = 35;
        for item in &mut first.items {
            item.type_key = "BED".to_owned();
        }
        let mut second = source(17, 4)?;
        second.items.retain(|item| item.native_id >= 33);
        for item in &mut second.items {
            item.type_key = "BED".to_owned();
            match item.native_id {
                44 => item.material_index = 8,
                45 => {
                    item.type_key = "CHAIR".to_owned();
                    item.item_type = 2;
                }
                46 => item.flags |= 1 << 3,
                _ => {}
            }
        }
        let fixture = archive(profile, first, vec![second], 8192)?;
        assert_eq!(observe(&fixture.session)?["ok"], true);
        let rows = listing(&fixture.session)?["rows"].clone();
        let mut query = json!({"kind":"historical_furniture_arrivals","from":record(&rows[0]),
            "to":record(&rows[1]),"furniture_kind":"bed","material":[0,1]});
        let result = ask(&fixture.session, query.clone())?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(result["before_item_count"], 3);
        assert_eq!(result["after_item_count"], 11);
        assert_eq!(result["net_item_count_change"], 8);
        assert_eq!(result["new_item_count"], 9);
        assert_eq!(result["before_next_item_id"], 35);
        assert_eq!(result["after_next_item_id"], 47);
        assert_eq!(result["examples_truncated"], true);
        let examples = result["new_item_examples"].as_array().ok_or_else(|| {
            error(ErrorCode::InternalInvariantViolation, "missing furniture examples")
        })?;
        assert_eq!(examples.len(), 8);
        for (offset, example) in examples.iter().enumerate() {
            let id = 35 + offset as u32;
            assert_eq!(example["native_item_id"], id);
            assert_eq!(example["entity_id"], item_entity_id(id).to_string());
            assert_eq!(example["generation"], 1);
            assert_eq!(example["material"], json!([0, 1]));
        }
        assert_eq!(result["order_attribution_proven"], false);
        assert_eq!(result["native_observations"], 0);
        assert_eq!(result["basis"], rows[0]["anchor"]);
        assert_eq!(result["anchor"], rows[1]["anchor"]);
        query["material"] = Value::Null;
        assert_eq!(ask(&fixture.session, query.clone())?["new_item_count"], 10);
        query["furniture_kind"] = json!("chair");
        let chair = ask(&fixture.session, query.clone())?;
        assert_eq!(chair["new_item_count"], 1);
        assert_eq!(chair["new_item_examples"][0]["native_item_id"], 45);
        query["from"] = record(&rows[1]);
        let same = ask(&fixture.session, query.clone())?;
        assert_eq!(same["new_item_count"], 0);
        assert_eq!(same["source_work"]["distinct_endpoints"], 1);
        query["from"]["record_digest"] = json!("0".repeat(64));
        assert_eq!(ask(&fixture.session, query)?["error"]["code"], "stale_anchor");
        assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
fn furniture_arrivals_refuse_source_resets_and_unbounded_or_untyped_filters() -> Result<()> {
    let _serial = crate::test_serial();
    let first = source(3, 3)?;
    let mut reset = source(1, 4)?;
    reset.jobs.bridge_generation += 1;
    let fixture = archive(OperationsProfile::PagedV1_4, first, vec![reset], 8192)?;
    assert_eq!(observe(&fixture.session)?["ok"], true);
    let rows = listing(&fixture.session)?["rows"].clone();
    let mut query = json!({"kind":"historical_furniture_arrivals","from":record(&rows[0]),
        "to":record(&rows[1]),"furniture_kind":"bed"});
    let refused = ask(&fixture.session, query.clone())?;
    assert_eq!(refused["error"]["code"], "stale_anchor");
    assert!(refused.get("new_item_count").is_none());
    for material in [json!([-1, 1]), json!([1, -2]), json!([1]), json!("wood")] {
        query["material"] = material;
        assert_eq!(ask(&fixture.session, query.clone())?["error"]["code"], "invalid_request");
    }
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}
