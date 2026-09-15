use super::*;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use dfmcp_adapter::operations_journal::TailRecovery;

static NEXT_ARCHIVE: AtomicUsize = AtomicUsize::new(1);
fn io_error(_: std::io::Error) -> DfmcpError { error(ErrorCode::CorruptLedger, "test archive I/O") }
struct Archive { directory: PathBuf, path: PathBuf }
impl Archive {
    fn new() -> Result<Self> {
        let n = NEXT_ARCHIVE.fetch_add(1, Ordering::SeqCst);
        let directory = std::env::temp_dir().join(format!("dfmcp-spatial-history-{}-{n}", std::process::id()));
        fs::create_dir(&directory).map_err(io_error)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
        let directory = directory.canonicalize().map_err(io_error)?;
        Ok(Self { path: directory.join("observations.bin"), directory })
    }
}
impl Drop for Archive {
    fn drop(&mut self) { let _ = fs::remove_file(&self.path); let _ = fs::remove_dir(&self.directory); }
}
fn attach(s: &Registered, path: &Path) -> Result<()> {
    let handle = resolve(s.handle())?; let mut session = lock(&handle)?;
    session.budget.max_wall_millis = 60_000;
    let c = session.context()?;
    history::attach(&mut session, path, TailRecovery::Refuse, &c)
}
fn first_record(s: &Registered) -> Result<Value> {
    let result = ask(s, json!({"kind":"history","limit":1}))?;
    assert_eq!(result["ok"], true, "{result}");
    result["rows"].as_array().and_then(|rows| rows.first()).cloned()
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "test archive missing first row"))
}
fn archived(row: &Value, query: Value) -> Value {
    json!({"kind":"historical_query","record":row["record"],"record_digest":row["record_digest"],"query":query})
}
fn watch(s: &Registered) -> Result<Value> {
    let result = ask(s, json!({"kind":"watch","key":"archive-review","label":"Wait for later live capture",
        "condition":{"op":"tick_at_least","value":105u64*403200+50},
        "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":1}))?;
    assert_eq!(result["ok"], true, "{result}"); Ok(result["record"]["watch"].clone())
}
fn release_watch(s: &Registered, handle: Value) -> Result<()> {
    assert_eq!(ask(s, json!({"kind":"cancel_watch","watch":handle}))?["ok"], true);
    assert_eq!(ask(s, json!({"kind":"release_watch","watch":handle}))?["ok"], true); Ok(())
}

#[test]
fn archived_inventory_and_its_route_use_past_terrain_without_advancing_current_work() -> Result<()> {
    let _serial = lock(&SERIAL)?; let archive = Archive::new()?; let s = full()?;
    attach(&s, &archive.path)?; let record = first_record(&s)?;
    let current = decode(&fortress_observe(s.handle()))?; assert_eq!(current["ok"], true);
    assert_eq!(ask(&s, allocation(1,8))?["model_feasible"], false);
    let w = watch(&s)?; let work = ask(&s, json!({"kind":"watches"}))?;
    let history = ask(&s, archived(&record, allocation(1,8)))?;
    assert_eq!(history["ok"], true, "{history}"); assert_eq!(history["model_feasible"], true);
    assert_eq!(history["anchor"], record["anchor"]); assert_eq!(history["current_live_anchor"], current["anchor"]);
    assert_eq!(history["source_digest"], record["source_digest"]);
    assert_eq!(history["agent_turn"]["coverage"]["current_freshness_proven"], false);
    assert_eq!(history["agent_turn"]["briefing"]["active_work_basis"], "current_session_not_archived");
    let drill = history["rows"][0]["route_query"].clone();
    assert_eq!(drill["query"]["kind"], "historical_query"); assert_eq!(drill["query"]["record_digest"], record["record_digest"]);
    let route = decode(&fortress_query(s.handle(),None,Some(drill)))?;
    assert_eq!(route["ok"], true, "{route}"); assert_eq!(route["status"], "candidate_found");
    assert_eq!(route["anchor"], record["anchor"]); assert_eq!(route["source_digest"], record["source_digest"]);
    assert_eq!(route["unit_path_proven"], false); assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    let after = ask(&s, json!({"kind":"watches"}))?;
    assert_eq!(after["rows"], work["rows"]); assert_eq!(after["agent_turn"]["active_work"], work["agent_turn"]["active_work"]);
    assert_eq!(after["agent_turn"]["anchor"], current["anchor"]);
    release_watch(&s, w)?; Ok(())
}

#[test]
fn reopened_archive_restores_source_history_not_session_authority() -> Result<()> {
    let _serial = lock(&SERIAL)?; let archive = Archive::new()?;
    let (record, expected, source, old_handle) = {
        let s = full()?; attach(&s, &archive.path)?; let record = first_record(&s)?;
        assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], true);
        let handle = resolve(s.handle())?; let session = lock(&handle)?;
        let expected = session.anchor()?;
        let source = session.state.observation().cloned().ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "test source"))?;
        (record, expected, source, s.handle())
    };
    assert!(resolve(old_handle).is_err());
    let s = full()?;
    {
        let handle = resolve(s.handle())?; let mut session = lock(&handle)?;
        let mut state = LiveSpatialState::default(); state.publish(source)?; session.state = state;
    }
    attach(&s, &archive.path)?;
    let listed = ask(&s, json!({"kind":"history"}))?;
    assert_eq!(listed["ok"], true); assert_eq!(listed["matched"], 2);
    assert_eq!(listed["agent_turn"]["anchor"], anchor_json(expected));
    let previous = ask(&s, archived(&record, allocation(1,8)))?;
    assert_eq!(previous["ok"], true); assert_eq!(previous["model_feasible"], true);
    assert_eq!(previous["anchor"], record["anchor"]); assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(previous["agent_turn"]["active_work"]["obligations"], json!([])); Ok(())
}

#[test]
fn source_failure_leaves_verified_archive_reads_available() -> Result<()> {
    let _serial = lock(&SERIAL)?; let archive = Archive::new()?; let s = full()?;
    attach(&s, &archive.path)?; let record = first_record(&s)?;
    let current = decode(&fortress_observe(s.handle()))?; assert_eq!(current["ok"], true);
    assert_eq!(decode(&fortress_observe(s.handle()))?["ok"], false);
    assert_eq!(ask(&s, allocation(1,8))?["error"]["code"], "adapter_unavailable");
    let list = ask(&s, json!({"kind":"history"}))?;
    assert_eq!(list["ok"], true); assert_eq!(list["source_stale"], true);
    let past = ask(&s, archived(&record, allocation(1,8)))?;
    assert_eq!(past["ok"], true); assert_eq!(past["model_feasible"], true);
    assert_eq!(past["current_live_anchor"], current["anchor"]);
    assert_eq!(past["agent_turn"]["briefing"]["live_source_fenced"], true);
    assert_eq!(past["agent_turn"]["continuity"]["status"], "partial");
    assert_eq!(s.calls.load(Ordering::SeqCst), 2); Ok(())
}

#[test]
fn changed_archive_fences_refresh_without_publishing_the_new_world() -> Result<()> {
    let _serial = lock(&SERIAL)?; let archive = Archive::new()?; let s = full()?;
    attach(&s, &archive.path)?; let record = first_record(&s)?;
    let mut file = OpenOptions::new().append(true).open(&archive.path).map_err(io_error)?;
    file.write_all(b"D").map_err(io_error)?; file.sync_all().map_err(io_error)?; drop(file);
    let bytes = fs::read(&archive.path).map_err(io_error)?;
    let result = decode(&fortress_observe(s.handle()))?;
    assert_eq!(result["ok"], false); assert_eq!(result["error"]["code"], "corrupt_ledger");
    assert_eq!(result["agent_turn"]["anchor"], record["anchor"]);
    assert_eq!(result["agent_turn"]["continuity"]["status"], "stale");
    assert_eq!(fs::read(&archive.path).map_err(io_error)?, bytes);
    assert_eq!(ask(&s, archived(&record, allocation(1,8)))?["error"]["code"], "corrupt_ledger"); Ok(())
}

#[test]
fn historical_allocation_paginates_at_8192_bytes_with_current_work_and_exact_drills() -> Result<()> {
    let _serial = lock(&SERIAL)?; let archive = Archive::new()?;
    let s = register(2048,vec![Capability::Query,Capability::Observe])?; stock(&s,40)?;
    attach(&s, &archive.path)?; let record = first_record(&s)?; let w = watch(&s)?;
    let mut query = allocation(40,128); let mut ids = Vec::new(); let mut pages = 0;
    loop {
        let raw = fortress_query(s.handle(),None,Some(json!({"schema":"dfmcp.query/1","query":archived(&record,query.clone())})));
        assert!(raw.len() <= 8192); let page = decode(&raw)?; assert_eq!(page["ok"],true,"{raw}");
        assert_eq!(page["summary"]["allocated_units"],40); assert_eq!(page["anchor"],record["anchor"]);
        assert_eq!(page["agent_turn"]["active_work"]["obligations"].as_array().map(Vec::len),Some(1));
        let rows = page["rows"].as_array().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"archive rows"))?;
        assert!(!rows.is_empty());
        for row in rows {
            ids.push(row["item"]["entity_id"].as_str().ok_or_else(||error(ErrorCode::InternalInvariantViolation,"archive item"))?.to_owned());
            assert_eq!(row["route_query"]["query"]["record_digest"],record["record_digest"]);
        }
        pages += 1; assert!(pages <= 40);
        if page["continuation"].is_null() { break; }
        query["continuation"] = page["continuation"].clone(); query["limit"] = json!(3);
    }
    assert!(pages > 1); assert_eq!(ids,(100..140).map(|id|item_entity_id(id).to_string()).collect::<Vec<_>>());
    assert_eq!(s.calls.load(Ordering::SeqCst),0); release_watch(&s,w)?; Ok(())
}

#[test]
fn discovery_and_historical_requests_reject_wrong_record_scope_and_stateful_work() -> Result<()> {
    let _serial = lock(&SERIAL)?; let archive = Archive::new()?; let s = full()?;
    attach(&s,&archive.path)?; let record = first_record(&s)?;
    let schema = decode(&fortress_query(s.handle(),Some("schema".to_owned()),None))?;
    assert_eq!(schema["ok"],true); assert_eq!(schema["query_schema"]["$defs"]["query"]["oneOf"].as_array().map(Vec::len),Some(20));
    for kind in ["watch","capture","changes","await_watch","historical_query","release_watch"] {
        assert_eq!(ask(&s,archived(&record,json!({"kind":kind})))?["error"]["code"],"invalid_request");
    }
    let mut bad = archived(&record,allocation(1,8)); bad["record_digest"] = json!("0".repeat(64));
    assert_eq!(ask(&s,bad)?["error"]["code"],"stale_anchor");
    assert_eq!(ask(&s,json!({"kind":"history","path":"/tmp/forbidden"}))?["error"]["code"],"invalid_request");
    assert!(allowed_environment("DFMCP_SPATIAL_JOURNAL")); assert!(allowed_environment("DFMCP_SPATIAL_JOURNAL_REPAIR"));
    assert!(!allowed_environment("DFMCP_OPERATIONS_JOURNAL")); assert!(!allowed_environment("DFMCP_ADMITTED_BRIDGE_PROTOCOL"));
    {
        let handle = resolve(s.handle())?; let mut session = lock(&handle)?;
        let now = session.anchor()?.tick;
        for grant in &mut session.grants { grant.expires_at_tick = Some(dfmcp_core::GameTick(now.0-1)); }
    }
    assert_eq!(ask(&s,archived(&record,allocation(1,8)))?["error"]["code"],"capability_denied");
    assert_eq!(s.calls.load(Ordering::SeqCst),0); Ok(())
}
