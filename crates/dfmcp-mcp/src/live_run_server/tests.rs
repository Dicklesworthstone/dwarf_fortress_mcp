//! Tests call the same generic dispatcher/rendering path as the MCP handlers.
//! Fake sources and in-memory journals do not qualify live DFHack or file custody.
use super::*;
use std::collections::BTreeMap;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};
use std::sync::Arc;

#[derive(Clone, Default)]
struct Memory(Arc<Mutex<Cursor<Vec<u8>>>>);
impl Read for Memory {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("test poison"))?
            .read(out)
    }
}
impl Seek for Memory {
    fn seek(&mut self, p: SeekFrom) -> io::Result<u64> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("test poison"))?
            .seek(p)
    }
}
impl Write for Memory {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("test poison"))?
            .write(b)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl EffectJournalStorage for Memory {
    fn sync(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn truncate(&mut self, _: u64) -> io::Result<()> {
        Err(io::Error::other("repair prohibited"))
    }
}
fn observed(sequence: u64, tick: u64, paused: bool) -> Result<RunObservation> {
    let mut bytes = b"DFMRO013".to_vec();
    for n in [41u64, sequence, tick] {
        bytes.extend_from_slice(&n.to_be_bytes());
    }
    bytes.extend_from_slice(&[1, 1, u8::from(paused)]);
    RunObservation::decode(&bytes)
}
fn native_record(plan: &RunPlan, phase: u8, reason: u8, tick: Option<u64>) -> Result<RunRecord> {
    let mut bytes = b"DFMRE013".to_vec();
    bytes.extend_from_slice(&plan.canonical_bytes());
    bytes.extend_from_slice(plan.digest().as_bytes());
    bytes.extend_from_slice(plan.token());
    bytes.extend_from_slice(&[
        phase,
        reason,
        u8::from([1, 2, 3, 5].contains(&phase)),
        u8::from(phase == 3),
        u8::from(tick.is_some()),
    ]);
    bytes.extend_from_slice(&tick.unwrap_or(0).to_be_bytes());
    let mut proof = b"dfmcp-bounded-run-receipt/1\0".to_vec();
    proof.extend_from_slice(&bytes);
    bytes.extend_from_slice(Digest32::of_bytes(&proof).as_bytes());
    RunRecord::decode(&bytes)
}
struct NativeState {
    records: BTreeMap<String, RunRecord>,
    observation: RunObservation,
    calls: Vec<&'static str>,
    lose_commit: bool,
    absent_query: bool,
    fail_observe: bool,
}
#[derive(Clone)]
struct Factory(Arc<Mutex<NativeState>>);
struct Source {
    state: Factory,
    manifest: RunManifest,
}
fn manifest() -> RunManifest {
    RunManifest {
        generation: 41,
        df_version: "fake-df".into(),
        dfhack_version: "fake-dfhack".into(),
    }
}
fn endpoint() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 5000))
}
fn test_lock<T>(m: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    m.lock()
        .map_err(|_| error(ErrorCode::InternalInvariantViolation, "test lock"))
}
impl SourceFactory for Factory {
    type Source = Source;
    fn connect(&mut self, _: &OperationContext) -> Result<Source> {
        test_lock(&self.0)?.calls.push("connect");
        Ok(Source {
            state: self.clone(),
            manifest: manifest(),
        })
    }
}
impl RunSource for Source {
    fn manifest(&self) -> &RunManifest {
        &self.manifest
    }
    fn endpoint(&self) -> Option<SocketAddr> {
        Some(endpoint())
    }
    fn observe(&mut self, _: &OperationContext) -> Result<RunObservation> {
        let mut s = test_lock(&self.state.0)?;
        s.calls.push("observe");
        if s.fail_observe {
            return Err(error(
                ErrorCode::AdapterUnavailable,
                "injected read failure",
            ));
        }
        Ok(s.observation.clone())
    }
    fn prepare(&mut self, p: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let mut s = test_lock(&self.state.0)?;
        s.calls.push("prepare");
        let r = native_record(p, 0, 0, None)?;
        s.records.insert(p.key().to_owned(), r.clone());
        Ok(r)
    }
    fn commit(&mut self, p: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let mut s = test_lock(&self.state.0)?;
        s.calls.push("commit");
        let r = native_record(p, 1, 0, Some(100))?;
        s.records.insert(p.key().to_owned(), r.clone());
        s.observation = observed(1, 100, false)?;
        if s.lose_commit {
            Err(error(ErrorCode::AdapterUnavailable, "lost commit reply"))
        } else {
            Ok(r)
        }
    }
    fn query(&mut self, p: &RunPlan, _: &OperationContext) -> Result<Option<RunRecord>> {
        let mut s = test_lock(&self.state.0)?;
        s.calls.push("query");
        if s.absent_query {
            return Ok(None);
        }
        if s.records
            .get(p.key())
            .is_some_and(|r| r.phase() == dfmcp_adapter::bounded_run::RunPhase::Running)
        {
            s.records
                .insert(p.key().to_owned(), native_record(p, 3, 1, Some(110))?);
            s.observation = observed(1, 110, true)?;
        }
        Ok(s.records.get(p.key()).cloned())
    }
    fn cancel(&mut self, p: &RunPlan, _: &OperationContext) -> Result<RunRecord> {
        let mut s = test_lock(&self.state.0)?;
        s.calls.push("cancel");
        let r = native_record(p, 3, 3, Some(103))?;
        s.records.insert(p.key().to_owned(), r.clone());
        s.observation = observed(1, 103, true)?;
        Ok(r)
    }
}
type State = RunSession<Memory, Factory>;
fn setup() -> Result<(State, Memory, Factory)> {
    let storage = Memory::default();
    let observed = observed(0, 100, true)?;
    let factory = Factory(Arc::new(Mutex::new(NativeState {
        records: BTreeMap::new(),
        observation: observed.clone(),
        calls: Vec::new(),
        lose_commit: false,
        absent_query: false,
        fail_observe: false,
    })));
    let c = OperationContext {
        session_id: SessionId::new(1),
        request_id: RequestId::new(1),
        anchor: source_anchor(&observed),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: MAX_BYTES,
            max_output_tokens: 16_384,
            max_entities: 256,
            max_actions: 1,
            max_game_ticks: 1200,
        },
        grants: grants(RunMode::Control, true)?,
        cancellation_requested: false,
    };
    let journal = RunJournal::open(
        storage.clone(),
        &c,
        RunMode::Control,
        Some(RunBinding::new(endpoint(), manifest())?),
        true,
    )?;
    Ok((
        RunSession {
            id: c.session_id,
            request: 1,
            anchor: c.anchor,
            budget: c.budget,
            grants: c.grants,
            journal,
            factory: Some(factory.clone()),
            selected: Some(observed),
            cursors: Cursors::default(),
        },
        storage,
        factory,
    ))
}
fn decode(text: &str) -> Result<Value> {
    serde_json::from_str(text)
        .map_err(|_| error(ErrorCode::InternalInvariantViolation, "test JSON"))
}
fn call(state: &mut State, operation: &str, action: Action) -> Result<Value> {
    let rows = match &action {
        Action::Query { limit, .. } => *limit,
        Action::Doctor | Action::Unavailable => 0,
        _ => 1,
    };
    let c = state.context(true, false)?;
    decode(&run_action(
        state,
        c,
        operation,
        Limits::default(),
        rows,
        Ok(action),
    ))
}
fn plan_action(key: &str) -> Result<Action> {
    Ok(Action::Plan {
        key: key.into(),
        spec: RunSpec::new(10, 1000)?,
        witness: observed(0, 100, true)?.witness(),
    })
}
fn plan_digest() -> Result<Digest32> {
    Ok(RunPlan::new("test", RunSpec::new(10, 1000)?, observed(0, 100, true)?)?.digest())
}
fn commit_action() -> Result<Action> {
    Ok(Action::Commit {
        key: "test".into(),
        digest: plan_digest()?,
        confirm: true,
    })
}
fn counts(factory: &Factory, name: &str) -> Result<usize> {
    Ok(test_lock(&factory.0)?
        .calls
        .iter()
        .filter(|c| **c == name)
        .count())
}
fn reopen(storage: Memory, mode: RunMode, factory: Option<Factory>) -> Result<State> {
    let observed = observed(0, 100, true)?;
    let c = OperationContext {
        session_id: SessionId::new(2),
        request_id: RequestId::new(1),
        anchor: source_anchor(&observed),
        budget: WorkBudget {
            max_wall_millis: 60_000,
            max_bytes: MAX_BYTES,
            max_output_tokens: 16_384,
            max_entities: 256,
            max_actions: 1,
            max_game_ticks: 1200,
        },
        grants: grants(mode, true)?,
        cancellation_requested: false,
    };
    let journal = RunJournal::open(storage, &c, mode, None, false)?;
    Ok(RunSession {
        id: c.session_id,
        request: 1,
        anchor: c.anchor,
        budget: c.budget,
        grants: c.grants,
        journal,
        factory,
        selected: None,
        cursors: Cursors::default(),
    })
}
#[test]
fn dispatcher_runs_plan_commit_wait_and_offline_receipt_recovery() -> Result<()> {
    let (mut s, storage, f) = setup()?;
    assert_eq!(
        call(&mut s, "fortress.plan", plan_action("test")?)?["result"]["effect"]["state"],
        "prepared"
    );
    assert_eq!(
        call(&mut s, "fortress.commit", commit_action()?)?["result"]["effect"]["state"],
        "tracking"
    );
    assert!(s.selected.is_none());
    let r = call(
        &mut s,
        "fortress.wait",
        Action::Wait {
            key: "test".into(),
            digest: plan_digest()?,
        },
    )?;
    assert_eq!(
        r["result"]["effect"]["native"]["historical_pause_verified"],
        true
    );
    assert_eq!(counts(&f, "commit")?, 1);
    let head = s.journal.head();
    drop(s);
    let mut offline = reopen(storage, RunMode::Offline, None)?;
    let r = call(
        &mut offline,
        "fortress.explain",
        Action::Explain {
            key: "test".into(),
            digest: plan_digest()?,
        },
    )?;
    assert_eq!(r["result"]["effect"]["state"], "terminal");
    assert_eq!(head, offline.journal.head());
    assert_eq!(r["agent_turn"]["briefing"]["current_pause_unproved"], true);
    Ok(())
}
#[test]
fn uncertain_commit_reconnects_only_for_query_never_for_redispatch() -> Result<()> {
    let (mut s, _, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    test_lock(&f.0)?.lose_commit = true;
    assert_eq!(
        call(&mut s, "fortress.commit", commit_action()?)?["result"]["ok"],
        false
    );
    let connects = counts(&f, "connect")?;
    call(&mut s, "fortress.commit", commit_action()?)?;
    assert_eq!(connects, counts(&f, "connect")?);
    assert_eq!(counts(&f, "commit")?, 1);
    let r = call(
        &mut s,
        "fortress.wait",
        Action::Wait {
            key: "test".into(),
            digest: plan_digest()?,
        },
    )?;
    assert_eq!(r["result"]["ok"], true);
    assert_eq!(counts(&f, "commit")?, 1);
    assert_eq!(counts(&f, "query")?, 1);
    Ok(())
}
#[test]
fn offline_cannot_gain_native_authority_even_with_injected_factory_and_grants() -> Result<()> {
    let (mut s, storage, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    drop(s);
    let prior = counts(&f, "connect")?;
    let mut offline = reopen(storage, RunMode::Offline, Some(f.clone()))?;
    offline.grants = grants(RunMode::Control, true)?;
    for (op, action) in [
        ("fortress.observe", Action::Observe),
        ("fortress.commit", commit_action()?),
        (
            "fortress.cancel",
            Action::Cancel {
                key: "test".into(),
                digest: plan_digest()?,
            },
        ),
    ] {
        assert_eq!(call(&mut offline, op, action)?["result"]["ok"], false);
    }
    assert_eq!(prior, counts(&f, "connect")?);
    Ok(())
}
#[test]
fn recover_mode_can_query_receipts_but_cannot_commit_or_cancel() -> Result<()> {
    let (mut s, storage, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    call(&mut s, "fortress.commit", commit_action()?)?;
    drop(s);
    let mut recover = reopen(storage, RunMode::Recover, Some(f.clone()))?;
    recover.grants = grants(RunMode::Control, true)?;
    assert_eq!(
        call(&mut recover, "fortress.commit", commit_action()?)?["result"]["ok"],
        false
    );
    assert_eq!(
        call(
            &mut recover,
            "fortress.cancel",
            Action::Cancel {
                key: "test".into(),
                digest: plan_digest()?
            }
        )?["result"]["ok"],
        false
    );
    assert_eq!(
        call(
            &mut recover,
            "fortress.wait",
            Action::Wait {
                key: "test".into(),
                digest: plan_digest()?
            }
        )?["result"]["ok"],
        true
    );
    assert_eq!(counts(&f, "cancel")?, 0);
    assert_eq!(counts(&f, "commit")?, 1);
    Ok(())
}
#[test]
fn cancellation_before_dispatch_never_connects_and_after_dispatch_preserves_pause_proof()
-> Result<()> {
    let (mut s, _, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    let n = counts(&f, "connect")?;
    let r = call(
        &mut s,
        "fortress.cancel",
        Action::Cancel {
            key: "test".into(),
            digest: plan_digest()?,
        },
    )?;
    assert_eq!(r["result"]["effect"]["state"], "cancelled_before_dispatch");
    assert_eq!(n, counts(&f, "connect")?);
    let (mut s, _, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    call(&mut s, "fortress.commit", commit_action()?)?;
    let r = call(
        &mut s,
        "fortress.cancel",
        Action::Cancel {
            key: "test".into(),
            digest: plan_digest()?,
        },
    )?;
    assert_eq!(r["result"]["effect"]["native"]["reason"], "cancelled");
    assert_eq!(counts(&f, "cancel")?, 1);
    Ok(())
}
#[test]
fn confirmation_authority_and_budget_refusals_precede_any_native_connection() -> Result<()> {
    let (mut s, _, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    let n = counts(&f, "connect")?;
    let r = call(
        &mut s,
        "fortress.commit",
        Action::Commit {
            key: "test".into(),
            digest: plan_digest()?,
            confirm: false,
        },
    )?;
    assert_eq!(r["result"]["ok"], false);
    assert_eq!(n, counts(&f, "connect")?);
    s.grants.retain(|g| g.capability == Capability::Query);
    call(&mut s, "fortress.commit", commit_action()?)?;
    assert_eq!(n, counts(&f, "connect")?);
    let head = s.journal.head();
    let c = s.context(true, false)?;
    let r = decode(&run_action(
        &mut s,
        c,
        "fortress.commit",
        Limits {
            tokens: Some(1),
            ..Limits::default()
        },
        1,
        Ok(commit_action()?),
    ))?;
    assert_eq!(r["result"]["ok"], false);
    assert_eq!(head, s.journal.head());
    assert_eq!(n, counts(&f, "connect")?);
    Ok(())
}
#[test]
fn whole_record_pages_have_complete_turns_and_head_bound_continuations() -> Result<()> {
    let (mut s, _, _) = setup()?;
    for n in 0..10 {
        call(
            &mut s,
            "fortress.plan",
            plan_action(&format!("test-{n:02}"))?,
        )?;
    }
    let mut continuation = None;
    let mut seen = Vec::new();
    loop {
        let r = call(
            &mut s,
            "fortress.query",
            Action::Query {
                filter: Filter::All,
                limit: 2,
                continuation,
            },
        )?;
        assert_eq!(r["agent_turn"]["schema"], "dfmcp.agent_turn/1");
        assert!(r.to_string().len() < BASE_RESERVE as usize + 2 * ROW_RESERVE as usize);
        for row in r["result"]["records"]
            .as_array()
            .ok_or_else(|| error(ErrorCode::InternalInvariantViolation, "test rows"))?
        {
            seen.push(row["idempotency_key"].clone());
        }
        continuation = r["result"]["continuation"].as_str().map(str::to_owned);
        if continuation.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 10);
    let r = call(
        &mut s,
        "fortress.query",
        Action::Query {
            filter: Filter::All,
            limit: 2,
            continuation: None,
        },
    )?;
    let old = r["result"]["continuation"].as_str().map(str::to_owned);
    call(&mut s, "fortress.plan", plan_action("more")?)?;
    assert_eq!(
        call(
            &mut s,
            "fortress.query",
            Action::Query {
                filter: Filter::All,
                limit: 2,
                continuation: old
            }
        )?["result"]["ok"],
        false
    );
    Ok(())
}
#[test]
fn continuation_cache_is_bounded_and_binds_every_selection_dimension() -> Result<()> {
    let mut cursors = Cursors::default();
    let key = PageKey {
        session: SessionId::new(1),
        journal: Digest32::of_bytes(b"j"),
        head: Digest32::of_bytes(b"h"),
        filter: Filter::All,
        limit: 1,
    };
    let first = cursors.issue(key.clone(), 1)?;
    assert_eq!(cursors.resolve(&first, &key)?, 1);
    for changed in [
        PageKey {
            session: SessionId::new(2),
            ..key.clone()
        },
        PageKey {
            head: Digest32::ZERO,
            ..key.clone()
        },
        PageKey {
            journal: Digest32::ZERO,
            ..key.clone()
        },
        PageKey {
            filter: Filter::Pending,
            ..key.clone()
        },
        PageKey {
            limit: 2,
            ..key.clone()
        },
    ] {
        assert!(cursors.resolve(&first, &changed).is_err());
    }
    for offset in 2..=256 {
        let token = cursors.issue(key.clone(), offset)?;
        assert_eq!(cursors.resolve(&token, &key)?, offset);
    }
    Ok(())
}
#[test]
fn failed_source_clears_selection_but_keeps_verified_local_history_available() -> Result<()> {
    let (mut s, _, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    test_lock(&f.0)?.fail_observe = true;
    assert_eq!(
        call(&mut s, "fortress.observe", Action::Observe)?["result"]["ok"],
        false
    );
    assert!(s.selected.is_none());
    assert_eq!(
        call(
            &mut s,
            "fortress.explain",
            Action::Explain {
                key: "test".into(),
                digest: plan_digest()?
            }
        )?["result"]["ok"],
        true
    );
    Ok(())
}
#[test]
fn absent_native_record_stays_unresolved_and_blocks_new_run() -> Result<()> {
    let (mut s, _, f) = setup()?;
    call(&mut s, "fortress.plan", plan_action("test")?)?;
    call(&mut s, "fortress.commit", commit_action()?)?;
    test_lock(&f.0)?.absent_query = true;
    let r = call(
        &mut s,
        "fortress.wait",
        Action::Wait {
            key: "test".into(),
            digest: plan_digest()?,
        },
    )?;
    assert_eq!(r["result"]["ok"], false);
    let r = call(
        &mut s,
        "fortress.query",
        Action::Query {
            filter: Filter::Unresolved,
            limit: 2,
            continuation: None,
        },
    )?;
    assert_eq!(r["result"]["matching_records"], 1);
    assert_eq!(counts(&f, "commit")?, 1);
    Ok(())
}
#[test]
fn exact_development_environment_and_canonical_digest_are_required() -> Result<()> {
    let keys = ALLOWED.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    environment_contract(Some("1"), Some("1"), &keys, false)?;
    for opt in [None, Some("true"), Some("01")] {
        assert!(environment_contract(opt, None, &[], false).is_err());
    }
    for clock in [Some("0"), Some("true")] {
        assert!(environment_contract(Some("1"), clock, &[], false).is_err());
    }
    assert!(environment_contract(Some("1"), None, &keys, true).is_err());
    for foreign in [
        "DFMCP_ADMITTED_BRIDGE_PROTOCOL",
        "DFMCP_RUN_COMMAND",
        "DFMCP_CONTROL_TOKEN",
    ] {
        assert!(environment_contract(Some("1"), None, &[foreign.into()], false).is_err());
    }
    for bad in ["", "ABC", &"A".repeat(64), &"0".repeat(65)] {
        assert!(digest(bad).is_err());
    }
    assert_eq!(digest(&"0".repeat(64))?, Digest32::ZERO);
    Ok(())
}
#[test]
fn public_open_refuses_missing_owned_runtime_before_environment_or_files() -> Result<()> {
    if asupersync::Cx::current().is_none() {
        let r = decode(&fortress_open_session(None, None, None, None, None))?;
        assert_eq!(r["result"]["ok"], false);
        assert_eq!(r["agent_turn"]["briefing"]["runtime_admitted"], false);
    }
    Ok(())
}
