//! Actual control handlers and private journals; no bridge credentials or live game.
use super::*;
use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::sync::atomic::{AtomicBool, Ordering};

static FILE_ID: AtomicUsize = AtomicUsize::new(0);
fn io_error(_: std::io::Error) -> DfmcpError {
    err(ErrorCode::CorruptLedger, "control close fixture I/O")
}
fn value(raw: String) -> Result<Value> {
    serde_json::from_str(&raw)
        .map_err(|_| err(ErrorCode::InvalidRequest, "control close response JSON"))
}
fn context() -> OperationContext {
    OperationContext {
        session_id: SessionId::new(900),
        request_id: RequestId::new(1),
        anchor: coordinator_anchor(),
        budget: WorkBudget::default(),
        cancellation_requested: false,
        grants: [Capability::Query, Capability::ControlClock]
            .into_iter()
            .map(|capability| CapabilityGrant {
                capability,
                scope: CapabilityScope::default(),
                max_risk: RiskTier::Reversible,
                expires_at_tick: None,
                remaining_uses: None,
            })
            .collect(),
    }
}
struct Fixture {
    directory: PathBuf,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Result<Self> {
        let directory = std::env::temp_dir()
            .canonicalize()
            .map_err(io_error)?
            .join(format!(
                "dfmcp-control-close-{}-{}",
                std::process::id(),
                FILE_ID.fetch_add(1, Ordering::Relaxed)
            ));
        fs::DirBuilder::new()
            .mode(0o700)
            .create(&directory)
            .map_err(io_error)?;
        let f = Self {
            path: directory.join("effects.bin"),
            directory,
        };
        let c = context();
        let mut journal = open_private_control_journal(&f.path, &c, 7, EffectTailRecovery::Refuse)?;
        for key in ["prepared", "started", "unknown", "applied", "cancelled"] {
            let plan = Digest32::of_bytes(key.as_bytes());
            journal.record_prepared(key.into(), plan, true, 10, 7, [1; 16], &c)?;
            match key {
                "prepared" => {}
                "cancelled" => {
                    journal.cancel_prepared(key, plan, &c)?;
                }
                _ => {
                    journal.begin_commit(key, plan, 7, &c)?;
                    if key == "unknown" {
                        journal.mark_indeterminate(key, plan, &c)?;
                    }
                    if key == "applied" {
                        journal.record_reconciliation(
                            key,
                            plan,
                            7,
                            true,
                            true,
                            true,
                            11,
                            Some(plan),
                            &c,
                        )?;
                    }
                }
            }
        }
        Ok(f)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // Only this fixture's exclusively created paths are removed.
        let _ = fs::remove_file(&self.path);
        let _ = fs::remove_dir(&self.directory);
    }
}
struct Registered {
    id: SessionId,
}
impl Registered {
    fn raw(&self) -> Option<String> {
        Some(self.id.to_string())
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        // A failing assertion must not leak the one global session permit.
        if let Ok(mut sessions) = SESSIONS.lock() {
            if let Some(handle) = sessions.remove(&self.id) {
                match handle.lock() {
                    Ok(mut g) => {
                        g.take();
                    }
                    Err(e) => {
                        e.into_inner().take();
                    }
                }
            }
        }
    }
}
fn register(f: &Fixture, read_only: bool) -> Result<Registered> {
    let c = context();
    let id = next_id()?;
    let slot = Slot::reserve()?;
    let journal = if read_only {
        open_private_control_recovery(&f.path, &c)?
    } else {
        open_private_control_journal(&f.path, &c, 7, EffectTailRecovery::Refuse)?
    };
    let grants = if read_only {
        c.grants
            .into_iter()
            .filter(|g| g.capability == Capability::Query)
            .collect()
    } else {
        c.grants
    };
    let session = ControlSession {
        id,
        connection: None,
        journal,
        request: 0,
        budget: c.budget,
        grants,
        _slot: slot,
    };
    lock(&SESSIONS)?.insert(id, Arc::new(Mutex::new(Some(session))));
    Ok(Registered { id })
}
fn close_call(s: &Registered) -> String {
    fortress_cancel(s.raw(), None, None, None, None, Some("session".into()))
}
fn ok(raw: String) -> Result<Value> {
    let v = value(raw)?;
    assert_eq!(v["result"]["ok"], true, "{v}");
    Ok(v)
}
fn code(raw: String) -> Result<String> {
    value(raw)?["result"]["error"]["code"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| err(ErrorCode::InvalidRequest, "expected an error response"))
}

#[test]
fn close_releases_custody_and_capacity_without_changing_any_effect_state() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let before = fs::read(&f.path).map_err(io_error)?;
    for read_only in [false, true] {
        let s = register(&f, read_only)?;
        let stale = resolve(s.raw())?;
        assert!(Slot::reserve().is_err());
        assert!(open_private_control_recovery(&f.path, &context()).is_err());
        let closed = ok(close_call(&s))?;
        assert_eq!(closed["result"]["mode"], "session_closed");
        assert_eq!(closed["result"]["prepared_effects_cancelled"], false);
        assert_eq!(closed["result"]["reconciliation_performed"], false);
        assert!(closed["result"]["prior_effects_requiring_reconciliation"].is_null());
        assert_eq!(
            closed["agent_turn"]["briefing"]["development_mutation_enabled"],
            false
        );
        assert!(lock(&stale)?.is_none());
        assert_eq!(SLOTS.load(Ordering::Acquire), 0);
        assert_eq!(code(fortress_doctor(s.raw()))?, "session_not_found");
        let reopened = open_private_control_recovery(&f.path, &context())?;
        for (key, state) in [
            ("prepared", DurablePauseState::Prepared),
            ("started", DurablePauseState::CommitStarted),
            ("unknown", DurablePauseState::Indeterminate),
            ("applied", DurablePauseState::VerifiedApplied),
            ("cancelled", DurablePauseState::CancelledBeforeDispatch),
        ] {
            assert_eq!(reopened.lookup(key).map(|r| r.state), Some(state));
        }
        drop(reopened);
        drop(stale);
    }
    assert_eq!(fs::read(&f.path).map_err(io_error)?, before);
    Ok(())
}

#[test]
fn refused_close_keeps_the_session_and_lock_until_a_complete_reply_fits() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let s = register(&f, true)?;
    for bytes in [0, 1, 64, u64::MAX] {
        assert_eq!(
            code(fortress_cancel(
                s.raw(),
                None,
                None,
                Some(bytes),
                None,
                Some("session".into())
            ))?,
            "budget_exceeded"
        );
        ok(fortress_doctor(s.raw()))?;
        assert!(open_private_control_recovery(&f.path, &context()).is_err());
    }
    assert_eq!(
        code(fortress_cancel(
            s.raw(),
            Some("prepared".into()),
            Some(Digest32::of_bytes(b"prepared").to_string()),
            None,
            None,
            Some("session".into())
        ))?,
        "invalid_request"
    );
    assert_eq!(
        code(fortress_cancel(
            s.raw(),
            None,
            None,
            None,
            None,
            Some("all".into())
        ))?,
        "invalid_request"
    );
    assert_eq!(
        code(fortress_cancel(s.raw(), None, None, None, None, None))?,
        "capability_denied"
    );
    ok(close_call(&s))?;
    Ok(())
}

#[test]
fn teardown_survives_revoked_grants_counter_exhaustion_and_fenced_journals() -> Result<()> {
    use std::io::Write;
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let s = register(&f, true)?;
    let h = resolve(s.raw())?;
    {
        let mut owned = lock(&h)?;
        let session = owned
            .as_mut()
            .ok_or_else(|| err(ErrorCode::SessionNotFound, "fixture"))?;
        session.grants.clear();
        session.request = u128::MAX;
    }
    fs::OpenOptions::new()
        .append(true)
        .open(&f.path)
        .and_then(|mut f| f.write_all(b"broken"))
        .map_err(io_error)?;
    let damaged = fs::read(&f.path).map_err(io_error)?;
    ok(close_call(&s))?;
    assert_eq!(fs::read(&f.path).map_err(io_error)?, damaged);
    // Teardown frees the lock but neither repairs damage nor claims recovery.
    assert!(open_private_control_recovery(&f.path, &context()).is_err());
    assert_eq!(SLOTS.load(Ordering::Acquire), 0);
    Ok(())
}

#[test]
fn closing_a_poisoned_session_drops_resources_without_resuming_its_operations() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let s = register(&f, true)?;
    let h = resolve(s.raw())?;
    let same = h.clone();
    let panicked = std::panic::catch_unwind(move || {
        let _guard = same.lock().unwrap_or_else(|e| e.into_inner());
        panic!("injected foreground panic");
    });
    assert!(panicked.is_err());
    assert_eq!(
        code(fortress_doctor(s.raw()))?,
        "internal_invariant_violation"
    );
    ok(close_call(&s))?;
    let owned = h.lock().unwrap_or_else(|e| e.into_inner());
    assert!(owned.is_none());
    drop(owned);
    drop(open_private_control_recovery(&f.path, &context())?);
    Ok(())
}

#[test]
fn close_waits_for_foreground_work_then_old_waiters_cannot_run() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let s = register(&f, true)?;
    let h = resolve(s.raw())?;
    let held = lock(&h)?;
    let started = Arc::new(AtomicBool::new(false));
    let notice = started.clone();
    let raw = s.raw();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let thread = std::thread::spawn(move || {
        notice.store(true, Ordering::Release);
        let out = fortress_cancel(raw, None, None, None, None, Some("session".into()));
        let _ = sender.send(out);
    });
    while !started.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    let premature = receiver.try_recv();
    drop(held);
    let response = receiver.recv_timeout(Duration::from_secs(5));
    let joined = thread.join();
    assert!(matches!(
        premature,
        Err(std::sync::mpsc::TryRecvError::Empty)
    ));
    assert!(joined.is_ok());
    ok(response.map_err(|_| err(ErrorCode::AdapterUnavailable, "close did not drain"))?)?;
    assert!(lock(&h)?.is_none());
    assert_eq!(SLOTS.load(Ordering::Acquire), 0);
    Ok(())
}

#[test]
fn simultaneous_closes_replay_one_receipt_and_release_the_permit_once() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let s = register(&f, false)?;
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let go = barrier.clone();
    let raw = s.raw();
    let closer = std::thread::spawn(move || {
        go.wait();
        fortress_cancel(raw, None, None, None, None, Some("session".into()))
    });
    barrier.wait();
    let first = close_call(&s);
    let second = closer
        .join()
        .map_err(|_| err(ErrorCode::InternalInvariantViolation, "closer panic"))?;
    ok(first.clone())?;
    assert_eq!(first, second);
    assert_eq!(SLOTS.load(Ordering::Acquire), 0);
    let new = register(&f, true)?;
    assert_ne!(new.id, s.id);
    assert_eq!(close_call(&s), first);
    ok(fortress_doctor(new.raw()))?;
    ok(close_call(&new))?;
    Ok(())
}

#[test]
fn receipt_retention_is_bounded_and_replay_does_not_close_a_new_session() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let first = register(&f, true)?;
    let original = close_call(&first);
    ok(original.clone())?;
    for _ in 0..RETAINED_CLOSE_RECEIPTS {
        let next = register(&f, true)?;
        ok(close_call(&next))?;
    }
    assert_eq!(code(close_call(&first))?, "session_not_found");
    let next = register(&f, true)?;
    let raw = close_call(&next);
    ok(raw.clone())?;
    assert_eq!(close_call(&next), raw);
    assert_eq!(
        code(fortress_cancel(
            next.raw(),
            None,
            None,
            Some(1),
            None,
            Some("session".into())
        ))?,
        "budget_exceeded"
    );
    assert_eq!(lock(&CLOSED)?.len(), RETAINED_CLOSE_RECEIPTS);
    let active = register(&f, true)?;
    assert_eq!(close_call(&next), raw);
    ok(fortress_doctor(active.raw()))?;
    ok(close_call(&active))?;
    Ok(())
}

#[test]
fn released_handles_cannot_alias_other_runtime_families_or_new_sessions() -> Result<()> {
    let _serial = lock(&SESSION_TESTS)?;
    let f = Fixture::new()?;
    let s = register(&f, true)?;
    for id in [
        SessionId::NIL,
        SessionId::new(1),
        SessionId::new((1u128 << 127) | (1u128 << 56) | 1),
    ] {
        assert_eq!(
            code(fortress_cancel(
                Some(id.to_string()),
                None,
                None,
                None,
                None,
                Some("session".into())
            ))?,
            "invalid_request"
        );
    }
    ok(fortress_doctor(s.raw()))?;
    ok(close_call(&s))?;
    let next = register(&f, true)?;
    assert_eq!(
        code(fortress_commit(
            s.raw(),
            "prepared".into(),
            Digest32::of_bytes(b"prepared").to_string(),
            "01".repeat(16)
        ))?,
        "session_not_found"
    );
    ok(fortress_doctor(next.raw()))?;
    ok(close_call(&next))?;
    Ok(())
}
