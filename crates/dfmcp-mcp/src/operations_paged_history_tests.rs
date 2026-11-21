//! Public operations-handler regressions over actual private journal files.
//! Native observations are scripted; journal framing, replay, projection,
//! capability checks, result shaping and public handlers are the real code.

use super::*;
use dfmcp_adapter::live_jobs::LiveJobObservation;
use dfmcp_adapter::live_operations::{LiveItem, item_entity_id};
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::{GameTick, MapCoord};
use std::collections::VecDeque;
use std::fs;
use std::io::{Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(1);

fn fixture_error(message: &str) -> DfmcpError {
    error(ErrorCode::InternalInvariantViolation, message)
}

fn io<T>(result: std::io::Result<T>) -> Result<T> {
    result.map_err(|failure| {
        DfmcpError::new(
            ErrorCode::InternalInvariantViolation,
            format!("paged-history fixture I/O: {failure}"),
        )
    })
}

fn decode(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| fixture_error("paged-history response JSON"))
}

fn observation(items: u32, tick: u32) -> LiveOperationsObservation {
    LiveOperationsObservation {
        jobs: LiveJobObservation {
            bridge_generation: 7,
            df_version: "history-test-df".to_owned(),
            dfhack_version: "history-test-dfhack".to_owned(),
            year: 105,
            year_tick: tick,
            paused: true,
            site_id: 1,
            world_folder: "paged-history-fortress".to_owned(),
            next_job_id: 0,
            jobs: Vec::new(),
        },
        next_building_id: 0,
        next_item_id: items,
        buildings: Vec::new(),
        attachments: Vec::new(),
        items: (0..items)
            .map(|native_id| LiveItem {
                native_id,
                item_type: 1,
                type_key: "BAR".to_owned(),
                subtype: -1,
                material_type: 0,
                material_index: 1,
                stack_size: 1,
                raw_position: MapCoord::new(0, 0, 0),
                flags: 64,
                container_native_id: None,
                holder_building_native_id: None,
            })
            .collect(),
    }
}

struct Script {
    pending: VecDeque<LiveOperationsObservation>,
    calls: Arc<AtomicUsize>,
    fenced: bool,
}

impl OperationsSource for Script {
    fn read(&mut self, _: Duration) -> Result<LiveOperationsObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.pending.pop_front().ok_or_else(|| {
            error(
                ErrorCode::AdapterUnavailable,
                "paged-history native fixture exhausted",
            )
        })
    }

    fn poisoned(&self) -> bool {
        self.fenced
    }

    fn fence(&mut self) {
        self.fenced = true;
    }
}

struct Registered {
    id: SessionId,
    calls: Arc<AtomicUsize>,
}

impl Registered {
    fn handle(&self) -> Option<String> {
        Some(self.id.to_string())
    }

    fn query_raw(&self, query: Value) -> String {
        fortress_query(
            self.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query})),
        )
    }

    fn query(&self, query: Value) -> Result<Value> {
        decode(&self.query_raw(query))
    }

    fn history(&self) -> Result<Value> {
        self.query(json!({"kind":"history","limit":64}))
    }

    fn observe(&self) -> Result<Value> {
        decode(&fortress_observe(self.handle()))
    }

    fn anchor(&self) -> Result<StateAnchor> {
        let handle = resolve(self.handle())?;
        lock(&handle)?.anchor()
    }

    fn detach(&self) -> Result<()> {
        lock(&SESSIONS)?.remove(&self.id);
        Ok(())
    }

    fn attach(&self, path: &Path, repair: TailRecovery) -> Result<()> {
        let handle = resolve(self.handle())?;
        let mut session = lock(&handle)?;
        let context = session.context()?;
        history::attach(&mut session, path, repair, &context)
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.id);
        }
    }
}

fn register(
    profile: OperationsProfile,
    initial: LiveOperationsObservation,
    pending: Vec<LiveOperationsObservation>,
    tokens: u32,
) -> Result<Registered> {
    let serial = NEXT_FIXTURE.fetch_add(1, Ordering::SeqCst) as u128;
    let family = match profile {
        OperationsProfile::V1_3 => 2_u128,
        OperationsProfile::PagedV1_4 => 3_u128,
    };
    let id = SessionId::new((1_u128 << 127) | (family << 60) | (1_u128 << 56) | serial);
    let mut state = LiveOperationsState::with_profile(profile);
    state.publish(initial)?;
    let fortress_id = state
        .snapshot()
        .ok_or_else(|| fixture_error("paged-history initial snapshot"))?
        .fortress_id;
    let limits = OperationsLimits {
        jobs: 4096,
        buildings: 4096,
        items: if profile == OperationsProfile::V1_3 {
            32768
        } else {
            65536
        },
        payload_bytes: profile.maximum_bytes(),
    };
    let calls = Arc::new(AtomicUsize::new(0));
    let session = OperationsSession {
        id,
        source: Box::new(Script {
            pending: pending.into(),
            calls: Arc::clone(&calls),
            fenced: false,
        }),
        state,
        journal: None,
        limits,
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_game_ticks: 1_000_000,
            max_entities: limits.entity_limit(),
            max_bytes: limits.payload_bytes as u64,
            max_output_tokens: tokens,
            max_actions: 1,
        },
        grants: [Capability::Observe, Capability::Query, Capability::Doctor]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        request: 0,
        _slot: SessionSlot::reserve()?,
    };
    assert!(
        lock(&SESSIONS)?
            .insert(id, Arc::new(Mutex::new(session)))
            .is_none()
    );
    Ok(Registered { id, calls })
}

struct Archive {
    session: Registered,
    path: PathBuf,
    directory: PathBuf,
}

impl Drop for Archive {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.session.id);
        }
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}

fn archived(
    profile: OperationsProfile,
    initial: LiveOperationsObservation,
    pending: Vec<LiveOperationsObservation>,
    tokens: u32,
) -> Result<Archive> {
    let session = register(profile, initial, pending, tokens)?;
    let directory = std::env::temp_dir().join(format!(
        "dfmcp-paged-history-{}-{}",
        std::process::id(),
        session.id
    ));
    io(fs::create_dir(&directory))?;
    io(fs::set_permissions(
        &directory,
        fs::Permissions::from_mode(0o700),
    ))?;
    let directory = io(directory.canonicalize())?;
    let path = directory.join("observations.bin");
    let archive = Archive {
        session,
        path,
        directory,
    };
    archive
        .session
        .attach(&archive.path, TailRecovery::Refuse)?;
    Ok(archive)
}

fn past(row: &Value, query: Value) -> Value {
    json!({
        "kind":"historical_query", "record":row["record"],
        "record_digest":row["record_digest"], "query":query,
    })
}

fn inspect(generation: u32) -> Value {
    json!({"kind":"inspect","entity_id":item_entity_id(0).to_string(),
        "generation":generation,"fields":["stack_size","forbidden"]})
}

#[test]
fn paged_archive_retains_a_roster_above_the_single_frame_profile_limit() -> Result<()> {
    let _serial = crate::test_serial();
    let first = observation(40_000, 3);
    assert!(first.encode_payload().is_err());
    let fixture = archived(OperationsProfile::PagedV1_4, first, Vec::new(), 2048)?;
    let listing = fixture.session.history()?;
    assert_eq!(listing["ok"], true, "{listing}");
    assert_eq!(listing["matched"], 1);
    assert_eq!(&io(fs::read(&fixture.path))?[..8], b"DFMPJ001");
    let raw = fixture
        .session
        .query_raw(past(&listing["rows"][0], inspect(1)));
    assert!(raw.len() <= 8192, "large source must retain bounded output");
    let result = decode(&raw)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["historical"], true);
    assert_eq!(result["row"]["fields"]["stack_size"]["value"]["value"], 1);
    assert_eq!(result["agent_turn"]["briefing"]["bridge_protocol"], "1.4");
    assert_eq!(
        result["agent_turn"]["coverage"]["current_freshness_proven"],
        false
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn historical_entity_pages_preserve_source_allowance_and_current_state_for_both_profiles()
-> Result<()> {
    let _serial = crate::test_serial();
    for profile in [OperationsProfile::V1_3, OperationsProfile::PagedV1_4] {
        let first = observation(120, 3);
        let mut second = first.clone();
        second.jobs.year_tick = 4;
        second.items[0].stack_size = 9;
        let fixture = archived(profile, first, vec![second], 2048)?;
        let first_row = fixture.session.history()?["rows"][0].clone();
        assert_eq!(fixture.session.observe()?["ok"], true);
        let current = fixture.session.anchor()?;
        let request = past(
            &first_row,
            json!({"kind":"entities","kinds":["item"],
            "fields":["stack_size"],"limit":1}),
        );
        let raw = fixture.session.query_raw(request.clone());
        assert!(raw.len() <= 8192);
        let result = decode(&raw)?;
        assert_eq!(result["ok"], true, "profile {profile:?}: {result}");
        assert_eq!(result["matched"], 120);
        assert_eq!(
            result["rows"][0]["fields"]["stack_size"]["value"]["value"],
            1
        );
        assert_eq!(result["anchor"], first_row["anchor"]);
        assert_eq!(result["current_live_anchor"], anchor_json(current));
        assert_eq!(result["agent_turn"]["anchor"], first_row["anchor"]);
        assert_eq!(result["agent_turn"]["continuity"]["status"], "partial");
        assert_eq!(
            result["agent_turn"]["briefing"]["bridge_protocol"],
            profile.protocol()
        );
        assert!(result["continuation"].is_string());
        let mut continued = request;
        continued["query"]["continuation"] = result["continuation"].clone();
        let page_two = fixture.session.query(continued)?;
        assert_eq!(page_two["ok"], true, "{page_two}");
        assert_ne!(
            page_two["rows"][0]["entity_id"],
            result["rows"][0]["entity_id"]
        );
        assert_eq!(fixture.session.anchor()?, current);
        let live = fixture.session.query(inspect(1))?;
        assert_eq!(live["row"]["fields"]["stack_size"]["value"]["value"], 9);
        assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
fn paged_reopen_preserves_retirement_generations_and_rejects_old_session_records() -> Result<()> {
    let _serial = crate::test_serial();
    let first = observation(2, 3);
    let mut removed = first.clone();
    removed.jobs.year_tick = 4;
    removed.items.remove(0);
    let mut returned = first.clone();
    returned.jobs.year_tick = 5;
    returned.items[0].stack_size = 7;
    let mut fixture = archived(
        OperationsProfile::PagedV1_4,
        first,
        vec![removed, returned.clone()],
        8192,
    )?;
    let watch = fixture
        .session
        .query(json!({"kind":"watch","key":"prior-owner",
        "label":"Old session monitor","condition":{"op":"tick_at_least","value":105u64*403200+100},
        "deadline_tick":105u64*403200+1000,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(watch["ok"], true, "{watch}");
    assert_eq!(fixture.session.observe()?["ok"], true);
    assert_eq!(fixture.session.observe()?["ok"], true);
    let before = fixture.session.history()?;
    assert_eq!(before["matched"], 3);
    assert_eq!(fixture.session.query(inspect(2))?["ok"], true);
    let old_session = fixture.session.id;
    let stale = resolve(fixture.session.handle())?;
    let before_close = io(fs::read(&fixture.path))?;
    let closed = decode(&fortress_cancel(
        fixture.session.handle(),
        Some("session".into()),
        Some(true),
    ))?;
    assert_eq!(closed["ok"], true, "{closed}");
    assert_eq!(
        closed["process_local_resources"]["process_local_watch_records_discarded"],
        1
    );
    assert_eq!(io(fs::read(&fixture.path))?, before_close);
    assert!(matches!(lock(&stale)?.context(), Err(e) if e.code == ErrorCode::SessionNotFound));
    let reopened = register(OperationsProfile::PagedV1_4, returned, Vec::new(), 8192)?;
    reopened.attach(&fixture.path, TailRecovery::Refuse)?;
    fixture.session = reopened;
    let after = fixture.session.history()?;
    assert_eq!(after["rows"], before["rows"]);
    assert_eq!(after["history"]["head"], before["history"]["head"]);
    assert_eq!(after["anchor"], before["anchor"]);
    assert_ne!(fixture.session.id, old_session);
    assert!(resolve(Some(old_session.to_string())).is_err());
    assert_eq!(fixture.session.query(inspect(2))?["ok"], true);
    assert_eq!(fixture.session.query(inspect(1))?["ok"], false);
    assert_eq!(
        fixture.session.query(past(&after["rows"][0], inspect(1)))?["ok"],
        true
    );
    assert_eq!(
        fixture.session.query(past(&after["rows"][2], inspect(2)))?["ok"],
        true
    );
    assert_eq!(
        fixture
            .session
            .query(json!({"kind":"poll_watch","watch":watch["record"]["watch"]}))?["ok"],
        false
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn operations_close_preflights_the_receipt_then_releases_the_unchanged_archive() -> Result<()> {
    let _serial = crate::test_serial();
    for profile in [OperationsProfile::V1_3, OperationsProfile::PagedV1_4] {
        let first = observation(2, 3);
        let fixture = archived(profile, first.clone(), Vec::new(), 8192)?;
        let before = fixture.session.history()?;
        let bytes = io(fs::read(&fixture.path))?;
        let stale = resolve(fixture.session.handle())?;
        let context = lock(&stale)?.context()?;
        lock(&stale)?.budget.max_output_tokens = 1;
        let refused = decode(&fortress_cancel(
            fixture.session.handle(),
            Some("session".into()),
            None,
        ))?;
        lock(&stale)?.budget.max_output_tokens = 2048;
        assert_eq!(refused["error"]["code"], "budget_exceeded");
        assert!(
            history::Journal::open(profile, &fixture.path, &context, TailRecovery::Refuse).is_err()
        );
        assert!(lock(&stale)?.journal.is_some());
        assert!(!lock(&stale)?.source.closed());
        assert_eq!(io(fs::read(&fixture.path))?, bytes);
        let closed = decode(&fortress_cancel(
            fixture.session.handle(),
            Some("session".into()),
            None,
        ))?;
        assert_eq!(closed["ok"], true, "{closed}");
        assert_eq!(closed["observation_journal_present"], true);
        assert_eq!(closed["observation_journal_modified"], false);
        assert_eq!(io(fs::read(&fixture.path))?, bytes);
        assert!(lock(&stale)?.journal.is_none());
        let reopened = register(profile, first, Vec::new(), 8192)?;
        reopened.attach(&fixture.path, TailRecovery::Refuse)?;
        assert_eq!(reopened.history()?["rows"], before["rows"]);
        assert_eq!(io(fs::read(&fixture.path))?, bytes);
        assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
        assert_eq!(reopened.calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            decode(&fortress_cancel(
                reopened.handle(),
                Some("session".into()),
                None
            ))?["ok"],
            true
        );
    }
    Ok(())
}

#[test]
fn historical_tick_neither_revokes_nor_renews_current_query_authority() -> Result<()> {
    let _serial = crate::test_serial();
    for profile in [OperationsProfile::V1_3, OperationsProfile::PagedV1_4] {
        let first = observation(2, 50);
        let mut reset = first.clone();
        reset.jobs.year_tick = 3;
        reset.jobs.bridge_generation += 1;
        let fixture = archived(profile, first, vec![reset], 8192)?;
        let row = fixture.session.history()?["rows"][0].clone();
        let refreshed = fixture.session.observe()?;
        assert_eq!(refreshed["ok"], true, "{refreshed}");
        let current = fixture.session.anchor()?;
        assert!(current.tick.0 < row["anchor"]["game_tick"].as_u64().unwrap_or(0));
        {
            let handle = resolve(fixture.session.handle())?;
            let mut session = lock(&handle)?;
            for grant in &mut session.grants {
                if grant.capability == Capability::Query {
                    grant.expires_at_tick = Some(GameTick(current.tick.0 + 1));
                }
            }
            session
                .grants
                .retain(|grant| grant.capability != Capability::Observe);
        }
        let result = fixture.session.query(past(&row, inspect(1)))?;
        assert_eq!(
            result["ok"], true,
            "current authority must govern {profile:?}: {result}"
        );
        assert_eq!(result["anchor"], row["anchor"]);
        assert_eq!(
            fixture.session.observe()?["error"]["code"],
            "capability_denied"
        );
        {
            let handle = resolve(fixture.session.handle())?;
            let mut session = lock(&handle)?;
            for grant in &mut session.grants {
                if grant.capability == Capability::Query {
                    grant.expires_at_tick = Some(GameTick(current.tick.0 - 1));
                }
            }
        }
        let denied = fixture.session.query(past(&row, inspect(1)))?;
        assert_eq!(denied["error"]["code"], "capability_denied");
        assert!(denied.get("row").is_none());
        assert!(denied.get("anchor").is_none());
        assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
fn paged_history_pages_bind_the_current_head_and_preserve_unsampled_watch_state() -> Result<()> {
    let _serial = crate::test_serial();
    let first = observation(2, 3);
    let pending = (4..=16).map(|tick| observation(2, tick)).collect();
    let fixture = archived(OperationsProfile::PagedV1_4, first, pending, 2048)?;
    let watch = fixture.session.query(json!({"kind":"watch","key":"history-pages",
        "label":"Current unsampled work","condition":{"op":"tick_at_least","value":105u64*403200+100},
        "deadline_tick":105u64*403200+1000,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(watch["ok"], true, "{watch}");
    for _ in 0..12 {
        assert_eq!(fixture.session.observe()?["ok"], true);
    }
    let work_before = fixture.session.query(json!({"kind":"watches"}))?;
    let mut input = json!({"kind":"history","limit":64});
    let mut records = Vec::new();
    let mut stale_cursor = None;
    for _ in 0..13 {
        let raw = fixture.session.query_raw(input.clone());
        assert!(raw.len() <= 8192);
        let result = decode(&raw)?;
        assert_eq!(result["ok"], true, "{result}");
        assert_eq!(
            result["agent_turn"]["active_work"]["obligations"][0]["watch"],
            watch["record"]["watch"]
        );
        for row in result["rows"]
            .as_array()
            .ok_or_else(|| fixture_error("history page rows"))?
        {
            records.push(
                row["record"]
                    .as_u64()
                    .ok_or_else(|| fixture_error("record ordinal"))?,
            );
        }
        if result["continuation"].is_null() {
            break;
        }
        if stale_cursor.is_none() {
            stale_cursor = Some(result["continuation"].clone());
        }
        input["continuation"] = result["continuation"].clone();
        input["limit"] = json!(3);
    }
    assert_eq!(records, (1..=13).collect::<Vec<_>>());
    assert!(stale_cursor.is_some());
    let work_after = fixture.session.query(json!({"kind":"watches"}))?;
    assert_eq!(work_after["records"], work_before["records"]);
    assert_eq!(fixture.session.observe()?["ok"], true);
    assert_eq!(
        fixture
            .session
            .query(json!({"kind":"history","continuation":stale_cursor}))?["error"]["code"],
        "stale_anchor"
    );
    assert_eq!(
        fixture
            .session
            .query(json!({"kind":"cancel_watch","watch":watch["record"]["watch"]}))?["ok"],
        true
    );
    assert_eq!(
        fixture
            .session
            .query(json!({"kind":"release_watch","watch":watch["record"]["watch"]}))?["ok"],
        true
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 13);
    Ok(())
}

#[test]
fn paged_history_survives_native_failure_without_refresh_or_retained_work_mutation() -> Result<()> {
    let _serial = crate::test_serial();
    let fixture = archived(
        OperationsProfile::PagedV1_4,
        observation(2, 3),
        Vec::new(),
        8192,
    )?;
    let row = fixture.session.history()?["rows"][0].clone();
    let anchor = fixture.session.anchor()?;
    assert_eq!(fixture.session.observe()?["ok"], false);
    assert_eq!(fixture.session.history()?["source_stale"], true);
    let result = fixture.session.query(past(&row, inspect(1)))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["agent_turn"]["briefing"]["live_source_fenced"], true);
    assert_eq!(result["agent_turn"]["briefing"]["live"], false);
    assert_eq!(result["agent_turn"]["briefing"]["bridge_protocol"], "1.4");
    for forbidden in [
        "watch",
        "poll_watch",
        "capture",
        "changes",
        "await_watch",
        "historical_query",
    ] {
        assert_eq!(
            fixture
                .session
                .query(past(&row, json!({"kind":forbidden})))?["error"]["code"],
            "invalid_request"
        );
    }
    assert_eq!(
        fixture.session.query(inspect(1))?["error"]["code"],
        "adapter_unavailable"
    );
    assert_eq!(
        decode(&fortress_commit(fixture.session.handle()))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(fixture.session.anchor()?, anchor);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn archives_cannot_cross_fixed_profiles_even_with_operator_tail_repair() -> Result<()> {
    let _serial = crate::test_serial();
    for (written, attempted) in [
        (OperationsProfile::V1_3, OperationsProfile::PagedV1_4),
        (OperationsProfile::PagedV1_4, OperationsProfile::V1_3),
    ] {
        let source = observation(2, 3);
        let fixture = archived(written, source.clone(), Vec::new(), 8192)?;
        fixture.session.detach()?;
        let before = io(fs::read(&fixture.path))?;
        let candidate = register(attempted, source, Vec::new(), 8192)?;
        let original_anchor = candidate.anchor()?;
        for repair in [TailRecovery::Refuse, TailRecovery::TruncateIncomplete] {
            let result = candidate.attach(&fixture.path, repair);
            assert!(matches!(result, Err(ref failure) if failure.code == ErrorCode::CorruptLedger));
            assert_eq!(candidate.anchor()?, original_anchor);
            let handle = resolve(candidate.handle())?;
            assert!(lock(&handle)?.journal.is_none());
            assert_eq!(io(fs::read(&fixture.path))?, before);
        }
        assert_eq!(candidate.calls.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn paged_journal_torn_tail_repair_is_explicit_and_reconstructs_the_original_chain() -> Result<()> {
    let _serial = crate::test_serial();
    let first = observation(2, 3);
    let second = observation(2, 4);
    let mut fixture = archived(
        OperationsProfile::PagedV1_4,
        first,
        vec![second.clone()],
        8192,
    )?;
    assert_eq!(fixture.session.observe()?["ok"], true);
    let listing = fixture.session.history()?;
    let complete = io(fs::read(&fixture.path))?;
    fixture.session.detach()?;
    let file = io(fs::OpenOptions::new().write(true).open(&fixture.path))?;
    io(file.set_len(complete.len() as u64 - 1))?;
    io(file.sync_all())?;
    drop(file);
    let torn = io(fs::read(&fixture.path))?;
    let reopened = register(OperationsProfile::PagedV1_4, second, Vec::new(), 8192)?;
    assert!(
        matches!(reopened.attach(&fixture.path, TailRecovery::Refuse), Err(ref failure)
        if failure.code == ErrorCode::CorruptLedger)
    );
    assert_eq!(io(fs::read(&fixture.path))?, torn);
    reopened.attach(&fixture.path, TailRecovery::TruncateIncomplete)?;
    fixture.session = reopened;
    let repaired = fixture.session.history()?;
    assert_eq!(repaired["rows"], listing["rows"]);
    assert_eq!(repaired["anchor"], listing["anchor"]);
    assert!(
        repaired["history"]["repaired_tail_bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes > 0)
    );
    assert_eq!(io(fs::read(&fixture.path))?, complete);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn paged_corrupt_history_fences_live_source_without_publishing_archived_rows() -> Result<()> {
    let _serial = crate::test_serial();
    let fixture = archived(
        OperationsProfile::PagedV1_4,
        observation(2, 3),
        vec![observation(2, 4)],
        8192,
    )?;
    let row = fixture.session.history()?["rows"][0].clone();
    let before = fixture.session.anchor()?;
    let mut file = io(fs::OpenOptions::new().write(true).open(&fixture.path))?;
    io(file.seek(SeekFrom::End(-1)))?;
    io(file.write_all(b"!"))?;
    io(file.sync_all())?;
    drop(file);
    let corrupt_bytes = io(fs::read(&fixture.path))?;
    let result = fixture.session.query(past(&row, inspect(1)))?;
    assert_eq!(result["error"]["code"], "corrupt_ledger");
    assert!(result.get("row").is_none());
    assert!(result.get("historical").is_none());
    assert_eq!(
        fixture.session.observe()?["error"]["code"],
        "adapter_unavailable"
    );
    assert_eq!(fixture.session.anchor()?, before);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 0);
    fixture.session.detach()?;
    let reopened = register(
        OperationsProfile::PagedV1_4,
        observation(2, 3),
        Vec::new(),
        8192,
    )?;
    assert!(
        matches!(reopened.attach(&fixture.path, TailRecovery::TruncateIncomplete), Err(ref failure)
        if failure.code == ErrorCode::CorruptLedger)
    );
    assert_eq!(io(fs::read(&fixture.path))?, corrupt_bytes);
    Ok(())
}

#[test]
fn paged_journal_custody_failure_preserves_last_published_anchor_and_bytes() -> Result<()> {
    let _serial = crate::test_serial();
    let fixture = archived(
        OperationsProfile::PagedV1_4,
        observation(2, 3),
        vec![observation(2, 4)],
        8192,
    )?;
    let before = fixture.session.anchor()?;
    let bytes = io(fs::read(&fixture.path))?;
    io(fs::set_permissions(
        &fixture.path,
        fs::Permissions::from_mode(0o400),
    ))?;
    let failure = fixture.session.observe()?;
    assert_eq!(failure["error"]["code"], "corrupt_ledger");
    assert_eq!(fixture.session.anchor()?, before);
    assert_eq!(io(fs::read(&fixture.path))?, bytes);
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    io(fs::set_permissions(
        &fixture.path,
        fs::Permissions::from_mode(0o600),
    ))?;
    assert_eq!(
        fixture.session.observe()?["error"]["code"],
        "adapter_unavailable"
    );
    assert_eq!(fixture.session.calls.load(Ordering::SeqCst), 1);
    Ok(())
}
