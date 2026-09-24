//! Actual MCP handlers with one injected source and real private archives.
use super::*;
#[path = "../../dfmcp-adapter/tests/support/production_spatial.rs"]
mod fixture;
use dfmcp_adapter::operations_journal::TailRecovery;
use std::collections::{BTreeSet, VecDeque};
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError {
    error(ErrorCode::CorruptLedger, "production test I/O")
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
                "dfmcp-production-{}-{}",
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
        self.values.pop_front().ok_or_else(|| {
            error(
                ErrorCode::AdapterUnavailable,
                "production capture exhausted",
            )
        })
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
    serde_json::from_str(raw).map_err(|_| error(ErrorCode::InvalidRequest, "production test JSON"))
}
fn ok(value: Value) -> Result<Value> {
    assert_eq!(value["ok"], true, "{value}");
    Ok(value)
}
fn register(
    files: &Files,
    jobs: u32,
    next: Vec<LiveSpatialCitizenObservation>,
) -> Result<Registered> {
    let first = fixture::observation(3, jobs, 7, false)?;
    let region = first.spatial().terrain().map.region;
    let limits = CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits::default(),
            region,
        },
        citizens: 4096,
    };
    let mut state = LiveSpatialCitizenState::default();
    state.publish(first)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "test state absent"))?
        .anchor();
    let calls = Arc::new(AtomicUsize::new(0));
    let mut s = Session {
        id: next_id()?,
        source: Box::new(Script {
            values: next.into(),
            calls: calls.clone(),
            fenced: false,
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
    let mut c = s.context()?;
    history::attach(&mut s, &files.observations, TailRecovery::Refuse, &c)?;
    c.anchor = s.anchor()?;
    durable_watches::finish_open(&mut s, &c, Some(&files.watches), json!({"ok":true}))?;
    let id = s.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(s)));
    Ok(Registered { id, calls })
}
fn ask(s: &Registered, query: Value) -> Result<Value> {
    decode(&fortress_query(
        s.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":query})),
    ))
}
fn diagnosis() -> Value {
    json!({"kind":"production_diagnosis","limit":128})
}
fn supply() -> Value {
    json!({"kind":"inventory_plan","quantity_unit":"stack_units","demands":[
    {"key":"a","units":4,"item_types":["item_type_3"]},{"key":"b","units":4,"item_types":["item_type_3"]}]})
}
fn first_record(s: &Registered) -> Result<Value> {
    let value = ok(ask(s, json!({"kind":"history","limit":1}))?)?;
    value["rows"]
        .as_array()
        .and_then(|r| r.first())
        .cloned()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "test history absent"))
}
fn historical(record: &Value, query: Value) -> Value {
    json!({"kind":"historical_query","record":record["record"],
    "record_digest":record["record_digest"],"query":query})
}
fn retain_watch(s: &Registered) -> Result<()> {
    ok(ask(
        s,
        json!({"kind":"watch","key":"production-progress","condition":{"op":"paused","value":true},
        "deadline_tick":105u64*403200+100,"stable_observations":64}),
    )?)?;
    Ok(())
}

#[test]
fn production_mode_and_supply_plans_use_the_coherent_source_without_sampling_watches() -> Result<()>
{
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 2, vec![])?;
    retain_watch(&s)?;
    let watches = ok(ask(&s, json!({"kind":"watches"}))?)?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    let result = ok(decode(&fortress_query(
        s.handle(),
        Some("production".into()),
        None,
    ))?)?;
    assert_eq!(result["kind"], "production_diagnosis");
    assert_eq!(result["summary"]["jobs_considered"], 2);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["rows"][0]["blocker_proven"], false);
    let (anchor, source) = {
        let handle = resolve(s.handle())?;
        let state = lock(&handle)?;
        (state.anchor()?, state.state.source_digest()?)
    };
    assert_eq!(result["anchor"], anchor_json(anchor));
    assert_eq!(result["source_digest"], source.to_string());
    let inspected = ok(decode(&fortress_query(
        s.handle(),
        None,
        Some(result["rows"][0]["inspect_assignment"].clone()),
    ))?)?;
    assert_eq!(inspected["anchor"], result["anchor"]);
    assert_eq!(
        inspected["row"]["fields"]["worker_entity"]["presence"],
        "known"
    );
    let inventory = ok(ask(&s, supply())?)?;
    assert_eq!(inventory["summary"]["allocated_units"], 7);
    assert_eq!(inventory["model_feasible"], false);
    assert_eq!(inventory["certificate"]["shortage"]["deficit"], 1);
    assert_eq!(inventory["reservation_created"], false);
    assert_eq!(inventory["commit_compatible"], false);
    assert_eq!(
        inventory["agent_turn"]["active_work"]["obligations"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        ok(ask(&s, json!({"kind":"watches"}))?)?["records"],
        watches["records"]
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        decode(&fortress_plan(s.handle()))?["error"]["code"],
        "capability_denied"
    );
    assert_eq!(
        decode(&fortress_commit(s.handle()))?["error"]["code"],
        "capability_denied"
    );
    Ok(())
}

#[test]
fn production_pages_preserve_complete_counts_and_reject_changed_capture_or_scope() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 16, vec![fixture::observation(4, 16, 7, false)?])?;
    retain_watch(&s)?;
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_output_tokens = 4096;
    }
    let mut query = diagnosis();
    let mut ids = BTreeSet::new();
    let mut pages = 0usize;
    let mut token = Value::Null;
    loop {
        let raw = fortress_query(
            s.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query.clone()})),
        );
        assert!(raw.len() <= 16384);
        let page = ok(decode(&raw)?)?;
        assert_eq!(page["total_rows"], 16);
        assert_eq!(page["summary"]["jobs_considered"], 16);
        for row in page["rows"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InvalidRequest, "rows absent"))?
        {
            assert!(
                ids.insert(
                    row["job"]["entity_id"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned()
                )
            );
        }
        pages += 1;
        assert!(pages <= 16);
        if page["continuation"].is_null() {
            break;
        }
        if token.is_null() {
            token = page["continuation"].clone();
        }
        query["continuation"] = page["continuation"].clone();
        query["limit"] = json!(2);
    }
    assert_eq!(ids.len(), 16);
    assert!(pages > 1);
    let mut changed = diagnosis();
    changed["continuation"] = token.clone();
    changed["include_clear_jobs"] = json!(true);
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.budget.max_output_tokens = 65536;
    }
    ok(decode(&fortress_observe(s.handle()))?)?;
    let mut changed = diagnosis();
    changed["continuation"] = token;
    assert_eq!(ask(&s, changed)?["error"]["code"], "stale_anchor");
    assert_eq!(s.calls.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn historical_diagnostics_and_inspections_keep_the_original_generation_after_id_reuse() -> Result<()>
{
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(
        &files,
        1,
        vec![
            fixture::observation(4, 0, 0, true)?,
            fixture::observation(5, 1, 7, false)?,
        ],
    )?;
    let first = first_record(&s)?;
    let before = ok(ask(&s, diagnosis())?)?;
    ok(decode(&fortress_observe(s.handle()))?)?;
    ok(decode(&fortress_observe(s.handle()))?)?;
    assert_eq!(
        ok(ask(&s, diagnosis())?)?["rows"][0]["job"]["generation"],
        2
    );
    assert_eq!(
        decode(&fortress_query(
            s.handle(),
            None,
            Some(before["rows"][0]["inspect_assignment"].clone())
        ))?["error"]["code"],
        "stale_anchor"
    );
    let old = ok(ask(&s, historical(&first, diagnosis()))?)?;
    assert_eq!(old["rows"][0]["job"]["generation"], 1);
    assert_eq!(old["anchor"], first["anchor"]);
    for field in ["inspect_assignment", "inspect_relationships"] {
        let request = old["rows"][0][field].clone();
        assert_eq!(request["query"]["record"], first["record"]);
        let result = ok(decode(&fortress_query(s.handle(), None, Some(request)))?)?;
        assert_eq!(result["anchor"], first["anchor"]);
        assert_eq!(result["historical"], true);
    }
    assert_eq!(
        ok(ask(&s, historical(&first, supply()))?)?["summary"]["allocated_units"],
        7
    );
    assert_eq!(
        ok(ask(&s, diagnosis())?)?["rows"][0]["job"]["generation"],
        2
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[test]
fn a_failed_live_source_still_allows_verified_history_and_offline_production_recovery() -> Result<()>
{
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![])?;
    let first = first_record(&s)?;
    let (limits, budget) = {
        let handle = resolve(s.handle())?;
        let mut session = lock(&handle)?;
        session.source.fence();
        (session.limits, session.budget)
    };
    assert_eq!(
        ask(&s, diagnosis())?["error"]["code"],
        "adapter_unavailable"
    );
    ok(ask(&s, historical(&first, diagnosis()))?)?;
    drop(s);
    let before = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
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
    let result = ok(decode(&fortress_query(
        s.handle(),
        Some("production".into()),
        None,
    ))?)?;
    assert_eq!(result["historical"], true);
    assert_eq!(result["bridge_connection_present"], false);
    assert_eq!(result["current_freshness_proven"], false);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(
        result["rows"][0]["inspect_assignment"]["query"]["kind"],
        "historical_query"
    );
    ok(ask(&s, supply())?)?;
    let schema = ok(decode(&fortress_query(
        s.handle(),
        Some("schema".into()),
        None,
    ))?)?;
    assert_eq!(
        schema["query_schema"]["$defs"]["archive_stateless"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(13)
    );
    assert_eq!(
        schema["query_schema"]["$defs"]["query"]["oneOf"]
            .as_array()
            .map(Vec::len),
        Some(16)
    );
    drop(s);
    assert_eq!(fs::read(&files.observations).map_err(io_error)?, before);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    Ok(())
}

#[test]
fn rejected_work_focus_authority_and_custody_do_not_acquire_or_publish() -> Result<()> {
    use std::io::Write;
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(&files, 1, vec![])?;
    let before = fs::read(&files.watches).map_err(io_error)?;
    for request in [
        json!({"kind":"production_diagnosis","max_work":1}),
        json!({"kind":"production_diagnosis","job":{"entity_id":"9","generation":99}}),
        json!({"kind":"production_diagnosis","limit":129}),
    ] {
        assert_eq!(ask(&s, request)?["ok"], false);
    }
    let grants = {
        let handle = resolve(s.handle())?;
        let mut session = lock(&handle)?;
        let grants = session.grants.clone();
        session.grants.clear();
        grants
    };
    assert_eq!(ask(&s, diagnosis())?["error"]["code"], "capability_denied");
    {
        let handle = resolve(s.handle())?;
        lock(&handle)?.grants = grants;
    }
    fs::OpenOptions::new()
        .append(true)
        .open(&files.observations)
        .and_then(|mut file| file.write_all(b"x"))
        .map_err(io_error)?;
    assert_eq!(ask(&s, diagnosis())?["error"]["code"], "corrupt_ledger");
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, before);
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn composed_schema_keeps_existing_features_and_specializes_only_spatial_history() -> Result<()> {
    let schema = watch_batch::extend_schema(workforce_queries::extend_schema(history::schema()?)?)?;
    let variants = schema["$defs"]["query"]["oneOf"]
        .as_array()
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "schema variants missing"))?;
    for name in [
        "production_diagnosis",
        "inventory_plan",
        "map_route",
        "workforce_plan",
        "watch",
        "await_watches",
        "historical_changes",
        "item_quantity",
    ] {
        assert_eq!(
            variants
                .iter()
                .filter(|v| v["properties"]["kind"]["const"] == name)
                .count(),
            1,
            "{name}"
        );
    }
    let history = variants
        .iter()
        .find(|v| v["properties"]["kind"]["const"] == "history")
        .ok_or_else(|| error(ErrorCode::InvalidRequest, "history variant missing"))?;
    assert_eq!(
        history["properties"]["continuation"]["oneOf"][1]["pattern"],
        "^sch1:[1-9][0-9]*:[0-9a-f]{64}$"
    );
    assert!(
        schema["$defs"]["watch_condition"]["oneOf"]
            .as_array()
            .is_some_and(|v| v
                .iter()
                .any(|v| v["properties"]["op"]["const"] == "entity_count"))
    );
    assert_eq!(
        shared::query_schema()?["$id"],
        "urn:dfmcp:operations-query:1"
    );
    Ok(())
}

#[path = "spatial_quantity_tests.rs"]
mod quantities;
