//! Full fortress.query dispatch, journal replay, packet budgeting and executable
//! historical drilldowns. Nested in the shared archive fixture module.
use super::*;
use dfmcp_adapter::live_jobs::LiveJob;
use dfmcp_core::MapCoord;

fn jobs_observation(
    tick: u32,
    jobs: u32,
    suspended: bool,
    generation: u64,
) -> Result<LiveSpatialCitizenObservation> {
    let empty = observation(tick, 0)?;
    let mut operations = empty.spatial().operations().clone();
    operations.jobs.next_job_id = 100;
    operations.jobs.jobs = (1..=jobs)
        .map(|id| LiveJob {
            native_id: id,
            job_type: 0,
            type_key: "ConstructBuilding".into(),
            reaction: String::new(),
            suspended,
            repeating: false,
            position: MapCoord::new(1, 0, 5),
            worker_native_id: None,
            holder_native_id: None,
            completion_timer: 0,
            attached_item_count: 0,
            required_item_filter_count: 0,
        })
        .collect();
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &empty.spatial().terrain().encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec();
    put(&mut citizens, 0);
    let mut full = b"DFMS1800".to_vec();
    part(&mut full, &spatial);
    part(&mut full, &citizens);
    LiveSpatialCitizenObservation::decode_payload(&full, generation, "df".into(), "dfhack".into())
}
fn populate_jobs(files: &Files, count: u32, reset: bool) -> Result<()> {
    let first = jobs_observation(3, count, true, 7)?;
    let c = writer_context(&first)?;
    let mut journal = open_profile_journal::<Spatial18>(
        &files.path,
        &c,
        JournalLimits::default(),
        TailRecovery::Refuse,
    )?;
    journal.append(first, &c)?;
    journal.append(
        jobs_observation(4, count, false, if reset { 8 } else { 7 })?,
        &c,
    )?;
    Ok(())
}
fn progress_request(session: &Registered) -> Result<Value> {
    let listed = ask(session, json!({"kind":"history"}))?;
    assert_eq!(listed["ok"], true, "{listed}");
    Ok(
        json!({"kind":"historical_job_progress","from":{"record":listed["rows"][0]["record"],"record_digest":listed["rows"][0]["record_digest"]},
        "to":{"record":listed["rows"][1]["record"],"record_digest":listed["rows"][1]["record_digest"]},"limit":128}),
    )
}

#[test]
fn actual_archive_job_progress_pages_and_inspect_calls_stay_on_their_exact_records() -> Result<()> {
    let _serial = crate::test_serial();
    let files = Files::new()?;
    populate_jobs(&files, 12, false)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (s, _) = register(&files, 4096)?;
    let mut query = progress_request(&s)?;
    let mut seen = BTreeSet::new();
    let mut pages = 0usize;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query})),
        );
        assert!(raw.len() <= 16384);
        let page = decode(&raw)?;
        assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["kind"], "historical_job_progress");
        assert_eq!(page["summary"]["jobs_before"], 12);
        assert_eq!(
            page["summary"]["same_generation_with_observed_indicator_change"],
            12
        );
        assert_eq!(page["archive_only"], true);
        assert_eq!(page["native_captures"], 0);
        assert_eq!(page["completion_proven"], false);
        assert_eq!(page["stall_proven"], false);
        assert_eq!(page["agent_turn"]["continuity"]["basis"], page["basis"]);
        assert_eq!(
            page["agent_turn"]["changes"][0]["kind"],
            "archived_job_progress"
        );
        assert_eq!(
            page["agent_turn"]["coverage"]["continuation"],
            page["continuation"]
        );
        for row in page["rows"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture rows absent"))?
        {
            assert!(seen.insert(row["entity_id"].as_str().unwrap_or_default().to_owned()));
            assert_eq!(row["indicators"]["suspension"], "changed");
            for (field, record, expected) in
                [("inspect_before", 1, true), ("inspect_after", 2, false)]
            {
                let inspected =
                    decode(&fortress_query(s.handle(), None, Some(row[field].clone())))?;
                assert_eq!(inspected["ok"], true, "{inspected}");
                assert_eq!(inspected["journal_record"]["record"], record);
                let encoded = inspected.to_string();
                // The typed inspect response retains source facts; the exact
                // archived row itself establishes which suspension is expected.
                assert!(encoded.contains("suspended"));
                assert_eq!(
                    row[if record == 1 { "before" } else { "after" }]["suspended"],
                    expected
                );
            }
        }
        pages += 1;
        assert!(pages <= 12);
        if page["continuation"].is_null() {
            break;
        }
        query["continuation"] = page["continuation"].clone();
        query["limit"] = json!(2);
    }
    assert_eq!(seen.len(), 12);
    assert!(pages > 1);
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}

#[test]
fn job_progress_reopen_rejects_old_cursor_and_validates_exact_endpoints_and_focus() -> Result<()> {
    let _serial = crate::test_serial();
    let files = Files::new()?;
    populate_jobs(&files, 3, false)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (s, _) = register(&files, 65536)?;
    let mut q = progress_request(&s)?;
    q["limit"] = json!(1);
    let original = ask(&s, q.clone())?;
    assert_eq!(original["ok"], true, "{original}");
    let cursor = original["continuation"].clone();
    for variant in 0..8 {
        let mut bad = q.clone();
        match variant {
            0 => bad["to"]["record_digest"] = json!("00".repeat(32)),
            1 => bad["to"]["record"] = json!(4096),
            2 => {
                bad["from"] = q["to"].clone();
                bad["to"] = q["from"].clone();
            }
            3 => bad["job"] = json!({"entity_id":"03","generation":0}),
            4 => bad["job"] = json!({"entity_id":"3","generation":u32::MAX}),
            5 => bad["max_work"] = json!(1),
            6 => bad["limit"] = json!(129),
            _ => bad["unbounded"] = json!(true),
        }
        assert_eq!(ask(&s, bad)?["ok"], false, "variant {variant}");
    }
    let same = ask(
        &s,
        json!({"kind":"historical_job_progress","from":q["from"],"to":q["from"],"include_unchanged":false}),
    )?;
    assert_eq!(same["ok"], true, "{same}");
    assert_eq!(
        same["summary"]["same_generation_with_all_indicators_unchanged"],
        3
    );
    assert_eq!(same["rows"], json!([]));
    drop(s);
    let (next, _) = register(&files, 65536)?;
    let repeated = ask(&next, q.clone())?;
    assert_eq!(repeated["rows"], original["rows"]);
    q["continuation"] = cursor;
    assert_eq!(ask(&next, q)?["error"]["code"], "stale_anchor");
    drop(next);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}

#[test]
fn job_progress_rejects_resets_and_current_authority_expiry_without_writing_history() -> Result<()>
{
    let _serial = crate::test_serial();
    let files = Files::new()?;
    populate_jobs(&files, 3, true)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (s, _) = register(&files, 65536)?;
    assert_eq!(
        ask(&s, progress_request(&s)?)?["error"]["code"],
        "stale_anchor"
    );
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    let files = Files::new()?;
    populate_jobs(&files, 3, false)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (s, _) = register(&files, 65536)?;
    let q = progress_request(&s)?;
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 1;
    }
    assert_eq!(ask(&s, q.clone())?["error"]["code"], "budget_exceeded");
    {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.budget.max_output_tokens = 65536;
        for grant in &mut session.grants {
            grant.expires_at_tick = Some(GameTick(1));
        }
    }
    assert_eq!(ask(&s, q)?["error"]["code"], "capability_denied");
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}

#[test]
fn live_job_progress_reads_survive_fenced_source_without_native_io_or_watch_progress() -> Result<()>
{
    let _serial = crate::test_serial();
    let files = Files::new()?;
    populate_jobs(&files, 3, false)?;
    let (s, calls) = register_live(&files)?;
    let w = ask(
        &s,
        json!({"kind":"watch","key":"job-progress-retains-watch","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"stable_observations":2}),
    )?;
    assert_eq!(w["ok"], true, "{w}");
    let q = progress_request(&s)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    {
        let h = resolve(s.handle())?;
        lock(&h)?.source.fence();
    }
    let report = ask(&s, q)?;
    assert_eq!(report["ok"], true, "{report}");
    assert_eq!(report["agent_turn"]["briefing"]["live_source_fenced"], true);
    assert_eq!(
        report["agent_turn"]["active_work"]["obligations"][0]["evidence_digest"],
        w["record"]["evidence_digest"]
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        ask(&s, json!({"kind":"baselines"}))?["baselines"],
        json!([])
    );
    assert_eq!(
        ask(
            &s,
            json!({"kind":"cancel_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    assert_eq!(
        ask(
            &s,
            json!({"kind":"release_watch","watch":w["record"]["watch"]})
        )?["ok"],
        true
    );
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}
