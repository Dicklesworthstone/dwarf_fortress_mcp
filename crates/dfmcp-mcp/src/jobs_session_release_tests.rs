//! Real jobs handlers exercise local teardown without a native endpoint.
//! Stale handles deliberately outlive release, including at the capacity limit.
use super::tests::{decode, register};
use super::*;
use dfmcp_core::GameTick;

struct CountedSource {
    calls: Arc<AtomicUsize>,
    drops: Arc<AtomicUsize>,
    fenced: bool,
}
impl JobSource for CountedSource {
    fn read(&mut self, _: Duration) -> Result<LiveJobObservation> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(error(
            ErrorCode::AdapterUnavailable,
            "unexpected read during session release",
        ))
    }
    fn poisoned(&self) -> bool {
        self.fenced
    }
    fn fence(&mut self) {
        self.fenced = true;
    }
}
impl Drop for CountedSource {
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
    fn new() -> Result<Self> {
        let value = Self {
            id: register()?,
            calls: Arc::new(AtomicUsize::new(0)),
            drops: Arc::new(AtomicUsize::new(0)),
        };
        lock(&resolve(value.handle())?)?.source = Box::new(CountedSource {
            calls: Arc::clone(&value.calls),
            drops: Arc::clone(&value.drops),
            fenced: false,
        });
        Ok(value)
    }
    fn handle(&self) -> Option<String> {
        Some(self.id.to_string())
    }
    fn close(&self, discard: Option<bool>) -> Result<Value> {
        decode(&fortress_cancel(
            self.handle(),
            Some("session".into()),
            discard,
            None,
            None,
        ))
    }
    fn ask(&self, query: Value) -> Result<Value> {
        decode(&fortress_query(
            self.handle(),
            None,
            Some(json!({"schema":"dfmcp.query/1","query":query})),
        ))
    }
    fn retain_query_work(&self) -> Result<()> {
        let captured = self.ask(json!({
            "kind":"capture","key":"release-baseline","max_game_ticks":100,
            "select":{"kind":"entities","kinds":["job"],"fields":["suspended"]}
        }))?;
        assert_eq!(captured["ok"], true, "{captured}");
        let watch = self.ask(json!({
            "kind":"watch","key":"release-watch",
            "condition":{"op":"tick_at_least","value":105u64*403_200+100},
            "deadline_tick":105u64*403_200+200,"stable_observations":1
        }))?;
        assert_eq!(watch["ok"], true, "{watch}");
        Ok(())
    }
}
impl Drop for Registered {
    fn drop(&mut self) {
        if let Ok(mut sessions) = SESSIONS.lock() {
            sessions.remove(&self.id);
        }
        let _ = semantic_query::release_session_resources(self.id, true, |_| Ok(String::new()));
    }
}

#[test]
fn close_releases_the_source_and_fences_every_stale_session_entrypoint() -> Result<()> {
    let _serial = crate::test_serial();
    let before = SLOTS.load(Ordering::SeqCst);
    let session = Registered::new()?;
    let stale = resolve(session.handle())?;
    let context = lock(&stale)?.context()?;
    let encoded = fortress_cancel(session.handle(), Some("session".into()), None, None, None);
    let result = decode(&encoded)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["release_only"], true);
    assert_eq!(result["native_captures"], 0);
    assert_eq!(result["game_effects_cancelled"], false);
    assert_eq!(result["game_state_observed"], false);
    assert!(result["agent_turn"]["anchor"].is_null());
    assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(SLOTS.load(Ordering::SeqCst), before);
    {
        let mut closed = lock(&stale)?;
        let request = closed.request;
        assert!(matches!(closed.context(), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert_eq!(closed.request, request);
        assert!(matches!(closed.refresh(&context), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert!(matches!(closed.anchor(), Err(e) if e.code == ErrorCode::SessionNotFound));
        assert!(
            matches!(query_view(&closed, &context), Err(e) if e.code == ErrorCode::SessionNotFound)
        );
        assert!(closed.state.snapshot().is_none());
        assert!(closed.state.observation().is_none());
        assert!(closed.grants.is_empty());
        assert!(!closed._slot.held);
    }
    assert_eq!(
        fortress_cancel(
            session.handle(),
            Some("session".into()),
            Some(true),
            None,
            None,
        ),
        encoded
    );
    for packet in [
        fortress_observe(session.handle()),
        fortress_query(session.handle(), None, None),
        fortress_wait(session.handle()),
        fortress_doctor(session.handle()),
    ] {
        let packet = decode(&packet)?;
        assert_eq!(packet["error"]["code"], "session_not_found", "{packet}");
        assert!(packet["agent_turn"]["anchor"].is_null());
    }
    drop(stale);
    assert_eq!(SLOTS.load(Ordering::SeqCst), before);
    assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    Ok(())
}

#[test]
fn a_full_runtime_accepts_replacement_sessions_while_closed_arcs_remain() -> Result<()> {
    let _serial = crate::test_serial();
    let before = SLOTS.load(Ordering::SeqCst);
    let sessions = (before..MAX_SESSIONS)
        .map(|_| Registered::new())
        .collect::<Result<Vec<_>>>()?;
    assert!(matches!(SessionSlot::reserve(), Err(e) if e.code == ErrorCode::BudgetExceeded));
    let stale = sessions
        .iter()
        .map(|session| resolve(session.handle()))
        .collect::<Result<Vec<_>>>()?;
    for session in &sessions {
        assert_eq!(session.close(None)?["ok"], true);
        let replacement = Registered::new()?;
        assert_eq!(replacement.close(None)?["ok"], true);
        assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    }
    assert_eq!(SLOTS.load(Ordering::SeqCst), before);
    drop(stale);
    assert_eq!(SLOTS.load(Ordering::SeqCst), before);
    Ok(())
}

#[test]
fn consent_and_complete_receipt_preflight_preserve_query_work_until_success() -> Result<()> {
    let _serial = crate::test_serial();
    let session = Registered::new()?;
    let other = Registered::new()?;
    session.retain_query_work()?;
    other.retain_query_work()?;
    let watches = session.ask(json!({"kind":"watches"}))?["records"].clone();
    let baselines = session.ask(json!({"kind":"baselines"}))?["baselines"].clone();
    let other_watches = other.ask(json!({"kind":"watches"}))?["records"].clone();
    let other_baselines = other.ask(json!({"kind":"baselines"}))?["baselines"].clone();
    for consent in [None, Some(false)] {
        let refused = session.close(consent)?;
        assert_eq!(refused["error"]["code"], "conflict");
        assert!(refused["agent_turn"]["anchor"].is_null());
    }
    let handle = resolve(session.handle())?;
    lock(&handle)?.budget.max_output_tokens = 1;
    let too_small = session.close(Some(true))?;
    lock(&handle)?.budget.max_output_tokens = 8192;
    assert_eq!(too_small["error"]["code"], "budget_exceeded");
    assert_eq!(session.ask(json!({"kind":"watches"}))?["records"], watches);
    assert_eq!(
        session.ask(json!({"kind":"baselines"}))?["baselines"],
        baselines
    );
    assert!(lock(&handle)?._slot.held);
    assert!(!lock(&handle)?.source.closed());
    assert_eq!(session.drops.load(Ordering::SeqCst), 0);
    let result = session.close(Some(true))?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(
        result["process_local_resources"]["process_local_baselines_discarded"],
        1
    );
    assert_eq!(
        result["process_local_resources"]["process_local_watch_records_discarded"],
        1
    );
    assert_eq!(
        other.ask(json!({"kind":"watches"}))?["records"],
        other_watches
    );
    assert_eq!(
        other.ask(json!({"kind":"baselines"}))?["baselines"],
        other_baselines
    );
    assert_eq!(other.close(None)?["error"]["code"], "conflict");
    assert_eq!(other.close(Some(true))?["ok"], true);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(other.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[test]
fn fenced_or_unauthorized_sessions_release_without_read_authority() -> Result<()> {
    let _serial = crate::test_serial();
    for case in 0..4 {
        let session = Registered::new()?;
        let handle = resolve(session.handle())?;
        {
            let mut held = lock(&handle)?;
            match case {
                0 => held.source.fence(),
                1 => held.request = u128::MAX,
                2 => held.grants.clear(),
                _ => {
                    let expired = GameTick(held.anchor()?.tick.0 - 1);
                    for grant in &mut held.grants {
                        grant.expires_at_tick = Some(expired);
                    }
                }
            }
        }
        let result = session.close(None)?;
        assert_eq!(result["ok"], true, "case {case}: {result}");
        assert_eq!(result["game_state_observed"], false);
        assert!(result.get("anchor").is_none());
        assert!(result["agent_turn"]["anchor"].is_null());
        assert_eq!(session.calls.load(Ordering::SeqCst), 0);
        assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    }
    Ok(())
}

#[test]
#[allow(clippy::panic)]
fn a_poisoned_session_can_release_without_reusing_or_unpoisoning_its_state() -> Result<()> {
    let _serial = crate::test_serial();
    let session = Registered::new()?;
    let handle = resolve(session.handle())?;
    let poison = Arc::clone(&handle);
    let panicked = std::thread::spawn(move || {
        if let Ok(_guard) = poison.lock() {
            panic!("injected jobs foreground panic");
        }
    })
    .join()
    .is_err();
    assert!(panicked);
    assert_eq!(decode(&fortress_observe(session.handle()))?["ok"], false);
    let result = session.close(None)?;
    assert_eq!(result["ok"], true, "{result}");
    assert_eq!(result["session_mutex_poisoned"], true);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    assert!(handle.is_poisoned());
    assert_eq!(session.close(None)?, result);
    Ok(())
}

#[test]
fn invalid_scope_and_foreign_handles_cannot_release_a_session() -> Result<()> {
    let _serial = crate::test_serial();
    let session = Registered::new()?;
    assert_eq!(
        decode(&fortress_cancel(session.handle(), None, None, None, None))?["error"]["code"],
        "capability_denied"
    );
    for (scope, discard) in [(None, Some(true)), (Some("game".into()), None)] {
        assert_eq!(
            decode(&fortress_cancel(
                session.handle(),
                scope,
                discard,
                None,
                None,
            ))?["error"]["code"],
            "invalid_request"
        );
    }
    for (mode, legacy_mode) in [
        (Some("action"), None),
        (Some(""), None),
        (None, Some("action")),
        (None, Some("")),
        (Some("action"), Some("action")),
    ] {
        let refused = decode(&fortress_cancel(
            session.handle(),
            Some("session".into()),
            Some(true),
            mode.map(str::to_owned),
            legacy_mode.map(str::to_owned),
        ))?;
        assert_eq!(refused["error"]["code"], "invalid_request", "{refused}");
        assert!(refused["agent_turn"]["anchor"].is_null());
        assert!(resolve(session.handle()).is_ok());
        assert_eq!(session.drops.load(Ordering::SeqCst), 0);
    }
    for handle in [
        None,
        Some("bad-handle".into()),
        Some(format!("{:032x}", (1u128 << 127) | FAMILY | 1)),
        Some(SessionId::new((1u128 << 127) | (2u128 << 60) | 1).to_string()),
    ] {
        assert_eq!(
            decode(&fortress_cancel(
                handle,
                Some("session".into()),
                None,
                None,
                None,
            ))?["error"]["code"],
            "invalid_request"
        );
    }
    assert_eq!(
        decode(&fortress_cancel(
            Some(next_id()?.to_string()),
            Some("session".into()),
            None,
            None,
            None
        ))?["error"]["code"],
        "session_not_found"
    );
    assert!(resolve(session.handle()).is_ok());
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(session.close(None)?["ok"], true);
    let definition = FortressCancel.definition();
    assert_eq!(definition.name, "fortress_cancel");
    for field in [
        "session_id",
        "scope",
        "discard_process_local_work",
        "mode",
        "_mode",
    ] {
        assert!(definition.input_schema["properties"].get(field).is_some());
        assert!(
            !definition.input_schema["required"]
                .as_array()
                .is_some_and(|fields| fields.iter().any(|value| value == field))
        );
    }
    Ok(())
}

#[test]
fn repeated_close_receipts_are_exact_and_have_bounded_retention() -> Result<()> {
    let _serial = crate::test_serial();
    let mut first = None;
    let mut recent = Vec::new();
    for index in 0..33 {
        let session = Registered::new()?;
        let packet = fortress_cancel(session.handle(), Some("session".into()), None, None, None);
        assert_eq!(decode(&packet)?["ok"], true);
        if index == 0 {
            first = session.handle();
        } else {
            recent.push((session.handle(), packet));
        }
    }
    assert_eq!(
        decode(&fortress_cancel(
            first,
            Some("session".into()),
            None,
            None,
            None
        ))?["error"]["code"],
        "session_not_found"
    );
    for (handle, packet) in recent {
        assert_eq!(
            fortress_cancel(handle, Some("session".into()), None, None, None),
            packet
        );
    }
    Ok(())
}

#[test]
fn concurrent_close_requests_return_one_receipt_and_release_once() -> Result<()> {
    let _serial = crate::test_serial();
    let before = SLOTS.load(Ordering::SeqCst);
    let session = Registered::new()?;
    let barrier = Arc::new(std::sync::Barrier::new(3));
    let first_id = session.handle();
    let second_id = session.handle();
    let first_barrier = Arc::clone(&barrier);
    let second_barrier = Arc::clone(&barrier);
    let first = std::thread::spawn(move || {
        first_barrier.wait();
        fortress_cancel(first_id, Some("session".into()), None, None, None)
    });
    let second = std::thread::spawn(move || {
        second_barrier.wait();
        fortress_cancel(second_id, Some("session".into()), None, None, None)
    });
    barrier.wait();
    let first = first.join().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "first closer panicked",
        )
    })?;
    let second = second.join().map_err(|_| {
        error(
            ErrorCode::InternalInvariantViolation,
            "second closer panicked",
        )
    })?;
    assert_eq!(decode(&first)?["ok"], true, "{first}");
    assert_eq!(first, second);
    assert_eq!(session.drops.load(Ordering::SeqCst), 1);
    assert_eq!(session.calls.load(Ordering::SeqCst), 0);
    assert_eq!(SLOTS.load(Ordering::SeqCst), before);
    Ok(())
}
