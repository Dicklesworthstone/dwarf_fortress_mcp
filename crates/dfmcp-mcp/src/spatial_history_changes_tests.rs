//! Nested in the archive handler fixture module to share its session lock and
//! exclusively owned test files, rather than competing for global session slots.
use super::*;

fn changed_observation(
    tick: u32,
    workers: u32,
    profession: u32,
    generation: u64,
) -> Result<LiveSpatialCitizenObservation> {
    let empty = observation(tick, 0)?;
    let mut citizens = b"DFMC1800".to_vec();
    put(&mut citizens, workers);
    for i in 0..workers {
        put(&mut citizens, 10 + i);
        text(&mut citizens, "Urist");
        text(&mut citizens, "DWARF");
        for n in [profession, 1, 0, 5] {
            put(&mut citizens, n);
        }
        citizens.extend_from_slice(&0x11fu16.to_be_bytes());
        put(&mut citizens, 6);
        citizens.extend_from_slice(&[1, 1]);
        citizens.extend_from_slice(&1u16.to_be_bytes());
        put(&mut citizens, 0);
        text(&mut citizens, "CARPENTRY");
        for n in [5, 5, 1] {
            put(&mut citizens, n);
        }
    }
    let mut bytes = b"DFMS1800".to_vec();
    part(&mut bytes, &empty.spatial().encode_payload()?);
    part(&mut bytes, &citizens);
    LiveSpatialCitizenObservation::decode_payload(&bytes, generation, "df".into(), "dfhack".into())
}
fn writer_context(value: &LiveSpatialCitizenObservation) -> Result<OperationContext> {
    let mut state = LiveSpatialCitizenState::default();
    state.publish(value.clone())?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "test snapshot absent",
            )
        })?
        .anchor();
    Ok(OperationContext {
        session_id: next_id()?,
        request_id: RequestId::new(1),
        anchor,
        budget: WorkBudget {
            max_entities: 100_000,
            max_bytes: 16 * 1024 * 1024,
            max_wall_millis: 60_000,
            ..WorkBudget::default()
        },
        grants: grants(
            &[Capability::Query, Capability::Observe, Capability::Doctor],
            Some(anchor.fortress_id),
        ),
        cancellation_requested: false,
    })
}
fn populate_changes(files: &Files, count: u32, reset: bool) -> Result<()> {
    let first = changed_observation(3, count, 1, 7)?;
    let c = writer_context(&first)?;
    let mut journal = open_profile_journal::<Spatial18>(
        &files.path,
        &c,
        JournalLimits::default(),
        TailRecovery::Refuse,
    )?;
    journal.append(first, &c)?;
    journal.append(
        changed_observation(4, count, 2, if reset { 8 } else { 7 })?,
        &c,
    )?;
    Ok(())
}
fn request(session: &Registered) -> Result<Value> {
    let listed = ask(session, json!({"kind":"history"}))?;
    assert_eq!(listed["ok"], true, "{listed}");
    Ok(json!({"kind":"historical_changes",
        "from":{"record":listed["rows"][0]["record"],"record_digest":listed["rows"][0]["record_digest"]},
        "to":{"record":listed["rows"][1]["record"],"record_digest":listed["rows"][1]["record_digest"]},
        "select":{"kind":"entities","kinds":["unit"],"fields":["profession"]},"limit":128}))
}

#[test]
fn historical_change_pages_fit_8192_bytes_and_compare_all_rows_before_paging() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 40, false)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (s, _) = register(&files, 2048)?;
    let mut q = request(&s)?;
    let mut ids = BTreeSet::new();
    let mut pages = 0;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":q.clone()})),
        );
        assert!(raw.len() <= 8192);
        let page = decode(&raw)?;
        assert_eq!(page["ok"], true, "{page}");
        assert_eq!(page["change_count"], 40);
        assert_eq!(page["matched_before"], 40);
        assert_eq!(page["matched_after"], 40);
        assert_eq!(page["change_counts"]["changed_in_result"], 40);
        assert_eq!(page["archive_only"], true);
        assert_eq!(page["native_captures"], 0);
        assert_eq!(page["current_freshness_proven"], false);
        assert_eq!(page["agent_turn"]["continuity"]["basis"], page["basis"]);
        assert_eq!(page["agent_turn"]["changes"][0]["change_count"], 40);
        for change in page["changes"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "test changes absent"))?
        {
            assert!(ids.insert(change["entity_id"].as_str().unwrap_or_default().to_owned()));
            assert!(change["before"].is_object());
            assert!(change["after"].is_object());
        }
        pages += 1;
        assert!(pages <= 40);
        if page["continuation"].is_null() {
            break;
        }
        assert_eq!(
            page["agent_turn"]["coverage"]["continuation"],
            page["continuation"]
        );
        q["continuation"] = page["continuation"].clone();
        q["limit"] = json!(2);
    }
    assert_eq!(ids.len(), 40);
    assert!(pages > 1);
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}

#[test]
fn historical_changes_reopen_without_a_baseline_and_reject_previous_session_tokens() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 3, false)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (first, _) = register(&files, 65536)?;
    let mut q = request(&first)?;
    q["limit"] = json!(1);
    let original = ask(&first, q.clone())?;
    assert_eq!(original["ok"], true);
    let token = original["continuation"].clone();
    drop(first);
    let (next, _) = register(&files, 65536)?;
    let repeated = ask(&next, q.clone())?;
    assert_eq!(repeated["ok"], true);
    assert_eq!(repeated["changes"], original["changes"]);
    assert_eq!(
        repeated["basis_result_digest"],
        original["basis_result_digest"]
    );
    assert_eq!(
        repeated["target_result_digest"],
        original["target_result_digest"]
    );
    q["continuation"] = token;
    assert_eq!(ask(&next, q)?["error"]["code"], "stale_anchor");
    drop(next);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}

struct NoReads {
    calls: Arc<AtomicUsize>,
    fenced: bool,
}
impl Source for NoReads {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(error(
            ErrorCode::AdapterUnavailable,
            "historical comparison must not acquire live data",
        ))
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn pages(&self) -> u32 {
        0
    }
}
fn register_live(files: &Files) -> Result<(Registered, Arc<AtomicUsize>)> {
    let c = writer_context(&changed_observation(4, 3, 2, 7)?)?;
    let journal = open_profile_journal::<Spatial18>(
        &files.path,
        &c,
        JournalLimits::default(),
        TailRecovery::Refuse,
    )?;
    let calls = Arc::new(AtomicUsize::new(0));
    let id = next_id()?;
    let session = Session {
        id,
        source: Box::new(NoReads {
            calls: Arc::clone(&calls),
            fenced: false,
        }),
        state: journal.state().clone(),
        limits: limits(),
        journal: Some(journal),
        budget: budget(8192),
        grants: c.grants,
        request: 0,
        _watch_journal: None,
        _slot: Slot::reserve()?,
    };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok((Registered { id }, calls))
}

#[test]
fn live_comparisons_preserve_current_watches_and_work_after_source_failure() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 3, false)?;
    let (s, calls) = register_live(&files)?;
    let w = ask(
        &s,
        json!({"kind":"watch","key":"remain-paused","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"stable_observations":2}),
    )?;
    assert_eq!(w["ok"], true, "{w}");
    let q = request(&s)?;
    let original = fs::read(&files.path).map_err(io_error)?;
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.source.fence();
    }
    let result = ask(&s, q)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["change_count"], 3);
    assert_eq!(result["agent_turn"]["briefing"]["live_source_fenced"], true);
    assert_eq!(
        result["agent_turn"]["active_work"]["obligations"][0]["watch"],
        w["record"]["watch"]
    );
    assert_eq!(
        result["agent_turn"]["active_work"]["obligations"][0]["evidence_digest"],
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
    assert_eq!(fs::read(&files.path).map_err(io_error)?, original);
    Ok(())
}

#[test]
fn exact_pair_validation_rejects_wrong_digests_missing_records_and_reversal() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 3, false)?;
    let (s, _) = register(&files, 65536)?;
    let q = request(&s)?;
    for case in 0..5 {
        let mut bad = q.clone();
        match case {
            0 => bad["to"]["record_digest"] = json!("00".repeat(32)),
            1 => bad["to"]["record"] = json!(4096),
            2 => bad["from"] = q["to"].clone(),
            3 => bad["select"]["continuation"] = json!("untrusted"),
            _ => bad["from"]["record"] = json!(0),
        }
        if case == 2 {
            bad["to"] = q["from"].clone();
        }
        assert_eq!(ask(&s, bad)?["ok"], false, "case={case}");
    }
    let same = ask(
        &s,
        json!({"kind":"historical_changes","from":q["from"],"to":q["from"],"select":q["select"]}),
    )?;
    assert_eq!(same["ok"], true);
    assert_eq!(same["change_count"], 0);
    assert_eq!(same["provenance_only_refreshes"], 0);
    Ok(())
}

#[test]
fn observation_reset_is_not_interpreted_as_mass_departure_and_arrival() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 3, true)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
    let (s, _) = register(&files, 65536)?;
    assert_eq!(ask(&s, request(&s)?)?["error"]["code"], "stale_anchor");
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}

#[test]
fn selected_record_corruption_is_detected_even_when_file_length_is_unchanged() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 3, false)?;
    let (s, _) = register(&files, 65536)?;
    let q = request(&s)?;
    let offset = {
        let h = resolve(s.handle())?;
        let session = lock(&h)?;
        session
            .journal
            .as_ref()
            .and_then(|j| j.entries().first())
            .map(|e| e.offset)
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "fixture entry absent"))?
    };
    let mut corrupt = fs::read(&files.path).map_err(io_error)?;
    corrupt[offset as usize + 50] ^= 1;
    fs::write(&files.path, &corrupt).map_err(io_error)?;
    assert_eq!(ask(&s, q)?["error"]["code"], "corrupt_ledger");
    assert_eq!(
        ask(&s, json!({"kind":"history"}))?["error"]["code"],
        "corrupt_ledger"
    );
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, corrupt);
    Ok(())
}

#[test]
fn output_refusal_and_expired_current_grants_do_not_change_history() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    populate_changes(&files, 3, false)?;
    let (s, _) = register(&files, 65536)?;
    let q = request(&s)?;
    let bytes = fs::read(&files.path).map_err(io_error)?;
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
    let denied = ask(&s, q)?;
    assert_eq!(denied["error"]["code"], "capability_denied");
    assert!(denied.get("comparison").is_none());
    drop(s);
    assert_eq!(fs::read(&files.path).map_err(io_error)?, bytes);
    Ok(())
}
