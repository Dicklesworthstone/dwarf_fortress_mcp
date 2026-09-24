//! Actual cancel/query handlers over injected sources and exclusively created files.
//! Run this module with --test-threads=1, as with the other bounded-session suites.
use super::*;
use dfmcp_adapter::operations_journal::{
    JournalLimits, PrivateJournalFile, Spatial18, TailRecovery,
};
use dfmcp_core::GameTick;
use std::collections::VecDeque;
use std::fs;
use std::io::Write;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
#[path = "../../dfmcp-adapter/tests/support/production_spatial.rs"]
mod fixture;
#[path = "spatial_session_release_recovery_tests.rs"]
mod recovery;

static SERIAL: Mutex<()> = Mutex::new(());
static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError {
    error(ErrorCode::CorruptLedger, "release fixture I/O")
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
                "dfmcp-session-release-{}-{}",
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
    drops: Arc<AtomicUsize>,
    fenced: bool,
}
impl Source for Script {
    fn read(&mut self, _: Duration) -> Result<LiveSpatialCitizenObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.values
            .pop_front()
            .ok_or_else(|| error(ErrorCode::AdapterUnavailable, "injected source exhausted"))
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
impl Drop for Script {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
struct Registered {
    id: SessionId,
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
}
impl Registered {
    fn handle(&self) -> Option<String> {
        Some(self.id.to_string())
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        // Test-only cleanup also handles a deliberately undersized response budget.
        let removed = SESSIONS
            .lock()
            .ok()
            .and_then(|mut sessions| sessions.remove(&self.id));
        if let Some(handle) = removed {
            let mut session = handle.lock().unwrap_or_else(|poison| poison.into_inner());
            let _ = semantic_query::release_session_resources(self.id, true, |_| Ok(String::new()));
            session._watch_journal = None;
            session.journal = None;
            session._slot.release();
        }
    }
}
fn register(files: Option<&Files>, durable: bool, tick: u32, next: &[u32]) -> Result<Registered> {
    let initial = fixture::observation(tick, 1, 5, false)?;
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
        .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "fixture snapshot"))?
        .anchor();
    let calls = Arc::new(AtomicUsize::new(0));
    let drops = Arc::new(AtomicUsize::new(0));
    let source = Script {
        values: next
            .iter()
            .map(|t| fixture::observation(*t, 1, 5, false))
            .collect::<Result<VecDeque<_>>>()?,
        calls: calls.clone(),
        drops: drops.clone(),
        fenced: false,
    };
    let mut session = Session {
        id: next_id()?,
        source: Box::new(source),
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
    if let Some(files) = files {
        history::attach(&mut session, &files.observations, TailRecovery::Refuse, &c)?;
        c.anchor = session.anchor()?;
        if durable {
            durable_watches::finish_open(
                &mut session,
                &c,
                Some(&files.watches),
                json!({"ok":true}),
            )?;
        }
    }
    let id = session.id;
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(session)));
    Ok(Registered { id, calls, drops })
}
fn decode(raw: &str) -> Result<Value> {
    serde_json::from_str(raw).map_err(|_| error(ErrorCode::InvalidRequest, "fixture response"))
}
fn ask(s: &Registered, query: Value) -> Result<Value> {
    decode(&fortress_query(
        s.handle(),
        None,
        Some(json!({"schema":"dfmcp.query/1", "query":query})),
    ))
}
fn successful(value: Value) -> Value {
    assert_eq!(value["ok"], true, "{value}");
    value
}
fn watch(s: &Registered) -> Result<Value> {
    let handle = resolve(s.handle())?;
    let deadline = lock(&handle)?.anchor()?.tick.0 + 50;
    Ok(successful(ask(
        s,
        json!({"kind":"watch", "key":"keep-intent", "condition":{"op":"paused","value":true},
        "deadline_tick":deadline, "stable_observations":2}),
    )?)["record"]["watch"]
        .clone())
}
fn baseline(s: &Registered) -> Result<Value> {
    Ok(successful(ask(
        s,
        json!({"kind":"capture", "key":"keep-baseline", "max_game_ticks":50,
        "select":{"kind":"entities","kinds":["job"],"fields":["suspended"]}}),
    )?)["captured"]
        .clone())
}
fn request_close(s: &Registered, discard: bool) -> Result<Value> {
    decode(&fortress_cancel(
        s.handle(),
        Some("session".into()),
        Some(discard),
    ))
}

#[test]
fn close_reclaims_slots_and_sources_even_with_already_resolved_arcs() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let a = register(None, false, 3, &[])?;
    let b = register(None, false, 3, &[])?;
    assert!(Slot::reserve().is_err());
    let old = resolve(a.handle())?;
    let context = lock(&old)?.context()?;
    let raw = fortress_cancel(a.handle(), Some("session".into()), None);
    successful(decode(&raw)?);
    assert_eq!(a.drops.load(Ordering::SeqCst), 1);
    assert_eq!(a.calls.load(Ordering::SeqCst), 0);
    {
        let mut stale = lock(&old)?;
        assert!(matches!(stale.context(), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert!(matches!(stale.refresh(&context), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert!(stale.state.snapshot().is_none());
        assert!(stale.grants.is_empty());
    }
    let c = register(None, false, 3, &[])?;
    assert_eq!(
        fortress_cancel(a.handle(), Some("session".into()), None),
        raw
    );
    assert_eq!(SLOTS.load(Ordering::SeqCst), 2);
    drop(old);
    assert_eq!(SLOTS.load(Ordering::SeqCst), 2);
    successful(request_close(&b, false)?);
    successful(request_close(&c, false)?);
    assert_eq!(SLOTS.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn volatile_records_need_consent_and_failed_close_keeps_both_registries() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(None, false, 3, &[])?;
    watch(&s)?;
    baseline(&s)?;
    let watches = successful(ask(&s, json!({"kind":"watches"}))?)["records"].clone();
    assert_eq!(request_close(&s, false)?["error"]["code"], "conflict");
    assert_eq!(
        successful(ask(&s, json!({"kind":"watches"}))?)["records"],
        watches
    );
    assert_eq!(
        successful(ask(&s, json!({"kind":"baselines"}))?)["baselines"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let closed = successful(request_close(&s, true)?);
    assert_eq!(
        closed["process_local_resources"]["process_local_baselines_discarded"],
        1
    );
    assert_eq!(
        closed["process_local_resources"]["process_local_watch_records_discarded"],
        1
    );
    assert_eq!(
        ask(&s, json!({"kind":"watches"}))?["error"]["code"],
        "session_not_found"
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn render_refusal_keeps_durable_files_locks_and_volatile_records() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(Some(&files), true, 3, &[])?;
    watch(&s)?;
    baseline(&s)?;
    let observation_bytes = fs::read(&files.observations).map_err(io_error)?;
    let watch_bytes = fs::read(&files.watches).map_err(io_error)?;
    let handle = resolve(s.handle())?;
    lock(&handle)?.budget.max_output_tokens = 1;
    let refused = request_close(&s, true)?;
    lock(&handle)?.budget.max_output_tokens = 65536;
    assert_eq!(refused["error"]["code"], "budget_exceeded");
    assert_eq!(
        successful(ask(&s, json!({"kind":"baselines"}))?)["baselines"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        successful(ask(&s, json!({"kind":"watches"}))?)["records"]
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    let c = lock(&handle)?.context()?;
    assert!(
        PrivateJournalFile::open_recovery::<Spatial18>(
            &files.observations,
            &c,
            JournalLimits::default()
        )
        .is_err()
    );
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observation_bytes
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watch_bytes);
    assert_eq!(s.drops.load(Ordering::SeqCst), 0);
    successful(request_close(&s, true)?);
    Ok(())
}

#[test]
fn durable_close_preserves_original_journals_and_reopen_recovers_fresh_handles() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(Some(&files), true, 3, &[])?;
    let old_watch = watch(&s)?;
    let old_observation = fs::read(&files.observations).map_err(io_error)?;
    let old_watches = fs::read(&files.watches).map_err(io_error)?;
    let old = resolve(s.handle())?;
    let closed = successful(request_close(&s, false)?);
    assert_eq!(
        closed["process_local_resources"]["durable_watch_recovery_required"],
        true
    );
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        old_observation
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, old_watches);
    let next = register(Some(&files), true, 3, &[4, 5])?;
    let records = successful(ask(&next, json!({"kind":"watches"}))?)["records"].clone();
    assert_ne!(records[0]["watch"], old_watch);
    assert_eq!(records[0]["fresh_observation_required"], true);
    assert_eq!(records[0]["stable_observations"], 0);
    assert_eq!(
        ask(&next, json!({"kind":"await_watches","watches":[old_watch]}))?["ok"],
        false
    );
    for step in 1..=2 {
        let result = successful(ask(&next, json!({"kind":"await_watches"}))?);
        assert_eq!(result["records"][0]["stable_observations"], step);
        assert_eq!(result["all_satisfied"], step == 2);
    }
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    assert_eq!(next.calls.load(Ordering::SeqCst), 2);
    drop(old);
    successful(request_close(&next, false)?);
    Ok(())
}

#[test]
fn damaged_journals_and_fenced_source_do_not_trap_session_ownership() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let files = Files::new()?;
    let s = register(Some(&files), true, 3, &[])?;
    watch(&s)?;
    for path in [&files.observations, &files.watches] {
        fs::OpenOptions::new()
            .append(true)
            .open(path)
            .and_then(|mut f| f.write_all(b"x"))
            .map_err(io_error)?;
    }
    let observations = fs::read(&files.observations).map_err(io_error)?;
    let watches = fs::read(&files.watches).map_err(io_error)?;
    let handle = resolve(s.handle())?;
    lock(&handle)?.source.fence();
    let c = lock(&handle)?.context()?;
    let result = successful(request_close(&s, false)?);
    assert_eq!(
        result["process_local_resources"]["watch_evidence_revalidated"],
        false
    );
    assert_eq!(s.drops.load(Ordering::SeqCst), 1);
    assert_eq!(
        fs::read(&files.observations).map_err(io_error)?,
        observations
    );
    assert_eq!(fs::read(&files.watches).map_err(io_error)?, watches);
    assert!(
        PrivateJournalFile::open_recovery::<Spatial18>(
            &files.observations,
            &c,
            JournalLimits::default()
        )
        .is_err()
    );
    assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn exhausted_request_ids_or_expired_grants_do_not_require_new_game_authority_to_close() -> Result<()>
{
    let _serial = lock(&SERIAL)?;
    for case in 0..3 {
        let s = register(None, false, 3, &[])?;
        let handle = resolve(s.handle())?;
        {
            let mut session = lock(&handle)?;
            match case {
                0 => session.request = u128::MAX,
                1 => {
                    let expired = GameTick(session.anchor()?.tick.0 - 1);
                    for grant in &mut session.grants {
                        grant.expires_at_tick = Some(expired);
                    }
                }
                _ => session.grants.clear(),
            }
        }
        let result = successful(request_close(&s, false)?);
        assert!(result.get("anchor").is_none());
        assert!(result["agent_turn"]["anchor"].is_null());
        assert_eq!(result["game_state_observed"], false);
        assert_eq!(s.calls.load(Ordering::SeqCst), 0);
    }
    Ok(())
}

#[test]
fn concurrent_closers_wait_for_session_ownership_and_return_one_receipt() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(None, false, 3, &[])?;
    let handle = resolve(s.handle())?;
    let guard = lock(&handle)?;
    let raw_a = s.handle();
    let raw_b = s.handle();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let a = std::thread::spawn(move || {
        let _ = started_tx.send(());
        let result = fortress_cancel(raw_a, Some("session".into()), None);
        let _ = done_tx.send(());
        result
    });
    let started = started_rx.recv_timeout(Duration::from_secs(5)).is_ok();
    let blocked = done_rx.recv_timeout(Duration::from_millis(20)).is_err();
    let b = std::thread::spawn(move || fortress_cancel(raw_b, Some("session".into()), None));
    drop(guard);
    // Join both workers before any assertion or error propagation. A failed
    // timing check or first-worker panic must not detach the second closer.
    let left = a.join();
    let right = b.join();
    let left = left.map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "close thread panicked",
        )
    })?;
    let right = right.map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "close thread panicked",
        )
    })?;
    assert!(started);
    assert!(blocked);
    assert_eq!(left, right);
    successful(decode(&left)?);
    assert_eq!(s.drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn closed_receipts_are_bounded_and_eviction_cannot_close_a_new_session() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let first = register(None, false, 3, &[])?;
    successful(request_close(&first, false)?);
    for _ in 0..MAX_CLOSE_RECEIPTS {
        let s = register(None, false, 3, &[])?;
        successful(request_close(&s, false)?);
    }
    let live = register(None, false, 3, &[])?;
    assert!(lock(&CLOSED)?.len() <= MAX_CLOSE_RECEIPTS);
    assert_eq!(
        request_close(&first, false)?["error"]["code"],
        "session_not_found"
    );
    assert!(resolve(live.handle()).is_ok());
    assert_eq!(live.drops.load(Ordering::SeqCst), 0);
    successful(request_close(&live, false)?);
    Ok(())
}

#[test]
fn scope_errors_and_legacy_cancel_never_release_a_session() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(None, false, 3, &[])?;
    assert_eq!(
        decode(&fortress_cancel(s.handle(), None, None))?["error"]["code"],
        "capability_denied"
    );
    for (scope, discard) in [
        (None, Some(true)),
        (Some("game".into()), None),
        (Some("".into()), None),
    ] {
        assert_eq!(
            decode(&fortress_cancel(s.handle(), scope, discard))?["error"]["code"],
            "invalid_request"
        );
    }
    for raw in [
        None,
        Some("bad".into()),
        Some("00000000000000000000000000000007".into()),
    ] {
        assert_eq!(
            decode(&fortress_cancel(raw, Some("session".into()), None))?["ok"],
            false
        );
    }
    assert!(resolve(s.handle()).is_ok());
    assert_eq!(s.drops.load(Ordering::SeqCst), 0);
    successful(request_close(&s, false)?);
    Ok(())
}

#[test]
fn poisoned_session_can_be_released_without_clearing_or_reusing_its_world() -> Result<()> {
    let _serial = lock(&SERIAL)?;
    let s = register(None, false, 3, &[])?;
    let handle = resolve(s.handle())?;
    let panicked = std::thread::spawn(move || {
        let _guard = handle.lock().expect("fixture lock");
        panic!("injected in-flight request panic");
    })
    .join()
    .is_err();
    assert!(panicked);
    let result = successful(request_close(&s, false)?);
    assert_eq!(result["session_mutex_poisoned"], true);
    assert_eq!(result["game_state_observed"], false);
    assert_eq!(s.drops.load(Ordering::SeqCst), 1);
    Ok(())
}
