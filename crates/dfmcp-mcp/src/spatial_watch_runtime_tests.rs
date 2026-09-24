//! Actual spatial/1.8 handlers and startup attachment, with an injected capture
//! source and real private journals. No environment mutation or native process.
use super::*;
use dfmcp_adapter::live_operations::OperationsProfile;
use dfmcp_adapter::live_spatial::LiveSpatialObservation;
use dfmcp_adapter::operations_journal::TailRecovery;
use std::collections::VecDeque;
use std::fs;
use std::os::unix::fs::DirBuilderExt;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError {
    error(
        ErrorCode::CorruptLedger,
        "spatial watch test filesystem failure",
    )
}
struct Files {
    directory: PathBuf,
    observations: PathBuf,
    watches: PathBuf,
}
impl Files {
    fn new() -> Result<Self> {
        let n = FILE_ID.fetch_add(1, Ordering::Relaxed);
        let directory = std::env::temp_dir()
            .canonicalize()
            .map_err(io_error)?
            .join(format!(
                "dfmcp-spatial-watch-handlers-{}-{n}",
                std::process::id()
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
                "injected capture source exhausted",
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
fn part(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn observation(tick: u32) -> Result<LiveSpatialCitizenObservation> {
    let hex = include_str!("../../dfmcp-adapter/tests/fixtures/spatial_v1_6.hex").trim();
    let bytes = (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "invalid spatial fixture hex",
                )
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let base =
        LiveSpatialObservation::decode_payload(&bytes, 7, "df".to_owned(), "dfhack".to_owned())?;
    let mut operations = base.operations().clone();
    let mut terrain = base.terrain().clone();
    operations.jobs.year_tick = tick;
    terrain.year_tick = tick;
    operations.jobs.paused = true;
    terrain.paused = true;
    let mut spatial = b"DFMS1600".to_vec();
    part(
        &mut spatial,
        &operations.encode_profile(OperationsProfile::PagedV1_4)?,
    );
    part(&mut spatial, &terrain.encode_payload()?);
    let mut citizens = b"DFMC1800".to_vec();
    citizens.extend_from_slice(&0u32.to_be_bytes());
    let mut combined = b"DFMS1800".to_vec();
    part(&mut combined, &spatial);
    part(&mut combined, &citizens);
    LiveSpatialCitizenObservation::decode_payload(
        &combined,
        7,
        "df".to_owned(),
        "dfhack".to_owned(),
    )
}
fn session(
    files: &Files,
    tick: u32,
    next: &[u32],
) -> Result<(Session, OperationContext, Arc<AtomicUsize>)> {
    let initial = observation(tick)?;
    let region = initial.spatial().terrain().map.region;
    let limits = CitizenSpatialLimits {
        spatial: SpatialLimits {
            operations: PagedOperationsLimits::default(),
            region,
        },
        citizens: 4096,
    };
    let mut state = LiveSpatialCitizenState::default();
    state.publish(initial)?;
    let anchor = state
        .snapshot()
        .ok_or_else(|| {
            error(
                ErrorCode::InternalInvariantViolation,
                "fixture snapshot absent",
            )
        })?
        .anchor();
    let calls = Arc::new(AtomicUsize::new(0));
    let values = next
        .iter()
        .map(|tick| observation(*tick))
        .collect::<Result<VecDeque<_>>>()?;
    let mut session = Session {
        id: next_id()?,
        source: Box::new(Script {
            values,
            calls: Arc::clone(&calls),
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
    let mut context = session.context()?;
    history::attach(
        &mut session,
        &files.observations,
        TailRecovery::Refuse,
        &context,
    )?;
    context.anchor = session.anchor()?;
    Ok((session, context, calls))
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
fn decode(text: &str) -> Result<Value> {
    serde_json::from_str(text).map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "handler JSON invalid",
        )
    })
}
fn register(files: &Files, tick: u32, next: &[u32]) -> Result<(Registered, Value)> {
    let (mut session, c, calls) = session(files, tick, next)?;
    let opened = finish_open(&mut session, &c, Some(&files.watches), json!({"ok":true}))?;
    let id = session.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok((Registered { id, calls }, decode(&opened)?))
}
fn ask(session: &Registered, query: Value) -> Result<Value> {
    decode(&fortress_query(
        session.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1","query":query})),
    ))
}
fn watch() -> Value {
    json!({"kind":"watch","key":"remain-paused","condition":{"op":"paused","value":true},
    "deadline_tick":105u64*403200+100,"poll_interval_ticks":1,"stable_observations":2})
}

#[test]
fn mcp_restart_discovers_watches_and_only_fresh_captures_restore_stability() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let (first, opened) = register(&files, 3, &[])?;
    assert_eq!(opened["agent_turn"]["continuity"]["status"], "bootstrap");
    let created = ask(&first, watch())?;
    assert_eq!(created["ok"], true, "{created}");
    assert_eq!(created["durable"], true);
    assert_eq!(created["record"]["stable_observations"], 1);
    let old = created["record"]["watch"].clone();
    drop(first);
    let (second, opened) = register(&files, 4, &[5, 6])?;
    assert_eq!(opened["watch_recovery"]["restored"], 1);
    assert_eq!(opened["agent_turn"]["continuity"]["status"], "partial");
    let work = &opened["agent_turn"]["active_work"]["obligations"][0];
    assert_eq!(work["fresh_observation_required"], true);
    assert_eq!(work["evaluation_current"], false);
    let listed = ask(&second, json!({"kind":"watches"}))?;
    let handle = listed["records"][0]["watch"].clone();
    assert_ne!(handle, old);
    assert_eq!(
        ask(&second, json!({"kind":"poll_watch","watch":old}))?["ok"],
        false
    );
    let same = ask(&second, json!({"kind":"poll_watch","watch":handle}))?;
    assert_eq!(same["record"]["stable_observations"], 0);
    assert_eq!(second.calls.load(Ordering::SeqCst), 0);
    let one = ask(&second, json!({"kind":"await_watch","watch":handle}))?;
    assert_eq!(one["ok"], true, "{one}");
    assert_eq!(one["record"]["stable_observations"], 1);
    assert_eq!(one["record"]["terminal"], false);
    let done = ask(&second, json!({"kind":"await_watch","watch":handle}))?;
    assert_eq!(done["ok"], true, "{done}");
    assert_eq!(done["record"]["status"], "satisfied");
    assert_eq!(done["record"]["sample_count"], 3);
    assert_eq!(second.calls.load(Ordering::SeqCst), 2);
    let bytes = fs::read(&files.watches).map_err(io_error)?;
    let record_digest = {
        let h = resolve(second.handle())?;
        let s = lock(&h)?;
        s.journal
            .as_ref()
            .and_then(|j| j.entries().first())
            .map(|e| e.record_digest.to_string())
            .ok_or_else(|| {
                error(
                    ErrorCode::InternalInvariantViolation,
                    "archive fixture absent",
                )
            })?
    };
    let historical = ask(
        &second,
        json!({"kind":"historical_query","record":1,"record_digest":record_digest,
        "query":{"kind":"aggregate","group_by":{"kind":"entity_kind"}}}),
    )?;
    assert_eq!(historical["ok"], true, "{historical}");
    assert_eq!(historical["historical"], true);
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, bytes);
    assert_eq!(second.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        ask(&second, json!({"kind":"release_watch","watch":handle}))?["ok"],
        true
    );
    drop(second);
    let (third, opened) = register(&files, 6, &[])?;
    assert_eq!(opened["watch_recovery"]["restored"], 0);
    assert_eq!(
        ask(&third, json!({"kind":"watches"}))?["records"],
        json!([])
    );
    drop(third);
    Ok(())
}

#[test]
fn mcp_reports_watch_corruption_without_hiding_it_as_output_overflow() -> Result<()> {
    use std::io::Write;
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let (s, _) = register(&files, 3, &[])?;
    assert_eq!(ask(&s, watch())?["ok"], true);
    fs::OpenOptions::new()
        .append(true)
        .open(&files.watches)
        .and_then(|mut f| f.write_all(b"x"))
        .map_err(io_error)?;
    let original = fs::read(&files.watches).map_err(io_error)?;
    let result = ask(&s, json!({"kind":"watches"}))?;
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "corrupt_ledger");
    assert_eq!(
        result["agent_turn"]["briefing"]["active_work_unavailable"],
        true
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    drop(s);
    assert!(register(&files, 4, &[]).is_err());
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, original);
    Ok(())
}

#[test]
fn rejected_bootstrap_packet_does_not_publish_a_recovered_handle() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let (s, _) = register(&files, 3, &[])?;
    assert_eq!(ask(&s, watch())?["ok"], true);
    drop(s);
    let original = fs::read(&files.watches).map_err(io_error)?;
    let (mut candidate, mut context, _) = session(&files, 4, &[])?;
    candidate.budget.max_output_tokens = 1;
    context.budget = candidate.budget;
    assert!(
        finish_open(
            &mut candidate,
            &context,
            Some(&files.watches),
            json!({"ok":true})
        )
        .is_err()
    );
    assert!(candidate._watch_journal.is_none());
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, original);
    drop(candidate);
    let (s, opened) = register(&files, 4, &[])?;
    assert_eq!(opened["watch_recovery"]["restored"], 1);
    assert_eq!(
        ask(&s, json!({"kind":"watches"}))?["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    drop(s);
    Ok(())
}
