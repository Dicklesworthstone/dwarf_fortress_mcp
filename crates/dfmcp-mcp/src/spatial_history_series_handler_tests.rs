//! Actual MCP query handlers over injected captures and real private journals.
use super::super::*;
#[path = "../../dfmcp-adapter/tests/support/production_spatial.rs"]
mod fixture;
use dfmcp_adapter::operations_journal::TailRecovery;
use dfmcp_core::GameTick;
use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError {
    error(ErrorCode::CorruptLedger, "timeline fixture I/O")
}
struct Files {
    directory: PathBuf,
    observations: PathBuf,
    watches: PathBuf,
}
impl Files {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir()
            .canonicalize()
            .map_err(io_error)?
            .join(format!(
                "dfmcp-resource-series-{}-{}",
                std::process::id(),
                FILE_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(io_error)?;
        Ok(Self {
            observations: directory.join("observations.bin"),
            watches: directory.join("watches.bin"),
            directory,
        })
    }
}
impl Drop for Files {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.watches);
        let _ = fs::remove_file(&self.observations);
        let _ = fs::remove_dir(&self.directory);
    }
}
struct Script {
    values: VecDeque<LiveSpatialCitizenObservation>,
    calls: Arc<AtomicUsize>,
    fenced: bool,
}
impl Source for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "timeline fixture exhausted"))
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
    fn pages(&self) -> u32 {
        1
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
}
impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.id);
        }
    }
}
fn decode(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| invalid("timeline fixture JSON"))
}
fn ok(value: Value) -> Value {
    assert_eq!(value["ok"], true, "{value}");
    value
}
fn ask(s: &Registered, query: Value) -> Result<Value> {
    decode(&fortress_query(
        s.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":query})),
    ))
}
fn measure() -> Value {
    json!({"kind":"item_quantity","scope":"observed_projection",
    "quantity_unit":"stack_units","predicate":{"op":"always"}})
}
fn register(files: &Files, next: &[(u32, u32)]) -> Result<Registered> {
    let first = fixture::observation(3, 1, 10, false)?;
    let limits = CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits::default(),
            region: first.spatial().terrain().map.region,
        },
        citizens: 4096,
    };
    let mut state = LiveSpatialCitizenState::default();
    state.publish(first)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| invalid("fixture state"))?
        .anchor();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut session = Session {
        id: next_id()?,
        source: Box::new(Script {
            fenced: false,
            calls: calls.clone(),
            values: next
                .iter()
                .map(|&(tick, units)| fixture::observation(tick, 1, units, false))
                .collect::<Result<_>>()?,
        }),
        state,
        limits,
        journal: None,
        budget: WorkBudget {
            max_entities: limits.entity_limit(),
            max_bytes: 1024 * 1024,
            max_output_tokens: 65536,
            max_wall_millis: 60000,
            ..WorkBudget::default()
        },
        grants: [Capability::Query, Capability::Observe, Capability::Doctor]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope {
                    fortress_id: Some(anchor.fortress_id),
                    ..CapabilityScope::default()
                },
                max_risk: RiskTier::ReadOnly,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
        request: 0,
        _watch_journal: None,
        _slot: Slot::reserve()?,
    };
    let mut c = session.context()?;
    history::attach(&mut session, &files.observations, TailRecovery::Refuse, &c)?;
    c.anchor = session.anchor()?;
    durable_watches::finish_open(&mut session, &c, Some(&files.watches), json!({"ok":true}))?;
    let id = session.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered { id, calls })
}
fn capture(s: &Registered) -> Result<()> {
    ok(decode(&fortress_observe(s.handle()))?);
    Ok(())
}
fn timeline(s: &Registered) -> Result<Value> {
    let history = ok(ask(s, json!({"kind":"history","limit":64}))?);
    let rows = history["rows"]
        .as_array()
        .ok_or_else(|| invalid("fixture history"))?;
    let first = rows
        .first()
        .ok_or_else(|| invalid("empty fixture history"))?;
    let last = rows
        .last()
        .ok_or_else(|| invalid("empty fixture history"))?;
    Ok(
        json!({"kind":"historical_series","from":{"record":first["record"],"record_digest":first["record_digest"]},
        "to":{"record":last["record"],"record_digest":last["record_digest"]},"measurement":measure(),"limit":32}),
    )
}
fn watch(s: &Registered) -> Result<()> {
    let tick = {
        let h = resolve(s.handle())?;
        let session = lock(&h)?;
        session.anchor()?.tick.0
    };
    ok(ask(
        s,
        json!({"kind":"watch","key":"timeline-preserves-watch","condition":{"op":"paused","value":true},
        "deadline_tick":tick+100,"stable_observations":64}),
    )?);
    Ok(())
}

#[test]
fn series_matches_exact_individual_measurements_and_leaves_watches_and_world_unchanged()
-> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7), (5, 12)])?;
    watch(&s)?;
    capture(&s)?;
    capture(&s)?;
    let query = timeline(&s)?;
    let before = ok(ask(&s, json!({"kind":"watches"}))?);
    let archive = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    let current = ok(ask(&s, measure())?);
    let result = ok(ask(&s, query)?);
    assert_eq!(result["returned"], 3);
    assert_eq!(result["historical"], true);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["current_freshness_proven"], false);
    assert_eq!(result["watch_evaluated"], false);
    let rows = result["rows"]
        .as_array()
        .ok_or_else(|| invalid("timeline rows"))?;
    assert!(rows[0]["change_from_previous"].is_null());
    assert_eq!(
        rows[1]["change_from_previous"]["net_change"]["minimum"],
        "-3"
    );
    assert_eq!(
        rows[2]["change_from_previous"]["net_change"]["minimum"],
        "5"
    );
    for row in rows {
        let record = &row["record"];
        let exact = ok(ask(
            &s,
            json!({"kind":"historical_query","record":record["record"],
            "record_digest":record["record_digest"],"query":measure()}),
        )?);
        assert_eq!(row["quantity"], exact["quantity"]);
        assert_eq!(row["evidence_digest"], exact["evidence_digest"]);
    }
    assert_eq!(ok(ask(&s, measure())?)["anchor"], current["anchor"]);
    assert_eq!(
        ok(ask(&s, json!({"kind":"watches"}))?)["records"],
        before["records"]
    );
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, archive);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn pages_preserve_cross_page_deltas_and_full_packet_budgets() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7), (5, 12), (6, 9)])?;
    for _ in 0..3 {
        capture(&s)?;
    }
    watch(&s)?;
    let mut query = timeline(&s)?;
    let complete = ok(ask(&s, query.clone())?);
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 4096;
    }
    query["limit"] = json!(1);
    let mut rows = Vec::new();
    let mut pages = 0;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query.clone()})),
        );
        assert!(raw.len() <= 16384);
        let result = ok(decode(&raw)?);
        let current = result["rows"]
            .as_array()
            .ok_or_else(|| invalid("paged rows"))?;
        assert!(!current.is_empty());
        rows.extend(current.iter().cloned());
        assert_eq!(
            result["anchor"],
            current.last().ok_or_else(|| invalid("last row"))?["record"]["anchor"]
        );
        assert_eq!(
            result["agent_turn"]["coverage"]["continuation"],
            result["continuation"]
        );
        assert_eq!(
            result["agent_turn"]["active_work"]["obligations"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        pages += 1;
        assert!(pages <= 4);
        if result["continuation"].is_null() {
            break;
        }
        query["continuation"] = result["continuation"].clone();
        query["limit"] = json!(2);
    }
    assert!(pages > 1);
    assert_eq!(Value::Array(rows), complete["rows"]);
    assert_eq!(s.calls.load(Ordering::SeqCst), 3);
    Ok(())
}

#[test]
fn measurement_range_and_archive_head_changes_reject_old_pages() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7), (5, 12)])?;
    capture(&s)?;
    let mut query = timeline(&s)?;
    query["limit"] = json!(1);
    let token = ok(ask(&s, query.clone())?)["continuation"].clone();
    assert!(token.is_string());
    query["continuation"] = token;
    let mut changed = query.clone();
    changed["measurement"]["predicate"] = json!({"op":"not","arg":{"op":"always"}});
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    capture(&s)?;
    assert_eq!(ask(&s, query)?["error"]["code"], "stale_anchor");
    Ok(())
}

#[test]
fn equal_tick_changes_and_reset_segments_never_invent_rates() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(3, 7), (2, 99)])?;
    capture(&s)?;
    capture(&s)?;
    let result = ok(ask(&s, timeline(&s)?)?);
    assert_eq!(
        result["rows"][1]["change_from_previous"]["rate_unavailable_reason"],
        "same_game_tick"
    );
    assert!(result["rows"][1]["change_from_previous"]["net_rate"].is_null());
    assert_eq!(
        result["rows"][2]["change_from_previous"]["status"],
        "epoch_or_clock_discontinuity"
    );
    assert!(result["rows"][2]["change_from_previous"]["net_change"].is_null());
    Ok(())
}

#[test]
fn offline_series_matches_live_history_and_rejects_old_session_continuations() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7)])?;
    capture(&s)?;
    let query = timeline(&s)?;
    let original = ok(ask(&s, query.clone())?);
    let mut paged = query.clone();
    paged["limit"] = json!(1);
    paged["continuation"] = ok(ask(&s, paged.clone())?)["continuation"].clone();
    let (limits, budget) = {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.source.fence();
        (session.limits, session.budget)
    };
    assert_eq!(ok(ask(&s, query.clone())?)["rows"], original["rows"]);
    drop(s);
    let bytes = fs::read(&files.observations).map_err(io_error)?;
    let watch_bytes = fs::read(&files.watches).map_err(io_error)?;
    let session = archive::open(
        next_id()?,
        Slot::reserve()?,
        &files.observations,
        limits,
        budget,
        &[Capability::Query],
    )?;
    let id = session.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    let s = Registered {
        id,
        calls: Arc::new(AtomicUsize::new(0)),
    };
    let result = ok(ask(&s, query)?);
    assert_eq!(result["rows"], original["rows"]);
    assert_eq!(result["archive_only"], true);
    assert_eq!(result["bridge_connection_present"], false);
    assert_eq!(ask(&s, paged)?["error"]["code"], "stale_anchor");
    let schema = ok(decode(&fortress_query(
        s.handle(),
        Some("schema".into()),
        None,
    ))?);
    let series = schema["query_schema"]["$defs"]["query"]["oneOf"]
        .as_array()
        .and_then(|v| {
            v.iter()
                .find(|v| v["properties"]["kind"]["const"] == "historical_series")
        })
        .ok_or_else(|| invalid("series schema"))?;
    let measurements = series["properties"]["measurement"]["oneOf"]
        .as_array()
        .ok_or_else(|| invalid("measurement variants"))?;
    assert_eq!(measurements.len(), 2);
    for kind in ["item_quantity", "condition_evaluation"] {
        assert!(
            measurements
                .iter()
                .any(|m| m["properties"]["kind"]["const"] == kind)
        );
    }
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, bytes);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watch_bytes);
    Ok(())
}

#[test]
fn failed_requests_do_not_sample_watches_or_revive_expired_current_grants() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7)])?;
    watch(&s)?;
    capture(&s)?;
    let query = timeline(&s)?;
    let bytes = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    for case in 0..6 {
        let mut bad = query.clone();
        match case {
            0 => bad["limit"] = json!(33),
            1 => bad["measurement"] = json!({"kind":"watch"}),
            2 => bad["from"]["record_digest"] = json!("00".repeat(32)),
            3 => bad["from"]["record"] = json!(4096),
            4 => bad["measurement"]["extra"] = json!(true),
            _ => bad["measurement"]["predicate"] = json!({"op":"field","field":"x"}),
        }
        assert_eq!(ask(&s, bad)?["ok"], false, "case={case}");
    }
    {
        let h = resolve(s.handle())?;
        lock(&h)?.budget.max_output_tokens = 1;
    }
    assert_eq!(ask(&s, query.clone())?["error"]["code"], "budget_exceeded");
    {
        let h = resolve(s.handle())?;
        let mut session = lock(&h)?;
        session.budget.max_output_tokens = 65536;
        for grant in &mut session.grants {
            grant.expires_at_tick = Some(GameTick(105u64 * 403200 + 3));
        }
    }
    assert_eq!(ask(&s, query)?["error"]["code"], "capability_denied");
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, bytes);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn same_size_prefix_corruption_cannot_produce_a_partial_timeline() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[(4, 7), (5, 12)])?;
    capture(&s)?;
    capture(&s)?;
    let mut query = timeline(&s)?;
    // Request a later start: replay must still verify the intervening prefix.
    let history = ok(ask(&s, json!({"kind":"history"}))?);
    query["from"] = json!({"record":3,"record_digest":history["rows"][2]["record_digest"]});
    let mut bytes = fs::read(&files.observations).map_err(io_error)?;
    let offset = {
        let h = resolve(s.handle())?;
        let session = lock(&h)?;
        session
            .journal
            .as_ref()
            .ok_or_else(|| invalid("archive"))?
            .entries()[1]
            .offset as usize
            + 50
    };
    bytes[offset] ^= 1;
    fs::write(&files.observations, &bytes).map_err(io_error)?;
    let result = ask(&s, query)?;
    assert_eq!(result["error"]["code"], "corrupt_ledger");
    assert!(result.get("rows").is_none());
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, bytes);
    let h = resolve(s.handle())?;
    assert!(lock(&h)?.source.poisoned());
    Ok(())
}

#[test]
fn schema_and_runtime_keep_series_out_of_recursive_historical_measurements() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, &[])?;
    let query = timeline(&s)?;
    let schema = ok(decode(&fortress_query(
        s.handle(),
        Some("schema".into()),
        None,
    ))?);
    assert!(
        schema["query_schema"]["$defs"]["query"]["oneOf"]
            .as_array()
            .is_some_and(|v| v
                .iter()
                .any(|v| v["properties"]["kind"]["const"] == "historical_series"))
    );
    let mut nested = query.clone();
    nested["measurement"] = query.clone();
    assert_eq!(ask(&s, nested)?["ok"], false);
    assert_eq!(
        ask(
            &s,
            json!({"kind":"historical_query","record":query["from"]["record"],
        "record_digest":query["from"]["record_digest"],"query":query})
        )?["ok"],
        false
    );
    Ok(())
}

#[path = "spatial_condition_history_tests.rs"]
mod condition_tests;
